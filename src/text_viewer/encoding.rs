//! 文本编码自动检测与解码：UTF-8（含/不含 BOM）、GBK、UTF-16 LE/BE。

use encoding_rs::{GBK, UTF_16BE, UTF_16LE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Gbk,
    Utf16Le,
    Utf16Be,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Gbk => "GBK",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
        }
    }
}

/// 解码结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    pub encoding: TextEncoding,
    pub has_bom: bool,
    pub text: String,
}

/// 检测编码，返回 `(编码, BOM 字节数)`。仅需前缀样本即可。
pub fn detect_encoding(bytes: &[u8]) -> (TextEncoding, usize) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (TextEncoding::Utf8Bom, 3);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return (TextEncoding::Utf16Le, 2);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return (TextEncoding::Utf16Be, 2);
    }

    let sample = &bytes[..bytes.len().min(64 * 1024)];
    if let Some(enc) = guess_utf16_without_bom(sample) {
        return (enc, 0);
    }
    if is_valid_utf8_allow_truncated_tail(sample) {
        return (TextEncoding::Utf8, 0);
    }
    (TextEncoding::Gbk, 0)
}

/// 无 BOM 的 UTF-16：ASCII 文本会在奇数（LE）或偶数（BE）字节位交错出现 0x00。
fn guess_utf16_without_bom(sample: &[u8]) -> Option<TextEncoding> {
    if sample.len() < 4 {
        return None;
    }
    let pairs = sample.len() / 2;
    let (mut even_zero, mut odd_zero) = (0usize, 0usize);
    for chunk in sample.as_chunks::<2>().0 {
        if chunk[0] == 0 {
            even_zero += 1;
        }
        if chunk[1] == 0 {
            odd_zero += 1;
        }
    }
    let threshold = pairs / 4;
    if odd_zero > threshold && odd_zero > even_zero * 4 {
        Some(TextEncoding::Utf16Le)
    } else if even_zero > threshold && even_zero > odd_zero * 4 {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

fn is_valid_utf8_allow_truncated_tail(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        // error_len 为 None 表示仅末尾多字节序列被样本截断
        Err(e) => e.error_len().is_none(),
    }
}

/// 自动检测并解码整段字节为字符串（非法序列以 U+FFFD 替换，不会失败）。
pub fn decode_bytes(bytes: &[u8]) -> DecodedText {
    let (encoding, bom_len) = detect_encoding(bytes);
    let body = &bytes[bom_len..];
    let text = match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => String::from_utf8_lossy(body).into_owned(),
        TextEncoding::Gbk => GBK.decode_without_bom_handling(body).0.into_owned(),
        TextEncoding::Utf16Le => UTF_16LE.decode_without_bom_handling(body).0.into_owned(),
        TextEncoding::Utf16Be => UTF_16BE.decode_without_bom_handling(body).0.into_owned(),
    };
    DecodedText {
        encoding,
        has_bom: bom_len > 0,
        text,
    }
}
