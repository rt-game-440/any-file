//! 第二层文件探测引擎（遵循 ADR-0006）：
//! 仅对当前打开的文件项执行 Magic Bytes、基础容器结构校验、音视频流懒探测与文本编码启发式，
//! 正确处理无扩展名文本、改错后缀媒体、`.ts`/`.svg` 歧义格式、损坏文件与纯二进制兜底，
//! 并在 Windows 上无缝支持超长路径（>260 字符）与 UNC / Unicode 路径。

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::domain::{FallbackInfo, FallbackReason, MediaCategory, ViewerKind};
use crate::ports::VideoBackend;

/// 懒探测读取上限（仅读头部 8KB，确保超大文件也瞬时完成）
const SNIFF_READ_LIMIT: usize = 8192;

/// 单个文件项的内容探测结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// 探测出的媒体大类；纯二进制或损坏文件为 `None`
    pub media_category: Option<MediaCategory>,
    /// 识别出的具体格式标签（用于界面状态展示）
    pub detected_format: String,
    /// 允许手动切换的查看器列表（首项通常为推荐查看器）
    pub available_viewers: Vec<ViewerKind>,
    /// 当无法正常预览时的兜底信息卡数据
    pub fallback: Option<FallbackInfo>,
}

impl ProbeResult {
    fn supported(
        category: MediaCategory,
        detected_format: impl Into<String>,
        available_viewers: Vec<ViewerKind>,
    ) -> Self {
        Self {
            media_category: Some(category),
            detected_format: detected_format.into(),
            available_viewers,
            fallback: None,
        }
    }

    fn fallback(
        path: &Path,
        file_size: u64,
        reason: FallbackReason,
        reason_message: impl Into<String>,
        detected_format: impl Into<String>,
    ) -> Self {
        let msg = reason_message.into();
        Self {
            media_category: None,
            detected_format: detected_format.into(),
            available_viewers: vec![ViewerKind::Unsupported],
            fallback: Some(FallbackInfo {
                path: path.to_path_buf(),
                file_size,
                reason,
                reason_message: msg,
            }),
        }
    }
}

/// 将路径转换为支持 Windows 超长路径（>260 字符）的 I/O 路径。
/// 对外展示的 `FileEntry.path` 仍保留原样，仅在系统文件 I/O 时调用。
pub fn to_io_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if raw.starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        // 超过 240 字符的绝对路径或 UNC 路径自动转换为扩展长度路径
        if raw.len() >= 240 && path.is_absolute() {
            if let Some(unc_rest) = raw.strip_prefix(r"\\") {
                return PathBuf::from(format!(r"\\?\UNC\{unc_rest}"));
            }
            return PathBuf::from(format!(r"\\?\{raw}"));
        }
        path.to_path_buf()
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// 判断路径是否存在（兼容 Windows 超长路径）
pub fn path_exists(path: &Path) -> bool {
    std::fs::metadata(to_io_path(path)).is_ok()
}

/// 仅按扩展名快速粗分媒体大类（供第一层同目录序列索引使用，绝不读取文件内容，遵循 ADR-0006）。
pub fn classify_by_extension(path: &Path) -> Option<MediaCategory> {
    let ext = lowercase_extension(path)?;
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "cur" | "tif" | "tiff"
        | "svg" | "avif" | "apng" => Some(MediaCategory::Image),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "m4v" | "3gp" => {
            Some(MediaCategory::Video)
        }
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" => {
            Some(MediaCategory::Audio)
        }
        "txt" | "md" | "markdown" | "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "cfg"
        | "conf" | "log" | "csv" | "tsv" | "rs" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx"
        | "py" | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "cs" | "java" | "go" | "rb"
        | "php" | "swift" | "kt" | "html" | "htm" | "css" | "scss" | "less" | "sql" | "sh"
        | "bash" | "zsh" | "ps1" | "bat" | "cmd" | "env" | "gitignore" | "slint" => {
            Some(MediaCategory::Text)
        }
        _ => None,
    }
}

