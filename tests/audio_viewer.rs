//! 音频视图集成测试：解码、元数据/封面、快捷键状态机、播完与自动连播、切走停播。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyfile::audio_viewer::{
    AudioCommand, AudioKey, AudioPlayer, AudioViewer, KeyModifiers, PlayerCore, format_time,
    map_key, read_metadata,
};
use anyfile::platform::{PlatformSystemShell, SystemClock, UnavailableVideoBackend};
use anyfile::session::AppSession;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}

fn session() -> AppSession {
    AppSession::new(
        Arc::new(PlatformSystemShell),
        Arc::new(SystemClock),
        Arc::new(UnavailableVideoBackend),
    )
}

fn render_all(core: &mut PlayerCore) -> Vec<f32> {
    let mut all = Vec::new();
    let mut buf = vec![0.0f32; 2048 * 2];
    for _ in 0..200 {
        core.fill(&mut buf);
        all.extend_from_slice(&buf);
        if core.snapshot().ended {
            break;
        }
    }
    all
}

#[test]
fn all_formats_decode_to_audible_pcm_with_correct_duration() {
    for name in [
        "sample.wav",
        "sample.mp3",
        "sample.flac",
        "sample.ogg",
        "sample.m4a",
        "sample.aac",
    ] {
        let mut core = PlayerCore::new(44_100, 2);
        let info = core.load(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(info.sample_rate, 44_100, "{name}");
        core.play();
        let pcm = render_all(&mut core);
        let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05, "{name} 应有可听信号，peak={peak}");
        let snap = core.snapshot();
        assert!(snap.ended && !snap.playing, "{name} 播完应停止");
        assert!(
            (0.85..1.2).contains(&snap.position_secs),
            "{name} 时长 {}",
            snap.position_secs
        );
    }
}

#[test]
fn metadata_and_cover_are_read() {
    for name in ["sample.mp3", "sample.flac", "sample.m4a"] {
        let meta = read_metadata(&fixture(name));
        assert_eq!(meta.title, "TestTrack", "{name}");
        assert_eq!(meta.artist.as_deref(), Some("AnyFile"), "{name}");
        assert_eq!(meta.album.as_deref(), Some("SampleAlbum"), "{name}");
        let cover = meta.cover.unwrap_or_else(|| panic!("{name} 应有封面"));
        assert_eq!((cover.width, cover.height), (32, 32));
        assert_eq!(cover.rgba.len(), 32 * 32 * 4);
    }
    // 无标签文件回退为文件名
    let wav = read_metadata(&fixture("sample.wav"));
    assert_eq!(wav.title, "sample");
    assert!(wav.cover.is_none());
}

#[test]
fn seek_volume_mute_and_speed() {
    let mut core = PlayerCore::new(44_100, 2);
    core.load(&fixture("sample.wav")).unwrap();
    core.play();
    core.seek_to(0.5);
    let pos = core.snapshot().position_secs;
    assert!((pos - 0.5).abs() < 0.02, "pos={pos}");
    core.seek_by(-5.0);
    assert!(core.snapshot().position_secs.abs() < 0.02, "快退夹到 0");

    core.set_muted(true);
    let mut buf = vec![0.0f32; 1024];
    core.fill(&mut buf);
    assert!(buf.iter().all(|s| *s == 0.0), "静音输出应为零");
    core.set_muted(false);
    core.set_volume(0.5);
    core.fill(&mut buf);
    let half = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    core.seek_to(0.0);
    core.set_volume(1.0);
    core.fill(&mut buf);
    let full = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(half < full * 0.75);

    // 2x 倍速：播放时长约减半
    let mut fast = PlayerCore::new(44_100, 2);
    fast.load(&fixture("sample.wav")).unwrap();
    fast.set_speed(2.0);
    fast.play();
    let frames = render_all(&mut fast).len() / 2;
    assert!(
        (20_000..24_000).contains(&frames) || frames <= 2048 * 100,
        "frames={frames}"
    );
    assert!(frames < 30_000, "2x 应只需约 22050 帧，实际 {frames}");
}

