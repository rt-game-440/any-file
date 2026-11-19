#Requires -Version 5.1
<#
.SYNOPSIS
    全量样例语料 E2E 冒烟 + 冷启动基准（结果写入 logs\）。
.DESCRIPTION
    在临时便携目录复制 release exe（含 portable.flag，不污染 %APPDATA%），生成覆盖
    图像/视频/音频/文本/兜底/损坏/错后缀/无扩展名的语料，逐个启动打开，确认存活一段时间后正常关闭、无崩溃。
    随后对 PNG 与 TXT 目标各测 N 次 --benchmark-first-frame 冷启动，输出 min/median/p95/max。
#>
[CmdletBinding()]
param([int]$Runs = 15, [int]$AliveSeconds = 3, [double]$TargetMs = 250, [switch]$SkipBench)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RepoRoot "target" }
$SrcExe = Join-Path $TargetRoot "release\anyfile.exe"
if (-not (Test-Path $SrcExe)) { throw "未找到 $SrcExe，请先 cargo build --release" }

$LogDir = Join-Path $RepoRoot "logs"; New-Item -ItemType Directory -Force $LogDir | Out-Null
$Stamp = Get-Date -Format "yyyy-MM-dd HH-mm-ss"
$LogFile = Join-Path $LogDir "$Stamp e2e.log"
function Log($m) { Write-Host $m; Add-Content -Path $LogFile -Value $m -Encoding UTF8 }

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("anyfile-e2e-" + [guid]::NewGuid().ToString("N"))
$App = Join-Path $Tmp "app"; $Corpus = Join-Path $Tmp "corpus"
New-Item -ItemType Directory -Force $App, $Corpus | Out-Null
Copy-Item $SrcExe $App
Set-Content (Join-Path $App "portable.flag") "e2e" -Encoding UTF8
$Mpv = Join-Path $RepoRoot "third_party\libmpv\mpv-2.dll"
if (Test-Path $Mpv) { Copy-Item $Mpv $App }
$Exe = Join-Path $App "anyfile.exe"