/// 对当前打开的文件项执行内容特征（Magic Bytes + 流信息 + 文本编码启发式）懒探测。
pub fn probe_file(path: &Path, video_backend: &dyn VideoBackend) -> ProbeResult {
    let io_path = to_io_path(path);
    let metadata = match std::fs::metadata(&io_path) {
        Ok(m) => m,
        Err(err) => {
            return ProbeResult::fallback(
                path,
                0,
                FallbackReason::Unreadable(err.to_string()),
                format!("无法读取文件元数据：{err}"),
                "不可读",
            );
        }
    };

    if metadata.is_dir() {
        return ProbeResult::fallback(
            path,
            0,
            FallbackReason::UnsupportedFormat("目标路径是目录而非文件".to_string()),
            "目标路径是一个文件夹，无法作为单个文件预览。",
            "目录",
        );
    }

    let file_size = metadata.len();
    let mut file = match File::open(&io_path) {
        Ok(f) => f,
        Err(err) => {
            return ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::Unreadable(err.to_string()),
                format!("无法打开文件（可能缺少权限或被占用）：{err}"),
                "不可读",
            );
        }
    };

    let mut header = vec![0u8; (file_size as usize).min(SNIFF_READ_LIMIT)];
    let read_len = match file.read(&mut header) {
        Ok(n) => n,
        Err(err) => {
            return ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::Unreadable(err.to_string()),
                format!("读取文件内容失败：{err}"),
                "不可读",
            );
        }
    };
    header.truncate(read_len);

    let ext = lowercase_extension(path);

    // 1. Magic Bytes 与二进制容器结构探测（优先于扩展名，可纠正改错后缀与识别损坏头）
    if let Some(res) = probe_magic_bytes(path, &header, file_size, ext.as_deref(), video_backend) {
        return res;
    }

    // 2. SVG 矢量图探测（基于 XML 文本特征，默认 Image，同时允许手动切到 Text）
    if looks_like_svg(&header) {
        return ProbeResult::supported(
            MediaCategory::Image,
            "SVG",
            vec![ViewerKind::Image, ViewerKind::Text],
        );
    }

    // 3. MPEG-TS 传输流探测（区分 .ts 视频流与 TypeScript 源码）
    if looks_like_mpeg_ts(&header) {
        return ProbeResult::supported(
            MediaCategory::Video,
            "MPEG-TS",
            vec![ViewerKind::Video, ViewerKind::Text],
        );
    }

    // 若扩展名为 .ts 且视频后端探测到视频流，按视频流处理
    if ext.as_deref() == Some("ts")
        && let Some(info) = video_backend.probe(&io_path)
        && info.has_video_stream
    {
        return ProbeResult::supported(
            MediaCategory::Video,
            "MPEG-TS",
            vec![ViewerKind::Video, ViewerKind::Text],
        );
    }

    // 4. 文本与多编码启发式探测（UTF-8 / UTF-8 BOM / UTF-16 LE & BE / GBK）
    if let Some(encoding_label) = detect_text_encoding(&header, ext.as_deref()) {
        let mut viewers = vec![ViewerKind::Text];
        let format_label = if ext.as_deref() == Some("ts") {
            viewers.push(ViewerKind::Video);
            format!("TypeScript（{encoding_label}）")
        } else {
            encoding_label.to_string()
        };
        return ProbeResult::supported(MediaCategory::Text, format_label, viewers);
    }

    // 5. 兜底判定：若扩展名声称为已知二进制媒体格式却未能通过 Magic Bytes 识别，归为损坏文件
    if let Some(ext_str) = ext.as_deref()
        && is_known_binary_media_extension(ext_str)
    {
        return ProbeResult::fallback(
            path,
            file_size,
            FallbackReason::CorruptedFile(format!(
                "文件扩展名为 .{ext_str}，但文件头特征缺失或已损坏"
            )),
            format!("文件扩展名为 .{ext_str}，但文件头无效或内容已损坏，无法安全解码。"),
            format!("损坏的 .{}", ext_str.to_uppercase()),
        );
    }

    ProbeResult::fallback(
        path,
        file_size,
        FallbackReason::PureBinary,
        "检测到纯二进制或未知格式文件，为避免乱码已展示安全信息卡。",
        "纯二进制",
    )
}

fn lowercase_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
}

fn is_known_binary_media_extension(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "avif"
            | "apng"
            | "mp4"
            | "mkv"
            | "webm"
            | "mov"
            | "avi"
            | "wmv"
            | "flv"
            | "mp3"
            | "wav"
            | "flac"
            | "ogg"
            | "m4a"
            | "aac"
            | "opus"
    )
}

