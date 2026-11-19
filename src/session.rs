//! `AppSession`：位于 Slint UI 绑定层正下方的无头应用核心控制器。
//! 统一编排文件项打开、媒体大类分发、同目录序列、文本双模式、动效配置与系统偏好同步。

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Instant;

use crate::domain::{
    CloseDecision, FallbackInfo, FileEntry, MediaCategory, MotionProfile, ResolvedTheme,
    SiblingSequence, TextMode, ThemePreference, ViewerKind,
};
use crate::ports::{Clock, SystemShell, VideoBackend};
use crate::probing;
use crate::sibling::{self, DirectoryWatcher};

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
    has_unsaved_changes: bool,
    theme_preference: ThemePreference,
    resolved_theme: ResolvedTheme,
    motion_profile: MotionProfile,
    started_at: Instant,
}

/// 无头应用会话控制器（测试与 UI 绑定的主接缝）
pub struct AppSession {
    shell: Arc<dyn SystemShell>,
    clock: Arc<dyn Clock>,
    video_backend: Arc<dyn VideoBackend>,
    state: Arc<RwLock<SessionState>>,
    watcher: Mutex<Option<DirectoryWatcher>>,
}

impl AppSession {
    /// 使用外部边界依赖创建新的无头会话，默认进入 `ViewerKind::Welcome` 空态。
    pub fn new(
        shell: Arc<dyn SystemShell>,
        clock: Arc<dyn Clock>,
        video_backend: Arc<dyn VideoBackend>,
    ) -> Self {
        let resolved_theme = shell.system_theme();
        let motion_profile = MotionProfile::for_system_setting(shell.prefers_reduced_motion());
        let started_at = clock.now();

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
                has_unsaved_changes: false,
                theme_preference: ThemePreference::System,
                resolved_theme,
                motion_profile,
                started_at,
            })),
            watcher: Mutex::new(None),
        }
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

        let mut guard = self.state.write().unwrap();
        if guard.current_file.is_none() {
            return Err("当前未打开任何文件项".to_string());
        }

        guard.active_viewer = target;
        if target == ViewerKind::Text {
            guard.text_mode = TextMode::Viewing;
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

        let mut guard = self.state.write().unwrap();
        guard.current_file = Some(entry.clone());
        guard.active_viewer = viewer;
        guard.detected_format = probe.detected_format;
        guard.available_viewers = probe.available_viewers;
        guard.fallback_info = probe.fallback;
        guard.sibling_filter_category = filter_cat;
        guard.sibling_sequence = sequence;
        guard.text_mode = TextMode::Viewing;
        guard.has_unsaved_changes = false;
        drop(guard);

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
