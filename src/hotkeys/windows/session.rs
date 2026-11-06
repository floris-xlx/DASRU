use super::{BackendEvent, FOCUS_GENERATION, WindowsBackend};
use crate::hotkeys::{
    HotkeyErrorCode,
    binding::HotkeyBinding,
    diagnostics::{HotkeyDiagnostics, Stage},
    manager::{ForegroundBackend, HotkeyEvent, HotkeyManager, HotkeyState, VerifiedGameProcess},
};
use crate::hotkeys::{
    dispatch::{ActionDispatcher, ConfiguredBinding},
    runtime::{RuntimeActionSink, RuntimeEvent},
};
use crate::{
    capability::{self, BuildEvidence, CapabilityEvidence, FeatureId, ProcessEvidence},
    identity::{BuildRegistry, BuildResolution, ExecutableIdentity},
    process::{
        PinnedProcessCandidate, ProcessInspection, ProcessScanError,
        enumerate_pinned_game_processes, recheck_pinned_process,
    },
};
use std::cell::RefCell;
use std::sync::mpsc;
use std::{
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows_sys::Win32::{
    System::Console::SetConsoleCtrlHandler,
    UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};
struct WindowsForeground {
    pinned: PinnedProcessCandidate,
    expected_pid: u32,
    expected_identity: crate::process::ProcessIdentity,
}

impl ForegroundBackend for WindowsForeground {
    fn verify_game(
        &mut self,
        expected_pid: u32,
        expected_identity: &crate::process::ProcessIdentity,
    ) -> Result<(), HotkeyErrorCode> {
        if expected_pid != self.expected_pid || expected_identity != &self.expected_identity {
            return Err(HotkeyErrorCode::IdentityChanged);
        }
        let current = enumerate_pinned_game_processes().map_err(process_scan_error)?;
        match current.as_slice() {
            [] => return Err(HotkeyErrorCode::NoProcess),
            [candidate] if candidate.candidate.pid == self.expected_pid => {
                match &candidate.candidate.inspection {
                    ProcessInspection::Available { identity }
                        if identity == &self.expected_identity => {}
                    ProcessInspection::Unavailable { failure }
                        if failure.kind == crate::process::ProcessFailureKind::AccessDenied =>
                    {
                        return Err(HotkeyErrorCode::AccessDenied);
                    }
                    ProcessInspection::Unavailable { failure }
                        if failure.kind == crate::process::ProcessFailureKind::ProcessExited =>
                    {
                        return Err(HotkeyErrorCode::ProcessExited);
                    }
                    _ => return Err(HotkeyErrorCode::IdentityChanged),
                }
            }
            [_] => return Err(HotkeyErrorCode::IdentityChanged),
            _ => return Err(HotkeyErrorCode::AmbiguousProcess),
        }
        match recheck_pinned_process(&self.pinned) {
            ProcessInspection::Available { identity } if identity == self.expected_identity => {
                Ok(())
            }
            ProcessInspection::Unavailable { failure }
                if failure.kind == crate::process::ProcessFailureKind::AccessDenied =>
            {
                Err(HotkeyErrorCode::AccessDenied)
            }
            ProcessInspection::Unavailable { failure }
                if failure.kind == crate::process::ProcessFailureKind::ProcessExited =>
            {
                Err(HotkeyErrorCode::ProcessExited)
            }
            _ => Err(HotkeyErrorCode::IdentityChanged),
        }
    }

    fn foreground_pid(&mut self) -> Result<Option<u32>, HotkeyErrorCode> {
        let window = unsafe {
            // SAFETY: GetForegroundWindow has no preconditions.
            GetForegroundWindow()
        };
        if window.is_null() {
            return Ok(None);
        }
        let mut pid = 0;
        let thread = unsafe {
            // SAFETY: window is non-null and pid points to writable storage.
            GetWindowThreadProcessId(window, &mut pid)
        };
        if thread == 0 || pid == 0 {
            Err(HotkeyErrorCode::ForegroundUnavailable)
        } else {
            Ok(Some(pid))
        }
    }
}

fn attach_to_game(
    cancel: &AtomicBool,
) -> Result<(VerifiedGameProcess, WindowsForeground), HotkeyErrorCode> {
    let mut candidates = enumerate_pinned_game_processes().map_err(process_scan_error)?;
    if candidates.is_empty() {
        return Err(HotkeyErrorCode::NoProcess);
    }
    if candidates.len() != 1 {
        return Err(HotkeyErrorCode::AmbiguousProcess);
    }
    let pinned = candidates.pop().ok_or(HotkeyErrorCode::NoProcess)?;
    let identity = match &pinned.candidate.inspection {
        ProcessInspection::Available { identity } => identity.clone(),
        ProcessInspection::Unavailable { failure }
            if failure.kind == crate::process::ProcessFailureKind::AccessDenied =>
        {
            return Err(HotkeyErrorCode::AccessDenied);
        }
        ProcessInspection::Unavailable { failure }
            if failure.kind == crate::process::ProcessFailureKind::ProcessExited =>
        {
            return Err(HotkeyErrorCode::ProcessExited);
        }
        ProcessInspection::Unavailable { .. } => {
            return Err(HotkeyErrorCode::ProcessInspectionFailed);
        }
    };
    let executable = ExecutableIdentity::inspect_cancellable(&identity.executable_path, cancel)
        .map_err(|error| match error {
            crate::identity::ExecutableError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                HotkeyErrorCode::AccessDenied
            }
            _ => HotkeyErrorCode::ExecutableInspectionFailed,
        })?;
    let registry = BuildRegistry::from_toml(include_str!("../../../data/builds.toml"))
        .map_err(|_| HotkeyErrorCode::ProcessInspectionFailed)?;
    let build = match registry.resolve(&executable) {
        BuildResolution::ExactMatch { build } => build,
        BuildResolution::UnknownBuild | BuildResolution::IdentityInconsistent { .. } => {
            return Err(HotkeyErrorCode::UnknownBuild);
        }
    };
    if !matches!(
        recheck_pinned_process(&pinned),
        ProcessInspection::Available { identity: ref current } if current == &identity
    ) {
        return Err(HotkeyErrorCode::IdentityChanged);
    }
    let pid = pinned.candidate.pid;
    Ok((
        VerifiedGameProcess {
            pid,
            identity: identity.clone(),
            build_id: build.id,
        },
        WindowsForeground {
            pinned,
            expected_pid: pid,
            expected_identity: identity,
        },
    ))
}

