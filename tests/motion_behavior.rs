use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyfile::domain::{MotionProfile, ResolvedTheme};
use anyfile::motion::{MotionChannel, MotionEngine};
use anyfile::ports::{Clock, SystemShell, VideoBackend, VideoProbeInfo};
use anyfile::session::AppSession;

struct FakeSystemShell {
    theme: Mutex<ResolvedTheme>,
    reduced_motion: Mutex<bool>,
}

impl FakeSystemShell {
    fn new(theme: ResolvedTheme, reduced_motion: bool) -> Self {
        Self {
            theme: Mutex::new(theme),
            reduced_motion: Mutex::new(reduced_motion),
        }
    }

    fn set_reduced_motion(&self, reduced_motion: bool) {
        *self.reduced_motion.lock().unwrap() = reduced_motion;
    }
}

impl SystemShell for FakeSystemShell {
    fn system_theme(&self) -> ResolvedTheme {
        *self.theme.lock().unwrap()
    }

    fn prefers_reduced_motion(&self) -> bool {
        *self.reduced_motion.lock().unwrap()
    }
}

struct FakeClock {
    now: Mutex<Instant>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
        }
    }

    fn advance(&self, delta: Duration) {
        *self.now.lock().unwrap() += delta;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
}

struct FakeVideoBackend;

impl VideoBackend for FakeVideoBackend {
    fn is_available(&self) -> bool {
        false
    }

    fn probe(&self, _path: &std::path::Path) -> Option<VideoProbeInfo> {
        None
    }
}

#[test]
fn spring_converges_smoothly_over_discrete_time_steps_without_divergence() {
    let mut engine = MotionEngine::new(false);
    engine.animate_from_to(
        MotionChannel::WelcomeEntrance,
        0.0,
        1.0,
        MotionProfile::smooth(),
    );

    assert!(engine.has_active_animations());
    assert_eq!(engine.value(MotionChannel::WelcomeEntrance), 0.0);

    let mut samples = Vec::new();
    for _ in 0..120 {
        engine.advance(Duration::from_millis(16));
        let v = engine.value(MotionChannel::WelcomeEntrance);
        assert!(v.is_finite(), "弹簧位置不能出现 NaN 或无穷大");
        assert!(
            (-0.15..=1.15).contains(&v),
            "平滑配置不应出现失控过冲，实际值: {v}"
        );
        samples.push(v);
        if !engine.has_active_animations() {
            break;
        }
    }

    assert!(
        samples.len() > 3,
        "正常动效模式下应经历多帧过渡而非瞬时跳变"
    );
    assert!(
        !engine.has_active_animations(),
        "弹簧应在合理时间内收敛并停止活跃状态"
    );
    assert_eq!(engine.value(MotionChannel::WelcomeEntrance), 1.0);
    assert_eq!(engine.velocity(MotionChannel::WelcomeEntrance), 0.0);
}

#[test]
fn large_dt_spike_is_clamped_and_stays_stable() {
    let mut engine = MotionEngine::new(false);
    engine.animate_from_to(
        MotionChannel::ViewTransition,
        0.0,
        100.0,
        MotionProfile::bouncy(),
    );

    // 模拟主线程卡顿或系统休眠 5 秒后的超大 dt
    engine.advance(Duration::from_secs(5));

    let v = engine.value(MotionChannel::ViewTransition);
    let vel = engine.velocity(MotionChannel::ViewTransition);
    assert!(v.is_finite() && vel.is_finite(), "超大 dt 下必须数值稳定");
    assert!(
        v > 0.0 && v < 100.0,
        "单帧大 dt 应被钳制，避免瞬间越过整个过渡过程，实际值: {v}"
    );
    assert!(engine.has_active_animations());
}

