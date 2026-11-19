//! 文本编辑态（Editing Mode，ADR-0004，工单 #11）核心：编辑缓冲区、撤销重做、查找替换、
//! 数据安全的原子保存（保留编码 / BOM / 换行符）与外部修改冲突检测。
//!
//! 编辑缓冲区内部统一使用 `\n` 换行，保存时按原文件的换行符还原。
//! 本模块与 UI 无关，可被无头测试完整覆盖。

pub mod atomic;
pub mod encode;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::probing::to_io_path;
use crate::text_viewer::{LARGE_FILE_THRESHOLD, LineEnding, TextEncoding, decode_bytes};

pub use atomic::{DiskState, FileFingerprint, WriteError, atomic_write, check_disk_state};
pub use encode::{Unrepresentable, encode_text, normalize_newlines};

/// 撤销栈最大条数。
pub const MAX_HISTORY: usize = 200;
/// 连续输入合并为一个撤销步的时间窗口。
pub const UNDO_COALESCE: Duration = Duration::from_millis(800);
/// 自动保存的停顿时间（最后一次编辑后）。
pub const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);
/// 查找 / 替换的最大匹配数。
pub const MAX_EDIT_MATCHES: usize = 20_000;

/// 保存时的可选行为。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveOptions {
    /// 忽略外部修改冲突，强制覆盖
    pub force: bool,
    /// 原编码无法表示字符时，用户确认后改以 UTF-8（无 BOM）保存
    pub convert_to_utf8: bool,
}

/// 外部修改冲突类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    Modified,
    Deleted,
}

/// 保存结果。任何非 `Saved` 结果都保证磁盘上的原文件未被改动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// 磁盘文件已被外部修改 / 删除：由用户选择覆盖、重新载入或另存为
    Conflict(ConflictKind),
    /// 含原编码无法表示的字符：由用户选择转 UTF-8 保存或返回修改
    Unrepresentable(Unrepresentable),
    /// 没有写入权限（只读 / UAC 保护路径）：引导“另存为”，不提权
    NeedSaveAs(String),
    Failed(String),
}

/// 编辑缓冲区。
#[derive(Debug, Clone)]
pub struct TextEditor {
    path: PathBuf,
    text: String,
    saved_text: String,
    encoding: TextEncoding,
    has_bom: bool,
    /// 保存时使用的换行符（混合换行取占多数者）
    line_ending: LineEnding,
    original_mixed: bool,
    fingerprint: FileFingerprint,
    undo: Vec<String>,
    redo: Vec<String>,
    last_edit: Option<Instant>,
    last_undo_push: Option<Instant>,
    external: Option<ConflictKind>,
}

impl TextEditor {
    /// 从磁盘载入以编辑。大文件（> 10MB）与无法无损往返的文件一律拒绝。
    pub fn open(path: &Path) -> Result<Self, String> {
        let io = to_io_path(path);
        let size = std::fs::metadata(&io)
            .map_err(|e| format!("无法读取文件信息: {e}"))?
            .len();
        if size > LARGE_FILE_THRESHOLD {
            return Err("大文件只读保护：已禁用编辑".to_string());
        }
        let bytes = std::fs::read(&io).map_err(|e| format!("读取文件失败: {e}"))?;
        let mut editor = Self::from_bytes(path, &bytes)?;
        editor.fingerprint = FileFingerprint::of_bytes(path, &bytes);
        Ok(editor)
    }

