//! 文本查看态（Viewing Mode，ADR-0004）核心：编码自动检测、换行符识别、语法高亮分词、
//! 页内查找、字号缩放与大文件只读保护。
//!
//! 对外主入口为 [`TextDocument::load`]（只读文档模型）与 `AppSession` 上的 `text_*` 方法。
//! 编辑态（工单 #11）可复用 [`TextMeta`] 中的 `encoding` / `has_bom` / `line_ending`
//! 在保存时按原编码与换行符写回，并以 `is_large_file_readonly` 禁用一键编辑。

mod encoding;
mod find;
mod highlight;
pub mod ui_model;
mod zoom;

use std::fs::File;
use std::io::Read;
use std::path::Path;

pub use encoding::{DecodedText, TextEncoding, decode_bytes, detect_encoding};
pub use find::{FindMatch, MAX_FIND_MATCHES, TextFindState, find_in_document};
pub use highlight::{HighlightToken, SyntaxKind, TokenKind, tokenize_document_lines};
pub use zoom::{DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE, clamp_font_size, zoom_for_wheel};

/// 超过该字节数（10MB）的文本自动进入大文件只读保护。
pub const LARGE_FILE_THRESHOLD: u64 = 10 * 1024 * 1024;
/// 大文件仅加载的前缀预览字节数，保证内存与解码耗时有界。
pub const LARGE_FILE_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
/// 单行最大展示字符数，超出部分截断（超长单行如压缩 JSON 不拖垮界面）。
pub const MAX_LINE_DISPLAY_CHARS: usize = 4000;
/// 启用语法高亮的最大文本字节数；超过则只展示纯文本以保证流畅。
pub const HIGHLIGHT_MAX_BYTES: usize = 2 * 1024 * 1024;
/// 单行高亮的最大字符数；更长的行不分词。
pub const HIGHLIGHT_MAX_LINE_CHARS: usize = 2000;
/// 制表符在展示文本中展开的对齐宽度。
pub const TAB_WIDTH: usize = 4;

/// 行尾换行符类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
    Cr,
    /// 同一文件混用多种换行符
    Mixed,
    /// 文件内没有任何换行符
    None,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Cr => "CR",
            Self::Mixed => "混合换行",
            Self::None => "无换行",
        }
    }
}

/// 文本文档元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMeta {
    pub encoding: TextEncoding,
    pub has_bom: bool,
    pub line_ending: LineEnding,
    /// 文件在磁盘上的总字节数
    pub file_size: u64,
    /// 是否超过 10MB：只读保护，禁用一键编辑
    pub is_large_file_readonly: bool,
    /// 是否仅加载了文件前缀预览（大文件）
    pub is_preview_truncated: bool,
    /// 是否启用了语法高亮
    pub highlight_enabled: bool,
    pub syntax: SyntaxKind,
}

/// 展示行：已展开制表符并按需截断的文本及高亮分词。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLine {
    /// 从 1 开始的行号
    pub line_number: usize,
    pub display_text: String,
    /// 连续覆盖 `display_text` 的分词（字节区间）；未高亮时为空
    pub tokens: Vec<HighlightToken>,
    /// 该行是否因超长被截断
    pub is_truncated: bool,
}

/// 只读文本文档模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDocument {
    pub meta: TextMeta,
    pub lines: Vec<TextLine>,
    /// 展示列宽（CJK 记 2 列）的最大值，用于水平滚动区域估算
    pub max_line_cells: usize,
}

impl TextDocument {
    /// 从磁盘加载文本文档：自动检测编码、换行符，按需高亮，大文件只读预览。
    pub fn load(path: &Path) -> Result<Self, String> {
        let io_path = crate::probing::to_io_path(path);
        let file_size = std::fs::metadata(&io_path)
            .map_err(|e| format!("无法读取文件信息: {e}"))?
            .len();
        let is_large = file_size > LARGE_FILE_THRESHOLD;
        let read_limit = if is_large {
            LARGE_FILE_PREVIEW_BYTES
        } else {
            file_size
        };

        let mut bytes = Vec::with_capacity(read_limit as usize);
        File::open(&io_path)
            .map_err(|e| format!("无法打开文件: {e}"))?
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("读取文件失败: {e}"))?;

