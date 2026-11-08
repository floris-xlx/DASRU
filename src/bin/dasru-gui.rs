#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    dasru::gui::shutdown_trace::configure(std::env::args_os().skip(1))?;
    let mut app = dasru::gui::DesktopApp::monitored()?;
    let shutdown = app.shutdown_report();
    let native = eframe::run_native(
        "DASRU",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(dasru::gui::settings::Preferences::default().size)
                .with_min_inner_size([
                    dasru::gui::settings::WINDOW_WIDTH,
                    dasru::gui::settings::MIN_HEIGHT,
                ])
                .with_max_inner_size([
                    dasru::gui::settings::WINDOW_WIDTH,
                    dasru::gui::settings::MAX_HEIGHT,
                ]),
            ..Default::default()
        },
        Box::new(move |creation| {
            app.restore_window(creation);
            Ok(Box::new(app))
        }),
    );
    tracing::info!(target: "dasru::shutdown", phase = "EFRAME_RETURNED", success = native.is_ok());
    native?;
    let result = shutdown
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or(Err("APP_SHUTDOWN_NOT_COMPLETED"));
    result.map_err(|code| std::io::Error::other(code).into())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("DASRU desktop requires Windows 10/11 x64");
    std::process::exit(1);
}
