use std::{cell::RefCell, rc::Rc};

use dasru::{
    hotkeys::{
        binding::HotkeyBinding,
        manager::{
            ForegroundBackend, HotkeyErrorCode, HotkeyEvent, HotkeyManager, HotkeyState,
            RegistrationBackend, RegistrationFailure, RegistrationOperation, VerifiedGameProcess,
        },
        state::{HotkeyLifecycle, LifecycleEvent},
    },
    process::ProcessIdentity,
};

struct FakeRegistration {
    operations: Rc<RefCell<Vec<RegistrationOperation>>>,
    failure_at: Option<(usize, RegistrationFailure)>,
    attempts: usize,
}

impl RegistrationBackend for FakeRegistration {
    fn register(&mut self, id: u32, binding: HotkeyBinding) -> Result<(), RegistrationFailure> {
        self.operations
            .borrow_mut()
            .push(RegistrationOperation::Register(id, binding));
        let failure = self
            .failure_at
            .filter(|(attempt, _)| *attempt == self.attempts)
            .map(|(_, failure)| failure);
        self.attempts += 1;
        if failure.is_some() {
            self.failure_at = None;
        }
        failure.map_or(Ok(()), Err)
    }

    fn unregister(&mut self, id: u32) -> Result<(), RegistrationFailure> {
        self.operations
            .borrow_mut()
            .push(RegistrationOperation::Unregister(id));
        Ok(())
    }
}

#[derive(Clone)]
struct FakeForeground(Rc<RefCell<ForegroundState>>);

#[derive(Clone)]
struct ForegroundState {
    candidate_count: usize,
    current_identity: Option<ProcessIdentity>,
    foreground_pid: Result<Option<u32>, HotkeyErrorCode>,
}

type TestRig = (
    HotkeyManager<FakeRegistration, FakeForeground>,
    Rc<RefCell<Vec<RegistrationOperation>>>,
    Rc<RefCell<ForegroundState>>,
);

impl ForegroundBackend for FakeForeground {
    fn verify_game(
        &mut self,
        expected_pid: u32,
        expected_identity: &ProcessIdentity,
    ) -> Result<(), HotkeyErrorCode> {
        let state = self.0.borrow();
        match state.candidate_count {
            0 => Err(HotkeyErrorCode::NoProcess),
            1 => match &state.current_identity {
                Some(identity) if identity == expected_identity => Ok(()),
                Some(_) => Err(HotkeyErrorCode::IdentityChanged),
                None => Err(HotkeyErrorCode::AccessDenied),
            },
            _ => Err(HotkeyErrorCode::AmbiguousProcess),
        }
        .and({
            if expected_pid == 42 {
                Ok(())
            } else {
                Err(HotkeyErrorCode::IdentityChanged)
            }
        })
    }

    fn foreground_pid(&mut self) -> Result<Option<u32>, HotkeyErrorCode> {
        self.0.borrow().foreground_pid
    }
}

fn identity(creation_time_filetime: u64) -> ProcessIdentity {
    ProcessIdentity {
        creation_time_filetime,
        executable_path: r"C:\Games\DOOMTheDarkAges.exe".into(),
        module_base: 0x1400_0000,
        image_size: 0x2000,
    }
}

fn rig() -> TestRig {
    rig_with_failure(None)
}

fn rig_with_failure(failure: Option<RegistrationFailure>) -> TestRig {
    rig_config(failure.map(|failure| (0, failure)), "steam-test-build")
}

fn rig_with_build_id(build_id: &str) -> TestRig {
    rig_config(None, build_id)
}

fn rig_config(failure_at: Option<(usize, RegistrationFailure)>, build_id: &str) -> TestRig {
    let identity = identity(1);
    let registration_events = Rc::new(RefCell::new(Vec::new()));
    let foreground = Rc::new(RefCell::new(ForegroundState {
        candidate_count: 1,
        current_identity: Some(identity.clone()),
        foreground_pid: Ok(Some(42)),
    }));
    let game = VerifiedGameProcess {
        pid: 42,
        identity,
        build_id: build_id.to_owned(),
    };
    let manager = HotkeyManager::new(
        vec![HotkeyBinding::parse("CTRL+ALT+F9").unwrap()],
        game,
        FakeRegistration {
            operations: registration_events.clone(),
            failure_at,
            attempts: 0,
        },
        FakeForeground(foreground.clone()),
    )
    .unwrap();
    (manager, registration_events, foreground)
}

