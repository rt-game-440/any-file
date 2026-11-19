//! 音频视图控制器：与 `AppSession` 联动。
//! 负责切文件即停播、同目录切歌、播完停止 / 自动播放下一首，以及向 UI 提供展示状态。

use std::path::{Path, PathBuf};

use crate::domain::ViewerKind;
use crate::session::AppSession;

use super::controls::{AudioCommand, AudioKey, KeyModifiers, format_speed, format_time, map_key};
use super::core::PlaybackSnapshot;
use super::metadata::{AudioMetadata, read_metadata};
use super::player::AudioPlayer;

/// 供 UI 绑定的展示状态
#[derive(Debug, Clone, PartialEq)]
pub struct AudioViewState {
    pub snapshot: PlaybackSnapshot,
    pub metadata: AudioMetadata,
    pub position_text: String,
    pub duration_text: String,
    pub speed_text: String,
    pub auto_next: bool,
    pub error: Option<String>,
}

/// 音频视图控制器
pub struct AudioViewer {
    player: AudioPlayer,
    auto_next: bool,
    autoplay_on_load: bool,
    current: Option<PathBuf>,
    metadata: AudioMetadata,
    error: Option<String>,
}

impl AudioViewer {
    pub fn new(player: AudioPlayer) -> Self {
        Self {
            player,
            auto_next: false,
            autoplay_on_load: true,
            current: None,
            metadata: AudioMetadata::default(),
            error: None,
        }
    }

    /// 关闭“载入后自动开始播放”（测试用）
    pub fn set_autoplay_on_load(&mut self, autoplay: bool) {
        self.autoplay_on_load = autoplay;
    }

    pub fn auto_next(&self) -> bool {
        self.auto_next
    }

    pub fn set_auto_next(&mut self, on: bool) {
        self.auto_next = on;
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    pub fn player(&self) -> &AudioPlayer {
        &self.player
    }

    pub fn player_mut(&mut self) -> &mut AudioPlayer {
        &mut self.player
    }

    /// 立即停止并释放当前音频输出
    pub fn stop(&mut self) {
        self.player.stop();
        self.current = None;
        self.error = None;
    }

    /// 与会话同步：当前为音频视图且文件变化则载入新文件；否则（切到其他文件项/视图）立即停播。
    /// 返回是否发生了载入或停止（UI 需要刷新）。
    pub fn sync_with_session(&mut self, session: &AppSession) -> bool {
        let target = (session.active_viewer() == ViewerKind::Audio)
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
                if self.current.is_some() {
                    self.stop();
                    true
                } else {
                    false
                }
            }
        }
    }

    fn load(&mut self, path: &Path) {
        self.player.stop();
        self.metadata = read_metadata(path);
        self.current = Some(path.to_path_buf());
        match self.player.load(path) {
            Ok(info) => {
                self.error = None;
                if self.metadata.duration_secs.is_none() {
                    self.metadata.duration_secs = info.duration_secs;
                }
                if self.autoplay_on_load {
                    self.player.play();
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// 处理键盘事件；返回是否被音频视图消费
    pub fn handle_key(&mut self, key: AudioKey, mods: KeyModifiers, session: &AppSession) -> bool {
        match map_key(key, mods) {
            Some(cmd) => {
                self.handle_command(cmd, session);
                true
            }
            None => false,
        }
    }

    /// 执行命令；切歌命令通过会话导航同目录序列
    pub fn handle_command(&mut self, cmd: AudioCommand, session: &AppSession) {
        match cmd {
            AudioCommand::PrevTrack => {
                if session.navigate_prev().is_ok_and(|e| e.is_some()) {
                    self.sync_with_session(session);
                }
            }
            AudioCommand::NextTrack => {
                if session.navigate_next().is_ok_and(|e| e.is_some()) {
                    self.sync_with_session(session);
                }
            }
            AudioCommand::ToggleAutoNext => self.auto_next = !self.auto_next,
            other => self.player.apply(other),
        }
    }

    /// 周期轮询：播完且开启自动播放下一首时切到同目录下一项。返回是否切换了曲目。
    pub fn poll(&mut self, session: &AppSession) -> bool {
        let snap = self.player.snapshot();
        // 已是最后一首时 navigate_next 无结果：停在末尾
        if snap.loaded
            && snap.ended
            && self.auto_next
            && session.navigate_next().is_ok_and(|e| e.is_some())
        {
            return self.sync_with_session(session);
        }
        false
    }

    pub fn view_state(&self) -> AudioViewState {
        let snapshot = self.player.snapshot();
        AudioViewState {
            position_text: format_time(snapshot.position_secs),
            duration_text: format_time(snapshot.duration_secs),
            speed_text: format_speed(snapshot.speed),
            snapshot,
            metadata: self.metadata.clone(),
            auto_next: self.auto_next,
            error: self
                .error
                .clone()
                .or_else(|| self.player.device_error().map(str::to_string)),
        }
    }
}
