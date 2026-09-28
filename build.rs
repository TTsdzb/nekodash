fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        // Align both LOAD segments and the end of RELRO for 16 KB Android pages.
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,max-page-size=16384");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,common-page-size=16384");
    }
    println!("cargo:rerun-if-env-changed=SLINT_STYLE");
    println!("cargo:rerun-if-changed=assets/icons/app.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/app.ico")
            .set("ProductName", "NekoDash")
            .set("FileDescription", "NekoDash")
            .compile()?;
    }
    slint_build::compile("ui/app-window.slint")?;
    Ok(())
}
