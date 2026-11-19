//! 视频视图（Video Viewer，ADR-0005 / ADR-0007）：基于 libmpv 的视频播放。
//!
//! - `controls`：快捷键映射、悬浮控制条淡出状态机、同目录外挂字幕发现（纯逻辑，可单测）。
//! - `player`：`MpvInstance` + 软件渲染上下文（RGBA 帧 -> `slint::Image`）。
//! - `viewer`：与 `AppSession` 联动的视图控制器。
//! - `audio_fallback`：音频视图在纯 Rust 解码不支持（Opus、HE-AAC 等）时回退到 libmpv 播放。

pub mod audio_fallback;
pub mod controls;
pub mod player;
pub mod viewer;

pub use audio_fallback::MpvAudioFallback;
pub use controls::{
    ControlsFade, VideoCommand, VideoKey, find_external_subtitles, map_video_key,
};
pub use player::{SubtitleTrack, VideoPlayer};
pub use viewer::{VideoViewState, VideoViewer};
