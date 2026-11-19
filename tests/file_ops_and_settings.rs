use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyfile::config::ConfigStore;
use anyfile::domain::{ResolvedTheme, ThemePreference, ViewerKind};
use anyfile::i18n::{Language, LanguagePreference};
use anyfile::ports::{Clock, SystemShell, VideoBackend, VideoProbeInfo};
use anyfile::session::AppSession;

#[derive(Default)]
struct Shell {
    locale: Mutex<String>,
    clipboard: Mutex<Vec<String>>,
    revealed: Mutex<Vec<PathBuf>>,
}
impl SystemShell for Shell {
    fn system_theme(&self) -> ResolvedTheme {
        ResolvedTheme::Light
    }
    fn prefers_reduced_motion(&self) -> bool {
        false
    }
    fn move_to_trash(&self, p: &Path) -> Result<(), String> {
        std::fs::remove_file(p).map_err(|e| e.to_string())
    }
    fn reveal_in_explorer(&self, p: &Path) -> Result<(), String> {
        self.revealed.lock().unwrap().push(p.to_path_buf());
        Ok(())
    }
    fn copy_text_to_clipboard(&self, t: &str) -> Result<(), String> {
        self.clipboard.lock().unwrap().push(t.to_string());
        Ok(())
    }
    fn system_language(&self) -> String {
        self.locale.lock().unwrap().clone()
    }
}
struct Clk;
impl Clock for Clk {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
struct NoVid;
impl VideoBackend for NoVid {
    fn is_available(&self) -> bool {
        false
    }
    fn probe(&self, _: &Path) -> Option<VideoProbeInfo> {
        None
    }
}

fn session(shell: Arc<Shell>, data: &Path) -> AppSession {
    AppSession::new(shell, Arc::new(Clk), Arc::new(NoVid))
        .with_config_store(ConfigStore::open(data).unwrap())
}

#[test]
fn delete_moves_to_next_then_prev_then_welcome_and_syncs_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(n), "hello").unwrap();
    }
    let s = session(Arc::new(Shell::default()), data.path());
    s.open_file(&dir.path().join("b.txt")).unwrap();
    let e = s.delete_current_file().unwrap().unwrap();
    assert!(e.path.ends_with("c.txt"));
    assert!(!dir.path().join("b.txt").exists());
    let seq = s.sibling_sequence();
    assert_eq!(seq.entries.len(), 2);
    assert!(seq.entries.iter().all(|x| !x.path.ends_with("b.txt")));
    let e = s.delete_current_file().unwrap().unwrap();
    assert!(e.path.ends_with("a.txt"));
    assert!(s.delete_current_file().unwrap().is_none());
    assert_eq!(s.active_viewer(), ViewerKind::Welcome);
    assert!(s.delete_current_file().is_err());
    assert!(s.recent_files().is_empty());
}

#[test]
fn copy_path_and_reveal() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "x").unwrap();
    let shell = Arc::new(Shell::default());
    let s = session(shell.clone(), data.path());
    assert!(s.copy_current_path().is_err());
    s.open_file(&f).unwrap();
    s.copy_current_path().unwrap();
    s.reveal_current_in_explorer().unwrap();
    assert_eq!(shell.clipboard.lock().unwrap()[0], f.to_string_lossy());
    assert_eq!(shell.revealed.lock().unwrap()[0], f);
}

#[test]
fn settings_persist_across_sessions_and_recent_controls() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "x").unwrap();
    let shell = Arc::new(Shell::default());
    *shell.locale.lock().unwrap() = "zh-CN".into();
    let s = session(shell.clone(), data.path());
    assert_eq!(s.language(), Language::ZhCn);
    s.set_language_preference(LanguagePreference::En);
    assert_eq!(s.language(), Language::En);
    s.set_theme_preference(ThemePreference::Dark);
    s.set_auto_save(true);
    s.open_file(&f).unwrap();
    assert_eq!(s.recent_files().len(), 1);

    let s2 = session(shell.clone(), data.path());
    assert_eq!(s2.language(), Language::En);
    assert_eq!(s2.theme_preference(), ThemePreference::Dark);
    assert_eq!(s2.resolved_theme(), ResolvedTheme::Dark);
    assert!(s2.auto_save());
    assert_eq!(s2.recent_files().len(), 1);
    s2.clear_recent();
    assert!(s2.recent_files().is_empty());
    s2.open_file(&f).unwrap();
    s2.set_recent_enabled(false);
    assert!(s2.recent_files().is_empty());
    s2.open_file(&f).unwrap();
    assert!(s2.recent_files().is_empty());
}
