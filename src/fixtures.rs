//! 样例文件语料库（Fixture Corpus）生成器。
//! 用于自动生成覆盖 4 大媒体大类、歧义文件、多编码/换行文本、无扩展名、错后缀、损坏文件与纯二进制兜底的真实测试文件，
//! 供工单 #2 及后续所有工单的集成测试与回归验收复用。

use std::fs;
use std::path::{Path, PathBuf};

/// 生成的样例语料库目录清单
#[derive(Debug, Clone)]
pub struct FixtureCorpus {
    pub root: PathBuf,
    // Image 图像样本
    pub png_valid: PathBuf,
    pub bmp_valid: PathBuf,
    pub svg_ambiguous: PathBuf,
    pub corrupted_png: PathBuf,
    pub misnamed_png_as_txt: PathBuf,
    // Text 文本与代码样本
    pub text_utf8_lf: PathBuf,
    pub text_utf8_bom_crlf: PathBuf,
    pub text_gbk: PathBuf,
    pub text_utf16_le: PathBuf,
    pub text_no_extension_makefile: PathBuf,
    pub ts_ambiguous: PathBuf,
    // Audio 音频样本
    pub wav_valid: PathBuf,
    // Video 视频样本（最小 MP4 容器头/可由 ffmpeg 生成）
    pub mp4_minimal: PathBuf,
    // Unsupported 纯二进制兜底样本
    pub binary_unknown: PathBuf,
}

impl FixtureCorpus {
    /// 在指定目录中确定性地生成全套样例文件语料库
    pub fn generate_in(root: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(root)?;

        let png_valid = root.join("sample_1x1.png");
        fs::write(&png_valid, minimal_valid_png_bytes())?;

        let bmp_valid = root.join("sample_1x1.bmp");
        fs::write(&bmp_valid, minimal_valid_bmp_bytes())?;

        let svg_ambiguous = root.join("vector_logo.svg");
        fs::write(
            &svg_ambiguous,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#2563EB"/></svg>"##,
        )?;

        let corrupted_png = root.join("corrupted.png");
        fs::write(&corrupted_png, b"\x89PNG\r\n\x1a\nCORRUPTED_DATA")?;

        let misnamed_png_as_txt = root.join("actually_image.txt");
        fs::write(&misnamed_png_as_txt, minimal_valid_png_bytes())?;

        let text_utf8_lf = root.join("notes_utf8_lf.txt");
        fs::write(&text_utf8_lf, "AnyFile UTF-8 LF 测试\n第二行内容\n")?;

        let text_utf8_bom_crlf = root.join("notes_utf8_bom_crlf.txt");
        let mut bom_bytes = vec![0xEF, 0xBB, 0xBF];
        bom_bytes.extend_from_slice("AnyFile UTF-8 BOM CRLF 测试\r\n第二行\r\n".as_bytes());
        fs::write(&text_utf8_bom_crlf, bom_bytes)?;

        // GBK 编码的“你好世界\r\n”字节序列（C4 E3 BA C3 CA C0 BD E7 0D 0A）
        let text_gbk = root.join("legacy_gbk.txt");
        fs::write(
            &text_gbk,
            [0xC4, 0xE3, 0xBA, 0xC3, 0xCA, 0xC0, 0xBD, 0xE7, 0x0D, 0x0A],
        )?;

        // UTF-16 LE BOM + "AnyFile\r\n"
        let text_utf16_le = root.join("notes_utf16le.txt");
        let mut utf16_bytes = vec![0xFF, 0xFE];
        for unit in "AnyFile\r\n".encode_utf16() {
            utf16_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&text_utf16_le, utf16_bytes)?;

        let text_no_extension_makefile = root.join("Makefile");
        fs::write(&text_no_extension_makefile, "all:\n\techo AnyFile\n")?;

        // 歧义扩展名 .ts（TypeScript 源码文本，需与 MPEG-TS 视频流区分）
        let ts_ambiguous = root.join("module.ts");
        fs::write(
            &ts_ambiguous,
            "export function greet(name: string): string {\n  return `Hello, ${name}`;\n}\n",
        )?;

        let wav_valid = root.join("tone_440hz.wav");
        fs::write(&wav_valid, minimal_valid_wav_bytes())?;

        let mp4_minimal = root.join("sample_box.mp4");
        fs::write(&mp4_minimal, minimal_mp4_ftyp_bytes())?;

        let binary_unknown = root.join("opaque.bin");
        fs::write(
            &binary_unknown,
            [0x00, 0x7F, 0xFE, 0xED, 0x00, 0x01, 0x02, 0xFF, 0x00, 0x99],
        )?;

        Ok(Self {
            root: root.to_path_buf(),
            png_valid,
            bmp_valid,
            svg_ambiguous,
            corrupted_png,
            misnamed_png_as_txt,
            text_utf8_lf,
            text_utf8_bom_crlf,
            text_gbk,
            text_utf16_le,
            text_no_extension_makefile,
            ts_ambiguous,
            wav_valid,
            mp4_minimal,
            binary_unknown,
        })
    }