#[test]
fn retargeting_mid_flight_inherits_velocity_and_reverses_smoothly() {
    let mut engine = MotionEngine::new(false);
    engine.animate_from_to(
        MotionChannel::ZoomScale,
        0.0,
        100.0,
        MotionProfile::snappy(),
    );

    // 推进 4 帧（64ms），弹簧处于高速正向运动中
    for _ in 0..4 {
        engine.advance(Duration::from_millis(16));
    }
    let pos_at_interrupt = engine.value(MotionChannel::ZoomScale);
    let vel_at_interrupt = engine.velocity(MotionChannel::ZoomScale);
    assert!(
        pos_at_interrupt > 5.0 && pos_at_interrupt < 95.0,
        "打断时应处于过渡中段，实际位置: {pos_at_interrupt}"
    );
    assert!(
        vel_at_interrupt > 50.0,
        "打断时应具备显著正向速度，实际速度: {vel_at_interrupt}"
    );

    // 中途反向切换目标回 0.0
    engine.animate_to(MotionChannel::ZoomScale, 0.0, MotionProfile::snappy());

    // 切换瞬间位置与速度必须完全连续（速度继承，绝不瞬移或清零速度）
    assert_eq!(engine.value(MotionChannel::ZoomScale), pos_at_interrupt);
    assert_eq!(engine.velocity(MotionChannel::ZoomScale), vel_at_interrupt);

    // 紧接着推进极小步（2ms），因正向惯性存在，位置会短暂延续正向后减速折返
    engine.advance(Duration::from_millis(2));
    let pos_after_tiny_step = engine.value(MotionChannel::ZoomScale);
    assert!(
        pos_after_tiny_step > pos_at_interrupt,
        "速度继承应使极短步长内延续原方向惯性: before={pos_at_interrupt}, after={pos_after_tiny_step}"
    );

    // 继续步进直到收敛到新目标 0.0
    let mut min_value = pos_after_tiny_step;
    for _ in 0..180 {
        engine.advance(Duration::from_millis(16));
        let v = engine.value(MotionChannel::ZoomScale);
        min_value = min_value.min(v);
        if !engine.has_active_animations() {
            break;
        }
    }

    assert!(!engine.has_active_animations(), "折返后应平滑收敛");
    assert_eq!(engine.value(MotionChannel::ZoomScale), 0.0);
    assert_eq!(engine.velocity(MotionChannel::ZoomScale), 0.0);
    assert!(
        min_value > -15.0,
        "折返收敛不应产生失控反向过冲，最小值: {min_value}"
    );
}

#[test]
fn instant_profile_and_reduced_motion_toggle_snap_immediately() {
    let mut engine = MotionEngine::new(false);

    // Instant 配置即使在非减弱动效下也瞬时完成
    engine.animate_from_to(
        MotionChannel::ChromeOpacity,
        0.0,
        1.0,
        MotionProfile::instant(),
    );
    assert_eq!(engine.value(MotionChannel::ChromeOpacity), 1.0);
    assert!(!engine.has_active_animations());

    // 启动正常弹簧过渡并推进 2 帧
    engine.animate_from_to(
        MotionChannel::WelcomeEntrance,
        0.0,
        1.0,
        MotionProfile::smooth(),
    );
    engine.advance(Duration::from_millis(16));
    assert!(engine.has_active_animations());
    assert!(engine.value(MotionChannel::WelcomeEntrance) < 1.0);

    // 运行时开启系统“减弱动态效果”：活跃通道必须立即锁定到目标值并停止空转
    engine.set_reduced_motion(true);
    assert_eq!(engine.value(MotionChannel::WelcomeEntrance), 1.0);
    assert_eq!(engine.velocity(MotionChannel::WelcomeEntrance), 0.0);
    assert!(!engine.has_active_animations());

    // 减弱动效期间新触发的动画也瞬时到位
    engine.animate_from_to(
        MotionChannel::ViewTransition,
        0.0,
        1.0,
        MotionProfile::bouncy(),
    );
    assert_eq!(engine.value(MotionChannel::ViewTransition), 1.0);
    assert!(!engine.has_active_animations());

    // 关闭减弱动态效果后，后续动画恢复物理弹簧过渡
    engine.set_reduced_motion(false);
    engine.animate_from_to(
        MotionChannel::ViewTransition,
        0.0,
        1.0,
        MotionProfile::snappy(),
    );
    assert!(engine.has_active_animations());
    assert_eq!(engine.value(MotionChannel::ViewTransition), 0.0);
}

#[test]
fn session_drives_welcome_entrance_via_deterministic_clock() {
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Dark, false));
    let clock = Arc::new(FakeClock::new());
    let video = Arc::new(FakeVideoBackend);

    let session = AppSession::new(shell, clock.clone(), video);

    let initial = session.motion_snapshot();
    assert!(!initial.reduced_motion);
    assert!(initial.has_active_animations);
    assert_eq!(initial.welcome_entrance, 0.0);
    assert_eq!(initial.button_scale, 1.0);

    // 不推进时钟时调用 tick_motion，位置应保持不变
    let zero_dt = session.tick_motion();
    assert_eq!(zero_dt.welcome_entrance, 0.0);

    // 使用确定性时钟按 16ms 步进直到收敛
    let mut frames = 0;
    let mut last_snapshot = zero_dt;
    while session.has_active_animations() && frames < 120 {
        clock.advance(Duration::from_millis(16));
        last_snapshot = session.tick_motion();
        frames += 1;
    }

    assert!(frames > 3, "正常模式欢迎页入场应经历多帧平滑过渡");
    assert!(!last_snapshot.has_active_animations);
    assert_eq!(last_snapshot.welcome_entrance, 1.0);
}

