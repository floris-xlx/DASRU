use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        Input::{
            GetRawInputData, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RAWKEYBOARD, RID_INPUT,
            RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
        },
        WindowsAndMessaging::{
            CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
            EVENT_SYSTEM_FOREGROUND, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, MSG,
            PM_NOREMOVE, PM_REMOVE, PeekMessageW, PostThreadMessageW, RegisterClassW,
            SetWindowLongPtrW, TranslateMessage, UnregisterClassW, WINEVENT_OUTOFCONTEXT, WM_APP,
            WM_DESTROY, WM_INPUT, WM_NCCREATE, WNDCLASSW,
        },
    },
};

use super::{
    HotkeyErrorCode,
    binding::HotkeyBinding,
    diagnostics::{HotkeyDiagnostics, Stage},
    manager::{RegistrationBackend, RegistrationFailure},
};

const COMMAND_MESSAGE: u32 = WM_APP + 1;
const FOREGROUND_MESSAGE: u32 = WM_APP + 2;
const STOP_MESSAGE: u32 = WM_APP + 3;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_SHIFT: u16 = 0x10;
const VK_LCONTROL: u16 = 0xa2;
const VK_RCONTROL: u16 = 0xa3;
const VK_LMENU: u16 = 0xa4;
const VK_RMENU: u16 = 0xa5;
const VK_LSHIFT: u16 = 0xa0;
const VK_RSHIFT: u16 = 0xa1;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
static FOCUS_GENERATION: AtomicU64 = AtomicU64::new(0);

enum Command {
    Register {
        id: i32,
        reply: SyncSender<Result<(), RegistrationFailure>>,
    },
    Unregister {
        id: i32,
        reply: SyncSender<Result<(), RegistrationFailure>>,
    },
}

enum BackendEvent {
    KeyboardInput {
        virtual_key: u16,
        is_down: bool,
        generation: u64,
    },
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
    overloaded: Arc<AtomicBool>,
}

impl WindowsBackend {
    #[cfg_attr(not(test), allow(dead_code))]
    fn start(bindings: &[HotkeyBinding]) -> Result<Self, HotkeyErrorCode> {
        Self::start_with_diagnostics(bindings, Arc::new(HotkeyDiagnostics::default()))
    }

    fn start_with_diagnostics(
        bindings: &[HotkeyBinding],
        diagnostics: Arc<HotkeyDiagnostics>,
    ) -> Result<Self, HotkeyErrorCode> {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::sync_channel(256);
        let overloaded = Arc::new(AtomicBool::new(false));
        let worker_overloaded = overloaded.clone();
        let (startup_tx, startup_rx) = mpsc::sync_channel(2);
        let (completed_tx, completed_rx) = mpsc::sync_channel(1);
        let abort = Arc::new(AtomicBool::new(false));
        let worker_abort = Arc::clone(&abort);
        let worker_bindings = bindings.to_vec();
        let worker_diagnostics = Arc::clone(&diagnostics);
        let worker = thread::Builder::new()
            .name("dasru-hotkeys".to_owned())
            .spawn(move || {
                let result = message_loop(
                    commands_rx,
                    events_tx,
                    startup_tx,
                    worker_abort,
                    worker_bindings,
                    worker_diagnostics,
                    worker_overloaded,
                );
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
            overloaded,
        })
    }

