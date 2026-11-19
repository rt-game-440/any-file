use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyfile::domain::{FallbackReason, MediaCategory, ResolvedTheme, ViewerKind};
use anyfile::fixtures::{FixtureCorpus, minimal_mp4_ftyp_bytes, minimal_valid_png_bytes};
use anyfile::ports::{Clock, SystemShell, VideoBackend, VideoProbeInfo};
use anyfile::session::AppSession;

struct TestShell {
    opened_with: Mutex<Vec<PathBuf>>,
}

impl TestShell {
    fn new() -> Self {
        Self {
            opened_with: Mutex::new(Vec::new()),
        }
    }
}

impl SystemShell for TestShell {
    fn system_theme(&self) -> ResolvedTheme {
        ResolvedTheme::Light
    }

    fn prefers_reduced_motion(&self) -> bool {
        false
    }

    fn open_with_dialog(&self, path: &Path) -> Result<(), String> {
        self.opened_with.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct StubVideoBackend {
    probe_result: Mutex<Option<VideoProbeInfo>>,
}

impl StubVideoBackend {
    fn unavailable() -> Self {
        Self {
            probe_result: Mutex::new(None),
        }
    }

    fn with_probe(info: VideoProbeInfo) -> Self {
        Self {
            probe_result: Mutex::new(Some(info)),
        }
    }
}

impl VideoBackend for StubVideoBackend {
    fn is_available(&self) -> bool {
        self.probe_result.lock().unwrap().is_some()
    }

    fn probe(&self, _path: &Path) -> Option<VideoProbeInfo> {
        self.probe_result.lock().unwrap().clone()
    }
}

fn new_session() -> AppSession {
    AppSession::new(
        Arc::new(TestShell::new()),
        Arc::new(TestClock),
        Arc::new(StubVideoBackend::unavailable()),
    )
}

#[test]
fn probes_magic_bytes_misnamed_no_ext_ambiguous_corrupted_and_binary() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let corpus = FixtureCorpus::generate_in(dir.path()).expect("生成语料库失败");
    let session = new_session();

    // 1. 正常图像（PNG / BMP）
    let entry = session.open_file(&corpus.png_valid).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Image));
    assert_eq!(session.active_viewer(), ViewerKind::Image);
    assert!(session.fallback_info().is_none());

    let entry = session.open_file(&corpus.bmp_valid).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Image));
    assert_eq!(session.active_viewer(), ViewerKind::Image);

    // 2. 改错后缀的媒体文件：实际为 PNG 但命名为 .txt -> 必须识别为 Image
    let entry = session.open_file(&corpus.misnamed_png_as_txt).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Image));
    assert_eq!(session.active_viewer(), ViewerKind::Image);

    // 3. 无扩展名文本：Makefile 与 LICENSE
    let entry = session.open_file(&corpus.text_no_extension_makefile).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Text));
    assert_eq!(session.active_viewer(), ViewerKind::Text);

    let license_path = dir.path().join("LICENSE");
    fs::write(&license_path, "GNU GENERAL PUBLIC LICENSE\nVersion 3\n").unwrap();
    let entry = session.open_file(&license_path).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Text));
    assert_eq!(session.active_viewer(), ViewerKind::Text);

    // 4. 多编码文本：UTF-8 BOM / GBK / UTF-16 LE
    for text_path in [
        &corpus.text_utf8_lf,
        &corpus.text_utf8_bom_crlf,
        &corpus.text_gbk,
        &corpus.text_utf16_le,
    ] {
        let entry = session.open_file(text_path).unwrap();
        assert_eq!(
            entry.media_category,
            Some(MediaCategory::Text),
            "文本文件应识别为 Text: {}",
            text_path.display()
        );
        assert_eq!(session.active_viewer(), ViewerKind::Text);
    }

    // 5. 歧义格式 .svg -> 默认 Image，且支持切换查看器到 Text
    let entry = session.open_file(&corpus.svg_ambiguous).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Image));
    assert_eq!(session.active_viewer(), ViewerKind::Image);

    // 6. 歧义格式 .ts：TypeScript 源码 -> Text；MPEG-TS 流 -> Video
    let entry = session.open_file(&corpus.ts_ambiguous).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Text));
    assert_eq!(session.active_viewer(), ViewerKind::Text);

    let mpeg_ts_path = dir.path().join("broadcast.ts");
    let mut mpeg_ts_bytes = vec![0u8; 188 * 4];
    for pkt in 0..4 {
        mpeg_ts_bytes[pkt * 188] = 0x47; // MPEG-TS sync byte
        mpeg_ts_bytes[pkt * 188 + 1] = 0x40;
        mpeg_ts_bytes[pkt * 188 + 2] = 0x00;
        mpeg_ts_bytes[pkt * 188 + 3] = 0x10;
    }
    fs::write(&mpeg_ts_path, mpeg_ts_bytes).unwrap();
    let entry = session.open_file(&mpeg_ts_path).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Video));
    assert_eq!(session.active_viewer(), ViewerKind::Video);

    // 7. 音频（WAV）与视频（MP4）
    let entry = session.open_file(&corpus.wav_valid).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Audio));
    assert_eq!(session.active_viewer(), ViewerKind::Audio);

    let entry = session.open_file(&corpus.mp4_minimal).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Video));
    assert_eq!(session.active_viewer(), ViewerKind::Video);

    // 8. 损坏 PNG -> Unsupported 兜底信息卡，且原因是 CorruptedFile
    let entry = session.open_file(&corpus.corrupted_png).unwrap();
    assert_eq!(entry.media_category, None);
    assert_eq!(session.active_viewer(), ViewerKind::Unsupported);
    let fallback = session.fallback_info().expect("损坏文件应生成兜底信息卡");
    assert_eq!(fallback.path, corpus.corrupted_png);
    assert!(fallback.file_size > 0);
    assert!(matches!(fallback.reason, FallbackReason::CorruptedFile(_)));
    assert!(!fallback.reason_message.is_empty());

    // 9. 纯二进制未知文件 -> Unsupported 兜底信息卡，且原因是 PureBinary
    let entry = session.open_file(&corpus.binary_unknown).unwrap();
    assert_eq!(entry.media_category, None);
    assert_eq!(session.active_viewer(), ViewerKind::Unsupported);
    let fallback = session.fallback_info().expect("纯二进制文件应生成兜底信息卡");
    assert_eq!(fallback.path, corpus.binary_unknown);
    assert_eq!(fallback.file_size, 10);
    assert_eq!(fallback.reason, FallbackReason::PureBinary);
}

