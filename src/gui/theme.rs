use super::model::Connection;
use egui::Color32;
pub const BACKGROUND: Color32 = Color32::from_rgb(35, 35, 35);
pub const SURFACE: Color32 = Color32::from_rgb(45, 45, 45);
pub const RAISED: Color32 = Color32::from_rgb(56, 56, 56);
pub const INPUT: Color32 = Color32::from_rgb(70, 70, 70);
pub const TEXT: Color32 = Color32::from_rgb(230, 230, 230);
pub const SECONDARY: Color32 = Color32::from_rgb(168, 168, 168);
pub const ACCENT: Color32 = Color32::from_rgb(190, 34, 34);
pub const SUCCESS: Color32 = Color32::from_rgb(93, 173, 120);
pub const WARNING: Color32 = Color32::from_rgb(214, 163, 76);
pub const ERROR: Color32 = Color32::from_rgb(210, 93, 93);
pub fn connection_color(connection: Connection) -> Color32 {
    match connection {
        Connection::Verified => SUCCESS,
        Connection::Error | Connection::AccessDenied => ERROR,
        Connection::UnknownBuild
        | Connection::Ambiguous
        | Connection::Stale
        | Connection::ReverificationRequired
        | Connection::Unsupported => WARNING,
        _ => SECONDARY,
    }
}
pub fn apply(ctx: &egui::Context, dark: bool) {
    let selected = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(selected);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = SURFACE;
        visuals.extreme_bg_color = INPUT;
        visuals.faint_bg_color = SURFACE;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = ACCENT;
        visuals.widgets.inactive.bg_fill = RAISED;
    }
    visuals.selection.bg_fill = ACCENT;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(3);
    }
    visuals.window_corner_radius = egui::CornerRadius::same(3);
    ctx.set_visuals_of(selected, visuals);
}
pub fn value(ui: &mut egui::Ui, label: &str, value: impl ToString) {
    ui.label(label);
    let mut value = value.to_string();
    ui.add_enabled(
        false,
        egui::TextEdit::singleline(&mut value)
            .desired_width(f32::INFINITY)
            .min_size(egui::vec2(180.0, 0.0)),
    );
    ui.end_row();
}
pub fn skeleton(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width.min(ui.available_width()).max(24.0), 24.0),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 3, ui.visuals().widgets.inactive.bg_fill);
}
pub fn connection(ui: &mut egui::Ui, connection: Connection) {
    ui.label("Connection");
    let mut text = format!("{connection:?}");
    ui.add_enabled(
        false,
        egui::TextEdit::singleline(&mut text)
            .text_color(connection_color(connection))
            .min_size(egui::vec2(180.0, 0.0))
            .desired_width(f32::INFINITY),
    );
    ui.end_row();
}