try {
    # ---- 语料 ----
    $png = Join-Path $Corpus "red.png"
    & ffmpeg -v error -y -f lavfi -i "color=c=red:s=640x360" -frames:v 1 $png
    & ffmpeg -v error -y -i $png (Join-Path $Corpus "red.jpg")
    & ffmpeg -v error -y -i $png (Join-Path $Corpus "red.bmp")
    & ffmpeg -v error -y -i $png (Join-Path $Corpus "red.webp")
    & ffmpeg -v error -y -f lavfi -i "testsrc=s=320x240:d=2:r=24" -f lavfi -i "sine=f=440:d=2" -shortest -c:v libx264 -pix_fmt yuv420p -c:a aac (Join-Path $Corpus "clip.mp4")
    & ffmpeg -v error -y -f lavfi -i "sine=f=440:d=2" (Join-Path $Corpus "tone.wav")
    & ffmpeg -v error -y -f lavfi -i "sine=f=440:d=2" (Join-Path $Corpus "tone.mp3")
    Copy-Item (Join-Path $RepoRoot "tests\fixtures\image\*") $Corpus
    Copy-Item (Join-Path $RepoRoot "tests\fixtures\audio\*") $Corpus
    Set-Content (Join-Path $Corpus "vector.svg") '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#2563EB"/></svg>'
    Set-Content (Join-Path $Corpus "notes.txt") "AnyFile 文本`r`n第二行" -Encoding UTF8
    Set-Content (Join-Path $Corpus "main.rs") "fn main() { println!(`"hi`"); }" -Encoding UTF8
    Set-Content (Join-Path $Corpus "Makefile") "all:`n`techo hi"
    [IO.File]::WriteAllBytes((Join-Path $Corpus "legacy_gbk.txt"), [Text.Encoding]::GetEncoding(936).GetBytes("中文GBK测试内容，用于编码探测。"))
    [IO.File]::WriteAllBytes((Join-Path $Corpus "utf16.txt"), ([byte[]](0xFF,0xFE) + [Text.Encoding]::Unicode.GetBytes("UTF16 文本")))
    Copy-Item $png (Join-Path $Corpus "actually_image.txt")
    [IO.File]::WriteAllBytes((Join-Path $Corpus "corrupted.png"), ([byte[]](0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A) + [Text.Encoding]::ASCII.GetBytes("CORRUPTED")))
    [IO.File]::WriteAllBytes((Join-Path $Corpus "opaque.bin"), [byte[]](0..255 + 0..255))
    [IO.File]::WriteAllBytes((Join-Path $Corpus "empty.dat"), [byte[]]@())

    # ---- 逐个打开 ----
    $files = Get-ChildItem $Corpus -File | Sort-Object Name
    Log "== E2E 冒烟：$($files.Count) 个文件，渲染器由 exe 自选，存活窗口 ${AliveSeconds}s =="
    $fail = 0
    foreach ($f in $files) {
        $p = Start-Process -FilePath $Exe -ArgumentList "`"$($f.FullName)`"" -PassThru
        Start-Sleep -Seconds $AliveSeconds
        if ($p.HasExited) { Log ("FAIL  {0}  过早退出 code={1}" -f $f.Name, $p.ExitCode); $fail++; continue }
        $closed = $p.CloseMainWindow()
        if (-not $p.WaitForExit(5000)) { Log ("FAIL  {0}  关闭超时" -f $f.Name); $p.Kill(); $fail++; continue }
        if ($p.ExitCode -ne 0) { Log ("FAIL  {0}  退出码 {1}" -f $f.Name, $p.ExitCode); $fail++; continue }
        Log ("OK    {0}" -f $f.Name)
    }
    Log "== 冒烟结果：通过 $($files.Count - $fail) / $($files.Count)，崩溃/失败 $fail =="

    # ---- 冷启动基准 ----
    $benchFail = $false
    if (-not $SkipBench) {
        foreach ($t in @(@("图片", $png), @("文本", (Join-Path $Corpus "notes.txt")), @("无参数(空态)", ""))) {
            $vals = @()
            for ($i = 0; $i -lt $Runs; $i++) {
                $out = Join-Path $Tmp "bench.txt"; Remove-Item $out -ErrorAction SilentlyContinue
                $args = @("--benchmark-first-frame", "--benchmark-out", "`"$out`"")
                if ($t[1]) { $args += "`"$($t[1])`"" }
                $p = Start-Process -FilePath $Exe -ArgumentList $args -PassThru -Wait
                if (Test-Path $out) {
                    $m = [regex]::Match((Get-Content $out -Raw), 'ANYFILE_BENCH_FIRST_FRAME_MS=([0-9.]+)')
                    if ($m.Success) { $vals += [double]$m.Groups[1].Value }
                }
            }
            if ($vals.Count -eq 0) { Log "基准[$($t[0])]：无有效采样"; $benchFail = $true; continue }
            $s = $vals | Sort-Object
            $med = $s[[int][math]::Floor($s.Count / 2)]; $p95 = $s[[int][math]::Min($s.Count - 1, [math]::Ceiling($s.Count * 0.95) - 1)]
            $ok = $med -lt $TargetMs
            if ($t[0] -ne "无参数(空态)" -and -not $ok) { $benchFail = $true }
            Log ("基准[{0}] n={1} min={2:F1} median={3:F1} p95={4:F1} max={5:F1} ms  目标<{6}ms {7}" -f $t[0], $s.Count, $s[0], $med, $p95, $s[-1], $TargetMs, $(if ($ok) { "PASS" } else { "FAIL" }))
        }
    }
    Log "日志：$LogFile"
    if ($fail -gt 0 -or $benchFail) { exit 1 }
}
finally {
    Get-Process anyfile -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($Tmp) } | Stop-Process -Force
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