    fn registrar(&self) -> WindowsRegistrar {
        WindowsRegistrar {
            thread_id: self.thread_id,
            commands: self.commands.clone(),
        }
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<BackendEvent, mpsc::RecvTimeoutError> {
        if self.overloaded.load(Ordering::Acquire) {
            return Ok(BackendEvent::Failed(HotkeyErrorCode::EventQueueOverflow));
        }
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
    fn register(&mut self, id: u32, _binding: HotkeyBinding) -> Result<(), RegistrationFailure> {
        let id = i32::try_from(id).map_err(|_| RegistrationFailure::Failed)?;
        self.request(|reply| Command::Register { id, reply })
    }

    fn unregister(&mut self, id: u32) -> Result<(), RegistrationFailure> {
        let id = i32::try_from(id).map_err(|_| RegistrationFailure::Failed)?;
        self.request(|reply| Command::Unregister { id, reply })
    }
}

fn message_loop(
    commands: Receiver<Command>,
    events: SyncSender<BackendEvent>,
    startup: SyncSender<StartupMessage>,
    abort: Arc<AtomicBool>,
    bindings: Vec<HotkeyBinding>,
    diagnostics: Arc<HotkeyDiagnostics>,
    overloaded: Arc<AtomicBool>,
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

    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    if instance.is_null() {
        let _ = startup.send(StartupMessage::HookReady {
            thread_id,
            result: Err(HotkeyErrorCode::MessageLoopStartFailed),
        });
        return Err(HotkeyErrorCode::MessageLoopStartFailed);
    }
    let class_name: Vec<u16> = format!("DASRU_HOTKEY_INPUT_{thread_id}\0")
        .encode_utf16()
        .collect();
    let mut window_state = Box::new(InputWindowState {
        events: events.clone(),
        monitored_keys: monitored_keys(&bindings),
        diagnostics: Arc::clone(&diagnostics),
        overloaded: overloaded.clone(),
    });
    let class = WNDCLASSW {
        lpfnWndProc: Some(input_window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..WNDCLASSW::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        let _ = startup.send(StartupMessage::HookReady {
            thread_id,
            result: Err(HotkeyErrorCode::MessageLoopStartFailed),
        });
        return Err(HotkeyErrorCode::MessageLoopStartFailed);
    }
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            (&mut *window_state as *mut InputWindowState).cast(),
        )
    };
    if window.is_null() {
        unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
        let _ = startup.send(StartupMessage::HookReady {
            thread_id,
            result: Err(HotkeyErrorCode::MessageLoopStartFailed),
        });
        return Err(HotkeyErrorCode::MessageLoopStartFailed);
    }
    let mut raw_registered = false;
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
        cleanup_input_window(window, instance, &class_name, &mut raw_registered);
        let _ = startup.send(StartupMessage::HookReady {
            thread_id,
            result: Err(HotkeyErrorCode::MessageLoopStartFailed),
        });
        return Err(HotkeyErrorCode::MessageLoopStartFailed);
    }
    if abort.load(Ordering::SeqCst) {
        unsafe {
            // SAFETY: hook was successfully created above and has not been released.
            UnhookWinEvent(hook);
        }
        cleanup_input_window(window, instance, &class_name, &mut raw_registered);
        return Err(HotkeyErrorCode::StartupTimeout);
    }
    if startup
        .send(StartupMessage::HookReady {
            thread_id,
            result: Ok(()),
        })
        .is_err()
    {
        unsafe { UnhookWinEvent(hook) };
        cleanup_input_window(window, instance, &class_name, &mut raw_registered);
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
            let _ = events.try_send(BackendEvent::Failed(
                HotkeyErrorCode::MessageLoopStartFailed,
            ));
            result = Err(HotkeyErrorCode::MessageLoopStartFailed);
            break;
        }
        if status == 0 || message.message == STOP_MESSAGE {
            break;
        }
        match message.message {
            FOREGROUND_MESSAGE => {
                // Discard queued keys so a chord cannot survive a focus cycle.
                discard_pending_raw_input(window, &diagnostics);
                if events.try_send(BackendEvent::ForegroundChanged).is_err() {
                    diagnostics.increment(Stage::ForwardFailed);
                    invalidate_overflow(&overloaded);
                }
            }
            COMMAND_MESSAGE => {
                while let Ok(command) = commands.try_recv() {
                    match command {
                        Command::Register { id, reply } => {
                            let result = if registered.contains(&id) {
                                Err(RegistrationFailure::Conflict)
                            } else if registered.is_empty() && !raw_registered {
                                if register_raw_keyboard(window) {
                                    raw_registered = true;
                                    registered.insert(id);
                                    Ok(())
                                } else {
                                    Err(RegistrationFailure::RawInputRegistrationFailed)
                                }
                            } else {
                                registered.insert(id);
                                Ok(())
                            };
                            let _ = reply.send(result);
                        }
                        Command::Unregister { id, reply } => {
                            let result = if !registered.contains(&id)
                                || (registered.len() == 1
                                    && raw_registered
                                    && !unregister_raw_keyboard())
                            {
                                Err(RegistrationFailure::Failed)
                            } else {
                                if registered.len() == 1 && raw_registered {
                                    discard_pending_raw_input(window, &diagnostics);
                                }
                                registered.remove(&id);
                                if registered.is_empty() {
                                    raw_registered = false;
                                }
                                Ok(())
                            };
                            if result.is_ok() {
                                let _ = reply.send(Ok(()));
                            } else {
                                let _ = reply.send(result);
                            }
                        }
                    }
                }
            }
            _ => unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }

