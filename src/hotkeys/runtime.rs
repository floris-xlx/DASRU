use super::{
    HotkeyErrorCode,
    binding::HotkeyBinding,
    manager::{HotkeyEvent, HotkeyState},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    State(HotkeyState),
    Observation(HotkeyEvent),
    Action(super::dispatch::ActionEvent),
    Failure(HotkeyErrorCode),
    Summary(String),
}

#[cfg(any(windows, test))]
pub(crate) struct RuntimeActionSink<'a, 'b>(
    pub &'a std::cell::RefCell<&'b mut dyn FnMut(RuntimeEvent)>,
);
#[cfg(any(windows, test))]
impl super::dispatch::ActionEventSink for RuntimeActionSink<'_, '_> {
    fn emit(&mut self, event: super::dispatch::ActionEvent) {
        self.0.borrow_mut()(RuntimeEvent::Action(event));
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub state: HotkeyState,
    pub accepted: u64,
    pub rejected: u64,
    pub last_event: Option<HotkeyEvent>,
    pub last_error: Option<HotkeyErrorCode>,
    pub diagnostics: String,
}
impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            state: HotkeyState::Disabled,
            accepted: 0,
            rejected: 0,
            last_event: None,
            last_error: None,
            diagnostics: String::new(),
        }
    }
}

pub trait RuntimeBackend: Send + 'static {
    fn run(
        &mut self,
        binding: HotkeyBinding,
        stop: &AtomicBool,
        sink: &mut dyn FnMut(RuntimeEvent),
    ) -> Result<(), HotkeyErrorCode>;
}

#[derive(Default)]
pub struct HotkeyRuntime {
    snapshot: Arc<Mutex<RuntimeSnapshot>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HotkeyRuntime {
    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }
    pub fn start(&mut self, binding: HotkeyBinding) -> Result<(), HotkeyErrorCode> {
        self.start_with(binding, NativeRuntime)
    }
    pub fn start_with(
        &mut self,
        binding: HotkeyBinding,
        mut backend: impl RuntimeBackend,
    ) -> Result<(), HotkeyErrorCode> {
        if self.is_running() {
            return Err(HotkeyErrorCode::RegistrationConflict);
        }
        self.stop()?;
        super::binding::validate_bindings(&[binding])
            .map_err(|_| HotkeyErrorCode::InvalidBinding)?;
        self.stop.store(false, Ordering::Release);
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = RuntimeSnapshot {
            state: HotkeyState::Registering,
            ..Default::default()
        };
        let stop = self.stop.clone();
        let snapshot = self.snapshot.clone();
        self.worker = Some(
            thread::Builder::new()
                .name("dasru-observation".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        backend.run(binding, &stop, &mut |event| {
                            let mut snapshot = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                            if stop.load(Ordering::Acquire)
                                && matches!(
                                    &event,
                                    RuntimeEvent::Observation(_) | RuntimeEvent::Action(_)
                                )
                            {
                                return;
                            }
                            match event {
                                RuntimeEvent::State(state) => {
                                    snapshot.state = state;
                                    if state == HotkeyState::Active {
                                        snapshot.last_error = None;
                                    }
                                }
                                RuntimeEvent::Observation(event) => {
                                    match event {
                                        HotkeyEvent::Accepted(_) => {
                                            snapshot.accepted = snapshot.accepted.saturating_add(1)
                                        }
                                        HotkeyEvent::Rejected { .. } => {
                                            snapshot.rejected = snapshot.rejected.saturating_add(1)
                                        }
                                    }
                                    snapshot.last_event = Some(event);
                                }
                                RuntimeEvent::Failure(code) => snapshot.last_error = Some(code),
                                RuntimeEvent::Summary(summary) => snapshot.diagnostics = summary,
                                RuntimeEvent::Action(_) => {}
                            }
                        })
                    }))
                    .unwrap_or(Err(HotkeyErrorCode::MessageLoopStartFailed));
                    let mut snapshot = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                    if let Err(code) = result {
                        snapshot.last_error = Some(code);
                        snapshot.state = HotkeyState::Error;
                    } else {
                        snapshot.state = HotkeyState::Disabled;
                    }
                })
                .map_err(|_| HotkeyErrorCode::MessageLoopStartFailed)?,
        );
        Ok(())
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        if self.is_running() {
            self.snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .state = HotkeyState::Stopping;
        }
    }
    pub fn stop(&mut self) -> Result<(), HotkeyErrorCode> {
        self.request_stop();
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| HotkeyErrorCode::ShutdownFailed)?;
        }
        match self.snapshot().last_error {
            Some(error @ (HotkeyErrorCode::ShutdownFailed | HotkeyErrorCode::ShutdownTimeout)) => {
                Err(error)
            }
            _ => Ok(()),
        }
    }
}
impl Drop for HotkeyRuntime {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct NativeRuntime;
impl RuntimeBackend for NativeRuntime {
    fn run(
        &mut self,
        binding: HotkeyBinding,
        stop: &AtomicBool,
        sink: &mut dyn FnMut(RuntimeEvent),
    ) -> Result<(), HotkeyErrorCode> {
        #[cfg(windows)]
        {
            let mut failure = None;
            let configured = super::dispatch::ConfiguredBinding {
                id: "gui-observation".into(),
                binding,
                action: binding.action(),
            };
            let result = super::windows::run_session(vec![configured], stop, &mut |event| {
                if let RuntimeEvent::Failure(code) = &event {
                    failure = Some(*code);
                }
                sink(event);
            });
            if result == std::process::ExitCode::SUCCESS {
                Ok(())
            } else {
                Err(failure.unwrap_or(HotkeyErrorCode::MessageLoopStartFailed))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (binding, stop, sink);
            Err(HotkeyErrorCode::UnsupportedPlatform)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::dispatch::{ActionDispatcher, ConfiguredBinding};
    #[test]
    fn configured_actions_keep_their_id_and_focus_generation_fence() {
        let binding = HotkeyBinding::parse("Ctrl+Alt+F9").unwrap();
        let mut events = Vec::new();
        let mut callback = |event| events.push(event);
        {
            let sink = std::cell::RefCell::new(&mut callback as &mut dyn FnMut(RuntimeEvent));
            let mut dispatcher = ActionDispatcher::new(
                vec![ConfiguredBinding {
                    id: "configured-observe".into(),
                    binding,
                    action: binding.action(),
                }],
                RuntimeActionSink(&sink),
            );
            assert!(!dispatcher.dispatch(HotkeyEvent::Accepted(binding), 1, 2));
            assert!(dispatcher.dispatch(HotkeyEvent::Accepted(binding), 2, 2));
            dispatcher.shutdown();
            assert!(!dispatcher.dispatch(HotkeyEvent::Accepted(binding), 2, 2));
        }
        assert!(
            matches!(events.as_slice(),[RuntimeEvent::Action(event)] if event.id=="configured-observe" && event.binding==binding)
        );
    }
}
