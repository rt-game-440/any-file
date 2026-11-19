//! `AppSession`：位于 Slint UI 绑定层正下方的无头应用核心控制器。
//! 统一编排文件项打开、媒体大类分发、同目录序列、文本双模式、动效配置与系统偏好同步。

use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::domain::{
    CloseDecision, FileEntry, MediaCategory, MotionProfile, ResolvedTheme, SiblingSequence,
    TextMode, ThemePreference, ViewerKind,
};
use crate::ports::{Clock, SystemShell, VideoBackend};

/// `AppSession` 内部快照状态
#[derive(Debug, Clone)]
struct SessionState {
    active_viewer: ViewerKind,
    current_file: Option<FileEntry>,
    sibling_sequence: SiblingSequence,
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
    state: RwLock<SessionState>,
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
            state: RwLock::new(SessionState {
                active_viewer: ViewerKind::Welcome,
                current_file: None,
                sibling_sequence: SiblingSequence::empty(),
                text_mode: TextMode::Viewing,
                has_unsaved_changes: false,
                theme_preference: ThemePreference::System,
                resolved_theme,
                motion_profile,
                started_at,
            }),
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

    /// 打开指定路径的文件项。
    /// 工单 #2 提供最小基础骨架（校验文件存在并按基础扩展名粗分，后续由工单 #4 替换为两层探测与同目录序列）。
    pub fn open_file(&self, path: &Path) -> Result<FileEntry, String> {
        if !path.exists() {
            return Err(format!("文件不存在: {}", path.display()));
        }

        let category = Self::quick_classify_extension(path);
        let entry = FileEntry::new(path.to_path_buf(), category);
        let viewer = ViewerKind::from(category);

        let mut guard = self.state.write().unwrap();
        guard.current_file = Some(entry.clone());
        guard.active_viewer = viewer;
        guard.text_mode = TextMode::Viewing;
        guard.has_unsaved_changes = false;

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

    /// 最小扩展名粗分占位（工单 #4 将替换为 Magic Bytes + 流探测的两层探测引擎）
    fn quick_classify_extension(path: &Path) -> Option<MediaCategory> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())?;

        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "tif" | "tiff" | "svg"
            | "avif" | "apng" => Some(MediaCategory::Image),
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" => Some(MediaCategory::Video),
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" => Some(MediaCategory::Audio),
            "txt" | "md" | "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "log" | "csv"
            | "rs" | "js" | "ts" | "py" | "c" | "cpp" | "h" | "hpp" | "html" | "css" | "sh"
            | "ps1" => Some(MediaCategory::Text),
            _ => None,
        }
    }
}
