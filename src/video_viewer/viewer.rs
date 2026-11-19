//! 视频视图控制器：与 `AppSession` 联动（切文件即换片、同目录切换、外挂字幕、悬浮控制条淡出、全屏状态）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::audio_viewer::controls::{AudioCommand, KeyModifiers, format_speed, format_time};
use crate::domain::ViewerKind;
use crate::session::AppSession;
use crate::video::compositing::VideoFrameRgba;
use crate::video::mpv_backend::LazyMpvVideoBackend;

use super::controls::{ControlsFade, VideoCommand, VideoKey, find_external_subtitles, map_video_key};
use super::player::VideoPlayer;

/// 供 UI 绑定的展示状态
#[derive(Debug, Clone, PartialEq)]
pub struct VideoViewState {
    pub loaded: bool,
    pub playing: bool,
    pub ended: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub position_text: String,
    pub duration_text: String,
    pub speed_text: String,
    pub volume: f32,
    pub muted: bool,
    pub subtitles_visible: bool,
    /// 是否存在可切换的字幕轨（内嵌或外挂）
    pub has_subtitles: bool,
    pub fullscreen: bool,
    pub controls_visible: bool,
    pub hwdec: String,
    /// 面向用户的错误（中文原文，由界面层 `localize`）
    pub error: Option<String>,
}

pub struct VideoViewer {
    backend: Arc<LazyMpvVideoBackend>,
    player: Option<VideoPlayer>,
    current: Option<PathBuf>,
    error: Option<String>,
    fade: ControlsFade,
    fullscreen: bool,
    autoplay_on_load: bool,
    started: Instant,
    has_external_subs: bool,
}

impl VideoViewer {
    pub fn new(backend: Arc<LazyMpvVideoBackend>) -> Self {
        Self {
            backend,
            player: None,
            current: None,
            error: None,
            fade: ControlsFade::default(),
            fullscreen: false,
            autoplay_on_load: true,
            started: Instant::now(),
            has_external_subs: false,
        }
    }

