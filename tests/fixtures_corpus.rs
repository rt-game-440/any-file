use std::fs;

use anyfile::fixtures::FixtureCorpus;

#[test]
fn fixture_corpus_generates_all_four_categories_and_edge_cases() {
    let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let corpus = FixtureCorpus::generate_in(temp_dir.path()).expect("生成样例文件语料库失败");

    assert!(
        corpus.all_paths().len() >= 14,
        "语料库应至少包含 14 个核心样例文件"
    );
    for path in corpus.all_paths() {
        let meta = fs::metadata(path)
            .unwrap_or_else(|e| panic!("样例文件缺失 {}: {e}", path.display()));
        assert!(meta.len() > 0, "样例文件不应为空: {}", path.display());
    }

    // 校验 PNG Magic Bytes
    let png_bytes = fs::read(&corpus.png_valid).unwrap();
    assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n");

    // 校验错后缀文件实际内容为 PNG Magic Bytes
    let misnamed_bytes = fs::read(&corpus.misnamed_png_as_txt).unwrap();
    assert_eq!(&misnamed_bytes[..8], b"\x89PNG\r\n\x1a\n");

    // 校验 BMP Magic Bytes
    let bmp_bytes = fs::read(&corpus.bmp_valid).unwrap();
    assert_eq!(&bmp_bytes[..2], b"BM");

    // 校验 WAV RIFF/WAVE 头
    let wav_bytes = fs::read(&corpus.wav_valid).unwrap();
    assert_eq!(&wav_bytes[..4], b"RIFF");
    assert_eq!(&wav_bytes[8..12], b"WAVE");

    // 校验 MP4 ftyp box
    let mp4_bytes = fs::read(&corpus.mp4_minimal).unwrap();
    assert_eq!(&mp4_bytes[4..8], b"ftyp");

    // 校验多编码与换行样本
    let utf8_bom = fs::read(&corpus.text_utf8_bom_crlf).unwrap();
    assert_eq!(&utf8_bom[..3], &[0xEF, 0xBB, 0xBF]);
    assert!(utf8_bom.windows(2).any(|w| w == b"\r\n"));

    let utf16_le = fs::read(&corpus.text_utf16_le).unwrap();
    assert_eq!(&utf16_le[..2], &[0xFF, 0xFE]);

    let gbk = fs::read(&corpus.text_gbk).unwrap();
    assert!(std::str::from_utf8(&gbk).is_err(), "GBK 样本不应为合法 UTF-8");

    // 校验无扩展名文件与歧义文件
    assert!(corpus.text_no_extension_makefile.extension().is_none());
    assert_eq!(
        corpus.svg_ambiguous.extension().and_then(|s| s.to_str()),
        Some("svg")
    );
    assert_eq!(
        corpus.ts_ambiguous.extension().and_then(|s| s.to_str()),
        Some("ts")
    );
}
