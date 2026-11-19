//! 工单 #8：libmpv 安全动态加载、沙箱隔离与视频解码契约的行为测试。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyfile::ports::VideoBackend;
use anyfile::video::{
    CompositingStrategy, LazyMpvVideoBackend, MpvInstance, MpvLibrary, MpvSandboxPolicy,
    VideoError, is_mpv_module_loaded_in_process, validate_local_media_path,
};

#[test]
fn sandbox_policy_enforces_no_config_no_scripts_no_ytdl_and_local_protocol_whitelist() {
    let options = MpvSandboxPolicy::required_options();

    assert!(
        options.contains(&("config", "no")),
        "必须禁用 mpv 外部/用户配置"
    );
    assert!(
        options.contains(&("load-scripts", "no")),
        "必须禁用 mpv Lua/JS 脚本加载"
    );
    assert!(
        options.contains(&("ytdl", "no")),
        "必须禁用 yt-dlp 网络解析钩子"
    );
    assert!(
        options
            .iter()
            .any(|(k, v)| *k == "demuxer-lavf-o" && v.contains("protocol_whitelist=file")),
        "必须通过 demuxer-lavf-o 限制协议白名单为本地文件"
    );
}

#[test]
fn local_media_path_guard_rejects_network_and_unsafe_schemes_while_allowing_local_paths() {
    let blocked = [
        "http://example.com/video.mp4",
        "HTTPS://example.com/stream.m3u8",
        "rtsp://127.0.0.1:554/live",
        "rtmp://example.com/live/stream",
        "ftp://example.com/a.mp4",
        "edl://%10%http://evil",
        "smb://server/share/a.mp4",
        "av://lavfi:testsrc",
        "",
    ];

    for uri in blocked {
        let err = validate_local_media_path(Path::new(uri))
            .err()
            .unwrap_or_else(|| panic!("应拒绝非本地协议或空路径: {uri}"));
        assert!(
            matches!(err, VideoError::UnsafeMediaPath(_)),
            "拒绝原因应为 UnsafeMediaPath，实际为 {err:?}"
        );
    }

    let allowed = [
        r"C:\Videos\测试视频.mp4",
        r"\\?\C:\Very\Long\Path\clip.mkv",
        r"\\nas-server\media\movie.webm",
        r"relative\sample.mp4",
    ];

    for path in allowed {
        let normalized = validate_local_media_path(Path::new(path))
            .unwrap_or_else(|e| panic!("应允许合法本地/UNC 路径 {path}: {e}"));
        assert!(!normalized.is_empty());
    }
}

fn third_party_mpv_dll() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("third_party")
        .join("libmpv")
        .join("mpv-2.dll")
}

#[test]
fn loader_rejects_relative_and_missing_dll_paths_before_loading() {
    let rel_err = MpvLibrary::load_from_absolute_path(Path::new("mpv-2.dll"))
        .expect_err("必须拒绝相对 DLL 路径以防 DLL 劫持");
    assert!(matches!(rel_err, VideoError::RelativeDllPathRejected(_)));

    let missing_path = std::env::temp_dir().join("anyfile-nonexistent-dir-99999").join("mpv-2.dll");
    let missing_err = MpvLibrary::load_from_absolute_path(&missing_path)
        .expect_err("不存在的绝对路径应返回 LibraryNotFound");
    assert!(matches!(missing_err, VideoError::LibraryNotFound(_)));
}

#[test]
fn lazy_video_backend_does_not_load_mpv_dll_on_construction() {
    let backend = LazyMpvVideoBackend::for_exe_dir();
    assert!(
        !backend.is_loaded(),
        "仅构造 LazyMpvVideoBackend 时不应触发 mpv-2.dll 加载（非视频启动路径零加载）"
    );
}

#[test]
fn loader_safely_loads_real_mpv_dll_from_trusted_absolute_path_when_present() {
    let dll_path = third_party_mpv_dll();
    if !dll_path.is_file() {
        eprintln!(
            "[SKIP] 未找到 {}（可运行 scripts/fetch_libmpv.ps1 获取）",
            dll_path.display()
        );
        return;
    }

    let lib = MpvLibrary::load_from_absolute_path(&dll_path)
        .expect("应通过绝对路径与 SetDefaultDllDirectories 成功加载真实 mpv-2.dll");
    let ver = lib.client_api_version();
    assert!(
        (ver >> 16) >= 2,
        "libmpv ABI 主版本应 >= 2，实际为 {ver:#x}"
    );
    assert!(
        is_mpv_module_loaded_in_process(),
        "加载后进程内应可观测到 mpv-2.dll 模块"
    );
}

fn generate_test_mp4(path: &Path) -> bool {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=320x180:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(path)
        .status();
    matches!(status, Ok(s) if s.success()) && path.is_file()
}

