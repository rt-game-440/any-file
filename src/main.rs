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

use anyfile::domain::{CloseDecision, MotionProfile, ViewerKind};
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
    let renderer_label = active_renderer.display_label();
    sync_session_to_window(&window, &session, renderer_label);
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

    // 绑定同目录导航、筛选切换、查看器切换、兜底卡“用其他应用打开”与拖放回调
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        let start_loop = Rc::clone(&start_motion_loop);
        window.on_prev_sibling_clicked(move || {
            if session.navigate_prev().is_ok_and(|opt| opt.is_some()) {
                if let Some(win) = window_weak.upgrade() {
                    sync_session_to_window(&win, &session, renderer_label);
                }
                start_loop();
            }
        });
    }
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        let start_loop = Rc::clone(&start_motion_loop);
        window.on_next_sibling_clicked(move || {
            if session.navigate_next().is_ok_and(|opt| opt.is_some()) {
                if let Some(win) = window_weak.upgrade() {
                    sync_session_to_window(&win, &session, renderer_label);
                }
                start_loop();
            }
        });
    }
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        window.on_toggle_filter_clicked(move || {
            session.toggle_sibling_filter();
            if let Some(win) = window_weak.upgrade() {
                sync_session_to_window(&win, &session, renderer_label);
            }
        });
    }
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        window.on_switch_viewer_clicked(move || {
            let _ = session.cycle_viewer();
            if let Some(win) = window_weak.upgrade() {
                sync_session_to_window(&win, &session, renderer_label);
            }
        });
    }
    {
        let session = Arc::clone(&session);
        window.on_open_with_other_app_clicked(move || {
            let _ = session.open_with_other_app();
        });
    }
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        let start_loop = Rc::clone(&start_motion_loop);
        window.on_file_dropped(move |dropped_path| {
            let path = PathBuf::from(dropped_path.as_str());
            if session.drop_file(&path).is_ok() {
                if let Some(win) = window_weak.upgrade() {
                    sync_session_to_window(&win, &session, renderer_label);
                }
                start_loop();
            }
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

    // 4. 显式显示窗口，并在 Windows 上挂载无边框自定义标题栏壳层（HTMAXBUTTON / Aero Snap / DWM 圆角阴影）
    window.show()?;

    #[cfg(target_os = "windows")]
    install_windows_window_shell(&window, &session);

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

    let viewer = session.active_viewer();
    let viewer_key = match viewer {
        ViewerKind::Welcome => "welcome",
        ViewerKind::Image => "image",
        ViewerKind::Video => "video",
        ViewerKind::Audio => "audio",
        ViewerKind::Text => "text",
        ViewerKind::Unsupported => "unsupported",
    };
    window.set_active_viewer(viewer_key.into());
    window.set_detected_format(session.detected_format().into());

    let seq = session.sibling_sequence();
    window.set_filter_by_category(seq.filter_by_category);
    match seq.current_index {
        Some(idx) if !seq.entries.is_empty() => {
            window.set_can_go_prev(idx > 0);
            window.set_can_go_next(idx + 1 < seq.entries.len());
            window.set_sibling_position_text(format!("{} / {}", idx + 1, seq.entries.len()).into());
        }
        _ => {
            window.set_can_go_prev(false);
            window.set_can_go_next(false);
            window.set_sibling_position_text("".into());
        }
    }

    let viewers = session.available_viewers();
    let show_switch = viewer != ViewerKind::Welcome
        && viewer != ViewerKind::Unsupported
        && (viewers.len() > 1 || viewer != ViewerKind::Text);
    window.set_show_switch_viewer(show_switch);
    let switch_label = if viewer == ViewerKind::Text && viewers.len() > 1 {
        "切换查看器"
    } else {
        "以文本打开"
    };
    window.set_switch_viewer_label(switch_label.into());

    if let Some(fallback) = session.fallback_info() {
        window.set_fallback_size_text(fallback.formatted_size().into());
        window.set_fallback_reason(fallback.reason_message.into());
    } else {
        window.set_fallback_size_text("".into());
        window.set_fallback_reason("".into());
    }

    if let Some(entry) = session.current_file_entry() {
        let file_name = entry
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("未命名文件")
            .to_string();
        window.set_app_title(format!("{file_name} — AnyFile").into());
        window.set_current_file_name(file_name.into());
        window.set_current_file_path(entry.path.display().to_string().into());
        window.set_status_subtitle(format!("已打开：{}", entry.path.display()).into());
    } else {
        window.set_app_title("AnyFile".into());
        window.set_current_file_name("".into());
        window.set_current_file_path("".into());
        window.set_status_subtitle("拖放任意文件到此处，或点击下方按钮打开".into());
    }
}

#[cfg(target_os = "windows")]
fn install_windows_window_shell(window: &MainWindow, session: &AppSession) {
    use anyfile::window_shell::win32::{ShellCallbacks, WindowsShellHandle};
    use anyfile::window_shell::{TitleBarMetrics, WindowSizingConstraints};

    // winit 0.30 在事件循环启动（resumed）后才真正创建原生 Win32 HWND，
    // 因此通过 invoke_from_event_loop 在首帧事件派发时挂载子类化与工作区自适应尺寸。
    let win_weak = window.as_weak();
    let dark_mode = session.resolved_theme().is_dark();

    let _ = slint::invoke_from_event_loop(move || {
        let Some(win) = win_weak.upgrade() else {
            return;
        };

        let win_weak_max = win.as_weak();
        let win_weak_hover = win.as_weak();
        let callbacks = ShellCallbacks {
            on_maximized_changed: Box::new(move |maximized| {
                let win_weak = win_weak_max.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = win_weak.upgrade() {
                        win.set_is_maximized(maximized);
                    }
                });
            }),
            on_max_button_hover_changed: Box::new(move |hovered| {
                let win_weak = win_weak_hover.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = win_weak.upgrade() {
                        win.set_max_button_hovered(hovered);
                    }
                });
            }),
        };

        let Ok(shell_handle) = WindowsShellHandle::install(
            win.window(),
            TitleBarMetrics::default(),
            WindowSizingConstraints::default(),
            dark_mode,
            callbacks,
        ) else {
            return;
        };

        win.set_is_maximized(shell_handle.is_maximized());
        shell_handle.apply_adaptive_size(None);

        win.on_minimize_clicked(move || {
            shell_handle.minimize();
        });
        win.on_maximize_clicked(move || {
            shell_handle.toggle_maximize();
        });
        win.on_close_clicked(move || {
            shell_handle.close();
        });
    });
}
