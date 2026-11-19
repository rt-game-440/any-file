//! 基于 `MpvInstance` 的通用播放传输控制（暂停/跳转/倍速/音量/静音）与状态快照。
//! 视频视图与音频视图的 libmpv 回退共用，所有调用必须在同一线程（UI 线程）进行。

use std::path::Path;

use crate::audio_viewer::controls::{SPEED_STEPS, clamp_speed};
use crate::video::error::VideoError;
use crate::video::mpv_backend::MpvInstance;

/// 播放状态快照（音量为 0.0..=1.0）。
#[derive(Debug, Clone, PartialEq)]
pub struct TransportSnapshot {
    pub loaded: bool,
    pub paused: bool,
    pub ended: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub speed: f32,
    pub volume: f32,
    pub muted: bool,
}

/// 传输控制器。
pub struct MpvTransport {
    mpv: MpvInstance,
    loaded: bool,
}

impl MpvTransport {
    pub fn new(mpv: MpvInstance) -> Self {
        Self { mpv, loaded: false }
    }

    pub fn mpv(&self) -> &MpvInstance {
        &self.mpv
    }

    /// 载入本地文件（经沙箱路径守卫），等待加载完成。载入后保持当前暂停选项。
    pub fn load(&mut self, path: &Path) -> Result<(), VideoError> {
        self.loaded = false;
        self.mpv.load_local_file(path)?;
        self.mpv
            .wait_until_file_loaded(std::time::Duration::from_secs(5))?;
        self.loaded = true;
        Ok(())
    }

    pub fn unload(&mut self) {
        let _ = self.mpv.command(&["stop"]);
        self.loaded = false;
    }

    fn flag(&self, name: &str) -> Option<bool> {
        self.mpv.get_property_string(name).map(|v| v == "yes")
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.mpv.set_property_string("pause", if paused { "yes" } else { "no" });
    }

    /// 播放/暂停切换；已播完时从头开始播放。
    pub fn toggle_play(&self) {
        let s = self.snapshot();
        if s.ended {
            let _ = self.mpv.command(&["seek", "0", "absolute"]);
            self.set_paused(false);
        } else {
            self.set_paused(!s.paused);
        }
    }

    pub fn seek_by(&self, delta: f64) {
        let _ = self.mpv.command(&["seek", &format!("{delta}"), "relative"]);
    }

    pub fn seek_to(&self, secs: f64) {
        let _ = self
            .mpv
            .command(&["seek", &format!("{}", secs.max(0.0)), "absolute"]);
    }

    pub fn set_speed(&self, speed: f32) {
        let _ = self
            .mpv
            .set_property_string("speed", &format!("{}", clamp_speed(speed)));
    }

    pub fn cycle_speed(&self) {
        let cur = self.snapshot().speed;
        let next = SPEED_STEPS
            .iter()
            .copied()
            .find(|s| *s > cur + 1e-3)
            .unwrap_or(SPEED_STEPS[0]);
        self.set_speed(next);
    }

    pub fn set_volume(&self, volume: f32) {
        let v = volume.clamp(0.0, 1.0);
        let _ = self.mpv.set_property_string("volume", &format!("{}", v * 100.0));
        if v > 0.0 {
            let _ = self.mpv.set_property_string("mute", "no");
        }
    }

    pub fn toggle_mute(&self) {
        let m = self.flag("mute").unwrap_or(false);
        let _ = self.mpv.set_property_string("mute", if m { "no" } else { "yes" });
    }

    pub fn snapshot(&self) -> TransportSnapshot {
        let dur = self
            .mpv
            .get_property_f64("duration")
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(0.0);
        let ended = self.loaded && self.flag("eof-reached").unwrap_or(false);
        let pos = if ended {
            dur
        } else {
            self.mpv
                .get_property_f64("time-pos")
                .filter(|p| p.is_finite())
                .unwrap_or(0.0)
                .clamp(0.0, if dur > 0.0 { dur } else { f64::MAX })
        };
        TransportSnapshot {
            loaded: self.loaded,
            paused: self.flag("pause").unwrap_or(true),
            ended,
            position_secs: pos,
            duration_secs: dur,
            speed: self.mpv.get_property_f64("speed").unwrap_or(1.0) as f32,
            volume: (self.mpv.get_property_f64("volume").unwrap_or(100.0) / 100.0) as f32,
            muted: self.flag("mute").unwrap_or(false),
        }
    }
}