fn generate_test_wav(path: &Path) -> bool {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
        ])
        .arg(path)
        .status();
    matches!(status, Ok(s) if s.success()) && path.is_file()
}

#[test]
fn sandboxed_mpv_instance_enforces_isolation_properties_at_runtime() {
    let dll_path = third_party_mpv_dll();
    if !dll_path.is_file() {
        eprintln!("[SKIP] 未找到 mpv-2.dll，跳过真实沙箱运行时测试");
        return;
    }

    let lib = MpvLibrary::load_from_absolute_path(&dll_path).expect("加载 mpv-2.dll");
    let mpv = MpvInstance::new_sandboxed(lib, &[("vo", "null"), ("ao", "null")])
        .expect("沙箱化 mpv 实例应初始化成功");

    assert_eq!(mpv.get_property_string("config").as_deref(), Some("no"));
    assert_eq!(
        mpv.get_property_string("load-scripts").as_deref(),
        Some("no")
    );
    assert_eq!(mpv.get_property_string("ytdl").as_deref(), Some("no"));
    let lavf_opts = mpv
        .get_property_string("demuxer-lavf-o")
        .unwrap_or_default();
    assert!(
        lavf_opts.contains("protocol_whitelist=file"),
        "运行中 mpv 实例的 demuxer-lavf-o 应包含 protocol_whitelist=file，实际为: {lavf_opts}"
    );

    let err = mpv
        .load_local_file(Path::new("http://127.0.0.1:9/blocked.mp4"))
        .expect_err("应拒绝加载网络 URL");
    assert!(matches!(err, VideoError::UnsafeMediaPath(_)));
}

#[test]
fn lazy_mpv_backend_probes_real_video_and_pure_audio_files() {
    let dll_path = third_party_mpv_dll();
    if !dll_path.is_file() {
        eprintln!("[SKIP] 未找到 mpv-2.dll，跳过真实探测测试");
        return;
    }

    let tmp = tempfile::tempdir().expect("创建临时目录");
    let mp4_path = tmp.path().join("sample.mp4");
    let wav_path = tmp.path().join("tone.wav");
    if !generate_test_mp4(&mp4_path) || !generate_test_wav(&wav_path) {
        eprintln!("[SKIP] 本机未安装 ffmpeg，跳过真实媒体生成与探测测试");
        return;
    }

    let backend = LazyMpvVideoBackend::with_absolute_dll_path(dll_path);
    assert!(!backend.is_loaded(), "探测前尚未触发懒加载");

    let video_info = backend
        .probe(&mp4_path)
        .expect("应对真实 MP4 返回有效 VideoProbeInfo");
    assert!(backend.is_loaded(), "探测后应已触发懒加载");
    assert!(video_info.has_video_stream);
    assert!(video_info.has_audio_stream);
    assert_eq!(video_info.width, 320);
    assert_eq!(video_info.height, 180);
    let dur = video_info.duration_ms.expect("应返回有效时长");
    assert!(
        (700..=1500).contains(&dur),
        "1s 视频探测时长应在 700..=1500ms 范围，实际={dur}ms"
    );

    let audio_info = backend
        .probe(&wav_path)
        .expect("应对真实纯音频 WAV 返回有效 VideoProbeInfo");
    assert!(!audio_info.has_video_stream);
    assert!(audio_info.has_audio_stream);
}

#[test]
fn software_render_fallback_produces_non_empty_rgba_video_frame() {
    let dll_path = third_party_mpv_dll();
    if !dll_path.is_file() {
        eprintln!("[SKIP] 未找到 mpv-2.dll，跳过软件帧回退测试");
        return;
    }

    let tmp = tempfile::tempdir().expect("创建临时目录");
    let mp4_path = tmp.path().join("sw_frame.mp4");
    if !generate_test_mp4(&mp4_path) {
        eprintln!("[SKIP] 本机未安装 ffmpeg，跳过软件帧回退测试");
        return;
    }

    let backend = LazyMpvVideoBackend::with_absolute_dll_path(dll_path);
    let frame = backend
        .render_first_frame_sw(&mp4_path, 320, 180)
        .expect("软件渲染回退应成功输出首帧 RGBA 像素");

    assert_eq!(frame.width, 320);
    assert_eq!(frame.height, 180);
    assert_eq!(frame.rgba.len(), 320 * 180 * 4);
    assert!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0] > 16 || px[1] > 16 || px[2] > 16),
        "解码出的测试视频首帧不应为全黑"
    );
    assert_eq!(
        CompositingStrategy::recommended_for_renderer(anyfile::ActiveRenderer::Software),
        CompositingStrategy::SoftwareFrameUpload
    );
}
