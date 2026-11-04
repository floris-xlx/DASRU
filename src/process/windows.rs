use std::{ffi::OsString, io, mem::size_of, os::windows::ffi::OsStringExt};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, FILETIME, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
            PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPMODULE,
            TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_VM_READ,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
};

use super::{
    GAME_EXECUTABLE, PinnedProcessCandidate, ProcessCandidate, ProcessFailure, ProcessFailureKind,
    ProcessIdentity, ProcessInspection, ProcessScanError, is_game_executable_name,
};

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NO_MORE_FILES: u32 = 18;

pub(super) struct OwnedHandle(pub(super) HANDLE);

pub(crate) struct ReadOnlyProcessHandle(OwnedHandle);

impl ReadOnlyProcessHandle {
    pub(crate) fn raw(&self) -> HANDLE {
        self.0.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is only created for a valid, owned Win32 handle.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn enumerate_game_processes() -> Result<Vec<ProcessCandidate>, ProcessScanError> {
    enumerate_pinned_game_processes().map(|candidates| {
        candidates
            .into_iter()
            .map(|candidate| candidate.candidate)
            .collect()
    })
}

pub(super) fn enumerate_pinned_game_processes()
-> Result<Vec<PinnedProcessCandidate>, ProcessScanError> {
    // SAFETY: the snapshot flags and process id are valid scalar arguments.
    let raw_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw_snapshot == INVALID_HANDLE_VALUE {
        return Err(scan_error("CreateToolhelp32Snapshot", last_error()));
    }
    let snapshot = OwnedHandle(raw_snapshot);

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: entry is initialized, writable, and its dwSize matches the API struct.
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        let error = last_error();
        if error == ERROR_NO_MORE_FILES {
            return Ok(Vec::new());
        }
        return Err(scan_error("Process32FirstW", error));
    }

    let mut candidates = Vec::new();
    loop {
        let name = from_wide(&entry.szExeFile);
        if is_game_executable_name(&name) {
            let pid = entry.th32ProcessID;
            tracing::debug!(pid, reported_name = %name, "found game process candidate by executable name");
            let (inspection, handle) = match inspect_process(pid) {
                Ok((identity, handle)) => (ProcessInspection::Available { identity }, Some(handle)),
                Err(failure) => (failure, None),
            };
            candidates.push(PinnedProcessCandidate {
                candidate: ProcessCandidate {
                    pid,
                    reported_name: name,
                    inspection,
                },
                handle,
            });
        }

        entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: entry is initialized, writable, and its dwSize matches the API struct.
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            let error = last_error();
            if error == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(scan_error("Process32NextW", error));
        }
    }

    Ok(candidates)
}

fn inspect_process(pid: u32) -> Result<(ProcessIdentity, OwnedHandle), ProcessInspection> {
    if pid == 0 {
        return Err(inconsistent("the process snapshot returned PID 0"));
    }

    // SAFETY: OpenProcess receives a PID from the process snapshot and only query/synchronization rights.
    let raw_process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if raw_process.is_null() {
        let error = last_error();
        return Err(if error == ERROR_INVALID_PARAMETER {
            exited("OpenProcess")
        } else {
            unavailable("OpenProcess", error)
        });
    }
    let process = OwnedHandle(raw_process);
    inspect_open_process(process.0, pid).map(|identity| (identity, process))
}

pub(crate) fn open_readonly_process(
    pid: u32,
    expected: &ProcessIdentity,
) -> Result<ReadOnlyProcessHandle, ProcessInspection> {
    // VirtualQueryEx needs query access; memory reads add PROCESS_VM_READ only.
    let raw_process = unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if raw_process.is_null() {
        let error = last_error();
        return Err(if error == ERROR_INVALID_PARAMETER {
            exited("OpenProcess for read-only memory scan")
        } else {
            unavailable("OpenProcess for read-only memory scan", error)
        });
    }
    let handle = OwnedHandle(raw_process);
    let observed = inspect_open_process(handle.0, pid)?;
    if observed != *expected {
        return Err(inconsistent(
            "read-only memory handle identity differs from the inspected process",
        ));
    }
    Ok(ReadOnlyProcessHandle(handle))
}

