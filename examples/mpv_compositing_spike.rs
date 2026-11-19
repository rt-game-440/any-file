//! 工单 #8 技术验证 Spike：在 Windows 10/11 上实测对比 Slint Skia 与 libmpv 的三种视频画面合成方案：
//! - `--mode default-skia`：探测默认 `skia` 后端实际暴露的 `GraphicsAPI`
//! - `--mode gl`（方案 A）：Slint `skia-opengl` + `mpv_render_context(opengl)` 离屏 FBO 纹理 + `BorrowedOpenGLTextureBuilder`
//! - `--mode child-hwnd`（方案 B）：Win32 HWND + mpv `wid` 嵌入 (`vo=gpu` + `hwdec=auto-safe`)
//! - `--mode sw`（方案 C / 软件回退）：`mpv_render_context(sw)` -> `SharedPixelBuffer<Rgba8Pixel>` -> `slint::Image`

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyfile::video::ffi::*;
use anyfile::video::{MpvInstance, MpvLibrary};
use slint::{
    BorrowedOpenGLTextureBuilder, BorrowedOpenGLTextureOrigin, ComponentHandle, GraphicsAPI, Image,
    RenderingState, Rgba8Pixel, SharedPixelBuffer,
};

slint::slint! {
    #[edition = "1.19"]
    export component SpikeVideoWindow inherits Window {
        in property <string> mode_title: "AnyFile Video Spike";
        in property <image> video_frame;
        in property <float> overlay_opacity: 0.85;
        width: 960px;
        height: 540px;
        title: mode_title;
        background: #0d0f12;

        // 底层视频画面（方案 A 为 OpenGL 纹理零拷贝导入，方案 C 为软件 RGBA 帧）
        Image {
            width: 100%;
            height: 100%;
            source: root.video_frame;
            image-fit: contain;
        }

        // 顶层半透明渐隐标题栏（验证与视频画面同帧 alpha 混合，无 Airspace 遮挡）
        Rectangle {
            x: 16px;
            y: 12px;
            width: parent.width - 32px;
            height: 40px;
            border-radius: 10px;
            background: #141820cc;
            opacity: root.overlay_opacity;

            Text {
                x: 16px;
                text: root.mode_title;
                color: #f3f6fa;
                font-size: 14px;
                vertical-alignment: center;
                height: 100%;
            }
        }

        // 底部半透明渐隐控制条
        Rectangle {
            x: 48px;
            y: parent.height - 64px;
            width: parent.width - 96px;
            height: 44px;
            border-radius: 12px;
            background: #141820cc;
            opacity: root.overlay_opacity;

            Text {
                text: "Space 播放/暂停  |  ←/→ 步进 5s  |  半透明渐隐控件叠加验证";
                color: #dce4f0;
                font-size: 13px;
                horizontal-alignment: center;
                vertical-alignment: center;
                width: 100%;
                height: 100%;
            }
        }
    }
}

#[cfg(windows)]
fn process_cpu_time_ms() -> f64 {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let (mut c, mut e, mut k, mut u) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        return 0.0;
    }
    let kt = ((k.high as u64) << 32) | (k.low as u64);
    let ut = ((u.high as u64) << 32) | (u.low as u64);
    // FILETIME 单位为 100ns，除以 10_000 得到毫秒
    (kt + ut) as f64 / 10_000.0
}

#[cfg(not(windows))]
fn process_cpu_time_ms() -> f64 {
    0.0
}

fn default_dll_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("third_party")
        .join("libmpv")
        .join("mpv-2.dll")
}

fn parse_arg(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = parse_arg("--mode").unwrap_or_else(|| "gl".to_string());
    let video_path = parse_arg("--video")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("sample.mp4"));
    let dll_path = parse_arg("--dll")
        .map(PathBuf::from)
        .unwrap_or_else(default_dll_path);

    match mode.as_str() {
        "default-skia" => run_probe_default_skia(),
        "gl" => run_scheme_a_opengl(&dll_path, &video_path),
        "child-hwnd" => run_scheme_b_child_hwnd(&dll_path, &video_path),
        "sw" => run_scheme_c_software(&dll_path, &video_path),
        other => Err(format!("未知 --mode: {other}").into()),
    }
}

