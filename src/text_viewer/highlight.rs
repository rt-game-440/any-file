//! 轻量语法高亮分词：按扩展名/文件名选择语法，逐行产出连续覆盖的 `HighlightToken`。

use std::path::Path;

use super::{HIGHLIGHT_MAX_LINE_CHARS, TextLine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain = 0,
    Keyword = 1,
    String = 2,
    Comment = 3,
    Number = 4,
    Key = 5,
    Tag = 6,
    Heading = 7,
    Type = 8,
}

/// 分词：`display_text` 内的字节区间及类别。同一行的分词连续且完整覆盖整行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightToken {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    PlainText,
    Markdown,
    Json,
    Yaml,
    Toml,
    Xml,
    Html,
    Ini,
    Log,
    Csv,
    Rust,
    Python,
    JavaScript,
    TypeScript,
    C,
    Cpp,
    Java,
    CSharp,
    Go,
    Shell,
    Css,
    Sql,
}

impl SyntaxKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::PlainText => "纯文本",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Xml => "XML",
            Self::Html => "HTML",
            Self::Ini => "INI",
            Self::Log => "日志",
            Self::Csv => "CSV",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Java => "Java",
            Self::CSharp => "C#",
            Self::Go => "Go",
            Self::Shell => "Shell",
            Self::Css => "CSS",
            Self::Sql => "SQL",
        }
    }

    pub fn from_path(path: &Path) -> Self {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            let lower = name.to_ascii_lowercase();
            if matches!(lower.as_str(), "makefile" | "dockerfile" | ".bashrc" | ".zshrc") {
                return Self::Shell;
            }
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "md" | "markdown" => Self::Markdown,
            "json" | "jsonc" | "json5" | "geojson" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "toml" => Self::Toml,
            "xml" | "svg" | "xaml" | "csproj" | "plist" => Self::Xml,
            "html" | "htm" => Self::Html,
            "ini" | "cfg" | "conf" | "properties" | "desktop" => Self::Ini,
            "log" => Self::Log,
            "csv" | "tsv" => Self::Csv,
            "rs" => Self::Rust,
            "py" | "pyw" => Self::Python,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "java" | "kt" | "kts" => Self::Java,
            "cs" => Self::CSharp,
            "go" => Self::Go,
            "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" => Self::Shell,
            "css" | "scss" | "less" => Self::Css,
            "sql" => Self::Sql,
            _ => Self::PlainText,
        }
    }
}

/// 跨行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    Normal,
    /// 位于块注释中（代码语言 `/* */`、XML `<!-- -->`）
    BlockComment,
    /// 位于三引号字符串中（Python）或 Markdown 围栏代码块
    BlockString,
}

struct Lang {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    triple_quote: bool,
    case_insensitive: bool,
}

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while",
];
const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];
const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete",
    "do", "else", "export", "extends", "false", "finally", "for", "from", "function", "if",
    "import", "in", "instanceof", "let", "new", "null", "of", "return", "super", "switch", "this",
    "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "yield",
    // TypeScript
    "interface", "type", "enum", "implements", "public", "private", "protected", "readonly",
    "as", "namespace", "declare", "abstract",
];
const C_KW: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union",
    "unsigned", "void", "volatile", "while", "bool", "true", "false", "NULL",
    // C++
    "class", "namespace", "template", "typename", "public", "private", "protected", "virtual",
    "override", "new", "delete", "this", "nullptr", "using", "try", "catch", "throw", "constexpr",
    "operator", "final",
];
const JAVA_KW: &[&str] = &[
    "abstract", "boolean", "break", "byte", "case", "catch", "char", "class", "const",
    "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally",
    "float", "for", "if", "implements", "import", "instanceof", "int", "interface", "long",
    "new", "null", "package", "private", "protected", "public", "return", "short", "static",
    "super", "switch", "this", "throw", "throws", "true", "false", "try", "void", "while",
    // Kotlin / C#
    "fun", "val", "var", "when", "object", "namespace", "using", "string", "bool", "override",
    "virtual", "async", "await", "readonly", "sealed", "internal", "get", "set",
];
const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
    "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
    "return", "select", "struct", "switch", "type", "var", "nil", "true", "false",
];
const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function",
    "in", "return", "export", "local", "echo", "set", "unset", "exit", "param", "foreach",
];
const CSS_KW: &[&str] = &["important", "inherit", "initial", "none", "auto"];
const SQL_KW: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create",
    "table", "drop", "alter", "join", "left", "right", "inner", "outer", "on", "group", "by",
    "order", "having", "limit", "and", "or", "not", "null", "as", "distinct", "union", "index",
    "primary", "key", "foreign", "references", "in", "like", "is", "case", "when", "then",
    "else", "end",
];