    /// 由字节构建（指纹的修改时间取当前磁盘值）。
    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self, String> {
        let decoded = decode_bytes(bytes);
        let (text, ending) = normalize_newlines(&decoded.text);
        let original_mixed = has_mixed_newlines(&decoded.text);

        // 无损往返检查：解码有损（如损坏字节）时拒绝编辑，避免保存时静默篡改内容
        let lossy = if original_mixed {
            decoded.text.contains('\u{FFFD}')
        } else {
            match encode_text(&text, decoded.encoding, decoded.has_bom, ending) {
                Ok(round) => round != bytes,
                Err(_) => true,
            }
        };
        if lossy {
            return Err("文件含有无法无损解码的字节，已禁用编辑以防数据损坏".to_string());
        }

        Ok(Self {
            path: path.to_path_buf(),
            saved_text: text.clone(),
            text,
            encoding: decoded.encoding,
            has_bom: decoded.has_bom,
            line_ending: ending,
            original_mixed,
            fingerprint: FileFingerprint::of_bytes(path, bytes),
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            last_undo_push: None,
            external: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }
    pub fn original_mixed(&self) -> bool {
        self.original_mixed
    }
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn external_conflict(&self) -> Option<ConflictKind> {
        self.external
    }
    pub fn fingerprint(&self) -> &FileFingerprint {
        &self.fingerprint
    }

    /// 用户输入后同步整个缓冲区。`\r\n` / `\r` 会被规整为 `\n`。
    /// 返回缓冲区是否发生变化。
    pub fn set_text(&mut self, new_text: &str, now: Instant) -> bool {
        let new_text = if new_text.contains('\r') {
            normalize_newlines(new_text).0
        } else {
            new_text.to_string()
        };
        if new_text == self.text {
            return false;
        }
        self.record_undo(now);
        self.text = new_text;
        self.redo.clear();
        self.last_edit = Some(now);
        true
    }

    fn record_undo(&mut self, now: Instant) {
        let coalesce = self
            .last_undo_push
            .is_some_and(|t| now.saturating_duration_since(t) < UNDO_COALESCE)
            && !self.undo.is_empty();
        if !coalesce {
            self.undo.push(self.text.clone());
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
            }
        }
        self.last_undo_push = Some(now);
    }

    /// 撤销一步；返回是否有变化。
    pub fn undo(&mut self, now: Instant) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.text, prev));
        self.last_undo_push = None;
        self.last_edit = Some(now);
        true
    }

    /// 重做一步；返回是否有变化。
    pub fn redo(&mut self, now: Instant) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.text, next));
        self.last_undo_push = None;
        self.last_edit = Some(now);
        true
    }

    /// 查找所有匹配（字节区间，最多 [`MAX_EDIT_MATCHES`] 个）。
    pub fn find_all(&self, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
        find_ranges(&self.text, query, case_sensitive)
    }

    /// 替换 `from_byte` 之后（含）的第一个匹配；到末尾则回绕。
    /// 返回被替换匹配的起点，以及替换后应选中的区间。
    pub fn replace_next(
        &mut self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
        from_byte: usize,
        now: Instant,
    ) -> Option<(usize, usize)> {
        let ranges = self.find_all(query, case_sensitive);
        let (s, e) = ranges
            .iter()
            .copied()
            .find(|(s, _)| *s >= from_byte)
            .or_else(|| ranges.first().copied())?;
        let mut new_text = String::with_capacity(self.text.len() + replacement.len());
        new_text.push_str(&self.text[..s]);
        new_text.push_str(replacement);
        new_text.push_str(&self.text[e..]);
        self.record_undo(now);
        self.text = new_text;
        self.redo.clear();
        self.last_edit = Some(now);
        // 下一次从替换结尾继续
        Some((s, s + replacement.len()))
    }

    /// 全部替换（一次撤销即可还原）；返回替换数量。
    pub fn replace_all(
        &mut self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
        now: Instant,
    ) -> usize {
        let ranges = self.find_all(query, case_sensitive);
        if ranges.is_empty() {
            return 0;
        }
        let mut out = String::with_capacity(self.text.len());
        let mut last = 0;
        for (s, e) in &ranges {
            out.push_str(&self.text[last..*s]);
            out.push_str(replacement);
            last = *e;
        }
        out.push_str(&self.text[last..]);
        // 全部替换总是独立的撤销步
        self.last_undo_push = None;
        self.record_undo(now);
        self.last_undo_push = None;
        self.text = out;
        self.redo.clear();
        self.last_edit = Some(now);
        ranges.len()
    }

    /// 轮询磁盘状态（轻量：长度 / 修改时间不变则不读内容）。冲突状态会被记录。
    pub fn check_external(&mut self, deep: bool) -> Option<ConflictKind> {
        let state = check_disk_state(&self.path, &self.fingerprint, deep);
        self.external = match state {
            DiskState::Unchanged => None,
            DiskState::Modified => Some(ConflictKind::Modified),
            DiskState::Deleted => Some(ConflictKind::Deleted),
        };
        self.external
    }

    /// 放弃缓冲区，重新从磁盘载入（“重新载入”选项）。
    pub fn reload(&mut self) -> Result<(), String> {
        let fresh = Self::open(&self.path)?;
        *self = fresh;
        Ok(())
    }

    /// 自动保存是否到期：有未保存修改、无冲突，且距最后一次编辑已超过停顿时间。
    pub fn autosave_due(&self, now: Instant) -> bool {
        self.is_dirty()
            && self.external.is_none()
            && self
                .last_edit
                .is_some_and(|t| now.saturating_duration_since(t) >= AUTOSAVE_DELAY)
    }

    /// 保存到当前路径。
    pub fn save(&mut self, opts: SaveOptions) -> SaveOutcome {
        if !opts.force
            && let Some(kind) = self.check_external(true)
        {
            return SaveOutcome::Conflict(kind);
        }
        let (encoding, has_bom) = if opts.convert_to_utf8 {
            (TextEncoding::Utf8, false)
        } else {
            (self.encoding, self.has_bom)
        };
        let bytes = match encode_text(&self.text, encoding, has_bom, self.line_ending) {
            Ok(b) => b,
            Err(u) => return SaveOutcome::Unrepresentable(u),
        };
        let target = self.path.clone();
        self.write_and_commit(&target, &bytes, encoding, has_bom)
    }

    /// 另存为：写入新路径并让编辑器改为跟踪新文件。不做冲突检测（覆盖确认由 UI 对话框负责）。
    pub fn save_as(&mut self, new_path: &Path, opts: SaveOptions) -> SaveOutcome {
        let (encoding, has_bom) = if opts.convert_to_utf8 {
            (TextEncoding::Utf8, false)
        } else {
            (self.encoding, self.has_bom)
        };
        let bytes = match encode_text(&self.text, encoding, has_bom, self.line_ending) {
            Ok(b) => b,
            Err(u) => return SaveOutcome::Unrepresentable(u),
        };
        self.write_and_commit(new_path, &bytes, encoding, has_bom)
    }

    fn write_and_commit(
        &mut self,
        target: &Path,
        bytes: &[u8],
        encoding: TextEncoding,
        has_bom: bool,
    ) -> SaveOutcome {
        match atomic_write(target, bytes) {
            Ok(()) => {
                self.path = target.to_path_buf();
                self.encoding = encoding;
                self.has_bom = has_bom;
                self.saved_text = self.text.clone();
                self.fingerprint = FileFingerprint::of_bytes(target, bytes);
                self.external = None;
                self.last_edit = None;
                SaveOutcome::Saved
            }
            Err(WriteError::AccessDenied(m)) => SaveOutcome::NeedSaveAs(m),
            Err(WriteError::Io(m)) => SaveOutcome::Failed(m),
        }
    }
}

