//! 图像解码：纯 Rust / 静态链接，运行时零额外 DLL（ADR-0005）。
//! 覆盖 PNG/APNG、JPEG、WebP、GIF（动图）、BMP、ICO、TIFF、SVG（resvg）、AVIF（rav1d 纯 Rust 分叉）。
//! 输出统一为 RGBA8（非预乘），已应用 EXIF 方向与基础 ICC -> sRGB 转换。

use std::io::{BufReader, Cursor};
use std::path::Path;
use std::time::Duration;

use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, RgbaImage};

/// 动图帧数上限与总像素字节上限，防止恶意/超长动图耗尽内存。
const MAX_FRAMES: usize = 600;
const MAX_ANIM_BYTES: usize = 384 * 1024 * 1024;
const SVG_MAX_SIDE: f32 = 4096.0;

/// 单帧
#[derive(Debug, Clone)]
pub struct Frame {
    pub rgba: RgbaImage,
    /// 该帧显示时长（静态图为 0）
    pub delay: Duration,
}

/// 解码结果
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub frames: Vec<Frame>,
    /// 格式标签（如 "PNG"、"AVIF"）
    pub format: &'static str,
}

impl DecodedImage {
    pub fn width(&self) -> u32 {
        self.frames[0].rgba.width()
    }
    pub fn height(&self) -> u32 {
        self.frames[0].rgba.height()
    }
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }
    fn single(rgba: RgbaImage, format: &'static str) -> Self {
        Self {
            frames: vec![Frame {
                rgba,
                delay: Duration::ZERO,
            }],
            format,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Io(String),
    Unsupported,
    Corrupt(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "无法读取图像文件：{e}"),
            Self::Unsupported => write!(f, "不支持的图像格式"),
            Self::Corrupt(e) => write!(f, "图像解码失败：{e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Raster(ImageFormat),
    Avif,
    Svg,
}

/// 按内容魔数嗅探格式
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" || brand == b"mif1" {
            return Some(Kind::Avif);
        }
    }
    if let Ok(fmt) = image::guess_format(bytes) {
        return Some(Kind::Raster(fmt));
    }
    let head = &bytes[..bytes.len().min(2048)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("<svg")
        || ((t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE"))
            && text.contains("<svg"))
    {
        return Some(Kind::Svg);
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Some(Kind::Svg);
    }
    None
}

pub fn decode_file(path: &Path) -> Result<DecodedImage, DecodeError> {
    let bytes = std::fs::read(path).map_err(|e| DecodeError::Io(e.to_string()))?;
    decode_bytes(&bytes)
}

pub fn decode_bytes(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    match sniff(bytes).ok_or(DecodeError::Unsupported)? {
        Kind::Svg => decode_svg(bytes),
        Kind::Avif => decode_avif(bytes),
        Kind::Raster(fmt) => decode_raster(bytes, fmt),
    }
}

fn corrupt<E: std::fmt::Display>(e: E) -> DecodeError {
    DecodeError::Corrupt(e.to_string())
}

fn format_label(fmt: ImageFormat) -> &'static str {
    match fmt {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WebP",
        ImageFormat::Bmp => "BMP",
        ImageFormat::Ico => "ICO",
        ImageFormat::Tiff => "TIFF",
        _ => "Image",
    }
}

fn decode_raster(bytes: &[u8], fmt: ImageFormat) -> Result<DecodedImage, DecodeError> {
    let label = format_label(fmt);
    let cursor = || BufReader::new(Cursor::new(bytes));
    match fmt {
        ImageFormat::Gif => {
            let dec = image::codecs::gif::GifDecoder::new(cursor()).map_err(corrupt)?;
            return collect_frames(dec.into_frames(), label);
        }
        ImageFormat::Png => {
            let dec = image::codecs::png::PngDecoder::new(cursor()).map_err(corrupt)?;
            if dec.is_apng().map_err(corrupt)? {
                let apng = dec.apng().map_err(corrupt)?;
                return collect_frames(apng.into_frames(), "APNG");
            }
        }
        ImageFormat::WebP => {
            let dec = image::codecs::webp::WebPDecoder::new(cursor()).map_err(corrupt)?;
            if dec.has_animation() {
                return collect_frames(dec.into_frames(), label);
            }
        }
        _ => {}
    }
    let mut decoder = image::ImageReader::with_format(cursor(), fmt)
        .into_decoder()
        .map_err(corrupt)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = DynamicImage::from_decoder(decoder).map_err(corrupt)?;
    img.apply_orientation(orientation);
    let mut rgba = img.into_rgba8();
    if let Some(icc) = icc {
        apply_icc_to_srgb(&mut rgba, &icc);
    }
    Ok(DecodedImage::single(rgba, label))
}

fn collect_frames(
    frames: image::Frames<'_>,
    label: &'static str,
) -> Result<DecodedImage, DecodeError> {
    let mut out: Vec<Frame> = Vec::new();
    let mut bytes = 0usize;
    for f in frames {
        let f = f.map_err(corrupt)?;
        let (n, d) = f.delay().numer_denom_ms();
        let mut ms = n.checked_div(d).unwrap_or(100);
        if ms < 20 {
            ms = 100;
        }
        let rgba = f.into_buffer();
        bytes += rgba.as_raw().len();
        out.push(Frame {
            rgba,
            delay: Duration::from_millis(u64::from(ms)),
        });
        if out.len() >= MAX_FRAMES || bytes >= MAX_ANIM_BYTES {
            break;
        }
    }
    if out.is_empty() {
        return Err(DecodeError::Corrupt("无可用帧".into()));
    }
    if out.len() == 1 {
        out[0].delay = Duration::ZERO;
    }
    Ok(DecodedImage {
        frames: out,
        format: label,
    })
}

