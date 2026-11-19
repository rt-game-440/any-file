//! 图像变换状态机（纯逻辑，时间显式传入，便于确定性测试）：
//! 适应窗口 / 1:1、滚轮缩放（光标锚点 + 物理弹簧回弹）、拖拽平移 + 惯性、旋转弹簧。

/// 缩放极限（相对于 1:1 的绝对比例）
pub const MIN_ZOOM: f64 = 0.02;
pub const MAX_ZOOM: f64 = 64.0;
/// 滚轮停止多久后触发越界回弹
const WHEEL_SETTLE_SECS: f64 = 0.12;
/// 弹簧参数（略欠阻尼，带轻微回弹手感）
const STIFFNESS: f64 = 260.0;
const DAMPING: f64 = 24.0;
/// 惯性摩擦（每秒速度衰减系数）
const FRICTION_PER_SEC: f64 = 5.0;
const SUBSTEP: f64 = 1.0 / 240.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    Fit,
    Actual,
    Custom,
}

#[derive(Debug, Clone)]
pub struct ImageTransform {
    img_w: f64,
    img_h: f64,
    view_w: f64,
    view_h: f64,
    /// 旋转目标（四分之一圈数，可为负，累计）
    quarter_turns: i32,
    angle_deg: f64,
    angle_vel: f64,
    scale: f64,
    scale_target: f64,
    scale_vel: f64,
    /// 图像中心相对视口中心的偏移（像素）
    pan: (f64, f64),
    pan_vel: (f64, f64),
    mode: FitMode,
    reduced_motion: bool,
    /// 缩放锚点（视口中心坐标系）
    anchor: (f64, f64),
    last_wheel: Option<f64>,
    dragging: bool,
    drag_last: (f64, f64, f64),
    now: f64,
}

