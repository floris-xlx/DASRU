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

#[test]
fn shutdown_cancels_queued_export_and_rejects_new_work() {
    use dasru::gui::controller::ExportStatus;
    use std::sync::mpsc;

    struct UntilCancelled(mpsc::Sender<()>);
    impl MonitorService for UntilCancelled {
        fn observe(&mut self, cancel: &AtomicBool) -> Result<Observation, &'static str> {
            self.0.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !cancel.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < deadline,
                    "controller failed to cancel probe"
                );
                std::thread::yield_now();
            }
            Err("PROBE_CANCELLED")
        }
    }
    let (ready, entered) = mpsc::channel();
    let mut controller = Controller::start(UntilCancelled(ready), Duration::from_secs(30)).unwrap();
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    controller
        .export("unused-support-destination.json".into())
        .unwrap();
    controller.shutdown().unwrap();
    assert_eq!(
        controller.export_status(),
        ExportStatus::Failed("SUPPORT_EXPORT_CANCELLED")
    );
    assert_eq!(
        controller.export("unused-support-destination.json".into()),
        Err("CONTROLLER_STOPPED")
    );
    controller.refresh();
    controller.shutdown().unwrap();
    assert_eq!(
        controller.snapshot().connection(Instant::now()),
        Connection::Stopped
    );
    assert!(!controller.listener_running());
}

#[test]
fn shutdown_during_export_joins_without_starting_another_probe() {
    use std::sync::mpsc;
    struct ExportService {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        probes: Arc<AtomicUsize>,
    }
    impl MonitorService for ExportService {
        fn observe(&mut self, _: &AtomicBool) -> Result<Observation, &'static str> {
            self.probes.fetch_add(1, Ordering::SeqCst);
            Ok(Observation::new(ProbeStatus::NoProcess))
        }
        fn export(&mut self, _: &std::path::Path) -> Result<(), &'static str> {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        }
    }
    let (entered, entry) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let probes = Arc::new(AtomicUsize::new(0));
    let controller = Controller::start(
        ExportService {
            entered,
            release: gate,
            probes: probes.clone(),
        },
        Duration::from_secs(30),
    )
    .unwrap();
    controller
        .export("unused-support-destination.json".into())
        .unwrap();
    entry.recv_timeout(Duration::from_secs(2)).unwrap();
    let before = probes.load(Ordering::SeqCst);
    let (done, completion) = mpsc::channel();
    let shutdown = std::thread::spawn(move || {
        let mut controller = controller;
        controller.shutdown().unwrap();
        done.send(controller.export_status()).unwrap();
    });
    let early = completion.recv_timeout(Duration::from_millis(100)).ok();
    release.send(()).unwrap();
    shutdown.join().unwrap();
    assert!(early.is_none(), "shutdown detached an active export");
    assert_eq!(
        completion.recv().unwrap(),
        dasru::gui::controller::ExportStatus::Complete
    );
    assert_eq!(
        probes.load(Ordering::SeqCst),
        before,
        "probe began after cancellation during export"
    );
}
