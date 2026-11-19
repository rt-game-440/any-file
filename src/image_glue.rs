//! 图像视图（工单 #6）的 Slint 绑定层：加载（两阶段）、物理弹簧变换驱动、动图帧、
//! 沉浸式界面淡出、全屏与复制图像。所有状态都在 UI 线程内（thread_local），无需跨线程同步。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use anyfile::MainWindow;
use anyfile::domain::ViewerKind;
use anyfile::i18n::UiStrings;
use anyfile::image_viewer::chrome::ChromeIdle;
use anyfile::image_viewer::decode::{DecodedImage, downsample};
use anyfile::image_viewer::loader::{LoadEvent, spawn_load};
use anyfile::image_viewer::transform::ImageTransform;
use anyfile::ports::SystemShell;
use anyfile::platform::PlatformSystemShell;
use anyfile::session::AppSession;
use anyfile::ImageState;
use image::RgbaImage;
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

/// 超过该边长的图像仅以降采样副本送入渲染器（原图保留用于复制），避免超出 GPU 纹理上限
const MAX_TEXTURE_SIDE: u32 = 8192;
/// 像素化显示（放大到可看见像素格）的比例阈值
const PIXELATED_SCALE: f64 = 3.0;

struct ViewState {
    path: Option<PathBuf>,
    generation: u64,
    rx: Option<Receiver<LoadEvent>>,
    transform: Option<ImageTransform>,
    full: Option<Rc<DecodedImage>>,
    frame_idx: usize,
    view: (f64, f64),
    chrome: ChromeIdle,
    last_cursor: Option<(i32, i32)>,
    start: Instant,
    last_tick: f64,
    reduced_motion: bool,
}

impl ViewState {
    fn new() -> Self {
        Self {
            path: None,
            generation: 0,
            rx: None,
            transform: None,
            full: None,
            frame_idx: 0,
            view: (800.0, 500.0),
            chrome: ChromeIdle::new(0.0),
            last_cursor: None,
            start: Instant::now(),
            last_tick: 0.0,
            reduced_motion: false,
        }
    }
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

thread_local! {
    static STATE: RefCell<ViewState> = RefCell::new(ViewState::new());
    static ANIM: Timer = Timer::default();
    static FRAME: Timer = Timer::default();
    static LOAD_POLL: Timer = Timer::default();
    static CHROME: Timer = Timer::default();
    static WINDOW: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
    static SESSION: RefCell<Option<Arc<AppSession>>> = const { RefCell::new(None) };
}

fn main_window() -> Option<MainWindow> {
    WINDOW.with(|w| w.borrow().as_ref().and_then(|w| w.upgrade()))
}

fn to_slint_image(img: &RgbaImage) -> slint::Image {
    let shown;
    let src = if img.width().max(img.height()) > MAX_TEXTURE_SIDE {
        shown = downsample(img, MAX_TEXTURE_SIDE);
        &shown
    } else {
        img
    };
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(src.as_raw(), src.width(), src.height());
    slint::Image::from_rgba8(buf)
}

/// 把变换结果写入 Slint 全局状态
fn push_geometry(st: &ViewState) {
    let Some(t) = st.transform.as_ref() else { return };
    let st_global = |f: &dyn Fn(&ImageState<'_>)| {
        if let Some(win) = main_window() {
            f(&win.global::<ImageState>());
        }
    };
    let (dw, dh) = t.display_size();
    let (pan_x, pan_y) = t.pan();
    let x = st.view.0 / 2.0 + pan_x - dw / 2.0;
    let y = st.view.1 / 2.0 + pan_y - dh / 2.0;
    let zoom = format!("{}%", t.zoom_percent());
    let pixelated = t.scale() >= PIXELATED_SCALE;
    let angle = t.angle_deg() as f32;
    st_global(&|g| {
        g.set_img_x(x as f32);
        g.set_img_y(y as f32);
        g.set_img_w(dw as f32);
        g.set_img_h(dh as f32);
        g.set_angle(angle);
        g.set_pixelated(pixelated);
        g.set_zoom_text(zoom.as_str().into());
    });
}

fn ensure_anim_running() {
    ANIM.with(|timer| {
        if timer.running() {
            return;
        }
        timer.start(TimerMode::Repeated, Duration::from_millis(8), || {
            let keep = STATE.with(|s| {
                let mut st = s.borrow_mut();
                let now = st.now();
                let dt = (now - st.last_tick).max(0.0);
                st.last_tick = now;
                let Some(t) = st.transform.as_mut() else { return false };
                t.tick(dt, now);
                let animating = t.is_animating();
                push_geometry(&st);
                animating
            });
            if !keep {
                ANIM.with(|t| t.stop());
            }
        });
    });
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.last_tick = st.now();
    });
}