    registered.clear();
    if unsafe {
        // SAFETY: hook was created by this thread and is released exactly once here.
        UnhookWinEvent(hook)
    } == 0
    {
        result = Err(HotkeyErrorCode::ShutdownFailed);
    }
    if !cleanup_input_window(window, instance, &class_name, &mut raw_registered) {
        result = Err(HotkeyErrorCode::ShutdownFailed);
    }
    result
}

struct InputWindowState {
    events: SyncSender<BackendEvent>,
    monitored_keys: HashSet<u16>,
    diagnostics: Arc<HotkeyDiagnostics>,
    overloaded: Arc<AtomicBool>,
}

fn monitored_keys(bindings: &[HotkeyBinding]) -> HashSet<u16> {
    let mut keys = HashSet::from([
        VK_CONTROL,
        VK_MENU,
        VK_SHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LSHIFT,
        VK_RSHIFT,
    ]);
    keys.extend(bindings.iter().map(|binding| binding.virtual_key() as u16));
    keys
}

fn register_raw_keyboard(window: HWND) -> bool {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x06,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: window,
    };
    unsafe {
        RegisterRawInputDevices(&device, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0
    }
}

fn unregister_raw_keyboard() -> bool {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x06,
        dwFlags: RIDEV_REMOVE,
        hwndTarget: std::ptr::null_mut(),
    };
    unsafe {
        RegisterRawInputDevices(&device, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0
    }
}

fn discard_pending_raw_input(window: HWND, diagnostics: &HotkeyDiagnostics) {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, window, WM_INPUT, WM_INPUT, PM_REMOVE) } != 0 {
        diagnostics.increment(Stage::QueueDiscarded);
        if message.wParam as u32 == windows_sys::Win32::UI::WindowsAndMessaging::RIM_INPUT {
            unsafe {
                DefWindowProcW(window, message.message, message.wParam, message.lParam);
            }
        }
    }
}

fn cleanup_input_window(
    window: HWND,
    instance: windows_sys::Win32::Foundation::HINSTANCE,
    class_name: &[u16],
    raw_registered: &mut bool,
) -> bool {
    let raw_removed = !*raw_registered || unregister_raw_keyboard();
    if raw_removed {
        *raw_registered = false;
    }
    let window_destroyed = unsafe { DestroyWindow(window) != 0 };
    let class_removed = unsafe { UnregisterClassW(class_name.as_ptr(), instance) != 0 };
    raw_removed && window_destroyed && class_removed
}

