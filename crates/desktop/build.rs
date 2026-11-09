fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Missing manifest directory");
        let icon = std::path::Path::new(&manifest).join("../../assets/nolae.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        winresource::WindowsResource::new()
            .set_icon_with_id(icon.to_str().expect("Invalid icon path"), "1")
            .set("ProductName", "Nolae Companion")
            .set("FileDescription", "Nolae Companion")
            .set("OriginalFilename", "nolae-companion-ui.exe")
            .compile()
            .expect("Could not embed Nolae's Windows icon and metadata");
    }
}
