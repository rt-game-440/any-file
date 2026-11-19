//! 文本查看态（Viewing Mode，工单 #10）主行为集成测试。
//! 严格通过 `AppSession` 公共接缝与真实临时文件验证：
//! 1. 多编码自动检测与无乱码解码（UTF-8 / UTF-8 BOM / GBK / UTF-16 LE & BE）及换行符元数据；
//! 2. 常见文本与代码格式语法高亮分词与行号；
//! 3. 页内查找（Ctrl+F）高亮跳转与 Ctrl+滚轮 字号缩放；
//! 4. 超过 10MB 大文件只读保护与超长单行流畅浏览策略。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyfile::domain::{ResolvedTheme, TextMode, ViewerKind};
use anyfile::fixtures::FixtureCorpus;
use anyfile::ports::{Clock, SystemShell, VideoBackend, VideoProbeInfo};
use anyfile::session::AppSession;
use anyfile::text_viewer::{LineEnding, TextEncoding};

struct FakeSystemShell {
    theme: Mutex<ResolvedTheme>,
}

impl FakeSystemShell {
    fn new(theme: ResolvedTheme) -> Self {
        Self {
            theme: Mutex::new(theme),
        }
    }
}

impl SystemShell for FakeSystemShell {
    fn system_theme(&self) -> ResolvedTheme {
        *self.theme.lock().unwrap()
    }

    fn prefers_reduced_motion(&self) -> bool {
        true
    }
}

struct FakeClock {
    now: Instant,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Instant::now(),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.now
    }
}

struct FakeVideoBackend;

impl VideoBackend for FakeVideoBackend {
    fn is_available(&self) -> bool {
        false
    }

    fn probe(&self, _path: &std::path::Path) -> Option<VideoProbeInfo> {
        None
    }
}

fn new_session(theme: ResolvedTheme) -> AppSession {
    AppSession::new(
        Arc::new(FakeSystemShell::new(theme)),
        Arc::new(FakeClock::new()),
        Arc::new(FakeVideoBackend),
    )
}

#[test]
fn decodes_corpus_utf8_utf8_bom_gbk_and_utf16_without_mojibake() {
    let temp = tempfile::tempdir().expect("创建临时目录失败");
    let corpus = FixtureCorpus::generate_in(temp.path()).expect("生成语料库失败");
    let session = new_session(ResolvedTheme::Dark);

    // 1. UTF-8 (无 BOM, LF)
    session
        .open_file(&corpus.text_utf8_lf)
        .expect("打开 UTF-8 LF 文件应成功");
    assert_eq!(session.active_viewer(), ViewerKind::Text);
    assert_eq!(session.text_mode(), TextMode::Viewing);
    let doc = session.text_viewer().expect("文本查看器状态应已加载");
    assert_eq!(doc.meta.encoding, TextEncoding::Utf8);
    assert!(!doc.meta.has_bom);
    assert_eq!(doc.meta.line_ending, LineEnding::Lf);
    assert!(!doc.meta.is_large_file_readonly);
    assert_eq!(doc.lines.len(), 2);
    assert_eq!(doc.lines[0].line_number, 1);
    assert_eq!(doc.lines[0].display_text, "AnyFile UTF-8 LF 测试");
    assert_eq!(doc.lines[1].line_number, 2);
    assert_eq!(doc.lines[1].display_text, "第二行内容");

    // 2. UTF-8 BOM (CRLF)
    session
        .open_file(&corpus.text_utf8_bom_crlf)
        .expect("打开 UTF-8 BOM CRLF 文件应成功");
    let doc = session.text_viewer().expect("文本查看器状态应已加载");
    assert_eq!(doc.meta.encoding, TextEncoding::Utf8Bom);
    assert!(doc.meta.has_bom);
    assert_eq!(doc.meta.line_ending, LineEnding::Crlf);
    assert_eq!(doc.lines[0].display_text, "AnyFile UTF-8 BOM CRLF 测试");
    assert_eq!(doc.lines[1].display_text, "第二行");

    // 3. GBK (CRLF，中文“你好世界”)
    session
        .open_file(&corpus.text_gbk)
        .expect("打开 GBK 文件应成功");
    let doc = session.text_viewer().expect("文本查看器状态应已加载");
    assert_eq!(doc.meta.encoding, TextEncoding::Gbk);
    assert!(!doc.meta.has_bom);
    assert_eq!(doc.meta.line_ending, LineEnding::Crlf);
    assert_eq!(doc.lines[0].display_text, "你好世界");

    // 4. UTF-16 LE BOM (CRLF)
    session
        .open_file(&corpus.text_utf16_le)
        .expect("打开 UTF-16 LE 文件应成功");
    let doc = session.text_viewer().expect("文本查看器状态应已加载");
    assert_eq!(doc.meta.encoding, TextEncoding::Utf16Le);
    assert!(doc.meta.has_bom);
    assert_eq!(doc.meta.line_ending, LineEnding::Crlf);
    assert_eq!(doc.lines[0].display_text, "AnyFile");

    // 5. UTF-16 BE BOM + 中文 + Mixed 换行符
    let utf16_be_path = temp.path().join("notes_utf16be.txt");
    let mut utf16_be = vec![0xFE, 0xFF];
    for unit in "中文标题\r\n第二行\n".encode_utf16() {
        utf16_be.extend_from_slice(&unit.to_be_bytes());
    }
    std::fs::write(&utf16_be_path, utf16_be).expect("写入 UTF-16 BE 文件失败");

    session
        .open_file(&utf16_be_path)
        .expect("打开 UTF-16 BE 文件应成功");
    let doc = session.text_viewer().expect("文本查看器状态应已加载");
    assert_eq!(doc.meta.encoding, TextEncoding::Utf16Be);
    assert!(doc.meta.has_bom);
    assert_eq!(doc.meta.line_ending, LineEnding::Mixed);
    assert_eq!(doc.lines[0].display_text, "中文标题");
    assert_eq!(doc.lines[1].display_text, "第二行");
}

