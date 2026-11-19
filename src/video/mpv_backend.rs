//! 懒加载视频后端实现与沙箱化 `MpvInstance` 封装。

use std::ffi::{CStr, CString, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::ports::{VideoBackend, VideoProbeInfo};
use crate::video::compositing::VideoFrameRgba;
use crate::video::error::VideoError;
use crate::video::ffi::*;
use crate::video::loader::MpvLibrary;
use crate::video::sandbox::{MpvSandboxPolicy, validate_local_media_path};

/// 沙箱化 libmpv 实例（RAII 自动释放底层 `mpv_handle`）。
///
/// 构造时强制先应用全部 `MpvSandboxPolicy::required_options()` 再调用 `mpv_initialize()`；
/// 任何沙箱参数设置失败均会立即销毁句柄并返回错误。
pub struct MpvInstance {
    lib: Arc<MpvLibrary>,
    handle: *mut MpvHandle,
}

// SAFETY: `MpvInstance` 独占底层 `mpv_handle` 并在调用侧按需加锁或单线程访问。
unsafe impl Send for MpvInstance {}

impl Drop for MpvInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 释放由 `mpv_create` 创建的有效句柄。
            unsafe {
                (self.lib.mpv_terminate_destroy)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

impl MpvInstance {
    /// 创建一个已应用全部强制沙箱参数的 `MpvInstance`。
    /// `extra_options` 可用于设置渲染/探测相关非安全选项（如 `vo=libmpv`、`hwdec=auto-safe`、`wid` 等），
    /// 随后会再次覆写强制沙箱参数，确保调用方无法绕过沙箱策略。
    pub fn new_sandboxed(
        lib: Arc<MpvLibrary>,
        extra_options: &[(&str, &str)],
    ) -> Result<Self, VideoError> {
        // SAFETY: `mpv_create` 无前置状态要求，返回空指针表示分配失败。
        let handle = unsafe { (lib.mpv_create)() };
        if handle.is_null() {
            return Err(VideoError::CreateFailed);
        }

        let instance = Self { lib, handle };

        // 先应用额外选项，再强制应用沙箱选项以确保沙箱参数拥有最终覆盖权
        for (opt, val) in extra_options {
            instance.set_option_string(opt, val)?;
        }
        for (opt, val) in MpvSandboxPolicy::required_options() {
            if let Err(err) = instance.set_option_string(opt, val) {
                return Err(match err {
                    VideoError::MpvApiError { code, message, .. } => {
                        VideoError::SandboxOptionFailed {
                            option: (*opt).to_string(),
                            value: (*val).to_string(),
                            code,
                            message,
                        }
                    }
                    other => other,
                });
            }
        }

        // SAFETY: `instance.handle` 为有效的未初始化 `mpv_handle`。
        let code = unsafe { (instance.lib.mpv_initialize)(instance.handle) };
        if code < 0 {
            return Err(VideoError::InitializeFailed {
                code,
                message: instance.lib.error_string(code),
            });
        }

        Ok(instance)
    }

    /// 访问底层共享库句柄（供 Spike 或高级渲染封装使用）。
    pub fn library(&self) -> &Arc<MpvLibrary> {
        &self.lib
    }

    /// 访问原始 `mpv_handle` 指针（供创建 `mpv_render_context` 使用）。
    pub fn raw_handle(&self) -> *mut MpvHandle {
        self.handle
    }

    /// 设置字符串选项（通常在 `mpv_initialize` 之前调用）。
    pub fn set_option_string(&self, name: &str, value: &str) -> Result<(), VideoError> {
        let c_name = CString::new(name).map_err(|_| VideoError::MpvApiError {
            operation: "mpv_set_option_string",
            code: -4,
            message: "选项名包含 NUL 字节".to_string(),
        })?;
        let c_val = CString::new(value).map_err(|_| VideoError::MpvApiError {
            operation: "mpv_set_option_string",
            code: -4,
            message: "选项值包含 NUL 字节".to_string(),
        })?;
        // SAFETY: 句柄和 C 字符串均有效。
        let code = unsafe {
            (self.lib.mpv_set_option_string)(self.handle, c_name.as_ptr(), c_val.as_ptr())
        };
        if code < 0 {
            return Err(VideoError::MpvApiError {
                operation: "mpv_set_option_string",
                code,
                message: self.lib.error_string(code),
            });
        }
        Ok(())
    }

    /// 设置运行时字符串属性。
    pub fn set_property_string(&self, name: &str, value: &str) -> Result<(), VideoError> {
        let c_name = CString::new(name).map_err(|_| VideoError::MpvApiError {
            operation: "mpv_set_property_string",
            code: -4,
            message: "属性名包含 NUL 字节".to_string(),
        })?;
        let c_val = CString::new(value).map_err(|_| VideoError::MpvApiError {
            operation: "mpv_set_property_string",
            code: -4,
            message: "属性值包含 NUL 字节".to_string(),
        })?;
        // SAFETY: 句柄和 C 字符串均有效。
        let code = unsafe {
            (self.lib.mpv_set_property_string)(self.handle, c_name.as_ptr(), c_val.as_ptr())
        };
        if code < 0 {
            return Err(VideoError::MpvApiError {
                operation: "mpv_set_property_string",
                code,
                message: self.lib.error_string(code),
            });
        }
        Ok(())
    }

    /// 查询运行时字符串属性。
    pub fn get_property_string(&self, name: &str) -> Option<String> {
        let c_name = CString::new(name).ok()?;
        // SAFETY: 句柄有效；返回指针非空时需使用 `mpv_free` 释放。
        let ptr = unsafe { (self.lib.mpv_get_property_string)(self.handle, c_name.as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            (self.lib.mpv_free)(ptr.cast::<c_void>());
        }
        Some(value)
    }

    /// 查询整数属性（`MPV_FORMAT_INT64`）。
    pub fn get_property_i64(&self, name: &str) -> Option<i64> {
        let c_name = CString::new(name).ok()?;
        let mut out: i64 = 0;
        // SAFETY: 传入匹配的 `MPV_FORMAT_INT64` 与有效 `i64` 输出指针。
        let code = unsafe {
            (self.lib.mpv_get_property)(
                self.handle,
                c_name.as_ptr(),
                MPV_FORMAT_INT64,
                (&mut out as *mut i64).cast::<c_void>(),
            )
        };
        if code >= 0 { Some(out) } else { None }
    }

    /// 查询浮点属性（`MPV_FORMAT_DOUBLE`）。
    pub fn get_property_f64(&self, name: &str) -> Option<f64> {
        let c_name = CString::new(name).ok()?;
        let mut out: f64 = 0.0;
        // SAFETY: 传入匹配的 `MPV_FORMAT_DOUBLE` 与有效 `f64` 输出指针。
        let code = unsafe {
            (self.lib.mpv_get_property)(
                self.handle,
                c_name.as_ptr(),
                MPV_FORMAT_DOUBLE,
                (&mut out as *mut f64).cast::<c_void>(),
            )
        };
        if code >= 0 { Some(out) } else { None }
    }

    /// 经本地路径沙箱守卫校验后，执行 `loadfile` 加载本地媒体文件。
    pub fn load_local_file(&self, path: &Path) -> Result<(), VideoError> {
        let safe_path = validate_local_media_path(path)?;
        self.command(&["loadfile", &safe_path, "replace"])
    }

    /// 执行同步 mpv 命令。
    pub fn command(&self, args: &[&str]) -> Result<(), VideoError> {
        let c_args: Vec<CString> = args
            .iter()
            .map(|s| {
                CString::new(*s).map_err(|_| VideoError::MpvApiError {
                    operation: "mpv_command",
                    code: -4,
                    message: "命令参数包含 NUL 字节".to_string(),
                })
            })
            .collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const std::ffi::c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());

        // SAFETY: `ptrs` 以 NULL 结尾且所有 CString 在调用期间存活。
        let code = unsafe { (self.lib.mpv_command)(self.handle, ptrs.as_ptr()) };
        if code < 0 {
            return Err(VideoError::MpvApiError {
                operation: "mpv_command",
                code,
                message: self.lib.error_string(code),
            });
        }
        Ok(())
    }

    /// 等待下一条 mpv 事件，返回 `(event_id, error)`。
    pub fn wait_event(&self, timeout_secs: f64) -> (c_int, c_int) {
        // SAFETY: `mpv_wait_event` 返回指向 mpv 内部事件缓冲的只读指针，下次调用前有效。
        let ev = unsafe { (self.lib.mpv_wait_event)(self.handle, timeout_secs) };
        if ev.is_null() {
            return (MPV_EVENT_NONE, 0);
        }
        unsafe { ((*ev).event_id, (*ev).error) }
    }

    /// 等待文件加载完成（`MPV_EVENT_FILE_LOADED`）或超时/失败。
    pub fn wait_until_file_loaded(&self, timeout: Duration) -> Result<(), VideoError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(VideoError::MpvApiError {
                    operation: "wait_until_file_loaded",
                    code: -1,
                    message: "等待 mpv 加载媒体文件超时".to_string(),
                });
            }
            let wait_s = remaining.as_secs_f64().min(0.05);
            let (ev_id, err) = self.wait_event(wait_s);
            if ev_id == MPV_EVENT_FILE_LOADED {
                return Ok(());
            }
            if ev_id == MPV_EVENT_END_FILE && err < 0 {
                return Err(VideoError::MpvApiError {
                    operation: "loadfile",
                    code: err,
                    message: self.lib.error_string(err),
                });
            }
        }
    }
}

