//! 安装脚本静态检查（工单 #13）：不运行安装包、不写注册表。

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn iss() -> String {
    fs::read_to_string(root().join("installer").join("anyfile.iss")).expect("读取 anyfile.iss")
}

fn iss_list(src: &str, name: &str) -> Vec<String> {
    let prefix = format!("#define {name} \"");
    let line = src
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("缺少 {name}"));
    line[prefix.len()..line.len() - 1]
        .split(';')
        .map(str::to_string)
        .collect()
}

#[test]
fn installer_declares_per_user_default_four_categories_and_default_apps_jump() {
    let s = iss();
    for needle in [
        "PrivilegesRequired=lowest",
        "PrivilegesRequiredOverridesAllowed=dialog",
        "ChangesAssociations=yes",
        "ms-settings:defaultapps",
        "Name: \"assoc_image\"",
        "Name: \"assoc_video\"",
        "Name: \"assoc_audio\"",
        "Name: \"assoc_text\"",
        "Software\\RegisteredApplications",
        "Software\\AnyFile\\Capabilities",
    ] {
        assert!(s.contains(needle), "anyfile.iss 缺少 {needle}");
    }
    assert!(!s.contains("Root: HKCR") && !s.contains("Root: HKLM") && !s.contains("Root: HKCU"));
}

#[test]
fn installer_extension_lists_match_probing_rs() {
    let probing = fs::read_to_string(root().join("src").join("probing.rs"))
        .unwrap()
        .replace("\r\n", "\n");
    let start = probing.find("pub fn classify_by_extension").unwrap();
    let body = &probing[start..probing[start..].find("\n}\n").unwrap() + start];
    let s = iss();
    // 逐类对照：probing.rs 中每个 match 分支的扩展名集合
    let arms: Vec<&str> = body.split("=> Some(MediaCategory::").skip(0).collect();
    let cats = ["Image", "Video", "Audio", "Text"];
    let defs = ["ExtImage", "ExtVideo", "ExtAudio", "ExtText"];
    let mut prev_end = body.find("match ext.as_str()").unwrap();
    for (i, cat) in cats.iter().enumerate() {
        let marker = format!("=> Some(MediaCategory::{cat})");
        let idx = body.find(&marker).or_else(|| {
            // rustfmt 可能把 Some(...) 换行
            body.find(&format!("Some(MediaCategory::{cat})"))
        });
        let end = idx.unwrap_or_else(|| panic!("probing.rs 缺少 {cat}")) ;
        let mut exts: Vec<String> = body[prev_end..end]
            .split('"')
            .enumerate()
            .filter(|(n, _)| n % 2 == 1)
            .map(|(_, t)| t.to_string())
            .collect();
        exts.sort();
        let mut want = iss_list(&s, defs[i]);
        want.sort();
        assert_eq!(exts, want, "{cat} 扩展名清单与 probing.rs 不一致");
        prev_end = end + marker.len();
    }
    let _ = arms;
}

#[test]
fn installer_registry_symmetry_script_passes_when_iscc_available() {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root().join("scripts").join("verify_installer.ps1"))
        .output()
        .expect("启动 powershell");
    match out.status.code() {
        Some(0) => {}
        Some(2) => eprintln!("未找到 ISCC，跳过编译与对称性检查"),
        c => panic!(
            "verify_installer.ps1 失败 {c:?}\n{}",
            String::from_utf8_lossy(&out.stdout)
        ),
    }
}