    pub fn set_autoplay_on_load(&mut self, autoplay: bool) {
        self.autoplay_on_load = autoplay;
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    pub fn player(&self) -> Option<&VideoPlayer> {
        self.player.as_ref()
    }

    pub fn player_mut(&mut self) -> Option<&mut VideoPlayer> {
        self.player.as_mut()
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
    }

    fn now_secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// 鼠标移动 / 点击等：唤出悬浮控制条
    pub fn on_activity(&mut self) {
        let now = self.now_secs();
        self.fade.on_activity(now);
    }

    /// 立即停止播放并释放 mpv（切到其他视图 / 关闭窗口）
    pub fn stop(&mut self) {
        // 先销毁播放器：内部按“渲染上下文 -> mpv 句柄”顺序释放
        self.player = None;
        self.current = None;
        self.error = None;
        self.fullscreen = false;
    }

    /// 与会话同步；返回是否发生载入或停止。
    pub fn sync_with_session(&mut self, session: &AppSession) -> bool {
        let target = (session.active_viewer() == ViewerKind::Video)
            .then(|| session.current_file_entry())
            .flatten()
            .map(|e| e.path);
        match target {
            Some(path) => {
                if self.current.as_deref() == Some(path.as_path()) {
                    return false;
                }
                self.load(&path);
                true
            }
            None => {
                if self.current.is_some() || self.player.is_some() {
                    self.stop();
                    true
                } else {
                    false
                }
            }
        }
    }

    fn load(&mut self, path: &Path) {
        self.current = Some(path.to_path_buf());
        self.error = None;
        self.has_external_subs = false;
        if self.player.is_none() {
            match self.backend.library().and_then(VideoPlayer::new) {
                Ok(p) => self.player = Some(p),
                Err(e) => {
                    self.error = Some(format!("无法播放视频：{e}"));
                    return;
                }
            }
        }
        let autoplay = self.autoplay_on_load;
        let Some(player) = self.player.as_mut() else {
            return;
        };
        if let Err(e) = player.load(path, autoplay) {
            self.error = Some(format!("无法播放视频：{e}"));
            return;
        }
        // 同目录外挂字幕：全部挂载，首个（ASS 优先）被选中
        let subs = find_external_subtitles(path);
        for (i, sub) in subs.iter().enumerate() {
            if i == 0 {
                let _ = player.add_external_subtitle(sub);
            } else {
                let _ = player
                    .transport()
                    .mpv()
                    .command(&["sub-add", &sub.to_string_lossy(), "auto"]);
            }
        }
        self.has_external_subs = !subs.is_empty();
        let now = self.now_secs();
        self.fade.on_activity(now);
    }

    pub fn handle_key(&mut self, key: VideoKey, mods: KeyModifiers, session: &AppSession) -> bool {
        match map_video_key(key, mods) {
            Some(cmd) => {
                self.handle_command(cmd, session);
                true
            }
            None => false,
        }
    }

    pub fn handle_command(&mut self, cmd: VideoCommand, session: &AppSession) {
        self.on_activity();
        match cmd {
            VideoCommand::Playback(AudioCommand::PrevTrack) => {
                if session.navigate_prev().is_ok_and(|e| e.is_some()) {
                    self.sync_with_session(session);
                }
            }
            VideoCommand::Playback(AudioCommand::NextTrack) => {
                if session.navigate_next().is_ok_and(|e| e.is_some()) {
                    self.sync_with_session(session);
                }
            }
            VideoCommand::Playback(c) => {
                if let Some(p) = &self.player {
                    let t = p.transport();
                    match c {
                        AudioCommand::TogglePlay => t.toggle_play(),
                        AudioCommand::SeekBy(d) => t.seek_by(d),
                        AudioCommand::SeekTo(s) => t.seek_to(s),
                        AudioCommand::CycleSpeed => t.cycle_speed(),
                        AudioCommand::SetSpeed(s) => t.set_speed(s),
                        AudioCommand::SetVolume(v) => t.set_volume(v),
                        AudioCommand::ToggleMute => t.toggle_mute(),
                        _ => {}
                    }
                }
            }
            VideoCommand::ToggleFullscreen => self.fullscreen = !self.fullscreen,
            VideoCommand::ExitFullscreen => self.fullscreen = false,
            VideoCommand::ToggleSubtitles => {
                if let Some(p) = self.player.as_mut() {
                    p.toggle_subtitles();
                }
            }
            VideoCommand::CycleSubtitleTrack => {
                if let Some(p) = self.player.as_mut() {
                    p.cycle_subtitle_track();
                }
            }
        }
    }

    pub fn request_redraw(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.request_redraw();
        }
    }

    /// 周期轮询：若有新帧则渲染为 `width x height` 的 RGBA 帧。
    pub fn poll_frame(&mut self, width: u32, height: u32) -> Option<VideoFrameRgba> {
        let p = self.player.as_mut()?;
        self.current.as_ref()?;
        if !p.has_new_frame() {
            return None;
        }
        p.render_frame(width, height).ok()
    }

    pub fn view_state(&self, hovering_controls: bool) -> VideoViewState {
        let snap = self.player.as_ref().map(VideoPlayer::snapshot);
        let loaded = snap.as_ref().is_some_and(|s| s.loaded) && self.error.is_none();
        let s = snap.unwrap_or(crate::video::TransportSnapshot {
            loaded: false,
            paused: true,
            ended: false,
            position_secs: 0.0,
            duration_secs: 0.0,
            speed: 1.0,
            volume: 1.0,
            muted: false,
        });
        let has_subtitles = self
            .player
            .as_ref()
            .is_some_and(|p| !p.subtitle_tracks().is_empty());
        let paused = s.paused || s.ended || !loaded;
        VideoViewState {
            loaded,
            playing: loaded && !s.paused && !s.ended,
            ended: s.ended,
            position_secs: s.position_secs,
            duration_secs: s.duration_secs,
            position_text: format_time(s.position_secs),
            duration_text: format_time(s.duration_secs),
            speed_text: format_speed(s.speed),
            volume: s.volume,
            muted: s.muted,
            subtitles_visible: self.player.as_ref().is_none_or(|p| p.subtitles_visible()),
            has_subtitles: has_subtitles || self.has_external_subs,
            fullscreen: self.fullscreen,
            controls_visible: self.fade.visible(self.now_secs(), paused, hovering_controls),
            hwdec: self
                .player
                .as_ref()
                .and_then(VideoPlayer::active_hwdec)
                .unwrap_or_default(),
            error: self.error.clone(),
        }
    }
}