#[test]
fn disabled_engine_never_registers() {
    let registration_events = Rc::new(RefCell::new(Vec::new()));
    let foreground = Rc::new(RefCell::new(ForegroundState {
        candidate_count: 0,
        current_identity: None,
        foreground_pid: Ok(None),
    }));
    let mut manager = HotkeyManager::new_disabled(
        FakeRegistration {
            operations: registration_events.clone(),
            failure_at: None,
            attempts: 0,
        },
        FakeForeground(foreground),
    );

    manager.start().unwrap();

    assert_eq!(manager.state(), HotkeyState::Disabled);
    assert!(registration_events.borrow().is_empty());
}

#[test]
fn verified_foreground_process_activates_bindings() {
    let (mut manager, registration_events, _) = rig();

    manager.start().unwrap();

    assert_eq!(manager.state(), HotkeyState::Active);
    assert_eq!(registration_events.borrow().len(), 1);
}

#[test]
fn verified_process_outside_foreground_starts_inactive() {
    let (mut manager, registration_events, foreground) = rig();
    foreground.borrow_mut().foreground_pid = Ok(Some(99));

    manager.start().unwrap();

    assert_eq!(manager.state(), HotkeyState::Inactive);
    assert!(registration_events.borrow().is_empty());
}

#[test]
fn focus_loss_deactivates_bindings() {
    let (mut manager, registration_events, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(Some(99));

    manager.reconcile().unwrap();

    assert_eq!(manager.state(), HotkeyState::Inactive);
    assert_eq!(registration_events.borrow().len(), 2);
    assert!(matches!(
        registration_events.borrow()[1],
        RegistrationOperation::Unregister(1)
    ));
}

#[test]
fn process_exit_disables_bindings() {
    let (mut manager, registration_events, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().candidate_count = 0;

    assert_eq!(
        manager.reconcile().unwrap_err().code,
        HotkeyErrorCode::NoProcess
    );
    assert_eq!(manager.state(), HotkeyState::Disabled);
    assert!(matches!(
        registration_events.borrow().last(),
        Some(RegistrationOperation::Unregister(1))
    ));
}

#[test]
fn identity_change_requires_new_verification() {
    let (mut manager, registration_events, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().current_identity = Some(identity(2));

    assert_eq!(
        manager.reconcile().unwrap_err().code,
        HotkeyErrorCode::IdentityChanged
    );
    assert_eq!(manager.state(), HotkeyState::Disabled);
    assert_eq!(
        manager.reconcile().unwrap_err().code,
        HotkeyErrorCode::Disabled
    );
    assert_eq!(
        registration_events
            .borrow()
            .iter()
            .filter(|event| matches!(event, RegistrationOperation::Register(..)))
            .count(),
        1
    );
}

#[test]
fn registration_failure_enters_error_state() {
    let (mut manager, _, _) = rig_with_failure(Some(RegistrationFailure::Conflict));

    assert_eq!(
        manager.start().unwrap_err().code,
        HotkeyErrorCode::RegistrationConflict
    );
    assert_eq!(manager.state(), HotkeyState::Conflict);
}

#[test]
fn conflict_is_not_retried_until_focus_changes() {
    let (mut manager, operations, foreground) =
        rig_with_failure(Some(RegistrationFailure::Conflict));
    assert_eq!(
        manager.start().unwrap_err().code,
        HotkeyErrorCode::RegistrationConflict
    );
    assert_eq!(manager.state(), HotkeyState::Conflict);

    manager.reconcile().unwrap();

    assert_eq!(manager.state(), HotkeyState::Conflict);
    assert_eq!(
        operations
            .borrow()
            .iter()
            .filter(|event| matches!(event, RegistrationOperation::Register(..)))
            .count(),
        1
    );
    foreground.borrow_mut().foreground_pid = Ok(Some(99));
    manager.reconcile().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(Some(42));
    manager.reconcile().unwrap();
    assert_eq!(manager.state(), HotkeyState::Active);
}

#[test]
fn registration_error_enters_error_state() {
    let (mut manager, _, _) = rig_with_failure(Some(RegistrationFailure::Failed));

    assert_eq!(
        manager.start().unwrap_err().code,
        HotkeyErrorCode::RegistrationFailed
    );
    assert_eq!(manager.state(), HotkeyState::Error);
}

#[test]
fn raw_input_registration_failure_keeps_its_structured_error_code() {
    let (mut manager, _, _) =
        rig_with_failure(Some(RegistrationFailure::RawInputRegistrationFailed));

    assert_eq!(
        manager.start().unwrap_err().code,
        HotkeyErrorCode::RawInputRegistrationFailed
    );
    assert_eq!(manager.state(), HotkeyState::Error);
}

#[test]
fn partial_registration_failure_unregisters_earlier_bindings() {
    let identity = identity(1);
    let operations = Rc::new(RefCell::new(Vec::new()));
    let foreground = Rc::new(RefCell::new(ForegroundState {
        candidate_count: 1,
        current_identity: Some(identity.clone()),
        foreground_pid: Ok(Some(42)),
    }));
    let game = VerifiedGameProcess {
        pid: 42,
        identity,
        build_id: "steam-test-build".to_owned(),
    };
    let mut manager = HotkeyManager::new(
        vec![
            HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            HotkeyBinding::parse("CTRL+ALT+F10").unwrap(),
        ],
        game,
        FakeRegistration {
            operations: operations.clone(),
            failure_at: Some((1, RegistrationFailure::Conflict)),
            attempts: 0,
        },
        FakeForeground(foreground),
    )
    .unwrap();

    assert_eq!(
        manager.start().unwrap_err().code,
        HotkeyErrorCode::RegistrationConflict
    );
    assert_eq!(
        *operations.borrow(),
        vec![
            RegistrationOperation::Register(1, HotkeyBinding::parse("CTRL+ALT+F9").unwrap()),
            RegistrationOperation::Register(2, HotkeyBinding::parse("CTRL+ALT+F10").unwrap()),
            RegistrationOperation::Unregister(1),
        ]
    );
}

#[test]
fn ambiguous_or_unreadable_process_never_activates() {
    for (candidate_count, current_identity, expected) in [
        (0, Some(identity(1)), HotkeyErrorCode::NoProcess),
        (2, Some(identity(1)), HotkeyErrorCode::AmbiguousProcess),
        (1, None, HotkeyErrorCode::AccessDenied),
    ] {
        let (mut manager, operations, foreground) = rig();
        foreground.borrow_mut().candidate_count = candidate_count;
        foreground.borrow_mut().current_identity = current_identity;

        assert_eq!(manager.start().unwrap_err().code, expected);
        assert_eq!(manager.state(), HotkeyState::Disabled);
        assert!(operations.borrow().is_empty());
    }
}

#[test]
fn unknown_build_never_activates() {
    let (mut manager, operations, _) = rig_with_build_id("");

    assert_eq!(
        manager.start().unwrap_err().code,
        HotkeyErrorCode::UnknownBuild
    );
    assert_eq!(manager.state(), HotkeyState::Disabled);
    assert!(operations.borrow().is_empty());
}

#[test]
fn repeated_shutdown_is_idempotent() {
    let (mut manager, registration_events, _) = rig();
    manager.start().unwrap();

    manager.shutdown().unwrap();
    manager.shutdown().unwrap();

    assert_eq!(manager.state(), HotkeyState::Disabled);
    assert_eq!(
        registration_events
            .borrow()
            .iter()
            .filter(|event| matches!(event, RegistrationOperation::Unregister(_)))
            .count(),
        1
    );
}

#[test]
fn accepts_event_from_verified_foreground_game() {
    let (mut manager, _, _) = rig();
    manager.start().unwrap();

    assert_eq!(
        manager.handle_hotkey(1),
        Some(HotkeyEvent::Accepted(
            HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
        ))
    );
}

#[test]
fn physical_binding_make_is_delivered_once_and_repeats_are_suppressed() {
    let (mut manager, _, _) = rig();
    manager.start().unwrap();

    assert_eq!(manager.handle_keyboard_input(0xa2, true), None);
    assert_eq!(manager.handle_keyboard_input(0xa4, true), None);
    assert_eq!(
        manager.handle_keyboard_input(0x78, true),
        Some(HotkeyEvent::Accepted(
            HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
        ))
    );
    assert_eq!(manager.handle_keyboard_input(0x78, true), None);
    assert_eq!(manager.handle_keyboard_input(0x78, false), None);
}

#[test]
fn focus_reactivation_does_not_reuse_stale_modifier_state() {
    let (mut manager, _, foreground) = rig();
    manager.start().unwrap();
    manager.handle_keyboard_input(0xa2, true);
    manager.handle_keyboard_input(0xa4, true);

    foreground.borrow_mut().foreground_pid = Ok(Some(99));
    manager.reconcile().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(Some(42));
    manager.reconcile().unwrap();

    assert_eq!(manager.handle_keyboard_input(0x78, true), None);
    manager.handle_keyboard_input(0x78, false);
    manager.handle_keyboard_input(0xa2, true);
    manager.handle_keyboard_input(0xa4, true);
    assert!(matches!(
        manager.handle_keyboard_input(0x78, true),
        Some(HotkeyEvent::Accepted(_))
    ));
}

#[test]
fn rejects_event_from_unrelated_foreground_window() {
    let (mut manager, _, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(Some(99));

    assert_eq!(
        manager.handle_hotkey(1),
        Some(HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::ForegroundMismatch,
        })
    );
    assert_eq!(manager.state(), HotkeyState::Inactive);
}

#[test]
fn rejects_null_foreground_window() {
    let (mut manager, _, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(None);

    assert_eq!(
        manager.handle_hotkey(1),
        Some(HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::ForegroundUnavailable,
        })
    );
}

#[test]
fn rejects_same_pid_with_changed_creation_identity() {
    let (mut manager, _, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().current_identity = Some(identity(2));

    assert_eq!(
        manager.handle_hotkey(1),
        Some(HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::IdentityChanged,
        })
    );
    assert_eq!(manager.state(), HotkeyState::Disabled);
}

#[test]
fn rejects_ambiguous_game_processes() {
    let (mut manager, _, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().candidate_count = 2;

    assert_eq!(
        manager.handle_hotkey(1),
        Some(HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::AmbiguousProcess,
        })
    );
}

#[test]
fn rejects_event_when_process_revalidation_fails() {
    let (mut manager, _, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().current_identity = None;

    assert_eq!(
        manager.handle_hotkey(1),
        Some(HotkeyEvent::Rejected {
            binding: HotkeyBinding::parse("CTRL+ALT+F9").unwrap(),
            code: HotkeyErrorCode::AccessDenied,
        })
    );
}

#[test]
fn focus_reactivation_requires_fresh_verification() {
    let (mut manager, registration_events, foreground) = rig();
    manager.start().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(Some(99));
    manager.reconcile().unwrap();
    foreground.borrow_mut().foreground_pid = Ok(Some(42));
    foreground.borrow_mut().current_identity = Some(identity(2));

    assert_eq!(
        manager.reconcile().unwrap_err().code,
        HotkeyErrorCode::IdentityChanged
    );
    assert_eq!(manager.state(), HotkeyState::Disabled);
    assert_eq!(
        registration_events
            .borrow()
            .iter()
            .filter(|event| matches!(event, RegistrationOperation::Register(..)))
            .count(),
        1
    );
}

#[test]
fn invalid_state_transition_returns_a_typed_error() {
    let mut lifecycle = HotkeyLifecycle::default();

    assert_eq!(
        lifecycle.transition(LifecycleEvent::RegistrationSucceeded),
        Err(HotkeyErrorCode::InvalidTransition)
    );
}
