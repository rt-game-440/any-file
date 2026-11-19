//! 中英双语：语言偏好解析与全部界面字符串表（默认跟随系统语言，可运行时切换）。

use serde::{Deserialize, Serialize};

/// 解析后的实际界面语言
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    ZhCn,
    En,
}

/// 用户语言偏好
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguagePreference {
    /// 跟随系统语言（默认）
    #[default]
    System,
    /// 简体中文
    Zh,
    /// English
    En,
}

/// 依据系统区域标签（如 `zh-CN`、`en-US`）判定语言：中文系归入简体中文，其余为英文。
pub fn language_from_locale(tag: &str) -> Language {
    let lower = tag.trim().to_ascii_lowercase();
    if lower == "zh" || lower.starts_with("zh-") || lower.starts_with("zh_") {
        Language::ZhCn
    } else {
        Language::En
    }
}

impl LanguagePreference {
    pub fn resolve(self, system_locale: &str) -> Language {
        match self {
            Self::System => language_from_locale(system_locale),
            Self::Zh => Language::ZhCn,
            Self::En => Language::En,
        }
    }

    pub fn to_index(self) -> i32 {
        match self {
            Self::System => 0,
            Self::Zh => 1,
            Self::En => 2,
        }
    }

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Zh,
            2 => Self::En,
            _ => Self::System,
        }
    }
}

macro_rules! ui_strings {
    ($($name:ident : $zh:expr, $en:expr;)*) => {
        /// 一种语言下的全部界面字符串
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct UiStrings {
            $(pub $name: String,)*
        }

        impl UiStrings {
            pub fn for_language(lang: Language) -> Self {
                match lang {
                    Language::ZhCn => Self { $($name: $zh.to_string(),)* },
                    Language::En => Self { $($name: $en.to_string(),)* },
                }
            }

            /// 全部 `(键名, 文本)`，供完整性测试遍历
            pub fn entries(&self) -> Vec<(&'static str, &str)> {
                vec![$((stringify!($name), self.$name.as_str()),)*]
            }
        }
    };
}

ui_strings! {
    welcome_subtitle: "拖放任意文件到此处，或点击下方按钮打开", "Drop any file here, or click the button below to open";
    opened_prefix: "已打开：", "Opened: ";
    unnamed_file: "未命名文件", "Untitled";
    open_file: "打开文件", "Open file";
    prev_item: "← 上一项", "← Previous";
    next_item: "下一项 →", "Next →";
    filter_same_kind: "筛选：仅同类", "Filter: same type";
    filter_all: "筛选：全部格式", "Filter: all formats";
    open_as_text: "以文本打开", "Open as text";
    switch_viewer: "切换查看器", "Switch viewer";
    unsupported_title: "无法直接预览此文件", "This file can't be previewed";
    path_prefix: "路径：", "Path: ";
    size_prefix: "大小：", "Size: ";
    detected_prefix: "探测结果：", "Detected: ";
    current_viewer_prefix: "当前查看器：", "Viewer: ";
    recognized_prefix: "识别格式：", "Format: ";
    open_with_other_app: "用其他应用打开", "Open with another app";
    recent_title: "最近查看", "Recent files";
    recent_none: "最近查看：暂无记录", "Recent: none";
    recent_off: "最近查看：已关闭", "Recent: off";
    hint_siblings: "快捷键：← / → 切换同目录文件 · Delete 删除", "Shortcuts: ← / → browse folder · Delete to trash";
    render_prefix: "渲染：", "Renderer: ";
    reduced_motion_suffix: "（减弱动态效果）", " (reduced motion)";
    audio_prev: "上一首", "Previous";
    audio_play: "播放", "Play";
    audio_pause: "暂停", "Pause";
    audio_next: "下一首", "Next";
    audio_speed_prefix: "倍速 ", "Speed ";
    audio_unmute: "取消静音", "Unmute";
    audio_mute: "静音", "Mute";
    audio_auto_next: "自动播放下一首", "Auto-play next";
    audio_hint: "Space 播放/暂停 · ←/→ 快退/快进 5 秒 · PageUp/PageDown 或 Ctrl+←/→ 切歌 · M 静音", "Space play/pause · ←/→ seek 5 s · PageUp/PageDown or Ctrl+←/→ change track · M mute";
    op_delete: "删除", "Delete";
    op_copy_path: "复制路径", "Copy path";
    op_reveal: "在资源管理器中显示", "Show in Explorer";
    toast_deleted: "已移入回收站", "Moved to Recycle Bin";
    toast_copied: "路径已复制", "Path copied";
    toast_failed_prefix: "操作失败：", "Failed: ";
    settings: "设置", "Settings";
    close: "关闭", "Close";
    theme: "主题", "Theme";
    theme_system: "跟随系统", "System";
    theme_light: "浅色", "Light";
    theme_dark: "深色", "Dark";
    language: "语言", "Language";
    lang_system: "跟随系统", "System";
    lang_zh: "简体中文", "简体中文";
    lang_en: "English", "English";
    auto_save: "自动保存", "Auto save";
    keep_recent: "记录最近查看", "Keep recent files";
    clear_recent: "清空最近记录", "Clear recent files";
    check_update: "检查更新", "Check for updates";
    checking: "正在检查…", "Checking…";
    open_release_page: "前往下载页", "Open download page";
    default_apps: "打开 Windows 默认应用设置", "Open Windows Default apps settings";
    about: "关于", "About";
    version_prefix: "版本 ", "Version ";
    storage_portable: "便携模式", "Portable mode";
    storage_installed: "安装模式", "Installed mode";
    privacy_note: "零遥测、无后台常驻；仅在手动检查更新时联网。", "Zero telemetry, no background process; the network is used only when you check for updates.";
    update_up_to_date: "已是最新版本", "You're up to date";
    update_available_prefix: "发现新版本：", "New version available: ";
    update_failed_prefix: "检查失败：", "Check failed: ";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags_map_to_language() {
        assert_eq!(language_from_locale("zh-CN"), Language::ZhCn);
        assert_eq!(language_from_locale("zh-Hans-CN"), Language::ZhCn);
        assert_eq!(language_from_locale("en-US"), Language::En);
        assert_eq!(language_from_locale("fr-FR"), Language::En);
        assert_eq!(language_from_locale(""), Language::En);
    }

    #[test]
    fn preference_resolution() {
        assert_eq!(LanguagePreference::System.resolve("zh-CN"), Language::ZhCn);
        assert_eq!(LanguagePreference::System.resolve("en-GB"), Language::En);
        assert_eq!(LanguagePreference::En.resolve("zh-CN"), Language::En);
        assert_eq!(LanguagePreference::Zh.resolve("en-US"), Language::ZhCn);
        for p in [LanguagePreference::System, LanguagePreference::Zh, LanguagePreference::En] {
            assert_eq!(LanguagePreference::from_index(p.to_index()), p);
        }
    }

    #[test]
    fn both_languages_are_complete_and_differ() {
        let zh = UiStrings::for_language(Language::ZhCn);
        let en = UiStrings::for_language(Language::En);
        let fixed = ["lang_zh", "lang_en"];
        for ((k, z), (_, e)) in zh.entries().into_iter().zip(en.entries()) {
            assert!(!z.is_empty() && !e.is_empty(), "{k} 为空");
            if !fixed.contains(&k) {
                assert_ne!(z, e, "{k} 中英文相同");
            }
            let has_cjk = |s: &str| s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
            assert!(!has_cjk(e) || fixed.contains(&k), "{k} 英文含中文");
        }
    }
}
