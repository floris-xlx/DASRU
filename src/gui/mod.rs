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
}

impl DesktopApp {
    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn select(&mut self, screen: Screen) {
        self.screen = screen;
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.ctx().set_visuals(egui::Visuals::dark());
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
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(format!("{:?}", self.screen));
            ui.label("Desktop foundation: monitoring is not connected.");
        });
    }
}

#[cfg(windows)]
impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.show(ui);
    }
}
