//! `mpv-2.dll` 安全动态加载器（ADR-0005）。
//!
//! 安全约束：
//! 1. 仅允许从绝对路径（默认为当前可执行文件同目录）加载 `mpv-2.dll`，拒绝相对路径；
//! 2. Windows 平台首次加载前调用 `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)`，
//!    并使用 `LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)`
//!    避免 DLL 劫持（绝不回退搜索 CWD 或用户 PATH）；
//! 3. 非视频启动路径完全不调用本加载器，实现零额外 DLL 开销。

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::video::error::VideoError;
use crate::video::ffi::*;

/// 默认随包分发的 libmpv DLL 文件名
pub const MPV_DLL_NAME: &str = "mpv-2.dll";

/// 已成功动态加载并绑定全部必需符号的 `mpv-2.dll` 句柄。
pub struct MpvLibrary {
    #[cfg(windows)]
    module: windows_sys::Win32::Foundation::HMODULE,
    loaded_path: PathBuf,
    api_version: u64,
    #[allow(dead_code)]
    pub(crate) mpv_client_api_version: FnMpvClientApiVersion,
    pub(crate) mpv_error_string: FnMpvErrorString,
    pub(crate) mpv_create: FnMpvCreate,
    pub(crate) mpv_initialize: FnMpvInitialize,
    pub(crate) mpv_terminate_destroy: FnMpvTerminateDestroy,
    pub(crate) mpv_set_option_string: FnMpvSetOptionString,
    pub(crate) mpv_set_property_string: FnMpvSetPropertyString,
    pub(crate) mpv_get_property_string: FnMpvGetPropertyString,
    pub(crate) mpv_get_property: FnMpvGetProperty,
    pub(crate) mpv_free: FnMpvFree,
    pub(crate) mpv_command: FnMpvCommand,
    pub(crate) mpv_wait_event: FnMpvWaitEvent,
    pub(crate) mpv_render_context_create: FnMpvRenderContextCreate,
    pub(crate) mpv_render_context_render: FnMpvRenderContextRender,
    pub(crate) mpv_render_context_update: FnMpvRenderContextUpdate,
    #[allow(dead_code)]
    pub(crate) mpv_render_context_set_update_callback: FnMpvRenderContextSetUpdateCallback,
    #[allow(dead_code)]
    pub(crate) mpv_render_context_report_swap: FnMpvRenderContextReportSwap,
    pub(crate) mpv_render_context_free: FnMpvRenderContextFree,
}

// SAFETY: libmpv 导出函数指针与只读 HMODULE 句柄本身可在多线程间安全共享，
// 具体每个 `mpv_handle` 实例的线程安全由上层封装保证。
unsafe impl Send for MpvLibrary {}
unsafe impl Sync for MpvLibrary {}

impl std::fmt::Debug for MpvLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MpvLibrary")
            .field("loaded_path", &self.loaded_path)
            .field("api_version", &format_args!("{:#x}", self.api_version))
            .finish()
    }
}

#[cfg(windows)]
impl Drop for MpvLibrary {
    fn drop(&mut self) {
        if !self.module.is_null() {
            // SAFETY: 释放由 `LoadLibraryExW` 加载的模块句柄。
            unsafe {
                windows_sys::Win32::Foundation::FreeLibrary(self.module);
            }
        }
    }
}

impl MpvLibrary {
    /// 获取当前可执行文件同目录下 `mpv-2.dll` 的预期绝对路径。
    pub fn default_exe_dir_dll_path() -> Result<PathBuf, VideoError> {
        let exe = std::env::current_exe().map_err(|e| VideoError::LibraryLoadFailed {
            path: PathBuf::from(MPV_DLL_NAME),
            reason: format!("无法解析当前可执行文件路径：{e}"),
        })?;
        let dir = exe
            .parent()
            .ok_or_else(|| VideoError::LibraryNotFound(PathBuf::from(MPV_DLL_NAME)))?;
        Ok(dir.join(MPV_DLL_NAME))
    }

    /// 从当前可执行文件所在目录安全加载 `mpv-2.dll`。
    pub fn load_from_exe_dir() -> Result<Arc<Self>, VideoError> {
        let dll_path = Self::default_exe_dir_dll_path()?;
        Self::load_from_absolute_path(&dll_path)
    }

    /// 从指定可信绝对路径安全加载 `mpv-2.dll`。
    /// 强制校验路径为绝对路径且文件存在，防 DLL 劫持。
    pub fn load_from_absolute_path(path: &Path) -> Result<Arc<Self>, VideoError> {
        if !path.is_absolute() {
            return Err(VideoError::RelativeDllPathRejected(path.to_path_buf()));
        }
        if !path.is_file() {
            return Err(VideoError::LibraryNotFound(path.to_path_buf()));
        }

        #[cfg(windows)]
        {
            Self::load_windows(path)
        }

        #[cfg(not(windows))]
        {
            let _ = path;
            Err(VideoError::Unsupported(
                "当前仅在 Windows 平台实现 mpv-2.dll 安全加载".to_string(),
            ))
        }
    }

