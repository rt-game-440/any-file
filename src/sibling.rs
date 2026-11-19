//! 第一层文件索引：同目录序列（Sibling Sequence，遵循 ADR-0006）。
//! 仅按文件扩展名快速索引并按自然排序排列，绝不读取相邻文件内容，
//! 避免扫描大目录卡顿或触发 OneDrive 云占位文件自动下载。

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::domain::{FileEntry, MediaCategory, SiblingSequence};
use crate::probing::{classify_by_extension, to_io_path};

/// 同目录非递归文件变化监听器（基于 `notify`，Windows 下走 `ReadDirectoryChangesW`）。
/// Drop 时自动停止监听并释放句柄。
pub struct DirectoryWatcher {
    watched_dir: PathBuf,
    _watcher: RecommendedWatcher,
}

impl DirectoryWatcher {
    /// 启动对指定目录的非递归监听；当目录内发生文件增/删/重命名时触发 `on_change` 回调。
    pub fn watch<F>(dir: &Path, on_change: F) -> Result<Self, String>
    where
        F: Fn() + Send + 'static,
    {
        let mut watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res
                    && matches!(
                        event.kind,
                        EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Modify(notify::event::ModifyKind::Name(_))
                            | EventKind::Any
                    )
                {
                    on_change();
                }
            })
            .map_err(|e| format!("初始化目录监听器失败: {e}"))?;

        let io_dir = to_io_path(dir);
        watcher
            .watch(&io_dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("监听目录失败 ({}): {e}", dir.display()))?;

        Ok(Self {
            watched_dir: dir.to_path_buf(),
            _watcher: watcher,
        })
    }

    pub fn watched_dir(&self) -> &Path {
        &self.watched_dir
    }
}

/// 按自然排序比较两个文件名（如 `2.png` 排在 `10.png` 前，不区分大小写且保证严格全序）。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a_bytes = a.as_bytes();
    let mut b_bytes = b.as_bytes();

    while !a_bytes.is_empty() && !b_bytes.is_empty() {
        let a_is_digit = a_bytes[0].is_ascii_digit();
        let b_is_digit = b_bytes[0].is_ascii_digit();

        if a_is_digit && b_is_digit {
            let a_len = a_bytes
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let b_len = b_bytes
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();

            let a_num = &a_bytes[..a_len];
            let b_num = &b_bytes[..b_len];

            // 去除前导零按任意长度非负整数大小比较
            let a_trim = trim_leading_zeros(a_num);
            let b_trim = trim_leading_zeros(b_num);

            let ord = a_trim
                .len()
                .cmp(&b_trim.len())
                .then_with(|| a_trim.cmp(b_trim))
                // 数值相等时，前导零少者排前（如 "2" < "02"）
                .then_with(|| a_num.len().cmp(&b_num.len()));

            if ord != Ordering::Equal {
                return ord;
            }

            a_bytes = &a_bytes[a_len..];
            b_bytes = &b_bytes[b_len..];
        } else if a_is_digit != b_is_digit {
            return a_bytes[0]
                .to_ascii_lowercase()
                .cmp(&b_bytes[0].to_ascii_lowercase());
        } else {
            // 非数字段：按不区分大小写的 Unicode 字符比较，直到遇到数字或结束
            let a_str = std::str::from_utf8(a_bytes).unwrap_or("");
            let b_str = std::str::from_utf8(b_bytes).unwrap_or("");

            let a_non_digit_len = a_str
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(a_str.len());
            let b_non_digit_len = b_str
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(b_str.len());

            let a_seg = &a_str[..a_non_digit_len];
            let b_seg = &b_str[..b_non_digit_len];

            let ord = a_seg
                .chars()
                .flat_map(|c| c.to_lowercase())
                .cmp(b_seg.chars().flat_map(|c| c.to_lowercase()));

            if ord != Ordering::Equal {
                return ord;
            }

            a_bytes = &a_bytes[a_non_digit_len..];
            b_bytes = &b_bytes[b_non_digit_len..];
        }
    }

    a_bytes
        .len()
        .cmp(&b_bytes.len())
        .then_with(|| a.cmp(b))
}

fn trim_leading_zeros(digits: &[u8]) -> &[u8] {
    let first_non_zero = digits.iter().position(|&b| b != b'0');
    match first_non_zero {
        Some(idx) => &digits[idx..],
        None => &digits[digits.len().saturating_sub(1)..],
    }
}

/// 比较两条路径是否指向同一文件（Windows 上不区分大小写，且忽略 `\\?\` 扩展前缀差异）
pub fn paths_equivalent(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| -> String {
        let s = p.to_string_lossy();
        #[cfg(windows)]
        {
            let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
            stripped.replace('/', "\\").to_ascii_lowercase()
        }
        #[cfg(not(windows))]
        {
            s.into_owned()
        }
    };
    norm(a) == norm(b)
}

/// 仅按扩展名快速构建当前文件项所在目录的同目录序列（绝不读取相邻文件内容）。
pub fn build_sibling_sequence(
    current_entry: &FileEntry,
    filter_by_category: bool,
    filter_category: Option<MediaCategory>,
) -> SiblingSequence {
    let Some(parent) = current_entry.path.parent() else {
        return SiblingSequence {
            entries: vec![current_entry.clone()],
            current_index: Some(0),
            filter_by_category,
        };
    };

    let io_parent = to_io_path(parent);
    let read_dir = match std::fs::read_dir(&io_parent) {
        Ok(rd) => rd,
        Err(_) => {
            return SiblingSequence {
                entries: vec![current_entry.clone()],
                current_index: Some(0),
                filter_by_category,
            };
        }
    };

    let mut entries = Vec::new();
    let mut includes_current = false;

    for item in read_dir.flatten() {
        // 仅检查目录项元数据中的文件类型（Windows 上来自 WIN32_FIND_DATAW，不打开文件句柄）
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            continue;
        }

        // 用干净的 parent + file_name 组装路径，避免暴露 `\\?\` 前缀
        let path: PathBuf = parent.join(item.file_name());
        if paths_equivalent(&path, &current_entry.path) {
            entries.push(current_entry.clone());
            includes_current = true;
            continue;
        }

        // 第一层索引严格仅依据扩展名，绝不打开或读取相邻文件内容
        let ext_category = classify_by_extension(&path);
        let included = if filter_by_category {
            match (filter_category, ext_category) {
                (Some(target), Some(cat)) => target == cat,
                _ => false,
            }
        } else {
            ext_category.is_some()
        };

        if included {
            entries.push(FileEntry::new(path, ext_category));
        }
    }

    if !includes_current {
        entries.push(current_entry.clone());
    }

    entries.sort_by(|a, b| {
        let name_a = a
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let name_b = b
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        natural_cmp(name_a, name_b).then_with(|| a.path.cmp(&b.path))
    });

    let current_index = entries
        .iter()
        .position(|e| paths_equivalent(&e.path, &current_entry.path));

    SiblingSequence {
        entries,
        current_index,
        filter_by_category,
    }
}