fn lang_for(syntax: SyntaxKind) -> Option<Lang> {
    let hash: &[&str] = &["#"];
    let slashes: &[&str] = &["//"];
    let dq: &[char] = &['"', '\''];
    let l = match syntax {
        SyntaxKind::Rust => Lang {
            keywords: RUST_KW,
            line_comments: slashes,
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            triple_quote: false,
            case_insensitive: false,
        },
        SyntaxKind::Python => Lang {
            keywords: PY_KW,
            line_comments: hash,
            block_comment: None,
            quotes: dq,
            triple_quote: true,
            case_insensitive: false,
        },
        SyntaxKind::JavaScript | SyntaxKind::TypeScript => Lang {
            keywords: JS_KW,
            line_comments: slashes,
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            triple_quote: false,
            case_insensitive: false,
        },
        SyntaxKind::C | SyntaxKind::Cpp => Lang {
            keywords: C_KW,
            line_comments: slashes,
            block_comment: Some(("/*", "*/")),
            quotes: dq,
            triple_quote: false,
            case_insensitive: false,
        },
        SyntaxKind::Java | SyntaxKind::CSharp => Lang {
            keywords: JAVA_KW,
            line_comments: slashes,
            block_comment: Some(("/*", "*/")),
            quotes: dq,
            triple_quote: false,
            case_insensitive: false,
        },
        SyntaxKind::Go => Lang {
            keywords: GO_KW,
            line_comments: slashes,
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '`', '\''],
            triple_quote: false,
            case_insensitive: false,
        },
        SyntaxKind::Shell => Lang {
            keywords: SH_KW,
            line_comments: hash,
            block_comment: None,
            quotes: dq,
            triple_quote: false,
            case_insensitive: false,
        },
        SyntaxKind::Css => Lang {
            keywords: CSS_KW,
            line_comments: &[],
            block_comment: Some(("/*", "*/")),
            quotes: dq,
            triple_quote: false,
            case_insensitive: true,
        },
        SyntaxKind::Sql => Lang {
            keywords: SQL_KW,
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            quotes: dq,
            triple_quote: false,
            case_insensitive: true,
        },
        _ => return None,
    };
    Some(l)
}

/// 为整份文档的每一行生成分词（跨行状态依次传递）。
pub fn tokenize_document_lines(syntax: SyntaxKind, lines: &mut [TextLine]) {
    let mut state = LineState::Normal;
    for line in lines.iter_mut() {
        if line.display_text.chars().count() > HIGHLIGHT_MAX_LINE_CHARS {
            // 超长行不分词，整行按纯文本（保持状态不变）
            line.tokens = vec![HighlightToken {
                start: 0,
                end: line.display_text.len(),
                kind: TokenKind::Plain,
            }];
            continue;
        }
        let (tokens, next) = tokenize_line(syntax, &line.display_text, state);
        line.tokens = tokens;
        state = next;
    }
}

