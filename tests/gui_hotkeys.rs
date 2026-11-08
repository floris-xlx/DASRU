use dasru::hotkeys::{
    binding::HotkeyBinding,
    manager::{HotkeyEvent, HotkeyState},
    runtime::{HotkeyRuntime, RuntimeBackend, RuntimeEvent},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Backend(Arc<AtomicUsize>);
impl RuntimeBackend for Backend {
    fn run(
        &mut self,
        binding: HotkeyBinding,
        stop: &AtomicBool,
        sink: &mut dyn FnMut(RuntimeEvent),
    ) -> Result<(), dasru::hotkeys::HotkeyErrorCode> {
        self.0.fetch_add(1, Ordering::SeqCst);
        sink(RuntimeEvent::State(HotkeyState::Inactive));
        sink(RuntimeEvent::Action(
            dasru::hotkeys::dispatch::ActionEvent {
                id: "gui-observation".into(),
                binding,
                action: binding.action(),
            },
        ));
        sink(RuntimeEvent::Observation(HotkeyEvent::Accepted(binding)));
        while !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.0.fetch_sub(1, Ordering::SeqCst);
        sink(RuntimeEvent::State(HotkeyState::Disabled));
        Ok(())
    }
}
#[test]
fn duplicate_start_is_rejected_and_stop_joins_resource_owner() {
    let owners = Arc::new(AtomicUsize::new(0));
    let mut runtime = HotkeyRuntime::default();
    let binding = HotkeyBinding::parse("Ctrl+Alt+F9").unwrap();
    runtime
        .start_with(binding, Backend(owners.clone()))
        .unwrap();
    assert!(
        runtime
            .start_with(binding, Backend(owners.clone()))
            .is_err()
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.snapshot().accepted == 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(runtime.snapshot().state, HotkeyState::Inactive);
    assert_eq!(runtime.snapshot().accepted, 1);
    runtime.stop().unwrap();
    runtime.stop().unwrap();
    assert_eq!(owners.load(Ordering::SeqCst), 0);
    assert_eq!(runtime.snapshot().state, HotkeyState::Disabled);
}
#[test]
fn window_owner_drop_stops_runtime() {
    let owners = Arc::new(AtomicUsize::new(0));
    let mut runtime = HotkeyRuntime::default();
    runtime
        .start_with(
            HotkeyBinding::parse("Ctrl+Alt+F9").unwrap(),
            Backend(owners.clone()),
        )
        .unwrap();
    drop(runtime);
    assert_eq!(owners.load(Ordering::SeqCst), 0);
}

struct LateObservation;
impl RuntimeBackend for LateObservation {
    fn run(
        &mut self,
        binding: HotkeyBinding,
        stop: &AtomicBool,
        sink: &mut dyn FnMut(RuntimeEvent),
    ) -> Result<(), dasru::hotkeys::HotkeyErrorCode> {
        while !stop.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        sink(RuntimeEvent::Observation(HotkeyEvent::Accepted(binding)));
        Ok(())
    }
}
#[test]
fn stop_discards_an_in_flight_late_observation() {
    let mut runtime = HotkeyRuntime::default();
    runtime
        .start_with(
            HotkeyBinding::parse("Ctrl+Alt+F9").unwrap(),
            LateObservation,
        )
        .unwrap();
    runtime.stop().unwrap();
    assert_eq!(runtime.snapshot().accepted, 0);
    assert_eq!(runtime.snapshot().state, HotkeyState::Disabled);
}

struct FailedCleanup;
impl RuntimeBackend for FailedCleanup {
    fn run(
        &mut self,
        _: HotkeyBinding,
        _: &AtomicBool,
        _: &mut dyn FnMut(RuntimeEvent),
    ) -> Result<(), dasru::hotkeys::HotkeyErrorCode> {
        Err(dasru::hotkeys::HotkeyErrorCode::ShutdownFailed)
    }
}
#[test]
fn cleanup_failure_is_not_reported_as_success() {
    let mut runtime = HotkeyRuntime::default();
    runtime
        .start_with(HotkeyBinding::parse("Ctrl+Alt+F9").unwrap(), FailedCleanup)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.is_running() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        runtime.stop(),
        Err(dasru::hotkeys::HotkeyErrorCode::ShutdownFailed)
    );
}
