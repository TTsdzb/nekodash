// Slint's generated bindings use internal toolkit invariants. Scope this lint
// exception to generated code; application callbacks live outside this module.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented
)]
mod generated {
    slint::include_modules!();
}
mod app;
use generated::AppWindow;
use slint::ComponentHandle;

pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_with_directory(app::application_data_dir()?)
}

fn run_with_directory(
    directory: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let ui = AppWindow::new()?;
    #[cfg(debug_assertions)]
    if let Ok(size) = std::env::var("NEKODASH_WINDOW_SIZE") {
        let (width, height) = size.split_once('x').ok_or("Expected WIDTHxHEIGHT")?;
        ui.window()
            .set_size(slint::LogicalSize::new(width.parse()?, height.parse()?));
    }
    let application = app::Application::new(&ui, runtime.handle(), directory)?;
    let ui_result = ui.run();
    let shutdown_result = application.shutdown(&runtime);
    drop(application);
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    ui_result?;
    shutdown_result?;
    Ok(())
}

#[cfg(target_os = "android")]
// SAFETY: android-activity resolves this exact, unique Rust-ABI symbol. It calls
// it once with a valid AndroidApp on its application thread. Exporting the name
// is required by that entry-point contract; the body uses only safe Rust APIs.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(android_app: slint::android::AndroidApp) {
    let result = (|| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let directory = android_app
            .internal_data_path()
            .ok_or("Android app data directory is unavailable")?;
        slint::android::init(android_app)?;
        run_with_directory(directory.join("nekodash"))
    })();
    if let Err(error) = result {
        eprintln!("NekoDash: {error}");
    }
}