pub(super) fn inspect_open_process(
    process: HANDLE,
    pid: u32,
) -> Result<ProcessIdentity, ProcessInspection> {
    validate_process_handle(process, pid)?;

    let first_creation_time = match creation_time(process) {
        Ok(value) => value,
        Err(error) => return Err(process_api_failure(process, pid, "GetProcessTimes", error)),
    };
    let first_path = match process_path(process) {
        Ok(value) => value,
        Err(error) => {
            return Err(process_api_failure(
                process,
                pid,
                "QueryFullProcessImageNameW",
                error,
            ));
        }
    };
    if !first_path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(GAME_EXECUTABLE))
    {
        return Err(inconsistent(
            "process image path does not match the candidate executable name",
        ));
    }

    let (module_path, module_base, image_size) = main_module(process)?;
    let second_path = match process_path(process) {
        Ok(value) => value,
        Err(error) => {
            return Err(process_api_failure(
                process,
                pid,
                "QueryFullProcessImageNameW after module lookup",
                error,
            ));
        }
    };
    let second_creation_time = match creation_time(process) {
        Ok(value) => value,
        Err(error) => {
            return Err(process_api_failure(
                process,
                pid,
                "GetProcessTimes after module lookup",
                error,
            ));
        }
    };
    validate_process_handle(process, pid)?;

    if first_creation_time != second_creation_time
        || !super::same_process_image_paths(&[&first_path, &module_path, &second_path])
    {
        return Err(inconsistent(
            "process path, creation time, or main module changed during inspection",
        ));
    }

    Ok(ProcessIdentity {
        creation_time_filetime: first_creation_time,
        executable_path: first_path,
        module_base,
        image_size,
    })
}

fn main_module(process: HANDLE) -> Result<(std::path::PathBuf, u64, u32), ProcessInspection> {
    let pid = unsafe { GetProcessId(process) };
    if pid == 0 {
        return Err(unavailable(
            "GetProcessId for module snapshot",
            last_error(),
        ));
    }

    // SAFETY: the snapshot flags request modules for the PID from the current process table.
    let raw_snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if raw_snapshot == INVALID_HANDLE_VALUE {
        return Err(unavailable(
            "CreateToolhelp32Snapshot for modules",
            last_error(),
        ));
    }
    let snapshot = OwnedHandle(raw_snapshot);

    let mut module = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: module is initialized, writable, and its dwSize matches the API struct.
    if unsafe { Module32FirstW(snapshot.0, &mut module) } == 0 {
        let error = last_error();
        return Err(if error == ERROR_NO_MORE_FILES {
            inconsistent("the process had no modules to inspect")
        } else {
            unavailable("Module32FirstW", error)
        });
    }

    loop {
        let module_path = std::path::PathBuf::from(from_wide(&module.szExePath));
        if module_path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(GAME_EXECUTABLE))
        {
            if module.modBaseAddr.is_null() || module.modBaseSize == 0 {
                return Err(inconsistent(
                    "the main module has an invalid base or image size",
                ));
            }
            return Ok((
                module_path,
                module.modBaseAddr as usize as u64,
                module.modBaseSize,
            ));
        }

        module = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: module is initialized, writable, and its dwSize matches the API struct.
        if unsafe { Module32NextW(snapshot.0, &mut module) } == 0 {
            let error = last_error();
            return Err(if error == ERROR_NO_MORE_FILES {
                inconsistent("the process main module was absent from its module snapshot")
            } else {
                unavailable("Module32NextW", error)
            });
        }
    }
}

fn validate_process_handle(process: HANDLE, expected_pid: u32) -> Result<(), ProcessInspection> {
    let actual_pid = unsafe { GetProcessId(process) };
    if actual_pid == 0 {
        return Err(unavailable("GetProcessId", last_error()));
    }
    if actual_pid != expected_pid {
        return Err(inconsistent(
            "the process handle PID does not match the process snapshot",
        ));
    }

    match unsafe { WaitForSingleObject(process, 0) } {
        WAIT_TIMEOUT => Ok(()),
        WAIT_OBJECT_0 => Err(exited("WaitForSingleObject")),
        WAIT_FAILED => Err(unavailable("WaitForSingleObject", last_error())),
        _ => Err(unavailable_without_error(
            "WaitForSingleObject returned an unexpected result",
        )),
    }
}

