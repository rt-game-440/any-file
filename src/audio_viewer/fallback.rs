//! 纯 Rust 解码不支持的音频编码（如 Opus、HE-AAC）的回退播放引擎接口。
//! 具体实现由 libmpv 提供（见 `crate::video_viewer::audio_fallback`），此处只定义抽象，
//! 以便音频视图在没有 libmpv 时仍可编译、测试。

use std::path::Path;

use super::controls::AudioCommand;
use super::core::PlaybackSnapshot;

/// 回退播放引擎。
pub trait AudioFallback {
    /// 载入并（由调用方随后决定是否）播放；失败返回面向用户的错误文本。
    fn load(&mut self, path: &Path) -> Result<(), String>;
    fn play(&mut self);
    fn apply(&mut self, cmd: AudioCommand);
    fn snapshot(&self) -> PlaybackSnapshot;
    fn stop(&mut self);
}

/// 判断纯 Rust 解码错误是否属于“可由回退引擎尝试”的类型。
pub fn is_fallback_candidate(error: &str) -> bool {
    error.starts_with("不支持的音频编码")
        || error.starts_with("无法识别音频格式")
        || error.contains("没有可解码的音频轨道")
}
