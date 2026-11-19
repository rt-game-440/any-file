//! `AppSession`：位于 Slint UI 绑定层正下方的无头应用核心控制器。
//! 统一编排文件项打开、媒体大类分发、同目录序列、文本双模式、动效配置与系统偏好同步。

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Instant;

use crate::config::{AppConfig, ConfigStore, RecentEntry};
use crate::domain::{
    CloseDecision, FallbackInfo, FileEntry, MediaCategory, MotionProfile, ResolvedTheme,
    SiblingSequence, TextMode, ThemePreference, ViewerKind,
};
use crate::i18n::{Language, LanguagePreference};
use crate::motion::{MotionChannel, MotionEngine, MotionSnapshot};
use crate::ports::{Clock, SystemShell, VideoBackend};
use crate::probing;
use crate::sibling::{self, DirectoryWatcher};
use crate::text_viewer::{
    self, DEFAULT_FONT_SIZE, FindMatch, TextDocument, TextFindState, find_in_document,
};

/// `AppSession` 内部快照状态
#[derive(Debug, Clone)]
struct SessionState {
    active_viewer: ViewerKind,
    current_file: Option<FileEntry>,
    detected_format: String,
    available_viewers: Vec<ViewerKind>,
    fallback_info: Option<FallbackInfo>,
    sibling_sequence: SiblingSequence,
    /// 当前同目录序列的同类筛选锚点大类（切过损坏文件时保留原大类上下文）
    sibling_filter_category: Option<MediaCategory>,
    text_mode: TextMode,
    /// 当前文本文档（仅文本查看器激活时加载）
    text_document: Option<Arc<TextDocument>>,
    text_find: TextFindState,
    text_font_size: f32,
    has_unsaved_changes: bool,
    theme_preference: ThemePreference,
    language_preference: LanguagePreference,
    auto_save: bool,
    recent_enabled: bool,
    resolved_theme: ResolvedTheme,
    motion_profile: MotionProfile,
    motion_engine: MotionEngine,
    last_motion_tick: Instant,
    started_at: Instant,
}

/// 无头应用会话控制器（测试与 UI 绑定的主接缝）
pub struct AppSession {
    shell: Arc<dyn SystemShell>,
    clock: Arc<dyn Clock>,
    video_backend: Arc<dyn VideoBackend>,
    state: Arc<RwLock<SessionState>>,
    watcher: Mutex<Option<DirectoryWatcher>>,
    store: Option<ConfigStore>,
}

impl AppSession {
    /// 使用外部边界依赖创建新的无头会话，默认进入 `ViewerKind::Welcome` 空态。
    pub fn new(
        shell: Arc<dyn SystemShell>,
        clock: Arc<dyn Clock>,
        video_backend: Arc<dyn VideoBackend>,
    ) -> Self {
        let resolved_theme = shell.system_theme();
        let reduced_motion = shell.prefers_reduced_motion();
        let motion_profile = MotionProfile::for_system_setting(reduced_motion);
        let started_at = clock.now();

        let mut motion_engine = MotionEngine::new(reduced_motion);
        motion_engine.set_immediate(MotionChannel::ViewTransition, 1.0);
        motion_engine.set_immediate(MotionChannel::ButtonScale, 1.0);
        motion_engine.set_immediate(MotionChannel::ChromeOpacity, 1.0);
        motion_engine.animate_from_to(
            MotionChannel::WelcomeEntrance,
            0.0,
            1.0,
            MotionProfile::smooth(),
        );

        Self {
            shell,
            clock,
            video_backend,
            state: Arc::new(RwLock::new(SessionState {
                active_viewer: ViewerKind::Welcome,
                current_file: None,
                detected_format: String::new(),
                available_viewers: Vec::new(),
                fallback_info: None,
                sibling_sequence: SiblingSequence::empty(),
                sibling_filter_category: None,
                text_mode: TextMode::Viewing,
                text_document: None,
                text_find: TextFindState::default(),
                text_font_size: DEFAULT_FONT_SIZE,
                has_unsaved_changes: false,
                theme_preference: ThemePreference::System,
                language_preference: LanguagePreference::System,
                auto_save: false,
                recent_enabled: true,
                resolved_theme,
                motion_profile,
                motion_engine,
                last_motion_tick: started_at,
                started_at,
            })),
            watcher: Mutex::new(None),
            store: None,
        }
    }

