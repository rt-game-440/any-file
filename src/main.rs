//! AnyFile 主程序入口：极速冷启动、零后台常驻、自动跟随系统深浅色与 Skia 优先渲染。

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyfile::domain::{CloseDecision, ViewerKind};
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
    let renderer_label = active_renderer.display_label();
    sync_session_to_window(&window, &session, renderer_label);

    // 绑定同目录导航、筛选切换、查看器切换、兜底卡“用其他应用打开”与拖放回调
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        window.on_prev_sibling_clicked(move || {
            let _ = session.navigate_prev();
            if let Some(win) = window_weak.upgrade() {
                sync_session_to_window(&win, &session, renderer_label);
            }
        });
    }
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        window.on_next_sibling_clicked(move || {
            let _ = session.navigate_next();
            if let Some(win) = window_weak.upgrade() {
                sync_session_to_window(&win, &session, renderer_label);
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
        window.on_file_dropped(move |dropped_path| {
            let path = PathBuf::from(dropped_path.as_str());
            if session.drop_file(&path).is_ok()
                && let Some(win) = window_weak.upgrade()
            {
                sync_session_to_window(&win, &session, renderer_label);
            }
        });
    }

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
