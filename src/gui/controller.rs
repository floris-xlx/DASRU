use super::model::{Model, Observation};
use crate::hotkeys::{
    binding::HotkeyBinding,
    runtime::{HotkeyRuntime, RuntimeSnapshot},
};
use std::path::PathBuf;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub trait MonitorService: Send + 'static {
    fn export(&mut self, path: &std::path::Path) -> Result<(), &'static str> {
        crate::support::export_json(path).map_err(|error| error.code())
    }
    fn observe(&mut self, cancel: &AtomicBool) -> Result<Observation, &'static str>;
    fn observe_with_progress(
        &mut self,
        cancel: &AtomicBool,
        _progress: &mut dyn FnMut(super::model::Connection),
    ) -> Result<Observation, &'static str> {
        self.observe(cancel)
    }
}

struct Shared {
    model: Mutex<Model>,
    pending: Mutex<bool>,
    wake: Condvar,
    cancel: AtomicBool,
    export: Mutex<(Option<PathBuf>, ExportStatus)>,
    interval: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportStatus {
    #[default]
    Idle,
    Pending,
    Complete,
    Failed(&'static str),
}

pub struct Controller {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    hotkeys: HotkeyRuntime,
}

impl Controller {
    pub fn start(mut service: impl MonitorService, interval: Duration) -> std::io::Result<Self> {
        let interval = interval.clamp(Duration::from_secs(1), Duration::from_secs(30));
        let mut model = Model::with_stale_after(interval * 3);
        model.begin_refresh();
        let shared = Arc::new(Shared {
            model: Mutex::new(model),
            pending: Mutex::new(true),
            wake: Condvar::new(),
            cancel: AtomicBool::new(false),
            export: Mutex::new((None, ExportStatus::Idle)),
            interval: AtomicU64::new(interval.as_secs()),
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("dasru-monitor".into())
            .spawn(move || {
                loop {
                    let mut pending = state
                        .pending
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if !*pending && !state.cancel.load(Ordering::Acquire) {
                        let (guard, timeout) = state
                            .wake
                            .wait_timeout_while(
                                pending,
                                Duration::from_secs(state.interval.load(Ordering::Acquire)),
                                |pending| !*pending && !state.cancel.load(Ordering::Acquire),
                            )
                            .unwrap_or_else(|error| error.into_inner());
                        pending = guard;
                        if timeout.timed_out() {
                            state
                                .model
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .begin_refresh();
                        }
                    }
                    if state.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    *pending = false;
                    let generation = state
                        .model
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .generation();
                    drop(pending);
                    let export = state
                        .export
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                        .take();
                    if let Some(path) = export {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            service.export(&path)
                        }))
                        .map(|result| match result {
                            Ok(()) => ExportStatus::Complete,
                            Err(error) => ExportStatus::Failed(error),
                        })
                        .unwrap_or(ExportStatus::Failed("SUPPORT_EXPORT_FAILED"));
                        state.export.lock().unwrap_or_else(|e| e.into_inner()).1 = result;
                    }
                    if state.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        service.observe_with_progress(&state.cancel, &mut |phase| {
                            state
                                .model
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .progress(generation, phase);
                        })
                    }))
                    .unwrap_or(Err("WORKER_PANICKED"));
                    state
                        .model
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .apply(generation, result, Instant::now());
                }
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
            hotkeys: HotkeyRuntime::default(),
        })
    }

    pub fn set_interval(&self, seconds: u64) -> Result<(), &'static str> {
        if !(1..=30).contains(&seconds) {
            return Err("SETTINGS_INTERVAL_INVALID");
        }
        self.shared.interval.store(seconds, Ordering::Release);
        self.shared
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stale_after = Duration::from_secs(seconds * 3);
        self.refresh();
        Ok(())
    }
    pub fn export(&self, path: PathBuf) -> Result<(), &'static str> {
        if path.as_os_str().is_empty() {
            return Err("SUPPORT_DESTINATION_REQUIRED");
        }
        let mut export = self.shared.export.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.cancel.load(Ordering::Acquire) {
            return Err("CONTROLLER_STOPPED");
        }
        if export.1 == ExportStatus::Pending {
            return Err("SUPPORT_EXPORT_PENDING");
        }
        *export = (Some(path), ExportStatus::Pending);
        drop(export);
        self.refresh();
        Ok(())
    }
    pub fn export_status(&self) -> ExportStatus {
        self.shared
            .export
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1
    }
    pub fn listener_snapshot(&self) -> RuntimeSnapshot {
        self.hotkeys.snapshot()
    }
    pub fn listener_running(&self) -> bool {
        self.hotkeys.is_running()
    }
    pub fn start_listener(&mut self, binding: &str) -> Result<(), &'static str> {
        if self.shared.cancel.load(Ordering::Acquire) {
            return Err("CONTROLLER_STOPPED");
        }
        if self.snapshot().connection(Instant::now()) != super::model::Connection::Verified {
            return Err("HOTKEY_VERIFIED_GAME_REQUIRED");
        }
        let binding = HotkeyBinding::parse(binding).map_err(|_| "HOTKEY_INVALID_BINDING")?;
        self.hotkeys.start(binding).map_err(|error| error.as_str())
    }
    pub fn stop_listener(&self) {
        self.hotkeys.request_stop();
    }

    pub fn refresh(&self) {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.shared.cancel.load(Ordering::Acquire) {
            return;
        }
        if !*pending {
            self.shared
                .model
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .begin_refresh();
            *pending = true;
        }
        self.shared.wake.notify_one();
    }

    pub fn snapshot(&self) -> Model {
        self.shared
            .model
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn shutdown(&mut self) -> Result<(), &'static str> {
        let joining = self.worker.is_some();
        {
            let _pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.shared.cancel.store(true, Ordering::Release);
            let mut export = self.shared.export.lock().unwrap_or_else(|e| e.into_inner());
            if export.0.take().is_some() {
                export.1 = ExportStatus::Failed("SUPPORT_EXPORT_CANCELLED");
            }
            drop(export);
            self.shared
                .model
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .shutdown();
            self.shared.wake.notify_one();
        }
        if joining {
            tracing::info!(target: "dasru::shutdown", phase = "MONITOR_CANCEL_REQUESTED");
            tracing::info!(target: "dasru::shutdown", phase = "HOTKEY_STOP_REQUESTED");
        }
        let hotkeys = self.hotkeys.stop().map_err(|error| error.as_str());
        if joining {
            tracing::info!(target: "dasru::shutdown", phase = "HOTKEY_STOP_COMPLETED", success = hotkeys.is_ok(), code = hotkeys.err());
        }
        if let Some(worker) = self.worker.take() {
            let joined = worker.join().map_err(|_| "WORKER_SHUTDOWN_FAILED");
            tracing::info!(target: "dasru::shutdown", phase = "MONITOR_JOIN_COMPLETED", success = joined.is_ok(), code = joined.err());
            joined?;
        }
        hotkeys
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