/// 基础 ICC 处理：嵌入配置转换到 sRGB；失败则原样保留。
pub fn apply_icc_to_srgb(rgba: &mut RgbaImage, icc: &[u8]) {
    use moxcms::{ColorProfile, Layout, TransformOptions};
    let Ok(src) = ColorProfile::new_from_slice(icc) else {
        return;
    };
    let dst = ColorProfile::new_srgb();
    let Ok(transform) = src.create_transform_8bit(
        Layout::Rgba,
        &dst,
        Layout::Rgba,
        TransformOptions::default(),
    ) else {
        return;
    };
    let input = rgba.as_raw().clone();
    let _ = transform.transform(&input, rgba.as_mut());
}

fn decode_avif(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    use avif_decode::Image as A;
    let dec = avif_decode::Decoder::from_avif(bytes).map_err(corrupt)?;
    let img = dec.to_image().map_err(corrupt)?;
    let (w, h, data): (usize, usize, Vec<u8>) = match img {
        A::Rgb8(i) => (
            i.width(),
            i.height(),
            i.pixels().flat_map(|p| [p.r, p.g, p.b, 255]).collect(),
        ),
        A::Rgba8(i) => (
            i.width(),
            i.height(),
            i.pixels().flat_map(|p| [p.r, p.g, p.b, p.a]).collect(),
        ),
        A::Rgb16(i) => (
            i.width(),
            i.height(),
            i.pixels()
                .flat_map(|p| [(p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8, 255])
                .collect(),
        ),
        A::Rgba16(i) => (
            i.width(),
            i.height(),
            i.pixels()
                .flat_map(|p| {
                    [
                        (p.r >> 8) as u8,
                        (p.g >> 8) as u8,
                        (p.b >> 8) as u8,
                        (p.a >> 8) as u8,
                    ]
                })
                .collect(),
        ),
        A::Gray8(i) => (
            i.width(),
            i.height(),
            i.pixels()
                .flat_map(|p| {
                    let v = p.value();
                    [v, v, v, 255]
                })
                .collect(),
        ),
        A::Gray16(i) => (
            i.width(),
            i.height(),
            i.pixels()
                .flat_map(|p| {
                    let v = (p.value() >> 8) as u8;
                    [v, v, v, 255]
                })
                .collect(),
        ),
    };
    let rgba = RgbaImage::from_raw(w as u32, h as u32, data)
        .ok_or_else(|| corrupt("AVIF 像素缓冲尺寸不符"))?;
    Ok(DecodedImage::single(rgba, "AVIF"))
}

fn decode_svg(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    let mut opt = resvg::usvg::Options::default();
    // 仅在含文本时加载系统字体（枚举较慢）
    if bytes.windows(5).any(|w| w == b"<text") {
        opt.fontdb_mut().load_system_fonts();
    }
    let tree = resvg::usvg::Tree::from_data(bytes, &opt).map_err(corrupt)?;
    let size = tree.size();
    let (mut w, mut h) = (size.width().max(1.0), size.height().max(1.0));
    let longest = w.max(h);
    let k = if longest < 1024.0 {
        1024.0 / longest
    } else if longest > SVG_MAX_SIDE {
        SVG_MAX_SIDE / longest
    } else {
        1.0
    };
    w = (w * k).round().max(1.0);
    h = (h * k).round().max(1.0);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w as u32, h as u32)
        .ok_or_else(|| corrupt("SVG 尺寸无效"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    let mut data = pixmap.take();
    // tiny-skia 输出预乘 alpha，转回直通 alpha
    for px in data.as_chunks_mut::<4>().0.iter_mut() {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let rgba = RgbaImage::from_raw(w as u32, h as u32, data)
        .ok_or_else(|| corrupt("SVG 缓冲尺寸不符"))?;
    Ok(DecodedImage::single(rgba, "SVG"))
}

/// 读取 JPEG 内嵌 EXIF 缩略图（已按主图方向纠正）；无缩略图返回 None。仅读文件头，极快。
pub fn exif_thumbnail(path: &Path) -> Option<RgbaImage> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(256 * 1024)
        .read_to_end(&mut head)
        .ok()?;
    if !head.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(&head))
        .ok()?;
    let off = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let len = exif
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let data = exif.buf().get(off..off.checked_add(len)?)?;
    let mut img = image::load_from_memory_with_format(data, ImageFormat::Jpeg).ok()?;
    let orient = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .and_then(|v| Orientation::from_exif(v as u8))
        .unwrap_or(Orientation::NoTransforms);
    img.apply_orientation(orient);
    Some(img.into_rgba8())
}

/// 降采样：长边超过 `max_side` 时缩小，用于超大图的快速预览。
pub fn downsample(img: &RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max_side {
        return img.clone();
    }
    let k = f64::from(max_side) / f64::from(w.max(h));
    let nw = ((f64::from(w) * k) as u32).max(1);
    let nh = ((f64::from(h) * k) as u32).max(1);
    image::imageops::thumbnail(img, nw, nh)
}
