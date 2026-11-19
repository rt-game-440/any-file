//! 基于 libmpv 的音频回退引擎：纯 Rust 解码不支持的编码（Opus、HE-AAC 等）交给 libmpv 播放。
//! 惰性创建 `vo=null` 的 mpv 实例，首次回退时才加载 `mpv-2.dll`。

use std::path::Path;
use std::sync::Arc;

use crate::audio_viewer::controls::AudioCommand;
use crate::audio_viewer::core::PlaybackSnapshot;
use crate::audio_viewer::fallback::AudioFallback;
use crate::video::mpv_backend::{LazyMpvVideoBackend, MpvInstance};
use crate::video::transport::MpvTransport;

pub struct MpvAudioFallback {
    backend: Arc<LazyMpvVideoBackend>,
    transport: Option<MpvTransport>,
    /// 无输出设备 / 测试时使用 `ao=null`
    audio_output: &'static str,
}

impl MpvAudioFallback {
    pub fn new(backend: Arc<LazyMpvVideoBackend>) -> Self {
        Self {
            backend,
            transport: None,
            audio_output: "",
        }
    }

    /// 使用静音输出（`ao=null`）：测试与无声卡环境
    pub fn with_null_output(mut self) -> Self {
        self.audio_output = "null";
        self
    }

    fn ensure_transport(&mut self) -> Result<&mut MpvTransport, String> {
        if self.transport.is_none() {
            let lib = self
                .backend
                .library()
                .map_err(|e| format!("音频回退播放不可用：{e}"))?;
            let mut opts = vec![
                ("vo", "null"),
                ("hwdec", "no"),
                ("pause", "yes"),
                ("keep-open", "yes"),
                ("audio-display", "no"),
                ("idle", "yes"),
            ];
            if !self.audio_output.is_empty() {
                opts.push(("ao", self.audio_output));
            }
            let mpv = MpvInstance::new_sandboxed(lib, &opts)
                .map_err(|e| format!("音频回退播放不可用：{e}"))?;
            self.transport = Some(MpvTransport::new(mpv));
        }
        Ok(self.transport.as_mut().expect("just created"))
    }
}

impl AudioFallback for MpvAudioFallback {
    fn load(&mut self, path: &Path) -> Result<(), String> {
        let t = self.ensure_transport()?;
        t.set_paused(true);
        t.load(path).map_err(|e| format!("无法播放此音频：{e}"))
    }

    fn play(&mut self) {
        if let Some(t) = &self.transport {
            t.set_paused(false);
        }
    }

    fn apply(&mut self, cmd: AudioCommand) {
        let Some(t) = &self.transport else { return };
        match cmd {
            AudioCommand::TogglePlay => t.toggle_play(),
            AudioCommand::SeekBy(d) => t.seek_by(d),
            AudioCommand::SeekTo(s) => t.seek_to(s),
            AudioCommand::CycleSpeed => t.cycle_speed(),
            AudioCommand::SetSpeed(s) => t.set_speed(s),
            AudioCommand::SetVolume(v) => t.set_volume(v),
            AudioCommand::ToggleMute => t.toggle_mute(),
            AudioCommand::PrevTrack | AudioCommand::NextTrack | AudioCommand::ToggleAutoNext => {}
        }
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        match &self.transport {
            Some(t) => {
                let s = t.snapshot();
                PlaybackSnapshot {
                    loaded: s.loaded,
                    playing: s.loaded && !s.paused && !s.ended,
                    ended: s.ended,
                    position_secs: s.position_secs,
                    duration_secs: s.duration_secs,
                    speed: s.speed,
                    volume: s.volume,
                    muted: s.muted,
                }
            }
            None => PlaybackSnapshot::default(),
        }
    }

    fn stop(&mut self) {
        if let Some(t) = self.transport.as_mut() {
            t.set_paused(true);
            t.unload();
        }
    }
}
