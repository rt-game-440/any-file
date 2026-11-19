use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyfile::domain::{
    CloseDecision, MediaCategory, ResolvedTheme, ThemePreference, ViewerKind,
};
use anyfile::ports::{Clock, SystemShell, VideoBackend, VideoProbeInfo};
use anyfile::session::AppSession;

/// 可控系统壳层测试替身（仅替换 OS 边界）
struct FakeSystemShell {
    theme: Mutex<ResolvedTheme>,
    reduced_motion: Mutex<bool>,
}

impl FakeSystemShell {
    fn new(theme: ResolvedTheme, reduced_motion: bool) -> Self {
        Self {
            theme: Mutex::new(theme),
            reduced_motion: Mutex::new(reduced_motion),
        }
    }

    fn set_theme(&self, theme: ResolvedTheme) {
        *self.theme.lock().unwrap() = theme;
    }

    fn set_reduced_motion(&self, reduced_motion: bool) {
        *self.reduced_motion.lock().unwrap() = reduced_motion;
    }
}

impl SystemShell for FakeSystemShell {
    fn system_theme(&self) -> ResolvedTheme {
        *self.theme.lock().unwrap()
    }

    fn prefers_reduced_motion(&self) -> bool {
        *self.reduced_motion.lock().unwrap()
    }
}

/// 可控单调时钟测试替身
struct FakeClock {
    now: Mutex<Instant>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
        }
    }

    fn advance(&self, delta: Duration) {
        *self.now.lock().unwrap() += delta;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
}

/// 视频解码后端占位替身
struct FakeVideoBackend;

impl VideoBackend for FakeVideoBackend {
    fn is_available(&self) -> bool {
        false
    }

    fn probe(&self, _path: &std::path::Path) -> Option<VideoProbeInfo> {
        None
    }
}

#[test]
fn empty_launch_starts_in_welcome_viewer_and_follows_system_theme() {
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Dark, false));
    let clock = Arc::new(FakeClock::new());
    let video = Arc::new(FakeVideoBackend);

    let session = AppSession::new(shell, clock, video);

    assert_eq!(session.active_viewer(), ViewerKind::Welcome);
    assert!(session.current_file_entry().is_none());
    assert_eq!(session.resolved_theme(), ResolvedTheme::Dark);
    assert!(!session.motion_profile().reduced_motion);
    assert!(session.motion_profile().stiffness > 0.0);
    assert!(session.motion_profile().damping > 0.0);
}

#[test]
fn reduced_motion_system_setting_degrades_motion_profile_to_instant() {
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Light, true));
    let clock = Arc::new(FakeClock::new());
    let video = Arc::new(FakeVideoBackend);

    let session = AppSession::new(shell.clone(), clock, video);
    assert!(session.motion_profile().reduced_motion);
    assert_eq!(session.motion_profile().duration_scale(), 0.0);

    // 系统设置恢复后刷新生效
    shell.set_reduced_motion(false);
    session.refresh_system_preferences();
    assert!(!session.motion_profile().reduced_motion);
    assert_eq!(session.motion_profile().duration_scale(), 1.0);
}

#[test]
fn theme_preference_can_override_and_return_to_system() {
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Light, false));
    let clock = Arc::new(FakeClock::new());
    let video = Arc::new(FakeVideoBackend);

    let session = AppSession::new(shell.clone(), clock, video);
    assert_eq!(session.resolved_theme(), ResolvedTheme::Light);

    session.set_theme_preference(ThemePreference::Dark);
    assert_eq!(session.resolved_theme(), ResolvedTheme::Dark);

    // 切回跟随系统后应立即反映当前系统主题
    shell.set_theme(ResolvedTheme::Light);
    session.set_theme_preference(ThemePreference::System);
    assert_eq!(session.resolved_theme(), ResolvedTheme::Light);

    shell.set_theme(ResolvedTheme::Dark);
    session.refresh_system_preferences();
    assert_eq!(session.resolved_theme(), ResolvedTheme::Dark);
}

#[test]
fn opening_file_entry_updates_session_and_clean_close_exits_immediately() {
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Light, false));
    let clock = Arc::new(FakeClock::new());
    let video = Arc::new(FakeVideoBackend);

    let session = AppSession::new(shell, clock.clone(), video);
    clock.advance(Duration::from_millis(16));

    let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let sample_path: PathBuf = temp_dir.path().join("hello.txt");
    std::fs::write(&sample_path, "hello anyfile\n").expect("写入测试文件失败");

    session
        .open_file(&sample_path)
        .expect("打开真实文本文件应成功");

    let entry = session
        .current_file_entry()
        .expect("打开文件后应记录当前 FileEntry");
    assert_eq!(entry.path, sample_path);
    assert_eq!(entry.media_category, Some(MediaCategory::Text));
    assert_eq!(session.active_viewer(), ViewerKind::Text);

    // 无未保存修改时关闭窗口应允许立即退出（ADR-0002）
    assert_eq!(session.request_close(), CloseDecision::AllowExit);
}