fn probe_magic_bytes(
    path: &Path,
    bytes: &[u8],
    file_size: u64,
    _ext: Option<&str>,
    video_backend: &dyn VideoBackend,
) -> Option<ProbeResult> {
    let io_path = to_io_path(path);

    // PNG / APNG: 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        // 合法 PNG 至少需要 8(sig) + 4(len) + 4("IHDR") + 13(IHDR data) + 4(CRC) = 33 字节
        let valid_ihdr = bytes.len() >= 33
            && bytes[8..12] == [0x00, 0x00, 0x00, 0x0D]
            && &bytes[12..16] == b"IHDR";
        if !valid_ihdr {
            return Some(ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::CorruptedFile("PNG 文件头缺少合法的 IHDR 块".to_string()),
                "PNG 文件头损坏或数据不完整，无法解码。",
                "损坏的 PNG",
            ));
        }
        return Some(ProbeResult::supported(
            MediaCategory::Image,
            "PNG",
            vec![ViewerKind::Image],
        ));
    }

    // JPEG: FF D8 FF
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        if bytes.len() < 4 || bytes[3] < 0xC0 {
            return Some(ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::CorruptedFile("JPEG 标记段无效或被截断".to_string()),
                "JPEG 文件头损坏或数据被截断，无法解码。",
                "损坏的 JPEG",
            ));
        }
        return Some(ProbeResult::supported(
            MediaCategory::Image,
            "JPEG",
            vec![ViewerKind::Image],
        ));
    }

    // GIF: GIF87a / GIF89a
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        if bytes.len() < 13 {
            return Some(ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::CorruptedFile("GIF 逻辑屏幕描述符不完整".to_string()),
                "GIF 文件头损坏或数据不完整，无法解码。",
                "损坏的 GIF",
            ));
        }
        return Some(ProbeResult::supported(
            MediaCategory::Image,
            "GIF",
            vec![ViewerKind::Image],
        ));
    }

    // BMP: "BM"
    if bytes.starts_with(b"BM") {
        let valid_bmp = bytes.len() >= 26
            && u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]) >= 12;
        if !valid_bmp {
            return Some(ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::CorruptedFile("BMP DIB 头损坏或长度不足".to_string()),
                "BMP 文件头损坏或数据不完整，无法解码。",
                "损坏的 BMP",
            ));
        }
        return Some(ProbeResult::supported(
            MediaCategory::Image,
            "BMP",
            vec![ViewerKind::Image],
        ));
    }

    // RIFF 容器：WebP / WAV / AVI
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" {
        match &bytes[8..12] {
            b"WEBP" => {
                if bytes.len() < 16 {
                    return Some(ProbeResult::fallback(
                        path,
                        file_size,
                        FallbackReason::CorruptedFile("WebP 容器数据不完整".to_string()),
                        "WebP 文件头损坏或被截断，无法解码。",
                        "损坏的 WebP",
                    ));
                }
                return Some(ProbeResult::supported(
                    MediaCategory::Image,
                    "WebP",
                    vec![ViewerKind::Image],
                ));
            }
            b"WAVE" => {
                if bytes.len() < 36 || !bytes.windows(4).any(|w| w == b"fmt ") {
                    return Some(ProbeResult::fallback(
                        path,
                        file_size,
                        FallbackReason::CorruptedFile("WAV 缺少 fmt 块".to_string()),
                        "WAV 音频头损坏或不完整，无法播放。",
                        "损坏的 WAV",
                    ));
                }
                return Some(ProbeResult::supported(
                    MediaCategory::Audio,
                    "WAV",
                    vec![ViewerKind::Audio],
                ));
            }
            b"AVI " => {
                return Some(ProbeResult::supported(
                    MediaCategory::Video,
                    "AVI",
                    vec![ViewerKind::Video],
                ));
            }
            _ => {}
        }
    }

    // TIFF: II*\0 / MM\0*
    if bytes.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || bytes.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
    {
        if bytes.len() < 8 {
            return Some(ProbeResult::fallback(
                path,
                file_size,
                FallbackReason::CorruptedFile("TIFF 文件头不完整".to_string()),
                "TIFF 文件头损坏或被截断，无法解码。",
                "损坏的 TIFF",
            ));
        }
        return Some(ProbeResult::supported(
            MediaCategory::Image,
            "TIFF",
            vec![ViewerKind::Image],
        ));
    }

    // ICO / CUR: 00 00 01 00 或 00 00 02 00
    if bytes.len() >= 22
        && (bytes.starts_with(&[0x00, 0x00, 0x01, 0x00])
            || bytes.starts_with(&[0x00, 0x00, 0x02, 0x00]))
        && u16::from_le_bytes([bytes[4], bytes[5]]) > 0
    {
        return Some(ProbeResult::supported(
            MediaCategory::Image,
            "ICO",
            vec![ViewerKind::Image],
        ));
    }

    // ISO Base Media File Format (`ftyp` box): AVIF / M4A / MP4 / MOV
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let major_brand = &bytes[8..12];
        if matches!(major_brand, b"avif" | b"avis") {
            return Some(ProbeResult::supported(
                MediaCategory::Image,
                "AVIF",
                vec![ViewerKind::Image],
            ));
        }
        if matches!(major_brand, b"M4A " | b"M4B " | b"M4P ") {
            return Some(ProbeResult::supported(
                MediaCategory::Audio,
                "M4A",
                vec![ViewerKind::Audio, ViewerKind::Video],
            ));
        }
        // 使用 VideoBackend 进行流懒探测以区分纯音频 MP4 与视频 MP4
        if let Some(info) = video_backend.probe(&io_path)
            && !info.has_video_stream
            && info.has_audio_stream
        {
            return Some(ProbeResult::supported(
                MediaCategory::Audio,
                "MP4（纯音频）",
                vec![ViewerKind::Audio, ViewerKind::Video],
            ));
        }
        let label = if major_brand == b"qt  " {
            "MOV"
        } else {
            "MP4"
        };
        return Some(ProbeResult::supported(
            MediaCategory::Video,
            label,
            vec![ViewerKind::Video, ViewerKind::Audio],
        ));
    }

    // EBML 容器：MKV / WebM
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        if let Some(info) = video_backend.probe(&io_path)
            && !info.has_video_stream
            && info.has_audio_stream
        {
            return Some(ProbeResult::supported(
                MediaCategory::Audio,
                "Matroska（纯音频）",
                vec![ViewerKind::Audio, ViewerKind::Video],
            ));
        }
        let label = if bytes.windows(4).any(|w| w == b"webm") {
            "WebM"
        } else {
            "MKV"
        };
        return Some(ProbeResult::supported(
            MediaCategory::Video,
            label,
            vec![ViewerKind::Video, ViewerKind::Audio],
        ));
    }

    // FLV
    if bytes.starts_with(b"FLV\x01") {
        return Some(ProbeResult::supported(
            MediaCategory::Video,
            "FLV",
            vec![ViewerKind::Video],
        ));
    }

    // ASF / WMV / WMA
    const ASF_GUID: [u8; 16] = [
        0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE,
        0x6C,
    ];
    if bytes.starts_with(&ASF_GUID) {
        return Some(ProbeResult::supported(
            MediaCategory::Video,
            "WMV",
            vec![ViewerKind::Video],
        ));
    }

    // FLAC
    if bytes.starts_with(b"fLaC") {
        return Some(ProbeResult::supported(
            MediaCategory::Audio,
            "FLAC",
            vec![ViewerKind::Audio],
        ));
    }

    // OGG / OPUS
    if bytes.starts_with(b"OggS") {
        if let Some(info) = video_backend.probe(&io_path)
            && info.has_video_stream
        {
            return Some(ProbeResult::supported(
                MediaCategory::Video,
                "OGG 视频",
                vec![ViewerKind::Video, ViewerKind::Audio],
            ));
        }
        let label = if bytes.windows(8).any(|w| w == b"OpusHead") {
            "OPUS"
        } else {
            "OGG"
        };
        return Some(ProbeResult::supported(
            MediaCategory::Audio,
            label,
            vec![ViewerKind::Audio],
        ));
    }

    // MP3 (ID3v2 标签或严格 MPEG Audio Layer III Frame Sync，避免与 UTF-16 LE BOM `FF FE` 冲突)
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 4
            && bytes[0] == 0xFF
            && bytes[1] != 0xFE
            && (bytes[1] & 0xE0) == 0xE0
            && (bytes[1] & 0x18) != 0x08
            && (bytes[1] & 0x06) == 0x02
            && (bytes[2] & 0xF0) != 0xF0
            && (bytes[2] & 0x0C) != 0x0C)
    {
        return Some(ProbeResult::supported(
            MediaCategory::Audio,
            "MP3",
            vec![ViewerKind::Audio],
        ));
    }

    None
}