fn process_api_failure(
    process: HANDLE,
    pid: u32,
    operation: &str,
    error: u32,
) -> ProcessInspection {
    if let Err(failure) = validate_process_handle(process, pid) {
        if matches!(
            &failure,
            ProcessInspection::Unavailable {
                failure: ProcessFailure {
                    kind: ProcessFailureKind::ProcessExited,
                    ..
                }
            }
        ) {
            return failure;
        }
    }
    unavailable(operation, error)
}

fn process_path(process: HANDLE) -> Result<std::path::PathBuf, u32> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: buffer is writable and length is its capacity in UTF-16 code units.
    let succeeded =
        unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) };
    if succeeded == 0 {
        return Err(last_error());
    }
    buffer.truncate(length as usize);
    Ok(std::path::PathBuf::from(OsString::from_wide(&buffer)))
}

fn creation_time(process: HANDLE) -> Result<u64, u32> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all FILETIME outputs are valid and writable for the API call.
    let succeeded =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    if succeeded == 0 {
        return Err(last_error());
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn from_wide(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    OsString::from_wide(&value[..length])
        .to_string_lossy()
        .into_owned()
}

fn unavailable(operation: &str, error: u32) -> ProcessInspection {
    ProcessInspection::Unavailable {
        failure: ProcessFailure {
            operation: operation.to_owned(),
            kind: if error == ERROR_ACCESS_DENIED {
                ProcessFailureKind::AccessDenied
            } else {
                ProcessFailureKind::Unavailable
            },
            win32_error: Some(error),
        },
    }
}

fn unavailable_without_error(operation: &str) -> ProcessInspection {
    ProcessInspection::Unavailable {
        failure: ProcessFailure {
            operation: operation.to_owned(),
            kind: ProcessFailureKind::Unavailable,
            win32_error: None,
        },
    }
}

fn exited(operation: &str) -> ProcessInspection {
    ProcessInspection::Unavailable {
        failure: ProcessFailure {
            operation: operation.to_owned(),
            kind: ProcessFailureKind::ProcessExited,
            win32_error: None,
        },
    }
}

fn inconsistent(operation: &str) -> ProcessInspection {
    ProcessInspection::Unavailable {
        failure: ProcessFailure {
            operation: operation.to_owned(),
            kind: ProcessFailureKind::IdentityInconsistent,
            win32_error: None,
        },
    }
}

fn last_error() -> u32 {
    // SAFETY: GetLastError has no preconditions and is thread-local.
    unsafe { GetLastError() }
}

fn scan_error(operation: &str, error: u32) -> ProcessScanError {
    ProcessScanError::Io {
        operation: operation.to_owned(),
        source: io::Error::from_raw_os_error(error as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, thread, time::Duration};

    #[test]
    fn retained_process_handle_detects_exit_after_the_initial_scan() {
        let test_binary = std::env::current_exe().expect("test executable path is available");
        let mut child = Command::new(test_binary)
            .args([
                "--exact",
                "process::windows::tests::wait_for_parent_to_kill_child",
                "--ignored",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("child test process starts");
        let pid = child.id();
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        assert!(!raw.is_null(), "child process handle opens");
        let pinned = PinnedProcessCandidate {
            candidate: ProcessCandidate {
                pid,
                reported_name: GAME_EXECUTABLE.to_owned(),
                inspection: ProcessInspection::Unavailable {
                    failure: ProcessFailure {
                        operation: "test setup".to_owned(),
                        kind: ProcessFailureKind::Unavailable,
                        win32_error: None,
                    },
                },
            },
            handle: Some(OwnedHandle(raw)),
        };

        child.kill().expect("child process is terminated");
        child.wait().expect("child process exit is observed");

        assert!(matches!(
            super::super::recheck_pinned_process(&pinned),
            ProcessInspection::Unavailable {
                failure: ProcessFailure {
                    kind: ProcessFailureKind::ProcessExited,
                    ..
                }
            }
        ));
    }

    #[test]
    #[ignore]
    fn wait_for_parent_to_kill_child() {
        thread::sleep(Duration::from_secs(30));
    }
}
