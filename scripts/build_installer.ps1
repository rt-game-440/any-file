#Requires -Version 5.1
<#
.SYNOPSIS
    装配 dist\payload 并调用 ISCC 构建 Windows x64 安装包（不运行安装包）。
#>
[CmdletBinding()]
param([switch]$SkipBuild, [string]$MpvDll = "", [string]$Iscc = "")

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $MpvDll) { $MpvDll = Join-Path $RepoRoot "third_party\libmpv\mpv-2.dll" }
if (-not $Iscc) {
    $Iscc = @((Get-Command ISCC -ErrorAction SilentlyContinue).Source,
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Iscc) { throw "未找到 ISCC.exe：winget install --id JRSoftware.InnoSetup --scope user" }

$Version = ([regex]::Match((Get-Content (Join-Path $RepoRoot "Cargo.toml") -Raw), '(?m)^version\s*=\s*"([^"]+)"')).Groups[1].Value
Push-Location $RepoRoot
try {
    if (-not $SkipBuild) { cargo build --release --bin anyfile; if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" } }
    $TargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RepoRoot "target" }
    $Payload = Join-Path $RepoRoot "dist\payload"
    if (Test-Path $Payload) { Remove-Item -Recurse -Force $Payload }
    New-Item -ItemType Directory -Force -Path $Payload | Out-Null
    Copy-Item (Join-Path $TargetRoot "release\anyfile.exe") $Payload
    foreach ($doc in "LICENSE", "THIRD_PARTY_LICENSES.md", "GPL-COMPLIANCE.md") {
        if (Test-Path (Join-Path $RepoRoot $doc)) { Copy-Item (Join-Path $RepoRoot $doc) $Payload }
    }
    if (Test-Path $MpvDll) { Copy-Item $MpvDll (Join-Path $Payload "mpv-2.dll") }
    & $Iscc "/DAppVersion=$Version" "/DPayloadDir=$Payload" "/DOutputDir=$(Join-Path $RepoRoot 'dist')" (Join-Path $RepoRoot "installer\anyfile.iss")
    if ($LASTEXITCODE -ne 0) { throw "ISCC 编译失败" }
    Write-Host "[installer] 完成：dist\AnyFile-$Version-setup-x64.exe" -ForegroundColor Green
}
finally { Pop-Location }