    /// 挂接持久化存储并载入已保存的偏好（主题、语言、自动保存、最近记录开关）。
    pub fn with_config_store(mut self, store: ConfigStore) -> Self {
        if let Ok(cfg) = store.load_config() {
            self.apply_config(&cfg);
        }
        self.store = Some(store);
        self
    }

    fn apply_config(&self, cfg: &AppConfig) {
        let system_theme = self.shell.system_theme();
        let mut g = self.state.write().unwrap();
        g.theme_preference = cfg.theme;
        g.resolved_theme = match cfg.theme {
            ThemePreference::System => system_theme,
            ThemePreference::Light => ResolvedTheme::Light,
            ThemePreference::Dark => ResolvedTheme::Dark,
        };
        g.language_preference = cfg.language;
        g.auto_save = cfg.auto_save;
        g.recent_enabled = cfg.recent_enabled;
    }

    fn persist(&self, f: impl FnOnce(&mut AppConfig)) {
        if let Some(store) = &self.store
            && let Err(e) = store.update_config(f)
        {
            crate::diagnostics::error(&format!("保存配置失败：{e}"));
        }
    }

    pub fn language_preference(&self) -> LanguagePreference {
        self.state.read().unwrap().language_preference
    }

    /// 当前实际界面语言（默认跟随系统语言）
    pub fn language(&self) -> Language {
        self.language_preference()
            .resolve(&self.shell.system_language())
    }

    pub fn set_language_preference(&self, pref: LanguagePreference) {
        self.state.write().unwrap().language_preference = pref;
        self.persist(|c| c.language = pref);
    }

    pub fn auto_save(&self) -> bool {
        self.state.read().unwrap().auto_save
    }

    pub fn set_auto_save(&self, on: bool) {
        self.state.write().unwrap().auto_save = on;
        self.persist(|c| c.auto_save = on);
    }

    pub fn recent_enabled(&self) -> bool {
        self.state.read().unwrap().recent_enabled
    }

    /// 开关最近记录；关闭时彻底清空已有记录。
    pub fn set_recent_enabled(&self, on: bool) {
        self.state.write().unwrap().recent_enabled = on;
        if let Some(store) = &self.store
            && let Err(e) = store.set_recent_enabled(on)
        {
            crate::diagnostics::error(&format!("保存最近记录开关失败：{e}"));
        }
    }

    pub fn recent_files(&self) -> Vec<RecentEntry> {
        self.store
            .as_ref()
            .and_then(|s| s.recent().ok())
            .unwrap_or_default()
    }

    pub fn clear_recent(&self) {
        if let Some(store) = &self.store {
            let _ = store.clear_recent();
        }
    }

    /// 复制当前文件的完整路径到剪贴板
    pub fn copy_current_path(&self) -> Result<(), String> {
        let path = self.require_current_path()?;
        self.shell.copy_text_to_clipboard(&path.to_string_lossy())
    }

    /// 在资源管理器中显示当前文件
    pub fn reveal_current_in_explorer(&self) -> Result<(), String> {
        let path = self.require_current_path()?;
        self.shell.reveal_in_explorer(&path)
    }

    fn require_current_path(&self) -> Result<std::path::PathBuf, String> {
        self.state
            .read()
            .unwrap()
            .current_file
            .as_ref()
            .map(|e| e.path.clone())
            .ok_or_else(|| "当前未打开任何文件项".to_string())
    }

    /// 将当前文件移入回收站并同步同目录序列：优先切到下一项，否则上一项，否则回到欢迎空态。
    pub fn delete_current_file(&self) -> Result<Option<FileEntry>, String> {
        let (path, next) = {
            let g = self.state.read().unwrap();
            let cur = g
                .current_file
                .as_ref()
                .ok_or_else(|| "当前未打开任何文件项".to_string())?;
            let seq = &g.sibling_sequence;
            let next = seq.current_index.and_then(|i| {
                seq.entries
                    .get(i + 1)
                    .or_else(|| i.checked_sub(1).and_then(|j| seq.entries.get(j)))
                    .map(|e| e.path.clone())
            });
            (cur.path.clone(), next)
        };
        self.shell.move_to_trash(&path)?;
        if let Some(store) = &self.store {
            let _ = store.remove_recent(&path);
        }
        crate::diagnostics::info("已将文件移入回收站");
        if let Some(n) = next
            && let Ok(entry) = self.open_file_internal(&n, true)
        {
            return Ok(Some(entry));
        }
        let mut g = self.state.write().unwrap();
        g.current_file = None;
        g.active_viewer = ViewerKind::Welcome;
        g.detected_format.clear();
        g.available_viewers.clear();
        g.fallback_info = None;
        g.sibling_sequence = SiblingSequence::empty();
        g.sibling_filter_category = None;
        g.has_unsaved_changes = false;
        Ok(None)
    }

