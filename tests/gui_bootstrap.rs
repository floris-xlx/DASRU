#![cfg(feature = "desktop-ui")]
#[test]
fn application_initializes_without_game() {
    let app = dasru::gui::DesktopApp::default();
    assert_eq!(app.screen(), dasru::gui::Screen::Dashboard);
}

#[test]
fn navigation_survives_frames_at_minimum_size() {
    let mut app = dasru::gui::DesktopApp::default();
    app.select(dasru::gui::Screen::Diagnostics);
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| app.show(ui));
    output.textures_delta.clear();
    assert_eq!(app.screen(), dasru::gui::Screen::Diagnostics);
}
