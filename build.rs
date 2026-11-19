//! AnyFile 构建脚本：编译 Slint UI 并在 Windows 平台嵌入应用清单（PerMonitorV2 + longPathAware）。

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=ui/app.slint");
    slint_build::compile("ui/app.slint").expect("编译 ui/app.slint 失败");

    embed_windows_manifest();
}

#[cfg(target_os = "windows")]
fn embed_windows_manifest() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("缺少 CARGO_MANIFEST_DIR 环境变量"),
    );
    let manifest_path = manifest_dir.join("assets").join("anyfile.manifest");
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    // 通过 MSVC 链接器直接嵌入应用清单，不依赖外部 rc.exe，确保最终 exe 包含 PerMonitorV2 与 longPathAware。
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "msvc" {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn embed_windows_manifest() {}
