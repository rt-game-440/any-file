//! `AppSession` 的文本编辑态接口（工单 #11）：进入 / 退出编辑、撤销重做、查找替换、
//! 原子保存、外部修改检测、自动保存，以及“存在未保存修改时被拦截的动作”的决策流程。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::AppSession;
use crate::domain::{TextMode, ViewerKind};
use crate::text_editor::{ConflictKind, SaveOptions, SaveOutcome, TextEditor};
use crate::text_viewer::{LineEnding, TextEncoding, TextFindState};

/// 因存在未保存修改而被拦截的错误文案（UI 据此展示“保存 / 放弃 / 取消”对话框）。
pub const UNSAVED_CHANGES_ERROR: &str = "存在未保存的修改";

/// 被未保存修改拦截、等待用户决策的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    NavigatePrev,
    NavigateNext,
    OpenFile(PathBuf),
    SwitchViewer(ViewerKind),
    /// 在编辑态按 Esc 退出编辑
    ExitEditing,
    CloseWindow,
}

/// 用户对未保存修改的决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedDecision {
    Save,
    Discard,
    Cancel,
}

impl AppSession {
    /// 当前是否处于文本编辑态。
    pub fn text_edit_active(&self) -> bool {
        self.editor.lock().unwrap().is_some()
    }

    /// 编辑缓冲区是否有未保存修改。
    pub fn text_edit_dirty(&self) -> bool {
        self.editor
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(TextEditor::is_dirty)
    }

    /// 动作入口守卫：存在未保存修改时记录待决动作并返回错误。
    pub(super) fn guard_unsaved(&self, action: PendingAction) -> Result<(), String> {
        if self.text_edit_dirty() {
            *self.pending.lock().unwrap() = Some(action);
            return Err(UNSAVED_CHANGES_ERROR.to_string());
        }
        Ok(())
    }

    /// 待决动作（用于 UI 展示未保存对话框）。
    pub fn text_edit_pending(&self) -> Option<PendingAction> {
        self.pending.lock().unwrap().clone()
    }

    /// 取走“用户已确认退出”标记；为真时 UI 层应真正关闭窗口。
    pub fn take_exit_approved(&self) -> bool {
        self.exit_approved.swap(false, Ordering::SeqCst)
    }

    /// 进入编辑态。大文件只读保护、非文本视图或无法无损解码时拒绝。
    pub fn text_edit_begin(&self) -> Result<(), String> {
        if self.active_viewer() != ViewerKind::Text {
            return Err("当前不是文本视图".to_string());
        }
        if self.text_is_readonly_protected() {
            return Err("大文件只读保护：已禁用编辑".to_string());
        }
        if self.text_edit_active() {
            return Ok(());
        }
        let path = self
            .current_file_entry()
            .map(|e| e.path)
            .ok_or_else(|| "当前未打开任何文件项".to_string())?;
        let editor = TextEditor::open(&path)?;
        *self.editor.lock().unwrap() = Some(editor);
        let mut guard = self.state.write().unwrap();
        guard.text_mode = TextMode::Editing;
        guard.text_find = TextFindState::default();
        Ok(())
    }

    /// 请求退出编辑态（Esc / 按钮）：无未保存修改直接退出；否则记录待决动作并返回错误。
    pub fn text_edit_request_exit(&self) -> Result<(), String> {
        self.guard_unsaved(PendingAction::ExitEditing)?;
        self.exit_editing();
        Ok(())
    }

    /// 直接退出编辑态并按磁盘最新内容刷新查看态（调用前须已处理未保存修改）。
    fn exit_editing(&self) {
        *self.editor.lock().unwrap() = None;
        let path = self.current_file_entry().map(|e| e.path);
        let doc = path.and_then(|p| Self::load_text_document(&p));
        let mut guard = self.state.write().unwrap();
        guard.text_mode = TextMode::Viewing;
        guard.has_unsaved_changes = false;
        if doc.is_some() {
            guard.text_document = doc;
        }
        guard.text_find = TextFindState::default();
    }

    fn with_editor<R>(&self, f: impl FnOnce(&mut TextEditor) -> R) -> Option<R> {
        self.editor.lock().unwrap().as_mut().map(f)
    }

    fn sync_dirty_flag(&self) {
        let dirty = self.text_edit_dirty();
        self.state.write().unwrap().has_unsaved_changes = dirty;
    }

    /// 编辑缓冲区全文（`\n` 换行）。
    pub fn text_edit_text(&self) -> String {
        self.with_editor(|e| e.text().to_string()).unwrap_or_default()
    }

    /// 同步用户输入后的全文；返回是否变化。
    pub fn text_edit_set_text(&self, text: &str) -> bool {
        let now = self.clock.now();
        let changed = self
            .with_editor(|e| e.set_text(text, now))
            .unwrap_or(false);
        self.sync_dirty_flag();
        changed
    }

    pub fn text_edit_undo(&self) -> bool {
        let now = self.clock.now();
        let r = self.with_editor(|e| e.undo(now)).unwrap_or(false);
        self.sync_dirty_flag();
        r
    }

    pub fn text_edit_redo(&self) -> bool {
        let now = self.clock.now();
        let r = self.with_editor(|e| e.redo(now)).unwrap_or(false);
        self.sync_dirty_flag();
        r
    }

    pub fn text_edit_can_undo(&self) -> bool {
        self.with_editor(|e| e.can_undo()).unwrap_or(false)
    }

    pub fn text_edit_can_redo(&self) -> bool {
        self.with_editor(|e| e.can_redo()).unwrap_or(false)
    }