        let is_preview_truncated = is_large && (bytes.len() as u64) < file_size;
        let mut doc = Self::from_bytes(&bytes, path);
        doc.meta.file_size = file_size;
        doc.meta.is_large_file_readonly = is_large;
        doc.meta.is_preview_truncated = is_preview_truncated;
        Ok(doc)
    }

    /// 由原始字节构建文档（不涉及磁盘，便于测试）。
    pub fn from_bytes(bytes: &[u8], path: &Path) -> Self {
        let decoded = decode_bytes(bytes);
        let syntax = SyntaxKind::from_path(path);
        let (mut lines, line_ending, max_line_cells) = split_lines(&decoded.text);

        let highlight_enabled =
            decoded.text.len() <= HIGHLIGHT_MAX_BYTES && syntax != SyntaxKind::PlainText;
        if highlight_enabled {
            tokenize_document_lines(syntax, &mut lines);
        }

        Self {
            meta: TextMeta {
                encoding: decoded.encoding,
                has_bom: decoded.has_bom,
                line_ending,
                file_size: bytes.len() as u64,
                is_large_file_readonly: false,
                is_preview_truncated: false,
                highlight_enabled,
                syntax,
            },
            lines,
            max_line_cells,
        }
    }
}

fn cell_width(c: char) -> usize {
    match c as u32 {
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// 展示列宽（CJK 全角记 2 列）。
pub fn display_cells(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

/// 按 `\n` / `\r\n` / `\r` 切行，展开制表符并截断超长行。
fn split_lines(text: &str) -> (Vec<TextLine>, LineEnding, usize) {
    let bytes = text.as_bytes();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut lines = Vec::new();
    let mut max_cells = 0usize;
    let mut pos = 0usize;

    while pos < bytes.len() {
        let rel = bytes[pos..].iter().position(|&b| b == b'\n' || b == b'\r');
        let (line_end, next) = match rel {
            None => (bytes.len(), bytes.len()),
            Some(r) => {
                let idx = pos + r;
                if bytes[idx] == b'\n' {
                    lf += 1;
                    (idx, idx + 1)
                } else if bytes.get(idx + 1) == Some(&b'\n') {
                    crlf += 1;
                    (idx, idx + 2)
                } else {
                    cr += 1;
                    (idx, idx + 1)
                }
            }
        };

        let raw = &text[pos..line_end];
        let (display_text, is_truncated) = build_display_text(raw);
        max_cells = max_cells.max(display_cells(&display_text));
        lines.push(TextLine {
            line_number: lines.len() + 1,
            display_text,
            tokens: Vec::new(),
            is_truncated,
        });
        pos = next;
    }

    let kinds = [lf > 0, crlf > 0, cr > 0].iter().filter(|&&k| k).count();
    let ending = match kinds {
        0 => LineEnding::None,
        1 if lf > 0 => LineEnding::Lf,
        1 if crlf > 0 => LineEnding::Crlf,
        1 => LineEnding::Cr,
        _ => LineEnding::Mixed,
    };
    (lines, ending, max_cells)
}

fn build_display_text(raw: &str) -> (String, bool) {
    let mut out = String::with_capacity(raw.len().min(MAX_LINE_DISPLAY_CHARS + 4));
    let mut count = 0usize;
    for c in raw.chars() {
        if count >= MAX_LINE_DISPLAY_CHARS {
            out.push('…');
            return (out, true);
        }
        if c == '\t' {
            let n = TAB_WIDTH - (count % TAB_WIDTH);
            for _ in 0..n {
                out.push(' ');
            }
            count += n;
        } else if c.is_control() {
            out.push('\u{FFFD}');
            count += 1;
        } else {
            out.push(c);
            count += 1;
        }
    }
    (out, false)
}