impl ImageTransform {
    pub fn new(img_w: u32, img_h: u32, view_w: f64, view_h: f64) -> Self {
        let mut t = Self {
            img_w: f64::from(img_w.max(1)),
            img_h: f64::from(img_h.max(1)),
            view_w: view_w.max(1.0),
            view_h: view_h.max(1.0),
            quarter_turns: 0,
            angle_deg: 0.0,
            angle_vel: 0.0,
            scale: 1.0,
            scale_target: 1.0,
            scale_vel: 0.0,
            pan: (0.0, 0.0),
            pan_vel: (0.0, 0.0),
            mode: FitMode::Fit,
            reduced_motion: false,
            anchor: (0.0, 0.0),
            last_wheel: None,
            dragging: false,
            drag_last: (0.0, 0.0, 0.0),
            now: 0.0,
        };
        t.scale = t.fit_scale();
        t.scale_target = t.scale;
        t
    }

    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
    }
    pub fn mode(&self) -> FitMode {
        self.mode
    }
    pub fn scale(&self) -> f64 {
        self.scale
    }
    pub fn scale_target(&self) -> f64 {
        self.scale_target
    }
    pub fn pan(&self) -> (f64, f64) {
        self.pan
    }
    pub fn angle_deg(&self) -> f64 {
        self.angle_deg
    }
    pub fn quarter_turns(&self) -> i32 {
        self.quarter_turns
    }
    pub fn image_size(&self) -> (f64, f64) {
        (self.img_w, self.img_h)
    }

    /// 旋转目标下的有效图像尺寸
    fn eff_size(&self) -> (f64, f64) {
        if self.quarter_turns.rem_euclid(2) == 1 {
            (self.img_h, self.img_w)
        } else {
            (self.img_w, self.img_h)
        }
    }

    pub fn fit_scale(&self) -> f64 {
        let (w, h) = self.eff_size();
        (self.view_w / w).min(self.view_h / h).clamp(MIN_ZOOM, MAX_ZOOM)
    }

    /// 缩放比例的下限：不小于“适应窗口”的 0.5 倍（过小无意义）且不低于全局下限
    fn min_scale(&self) -> f64 {
        self.fit_scale().clamp(MIN_ZOOM, 1.0)
    }

    fn max_scale(&self) -> f64 {
        // 显示尺寸上限约 4 万像素，避免渲染器处理超大纹理
        MAX_ZOOM.min(40000.0 / self.img_w.max(self.img_h)).max(1.0)
    }

    /// 以给定缩放比例，在当前旋转下允许的最大平移（半幅）
    fn pan_limit(&self, scale: f64) -> (f64, f64) {
        let (w, h) = self.eff_size();
        (
            ((w * scale - self.view_w) / 2.0).max(0.0),
            ((h * scale - self.view_h) / 2.0).max(0.0),
        )
    }

    pub fn set_viewport(&mut self, w: f64, h: f64) {
        self.view_w = w.max(1.0);
        self.view_h = h.max(1.0);
        match self.mode {
            FitMode::Fit => {
                self.scale = self.fit_scale();
                self.scale_target = self.scale;
                self.pan = (0.0, 0.0);
            }
            _ => {
                let (lx, ly) = self.pan_limit(self.scale);
                self.pan = (self.pan.0.clamp(-lx, lx), self.pan.1.clamp(-ly, ly));
            }
        }
    }

    pub fn fit(&mut self) {
        self.mode = FitMode::Fit;
        self.scale_target = self.fit_scale();
        self.anchor = (0.0, 0.0);
        self.pan_vel = (0.0, 0.0);
        self.settle_if_reduced();
    }

    /// 1:1 原始像素，以 `at`（视口中心坐标系）为锚点
    pub fn actual(&mut self, at: (f64, f64)) {
        self.mode = FitMode::Actual;
        self.scale_target = 1.0;
        self.anchor = at;
        self.pan_vel = (0.0, 0.0);
        self.settle_if_reduced();
    }

    /// 双击：适应 <-> 1:1
    pub fn toggle_fit_actual(&mut self, at: (f64, f64)) {
        if self.mode == FitMode::Fit && (self.fit_scale() - 1.0).abs() > 1e-6 {
            self.actual(at);
        } else if self.mode == FitMode::Fit {
            // 图像恰好等于窗口：放大到 2 倍体现切换
            self.mode = FitMode::Custom;
            self.scale_target = 2.0;
            self.anchor = at;
        } else {
            self.fit();
        }
    }

    pub fn rotate(&mut self, clockwise: bool) {
        self.quarter_turns += if clockwise { 1 } else { -1 };
        if self.mode == FitMode::Fit {
            self.scale_target = self.fit_scale();
            self.pan = (0.0, 0.0);
        }
        self.settle_if_reduced();
    }

    /// 滚轮缩放：`delta` 为向上为正的滚动量（每格约 120 或 1 单位均可，内部归一）
    pub fn wheel(&mut self, delta: f64, at: (f64, f64), now: f64) {
        self.now = now;
        // 指数映射：一格滚轮（约 60~120 单位）约放大 10%~20%，触控板小增量平滑连续
        let factor = (delta.clamp(-480.0, 480.0) * 0.0016).exp();
        let lo = self.min_scale() * 0.6;
        let hi = self.max_scale() * 1.5;
        self.scale_target = (self.scale_target * factor).clamp(lo, hi);
        self.anchor = at;
        self.mode = FitMode::Custom;
        self.last_wheel = Some(now);
        self.pan_vel = (0.0, 0.0);
        self.settle_if_reduced();
    }

    pub fn drag_start(&mut self, x: f64, y: f64, now: f64) {
        self.now = now;
        self.dragging = true;
        self.drag_last = (x, y, now);
        self.pan_vel = (0.0, 0.0);
    }

    pub fn drag_move(&mut self, x: f64, y: f64, now: f64) {
        if !self.dragging {
            return;
        }
        let (lx, ly, lt) = self.drag_last;
        let (dx, dy) = (x - lx, y - ly);
        let dt = (now - lt).max(1e-4);
        let (limx, limy) = self.pan_limit(self.scale);
        // 越界橡皮筋阻尼
        let damp = |p: f64, lim: f64, d: f64| {
            let out = p.abs() - lim;
            if out > 0.0 && p.signum() == d.signum() { d * 0.35 } else { d }
        };
        self.pan.0 += damp(self.pan.0, limx, dx);
        self.pan.1 += damp(self.pan.1, limy, dy);
        // 速度低通
        let (vx, vy) = (dx / dt, dy / dt);
        self.pan_vel = (self.pan_vel.0 * 0.5 + vx * 0.5, self.pan_vel.1 * 0.5 + vy * 0.5);
        self.drag_last = (x, y, now);
        if self.mode == FitMode::Fit && (limx > 0.0 || limy > 0.0) {
            self.mode = FitMode::Custom;
        }
    }

    pub fn drag_end(&mut self, now: f64) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        // 停顿后松手无惯性
        if now - self.drag_last.2 > 0.08 || self.reduced_motion {
            self.pan_vel = (0.0, 0.0);
        }
        if self.reduced_motion {
            self.clamp_pan_now();
        }
    }

    fn clamp_pan_now(&mut self) {
        let (lx, ly) = self.pan_limit(self.scale);
        self.pan = (self.pan.0.clamp(-lx, lx), self.pan.1.clamp(-ly, ly));
    }

    fn settle_if_reduced(&mut self) {
        if self.reduced_motion {
            self.apply_anchor_to(self.scale_target);
            self.scale = self.scale_target;
            self.scale_vel = 0.0;
            self.angle_deg = f64::from(self.quarter_turns) * 90.0;
            self.angle_vel = 0.0;
            self.settle_bounds();
        }
    }

    fn settle_bounds(&mut self) {
        let t = self.scale_target.clamp(self.min_scale(), self.max_scale());
        if (t - self.scale).abs() > 1e-9 {
            self.scale = t;
            self.scale_target = t;
        }
        if self.mode == FitMode::Fit {
            self.pan = (0.0, 0.0);
        } else {
            self.clamp_pan_now();
        }
    }

    /// 缩放变化时保持锚点下的图像点不动
    fn apply_anchor_to(&mut self, new_scale: f64) {
        if self.scale.abs() < 1e-12 {
            return;
        }
        let r = new_scale / self.scale;
        self.pan.0 = self.anchor.0 - (self.anchor.0 - self.pan.0) * r;
        self.pan.1 = self.anchor.1 - (self.anchor.1 - self.pan.1) * r;
    }

    pub fn is_animating(&self) -> bool {
        let angle_target = f64::from(self.quarter_turns) * 90.0;
        let (limx, limy) = self.pan_limit(self.scale);
        let oob = self.pan.0.abs() > limx + 0.01 || self.pan.1.abs() > limy + 0.01;
        self.dragging
            || self.last_wheel.is_some()
            || (self.scale - self.scale_target).abs() > 1e-4
            || self.scale_vel.abs() > 1e-3
            || (self.angle_deg - angle_target).abs() > 0.01
            || self.angle_vel.abs() > 0.01
            || self.pan_vel.0.abs() > 0.5
            || self.pan_vel.1.abs() > 0.5
            || oob
    }

    /// 推进 `dt` 秒（内部子步进保证稳定）
    pub fn tick(&mut self, dt: f64, now: f64) {
        self.now = now;
        let mut left = dt.clamp(0.0, 0.05);
        while left > 1e-9 {
            let h = left.min(SUBSTEP);
            self.step(h);
            left -= h;
        }
        if let Some(t) = self.last_wheel
            && now - t >= WHEEL_SETTLE_SECS
        {
            self.last_wheel = None;
            // 滚轮结束：越界目标回弹到合法范围
            self.scale_target = self.scale_target.clamp(self.min_scale(), self.max_scale());
        }
        if !self.is_animating() {
            self.settle_snap();
        }
    }

    fn settle_snap(&mut self) {
        self.scale = self.scale_target;
        self.scale_vel = 0.0;
        self.angle_deg = f64::from(self.quarter_turns) * 90.0;
        self.angle_vel = 0.0;
        self.pan_vel = (0.0, 0.0);
        if self.mode == FitMode::Fit {
            self.pan = (0.0, 0.0);
        }
    }

    fn step(&mut self, h: f64) {
        // 缩放弹簧（对数空间，使放大缩小手感对称）
        let ls = self.scale.ln();
        let lt = self.scale_target.ln();
        let acc = STIFFNESS * (lt - ls) - DAMPING * self.scale_vel;
        self.scale_vel += acc * h;
        let new_scale = (ls + self.scale_vel * h).exp();
        self.apply_anchor_to(new_scale);
        self.scale = new_scale;

        // 旋转弹簧
        let at = f64::from(self.quarter_turns) * 90.0;
        let aacc = STIFFNESS * (at - self.angle_deg) - DAMPING * self.angle_vel;
        self.angle_vel += aacc * h;
        self.angle_deg += self.angle_vel * h;

        if self.dragging {
            return;
        }
        let (lx, ly) = if self.mode == FitMode::Fit { (0.0, 0.0) } else { self.pan_limit(self.scale) };
        // 平移：惯性 + 越界弹簧回拉
        let fr = (-FRICTION_PER_SEC * h).exp();
        for axis in 0..2 {
            let (p, v, lim) = if axis == 0 { (self.pan.0, self.pan_vel.0, lx) } else { (self.pan.1, self.pan_vel.1, ly) };
            let over = p.abs() - lim;
            let mut nv = v;
            if over > 0.0 {
                nv += (-STIFFNESS * over * p.signum() - 2.0 * STIFFNESS.sqrt() * v) * h;
            } else {
                nv *= fr;
            }
            let np = p + nv * h;
            if axis == 0 {
                self.pan.0 = np;
                self.pan_vel.0 = nv;
            } else {
                self.pan.1 = np;
                self.pan_vel.1 = nv;
            }
        }
    }

    /// 预览图被完整图替换：保持画面上的视觉尺寸不变（或在适应 / 1:1 模式下重新套用对应比例）
    pub fn replace_image(&mut self, img_w: u32, img_h: u32) {
        let (nw, nh) = (f64::from(img_w.max(1)), f64::from(img_h.max(1)));
        let ratio = self.img_w / nw;
        self.img_w = nw;
        self.img_h = nh;
        match self.mode {
            FitMode::Fit => {
                self.scale = self.fit_scale();
                self.scale_target = self.scale;
                self.pan = (0.0, 0.0);
            }
            FitMode::Actual => {
                self.scale = 1.0;
                self.scale_target = 1.0;
                self.clamp_pan_now();
            }
            FitMode::Custom => {
                self.scale *= ratio;
                self.scale_target *= ratio;
                self.clamp_pan_now();
            }
        }
    }

    /// 当前显示的像素尺寸（未旋转的图像框，供 UI 设置宽高后再旋转）
    pub fn display_size(&self) -> (f64, f64) {
        (self.img_w * self.scale, self.img_h * self.scale)
    }

    /// 缩放百分比文本（相对 1:1）
    pub fn zoom_percent(&self) -> i32 {
        (self.scale * 100.0).round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut ImageTransform, secs: f64) {
        let mut now = t.now;
        let n = (secs / 0.008) as usize;
        for _ in 0..n {
            now += 0.008;
            t.tick(0.008, now);
        }
    }

    #[test]
    fn fit_scale_matches_viewport() {
        let t = ImageTransform::new(2000, 1000, 1000.0, 1000.0);
        assert!((t.scale() - 0.5).abs() < 1e-9);
        assert_eq!(t.mode(), FitMode::Fit);
    }

    #[test]
    fn rotate_swaps_fit_and_animates_angle() {
        let mut t = ImageTransform::new(2000, 1000, 1000.0, 500.0);
        assert!((t.scale() - 0.5).abs() < 1e-9);
        t.rotate(true);
        run(&mut t, 2.0);
        assert!((t.angle_deg() - 90.0).abs() < 0.01);
        assert!((t.scale() - 0.25).abs() < 1e-3);
        assert!(!t.is_animating());
    }

    #[test]
    fn double_click_toggles_fit_and_actual() {
        let mut t = ImageTransform::new(2000, 1000, 1000.0, 500.0);
        t.toggle_fit_actual((0.0, 0.0));
        run(&mut t, 2.0);
        assert_eq!(t.mode(), FitMode::Actual);
        assert!((t.scale() - 1.0).abs() < 1e-3);
        t.toggle_fit_actual((0.0, 0.0));
        run(&mut t, 2.0);
        assert_eq!(t.mode(), FitMode::Fit);
        assert!((t.scale() - 0.5).abs() < 1e-3);
        assert!(t.pan().0.abs() < 0.5);
    }

    #[test]
    fn wheel_zoom_keeps_cursor_anchor() {
        let mut t = ImageTransform::new(2000, 2000, 1000.0, 1000.0);
        let at = (200.0, 100.0);
        // 图像点（视口中心坐标系）在缩放前的位置
        let before = ((at.0 - t.pan().0) / t.scale(), (at.1 - t.pan().1) / t.scale());
        t.wheel(120.0, at, 0.0);
        run(&mut t, 1.5);
        assert!(t.scale() > 0.5);
        let after = ((at.0 - t.pan().0) / t.scale(), (at.1 - t.pan().1) / t.scale());
        assert!((before.0 - after.0).abs() < 1.0 && (before.1 - after.1).abs() < 1.0);
    }

    #[test]
    fn wheel_overshoot_rebounds_to_limit() {
        let mut t = ImageTransform::new(2000, 2000, 1000.0, 1000.0);
        for i in 0..40 {
            t.wheel(-120.0, (0.0, 0.0), f64::from(i) * 0.01);
        }
        // 越界目标低于合法下限
        assert!(t.scale_target() < t.fit_scale().min(1.0));
        run(&mut t, 2.0);
        assert!((t.scale() - t.fit_scale().min(1.0)).abs() < 1e-3);
    }

    #[test]
    fn drag_inertia_moves_then_stops_within_bounds() {
        let mut t = ImageTransform::new(4000, 4000, 1000.0, 1000.0);
        t.actual((0.0, 0.0));
        run(&mut t, 1.0);
        let start = t.pan();
        t.drag_start(0.0, 0.0, 10.0);
        for i in 1..=5 {
            t.drag_move(-20.0 * f64::from(i), 0.0, 10.0 + 0.008 * f64::from(i));
        }
        t.drag_end(10.04);
        let released = t.pan().0;
        assert!(released < start.0 - 50.0);
        t.now = 10.04;
        run(&mut t, 3.0);
        assert!(t.pan().0 < released, "惯性应继续滑动");
        let lim = t.pan_limit(t.scale()).0;
        assert!(t.pan().0.abs() <= lim + 0.5);
        assert!(!t.is_animating());
    }

    #[test]
    fn drag_past_edge_springs_back() {
        let mut t = ImageTransform::new(3000, 3000, 1000.0, 1000.0);
        t.actual((0.0, 0.0));
        run(&mut t, 1.0);
        let lim = t.pan_limit(1.0).0;
        t.drag_start(0.0, 0.0, 0.0);
        for i in 1..=30 {
            t.drag_move(40.0 * f64::from(i), 0.0, 0.008 * f64::from(i));
        }
        t.drag_end(1.0);
        assert!(t.pan().0 > lim);
        run(&mut t, 3.0);
        assert!((t.pan().0 - lim).abs() < 0.5, "pan={} lim={} anim={}", t.pan().0, lim, t.is_animating());
    }

    #[test]
    fn reduced_motion_snaps_instantly() {
        let mut t = ImageTransform::new(2000, 1000, 1000.0, 500.0);
        t.set_reduced_motion(true);
        t.toggle_fit_actual((0.0, 0.0));
        assert!((t.scale() - 1.0).abs() < 1e-9);
        t.rotate(true);
        assert!((t.angle_deg() - 90.0).abs() < 1e-9);
    }

    #[test]
    fn replace_image_keeps_visual_size() {
        let mut t = ImageTransform::new(200, 100, 1000.0, 500.0);
        t.wheel(120.0, (0.0, 0.0), 0.0);
        run(&mut t, 1.5);
        let visual = t.display_size().0;
        t.replace_image(2000, 1000);
        assert!((t.display_size().0 - visual).abs() < 1.0);
        let mut f = ImageTransform::new(200, 100, 1000.0, 500.0);
        f.replace_image(2000, 1000);
        assert!((f.display_size().0 - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn viewport_resize_refits() {
        let mut t = ImageTransform::new(2000, 1000, 1000.0, 500.0);
        t.set_viewport(500.0, 500.0);
        assert!((t.scale() - 0.25).abs() < 1e-9);
    }
}