fn run_probe_default_skia() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .renderer_name("skia".to_string())
        .select()?;
    let win = SpikeVideoWindow::new()?;
    win.set_mode_title("Default Skia API Probe".into());

    let api_reported = Rc::new(RefCell::new(None::<String>));
    let api_clone = Rc::clone(&api_reported);
    let weak = win.as_weak();
    let _ = win
        .window()
        .set_rendering_notifier(move |state, graphics_api| {
            if matches!(state, RenderingState::RenderingSetup) {
                let desc = match graphics_api {
                    GraphicsAPI::NativeOpenGL { .. } => "NativeOpenGL",
                    GraphicsAPI::WebGL { .. } => "WebGL",
                    _ => "NonOpenGL(WGPU/D3D12/Vulkan)",
                };
                *api_clone.borrow_mut() = Some(desc.to_string());
                let w = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() {
                        let _ = win.hide();
                    }
                    let _ = slint::quit_event_loop();
                });
            }
        });

    win.show()?;
    let timer = slint::Timer::default();
    let weak_t = win.as_weak();
    timer.start(slint::TimerMode::SingleShot, Duration::from_secs(2), move || {
        if let Some(win) = weak_t.upgrade() {
            let _ = win.hide();
        }
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop()?;

    println!(
        "SPIKE_RESULT mode=default-skia graphics_api={}",
        api_reported
            .borrow()
            .as_deref()
            .unwrap_or("None(no_rendering_notifier)")
    );
    Ok(())
}

// -----------------------------------------------------------------------------
// 方案 A：Slint `skia-opengl` + `mpv_render_context(opengl)` 离屏 FBO 纹理共享
// -----------------------------------------------------------------------------

const GL_TEXTURE_2D: c_uint = 0x0DE1;
const GL_RGBA: c_uint = 0x1908;
const GL_RGBA8: c_int = 0x8058;
const GL_UNSIGNED_BYTE: c_uint = 0x1401;
const GL_TEXTURE_MIN_FILTER: c_uint = 0x2801;
const GL_TEXTURE_MAG_FILTER: c_uint = 0x2800;
const GL_TEXTURE_WRAP_S: c_uint = 0x2802;
const GL_TEXTURE_WRAP_T: c_uint = 0x2803;
const GL_LINEAR: c_int = 0x2601;
const GL_CLAMP_TO_EDGE: c_int = 0x812F;
const GL_FRAMEBUFFER: c_uint = 0x8D40;
const GL_COLOR_ATTACHMENT0: c_uint = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: c_uint = 0x8CD5;

struct MiniGl {
    gen_framebuffers: unsafe extern "system" fn(c_int, *mut c_uint),
    bind_framebuffer: unsafe extern "system" fn(c_uint, c_uint),
    delete_framebuffers: unsafe extern "system" fn(c_int, *const c_uint),
    framebuffer_texture_2d: unsafe extern "system" fn(c_uint, c_uint, c_uint, c_uint, c_int),
    check_framebuffer_status: unsafe extern "system" fn(c_uint) -> c_uint,
    gen_textures: unsafe extern "system" fn(c_int, *mut c_uint),
    bind_texture: unsafe extern "system" fn(c_uint, c_uint),
    delete_textures: unsafe extern "system" fn(c_int, *const c_uint),
    tex_image_2d: unsafe extern "system" fn(
        c_uint,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_uint,
        c_uint,
        *const c_void,
    ),
    tex_parameter_i: unsafe extern "system" fn(c_uint, c_uint, c_int),
    flush: unsafe extern "system" fn(),
}

impl MiniGl {
    unsafe fn load(get_proc: &dyn Fn(&CStr) -> *const c_void) -> Option<Self> {
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let c = CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes()).ok()?;
                let p = get_proc(c);
                if p.is_null() {
                    return None;
                }
                unsafe { std::mem::transmute::<*const c_void, $ty>(p) }
            }};
        }
        Some(Self {
            gen_framebuffers: sym!("glGenFramebuffers", unsafe extern "system" fn(c_int, *mut c_uint)),
            bind_framebuffer: sym!("glBindFramebuffer", unsafe extern "system" fn(c_uint, c_uint)),
            delete_framebuffers: sym!("glDeleteFramebuffers", unsafe extern "system" fn(c_int, *const c_uint)),
            framebuffer_texture_2d: sym!(
                "glFramebufferTexture2D",
                unsafe extern "system" fn(c_uint, c_uint, c_uint, c_uint, c_int)
            ),
            check_framebuffer_status: sym!(
                "glCheckFramebufferStatus",
                unsafe extern "system" fn(c_uint) -> c_uint
            ),
            gen_textures: sym!("glGenTextures", unsafe extern "system" fn(c_int, *mut c_uint)),
            bind_texture: sym!("glBindTexture", unsafe extern "system" fn(c_uint, c_uint)),
            delete_textures: sym!("glDeleteTextures", unsafe extern "system" fn(c_int, *const c_uint)),
            tex_image_2d: sym!(
                "glTexImage2D",
                unsafe extern "system" fn(
                    c_uint, c_int, c_int, c_int, c_int, c_int, c_uint, c_uint, *const c_void
                )
            ),
            tex_parameter_i: sym!("glTexParameteri", unsafe extern "system" fn(c_uint, c_uint, c_int)),
            flush: sym!("glFlush", unsafe extern "system" fn()),
        })
    }
}