fn tokenize_line(
    syntax: SyntaxKind,
    line: &str,
    state: LineState,
) -> (Vec<HighlightToken>, LineState) {
    let mut sink = Sink::new(line.len());
    let next = match syntax {
        SyntaxKind::Json => {
            lex_json(line, &mut sink);
            LineState::Normal
        }
        SyntaxKind::Yaml => {
            lex_keyvalue(line, &mut sink, &['#'], false);
            LineState::Normal
        }
        SyntaxKind::Toml | SyntaxKind::Ini => {
            lex_keyvalue(line, &mut sink, &['#', ';'], true);
            LineState::Normal
        }
        SyntaxKind::Xml | SyntaxKind::Html => lex_markup(line, &mut sink, state),
        SyntaxKind::Markdown => lex_markdown(line, &mut sink, state),
        SyntaxKind::Log => {
            lex_log(line, &mut sink);
            LineState::Normal
        }
        SyntaxKind::Csv => {
            lex_csv(line, &mut sink);
            LineState::Normal
        }
        other => match lang_for(other) {
            Some(lang) => lex_code(line, &lang, &mut sink, state),
            None => LineState::Normal,
        },
    };
    (sink.finish(), next)
}

/// 收集分词并在末尾补齐 Plain 间隙，保证连续覆盖。
struct Sink {
    len: usize,
    cursor: usize,
    tokens: Vec<HighlightToken>,
}

impl Sink {
    fn new(len: usize) -> Self {
        Self {
            len,
            cursor: 0,
            tokens: Vec::new(),
        }
    }

    fn push(&mut self, start: usize, end: usize, kind: TokenKind) {
        if end <= start || start < self.cursor {
            return;
        }
        if start > self.cursor {
            self.raw(self.cursor, start, TokenKind::Plain);
        }
        self.raw(start, end, kind);
        self.cursor = end;
    }

    fn raw(&mut self, start: usize, end: usize, kind: TokenKind) {
        if let Some(last) = self.tokens.last_mut()
            && last.kind == kind
            && last.end == start
        {
            last.end = end;
            return;
        }
        self.tokens.push(HighlightToken { start, end, kind });
    }

    fn finish(mut self) -> Vec<HighlightToken> {
        if self.cursor < self.len {
            self.raw(self.cursor, self.len, TokenKind::Plain);
        }
        self.tokens
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 从 `start`（引号字节位置）扫描字符串，返回结束字节位置（含收尾引号；未闭合则到行尾）。
fn scan_string(line: &str, start: usize, quote: char) -> usize {
    let mut escaped = false;
    for (off, c) in line[start + quote.len_utf8()..].char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return start + quote.len_utf8() + off + c.len_utf8();
        }
    }
    line.len()
}

