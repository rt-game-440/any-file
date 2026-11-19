//! 文本编辑态（工单 #11）的 Slint 绑定层：把 `EditorState` 全局与 `AppSession` 的编辑接口连接起来，
//! 并承载未保存 / 冲突 / 编码不可表示 / 无写入权限四类决策对话框、外部修改轮询与自动保存。

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyfile::session::{AppSession, PendingAction, UnsavedDecision};
use anyfile::text_editor::{ConflictKind, SaveOptions, SaveOutcome};
use anyfile::{EditorState, MainWindow};
use slint::{ComponentHandle, SharedString, Timer, TimerMode};

/// 当前展示的对话框种类。
#[derive(Debug, Clone, PartialEq)]
enum Dialog {
    None,
    Unsaved,
    Conflict(ConflictKind),
    Unrepresentable,
    NeedSaveAs,
}

struct Glue {
    window: slint::Weak<MainWindow>,
    session: Arc<AppSession>,
    resync: Rc<dyn Fn()>,
    dialog: RefCell<Dialog>,
    /// 查找 / 替换的字节游标
    find_pos: Cell<usize>,
}

/// 绑定编辑态回调并启动轮询定时器；返回的定时器须由调用方持有至程序结束。
pub fn wire(
    window: &MainWindow,
    session: &Arc<AppSession>,
    resync: Rc<dyn Fn()>,
) -> Vec<Timer> {
    let glue = Rc::new(Glue {
        window: window.as_weak(),
        session: Arc::clone(session),
        resync,
        dialog: RefCell::new(Dialog::None),
        find_pos: Cell::new(0),
    });
    let st = window.global::<EditorState>();

    macro_rules! hook {
        ($method:ident, $f:expr) => {{
            let g = Rc::clone(&glue);
            st.$method(move |a| ($f)(&g, a));
        }};
    }
    {
        let g = Rc::clone(&glue);
        st.on_begin(move || g.begin());
    }
    hook!(on_edited, |g: &Rc<Glue>, t: SharedString| g.edited(&t));
    {
        let g = Rc::clone(&glue);
        st.on_undo(move || g.history(true));
    }
    {
        let g = Rc::clone(&glue);
        st.on_redo(move || g.history(false));
    }
    {
        let g = Rc::clone(&glue);
        st.on_save(move || g.save(SaveOptions::default()));
    }
    {
        let g = Rc::clone(&glue);
        st.on_save_as(move || g.save_as(SaveOptions::default()));
    }
    {
        let g = Rc::clone(&glue);
        st.on_exit_request(move || g.exit_request());
    }
    {
        let g = Rc::clone(&glue);
        st.on_find_changed(move |q, cs| g.find_changed(&q, cs));
    }
    {
        let g = Rc::clone(&glue);
        st.on_find_next(move |q, cs| g.find_step(&q, cs, true));
    }
    {
        let g = Rc::clone(&glue);
        st.on_find_prev(move |q, cs| g.find_step(&q, cs, false));
    }
    {
        let g = Rc::clone(&glue);
        st.on_replace_one(move |q, r, cs| g.replace_one(&q, &r, cs));
    }
    {
        let g = Rc::clone(&glue);
        st.on_replace_all(move |q, r, cs| g.replace_all(&q, &r, cs));
    }
    {
        let g = Rc::clone(&glue);
        st.on_dialog_action(move |n| g.dialog_action(n));
    }

    // 100ms：待决动作 / 退出批准；每 10 个周期（约 1s）：外部修改检测与自动保存
    let tick = Rc::new(Cell::new(0u32));
    let timer = Timer::default();
    {
        let g = Rc::clone(&glue);
        timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            if g.session.take_exit_approved() {
                let _ = slint::quit_event_loop();
                return;
            }
            g.refresh_dialog();
            if !g.session.text_edit_active() {
                tick.set(0);
                return;
            }
            tick.set(tick.get() + 1);
            if tick.get().is_multiple_of(10) {
                g.periodic();
            }
        });
    }
    vec![timer]
}

