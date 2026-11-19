//! 平台适配层：提供真实 OS 壳层、单调时钟与默认视频后端占位实现。

use std::path::Path;
use std::time::Instant;

use crate::ports::{Clock, VideoBackend, VideoProbeInfo};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::WindowsSystemShell as PlatformSystemShell;

#[cfg(not(windows))]
mod fallback {
    use crate::domain::ResolvedTheme;
    use crate::ports::SystemShell;

    #[derive(Debug, Default)]
    pub struct FallbackSystemShell;

    impl SystemShell for FallbackSystemShell {
        fn system_theme(&self) -> ResolvedTheme {
            ResolvedTheme::Light
        }

        fn prefers_reduced_motion(&self) -> bool {
            false
        }
    }
}

#[cfg(not(windows))]
pub use fallback::FallbackSystemShell as PlatformSystemShell;

/// 基于 `std::time::Instant` 的真实单调时钟
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// 脚手架阶段的默认视频后端占位实现（未加载 `mpv-2.dll`，由工单 #3/#8 替换）
#[derive(Debug, Default)]
pub struct UnavailableVideoBackend;

impl VideoBackend for UnavailableVideoBackend {
    fn is_available(&self) -> bool {
        false
    }

    fn probe(&self, _path: &Path) -> Option<VideoProbeInfo> {
        None
    }
}