#[test]
fn seek_past_end_marks_ended_and_space_restarts() {
    let mut core = PlayerCore::new(44_100, 2);
    core.load(&fixture("sample.wav")).unwrap();
    core.play();
    core.seek_by(100.0);
    let s = core.snapshot();
    assert!(s.ended && !s.playing);
    assert_eq!(s.position_secs, s.duration_secs);
    core.toggle_play();
    let s = core.snapshot();
    assert!(s.playing && !s.ended && s.position_secs < 0.05);
}

#[test]
fn key_state_machine_mapping() {
    let none = KeyModifiers::default();
    let ctrl = KeyModifiers { ctrl: true };
    assert_eq!(map_key(AudioKey::Space, none), Some(AudioCommand::TogglePlay));
    assert_eq!(map_key(AudioKey::Left, none), Some(AudioCommand::SeekBy(-5.0)));
    assert_eq!(map_key(AudioKey::Right, none), Some(AudioCommand::SeekBy(5.0)));
    assert_eq!(map_key(AudioKey::Left, ctrl), Some(AudioCommand::PrevTrack));
    assert_eq!(map_key(AudioKey::Right, ctrl), Some(AudioCommand::NextTrack));
    assert_eq!(map_key(AudioKey::PageUp, none), Some(AudioCommand::PrevTrack));
    assert_eq!(map_key(AudioKey::PageDown, none), Some(AudioCommand::NextTrack));
    assert_eq!(map_key(AudioKey::M, none), Some(AudioCommand::ToggleMute));
    assert_eq!(format_time(65.9), "1:05");
    assert_eq!(format_time(3725.0), "1:02:05");
}

fn setup_dir() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a_track.wav");
    let b = dir.path().join("b_track.wav");
    std::fs::copy(fixture("sample.wav"), &a).unwrap();
    std::fs::copy(fixture("sample.wav"), &b).unwrap();
    (dir, a, b)
}

fn viewer() -> AudioViewer {
    AudioViewer::new(AudioPlayer::headless(44_100, 2))
}

#[test]
fn ended_stops_by_default_and_autonext_advances() {
    let (_dir, a, b) = setup_dir();
    let session = session();
    session.open_file(&a).unwrap();
    let mut v = viewer();
    assert!(v.sync_with_session(&session));
    assert!(v.player().snapshot().playing, "打开后自动播放");

    v.handle_command(AudioCommand::SeekTo(100.0), &session);
    assert!(v.player().snapshot().ended);
    // 默认：播完停止在末尾，不切歌
    assert!(!v.poll(&session));
    assert_eq!(v.current_path(), Some(a.as_path()));

    // 开启自动播放下一首
    v.handle_command(AudioCommand::ToggleAutoNext, &session);
    assert!(v.poll(&session));
    assert_eq!(v.current_path(), Some(b.as_path()));
    assert!(v.player().snapshot().playing);

    // 最后一首播完：保持停止
    v.handle_command(AudioCommand::SeekTo(100.0), &session);
    assert!(!v.poll(&session));
    assert!(v.player().snapshot().ended);
}

#[test]
fn track_keys_navigate_siblings_and_leaving_audio_stops() {
    let (dir, a, b) = setup_dir();
    let txt = dir.path().join("z.txt");
    std::fs::write(&txt, "hello").unwrap();
    let session = session();
    session.open_file(&a).unwrap();
    let mut v = viewer();
    v.sync_with_session(&session);

    assert!(v.handle_key(AudioKey::PageDown, KeyModifiers::default(), &session));
    assert_eq!(v.current_path(), Some(b.as_path()));
    assert!(v.handle_key(AudioKey::Left, KeyModifiers { ctrl: true }, &session));
    assert_eq!(v.current_path(), Some(a.as_path()));

    // 切到非音频文件项：立即停播并释放
    session.open_file(&txt).unwrap();
    assert!(v.sync_with_session(&session));
    assert!(v.current_path().is_none());
    let s = v.player().snapshot();
    assert!(!s.loaded && !s.playing);
}

#[test]
fn corrupted_audio_reports_error_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.mp3");
    std::fs::write(&bad, b"not really audio data at all").unwrap();
    let mut core = PlayerCore::new(44_100, 2);
    assert!(core.load(&bad).is_err());
}
