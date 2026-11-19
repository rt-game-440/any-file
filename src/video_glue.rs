//! 视频视图（工单 #9）的 Slint 绑定层：把 `VideoViewer` 与主窗口的 `video-*` 属性/回调连接，
//! 并以定时器驱动帧渲染（软件渲染路径，ADR-0007 回退方案 C）与控制条淡出。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyfile::audio_viewer::{AudioCommand, KeyModifiers};
use anyfile::domain::ViewerKind;
use anyfile::i18n::localize;
use anyfile::session::AppSession;
use anyfile::video::LazyMpvVideoBackend;
use anyfile::video_viewer::{VideoCommand, VideoKey, VideoViewer};
use anyfile::MainWindow;
use slint::{ComponentHandle, Timer, TimerMode};

/// 渲染分辨率上限（软件渲染路径的性能护栏，见 ADR-0007）
const MAX_RENDER_W: f32 = 1280.0;
const MAX_RENDER_H: f32 = 720.0;

/// 进程内共享的 libmpv 后端（惰性加载 `mpv-2.dll`；会话探测、视频视图、音频回退共用同一份）。
pub fn backend() -> Arc<LazyMpvVideoBackend> {
    static BACKEND: OnceLock<Arc<LazyMpvVideoBackend>> = OnceLock::new();
    Arc::clone(BACKEND.get_or_init(|| Arc::new(LazyMpvVideoBackend::for_exe_dir())))
}

