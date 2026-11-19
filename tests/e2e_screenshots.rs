//! 深浅色界面离屏截图检查（工单 #13）：用 Slint 软件渲染器把 MainWindow 画到内存缓冲，
//! 对欢迎页 / 文本视图 / 兜底信息卡 / 图像视图分别渲染深色与浅色，保存 PNG 到 `logs/screenshots/`，
//! 并自动断言：画面非空白、浅色亮度明显高于深色（主题确实生效）、两种主题下均有足够的内容像素差异。
//! 注意：软件渲染器平台只能在进程内设置一次，因此整个检查放在单个测试函数中。

use std::path::PathBuf;
use std::rc::Rc;

use anyfile::{ImageState, MainWindow, TextRow, TextSegment};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, SharedPixelBuffer, VecModel};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const W: u32 = 900;
const H: u32 = 600;

fn render(win: &Rc<MinimalSoftwareWindow>) -> SharedPixelBuffer<Rgb8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(W, H);
    win.request_redraw();
    win.draw_if_needed(|renderer| {
        renderer.render(buf.make_mut_slice(), W as usize);
    });
    buf
}

fn mean_luma(buf: &SharedPixelBuffer<Rgb8Pixel>) -> f64 {
    let s = buf.as_slice();
    let sum: f64 = s
        .iter()
        .map(|p| 0.2126 * p.r as f64 + 0.7152 * p.g as f64 + 0.0722 * p.b as f64)
        .sum();
    sum / s.len() as f64
}

fn distinct_colors(buf: &SharedPixelBuffer<Rgb8Pixel>) -> usize {
    let mut set = std::collections::HashSet::new();
    for p in buf.as_slice() {
        set.insert((p.r, p.g, p.b));
    }
    set.len()
}

fn shot_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("logs")
        .join("screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save_png(buf: &SharedPixelBuffer<Rgb8Pixel>, name: &str) {
    let raw: Vec<u8> = buf
        .as_slice()
        .iter()
        .flat_map(|p| [p.r, p.g, p.b])
        .collect();
    let img = image::RgbImage::from_raw(buf.width(), buf.height(), raw).unwrap();
    img.save(shot_dir().join(name)).unwrap();
}

fn text_rows() -> ModelRc<TextRow> {
    let lines = [
        "// AnyFile 文本视图截图样例",
        "fn main() {",
        "    println!(\"Hello, AnyFile\");",
        "}",
    ];
    let rows: Vec<TextRow> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| TextRow {
            number: i as i32 + 1,
            segments: ModelRc::new(VecModel::from(vec![TextSegment {
                text: (*l).into(),
                kind: 0,
                highlight: 0,
            }])),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

#[test]
fn light_and_dark_screens_render_and_differ() {
    let sw = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(sw.clone()))).unwrap();
    sw.set_size(PhysicalSize::new(W, H));

    let ui = MainWindow::new().unwrap();
    ui.set_reduced_motion(true);
    ui.show().unwrap();

    // 图像视图所需状态（2x1 红/绿色块）
    {
        let mut raw = SharedPixelBuffer::<slint::Rgba8Pixel>::new(2, 1);
        let s = raw.make_mut_slice();
        s[0] = slint::Rgba8Pixel { r: 255, g: 0, b: 0, a: 255 };
        s[1] = slint::Rgba8Pixel { r: 0, g: 255, b: 0, a: 255 };
        let g = ui.global::<ImageState>();
        g.set_full(slint::Image::from_rgba8(raw));
        g.set_full_visible(true);
        g.set_img_x(150.0);
        g.set_img_y(150.0);
        g.set_img_w(600.0);
        g.set_img_h(300.0);
        g.set_pixelated(true);
        g.set_chrome_visible(true);
    }

    // (名称, active-viewer, 是否要求浅色亮度显著高于深色)
    let screens = [
        ("welcome", "welcome", true),
        ("text", "text", true),
        ("fallback", "unsupported", true),
        // 图像视图使用固定深色画布，仅要求非空白
        ("image", "image", false),
    ];

    let mut report = String::new();
    for (name, viewer, theme_sensitive) in screens {
        ui.set_active_viewer(viewer.into());
        if viewer == "text" {
            ui.set_current_file_name("sample.rs".into());
            ui.set_text_rows(text_rows());
            ui.set_text_meta("UTF-8 · LF · 4 行".into());
        }
        if viewer == "unsupported" {
            ui.set_current_file_name("opaque.bin".into());
            ui.set_fallback_size_text("10 B".into());
            ui.set_fallback_reason("无法识别的二进制内容".into());
        }

        ui.set_dark_mode(false);
        let light = render(&sw);
        ui.set_dark_mode(true);
        let dark = render(&sw);
        save_png(&light, &format!("{name}_light.png"));
        save_png(&dark, &format!("{name}_dark.png"));

        let (ll, dl) = (mean_luma(&light), mean_luma(&dark));
        let (lc, dc) = (distinct_colors(&light), distinct_colors(&dark));
        report.push_str(&format!(
            "{name}: light_luma={ll:.1} dark_luma={dl:.1} light_colors={lc} dark_colors={dc}\n"
        ));
        assert!(lc >= 3 && dc >= 3, "{name}: 画面疑似空白（颜色数 {lc}/{dc}）");
        if theme_sensitive {
            assert!(
                ll > dl + 40.0,
                "{name}: 浅色亮度({ll:.1})应明显高于深色({dl:.1})，主题可能未生效"
            );
        }
    }
    std::fs::write(shot_dir().join("report.txt"), report).unwrap();
}
