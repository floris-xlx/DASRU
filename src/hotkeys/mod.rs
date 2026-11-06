pub mod binding;
pub mod manager;
pub mod state;
#[cfg(windows)]
mod windows;

use std::{ffi::OsString, process::ExitCode};

use binding::{BindingError, HotkeyBinding, validate_bindings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyErrorCode {
    InvalidArguments,
    InvalidBinding,
    UnmodifiedKey,
    ReservedBinding,
    InvalidVirtualKey,
    DuplicateBinding,
    UnsupportedPlatform,
    NoProcess,
    AmbiguousProcess,
    AccessDenied,
    UnknownBuild,
    IdentityChanged,
    ForegroundUnavailable,
    ForegroundMismatch,
    NotActive,
    Disabled,
    RegistrationConflict,
    RegistrationFailed,
    UnknownRegistration,
    ProcessInspectionFailed,
    ExecutableInspectionFailed,
    InvalidTransition,
    MessageLoopStartFailed,
    StartupTimeout,
    ShutdownFailed,
    ShutdownTimeout,
    ProcessExited,
}

impl HotkeyErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "HOTKEY_INVALID_ARGUMENTS",
            Self::InvalidBinding => "HOTKEY_INVALID_BINDING",
            Self::UnmodifiedKey => "HOTKEY_UNMODIFIED_KEY",
            Self::ReservedBinding => "HOTKEY_RESERVED_BINDING",
            Self::InvalidVirtualKey => "HOTKEY_INVALID_VIRTUAL_KEY",
            Self::DuplicateBinding => "HOTKEY_DUPLICATE_BINDING",
            Self::UnsupportedPlatform => "HOTKEY_UNSUPPORTED_PLATFORM",
            Self::NoProcess => "HOTKEY_NO_PROCESS",
            Self::AmbiguousProcess => "HOTKEY_AMBIGUOUS_PROCESS",
            Self::AccessDenied => "HOTKEY_ACCESS_DENIED",
            Self::UnknownBuild => "HOTKEY_UNKNOWN_BUILD",
            Self::IdentityChanged => "HOTKEY_IDENTITY_CHANGED",
            Self::ForegroundUnavailable => "HOTKEY_FOREGROUND_UNAVAILABLE",
            Self::ForegroundMismatch => "HOTKEY_FOREGROUND_MISMATCH",
            Self::NotActive => "HOTKEY_NOT_ACTIVE",
            Self::Disabled => "HOTKEY_DISABLED",
            Self::RegistrationConflict => "HOTKEY_REGISTRATION_CONFLICT",
            Self::RegistrationFailed => "HOTKEY_REGISTRATION_FAILED",
            Self::UnknownRegistration => "HOTKEY_UNKNOWN_REGISTRATION",
            Self::ProcessInspectionFailed => "HOTKEY_PROCESS_INSPECTION_FAILED",
            Self::ExecutableInspectionFailed => "HOTKEY_EXECUTABLE_INSPECTION_FAILED",
            Self::InvalidTransition => "HOTKEY_INVALID_TRANSITION",
            Self::MessageLoopStartFailed => "HOTKEY_MESSAGE_LOOP_START_FAILED",
            Self::StartupTimeout => "HOTKEY_STARTUP_TIMEOUT",
            Self::ShutdownFailed => "HOTKEY_SHUTDOWN_FAILED",
            Self::ShutdownTimeout => "HOTKEY_SHUTDOWN_TIMEOUT",
            Self::ProcessExited => "HOTKEY_PROCESS_EXITED",
        }
    }
}

impl std::fmt::Display for HotkeyErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn run_cli(arguments: &[OsString]) -> ExitCode {
    if arguments.first().and_then(|argument| argument.to_str()) != Some("listen") {
        return invalid_arguments();
    }

    let mut bindings = Vec::new();
    let mut index = 1;
    while index < arguments.len() {
        if arguments[index].to_str() != Some("--bind") {
            return invalid_arguments();
        }
        let Some(value) = arguments
            .get(index + 1)
            .and_then(|argument| argument.to_str())
        else {
            return invalid_arguments();
        };
        match HotkeyBinding::parse(value) {
            Ok(binding) => bindings.push(binding),
            Err(error) => return binding_error(error),
        }
        index += 2;
    }

    if bindings.is_empty() {
        return invalid_arguments();
    }
    if let Err(error) = validate_bindings(&bindings) {
        return binding_error(error);
    }

    #[cfg(not(windows))]
    {
        eprintln!(
            "HOTKEY_UNSUPPORTED_PLATFORM: Win32 global hotkeys require Windows; binding={}",
            bindings[0]
        );
        ExitCode::FAILURE
    }

    #[cfg(windows)]
    {
        windows::run_cli(bindings)
    }
}

fn invalid_arguments() -> ExitCode {
    eprintln!("HOTKEY_INVALID_ARGUMENTS: expected hotkeys listen --bind <shortcut>");
    ExitCode::from(2)
}

fn binding_error(error: BindingError) -> ExitCode {
    eprintln!("{}", error);
    ExitCode::from(2)
}
