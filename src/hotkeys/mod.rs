pub mod binding;
pub mod config;
#[cfg(any(windows, test))]
mod diagnostics;
pub mod dispatch;
pub mod keyboard;
pub mod manager;
pub mod runtime;
pub mod state;
#[cfg(windows)]
mod windows;

use std::{ffi::OsString, path::Path, process::ExitCode};

use binding::{BindingError, HotkeyBinding, validate_bindings};
use config::{HotkeyConfig, HotkeyConfigError};
use dispatch::ConfiguredBinding;

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
    RawInputRegistrationFailed,
    RawInputReadFailed,
    EventQueueOverflow,
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
            Self::RawInputRegistrationFailed => "HOTKEY_RAW_INPUT_REGISTRATION_FAILED",
            Self::RawInputReadFailed => "HOTKEY_RAW_INPUT_READ_FAILED",
            Self::EventQueueOverflow => "HOTKEY_EVENT_QUEUE_OVERFLOW",
        }
    }
}

impl std::fmt::Display for HotkeyErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn run_cli(arguments: &[OsString]) -> ExitCode {
    match arguments.first().and_then(|argument| argument.to_str()) {
        Some("validate") => {
            let Some(path) = config_argument(arguments, "validate") else {
                return invalid_arguments();
            };
            let config = match HotkeyConfig::load(Path::new(path)) {
                Ok(config) => config,
                Err(error) => return config_error(error),
            };
            let bindings = match ConfiguredBinding::from_config(&config) {
                Ok(bindings) => bindings,
                Err(_) => {
                    eprintln!("HOTKEY_CONFIG_UNKNOWN_ACTION");
                    return ExitCode::from(2);
                }
            };
            println!("HOTKEY_CONFIG_VALID bindings={}", bindings.len());
            ExitCode::SUCCESS
        }
        Some("listen") => listen_cli(arguments),
        _ => invalid_arguments(),
    }
}

fn config_argument<'a>(arguments: &'a [OsString], command: &str) -> Option<&'a std::ffi::OsStr> {
    match arguments {
        [action, flag, path] if action == command && flag == "--config" && !path.is_empty() => {
            Some(path)
        }
        _ => None,
    }
}

fn listen_cli(arguments: &[OsString]) -> ExitCode {
    let mut bindings = Vec::new();
    let mut config_path = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--bind") if config_path.is_none() => {
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
            Some("--config") if config_path.is_none() && bindings.is_empty() => {
                let Some(path) = arguments.get(index + 1).filter(|path| !path.is_empty()) else {
                    return invalid_arguments();
                };
                config_path = Some(path);
                index += 2;
                if index != arguments.len() {
                    return invalid_arguments();
                }
            }
            _ => return invalid_arguments(),
        }
    }

    let configured = if let Some(path) = config_path {
        let config = match HotkeyConfig::load(Path::new(path)) {
            Ok(config) => config,
            Err(error) => return config_error(error),
        };
        match ConfiguredBinding::from_config(&config) {
            Ok(bindings) => bindings,
            Err(_) => {
                eprintln!("HOTKEY_CONFIG_UNKNOWN_ACTION");
                return ExitCode::from(2);
            }
        }
    } else {
        if bindings.is_empty() {
            return invalid_arguments();
        }
        if let Err(error) = validate_bindings(&bindings) {
            return binding_error(error);
        }
        bindings
            .into_iter()
            .enumerate()
            .map(|(index, binding)| ConfiguredBinding {
                id: if index == 0 {
                    "legacy".to_owned()
                } else {
                    format!("legacy-{}", index + 1)
                },
                binding,
                action: binding.action(),
            })
            .collect()
    };

    #[cfg(not(windows))]
    {
        let _ = configured;
        eprintln!("HOTKEY_UNSUPPORTED_PLATFORM: Win32 raw input requires Windows");
        ExitCode::FAILURE
    }

    #[cfg(windows)]
    {
        windows::run_cli(configured)
    }
}

fn invalid_arguments() -> ExitCode {
    eprintln!(
        "HOTKEY_INVALID_ARGUMENTS: expected hotkeys validate --config <path> or hotkeys listen --config <path> or --bind <shortcut>"
    );
    ExitCode::from(2)
}

fn binding_error(error: BindingError) -> ExitCode {
    eprintln!("{}", error);
    ExitCode::from(2)
}

fn config_error(error: HotkeyConfigError) -> ExitCode {
    eprintln!("{}", error.code());
    ExitCode::from(2)
}
