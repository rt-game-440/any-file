//! AnyFile 主程序入口：极速冷启动、零后台常驻、自动跟随系统深浅色与 Skia 优先渲染。

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod editor_glue;
mod image_glue;
mod video_glue;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyfile::config::{ConfigStore, StorageMode, current_data_location};
use anyfile::diagnostics;
use anyfile::domain::ThemePreference;
use anyfile::i18n::{LanguagePreference, UiStrings, large_file_notice, localize, text_meta_line};
use anyfile::ports::SystemShell;
use anyfile::ui_text::apply_ui_strings;
use anyfile::update_check::{CurlReleaseSource, UpdateStatus, check_for_update};
use anyfile::audio_viewer::{AudioCommand, AudioKey, AudioPlayer, AudioViewer, KeyModifiers};
use anyfile::domain::{CloseDecision, MotionProfile, ViewerKind};
use anyfile::motion::{MotionChannel, MotionSnapshot};
use anyfile::platform::{PlatformSystemShell, SystemClock};
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
    let video = video_glue::backend();
    let data_loc = current_data_location();
    diagnostics::init(&data_loc.dir);
    diagnostics::info("AnyFile 启动");
    let mut session = AppSession::new(shell, clock, video);
    if let Ok(store) = ConfigStore::open(&data_loc.dir) {
        session = session.with_config_store(store);
    }
    let session = Arc::new(session);
    let storage_mode = data_loc.mode;

    if let Some(ref path) = opts.target_path {
        let _ = session.open_file(path);
    }

    // 3. 创建并绑定 Slint 主窗口
    let window = MainWindow::new()?;
    let renderer_label = active_renderer.display_label();
    image_glue::wire(&window, &session);
    sync_session_to_window(&window, &session, renderer_label);
    sync_motion_to_window(&window, session.motion_snapshot());
    window.set_version_text(env!("CARGO_PKG_VERSION").into());
    sync_settings_to_window(&window, &session, storage_mode);

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
    wire_text_view(&window, &session);
    let _editor_timers = {
        let weak = window.as_weak();
        let session_for_sync = Arc::clone(&session);
        let resync: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(win) = weak.upgrade() {
                sync_session_to_window(&win, &session_for_sync, renderer_label);
            }
        });
        editor_glue::wire(&window, &session, resync)
    };
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

    wire_settings_and_ops(&window, &session, renderer_label, storage_mode);

    // 视频视图（工单 #9）：回调 + 帧/状态定时器
    let _video_timer = {
        let weak = window.as_weak();
        let session_for_sync = Arc::clone(&session);
        let resync: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(win) = weak.upgrade() {
                sync_session_to_window(&win, &session_for_sync, renderer_label);
            }
        });
        video_glue::wire(&window, &session, resync)
    };

    // 音频视图回调（命令 -> AudioViewer；切歌后同步整个窗口状态）
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        window.on_audio_key(move |text, ctrl| {
            let Some(key) = audio_key_from_text(text.as_str()) else {
                return false;
            };
            let handled = with_audio(|a| a.handle_key(key, KeyModifiers { ctrl }, &session));
            if let Some(win) = window_weak.upgrade() {
                sync_session_to_window(&win, &session, renderer_label);
            }
            handled
        });
    }
    macro_rules! audio_cb {
        ($setter:ident, |$($arg:ident),*| $cmd:expr) => {{
            let session = Arc::clone(&session);
            let window_weak = window.as_weak();
            window.$setter(move |$($arg),*| {
                let cmd: AudioCommand = $cmd;
                with_audio(|a| a.handle_command(cmd, &session));
                if let Some(win) = window_weak.upgrade() {
                    sync_session_to_window(&win, &session, renderer_label);
                }
            });
        }};
    }
    audio_cb!(on_audio_toggle_play, | | AudioCommand::TogglePlay);
    audio_cb!(on_audio_seek_to, |v| AudioCommand::SeekTo(v as f64));
    audio_cb!(on_audio_cycle_speed, | | AudioCommand::CycleSpeed);
    audio_cb!(on_audio_set_volume, |v| AudioCommand::SetVolume(v));
    audio_cb!(on_audio_toggle_mute, | | AudioCommand::ToggleMute);
    audio_cb!(on_audio_prev_track, | | AudioCommand::PrevTrack);
    audio_cb!(on_audio_next_track, | | AudioCommand::NextTrack);
    audio_cb!(on_audio_toggle_auto_next, | | AudioCommand::ToggleAutoNext);

    // 音频播放进度/播完轮询：仅在音频视图激活期间以 100ms 运行
    let audio_timer = slint::Timer::default();
    {
        let session = Arc::clone(&session);
        let window_weak = window.as_weak();
        audio_timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            if session.active_viewer() != ViewerKind::Audio {
                return;
            }
            let advanced = with_audio(|a| a.poll(&session));
            if let Some(win) = window_weak.upgrade() {
                if advanced {
                    sync_session_to_window(&win, &session, renderer_label);
                } else {
                    sync_audio_to_window(&win, &session);
                }
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
            CloseDecision::AllowExit => {
                with_audio(|a| a.stop());
                video_glue::stop();
                CloseRequestResponse::HideWindow
            }
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

thread_local! {
    /// 音频视图控制器（仅 UI 线程；首次进入音频视图时才打开输出设备，避免拖慢冷启动）
    static AUDIO: RefCell<Option<AudioViewer>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut AudioViewer) -> R) -> R {
    AUDIO.with(|cell| {
        let mut slot = cell.borrow_mut();
        let viewer = slot.get_or_insert_with(|| {
            let mut v = AudioViewer::new(AudioPlayer::with_default_device());
            v.set_fallback(Box::new(anyfile::video_viewer::MpvAudioFallback::new(
                video_glue::backend(),
            )));
            v
        });
        f(viewer)
    })
}

fn audio_key_from_text(text: &str) -> Option<AudioKey> {
    use slint::platform::Key;
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c == ' ' {
        Some(AudioKey::Space)
    } else if c == char::from(Key::LeftArrow) {
        Some(AudioKey::Left)
    } else if c == char::from(Key::RightArrow) {
        Some(AudioKey::Right)
    } else if c == char::from(Key::PageUp) {
        Some(AudioKey::PageUp)
    } else if c == char::from(Key::PageDown) {
        Some(AudioKey::PageDown)
    } else if c.eq_ignore_ascii_case(&'m') {
        Some(AudioKey::M)
    } else {
        None
    }
}

/// 同步音频控制器与会话（切走即停播），并刷新音频视图属性
fn sync_audio_to_window(window: &MainWindow, session: &AppSession) {
    let is_audio = session.active_viewer() == ViewerKind::Audio;
    if !is_audio {
        // 未进入过音频视图则无需创建输出设备
        AUDIO.with(|cell| {
            if let Some(a) = cell.borrow_mut().as_mut() {
                a.sync_with_session(session);
            }
        });
        return;
    }
    let state = with_audio(|a| {
        a.sync_with_session(session);
        a.view_state()
    });
    let meta = &state.metadata;
    window.set_audio_title(meta.title.clone().into());
    window.set_audio_artist(meta.artist.clone().unwrap_or_default().into());
    window.set_audio_album(meta.album.clone().unwrap_or_default().into());
    let mut fmt = localize(&session.detected_format(), session.language());
    if let Some(rate) = meta.sample_rate {
        fmt.push_str(&format!(" · {:.1} kHz", rate as f32 / 1000.0));
    }
    if let Some(kbps) = meta.bitrate_kbps {
        fmt.push_str(&format!(" · {kbps} kbps"));
    }
    window.set_audio_format(fmt.into());
    match &meta.cover {
        Some(cover) => {
            let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                &cover.rgba,
                cover.width,
                cover.height,
            );
            window.set_audio_cover(slint::Image::from_rgba8(buf));
            window.set_audio_has_cover(true);
        }
        None => window.set_audio_has_cover(false),
    }
    let snap = state.snapshot;
    window.set_audio_position_secs(snap.position_secs as f32);
    window.set_audio_duration_secs(snap.duration_secs as f32);
    window.set_audio_position_text(state.position_text.into());
    window.set_audio_duration_text(state.duration_text.into());
    window.set_audio_playing(snap.playing);
    window.set_audio_speed_text(state.speed_text.into());
    window.set_audio_volume(snap.volume);
    window.set_audio_muted(snap.muted);
    window.set_audio_auto_next(state.auto_next);
    window.set_audio_error(localize(&state.error.unwrap_or_default(), session.language()).into());
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
    window.set_renderer_label(localize(renderer_label, session.language()).into());

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
    window.set_detected_format(localize(&session.detected_format(), session.language()).into());

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
    let tr = UiStrings::for_language(session.language());
    apply_ui_strings(window, &tr);
    let switch_label = if viewer == ViewerKind::Text && viewers.len() > 1 {
        tr.switch_viewer.clone()
    } else {
        tr.open_as_text.clone()
    };
    window.set_switch_viewer_label(switch_label.into());
    let recents = if session.recent_enabled() {
        session.recent_files()
    } else {
        Vec::new()
    };
    let names: Vec<slint::SharedString> = recents
        .iter()
        .take(5)
        .map(|r| {
            std::path::Path::new(&r.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| r.path.clone())
                .into()
        })
        .collect();
    let paths: Vec<slint::SharedString> = recents
        .iter()
        .take(5)
        .map(|r| r.path.as_str().into())
        .collect();
    window.set_recent_names(slint::ModelRc::new(slint::VecModel::from(names)));
    window.set_recent_paths(slint::ModelRc::new(slint::VecModel::from(paths)));
    let hint = if !session.recent_enabled() {
        tr.recent_off.clone()
    } else if recents.is_empty() {
        tr.recent_none.clone()
    } else {
        tr.recent_title.clone()
    };
    window.set_recent_hint(hint.into());

    sync_text_view(window, session);

    if let Some(fallback) = session.fallback_info() {
        window.set_fallback_size_text(fallback.formatted_size().into());
        window.set_fallback_reason(localize(&fallback.reason_message, session.language()).into());
    } else {
        window.set_fallback_size_text("".into());
        window.set_fallback_reason("".into());
    }

    if let Some(entry) = session.current_file_entry() {
        let file_name = entry
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(tr.unnamed_file.as_str())
            .to_string();
        window.set_app_title(format!("{file_name} — AnyFile").into());
        window.set_current_file_name(file_name.into());
        window.set_current_file_path(entry.path.display().to_string().into());
        window.set_status_subtitle(format!("{}{}", tr.opened_prefix, entry.path.display()).into());
    } else {
        window.set_app_title("AnyFile".into());
        window.set_current_file_name("".into());
        window.set_current_file_path("".into());
        window.set_status_subtitle(tr.welcome_subtitle.as_str().into());
    }
    sync_audio_to_window(window, session);
    video_glue::sync_state(window, session);
    image_glue::sync_state(window, session);
    editor_glue::sync_state(window, session);
}