fn scan_number(line: &str, start: usize) -> usize {
    let mut end = start;
    for (off, c) in line[start..].char_indices() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            end = start + off + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn lex_code(line: &str, lang: &Lang, sink: &mut Sink, mut state: LineState) -> LineState {
    let mut i = 0usize;
    let mut prev_ident = false;
    while i < line.len() {
        let rest = &line[i..];
        match state {
            LineState::BlockComment => {
                let end_marker = lang.block_comment.map_or("*/", |b| b.1);
                match rest.find(end_marker) {
                    Some(p) => {
                        let end = i + p + end_marker.len();
                        sink.push(i, end, TokenKind::Comment);
                        i = end;
                        state = LineState::Normal;
                    }
                    None => {
                        sink.push(i, line.len(), TokenKind::Comment);
                        return LineState::BlockComment;
                    }
                }
                prev_ident = false;
                continue;
            }
            LineState::BlockString => {
                match rest.find("\"\"\"").or_else(|| rest.find("'''")) {
                    Some(p) => {
                        let end = i + p + 3;
                        sink.push(i, end, TokenKind::String);
                        i = end;
                        state = LineState::Normal;
                    }
                    None => {
                        sink.push(i, line.len(), TokenKind::String);
                        return LineState::BlockString;
                    }
                }
                prev_ident = false;
                continue;
            }
            LineState::Normal => {}
        }

        if lang.line_comments.iter().any(|m| rest.starts_with(m)) {
            sink.push(i, line.len(), TokenKind::Comment);
            return LineState::Normal;
        }
        if let Some((open, _)) = lang.block_comment
            && rest.starts_with(open)
        {
            // 跳过开头标记避免 `/*/` 误判
            let after = i + open.len();
            let end_marker = lang.block_comment.map_or("*/", |b| b.1);
            match line[after..].find(end_marker) {
                Some(p) => {
                    let end = after + p + end_marker.len();
                    sink.push(i, end, TokenKind::Comment);
                    i = end;
                    state = LineState::Normal;
                }
                None => {
                    sink.push(i, line.len(), TokenKind::Comment);
                    return LineState::BlockComment;
                }
            }
            continue;
        }

        let c = rest.chars().next().unwrap();
        if lang.triple_quote && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
            let marker = &rest[..3];
            match rest[3..].find(marker) {
                Some(p) => {
                    let end = i + 3 + p + 3;
                    sink.push(i, end, TokenKind::String);
                    i = end;
                }
                None => {
                    sink.push(i, line.len(), TokenKind::String);
                    return LineState::BlockString;
                }
            }
            prev_ident = false;
            continue;
        }
        if lang.quotes.contains(&c) {
            let end = scan_string(line, i, c);
            sink.push(i, end, TokenKind::String);
            i = end;
            prev_ident = false;
            continue;
        }
        if c.is_ascii_digit() && !prev_ident {
            let end = scan_number(line, i);
            sink.push(i, end, TokenKind::Number);
            i = end;
            continue;
        }
        if is_ident_start(c) {
            let end = i + rest
                .char_indices()
                .find(|&(_, ch)| !is_ident_char(ch))
                .map_or(rest.len(), |(o, _)| o);
            let word = &line[i..end];
            let is_kw = if lang.case_insensitive {
                lang.keywords
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(word))
            } else {
                lang.keywords.contains(&word)
            };
            if is_kw {
                sink.push(i, end, TokenKind::Keyword);
            } else if c.is_uppercase() && word.len() > 1 {
                sink.push(i, end, TokenKind::Type);
            }
            i = end;
            prev_ident = true;
            continue;
        }
        i += c.len_utf8();
        prev_ident = false;
    }
    state
}

fn lex_json(line: &str, sink: &mut Sink) {
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();
        if c == '"' {
            let end = scan_string(line, i, '"');
            let after = line[end..].trim_start();
            let kind = if after.starts_with(':') {
                TokenKind::Key
            } else {
                TokenKind::String
            };
            sink.push(i, end, kind);
            i = end;
        } else if c.is_ascii_digit() || (c == '-' && rest[1..].starts_with(|d: char| d.is_ascii_digit()))
        {
            let end = i + 1 + scan_number(&line[i + 1..], 0);
            sink.push(i, end, TokenKind::Number);
            i = end;
        } else if is_ident_start(c) {
            let end = i + rest
                .char_indices()
                .find(|&(_, ch)| !is_ident_char(ch))
                .map_or(rest.len(), |(o, _)| o);
            if matches!(&line[i..end], "true" | "false" | "null") {
                sink.push(i, end, TokenKind::Keyword);
            }
            i = end;
        } else if rest.starts_with("//") {
            sink.push(i, line.len(), TokenKind::Comment);
            return;
        } else {
            i += c.len_utf8();
        }
    }
}

