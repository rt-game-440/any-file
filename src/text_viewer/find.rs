//! 页内查找（Ctrl+F）：在展示文本中查找并支持循环跳转。

use super::TextDocument;

/// 最大记录的匹配数，避免极端查询占用过多内存。
pub const MAX_FIND_MATCHES: usize = 10_000;

/// 单个匹配：行索引（从 0 开始）与该行 `display_text` 内的字节区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindMatch {
    pub line_index: usize,
    pub start: usize,
    pub end: usize,
}

/// 页内查找状态快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextFindState {
    pub query: String,
    pub case_sensitive: bool,
    pub matches: Vec<FindMatch>,
    /// 当前高亮匹配在 `matches` 中的索引
    pub current: Option<usize>,
    /// 匹配数是否触顶被截断
    pub capped: bool,
}

impl TextFindState {
    pub fn current_match(&self) -> Option<FindMatch> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }

    /// 形如 `3 / 12` 的状态文案；无匹配返回 `无结果`，空查询返回空串。
    pub fn status_text(&self) -> String {
        if self.query.is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            "无结果".to_string()
        } else {
            let plus = if self.capped { "+" } else { "" };
            format!(
                "{} / {}{}",
                self.current.map_or(0, |i| i + 1),
                self.matches.len(),
                plus
            )
        }
    }

    pub fn next_match(&mut self) -> Option<FindMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = self.current.map_or(0, |i| (i + 1) % self.matches.len());
        self.current = Some(idx);
        self.current_match()
    }

    pub fn prev_match(&mut self) -> Option<FindMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let len = self.matches.len();
        let idx = self.current.map_or(len - 1, |i| (i + len - 1) % len);
        self.current = Some(idx);
        self.current_match()
    }
}

/// 在文档中查找 `query`，默认将第一个匹配设为当前项。
pub fn find_in_document(doc: &TextDocument, query: &str, case_sensitive: bool) -> TextFindState {
    let mut state = TextFindState {
        query: query.to_string(),
        case_sensitive,
        ..Default::default()
    };
    if query.is_empty() {
        return state;
    }

    let ascii_query = query.is_ascii();
    let needle_lower = query.to_ascii_lowercase();
    'outer: for (line_index, line) in doc.lines.iter().enumerate() {
        let text = line.display_text.as_str();
        let found: Vec<(usize, usize)> = if case_sensitive {
            text.match_indices(query)
                .map(|(s, m)| (s, s + m.len()))
                .collect()
        } else if ascii_query && text.is_ascii() {
            text.to_ascii_lowercase()
                .match_indices(needle_lower.as_str())
                .map(|(s, m)| (s, s + m.len()))
                .collect()
        } else {
            find_case_insensitive(text, query)
        };
        for (start, end) in found {
            if state.matches.len() >= MAX_FIND_MATCHES {
                state.capped = true;
                break 'outer;
            }
            state.matches.push(FindMatch {
                line_index,
                start,
                end,
            });
        }
    }
    if !state.matches.is_empty() {
        state.current = Some(0);
    }
    state
}

/// 通用（含非 ASCII）忽略大小写匹配，返回字节区间。
fn find_case_insensitive(text: &str, query: &str) -> Vec<(usize, usize)> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut qi = 0;
        let mut j = i;
        let mut ok = true;
        while j < chars.len() && qi < q.len() {
            for lc in chars[j].1.to_lowercase() {
                if qi < q.len() && q[qi] == lc {
                    qi += 1;
                } else {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
            j += 1;
        }
        if ok && qi == q.len() {
            let end = chars.get(j).map_or(text.len(), |&(b, _)| b);
            out.push((chars[i].0, end));
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}
