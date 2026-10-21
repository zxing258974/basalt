use std::path::Path;

fn main() {
    let mut attributes = tauri_build::Attributes::new();
    // Windows(MSVC):tauri-build 默认只把 Common Controls v6 的清单嵌进 exe,`cargo test` 生成的测试程序没有,
    // 一启动就因为找不到 TaskDialogIndirect 退出(0xc0000139)。改成链接时给所有产物嵌同一份清单。
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo 会设置 CARGO_MANIFEST_DIR");
        let manifest = Path::new(&dir).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("tauri-build 失败:{error:#}");
    }
}
