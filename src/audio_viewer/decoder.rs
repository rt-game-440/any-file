//! 基于 symphonia 的流式音频解码器：按包解码为交错 `f32` PCM，支持精确 seek。

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::probing::to_io_path;

/// 音频流基本信息
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamInfo {
    pub sample_rate: u32,
    pub channels: usize,
    /// 总时长（秒）；容器未给出时为 `None`
    pub duration_secs: Option<f64>,
}

/// 流式音频解码器
pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    info: StreamInfo,
    sample_buf: Option<SampleBuffer<f32>>,
}

impl AudioDecoder {
    /// 打开音频文件并创建解码器
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(to_io_path(path)).map_err(|e| format!("无法打开音频文件：{e}"))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("无法识别音频格式：{e}"))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "文件中没有可解码的音频轨道".to_string())?;
        let params = track.codec_params.clone();
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| format!("不支持的音频编码：{e}"))?;
        let sample_rate = params.sample_rate.ok_or("音频缺少采样率信息")?;
        let channels = params.channels.map(|c| c.count()).unwrap_or(0);
        let duration_secs = params
            .n_frames
            .map(|n| n as f64 / sample_rate as f64)
            .filter(|d| *d > 0.0);
        Ok(Self {
            format,
            decoder,
            track_id,
            info: StreamInfo {
                sample_rate,
                channels: channels.max(1),
                duration_secs,
            },
            sample_buf: None,
        })
    }

    pub fn info(&self) -> StreamInfo {
        self.info
    }

    /// 解码下一个数据包，返回交错 PCM 采样；流结束返回 `Ok(None)`。
    /// 单个损坏数据包会被跳过，不中断播放。
    pub fn next_samples(&mut self) -> Result<Option<Vec<f32>>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(SymError::ResetRequired) => return Ok(None),
                Err(e) => return Err(format!("读取音频数据失败：{e}")),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let channels = spec.channels.count().max(1);
                    self.info.channels = channels;
                    let needed = decoded.capacity() as u64;
                    let buf = match &mut self.sample_buf {
                        Some(b) if b.capacity() as u64 >= needed * channels as u64 => b,
                        slot => slot.insert(SampleBuffer::<f32>::new(needed, spec)),
                    };
                    buf.copy_interleaved_ref(decoded);
                    if buf.samples().is_empty() {
                        continue;
                    }
                    return Ok(Some(buf.samples().to_vec()));
                }
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
                Err(e) => return Err(format!("解码失败：{e}")),
            }
        }
    }

    /// 跳转到指定秒数，返回实际落点（秒，已按精确 seek 修正）及需要丢弃的帧数。
    pub fn seek(&mut self, secs: f64) -> Result<(f64, u64), String> {
        let secs = secs.max(0.0);
        let time = Time::from(secs);
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| format!("跳转失败：{e}"))?;
        self.decoder.reset();
        let rate = self.info.sample_rate as f64;
        let actual_secs = seeked.actual_ts as f64 / rate;
        let skip = seeked.required_ts.saturating_sub(seeked.actual_ts);
        Ok((actual_secs, skip))
    }
}
