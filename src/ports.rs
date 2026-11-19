//! 外部不可控边界（OS 壳层、单调时钟、外部视频 DLL）的极薄 Port/Trait 抽象。
//! 仅在 OS/DLL 边界使用 trait，其余核心逻辑直接组合真实实现。

use std::path::Path;
use std::time::Instant;

use crate::domain::ResolvedTheme;

/// 系统壳层边界（Windows 主题、减弱动态效果查询，及后续回收站/资源管理器/默认应用跳转）。
pub trait SystemShell: Send + Sync {
    /// 查询操作系统当前的深浅色主题
    fn system_theme(&self) -> ResolvedTheme;

    /// 查询操作系统是否启用了“减弱动态效果 / 关闭动画效果”
    fn prefers_reduced_motion(&self) -> bool;

    /// 将文件移入系统回收站（后续工单完善具体实现）
    fn move_to_trash(&self, _path: &Path) -> Result<(), String> {
        Err("系统回收站操作尚未实现".to_string())
    }

    /// 在系统文件资源管理器中选中并显示指定文件（后续工单完善具体实现）
    fn reveal_in_explorer(&self, _path: &Path) -> Result<(), String> {
        Err("在资源管理器中显示尚未实现".to_string())
    }

    /// 调起系统“打开方式”对话框（后续工单完善具体实现）
    fn open_with_dialog(&self, _path: &Path) -> Result<(), String> {
        Err("打开方式对话框尚未实现".to_string())
    }

    /// 跳转到 Windows“默认应用”设置页
    fn open_default_apps_settings(&self) -> Result<(), String> {
        Err("默认应用设置跳转尚未实现".to_string())
    }

    /// 将文本写入系统剪贴板
    fn copy_text_to_clipboard(&self, _text: &str) -> Result<(), String> {
        Err("剪贴板操作尚未实现".to_string())
    }

    /// 用系统默认浏览器打开 URL（仅允许本项目 GitHub 地址）
    fn open_url(&self, _url: &str) -> Result<(), String> {
        Err("打开链接尚未实现".to_string())
    }

    /// 系统界面语言区域标签（如 `zh-CN`、`en-US`）
    fn system_language(&self) -> String {
        "en-US".to_string()
    }
}

/// 单调时钟边界：供弹簧动效时间步进与超时控制测试使用。
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// 视频流探测结果摘要（供后续工单 #3 / #8 扩展）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoProbeInfo {
    pub has_video_stream: bool,
    pub has_audio_stream: bool,
    pub width: u32,
    pub height: u32,
    pub duration_ms: Option<u64>,
}

/// 视频解码后端边界（隔离按需动态加载的 `mpv-2.dll`，ADR-0005）。
pub trait VideoBackend: Send + Sync {
    /// 当前运行环境是否成功加载可用的视频解码后端
    fn is_available(&self) -> bool;

    /// 探测指定媒体文件的流信息（如区分纯音频容器与视频容器）
    fn probe(&self, path: &Path) -> Option<VideoProbeInfo>;
}

/// 允许通过 `open_url` 打开的链接前缀（零遥测：仅本项目 GitHub 发布页）
pub const ALLOWED_URL_PREFIX: &str = "https://github.com/rt-game-440/any-file";

/// 校验 URL 是否在白名单内（防止被注入的发布信息打开任意链接）
pub fn is_allowed_url(url: &str) -> bool {
    url.starts_with(ALLOWED_URL_PREFIX)
        && url[ALLOWED_URL_PREFIX.len()..]
            .chars()
            .next()
            .is_none_or(|c| c == '/' || c == '?' || c == '#')
        && !url.chars().any(|c| c.is_control() || c == ' ' || c == '"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_allowlist() {
        assert!(is_allowed_url("https://github.com/rt-game-440/any-file/releases/tag/v0.2.0"));
        assert!(is_allowed_url("https://github.com/rt-game-440/any-file"));
        assert!(!is_allowed_url("https://github.com/rt-game-440/any-file-evil/x"));
        assert!(!is_allowed_url("https://evil.com/https://github.com/rt-game-440/any-file"));
        assert!(!is_allowed_url("https://github.com/rt-game-440/any-file/ x"));
    }
}
