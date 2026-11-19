//! 视频解码与画面合成模块（ADR-0005 / ADR-0007）。
//!
//! 负责 `mpv-2.dll` 安全动态加载、强制沙箱初始化、视频流探测及与 Slint Skia 的画面合成契约。

pub mod compositing;
pub mod error;
pub mod ffi;
pub mod loader;
pub mod mpv_backend;
pub mod sandbox;

pub use compositing::{CompositingStrategy, VideoCommand, VideoFrameRgba, VideoPlaybackState};
pub use error::VideoError;
pub use loader::{MPV_DLL_NAME, MpvLibrary, is_mpv_module_loaded_in_process};
pub use mpv_backend::{LazyMpvVideoBackend, MpvInstance};
pub use sandbox::{MpvSandboxPolicy, validate_local_media_path};
