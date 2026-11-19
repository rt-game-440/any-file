//! 与输出设备无关的 PCM 拉取式播放核心。
//!
//! 输出设备回调（或测试）反复调用 [`PlayerCore::fill`] 拉取交错 `f32` 采样；核心负责
//! 流式解码、线性插值重采样（同时实现倍速）、声道映射、音量/静音、位置推进与“播完停止在末尾”。
//! 倍速通过改变重采样步长实现，音高会随速度变化（v0.1 不做保音高时间拉伸）。

use std::collections::VecDeque;
use std::path::Path;

use super::controls::{SPEED_STEPS, clamp_speed};
use super::decoder::{AudioDecoder, StreamInfo};

/// 播放状态快照（供 UI 与测试读取）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackSnapshot {
    pub loaded: bool,
    pub playing: bool,
    /// 已播完：停止在末尾，`position_secs == duration_secs`
    pub ended: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub speed: f32,
    pub volume: f32,
    pub muted: bool,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            loaded: false,
            playing: false,
            ended: false,
            position_secs: 0.0,
            duration_secs: 0.0,
            speed: 1.0,
            volume: 1.0,
            muted: false,
        }
    }
}

/// 播放核心
pub struct PlayerCore {
    out_rate: u32,
    out_channels: usize,
    decoder: Option<AudioDecoder>,
    info: Option<StreamInfo>,
    /// 已解码、尚未消费的交错源采样
    queue: VecDeque<f32>,
    /// 相邻两个源帧之间的插值相位 `[0, 1)`
    frac: f64,
    /// 本次 seek 起点（源帧）
    base_frames: f64,
    /// 自起点起已消费的源帧数
    consumed_frames: u64,
    /// seek 后需要丢弃的源帧数（精确 seek 修正）
    skip_frames: u64,
    eof: bool,
    playing: bool,
    ended: bool,
    speed: f32,
    volume: f32,
    muted: bool,
    last_error: Option<String>,
}

impl PlayerCore {
    pub fn new(out_rate: u32, out_channels: usize) -> Self {
        Self {
            out_rate: out_rate.max(1),
            out_channels: out_channels.max(1),
            decoder: None,
            info: None,
            queue: VecDeque::new(),
            frac: 0.0,
            base_frames: 0.0,
            consumed_frames: 0,
            skip_frames: 0,
            eof: false,
            playing: false,
            ended: false,
            speed: 1.0,
            volume: 1.0,
            muted: false,
            last_error: None,
        }
    }

    pub fn out_format(&self) -> (u32, usize) {
        (self.out_rate, self.out_channels)
    }

    /// 载入音频文件，停在 0 秒并保持暂停；保留倍速/音量/静音设置。
    pub fn load(&mut self, path: &Path) -> Result<StreamInfo, String> {
        self.unload();
        let decoder = AudioDecoder::open(path)?;
        let info = decoder.info();
        self.decoder = Some(decoder);
        self.info = Some(info);
        Ok(info)
    }

    /// 卸载当前音频并停止播放
    pub fn unload(&mut self) {
        self.decoder = None;
        self.info = None;
        self.reset_stream_state();
        self.playing = false;
        self.ended = false;
        self.last_error = None;
    }

