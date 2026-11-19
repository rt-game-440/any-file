//! 音频标签与封面读取（lofty）。读取失败时回退为文件名，不影响播放。

use std::path::Path;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::Accessor;

use crate::probing::to_io_path;

/// 解码后的封面（RGBA8）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 音频基础元数据
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioMetadata {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub cover: Option<CoverImage>,
}

const COVER_MAX_EDGE: u32 = 512;

fn decode_cover(data: &[u8]) -> Option<CoverImage> {
    let img = image::load_from_memory(data).ok()?;
    let img = if img.width().max(img.height()) > COVER_MAX_EDGE {
        img.thumbnail(COVER_MAX_EDGE, COVER_MAX_EDGE)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(CoverImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// 读取元数据；标题缺失时使用文件名（不含扩展名）
pub fn read_metadata(path: &Path) -> AudioMetadata {
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("未命名音频")
        .to_string();
    let mut meta = AudioMetadata {
        title: file_stem,
        ..Default::default()
    };
    let Ok(tagged) = lofty::read_from_path(to_io_path(path)) else {
        return meta;
    };
    let props = tagged.properties();
    let dur = props.duration().as_secs_f64();
    if dur > 0.0 {
        meta.duration_secs = Some(dur);
    }
    meta.sample_rate = props.sample_rate();
    meta.channels = props.channels();
    meta.bitrate_kbps = props.audio_bitrate();

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
            meta.title = t.trim().to_string();
        }
        meta.artist = tag
            .artist()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        meta.album = tag
            .album()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        meta.cover = tag.pictures().iter().find_map(|p| decode_cover(p.data()));
    }
    meta
}
