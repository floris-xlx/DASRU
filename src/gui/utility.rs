use super::{DesktopApp, Dialog, theme};
use crate::hotkeys::manager::HotkeyState;

fn panel(ui: &mut egui::Ui, title: &str, height: f32, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.inactive.bg_fill,
        ))
        .corner_radius(3)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(height);
            ui.label(egui::RichText::new(title).strong().size(14.0));
            ui.add_space(6.0);
            contents(ui);
        });
}

impl DesktopApp {
    pub(super) fn utility(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("utility_footer")
            .show_separator_line(false)
            .exact_size(42.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    for (dialog, title) in [
                        (Dialog::Settings, "SETTINGS"),
                        (Dialog::Help, "HELP"),
                        (Dialog::Diagnostics, "DIAGNOSTICS"),
                    ] {
                        if ui
                            .add_sized([140.0, 28.0], egui::Button::new(title))
                            .clicked()
                        {
                            self.open_dialog(dialog);
                        }
                    }
                    ui.label(concat!("v", env!("CARGO_PKG_VERSION")));
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("DOOM TDA SPEEDRUN UTILITY")
                        .strong()
                        .size(25.0)
                        .color(theme::ACCENT),
                );
            });
            if let Some(error) = self.error {
                ui.colored_label(theme::ERROR, error);
            }
            ui.add_space(10.0);
            let listener = self
                .controller
                .as_ref()
                .map(|c| c.listener_snapshot())
                .unwrap_or_default();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.columns(2, |columns| {
                        panel(&mut columns[0], "KEYBINDS", 124.0, |ui| {
                            ui.label("Observation only; no game action.");
                            ui.label("Observation shortcut");
                            let running = self
                                .controller
                                .as_ref()
                                .is_some_and(|c| c.listener_running());
                            ui.add_enabled(
                                !running,
                                egui::TextEdit::singleline(&mut self.preferences.binding)
                                    .desired_width(f32::INFINITY),
                            );
                            egui::CollapsingHeader::new("More hotkeys / advanced").show(ui, |ui| {
                                ui.label("Additional bindings are not implemented.");
                                egui::Grid::new("advanced_hotkeys").show(ui, |ui| {
                                    theme::value(ui, "Accepted", listener.accepted);
                                    theme::value(ui, "Rejected", listener.rejected);
                                    theme::value(
                                        ui,
                                        "Last event",
                                        listener
                                            .last_event
                                            .map(|e| format!("{e:?}"))
                                            .unwrap_or_else(|| "None".into()),
                                    );
                                });
                            });
                        });
                        panel(&mut columns[1], "GAME STATUS", 124.0, |ui| {
                            egui::Grid::new("compact_status")
                                .spacing([10.0, 5.0])
                                .show(ui, |ui| {
                                    theme::connection(
                                        ui,
                                        self.model.connection(std::time::Instant::now()),
                                    );
                                    theme::value(
                                        ui,
                                        "Process",
                                        self.model
                                            .observation
                                            .as_ref()
                                            .map(|o| format!("{:?}", o.process))
                                            .unwrap_or_else(|| "Not evaluated".into()),
                                    );
                                    theme::value(ui, "Hotkeys", format!("{:?}", listener.state));
                                });
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        self.controller.is_some(),
                                        egui::Button::new("Refresh"),
                                    )
                                    .clicked()
                                    && let Some(controller) = &self.controller
                                {
                                    controller.refresh();
                                }
                                ui.label("Read-only monitoring");
                            });
                        });
                    });
                    ui.add_space(8.0);
                    ui.columns(2, |columns| {
                        panel(&mut columns[0], "FPS LIMITER", 104.0, |ui| {
                            ui.colored_label(theme::SECONDARY, "Unavailable - not implemented.");
                        });
                        panel(&mut columns[1], "OPTIONS", 104.0, |ui| {
                            self.listener_controls(ui);
                            ui.colored_label(theme::SECONDARY, "Freescroll: not implemented.");
                        });
                    });
                    ui.add_space(8.0);
                    ui.columns(2, |columns| {
                        panel(&mut columns[0], "RESOLUTION SCALING", 82.0, |ui| {
                            ui.colored_label(theme::SECONDARY, "Unavailable - not implemented.");
                        });
                        panel(&mut columns[1], "GAME VERSION", 82.0, |ui| {
                            egui::Grid::new("compact_build")
                                .spacing([10.0, 5.0])
                                .show(ui, |ui| {
                                    theme::value(
                                        ui,
                                        "Build",
                                        self.model
                                            .observation
                                            .as_ref()
                                            .and_then(|o| o.build.as_deref())
                                            .unwrap_or("Not recognized"),
                                    );
                                    theme::value(
                                        ui,
                                        "Updated",
                                        self.model
                                            .updated
                                            .map(|at| format!("{}s ago", at.elapsed().as_secs()))
                                            .unwrap_or_else(|| "Never".into()),
                                    );
                                });
                        });
                    });
                    ui.add_space(8.0);
                    panel(ui, "TRAINER / METRICS", 38.0, |ui| {
                        ui.label("Trainer unavailable. Live metrics are not collected.");
                    });
                    if let Some(error) = listener.last_error {
                        ui.colored_label(theme::ERROR, error.as_str());
                    }
                    if let Some(error) = self.model.last_error {
                        ui.colored_label(theme::WARNING, error);
                    }
                    if listener.state == HotkeyState::Active {
                        ui.label("Observing only while the verified game is foreground.");
                    }
                });
        });
    }
}