fn process_scan_error(error: ProcessScanError) -> HotkeyErrorCode {
    match error {
        ProcessScanError::UnsupportedPlatform => HotkeyErrorCode::UnsupportedPlatform,
        ProcessScanError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            HotkeyErrorCode::AccessDenied
        }
        ProcessScanError::Io { .. } => HotkeyErrorCode::ProcessInspectionFailed,
    }
}

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn console_control(_control: u32) -> i32 {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
    1
}

pub(crate) fn run_cli(configured: Vec<ConfiguredBinding>) -> ExitCode {
    let bindings: Vec<_> = configured.iter().map(|entry| entry.binding).collect();
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    let handler = unsafe {
        // SAFETY: static callback only sets an atomic flag.
        SetConsoleCtrlHandler(Some(console_control), 1)
    };
    if handler == 0 {
        eprintln!("HOTKEY_SHUTDOWN_FAILED");
        return ExitCode::FAILURE;
    }
    let result = run_session(configured, &STOP_REQUESTED, &mut |event| {
        use crate::hotkeys::runtime::RuntimeEvent;
        match event {
            RuntimeEvent::State(state) => print_runtime_state(state, &bindings),
            // Accepted output accompanies Action below to preserve CLI ordering without duplicates.
            RuntimeEvent::Observation(HotkeyEvent::Accepted(_)) => {}
            RuntimeEvent::Observation(event) => print_hotkey_event(event),
            RuntimeEvent::Action(event) => {
                print_hotkey_event(HotkeyEvent::Accepted(event.binding));
                let action = match event.action {
                    crate::hotkeys::binding::HotkeyAction::Observe => "observe",
                };
                println!("HOTKEY_ACTION_DISPATCHED id={} action={action}", event.id);
            }
            RuntimeEvent::Failure(code) => eprintln!("{code}"),
            RuntimeEvent::Summary(summary) => println!("{summary}"),
        }
    });
    unsafe {
        // SAFETY: removes the same static callback installed above.
        SetConsoleCtrlHandler(Some(console_control), 0);
    }
    result
}

