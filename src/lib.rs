//! AnyFile 核心库 crate 入口。
//! 导出无头应用控制器 `AppSession`、领域模型、外部边界 Ports、平台实现与 Slint UI 模块。

pub mod domain;
pub mod fixtures;
pub mod platform;
pub mod ports;
pub mod session;
pub mod window_shell;

slint::include_modules!();

/// 渲染后端选择结果（遵循 ADR-0003：Skia GPU 优先，失败自动回退软件渲染）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveRenderer {
    Skia,
    Software,
}

impl ActiveRenderer {
    pub fn display_label(self) -> &'static str {
        match self {
            Self::Skia => "Skia GPU",
            Self::Software => "Software（回退）",
        }
    }
}

/// 按 ADR-0003 优先级初始化 Slint 渲染后端：优先 Skia，失败自动回退 Software。
pub fn init_slint_backend() -> Result<ActiveRenderer, slint::PlatformError> {
    if slint::BackendSelector::new()
        .renderer_name("skia".to_string())
        .select()
        .is_ok()
    {
        return Ok(ActiveRenderer::Skia);
    }

    slint::BackendSelector::new()
        .renderer_name("software".to_string())
        .select()?;
    Ok(ActiveRenderer::Software)
}
