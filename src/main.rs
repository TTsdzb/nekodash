// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::error::Error;

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
use generated::AppWindow;
use slint::ComponentHandle;

fn main() -> Result<(), Box<dyn Error>> {
    let ui = AppWindow::new()?;

    let ui_handle = ui.as_weak();
    ui.on_request_increase_value(move || {
        if let Some(ui) = ui_handle.upgrade() {
            ui.set_counter(ui.get_counter().saturating_add(1));
        }
    });

    ui.run()?;

    Ok(())
}
