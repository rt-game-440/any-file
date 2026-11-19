//! 配置与最近查看列表的原子持久化：便携/安装模式目录隔离、跨进程文件锁、版本迁移。

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::ThemePreference;
use crate::i18n::LanguagePreference;

/// 程序目录下存在该文件即进入便携模式
pub const PORTABLE_MARKER: &str = "portable.flag";
pub const CONFIG_VERSION: u32 = 1;
pub const RECENT_VERSION: u32 = 1;
pub const MAX_RECENT: usize = 20;

const CONFIG_FILE: &str = "config.json";
const RECENT_FILE: &str = "recent.json";
const LOCK_FILE: &str = "store.lock";
const LOCK_STALE: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMode {
    Portable,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLocation {
    pub mode: StorageMode,
    pub dir: PathBuf,
}

/// 依据程序目录便携标记文件区分模式：便携 -> `<exe>/data`；否则 `%APPDATA%/AnyFile`。
pub fn resolve_data_location(exe_dir: &Path, appdata: Option<&Path>) -> DataLocation {
    if exe_dir.join(PORTABLE_MARKER).is_file() {
        return DataLocation {
            mode: StorageMode::Portable,
            dir: exe_dir.join("data"),
        };
    }
    let dir = match appdata {
        Some(a) => a.join("AnyFile"),
        None => exe_dir.join("data"),
    };
    DataLocation {
        mode: StorageMode::Installed,
        dir,
    }
}

/// 当前进程的数据位置（真实环境）
pub fn current_data_location() -> DataLocation {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    resolve_data_location(&exe_dir, appdata.as_deref())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub theme: ThemePreference,
    pub language: LanguagePreference,
    pub auto_save: bool,
    pub recent_enabled: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            theme: ThemePreference::System,
            language: LanguagePreference::System,
            auto_save: false,
            recent_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: String,
    pub opened_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentList {
    pub version: u32,
    pub entries: Vec<RecentEntry>,
}

impl Default for RecentList {
    fn default() -> Self {
        Self {
            version: RECENT_VERSION,
            entries: Vec::new(),
        }
    }
}

/// 将旧版本配置 JSON 迁移到当前版本。v0（无 version 字段）使用 `autosave` 键。
pub fn migrate_config(mut value: Value) -> Value {
    let Some(obj) = value.as_object_mut() else {
        return Value::Object(Default::default());
    };
    let ver = obj.get("version").and_then(Value::as_u64).unwrap_or(0);
    if ver < 1 {
        if let Some(v) = obj.remove("autosave") {
            obj.insert("auto_save".into(), v);
        }
        obj.insert("version".into(), Value::from(1));
    }
    value
}

fn migrate_recent(mut value: Value) -> Value {
    let Some(obj) = value.as_object_mut() else {
        return Value::Object(Default::default());
    };
    // v0：{"files": [路径字符串...]}
    if obj.get("version").is_none() {
        if let Some(Value::Array(files)) = obj.remove("files") {
            let entries: Vec<Value> = files
                .into_iter()
                .filter_map(|f| {
                    f.as_str()
                        .map(|s| serde_json::json!({"path": s, "opened_at": 0}))
                })
                .collect();
            obj.insert("entries".into(), Value::Array(entries));
        }
        obj.insert("version".into(), Value::from(1));
    }
    value
}

/// 跨进程文件锁（`create_new` 独占创建，过期锁自动回收）
struct FileLock {
    path: PathBuf,
}

impl FileLock {
    fn acquire(path: PathBuf) -> io::Result<Self> {
        let start = std::time::Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > LOCK_TIMEOUT {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "获取配置文件锁超时",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(3));
                }
                // Windows 上锁文件正被删除时可能返回 PermissionDenied，重试即可
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    if start.elapsed() > LOCK_TIMEOUT {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(3));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 原子写入：同目录临时文件写满并 fsync 后再 rename 覆盖目标。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        let mut last = None;
        for _ in 0..20 {
            match fs::rename(&tmp, path) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        Err(last.unwrap())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn same_path(a: &str, b: &str) -> bool {
    crate::sibling::paths_equivalent(Path::new(a), Path::new(b))
}

/// 配置与最近列表存储：每次读改写均持有文件锁，多窗口/多进程并发安全。
#[derive(Debug, Clone)]
pub struct ConfigStore {
    dir: PathBuf,
}

impl ConfigStore {
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn lock(&self) -> io::Result<FileLock> {
        FileLock::acquire(self.dir.join(LOCK_FILE))
    }

    fn read_json(&self, name: &str) -> Option<Value> {
        let path = self.dir.join(name);
        let text = fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => Some(v),
            Err(_) => {
                // 损坏文件改名留存，回落默认值
                let _ = fs::rename(&path, self.dir.join(format!("{name}.corrupt")));
                None
            }
        }
    }

    fn load_config_unlocked(&self) -> AppConfig {
        self.read_json(CONFIG_FILE)
            .map(migrate_config)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    fn load_recent_unlocked(&self) -> RecentList {
        self.read_json(RECENT_FILE)
            .map(migrate_recent)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    fn save_config_unlocked(&self, cfg: &AppConfig) -> io::Result<()> {
        let mut cfg = cfg.clone();
        cfg.version = cfg.version.max(CONFIG_VERSION);
        atomic_write(
            &self.dir.join(CONFIG_FILE),
            &serde_json::to_vec_pretty(&cfg)?,
        )
    }

    fn save_recent_unlocked(&self, list: &RecentList) -> io::Result<()> {
        atomic_write(
            &self.dir.join(RECENT_FILE),
            &serde_json::to_vec_pretty(list)?,
        )
    }

    pub fn load_config(&self) -> io::Result<AppConfig> {
        let _g = self.lock()?;
        Ok(self.load_config_unlocked())
    }

    /// 读-改-写配置（持锁）
    pub fn update_config(&self, f: impl FnOnce(&mut AppConfig)) -> io::Result<AppConfig> {
        let _g = self.lock()?;
        let mut cfg = self.load_config_unlocked();
        f(&mut cfg);
        self.save_config_unlocked(&cfg)?;
        Ok(cfg)
    }

    pub fn recent(&self) -> io::Result<Vec<RecentEntry>> {
        let _g = self.lock()?;
        Ok(self.load_recent_unlocked().entries)
    }

    /// 记录最近查看：已关闭记录时为空操作；重复路径前移去重；超出上限截断。
    pub fn add_recent(&self, path: &Path) -> io::Result<()> {
        let _g = self.lock()?;
        if !self.load_config_unlocked().recent_enabled {
            return Ok(());
        }
        let mut list = self.load_recent_unlocked();
        let p = path.to_string_lossy().into_owned();
        list.entries.retain(|e| !same_path(&e.path, &p));
        list.entries.insert(
            0,
            RecentEntry {
                path: p,
                opened_at: now_unix(),
            },
        );
        list.entries.truncate(MAX_RECENT);
        self.save_recent_unlocked(&list)
    }

    pub fn remove_recent(&self, path: &Path) -> io::Result<()> {
        let _g = self.lock()?;
        let mut list = self.load_recent_unlocked();
        let p = path.to_string_lossy();
        let before = list.entries.len();
        list.entries.retain(|e| !same_path(&e.path, &p));
        if list.entries.len() != before {
            self.save_recent_unlocked(&list)?;
        }
        Ok(())
    }

    pub fn clear_recent(&self) -> io::Result<()> {
        let _g = self.lock()?;
        self.save_recent_unlocked(&RecentList::default())
    }

    /// 开关最近记录；关闭时同时彻底清空已有记录。
    pub fn set_recent_enabled(&self, enabled: bool) -> io::Result<()> {
        let _g = self.lock()?;
        let mut cfg = self.load_config_unlocked();
        cfg.recent_enabled = enabled;
        self.save_config_unlocked(&cfg)?;
        if !enabled {
            self.save_recent_unlocked(&RecentList::default())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn portable_marker_selects_mode() {
        let exe = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();
        let inst = resolve_data_location(exe.path(), Some(appdata.path()));
        assert_eq!(inst.mode, StorageMode::Installed);
        assert_eq!(inst.dir, appdata.path().join("AnyFile"));

        fs::write(exe.path().join(PORTABLE_MARKER), "").unwrap();
        let port = resolve_data_location(exe.path(), Some(appdata.path()));
        assert_eq!(port.mode, StorageMode::Portable);
        assert_eq!(port.dir, exe.path().join("data"));
        assert_ne!(port.dir, inst.dir);
    }

    #[test]
    fn defaults_and_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        assert_eq!(s.load_config().unwrap(), AppConfig::default());
        s.update_config(|c| {
            c.theme = ThemePreference::Dark;
            c.language = LanguagePreference::En;
            c.auto_save = true;
        })
        .unwrap();
        let c = ConfigStore::open(d.path()).unwrap().load_config().unwrap();
        assert_eq!(c.theme, ThemePreference::Dark);
        assert_eq!(c.language, LanguagePreference::En);
        assert!(c.auto_save);
        let leftovers = fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn migrates_v0_config() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(CONFIG_FILE),
            r#"{"theme":"light","autosave":true}"#,
        )
        .unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        let c = s.load_config().unwrap();
        assert_eq!(c.version, CONFIG_VERSION);
        assert_eq!(c.theme, ThemePreference::Light);
        assert!(c.auto_save);
        assert!(c.recent_enabled);
    }

    #[test]
    fn migrates_v0_recent() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(RECENT_FILE),
            r#"{"files":["C:\\a.txt","C:\\b.txt"]}"#,
        )
        .unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        let r = s.recent().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].path, "C:\\a.txt");
    }

    #[test]
    fn corrupt_file_falls_back_and_is_preserved() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(CONFIG_FILE), "{not json").unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        assert_eq!(s.load_config().unwrap(), AppConfig::default());
        assert!(d.path().join("config.json.corrupt").exists());
    }

    #[test]
    fn recent_dedup_order_limit() {
        let d = tempfile::tempdir().unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        s.add_recent(Path::new("C:\\x\\a.txt")).unwrap();
        s.add_recent(Path::new("C:\\x\\b.txt")).unwrap();
        s.add_recent(Path::new("C:\\x\\a.txt")).unwrap();
        let r = s.recent().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].path, "C:\\x\\a.txt");
        for i in 0..(MAX_RECENT + 5) {
            s.add_recent(Path::new(&format!("C:\\x\\f{i}.txt")))
                .unwrap();
        }
        assert_eq!(s.recent().unwrap().len(), MAX_RECENT);
        s.remove_recent(Path::new("C:\\x\\f24.txt")).unwrap();
        assert_eq!(s.recent().unwrap().len(), MAX_RECENT - 1);
    }

    #[test]
    fn clear_and_disable_recent() {
        let d = tempfile::tempdir().unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        s.add_recent(Path::new("C:\\a.txt")).unwrap();
        s.clear_recent().unwrap();
        assert!(s.recent().unwrap().is_empty());

        s.add_recent(Path::new("C:\\a.txt")).unwrap();
        s.set_recent_enabled(false).unwrap();
        assert!(s.recent().unwrap().is_empty());
        s.add_recent(Path::new("C:\\b.txt")).unwrap();
        assert!(s.recent().unwrap().is_empty(), "关闭后不得再记录");
        assert!(!s.load_config().unwrap().recent_enabled);
        s.set_recent_enabled(true).unwrap();
        s.add_recent(Path::new("C:\\b.txt")).unwrap();
        assert_eq!(s.recent().unwrap().len(), 1);
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let d = tempfile::tempdir().unwrap();
        let s = ConfigStore::open(d.path()).unwrap();
        let lock = d.path().join(LOCK_FILE);
        fs::write(&lock, "").unwrap();
        let old = SystemTime::now() - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        s.update_config(|c| c.auto_save = true).unwrap();
        assert!(s.load_config().unwrap().auto_save);
    }

    #[test]
    fn concurrent_windows_do_not_lose_updates() {
        let d = tempfile::tempdir().unwrap();
        let dir = Arc::new(d.path().to_path_buf());
        let handles: Vec<_> = (0..6)
            .map(|t| {
                let dir = Arc::clone(&dir);
                std::thread::spawn(move || {
                    let s = ConfigStore::open(dir.as_path()).unwrap();
                    for i in 0..8 {
                        s.add_recent(Path::new(&format!("C:\\w{t}\\f{i}.txt")))
                            .unwrap();
                        s.update_config(|c| c.auto_save = !c.auto_save).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let s = ConfigStore::open(d.path()).unwrap();
        assert_eq!(s.recent().unwrap().len(), MAX_RECENT);
        // 48 次翻转回到 false；若有丢失更新则几乎必然不为 false
        assert!(!s.load_config().unwrap().auto_save);
    }
}