/// 同步文本查看态（行模型、字号、状态栏、查找状态）到窗口。
fn sync_text_view(window: &MainWindow, session: &AppSession) {
    use anyfile::text_viewer::ui_model::TextLinesModel;
    use slint::ModelRc;

    window.set_text_font_size(session.text_font_size());
    let Some(doc) = session.text_viewer() else {
        window.set_text_rows(ModelRc::default());
        window.set_text_meta("".into());
        window.set_text_notice("".into());
        return;
    };
    let find = session.text_find_state();
    let lang = session.language();
    window.set_text_find_status(localize(&find.status_text(), lang).into());
    window.set_text_gutter_digits(doc.lines.len().max(1).to_string().len() as i32);
    window.set_text_rows(ModelRc::new(TextLinesModel::new(doc.clone(), &find)));
    let m = &doc.meta;
    window.set_text_meta(
        text_meta_line(
            lang,
            m.encoding.label(),
            m.line_ending.label(),
            doc.lines.len(),
            m.syntax.label(),
            session.text_font_size(),
        )
        .into(),
    );
    let notice = if m.is_large_file_readonly {
        large_file_notice(lang, m.file_size as f64 / 1_048_576.0)
    } else {
        String::new()
    };
    window.set_text_notice(notice.into());
}

/// 把视图滚动到当前匹配行（居中）。
fn scroll_to_current_match(window: &MainWindow, session: &AppSession) {
    if let Some(m) = session.text_find_state().current_match() {
        window.set_text_scroll_line(m.line_index as i32);
        window.set_text_scroll_tick(window.get_text_scroll_tick().wrapping_add(1));
    }
}