fn has_mixed_newlines(raw: &str) -> bool {
    let b = raw.as_bytes();
    let (mut lf, mut crlf, mut cr) = (false, false, false);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf = true;
                i += 2;
                continue;
            }
            b'\r' => cr = true,
            b'\n' => lf = true,
            _ => {}
        }
        i += 1;
    }
    [lf, crlf, cr].iter().filter(|x| **x).count() > 1
}

/// 在文本中查找所有非重叠匹配（字节区间）。不区分大小写时按单字符小写逐字符比较，
/// 保证返回的偏移始终落在原文本的字符边界上。
pub fn find_ranges(text: &str, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return text
            .match_indices(query)
            .take(MAX_EDIT_MATCHES)
            .map(|(s, m)| (s, s + m.len()))
            .collect();
    }
    let fold = |c: char| {
        let mut it = c.to_lowercase();
        match (it.next(), it.next()) {
            (Some(l), None) => l,
            _ => c,
        }
    };
    let q: Vec<char> = query.chars().map(fold).collect();
    let hay: Vec<(usize, char)> = text.char_indices().map(|(i, c)| (i, fold(c))).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= hay.len() && out.len() < MAX_EDIT_MATCHES {
        if hay[i..i + q.len()].iter().zip(&q).all(|((_, a), b)| a == b) {
            let start = hay[i].0;
            let end = hay
                .get(i + q.len())
                .map(|(o, _)| *o)
                .unwrap_or(text.len());
            out.push((start, end));
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_ranges_case_insensitive_unicode() {
        let r = find_ranges("Hello 你好 hello", "HELLO", false);
        assert_eq!(r, vec![(0, 5), (13, 18)]);
        assert_eq!(find_ranges("aaa", "aa", true), vec![(0, 2)]);
        assert!(find_ranges("abc", "", true).is_empty());
    }

    #[test]
    fn undo_redo_and_coalescing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, "abc").unwrap();
        let mut ed = TextEditor::open(&p).unwrap();
        let t0 = Instant::now();
        assert!(ed.set_text("abcd", t0));
        assert!(ed.set_text("abcde", t0 + Duration::from_millis(100)));
        assert_eq!(ed.undo.len(), 1, "快速连续输入合并为一步");
        assert!(ed.set_text("abcdef", t0 + Duration::from_secs(3)));
        assert!(ed.undo(t0));
        assert_eq!(ed.text(), "abcde");
        assert!(ed.undo(t0));
        assert_eq!(ed.text(), "abc");
        assert!(!ed.undo(t0));
        assert!(ed.redo(t0));
        assert_eq!(ed.text(), "abcde");
        assert!(ed.is_dirty());
        assert!(ed.undo(t0));
        assert!(!ed.is_dirty());
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, "foo bar foo").unwrap();
        let mut ed = TextEditor::open(&p).unwrap();
        let t = Instant::now();
        assert_eq!(ed.replace_all("foo", "x", true, t), 2);
        assert_eq!(ed.text(), "x bar x");
        assert!(ed.undo(t));
        assert_eq!(ed.text(), "foo bar foo");
        let sel = ed.replace_next("foo", "yy", true, 1, t).unwrap();
        assert_eq!(sel, (8, 10));
        assert_eq!(ed.text(), "foo bar yy");
    }
}
