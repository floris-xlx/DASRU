use dasru::hotkeys::{
    binding::{HotkeyAction, HotkeyBinding},
    config::HotkeyConfig,
    dispatch::{ActionDispatcher, ActionEvent, ActionEventSink, ConfiguredBinding},
    manager::{
        ForegroundBackend, HotkeyErrorCode, HotkeyEvent, HotkeyManager, RegistrationBackend,
        RegistrationFailure, VerifiedGameProcess,
    },
};
use dasru::process::ProcessIdentity;
use std::{cell::RefCell, rc::Rc};

fn config(entries: &[(&str, &str, bool)]) -> HotkeyConfig {
    let mut source = String::from("schema_version = 1\n\n");
    for (id, shortcut, enabled) in entries {
        source.push_str(&format!(
            "[[bindings]]\nid = \"{id}\"\nshortcut = \"{shortcut}\"\naction = \"observe\"\nenabled = {enabled}\n\n"
        ));
    }
    HotkeyConfig::parse(&source).unwrap()
}

#[test]
fn resolves_primary_binding_to_action() {
    let config = config(&[("primary", "CTRL+ALT+F9", true)]);
    let resolved = ConfiguredBinding::from_config(&config).unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].id, "primary");
    assert_eq!(resolved[0].action, HotkeyAction::Observe);
}

#[test]
fn resolves_secondary_binding_to_action() {
    let config = config(&[
        ("primary", "CTRL+ALT+F9", true),
        ("secondary", "CTRL+ALT+F10", true),
    ]);
    let resolved = ConfiguredBinding::from_config(&config).unwrap();

    assert_eq!(resolved[1].id, "secondary");
    assert_eq!(resolved[1].action, HotkeyAction::Observe);
}

#[test]
fn preserves_action_identity_after_shortcut_normalization() {
    let config = config(&[("primary", "ALT+CTRL+F9", true)]);
    let resolved = ConfiguredBinding::from_config(&config).unwrap();

    assert_eq!(resolved[0].id, "primary");
    assert_eq!(resolved[0].binding.to_string(), "CTRL+ALT+F9");
}

#[test]
fn rejects_unconfigured_shortcut() {
    let config = config(&[("primary", "CTRL+ALT+F9", true)]);
    let resolved = ConfiguredBinding::from_config(&config).unwrap();

    assert!(
        ConfiguredBinding::find(&resolved, HotkeyBinding::parse("CTRL+ALT+F10").unwrap()).is_none()
    );
}

#[test]
fn rejects_disabled_action() {
    let config = config(&[
        ("primary", "CTRL+ALT+F9", true),
        ("secondary", "CTRL+ALT+F10", false),
    ]);
    let resolved = ConfiguredBinding::from_config(&config).unwrap();

    assert!(
        ConfiguredBinding::find(&resolved, HotkeyBinding::parse("CTRL+ALT+F10").unwrap()).is_none()
    );
}

#[derive(Clone)]
struct Sink(Rc<RefCell<Vec<ActionEvent>>>);

impl ActionEventSink for Sink {
    fn emit(&mut self, event: ActionEvent) {
        self.0.borrow_mut().push(event);
    }
}

struct Registrar;

impl RegistrationBackend for Registrar {
    fn register(&mut self, _id: u32, _binding: HotkeyBinding) -> Result<(), RegistrationFailure> {
        Ok(())
    }

    fn unregister(&mut self, _id: u32) -> Result<(), RegistrationFailure> {
        Ok(())
    }
}

struct Foreground(ProcessIdentity);

impl ForegroundBackend for Foreground {
    fn verify_game(
        &mut self,
        expected_pid: u32,
        expected_identity: &ProcessIdentity,
    ) -> Result<(), HotkeyErrorCode> {
        if expected_pid == 42 && expected_identity == &self.0 {
            Ok(())
        } else {
            Err(HotkeyErrorCode::IdentityChanged)
        }
    }

    fn foreground_pid(&mut self) -> Result<Option<u32>, HotkeyErrorCode> {
        Ok(Some(42))
    }
}

fn identity() -> ProcessIdentity {
    ProcessIdentity {
        creation_time_filetime: 1,
        executable_path: r"C:\Games\DOOMTheDarkAges.exe".into(),
        module_base: 0x1400_0000,
        image_size: 0x2000,
    }
}

