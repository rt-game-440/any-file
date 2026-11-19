//! 原子写入与外部修改检测。
//!
//! 先写同目录临时文件并 `sync_all`，再用 `ReplaceFileW`（Windows，保留原文件属性 / ACL）
//! 或 `rename` 原子替换目标；任何一步失败都不会损坏原文件，且会清理临时文件。

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::probing::to_io_path;

/// 文件指纹：长度 + 修改时间 + 内容哈希，用于检测外部修改。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFingerprint {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub hash: u64,
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(bytes);
    h.finish()
}

impl FileFingerprint {
    /// 以刚读取 / 刚写入的字节为基准生成指纹（修改时间取磁盘当前值）。
    pub fn of_bytes(path: &Path, bytes: &[u8]) -> Self {
        let modified = fs::metadata(to_io_path(path))
            .and_then(|m| m.modified())
            .ok();
        Self {
            len: bytes.len() as u64,
            modified,
            hash: hash_bytes(bytes),
        }
    }
}

/// 磁盘文件相对指纹的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    Modified,
    Deleted,
}

/// 检测磁盘文件是否被外部修改。`deep=false` 时长度与时间一致即视为未变（轮询用）。
pub fn check_disk_state(path: &Path, fp: &FileFingerprint, deep: bool) -> DiskState {
    let io = to_io_path(path);
    let Ok(meta) = fs::metadata(&io) else {
        return DiskState::Deleted;
    };
    if !deep && meta.len() == fp.len && meta.modified().ok() == fp.modified {
        return DiskState::Unchanged;
    }
    match fs::read(&io) {
        Ok(bytes) if bytes.len() as u64 == fp.len && hash_bytes(&bytes) == fp.hash => {
            DiskState::Unchanged
        }
        Ok(_) => DiskState::Modified,
        Err(_) => DiskState::Deleted,
    }
}

/// 写入失败的分类。
#[derive(Debug)]
pub enum WriteError {
    /// 只读属性 / 权限不足 / UAC 保护路径：不提权，引导“另存为”
    AccessDenied(String),
    Io(String),
}

fn classify(e: &std::io::Error, ctx: &str) -> WriteError {
    // 5 = ERROR_ACCESS_DENIED，19 = ERROR_WRITE_PROTECT，1314 = ERROR_PRIVILEGE_NOT_HELD
    let denied = e.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(e.raw_os_error(), Some(5) | Some(19) | Some(1314));
    if denied {
        WriteError::AccessDenied(format!("{ctx}：没有写入权限（{e}）"))
    } else {
        WriteError::Io(format!("{ctx}：{e}"))
    }
}

fn temp_path_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    target.with_file_name(format!(".{name}.anyfile-{}.tmp", std::process::id()))
}

/// 原子写入 `bytes` 到 `target`。目标存在时保留其文件属性；只读目标直接拒绝。
pub fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    let io_target = to_io_path(target);
    let existing = fs::metadata(&io_target).ok();
    if let Some(meta) = &existing
        && meta.permissions().readonly()
    {
        return Err(WriteError::AccessDenied("文件为只读属性".to_string()));
    }

    let tmp = temp_path_for(&io_target);
    let write_tmp = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    };
    if let Err(e) = write_tmp() {
        let _ = fs::remove_file(&tmp);
        return Err(classify(&e, "无法写入临时文件"));
    }

    let replaced = if existing.is_some() {
        replace_existing(&io_target, &tmp)
    } else {
        fs::rename(&tmp, &io_target)
    };
    if let Err(e) = replaced {
        let _ = fs::remove_file(&tmp);
        return Err(classify(&e, "无法替换目标文件"));
    }
    Ok(())
}

#[cfg(windows)]
fn replace_existing(target: &Path, tmp: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};

    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (t, r) = (wide(target), wide(tmp));
    // SAFETY: 两个路径均为以 0 结尾的有效 UTF-16 缓冲区，其余指针参数允许为空。
    let ok = unsafe {
        ReplaceFileW(
            t.as_ptr(),
            r.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn replace_existing(target: &Path, tmp: &Path) -> std::io::Result<()> {
    if let Ok(meta) = fs::metadata(target) {
        let _ = fs::set_permissions(tmp, meta.permissions());
    }
    fs::rename(tmp, target)
}
