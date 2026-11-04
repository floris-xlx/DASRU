use std::{io, path::PathBuf};

use serde::Serialize;
use thiserror::Error;

pub const GAME_EXECUTABLE: &str = "DOOMTheDarkAges.exe";

pub fn is_game_executable_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(GAME_EXECUTABLE)
}

#[cfg(any(windows, test))]
pub(crate) fn same_process_image_paths(paths: &[&std::path::Path]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let canonical = paths
        .iter()
        .map(|path| std::fs::canonicalize(path).ok())
        .collect::<Option<Vec<_>>>();
    let comparable = canonical
        .as_ref()
        .map(|paths| {
            paths
                .iter()
                .map(std::path::PathBuf::as_path)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| paths.to_vec());
    let Some(first) = comparable.first() else {
        return false;
    };
    let first = normalize_process_path(first);
    comparable
        .iter()
        .skip(1)
        .all(|path| normalize_process_path(path) == first)
}

#[cfg(any(windows, test))]
fn normalize_process_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    value
        .strip_prefix("\\\\?\\")
        .or_else(|| value.strip_prefix("\\??\\"))
        .unwrap_or(&value)
        .to_lowercase()
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

pub(crate) struct PinnedProcessCandidate {
    pub candidate: ProcessCandidate,
    #[cfg(windows)]
    handle: Option<windows::OwnedHandle>,
}

#[cfg(windows)]
pub(crate) fn enumerate_pinned_game_processes()
-> Result<Vec<PinnedProcessCandidate>, ProcessScanError> {
    windows::enumerate_pinned_game_processes()
}

#[cfg(not(windows))]
// shortcut: non-Windows probes stay unsupported until that backend can retain a process handle too.
pub(crate) fn enumerate_pinned_game_processes()
-> Result<Vec<PinnedProcessCandidate>, ProcessScanError> {
    enumerate_game_processes().map(|candidates| {
        candidates
            .into_iter()
            .map(|candidate| PinnedProcessCandidate { candidate })
            .collect()
    })
}

pub(crate) fn recheck_pinned_process(candidate: &PinnedProcessCandidate) -> ProcessInspection {
    #[cfg(windows)]
    if let Some(handle) = &candidate.handle {
        return match windows::inspect_open_process(handle.0, candidate.candidate.pid) {
            Ok(identity) => ProcessInspection::Available { identity },
            Err(failure) => failure,
        };
    }

    candidate.candidate.inspection.clone()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_normalized_path_equality_when_canonicalization_fails() {
        let paths = [
            std::path::Path::new(r"\\?\C:\Games\DOOMTheDarkAges\DOOMTheDarkAges.exe"),
            std::path::Path::new("c:/games/doomthedarkages/doomthedarkages.exe"),
        ];

        assert!(same_process_image_paths(&paths));
    }
}
