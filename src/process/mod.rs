use std::{io, path::PathBuf};

use serde::Serialize;
use thiserror::Error;

pub const GAME_EXECUTABLE: &str = "DOOMTheDarkAges.exe";

pub fn is_game_executable_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(GAME_EXECUTABLE)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessIdentity {
    pub creation_time_filetime: u64,
    pub executable_path: PathBuf,
    pub module_base: u64,
    pub image_size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProcessInspection {
    Available { identity: ProcessIdentity },
    Unavailable { failure: ProcessFailure },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProcessFailureKind {
    AccessDenied,
    Unavailable,
    IdentityInconsistent,
    ProcessExited,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessFailure {
    pub operation: String,
    pub kind: ProcessFailureKind,
    pub win32_error: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessCandidate {
    pub pid: u32,
    pub reported_name: String,
    pub inspection: ProcessInspection,
}

#[derive(Debug, Error)]
pub enum ProcessScanError {
    #[error("process discovery is not implemented on this operating system")]
    UnsupportedPlatform,
    #[error("process discovery failed during {operation}: {source}")]
    Io {
        operation: String,
        #[source]
        source: io::Error,
    },
}

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
pub use windows::enumerate_game_processes;

#[cfg(target_os = "linux")]
pub use linux::enumerate_game_processes;

#[cfg(all(not(windows), not(target_os = "linux")))]
pub fn enumerate_game_processes() -> Result<Vec<ProcessCandidate>, ProcessScanError> {
    Err(ProcessScanError::UnsupportedPlatform)
}
