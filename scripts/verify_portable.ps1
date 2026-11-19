#Requires -Version 5.1
<#
.SYNOPSIS
    验证 Portable ZIP：解压到临时目录 -> 启动 exe 打开样例文件 -> 确认配置写入本地 data，且 %APPDATA%\AnyFile 未被创建/修改。
.PARAMETER Zip  待验证 ZIP（默认 dist\ 下最新的 portable zip）
#>
[CmdletBinding()]
param([string]$Zip = "", [int]$WaitSeconds = 4)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $Zip) {
    $Zip = (Get-ChildItem (Join-Path $RepoRoot "dist") -Filter "AnyFile-*-portable-x64.zip" | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
}
if (-not $Zip -or -not (Test-Path $Zip)) { throw "未找到 portable zip，请先运行 scripts\build_portable.ps1" }

function Get-AppDataSnapshot {
    $dir = Join-Path $env:APPDATA "AnyFile"
    if (-not (Test-Path $dir)) { return "<absent>" }
    (Get-ChildItem $dir -Recurse -File | Sort-Object FullName | ForEach-Object { "$($_.FullName)|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)" }) -join "`n"
}

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("anyfile-portable-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
$failures = New-Object System.Collections.Generic.List[string]
try {
    Expand-Archive -Path $Zip -DestinationPath $Tmp
    $AppDir = Join-Path $Tmp "AnyFile"
    $Exe = Join-Path $AppDir "anyfile.exe"
    if (-not (Test-Path $Exe)) { throw "ZIP 内缺少 AnyFile\anyfile.exe" }
    if (-not (Test-Path (Join-Path $AppDir "portable.flag"))) { $failures.Add("ZIP 内缺少 portable.flag") }
    if (Test-Path (Join-Path $AppDir "data")) { $failures.Add("ZIP 不应预置 data 目录") }

    $Sample = Join-Path $Tmp "sample.txt"
    Set-Content -Path $Sample -Value "AnyFile portable verification" -Encoding UTF8

    $before = Get-AppDataSnapshot
    $p = Start-Process -FilePath $Exe -ArgumentList "`"$Sample`"" -PassThru
    Start-Sleep -Seconds $WaitSeconds
    if ($p.HasExited) { $failures.Add("进程过早退出，退出码 $($p.ExitCode)") }
    else { [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(5000)) { $p.Kill() } }
    $after = Get-AppDataSnapshot

    $Data = Join-Path $AppDir "data"
    if (-not (Test-Path $Data)) { $failures.Add("未创建本地 data 目录") }
    else {
        $files = Get-ChildItem $Data -Recurse -File
        if (-not $files) { $failures.Add("data 目录为空（配置/最近列表/日志均未写入）") }
        Write-Host ("[verify_portable] data 内文件: " + (($files | ForEach-Object { $_.Name }) -join ", "))
    }
    if ($before -ne $after) { $failures.Add("%APPDATA%\AnyFile 被创建或修改") }

    if ($failures.Count) { $failures | ForEach-Object { Write-Host "  FAIL: $_" -ForegroundColor Red }; exit 1 }
    Write-Host "[verify_portable] 通过：配置写入本地 data，%APPDATA%\AnyFile 未受污染" -ForegroundColor Green
}
finally {
    Get-Process anyfile -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($Tmp) } | Stop-Process -Force
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
