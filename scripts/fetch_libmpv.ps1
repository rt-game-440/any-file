#Requires -Version 5.1
<#
.SYNOPSIS
    下载并校验 AnyFile 使用的官方 libmpv Windows x64 二进制（mpv-2.dll）。

.DESCRIPTION
    从官方 shinchiro/mpv-winbuild-cmake GitHub Releases 获取固定版本的 mpv-dev-x86_64 归档，
    校验 SHA256 摘要，解压 libmpv-2.dll（同时提供 mpv-2.dll 副本供安全动态加载器加载）与头文件，
    并记录上游来源与构建信息以满足 GPLv3 合规要求（ADR-0001 / ADR-0005）。
#>
[CmdletBinding()]
param(
    [string]$OutDir = "",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

# 固定上游发布版本（GPLv3 合规可复现构建元数据）
$ReleaseTag   = "20261111"
$AssetName    = "mpv-dev-x86_64-20261111-git-bcde6b93cc.7z"
$DownloadUrl  = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/$ReleaseTag/$AssetName"
$ExpectedHash = "273c079b11ea5575822f972756f4c9a71149a5c10130353abeb8586738938452"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $RepoRoot "third_party\libmpv"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$ArchivePath = Join-Path $OutDir $AssetName
$MpvDllPath  = Join-Path $OutDir "mpv-2.dll"
$LibMpvPath  = Join-Path $OutDir "libmpv-2.dll"

if (-not $Force -and (Test-Path $MpvDllPath)) {
    Write-Host "[fetch_libmpv] 已存在 $MpvDllPath，跳过下载（如需覆盖请加 -Force）。"
    exit 0
}

if ($Force -or -not (Test-Path $ArchivePath)) {
    Write-Host "[fetch_libmpv] 正在下载 $DownloadUrl ..."
    $ghCmd = Get-Command gh -ErrorAction SilentlyContinue
    if ($ghCmd) {
        & gh release download $ReleaseTag --repo shinchiro/mpv-winbuild-cmake --pattern $AssetName --dir $OutDir --clobber
    } else {
        Invoke-WebRequest -Uri $DownloadUrl -OutFile $ArchivePath -UseBasicParsing
    }
}

Write-Host "[fetch_libmpv] 正在校验 SHA256 ..."
$ActualHash = (Get-FileHash -Algorithm SHA256 -Path $ArchivePath).Hash.ToLowerInvariant()
if ($ActualHash -ne $ExpectedHash) {
    throw "SHA256 校验失败！期望: $ExpectedHash，实际: $ActualHash"
}
Write-Host "[fetch_libmpv] SHA256 校验通过: $ActualHash"

$SevenZip = Get-Command 7z -ErrorAction SilentlyContinue
if (-not $SevenZip) {
    throw "未找到 7z 命令，请先安装 7-Zip 并加入 PATH。"
}

Write-Host "[fetch_libmpv] 正在解压 libmpv-2.dll 与头文件 ..."
& 7z e -y "-o$OutDir" $ArchivePath "libmpv-2.dll" | Out-Null
& 7z x -y "-o$OutDir" $ArchivePath "include\mpv\*" | Out-Null

if (-not (Test-Path $LibMpvPath)) {
    throw "归档中未找到 libmpv-2.dll"
}

# AnyFile 统一按 mpv-2.dll 名称加载（ADR-0005）
Copy-Item -Path $LibMpvPath -Destination $MpvDllPath -Force

$DllHash = (Get-FileHash -Algorithm SHA256 -Path $MpvDllPath).Hash.ToLowerInvariant()
Write-Host "[fetch_libmpv] 完成：$MpvDllPath (SHA256=$DllHash)"
