use std::{
    collections::HashSet,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError, HWND},
    System::{Console::SetConsoleCtrlHandler, Threading::GetCurrentThreadId},
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
        WindowsAndMessaging::{
            EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
            MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WINEVENT_OUTOFCONTEXT, WM_APP,
            WM_HOTKEY,
        },
    },
};

use crate::{
    capability::{self, BuildEvidence, CapabilityEvidence, FeatureId, ProcessEvidence},
    identity::{BuildRegistry, BuildResolution, ExecutableIdentity},
    process::{
        PinnedProcessCandidate, ProcessInspection, ProcessScanError,
        enumerate_pinned_game_processes, recheck_pinned_process,
    },
};

use super::{
    HotkeyErrorCode,
    binding::HotkeyBinding,
    manager::{
        ForegroundBackend, HotkeyEvent, HotkeyManager, HotkeyState, RegistrationBackend,
        RegistrationFailure, VerifiedGameProcess,
    },
};

const COMMAND_MESSAGE: u32 = WM_APP + 1;
const FOREGROUND_MESSAGE: u32 = WM_APP + 2;
const STOP_MESSAGE: u32 = WM_APP + 3;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

enum Command {
    Register {
        id: i32,
        binding: HotkeyBinding,
        reply: SyncSender<Result<(), RegistrationFailure>>,
    },
    Unregister {
        id: i32,
        reply: SyncSender<Result<(), RegistrationFailure>>,
    },
}

enum BackendEvent {
    Hotkey(u32),
    ForegroundChanged,
    Failed(HotkeyErrorCode),
}

enum StartupMessage {
    QueueReady {
        thread_id: u32,
    },
    HookReady {
        thread_id: u32,
        result: Result<(), HotkeyErrorCode>,
    },
}

pub(super) struct WindowsBackend {
    thread_id: u32,
    commands: Sender<Command>,
    events: Receiver<BackendEvent>,
    completed: Receiver<Result<(), HotkeyErrorCode>>,
    thread: Option<JoinHandle<()>>,
    stopped: bool,
}

impl WindowsBackend {
    fn start() -> Result<Self, HotkeyErrorCode> {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(2);
        let (completed_tx, completed_rx) = mpsc::sync_channel(1);
        let abort = Arc::new(AtomicBool::new(false));
        let worker_abort = Arc::clone(&abort);
        let worker = thread::Builder::new()
            .name("dasru-hotkeys".to_owned())
            .spawn(move || {
                let result = message_loop(commands_rx, events_tx, startup_tx, worker_abort);
                let _ = completed_tx.send(result);
            })
            .map_err(|_| HotkeyErrorCode::MessageLoopStartFailed)?;

        let (thread_id, hook_ready) = match startup_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(StartupMessage::QueueReady { thread_id }) => (thread_id, false),
            Ok(StartupMessage::HookReady { thread_id, result }) => {
                if let Err(code) = result {
                    let _ = worker.join();
                    return Err(code);
                }
                (thread_id, true)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                abort_startup(worker, completed_rx, &abort, None);
                return Err(HotkeyErrorCode::StartupTimeout);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                return Err(HotkeyErrorCode::MessageLoopStartFailed);
            }
        };
        if !hook_ready {
            match startup_rx.recv_timeout(STARTUP_TIMEOUT) {
                Ok(StartupMessage::HookReady {
                    thread_id: hook_thread_id,
                    result: Ok(()),
                }) if hook_thread_id == thread_id => {}
                Ok(StartupMessage::HookReady {
                    result: Err(code), ..
                }) => {
                    let _ = worker.join();
                    return Err(code);
                }
                Ok(StartupMessage::QueueReady { .. })
                | Ok(StartupMessage::HookReady { result: Ok(()), .. })
                | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    abort_startup(worker, completed_rx, &abort, Some(thread_id));
                    return Err(HotkeyErrorCode::MessageLoopStartFailed);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    abort_startup(worker, completed_rx, &abort, Some(thread_id));
                    return Err(HotkeyErrorCode::StartupTimeout);
                }
            }
        }

        Ok(Self {
            thread_id,
            commands: commands_tx,
            events: events_rx,
            completed: completed_rx,
            thread: Some(worker),
            stopped: false,
        })
    }

    fn registrar(&self) -> WindowsRegistrar {
        WindowsRegistrar {
            thread_id: self.thread_id,
            commands: self.commands.clone(),
        }
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<BackendEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    fn shutdown(&mut self) -> Result<(), HotkeyErrorCode> {
        if self.stopped {
            return Ok(());
        }
        let posted = unsafe {
            // SAFETY: this thread id was acknowledged by the live message-loop thread.
            PostThreadMessageW(self.thread_id, STOP_MESSAGE, 0, 0)
        };
        if posted == 0 {
            return Err(HotkeyErrorCode::ShutdownFailed);
        }
        let result =
            self.completed
                .recv_timeout(SHUTDOWN_TIMEOUT)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => HotkeyErrorCode::ShutdownTimeout,
                    mpsc::RecvTimeoutError::Disconnected => HotkeyErrorCode::ShutdownFailed,
                })?;
        if self
            .thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            return Err(HotkeyErrorCode::ShutdownFailed);
        }
        self.stopped = true;
        result
    }

    #[cfg(test)]
    fn is_stopped(&self) -> bool {
        self.stopped
    }
}

