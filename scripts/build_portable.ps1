#Requires -Version 5.1
<#
.SYNOPSIS
    构建 AnyFile Portable ZIP（release 构建 + portable.flag + 可选 mpv-2.dll + 许可文档）。
.PARAMETER SkipBuild   跳过 cargo build，直接使用已有 release 产物
.PARAMETER MpvDll      可选：mpv-2.dll 路径（默认 third_party\libmpv\mpv-2.dll，存在才附带）
.PARAMETER OutDir      输出目录（默认 <repo>\dist）
#>
[CmdletBinding()]
param([switch]$SkipBuild, [string]$MpvDll = "", [string]$OutDir = "")

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot "dist" }
if (-not $MpvDll) { $MpvDll = Join-Path $RepoRoot "third_party\libmpv\mpv-2.dll" }

$Version = ([regex]::Match((Get-Content (Join-Path $RepoRoot "Cargo.toml") -Raw), '(?m)^version\s*=\s*"([^"]+)"')).Groups[1].Value
Push-Location $RepoRoot
try {
    if (-not $SkipBuild) {
        cargo build --release --bin anyfile
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release 失败" }
    }
    $TargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RepoRoot "target" }
    $Exe = Join-Path $TargetRoot "release\anyfile.exe"
    if (-not (Test-Path $Exe)) { throw "未找到 $Exe" }

    $Stage = Join-Path $OutDir "portable\AnyFile"
    if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
    New-Item -ItemType Directory -Force -Path $Stage | Out-Null
    Copy-Item $Exe $Stage
    Set-Content -Path (Join-Path $Stage "portable.flag") -Value "AnyFile portable mode: settings are stored in .\data" -Encoding UTF8
    foreach ($doc in "LICENSE", "THIRD_PARTY_LICENSES.md", "GPL-COMPLIANCE.md") {
        $p = Join-Path $RepoRoot $doc
        if (Test-Path $p) { Copy-Item $p $Stage }
    }
    $withMpv = Test-Path $MpvDll
    if ($withMpv) { Copy-Item $MpvDll (Join-Path $Stage "mpv-2.dll") } else { Write-Host "[portable] 未找到 mpv-2.dll，视频将不可用（先运行 scripts\fetch_libmpv.ps1）" -ForegroundColor Yellow }

    $Zip = Join-Path $OutDir "AnyFile-$Version-portable-x64.zip"
    if (Test-Path $Zip) { Remove-Item -Force $Zip }
    Compress-Archive -Path $Stage -DestinationPath $Zip -CompressionLevel Optimal
    $size = [math]::Round((Get-Item $Zip).Length / 1MB, 2)
    Write-Host "[portable] 完成：$Zip ($size MB, mpv-2.dll=$withMpv)" -ForegroundColor Green
    $Zip
}
finally { Pop-Location }
