//! 编辑态保存前的文本编码：按原文件的编码 / BOM / 换行符原样写回。

use encoding_rs::GBK;

use crate::text_viewer::{LineEnding, TextEncoding};

/// 原编码无法表示的字符信息（如 GBK 文件中输入 Emoji）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrepresentable {
    /// 去重后的前若干个不可表示字符
    pub chars: Vec<char>,
    /// 首个不可表示字符所在的行（从 1 开始）
    pub first_line: usize,
}

impl Unrepresentable {
    /// 面向用户的提示文案。
    pub fn message(&self, encoding: TextEncoding) -> String {
        let sample: String = self.chars.iter().take(5).collect();
        format!(
            "第 {} 行含有 {} 无法表示的字符「{}」，是否转为 UTF-8 保存？",
            self.first_line,
            encoding.label(),
            sample
        )
    }
}

/// 把编辑缓冲区（内部统一为 `\n`）中的换行替换为目标换行符。
pub fn apply_line_ending(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Crlf => text.replace('\n', "\r\n"),
        LineEnding::Cr => text.replace('\n', "\r"),
        LineEnding::Lf | LineEnding::None | LineEnding::Mixed => text.to_string(),
    }
}

/// 把磁盘解码文本统一为 `\n` 换行（编辑缓冲区内部格式），并返回保存时应使用的换行符。
/// 混合换行取出现次数最多者；平局优先 CRLF > LF > CR。无换行视为 LF。
pub fn normalize_newlines(raw: &str) -> (String, LineEnding) {
    let bytes = raw.as_bytes();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                out.push_str(&raw[last..i]);
                out.push('\n');
                i += 2;
                last = i;
            }
            b'\r' => {
                cr += 1;
                out.push_str(&raw[last..i]);
                out.push('\n');
                i += 1;
                last = i;
            }
            b'\n' => {
                lf += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    out.push_str(&raw[last..]);
    let ending = if crlf == 0 && lf == 0 && cr == 0 {
        LineEnding::Lf
    } else if crlf >= lf && crlf >= cr {
        LineEnding::Crlf
    } else if lf >= cr {
        LineEnding::Lf
    } else {
        LineEnding::Cr
    };
    (out, ending)
}

/// 按指定编码 / BOM / 换行符编码为待写入字节。
/// 原编码无法表示的字符返回 `Err(Unrepresentable)`，绝不静默替换。
pub fn encode_text(
    text: &str,
    encoding: TextEncoding,
    has_bom: bool,
    ending: LineEnding,
) -> Result<Vec<u8>, Unrepresentable> {
    let body = apply_line_ending(text, ending);
    match encoding {
        TextEncoding::Utf8 => Ok(body.into_bytes()),
        TextEncoding::Utf8Bom => {
            let mut out = vec![0xEF, 0xBB, 0xBF];
            out.extend_from_slice(body.as_bytes());
            Ok(out)
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let le = encoding == TextEncoding::Utf16Le;
            let mut out = Vec::with_capacity(body.len() * 2 + 2);
            if has_bom {
                out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
            }
            for unit in body.encode_utf16() {
                let b = if le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                out.extend_from_slice(&b);
            }
            Ok(out)
        }
        TextEncoding::Gbk => {
            let (bytes, _, had_unmappable) = GBK.encode(&body);
            if had_unmappable {
                return Err(find_unrepresentable(&body));
            }
            Ok(bytes.into_owned())
        }
    }
}

fn find_unrepresentable(body: &str) -> Unrepresentable {
    let mut chars = Vec::new();
    let mut first_line = 0usize;
    let mut line = 1usize;
    let mut buf = [0u8; 4];
    for c in body.chars() {
        if c == '\n' {
            line += 1;
            continue;
        }
        let (_, _, bad) = GBK.encode(c.encode_utf8(&mut buf));
        if bad {
            if first_line == 0 {
                first_line = line;
            }
            if !chars.contains(&c) && chars.len() < 16 {
                chars.push(c);
            }
        }
    }
    Unrepresentable {
        chars,
        first_line: first_line.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_detects_dominant_ending() {
        let (t, e) = normalize_newlines("a\r\nb\r\nc\n");
        assert_eq!(t, "a\nb\nc\n");
        assert_eq!(e, LineEnding::Crlf);
        let (_, e) = normalize_newlines("single");
        assert_eq!(e, LineEnding::Lf);
        let (t, e) = normalize_newlines("a\rb\rc");
        assert_eq!((t.as_str(), e), ("a\nb\nc", LineEnding::Cr));
    }

    #[test]
    fn gbk_roundtrip_and_emoji_rejected() {
        let b = encode_text("中文\n", TextEncoding::Gbk, false, LineEnding::Crlf).unwrap();
        assert_eq!(b, [0xD6, 0xD0, 0xCE, 0xC4, 0x0D, 0x0A]);
        let err = encode_text("ok\n中😀\n", TextEncoding::Gbk, false, LineEnding::Lf).unwrap_err();
        assert_eq!(err.chars, vec!['😀']);
        assert_eq!(err.first_line, 2);
    }

    #[test]
    fn utf16_bom_and_utf8_bom() {
        let b = encode_text("A", TextEncoding::Utf16Le, true, LineEnding::Lf).unwrap();
        assert_eq!(b, [0xFF, 0xFE, 0x41, 0x00]);
        let b = encode_text("A", TextEncoding::Utf16Be, false, LineEnding::Lf).unwrap();
        assert_eq!(b, [0x00, 0x41]);
        let b = encode_text("A", TextEncoding::Utf8Bom, true, LineEnding::Lf).unwrap();
        assert_eq!(b, [0xEF, 0xBB, 0xBF, 0x41]);
    }
}