struct GlSpikeState {
    gl: MiniGl,
    lib: Arc<MpvLibrary>,
    mpv: MpvInstance,
    render_ctx: *mut MpvRenderContext,
    #[cfg(windows)]
    hdc: *mut c_void,
    #[cfg(windows)]
    slint_glrc: *mut c_void,
    #[cfg(windows)]
    mpv_glrc: *mut c_void,
    fbo: c_uint,
    tex: c_uint,
    tex_w: i32,
    tex_h: i32,
    frames_rendered: u32,
    render_time_sum_ms: f64,
    first_frame_ms: Option<f64>,
    start_instant: Instant,
}

impl Drop for GlSpikeState {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            wglMakeCurrent(self.hdc, self.mpv_glrc);
        }
        if !self.render_ctx.is_null() {
            unsafe {
                self.lib.render_context_free(self.render_ctx);
            }
            self.render_ctx = std::ptr::null_mut();
        }
        unsafe {
            if self.fbo != 0 {
                (self.gl.delete_framebuffers)(1, &self.fbo);
            }
        }
        #[cfg(windows)]
        unsafe {
            wglMakeCurrent(self.hdc, self.slint_glrc);
            if !self.mpv_glrc.is_null() {
                wglDeleteContext(self.mpv_glrc);
            }
        }
        unsafe {
            if self.tex != 0 {
                (self.gl.delete_textures)(1, &self.tex);
            }
        }
    }
}

#[cfg(windows)]
#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglGetCurrentDC() -> *mut c_void;
    fn wglGetCurrentContext() -> *mut c_void;
    fn wglCreateContext(hdc: *mut c_void) -> *mut c_void;
    fn wglShareLists(hglrc1: *mut c_void, hglrc2: *mut c_void) -> i32;
    fn wglMakeCurrent(hdc: *mut c_void, hglrc: *mut c_void) -> i32;
    fn wglDeleteContext(hglrc: *mut c_void) -> i32;
}

unsafe extern "C" fn mpv_gl_get_proc_address(
    ctx: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    if ctx.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `ctx` 指向仅在 `mpv_render_context_create` 调用期间存活的闭包引用。
    let getter = unsafe { &*(ctx as *const &dyn Fn(&CStr) -> *const c_void) };
    let cname = unsafe { CStr::from_ptr(name) };
    getter(cname) as *mut c_void
}

