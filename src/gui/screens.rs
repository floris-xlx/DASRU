use super::{model::Model, theme};
use crate::capability::FeatureId;
use std::time::Instant;
pub fn dashboard(
    ui: &mut egui::Ui,
    model: &Model,
    listener: &crate::hotkeys::runtime::RuntimeSnapshot,
) {
    ui.heading("DOOM: THE DARK AGES");
    ui.add_space(16.0);
    let connection = model.connection(Instant::now());
    egui::Grid::new("game_status")
        .num_columns(2)
        .spacing([24.0, 12.0])
        .show(ui, |ui| {
            theme::connection(ui, connection);
            theme::value(ui, "Hotkey listener", format!("{:?}", listener.state));
            theme::value(
                ui,
                "Process inspection",
                model
                    .observation
                    .as_ref()
                    .map(|o| format!("{:?}", o.process))
                    .unwrap_or_else(|| "Not evaluated".into()),
            );
            theme::value(
                ui,
                "Registered build",
                model
                    .observation
                    .as_ref()
                    .and_then(|o| o.build.as_deref())
                    .unwrap_or("Not recognized"),
            );
            theme::value(
                ui,
                "Last successful update",
                model
                    .updated
                    .map(|at| format!("{} seconds ago", at.elapsed().as_secs()))
                    .unwrap_or_else(|| "Never".into()),
            );
            theme::value(ui, "Session", "No game modification session");
        });
    if let Some(error) = model.last_error {
        ui.colored_label(theme::ERROR, error);
    }
    ui.add_space(24.0);
    ui.heading("CAPABILITIES");
    capabilities(ui, model, listener);
}
pub fn capabilities(
    ui: &mut egui::Ui,
    model: &Model,
    listener: &crate::hotkeys::runtime::RuntimeSnapshot,
) {
    let mut evidence = model
        .observation
        .as_ref()
        .map(|o| o.evidence)
        .unwrap_or_default();
    if matches!(
        model.connection(Instant::now()),
        super::model::Connection::Error
            | super::model::Connection::Stale
            | super::model::Connection::Stopped
    ) {
        evidence = Default::default();
    }
    evidence.hotkey_registration = listener.state.registration_evidence();
    egui::Grid::new("capabilities")
        .num_columns(2)
        .spacing([24.0, 8.0])
        .show(ui, |ui| {
            for feature in [
                FeatureId::ProcessProbe,
                FeatureId::ExecutableInspection,
                FeatureId::MemoryScan,
                FeatureId::MetricsDiscovery,
                FeatureId::ExperimentalFpsMarker,
                FeatureId::GlobalHotkeys,
                FeatureId::KeyboardInput,
                FeatureId::Clipboard,
            ] {
                let decision = crate::capability::evaluate(feature, &evidence);
                theme::value(
                    ui,
                    &format!("{feature:?}"),
                    format!(
                        "{} | {:?}",
                        decision.implementation_status(),
                        decision.reason
                    ),
                );
            }
        });
}
pub fn diagnostics(
    ui: &mut egui::Ui,
    model: &Model,
    listener: &crate::hotkeys::runtime::RuntimeSnapshot,
) {
    dashboard(ui, model, listener);
    ui.add_space(16.0);
    egui::Grid::new("listener_diagnostics")
        .num_columns(2)
        .show(ui, |ui| {
            theme::value(ui, "Accepted observations", listener.accepted);
            theme::value(ui, "Rejected observations", listener.rejected);
            theme::value(
                ui,
                "Last listener error",
                listener.last_error.map(|e| e.as_str()).unwrap_or("None"),
            );
        });
    if let Some(observation) = &model.observation {
        for error in &observation.errors {
            ui.colored_label(theme::ERROR, *error);
        }
    }
}
