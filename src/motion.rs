//! 统一物理弹簧动效驱动（Motion Engine，遵循 ADR-0003）。
//!
//! 采用精确解析阻尼谐振子解算器（支持欠阻尼、临界阻尼与过阻尼），具备以下特性：
//! - 基于真实时间步长 `dt` 连续步进，并对卡顿/休眠产生的超大 `dt` 进行安全钳制；
//! - 目标中途切换时无条件保留当前位置与速度（可中断、速度继承）；
//! - 自动尊重 Windows“减弱动态效果”设置，开启时立即降级为瞬时切换并停止空转。

use std::collections::HashMap;
use std::time::Duration;

use crate::domain::MotionProfile;

/// 单帧允许推进的最大时间步长（50ms），避免主线程偶发卡顿或休眠唤醒后跳帧。
pub const MAX_DT_SECS: f64 = 0.05;

/// 位置收敛阈值：当位移误差与速度均落入阈值内时锁定到目标值并标记静止。
const POSITION_EPSILON: f64 = 1e-3;
const VELOCITY_EPSILON: f64 = 1e-3;

/// 应用级标准动效通道标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotionChannel {
    /// 空态欢迎页入场进度（`0.0 -> 1.0`）
    WelcomeEntrance,
    /// 视图切换过渡进度（`0.0 -> 1.0`）
    ViewTransition,
    /// 主操作按钮弹性缩放（默认 `1.0`，悬停/按压微缩放）
    ButtonScale,
    /// 悬浮标题栏/控制条渐隐透明度（`0.0..=1.0`）
    ChromeOpacity,
    /// 图像/画布缩放回弹通道
    ZoomScale,
    /// 自定义命名通道（供后续查看器扩展）
    Custom(&'static str),
}

/// 单个物理弹簧通道的运行时状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringState {
    pub value: f64,
    pub target: f64,
    pub velocity: f64,
    pub profile: MotionProfile,
    pub settled: bool,
}

impl SpringState {
    /// 创建已静止于指定初值的弹簧状态
    pub fn settled_at(value: f32, profile: MotionProfile) -> Self {
        let v = f64::from(value);
        Self {
            value: v,
            target: v,
            velocity: 0.0,
            profile,
            settled: true,
        }
    }

    /// 立即锁定到目标值并清零速度
    pub fn snap_to_target(&mut self) {
        self.value = self.target;
        self.velocity = 0.0;
        self.settled = true;
    }

    /// 更新目标与动效配置；保持当前 `value` 与 `velocity`（速度继承）。
    pub fn retarget(&mut self, target: f32, profile: MotionProfile, reduced_motion: bool) {
        self.target = f64::from(target);
        self.profile = profile.with_reduced_motion(reduced_motion || profile.reduced_motion);

        if self.profile.is_instant() {
            self.snap_to_target();
            return;
        }

        if (self.target - self.value).abs() <= POSITION_EPSILON
            && self.velocity.abs() <= VELOCITY_EPSILON
        {
            self.snap_to_target();
        } else {
            self.settled = false;
        }
    }

    /// 施加瞬时速度冲量（用于惯性手势释放或弹性反馈）
    pub fn add_impulse(&mut self, delta_velocity: f32, reduced_motion: bool) {
        if reduced_motion || self.profile.is_instant() {
            return;
        }
        self.velocity += f64::from(delta_velocity);
        if self.velocity.abs() > VELOCITY_EPSILON {
            self.settled = false;
        }
    }

    /// 按精确解析阻尼谐振子模型推进 `dt_secs` 秒；返回推进后是否仍处于活跃状态。
    pub fn step(&mut self, dt_secs: f64, reduced_motion: bool) -> bool {
        if self.settled {
            return false;
        }
        if reduced_motion || self.profile.is_instant() {
            self.snap_to_target();
            return false;
        }
        if dt_secs <= 0.0 || !dt_secs.is_finite() {
            return !self.settled;
        }

        let dt = dt_secs.min(MAX_DT_SECS);
        let k = f64::from(self.profile.stiffness);
        let c = f64::from(self.profile.damping);
        let m = f64::from(self.profile.mass);

        let x0 = self.value - self.target;
        let v0 = self.velocity;

        let omega0 = (k / m).sqrt();
        let zeta = c / (2.0 * (k * m).sqrt());

        let (x, v) = if (zeta - 1.0).abs() <= 1e-6 {
            // 临界阻尼（critically damped）
            let decay = (-omega0 * dt).exp();
            let a = x0;
            let b = v0 + omega0 * x0;
            let pos = decay * (a + b * dt);
            let vel = decay * (b - omega0 * (a + b * dt));
            (pos, vel)
        } else if zeta < 1.0 {
            // 欠阻尼（underdamped）
            let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega0 * dt).exp();
            let a = x0;
            let b = (v0 + zeta * omega0 * x0) / omega_d;
            let (sin_t, cos_t) = (omega_d * dt).sin_cos();
            let pos = decay * (a * cos_t + b * sin_t);
            let vel = decay
                * ((b * omega_d - a * zeta * omega0) * cos_t
                    - (a * omega_d + b * zeta * omega0) * sin_t);
            (pos, vel)
        } else {
            // 过阻尼（overdamped）
            let gamma = omega0 * (zeta * zeta - 1.0).sqrt();
            let r1 = -zeta * omega0 + gamma;
            let r2 = -zeta * omega0 - gamma;
            let c1 = (v0 - r2 * x0) / (2.0 * gamma);
            let c2 = x0 - c1;
            let e1 = (r1 * dt).exp();
            let e2 = (r2 * dt).exp();
            let pos = c1 * e1 + c2 * e2;
            let vel = c1 * r1 * e1 + c2 * r2 * e2;
            (pos, vel)
        };