fn run_scheme_a_opengl(dll_path: &Path, video_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cpu_start = process_cpu_time_ms();
    let wall_start = Instant::now();

    slint::BackendSelector::new()
        .renderer_name("skia-opengl".to_string())
        .select()?;

    let lib = MpvLibrary::load_from_absolute_path(dll_path)?;
    let win = SpikeVideoWindow::new()?;
    win.set_mode_title("Scheme A: Skia OpenGL + mpv_render_context(GL FBO)".into());

    let state_cell: Rc<RefCell<Option<GlSpikeState>>> = Rc::new(RefCell::new(None));
    let hwdec_observed = Rc::new(RefCell::new(String::from("unknown")));
    let stats_out = Rc::new(RefCell::new((0u32, 0.0f64, None::<f64>)));

    let state_cb = Rc::clone(&state_cell);
    let hwdec_cb = Rc::clone(&hwdec_observed);
    let stats_cb = Rc::clone(&stats_out);
    let win_weak = win.as_weak();
    let video_path_owned = video_path.to_path_buf();

    win.window()
        .set_rendering_notifier(move |render_state, graphics_api| match render_state {
            RenderingState::RenderingSetup => {
                if let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api {
                    let Some(gl) = (unsafe { MiniGl::load(get_proc_address) }) else {
                        eprintln!("[Scheme A] 解析 OpenGL FBO 函数失败");
                        return;
                    };

                    let tex_w = 1280;
                    let tex_h = 720;

                    #[cfg(windows)]
                    let (hdc, slint_glrc, mpv_glrc) = unsafe {
                        let hdc = wglGetCurrentDC();
                        let slint_glrc = wglGetCurrentContext();
                        let mpv_glrc = wglCreateContext(hdc);
                        if mpv_glrc.is_null() {
                            eprintln!("[Scheme A] wglCreateContext 失败");
                            return;
                        }
                        if wglShareLists(slint_glrc, mpv_glrc) == 0 {
                            eprintln!("[Scheme A] wglShareLists 失败");
                            wglDeleteContext(mpv_glrc);
                            return;
                        }
                        (hdc, slint_glrc, mpv_glrc)
                    };

                    // 切换到独立的 mpv_glrc 创建共享纹理、FBO 与 mpv_render_context，零污染 Skia 状态缓存
                    #[cfg(windows)]
                    unsafe {
                        wglMakeCurrent(hdc, mpv_glrc);
                    }

                    let mut fbo: c_uint = 0;
                    let mut tex: c_uint = 0;
                    unsafe {
                        (gl.gen_textures)(1, &mut tex);
                        (gl.bind_texture)(GL_TEXTURE_2D, tex);
                        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                        (gl.tex_image_2d)(
                            GL_TEXTURE_2D,
                            0,
                            GL_RGBA8,
                            tex_w,
                            tex_h,
                            0,
                            GL_RGBA,
                            GL_UNSIGNED_BYTE,
                            std::ptr::null(),
                        );
                        (gl.gen_framebuffers)(1, &mut fbo);
                        (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
                        (gl.framebuffer_texture_2d)(
                            GL_FRAMEBUFFER,
                            GL_COLOR_ATTACHMENT0,
                            GL_TEXTURE_2D,
                            tex,
                            0,
                        );
                        let status = (gl.check_framebuffer_status)(GL_FRAMEBUFFER);
                        (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                        (gl.bind_texture)(GL_TEXTURE_2D, 0);
                        if status != GL_FRAMEBUFFER_COMPLETE {
                            eprintln!("[Scheme A] FBO 不完整: {status:#x}");
                            #[cfg(windows)]
                            wglMakeCurrent(hdc, slint_glrc);
                            return;
                        }
                    }

                    let mpv = match MpvInstance::new_sandboxed(
                        Arc::clone(&lib),
                        &[
                            ("vo", "libmpv"),
                            ("ao", "null"),
                            ("hwdec", "auto-safe"),
                            ("loop-file", "inf"),
                        ],
                    ) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("[Scheme A] mpv 初始化失败: {e}");
                            #[cfg(windows)]
                            unsafe {
                                wglMakeCurrent(hdc, slint_glrc);
                            }
                            return;
                        }
                    };

                    let api_type = CString::new("opengl").unwrap();
                    let getter_trait_obj: &dyn Fn(&CStr) -> *const c_void = get_proc_address;
                    let mut gl_init = MpvOpenGLInitParams {
                        get_proc_address: Some(mpv_gl_get_proc_address),
                        get_proc_address_ctx: (&getter_trait_obj
                            as *const &dyn Fn(&CStr) -> *const c_void)
                            as *mut c_void,
                    };
                    let mut params = [
                        MpvRenderParam {
                            param_type: MPV_RENDER_PARAM_API_TYPE,
                            data: api_type.as_ptr() as *mut c_void,
                        },
                        MpvRenderParam {
                            param_type: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                            data: (&mut gl_init as *mut MpvOpenGLInitParams).cast::<c_void>(),
                        },
                        MpvRenderParam::terminator(),
                    ];

                    let mut render_ctx: *mut MpvRenderContext = std::ptr::null_mut();
                    let rc = unsafe {
                        lib.render_context_create(
                            &mut render_ctx,
                            mpv.raw_handle(),
                            params.as_mut_ptr(),
                        )
                    };

                    // 立即切回 Skia 的原始 WGL 上下文
                    #[cfg(windows)]
                    unsafe {
                        (gl.flush)();
                        wglMakeCurrent(hdc, slint_glrc);
                    }

                    if rc < 0 || render_ctx.is_null() {
                        eprintln!(
                            "[Scheme A] mpv_render_context_create(opengl) 失败: {}",
                            lib.error_string(rc)
                        );
                        return;
                    }

                    let _ = mpv.load_local_file(&video_path_owned);

                    if let (Some(w), Some(nz_tex)) = (win_weak.upgrade(), NonZeroU32::new(tex)) {
                        let img = unsafe {
                            BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                                nz_tex,
                                [tex_w as u32, tex_h as u32].into(),
                            )
                            .origin(BorrowedOpenGLTextureOrigin::BottomLeft)
                            .build()
                        };
                        w.set_video_frame(img);
                    }

                    *state_cb.borrow_mut() = Some(GlSpikeState {
                        gl,
                        lib: Arc::clone(&lib),
                        mpv,
                        render_ctx,
                        #[cfg(windows)]
                        hdc,
                        #[cfg(windows)]
                        slint_glrc,
                        #[cfg(windows)]
                        mpv_glrc,
                        fbo,
                        tex,
                        tex_w,
                        tex_h,
                        frames_rendered: 0,
                        render_time_sum_ms: 0.0,
                        first_frame_ms: None,
                        start_instant: wall_start,
                    });
                }
            }
            RenderingState::BeforeRendering => {
                let mut borrow = state_cb.borrow_mut();
                let Some(st) = borrow.as_mut() else { return };

                // 排空事件队列
                while st.mpv.wait_event(0.0).0 != MPV_EVENT_NONE {}

                #[cfg(windows)]
                unsafe {
                    wglMakeCurrent(st.hdc, st.mpv_glrc);
                }

                let flags = unsafe { st.lib.render_context_update(st.render_ctx) };
                if (flags & MPV_RENDER_UPDATE_FRAME) != 0 {
                    let t0 = Instant::now();
                    let mut fbo_param = MpvOpenGLFbo {
                        fbo: st.fbo as c_int,
                        w: st.tex_w,
                        h: st.tex_h,
                        internal_format: GL_RGBA8,
                    };
                    let mut flip_y: c_int = 0;
                    let mut block: c_int = 0;
                    let mut render_params = [
                        MpvRenderParam {
                            param_type: MPV_RENDER_PARAM_OPENGL_FBO,
                            data: (&mut fbo_param as *mut MpvOpenGLFbo).cast::<c_void>(),
                        },
                        MpvRenderParam {
                            param_type: MPV_RENDER_PARAM_FLIP_Y,
                            data: (&mut flip_y as *mut c_int).cast::<c_void>(),
                        },
                        MpvRenderParam {
                            param_type: MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                            data: (&mut block as *mut c_int).cast::<c_void>(),
                        },
                        MpvRenderParam::terminator(),
                    ];
                    let _ = unsafe {
                        st.lib
                            .render_context_render(st.render_ctx, render_params.as_mut_ptr())
                    };
                    unsafe {
                        (st.gl.flush)();
                    }
                    let dt_ms = t0.elapsed().as_secs_f64() * 1000.0;
                    st.frames_rendered += 1;
                    st.render_time_sum_ms += dt_ms;
                    if st.first_frame_ms.is_none() {
                        st.first_frame_ms = Some(st.start_instant.elapsed().as_secs_f64() * 1000.0);
                    }
                    if let Some(hw) = st.mpv.get_property_string("hwdec-current") {
                        *hwdec_cb.borrow_mut() = hw;
                    }
                    *stats_cb.borrow_mut() =
                        (st.frames_rendered, st.render_time_sum_ms, st.first_frame_ms);
                }

                #[cfg(windows)]
                unsafe {
                    wglMakeCurrent(st.hdc, st.slint_glrc);
                }
            }
            RenderingState::RenderingTeardown => {
                *state_cb.borrow_mut() = None;
            }
            _ => {}
        })?;

    win.show()?;

    // 约 60Hz 驱动持续请求重绘，运行 2.2 秒后自动收口
    let redraw_timer = slint::Timer::default();
    let weak_redraw = win.as_weak();
    redraw_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            if let Some(w) = weak_redraw.upgrade() {
                w.window().request_redraw();
            }
        },
    );

    let stop_timer = slint::Timer::default();
    let weak_stop = win.as_weak();
    stop_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(2200),
        move || {
            if let Some(w) = weak_stop.upgrade() {
                let _ = w.hide();
            }
            let _ = slint::quit_event_loop();
        },
    );

    slint::run_event_loop()?;

    let cpu_delta_ms = process_cpu_time_ms() - cpu_start;
    let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
    let (frames, sum_ms, first_ms) = *stats_out.borrow();
    let avg_ms = if frames > 0 {
        sum_ms / frames as f64
    } else {
        0.0
    };
    println!(
        "SPIKE_RESULT mode=gl frames={frames} first_frame_ms={:.2} avg_render_ms={:.3} hwdec={} cpu_ms={:.1} wall_ms={:.1}",
        first_ms.unwrap_or(-1.0),
        avg_ms,
        hwdec_observed.borrow(),
        cpu_delta_ms,
        wall_ms
    );
    Ok(())
}