    /// 返回 `mpv_client_api_version()` 值（高 16 位为主版本号，低 16 位为次版本号）。
    pub fn client_api_version(&self) -> u64 {
        self.api_version
    }

    /// 返回实际加载的 `mpv-2.dll` 绝对路径。
    pub fn loaded_path(&self) -> &Path {
        &self.loaded_path
    }

    /// 将 libmpv 错误码转换为可读字符串。
    pub fn error_string(&self, code: i32) -> String {
        // SAFETY: `mpv_error_string` 返回指向静态只读 C 字符串的指针。
        let ptr = unsafe { (self.mpv_error_string)(code) };
        if ptr.is_null() {
            return format!("mpv error {code}");
        }
        // SAFETY: libmpv 保证返回以 `\0` 结尾的静态 UTF-8/ASCII 字符串。
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }

    /// 创建 `mpv_render_context`（供 OpenGL 或软件渲染上下文使用）。
    ///
    /// # Safety
    /// 调用方必须保证 `mpv` 为已初始化的有效 `mpv_handle`，`params` 以 `MPV_RENDER_PARAM_INVALID` 结尾，
    /// 且在销毁 `mpv` 之前先调用 [`Self::render_context_free`]。
    pub unsafe fn render_context_create(
        &self,
        res: *mut *mut MpvRenderContext,
        mpv: *mut MpvHandle,
        params: *mut MpvRenderParam,
    ) -> i32 {
        unsafe { (self.mpv_render_context_create)(res, mpv, params) }
    }

    /// 查询渲染上下文是否有待渲染的新帧（返回包含 `MPV_RENDER_UPDATE_FRAME` 的位掩码）。
    ///
    /// # Safety
    /// `ctx` 必须为有效 `mpv_render_context` 指针。
    pub unsafe fn render_context_update(&self, ctx: *mut MpvRenderContext) -> u64 {
        unsafe { (self.mpv_render_context_update)(ctx) }
    }

    /// 执行一帧渲染（OpenGL FBO 或 SW 内存表面）。
    ///
    /// # Safety
    /// `ctx` 必须有效，`params` 以 `MPV_RENDER_PARAM_INVALID` 结尾，OpenGL 模式下当前线程必须绑定同一 GL 上下文。
    pub unsafe fn render_context_render(
        &self,
        ctx: *mut MpvRenderContext,
        params: *mut MpvRenderParam,
    ) -> i32 {
        unsafe { (self.mpv_render_context_render)(ctx, params) }
    }

    /// 注册渲染帧更新通知回调（回调中禁止调用任何 libmpv 函数，仅用于唤醒 UI 重绘）。
    ///
    /// # Safety
    /// `ctx` 必须有效，`callback_ctx` 在回调取消或上下文释放前必须保持有效。
    pub unsafe fn render_context_set_update_callback(
        &self,
        ctx: *mut MpvRenderContext,
        callback: Option<MpvRenderUpdateFn>,
        callback_ctx: *mut std::ffi::c_void,
    ) {
        unsafe { (self.mpv_render_context_set_update_callback)(ctx, callback, callback_ctx) }
    }

    /// 通知 libmpv 已完成一次帧交换（用于帧同步统计）。
    ///
    /// # Safety
    /// `ctx` 必须有效。
    pub unsafe fn render_context_report_swap(&self, ctx: *mut MpvRenderContext) {
        unsafe { (self.mpv_render_context_report_swap)(ctx) }
    }

    /// 释放 `mpv_render_context`（必须在销毁对应 `mpv_handle` 之前调用）。
    ///
    /// # Safety
    /// `ctx` 必须为有效且尚未释放的 `mpv_render_context` 指针。
    pub unsafe fn render_context_free(&self, ctx: *mut MpvRenderContext) {
        unsafe { (self.mpv_render_context_free)(ctx) }
    }

    #[cfg(windows)]
    fn harden_process_dll_search_order() {
        use std::sync::Once;
        use windows_sys::Win32::System::LibraryLoader::{
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories,
        };

        static HARDENED: Once = Once::new();
        HARDENED.call_once(|| {
            // SAFETY: 进程级设置默认安全 DLL 搜索目录（排除 CWD 与用户 PATH）。
            unsafe {
                let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
            }
        });
    }

