//! Windows 10/11 原生无边框窗口壳层子类化实现：
//! - 保留原生窗口样式与 DWM 圆角/阴影（`WS_THICKFRAME | WS_CAPTION | WS_MAXIMIZEBOX | WS_MINIMIZEBOX`）；
//! - 拦截 `WM_NCCALCSIZE` 去除系统标题栏，并在最大化时按实时 DPI 内缩防裁切；
//! - 拦截 `WM_NCHITTEST` 实现四边缩放、标题栏拖拽（`HTCAPTION` / Aero Snap）与最大化按钮（`HTMAXBUTTON` / Win11 Snap Layouts）；
//! - 完整透传 `WM_IME_*`、`WM_CHAR` 与 `WM_DPICHANGED`，不干扰中文输入法与 Per-Monitor V2 高 DPI 缩放。

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetWindowLongPtrW, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
    IsZoomed, MINMAXINFO, NCCALCSIZE_PARAMS, PostMessageW, SM_CXPADDEDBORDER, SM_CXSIZEFRAME,
    SM_CYSIZEFRAME, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WM_CLOSE, WM_GETMINMAXINFO, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SIZE, WS_CAPTION,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

use super::{
    HitTestInput, HitTestRegion, PhysicalPoint, PhysicalRect, PhysicalSize, TitleBarMetrics,
    WindowSizingConstraints, WorkArea, adjust_maximized_client_rect, compute_adaptive_window_rect,
    hit_test_window,
};

const SUBCLASS_ID: usize = 0x414E_5946; // "ANYF"

/// 从 Win32 窗口子类化层向 Slint UI 同步状态的回调集合
pub struct ShellCallbacks {
    pub on_maximized_changed: Box<dyn Fn(bool) + Send + Sync>,
    pub on_max_button_hover_changed: Box<dyn Fn(bool) + Send + Sync>,
}

impl Default for ShellCallbacks {
    fn default() -> Self {
        Self {
            on_maximized_changed: Box::new(|_| {}),
            on_max_button_hover_changed: Box::new(|_| {}),
        }
    }
}

struct SubclassState {
    title_bar: TitleBarMetrics,
    constraints: WindowSizingConstraints,
    callbacks: ShellCallbacks,
    max_button_hovered: bool,
    max_button_pressed: bool,
    last_maximized: bool,
}

/// 已挂载 Win32 无边框壳层的窗口操作句柄
#[derive(Debug, Clone, Copy)]
pub struct WindowsShellHandle {
    hwnd: isize,
    constraints: WindowSizingConstraints,
}

impl WindowsShellHandle {
    /// 为已显示（拥有原生 `HWND`）的 Slint 窗口挂载 Win32 无边框子类化与 DWM 原生圆角阴影。
    pub fn install(
        window: &slint::Window,
        title_bar: TitleBarMetrics,
        constraints: WindowSizingConstraints,
        dark_mode: bool,
        callbacks: ShellCallbacks,
    ) -> Result<Self, String> {
        let rwh = window.window_handle();
        let handle = rwh
            .window_handle()
            .map_err(|e| format!("获取原生窗口句柄失败: {e}"))?;

        let hwnd = match handle.as_raw() {
            RawWindowHandle::Win32(win32) => win32.hwnd.get() as HWND,
            other => return Err(format!("不支持的原生窗口句柄类型: {other:?}")),
        };

        // 1. 确保具备原生边框、缩放、Aero Snap 与最大化按钮所需的窗口样式
        // SAFETY: hwnd 来自已创建的 Slint Win32 窗口，调用标准 Win32 样式更新 API。
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let desired =
                style | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
            if desired != style {
                SetWindowLongPtrW(hwnd, GWL_STYLE, desired as isize);
            }

            // 扩展 1px 顶部 DWM 框架到客户区，激活原生窗口阴影（不侵占内容绘制）
            let margins = MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 1,
                cyBottomHeight: 0,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        }

        let is_maximized = unsafe { IsZoomed(hwnd) != 0 };
        apply_dwm_corner_preference(hwnd, !is_maximized);
        apply_dwm_dark_mode(hwnd, dark_mode);

        // 2. 挂载 Win32 窗口子类化，在 WM_NCDESTROY 时自动回收状态堆内存
        let state = Box::new(SubclassState {
            title_bar,
            constraints,
            callbacks,
            max_button_hovered: false,
            max_button_pressed: false,
            last_maximized: is_maximized,
        });
        let state_ptr = Box::into_raw(state);

        // SAFETY: state_ptr 归子类化回调所有，并在 WM_NCDESTROY 时配对释放。
        let installed = unsafe {
            SetWindowSubclass(
                hwnd,
                Some(window_shell_subclass_proc),
                SUBCLASS_ID,
                state_ptr as usize,
            )
        };

