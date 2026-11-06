#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    Settings,
    Help,
    Diagnostics,
}

pub type ShutdownReport = std::sync::Arc<std::sync::Mutex<Option<Result<(), &'static str>>>>;

#[derive(Default)]
pub struct DesktopApp {
    dialog: Option<Dialog>,
    controller: Option<controller::Controller>,
    model: model::Model,
    preferences: settings::Preferences,
    writer: Option<settings::SettingsWriter>,
    error: Option<&'static str>,
    destination: String,
    last_export_destination: Option<String>,
    shutdown_report: ShutdownReport,
}

impl DesktopApp {
    #[cfg(windows)]
    pub fn restore_window(&mut self, creation: &eframe::CreationContext<'_>) {
        if let Some(window) = creation.winit_window() {
            let scale = window.scale_factor();
            let displays: Vec<_> = window
                .available_monitors()
                .map(|monitor| {
                    let position = monitor.position().to_logical::<f32>(scale);
                    let size = monitor.size().to_logical::<f32>(scale);
                    ([position.x, position.y], [size.width, size.height])
                })
                .collect();
            let display = self
                .preferences
                .position
                .and_then(|position| {
                    displays.iter().find(|(origin, size)| {
                        position[0] >= origin[0]
                            && position[1] >= origin[1]
                            && position[0] < origin[0] + size[0]
                            && position[1] < origin[1] + size[1]
                    })
                })
                .or(displays.first());
            if let Some((origin, size)) = display {
                self.preferences.clamp_geometry(*origin, *size);
            }
            creation
                .egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    self.preferences.size.into(),
                ));
            if let Some(position) = self.preferences.position {
                creation
                    .egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::OuterPosition(position.into()));
            }
        }
    }
    pub fn monitored() -> std::io::Result<Self> {
        let (writer, preferences, error) = match settings::user_path() {
            Ok(path) => {
                let (store, preferences, error) = settings::SettingsStore::load(path);
                (
                    if error.is_none() {
                        Some(settings::SettingsWriter::start(store)?)
                    } else {
                        None
                    },
                    preferences,
                    error,
                )
            }
            Err(error) => (None, settings::Preferences::default(), Some(error)),
        };
        Ok(Self {
            controller: Some(controller::Controller::start(
                services::ProbeService::new(services::NativeProbe),
                std::time::Duration::from_secs(preferences.refresh_seconds),
            )?),
            writer,
            preferences,
            error,
            ..Self::default()
        })
    }
    pub fn dialog(&self) -> Option<Dialog> {
        self.dialog
    }
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
    }
    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(controller) = &self.controller {
            self.model = controller.snapshot();
        }
        theme::apply(ui.ctx(), self.preferences.dark);
        let mut style = (*ui.ctx().style_of(if self.preferences.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        }))
        .clone();
        style.text_styles = egui::Style::default().text_styles;
        for font in style.text_styles.values_mut() {
            font.size *= self.preferences.text_scale;
        }
        ui.ctx().set_style_of(
            if self.preferences.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            },
            style.clone(),
        );
        ui.set_style(style);
        if let Some(rect) = ui.ctx().input(|input| input.viewport().inner_rect) {
            self.preferences.size = [
                settings::WINDOW_WIDTH,
                rect.height()
                    .clamp(settings::MIN_HEIGHT, settings::MAX_HEIGHT),
            ];
        }
        if let Some(rect) = ui.ctx().input(|input| input.viewport().outer_rect) {
            self.preferences.position = Some([rect.min.x, rect.min.y]);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
        self.utility(ui);
        self.show_dialog(ui.ctx());
    }
}

#[cfg(windows)]
impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.show(ui);
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        let _ = self.shutdown();
    }
}
impl DesktopApp {
    pub fn shutdown_report(&self) -> ShutdownReport {
        self.shutdown_report.clone()
    }

    pub fn shutdown(&mut self) -> Result<(), &'static str> {
        if let Some(result) = *self
            .shutdown_report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            return result;
        }
        let mut result = Ok(());
        if let Some(controller) = &mut self.controller
            && let Err(error) = controller.shutdown()
        {
            result = Err(error);
        }
        if let Some(mut writer) = self.writer.take()
            && let Err(error) = writer.finish(self.preferences.clone())
        {
            result = result.and(Err(error));
        }
        if let Err(error) = result {
            self.error = Some(error);
        }
        *self
            .shutdown_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(result);
        result
    }
}
pub mod controller;
mod controls;
mod dialogs;
pub mod model;
mod screens;
pub mod services;
pub mod settings;
mod settings_writer;
pub mod theme;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_failure_survives_app_destruction_and_repeated_close() {
        let path =
            std::env::temp_dir().join(format!("dasru-shutdown-error-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (store, _, error) = settings::SettingsStore::load(path.clone());
        assert!(error.is_none());
        std::fs::write(&path, b"preserve external change").unwrap();
        let mut app = DesktopApp {
            writer: Some(settings::SettingsWriter::start(store).unwrap()),
            ..Default::default()
        };
        let report = app.shutdown_report();
        let first = app.shutdown();
        let repeated = app.shutdown();
        drop(app);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(first, Err("SETTINGS_CHANGED_EXTERNALLY"));
        assert_eq!(repeated, first);
        assert_eq!(*report.lock().unwrap(), Some(first));
        assert_eq!(bytes, b"preserve external change");
    }

    #[test]
    fn separate_dialogs_inherit_the_text_scale_preference() {
        let mut app = DesktopApp::default();
        app.preferences.text_scale = 1.4;
        app.open_dialog(Dialog::Help);
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.show(ui));
        output.textures_delta.clear();
        let size = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.text().starts_with("Start the game") =>
                {
                    Some(text.galley.job.sections[0].format.font_id.size)
                }
                _ => None,
            })
            .expect("help instructions are visible in a separate dialog");
        assert!(
            (size - egui::Style::default().text_styles[&egui::TextStyle::Body].size * 1.4).abs()
                < 0.01
        );
    }
}
mod utility;