/// 把会话中的编辑态同步到 `EditorState`（并给标题加未保存圆点）。可被频繁调用。
pub fn sync_state(window: &MainWindow, session: &AppSession) {
    let st = window.global::<EditorState>();
    let active = session.text_edit_active();
    st.set_active(active);
    st.set_dark_mode(window.get_dark_mode());
    st.set_font_size(session.text_font_size());
    let dirty = session.text_edit_dirty();
    st.set_dirty(dirty);
    st.set_can_undo(session.text_edit_can_undo());
    st.set_can_redo(session.text_edit_can_redo());
    if let Some((enc, ending, mixed)) = session.text_edit_meta() {
        let note = if mixed { "（混合换行将统一保存）" } else { "" };
        st.set_meta(format!("{} · {}{}", enc.label(), ending.label(), note).into());
    }
    if !active {
        st.set_status("".into());
    }
    let title = window.get_app_title().to_string();
    let base = title.trim_start_matches("● ");
    window.set_app_title(if dirty { format!("● {base}") } else { base.to_string() }.into());
}

impl Glue {
    fn win(&self) -> Option<MainWindow> {
        self.window.upgrade()
    }

    fn state_sync(&self) {
        if let Some(w) = self.win() {
            sync_state(&w, &self.session);
        }
    }

    fn begin(&self) {
        match self.session.text_edit_begin() {
            Ok(()) => {
                if let Some(w) = self.win() {
                    let st = w.global::<EditorState>();
                    st.set_text(self.session.text_edit_text().into());
                    st.set_find_status("".into());
                    st.set_status("".into());
                }
                self.find_pos.set(0);
                (self.resync)();
            }
            Err(e) => {
                if let Some(w) = self.win() {
                    w.set_text_notice(e.into());
                }
            }
        }
    }

    fn edited(&self, text: &str) {
        self.session.text_edit_set_text(text);
        self.state_sync();
    }

    fn push_text(&self) {
        if let Some(w) = self.win() {
            w.global::<EditorState>()
                .set_text(self.session.text_edit_text().into());
        }
    }

    fn history(&self, undo: bool) {
        let changed = if undo {
            self.session.text_edit_undo()
        } else {
            self.session.text_edit_redo()
        };
        if changed {
            self.push_text();
        }
        self.state_sync();
    }

    fn set_status(&self, msg: &str) {
        if let Some(w) = self.win() {
            w.global::<EditorState>().set_status(msg.into());
        }
    }

    fn save(&self, opts: SaveOptions) {
        let outcome = self.session.text_edit_save(opts);
        self.handle_outcome(outcome);
    }

    fn handle_outcome(&self, outcome: SaveOutcome) {
        match outcome {
            SaveOutcome::Saved => {
                self.set_status("");
                self.close_dialog();
            }
            SaveOutcome::Conflict(kind) => {
                let (title, text, b1) = match kind {
                    ConflictKind::Modified => (
                        "文件已被外部修改",
                        "磁盘上的文件在你编辑期间被其他程序修改。直接保存会覆盖对方的改动。",
                        "覆盖保存",
                    ),
                    ConflictKind::Deleted => (
                        "文件已被删除或移动",
                        "原文件已不存在。可以重新创建并保存，或另存为其他位置。",
                        "重新创建并保存",
                    ),
                };
                let b2 = if kind == ConflictKind::Modified { "重新载入" } else { "" };
                self.show_dialog(Dialog::Conflict(kind), title, text, b1, b2, "另存为…");
            }
            SaveOutcome::Unrepresentable(u) => {
                let enc = self
                    .session
                    .text_edit_meta()
                    .map(|m| m.0)
                    .unwrap_or(anyfile::text_viewer::TextEncoding::Utf8);
                self.show_dialog(
                    Dialog::Unrepresentable,
                    "存在无法用原编码表示的字符",
                    &u.message(enc),
                    "转为 UTF-8 保存",
                    "",
                    "",
                );
            }
            SaveOutcome::NeedSaveAs(m) => {
                self.show_dialog(
                    Dialog::NeedSaveAs,
                    "没有写入权限",
                    &format!("{m}。AnyFile 不会提升权限，请另存到有权限的位置。"),
                    "另存为…",
                    "",
                    "",
                );
            }
            SaveOutcome::Failed(m) => {
                self.close_dialog();
                self.set_status(&format!("保存失败：{m}"));
            }
        }
        (self.resync)();
    }

