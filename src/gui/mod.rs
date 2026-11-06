#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Dashboard,
    Hotkeys,
    Diagnostics,
    Settings,
}

#[derive(Default)]
pub struct DesktopApp {
    screen: Screen,
    controller: Option<controller::Controller>,
    model: model::Model,
    preferences: settings::Preferences,
    writer: Option<settings::SettingsWriter>,
    error: Option<&'static str>,
    destination: String,
    last_export_destination: Option<String>,
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
    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn select(&mut self, screen: Screen) {
        self.screen = screen;
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
        ui.set_style(style);
        if let Some(rect) = ui.ctx().input(|input| input.viewport().inner_rect) {
            self.preferences.size = [rect.width().max(860.0), rect.height().max(560.0)];
        }
        if let Some(rect) = ui.ctx().input(|input| input.viewport().outer_rect) {
            self.preferences.position = Some([rect.min.x, rect.min.y]);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
        egui::Panel::left("navigation").show(ui, |ui| {
            ui.heading("DASRU");
            for (screen, title) in [
                (Screen::Dashboard, "Dashboard"),
                (Screen::Hotkeys, "Hotkeys"),
                (Screen::Diagnostics, "Diagnostics"),
                (Screen::Settings, "Settings"),
            ] {
                if ui.selectable_label(self.screen == screen, title).clicked() {
                    self.select(screen);
                }
            }
        });
        let listener = self
            .controller
            .as_ref()
            .map(|c| c.listener_snapshot())
            .unwrap_or_default();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(format!("{:?}", self.screen));
            if let Some(error) = self.error {
                ui.colored_label(theme::ERROR, error);
            }
            if ui.button("Refresh").clicked()
                && let Some(controller) = &self.controller
            {
                controller.refresh();
            }
            egui::ScrollArea::vertical().show(ui, |ui| match self.screen {
                Screen::Dashboard => screens::dashboard(ui, &self.model, &listener),
                Screen::Diagnostics => {
                    screens::diagnostics(ui, &self.model, &listener);
                    self.support_controls(ui);
                }
                Screen::Hotkeys => self.hotkey_controls(ui),
                Screen::Settings => self.settings_controls(ui),
            });
        });
    }
}

#[cfg(windows)]
impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.show(ui);
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}
impl DesktopApp {
    pub fn shutdown(&mut self) {
        if let Some(controller) = &mut self.controller
            && let Err(error) = controller.shutdown()
        {
            self.error = Some(error);
        }
        if let Some(mut writer) = self.writer.take()
            && let Err(error) = writer.finish(self.preferences.clone())
        {
            self.error = Some(error);
        }
    }
}
pub mod controller;
mod controls;
pub mod model;
mod screens;
pub mod services;
pub mod settings;
mod settings_writer;
pub mod theme;
