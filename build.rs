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
    let selected = std::env::var("SLINT_STYLE").unwrap_or_else(|_| "fluent".into());
    let style = match selected.as_str() {
        "fluent" => "nekodash-fluent",
        "material" => "nekodash-material",
        other => other,
    };
    let config = slint_build::CompilerConfiguration::new()
        .with_style(style.into())
        .with_include_paths(vec![
            "ui/styles".into(),
            format!("ui/styles/{style}").into(),
        ]);
    slint_build::compile_with_config("ui/app-window.slint", config)?;
    Ok(())
}
