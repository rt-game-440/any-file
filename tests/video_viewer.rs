//! 视频视图与音频 libmpv 回退集成测试。
//! 需要真实 `third_party/libmpv/mpv-2.dll`（脚本 `scripts/fetch_libmpv.ps1` 获取，不入库）与 `ffmpeg`；
//! 缺失时跳过（打印 `[SKIP]`），不使 CI 失败。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyfile::audio_viewer::{AudioPlayer, AudioViewer};
use anyfile::platform::{PlatformSystemShell, SystemClock};
use anyfile::session::AppSession;
use anyfile::video::LazyMpvVideoBackend;
use anyfile::video_viewer::{MpvAudioFallback, VideoPlayer, VideoViewer};

fn dll() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("third_party")
        .join("libmpv")
        .join("mpv-2.dll");
    p.is_file().then_some(p)
}

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

macro_rules! require_env {
    () => {
        match dll() {
            Some(p) if ffmpeg_ok() => p,
            _ => {
                eprintln!("[SKIP] 缺少 third_party/libmpv/mpv-2.dll 或 ffmpeg");
                return;
            }
        }
    };
}

fn backend(dll: PathBuf) -> Arc<LazyMpvVideoBackend> {
    Arc::new(LazyMpvVideoBackend::with_absolute_dll_path(dll))
}

fn ffmpeg(args: &[&str], out: &Path) -> bool {
    Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x180:rate=30"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
        .args(args)
        .arg("-shortest")
        .arg(out)
        .status()
        .is_ok_and(|s| s.success())
        && out.is_file()
}

fn gen_audio(codec_args: &[&str], out: &Path) -> bool {
    Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
        .args(codec_args)
        .arg(out)
        .status()
        .is_ok_and(|s| s.success())
        && out.is_file()
}

fn pump_until(player: &mut VideoPlayer, timeout: Duration, mut f: impl FnMut(&mut VideoPlayer) -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f(player) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    false
}

fn frame_brightness(rgba: &[u8]) -> u64 {
    rgba.as_chunks::<4>().0.iter().map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2])).sum()
}