/// 判断字节片段是否为 SVG 矢量图内容
fn looks_like_svg(bytes: &[u8]) -> bool {
    let trimmed = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(trim_incomplete_utf8_tail(trimmed)) else {
        return false;
    };
    let head = text.trim_start();
    if !(head.starts_with("<svg")
        || head.starts_with("<SVG")
        || head.starts_with("<?xml")
        || head.starts_with("<!DOCTYPE svg")
        || head.starts_with("<!--"))
    {
        return false;
    }
    let lower = head[..head.len().min(4096)].to_ascii_lowercase();
    lower.contains("<svg")
}

/// 判断字节片段是否为 MPEG-TS 传输流（188 字节包对齐，每包首字节为 0x47）
fn looks_like_mpeg_ts(bytes: &[u8]) -> bool {
    const PACKET_SIZE: usize = 188;
    if bytes.len() < PACKET_SIZE * 2 {
        return false;
    }
    let check_packets = (bytes.len() / PACKET_SIZE).min(5);
    if check_packets < 2 {
        return false;
    }
    (0..check_packets).all(|idx| bytes[idx * PACKET_SIZE] == 0x47)
}

/// 文本编码启发式检测（支持 UTF-8 / UTF-8 BOM / UTF-16 LE & BE / GBK）
fn detect_text_encoding(bytes: &[u8], ext: Option<&str>) -> Option<&'static str> {
    if bytes.is_empty() {
        // 空的明确二进制扩展名按兜底处理，其他空文件视为空文本
        if ext.is_some_and(|e| matches!(e, "bin" | "exe" | "dll" | "so" | "dat")) {
            return None;
        }
        return Some("UTF-8 文本");
    }

    // 1. 显式 BOM 检测
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let rest = trim_incomplete_utf8_tail(&bytes[3..]);
        if std::str::from_utf8(rest).is_ok() && !has_excessive_binary_controls(rest) {
            return Some("UTF-8 BOM 文本");
        }
    }
    if bytes.starts_with(&[0xFF, 0xFE]) && bytes.len() >= 4 {
        return Some("UTF-16 LE 文本");
    }
    if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() >= 4 {
        return Some("UTF-16 BE 文本");
    }

    // 2. 无 BOM UTF-16 LE / BE 启发式（常见 ASCII 文本每 2 字节交错出现 0x00）
    if bytes.len() >= 8 && bytes.contains(&0x00) {
        let pairs = bytes.len() / 2;
        let even_zeros = (0..pairs).filter(|&i| bytes[i * 2] == 0).count();
        let odd_zeros = (0..pairs).filter(|&i| bytes[i * 2 + 1] == 0).count();
        if odd_zeros * 10 >= pairs * 7 && even_zeros == 0 {
            return Some("UTF-16 LE 文本");
        }
        if even_zeros * 10 >= pairs * 7 && odd_zeros == 0 {
            return Some("UTF-16 BE 文本");
        }
        // 包含 NUL 字节且不符合 UTF-16 规律，直接判定非文本
        return None;
    }

    if bytes.contains(&0x00) {
        return None;
    }

    // 3. UTF-8 合法校验
    let utf8_slice = trim_incomplete_utf8_tail(bytes);
    if std::str::from_utf8(utf8_slice).is_ok() && !has_excessive_binary_controls(utf8_slice) {
        return Some("UTF-8 文本");
    }

    // 4. GBK / GB18030 双字节中文启发式
    if looks_like_gbk_text(bytes) && !has_excessive_binary_controls(bytes) {
        return Some("GBK 文本");
    }

    None
}