#[test]
fn audio_only_mp4_container_uses_video_backend_stream_probe() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let mp4_path = dir.path().join("podcast_track.mp4");
    fs::write(&mp4_path, minimal_mp4_ftyp_bytes()).unwrap();

    let session = AppSession::new(
        Arc::new(TestShell::new()),
        Arc::new(TestClock),
        Arc::new(StubVideoBackend::with_probe(VideoProbeInfo {
            has_video_stream: false,
            has_audio_stream: true,
            width: 0,
            height: 0,
            duration_ms: Some(120_000),
        })),
    );

    let entry = session.open_file(&mp4_path).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Audio));
    assert_eq!(session.active_viewer(), ViewerKind::Audio);
}

#[test]
fn supports_unicode_and_long_paths_over_260_chars() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let session = new_session();

    // Unicode 路径（中文 + 日文 + Emoji）
    let unicode_dir = dir.path().join("测试目录_日本語_🎵");
    fs::create_dir_all(&unicode_dir).unwrap();
    let unicode_file = unicode_dir.join("示例图片_テスト.png");
    fs::write(&unicode_file, minimal_valid_png_bytes()).unwrap();

    let entry = session.open_file(&unicode_file).unwrap();
    assert_eq!(entry.media_category, Some(MediaCategory::Image));
    assert_eq!(session.active_viewer(), ViewerKind::Image);

    // 超长路径（> 260 字符）
    let mut nested = dir.path().to_path_buf();
    for idx in 0..8 {
        nested.push(format!("very_long_directory_segment_name_{idx:02}_abcdefghij"));
    }
    let long_file = nested.join("target_note_with_long_name.txt");
    assert!(
        long_file.as_os_str().len() > 260,
        "测试路径长度应超过 260 字符，实际为 {}",
        long_file.as_os_str().len()
    );

    // 使用扩展路径创建目录与文件，确保即使系统未全局开启 LongPathsEnabled 也能稳定创建
    #[cfg(windows)]
    let io_dir = {
        let s = nested.to_string_lossy();
        if s.starts_with(r"\\?\") {
            nested.clone()
        } else {
            PathBuf::from(format!(r"\\?\{s}"))
        }
    };
    #[cfg(not(windows))]
    let io_dir = nested.clone();

    fs::create_dir_all(&io_dir).expect("创建超长嵌套目录失败");
    fs::write(io_dir.join("target_note_with_long_name.txt"), "长路径文本内容\n")
        .expect("写入超长路径文件失败");

    let entry = session.open_file(&long_file).expect("AppSession 应能打开超长路径文件");
    assert_eq!(entry.media_category, Some(MediaCategory::Text));
    assert_eq!(session.active_viewer(), ViewerKind::Text);
}