    /// 获取当前激活的查看器视图
    pub fn active_viewer(&self) -> ViewerKind {
        self.state.read().unwrap().active_viewer
    }

    /// 获取当前打开的文件项（空态时返回 `None`）
    pub fn current_file_entry(&self) -> Option<FileEntry> {
        self.state.read().unwrap().current_file.clone()
    }

    /// 获取当前同目录序列快照
    pub fn sibling_sequence(&self) -> SiblingSequence {
        self.state.read().unwrap().sibling_sequence.clone()
    }

    /// 获取当前兜底信息卡数据（仅在 `ViewerKind::Unsupported` 时为 `Some`）
    pub fn fallback_info(&self) -> Option<FallbackInfo> {
        self.state.read().unwrap().fallback_info.clone()
    }

    /// 获取当前文件识别出的格式标签（如 `PNG`、`SVG`、`UTF-8 文本`、`纯二进制`）
    pub fn detected_format(&self) -> String {
        self.state.read().unwrap().detected_format.clone()
    }

    /// 获取当前文件支持手动切换的查看器列表（如 `.svg` 为 `[Image, Text]`，`.ts` 为 `[Text, Video]`）
    pub fn available_viewers(&self) -> Vec<ViewerKind> {
        self.state.read().unwrap().available_viewers.clone()
    }

    /// 获取当前文本模式（查看态 / 编辑态）
    pub fn text_mode(&self) -> TextMode {
        self.state.read().unwrap().text_mode
    }

    /// 获取当前文本文档（仅文本查看器激活或曾切到文本时存在）
    pub fn text_viewer(&self) -> Option<Arc<TextDocument>> {
        self.state.read().unwrap().text_document.clone()
    }

    fn load_text_document(path: &Path) -> Option<Arc<TextDocument>> {
        TextDocument::load(path).ok().map(Arc::new)
    }

    /// 当前文本是否处于大文件只读保护（编辑态应据此禁用一键编辑）
    pub fn text_is_readonly_protected(&self) -> bool {
        self.state
            .read()
            .unwrap()
            .text_document
            .as_ref()
            .is_some_and(|d| d.meta.is_large_file_readonly)
    }

    /// 页内查找（Ctrl+F）：设置查询并跳到第一个匹配；空查询清除高亮。
    pub fn text_find(&self, query: &str, case_sensitive: bool) -> TextFindState {
        let mut guard = self.state.write().unwrap();
        let state = match guard.text_document.as_ref() {
            Some(doc) => find_in_document(doc, query, case_sensitive),
            None => TextFindState::default(),
        };
        guard.text_find = state.clone();
        state
    }

    /// 跳到下一个匹配（循环），返回该匹配
    pub fn text_find_next(&self) -> Option<FindMatch> {
        self.state.write().unwrap().text_find.next_match()
    }

    /// 跳到上一个匹配（循环），返回该匹配
    pub fn text_find_prev(&self) -> Option<FindMatch> {
        self.state.write().unwrap().text_find.prev_match()
    }

    /// 当前页内查找状态快照
    pub fn text_find_state(&self) -> TextFindState {
        self.state.read().unwrap().text_find.clone()
    }

    /// 关闭查找并清除高亮
    pub fn text_find_clear(&self) {
        self.state.write().unwrap().text_find = TextFindState::default();
    }

    /// 当前文本字号（px）
    pub fn text_font_size(&self) -> f32 {
        self.state.read().unwrap().text_font_size
    }

    /// Ctrl+滚轮缩放字号：未按 Ctrl 不变；返回新字号
    pub fn text_zoom_by_wheel(&self, delta_y: f32, ctrl_pressed: bool) -> f32 {
        let mut guard = self.state.write().unwrap();
        guard.text_font_size =
            text_viewer::zoom_for_wheel(guard.text_font_size, delta_y, ctrl_pressed);
        guard.text_font_size
    }

    /// 重置字号为默认值
    pub fn text_zoom_reset(&self) -> f32 {
        let mut guard = self.state.write().unwrap();
        guard.text_font_size = DEFAULT_FONT_SIZE;
        guard.text_font_size
    }

    /// 获取当前解析后的深浅色主题
    pub fn resolved_theme(&self) -> ResolvedTheme {
        self.state.read().unwrap().resolved_theme
    }