    /// 查找所有匹配（字节区间）。
    pub fn text_edit_find_all(&self, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
        self.with_editor(|e| e.find_all(query, case_sensitive))
            .unwrap_or_default()
    }

    /// 替换 `from_byte` 起的下一处匹配；返回替换后应选中的字节区间。
    pub fn text_edit_replace_next(
        &self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
        from_byte: usize,
    ) -> Option<(usize, usize)> {
        let now = self.clock.now();
        let r = self
            .with_editor(|e| e.replace_next(query, replacement, case_sensitive, from_byte, now))
            .flatten();
        self.sync_dirty_flag();
        r
    }

    /// 全部替换；返回替换数量。
    pub fn text_edit_replace_all(
        &self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
    ) -> usize {
        let now = self.clock.now();
        let n = self
            .with_editor(|e| e.replace_all(query, replacement, case_sensitive, now))
            .unwrap_or(0);
        self.sync_dirty_flag();
        n
    }

    /// 编辑态状态栏信息：编码 / 换行符 / 提示。
    pub fn text_edit_meta(&self) -> Option<(TextEncoding, LineEnding, bool)> {
        self.with_editor(|e| (e.encoding(), e.line_ending(), e.original_mixed()))
    }

    /// 轮询外部修改（轻量；`deep` 时比较内容哈希）。
    pub fn text_edit_check_external(&self, deep: bool) -> Option<ConflictKind> {
        self.with_editor(|e| e.check_external(deep)).flatten()
    }

    pub fn text_edit_external_conflict(&self) -> Option<ConflictKind> {
        self.with_editor(|e| e.external_conflict()).flatten()
    }

    /// 保存到当前路径。成功后若存在待决动作则继续执行之。
    pub fn text_edit_save(&self, opts: SaveOptions) -> SaveOutcome {
        let Some(outcome) = self.with_editor(|e| e.save(opts)) else {
            return SaveOutcome::Failed("当前不在编辑态".to_string());
        };
        self.after_save(&outcome);
        outcome
    }

    /// 另存为新路径。成功后改为跟踪新文件（重新载入并保持编辑态），随后继续待决动作。
    pub fn text_edit_save_as(&self, path: &Path, opts: SaveOptions) -> SaveOutcome {
        let Some(outcome) = self.with_editor(|e| e.save_as(path, opts)) else {
            return SaveOutcome::Failed("当前不在编辑态".to_string());
        };
        if outcome == SaveOutcome::Saved {
            // 编辑器已改为跟踪新路径并视为已保存；刷新会话中的当前文件项
            let _ = self.open_file_internal(path, false);
            let _ = self.text_edit_begin();
        }
        self.after_save(&outcome);
        outcome
    }

    fn after_save(&self, outcome: &SaveOutcome) {
        self.sync_dirty_flag();
        if *outcome == SaveOutcome::Saved {
            self.run_pending();
        }
    }

    /// 自动保存：到期且无冲突时静默保存；返回保存结果（未到期为 `None`）。
    pub fn text_edit_autosave_tick(&self) -> Option<SaveOutcome> {
        let now = self.clock.now();
        let due = self.with_editor(|e| e.autosave_due(now)).unwrap_or(false);
        if !due {
            return None;
        }
        // 自动保存不处理待决动作，也不弹决策：失败结果交给调用方展示
        let outcome = self.with_editor(|e| e.save(SaveOptions::default()))?;
        self.sync_dirty_flag();
        Some(outcome)
    }

    /// 用户对未保存修改作出决策（保存会走 [`Self::text_edit_save`] 流程，可能返回冲突等结果）。
    pub fn text_edit_decide(&self, decision: UnsavedDecision) -> Option<SaveOutcome> {
        match decision {
            UnsavedDecision::Save => Some(self.text_edit_save(SaveOptions::default())),
            UnsavedDecision::Discard => {
                self.text_edit_discard();
                None
            }
            UnsavedDecision::Cancel => {
                self.text_edit_cancel();
                None
            }
        }
    }

    /// 放弃未保存修改并继续待决动作。
    pub fn text_edit_discard(&self) {
        *self.editor.lock().unwrap() = None;
        self.state.write().unwrap().has_unsaved_changes = false;
        self.exit_editing();
        self.run_pending();
    }

    /// 取消待决动作，保持编辑态。
    pub fn text_edit_cancel(&self) {
        *self.pending.lock().unwrap() = None;
    }

    /// 放弃缓冲区并重新从磁盘载入（外部修改冲突的“重新载入”）。
    pub fn text_edit_reload(&self) -> Result<(), String> {
        let r = self
            .with_editor(|e| e.reload())
            .unwrap_or_else(|| Err("当前不在编辑态".to_string()));
        self.sync_dirty_flag();
        if r.is_ok() {
            *self.pending.lock().unwrap() = None;
        }
        r
    }

    fn run_pending(&self) {
        let Some(action) = self.pending.lock().unwrap().take() else {
            return;
        };
        match action {
            PendingAction::NavigatePrev => {
                let _ = self.navigate_prev();
            }
            PendingAction::NavigateNext => {
                let _ = self.navigate_next();
            }
            PendingAction::OpenFile(p) => {
                let _ = self.open_file(&p);
            }
            PendingAction::SwitchViewer(v) => {
                let _ = self.switch_viewer(v);
            }
            PendingAction::ExitEditing => self.exit_editing(),
            PendingAction::CloseWindow => self.exit_approved.store(true, Ordering::SeqCst),
        }
    }
}