// -----------------------------------------------------------------------------
// 方案 B：Win32 HWND + mpv `wid` 嵌入 (`vo=gpu` + `hwdec=auto-safe`)
// -----------------------------------------------------------------------------

#[cfg(windows)]
fn run_scheme_b_child_hwnd(
    dll_path: &Path,
    video_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
        TranslateMessage, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    let cpu_start = process_cpu_time_ms();
    let wall_start = Instant::now();

    let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let title: Vec<u16> = "Scheme B: HWND wid Spike\0".encode_utf16().collect();
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            80,
            80,
            960,
            540,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("CreateWindowExW 创建宿主机测试 HWND 失败".into());
    }

    let lib = MpvLibrary::load_from_absolute_path(dll_path)?;
    let wid_str = (hwnd as isize).to_string();
    let mpv = MpvInstance::new_sandboxed(
        lib,
        &[
            ("wid", &wid_str),
            ("vo", "gpu"),
            ("ao", "null"),
            ("hwdec", "auto-safe"),
            ("loop-file", "inf"),
        ],
    )?;

    mpv.load_local_file(video_path)?;
    mpv.wait_until_file_loaded(Duration::from_secs(5))?;
    let first_loaded_ms = wall_start.elapsed().as_secs_f64() * 1000.0;

    let run_until = Instant::now() + Duration::from_secs(2);
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while Instant::now() < run_until {
        unsafe {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = mpv.wait_event(0.01);
    }

    let hwdec = mpv
        .get_property_string("hwdec-current")
        .unwrap_or_else(|| "none".to_string());
    let vf_fps = mpv.get_property_f64("estimated-vf-fps").unwrap_or(0.0);
    let dropped = mpv.get_property_i64("frame-drop-count").unwrap_or(0);

    drop(mpv);
    unsafe {
        DestroyWindow(hwnd);
    }

    let cpu_delta_ms = process_cpu_time_ms() - cpu_start;
    let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "SPIKE_RESULT mode=child-hwnd first_loaded_ms={first_loaded_ms:.2} est_vf_fps={vf_fps:.2} dropped={dropped} hwdec={hwdec} cpu_ms={cpu_delta_ms:.1} wall_ms={wall_ms:.1}"
    );
    Ok(())
}