fn wire_text_view(window: &MainWindow, session: &Arc<AppSession>) {
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_text_find_changed(move |q, case| {
            session.text_find(q.as_str(), case);
            if let Some(win) = weak.upgrade() {
                sync_text_view(&win, &session);
                scroll_to_current_match(&win, &session);
            }
        });
    }
    for next in [true, false] {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        let cb = move || {
            if next {
                session.text_find_next();
            } else {
                session.text_find_prev();
            }
            if let Some(win) = weak.upgrade() {
                sync_text_view(&win, &session);
                scroll_to_current_match(&win, &session);
            }
        };
        if next {
            window.on_text_find_next(cb);
        } else {
            window.on_text_find_prev(cb);
        }
    }
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_text_find_closed(move || {
            session.text_find_clear();
            if let Some(win) = weak.upgrade() {
                sync_text_view(&win, &session);
            }
        });
    }
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_text_zoom_wheel(move |d| {
            session.text_zoom_by_wheel(d, true);
            if let Some(win) = weak.upgrade() {
                win.set_text_font_size(session.text_font_size());
                sync_text_view(&win, &session);
            }
        });
    }
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_text_zoom_reset(move || {
            session.text_zoom_reset();
            if let Some(win) = weak.upgrade() {
                sync_text_view(&win, &session);
            }
        });
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