fn abort_startup(
    worker: JoinHandle<()>,
    completed: Receiver<Result<(), HotkeyErrorCode>>,
    abort: &AtomicBool,
    thread_id: Option<u32>,
) {
    abort.store(true, Ordering::SeqCst);
    if let Some(thread_id) = thread_id {
        unsafe {
            // SAFETY: the worker acknowledged this message queue during startup.
            PostThreadMessageW(thread_id, STOP_MESSAGE, 0, 0);
        }
    }
    if completed.recv_timeout(SHUTDOWN_TIMEOUT).is_ok() {
        let _ = worker.join();
    }
}

impl Drop for WindowsBackend {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct WindowsRegistrar {
    thread_id: u32,
    commands: Sender<Command>,
}

impl WindowsRegistrar {
    fn request(
        &mut self,
        command: impl FnOnce(SyncSender<Result<(), RegistrationFailure>>) -> Command,
    ) -> Result<(), RegistrationFailure> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(command(reply_tx))
            .map_err(|_| RegistrationFailure::Failed)?;
        let posted = unsafe {
            // SAFETY: the acknowledged worker owns this thread's message queue.
            PostThreadMessageW(self.thread_id, COMMAND_MESSAGE, 0, 0)
        };
        if posted == 0 {
            return Err(RegistrationFailure::Failed);
        }
        reply_rx
            .recv_timeout(STARTUP_TIMEOUT)
            .unwrap_or(Err(RegistrationFailure::Failed))
    }
}

impl RegistrationBackend for WindowsRegistrar {
    fn register(&mut self, id: u32, binding: HotkeyBinding) -> Result<(), RegistrationFailure> {
        let id = i32::try_from(id).map_err(|_| RegistrationFailure::Failed)?;
        self.request(|reply| Command::Register { id, binding, reply })
    }

    fn unregister(&mut self, id: u32) -> Result<(), RegistrationFailure> {
        let id = i32::try_from(id).map_err(|_| RegistrationFailure::Failed)?;
        self.request(|reply| Command::Unregister { id, reply })
    }
}