/// 对变换执行操作并刷新界面、按需启动动画
fn with_transform(f: impl FnOnce(&mut ImageTransform, f64)) {
    let animating = STATE.with(|s| {
        let mut st = s.borrow_mut();
        let now = st.now();
        let reduced = st.reduced_motion;
        let Some(t) = st.transform.as_mut() else { return false };
        t.set_reduced_motion(reduced);
        f(t, now);
        let a = t.is_animating();
        push_geometry(&st);
        a
    });
    if animating {
        ensure_anim_running();
    }
}

fn poll_loader() {
    let mut events = Vec::new();
    STATE.with(|s| {
        let st = s.borrow();
        if let Some(rx) = st.rx.as_ref() {
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
    });
    let mut done = false;
    for ev in events {
        let current = STATE.with(|s| s.borrow().generation);
        if ev.generation() != current {
            continue;
        }
        match ev {
            LoadEvent::Preview { image, .. } => apply_preview(&image),
            LoadEvent::Full { image, .. } => {
                apply_full(image);
                done = true;
            }
            LoadEvent::Failed { message, .. } => {
                apply_failure(&message);
                done = true;
            }
        }
    }
    if done {
        LOAD_POLL.with(|t| t.stop());
        STATE.with(|s| s.borrow_mut().rx = None);
    }
}

fn apply_preview(img: &RgbaImage) {
    let Some(win) = main_window() else { return };
    let g = win.global::<ImageState>();
    g.set_thumb(to_slint_image(img));
    g.set_has_thumb(true);
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.transform.is_none() {
            st.transform = Some(ImageTransform::new(img.width(), img.height(), st.view.0, st.view.1));
        }
        push_geometry(&st);
    });
}

fn apply_full(image: DecodedImage) {
    let Some(win) = main_window() else { return };
    let g = win.global::<ImageState>();
    let (w, h) = (image.width(), image.height());
    let info = format!("{} · {} × {}", image.format, w, h);
    let first = to_slint_image(&image.frames[0].rgba);
    let animated = image.is_animated();
    g.set_full(first);
    g.set_full_visible(true);
    g.set_info_text(info.as_str().into());
    g.set_loading(false);
    g.set_error_text("".into());
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        match st.transform.as_mut() {
            Some(t) => t.replace_image(w, h),
            None => st.transform = Some(ImageTransform::new(w, h, st.view.0, st.view.1)),
        }
        st.full = Some(Rc::new(image));
        st.frame_idx = 0;
        push_geometry(&st);
    });
    if animated {
        schedule_next_frame();
    }
}

fn apply_failure(_message: &str) {
    let Some(win) = main_window() else { return };
    let lang = SESSION.with(|s| s.borrow().as_ref().map(|s| s.language()));
    let text = lang
        .map(|l| UiStrings::for_language(l).image_decode_failed)
        .unwrap_or_default();
    let g = win.global::<ImageState>();
    g.set_loading(false);
    g.set_error_text(text.as_str().into());
}

fn schedule_next_frame() {
    let delay = STATE.with(|s| {
        let st = s.borrow();
        st.full.as_ref().map(|f| f.frames[st.frame_idx % f.frames.len()].delay)
    });
    let Some(delay) = delay else { return };
    FRAME.with(|timer| {
        timer.start(TimerMode::SingleShot, delay, || {
            let img = STATE.with(|s| {
                let mut st = s.borrow_mut();
                let full = st.full.clone()?;
                st.frame_idx = (st.frame_idx + 1) % full.frames.len();
                Some(to_slint_image(&full.frames[st.frame_idx].rgba))
            });
            if let (Some(img), Some(win)) = (img, main_window()) {
                win.global::<ImageState>().set_full(img);
                schedule_next_frame();
            }
        });
    });
}