        if installed == 0 {
            // SAFETY: 挂载失败时立即回收未移交的内存。
            unsafe {
                drop(Box::from_raw(state_ptr));
            }
            return Err("SetWindowSubclass 挂载失败".to_string());
        }

        // 3. 触发 WM_NCCALCSIZE 重新计算客户区并刷新窗口框架
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER,
            );
        }

        Ok(Self {
            hwnd: hwnd as isize,
            constraints,
        })
    }

    fn raw_hwnd(&self) -> HWND {
        self.hwnd as HWND
    }

    /// 返回底层原生 Win32 `HWND` 数值句柄（用于跨模块集成或诊断验证）
    pub fn hwnd_raw(&self) -> isize {
        self.hwnd
    }

    /// 同步 DWM 深浅色边框外观
    pub fn set_dark_mode(&self, dark_mode: bool) {
        apply_dwm_dark_mode(self.raw_hwnd(), dark_mode);
    }

    /// 最小化窗口
    pub fn minimize(&self) {
        unsafe {
            ShowWindow(self.raw_hwnd(), SW_MINIMIZE);
        }
    }

    /// 切换最大化 / 还原状态
    pub fn toggle_maximize(&self) {
        let hwnd = self.raw_hwnd();
        unsafe {
            if IsZoomed(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            } else {
                ShowWindow(hwnd, SW_MAXIMIZE);
            }
        }
    }

    /// 请求关闭窗口（走标准 `WM_CLOSE` 流程，触发 `AppSession::request_close` 检查）
    pub fn close(&self) {
        unsafe {
            PostMessageW(self.raw_hwnd(), WM_CLOSE, 0, 0);
        }
    }

    /// 查询窗口是否处于最大化状态
    pub fn is_maximized(&self) -> bool {
        unsafe { IsZoomed(self.raw_hwnd()) != 0 }
    }

    /// 查询窗口当前所在显示器的工作区矩形与实时 DPI
    pub fn current_work_area(&self) -> Option<WorkArea> {
        let hwnd = self.raw_hwnd();
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            if monitor.is_null() {
                return None;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let dpi = query_window_dpi(hwnd);
            let rc = info.rcWork;
            Some(WorkArea {
                rect: PhysicalRect {
                    x: rc.left,
                    y: rc.top,
                    width: rc.right - rc.left,
                    height: rc.bottom - rc.top,
                },
                dpi,
            })
        }
    }

    /// 按目标内容宽高比自适应当前显示器工作区尺寸并居中显示（若已最大化则不打断最大化态）
    pub fn apply_adaptive_size(&self, content_size: Option<PhysicalSize>) {
        if self.is_maximized() {
            return;
        }
        let Some(work_area) = self.current_work_area() else {
            return;
        };
        let target = compute_adaptive_window_rect(content_size, work_area, self.constraints);
        unsafe {
            SetWindowPos(
                self.raw_hwnd(),
                std::ptr::null_mut(),
                target.x,
                target.y,
                target.width,
                target.height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

fn query_window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 96 } else { dpi }
}

fn apply_dwm_dark_mode(hwnd: HWND, dark_mode: bool) {
    let enabled: i32 = if dark_mode { 1 } else { 0 };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&enabled as *const i32).cast::<c_void>(),
            std::mem::size_of::<i32>() as u32,
        );
    }
}

fn apply_dwm_corner_preference(hwnd: HWND, rounded: bool) {
    let preference = if rounded {
        DWMWCP_ROUND
    } else {
        DWMWCP_DONOTROUND
    };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&preference as *const i32).cast::<c_void>(),
            std::mem::size_of::<i32>() as u32,
        );
    }
}

fn map_hit_test_region(region: HitTestRegion) -> LRESULT {
    match region {
        HitTestRegion::Client => HTCLIENT as LRESULT,
        HitTestRegion::Caption => HTCAPTION as LRESULT,
        HitTestRegion::MaxButton => HTMAXBUTTON as LRESULT,
        HitTestRegion::Top => HTTOP as LRESULT,
        HitTestRegion::Bottom => HTBOTTOM as LRESULT,
        HitTestRegion::Left => HTLEFT as LRESULT,
        HitTestRegion::Right => HTRIGHT as LRESULT,
        HitTestRegion::TopLeft => HTTOPLEFT as LRESULT,
        HitTestRegion::TopRight => HTTOPRIGHT as LRESULT,
        HitTestRegion::BottomLeft => HTBOTTOMLEFT as LRESULT,
        HitTestRegion::BottomRight => HTBOTTOMRIGHT as LRESULT,
    }
}