pub(crate) fn run_session(
    configured: Vec<ConfiguredBinding>,
    stop: &AtomicBool,
    sink: &mut dyn FnMut(crate::hotkeys::runtime::RuntimeEvent),
) -> ExitCode {
    let sink = RefCell::new(sink);
    let bindings: Vec<_> = configured.iter().map(|entry| entry.binding).collect();
    let (game, foreground) = match attach_to_game(stop) {
        Ok(attached) => attached,
        Err(code) => {
            sink.borrow_mut()(RuntimeEvent::Failure(code));
            return ExitCode::FAILURE;
        }
    };
    let diagnostics = Arc::new(HotkeyDiagnostics::default());
    let mut backend =
        match WindowsBackend::start_with_diagnostics(&bindings, Arc::clone(&diagnostics)) {
            Ok(backend) => backend,
            Err(code) => {
                sink.borrow_mut()(RuntimeEvent::Failure(code));
                return ExitCode::FAILURE;
            }
        };
    let mut manager =
        match HotkeyManager::new(bindings.clone(), game, backend.registrar(), foreground) {
            Ok(manager) => manager,
            Err(_) => {
                sink.borrow_mut()(RuntimeEvent::Failure(HotkeyErrorCode::InvalidBinding));
                let _ = backend.shutdown();
                return ExitCode::FAILURE;
            }
        };
    let mut dispatcher = ActionDispatcher::new(configured, RuntimeActionSink(&sink));
    if let Err(failure) = manager.start() {
        sink.borrow_mut()(RuntimeEvent::Failure(failure.code));
        if failure.code != HotkeyErrorCode::RegistrationConflict {
            let _ = manager.shutdown();
            let _ = backend.shutdown();
            return ExitCode::FAILURE;
        }
    }

    let mut last_reported_state = None;
    let mut last_focus_generation = FOCUS_GENERATION.load(Ordering::Acquire);
    report_state_change(
        manager.state(),
        &mut last_reported_state,
        &sink,
        &diagnostics,
    );
    let mut exit_code = ExitCode::SUCCESS;
    while !stop.load(Ordering::Acquire) {
        let received = backend.recv_timeout(Duration::from_millis(250));
        if stop.load(Ordering::Acquire) {
            break;
        }
        let focus_generation = FOCUS_GENERATION.load(Ordering::Acquire);
        let focus_changes = focus_generation.wrapping_sub(last_focus_generation);
        if focus_changes != 0 {
            diagnostics.increment_by(Stage::FocusChanged, focus_changes);
            last_focus_generation = focus_generation;
        }
        let focus_changed = matches!(&received, Ok(BackendEvent::ForegroundChanged));
        match received {
            Ok(BackendEvent::KeyboardInput {
                virtual_key,
                is_down,
                generation,
            }) => {
                let current_generation = FOCUS_GENERATION.load(Ordering::Acquire);
                let event = manager.handle_keyboard_input_at_generation(
                    virtual_key,
                    is_down,
                    generation,
                    current_generation,
                );
                if generation != current_generation {
                    diagnostics.increment(Stage::StaleDropped);
                } else {
                    diagnostics.increment(Stage::InputProcessed);
                    if generation != FOCUS_GENERATION.load(Ordering::Acquire) {
                        diagnostics.increment(Stage::StaleDropped);
                        manager.reset_keyboard_state();
                    } else if let Some(event) = event {
                        diagnostics.increment(Stage::ChordCandidate);
                        match event {
                            HotkeyEvent::Accepted(_) => {
                                if dispatcher.dispatch(
                                    event,
                                    generation,
                                    FOCUS_GENERATION.load(Ordering::Acquire),
                                ) {
                                    diagnostics.increment(Stage::Accepted);
                                    sink.borrow_mut()(RuntimeEvent::Observation(event));
                                } else {
                                    diagnostics.increment(Stage::StaleDropped);
                                    manager.reset_keyboard_state();
                                }
                            }
                            HotkeyEvent::Rejected { code, .. } => {
                                diagnostics.increment(Stage::Rejected);
                                if matches!(
                                    code,
                                    HotkeyErrorCode::IdentityChanged
                                        | HotkeyErrorCode::NoProcess
                                        | HotkeyErrorCode::AmbiguousProcess
                                        | HotkeyErrorCode::AccessDenied
                                        | HotkeyErrorCode::ProcessExited
                                ) {
                                    diagnostics.increment(Stage::IdentityRejected);
                                }
                                sink.borrow_mut()(RuntimeEvent::Observation(event));
                            }
                        }
                    }
                }
                report_state_change(
                    manager.state(),
                    &mut last_reported_state,
                    &sink,
                    &diagnostics,
                );
            }
            Ok(BackendEvent::ForegroundChanged) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if focus_changed {
                    manager.reset_keyboard_state();
                }
                if let Err(failure) = manager.reconcile() {
                    sink.borrow_mut()(RuntimeEvent::Failure(failure.code));
                    if failure.code != HotkeyErrorCode::RegistrationConflict {
                        exit_code = ExitCode::FAILURE;
                        break;
                    }
                    report_state_change(
                        manager.state(),
                        &mut last_reported_state,
                        &sink,
                        &diagnostics,
                    );
                } else {
                    report_state_change(
                        manager.state(),
                        &mut last_reported_state,
                        &sink,
                        &diagnostics,
                    );
                }
            }
            Ok(BackendEvent::Failed(code)) => {
                sink.borrow_mut()(RuntimeEvent::Failure(code));
                exit_code = ExitCode::FAILURE;
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                sink.borrow_mut()(RuntimeEvent::Failure(
                    HotkeyErrorCode::MessageLoopStartFailed,
                ));
                exit_code = ExitCode::FAILURE;
                break;
            }
        }
    }

    if let Err(failure) = manager.shutdown() {
        sink.borrow_mut()(RuntimeEvent::Failure(failure.code));
        exit_code = ExitCode::FAILURE;
    }
    dispatcher.shutdown();
    report_state_change(
        manager.state(),
        &mut last_reported_state,
        &sink,
        &diagnostics,
    );
    if let Err(code) = backend.shutdown() {
        sink.borrow_mut()(RuntimeEvent::Failure(code));
        exit_code = ExitCode::FAILURE;
    }
    sink.borrow_mut()(RuntimeEvent::Summary(diagnostics.summary()));
    exit_code
}

