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

#[test]
fn all_screens_render_at_supported_sizes_and_scaling() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for screen in [
            dasru::gui::Screen::Dashboard,
            dasru::gui::Screen::Hotkeys,
            dasru::gui::Screen::Diagnostics,
            dasru::gui::Screen::Settings,
        ] {
            let mut app = dasru::gui::DesktopApp::default();
            app.select(screen);
            let context = egui::Context::default();
            context.set_pixels_per_point(scale);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(860.0, 560.0),
                )),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| app.show(ui));
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty());
            assert_eq!(app.screen(), screen);
        }
    }
}

#[test]
fn unsupported_and_verified_states_use_distinct_colors() {
    use dasru::gui::{model::Connection, theme::connection_color};
    assert_ne!(
        connection_color(Connection::Verified),
        connection_color(Connection::Unsupported)
    );
}

#[test]
fn dark_theme_tokens_apply_even_when_windows_prefers_light() {
    let mut app = dasru::gui::DesktopApp::default();
    let context = egui::Context::default();
    context.set_theme(egui::Theme::Light);
    let mut color = None;
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.show(ui);
        color = Some(ui.visuals().selection.bg_fill);
    });
    output.textures_delta.clear();
    assert_eq!(color, Some(dasru::gui::theme::ACCENT));
}
