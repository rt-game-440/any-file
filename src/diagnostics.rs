//! 本地诊断日志：写入数据目录 `logs/yyyy-MM-dd HH-mm-ss.log`，一次启动复用同一文件。
//! 零遥测：日志只落本地，不含密钥，不上传。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const KEEP_LOGS: usize = 20;

/// 将 Unix 秒转为 `yyyy-MM-dd HH-mm-ss`（传入已换算为本地时间的秒数，避免引入时间库）
pub fn format_timestamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}-{:02}-{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub struct Logger {
    path: PathBuf,
    file: Mutex<File>,
}

impl Logger {
    pub fn create(data_dir: &Path, now_secs: u64) -> std::io::Result<Self> {
        let dir = data_dir.join("logs");
        fs::create_dir_all(&dir)?;
        let mut path = dir.join(format!("{}.log", format_timestamp(now_secs)));
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{} ({n}).log", format_timestamp(now_secs)));
            n += 1;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        prune(&dir, KEEP_LOGS);
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn log(&self, level: &str, msg: &str) {
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "[{level}] {msg}");
        }
    }
}

fn prune(dir: &Path, keep: usize) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .collect();
    logs.sort();
    if logs.len() > keep {
        for old in &logs[..logs.len() - keep] {
            let _ = fs::remove_file(old);
        }
    }
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// 初始化全局日志（失败则静默，日志不应影响主流程）
pub fn init(data_dir: &Path) -> Option<&'static Logger> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(l) = Logger::create(data_dir, to_local_secs(now, local_utc_offset_secs())) {
        let _ = GLOBAL.set(l);
    }
    GLOBAL.get()
}

/// UTC 秒 + 本地时区偏移（秒）-> 本地“墙钟”秒
pub fn to_local_secs(utc_secs: u64, offset_secs: i64) -> u64 {
    (utc_secs as i64 + offset_secs).max(0) as u64
}

/// 本地相对 UTC 的偏移（秒，含夏令时）；非 Windows 或失败时为 0
pub fn local_utc_offset_secs() -> i64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        // SAFETY: 仅向有效的本地结构体写入
        unsafe {
            let mut tz: TIME_ZONE_INFORMATION = std::mem::zeroed();
            let id = GetTimeZoneInformation(&mut tz);
            if id == 0xFFFF_FFFF {
                return 0;
            }
            let bias = tz.Bias + if id == 2 { tz.DaylightBias } else { tz.StandardBias };
            -(bias as i64) * 60
        }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

pub fn info(msg: &str) {
    if let Some(l) = GLOBAL.get() {
        l.log("INFO", msg);
    }
}

pub fn error(msg: &str) {
    if let Some(l) = GLOBAL.get() {
        l.log("ERROR", msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_shifts_by_offset() {
        // UTC 2024-01-01 00:00:00 在 UTC+8 为 08:00:00
        let utc = 1_704_067_200;
        assert_eq!(
            format_timestamp(to_local_secs(utc, 8 * 3600)),
            "2024-01-01 08-00-00"
        );
        assert_eq!(
            format_timestamp(to_local_secs(utc, -5 * 3600)),
            "2023-12-31 19-00-00"
        );
        assert!(local_utc_offset_secs().abs() <= 14 * 3600);
    }

    #[test]
    fn timestamp_format() {
        assert_eq!(format_timestamp(0), "1970-01-01 00-00-00");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14 22-13-20");
    }

    #[test]
    fn one_file_per_launch_and_pruned() {
        let d = tempfile::tempdir().unwrap();
        let l = Logger::create(d.path(), 1_700_000_000).unwrap();
        l.log("INFO", "a");
        l.log("INFO", "b");
        let text = fs::read_to_string(l.path()).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(
            l.path()
                .to_string_lossy()
                .ends_with("2023-11-14 22-13-20.log")
        );
        for i in 0..30 {
            Logger::create(d.path(), 1_600_000_000 + i).unwrap();
        }
        assert!(fs::read_dir(d.path().join("logs")).unwrap().count() <= KEEP_LOGS + 1);
    }
}
