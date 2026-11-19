//! 播放控制的纯逻辑：快捷键状态机、倍速档位、时间格式化。
//! 与解码/设备无关，音频视图与后续视频视图（#9）共用。

/// 倍速档位（循环切换）
pub const SPEED_STEPS: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// 快进/快退步长（秒）
pub const SEEK_STEP_SECS: f64 = 5.0;

/// 把任意倍速夹到支持范围 `[0.5, 2.0]`
pub fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(SPEED_STEPS[0], SPEED_STEPS[SPEED_STEPS.len() - 1])
    } else {
        1.0
    }
}

/// 播放控制命令
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioCommand {
    TogglePlay,
    /// 相对跳转（秒）
    SeekBy(f64),
    /// 绝对跳转（秒）
    SeekTo(f64),
    CycleSpeed,
    SetSpeed(f32),
    SetVolume(f32),
    ToggleMute,
    /// 同目录上一首
    PrevTrack,
    /// 同目录下一首
    NextTrack,
    ToggleAutoNext,
}

/// 音频视图关心的按键
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioKey {
    Space,
    Left,
    Right,
    PageUp,
    PageDown,
    M,
}

/// 修饰键状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
}

/// 快捷键映射：
/// - Space 播放/暂停；← / → 快退/快进 5 秒；M 静音；
/// - PageUp / PageDown 或 Ctrl+← / Ctrl+→ 切换上一首/下一首。
pub fn map_key(key: AudioKey, mods: KeyModifiers) -> Option<AudioCommand> {
    Some(match (key, mods.ctrl) {
        (AudioKey::Space, false) => AudioCommand::TogglePlay,
        (AudioKey::Left, false) => AudioCommand::SeekBy(-SEEK_STEP_SECS),
        (AudioKey::Right, false) => AudioCommand::SeekBy(SEEK_STEP_SECS),
        (AudioKey::Left, true) | (AudioKey::PageUp, _) => AudioCommand::PrevTrack,
        (AudioKey::Right, true) | (AudioKey::PageDown, _) => AudioCommand::NextTrack,
        (AudioKey::M, false) => AudioCommand::ToggleMute,
        _ => return None,
    })
}

/// 把秒数格式化为 `m:ss` 或 `h:mm:ss`
pub fn format_time(secs: f64) -> String {
    let total = if secs.is_finite() && secs > 0.0 {
        secs.floor() as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// 倍速显示文本，如 `1.25x`、`1x`
pub fn format_speed(speed: f32) -> String {
    let s = format!("{speed:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}x")
}