use anyfile::text_viewer::{SyntaxKind, TokenKind, LARGE_FILE_THRESHOLD, MAX_LINE_DISPLAY_CHARS};

fn open_text(session: &AppSession, dir: &std::path::Path, name: &str, content: &[u8]) {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    session.open_file(&p).expect("打开应成功");
    assert_eq!(session.active_viewer(), ViewerKind::Text);
}

fn kinds_of(session: &AppSession, line: usize) -> Vec<(String, TokenKind)> {
    let doc = session.text_viewer().unwrap();
    let l = &doc.lines[line];
    l.tokens
        .iter()
        .map(|t| (l.display_text[t.start..t.end].to_string(), t.kind))
        .collect()
}

#[test]
fn tokenizes_common_formats_with_contiguous_coverage() {
    let temp = tempfile::tempdir().unwrap();
    let s = new_session(ResolvedTheme::Light);

    open_text(&s, temp.path(), "a.rs", b"fn main() { let x = 42; } // hi\n/* a\nb */ let y = \"s\";\n");
    let doc = s.text_viewer().unwrap();
    assert_eq!(doc.meta.syntax, SyntaxKind::Rust);
    let k = kinds_of(&s, 0);
    assert!(k.contains(&("fn".into(), TokenKind::Keyword)));
    assert!(k.contains(&("42".into(), TokenKind::Number)));
    assert!(k.iter().any(|(t, c)| t.contains("hi") && *c == TokenKind::Comment));
    // 块注释跨行
    assert!(kinds_of(&s, 1).iter().all(|(_, c)| *c == TokenKind::Comment));
    assert!(kinds_of(&s, 2).contains(&("\"s\"".into(), TokenKind::String)));

    open_text(&s, temp.path(), "b.json", b"{\"k\": [1, true, \"v\"]}\n");
    let k = kinds_of(&s, 0);
    assert!(k.contains(&("\"k\"".into(), TokenKind::Key)));
    assert!(k.contains(&("true".into(), TokenKind::Keyword)));
    assert!(k.contains(&("\"v\"".into(), TokenKind::String)));

    open_text(&s, temp.path(), "c.toml", b"[pkg]\nname = \"x\" # c\n");
    assert_eq!(kinds_of(&s, 0)[0].1, TokenKind::Heading);
    assert!(kinds_of(&s, 1).contains(&("name".into(), TokenKind::Key)));

    open_text(&s, temp.path(), "d.md", b"# Title\n`code`\n");
    assert_eq!(kinds_of(&s, 0)[0].1, TokenKind::Heading);
    assert!(kinds_of(&s, 1).contains(&("`code`".into(), TokenKind::String)));

    open_text(&s, temp.path(), "e.xml", b"<a b=\"c\"><!-- x --></a>\n");
    assert!(kinds_of(&s, 0).contains(&("<a".into(), TokenKind::Tag)));

    // 覆盖完整性
    for name in ["a.rs", "b.json", "c.toml", "d.md", "e.xml"] {
        open_text(&s, temp.path(), name, &std::fs::read(temp.path().join(name)).unwrap());
        let doc = s.text_viewer().unwrap();
        for l in &doc.lines {
            let mut pos = 0;
            for t in &l.tokens {
                assert_eq!(t.start, pos);
                pos = t.end;
            }
            if !l.tokens.is_empty() {
                assert_eq!(pos, l.display_text.len());
            }
        }
    }
}

