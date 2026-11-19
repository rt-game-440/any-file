//! AnyFile 主程序入口：极速冷启动、零后台常驻、自动跟随系统深浅色与 Skia 优先渲染。

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyfile::domain::CloseDecision;
use anyfile::platform::{PlatformSystemShell, SystemClock, UnavailableVideoBackend};
use anyfile::session::AppSession;
use anyfile::{MainWindow, init_slint_backend};
use slint::{CloseRequestResponse, ComponentHandle, RenderingState};

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