/// 裁剪因 8KB 读缓冲截断在末尾的不完整 UTF-8 多字节序列
fn trim_incomplete_utf8_tail(bytes: &[u8]) -> &[u8] {
    if bytes.is_empty() {
        return bytes;
    }
    let max_back = bytes.len().min(4);
    for trim in 0..max_back {
        let candidate = &bytes[..bytes.len() - trim];
        if std::str::from_utf8(candidate).is_ok() {
            return candidate;
        }
    }
    bytes
}

/// 检查是否存在大量非文本控制字符
fn has_excessive_binary_controls(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let bad_controls = bytes
        .iter()
        .filter(|&&b| {
            b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B) || b == 0x7F
        })
        .count();
    bad_controls * 100 > bytes.len() * 5
}

/// GBK 双字节编码合法性启发式
fn looks_like_gbk_text(bytes: &[u8]) -> bool {
    let mut i = 0;
    let mut non_ascii_pairs = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            i += 1;
            continue;
        }
        // GBK 首字节范围：0x81..=0xFE
        if !(0x81..=0xFE).contains(&b) {
            return false;
        }
        if i + 1 >= bytes.len() {
            // 末尾截断单个字节，在读满缓冲区时允许
            break;
        }
        let b2 = bytes[i + 1];
        // GBK 次字节范围：0x40..=0xFE 且 != 0x7F
        if !(0x40..=0xFE).contains(&b2) || b2 == 0x7F {
            return false;
        }
        non_ascii_pairs += 1;
        i += 2;
    }
    non_ascii_pairs > 0
}