#[test]
fn session_reduced_motion_at_launch_and_runtime_refresh_degrades_instantly() {
    // 场景 A：启动时系统已开启减弱动态效果
    let shell_reduced = Arc::new(FakeSystemShell::new(ResolvedTheme::Light, true));
    let clock_a = Arc::new(FakeClock::new());
    let session_a = AppSession::new(shell_reduced, clock_a, Arc::new(FakeVideoBackend));

    let snap_a = session_a.motion_snapshot();
    assert!(snap_a.reduced_motion);
    assert!(!snap_a.has_active_animations);
    assert_eq!(snap_a.welcome_entrance, 1.0);
    assert_eq!(snap_a.view_transition, 1.0);

    // 场景 B：启动时未开启，过渡进行到一半时系统设置开启减弱动态效果
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Light, false));
    let clock = Arc::new(FakeClock::new());
    let session = AppSession::new(shell.clone(), clock.clone(), Arc::new(FakeVideoBackend));

    clock.advance(Duration::from_millis(16));
    let mid = session.tick_motion();
    assert!(mid.has_active_animations);
    assert!(mid.welcome_entrance > 0.0 && mid.welcome_entrance < 1.0);

    // 系统开启减弱动态效果并刷新偏好
    shell.set_reduced_motion(true);
    session.refresh_system_preferences();

    let after_toggle = session.motion_snapshot();
    assert!(after_toggle.reduced_motion);
    assert!(!after_toggle.has_active_animations);
    assert_eq!(after_toggle.welcome_entrance, 1.0);

    // 系统关闭减弱动态效果后，后续新动画恢复物理弹簧驱动
    shell.set_reduced_motion(false);
    session.refresh_system_preferences();
    session.animate_channel(MotionChannel::ButtonScale, 0.96, MotionProfile::bouncy());
    assert!(session.has_active_animations());
}

#[test]
fn session_open_file_triggers_view_transition_and_interruptible_button_spring() {
    let shell = Arc::new(FakeSystemShell::new(ResolvedTheme::Light, false));
    let clock = Arc::new(FakeClock::new());
    let session = AppSession::new(shell, clock.clone(), Arc::new(FakeVideoBackend));

    // 先推进欢迎页入场收敛
    for _ in 0..120 {
        clock.advance(Duration::from_millis(16));
        if !session.tick_motion().has_active_animations {
            break;
        }
    }
    assert!(!session.has_active_animations());

    // 打开真实临时文件，应触发 ViewTransition 0.0 -> 1.0
    let temp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let sample_path: PathBuf = temp_dir.path().join("note.txt");
    std::fs::write(&sample_path, "spring motion test").expect("写入临时文件失败");

    session.open_file(&sample_path).expect("打开文件应成功");
    let after_open = session.motion_snapshot();
    assert_eq!(after_open.view_transition, 0.0);
    assert!(after_open.has_active_animations);

    // 同步触发按钮按压缩放并在中途打断回弹
    session.animate_channel(MotionChannel::ButtonScale, 0.94, MotionProfile::bouncy());
    clock.advance(Duration::from_millis(32));
    let pressed_mid = session.tick_motion();
    assert!(pressed_mid.button_scale < 1.0);

    // 中途松开按钮，平滑回弹到 1.0
    session.animate_channel(MotionChannel::ButtonScale, 1.0, MotionProfile::bouncy());
    for _ in 0..150 {
        clock.advance(Duration::from_millis(16));
        if !session.tick_motion().has_active_animations {
            break;
        }
    }

    let final_snap = session.motion_snapshot();
    assert!(!final_snap.has_active_animations);
    assert_eq!(final_snap.view_transition, 1.0);
    assert_eq!(final_snap.button_scale, 1.0);
}

#[test]
fn critically_damped_and_overdamped_profiles_converge_without_overshoot() {
    let critical = MotionProfile {
        stiffness: 400.0,
        damping: 40.0, // 2 * sqrt(400 * 1) = 40.0
        mass: 1.0,
        reduced_motion: false,
    };
    let overdamped = MotionProfile {
        stiffness: 300.0,
        damping: 60.0, // > 2 * sqrt(300 * 1)
        mass: 1.0,
        reduced_motion: false,
    };

    for (name, profile) in [("critical", critical), ("overdamped", overdamped)] {
        let mut engine = MotionEngine::new(false);
        engine.animate_from_to(MotionChannel::Custom(name), 0.0, 1.0, profile);

        for _ in 0..240 {
            engine.advance(Duration::from_millis(16));
            let v = engine.value(MotionChannel::Custom(name));
            assert!(
                (0.0..=1.0001).contains(&v),
                "{name} 阻尼区间不应产生过冲，实际值: {v}"
            );
            if !engine.has_active_animations() {
                break;
            }
        }
        assert!(!engine.has_active_animations(), "{name} 应在合理帧数内收敛");
        assert_eq!(engine.value(MotionChannel::Custom(name)), 1.0);
    }
}

#[cfg(windows)]
#[test]
fn windows_system_shell_queries_reduced_motion_without_panicking() {
    use anyfile::platform::PlatformSystemShell;

    let shell = PlatformSystemShell;
    // 验证真实 Win32 SPI_GETCLIENTAREAANIMATION 查询可安全调用并返回布尔值
    let _reduced: bool = shell.prefers_reduced_motion();
}