fn dispatcher() -> (ActionDispatcher<Sink>, Rc<RefCell<Vec<ActionEvent>>>) {
    let configured =
        ConfiguredBinding::from_config(&config(&[("primary", "CTRL+ALT+F9", true)])).unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    (
        ActionDispatcher::new(configured, Sink(events.clone())),
        events,
    )
}

fn manager() -> HotkeyManager<Registrar, Foreground> {
    let identity = identity();
    HotkeyManager::new(
        vec![HotkeyBinding::parse("CTRL+ALT+F9").unwrap()],
        VerifiedGameProcess {
            pid: 42,
            identity: identity.clone(),
            build_id: "test-build".to_owned(),
        },
        Registrar,
        Foreground(identity),
    )
    .unwrap()
}

#[test]
fn dispatches_exactly_once_per_keypress_and_suppresses_held_repeat() {
    let (mut dispatcher, events) = dispatcher();
    let mut manager = manager();
    manager.start().unwrap();

    assert_eq!(manager.handle_keyboard_input(0xa2, true), None);
    assert_eq!(manager.handle_keyboard_input(0xa4, true), None);
    for _ in 0..4 {
        if let Some(event) = manager.handle_keyboard_input(0x78, true) {
            dispatcher.dispatch(event, 0, 0);
        }
    }

    assert_eq!(events.borrow().len(), 1);
    assert_eq!(events.borrow()[0].id, "primary");
    assert_eq!(events.borrow()[0].action, HotkeyAction::Observe);
}

#[test]
fn dispatches_secondary_shortcut_to_its_configured_id() {
    let configured = ConfiguredBinding::from_config(&config(&[
        ("primary", "CTRL+ALT+F9", true),
        ("secondary", "CTRL+ALT+F10", true),
    ]))
    .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut dispatcher = ActionDispatcher::new(configured, Sink(events.clone()));

    assert!(dispatcher.dispatch(
        HotkeyEvent::Accepted(HotkeyBinding::parse("CTRL+ALT+F10").unwrap()),
        0,
        0,
    ));
    assert_eq!(events.borrow().len(), 1);
    assert_eq!(events.borrow()[0].id, "secondary");
}

#[test]
fn rejects_events_when_unfocused() {
    let (mut dispatcher, events) = dispatcher();
    assert!(!dispatcher.dispatch(
        HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::ForegroundMismatch,
        },
        0,
        0,
    ));
    assert!(events.borrow().is_empty());
}

#[test]
fn rejects_stale_focus_generation() {
    let (mut dispatcher, events) = dispatcher();
    assert!(!dispatcher.dispatch(
        HotkeyEvent::Accepted(HotkeyBinding::parse("CTRL+ALT+F9").unwrap()),
        4,
        5,
    ));
    assert!(events.borrow().is_empty());
}

#[test]
fn rejects_changed_process_identity() {
    let (mut dispatcher, events) = dispatcher();
    assert!(!dispatcher.dispatch(
        HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::IdentityChanged,
        },
        0,
        0,
    ));
    assert!(events.borrow().is_empty());
}

#[test]
fn does_not_dispatch_unconfigured_or_disabled_shortcuts() {
    let configured = ConfiguredBinding::from_config(&config(&[
        ("primary", "CTRL+ALT+F9", true),
        ("disabled", "CTRL+ALT+F10", false),
    ]))
    .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut dispatcher = ActionDispatcher::new(configured, Sink(events.clone()));
    for shortcut in ["CTRL+ALT+F10", "CTRL+ALT+F11"] {
        assert!(!dispatcher.dispatch(
            HotkeyEvent::Accepted(HotkeyBinding::parse(shortcut).unwrap()),
            0,
            0,
        ));
    }
    assert!(events.borrow().is_empty());
}

#[test]
fn clears_modifier_state_on_focus_loss() {
    let mut manager = manager();
    manager.start().unwrap();
    manager.handle_keyboard_input(0xa2, true);
    manager.handle_keyboard_input(0xa4, true);
    manager.shutdown().unwrap();
    assert_eq!(manager.handle_keyboard_input(0x78, true), None);
}

#[test]
fn dispatches_nothing_after_shutdown() {
    let (mut dispatcher, events) = dispatcher();
    dispatcher.shutdown();
    assert!(!dispatcher.dispatch(
        HotkeyEvent::Accepted(HotkeyBinding::parse("CTRL+ALT+F9").unwrap()),
        0,
        0,
    ));
    assert!(events.borrow().is_empty());
}