/// 懒加载的 libmpv 视频后端（实现 `VideoBackend` trait）。
///
/// 构造阶段完全不触碰文件系统或加载 `mpv-2.dll`，确保图片、音频、文本与空态启动路径零额外 DLL 开销；
/// 仅在首次调用 `library()`、`is_available()` 或 `probe()` 时才按可信绝对路径懒加载。
#[derive(Debug)]
pub struct LazyMpvVideoBackend {
    dll_path: Option<PathBuf>,
    library: OnceLock<Result<Arc<MpvLibrary>, VideoError>>,
}

impl LazyMpvVideoBackend {
    /// 使用默认策略（当前可执行文件同目录下的 `mpv-2.dll`）构造懒加载后端。
    pub fn for_exe_dir() -> Self {
        Self {
            dll_path: None,
            library: OnceLock::new(),
        }
    }

    /// 使用显式可信绝对路径构造懒加载后端（便于开发 Spike 与自动化集成测试）。
    pub fn with_absolute_dll_path(path: PathBuf) -> Self {
        Self {
            dll_path: Some(path),
            library: OnceLock::new(),
        }
    }

    /// 检查是否已触发过加载尝试（未触发前为 `false`，用于验证非视频路径零加载）。
    pub fn is_loaded(&self) -> bool {
        self.library.get().is_some()
    }

