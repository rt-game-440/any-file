<#
.SYNOPSIS
    AnyFile 冷启动首帧渲染耗时自动化基准测试脚本（ADR-0002 / 工单 #2）。
.DESCRIPTION
    构建 release 版 anyfile.exe 并连续执行多次冷启动，测量“进程入口到首帧渲染完成”的耗时毫秒数，
    输出 min / median / p95 / max 汇总指标，作为后续 < 150-250ms 回归守护基线。
#>

param(
    [int]$Runs = 5,
    [switch]$SkipBuild,
    [double]$TargetMs = 250.0
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot

try {
    if (-not $SkipBuild) {
        Write-Host "[AnyFile Bench] 正在构建 release 可执行文件..." -ForegroundColor Cyan
        cargo build --release --bin anyfile
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release 失败（退出码 $LASTEXITCODE）"
        }
    }

    $TargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RepoRoot "target" }
    $ExePath = Join-Path $TargetRoot "release\anyfile.exe"
    if (-not (Test-Path $ExePath)) {
        throw "未找到 release 可执行文件: $ExePath"
    }

    Write-Host "[AnyFile Bench] 开始执行 $Runs 次冷启动基准（目标基线 < $TargetMs ms）..." -ForegroundColor Cyan
    $Samples = New-Object System.Collections.Generic.List[double]
    $Renderer = "Unknown"

    for ($i = 1; $i -le $Runs; $i++) {
        $OutFile = Join-Path ([System.IO.Path]::GetTempPath()) ("anyfile-bench-{0}-{1}.txt" -f $PID, $i)
        if (Test-Path $OutFile) { Remove-Item -Force $OutFile }

        $WallSw = [System.Diagnostics.Stopwatch]::StartNew()
        $Proc = Start-Process -FilePath $ExePath -ArgumentList @("--benchmark-first-frame", "--benchmark-out", $OutFile) -Wait -PassThru
        $WallSw.Stop()

        if ($Proc.ExitCode -ne 0) {
            throw "第 $i 次运行失败（退出码 $($Proc.ExitCode)）"
        }

        if (-not (Test-Path $OutFile)) {
            throw "第 $i 次运行未生成基准输出文件: $OutFile"
        }

        $Content = Get-Content -Path $OutFile -Raw -Encoding UTF8
        Remove-Item -Force $OutFile

        $Line = $Content -split "`r?`n" | Where-Object { $_ -match "^ANYFILE_BENCH_FIRST_FRAME_MS=" } | Select-Object -First 1
        if (-not $Line) {
            throw "第 $i 次运行未输出耗时标记，原始输出: $Content"
        }

        if ($Line -match 'ANYFILE_BENCH_FIRST_FRAME_MS=([0-9\.]+)\s+RENDERER="([^"]+)"') {
            $FirstFrameMs = [double]$Matches[1]
            $Renderer = $Matches[2]
            $Samples.Add($FirstFrameMs)
            Write-Host ("  Run #{0}: first_frame={1:F2} ms | process_wall={2:F2} ms | renderer={3}" -f $i, $FirstFrameMs, $WallSw.Elapsed.TotalMilliseconds, $Renderer)
        } else {
            throw "无法解析输出行: $Line"
        }
    }

    $Sorted = $Samples | Sort-Object
    $Count = $Sorted.Count
    $Min = $Sorted[0]
    $Max = $Sorted[$Count - 1]
    $Median = if ($Count % 2 -eq 1) {
        $Sorted[[int]($Count / 2)]
    } else {
        ($Sorted[$Count / 2 - 1] + $Sorted[$Count / 2]) / 2.0
    }
    $P95Index = [Math]::Min($Count - 1, [int][Math]::Ceiling($Count * 0.95) - 1)
    $P95 = $Sorted[$P95Index]

    Write-Host ""
    Write-Host "================ AnyFile 冷启动首帧渲染基准汇总 ================" -ForegroundColor Green
    Write-Host ("  可执行文件 : {0}" -f $ExePath)
    Write-Host ("  渲染后端   : {0}" -f $Renderer)
    Write-Host ("  采样次数   : {0}" -f $Count)
    Write-Host ("  Min        : {0:F2} ms" -f $Min)
    Write-Host ("  Median     : {0:F2} ms" -f $Median)
    Write-Host ("  P95        : {0:F2} ms" -f $P95)
    Write-Host ("  Max        : {0:F2} ms" -f $Max)
    Write-Host ("  目标基线   : < {0:F0} ms ({1})" -f $TargetMs, $(if ($Median -lt $TargetMs) { "PASS" } else { "WARN: 超出目标基线" }))
    Write-Host "================================================================" -ForegroundColor Green
}
finally {
    Pop-Location
}