thread_local! {
    static VIDEO: RefCell<Option<VideoViewer>> = const { RefCell::new(None) };
    static HOVER: Cell<bool> = const { Cell::new(false) };
    /// 画面区域逻辑尺寸
    static AREA: Cell<(f32, f32)> = const { Cell::new((640.0, 360.0)) };
    static LAST_RENDER: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

fn with_video<R>(f: impl FnOnce(&mut VideoViewer) -> R) -> R {
    VIDEO.with(|cell| {
        let mut slot = cell.borrow_mut();
        f(slot.get_or_insert_with(|| VideoViewer::new(backend())))
    })
}

/// 立即停止并释放视频播放（窗口关闭）
pub fn stop() {
    VIDEO.with(|cell| {
        if let Some(v) = cell.borrow_mut().as_mut() {
            v.stop();
        }
    });
}

fn video_key_from_text(text: &str) -> Option<VideoKey> {
    use slint::platform::Key;
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c == ' ' {
        Some(VideoKey::Space)
    } else if c == char::from(Key::LeftArrow) {
        Some(VideoKey::Left)
    } else if c == char::from(Key::RightArrow) {
        Some(VideoKey::Right)
    } else if c == char::from(Key::PageUp) {
        Some(VideoKey::PageUp)
    } else if c == char::from(Key::PageDown) {
        Some(VideoKey::PageDown)
    } else if c == char::from(Key::Escape) {
        Some(VideoKey::Escape)
    } else if c.eq_ignore_ascii_case(&'m') {
        Some(VideoKey::M)
    } else if c.eq_ignore_ascii_case(&'f') {
        Some(VideoKey::F)
    } else if c.eq_ignore_ascii_case(&'s') {
        Some(VideoKey::S)
    } else {
        None
    }
}

/// 渲染目标尺寸（物理像素，保持区域宽高比并限制在 720p 内）
fn render_size(window: &MainWindow) -> (u32, u32) {
    let (w, h) = AREA.with(Cell::get);
    let scale = window.window().scale_factor();
    let (mut pw, mut ph) = ((w * scale).max(2.0), (h * scale).max(2.0));
    let k = (MAX_RENDER_W / pw).min(MAX_RENDER_H / ph).min(1.0);
    pw *= k;
    ph *= k;
    ((pw as u32).max(2) & !1, (ph as u32).max(2) & !1)
}

/// 视频视图状态 -> 窗口属性；并按会话载入/停止播放，同步全屏状态。
pub fn sync_state(window: &MainWindow, session: &AppSession) {
    if session.active_viewer() != ViewerKind::Video {
        let had = VIDEO.with(|c| c.borrow().is_some());
        if had {
            with_video(|v| {
                v.sync_with_session(session);
            });
        }
        if window.window().is_fullscreen() {
            window.window().set_fullscreen(false);
        }
        window.set_video_fullscreen(false);
        window.set_video_has_frame(false);
        return;
    }
    let (state, loaded_changed) = with_video(|v| {
        let changed = v.sync_with_session(session);
        (v.view_state(HOVER.with(Cell::get)), changed)
    });
    if loaded_changed {
        window.set_video_has_frame(false);
        LAST_RENDER.with(|c| c.set((0, 0)));
    }
    window.set_video_position_secs(state.position_secs as f32);
    window.set_video_duration_secs(state.duration_secs as f32);
    window.set_video_position_text(state.position_text.into());
    window.set_video_duration_text(state.duration_text.into());
    window.set_video_playing(state.playing);
    window.set_video_speed_text(state.speed_text.into());
    window.set_video_volume(state.volume);
    window.set_video_muted(state.muted);
    window.set_video_subtitles_visible(state.subtitles_visible);
    window.set_video_has_subtitles(state.has_subtitles);
    window.set_video_controls_visible(state.controls_visible);
    window.set_video_error(localize(&state.error.unwrap_or_default(), session.language()).into());
    if window.get_video_fullscreen() != state.fullscreen {
        window.set_video_fullscreen(state.fullscreen);
    }
    if window.window().is_fullscreen() != state.fullscreen {
        window.window().set_fullscreen(state.fullscreen);
    }
}

/// 绑定视频视图回调并启动帧/状态定时器；返回的定时器须由调用方持有至程序结束。
pub fn wire(window: &MainWindow, session: &Arc<AppSession>, resync: Rc<dyn Fn()>) -> Timer {
    {
        let session = Arc::clone(session);
        let resync = Rc::clone(&resync);
        window.on_video_key(move |text, ctrl| {
            let Some(key) = video_key_from_text(text.as_str()) else {
                return false;
            };
            let handled = with_video(|v| v.handle_key(key, KeyModifiers { ctrl }, &session));
            resync();
            handled
        });
    }
    macro_rules! video_cb {
        ($setter:ident, |$($arg:ident),*| $cmd:expr) => {{
            let session = Arc::clone(session);
            let resync = Rc::clone(&resync);
            window.$setter(move |$($arg),*| {
                let cmd: VideoCommand = $cmd;
                with_video(|v| v.handle_command(cmd, &session));
                resync();
            });
        }};
    }
    video_cb!(on_video_toggle_play, | | VideoCommand::Playback(AudioCommand::TogglePlay));
    video_cb!(on_video_seek_to, |t| VideoCommand::Playback(AudioCommand::SeekTo(f64::from(t))));
    video_cb!(on_video_cycle_speed, | | VideoCommand::Playback(AudioCommand::CycleSpeed));
    video_cb!(on_video_set_volume, |x| VideoCommand::Playback(AudioCommand::SetVolume(x)));
    video_cb!(on_video_toggle_mute, | | VideoCommand::Playback(AudioCommand::ToggleMute));
    video_cb!(on_video_prev_file, | | VideoCommand::Playback(AudioCommand::PrevTrack));
    video_cb!(on_video_next_file, | | VideoCommand::Playback(AudioCommand::NextTrack));
    video_cb!(on_video_toggle_subtitles, | | VideoCommand::ToggleSubtitles);
    video_cb!(on_video_toggle_fullscreen, | | VideoCommand::ToggleFullscreen);
    window.on_video_activity(|| with_video(VideoViewer::on_activity));
    window.on_video_hover_controls(|h| HOVER.with(|c| c.set(h)));
    {
        let weak = window.as_weak();
        window.on_video_area_resized(move |w, h| {
            AREA.with(|c| c.set((w, h)));
            // 尺寸变化后需要重绘（暂停时没有新帧也要按新尺寸重渲染）
            if weak.upgrade().is_some() {
                with_video(VideoViewer::request_redraw);
            }
        });
    }

    let timer = Timer::default();
    let weak = window.as_weak();
    let session = Arc::clone(session);
    let tick = Cell::new(0u32);
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        if session.active_viewer() != ViewerKind::Video {
            return;
        }
        let Some(win) = weak.upgrade() else { return };
        let (w, h) = render_size(&win);
        if let Some(frame) = with_video(|v| v.poll_frame(w, h)) {
            let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                &frame.rgba,
                frame.width,
                frame.height,
            );
            win.set_video_frame(slint::Image::from_rgba8(buf));
            win.set_video_has_frame(true);
            LAST_RENDER.with(|c| c.set((w, h)));
        }
        // 播放状态约 8 次/秒刷新，避免每帧查询 mpv 属性
        let n = tick.get().wrapping_add(1);
        tick.set(n);
        if n.is_multiple_of(7) {
            sync_state(&win, &session);
        }
    });
    timer
}