unsafe extern "system" fn input_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
        return 1;
    }
    if message == WM_INPUT {
        let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut InputWindowState };
        if !state.is_null() {
            read_raw_keyboard(state, lparam);
        }
        if wparam as u32 == windows_sys::Win32::UI::WindowsAndMessaging::RIM_INPUT {
            return unsafe { DefWindowProcW(window, message, wparam, lparam) };
        }
        return 0;
    }
    if message == WM_DESTROY {
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn read_raw_keyboard(state: *mut InputWindowState, lparam: LPARAM) {
    let mut raw = std::mem::MaybeUninit::<RAWINPUT>::zeroed();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(
            lparam as *mut core::ffi::c_void,
            RID_INPUT,
            raw.as_mut_ptr().cast(),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    let required = std::mem::size_of::<RAWINPUTHEADER>() + std::mem::size_of::<RAWKEYBOARD>();
    if read == u32::MAX || read < required as u32 {
        let state = unsafe { &*state };
        state.diagnostics.increment(Stage::RawInputReadFailed);
        if state
            .events
            .try_send(BackendEvent::Failed(HotkeyErrorCode::RawInputReadFailed))
            .is_err()
        {
            state.diagnostics.increment(Stage::ForwardFailed);
            invalidate_overflow(&state.overloaded);
        }
        return;
    }
    let raw = unsafe { raw.assume_init() };
    if raw.header.dwType != RIM_TYPEKEYBOARD {
        return;
    }
    let keyboard = unsafe { raw.data.keyboard };
    let virtual_key = normalize_raw_virtual_key(keyboard);
    let state = unsafe { &*state };
    if state.monitored_keys.contains(&virtual_key) {
        let is_down =
            keyboard.Flags & windows_sys::Win32::UI::WindowsAndMessaging::RI_KEY_BREAK as u16 == 0;
        state.diagnostics.increment(Stage::RawInputObserved);
        let event = BackendEvent::KeyboardInput {
            virtual_key,
            is_down,
            generation: FOCUS_GENERATION.load(Ordering::Acquire),
        };
        if state.events.try_send(event).is_ok() {
            state.diagnostics.increment(Stage::InputForwarded);
        } else {
            state.diagnostics.increment(Stage::ForwardFailed);
            invalidate_overflow(&state.overloaded);
        }
    }
}

fn invalidate_overflow(overloaded: &AtomicBool) {
    // Invalidate queued chords before the dispatcher observes the terminal overflow.
    FOCUS_GENERATION.fetch_add(1, Ordering::AcqRel);
    overloaded.store(true, Ordering::Release);
}
fn normalize_raw_virtual_key(keyboard: RAWKEYBOARD) -> u16 {
    match keyboard.VKey {
        VK_SHIFT if keyboard.MakeCode == 0x36 => VK_RSHIFT,
        VK_SHIFT => VK_LSHIFT,
        VK_CONTROL
            if keyboard.Flags & windows_sys::Win32::UI::WindowsAndMessaging::RI_KEY_E0 as u16
                != 0 =>
        {
            VK_RCONTROL
        }
        VK_CONTROL => VK_LCONTROL,
        VK_MENU
            if keyboard.Flags & windows_sys::Win32::UI::WindowsAndMessaging::RI_KEY_E0 as u16
                != 0 =>
        {
            VK_RMENU
        }
        VK_MENU => VK_LMENU,
        virtual_key => virtual_key,
    }
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
        FOCUS_GENERATION.fetch_add(1, Ordering::AcqRel);
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

mod session;
pub(super) use session::{run_cli, run_session};

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::WindowsBackend;
    use crate::hotkeys::binding::HotkeyBinding;
    use crate::hotkeys::manager::RegistrationBackend;

    static LOCK: Mutex<()> = Mutex::new(());

    fn test_lock() -> MutexGuard<'static, ()> {
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn binding() -> HotkeyBinding {
        HotkeyBinding::parse("CTRL+ALT+SHIFT+F24").unwrap()
    }

    #[test]
    fn event_queue_overflow_is_terminal_and_backend_still_shuts_down() {
        let _lock = test_lock();
        let mut backend = WindowsBackend::start(&[binding()]).unwrap();
        backend
            .overloaded
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(matches!(
            backend.recv_timeout(std::time::Duration::ZERO),
            Ok(super::BackendEvent::Failed(
                super::HotkeyErrorCode::EventQueueOverflow
            ))
        ));
        backend.shutdown().unwrap();
        assert!(backend.is_stopped());
    }

    #[test]
    fn activates_and_deactivates_binding_on_raw_input_backend() {
        let _lock = test_lock();
        let mut backend = WindowsBackend::start(&[binding()]).unwrap();
        let mut registrar = backend.registrar();

        assert!(!keyboard_raw_input_registered());
        registrar.register(7, binding()).unwrap();
        assert!(keyboard_raw_input_registered());
        registrar.unregister(7).unwrap();
        assert!(!keyboard_raw_input_registered());
        registrar.register(8, binding()).unwrap();
        assert!(keyboard_raw_input_registered());
        backend.shutdown().unwrap();
        assert!(!keyboard_raw_input_registered());
    }

    #[test]
    fn removes_raw_keyboard_registration_after_shutdown() {
        let _lock = test_lock();
        let mut backend = WindowsBackend::start(&[binding()]).unwrap();
        backend.registrar().register(1, binding()).unwrap();
        assert!(keyboard_raw_input_registered());
        backend.shutdown().unwrap();
        assert!(!keyboard_raw_input_registered());
    }

    #[test]
    fn multiple_bindings_activate_together_and_deactivate_together() {
        let _lock = test_lock();
        let bindings = [
            binding(),
            HotkeyBinding::parse("CTRL+ALT+SHIFT+F23").unwrap(),
        ];
        let mut backend = WindowsBackend::start(&bindings).unwrap();
        let mut registrar = backend.registrar();

        registrar.register(1, bindings[0]).unwrap();
        registrar.register(2, bindings[1]).unwrap();
        assert!(keyboard_raw_input_registered());
        registrar.unregister(1).unwrap();
        assert!(keyboard_raw_input_registered());
        registrar.unregister(2).unwrap();
        assert!(!keyboard_raw_input_registered());
        backend.shutdown().unwrap();
        assert!(!keyboard_raw_input_registered());
    }

    #[test]
    fn repeated_start_stop_releases_resources() {
        let _lock = test_lock();
        for _ in 0..3 {
            let mut backend = WindowsBackend::start(&[binding()]).unwrap();
            backend.registrar().register(1, binding()).unwrap();
            assert!(keyboard_raw_input_registered());
            backend.shutdown().unwrap();
            assert!(!keyboard_raw_input_registered());
        }
    }

    #[test]
    fn raw_input_listener_coexists_with_an_owned_global_hotkey() {
        let _lock = test_lock();
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};

        let id = 0xDA57;
        let reserved = unsafe {
            RegisterHotKey(
                std::ptr::null_mut(),
                id,
                binding().modifiers(),
                binding().virtual_key(),
            )
        };
        assert_ne!(reserved, 0);

        let mut backend = WindowsBackend::start(&[binding()]).unwrap();
        backend.registrar().register(1, binding()).unwrap();
        assert!(keyboard_raw_input_registered());
        backend.shutdown().unwrap();
        assert!(!keyboard_raw_input_registered());

        assert_ne!(unsafe { UnregisterHotKey(std::ptr::null_mut(), id) }, 0);
    }

    #[test]
    fn message_loop_terminates_on_shutdown() {
        let _lock = test_lock();
        let mut backend = WindowsBackend::start(&[binding()]).unwrap();

        backend.shutdown().unwrap();

        assert!(backend.is_stopped());
    }

    fn keyboard_raw_input_registered() -> bool {
        use windows_sys::Win32::UI::Input::{GetRegisteredRawInputDevices, RAWINPUTDEVICE};

        let mut count = 0;
        let reported = unsafe {
            GetRegisteredRawInputDevices(
                std::ptr::null_mut(),
                &mut count,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        if reported == u32::MAX || count == 0 {
            return false;
        }
        let mut devices = vec![RAWINPUTDEVICE::default(); count as usize];
        let read = unsafe {
            GetRegisteredRawInputDevices(
                devices.as_mut_ptr(),
                &mut count,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        read != u32::MAX
            && devices[..count as usize]
                .iter()
                .any(|device| device.usUsagePage == 0x01 && device.usUsage == 0x06)
    }
}
