use super::{DesktopApp, Dialog, screens, theme};

impl DesktopApp {
    pub(super) fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog else {
            return;
        };
        let title = match dialog {
            Dialog::Settings => "Settings",
            Dialog::Help => "Help",
            Dialog::Diagnostics => "Diagnostics",
        };
        let mut open = true;
        egui::Window::new(title).open(&mut open).collapsible(false).default_width(600.0)
            .default_pos(egui::pos2(45.0,65.0)).resizable(true).show(ctx,|ui| {
                egui::ScrollArea::vertical().max_height(470.0).show(ui,|ui| match dialog {
                    Dialog::Settings=>self.settings_controls(ui),
                    Dialog::Help=>{
                        ui.heading("DASRU observation utility");
                        ui.label("Start the game and wait for a verified registered build. Set a modified shortcut, then start the listener from Options.");
                        ui.label("The listener is inactive outside TDA. In the verified game, each physical chord records one observation. It does not perform an action in the game.");
                        ui.label("Stop the listener before editing its binding. Closing DASRU stops its workers and saves validated preferences.");
                        ui.colored_label(theme::WARNING,"FPS limiting, freescroll, resolution scaling and trainer controls are unavailable.");
                        ui.label("Diagnostics shows capability evidence and exports privacy-safe JSON without overwriting existing files.");
                    }
                    Dialog::Diagnostics=>{
                        let listener=self.controller.as_ref().map(|c|c.listener_snapshot()).unwrap_or_default();
                        screens::diagnostics(ui,&self.model,&listener);
                        self.support_controls(ui);
                    }
                });
            });
        if !open {
            self.close_dialog();
        }
    }
}
