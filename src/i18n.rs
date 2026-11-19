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
    editor_begin: "编辑 Ctrl+E", "Edit Ctrl+E";
    editor_save: "保存", "Save";
    editor_save_dirty: "保存 ●", "Save ●";
    editor_save_as: "另存为", "Save as";
    editor_undo: "撤销", "Undo";
    editor_redo: "重做", "Redo";
    editor_find_replace: "查找替换", "Find/Replace";
    editor_autosave: "自动保存", "Auto-save";
    editor_autosave_on: "自动保存 ✓", "Auto-save ✓";
    editor_exit: "退出编辑 Esc", "Exit editing Esc";
    editor_replace: "替换", "Replace";
    editor_replace_all: "全部替换", "Replace all";
    editor_cancel: "取消", "Cancel";
    editor_mixed_eol_note: "（混合换行将统一保存）", " (mixed line endings will be normalized on save)";
    dlg_conflict_mod_title: "文件已被外部修改", "File modified externally";
    dlg_conflict_mod_text: "磁盘上的文件在你编辑期间被其他程序修改。直接保存会覆盖对方的改动。", "The file on disk was changed by another program while you were editing. Saving now will overwrite those changes.";
    dlg_overwrite: "覆盖保存", "Overwrite";
    dlg_conflict_del_title: "文件已被删除或移动", "File deleted or moved";
    dlg_conflict_del_text: "原文件已不存在。可以重新创建并保存，或另存为其他位置。", "The original file no longer exists. Recreate and save it, or save elsewhere.";
    dlg_recreate: "重新创建并保存", "Recreate and save";
    dlg_reload: "重新载入", "Reload";
    dlg_save_as: "另存为…", "Save as…";
    dlg_unrep_title: "存在无法用原编码表示的字符", "Characters cannot be represented in the original encoding";
    dlg_to_utf8: "转为 UTF-8 保存", "Convert to UTF-8 and save";
    dlg_noperm_title: "没有写入权限", "No write permission";
    dlg_noperm_suffix: "。AnyFile 不会提升权限，请另存到有权限的位置。", ". AnyFile will not elevate privileges; please save to a location you can write to.";
    dlg_unsaved_title: "有未保存的修改", "Unsaved changes";
    dlg_unsaved_close: "关闭窗口前", "Before closing the window, ";
    dlg_unsaved_exit: "退出编辑前", "Before leaving edit mode, ";
    dlg_unsaved_continue: "继续之前", "Before continuing, ";
    dlg_unsaved_question: "是否保存对该文件的修改？", "do you want to save your changes to this file?";
    dlg_save: "保存", "Save";
    dlg_discard: "不保存", "Don't save";
    status_save_failed_prefix: "保存失败：", "Save failed: ";
    status_reload_failed_prefix: "重新载入失败：", "Reload failed: ";
    status_ext_modified: "磁盘文件已被外部修改：保存时将询问如何处理", "File changed on disk: you will be asked how to proceed when saving";
    status_ext_deleted: "磁盘文件已被删除或移动：保存时将询问如何处理", "File deleted or moved on disk: you will be asked how to proceed when saving";
    status_autosave_conflict: "自动保存已暂停：磁盘文件发生外部变化", "Auto-save paused: the file changed on disk";
    status_autosave_unrep: "自动保存已暂停：含原编码无法表示的字符，请手动保存", "Auto-save paused: contains characters the original encoding cannot represent; save manually";
    status_autosave_failed_prefix: "自动保存失败：", "Auto-save failed: ";
    find_none: "无匹配", "No matches";
    find_matches_suffix: " 处匹配", " matches";
    replaced_prefix: "已替换 ", "Replaced ";
    replaced_suffix: " 处", " occurrence(s)";
    unrep_message_fmt: "第 {line} 行含有 {enc} 无法表示的字符「{sample}」，是否转为 UTF-8 保存？", "Line {line} contains characters {enc} cannot represent: \"{sample}\". Convert to UTF-8 and save?";
    file_filter_all: "所有文件 (*.*)", "All files (*.*)";
}