    /// 触发懒加载并返回共享 `MpvLibrary` 句柄。
    pub fn library(&self) -> Result<Arc<MpvLibrary>, VideoError> {
        self.library
            .get_or_init(|| match &self.dll_path {
                Some(path) => MpvLibrary::load_from_absolute_path(path),
                None => MpvLibrary::load_from_exe_dir(),
            })
            .clone()
    }

    /// 使用 `MPV_RENDER_API_TYPE_SW` 渲染指定本地视频的一帧 RGBA8 像素（软件渲染回退验证）。
    pub fn render_first_frame_sw(
        &self,
        path: &Path,
        width: u32,
        height: u32,
    ) -> Result<VideoFrameRgba, VideoError> {
        let lib = self.library()?;
        let mpv = MpvInstance::new_sandboxed(
            Arc::clone(&lib),
            &[
                ("vo", "libmpv"),
                ("ao", "null"),
                ("pause", "yes"),
                ("hwdec", "no"),
                ("sws-fast", "yes"),
            ],
        )?;

        let api_type = CString::new("sw").expect("static cstr");
        let mut init_params = [
            MpvRenderParam {
                param_type: MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            MpvRenderParam::terminator(),
        ];

        let mut render_ctx: *mut MpvRenderContext = std::ptr::null_mut();
        // SAFETY: `mpv.raw_handle()` 已初始化，`init_params` 以 `MPV_RENDER_PARAM_INVALID` 结尾。
        let code = unsafe {
            (lib.mpv_render_context_create)(
                &mut render_ctx,
                mpv.raw_handle(),
                init_params.as_mut_ptr(),
            )
        };
        if code < 0 || render_ctx.is_null() {
            return Err(VideoError::MpvApiError {
                operation: "mpv_render_context_create(sw)",
                code,
                message: lib.error_string(code),
            });
        }

        // 确保任何提前返回都严格先释放 `render_ctx`，再释放 `mpv`（libmpv 生命周期硬约束）
        struct RenderCtxGuard {
            lib: Arc<MpvLibrary>,
            ctx: *mut MpvRenderContext,
        }
        impl Drop for RenderCtxGuard {
            fn drop(&mut self) {
                if !self.ctx.is_null() {
                    unsafe {
                        (self.lib.mpv_render_context_free)(self.ctx);
                    }
                }
            }
        }
        let guard = RenderCtxGuard {
            lib: Arc::clone(&lib),
            ctx: render_ctx,
        };

        mpv.load_local_file(path)?;
        mpv.wait_until_file_loaded(Duration::from_secs(5))?;

        // 等待解码器就绪第一帧
        let frame_deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < frame_deadline {
            let flags = unsafe { (lib.mpv_render_context_update)(guard.ctx) };
            if (flags & MPV_RENDER_UPDATE_FRAME) != 0 {
                break;
            }
            let _ = mpv.wait_event(0.02);
        }

        let mut size: [c_int; 2] = [width as c_int, height as c_int];
        let fmt = CString::new("rgb0").expect("static cstr");
        let mut stride: usize = (width as usize) * 4;
        let mut rgba = vec![0u8; stride * (height as usize)];

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
                data: rgba.as_mut_ptr().cast::<c_void>(),
            },
            MpvRenderParam::terminator(),
        ];

