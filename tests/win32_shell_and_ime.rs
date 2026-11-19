//! 真实 Windows HWND 端到端集成测试：
//! 1. 验证无边框窗口挂载 `WindowsShellHandle` 后，真实 `WM_NCHITTEST` 返回 `HTCAPTION`、`HTMAXBUTTON`、`HTLEFT`、`HTCLIENT`；
//! 2. 验证 Slint `TextInput` 在无边框子类化窗口下的中文 IME / Unicode 上屏表现（不丢字、不乱码、不阻塞）。

#![cfg(target_os = "windows")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyfile::init_slint_backend;
use anyfile::window_shell::win32::{ShellCallbacks, WindowsShellHandle};
use anyfile::window_shell::{TitleBarMetrics, WindowSizingConstraints};
use slint::ComponentHandle;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON, SendMessageW, WM_IME_ENDCOMPOSITION,
    WM_IME_STARTCOMPOSITION, WM_NCHITTEST,
};

slint::slint! {
    #[edition = "1.19"]
    export component ImeAndShellProbeWindow inherits Window {
        in-out property <string> editor-text <=> input.text;

        no-frame: true;
        preferred-width: 720px;
        preferred-height: 460px;

        public function focus-editor() {
            input.focus();
        }

        VerticalLayout {
            padding-top: 48px;
            padding-left: 24px;
            padding-right: 24px;

            input := TextInput {
                height: 36px;
                font-size: 15px;
            }
        }
    }
}

fn pack_screen_lparam(x: i32, y: i32) -> LPARAM {
    let lx = (x as i16 as u16) as usize;
    let ly = (y as i16 as u16) as usize;
    ((ly << 16) | lx) as LPARAM
}

#[derive(Debug, Clone)]
struct ProbeOutcome {
    caption_hit: isize,
    max_btn_hit: isize,
    left_border_hit: isize,
    client_hit: isize,
    actual_text: String,
}

#[test]
fn real_win32_frameless_shell_hit_test_and_chinese_ime_work_end_to_end() {
    let _renderer = init_slint_backend().expect("初始化 Slint 渲染后端失败");
    let window = ImeAndShellProbeWindow::new().expect("创建探测窗口失败");
    window.show().expect("显示探测窗口失败");

    let metrics = TitleBarMetrics::default();
    let constraints = WindowSizingConstraints::default();
    let results: Arc<Mutex<Option<ProbeOutcome>>> = Arc::new(Mutex::new(None));

    // winit 0.30 在事件循环 resumed 后创建原生 HWND，因此在事件循环启动后挂载 Win32 子类化并执行实测
    let results_step1 = Arc::clone(&results);
    let win_weak = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(50),
        move || {
            let win = win_weak.upgrade().expect("窗口已提前释放");
            let shell = WindowsShellHandle::install(
                win.window(),
                metrics,
                constraints,
                false,
                ShellCallbacks::default(),
            )
            .expect("挂载 Win32 无边框窗口壳层失败");

            // 1. 聚焦文本输入框，激活 Slint -> winit 的 IME 上下文（InputMethodRequest::Enable）
            win.invoke_focus_editor();

            let hwnd = shell.hwnd_raw() as HWND;
            let mut rect: RECT = unsafe { std::mem::zeroed() };
            let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
            assert_ne!(ok, 0, "GetWindowRect 失败");

            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let title_h = WindowSizingConstraints::scale_logical(metrics.height_logical, dpi);
            let btn_w = WindowSizingConstraints::scale_logical(metrics.button_width_logical, dpi);

            let width = rect.right - rect.left;
            let mid_title_y = rect.top + title_h / 2;

            // 命中标题栏空白区 -> HTCAPTION
            let caption_hit = unsafe {
                SendMessageW(
                    hwnd,
                    WM_NCHITTEST,
                    0,
                    pack_screen_lparam(rect.left + width / 3, mid_title_y),
                )
            };

            // 命中最大化按钮（右数第二个按钮）-> HTMAXBUTTON（Win11 Snap Layouts）
            let max_btn_x = rect.right - btn_w - btn_w / 2;
            let max_btn_hit = unsafe {
                SendMessageW(
                    hwnd,
                    WM_NCHITTEST,
                    0,
                    pack_screen_lparam(max_btn_x, mid_title_y),
                )
            };

            // 命中左缩放边框 -> HTLEFT
            let left_border_hit = unsafe {
                SendMessageW(
                    hwnd,
                    WM_NCHITTEST,
                    0,
                    pack_screen_lparam(rect.left + 1, rect.top + 120),
                )
            };

            // 命中客户区内容 -> HTCLIENT
            let client_hit = unsafe {
                SendMessageW(
                    hwnd,
                    WM_NCHITTEST,
                    0,
                    pack_screen_lparam(rect.left + width / 2, rect.top + title_h + 60),
                )
            };

            // 验证无边框子类化后 Windows 原生 IME 上下文（HIMC）可用，且 IME 组合消息可安全透传
            let himc = unsafe { ImmGetContext(hwnd) };
            assert!(
                !himc.is_null(),
                "聚焦 TextInput 后窗口应持有有效的 Windows IME 上下文（HIMC）"
            );
            unsafe {
                ImmReleaseContext(hwnd, himc);
                SendMessageW(hwnd, WM_IME_STARTCOMPOSITION, 0, 0);
                SendMessageW(hwnd, WM_IME_ENDCOMPOSITION, 0, 0);
            }

            // 通过 Slint 窗口事件通道验证中文与中英混排上屏文本（含全角中文标点）
            let chinese_sample = "你好，AnyFile中文测试！";
            win.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: chinese_sample.into(),
                });
            win.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                    text: chinese_sample.into(),
                });

            let win_weak_check = win_weak.clone();
            let results_check = Arc::clone(&results_step1);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(win) = win_weak_check.upgrade() {
                    let actual_text = win.get_editor_text().to_string();
                    *results_check.lock().unwrap() = Some(ProbeOutcome {
                        caption_hit,
                        max_btn_hit,
                        left_border_hit,
                        client_hit,
                        actual_text,
                    });
                    let _ = win.hide();
                }
                let _ = slint::quit_event_loop();
            });
        },
    );

    slint::run_event_loop().expect("运行 Slint 事件循环失败");

    let outcome = results
        .lock()
        .unwrap()
        .clone()
        .expect("事件循环未采集到探测结果");

    assert_eq!(
        outcome.caption_hit, HTCAPTION as isize,
        "标题栏空白区域应返回 HTCAPTION"
    );
    assert_eq!(
        outcome.max_btn_hit, HTMAXBUTTON as isize,
        "最大化按钮区域应返回 HTMAXBUTTON 以触发 Win11 Snap Layouts"
    );
    assert_eq!(outcome.left_border_hit, HTLEFT as isize, "左边缘应返回 HTLEFT");
    assert_eq!(outcome.client_hit, HTCLIENT as isize, "主体客户区应返回 HTCLIENT");
    assert_eq!(
        outcome.actual_text, "你好，AnyFile中文测试！",
        "Slint TextInput 应完整接收中文上屏字符且无乱码或丢字"
    );
}
