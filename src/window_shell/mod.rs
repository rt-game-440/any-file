//! 窗口壳层（Window Shell）：
//! - 跨平台纯几何引擎：按内容宽高比自适应屏幕工作区、自定义标题栏与多 DPI 命中测试、最大化内缩校正。
//! - Windows 原生子类化钩子（`win32` 子模块）：无边框沉浸式标题栏、Win11 贴靠布局（`HTMAXBUTTON`）、Aero Snap 与 DWM 圆角阴影。

#[cfg(target_os = "windows")]
pub mod win32;

/// 物理像素坐标点
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

/// 物理像素尺寸
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalSize {
    pub width: i32,
    pub height: i32,
}

/// 物理像素矩形（左上角起点 + 宽高）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PhysicalRect {
    /// 判断点是否落在矩形内（左闭右开、上闭下开）
    pub fn contains(self, point: PhysicalPoint) -> bool {
        point.x >= self.x
            && point.x < self.x.saturating_add(self.width)
            && point.y >= self.y
            && point.y < self.y.saturating_add(self.height)
    }
}

/// 显示器工作区（已扣除任务栏）与当前 DPI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkArea {
    pub rect: PhysicalRect,
    /// 显示器 DPI（96 = 100%, 120 = 125%, 144 = 150%, 192 = 200%）
    pub dpi: u32,
}

/// 窗口自适应尺寸约束（逻辑像素基准，运行时按 DPI 换算）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSizingConstraints {
    pub min_width_logical: i32,
    pub min_height_logical: i32,
    pub default_width_logical: i32,
    pub default_height_logical: i32,
    pub title_bar_height_logical: i32,
    /// 自适应大图时允许占用的工作区最大比例（保留舒适留白，不贴满屏幕）
    pub max_work_area_ratio: f64,
}

impl Default for WindowSizingConstraints {
    fn default() -> Self {
        Self {
            min_width_logical: 480,
            min_height_logical: 360,
            default_width_logical: 760,
            default_height_logical: 520,
            title_bar_height_logical: 36,
            max_work_area_ratio: 0.86,
        }
    }
}

impl WindowSizingConstraints {
    /// 将逻辑像素按窗口 DPI 换算为物理像素
    pub fn scale_logical(logical: i32, dpi: u32) -> i32 {
        let scale = dpi_scale_factor(dpi);
        (logical as f64 * scale).round() as i32
    }

    pub fn min_width_physical(self, dpi: u32) -> i32 {
        Self::scale_logical(self.min_width_logical, dpi)
    }

    pub fn min_height_physical(self, dpi: u32) -> i32 {
        Self::scale_logical(self.min_height_logical, dpi)
    }

    pub fn title_bar_height_physical(self, dpi: u32) -> i32 {
        Self::scale_logical(self.title_bar_height_logical, dpi)
    }
}

/// 将 Windows DPI 值转换为缩放倍率（96 DPI -> 1.0）
pub fn dpi_scale_factor(dpi: u32) -> f64 {
    (dpi.max(48) as f64) / 96.0
}

/// 根据目标内容尺寸、当前显示器工作区与 DPI 约束，计算居中且不越界的初始窗口矩形。
///
/// - `content_size == None` 时采用欢迎空态的舒适默认尺寸；
/// - 小内容不会被拉伸满屏，但保证不小于最小窗口尺寸；
/// - 大内容按原始宽高比等比缩放，确保整体窗口严格落在工作区边界内。
pub fn compute_adaptive_window_rect(
    content_size: Option<PhysicalSize>,
    work_area: WorkArea,
    constraints: WindowSizingConstraints,
) -> PhysicalRect {
    let wa = work_area.rect;
    let wa_w = wa.width.max(1);
    let wa_h = wa.height.max(1);

    let min_w = constraints.min_width_physical(work_area.dpi).min(wa_w).max(1);
    let min_h = constraints.min_height_physical(work_area.dpi).min(wa_h).max(1);
    let title_bar_h = constraints
        .title_bar_height_physical(work_area.dpi)
        .min(wa_h / 3)
        .max(0);

    let ratio = constraints.max_work_area_ratio.clamp(0.5, 1.0);
    let max_w = ((wa_w as f64) * ratio).round() as i32;
    let max_h = ((wa_h as f64) * ratio).round() as i32;
    let max_w = max_w.clamp(min_w, wa_w);
    let max_h = max_h.clamp(min_h, wa_h);

    let (win_w, win_h) = match content_size {
        Some(size) if size.width > 0 && size.height > 0 => {
            let max_content_w = max_w.max(1) as f64;
            let max_content_h = (max_h - title_bar_h).max(1) as f64;
            let scale = (max_content_w / size.width as f64)
                .min(max_content_h / size.height as f64)
                .min(1.0);

            let content_w = (size.width as f64 * scale).round() as i32;
            let content_h = (size.height as f64 * scale).round() as i32;

            (
                content_w.clamp(min_w, wa_w),
                (content_h + title_bar_h).clamp(min_h, wa_h),
            )
        }
        _ => {
            let default_w = WindowSizingConstraints::scale_logical(
                constraints.default_width_logical,
                work_area.dpi,
            );
            let default_h = WindowSizingConstraints::scale_logical(
                constraints.default_height_logical,
                work_area.dpi,
            );
            (default_w.clamp(min_w, max_w), default_h.clamp(min_h, max_h))
        }
    };

    let x = wa.x + (wa_w - win_w) / 2;
    let y = wa.y + (wa_h - win_h) / 2;

    PhysicalRect {
        x: x.clamp(wa.x, wa.x + wa_w - win_w),
        y: y.clamp(wa.y, wa.y + wa_h - win_h),
        width: win_w,
        height: win_h,
    }
}