#[test]
fn sibling_sequence_uses_natural_sort_extension_only_and_supports_category_toggle_and_navigation() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let png = minimal_valid_png_bytes();

    // 刻意乱序创建，验证自然排序：1.png < 2.png < 10.png < a2.png < a10.png
    fs::write(dir.path().join("10.png"), &png).unwrap();
    fs::write(dir.path().join("2.png"), b"\x00\x01NOT_A_REAL_PNG").unwrap(); // 仅扩展名为 .png，内容损坏
    fs::write(dir.path().join("1.png"), &png).unwrap();
    fs::write(dir.path().join("a10.png"), &png).unwrap();
    fs::write(dir.path().join("a2.png"), &png).unwrap();
    fs::write(dir.path().join("readme.txt"), "hello").unwrap();
    fs::write(dir.path().join("opaque.bin"), [0x00, 0xFF, 0x01, 0x02]).unwrap();

    let session = new_session();
    session.open_file(&dir.path().join("1.png")).unwrap();

    let seq = session.sibling_sequence();
    assert!(seq.filter_by_category);
    let names: Vec<String> = seq
        .entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    // 2.png 虽然内容损坏，但第一层仅按扩展名索引（绝不读相邻文件内容），因此应被纳入 Image 序列
    assert_eq!(names, vec!["1.png", "2.png", "10.png", "a2.png", "a10.png"]);
    assert_eq!(seq.current_index, Some(0));

    // 在首项向前导航不应越界
    assert!(session.navigate_prev().unwrap().is_none());

    // 向后导航到 2.png：此时才对 2.png 执行第二层懒探测，识别为损坏文件 -> Unsupported 兜底卡
    let second = session.navigate_next().unwrap().expect("应切换到 2.png");
    assert_eq!(second.path.file_name().unwrap(), "2.png");
    assert_eq!(session.active_viewer(), ViewerKind::Unsupported);
    assert!(session.fallback_info().is_some());
    assert_eq!(session.sibling_sequence().current_index, Some(1));

    // 继续向后导航到 10.png -> 恢复 Image 视图
    let third = session.navigate_next().unwrap().expect("应切换到 10.png");
    assert_eq!(third.path.file_name().unwrap(), "10.png");
    assert_eq!(session.active_viewer(), ViewerKind::Image);
    assert!(session.fallback_info().is_none());

    // 切换为“全部支持格式”：应包含 readme.txt，不包含未知扩展名 opaque.bin，且当前索引仍指向 10.png
    session.set_sibling_filter_by_category(false);
    let all_seq = session.sibling_sequence();
    assert!(!all_seq.filter_by_category);
    let all_names: Vec<String> = all_seq
        .entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        all_names,
        vec!["1.png", "2.png", "10.png", "a2.png", "a10.png", "readme.txt"]
    );
    let cur_idx = all_seq.current_index.expect("切换筛选后应保持当前索引");
    assert_eq!(all_seq.entries[cur_idx].path.file_name().unwrap(), "10.png");

    // 切回仅同类筛选
    session.toggle_sibling_filter();
    assert!(session.sibling_sequence().filter_by_category);
    assert_eq!(session.sibling_sequence().entries.len(), 5);
}

