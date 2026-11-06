#![cfg(feature = "desktop-ui")]
use dasru::{
    gui::{
        controller::MonitorService,
        services::{ProbeBackend, ProbeService},
    },
    probe::ProbeReport,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Backend {
    identity: Arc<AtomicUsize>,
    hashes: Arc<AtomicUsize>,
}
impl ProbeBackend for Backend {
    type Key = usize;
    fn key(&mut self) -> Option<usize> {
        Some(self.identity.load(Ordering::SeqCst))
    }
    fn probe(&mut self, _: &AtomicBool) -> ProbeReport {
        self.hashes.fetch_add(1, Ordering::SeqCst);
        let mut report = dasru::probe::run_probe();
        report.status = dasru::probe::ProbeStatus::UnknownBuild;
        report
    }
    fn report_key(&mut self, _: &ProbeReport) -> Option<usize> {
        self.key()
    }
}
#[test]
fn unchanged_identity_avoids_hashing_and_changed_identity_reverifies() {
    let identity = Arc::new(AtomicUsize::new(1));
    let hashes = Arc::new(AtomicUsize::new(0));
    let mut service = ProbeService::new(Backend {
        identity: identity.clone(),
        hashes: hashes.clone(),
    });
    let cancel = AtomicBool::new(false);
    service.observe(&cancel).unwrap();
    service.observe(&cancel).unwrap();
    assert_eq!(hashes.load(Ordering::SeqCst), 1);
    identity.store(2, Ordering::SeqCst);
    service.observe(&cancel).unwrap();
    assert_eq!(hashes.load(Ordering::SeqCst), 2);
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(service.observe(&cancel).unwrap_err(), "WORKER_CANCELLED");
    assert_eq!(hashes.load(Ordering::SeqCst), 2);
}
