//! 窗口壳层（Window Shell）外部行为测试：
//! 覆盖工作区自适应尺寸计算、无边框自定义标题栏命中测试（含 Win11 HTMAXBUTTON 与多 DPI）及最大化内缩校正。

use anyfile::window_shell::{
    HitTestInput, HitTestRegion, PhysicalPoint, PhysicalRect, PhysicalSize, TitleBarMetrics,
    WindowSizingConstraints, WorkArea, adjust_maximized_client_rect, compute_adaptive_window_rect,
    hit_test_window,
};

fn work_area_1080p() -> WorkArea {
    WorkArea {
        rect: PhysicalRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040, // 扣除 40px 任务栏
        },
        dpi: 96,
    }
}

#[test]
fn large_image_scales_proportionally_inside_work_area_and_centers() {
    let work_area = work_area_1080p();
    let constraints = WindowSizingConstraints::default();
    // 4000x3000 (4:3) 大图不应撑出 1920x1040 工作区，且应保留一定工作区留白并居中
    let rect = compute_adaptive_window_rect(
        Some(PhysicalSize {
            width: 4000,
            height: 3000,
        }),
        work_area,
        constraints,
    );

    assert!(rect.width <= work_area.rect.width, "窗口宽度超出工作区: {rect:?}");
    assert!(rect.height <= work_area.rect.height, "窗口高度超出工作区: {rect:?}");
    assert!(rect.x >= work_area.rect.x, "窗口左边界超出工作区: {rect:?}");
    assert!(rect.y >= work_area.rect.y, "窗口上边界超出工作区: {rect:?}");
    assert!(
        rect.x + rect.width <= work_area.rect.x + work_area.rect.width,
        "窗口右边界超出工作区: {rect:?}"
    );
    assert!(
        rect.y + rect.height <= work_area.rect.y + work_area.rect.height,
        "窗口下边界超出工作区: {rect:?}"
    );

    // 扣除标题栏后，内容区域应近似保持 4:3 宽高比（误差 <= 2%）
    let content_h = (rect.height - constraints.title_bar_height_physical(work_area.dpi)) as f64;
    let ratio = rect.width as f64 / content_h;
    assert!(
        (ratio - (4.0 / 3.0)).abs() < 0.03,
        "内容区宽高比偏离 4:3: actual={ratio:.4}, rect={rect:?}"
    );

    // 应基本居中于工作区
    let expected_center_x = work_area.rect.x + work_area.rect.width / 2;
    let actual_center_x = rect.x + rect.width / 2;
    assert!((actual_center_x - expected_center_x).abs() <= 2);
}

#[test]
fn small_content_is_not_stretched_to_fill_screen_and_respects_minimum_size() {
    let work_area = work_area_1080p();
    let constraints = WindowSizingConstraints::default();
    // 320x240 小图不应拉伸占满屏幕，但窗口应满足最小尺寸约束
    let rect = compute_adaptive_window_rect(
        Some(PhysicalSize {
            width: 320,
            height: 240,
        }),
        work_area,
        constraints,
    );

    assert!(rect.width >= constraints.min_width_physical(work_area.dpi));
    assert!(rect.height >= constraints.min_height_physical(work_area.dpi));
    assert!(
        rect.width <= 700 && rect.height <= 600,
        "小图不应被过度拉伸: {rect:?}"
    );
}

#[test]
fn secondary_monitor_with_negative_origin_and_high_dpi_stays_inside_work_area() {
    // 左侧副屏：起点 (-2560, 120)，150% DPI (144)
    let work_area = WorkArea {
        rect: PhysicalRect {
            x: -2560,
            y: 120,
            width: 2560,
            height: 1380,
        },
        dpi: 144,
    };
    let constraints = WindowSizingConstraints::default();
    let rect = compute_adaptive_window_rect(
        Some(PhysicalSize {
            width: 3840,
            height: 2160,
        }),
        work_area,
        constraints,
    );

    assert!(rect.x >= work_area.rect.x);
    assert!(rect.y >= work_area.rect.y);
    assert!(rect.x + rect.width <= work_area.rect.x + work_area.rect.width);
    assert!(rect.y + rect.height <= work_area.rect.y + work_area.rect.height);
    // 150% DPI 下的最小尺寸换算也应生效
    assert!(rect.width >= constraints.min_width_physical(144));
    assert!(rect.height >= constraints.min_height_physical(144));
}

#[test]
fn empty_welcome_state_uses_comfortable_default_size_inside_small_screen() {
    // 模拟 1280x720 150% DPI 紧凑屏幕
    let work_area = WorkArea {
        rect: PhysicalRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 660,
        },
        dpi: 144,
    };
    let constraints = WindowSizingConstraints::default();
    let rect = compute_adaptive_window_rect(None, work_area, constraints);

    assert!(rect.width <= work_area.rect.width);
    assert!(rect.height <= work_area.rect.height);
    assert!(rect.x >= 0 && rect.y >= 0);
}

