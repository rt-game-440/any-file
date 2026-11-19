//! `PlayerCore` + cpal 输出流封装。
//!
//! 输出流按需创建并随播放状态 `play()/pause()`，暂停与播完后不占用音频回调。
//! 无输出设备或创建失败时降级为 headless（核心不被设备拉取，测试可用 [`AudioPlayer::pump`] 手动驱动）。

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::controls::AudioCommand;
use super::core::{PlaybackSnapshot, PlayerCore};
use super::decoder::StreamInfo;

/// 音频播放器（UI 线程使用；`cpal::Stream` 不可跨线程移动）
pub struct AudioPlayer {
    core: Arc<Mutex<PlayerCore>>,
    stream: Option<cpal::Stream>,
    device_error: Option<String>,
}

fn open_default_stream(core: &Arc<Mutex<PlayerCore>>) -> Result<(cpal::Stream, u32, usize), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "未找到可用的音频输出设备".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("无法获取输出配置：{e}"))?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();
    {
        let mut c = core.lock().unwrap();
        let mut fresh = PlayerCore::new(rate, channels);
        // 保留已有的音量/倍速设置：仅在尚未载入时替换核心
        if !c.is_loaded() {
            let snap = c.snapshot();
            fresh.set_speed(snap.speed);
            fresh.set_volume(snap.volume);
            fresh.set_muted(snap.muted);
            *c = fresh;
        }
    }
    let err_fn = |e| eprintln!("音频输出流错误：{e}");
    let core_cb = Arc::clone(core);
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _| {
                if let Ok(mut c) = core_cb.try_lock() {
                    c.fill(data);
                } else {
                    data.fill(0.0);
                }
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => {
            let mut tmp: Vec<f32> = Vec::new();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| {
                    tmp.resize(data.len(), 0.0);
                    if let Ok(mut c) = core_cb.try_lock() {
                        c.fill(&mut tmp);
                    } else {
                        tmp.fill(0.0);
                    }
                    for (o, s) in data.iter_mut().zip(&tmp) {
                        *o = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                err_fn,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let mut tmp: Vec<f32> = Vec::new();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [u16], _| {
                    tmp.resize(data.len(), 0.0);
                    if let Ok(mut c) = core_cb.try_lock() {
                        c.fill(&mut tmp);
                    } else {
                        tmp.fill(0.0);
                    }
                    for (o, s) in data.iter_mut().zip(&tmp) {
                        *o = ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                    }
                },
                err_fn,
                None,
            )
        }
        other => return Err(format!("不支持的输出采样格式：{other:?}")),
    }
    .map_err(|e| format!("无法创建音频输出流：{e}"))?;
    Ok((stream, rate, channels))
}

impl AudioPlayer {
    /// 使用系统默认输出设备；失败时降级为 headless（`device_error()` 给出原因）
    pub fn with_default_device() -> Self {
        let core = Arc::new(Mutex::new(PlayerCore::new(48_000, 2)));
        match open_default_stream(&core) {
            Ok((stream, _, _)) => {
                let _ = stream.pause();
                Self {
                    core,
                    stream: Some(stream),
                    device_error: None,
                }
            }
            Err(e) => Self {
                core,
                stream: None,
                device_error: Some(e),
            },
        }
    }

    /// 无设备模式（测试与无声卡环境）
    pub fn headless(out_rate: u32, out_channels: usize) -> Self {
        Self {
            core: Arc::new(Mutex::new(PlayerCore::new(out_rate, out_channels))),
            stream: None,
            device_error: None,
        }
    }

    pub fn device_error(&self) -> Option<&str> {
        self.device_error.as_deref()
    }

    fn core(&self) -> MutexGuard<'_, PlayerCore> {
        self.core.lock().unwrap()
    }

    /// 载入文件（暂停在 0 秒）
    pub fn load(&mut self, path: &Path) -> Result<StreamInfo, String> {
        let r = self.core().load(path);
        self.sync_stream();
        r
    }

    /// 立即停止并释放当前音频
    pub fn stop(&mut self) {
        self.core().unload();
        self.sync_stream();
    }

    /// 执行播放类命令（曲目切换/自动连播命令由上层 `AudioViewer` 处理）
    pub fn apply(&mut self, cmd: AudioCommand) {
        {
            let mut c = self.core();
            match cmd {
                AudioCommand::TogglePlay => c.toggle_play(),
                AudioCommand::SeekBy(d) => c.seek_by(d),
                AudioCommand::SeekTo(s) => c.seek_to(s),
                AudioCommand::CycleSpeed => c.cycle_speed(),
                AudioCommand::SetSpeed(s) => c.set_speed(s),
                AudioCommand::SetVolume(v) => {
                    c.set_volume(v);
                    if v > 0.0 {
                        c.set_muted(false);
                    }
                }
                AudioCommand::ToggleMute => c.toggle_mute(),
                AudioCommand::PrevTrack | AudioCommand::NextTrack | AudioCommand::ToggleAutoNext => {}
            }
        }
        self.sync_stream();
    }

    pub fn play(&mut self) {
        self.core().play();
        self.sync_stream();
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        self.core().snapshot()
    }

    /// 根据核心播放状态启停输出流（并同步播完后的停止状态）
    pub fn sync_stream(&mut self) {
        let playing = self.core().is_playing();
        if let Some(stream) = &self.stream {
            if playing {
                let _ = stream.play();
            } else {
                let _ = stream.pause();
            }
        }
    }

    /// headless 模式下手动拉取 `frames` 个输出帧（模拟设备回调），返回交错采样
    pub fn pump(&mut self, frames: usize) -> Vec<f32> {
        let mut c = self.core();
        let (_, ch) = c.out_format();
        let mut buf = vec![0.0; frames * ch];
        c.fill(&mut buf);
        buf
    }
}
