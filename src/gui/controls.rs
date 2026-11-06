use super::{
    DesktopApp, controller::ExportStatus, model::Connection, settings::WriteStatus, theme,
};
use crate::hotkeys::binding::HotkeyBinding;

impl DesktopApp {
    pub(super) fn listener_controls(&mut self, ui: &mut egui::Ui) {
        let snapshot = self
            .controller
            .as_ref()
            .map(|c| c.listener_snapshot())
            .unwrap_or_default();
        let running = self
            .controller
            .as_ref()
            .is_some_and(|c| c.listener_running());
        let valid = HotkeyBinding::parse(&self.preferences.binding).is_ok();
        let verified = self.model.connection(std::time::Instant::now()) == Connection::Verified;
        let cleanup_failed = matches!(
            snapshot.last_error,
            Some(
                crate::hotkeys::HotkeyErrorCode::ShutdownFailed
                    | crate::hotkeys::HotkeyErrorCode::ShutdownTimeout
            )
        );
        if !valid {
            ui.colored_label(theme::ERROR, "Choose a valid modified shortcut.");
        }
        if !verified {
            ui.colored_label(
                theme::WARNING,
                "Start requires a verified registered build.",
            );
        }
        if let Some(error) = snapshot.last_error {
            ui.colored_label(theme::ERROR, error.as_str());
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    verified && valid && !running && !cleanup_failed && self.controller.is_some(),
                    egui::Button::new("Start listener"),
                )
                .clicked()
                && let Some(controller) = &mut self.controller
            {
                self.error = controller.start_listener(&self.preferences.binding).err();
            }
            if ui
                .add_enabled(running, egui::Button::new("Stop listener"))
                .clicked()
                && let Some(controller) = &self.controller
            {
                controller.stop_listener();
            }
        });
    }
    pub(super) fn support_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(20.0);
        ui.heading("SUPPORT EXPORT");
        let status = self
            .controller
            .as_ref()
            .map(|c| c.export_status())
            .unwrap_or_default();
        if status == ExportStatus::Pending {
            theme::skeleton(ui, 400.0);
            theme::skeleton(ui, 160.0);
        } else {
            let destination_label = ui.label("Destination JSON file");
            ui.text_edit_singleline(&mut self.destination)
                .labelled_by(destination_label.id);
            if ui
                .add_enabled(
                    !self.destination.trim().is_empty()
                        && self.controller.is_some()
                        && !(status == ExportStatus::Failed("SUPPORT_EXPORT_PATH_EXISTS")
                            && self.last_export_destination.as_deref()
                                == Some(self.destination.trim())),
                    egui::Button::new("Export support report"),
                )
                .clicked()
                && let Some(controller) = &self.controller
            {
                self.error = controller.export(self.destination.trim().into()).err();
                if self.error.is_none() {
                    self.last_export_destination = Some(self.destination.trim().to_owned());
                }
            }
        }
        match status {
            ExportStatus::Complete => {
                ui.colored_label(theme::SUCCESS, "Support report exported.");
            }
            ExportStatus::Failed(error) => {
                ui.colored_label(theme::ERROR, error);
            }
            _ => {}
        }
    }
    pub(super) fn settings_controls(&mut self, ui: &mut egui::Ui) {
        let status = self.writer.as_ref().map(|w| w.status()).unwrap_or_default();
        if status == WriteStatus::Pending {
            for width in [140.0, 320.0, 320.0, 280.0, 160.0] {
                theme::skeleton(ui, width);
                ui.add_space(8.0);
            }
            return;
        }
        ui.checkbox(&mut self.preferences.dark, "Dark theme");
        ui.add(egui::Slider::new(&mut self.preferences.text_scale, 0.8..=1.6).text("Text scale"));
        ui.add(
            egui::Slider::new(&mut self.preferences.refresh_seconds, 1..=30)
                .text("Refresh interval (seconds)"),
        );
        let binding_label = ui.label("Observation shortcut");
        let running = self
            .controller
            .as_ref()
            .is_some_and(|c| c.listener_running());
        ui.add_enabled(
            !running,
            egui::TextEdit::singleline(&mut self.preferences.binding).interactive(!running),
        )
        .labelled_by(binding_label.id);
        let valid = self.preferences.validate().is_ok();
        if !valid {
            ui.colored_label(theme::ERROR, "Preferences contain an invalid value.");
        }
        if self.writer.is_none() {
            ui.colored_label(theme::WARNING, "Preferences storage is unavailable.");
        }
        if ui
            .add_enabled(
                valid && self.writer.is_some(),
                egui::Button::new("Save preferences"),
            )
            .clicked()
        {
            self.error = self
                .writer
                .as_ref()
                .and_then(|writer| writer.save(self.preferences.clone()).err());
            if self.error.is_none()
                && let Some(controller) = &self.controller
            {
                self.error = controller
                    .set_interval(self.preferences.refresh_seconds)
                    .err();
            }
        }
        match status {
            WriteStatus::Saved => {
                ui.colored_label(theme::SUCCESS, "Preferences saved.");
            }
            WriteStatus::Failed(error) => {
                ui.colored_label(theme::ERROR, error);
            }
            _ => {}
        }
    }
}
