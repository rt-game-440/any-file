//! AnyFile 主程序入口：极速冷启动、零后台常驻、自动跟随系统深浅色与 Skia 优先渲染。

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyfile::domain::{CloseDecision, MotionProfile};
use anyfile::motion::{MotionChannel, MotionSnapshot};
use anyfile::platform::{PlatformSystemShell, SystemClock, UnavailableVideoBackend};
use anyfile::session::AppSession;
use anyfile::{MainWindow, init_slint_backend};
use slint::{CloseRequestResponse, ComponentHandle, RenderingState, TimerMode};

/// 命令行启动参数
#[derive(Debug, Default)]
struct LaunchOptions {
    /// 启动时打开的目标文件路径（无参数则进入欢迎空态）
    target_path: Option<PathBuf>,
    /// 冷启动基准模式：完成首帧渲染后打印耗时并立即退出
    benchmark_first_frame: bool,
    /// 可选：将基准结果写入指定文件路径（适用于 Windows GUI 子系统无控制台场景）
    benchmark_out: Option<PathBuf>,
}

impl LaunchOptions {
    fn from_args() -> Self {
        let mut opts = Self::default();
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--benchmark-first-frame" {
                opts.benchmark_first_frame = true;
            } else if arg == "--benchmark-out" {
                if let Some(out_path) = args.next() {
                    opts.benchmark_out = Some(PathBuf::from(out_path));
                }
            } else if opts.target_path.is_none() {
                opts.target_path = Some(PathBuf::from(arg));
            }
        }
        opts
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 尽早记录进程入口时间，用于冷启动首帧渲染耗时统计
    let process_start = Instant::now();
    let opts = LaunchOptions::from_args();

    // 1. 按 ADR-0003 初始化渲染后端（Skia GPU 优先，失败自动回退 Software）
    let active_renderer = init_slint_backend()?;

    // 2. 构建无头应用核心会话 AppSession
    let shell = Arc::new(PlatformSystemShell);
    let clock = Arc::new(SystemClock);
    let video = Arc::new(UnavailableVideoBackend);
    let session = Arc::new(AppSession::new(shell, clock, video));

    if let Some(ref path) = opts.target_path {
        let _ = session.open_file(path);
    }

    // 3. 创建并绑定 Slint 主窗口
    let window = MainWindow::new()?;
    sync_session_to_window(&window, &session, active_renderer.display_label());
    sync_motion_to_window(&window, session.motion_snapshot());

    // 3.1 按需物理弹簧动画帧驱动（仅在存在活跃动画时以 120Hz 节拍推进，收敛后自动停表零空转）
    let motion_timer = Rc::new(slint::Timer::default());
    let start_motion_loop = {
        let motion_timer = Rc::clone(&motion_timer);
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        Rc::new(move || {
            if !session.has_active_animations() {
                if let Some(win) = window_weak.upgrade() {
                    sync_motion_to_window(&win, session.motion_snapshot());
                }
                motion_timer.stop();
                return;
            }
            if motion_timer.running() {
                return;
            }
            let session_tick = Arc::clone(&session);
            let window_tick = window_weak.clone();
            let timer_stop = Rc::clone(&motion_timer);
            motion_timer.start(TimerMode::Repeated, Duration::from_millis(8), move || {
                let snapshot = session_tick.tick_motion();
                if let Some(win) = window_tick.upgrade() {
                    sync_motion_to_window(&win, snapshot);
                }
                if !snapshot.has_active_animations {
                    timer_stop.stop();
                }
            });
        })
    };

    // 绑定“打开文件”按钮物理弹性缩放反馈（支持中途打断与减弱动效瞬时降级）
    {
        let session_btn = Arc::clone(&session);
        let start_loop = Rc::clone(&start_motion_loop);
        window.on_open_button_state_changed(move |hovered, pressed| {
            let target_scale = if pressed {
                0.96
            } else if hovered {
                1.025
            } else {
                1.0
            };
            session_btn.animate_channel(
                MotionChannel::ButtonScale,
                target_scale,
                MotionProfile::bouncy(),
            );
            start_loop();
        });
    }

    // 启动初期若有活跃入场动画，按需拉起帧驱动
    start_motion_loop();

    // 3.2 低频监听系统主题与 Windows“减弱动态效果”变更（非冷启动基准模式下启用）
    let _system_pref_timer = if !opts.benchmark_first_frame {
        let timer = slint::Timer::default();
        let session_pref = Arc::clone(&session);
        let window_pref = window.as_weak();
        let timer_motion = Rc::clone(&motion_timer);
        timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            let prev_dark = session_pref.resolved_theme().is_dark();
            let prev_reduced = session_pref.motion_profile().reduced_motion;
            session_pref.refresh_system_preferences();
            let now_dark = session_pref.resolved_theme().is_dark();
            let now_reduced = session_pref.motion_profile().reduced_motion;
            if (prev_dark != now_dark || prev_reduced != now_reduced)
                && let Some(win) = window_pref.upgrade()
            {
                win.set_dark_mode(now_dark);
                let snap = session_pref.motion_snapshot();
                sync_motion_to_window(&win, snap);
                if !snap.has_active_animations {
                    timer_motion.stop();
                }
            }
        });
        Some(timer)
    } else {
        None
    };

    // 窗口关闭即彻底退出进程（ADR-0002）
    let session_for_close = Arc::clone(&session);
    window.window().on_close_requested(move || {
        match session_for_close.request_close() {
            CloseDecision::AllowExit => CloseRequestResponse::HideWindow,
            CloseDecision::ConfirmUnsavedChanges => CloseRequestResponse::KeepWindowShown,
        }
    });

    // 4. 显式显示窗口，并在冷启动基准模式下捕获首帧渲染完成时机
    window.show()?;

    let _bench_fallback_timer = if opts.benchmark_first_frame {
        let recorded = Arc::new(AtomicBool::new(false));
        let renderer_label = active_renderer.display_label();

        let record_and_quit = {
            let recorded = Arc::clone(&recorded);
            let window_weak = window.as_weak();
            let benchmark_out = opts.benchmark_out.clone();
            move |source: &'static str| {
                if recorded.swap(true, Ordering::SeqCst) {
                    return;
                }
                let elapsed_ms = process_start.elapsed().as_secs_f64() * 1000.0;
                let line = format!(
                    "ANYFILE_BENCH_FIRST_FRAME_MS={elapsed_ms:.2} RENDERER=\"{renderer_label}\" SOURCE=\"{source}\""
                );
                if let Some(ref out_path) = benchmark_out {
                    let _ = std::fs::write(out_path, format!("{line}\n"));
                }
                println!("{line}");
                let window_weak = window_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = window_weak.upgrade() {
                        let _ = win.hide();
                    }
                    let _ = slint::quit_event_loop();
                });
            }
        };

        let notifier_cb = record_and_quit.clone();
        let _ = window
            .window()
            .set_rendering_notifier(move |state, _graphics_api| {
                if matches!(state, RenderingState::AfterRendering) {
                    notifier_cb("after_rendering");
                }
            });

        window.window().request_redraw();

        // 若当前图形后端（如 Skia D3D12 或软件回退）不触发 AfterRendering 回调，
        // 在首帧事件循环派发完成后由单次定时器收口并记录真实启动到首帧事件就绪耗时。
        let fallback_timer = slint::Timer::default();
        fallback_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(16), move || {
            record_and_quit("event_loop_ready");
        });
        Some(fallback_timer)
    } else {
        None
    };

    slint::run_event_loop()?;
    Ok(())
}

fn sync_motion_to_window(window: &MainWindow, snapshot: MotionSnapshot) {
    window.set_welcome_progress(snapshot.welcome_entrance);
    window.set_view_transition(snapshot.view_transition);
    window.set_button_scale(snapshot.button_scale);
    window.set_reduced_motion(snapshot.reduced_motion);
}

fn sync_session_to_window(window: &MainWindow, session: &AppSession, renderer_label: &str) {
    window.set_dark_mode(session.resolved_theme().is_dark());
    window.set_reduced_motion(session.motion_profile().reduced_motion);
    window.set_renderer_label(renderer_label.into());

    if let Some(entry) = session.current_file_entry() {
        let title = entry
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|name| format!("{name} — AnyFile"))
            .unwrap_or_else(|| "AnyFile".to_string());
        window.set_app_title(title.into());
        window.set_status_subtitle(format!("已打开：{}", entry.path.display()).into());
    } else {
        window.set_app_title("AnyFile".into());
        window.set_status_subtitle("拖放任意文件到此处，或点击下方按钮打开".into());
    }
}
