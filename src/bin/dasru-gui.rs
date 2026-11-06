#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> eframe::Result {
    eframe::run_native(
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
        Box::new(|creation| {
            let mut app = dasru::gui::DesktopApp::monitored()?;
            app.restore_window(creation);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(not(windows))]
fn main() {
    eprintln!("DASRU desktop requires Windows 10/11 x64");
    std::process::exit(1);
}