    fn save_as(&self, opts: SaveOptions) {
        let current = self
            .session
            .current_file_entry()
            .map(|e| e.path)
            .unwrap_or_default();
        let Some(target) = pick_save_path(&current) else {
            return;
        };
        let outcome = self.session.text_edit_save_as(&target, opts);
        if outcome == SaveOutcome::Saved {
            self.push_text();
        }
        self.handle_outcome(outcome);
    }

    fn exit_request(&self) {
        let _ = self.session.text_edit_request_exit();
        self.refresh_dialog();
        (self.resync)();
    }

    // ----- 对话框 -----

    fn show_dialog(&self, kind: Dialog, title: &str, text: &str, b1: &str, b2: &str, b3: &str) {
        *self.dialog.borrow_mut() = kind;
        if let Some(w) = self.win() {
            let st = w.global::<EditorState>();
            st.set_dialog_title(title.into());
            st.set_dialog_text(text.into());
            st.set_dialog_b1(b1.into());
            st.set_dialog_b2(b2.into());
            st.set_dialog_b3(b3.into());
            st.set_dialog(1);
        }
    }

    fn close_dialog(&self) {
        *self.dialog.borrow_mut() = Dialog::None;
        if let Some(w) = self.win() {
            w.global::<EditorState>().set_dialog(0);
        }
    }

    /// 存在被拦截的待决动作且当前没有对话框时，弹出“保存 / 不保存 / 取消”。
    fn refresh_dialog(&self) {
        if *self.dialog.borrow() != Dialog::None {
            return;
        }
        if self.session.text_edit_pending().is_some() && self.session.text_edit_dirty() {
            let what = match self.session.text_edit_pending() {
                Some(PendingAction::CloseWindow) => "关闭窗口前",
                Some(PendingAction::ExitEditing) => "退出编辑前",
                _ => "继续之前",
            };
            self.show_dialog(
                Dialog::Unsaved,
                "有未保存的修改",
                &format!("{what}是否保存对该文件的修改？"),
                "保存",
                "不保存",
                "",
            );
        }
    }

    fn dialog_action(&self, n: i32) {
        let kind = self.dialog.borrow().clone();
        if n == 0 {
            // 取消：放弃被拦截的动作，留在编辑态
            self.session.text_edit_cancel();
            self.close_dialog();
            self.state_sync();
            return;
        }
        match (kind, n) {
            (Dialog::Unsaved, 1) => {
                self.close_dialog();
                if let Some(outcome) = self.session.text_edit_decide(UnsavedDecision::Save) {
                    self.handle_outcome(outcome);
                }
            }
            (Dialog::Unsaved, 2) => {
                self.close_dialog();
                self.session.text_edit_decide(UnsavedDecision::Discard);
                (self.resync)();
            }
            (Dialog::Conflict(_), 1) => {
                self.close_dialog();
                self.save(SaveOptions { force: true, ..Default::default() });
            }
            (Dialog::Conflict(_), 2) => {
                self.close_dialog();
                if let Err(e) = self.session.text_edit_reload() {
                    self.set_status(&format!("重新载入失败：{e}"));
                }
                self.push_text();
                (self.resync)();
            }
            (Dialog::Conflict(_), 3) | (Dialog::NeedSaveAs, 1) => {
                self.close_dialog();
                self.save_as(SaveOptions::default());
            }
            (Dialog::Unrepresentable, 1) => {
                self.close_dialog();
                self.save(SaveOptions { convert_to_utf8: true, ..Default::default() });
            }
            _ => self.close_dialog(),
        }
    }

    // ----- 周期任务 -----