    #[cfg(windows)]
    fn load_windows(path: &Path) -> Result<Arc<Self>, VideoError> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError};
        use windows_sys::Win32::System::LibraryLoader::{
            GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
            LoadLibraryExW,
        };

        Self::harden_process_dll_search_order();

        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: 传入有效 NUL 结尾 UTF-16 绝对路径，并限定依赖解析目录仅为 DLL 所在目录与 System32。
        let module = unsafe {
            LoadLibraryExW(
                wide_path.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };

        if module.is_null() {
            // SAFETY: 读取紧邻失败的 Win32 错误码。
            let err_code = unsafe { GetLastError() };
            return Err(VideoError::LibraryLoadFailed {
                path: path.to_path_buf(),
                reason: format!("LoadLibraryExW 失败，Win32 错误码={err_code}"),
            });
        }

        // 局部加载辅助闭包：解析失败时确保立即释放句柄不泄漏
        macro_rules! load_sym {
            ($name:literal, $ty:ty) => {{
                let sym_cstr = concat!($name, "\0");
                // SAFETY: `module` 为有效已加载句柄，`sym_cstr` 为 NUL 结尾 ASCII 符号名。
                let proc = unsafe { GetProcAddress(module, sym_cstr.as_ptr()) };
                match proc {
                    Some(f) => unsafe {
                        std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(f)
                    },
                    None => {
                        unsafe {
                            FreeLibrary(module);
                        }
                        return Err(VideoError::MissingSymbol { symbol: $name });
                    }
                }
            }};
        }

        let mpv_client_api_version = load_sym!("mpv_client_api_version", FnMpvClientApiVersion);
        let mpv_error_string = load_sym!("mpv_error_string", FnMpvErrorString);
        let mpv_create = load_sym!("mpv_create", FnMpvCreate);
        let mpv_initialize = load_sym!("mpv_initialize", FnMpvInitialize);
        let mpv_terminate_destroy = load_sym!("mpv_terminate_destroy", FnMpvTerminateDestroy);
        let mpv_set_option_string = load_sym!("mpv_set_option_string", FnMpvSetOptionString);
        let mpv_set_property_string = load_sym!("mpv_set_property_string", FnMpvSetPropertyString);
        let mpv_get_property_string = load_sym!("mpv_get_property_string", FnMpvGetPropertyString);
        let mpv_get_property = load_sym!("mpv_get_property", FnMpvGetProperty);
        let mpv_free = load_sym!("mpv_free", FnMpvFree);
        let mpv_command = load_sym!("mpv_command", FnMpvCommand);
        let mpv_wait_event = load_sym!("mpv_wait_event", FnMpvWaitEvent);
        let mpv_render_context_create =
            load_sym!("mpv_render_context_create", FnMpvRenderContextCreate);
        let mpv_render_context_render =
            load_sym!("mpv_render_context_render", FnMpvRenderContextRender);
        let mpv_render_context_update =
            load_sym!("mpv_render_context_update", FnMpvRenderContextUpdate);
        let mpv_render_context_set_update_callback = load_sym!(
            "mpv_render_context_set_update_callback",
            FnMpvRenderContextSetUpdateCallback
        );
        let mpv_render_context_report_swap = load_sym!(
            "mpv_render_context_report_swap",
            FnMpvRenderContextReportSwap
        );
        let mpv_render_context_free = load_sym!("mpv_render_context_free", FnMpvRenderContextFree);

        // SAFETY: 无参数纯查询函数，返回 libmpv ABI 版本号。
        let api_version = unsafe { (mpv_client_api_version)() } as u64;
        if (api_version >> 16) < 2 {
            unsafe {
                FreeLibrary(module);
            }
            return Err(VideoError::IncompatibleAbiVersion {
                found: api_version,
                required_major: 2,
            });
        }

        Ok(Arc::new(Self {
            module,
            loaded_path: path.to_path_buf(),
            api_version,
            mpv_client_api_version,
            mpv_error_string,
            mpv_create,
            mpv_initialize,
            mpv_terminate_destroy,
            mpv_set_option_string,
            mpv_set_property_string,
            mpv_get_property_string,
            mpv_get_property,
            mpv_free,
            mpv_command,
            mpv_wait_event,
            mpv_render_context_create,
            mpv_render_context_render,
            mpv_render_context_update,
            mpv_render_context_set_update_callback,
            mpv_render_context_report_swap,
            mpv_render_context_free,
        }))
    }
}

/// 查询当前进程地址空间内是否已加载 `mpv-2.dll`（用于验证非视频路径零加载）。
pub fn is_mpv_module_loaded_in_process() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        let wide: Vec<u16> = MPV_DLL_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: 传入有效 NUL 结尾宽字符串，不增加引用计数。
        let handle = unsafe { GetModuleHandleW(wide.as_ptr()) };
        !handle.is_null()
    }

    #[cfg(not(windows))]
    {
        false
    }
}