/// 自定义标题栏的逻辑像素度量（与 `ui/app.slint` 中标题栏及窗口控制按钮尺寸保持一致）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleBarMetrics {
    /// 标题栏高度（逻辑像素，默认 36px）
    pub height_logical: i32,
    /// 单个窗口控制按钮（最小化 / 最大化 / 关闭）宽度（逻辑像素，默认 46px）
    pub button_width_logical: i32,
    /// 未最大化时的缩放边框宽度（逻辑像素，默认 6px）
    pub resize_border_logical: i32,
}

impl Default for TitleBarMetrics {
    fn default() -> Self {
        Self {
            height_logical: 36,
            button_width_logical: 46,
            resize_border_logical: 6,
        }
    }
}

/// 命中测试输入参数（窗口相对物理坐标）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitTestInput {
    /// 窗口当前物理尺寸
    pub window_size: PhysicalSize,
    /// 光标相对于窗口左上角的物理坐标
    pub cursor: PhysicalPoint,
    /// 当前窗口所在显示器的 DPI（由 `GetDpiForWindow` 实时获取）
    pub dpi: u32,
    /// 当前窗口是否处于最大化状态
    pub is_maximized: bool,
    /// 标题栏逻辑度量
    pub title_bar: TitleBarMetrics,
}

/// 跨平台命中测试语义结果（在 Win32 子类化层映射为 `HT*` 常量）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTestRegion {
    Client,
    Caption,
    MaxButton,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// 无边框自定义标题栏命中测试纯几何实现：
/// 1. 未最大化时优先判定四边/四角缩放边框（按当前 DPI 换算）；
/// 2. 在标题栏高度内，右侧三按钮依次为最小化（`Client`）、最大化（`MaxButton` -> Win11 Snap Layouts）、关闭（`Client`）；
/// 3. 标题栏其余区域返回 `Caption`（原生拖拽移动、双击最大化与 Aero Snap）；
/// 4. 其余客户区返回 `Client`。
pub fn hit_test_window(input: &HitTestInput) -> HitTestRegion {
    let w = input.window_size.width;
    let h = input.window_size.height;
    let x = input.cursor.x;
    let y = input.cursor.y;

    if w <= 0 || h <= 0 {
        return HitTestRegion::Client;
    }

    let border = WindowSizingConstraints::scale_logical(input.title_bar.resize_border_logical, input.dpi).max(4);
    let title_bar_h = WindowSizingConstraints::scale_logical(input.title_bar.height_logical, input.dpi).max(1);
    let btn_w = WindowSizingConstraints::scale_logical(input.title_bar.button_width_logical, input.dpi).max(1);

    // 1. 非最大化状态下判定四边与四角缩放边框
    if !input.is_maximized {
        let on_left = x < border;
        let on_right = x >= w - border;
        let on_top = y < border;
        let on_bottom = y >= h - border;

        match (on_left, on_right, on_top, on_bottom) {
            (true, _, true, _) => return HitTestRegion::TopLeft,
            (_, true, true, _) => return HitTestRegion::TopRight,
            (true, _, _, true) => return HitTestRegion::BottomLeft,
            (_, true, _, true) => return HitTestRegion::BottomRight,
            (_, _, true, _) => return HitTestRegion::Top,
            (_, _, _, true) => return HitTestRegion::Bottom,
            (true, _, _, _) => return HitTestRegion::Left,
            (_, true, _, _) => return HitTestRegion::Right,
            _ => {}
        }
    }

    // 2. 标题栏区域（右侧依次为：最小化 | 最大化 | 关闭）
    if y >= 0 && y < title_bar_h {
        let close_left = w - btn_w;
        let max_left = w - btn_w * 2;
        let min_left = w - btn_w * 3;

        if x >= close_left {
            return HitTestRegion::Client;
        }
        if x >= max_left {
            return HitTestRegion::MaxButton;
        }
        if x >= min_left {
            return HitTestRegion::Client;
        }
        return HitTestRegion::Caption;
    }

    HitTestRegion::Client
}

/// 最大化时校正 `WM_NCCALCSIZE` 客户区矩形：
/// Windows 最大化时会把无边框窗口外扩 `frame_x`/`frame_y` 像素，必须向内收缩以避免标题栏与内容被屏幕裁切。
pub fn adjust_maximized_client_rect(
    raw_rect: PhysicalRect,
    frame_inset_x: i32,
    frame_inset_y: i32,
) -> PhysicalRect {
    let inset_x = frame_inset_x.max(0);
    let inset_y = frame_inset_y.max(0);
    PhysicalRect {
        x: raw_rect.x + inset_x,
        y: raw_rect.y + inset_y,
        width: (raw_rect.width - inset_x * 2).max(1),
        height: (raw_rect.height - inset_y * 2).max(1),
    }
}