fn sync_settings_to_window(window: &MainWindow, session: &AppSession, mode: StorageMode) {
    let tr = UiStrings::for_language(session.language());
    window.set_theme_index(match session.theme_preference() {
        ThemePreference::System => 0,
        ThemePreference::Light => 1,
        ThemePreference::Dark => 2,
    });
    window.set_language_index(session.language_preference().to_index());
    window.set_auto_save(session.auto_save());
    window.set_recent_enabled(session.recent_enabled());
    let storage = match mode {
        StorageMode::Portable => tr.storage_portable,
        StorageMode::Installed => tr.storage_installed,
    };
    window.set_storage_text(storage.as_str().into());
}

fn show_toast(window: &MainWindow, text: String) {
    window.set_toast_text(text.into());
    let weak = window.as_weak();
    slint::Timer::single_shot(Duration::from_millis(2500), move || {
        if let Some(w) = weak.upgrade() {
            w.set_toast_text("".into());
        }
    });
}

thread_local! {
    /// 最近一次检查到的新版本发布页（仅 UI 线程）
    static RELEASE_URL: RefCell<String> =
        RefCell::new(anyfile::update_check::RELEASES_PAGE.to_string());
}

/// 绑定文件轻量操作与设置 / 关于页回调
fn wire_settings_and_ops(
    window: &MainWindow,
    session: &Arc<AppSession>,
    renderer_label: &'static str,
    mode: StorageMode,
) {
    let refresh = {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        Rc::new(move || {
            if let Some(w) = weak.upgrade() {
                sync_session_to_window(&w, &session, renderer_label);
                sync_settings_to_window(&w, &session, mode);
            }
        })
    };

    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        let refresh = Rc::clone(&refresh);
        window.on_delete_clicked(move || {
            let result = session.delete_current_file();
            refresh();
            if let Some(w) = weak.upgrade() {
                let tr = UiStrings::for_language(session.language());
                match result {
                    Ok(_) => show_toast(&w, tr.toast_deleted),
                    Err(e) => show_toast(&w, format!("{}{}", tr.toast_failed_prefix, localize(&e, session.language()))),
                }
            }
        });
    }
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_copy_path_clicked(move || {
            let result = session.copy_current_path();
            if let Some(w) = weak.upgrade() {
                let tr = UiStrings::for_language(session.language());
                match result {
                    Ok(()) => show_toast(&w, tr.toast_copied),
                    Err(e) => show_toast(&w, format!("{}{}", tr.toast_failed_prefix, localize(&e, session.language()))),
                }
            }
        });
    }
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_reveal_clicked(move || {
            if let Err(e) = session.reveal_current_in_explorer()
                && let Some(w) = weak.upgrade()
            {
                let tr = UiStrings::for_language(session.language());
                show_toast(&w, format!("{}{}", tr.toast_failed_prefix, localize(&e, session.language())));
            }
        });
    }
    {
        let weak = window.as_weak();
        window.on_settings_clicked(move || {
            if let Some(w) = weak.upgrade() {
                w.set_settings_open(true);
            }
        });
        let weak = window.as_weak();
        window.on_settings_closed(move || {
            if let Some(w) = weak.upgrade() {
                w.set_settings_open(false);
            }
        });
    }
    {
        let session = Arc::clone(session);
        let refresh = Rc::clone(&refresh);
        window.on_theme_selected(move |i| {
            session.set_theme_preference(match i {
                1 => ThemePreference::Light,
                2 => ThemePreference::Dark,
                _ => ThemePreference::System,
            });
            refresh();
        });
    }
    {
        let session = Arc::clone(session);
        let refresh = Rc::clone(&refresh);
        window.on_language_selected(move |i| {
            session.set_language_preference(LanguagePreference::from_index(i));
            refresh();
        });
    }
    {
        let session = Arc::clone(session);
        let refresh = Rc::clone(&refresh);
        window.on_auto_save_toggled(move || {
            session.set_auto_save(!session.auto_save());
            refresh();
        });
    }
    {
        let session = Arc::clone(session);
        let refresh = Rc::clone(&refresh);
        window.on_recent_toggled(move || {
            session.set_recent_enabled(!session.recent_enabled());
            refresh();
        });
    }
    {
        let session = Arc::clone(session);
        let refresh = Rc::clone(&refresh);
        window.on_clear_recent_clicked(move || {
            session.clear_recent();
            refresh();
        });
    }
    {
        let session = Arc::clone(session);
        let weak = window.as_weak();
        window.on_check_update_clicked(move || {
            let Some(w) = weak.upgrade() else { return };
            if w.get_update_checking() {
                return;
            }
            w.set_update_checking(true);
            w.set_update_available(false);
            w.set_update_text("".into());
            let weak = weak.clone();
            let lang = session.language();
            // 仅用户点击时联网一次；线程用完即退，无常驻
            std::thread::spawn(move || {
                let status = check_for_update(&CurlReleaseSource, env!("CARGO_PKG_VERSION"));
                diagnostics::info("手动检查更新完成");
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(w) = weak.upgrade() else { return };
                    let tr = UiStrings::for_language(lang);
                    w.set_update_checking(false);
                    match status {
                        UpdateStatus::UpToDate => {
                            w.set_update_text(tr.update_up_to_date.as_str().into())
                        }
                        UpdateStatus::Available { version, url } => {
                            w.set_update_available(true);
                            w.set_update_text(
                                format!("{}{version}", tr.update_available_prefix).into(),
                            );
                            RELEASE_URL.with(|u| *u.borrow_mut() = url);
                        }
                        UpdateStatus::Failed(e) => w
                            .set_update_text(format!("{}{}", tr.update_failed_prefix, localize(&e, lang)).into()),
                    }
                });
            });
        });
    }
    {
        let shell = PlatformSystemShell;
        window.on_open_release_page_clicked(move || {
            let url = RELEASE_URL.with(|u| u.borrow().clone());
            let _ = shell.open_url(&url);
        });
        let shell = PlatformSystemShell;
        window.on_open_default_apps_clicked(move || {
            let _ = shell.open_default_apps_settings();
        });
    }
}
