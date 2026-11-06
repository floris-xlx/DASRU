#![cfg(feature = "desktop-ui")]
use dasru::{
    gui::{
        controller::{Controller, MonitorService},
        model::{Connection, Observation},
    },
    probe::ProbeStatus,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct SlowService {
    calls: Arc<AtomicUsize>,
    release: Arc<AtomicBool>,
}
impl MonitorService for SlowService {
    fn observe(&mut self, cancel: &AtomicBool) -> Result<Observation, &'static str> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Observation::new(ProbeStatus::NoProcess))
    }
}

#[test]
fn slow_worker_is_nonblocking_refreshes_coalesce_and_shutdown_cancels() {
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let mut controller = Controller::start(
        SlowService {
            calls: calls.clone(),
            release: release.clone(),
        },
        Duration::from_secs(30),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let start = Instant::now();
    for _ in 0..100 {
        controller.refresh();
        let _ = controller.snapshot();
    }
    assert!(start.elapsed() < Duration::from_millis(100));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    release.store(true, Ordering::SeqCst);
    while calls.load(Ordering::SeqCst) < 2 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    controller.shutdown().unwrap();
    controller.shutdown().unwrap();
    assert!(calls.load(Ordering::SeqCst) <= 2);
    assert_eq!(
        controller.snapshot().connection(Instant::now()),
        Connection::Stopped
    );
}

struct PanicService;
impl MonitorService for PanicService {
    fn observe(&mut self, _: &AtomicBool) -> Result<Observation, &'static str> {
        panic!("private failure detail")
    }
}

#[test]
fn panic_is_a_structured_failure_and_does_not_crash_caller() {
    let mut controller = Controller::start(PanicService, Duration::from_secs(30)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while controller.snapshot().last_error.is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(controller.snapshot().last_error, Some("WORKER_PANICKED"));
    controller.shutdown().unwrap();
}