/// YAML / TOML / INI：注释、`[section]`、`key:`/`key =`、引号字符串、数字与布尔值。
fn lex_keyvalue(line: &str, sink: &mut Sink, comment_chars: &[char], sections: bool) {
    let trimmed_start = line.len() - line.trim_start().len();
    let body = &line[trimmed_start..];
    if body.is_empty() {
        return;
    }
    if body.starts_with(|c| comment_chars.contains(&c)) {
        sink.push(trimmed_start, line.len(), TokenKind::Comment);
        return;
    }
    if sections && body.starts_with('[') {
        sink.push(trimmed_start, line.len(), TokenKind::Heading);
        return;
    }

    let mut value_start = trimmed_start;
    let mut work = body;
    let mut base = trimmed_start;
    // YAML 列表项前缀
    if !sections && work.starts_with("- ") {
        base += 2;
        work = &line[base..];
        value_start = base;
    }
    let sep = if sections { '=' } else { ':' };
    if let Some(p) = find_key_separator(work, sep) {
        let key_end = base + work[..p].trim_end().len();
        sink.push(base, key_end, TokenKind::Key);
        value_start = base + p + 1;
    }

    let mut i = value_start;
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();
        if comment_chars.contains(&c) && (i == 0 || line[..i].ends_with(char::is_whitespace)) {
            sink.push(i, line.len(), TokenKind::Comment);
            return;
        }
        if c == '"' || c == '\'' {
            let end = scan_string(line, i, c);
            sink.push(i, end, TokenKind::String);
            i = end;
        } else if c.is_ascii_digit() && !line[..i].ends_with(is_ident_char) {
            let end = scan_number(line, i);
            sink.push(i, end, TokenKind::Number);
            i = end;
        } else if is_ident_start(c) {
            let end = i + rest
                .char_indices()
                .find(|&(_, ch)| !is_ident_char(ch))
                .map_or(rest.len(), |(o, _)| o);
            if matches!(
                &line[i..end],
                "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "True" | "False"
            ) {
                sink.push(i, end, TokenKind::Keyword);
            }
            i = end;
        } else {
            i += c.len_utf8();
        }
    }
}

/// 查找键值分隔符：YAML 需 `:` 后为空白/行尾；忽略引号内的分隔符。
fn find_key_separator(text: &str, sep: char) -> Option<usize> {
    let mut in_quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match in_quote {
            Some(q) => {
                if c == q {
                    in_quote = None;
                }
            }
            None => {
                if (c == '"' || c == '\'') && i == 0 {
                    in_quote = Some(c);
                } else if c == sep {
                    if sep == ':' {
                        let after = &text[i + 1..];
                        if after.is_empty() || after.starts_with(char::is_whitespace) {
                            return Some(i);
                        }
                    } else {
                        return Some(i);
                    }
                } else if c == '#' || (sep == '=' && c == ';') {
                    return None;
                }
            }
        }
    }
    None
}

fn lex_markup(line: &str, sink: &mut Sink, mut state: LineState) -> LineState {
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        if state == LineState::BlockComment {
            match rest.find("-->") {
                Some(p) => {
                    sink.push(i, i + p + 3, TokenKind::Comment);
                    i += p + 3;
                    state = LineState::Normal;
                }
                None => {
                    sink.push(i, line.len(), TokenKind::Comment);
                    return LineState::BlockComment;
                }
            }
            continue;
        }
        if let Some(inner) = rest.strip_prefix("<!--") {
            match inner.find("-->") {
                Some(p) => {
                    sink.push(i, i + 4 + p + 3, TokenKind::Comment);
                    i += 4 + p + 3;
                }
                None => {
                    sink.push(i, line.len(), TokenKind::Comment);
                    return LineState::BlockComment;
                }
            }
            continue;
        }
        if rest.starts_with('<') {
            // 标签：<name 与 >，其间的引号值为字符串，属性名为 Key
            let name_end = rest
                .char_indices()
                .skip(1)
                .find(|&(_, c)| c.is_whitespace() || c == '>' || c == '/')
                .map_or(rest.len(), |(o, _)| o);
            sink.push(i, i + name_end, TokenKind::Tag);
            i += name_end;
            while i < line.len() {
                let r = &line[i..];
                let c = r.chars().next().unwrap();
                if c == '>' {
                    sink.push(i, i + 1, TokenKind::Tag);
                    i += 1;
                    break;
                } else if c == '/' && r[1..].starts_with('>') {
                    sink.push(i, i + 2, TokenKind::Tag);
                    i += 2;
                    break;
                } else if c == '"' || c == '\'' {
                    let end = scan_string(line, i, c);
                    sink.push(i, end, TokenKind::String);
                    i = end;
                } else if is_ident_start(c) {
                    let end = i + r
                        .char_indices()
                        .find(|&(_, ch)| !(is_ident_char(ch) || ch == '-' || ch == ':'))
                        .map_or(r.len(), |(o, _)| o);
                    sink.push(i, end, TokenKind::Key);
                    i = end;
                } else {
                    i += c.len_utf8();
                }
            }
            continue;
        }
        i += rest.chars().next().unwrap().len_utf8();
    }
    state
}