        // SAFETY: `rgba` 缓冲区容量严格等于 `stride * height`，且所有指针在调用期间存活。
        let render_code =
            unsafe { (lib.mpv_render_context_render)(guard.ctx, render_params.as_mut_ptr()) };
        if render_code < 0 {
            return Err(VideoError::MpvApiError {
                operation: "mpv_render_context_render(sw)",
                code: render_code,
                message: lib.error_string(render_code),
            });
        }

        // 将 `rgb0` 尾字节规范为不透明 alpha=255，匹配 Slint `Rgba8Pixel`
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }

        Ok(VideoFrameRgba {
            width,
            height,
            rgba,
        })
    }
}

impl VideoBackend for LazyMpvVideoBackend {
    fn is_available(&self) -> bool {
        self.library().is_ok()
    }

    fn probe(&self, path: &Path) -> Option<VideoProbeInfo> {
        let lib = self.library().ok()?;
        let mpv = MpvInstance::new_sandboxed(
            lib,
            &[
                ("vo", "null"),
                ("ao", "null"),
                ("pause", "yes"),
                ("keep-open", "yes"),
            ],
        )
        .ok()?;

        mpv.load_local_file(path).ok()?;
        mpv.wait_until_file_loaded(Duration::from_secs(5)).ok()?;

        let track_count = mpv.get_property_i64("track-list/count").unwrap_or(0);
        let mut has_video_stream = false;
        let mut has_audio_stream = false;
        for idx in 0..track_count {
            if let Some(kind) = mpv.get_property_string(&format!("track-list/{idx}/type")) {
                match kind.as_str() {
                    "video" => {
                        let is_albumart = mpv
                            .get_property_string(&format!("track-list/{idx}/albumart"))
                            .as_deref()
                            == Some("yes");
                        if !is_albumart {
                            has_video_stream = true;
                        }
                    }
                    "audio" => has_audio_stream = true,
                    _ => {}
                }
            }
        }

        let width = mpv
            .get_property_i64("width")
            .or_else(|| mpv.get_property_i64("video-params/w"))
            .unwrap_or(0)
            .max(0) as u32;
        let height = mpv
            .get_property_i64("height")
            .or_else(|| mpv.get_property_i64("video-params/h"))
            .unwrap_or(0)
            .max(0) as u32;
        let duration_ms = mpv
            .get_property_f64("duration")
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(|d| (d * 1000.0).round() as u64);

        Some(VideoProbeInfo {
            has_video_stream,
            has_audio_stream,
            width,
            height,
            duration_ms,
        })
    }
}