unsafe extern "system" fn window_shell_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid_subclass: usize,
    dw_ref_data: usize,
) -> LRESULT {
    let state_ptr = dw_ref_data as *mut SubclassState;
    if state_ptr.is_null() {
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }

    match msg {
        WM_NCCALCSIZE if wparam != 0 => {
            // SAFETY: wparam != 0 时 lparam 指向有效的 NCCALCSIZE_PARAMS。
            let params = unsafe { &mut *(lparam as *mut NCCALCSIZE_PARAMS) };
            let is_zoomed = unsafe { IsZoomed(hwnd) != 0 };
            if is_zoomed {
                let dpi = query_window_dpi(hwnd);
                let padded = unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
                let frame_x = unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) } + padded;
                let frame_y = unsafe { GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) } + padded;

                let raw = params.rgrc[0];
                let adjusted = adjust_maximized_client_rect(
                    PhysicalRect {
                        x: raw.left,
                        y: raw.top,
                        width: raw.right - raw.left,
                        height: raw.bottom - raw.top,
                    },
                    frame_x,
                    frame_y,
                );
                params.rgrc[0].left = adjusted.x;
                params.rgrc[0].top = adjusted.y;
                params.rgrc[0].right = adjusted.x + adjusted.width;
                params.rgrc[0].bottom = adjusted.y + adjusted.height;
            }
            0
        }

        WM_NCHITTEST => {
            // 必须做有符号 i16 转换以支持多显示器负坐标
            let screen_x = (lparam & 0xFFFF) as i16 as i32;
            let screen_y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            let mut win_rect: RECT = unsafe { std::mem::zeroed() };
            if unsafe { GetWindowRect(hwnd, &mut win_rect) } == 0 {
                return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            }

            let state = unsafe { &*state_ptr };
            let dpi = query_window_dpi(hwnd);
            let is_maximized = unsafe { IsZoomed(hwnd) != 0 };

            let region = hit_test_window(&HitTestInput {
                window_size: PhysicalSize {
                    width: win_rect.right - win_rect.left,
                    height: win_rect.bottom - win_rect.top,
                },
                cursor: PhysicalPoint {
                    x: screen_x - win_rect.left,
                    y: screen_y - win_rect.top,
                },
                dpi,
                is_maximized,
                title_bar: state.title_bar,
            });

            map_hit_test_region(region)
        }

        WM_NCMOUSEMOVE => {
            let state = unsafe { &mut *state_ptr };
            let hovering_max = wparam == HTMAXBUTTON as WPARAM;
            if hovering_max != state.max_button_hovered {
                state.max_button_hovered = hovering_max;
                (state.callbacks.on_max_button_hover_changed)(hovering_max);
                if hovering_max {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        TrackMouseEvent(&mut tme);
                    }
                }
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        WM_NCMOUSELEAVE => {
            let state = unsafe { &mut *state_ptr };
            if state.max_button_hovered {
                state.max_button_hovered = false;
                (state.callbacks.on_max_button_hover_changed)(false);
            }
            state.max_button_pressed = false;
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        WM_NCLBUTTONDOWN if wparam == HTMAXBUTTON as WPARAM => {
            // 消费最大化按钮的非客户区按下消息，防止系统绘制传统 Win32 按钮覆盖自定义标题栏，
            // 同时完整保留 Windows 11 悬停 HTMAXBUTTON 时的原生 Snap Layouts 贴靠菜单。
            let state = unsafe { &mut *state_ptr };
            state.max_button_pressed = true;
            0
        }

        WM_NCLBUTTONUP if wparam == HTMAXBUTTON as WPARAM => {
            let state = unsafe { &mut *state_ptr };
            let was_pressed = state.max_button_pressed;
            state.max_button_pressed = false;
            if was_pressed {
                unsafe {
                    if IsZoomed(hwnd) != 0 {
                        ShowWindow(hwnd, SW_RESTORE);
                    } else {
                        ShowWindow(hwnd, SW_MAXIMIZE);
                    }
                }
                return 0;
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        WM_GETMINMAXINFO => {
            let state = unsafe { &*state_ptr };
            let dpi = query_window_dpi(hwnd);
            let mmi = unsafe { &mut *(lparam as *mut MINMAXINFO) };
            mmi.ptMinTrackSize.x = state.constraints.min_width_physical(dpi);
            mmi.ptMinTrackSize.y = state.constraints.min_height_physical(dpi);
            0
        }

        WM_SIZE => {
            let state = unsafe { &mut *state_ptr };
            let is_maximized = unsafe { IsZoomed(hwnd) != 0 };
            if is_maximized != state.last_maximized {
                state.last_maximized = is_maximized;
                apply_dwm_corner_preference(hwnd, !is_maximized);
                (state.callbacks.on_maximized_changed)(is_maximized);
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        WM_NCDESTROY => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            unsafe {
                RemoveWindowSubclass(hwnd, Some(window_shell_subclass_proc), SUBCLASS_ID);
                drop(Box::from_raw(state_ptr));
            }
            result
        }

        // 所有其他消息（含 WM_IME_*、WM_CHAR、WM_DPICHANGED）原样透传给 winit 原窗口过程
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}