/// 每种容器/编码：载入、渲染出非黑首帧、位置推进。
#[test]
fn formats_play_and_render_nonblack_frames() {
    let dll = require_env!();
    let b = backend(dll);
    let dir = tempfile::tempdir().unwrap();
    let cases: &[(&str, &[&str])] = &[
        ("a.mp4", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
        ("a.mkv", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
        ("a.webm", &["-c:v", "libvpx", "-b:v", "500k", "-c:a", "libvorbis"]),
        ("a.mov", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
        ("a.avi", &["-c:v", "mpeg4", "-c:a", "mp3"]),
        ("a.wmv", &["-c:v", "wmv2", "-c:a", "wmav2"]),
        ("a.flv", &["-c:v", "flv", "-c:a", "mp3", "-ar", "44100"]),
    ];
    let mut tested = 0;
    for (name, args) in cases {
        let path = dir.path().join(name);
        if !ffmpeg(args, &path) {
            eprintln!("[SKIP] ffmpeg 无法生成 {name}（缺少编码器）");
            continue;
        }
        let mut p = VideoPlayer::new(b.library().unwrap()).unwrap();
        p.load(&path, true).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            pump_until(&mut p, Duration::from_secs(5), |p| p.snapshot().position_secs > 0.2),
            "{name} 位置应推进"
        );
        assert!(pump_until(&mut p, Duration::from_secs(3), |p| p.has_new_frame()), "{name} 应有新帧");
        let f = p.render_frame(320, 180).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((f.width, f.height), (320, 180));
        assert!(frame_brightness(&f.rgba) > 320 * 180 * 20, "{name} 帧不应为黑");
        assert!(f.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
        tested += 1;
    }
    assert!(tested >= 3, "至少应覆盖 3 种容器，实际 {tested}");
}

#[test]
fn transport_controls_seek_speed_volume_mute_and_end() {
    let dll = require_env!();
    let b = backend(dll);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.mp4");
    assert!(ffmpeg(&["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"], &path));
    let mut p = VideoPlayer::new(b.library().unwrap()).unwrap();
    p.load(&path, false).unwrap();
    let t = p.transport();
    assert!(t.snapshot().paused, "autoplay=false 应保持暂停");
    assert!((t.snapshot().duration_secs - 2.0).abs() < 0.3);

    t.seek_to(1.0);
    std::thread::sleep(Duration::from_millis(200));
    assert!((t.snapshot().position_secs - 1.0).abs() < 0.3, "{:?}", t.snapshot());
    t.seek_by(-0.5);
    std::thread::sleep(Duration::from_millis(200));
    assert!((t.snapshot().position_secs - 0.5).abs() < 0.3, "{:?}", t.snapshot());

    t.set_speed(2.0);
    assert!((t.snapshot().speed - 2.0).abs() < 1e-3);
    t.set_volume(0.25);
    assert!((t.snapshot().volume - 0.25).abs() < 0.01);
    t.toggle_mute();
    assert!(t.snapshot().muted);
    t.toggle_mute();
    assert!(!t.snapshot().muted);

    t.set_speed(4.0);
    t.seek_to(1.5);
    t.set_paused(false);
    let p_ref = &mut p;
    assert!(
        pump_until(p_ref, Duration::from_secs(5), |p| p.snapshot().ended),
        "应播放到结尾并停住"
    );
    let s = p.snapshot();
    assert!(s.ended && (s.position_secs - s.duration_secs).abs() < 0.2);
    p.transport().toggle_play();
    assert!(!p.snapshot().ended, "播完后再点播放应从头开始");
}

#[test]
fn external_and_embedded_subtitles_can_be_toggled() {
    let dll = require_env!();
    let b = backend(dll);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.mp4");
    assert!(ffmpeg(&["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"], &path));
    std::fs::write(
        dir.path().join("movie.srt"),
        "1\n00:00:00,000 --> 00:00:02,000\nHELLO SUBTITLE\n\n",
    )
    .unwrap();
    let mut v = VideoViewer::new(b);
    v.set_autoplay_on_load(false);
    let session = session_with_video(&path, &dll_backend());
    v.sync_with_session(&session);
    let st = v.view_state(false);
    assert!(st.error.is_none(), "{:?}", st.error);
    assert!(st.loaded && st.has_subtitles, "{st:?}");
    {
        let p = v.player().unwrap();
        let tracks = p.subtitle_tracks();
        assert!(tracks.iter().any(|t| t.external && t.selected), "{tracks:?}");
    }
    // 字幕开启 vs 关闭渲染结果应不同
    v.player().unwrap().transport().seek_to(0.5);
    std::thread::sleep(Duration::from_millis(300));
    let render = |v: &mut VideoViewer| {
        let p = v.player_mut().unwrap();
        let _ = p.has_new_frame();
        p.render_frame(640, 360).unwrap().rgba
    };
    let with_sub = render(&mut v);
    v.player_mut().unwrap().set_subtitles_visible(false);
    assert!(!v.view_state(false).subtitles_visible);
    std::thread::sleep(Duration::from_millis(200));
    let without = render(&mut v);
    assert_ne!(with_sub, without, "字幕开关应改变画面");

    // 内嵌字幕轨（mkv + srt 流）可被识别
    let mkv = dir.path().join("emb.mkv");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(&path)
        .arg("-i")
        .arg(dir.path().join("movie.srt"))
        .args(["-c", "copy", "-c:s", "srt"])
        .arg(&mkv)
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        let mut p = VideoPlayer::new(dll_backend().library().unwrap()).unwrap();
        p.load(&mkv, false).unwrap();
        assert!(p.subtitle_tracks().iter().any(|t| !t.external), "应识别内嵌字幕轨");
    }
}

fn dll_backend() -> Arc<LazyMpvVideoBackend> {
    backend(dll().expect("dll"))
}

fn session_with_video(path: &Path, b: &Arc<LazyMpvVideoBackend>) -> AppSession {
    let s = AppSession::new(Arc::new(PlatformSystemShell), Arc::new(SystemClock), b.clone());
    s.open_file(path).unwrap();
    s
}

#[test]
fn viewer_follows_session_and_stops_when_leaving() {
    let dll = require_env!();
    let b = backend(dll);
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.mp4");
    let c = dir.path().join("c.mp4");
    assert!(ffmpeg(&["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"], &a));
    std::fs::copy(&a, &c).unwrap();
    let txt = dir.path().join("z.txt");
    std::fs::write(&txt, "hi").unwrap();

    let session = session_with_video(&a, &b);
    assert_eq!(session.active_viewer(), anyfile::domain::ViewerKind::Video);
    let mut v = VideoViewer::new(b);
    v.set_autoplay_on_load(false);
    assert!(v.sync_with_session(&session));
    assert_eq!(v.current_path(), Some(a.as_path()));
    assert!(!v.sync_with_session(&session), "同一文件不重复载入");

    // Ctrl+Right / PageDown 切下一个视频
    use anyfile::audio_viewer::KeyModifiers;
    use anyfile::video_viewer::VideoKey;
    assert!(v.handle_key(VideoKey::PageDown, KeyModifiers { ctrl: false }, &session));
    assert_eq!(v.current_path(), Some(c.as_path()));
    assert!(v.handle_key(VideoKey::Left, KeyModifiers { ctrl: true }, &session));
    assert_eq!(v.current_path(), Some(a.as_path()));

    // 全屏 / 字幕键
    assert!(v.handle_key(VideoKey::F, KeyModifiers { ctrl: false }, &session));
    assert!(v.fullscreen());
    assert!(v.handle_key(VideoKey::Escape, KeyModifiers { ctrl: false }, &session));
    assert!(!v.fullscreen());

    // 切到文本：停止播放并释放
    session.open_file(&txt).unwrap();
    assert!(v.sync_with_session(&session));
    assert!(v.player().is_none() && v.current_path().is_none());
}

#[test]
fn viewer_stays_idle_when_backend_unavailable() {
    let b = Arc::new(LazyMpvVideoBackend::with_absolute_dll_path(PathBuf::from(
        r"C:\definitely\missing\mpv-2.dll",
    )));
    let mut v = VideoViewer::new(b);
    // 直接走载入路径：没有会话文件也应安全
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x.mp4");
    std::fs::write(&f, b"x").unwrap();
    let s = AppSession::new(
        Arc::new(PlatformSystemShell),
        Arc::new(SystemClock),
        Arc::new(anyfile::platform::UnavailableVideoBackend),
    );
    s.open_file(&f).unwrap();
    // 视频不可用时会话不会选择视频视图，控制器应保持空闲
    assert!(!v.sync_with_session(&s));
    assert!(v.player().is_none());
}

/// 纯 Rust 解码不支持的 Opus 自动回退到 libmpv（HE-AAC 走同一路径；本机 ffmpeg 无 HE-AAC 编码器故无法生成样例）。
#[test]
fn audio_viewer_falls_back_to_libmpv_for_opus() {
    let dll = require_env!();
    let b = backend(dll);
    let dir = tempfile::tempdir().unwrap();
    let opus = dir.path().join("a.opus");
    let ogg_opus = dir.path().join("b.ogg");
    let wav = dir.path().join("c.wav");
    let mut files = Vec::new();
    if gen_audio(&["-c:a", "libopus"], &opus) {
        files.push(opus);
    }
    if gen_audio(&["-c:a", "libopus", "-f", "ogg"], &ogg_opus) {
        files.push(ogg_opus);
    }
    assert!(gen_audio(&["-c:a", "pcm_s16le"], &wav));
    assert!(!files.is_empty(), "ffmpeg 应能生成 opus 样例");

    for f in files {
        let session = AppSession::new(Arc::new(PlatformSystemShell), Arc::new(SystemClock), b.clone());
        session.open_file(&f).unwrap();
        assert_eq!(session.active_viewer(), anyfile::domain::ViewerKind::Audio, "{f:?}");
        let mut av = AudioViewer::new(AudioPlayer::headless(48_000, 2));
        av.set_fallback(Box::new(MpvAudioFallback::new(b.clone()).with_null_output()));
        assert!(av.sync_with_session(&session));
        assert!(av.using_fallback(), "{f:?} 应回退到 libmpv");
        let st = av.view_state();
        assert!(st.error.is_none(), "{:?}", st.error);
        assert!(st.snapshot.loaded && (st.snapshot.duration_secs - 2.0).abs() < 0.3, "{st:?}");
        // 命令经回退引擎生效
        av.handle_command(anyfile::audio_viewer::AudioCommand::SeekTo(1.0), &session);
        std::thread::sleep(Duration::from_millis(200));
        assert!((av.view_state().snapshot.position_secs - 1.0).abs() < 0.4);
        av.handle_command(anyfile::audio_viewer::AudioCommand::ToggleMute, &session);
        assert!(av.view_state().snapshot.muted);
        av.stop();
        assert!(!av.using_fallback());
    }

    // 纯 Rust 能解的格式不应走回退
    let session = AppSession::new(Arc::new(PlatformSystemShell), Arc::new(SystemClock), b.clone());
    session.open_file(&wav).unwrap();
    let mut av = AudioViewer::new(AudioPlayer::headless(48_000, 2));
    av.set_fallback(Box::new(MpvAudioFallback::new(b).with_null_output()));
    av.sync_with_session(&session);
    assert!(!av.using_fallback());
    assert!(av.view_state().error.is_none());
}

#[test]
fn audio_without_fallback_still_reports_unsupported() {
    if !ffmpeg_ok() {
        eprintln!("[SKIP] 缺少 ffmpeg");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let opus = dir.path().join("a.opus");
    if !gen_audio(&["-c:a", "libopus"], &opus) {
        eprintln!("[SKIP] 无 libopus 编码器");
        return;
    }
    let session = AppSession::new(
        Arc::new(PlatformSystemShell),
        Arc::new(SystemClock),
        Arc::new(anyfile::platform::UnavailableVideoBackend),
    );
    session.open_file(&opus).unwrap();
    let mut av = AudioViewer::new(AudioPlayer::headless(48_000, 2));
    if session.active_viewer() == anyfile::domain::ViewerKind::Audio {
        av.sync_with_session(&session);
        assert!(av.view_state().error.is_some(), "无回退时应给出错误而非崩溃");
        assert!(!av.using_fallback());
    }
}
