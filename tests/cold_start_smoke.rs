use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// 获取当前测试构建生成的真实 `anyfile.exe` 路径
fn built_binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_anyfile"))
}

#[test]
fn compiled_binary_embeds_per_monitor_v2_and_long_path_manifest() {
    let exe_path = built_binary_path();
    assert!(
        exe_path.exists(),
        "未找到编译产物可执行文件: {}",
        exe_path.display()
    );

    let exe_bytes = fs::read(&exe_path).expect("读取编译产物失败");
    let has_per_monitor_v2 = exe_bytes
        .windows(b"PerMonitorV2".len())
        .any(|w| w == b"PerMonitorV2");
    let has_long_path_aware = exe_bytes
        .windows(b"longPathAware".len())
        .any(|w| w == b"longPathAware");

    assert!(
        has_per_monitor_v2,
        "编译产物 {} 中未检测到嵌入的 PerMonitorV2 清单声明",
        exe_path.display()
    );
    assert!(
        has_long_path_aware,
        "编译产物 {} 中未检测到嵌入的 longPathAware 清单声明",
        exe_path.display()
    );
}

#[test]
fn cold_start_first_frame_benchmark_renders_and_exits_cleanly() {
    let exe_path = built_binary_path();
    let output = Command::new(&exe_path)
        .arg("--benchmark-first-frame")
        .output()
        .expect("启动 anyfile 真实二进制失败");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "anyfile --benchmark-first-frame 异常退出: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout,
        stderr
    );

    let metric_line = stdout
        .lines()
        .find(|line| line.starts_with("ANYFILE_BENCH_FIRST_FRAME_MS="))
        .unwrap_or_else(|| {
            panic!(
                "标准输出中缺少冷启动首帧耗时标记。\nstdout: {}\nstderr: {}",
                stdout, stderr
            )
        });

    let ms_str = metric_line
        .trim_start_matches("ANYFILE_BENCH_FIRST_FRAME_MS=")
        .split_whitespace()
        .next()
        .expect("无法解析耗时数值");
    let ms: f64 = ms_str
        .parse()
        .unwrap_or_else(|e| panic!("耗时数值解析失败 `{ms_str}`: {e}"));

    assert!(
        ms > 0.0 && ms < 30_000.0,
        "首帧渲染耗时超出合理范围: {ms:.2} ms"
    );
}
