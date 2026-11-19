//! 异步两阶段加载：先发 EXIF 缩略图 / 降采样预览（几乎瞬时），再发完整解码结果。
//! 工作线程通过 mpsc 通道回传，UI 线程在定时器里轮询；用世代号丢弃过期结果。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use image::RgbaImage;

use super::decode::{DecodedImage, decode_file, exif_thumbnail};

pub enum LoadEvent {
    /// 低清预览（保持与完整图相同的长宽比）
    Preview { generation: u64, image: RgbaImage },
    Full { generation: u64, image: DecodedImage },
    Failed { generation: u64, message: String },
}

impl LoadEvent {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Preview { generation, .. }
            | Self::Full { generation, .. }
            | Self::Failed { generation, .. } => *generation,
        }
    }
}

/// 启动后台加载，返回事件接收端。
pub fn spawn_load(path: PathBuf, generation: u64) -> Receiver<LoadEvent> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("image-loader".into())
        .spawn(move || run(&path, generation, &tx))
        .ok();
    rx
}

fn run(path: &Path, generation: u64, tx: &Sender<LoadEvent>) {
    if let Some(thumb) = exif_thumbnail(path) {
        let _ = tx.send(LoadEvent::Preview { generation, image: thumb });
    }
    match decode_file(path) {
        Ok(full) => {
            let _ = tx.send(LoadEvent::Full { generation, image: full });
        }
        Err(e) => {
            let _ = tx.send(LoadEvent::Failed { generation, message: e.to_string() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn loads_png_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("a.png");
        RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255])).save(&ok).unwrap();
        let rx = spawn_load(ok, 7);
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            LoadEvent::Full { generation, image } => {
                assert_eq!(generation, 7);
                assert_eq!((image.width(), image.height()), (8, 4));
            }
            _ => panic!("应直接得到完整结果"),
        }
        let bad = dir.path().join("b.png");
        std::fs::write(&bad, b"junk").unwrap();
        let rx = spawn_load(bad, 8);
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), LoadEvent::Failed { generation: 8, .. }));
    }
}
