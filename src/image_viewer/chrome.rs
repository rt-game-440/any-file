//! 沉浸式界面（标题栏 / 底部工具栏）的“静止即淡出”逻辑：纯状态机，时间显式传入。

/// 鼠标 / 键盘静止多久后隐藏界面
pub const IDLE_HIDE_SECS: f64 = 2.5;

#[derive(Debug, Clone)]
pub struct ChromeIdle {
    visible: bool,
    last_activity: f64,
    /// 是否有强制保持显示的原因（正在拖拽、鼠标悬停在工具栏上等）
    enabled: bool,
}

impl ChromeIdle {
    pub fn new(now: f64) -> Self {
        Self { visible: true, last_activity: now, enabled: true }
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// 任意鼠标移动 / 点击 / 按键
    pub fn activity(&mut self, now: f64) {
        self.last_activity = now;
        self.visible = true;
    }

    /// 周期调用；`hold` 为真时视为持续活动。返回可见性是否发生变化。
    pub fn tick(&mut self, now: f64, hold: bool) -> bool {
        if hold {
            self.last_activity = now;
        }
        let want = !self.enabled || now - self.last_activity < IDLE_HIDE_SECS;
        let changed = want != self.visible;
        self.visible = want;
        changed
    }

    /// 非图像视图时关闭淡出（始终可见）
    pub fn set_enabled(&mut self, enabled: bool, now: f64) {
        self.enabled = enabled;
        self.activity(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_after_idle_and_reappears_on_activity() {
        let mut c = ChromeIdle::new(0.0);
        assert!(!c.tick(1.0, false));
        assert!(c.visible());
        assert!(c.tick(IDLE_HIDE_SECS + 0.1, false));
        assert!(!c.visible());
        c.activity(10.0);
        assert!(c.visible());
        assert!(!c.tick(10.5, false));
    }

    #[test]
    fn hold_keeps_visible() {
        let mut c = ChromeIdle::new(0.0);
        for i in 1..100 {
            c.tick(f64::from(i), true);
        }
        assert!(c.visible());
        assert!(c.tick(100.0 + IDLE_HIDE_SECS + 0.1, false));
    }

    #[test]
    fn disabled_is_always_visible() {
        let mut c = ChromeIdle::new(0.0);
        c.set_enabled(false, 0.0);
        c.tick(1000.0, false);
        assert!(c.visible());
    }
}
