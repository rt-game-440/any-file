//! Windows 10/11 系统壳层真实实现：读取系统深浅色主题与“减弱动态效果”设置。

use std::ffi::c_void;
use std::path::Path;
use std::process::Command;

use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};

use crate::domain::ResolvedTheme;
use crate::ports::SystemShell;

/// Windows 平台真实 `SystemShell` 实现
#[derive(Debug, Default)]
pub struct WindowsSystemShell;

impl WindowsSystemShell {
    /// 将 Rust 字符串编码为以 `\0` 结尾的 UTF-16 宽字符串
    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 读取 `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`
    /// 返回 `Some(true)` 表示浅色，`Some(false)` 表示深色。
    fn query_apps_use_light_theme() -> Option<bool> {
        let subkey =
            Self::to_wide_null(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value_name = Self::to_wide_null("AppsUseLightTheme");

        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: 传入有效的以 `\0` 结尾的宽字符串指针及有效输出指针。
        let open_status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if open_status != 0 {
            return None;
        }

        let mut data: u32 = 1;
        let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
        let mut value_type: u32 = 0;

        // SAFETY: hkey 已成功打开，输出缓冲区大小为 4 字节匹配 REG_DWORD。
        let query_status = unsafe {
            RegQueryValueExW(
                hkey,
                value_name.as_ptr(),
                std::ptr::null_mut(),
                &mut value_type,
                (&mut data as *mut u32).cast::<u8>(),
                &mut data_size,
            )
        };

        // SAFETY: 释放注册表句柄。
        unsafe {
            RegCloseKey(hkey);
        }

        if query_status == 0 && value_type == REG_DWORD {
            Some(data != 0)
        } else {
            None
        }
    }

    /// 查询 `SPI_GETCLIENTAREAANIMATION`（Windows“在 Windows 中显示动画”设置）。
    /// 当动画关闭（返回 0）时，表示系统开启了“减弱动态效果”。
    fn query_client_area_animation_enabled() -> Option<bool> {
        let mut enabled: i32 = 1;
        // SAFETY: SPI_GETCLIENTAREAANIMATION 期望指向 BOOL (i32) 的指针。
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut enabled as *mut i32).cast::<c_void>(),
                0,
            )
        };
        if ok != 0 {
            Some(enabled != 0)
        } else {
            None
        }
    }
}

impl SystemShell for WindowsSystemShell {
    fn system_theme(&self) -> ResolvedTheme {
        match Self::query_apps_use_light_theme() {
            Some(false) => ResolvedTheme::Dark,
            Some(true) | None => ResolvedTheme::Light,
        }
    }

    fn prefers_reduced_motion(&self) -> bool {
        match Self::query_client_area_animation_enabled() {
            Some(false) => true,
            Some(true) | None => false,
        }
    }

    fn open_with_dialog(&self, path: &Path) -> Result<(), String> {
        let arg = format!("shell32.dll,OpenAs_RunDLL {}", path.display());
        Command::new("rundll32.exe")
            .arg(arg)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("调起系统“打开方式”对话框失败：{e}"))
    }
}