#[test]
fn find_highlights_jumps_and_zoom_clamps() {
    let temp = tempfile::tempdir().unwrap();
    let s = new_session(ResolvedTheme::Light);
    open_text(&s, temp.path(), "f.txt", "Foo bar\nfoo 你好 FOO\n".as_bytes());

    let st = s.text_find("foo", false);
    assert_eq!(st.matches.len(), 3);
    assert_eq!(st.status_text(), "1 / 3");
    assert_eq!(s.text_find_next().unwrap().line_index, 1);
    s.text_find_next();
    assert_eq!(s.text_find_next().unwrap().line_index, 0, "循环回到首个");
    assert_eq!(s.text_find_prev().unwrap().line_index, 1);
    assert_eq!(s.text_find("foo", true).matches.len(), 1);
    assert_eq!(s.text_find("你好", true).matches.len(), 1);
    assert_eq!(s.text_find("zzz", false).status_text(), "无结果");
    s.text_find_clear();
    assert!(s.text_find_state().matches.is_empty());

    let base = s.text_font_size();
    assert_eq!(s.text_zoom_by_wheel(120.0, false), base);
    assert_eq!(s.text_zoom_by_wheel(120.0, true), base + 1.0);
    for _ in 0..100 {
        s.text_zoom_by_wheel(-1.0, true);
    }
    assert_eq!(s.text_font_size(), anyfile::text_viewer::MIN_FONT_SIZE);
    assert_eq!(s.text_zoom_reset(), anyfile::text_viewer::DEFAULT_FONT_SIZE);
}

#[test]
fn large_file_is_readonly_with_bounded_preview_and_long_line_truncated() {
    let temp = tempfile::tempdir().unwrap();
    let s = new_session(ResolvedTheme::Light);

    // 超过 10MB 的多行文本
    let big = temp.path().join("big.log");
    let line = "2024-01-01 INFO something happened here\n";
    let n = (LARGE_FILE_THRESHOLD as usize / line.len()) + 10;
    std::fs::write(&big, line.repeat(n)).unwrap();
    let t = Instant::now();
    s.open_file(&big).unwrap();
    let doc = s.text_viewer().unwrap();
    assert!(doc.meta.is_large_file_readonly);
    assert!(doc.meta.is_preview_truncated);
    assert!(s.text_is_readonly_protected());
    assert!(doc.lines.len() < n, "仅加载前缀预览");
    assert!(t.elapsed().as_secs() < 10);

    // 超长单行（压缩 JSON）
    let json = format!("[{}]", "1,".repeat(2_000_000));
    open_text(&s, temp.path(), "min.json", json.as_bytes());
    let doc = s.text_viewer().unwrap();
    assert!(!doc.meta.is_large_file_readonly);
    assert_eq!(doc.lines.len(), 1);
    assert!(doc.lines[0].is_truncated);
    assert!(doc.lines[0].display_text.chars().count() <= MAX_LINE_DISPLAY_CHARS + 1);
}