/// 中文固定短语 -> 英文；长短语在前（按顺序匹配替换）。
const PHRASES: &[(&str, &str)] = &[
    ("存在未保存的修改", "There are unsaved changes"),
    ("当前不是文本视图", "Not in the text view"),
    ("当前不在编辑态", "Not in edit mode"),
    ("大文件只读保护：已禁用编辑", "Large file is read-only: editing disabled"),
    ("文件含有无法无损解码的字节，已禁用编辑以防数据损坏", "The file contains bytes that cannot be decoded losslessly; editing disabled to prevent data corruption"),
    ("没有写入权限", "No write permission"),
    ("文件为只读属性", "The file is read-only"),
    ("无法写入临时文件", "Cannot write temporary file"),
    ("无法替换目标文件", "Cannot replace target file"),
    ("文件扩展名为 .", "File extension is ."),
    ("，但文件头特征缺失或已损坏", ", but the file header signature is missing or damaged"),
    ("，但文件头无效或内容已损坏，无法安全解码。", ", but the header is invalid or the content is damaged and cannot be decoded safely."),
    ("目标路径是目录而非文件", "The target path is a directory, not a file"),
    ("目标路径是一个文件夹，无法作为单个文件预览。", "The target path is a folder and cannot be previewed as a single file."),
    ("检测到纯二进制或未知格式文件，为避免乱码已展示安全信息卡。", "Binary or unknown format detected; showing a safe info card to avoid garbled text."),
    ("无法读取文件元数据", "Cannot read file metadata"),
    ("无法打开文件（可能缺少权限或被占用）", "Cannot open file (permission missing or file in use)"),
    ("读取文件内容失败", "Failed to read file content"),
    ("文件头缺少合法的 IHDR 块", "header is missing a valid IHDR chunk"),
    ("文件头损坏或数据不完整，无法解码。", "header is damaged or data is incomplete; cannot decode."),
    ("文件头损坏或数据被截断，无法解码。", "header is damaged or data is truncated; cannot decode."),
    ("文件头损坏或被截断，无法解码。", "header is damaged or truncated; cannot decode."),
    ("音频头损坏或不完整，无法播放。", "audio header is damaged or incomplete; cannot play."),
    ("标记段无效或被截断", "marker segment is invalid or truncated"),
    ("逻辑屏幕描述符不完整", "logical screen descriptor is incomplete"),
    ("头损坏或长度不足", "header is damaged or too short"),
    ("容器数据不完整", "container data is incomplete"),
    ("缺少 fmt 块", "is missing the fmt chunk"),
    ("文件头不完整", "header is incomplete"),
    ("（纯音频）", " (audio only)"),
    ("损坏的 ", "Corrupted "),
    ("纯二进制", "Binary"),
    ("不可读", "Unreadable"),
    ("目录", "Directory"),
    ("纯文本", "Plain text"),
    ("日志", "Log"),
    ("混合换行", "Mixed line endings"),
    ("无换行", "No line ending"),
    ("无结果", "No results"),
    ("文本", "text"),
    ("无法读取文件信息", "Cannot read file info"),
    ("无法打开文件", "Cannot open file"),
    ("读取文件失败", "Failed to read file"),
    ("文件不存在", "File does not exist"),
    ("当前未打开任何文件项", "No file is currently open"),
    ("无法手动切换到欢迎空态", "Cannot switch to the welcome screen manually"),
    ("系统回收站操作尚未实现", "Recycle Bin is not supported on this platform"),
    ("在资源管理器中显示尚未实现", "Show in Explorer is not supported on this platform"),
    ("打开方式对话框尚未实现", "The Open With dialog is not supported on this platform"),
    ("默认应用设置跳转尚未实现", "Opening default apps settings is not supported on this platform"),
    ("剪贴板操作尚未实现", "Clipboard is not supported on this platform"),
    ("打开链接尚未实现", "Opening links is not supported on this platform"),
    ("打开失败（错误码 ", "Open failed (error code "),
    ("移入回收站失败（错误码 ", "Move to Recycle Bin failed (error code "),
    ("打开资源管理器失败", "Failed to open Explorer"),
    ("不允许打开该链接", "Opening this link is not allowed"),
    ("无法打开剪贴板", "Cannot open the clipboard"),
    ("剪贴板内存分配失败", "Clipboard memory allocation failed"),
    ("剪贴板内存锁定失败", "Clipboard memory lock failed"),
    ("写入剪贴板失败", "Failed to write to the clipboard"),
    ("调起系统“打开方式”对话框失败", "Failed to show the system Open With dialog"),
    ("无法调用 curl", "Cannot run curl"),
    ("响应解析失败", "Failed to parse response"),
    ("响应缺少 tag_name", "Response is missing tag_name"),
    ("获取配置文件锁超时", "Timed out acquiring the config file lock"),
    ("无法打开音频文件", "Cannot open audio file"),
    ("无法识别音频格式", "Cannot recognize audio format"),
    ("文件中没有可解码的音频轨道", "No decodable audio track in the file"),
    ("不支持的音频编码", "Unsupported audio codec"),
    ("音频缺少采样率信息", "Audio is missing sample rate information"),
    ("读取音频数据失败", "Failed to read audio data"),
    ("解码失败", "Decoding failed"),
    ("跳转失败", "Seek failed"),
    ("未命名音频", "Untitled audio"),
    ("未找到可用的音频输出设备", "No audio output device found"),
    ("无法获取输出配置", "Cannot get output configuration"),
    ("不支持的输出采样格式", "Unsupported output sample format"),
    ("无法创建音频输出流", "Cannot create audio output stream"),
    ("初始化目录监听器失败", "Failed to initialize directory watcher"),
    ("监听目录失败", "Failed to watch directory"),
    ("拒绝从相对路径加载 mpv-2.dll", "Refused to load mpv-2.dll from a relative path"),
    ("未在可信目录找到 mpv-2.dll", "mpv-2.dll not found in a trusted directory"),
    ("缺少导出符号", "is missing export symbol"),
    ("libmpv ABI 版本不兼容：实际", "libmpv ABI version incompatible: found"),
    ("要求主版本 >=", "required major >="),
    ("拒绝不安全的媒体路径", "Refused unsafe media path"),
    ("不支持的视频操作", "Unsupported video operation"),
    ("无法解析当前可执行文件路径", "Cannot resolve the current executable path"),
    ("当前仅在 Windows 平台实现 mpv-2.dll 安全加载", "Secure mpv-2.dll loading is only implemented on Windows"),
    ("等待 mpv 加载媒体文件超时", "Timed out waiting for mpv to load the media file"),
    ("路径包含无效 UTF-8 字符", "Path contains invalid UTF-8"),
    ("媒体路径为空", "Media path is empty"),
    ("媒体路径包含非法 NUL 字节", "Media path contains an illegal NUL byte"),
    ("禁止非本地协议", "Non-local protocol is forbidden"),
    ("禁止 mpv 特殊伪协议前缀", "mpv special pseudo-protocol prefix is forbidden"),
    ("获取原生窗口句柄失败", "Failed to get native window handle"),
    ("应用强制沙箱参数", "Failed to apply sandbox option"),
    ("失败", "failed"),
    ("行", "lines"),
    ("字号", "font size"),
    ("大文件只读保护", "Large file read-only protection"),
    ("仅预览前 2 MB，已禁用编辑", "previewing only the first 2 MB; editing disabled"),
    ("（回退）", " (fallback)"),
    ("：", ": "),
    ("（", " ("),
    ("）", ")"),
    ("，", ", "),
    ("。", "."),
];