#[cfg(not(windows))]
fn run_scheme_b_child_hwnd(
    _dll_path: &Path,
    _video_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    Err("仅支持 Windows".into())
}

// -----------------------------------------------------------------------------
// 方案 C：软件渲染回退 `MPV_RENDER_API_TYPE_SW` -> `SharedPixelBuffer<Rgba8Pixel>` -> `slint::Image`
// -----------------------------------------------------------------------------

fn run_scheme_c_software(
    dll_path: &Path,
    video_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let cpu_start = process_cpu_time_ms();
    let wall_start = Instant::now();

    let lib = MpvLibrary::load_from_absolute_path(dll_path)?;
    let mpv = MpvInstance::new_sandboxed(
        Arc::clone(&lib),
        &[
            ("vo", "libmpv"),
            ("ao", "null"),
            ("hwdec", "no"),
            ("sws-fast", "yes"),
            ("loop-file", "inf"),
        ],
    )?;

    let api_type = CString::new("sw")?;
    let mut init_params = [
        MpvRenderParam {
            param_type: MPV_RENDER_PARAM_API_TYPE,
            data: api_type.as_ptr() as *mut c_void,
        },
        MpvRenderParam::terminator(),
    ];
    let mut render_ctx: *mut MpvRenderContext = std::ptr::null_mut();
    let rc = unsafe {
        lib.render_context_create(&mut render_ctx, mpv.raw_handle(), init_params.as_mut_ptr())
    };
    if rc < 0 || render_ctx.is_null() {
        return Err(format!("mpv_render_context_create(sw) 失败: {}", lib.error_string(rc)).into());
    }

    mpv.load_local_file(video_path)?;
    mpv.wait_until_file_loaded(Duration::from_secs(5))?;

    let w = 1280u32;
    let h = 720u32;
    let mut size: [c_int; 2] = [w as c_int, h as c_int];
    let fmt = CString::new("rgb0")?;
    let mut stride: usize = (w as usize) * 4;
    let mut buf = vec![0u8; stride * (h as usize)];

    let mut frames = 0u32;
    let mut render_sum_ms = 0.0f64;
    let mut first_frame_ms = None;
    let deadline = Instant::now() + Duration::from_secs(2);

    while Instant::now() < deadline && frames < 60 {
        let flags = unsafe { lib.render_context_update(render_ctx) };
        if (flags & MPV_RENDER_UPDATE_FRAME) != 0 {
            let t0 = Instant::now();
            let mut render_params = [
                MpvRenderParam {
                    param_type: MPV_RENDER_PARAM_SW_SIZE,
                    data: size.as_mut_ptr().cast::<c_void>(),
                },
                MpvRenderParam {
                    param_type: MPV_RENDER_PARAM_SW_FORMAT,
                    data: fmt.as_ptr() as *mut c_void,
                },
                MpvRenderParam {
                    param_type: MPV_RENDER_PARAM_SW_STRIDE,
                    data: (&mut stride as *mut usize).cast::<c_void>(),
                },
                MpvRenderParam {
                    param_type: MPV_RENDER_PARAM_SW_POINTER,
                    data: buf.as_mut_ptr().cast::<c_void>(),
                },
                MpvRenderParam::terminator(),
            ];
            let _ = unsafe { lib.render_context_render(render_ctx, render_params.as_mut_ptr()) };
            for px in buf.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
            let pixel_buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&buf, w, h);
            let _img = Image::from_rgba8(pixel_buf);
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            render_sum_ms += dt;
            frames += 1;
            if first_frame_ms.is_none() {
                first_frame_ms = Some(wall_start.elapsed().as_secs_f64() * 1000.0);
            }
        } else {
            let _ = mpv.wait_event(0.005);
        }
    }

    unsafe {
        lib.render_context_free(render_ctx);
    }
    drop(mpv);

    let cpu_delta_ms = process_cpu_time_ms() - cpu_start;
    let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
    let avg_ms = if frames > 0 {
        render_sum_ms / frames as f64
    } else {
        0.0
    };
    println!(
        "SPIKE_RESULT mode=sw size={w}x{h} frames={frames} first_frame_ms={:.2} avg_render_ms={:.3} cpu_ms={:.1} wall_ms={:.1}",
        first_frame_ms.unwrap_or(-1.0),
        avg_ms,
        cpu_delta_ms,
        wall_ms
    );
    Ok(())
}