fn message_loop(
    commands: Receiver<Command>,
    events: Sender<BackendEvent>,
    startup: SyncSender<StartupMessage>,
    abort: Arc<AtomicBool>,
) -> Result<(), HotkeyErrorCode> {
    let mut message = MSG::default();
    unsafe {
        // SAFETY: PeekMessageW initializes this thread's message queue before PostThreadMessageW is used.
        PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
    }
    let thread_id = unsafe {
        // SAFETY: GetCurrentThreadId has no preconditions.
        GetCurrentThreadId()
    };
    if startup
        .send(StartupMessage::QueueReady { thread_id })
        .is_err()
        || abort.load(Ordering::SeqCst)
    {
        return Err(HotkeyErrorCode::MessageLoopStartFailed);
    }
    let hook = unsafe {
        // SAFETY: callback is static, the event range is valid, and the hook is owned by this thread.
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(foreground_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_null() {
        let _ = startup.send(StartupMessage::HookReady {
            thread_id,
            result: Err(HotkeyErrorCode::MessageLoopStartFailed),
        });
        return Err(HotkeyErrorCode::MessageLoopStartFailed);
    }
    if abort.load(Ordering::SeqCst)
        || startup
            .send(StartupMessage::HookReady {
                thread_id,
                result: Ok(()),
            })
            .is_err()
    {
        unsafe {
            // SAFETY: hook was successfully created above and has not been released.
            UnhookWinEvent(hook);
        }
        return Err(HotkeyErrorCode::StartupTimeout);
    }

    let mut registered = HashSet::new();
    let mut result = Ok(());
    loop {
        let status = unsafe {
            // SAFETY: message is writable and a null HWND selects this thread's queue.
            GetMessageW(&mut message, std::ptr::null_mut(), 0, 0)
        };
        if status == -1 {
            let _ = events.send(BackendEvent::Failed(
                HotkeyErrorCode::MessageLoopStartFailed,
            ));
            result = Err(HotkeyErrorCode::MessageLoopStartFailed);
            break;
        }
        if status == 0 || message.message == STOP_MESSAGE {
            break;
        }
        match message.message {
            WM_HOTKEY => {
                let _ = events.send(BackendEvent::Hotkey(message.wParam as u32));
            }
            FOREGROUND_MESSAGE => {
                let _ = events.send(BackendEvent::ForegroundChanged);
            }
            COMMAND_MESSAGE => {
                while let Ok(command) = commands.try_recv() {
                    match command {
                        Command::Register { id, binding, reply } => {
                            let success = unsafe {
                                // SAFETY: registration is thread-owned; the id and virtual key are validated scalars.
                                RegisterHotKey(
                                    std::ptr::null_mut(),
                                    id,
                                    binding.modifiers() | MOD_NOREPEAT,
                                    binding.virtual_key(),
                                )
                            };
                            if success != 0 {
                                registered.insert(id);
                                let _ = reply.send(Ok(()));
                            } else {
                                let failure = unsafe {
                                    // SAFETY: GetLastError reads the error from the immediately preceding Win32 call.
                                    GetLastError()
                                };
                                let _ = reply.send(Err(
                                    if failure == ERROR_HOTKEY_ALREADY_REGISTERED {
                                        RegistrationFailure::Conflict
                                    } else {
                                        RegistrationFailure::Failed
                                    },
                                ));
                            }
                        }
                        Command::Unregister { id, reply } => {
                            let success = unsafe {
                                // SAFETY: this id is owned by the current thread's registration set.
                                UnregisterHotKey(std::ptr::null_mut(), id)
                            };
                            if success != 0 {
                                registered.remove(&id);
                                let _ = reply.send(Ok(()));
                            } else {
                                let _ = reply.send(Err(RegistrationFailure::Failed));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for id in registered {
        let success = unsafe {
            // SAFETY: all remaining IDs were registered by this same thread.
            UnregisterHotKey(std::ptr::null_mut(), id)
        };
        if success == 0 {
            result = Err(HotkeyErrorCode::ShutdownFailed);
        }
    }
    if unsafe {
        // SAFETY: hook was created by this thread and is released exactly once here.
        UnhookWinEvent(hook)
    } == 0
    {
        result = Err(HotkeyErrorCode::ShutdownFailed);
    }
    result
}

unsafe extern "system" fn foreground_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    _window: HWND,
    _object: i32,
    _child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        let thread_id = unsafe {
            // SAFETY: GetCurrentThreadId has no preconditions.
            GetCurrentThreadId()
        };
        unsafe {
            // SAFETY: the WinEvent callback runs on the hook-owning message thread.
            PostThreadMessageW(thread_id, FOREGROUND_MESSAGE, 0, 0);
        }
    }
}

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

fn attach_to_game() -> Result<(VerifiedGameProcess, WindowsForeground), HotkeyErrorCode> {
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
    let executable =
        ExecutableIdentity::inspect(&identity.executable_path).map_err(|error| match error {
            crate::identity::ExecutableError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                HotkeyErrorCode::AccessDenied
            }
            _ => HotkeyErrorCode::ExecutableInspectionFailed,
        })?;
    let registry = BuildRegistry::from_toml(include_str!("../../data/builds.toml"))
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

pub(super) fn run_cli(bindings: Vec<HotkeyBinding>) -> ExitCode {
    let (game, foreground) = match attach_to_game() {
        Ok(attached) => attached,
        Err(code) => {
            eprintln!("{code}");
            return ExitCode::FAILURE;
        }
    };
    let mut backend = match WindowsBackend::start() {
        Ok(backend) => backend,
        Err(code) => {
            eprintln!("{code}");
            return ExitCode::FAILURE;
        }
    };
    let mut manager =
        match HotkeyManager::new(bindings.clone(), game, backend.registrar(), foreground) {
            Ok(manager) => manager,
            Err(_) => {
                eprintln!("HOTKEY_INVALID_BINDING");
                let _ = backend.shutdown();
                return ExitCode::FAILURE;
            }
        };
    if let Err(failure) = manager.start() {
        eprintln!("{}", failure.code);
        if failure.code != HotkeyErrorCode::RegistrationConflict {
            let _ = manager.shutdown();
            let _ = backend.shutdown();
            return ExitCode::FAILURE;
        }
    }

    STOP_REQUESTED.store(false, Ordering::SeqCst);
    let handler = unsafe {
        // SAFETY: callback is static and only sets an atomic flag.
        SetConsoleCtrlHandler(Some(console_control), 1)
    };
    if handler == 0 {
        eprintln!("HOTKEY_SHUTDOWN_FAILED");
        let _ = manager.shutdown();
        let _ = backend.shutdown();
        return ExitCode::FAILURE;
    }

    let mut last_reported_state = None;
    report_state_change(manager.state(), &mut last_reported_state, &bindings);
    let mut exit_code = ExitCode::SUCCESS;
    while !STOP_REQUESTED.load(Ordering::SeqCst) {
        match backend.recv_timeout(Duration::from_millis(250)) {
            Ok(BackendEvent::Hotkey(id)) => {
                if id == 0 || id as usize > bindings.len() {
                    eprintln!("{}", HotkeyErrorCode::UnknownRegistration);
                } else if let Some(event) = manager.handle_hotkey(id) {
                    print_hotkey_event(event);
                }
            }
            Ok(BackendEvent::ForegroundChanged) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Err(failure) = manager.reconcile() {
                    eprintln!("{}", failure.code);
                    if failure.code != HotkeyErrorCode::RegistrationConflict {
                        exit_code = ExitCode::FAILURE;
                        break;
                    }
                    report_state_change(manager.state(), &mut last_reported_state, &bindings);
                } else {
                    report_state_change(manager.state(), &mut last_reported_state, &bindings);
                }
            }
            Ok(BackendEvent::Failed(code)) => {
                eprintln!("{code}");
                exit_code = ExitCode::FAILURE;
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("HOTKEY_MESSAGE_LOOP_START_FAILED");
                exit_code = ExitCode::FAILURE;
                break;
            }
        }
    }

    let _ = unsafe {
        // SAFETY: removes the same static handler installed above.
        SetConsoleCtrlHandler(Some(console_control), 0)
    };
    if let Err(failure) = manager.shutdown() {
        eprintln!("{}", failure.code);
        exit_code = ExitCode::FAILURE;
    }
    if let Err(code) = backend.shutdown() {
        eprintln!("{code}");
        exit_code = ExitCode::FAILURE;
    }
    exit_code
}

fn report_state_change(
    state: HotkeyState,
    last_reported: &mut Option<HotkeyState>,
    bindings: &[HotkeyBinding],
) {
    if *last_reported == Some(state) {
        return;
    }
    *last_reported = Some(state);
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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use crate::{
        hotkeys::{
            binding::HotkeyBinding,
            manager::{
                ForegroundBackend, HotkeyErrorCode, HotkeyManager, HotkeyState,
                RegistrationBackend, RegistrationFailure, VerifiedGameProcess,
            },
        },
        process::ProcessIdentity,
    };

    use super::WindowsBackend;

    static LOCK: Mutex<()> = Mutex::new(());

    fn test_lock() -> MutexGuard<'static, ()> {
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn binding() -> HotkeyBinding {
        HotkeyBinding::parse("CTRL+ALT+SHIFT+F24").unwrap()
    }

    fn another_binding() -> HotkeyBinding {
        HotkeyBinding::parse("CTRL+ALT+SHIFT+F23").unwrap()
    }

    #[test]
    fn registers_and_unregisters_valid_hotkey() {
        let _lock = test_lock();
        let mut backend = WindowsBackend::start().unwrap();
        let mut registrar = backend.registrar();

        registrar.register(7, binding()).unwrap();
        registrar.unregister(7).unwrap();
        registrar.register(8, binding()).unwrap();
        backend.shutdown().unwrap();
    }

    #[test]
    fn reports_existing_hotkey_conflict() {
        let _lock = test_lock();
        let mut owner = WindowsBackend::start().unwrap();
        owner.registrar().register(1, binding()).unwrap();
        let mut contender = WindowsBackend::start().unwrap();

        assert_eq!(
            contender.registrar().register(1, binding()),
            Err(RegistrationFailure::Conflict)
        );
        contender.shutdown().unwrap();
        owner.shutdown().unwrap();
    }

    #[test]
    fn releases_binding_after_shutdown() {
        let _lock = test_lock();
        let mut first = WindowsBackend::start().unwrap();
        first.registrar().register(1, binding()).unwrap();
        first.shutdown().unwrap();

        let mut second = WindowsBackend::start().unwrap();
        second.registrar().register(1, binding()).unwrap();
        second.shutdown().unwrap();
    }

    #[test]
    fn repeated_start_stop_releases_resources() {
        let _lock = test_lock();
        for _ in 0..3 {
            let mut backend = WindowsBackend::start().unwrap();
            backend.registrar().register(1, binding()).unwrap();
            backend.shutdown().unwrap();
        }
    }

    #[test]
    fn partial_manager_registration_failure_rolls_back_prior_binding() {
        let _lock = test_lock();
        let mut owner = WindowsBackend::start().unwrap();
        owner.registrar().register(1, another_binding()).unwrap();
        let backend = WindowsBackend::start().unwrap();
        let identity = ProcessIdentity {
            creation_time_filetime: 1,
            executable_path: r"C:\Games\DOOMTheDarkAges.exe".into(),
            module_base: 0x1400_0000,
            image_size: 0x2000,
        };
        let mut manager = HotkeyManager::new(
            vec![binding(), another_binding()],
            VerifiedGameProcess {
                pid: 42,
                identity: identity.clone(),
                build_id: "registered-test-build".to_owned(),
            },
            backend.registrar(),
            TestForeground { identity },
        )
        .unwrap();

        assert_eq!(
            manager.start().unwrap_err().code,
            HotkeyErrorCode::RegistrationConflict
        );
        assert_eq!(manager.state(), HotkeyState::Conflict);
        let mut contender = WindowsBackend::start().unwrap();
        contender.registrar().register(1, binding()).unwrap();

        contender.shutdown().unwrap();
        owner.shutdown().unwrap();
        let mut test_backend = backend;
        test_backend.shutdown().unwrap();
    }

    #[test]
    fn message_loop_terminates_on_shutdown() {
        let _lock = test_lock();
        let mut backend = WindowsBackend::start().unwrap();

        backend.shutdown().unwrap();

        assert!(backend.is_stopped());
    }

    struct TestForeground {
        identity: ProcessIdentity,
    }

    impl ForegroundBackend for TestForeground {
        fn verify_game(
            &mut self,
            expected_pid: u32,
            expected_identity: &ProcessIdentity,
        ) -> Result<(), HotkeyErrorCode> {
            if expected_pid == 42 && expected_identity == &self.identity {
                Ok(())
            } else {
                Err(HotkeyErrorCode::IdentityChanged)
            }
        }

        fn foreground_pid(&mut self) -> Result<Option<u32>, HotkeyErrorCode> {
            Ok(Some(42))
        }
    }
}