    fn periodic(&self) {
        if let Some(kind) = self.session.text_edit_check_external(false) {
            let msg = match kind {
                ConflictKind::Modified => "磁盘文件已被外部修改：保存时将询问如何处理",
                ConflictKind::Deleted => "磁盘文件已被删除或移动：保存时将询问如何处理",
            };
            self.set_status(msg);
        }
        let autosave = self
            .win()
            .is_some_and(|w| w.global::<EditorState>().get_autosave());
        if autosave && let Some(outcome) = self.session.text_edit_autosave_tick() {
            match outcome {
                SaveOutcome::Saved => self.set_status(""),
                SaveOutcome::Conflict(_) => self.set_status("自动保存已暂停：磁盘文件发生外部变化"),
                SaveOutcome::Unrepresentable(_) => {
                    self.set_status("自动保存已暂停：含原编码无法表示的字符，请手动保存")
                }
                SaveOutcome::NeedSaveAs(m) | SaveOutcome::Failed(m) => {
                    self.set_status(&format!("自动保存失败：{m}"))
                }
            }
            self.state_sync();
        }
    }

    // ----- 查找 / 替换 -----

    fn find_changed(&self, query: &str, cs: bool) {
        let n = self.session.text_edit_find_all(query, cs).len();
        let status = if query.is_empty() {
            String::new()
        } else if n == 0 {
            "无匹配".to_string()
        } else {
            format!("{n} 处匹配")
        };
        if let Some(w) = self.win() {
            w.global::<EditorState>().set_find_status(status.into());
        }
    }

    fn select(&self, s: usize, e: usize) {
        if let Some(w) = self.win() {
            let st = w.global::<EditorState>();
            st.set_sel_start(s as i32);
            st.set_sel_end(e as i32);
            st.set_sel_tick(st.get_sel_tick() + 1);
        }
    }

    fn find_step(&self, query: &str, cs: bool, forward: bool) {
        let ranges = self.session.text_edit_find_all(query, cs);
        self.find_changed(query, cs);
        if ranges.is_empty() {
            return;
        }
        let pos = self.find_pos.get();
        let hit = if forward {
            ranges.iter().find(|(s, _)| *s >= pos).or(ranges.first())
        } else {
            let cur = ranges.iter().rposition(|(s, _)| *s < pos.saturating_sub(1));
            cur.map(|i| &ranges[i]).or(ranges.last())
        };
        if let Some(&(s, e)) = hit {
            self.find_pos.set(if forward { e } else { s });
            self.select(s, e);
        }
    }

    fn replace_one(&self, query: &str, replacement: &str, cs: bool) {
        if let Some((s, e)) = self.session.text_edit_replace_next(
            query,
            replacement,
            cs,
            self.find_pos.get(),
        ) {
            self.push_text();
            self.find_pos.set(e);
            self.select(s, e);
            self.find_changed(query, cs);
        }
        self.state_sync();
    }

    fn replace_all(&self, query: &str, replacement: &str, cs: bool) {
        let n = self.session.text_edit_replace_all(query, replacement, cs);
        if n > 0 {
            self.push_text();
        }
        if let Some(w) = self.win() {
            w.global::<EditorState>()
                .set_find_status(format!("已替换 {n} 处").into());
        }
        self.state_sync();
    }
}

/// 弹出系统“另存为”对话框；取消或非 Windows 返回 `None`。
#[cfg(windows)]
fn pick_save_path(current: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetSaveFileNameW, OFN_EXPLORER, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let mut buf = [0u16; 1024];
    if let Some(name) = current.file_name() {
        for (i, c) in name.encode_wide().take(buf.len() - 1).enumerate() {
            buf[i] = c;
        }
    }
    let dir: Vec<u16> = current
        .parent()
        .map(|p| p.as_os_str().encode_wide().chain(Some(0)).collect())
        .unwrap_or_else(|| vec![0]);
    // 过滤器：成对以 0 结尾，整体以双 0 结束
    let filter: Vec<u16> = "所有文件 (*.*)\0*.*\0\0".encode_utf16().collect();

    // SAFETY: OPENFILENAMEW 全零初始化后按文档填写必需字段；所有缓冲区在调用期间保持有效。
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrInitialDir = dir.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
    // SAFETY: ofn 已正确初始化。
    let ok = unsafe { GetSaveFileNameW(&mut ofn) };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])))
}

#[cfg(not(windows))]
fn pick_save_path(_current: &Path) -> Option<PathBuf> {
    None
}
