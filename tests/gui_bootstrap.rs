#![cfg(feature = "desktop-ui")]
#[test]
fn advanced_hotkeys_respond_to_accessible_expand_and_collapse() {
    let mut app = dasru::gui::DesktopApp::default();
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut render = |time, events| {
        let mut output = context.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(720.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
        output
    };
    let first = render(0.0, vec![]);
    let target = first
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("More hotkeys / advanced"))
        .map(|(id, _)| *id)
        .unwrap();
    let action = |action| {
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: target,
            data: None,
        })
    };
    render(0.5, vec![action(egui::accesskit::Action::Expand)]);
    let expanded = render(1.0, vec![]);
    let painted_counter = |output: &egui::FullOutput| {
        output.shapes.iter().any(|shape|
        matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Accepted"))
    };
    assert!(
        painted_counter(&expanded),
        "accessible expand did not open observation counters"
    );
    render(1.5, vec![action(egui::accesskit::Action::Collapse)]);
    assert!(!painted_counter(&render(2.0, vec![])));
}

#[test]
fn status_inputs_expose_read_only_semantics_to_accessibility() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        egui::Grid::new("status_accessibility").show(ui, |ui| {
            dasru::gui::theme::value(ui, "Build", "registered-fixture");
            dasru::gui::theme::connection(ui, dasru::gui::model::Connection::Verified);
        });
    });
    output.textures_delta.clear();
    let update = output.platform_output.accesskit_update.unwrap();
    let inputs: Vec<_> = update
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == egui::accesskit::Role::TextInput)
        .collect();
    assert!(!inputs.is_empty());
    for (_, node) in inputs {
        assert!(node.is_disabled());
        assert!(
            node.is_read_only(),
            "status input is incorrectly advertised as editable"
        );
    }
}

#[test]
fn text_inputs_expose_their_visible_labels_to_accessibility() {
    for dialog in [
        None,
        Some(dasru::gui::Dialog::Settings),
        Some(dasru::gui::Dialog::Diagnostics),
    ] {
        let mut app = dasru::gui::DesktopApp::default();
        if let Some(dialog) = dialog {
            app.open_dialog(dialog);
        }
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(720.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree is enabled");
        let inputs: Vec<_> = update
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::TextInput)
            .collect();
        assert!(!inputs.is_empty());
        for (id, node) in inputs {
            assert!(
                !node.labelled_by().is_empty(),
                "text input {id:?} lacks its visible label in {dialog:?}"
            );
        }
    }
}

#[test]
fn utility_idle_does_not_request_continuous_repainting() {
    let mut app = dasru::gui::DesktopApp::default();
    let context = egui::Context::default();
    let mut delay = std::time::Duration::ZERO;
    for frame in 0..10 {
        let mut output = context.run_ui(
            egui::RawInput {
                time: Some(frame as f64),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(720.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        output.textures_delta.clear();
    }
    assert!(
        delay >= std::time::Duration::from_millis(100),
        "idle repaint delay was {delay:?}"
    );
}
#[test]
fn compact_utility_shows_primary_panels_without_sidebar() {
    let mut app = dasru::gui::DesktopApp::default();
    let context = egui::Context::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 700.0),
            )),
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .collect();
    for label in [
        "KEYBINDS",
        "GAME STATUS",
        "FPS LIMITER",
        "OPTIONS",
        "RESOLUTION SCALING",
        "GAME VERSION",
        "TRAINER / METRICS",
        "SETTINGS",
        "HELP",
        "DIAGNOSTICS",
    ] {
        assert!(labels.contains(&label), "missing {label}: {labels:?}");
    }
    assert!(!labels.contains(&"Dashboard"));
    let heading = |name: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == name => Some(text.pos),
                _ => None,
            })
            .unwrap()
    };
    for (left, right) in [
        ("KEYBINDS", "GAME STATUS"),
        ("FPS LIMITER", "OPTIONS"),
        ("RESOLUTION SCALING", "GAME VERSION"),
    ] {
        assert!(heading(left).x < heading(right).x);
        assert!((heading(left).y - heading(right).y).abs() < 1.0);
    }
    for name in ["TRAINER / METRICS", "SETTINGS", "HELP", "DIAGNOSTICS"] {
        assert!(
            heading(name).y < 680.0,
            "{name} is outside the compact window"
        );
    }
    for label in ["NotRunning", "Not recognized", "Disabled"] {
        let shape=output.shapes.iter().find(|shape| matches!(&shape.shape,egui::epaint::Shape::Text(text) if text.galley.text()==label)).expect("status value is painted");
        let egui::epaint::Shape::Text(text) = &shape.shape else {
            unreachable!()
        };
        assert!(
            text.pos.x + text.galley.size().x <= shape.clip_rect.max.x + 1.0,
            "{label} is clipped"
        );
    }
    output.textures_delta.clear();
}
#[test]
fn application_initializes_without_game_or_open_dialog() {
    let app = dasru::gui::DesktopApp::default();
    assert_eq!(app.dialog(), None);
}
#[test]
fn dialogs_open_and_close_without_replacing_primary_controls() {
    use dasru::gui::Dialog;
    for dialog in [Dialog::Settings, Dialog::Help, Dialog::Diagnostics] {
        let mut app = dasru::gui::DesktopApp::default();
        app.open_dialog(dialog);
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.show(ui));
        output.textures_delta.clear();
        assert_eq!(app.dialog(), Some(dialog));
        app.close_dialog();
        assert_eq!(app.dialog(), None);
    }
}
#[test]
fn compact_utility_and_dialogs_render_at_supported_scaling() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for dialog in [
            None,
            Some(dasru::gui::Dialog::Settings),
            Some(dasru::gui::Dialog::Help),
            Some(dasru::gui::Dialog::Diagnostics),
        ] {
            let mut app = dasru::gui::DesktopApp::default();
            if let Some(dialog) = dialog {
                app.open_dialog(dialog);
            }
            let context = egui::Context::default();
            context.set_pixels_per_point(scale);
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(720.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty());
            assert_eq!(app.dialog(), dialog);
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
