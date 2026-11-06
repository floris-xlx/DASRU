#![cfg(feature = "desktop-ui")]
use dasru::gui::{
    controller::{Controller, ExportStatus, MonitorService},
    model::Observation,
};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
struct Service;
impl MonitorService for Service {
    fn observe(&mut self, _: &AtomicBool) -> Result<Observation, &'static str> {
        Ok(Observation::new(dasru::probe::ProbeStatus::NoProcess))
    }
}
#[test]
fn export_runs_off_thread_and_preserves_no_overwrite() {
    let path = std::env::temp_dir().join(format!("dasru-gui-export-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut controller = Controller::start(Service, Duration::from_secs(30)).unwrap();
    controller.export(path.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while controller.export_status() == ExportStatus::Pending {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(controller.export_status(), ExportStatus::Complete);
    let bytes = std::fs::read(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json.get("executable_compatibility").is_some());
    assert!(!String::from_utf8_lossy(&bytes).contains("executable_path"));
    controller.export(path.clone()).unwrap();
    while controller.export_status() == ExportStatus::Pending {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        controller.export_status(),
        ExportStatus::Failed("SUPPORT_EXPORT_PATH_EXISTS")
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    controller.shutdown().unwrap();
    std::fs::remove_file(path).unwrap();
}