        if !x.is_finite() || !v.is_finite() {
            self.snap_to_target();
            return false;
        }

        self.value = self.target + x;
        self.velocity = v;

        if x.abs() <= POSITION_EPSILON && v.abs() <= VELOCITY_EPSILON {
            self.snap_to_target();
            return false;
        }

        true
    }
}

/// 供 UI 绑定层消费的当前帧动效状态快照
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSnapshot {
    /// 欢迎页入场进度（`0.0..=1.0`）
    pub welcome_entrance: f32,
    /// 视图切换过渡进度（`0.0..=1.0`）
    pub view_transition: f32,
    /// 主按钮弹性缩放倍率
    pub button_scale: f32,
    /// 悬浮标题栏/控制条透明度（`0.0..=1.0`）
    pub chrome_opacity: f32,
    /// 是否仍有未收敛的活跃动画（为 `false` 时 UI 可停止重绘定时器）
    pub has_active_animations: bool,
    /// 当前是否处于“减弱动态效果”瞬时降级模式
    pub reduced_motion: bool,
}

/// 多通道统一物理弹簧动效驱动引擎
#[derive(Debug, Clone)]
pub struct MotionEngine {
    reduced_motion: bool,
    channels: HashMap<MotionChannel, SpringState>,
}

impl MotionEngine {
    /// 创建多通道动效驱动；`reduced_motion` 为 `true` 时所有通道瞬时切换。
    pub fn new(reduced_motion: bool) -> Self {
        Self {
            reduced_motion,
            channels: HashMap::new(),
        }
    }

    /// 当前是否处于减弱动态效果模式
    pub fn is_reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// 实时同步系统“减弱动态效果”状态；开启时立即将所有活跃通道收敛到目标值。
    pub fn set_reduced_motion(&mut self, reduced_motion: bool) {
        self.reduced_motion = reduced_motion;
        if reduced_motion {
            for state in self.channels.values_mut() {
                state.profile = state.profile.with_reduced_motion(true);
                state.snap_to_target();
            }
        }
    }

    /// 立即将指定通道设为给定值（静止状态，不触发动画）
    pub fn set_immediate(&mut self, channel: MotionChannel, value: f32) {
        let profile = MotionProfile::for_system_setting(self.reduced_motion);
        self.channels
            .insert(channel, SpringState::settled_at(value, profile));
    }

    /// 将通道从当前位置与当前速度平滑过渡到 `target`（支持中途打断与速度继承）。
    pub fn animate_to(&mut self, channel: MotionChannel, target: f32, profile: MotionProfile) {
        let entry = self
            .channels
            .entry(channel)
            .or_insert_with(|| SpringState::settled_at(0.0, profile));
        entry.retarget(target, profile, self.reduced_motion);
    }

    /// 从指定起始值 `from` 过渡到 `to`；减弱动效模式下直接落到 `to`。
    pub fn animate_from_to(
        &mut self,
        channel: MotionChannel,
        from: f32,
        to: f32,
        profile: MotionProfile,
    ) {
        let mut state = SpringState::settled_at(from, profile);
        state.retarget(to, profile, self.reduced_motion);
        self.channels.insert(channel, state);
    }

    /// 对指定通道施加瞬时速度冲量
    pub fn impulse(&mut self, channel: MotionChannel, delta_velocity: f32) {
        if let Some(state) = self.channels.get_mut(&channel) {
            state.add_impulse(delta_velocity, self.reduced_motion);
        }
    }

    /// 按真实时间步长推进所有活跃通道；返回推进后是否仍有活跃动画。
    pub fn advance(&mut self, dt: Duration) -> bool {
        let dt_secs = dt.as_secs_f64();
        let mut any_active = false;
        for state in self.channels.values_mut() {
            if state.step(dt_secs, self.reduced_motion) {
                any_active = true;
            }
        }
        any_active
    }

    /// 查询通道当前值（未注册通道返回 `0.0`）
    pub fn value(&self, channel: MotionChannel) -> f32 {
        self.channels
            .get(&channel)
            .map(|s| s.value as f32)
            .unwrap_or(0.0)
    }

    /// 查询通道当前值，若未注册则返回默认值
    pub fn value_or(&self, channel: MotionChannel, default: f32) -> f32 {
        self.channels
            .get(&channel)
            .map(|s| s.value as f32)
            .unwrap_or(default)
    }

    /// 查询通道当前速度（单位/秒）
    pub fn velocity(&self, channel: MotionChannel) -> f32 {
        self.channels
            .get(&channel)
            .map(|s| s.velocity as f32)
            .unwrap_or(0.0)
    }

    /// 查询通道当前目标值
    pub fn target(&self, channel: MotionChannel) -> f32 {
        self.channels
            .get(&channel)
            .map(|s| s.target as f32)
            .unwrap_or(0.0)
    }

    /// 查询指定通道是否已静止收敛
    pub fn is_settled(&self, channel: MotionChannel) -> bool {
        self.channels.get(&channel).is_none_or(|s| s.settled)
    }

    /// 是否存在任意仍在运动中的弹簧通道
    pub fn has_active_animations(&self) -> bool {
        self.channels.values().any(|s| !s.settled)
    }

    /// 生成供 UI 绑定的标准通道快照
    pub fn snapshot(&self) -> MotionSnapshot {
        MotionSnapshot {
            welcome_entrance: self.value_or(MotionChannel::WelcomeEntrance, 1.0),
            view_transition: self.value_or(MotionChannel::ViewTransition, 1.0),
            button_scale: self.value_or(MotionChannel::ButtonScale, 1.0),
            chrome_opacity: self.value_or(MotionChannel::ChromeOpacity, 1.0),
            has_active_animations: self.has_active_animations(),
            reduced_motion: self.reduced_motion,
        }
    }
}
