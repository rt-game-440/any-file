//! Slint Skia 与 libmpv 视频画面合成策略及面向工单 #9（视频视图）的核心接口契约（ADR-0007）。

use crate::ActiveRenderer;

/// 视频画面与 Slint UI 的合成策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositingStrategy {
    /// 主路径：Slint `skia-opengl` + `mpv_render_context` (`MPV_RENDER_API_TYPE_OPENGL`)
    /// 将视频帧渲染至离屏 OpenGL FBO 纹理并通过 `slint::BorrowedOpenGLTextureBuilder` 交给 Skia 合成。
    /// 零 Airspace 遮挡，完整支持渐隐悬浮控件、弹簧动效、圆角裁剪与高 DPI 缩放。
    SkiaOpenGLSharedContext,
    /// 软件回退路径：`mpv_render_context` (`MPV_RENDER_API_TYPE_SW`) 将视频帧渲染至 RGBA 内存缓冲
    /// 并上传为 `slint::Image`。适用于无 OpenGL 驱动或 Slint 回退软件渲染的环境。
    SoftwareFrameUpload,
    /// 备选评估路径：Win32 子 HWND (`wid`) 嵌入 mpv + 透明顶层浮层。
    /// 存在原生 HWND Airspace 遮挡与跨窗口高 DPI 同步复杂度，仅作为 ADR-0007 对比项记录。
    ChildHwndOverlay,
}

impl CompositingStrategy {
    /// 根据当前激活的 Slint 渲染后端推荐默认合成策略。
    pub fn recommended_for_renderer(renderer: ActiveRenderer) -> Self {
        match renderer {
            ActiveRenderer::Skia => Self::SkiaOpenGLSharedContext,
            ActiveRenderer::Software => Self::SoftwareFrameUpload,
        }
    }
}

/// CPU 内存中的一帧 RGBA8 视频像素（软件回退路径或离屏首帧抓帧使用）。
#[derive(Debug, Clone)]
pub struct VideoFrameRgba {
    pub width: u32,
    pub height: u32,
    /// 紧凑排列的 RGBA8 像素数据，长度恒等于 `width * height * 4`
    pub rgba: Vec<u8>,
}

/// 供工单 #9（视频视图）承接的播放状态快照。
#[derive(Debug, Clone, PartialEq)]
pub struct VideoPlaybackState {
    pub paused: bool,
    pub position_sec: f64,
    pub duration_sec: f64,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub active_hwdec: String,
    pub compositing: CompositingStrategy,
}

impl Default for VideoPlaybackState {
    fn default() -> Self {
        Self {
            paused: true,
            position_sec: 0.0,
            duration_sec: 0.0,
            volume: 100.0,
            muted: false,
            speed: 1.0,
            active_hwdec: "no".to_string(),
            compositing: CompositingStrategy::SkiaOpenGLSharedContext,
        }
    }
}

/// 供工单 #9（视频视图）向视频播放器核心发送的控制命令。
#[derive(Debug, Clone, PartialEq)]
pub enum VideoCommand {
    TogglePause,
    SetPaused(bool),
    SeekRelative(f64),
    SeekAbsolute(f64),
    SetVolume(f64),
    ToggleMute,
    SetSpeed(f64),
    CycleSubtitleTrack,
    LoadExternalSubtitle(std::path::PathBuf),
}
