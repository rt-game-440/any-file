//! Rust 侧用户可见消息的英文本地化完整性：英文下不得残留中文。

use std::path::Path;

use anyfile::i18n::{Language, large_file_notice, localize, text_meta_line};
use anyfile::ports::{VideoBackend, VideoProbeInfo};
use anyfile::probing::probe_file;

struct NoVid;
impl VideoBackend for NoVid {
    fn is_available(&self) -> bool {
        false
    }
    fn probe(&self, _: &Path) -> Option<VideoProbeInfo> {
        None
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c) || ('\u{ff00}'..='\u{ffef}').contains(&c))
}

#[test]
fn probe_fallback_messages_are_fully_english() {
    let dir = tempfile::tempdir().unwrap();
    let mut samples: Vec<std::path::PathBuf> = vec![dir.path().to_path_buf(), dir.path().join("missing.bin")];
    for (name, bytes) in [
        ("bad.png", &b"\x89PNG\r\n\x1a\nxx"[..]),
        ("bad.jpg", b"\xff\xd8\xff"),
        ("bad.gif", b"GIF89a"),
        ("bad.bmp", b"BM\0\0"),
        ("bad.webp", b"RIFF\0\0\0\0WEBP"),
        ("bad.wav", b"RIFF\0\0\0\0WAVE"),
        ("bad.tiff", b"II*\0"),
        ("bin.dat", b"\x00\x01\x02\xff\xfe\x00\x00\x80"),
        ("fake.png", b"hello text"),
        ("a.txt", b"plain"),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        samples.push(p);
    }
    for p in samples {
        let r = probe_file(&p, &NoVid);
        let mut texts = vec![r.detected_format.clone()];
        if let Some(f) = r.fallback {
            texts.push(f.reason_message.clone());
        }
        for t in texts {
            let en = localize(&t, Language::En);
            assert!(!has_cjk(&en), "{p:?}: 英文残留中文：{en}（原文 {t}）");
            assert_eq!(localize(&t, Language::ZhCn), t);
        }
    }
}

#[test]
fn shell_audio_text_messages_are_fully_english() {
    for m in [
        "系统回收站操作尚未实现",
        "打开失败（错误码 5）",
        "移入回收站失败（错误码 0x7c）",
        "无法打开剪贴板",
        "不允许打开该链接",
        "当前未打开任何文件项",
        "文件不存在: C:/a.txt",
        "无法打开音频文件：x",
        "未找到可用的音频输出设备",
        "文件中没有可解码的音频轨道",
        "解码失败：x",
        "无法调用 curl：x",
        "响应解析失败：x",
        "无结果",
        "纯文本",
        "日志",
        "混合换行",
        "无换行",
        "无法读取文件信息: x",
        "读取文件失败: x",
        "Software（回退）",
        "MP4（纯音频）",
        "UTF-8 文本",
        "GBK 文本",
        "损坏的 PNG",
        "纯二进制",
    ] {
        let en = localize(m, Language::En);
        assert!(!has_cjk(&en), "英文残留中文：{en}（原文 {m}）");
        assert_ne!(en, m);
    }
    let meta = text_meta_line(Language::En, "UTF-8", "混合换行", 3, "日志", 14.0);
    assert!(!has_cjk(&meta), "{meta}");
    assert!(!has_cjk(&large_file_notice(Language::En, 12.3)));
    assert!(has_cjk(&text_meta_line(Language::ZhCn, "UTF-8", "LF", 3, "日志", 14.0)));
}

#[test]
fn editor_messages_are_fully_english() {
    for m in [
        "存在未保存的修改",
        "当前不是文本视图",
        "当前不在编辑态",
        "大文件只读保护：已禁用编辑",
        "文件含有无法无损解码的字节，已禁用编辑以防数据损坏",
        "无法写入临时文件：x",
        "保存：没有写入权限（x）",
        "文件为只读属性",
        "无法替换目标文件",
    ] {
        let en = localize(m, Language::En);
        assert!(!has_cjk(&en), "英文残留中文：{en}（原文 {m}）");
    }
}