fn report_state_change(
    state: HotkeyState,
    last_reported: &mut Option<HotkeyState>,
    sink: &RefCell<&mut dyn FnMut(RuntimeEvent)>,
    diagnostics: &HotkeyDiagnostics,
) {
    if *last_reported == Some(state) {
        return;
    }
    match *last_reported {
        Some(previous) if previous != HotkeyState::Active && state == HotkeyState::Active => {
            diagnostics.increment(Stage::Activated);
        }
        Some(previous) if previous == HotkeyState::Active && state != HotkeyState::Active => {
            diagnostics.increment(Stage::Deactivated);
        }
        None if state == HotkeyState::Active => diagnostics.increment(Stage::Activated),
        _ => {}
    }
    *last_reported = Some(state);
    sink.borrow_mut()(RuntimeEvent::State(state));
}

fn print_runtime_state(state: HotkeyState, bindings: &[HotkeyBinding]) {
    let evidence = CapabilityEvidence {
        process: ProcessEvidence::Verified,
        build: BuildEvidence::ExactMatch,
        hotkey_registration: state.registration_evidence(),
        ..CapabilityEvidence::default()
    };
    let capability = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);
    let status = match capability.status {
        capability::FeatureStatus::Supported => "SUPPORTED",
        capability::FeatureStatus::Unsupported => "UNSUPPORTED",
        capability::FeatureStatus::Unknown => "UNKNOWN",
        capability::FeatureStatus::Experimental => "EXPERIMENTAL",
    };
    let listed = bindings
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    println!("HOTKEY_STATE state={state:?} capability={status} bindings={listed}");
}

fn print_hotkey_event(event: HotkeyEvent) {
    match event {
        HotkeyEvent::Accepted(binding) => println!("HOTKEY_ACCEPTED binding={binding}"),
        HotkeyEvent::Rejected { binding, code } => {
            println!("HOTKEY_REJECTED binding={binding} code={code}");
        }
    }
}