fn lex_markdown(line: &str, sink: &mut Sink, state: LineState) -> LineState {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        sink.push(0, line.len(), TokenKind::String);
        return if state == LineState::BlockString {
            LineState::Normal
        } else {
            LineState::BlockString
        };
    }
    if state == LineState::BlockString {
        sink.push(0, line.len(), TokenKind::String);
        return state;
    }
    if trimmed.starts_with('#') {
        sink.push(indent, line.len(), TokenKind::Heading);
        return state;
    }
    if trimmed.starts_with('>') {
        sink.push(indent, line.len(), TokenKind::Comment);
        return state;
    }
    let mut i = indent;
    // 列表标记
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        sink.push(i, i + 1, TokenKind::Keyword);
        i += 1;
    }
    while i < line.len() {
        let rest = &line[i..];
        if rest.starts_with('`')
            && let Some(p) = rest[1..].find('`')
        {
            sink.push(i, i + p + 2, TokenKind::String);
            i += p + 2;
            continue;
        }
        if rest.starts_with('[')
            && let Some(p) = rest.find("](")
            && let Some(q) = rest[p..].find(')')
        {
            sink.push(i, i + p + q + 1, TokenKind::Tag);
            i += p + q + 1;
            continue;
        }
        i += rest.chars().next().unwrap().len_utf8();
    }
    state
}

fn lex_log(line: &str, sink: &mut Sink) {
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();
        if c.is_ascii_digit() {
            let end = i + rest
                .char_indices()
                .find(|&(_, ch)| !(ch.is_ascii_digit() || matches!(ch, '-' | ':' | '.' | '/' | ',')))
                .map_or(rest.len(), |(o, _)| o);
            sink.push(i, end, TokenKind::Number);
            i = end;
        } else if is_ident_start(c) {
            let end = i + rest
                .char_indices()
                .find(|&(_, ch)| !is_ident_char(ch))
                .map_or(rest.len(), |(o, _)| o);
            let kind = match &line[i..end] {
                "ERROR" | "FATAL" | "SEVERE" | "CRITICAL" | "Exception" | "error" | "fatal" => {
                    Some(TokenKind::Keyword)
                }
                "WARN" | "WARNING" | "warn" | "warning" => Some(TokenKind::Type),
                "INFO" | "DEBUG" | "TRACE" | "info" | "debug" | "trace" => Some(TokenKind::Tag),
                _ => None,
            };
            if let Some(k) = kind {
                sink.push(i, end, k);
            }
            i = end;
        } else if c == '[' || c == ']' {
            sink.push(i, i + 1, TokenKind::Comment);
            i += 1;
        } else if c == '"' {
            let end = scan_string(line, i, '"');
            sink.push(i, end, TokenKind::String);
            i = end;
        } else {
            i += c.len_utf8();
        }
    }
}

fn lex_csv(line: &str, sink: &mut Sink) {
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();
        if c == '"' {
            let end = scan_string(line, i, '"');
            sink.push(i, end, TokenKind::String);
            i = end;
        } else if c == ',' || c == '\t' || c == ';' {
            sink.push(i, i + 1, TokenKind::Comment);
            i += 1;
        } else if c.is_ascii_digit() || c == '-' {
            let end = i + rest
                .char_indices()
                .find(|&(_, ch)| matches!(ch, ',' | '\t' | ';' | '"'))
                .map_or(rest.len(), |(o, _)| o);
            let field = &line[i..end];
            if field.trim().parse::<f64>().is_ok() {
                sink.push(i, end, TokenKind::Number);
            }
            i = end.max(i + 1);
        } else {
            i += c.len_utf8();
        }
    }
}
