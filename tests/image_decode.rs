//! 图像解码行为：多格式、EXIF 方向、ICC、缩略图（样例全部由代码生成，不入库）。

use anyfile::image_viewer::decode::{decode_bytes, decode_file, downsample, exif_thumbnail};
use image::{
    codecs::gif::GifEncoder, codecs::jpeg::JpegEncoder, ImageEncoder, ImageFormat, Rgba, RgbaImage,
};
use std::io::Cursor;

fn gradient(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        // 左上角红色标记，用于验证方向
        if x < w / 4 && y < h / 4 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255])
        }
    })
}

fn encode(img: &RgbaImage, fmt: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let dynimg = image::DynamicImage::ImageRgba8(img.clone());
    let dynimg = if fmt == ImageFormat::Jpeg { image::DynamicImage::ImageRgb8(dynimg.to_rgb8()) } else { dynimg };
    dynimg.write_to(&mut out, fmt).unwrap();
    out.into_inner()
}

fn jpeg_with_orientation(img: &RgbaImage, orientation: u16) -> Vec<u8> {
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 90)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .unwrap();
    let mut exif = b"Exif\0\0MM\0\x2A\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
    exif.extend_from_slice(&orientation.to_be_bytes());
    exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let len = (exif.len() + 2) as u16;
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&exif);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[test]
fn decodes_still_formats() {
    let img = gradient(32, 16);
    for (fmt, label) in [
        (ImageFormat::Png, "PNG"),
        (ImageFormat::Jpeg, "JPEG"),
        (ImageFormat::WebP, "WebP"),
        (ImageFormat::Bmp, "BMP"),
        (ImageFormat::Tiff, "TIFF"),
        (ImageFormat::Ico, "ICO"),
        (ImageFormat::Gif, "GIF"),
    ] {
        let bytes = encode(&img, fmt);
        let d = decode_bytes(&bytes).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!((d.width(), d.height()), (32, 16), "{label}");
        assert_eq!(d.format, label);
    }
}

#[test]
fn decodes_animated_gif_with_delays() {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..3u8 {
            let frame = RgbaImage::from_pixel(8, 8, Rgba([i * 80, 0, 0, 255]));
            let f = image::Frame::from_parts(
                frame,
                0,
                0,
                image::Delay::from_numer_denom_ms(200, 1),
            );
            enc.encode_frame(f).unwrap();
        }
    }
    let d = decode_bytes(&out).unwrap();
    assert!(d.is_animated());
    assert_eq!(d.frames.len(), 3);
    assert_eq!(d.frames[1].delay.as_millis(), 200);
}

#[test]
fn exif_orientation_rotates_pixels() {
    let img = gradient(40, 20);
    // 方向 6：顺时针 90 度显示 -> 尺寸互换，原左上角红块移到右上角
    let d = decode_bytes(&jpeg_with_orientation(&img, 6)).unwrap();
    assert_eq!((d.width(), d.height()), (20, 40));
    let p = d.frames[0].rgba.get_pixel(18, 2);
    assert!(p[0] > 200 && p[1] < 80, "红块应位于右上角: {p:?}");
    // 方向 1：保持不变
    let d = decode_bytes(&jpeg_with_orientation(&img, 1)).unwrap();
    assert_eq!((d.width(), d.height()), (40, 20));
    let p = d.frames[0].rgba.get_pixel(2, 2);
    assert!(p[0] > 200 && p[1] < 80);
}

#[test]
fn decodes_svg() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="#ff0000"/></svg>"##;
    let d = decode_bytes(svg).unwrap();
    assert_eq!(d.format, "SVG");
    assert!(d.width() >= 100 && d.width() / d.height() == 2);
    let p = d.frames[0].rgba.get_pixel(d.width() / 2, d.height() / 2);
    assert_eq!(p.0, [255, 0, 0, 255]);
}

#[test]
fn corrupt_and_unknown_are_errors() {
    assert!(decode_bytes(b"not an image at all").is_err());
    let mut png = encode(&gradient(16, 16), ImageFormat::Png);
    png.truncate(png.len() / 2);
    assert!(decode_bytes(&png).is_err());
}

#[test]
fn decode_file_and_downsample_and_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let img = gradient(400, 200);
    let path = dir.path().join("a.jpg");
    std::fs::write(&path, jpeg_with_orientation(&img, 1)).unwrap();
    let d = decode_file(&path).unwrap();
    assert_eq!((d.width(), d.height()), (400, 200));
    let small = downsample(&d.frames[0].rgba, 100);
    assert_eq!((small.width(), small.height()), (100, 50));
    // 无内嵌缩略图的 JPEG：返回 None 而非崩溃
    assert!(exif_thumbnail(&path).is_none());
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/image").join(name)
}

#[test]
fn decodes_avif_fixture() {
    let d = decode_file(&fixture("red.avif")).unwrap();
    assert_eq!(d.format, "AVIF");
    assert_eq!((d.width(), d.height()), (64, 32));
    let p = d.frames[0].rgba.get_pixel(32, 16);
    assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "{p:?}");
}

#[test]
fn decodes_apng_fixture_as_animation() {
    let d = decode_file(&fixture("anim.apng")).unwrap();
    assert_eq!(d.format, "APNG");
    assert!(d.is_animated());
    assert_eq!((d.width(), d.height()), (32, 32));
}