fn reset_view() {
    FRAME.with(|t| t.stop());
    ANIM.with(|t| t.stop());
    LOAD_POLL.with(|t| t.stop());
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.generation += 1;
        st.rx = None;
        st.transform = None;
        st.full = None;
        st.frame_idx = 0;
        st.path = None;
    });
    if let Some(win) = main_window() {
        let g = win.global::<ImageState>();
        g.set_thumb(slint::Image::default());
        g.set_full(slint::Image::default());
        g.set_has_thumb(false);
        g.set_full_visible(false);
        g.set_loading(false);
        g.set_error_text("".into());
        g.set_info_text("".into());
        g.set_zoom_text("".into());
    }
}

fn start_load(path: PathBuf) {
    reset_view();
    let generation = STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.path = Some(path.clone());
        st.generation
    });
    if let Some(win) = main_window() {
        win.global::<ImageState>().set_loading(true);
    }
    let rx = spawn_load(path, generation);
    STATE.with(|s| s.borrow_mut().rx = Some(rx));
    LOAD_POLL.with(|t| t.start(TimerMode::Repeated, Duration::from_millis(16), poll_loader));
}

/// 会话变化后同步：进入 / 切换图像时启动加载，离开时释放内存并退出全屏
pub fn sync_state(window: &MainWindow, session: &AppSession) {
    let is_image = session.active_viewer() == ViewerKind::Image;
    let reduced = session.motion_profile().reduced_motion;
    let current = session.current_file_entry().map(|e| e.path);
    let loaded = STATE.with(|s| s.borrow().path.clone());
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.reduced_motion = reduced;
        let now = st.now();
        st.chrome.set_enabled(is_image, now);
    });
    let g = window.global::<ImageState>();
    g.set_chrome_visible(true);
    if is_image {
        if current != loaded
            && let Some(path) = current
        {
            start_load(path);
        }
    } else if loaded.is_some() {
        reset_view();
        if g.get_fullscreen() {
            g.set_fullscreen(false);
            window.window().set_fullscreen(false);
        }
    }
}

fn show_toast(text: String) {
    let Some(win) = main_window() else { return };
    win.set_toast_text(text.into());
    let weak = win.as_weak();
    Timer::single_shot(Duration::from_millis(2500), move || {
        if let Some(w) = weak.upgrade() {
            w.set_toast_text("".into());
        }
    });
}

fn copy_image() {
    let (frame, lang) = STATE.with(|s| {
        let st = s.borrow();
        let f = st.full.as_ref().map(|f| f.frames[st.frame_idx % f.frames.len()].rgba.clone());
        (f, SESSION.with(|x| x.borrow().as_ref().map(|x| x.language())))
    });
    let Some(lang) = lang else { return };
    let tr = UiStrings::for_language(lang);
    let Some(rgba) = frame else { return };
    // 旋转状态一并复制：按当前四分之一圈数旋转像素
    let turns = STATE.with(|s| s.borrow().transform.as_ref().map_or(0, |t| t.quarter_turns().rem_euclid(4)));
    let rgba = match turns {
        1 => image::imageops::rotate90(&rgba),
        2 => image::imageops::rotate180(&rgba),
        3 => image::imageops::rotate270(&rgba),
        _ => rgba,
    };
    match PlatformSystemShell.copy_image_to_clipboard(rgba.width(), rgba.height(), rgba.as_raw()) {
        Ok(()) => show_toast(tr.toast_image_copied),
        Err(e) => show_toast(format!("{}{}", tr.toast_failed_prefix, anyfile::i18n::localize(&e, lang))),
    }
}

fn toggle_fullscreen() {
    let Some(win) = main_window() else { return };
    let g = win.global::<ImageState>();
    let next = !g.get_fullscreen();
    g.set_fullscreen(next);
    win.window().set_fullscreen(next);
}

fn key_char_is(text: &str, key: slint::platform::Key) -> bool {
    let mut it = text.chars();
    matches!((it.next(), it.next()), (Some(c), None) if c == char::from(key))
}