    /// 获取当前主题偏好设置
    pub fn theme_preference(&self) -> ThemePreference {
        self.state.read().unwrap().theme_preference
    }

    /// 获取当前物理动效配置
    pub fn motion_profile(&self) -> MotionProfile {
        self.state.read().unwrap().motion_profile
    }

    /// 获取当前动效状态快照（不推进时钟）
    pub fn motion_snapshot(&self) -> MotionSnapshot {
        self.state.read().unwrap().motion_engine.snapshot()
    }

    /// 当前是否存在仍在运行中的物理弹簧动画
    pub fn has_active_animations(&self) -> bool {
        self.state
            .read()
            .unwrap()
            .motion_engine
            .has_active_animations()
    }

    /// 依据注入的 `Clock` 计算真实 `dt` 并推进一步物理弹簧解算，返回最新快照。
    pub fn tick_motion(&self) -> MotionSnapshot {
        let now = self.clock.now();
        let mut guard = self.state.write().unwrap();
        let dt = now.saturating_duration_since(guard.last_motion_tick);
        guard.last_motion_tick = now;
        guard.motion_engine.advance(dt);
        guard.motion_engine.snapshot()
    }

    /// 驱动指定动效通道向目标值过渡（支持中途打断与速度继承）。
    pub fn animate_channel(&self, channel: MotionChannel, target: f32, profile: MotionProfile) {
        let now = self.clock.now();
        let mut guard = self.state.write().unwrap();
        if !guard.motion_engine.has_active_animations() {
            guard.last_motion_tick = now;
        }
        guard.motion_engine.animate_to(channel, target, profile);
    }

    /// 触发一次标准视图切换过渡（`0.0 -> 1.0`）；减弱动效模式下立即落到 `1.0`。
    pub fn trigger_view_transition(&self) {
        let now = self.clock.now();
        let mut guard = self.state.write().unwrap();
        if !guard.motion_engine.has_active_animations() {
            guard.last_motion_tick = now;
        }
        guard.motion_engine.animate_from_to(
            MotionChannel::ViewTransition,
            0.0,
            1.0,
            MotionProfile::snappy(),
        );
    }

    /// 查询视频后端是否可用
    pub fn is_video_backend_available(&self) -> bool {
        self.video_backend.is_available()
    }

    /// 获取会话自创建以来经过的毫秒数（基于注入的 `Clock`）
    pub fn uptime_ms(&self) -> u64 {
        let started_at = self.state.read().unwrap().started_at;
        self.clock
            .now()
            .saturating_duration_since(started_at)
            .as_millis() as u64
    }

    /// 更改主题偏好并立即重新解析最终主题
    pub fn set_theme_preference(&self, preference: ThemePreference) {
        let system_theme = self.shell.system_theme();
        let mut guard = self.state.write().unwrap();
        guard.theme_preference = preference;
        guard.resolved_theme = match preference {
            ThemePreference::System => system_theme,
            ThemePreference::Light => ResolvedTheme::Light,
            ThemePreference::Dark => ResolvedTheme::Dark,
        };
        drop(guard);
        self.persist(|c| c.theme = preference);
    }

    /// 重新从 `SystemShell` 同步系统深浅色主题与“减弱动态效果”设置
    pub fn refresh_system_preferences(&self) {
        let system_theme = self.shell.system_theme();
        let reduced_motion = self.shell.prefers_reduced_motion();
        let mut guard = self.state.write().unwrap();
        if guard.theme_preference == ThemePreference::System {
            guard.resolved_theme = system_theme;
        }
        guard.motion_profile = MotionProfile::for_system_setting(reduced_motion);
        guard.motion_engine.set_reduced_motion(reduced_motion);
    }

    /// 打开指定路径的文件项：执行 Magic Bytes / 流信息 / 编码懒探测，并按扩展名构建同目录序列（ADR-0006）。
    pub fn open_file(&self, path: &Path) -> Result<FileEntry, String> {
        self.open_file_internal(path, false)
    }

    /// 拖入文件替换当前视图（行为等同于打开新文件项并刷新同目录序列）。
    pub fn drop_file(&self, path: &Path) -> Result<FileEntry, String> {
        self.open_file(path)
    }

    /// 手动“以文本打开”当前文件项（便于在 `.svg` / `.ts` 等歧义格式下查看源码）。
    pub fn open_as_text(&self) -> Result<ViewerKind, String> {
        self.switch_viewer(ViewerKind::Text)
    }