#[test]
fn sibling_sequence_auto_updates_when_directory_changes_externally() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let png = minimal_valid_png_bytes();
    fs::write(dir.path().join("1.png"), &png).unwrap();
    fs::write(dir.path().join("2.png"), &png).unwrap();
    fs::write(dir.path().join("10.png"), &png).unwrap();

    let session = new_session();
    session.open_file(&dir.path().join("2.png")).unwrap();
    assert_eq!(session.sibling_sequence().entries.len(), 3);
    assert_eq!(session.sibling_sequence().current_index, Some(1));

    // 外部执行新增、删除与重命名
    fs::write(dir.path().join("3.png"), &png).unwrap();
    fs::remove_file(dir.path().join("10.png")).unwrap();
    fs::rename(dir.path().join("1.png"), dir.path().join("0.png")).unwrap();

    // 等待 OS 文件监听自动刷新（不调用任何手动刷新方法）
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let expected = vec!["0.png", "2.png", "3.png"];
    let mut observed = Vec::new();
    while Instant::now() < deadline {
        let seq = session.sibling_sequence();
        observed = seq
            .entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        if observed == expected && seq.current_index == Some(1) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }

    assert_eq!(observed, expected, "目录外部增删改后同目录序列应自动更新");
    assert_eq!(session.sibling_sequence().current_index, Some(1));
}

#[test]
fn manual_viewer_switching_open_with_other_app_and_drop_file_replace_view() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let corpus = FixtureCorpus::generate_in(dir.path()).expect("生成语料库失败");

    let shell = Arc::new(TestShell::new());
    let session = AppSession::new(
        shell.clone(),
        Arc::new(TestClock),
        Arc::new(StubVideoBackend::unavailable()),
    );

    // 空态下不能切换查看器或用其他应用打开
    assert!(session.open_as_text().is_err());
    assert!(session.open_with_other_app().is_err());

    // 1. SVG 歧义格式：默认 Image，可手动“以文本打开”再切回 Image
    session.open_file(&corpus.svg_ambiguous).unwrap();
    assert_eq!(session.active_viewer(), ViewerKind::Image);
    assert_eq!(
        session.available_viewers(),
        vec![ViewerKind::Image, ViewerKind::Text]
    );

    session.open_as_text().expect("SVG 应支持手动以文本打开");
    assert_eq!(session.active_viewer(), ViewerKind::Text);

    session
        .switch_viewer(ViewerKind::Image)
        .expect("SVG 应支持切回图像视图");
    assert_eq!(session.active_viewer(), ViewerKind::Image);

    // 2. 纯二进制兜底卡：可点击“用其他应用打开”调用 SystemShell::open_with_dialog
    session.open_file(&corpus.binary_unknown).unwrap();
    assert_eq!(session.active_viewer(), ViewerKind::Unsupported);
    session
        .open_with_other_app()
        .expect("兜底信息卡应能调用系统打开方式");
    assert_eq!(
        shell.opened_with.lock().unwrap().as_slice(),
        std::slice::from_ref(&corpus.binary_unknown)
    );

    // 3. 拖入新文件替换当前视图并清除旧兜底状态
    let replaced = session
        .drop_file(&corpus.ts_ambiguous)
        .expect("拖入文件应替换当前视图");
    assert_eq!(replaced.path, corpus.ts_ambiguous);
    assert_eq!(session.active_viewer(), ViewerKind::Text);
    assert!(session.fallback_info().is_none());
    assert_eq!(
        session.available_viewers(),
        vec![ViewerKind::Text, ViewerKind::Video]
    );
}
