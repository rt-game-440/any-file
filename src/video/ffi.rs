//! libmpv 2.x 精简 C ABI FFI 定义（对照官方 `mpv/client.h`、`mpv/render.h`、`mpv/render_gl.h`）。
//! 零外部 C 编译器/bindgen 依赖，供安全动态加载器通过 `GetProcAddress` 绑定符号。

use std::ffi::{c_char, c_double, c_int, c_ulong, c_void};

/// 不透明的 `mpv_handle` 指针目标类型
#[repr(C)]
pub struct MpvHandle {
    _private: [u8; 0],
}

/// 不透明的 `mpv_render_context` 指针目标类型
#[repr(C)]
pub struct MpvRenderContext {
    _private: [u8; 0],
}

/// `mpv_format` 枚举常量
pub const MPV_FORMAT_NONE: c_int = 0;
pub const MPV_FORMAT_STRING: c_int = 1;
pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_INT64: c_int = 4;
pub const MPV_FORMAT_DOUBLE: c_int = 5;

/// `mpv_event_id` 常用常量
pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_VIDEO_RECONFIG: c_int = 17;

/// `mpv_event` C 结构体布局
#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

/// `mpv_render_param_type` 常量（严格对应 `mpv/render.h`）
pub const MPV_RENDER_PARAM_INVALID: c_int = 0;
pub const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: c_int = 4;
pub const MPV_RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
pub const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub const MPV_RENDER_PARAM_SW_SIZE: c_int = 17;
pub const MPV_RENDER_PARAM_SW_FORMAT: c_int = 18;
pub const MPV_RENDER_PARAM_SW_STRIDE: c_int = 19;
pub const MPV_RENDER_PARAM_SW_POINTER: c_int = 20;

/// `mpv_render_context_update` 返回的刷新标志位
pub const MPV_RENDER_UPDATE_FRAME: u64 = 1 << 0;

/// `mpv_render_param` C 结构体布局
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MpvRenderParam {
    pub param_type: c_int,
    pub data: *mut c_void,
}

impl MpvRenderParam {
    pub const fn terminator() -> Self {
        Self {
            param_type: MPV_RENDER_PARAM_INVALID,
            data: std::ptr::null_mut(),
        }
    }
}

/// `mpv_opengl_init_params` C 结构体布局（对应 `mpv/render_gl.h`）
#[repr(C)]
pub struct MpvOpenGLInitParams {
    pub get_proc_address:
        Option<unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void>,
    pub get_proc_address_ctx: *mut c_void,
}

/// `mpv_opengl_fbo` C 结构体布局（对应 `mpv/render_gl.h`）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MpvOpenGLFbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

/// `mpv_render_update_fn` 回调签名
pub type MpvRenderUpdateFn = unsafe extern "C" fn(cb_ctx: *mut c_void);

// 动态解析的 libmpv C API 函数指针签名
pub type FnMpvClientApiVersion = unsafe extern "C" fn() -> c_ulong;
pub type FnMpvErrorString = unsafe extern "C" fn(error: c_int) -> *const c_char;
pub type FnMpvCreate = unsafe extern "C" fn() -> *mut MpvHandle;
pub type FnMpvInitialize = unsafe extern "C" fn(ctx: *mut MpvHandle) -> c_int;
pub type FnMpvTerminateDestroy = unsafe extern "C" fn(ctx: *mut MpvHandle);
pub type FnMpvSetOptionString =
    unsafe extern "C" fn(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
pub type FnMpvSetPropertyString =
    unsafe extern "C" fn(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
pub type FnMpvGetPropertyString =
    unsafe extern "C" fn(ctx: *mut MpvHandle, name: *const c_char) -> *mut c_char;
pub type FnMpvGetProperty = unsafe extern "C" fn(
    ctx: *mut MpvHandle,
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
) -> c_int;
pub type FnMpvFree = unsafe extern "C" fn(data: *mut c_void);
pub type FnMpvCommand =
    unsafe extern "C" fn(ctx: *mut MpvHandle, args: *const *const c_char) -> c_int;
pub type FnMpvWaitEvent =
    unsafe extern "C" fn(ctx: *mut MpvHandle, timeout: c_double) -> *mut MpvEvent;
pub type FnMpvRenderContextCreate = unsafe extern "C" fn(
    res: *mut *mut MpvRenderContext,
    mpv: *mut MpvHandle,
    params: *mut MpvRenderParam,
) -> c_int;
pub type FnMpvRenderContextRender = unsafe extern "C" fn(
    ctx: *mut MpvRenderContext,
    params: *mut MpvRenderParam,
) -> c_int;
pub type FnMpvRenderContextUpdate = unsafe extern "C" fn(ctx: *mut MpvRenderContext) -> u64;
pub type FnMpvRenderContextSetUpdateCallback = unsafe extern "C" fn(
    ctx: *mut MpvRenderContext,
    callback: Option<MpvRenderUpdateFn>,
    callback_ctx: *mut c_void,
);
pub type FnMpvRenderContextReportSwap = unsafe extern "C" fn(ctx: *mut MpvRenderContext);
pub type FnMpvRenderContextFree = unsafe extern "C" fn(ctx: *mut MpvRenderContext);