    /// 返回语料库中全部样例文件路径
    pub fn all_paths(&self) -> Vec<&Path> {
        vec![
            &self.png_valid,
            &self.bmp_valid,
            &self.svg_ambiguous,
            &self.corrupted_png,
            &self.misnamed_png_as_txt,
            &self.text_utf8_lf,
            &self.text_utf8_bom_crlf,
            &self.text_gbk,
            &self.text_utf16_le,
            &self.text_no_extension_makefile,
            &self.ts_ambiguous,
            &self.wav_valid,
            &self.mp4_minimal,
            &self.binary_unknown,
        ]
    }
}

/// 标准合法 1x1 蓝色像素 PNG 字节流
pub fn minimal_valid_png_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, // IHDR length
        0x49, 0x48, 0x44, 0x52, // IHDR
        0x00, 0x00, 0x00, 0x01, // width = 1
        0x00, 0x00, 0x00, 0x01, // height = 1
        0x08, 0x02, 0x00, 0x00, 0x00, // 8-bit RGB
        0x90, 0x77, 0x53, 0xDE, // IHDR CRC
        0x00, 0x00, 0x00, 0x0C, // IDAT length
        0x49, 0x44, 0x41, 0x54, // IDAT
        0x08, 0xD7, 0x63, 0x60, 0x60, 0xF8, 0x0F, 0x00, 0x01, 0x04, 0x01, 0x00, // zlib data
        0x1B, 0x0C, 0x6C, 0x7D, // IDAT CRC
        0x00, 0x00, 0x00, 0x00, // IEND length
        0x49, 0x45, 0x4E, 0x44, // IEND
        0xAE, 0x42, 0x60, 0x82, // IEND CRC
    ]
}

/// 标准合法 1x1 24-bit BMP 字节流
pub fn minimal_valid_bmp_bytes() -> Vec<u8> {
    vec![
        0x42, 0x4D, // "BM"
        0x3A, 0x00, 0x00, 0x00, // file size = 58
        0x00, 0x00, 0x00, 0x00, // reserved
        0x36, 0x00, 0x00, 0x00, // pixel data offset = 54
        0x28, 0x00, 0x00, 0x00, // DIB header size = 40
        0x01, 0x00, 0x00, 0x00, // width = 1
        0x01, 0x00, 0x00, 0x00, // height = 1
        0x01, 0x00, // planes = 1
        0x18, 0x00, // 24 bpp
        0x00, 0x00, 0x00, 0x00, // BI_RGB
        0x04, 0x00, 0x00, 0x00, // image size = 4
        0x13, 0x0B, 0x00, 0x00, // X ppm
        0x13, 0x0B, 0x00, 0x00, // Y ppm
        0x00, 0x00, 0x00, 0x00, // colors
        0x00, 0x00, 0x00, 0x00, // important colors
        0xEB, 0x63, 0x25, 0x00, // 1 pixel BGR + padding
    ]
}

/// 标准合法单声道 8kHz PCM WAV 音频字节流（含 800 采样静音/短脉冲，约 0.1s）
pub fn minimal_valid_wav_bytes() -> Vec<u8> {
    let sample_rate: u32 = 8000;
    let num_samples: u32 = 800;
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let data_size: u32 = num_samples * u32::from(num_channels) * u32::from(bits_per_sample / 8);
    let chunk_size: u32 = 36 + data_size;
    let byte_rate: u32 = sample_rate * u32::from(num_channels) * u32::from(bits_per_sample / 8);
    let block_align: u16 = num_channels * (bits_per_sample / 8);

    let mut bytes = Vec::with_capacity((44 + data_size) as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&chunk_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&num_channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits_per_sample.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..num_samples {
        let sample: i16 = if (i / 10) % 2 == 0 { 1200 } else { -1200 };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// 最小合法 ISO Base Media File Format (`ftypisom`) MP4 头字节流（用于容器探测与基础测试）
pub fn minimal_mp4_ftyp_bytes() -> Vec<u8> {
    vec![
        0x00, 0x00, 0x00, 0x18, // box size = 24
        b'f', b't', b'y', b'p', // box type = ftyp
        b'i', b's', b'o', b'm', // major brand
        0x00, 0x00, 0x02, 0x00, // minor version
        b'i', b's', b'o', b'm', // compatible brand 1
        b'm', b'p', b'4', b'1', // compatible brand 2
    ]
}
