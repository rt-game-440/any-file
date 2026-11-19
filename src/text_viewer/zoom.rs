//! Ctrl+滚轮 字号缩放规则。

pub const DEFAULT_FONT_SIZE: f32 = 14.0;
pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 40.0;
const ZOOM_STEP: f32 = 1.0;

/// 将字号限制在 `[MIN_FONT_SIZE, MAX_FONT_SIZE]`。
pub fn clamp_font_size(size: f32) -> f32 {
    if size.is_finite() {
        size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    } else {
        DEFAULT_FONT_SIZE
    }
}

/// 根据滚轮增量计算新字号：向上滚（`delta_y > 0`）放大，向下缩小；未按 Ctrl 时不变。
pub fn zoom_for_wheel(current: f32, delta_y: f32, ctrl_pressed: bool) -> f32 {
    if !ctrl_pressed || delta_y == 0.0 || !delta_y.is_finite() {
        return clamp_font_size(current);
    }
    let step = if delta_y > 0.0 { ZOOM_STEP } else { -ZOOM_STEP };
    clamp_font_size(current + step)
}