#[test]
fn hit_test_detects_resize_borders_and_corners_when_not_maximized() {
    let input_base = HitTestInput {
        window_size: PhysicalSize {
            width: 1000,
            height: 700,
        },
        cursor: PhysicalPoint { x: 0, y: 0 },
        dpi: 96,
        is_maximized: false,
        title_bar: TitleBarMetrics::default(),
    };

    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 2, y: 2 },
            ..input_base
        }),
        HitTestRegion::TopLeft
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 998, y: 2 },
            ..input_base
        }),
        HitTestRegion::TopRight
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 2, y: 698 },
            ..input_base
        }),
        HitTestRegion::BottomLeft
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 998, y: 698 },
            ..input_base
        }),
        HitTestRegion::BottomRight
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 500, y: 1 },
            ..input_base
        }),
        HitTestRegion::Top
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 500, y: 698 },
            ..input_base
        }),
        HitTestRegion::Bottom
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 1, y: 350 },
            ..input_base
        }),
        HitTestRegion::Left
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 998, y: 350 },
            ..input_base
        }),
        HitTestRegion::Right
    );
}

#[test]
fn hit_test_returns_max_button_for_win11_snap_layouts_and_caption_for_drag_area() {
    let metrics = TitleBarMetrics {
        height_logical: 36,
        button_width_logical: 46,
        resize_border_logical: 6,
    };
    let input_base = HitTestInput {
        window_size: PhysicalSize {
            width: 1000,
            height: 700,
        },
        cursor: PhysicalPoint { x: 0, y: 0 },
        dpi: 96,
        is_maximized: false,
        title_bar: metrics,
    };

    // 右侧按钮自右向左依次为：关闭 [954..1000)、最大化 [908..954)、最小化 [862..908)
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 970, y: 18 },
            ..input_base
        }),
        HitTestRegion::Client,
        "关闭按钮由 Slint 自行响应"
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 930, y: 18 },
            ..input_base
        }),
        HitTestRegion::MaxButton,
        "最大化按钮必须返回 HTMAXBUTTON 以触发 Win11 Snap Layouts"
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 880, y: 18 },
            ..input_base
        }),
        HitTestRegion::Client,
        "最小化按钮由 Slint 自行响应"
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 400, y: 18 },
            ..input_base
        }),
        HitTestRegion::Caption,
        "标题栏空白区域必须返回 HTCAPTION 以支持原生拖拽、双击最大化与 Aero Snap"
    );
    assert_eq!(
        hit_test_window(&HitTestInput {
            cursor: PhysicalPoint { x: 400, y: 100 },
            ..input_base
        }),
        HitTestRegion::Client,
        "标题栏下方内容区域返回 HTCLIENT"
    );
}

#[test]
fn hit_test_scales_accurately_at_150_percent_dpi_and_disables_resize_when_maximized() {
    let metrics = TitleBarMetrics {
        height_logical: 36,       // 150% DPI -> 54px
        button_width_logical: 46, // 150% DPI -> 69px
        resize_border_logical: 6, // 150% DPI -> 9px
    };

    // 150% DPI (144) 下：关闭 [1431..1500)，最大化 [1362..1431)，标题栏高度 54px
    let max_btn_hit = hit_test_window(&HitTestInput {
        window_size: PhysicalSize {
            width: 1500,
            height: 900,
        },
        cursor: PhysicalPoint { x: 1390, y: 30 },
        dpi: 144,
        is_maximized: false,
        title_bar: metrics,
    });
    assert_eq!(max_btn_hit, HitTestRegion::MaxButton);

    // 标题栏在 150% DPI 下 y=48 仍属于标题栏（超过 96 DPI 时的 36px，但小于 54px）
    let caption_hit = hit_test_window(&HitTestInput {
        window_size: PhysicalSize {
            width: 1500,
            height: 900,
        },
        cursor: PhysicalPoint { x: 500, y: 48 },
        dpi: 144,
        is_maximized: false,
        title_bar: metrics,
    });
    assert_eq!(caption_hit, HitTestRegion::Caption);

    // 最大化状态下，边缘 y=0 处不再是 Top 缩放边框，而是直接命中 Caption / MaxButton
    let maximized_top_hit = hit_test_window(&HitTestInput {
        window_size: PhysicalSize {
            width: 1500,
            height: 900,
        },
        cursor: PhysicalPoint { x: 500, y: 0 },
        dpi: 144,
        is_maximized: true,
        title_bar: metrics,
    });
    assert_eq!(maximized_top_hit, HitTestRegion::Caption);

    let maximized_max_btn_top_hit = hit_test_window(&HitTestInput {
        window_size: PhysicalSize {
            width: 1500,
            height: 900,
        },
        cursor: PhysicalPoint { x: 1390, y: 0 },
        dpi: 144,
        is_maximized: true,
        title_bar: metrics,
    });
    assert_eq!(maximized_max_btn_top_hit, HitTestRegion::MaxButton);
}

#[test]
fn maximized_client_rect_insets_window_frame_so_title_bar_is_not_clipped() {
    // Windows 最大化时窗口外扩 8px 到 (-8, -8, 1936, 1056)
    let raw_maximized = PhysicalRect {
        x: -8,
        y: -8,
        width: 1936,
        height: 1056,
    };
    let adjusted = adjust_maximized_client_rect(raw_maximized, 8, 8);
    assert_eq!(
        adjusted,
        PhysicalRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        }
    );
}
