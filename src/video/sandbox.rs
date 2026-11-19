//! libmpv 强制沙箱策略与本地媒体路径守卫（ADR-0005）。
//!
//! 播放不可信本地视频文件时，彻底隔离系统/用户 mpv 配置、禁用脚本与 yt-dlp、
//! 禁用非本地网络协议，防止恶意视频或播放列表触发任意代码执行或网络外连。

use std::path::Path;

use crate::video::error::VideoError;

/// libmpv 强制沙箱参数策略。
pub struct MpvSandboxPolicy;

impl MpvSandboxPolicy {
    /// 初始化 `mpv_handle` 之前必须全部强制应用的沙箱选项。
    /// 任何一项设置失败都必须立即中止初始化，绝不降级放行。
    pub const fn required_options() -> &'static [(&'static str, &'static str)] {
        &[
            // 彻底忽略系统目录与用户 %APPDATA%\mpv 配置
            ("config", "no"),
            // 禁止加载任何 Lua / JavaScript 脚本
            ("load-scripts", "no"),
            // 禁用 yt-dlp / youtube-dl 外连钩子
            ("ytdl", "no"),
            // 禁用自动配置 profile 与内置 OSD 控制台/统计脚本
            ("load-auto-profiles", "no"),
            ("load-stats-overlay", "no"),
            ("load-console", "no"),
            ("osc", "no"),
            ("terminal", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            // FFmpeg lavf 协议白名单仅允许本地文件解封装与流读取，阻断 http/https/rtsp/smb 等网络外连
            ("demuxer-lavf-o", "protocol_whitelist=file"),
            ("stream-lavf-o", "protocol_whitelist=file"),
        ]
    }
}

/// 校验并规范化待交给 libmpv 打开的本地媒体路径。
///
/// 明确拒绝所有 URL scheme（如 `http://`、`https://`、`rtsp://`、`edl://`、`smb://` 等）、
/// 空路径与含 NUL 字节的路径；允许 Windows 盘符路径、超长路径（`\\?\...`）、UNC 路径（`\\server\share\...`）
/// 以及无协议前缀的普通本地相对路径。
pub fn validate_local_media_path(path: &Path) -> Result<String, VideoError> {
    let raw = path
        .to_str()
        .ok_or_else(|| VideoError::UnsafeMediaPath("路径包含无效 UTF-8 字符".to_string()))?;

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(VideoError::UnsafeMediaPath("媒体路径为空".to_string()));
    }

    if trimmed.contains('\0') {
        return Err(VideoError::UnsafeMediaPath(
            "媒体路径包含非法 NUL 字节".to_string(),
        ));
    }

    // 检查是否存在 `<scheme>://` 形式（如 http://, edl://, av://, smb://）
    // 注意 Windows 盘符 `C:\` 或 `C:/` 只有一个冒号加单个斜杠，不含 `://`
    if let Some(scheme_pos) = trimmed.find("://") {
        let scheme = &trimmed[..scheme_pos];
        if !scheme.is_empty()
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        {
            return Err(VideoError::UnsafeMediaPath(format!(
                "禁止非本地协议：{scheme}://"
            )));
        }
    }

    // 额外拦截 mpv 特有的无双斜杠伪协议前缀（如 `edl:`, `memory:`, `fd://`, `lavfi:`）
    let lower = trimmed.to_ascii_lowercase();
    for forbidden_prefix in [
        "edl:",
        "memory:",
        "fd:",
        "fdclose:",
        "hex:",
        "lavfi:",
        "av:",
        "mf:",
        "dvd:",
        "bd:",
        "cdda:",
        "dvb:",
        "tv:",
    ] {
        if lower.starts_with(forbidden_prefix) {
            return Err(VideoError::UnsafeMediaPath(format!(
                "禁止 mpv 特殊伪协议前缀：{forbidden_prefix}"
            )));
        }
    }

    Ok(trimmed.to_string())
}