/// 将 Rust 侧产生的中文用户可见消息翻译为目标语言；中文原样返回。
pub fn localize(msg: &str, lang: Language) -> String {
    if lang == Language::ZhCn {
        return msg.to_string();
    }
    let mut out = msg.to_string();
    for (zh, en) in PHRASES {
        if out.contains(zh) {
            out = out.replace(zh, en);
        }
    }
    strip_os_localized_text(out)
}

/// 操作系统错误文案按系统语言本地化，英文界面下去掉其中的中文，仅保留 `(os error N)`。
fn strip_os_localized_text(s: String) -> String {
    let is_cjk =
        |c: char| ('\u{3000}'..='\u{9fff}').contains(&c) || ('\u{ff00}'..='\u{ffef}').contains(&c);
    let Some(first) = s.find(is_cjk) else {
        return s;
    };
    let start = s[..first].rfind(": ").map_or(first, |i| i + 2);
    match s.find("(os error") {
        Some(end) if end >= start => format!("{}{}", &s[..start], &s[end..]),
        _ => s[..start].trim_end_matches([' ', ':']).to_string(),
    }
}

/// 文本视图元信息行：编码 · 换行 · N 行 · 语法 · 字号
pub fn text_meta_line(lang: Language, enc: &str, eol: &str, lines: usize, syntax: &str, px: f32) -> String {
    match lang {
        Language::ZhCn => format!("{enc} · {eol} · {lines} 行 · {syntax} · 字号 {px:.0}px"),
        Language::En => format!(
            "{enc} · {} · {lines} lines · {} · font {px:.0}px",
            localize(eol, lang),
            localize(syntax, lang)
        ),
    }
}

/// 大文件只读提示
pub fn large_file_notice(lang: Language, mb: f64) -> String {
    match lang {
        Language::ZhCn => format!("大文件只读保护（{mb:.1} MB）：仅预览前 2 MB，已禁用编辑"),
        Language::En => format!("Large file read-only protection ({mb:.1} MB): previewing only the first 2 MB; editing disabled"),
    }
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
