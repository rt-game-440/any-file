use std::fs;
use std::path::PathBuf;

use anyfile::platform::PlatformSystemShell;
use anyfile::ports::SystemShell;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn license_is_complete_gplv3_and_matches_cargo_metadata() {
    let license_text =
        fs::read_to_string(repo_root().join("LICENSE")).expect("读取根目录 LICENSE 失败");

    assert!(
        license_text.contains("GNU GENERAL PUBLIC LICENSE"),
        "LICENSE 必须包含 GPLv3 标题"
    );
    assert!(
        license_text.contains("Version 3, 29 June 2007"),
        "LICENSE 必须是 GPL Version 3"
    );
    assert!(
        license_text.contains("END OF TERMS AND CONDITIONS"),
        "LICENSE 必须包含完整的条款正文"
    );
    assert!(
        license_text.len() > 30_000,
        "LICENSE 文本过短，疑似不完整: {} 字节",
        license_text.len()
    );

    let cargo_toml =
        fs::read_to_string(repo_root().join("Cargo.toml")).expect("读取 Cargo.toml 失败");
    assert!(
        cargo_toml.contains(r#"license = "GPL-3.0-or-later""#),
        "Cargo.toml 必须声明 license = \"GPL-3.0-or-later\""
    );
}

#[test]
fn windows_manifest_declares_per_monitor_v2_and_long_path_aware() {
    let manifest_text = fs::read_to_string(repo_root().join("assets").join("anyfile.manifest"))
        .expect("读取 assets/anyfile.manifest 失败");

    assert!(
        manifest_text.contains("PerMonitorV2"),
        "Windows 应用清单必须声明 PerMonitorV2 高 DPI 感知"
    );
    assert!(
        manifest_text.contains("<longPathAware") && manifest_text.contains(">true</longPathAware>"),
        "Windows 应用清单必须声明 longPathAware = true"
    );
}

#[test]
fn platform_system_shell_queries_os_preferences_without_panicking() {
    let shell = PlatformSystemShell;
    let _theme = shell.system_theme();
    let _reduced = shell.prefers_reduced_motion();
}