    /// 手动切换当前文件项的查看器视图（不允许切回 `Welcome` 或在未打开文件时切换）。
    pub fn switch_viewer(&self, target: ViewerKind) -> Result<ViewerKind, String> {
        if target == ViewerKind::Welcome {
            return Err("无法手动切换到欢迎空态".to_string());
        }

        let path = {
            let guard = self.state.read().unwrap();
            guard
                .current_file
                .as_ref()
                .map(|e| e.path.clone())
                .ok_or_else(|| "当前未打开任何文件项".to_string())?
        };
        let loaded = (target == ViewerKind::Text).then(|| Self::load_text_document(&path));

        let mut guard = self.state.write().unwrap();
        guard.active_viewer = target;
        if let Some(doc) = loaded {
            guard.text_mode = TextMode::Viewing;
            guard.text_document = doc;
            guard.text_find = TextFindState::default();
        }
        if !guard.available_viewers.contains(&target) {
            guard.available_viewers.push(target);
        }
        Ok(target)
    }

    /// 在当前文件可用的查看器之间循环切换（工具栏“切换查看器”快捷入口）。
    pub fn cycle_viewer(&self) -> Result<ViewerKind, String> {
        let next = {
            let guard = self.state.read().unwrap();
            if guard.current_file.is_none() {
                return Err("当前未打开任何文件项".to_string());
            }
            if guard.available_viewers.len() > 1 {
                let pos = guard
                    .available_viewers
                    .iter()
                    .position(|&v| v == guard.active_viewer)
                    .unwrap_or(0);
                guard.available_viewers[(pos + 1) % guard.available_viewers.len()]
            } else if guard.active_viewer != ViewerKind::Text {
                ViewerKind::Text
            } else {
                guard.active_viewer
            }
        };
        self.switch_viewer(next)
    }

    /// 调用系统壳层“用其他应用打开”当前文件（主要用于兜底信息卡与通用右键菜单）。
    pub fn open_with_other_app(&self) -> Result<(), String> {
        let path = {
            let guard = self.state.read().unwrap();
            guard
                .current_file
                .as_ref()
                .map(|entry| entry.path.clone())
                .ok_or_else(|| "当前未打开任何文件项".to_string())?
        };
        self.shell.open_with_dialog(&path)
    }

    /// 切换到同目录序列中的上一项（到达开头时返回 `Ok(None)`，不越界）。
    pub fn navigate_prev(&self) -> Result<Option<FileEntry>, String> {
        let target = {
            let guard = self.state.read().unwrap();
            let seq = &guard.sibling_sequence;
            match seq.current_index {
                Some(idx) if idx > 0 => seq.entries.get(idx - 1).map(|e| e.path.clone()),
                _ => None,
            }
        };

        match target {
            Some(path) => self.open_file_internal(&path, true).map(Some),
            None => Ok(None),
        }
    }

    /// 切换到同目录序列中的下一项（到达末尾时返回 `Ok(None)`，不越界）。
    pub fn navigate_next(&self) -> Result<Option<FileEntry>, String> {
        let target = {
            let guard = self.state.read().unwrap();
            let seq = &guard.sibling_sequence;
            match seq.current_index {
                Some(idx) if idx + 1 < seq.entries.len() => {
                    seq.entries.get(idx + 1).map(|e| e.path.clone())
                }
                _ => None,
            }
        };

        match target {
            Some(path) => self.open_file_internal(&path, true).map(Some),
            None => Ok(None),
        }
    }

    /// 设置同目录序列筛选模式：`true` 为仅当前媒体大类，`false` 为全部支持格式。
    pub fn set_sibling_filter_by_category(&self, filter_by_category: bool) {
        let mut guard = self.state.write().unwrap();
        guard.sibling_sequence.filter_by_category = filter_by_category;
        if let Some(current) = guard.current_file.clone() {
            let filter_cat = guard.sibling_filter_category;
            guard.sibling_sequence =
                sibling::build_sibling_sequence(&current, filter_by_category, filter_cat);
        }
    }

    /// 一键在“仅当前媒体大类”与“全部支持格式”之间切换同目录序列筛选模式。
    pub fn toggle_sibling_filter(&self) {
        let current = self
            .state
            .read()
            .unwrap()
            .sibling_sequence
            .filter_by_category;
        self.set_sibling_filter_by_category(!current);
    }

    /// 主动重新扫描当前文件所在目录并更新同目录序列（仅按扩展名快速索引）。
    pub fn refresh_sibling_sequence(&self) {
        Self::refresh_state_sequence(&self.state);
    }