    fn reset_stream_state(&mut self) {
        self.queue.clear();
        self.frac = 0.0;
        self.base_frames = 0.0;
        self.consumed_frames = 0;
        self.skip_frames = 0;
        self.eof = false;
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn is_loaded(&self) -> bool {
        self.decoder.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn play(&mut self) {
        if self.decoder.is_none() {
            return;
        }
        if self.ended {
            self.seek_to(0.0);
        }
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = clamp_speed(speed);
    }

    /// 切换到下一档倍速（循环）
    pub fn cycle_speed(&mut self) {
        let idx = SPEED_STEPS
            .iter()
            .position(|s| (*s - self.speed).abs() < 1e-3)
            .map_or(0, |i| (i + 1) % SPEED_STEPS.len());
        self.speed = SPEED_STEPS[idx];
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
    }

    fn duration_secs(&self) -> f64 {
        self.info.and_then(|i| i.duration_secs).unwrap_or(0.0)
    }

    fn source_rate(&self) -> f64 {
        self.info.map_or(1.0, |i| i.sample_rate as f64)
    }

    fn raw_position_secs(&self) -> f64 {
        (self.base_frames + self.consumed_frames as f64 + self.frac) / self.source_rate()
    }

    /// 跳转到指定秒数（不改变播放/暂停状态；越过末尾则视为播完）
    pub fn seek_to(&mut self, secs: f64) {
        if self.decoder.is_none() {
            return;
        }
        let duration = self.info.and_then(|i| i.duration_secs);
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        if let Some(d) = duration
            && secs >= d
        {
            self.reset_stream_state();
            self.base_frames = d * self.source_rate();
            self.eof = true;
            self.finish();
            return;
        }
        let result = self.decoder.as_mut().map(|d| d.seek(secs));
        match result {
            Some(Ok((actual, skip))) => {
                self.reset_stream_state();
                self.skip_frames = skip;
                // 被丢弃的对齐帧计入起点，使位置从请求点开始
                self.base_frames = actual * self.source_rate() + skip as f64;
                self.ended = false;
            }
            Some(Err(e)) => self.last_error = Some(e),
            None => {}
        }
    }

    /// 相对跳转（秒，可为负）；基于当前位置并夹在 `[0, 时长]`
    pub fn seek_by(&mut self, delta_secs: f64) {
        let current = if self.ended {
            self.duration_secs()
        } else {
            self.raw_position_secs()
        };
        self.seek_to((current + delta_secs).max(0.0));
    }

    fn finish(&mut self) {
        self.playing = false;
        self.ended = true;
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        let duration = self.duration_secs();
        let position = if self.ended {
            duration
        } else if duration > 0.0 {
            self.raw_position_secs().min(duration)
        } else {
            self.raw_position_secs()
        };
        PlaybackSnapshot {
            loaded: self.decoder.is_some(),
            playing: self.playing,
            ended: self.ended,
            position_secs: position,
            duration_secs: duration,
            speed: self.speed,
            volume: self.volume,
            muted: self.muted,
        }
    }

    fn src_channels(&self) -> usize {
        self.info.map_or(1, |i| i.channels).max(1)
    }

    /// 保证队列至少有 `frames` 个源帧（除非流已结束）
    fn ensure_frames(&mut self, frames: usize) {
        let ch = self.src_channels();
        while !self.eof && self.queue.len() / ch < frames {
            let Some(decoder) = self.decoder.as_mut() else {
                self.eof = true;
                return;
            };
            match decoder.next_samples() {
                Ok(Some(samples)) => {
                    self.queue.extend(samples);
                    if self.skip_frames > 0 {
                        let drop_frames = (self.queue.len() / ch).min(self.skip_frames as usize);
                        self.queue.drain(..drop_frames * ch);
                        self.skip_frames -= drop_frames as u64;
                    }
                }
                Ok(None) => self.eof = true,
                Err(e) => {
                    self.last_error = Some(e);
                    self.eof = true;
                }
            }
        }
    }

    /// 拉取 `out.len() / out_channels` 帧输出采样。暂停/未载入/播完时输出静音。
    pub fn fill(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if !self.playing || self.decoder.is_none() {
            return;
        }
        let src_ch = self.src_channels();
        let out_ch = self.out_channels;
        let step = self.source_rate() / self.out_rate as f64 * self.speed as f64;
        let gain = if self.muted { 0.0 } else { self.volume };

        for frame in out.chunks_mut(out_ch) {
            self.ensure_frames(2);
            let frames = self.queue.len() / src_ch;
            if frames == 0 {
                self.finish();
                return;
            }
            let next = if frames >= 2 { 1 } else { 0 };
            let t = self.frac as f32;
            for (c, slot) in frame.iter_mut().enumerate() {
                let sc = if src_ch == 1 { 0 } else { c % src_ch };
                let a = self.queue[sc];
                let b = self.queue[next * src_ch + sc];
                *slot = (a + (b - a) * t) * gain;
            }
            self.frac += step;
            while self.frac >= 1.0 && self.queue.len() >= src_ch {
                self.queue.drain(..src_ch);
                self.consumed_frames += 1;
                self.frac -= 1.0;
            }
            if self.queue.is_empty() {
                self.ensure_frames(1);
                if self.queue.is_empty() {
                    self.finish();
                    return;
                }
            }
        }
    }
}