fn handle_key(text: &str, ctrl: bool, shift: bool) -> bool {
    use slint::platform::Key;
    let active = STATE.with(|s| s.borrow().path.is_some());
    if !active {
        return false;
    }
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let now = st.now();
        st.chrome.activity(now);
    });
    if let Some(win) = main_window() {
        win.global::<ImageState>().set_chrome_visible(true);
    }
    if key_char_is(text, Key::F11) {
        toggle_fullscreen();
        return true;
    }
    if key_char_is(text, Key::Escape) {
        let fs = main_window().is_some_and(|w| w.global::<ImageState>().get_fullscreen());
        if fs {
            toggle_fullscreen();
            return true;
        }
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if ctrl {
        match lower.as_str() {
            "c" => {
                copy_image();
                return true;
            }
            "0" => {
                with_transform(|t, _| t.fit());
                return true;
            }
            "1" => {
                with_transform(|t, _| t.actual((0.0, 0.0)));
                return true;
            }
            _ => {}
        }
        // 部分键盘布局下 Ctrl+C 的 text 为控制字符 \u{3}
        if text == "\u{3}" {
            copy_image();
            return true;
        }
        return false;
    }
    match lower.as_str() {
        "r" => {
            with_transform(|t, _| t.rotate(!shift));
            true
        }
        "+" | "=" => {
            with_transform(|t, now| t.wheel(120.0, (0.0, 0.0), now));
            true
        }
        "-" | "_" => {
            with_transform(|t, now| t.wheel(-120.0, (0.0, 0.0), now));
            true
        }
        "0" => {
            with_transform(|t, _| t.fit());
            true
        }
        "1" => {
            with_transform(|t, _| t.actual((0.0, 0.0)));
            true
        }
        _ => false,
    }
}

/// 绑定回调与定时器（只调用一次）
pub fn wire(window: &MainWindow, session: &Arc<AppSession>) {
    WINDOW.with(|w| *w.borrow_mut() = Some(window.as_weak()));
    SESSION.with(|s| *s.borrow_mut() = Some(Arc::clone(session)));
    let g = window.global::<ImageState>();

    g.on_viewport_changed(|w, h| {
        STATE.with(|s| s.borrow_mut().view = (f64::from(w), f64::from(h)));
        with_transform(|t, _| t.set_viewport(f64::from(w), f64::from(h)));
    });
    g.on_wheel(|delta, x, y| {
        let (cx, cy) = STATE.with(|s| {
            let v = s.borrow().view;
            (v.0 / 2.0, v.1 / 2.0)
        });
        with_transform(|t, now| t.wheel(f64::from(delta), (f64::from(x) - cx, f64::from(y) - cy), now));
    });
    g.on_drag_start(|x, y| with_transform(|t, now| t.drag_start(f64::from(x), f64::from(y), now)));
    g.on_drag_move(|x, y| with_transform(|t, now| t.drag_move(f64::from(x), f64::from(y), now)));
    g.on_drag_end(|| with_transform(|t, now| t.drag_end(now)));
    g.on_double_clicked(|x, y| {
        let (cx, cy) = STATE.with(|s| {
            let v = s.borrow().view;
            (v.0 / 2.0, v.1 / 2.0)
        });
        with_transform(|t, _| t.toggle_fit_actual((f64::from(x) - cx, f64::from(y) - cy)));
    });
    g.on_activity(|| {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            let now = st.now();
            st.chrome.activity(now);
        });
        if let Some(win) = main_window() {
            win.global::<ImageState>().set_chrome_visible(true);
        }
    });
    g.on_key(|text, ctrl, shift| handle_key(text.as_str(), ctrl, shift));
    g.on_rotate(|| with_transform(|t, _| t.rotate(true)));
    g.on_fit(|| with_transform(|t, _| t.fit()));
    g.on_actual(|| with_transform(|t, _| t.actual((0.0, 0.0))));
    g.on_copy(copy_image);
    g.on_toggle_fullscreen(toggle_fullscreen);

    // 沉浸式界面：轮询系统光标位置（标题栏区域的鼠标移动由系统处理，Slint 收不到事件）
    CHROME.with(|timer| {
        timer.start(TimerMode::Repeated, Duration::from_millis(100), || {
            let Some(win) = main_window() else { return };
            let g = win.global::<ImageState>();
            let cursor = PlatformSystemShell.cursor_screen_position();
            let visible = STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.path.as_ref()?;
                let now = st.now();
                if cursor.is_some() && cursor != st.last_cursor {
                    st.chrome.activity(now);
                }
                st.last_cursor = cursor;
                let hold = g.get_dragging() || g.get_toolbar_hover();
                st.chrome.tick(now, hold);
                Some(st.chrome.visible())
            });
            if let Some(v) = visible
                && g.get_chrome_visible() != v
            {
                g.set_chrome_visible(v);
            }
        });
    });
}
