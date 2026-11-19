//! 视频视图的纯逻辑：快捷键映射、悬浮控制条淡出状态机、同目录外挂字幕发现。

use std::path::{Path, PathBuf};

use crate::audio_viewer::controls::{AudioCommand, AudioKey, KeyModifiers, map_key};

/// 视频视图键位（在音频键位基础上增加全屏、字幕）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoKey {
    Space,
    Left,
    Right,
    PageUp,
    PageDown,
    M,
    F,
    Escape,
    S,
}

/// 视频视图命令
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoCommand {
    /// 播放控制与同目录切换（沿用音频命令集）
    Playback(AudioCommand),
    ToggleFullscreen,
    ExitFullscreen,
    ToggleSubtitles,
    CycleSubtitleTrack,
}

/// 键位 -> 命令；未映射返回 `None`。
/// Space 播放/暂停，←/→ ±5s，PageUp/PageDown 与 Ctrl+←/→ 切换同目录文件，M 静音，F 全屏，Esc 退出全屏，S 字幕开关。
pub fn map_video_key(key: VideoKey, mods: KeyModifiers) -> Option<VideoCommand> {
    let audio = match key {
        VideoKey::Space => Some(AudioKey::Space),
        VideoKey::Left => Some(AudioKey::Left),
        VideoKey::Right => Some(AudioKey::Right),
        VideoKey::PageUp => Some(AudioKey::PageUp),
        VideoKey::PageDown => Some(AudioKey::PageDown),
        VideoKey::M => Some(AudioKey::M),
        _ => None,
    };
    if let Some(a) = audio {
        return map_key(a, mods).map(VideoCommand::Playback);
    }
    match key {
        VideoKey::F => Some(VideoCommand::ToggleFullscreen),
        VideoKey::Escape => Some(VideoCommand::ExitFullscreen),
        VideoKey::S => Some(if mods.ctrl {
            VideoCommand::CycleSubtitleTrack
        } else {
            VideoCommand::ToggleSubtitles
        }),
        _ => None,
    }
}

/// 悬浮控制条显隐状态机：鼠标/键盘活动后显示，空闲超时淡出；暂停、悬停控制条、未载入时常显。
#[derive(Debug, Clone)]
pub struct ControlsFade {
    idle_timeout_secs: f64,
    last_activity_secs: f64,
}

impl ControlsFade {
    pub const DEFAULT_IDLE_SECS: f64 = 3.0;

    pub fn new(idle_timeout_secs: f64) -> Self {
        Self {
            idle_timeout_secs,
            last_activity_secs: f64::NEG_INFINITY,
        }
    }

    /// 记录一次鼠标移动 / 按键等活动
    pub fn on_activity(&mut self, now_secs: f64) {
        self.last_activity_secs = now_secs;
    }

    /// 当前是否应显示控制条
    pub fn visible(&self, now_secs: f64, paused: bool, hovering_controls: bool) -> bool {
        paused
            || hovering_controls
            || now_secs - self.last_activity_secs < self.idle_timeout_secs
    }
}

impl Default for ControlsFade {
    fn default() -> Self {
        Self::new(Self::DEFAULT_IDLE_SECS)
    }
}

/// 外挂字幕扩展名
const SUBTITLE_EXTS: [&str; 3] = ["ass", "ssa", "srt"];

/// 发现与视频同目录、同主文件名（忽略大小写）的外挂字幕：`movie.srt`、`movie.zh.ass` 等。
/// 结果按文件名排序，ASS/SSA 优先于 SRT（样式更丰富）。
pub fn find_external_subtitles(video: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str()))
    else {
        return Vec::new();
    };
    let stem = stem.to_lowercase();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(u8, String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            let ext = p.extension()?.to_str()?.to_lowercase();
            if !SUBTITLE_EXTS.contains(&ext.as_str()) {
                return None;
            }
            let sub_stem = p.file_stem()?.to_str()?.to_lowercase();
            let matches = sub_stem == stem || sub_stem.starts_with(&format!("{stem}."));
            if !matches {
                return None;
            }
            let rank = u8::from(ext == "srt");
            Some((rank, p.file_name()?.to_string_lossy().to_lowercase(), p))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO: KeyModifiers = KeyModifiers { ctrl: false };
    const CTRL: KeyModifiers = KeyModifiers { ctrl: true };

    #[test]
    fn keys_map_to_playback_and_view_commands() {
        assert_eq!(
            map_video_key(VideoKey::Space, NO),
            Some(VideoCommand::Playback(AudioCommand::TogglePlay))
        );
        assert_eq!(
            map_video_key(VideoKey::Left, NO),
            Some(VideoCommand::Playback(AudioCommand::SeekBy(-5.0)))
        );
        assert_eq!(
            map_video_key(VideoKey::Right, NO),
            Some(VideoCommand::Playback(AudioCommand::SeekBy(5.0)))
        );
        assert_eq!(
            map_video_key(VideoKey::Left, CTRL),
            Some(VideoCommand::Playback(AudioCommand::PrevTrack))
        );
        assert_eq!(
            map_video_key(VideoKey::PageDown, NO),
            Some(VideoCommand::Playback(AudioCommand::NextTrack))
        );
        assert_eq!(map_video_key(VideoKey::F, NO), Some(VideoCommand::ToggleFullscreen));
        assert_eq!(map_video_key(VideoKey::Escape, NO), Some(VideoCommand::ExitFullscreen));
        assert_eq!(map_video_key(VideoKey::S, NO), Some(VideoCommand::ToggleSubtitles));
        assert_eq!(map_video_key(VideoKey::S, CTRL), Some(VideoCommand::CycleSubtitleTrack));
    }

    #[test]
    fn controls_fade_after_idle_but_stay_when_paused_or_hovered() {
        let mut f = ControlsFade::default();
        assert!(!f.visible(0.0, false, false), "无活动且播放中应隐藏");
        f.on_activity(10.0);
        assert!(f.visible(11.0, false, false));
        assert!(f.visible(12.9, false, false));
        assert!(!f.visible(13.0, false, false), "3 秒空闲后淡出");
        assert!(f.visible(100.0, true, false), "暂停时常显");
        assert!(f.visible(100.0, false, true), "悬停控制条时常显");
        f.on_activity(100.0);
        assert!(f.visible(101.0, false, false), "再次活动恢复显示");
    }

    #[test]
    fn finds_same_stem_subtitles_ass_first() {
        let dir = std::env::temp_dir().join(format!("anyfile-sub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["Movie.mp4", "movie.srt", "Movie.zh.ASS", "movie2.srt", "other.srt", "movie.txt"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let subs = find_external_subtitles(&dir.join("Movie.mp4"));
        let names: Vec<String> = subs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Movie.zh.ASS", "movie.srt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
