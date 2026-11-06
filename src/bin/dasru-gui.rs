#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> eframe::Result {
    eframe::run_native(
        "DASRU",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1000.0, 680.0])
                .with_min_inner_size([860.0, 560.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(dasru::gui::DesktopApp::default()))),
    )
}

#[cfg(not(windows))]
fn main() {
    eprintln!("DASRU desktop requires Windows 10/11 x64");
    std::process::exit(1);
}
