fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_bundled_translations("lang")
            .with_default_translation_context(slint_build::DefaultTranslationContext::None),
    )
    .unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/z-overlay.ico")
            .set("ProductName", "z-overlay")
            .set("FileDescription", "z-overlay")
            .set("CompanyName", "zoosewu")
            .set("LegalCopyright", "Copyright (c) 2026 zoosewu. MIT License.")
            .set("OriginalFilename", "z-overlay.exe")
            .set("InternalName", "z-overlay")
            .compile()
            .unwrap();
    }
    let manifest =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed=assets/z-overlay.ico");
    println!("cargo:rustc-link-arg-bin=z-overlay=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=z-overlay=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-bin=z-overlay=/MANIFESTUAC:level='asInvoker' uiAccess='false'");
}