    fn refresh_state_sequence(state: &Arc<RwLock<SessionState>>) {
        let snapshot = {
            let guard = state.read().unwrap();
            guard.current_file.clone().map(|cur| {
                (
                    cur,
                    guard.sibling_sequence.filter_by_category,
                    guard.sibling_filter_category,
                )
            })
        };

        let Some((current, filter_by_category, filter_cat)) = snapshot else {
            return;
        };

        let updated = sibling::build_sibling_sequence(&current, filter_by_category, filter_cat);
        let mut guard = state.write().unwrap();
        // 仅当期间当前文件未切换时才回写，避免并发竞态覆盖新文件序列
        if guard
            .current_file
            .as_ref()
            .is_some_and(|cur| sibling::paths_equivalent(&cur.path, &current.path))
        {
            guard.sibling_sequence = updated;
        }
    }

    fn ensure_watching_parent(&self, path: &Path) {
        let Some(parent) = path.parent() else {
            return;
        };

        let mut watcher_guard = self.watcher.lock().unwrap();
        if watcher_guard
            .as_ref()
            .is_some_and(|w| sibling::paths_equivalent(w.watched_dir(), parent))
        {
            return;
        }

        let weak_state: Weak<RwLock<SessionState>> = Arc::downgrade(&self.state);
        match DirectoryWatcher::watch(parent, move || {
            if let Some(state) = weak_state.upgrade() {
                Self::refresh_state_sequence(&state);
            }
        }) {
            Ok(new_watcher) => {
                *watcher_guard = Some(new_watcher);
            }
            Err(_) => {
                // 某些网络共享盘或受限路径不支持目录监听时静默降级，不影响文件查看
                *watcher_guard = None;
            }
        }
    }

    fn open_file_internal(
        &self,
        path: &Path,
        preserve_filter_context: bool,
    ) -> Result<FileEntry, String> {
        if !probing::path_exists(path) {
            return Err(format!("文件不存在: {}", path.display()));
        }

        let probe = probing::probe_file(path, self.video_backend.as_ref());
        let entry = FileEntry::new(path.to_path_buf(), probe.media_category);
        let viewer = ViewerKind::from(probe.media_category);
        let now = self.clock.now();

        let (filter_by_category, filter_cat) = {
            let guard = self.state.read().unwrap();
            let filter_by_cat = guard.sibling_sequence.filter_by_category;
            let next_cat = if preserve_filter_context && guard.sibling_filter_category.is_some() {
                // 在同一序列中翻页时，即使遇到损坏文件也保留原筛选大类，避免被困住
                if filter_by_cat {
                    guard.sibling_filter_category
                } else {
                    probe
                        .media_category
                        .or_else(|| probing::classify_by_extension(path))
                        .or(guard.sibling_filter_category)
                }
            } else {
                probe
                    .media_category
                    .or_else(|| probing::classify_by_extension(path))
            };
            (filter_by_cat, next_cat)
        };

        let sequence = sibling::build_sibling_sequence(&entry, filter_by_category, filter_cat);
        let text_document = if viewer == ViewerKind::Text {
            Self::load_text_document(path)
        } else {
            None
        };

        let mut guard = self.state.write().unwrap();
        guard.text_document = text_document;
        guard.text_find = TextFindState::default();
        guard.current_file = Some(entry.clone());
        guard.active_viewer = viewer;
        guard.detected_format = probe.detected_format;
        guard.available_viewers = probe.available_viewers;
        guard.fallback_info = probe.fallback;
        guard.sibling_filter_category = filter_cat;
        guard.sibling_sequence = sequence;
        guard.text_mode = TextMode::Viewing;
        guard.has_unsaved_changes = false;
        if !guard.motion_engine.has_active_animations() {
            guard.last_motion_tick = now;
        }
        guard.motion_engine.animate_from_to(
            MotionChannel::ViewTransition,
            0.0,
            1.0,
            MotionProfile::snappy(),
        );
        drop(guard);

        if let Some(store) = &self.store {
            let _ = store.add_recent(path);
        }
        self.ensure_watching_parent(path);

        Ok(entry)
    }

    /// 请求关闭窗口：无未保存修改时立即允许退出（遵循 ADR-0002，零后台驻留）。
    pub fn request_close(&self) -> CloseDecision {
        let guard = self.state.read().unwrap();
        if guard.has_unsaved_changes {
            CloseDecision::ConfirmUnsavedChanges
        } else {
            CloseDecision::AllowExit
        }
    }
}
