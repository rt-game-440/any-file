#Requires -Version 5.1
<#
.SYNOPSIS
    v0.1 一键自动验收入口：release 构建 -> 安装脚本校验 -> Portable 构建与校验 -> E2E 冒烟与冷启动基准 -> 深浅色截图检查。
    不运行安装包，不写文件关联注册表（这些属人工验收，见 docs/release-acceptance.md）。
#>
[CmdletBinding()]
param([switch]$SkipBuild)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = "E:\devlib\anyfile-target" }
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = "2" }
Push-Location $Root
$results = [ordered]@{}
function Step($name, [scriptblock]$sb) {
    Write-Host "`n>>> $name" -ForegroundColor Cyan
    & $sb
    $results[$name] = ($LASTEXITCODE -eq 0)
}
try {
    if (-not $SkipBuild) { Step "release 构建" { cargo build --release --bin anyfile } }
    Step "安装脚本校验" { powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify_installer.ps1 }
    Step "Portable 构建" { powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build_portable.ps1 -SkipBuild | Out-Host }
    Step "Portable 校验" { powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify_portable.ps1 }
    Step "E2E 冒烟与冷启动基准" { powershell -NoProfile -ExecutionPolicy Bypass -File scripts\e2e_smoke.ps1 }
    Step "深浅色截图检查" { cargo test --test e2e_screenshots -- --test-threads=1 }
    Write-Host "`n===== 汇总 ====="
    $results.GetEnumerator() | ForEach-Object { "{0,-24} {1}" -f $_.Key, $(if ($_.Value) { "PASS" } else { "FAIL" }) }
    if ($results.Values -contains $false) { exit 1 }
}
finally { Pop-Location }
