//! 文本查看器与 Slint 的桥接：按行懒构建可见行的着色分段，避免一次性转换整份文档。

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};

use super::{TextDocument, TextFindState, TextLine};

/// 着色分段（与 `TextSegment` 一一对应）。`highlight`：0 无，1 查找匹配，2 当前匹配。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSegment {
    pub text: String,
    pub kind: i32,
    pub highlight: i32,
}

/// 单行内的查找匹配 `(start, end, is_current)`（字节区间）。
pub type LineMatch = (usize, usize, bool);

/// 把一行按分词与查找匹配边界切成着色分段。
pub fn build_row_segments(line: &TextLine, matches: &[LineMatch]) -> Vec<RowSegment> {
    let text = line.display_text.as_str();
    if text.is_empty() {
        return Vec::new();
    }
    if line.tokens.is_empty() && matches.is_empty() {
        return vec![RowSegment {
            text: text.to_string(),
            kind: 0,
            highlight: 0,
        }];
    }

    let mut points: Vec<usize> = vec![0, text.len()];
    for t in &line.tokens {
        points.push(t.start);
        points.push(t.end);
    }
    for &(s, e, _) in matches {
        points.push(s);
        points.push(e);
    }
    points.retain(|&p| p <= text.len() && text.is_char_boundary(p));
    points.sort_unstable();
    points.dedup();

    let mut out = Vec::with_capacity(points.len());
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let kind = line
            .tokens
            .iter()
            .find(|t| t.start <= a && a < t.end)
            .map_or(0, |t| t.kind as i32);
        let highlight = matches
            .iter()
            .find(|&&(s, e, _)| s <= a && a < e)
            .map_or(0, |&(_, _, cur)| if cur { 2 } else { 1 });
        out.push(RowSegment {
            text: text[a..b].to_string(),
            kind,
            highlight,
        });
    }
    out
}

/// 按行索引归组查找匹配。
pub fn group_matches_by_line(find: &TextFindState) -> HashMap<usize, Vec<LineMatch>> {
    let mut map: HashMap<usize, Vec<LineMatch>> = HashMap::new();
    for (i, m) in find.matches.iter().enumerate() {
        map.entry(m.line_index)
            .or_default()
            .push((m.start, m.end, find.current == Some(i)));
    }
    map
}

/// 懒加载行模型：仅在 `ListView` 请求可见行时才构建分段。
pub struct TextLinesModel {
    doc: Arc<TextDocument>,
    matches: HashMap<usize, Vec<LineMatch>>,
    notify: ModelNotify,
}

impl TextLinesModel {
    pub fn new(doc: Arc<TextDocument>, find: &TextFindState) -> Self {
        Self {
            doc,
            matches: group_matches_by_line(find),
            notify: ModelNotify::default(),
        }
    }
}

impl Model for TextLinesModel {
    type Data = crate::TextRow;

    fn row_count(&self) -> usize {
        self.doc.lines.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let line = self.doc.lines.get(row)?;
        let empty = Vec::new();
        let matches = self.matches.get(&row).unwrap_or(&empty);
        let segments: Vec<crate::TextSegment> = build_row_segments(line, matches)
            .into_iter()
            .map(|s| crate::TextSegment {
                text: SharedString::from(s.text),
                kind: s.kind,
                highlight: s.highlight,
            })
            .collect();
        Some(crate::TextRow {
            number: line.line_number as i32,
            segments: ModelRc::new(VecModel::from(segments)),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
