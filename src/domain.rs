//! AnyFile 核心领域模型，术语严格遵循根目录 `GLOSSARY.md`。

use std::path::PathBuf;

/// 媒体大类（Media Category）：决定启用哪个查看器视图及同目录切换的默认筛选范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaCategory {
    /// 图像（PNG/JPG/WebP/GIF/BMP/ICO/TIFF/SVG/AVIF/APNG 等）
    Image,
    /// 视频（MP4/MKV/WebM/MOV/AVI/WMV/FLV 等）
    Video,
    /// 音频（MP3/WAV/FLAC/OGG/M4A/AAC/OPUS 等）
    Audio,
    /// 纯文本与代码（TXT/MD/JSON/YAML/TOML/源码等）
    Text,
}

/// 文件项（File Entry）：磁盘上当前打开或位于同目录序列中的单个文件对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// 文件在磁盘上的完整路径
    pub path: PathBuf,
    /// 探测出的媒体大类；无法归类时为 `None`（走兜底信息卡）
    pub media_category: Option<MediaCategory>,
}

impl FileEntry {
    pub fn new(path: PathBuf, media_category: Option<MediaCategory>) -> Self {
        Self {
            path,
            media_category,
        }
    }
}

/// 查看器视图（Viewer）：负责呈现特定媒体大类或空态/兜底态的专用视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewerKind {
    /// 无参数启动时的极简欢迎/拖放空态
    Welcome,
    /// 图像视图
    Image,
    /// 视频视图
    Video,
    /// 音频视图
    Audio,
    /// 文本视图
    Text,
    /// 无法识别或损坏文件的兜底信息卡
    Unsupported,
}

impl From<Option<MediaCategory>> for ViewerKind {
    fn from(category: Option<MediaCategory>) -> Self {
        match category {
            Some(MediaCategory::Image) => ViewerKind::Image,
            Some(MediaCategory::Video) => ViewerKind::Video,
            Some(MediaCategory::Audio) => ViewerKind::Audio,
            Some(MediaCategory::Text) => ViewerKind::Text,
            None => ViewerKind::Unsupported,
        }
    }
}

/// 同目录序列（Sibling Sequence）：与当前文件项位于同一目录下的可浏览文件有序列表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiblingSequence {
    /// 按自然排序排列的同目录文件项列表
    pub entries: Vec<FileEntry>,
    /// 当前激活项在序列中的索引
    pub current_index: Option<usize>,
    /// 是否仅筛选当前媒体大类（默认 true，可一键切换为全部支持格式）
    pub filter_by_category: bool,
}

impl SiblingSequence {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            current_index: None,
            filter_by_category: true,
        }
    }
}

/// 文本视图双模式（Viewing Mode / Editing Mode，遵循 ADR-0004）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextMode {
    /// 查看态：默认只读状态，提供语法高亮、行号与查找
    #[default]
    Viewing,
    /// 编辑态：纯文本编辑状态，支持显式保存或可选自动保存
    Editing,
}

/// 动效配置（Motion Profile，遵循 ADR-0003）。
/// 驱动界面过渡与直接操作反馈的物理弹簧参数；系统开启“减弱动态效果”时自动降级为瞬时切换。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionProfile {
    /// 弹簧刚度（stiffness）
    pub stiffness: f32,
    /// 弹簧阻尼（damping）
    pub damping: f32,
    /// 质量（mass）
    pub mass: f32,
    /// 是否尊重系统“减弱动态效果”并降级为瞬时切换
    pub reduced_motion: bool,
}

impl MotionProfile {
    /// 默认流体物理弹簧配置（对标 iOS 优雅跟手感）
    pub fn fluid() -> Self {
        Self {
            stiffness: 340.0,
            damping: 30.0,
            mass: 1.0,
            reduced_motion: false,
        }
    }

    /// 根据系统“减弱动态效果”标志生成动效配置
    pub fn for_system_setting(reduced_motion: bool) -> Self {
        Self {
            reduced_motion,
            ..Self::fluid()
        }
    }

    /// 动效时长倍率：开启减弱动态效果时为 `0.0`（瞬时切换），否则为 `1.0`
    pub fn duration_scale(&self) -> f32 {
        if self.reduced_motion { 0.0 } else { 1.0 }
    }
}

/// 用户主题偏好设置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreference {
    /// 跟随 Windows 深浅色系统设置（默认）
    #[default]
    System,
    /// 强制浅色
    Light,
    /// 强制深色
    Dark,
}

/// 解析后的实际界面主题
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

impl ResolvedTheme {
    pub fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

/// 关闭窗口请求的决策结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// 无未保存状态，允许立即退出进程（ADR-0002）
    AllowExit,
    /// 存在未保存修改，需要提示用户确认
    ConfirmUnsavedChanges,
}
