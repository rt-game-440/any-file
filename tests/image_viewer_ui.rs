//! 图像视图界面离屏截图冒烟：用 Slint 软件渲染器把 MainWindow 画到内存缓冲，检查图像像素与沉浸式淡出。

use std::rc::Rc;

use anyfile::{ImageState, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn render(win: &Rc<MinimalSoftwareWindow>, w: u32, h: u32) -> SharedPixelBuffer<Rgb8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(w, h);
    win.request_redraw();
    win.draw_if_needed(|renderer| {
        let stride = w as usize;
        renderer.render(buf.make_mut_slice(), stride);
    });
    buf
}

fn px(buf: &SharedPixelBuffer<Rgb8Pixel>, x: u32, y: u32) -> [u8; 3] {
    let p = buf.as_slice()[(y * buf.width() + x) as usize];
    [p.r, p.g, p.b]
}

#[test]
fn image_panel_renders_pixels_and_fades_chrome() {
    let sw = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(sw.clone()))).unwrap();
    sw.set_size(PhysicalSize::new(800, 520));

    let ui = MainWindow::new().unwrap();
    ui.set_active_viewer("image".into());
    ui.set_dark_mode(true);
    ui.set_reduced_motion(true);
    let g = ui.global::<ImageState>();
    // 4x2 的纯色块：左半红、右半绿，放大显示于窗口中央
    let mut raw = SharedPixelBuffer::<slint::Rgba8Pixel>::new(2, 1);
    {
        let s = raw.make_mut_slice();
        s[0] = slint::Rgba8Pixel { r: 255, g: 0, b: 0, a: 255 };
        s[1] = slint::Rgba8Pixel { r: 0, g: 255, b: 0, a: 255 };
    }
    g.set_full(slint::Image::from_rgba8(raw));
    g.set_full_visible(true);
    g.set_img_x(100.0);
    g.set_img_y(100.0);
    g.set_img_w(600.0);
    g.set_img_h(300.0);
    g.set_pixelated(true);
    g.set_chrome_visible(true);
    ui.show().unwrap();

    let shot = render(&sw, 800, 520);
    assert_eq!(px(&shot, 200, 250), [255, 0, 0], "左半应为红色");
    assert_eq!(px(&shot, 600, 250), [0, 255, 0], "右半应为绿色");
    // 工具栏可见：底部居中区域不再是画布背景色
    let bg_with_chrome = px(&shot, 400, 520 - 44 - 22);
    assert_ne!(bg_with_chrome, [14, 15, 18]);

    // 静止淡出：工具栏与标题栏隐藏（减弱动态下立即生效）
    g.set_chrome_visible(false);
    let shot = render(&sw, 800, 520);
    assert_eq!(px(&shot, 400, 520 - 44 - 22), [14, 15, 18], "工具栏应淡出为画布背景");
    assert_eq!(px(&shot, 400, 10), [14, 15, 18], "标题栏应淡出");
    assert_eq!(px(&shot, 200, 250), [255, 0, 0]);

    // 平移到右侧后，原左半位置变为背景
    g.set_img_x(400.0);
    let shot = render(&sw, 800, 520);
    assert_eq!(px(&shot, 200, 250), [14, 15, 18]);
    assert_eq!(px(&shot, 500, 250), [255, 0, 0]);
}
