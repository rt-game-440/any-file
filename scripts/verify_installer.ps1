#Requires -Version 5.1
<#
.SYNOPSIS
    编译 installer/anyfile.iss 并静态检查注册表项与卸载清理的对称性（不运行安装包、不写注册表）。
.DESCRIPTION
    1. 在临时目录放置桩 payload，调用 ISCC 真实编译，并导出 ISPP 预处理后的完整脚本；
    2. 检查 [Registry]：仅 HKA 根；每条均带 uninsdeletekey/uninsdeletevalue；写入路径落在白名单前缀内；
       四类 ProgID 与 Tasks 一一对应；RegisteredApplications 指向 Capabilities；
       每个 Capabilities\FileAssociations 扩展名都有对应的 OpenWithProgids（反之亦然）；
    3. 检查 [Run] 含 ms-settings:defaultapps，[Setup] 含免 UAC 与可切换全机安装配置。
    未找到 ISCC 时退出码 2（不视为通过）。
#>
[CmdletBinding()]
param([string]$Iscc = "")

$ErrorActionPreference = "Stop"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$Script = Join-Path $RepoRoot "installer\anyfile.iss"

if (-not $Iscc) {
    $cands = @(
        (Get-Command ISCC -ErrorAction SilentlyContinue).Source,
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path $_) }
    $Iscc = $cands | Select-Object -First 1
}
if (-not $Iscc) { Write-Host "[verify_installer] 未找到 ISCC.exe"; exit 2 }

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("anyfile-iss-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
try {
    $Payload = Join-Path $Tmp "payload"; New-Item -ItemType Directory -Force $Payload | Out-Null
    Set-Content (Join-Path $Payload "anyfile.exe") "stub"; Set-Content (Join-Path $Payload "LICENSE") "stub"
    $Dump = Join-Path $Tmp "dump.iss"
    & $Iscc "/Q" "/DPayloadDir=$Payload" "/DOutputDir=$Tmp\out" "/DDumpPreprocessed=$Dump" $Script
    if ($LASTEXITCODE -ne 0) { throw "ISCC 编译失败（退出码 $LASTEXITCODE）" }
    if (-not (Get-ChildItem "$Tmp\out" -Filter *.exe)) { throw "未产出安装包" }
    Write-Host "[verify_installer] ISCC 编译通过"

    $errors = New-Object System.Collections.Generic.List[string]
    $text = Get-Content $Dump -Raw -Encoding UTF8
    $lines = $text -split "`r?`n"

    function Section($name) {
        $in = $false; $out = @()
        foreach ($l in $lines) {
            if ($l -match '^\[(\w+)\]') { $in = ($Matches[1] -eq $name); continue }
            if ($in -and $l.Trim() -and -not $l.Trim().StartsWith(";")) { $out += $l }
        }
        $out
    }

    $reg = Section "Registry"
    if ($reg.Count -lt 100) { $errors.Add("Registry 条目过少: $($reg.Count)") }
    $allowed = @('^Software\\Classes\\AnyFile\.(Image|Video|Audio|Text)(\\|$)',
                 '^Software\\Classes\\Applications\\anyfile\.exe(\\|$)',
                 '^Software\\Classes\\\.[A-Za-z0-9]+\\OpenWithProgids$',
                 '^Software\\AnyFile(\\|$)',
                 '^Software\\RegisteredApplications$')
    $owp = @{}; $caps = @{}
    foreach ($l in $reg) {
        $root = if ($l -match 'Root:\s*(\w+)') { $Matches[1] } else { '' }
        $sub = if ($l -match 'Subkey:\s*"([^"]+)"') { $Matches[1] } else { '' }
        $flags = if ($l -match 'Flags:\s*([^;]+)') { $Matches[1] } else { '' }
        if ($root -ne 'HKA') { $errors.Add("非 HKA 根: $l") }
        if (-not ($allowed | Where-Object { $sub -match $_ })) { $errors.Add("写入路径不在白名单: $sub") }
        if ($flags -notmatch 'uninsdeletekey|uninsdeletevalue|uninsdeletekeyifempty') { $errors.Add("缺少卸载清理标记: $l") }
        if ($flags -match 'uninsneveruninstall') { $errors.Add("禁止 uninsneveruninstall: $l") }
        # 扩展名键本身不得整键删除（会破坏其他程序的关联）
        if ($sub -match '^Software\\Classes\\\.[^\\]+$') { $errors.Add("不得直接写扩展名键: $sub") }
        if ($sub -match 'OpenWithProgids$' -and $flags -notmatch 'uninsdeletevalue') { $errors.Add("OpenWithProgids 必须按值删除: $l") }
        if ($sub -match '^Software\\Classes\\(\.[^\\]+)\\OpenWithProgids$') {
            $ext = $Matches[1]
            $owp["$ext|" + [regex]::Match($l,'ValueName:\s*"([^"]+)"').Groups[1].Value] = 1
        }
        if ($sub -eq 'Software\AnyFile\Capabilities\FileAssociations') {
            $n = [regex]::Match($l,'ValueName:\s*"([^"]+)"').Groups[1].Value
            $d = [regex]::Match($l,'ValueData:\s*"([^"]+)"').Groups[1].Value
            $caps["$n|$d"] = 1
        }
    }
    # 对称：每个 Capabilities 扩展名都有 OpenWithProgids，反之亦然
    foreach ($k in $caps.Keys) { if (-not $owp.ContainsKey($k)) { $errors.Add("Capabilities 有而 OpenWithProgids 无: $k") } }
    foreach ($k in $owp.Keys) { if ($k -like '.*' -and -not $caps.ContainsKey($k)) { $errors.Add("OpenWithProgids 有而 Capabilities 无: $k") } }
    if ($caps.Count -lt 50) { $errors.Add("文件关联扩展名过少: $($caps.Count)") }

    foreach ($pair in @(@('Image','assoc_image'),@('Video','assoc_video'),@('Audio','assoc_audio'),@('Text','assoc_text'))) {
        $p = "AnyFile.$($pair[0])"
        $cmd = $reg | Where-Object { $_ -match [regex]::Escape("Software\Classes\$p\shell\open\command") }
        if (-not $cmd) { $errors.Add("缺少 $p shell\open\command") }
        $nonTask = $reg | Where-Object { $_ -match "Classes\\$([regex]::Escape($p))" -and $_ -notmatch "Tasks:\s*$($pair[1])" }
        if ($nonTask) { $errors.Add("$p 条目未受任务 $($pair[1]) 约束") }
        $assoc = $reg | Where-Object { $_ -match "ValueData:\s*`"$([regex]::Escape($p))`"" -and $_ -notmatch "Tasks:\s*$($pair[1])" }
        if ($assoc) { $errors.Add("$p 关联条目未受任务约束") }
        if (-not ($caps.Keys | Where-Object { $_ -like "*|$p" })) { $errors.Add("Capabilities 未包含 $p 的扩展名") }
    }
    $ra = $reg | Where-Object { $_ -match 'Subkey:\s*"Software\\RegisteredApplications"' }
    if (-not ($ra -match 'ValueData:\s*"Software\\AnyFile\\Capabilities"' -and $ra -match 'uninsdeletevalue')) { $errors.Add("RegisteredApplications 未正确指向 Capabilities 或不可卸载") }

    $setup = (Section "Setup") -join "`n"
    if ($setup -notmatch 'PrivilegesRequired=lowest') { $errors.Add("缺少 PrivilegesRequired=lowest") }
    if ($setup -notmatch 'PrivilegesRequiredOverridesAllowed=dialog') { $errors.Add("缺少 PrivilegesRequiredOverridesAllowed=dialog") }
    if ($setup -notmatch 'ChangesAssociations=yes') { $errors.Add("缺少 ChangesAssociations=yes") }
    if (((Section "Run") -join "`n") -notmatch 'ms-settings:defaultapps') { $errors.Add("[Run] 缺少 ms-settings:defaultapps") }

    if ($errors.Count) { $errors | ForEach-Object { Write-Host "  FAIL: $_" -ForegroundColor Red }; exit 1 }
    Write-Host ("[verify_installer] 通过：Registry {0} 条，文件关联扩展名 {1} 个，卸载清理对称" -f $reg.Count, $caps.Count) -ForegroundColor Green
}
finally { Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue }
