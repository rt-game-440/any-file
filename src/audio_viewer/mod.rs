//! 音频视图（Audio Viewer，ADR-0005）：纯 Rust 解码（symphonia）+ 系统默认输出设备播放（cpal），
//! 运行时零额外 DLL 依赖。
//!
//! 模块分层（视频视图 #9 可直接复用 `controls` / `core` / `player` 的播放控制能力）：
//! - `decoder`：symphonia 流式解码与精确 seek，输出交错 `f32` PCM。
//! - `metadata`：lofty 读取标签、封面（解码为 RGBA）与时长。
//! - `controls`：快捷键 → 播放命令的纯函数状态机，以及倍速档位、时间格式化。
//! - `core`：与设备无关的 PCM 拉取式播放核心（重采样、倍速、音量、位置、播完停止）。
//! - `player`：`core` + cpal 输出流封装；无输出设备时降级为无声的 headless 模式。
//! - `viewer`：与 `AppSession` 联动的视图控制器（切文件停播、同目录切歌、自动播放下一首）。

pub mod controls;
pub mod core;
pub mod decoder;
pub mod fallback;
pub mod metadata;
pub mod player;
pub mod viewer;

pub use controls::{AudioCommand, AudioKey, KeyModifiers, SPEED_STEPS, format_time, map_key};
pub use fallback::{AudioFallback, is_fallback_candidate};
pub use core::{PlaybackSnapshot, PlayerCore};
pub use decoder::{AudioDecoder, StreamInfo};
pub use metadata::{AudioMetadata, CoverImage, read_metadata};
pub use player::AudioPlayer;
pub use viewer::{AudioViewer, AudioViewState};
