use std::{ffi::c_void, mem::size_of};

use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE},
    System::{
        Diagnostics::Debug::ReadProcessMemory,
        Memory::{
            MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
            PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
            PAGE_WRITECOPY, VirtualQueryEx,
        },
    },
};

use crate::{
    probe::{ProbeReport, ReportError},
    process::{
        ProcessIdentity, ProcessInspection, ProcessScanError, enumerate_pinned_game_processes,
        open_readonly_process, recheck_pinned_process,
    },
};

use super::{
    FPS_SIGNATURE, MemoryScanStats, MetricsMatch, MetricsScanReport, MetricsScanStatus, scan,
};

const CHUNK_SIZE: usize = 1024 * 1024;

struct ReadableRegion {
    base: u64,
    size: u64,
    state: u32,
    protection: u32,
}

pub(super) fn scan_metrics(probe: ProbeReport) -> MetricsScanReport {
    if probe.processes.len() != 1 {
        return fail(
            probe,
            MetricsScanStatus::IdentityInconsistent,
            "process",
            "PROCESS_IDENTITY_MISSING",
            "the probe did not retain exactly one process candidate",
        );
    }
    let identity_result = match &probe.processes[0].inspection {
        ProcessInspection::Available { identity } => Ok((probe.processes[0].pid, identity.clone())),
        ProcessInspection::Unavailable { failure } => Err((
            if failure.kind == crate::process::ProcessFailureKind::AccessDenied {
                MetricsScanStatus::AccessDenied
            } else {
                MetricsScanStatus::IdentityInconsistent
            },
            format!("{} failed: {:?}", failure.operation, failure.kind),
        )),
    };
    let (pid, expected) = match identity_result {
        Ok(identity) => identity,
        Err((status, message)) => {
            return fail(
                probe,
                status,
                "process",
                "PROCESS_IDENTITY_UNAVAILABLE",
                &message,
            );
        }
    };
    let pinned = match enumerate_pinned_game_processes() {
        Ok(candidates) => candidates,
        Err(error) => {
            let (status, message) = process_scan_error(error);
            return fail(
                probe,
                status,
                "process",
                "PROCESS_REVALIDATION_FAILED",
                &message,
            );
        }
    };
    if pinned.len() != 1 {
        return fail(
            probe,
            if pinned.is_empty() {
                MetricsScanStatus::IdentityInconsistent
            } else {
                MetricsScanStatus::AmbiguousProcess
            },
            "process",
            "PROCESS_SET_CHANGED",
            "the game process set changed before memory scanning",
        );
    }
    let pinned_candidate = &pinned[0].candidate;
    if pinned_candidate.pid != pid
        || !same_process_identity(&pinned_candidate.inspection, &expected)
    {
        return fail(
            probe,
            MetricsScanStatus::IdentityInconsistent,
            "process",
            "PROCESS_IDENTITY_CHANGED",
            "PID, creation time, executable path, or module identity changed before scanning",
        );
    }

    let process_handle = match open_readonly_process(pid, &expected) {
        Ok(handle) => handle,
        Err(ProcessInspection::Unavailable { failure }) => {
            let status = if failure.kind == crate::process::ProcessFailureKind::AccessDenied {
                MetricsScanStatus::AccessDenied
            } else {
                MetricsScanStatus::IdentityInconsistent
            };
            return fail(
                probe,
                status,
                "process",
                "READ_ONLY_ATTACH_FAILED",
                &format!("{} failed: {:?}", failure.operation, failure.kind),
            );
        }
        Err(ProcessInspection::Available { .. }) => {
            return fail(
                probe,
                MetricsScanStatus::IdentityInconsistent,
                "process",
                "READ_ONLY_ATTACH_FAILED",
                "read-only process attachment returned no usable handle",
            );
        }
    };

    let (matches, stats) = match scan_module(
        process_handle.raw(),
        expected.module_base,
        u64::from(expected.image_size),
    ) {
        Ok(result) => result,
        Err(message) => {
            return fail(
                probe,
                MetricsScanStatus::ProbeError,
                "memory.scan",
                "MEMORY_SCAN_FAILED",
                &message,
            );
        }
    };

    if !same_process_identity(&recheck_pinned_process(&pinned[0]), &expected) {
        return fail(
            probe,
            MetricsScanStatus::IdentityInconsistent,
            "process",
            "PROCESS_IDENTITY_CHANGED_DURING_SCAN",
            "the retained process identity changed while memory was scanned",
        );
    }

    let status = match matches.len() {
        0 => MetricsScanStatus::NoMatch,
        1 => MetricsScanStatus::FoundUniqueMatch,
        _ => MetricsScanStatus::AmbiguousMatch,
    };
    let mut report = MetricsScanReport::new(status, probe);
    report.scan_stats = Some(stats);
    report.matches = matches;
    report
}

fn same_process_identity(inspection: &ProcessInspection, expected: &ProcessIdentity) -> bool {
    matches!(inspection, ProcessInspection::Available { identity } if identity == expected)
}

fn fail(
    probe: ProbeReport,
    status: MetricsScanStatus,
    field: &'static str,
    code: &'static str,
    message: &str,
) -> MetricsScanReport {
    let mut report = MetricsScanReport::new(status, probe);
    report.errors.push(ReportError {
        field,
        code,
        message: message.to_owned(),
    });
    report
}

fn process_scan_error(error: ProcessScanError) -> (MetricsScanStatus, String) {
    match error {
        ProcessScanError::UnsupportedPlatform => (
            MetricsScanStatus::UnsupportedPlatform,
            "Windows process discovery is unsupported on this platform".to_owned(),
        ),
        ProcessScanError::Io { operation, source } => (
            if source.kind() == std::io::ErrorKind::PermissionDenied {
                MetricsScanStatus::AccessDenied
            } else {
                MetricsScanStatus::ProbeError
            },
            format!("{operation} failed: {source}"),
        ),
    }
}

fn scan_module(
    process: HANDLE,
    module_base: u64,
    module_size: u64,
) -> Result<(Vec<MetricsMatch>, MemoryScanStats), String> {
    let start = usize::try_from(module_base)
        .map_err(|_| "module base does not fit this process address space".to_owned())?;
    let size = usize::try_from(module_size)
        .map_err(|_| "module size does not fit this process address space".to_owned())?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| "module address range overflows".to_owned())?;
    let mut stats = MemoryScanStats {
        module_base,
        module_size,
        readable_regions: 0,
        skipped_regions: 0,
        bytes_read: 0,
        bytes_unreadable: 0,
        failed_reads: 0,
        coverage: "READABLE_REGIONS_ONLY",
    };
    let mut scanner = scan::ChunkScanner::new(FPS_SIGNATURE);
    let mut readable_regions = Vec::new();
    let mut cursor = start;

    while cursor < end {
        // SAFETY: a zeroed output structure is valid for VirtualQueryEx to fill.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: process is a live read-only process handle and info is writable for the call.
        let received = unsafe {
            VirtualQueryEx(
                process,
                cursor as *const c_void,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if received == 0 {
            return Err(format!(
                "VirtualQueryEx failed at 0x{cursor:X} (Win32 error {})",
                unsafe { GetLastError() }
            ));
        }
        if received < size_of::<MEMORY_BASIC_INFORMATION>() {
            return Err("VirtualQueryEx returned a truncated memory region".to_owned());
        }
        let region_base = info.BaseAddress as usize;
        let region_size = info.RegionSize;
        let region_end = region_base
            .checked_add(region_size)
            .ok_or_else(|| "memory region address range overflows".to_owned())?;
        if region_end <= cursor {
            return Err("VirtualQueryEx did not advance through the module".to_owned());
        }
        let range_start = cursor.max(region_base);
        let range_end = end.min(region_end);
        let readable = info.State == MEM_COMMIT && readable_protection(info.Protect);
        if readable {
            stats.readable_regions += 1;
            readable_regions.push(ReadableRegion {
                base: region_base as u64,
                size: region_size as u64,
                state: info.State,
                protection: info.Protect,
            });
            let mut address = range_start;
            while address < range_end {
                let length = CHUNK_SIZE.min(range_end - address);
                let mut buffer = vec![0u8; length];
                let mut bytes_read = 0usize;
                // SAFETY: process is read-only, the source is inside a committed readable region,
                // and buffer is writable for the requested length.
                let succeeded = unsafe {
                    ReadProcessMemory(
                        process,
                        address as *const c_void,
                        buffer.as_mut_ptr().cast(),
                        length,
                        &mut bytes_read,
                    )
                };
                if bytes_read > length {
                    return Err("ReadProcessMemory reported more bytes than requested".to_owned());
                }
                if bytes_read > 0 {
                    scanner.push(address as u64, &buffer[..bytes_read]);
                    stats.bytes_read += bytes_read as u64;
                }
                if succeeded == 0 || bytes_read != length {
                    stats.failed_reads += 1;
                    stats.bytes_unreadable += (length - bytes_read) as u64;
                    if bytes_read != length {
                        scanner.unreadable_gap();
                    }
                }
                address += length;
            }
        } else {
            stats.skipped_regions += 1;
            stats.bytes_unreadable += (range_end - range_start) as u64;
            scanner.unreadable_gap();
        }
        cursor = region_end;
    }

    stats.coverage = if stats.bytes_read == module_size
        && stats.bytes_unreadable == 0
        && stats.failed_reads == 0
    {
        "FULL_MODULE"
    } else {
        "READABLE_REGIONS_ONLY"
    };
    let addresses = scanner.finish();
    let matches = addresses
        .into_iter()
        .filter_map(|address| {
            readable_regions
                .iter()
                .find(|region| address >= region.base && address < region.base + region.size)
                .map(|region| MetricsMatch {
                    address,
                    region_base: region.base,
                    region_size: region.size,
                    state: region.state,
                    protection: region.protection,
                    protection_name: protection_name(region.protection),
                })
        })
        .collect();
    Ok((matches, stats))
}

fn readable_protection(protection: u32) -> bool {
    if protection & (PAGE_GUARD | PAGE_NOACCESS) != 0 {
        return false;
    }
    matches!(
        protection & 0xff,
        PAGE_READONLY
            | PAGE_READWRITE
            | PAGE_WRITECOPY
            | PAGE_EXECUTE_READ
            | PAGE_EXECUTE_READWRITE
            | PAGE_EXECUTE_WRITECOPY
    )
}

fn protection_name(protection: u32) -> &'static str {
    match protection & 0xff {
        PAGE_READONLY => "PAGE_READONLY",
        PAGE_READWRITE => "PAGE_READWRITE",
        PAGE_WRITECOPY => "PAGE_WRITECOPY",
        PAGE_EXECUTE_READ => "PAGE_EXECUTE_READ",
        PAGE_EXECUTE_READWRITE => "PAGE_EXECUTE_READWRITE",
        PAGE_EXECUTE_WRITECOPY => "PAGE_EXECUTE_WRITECOPY",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_filter_excludes_guard_and_execute_only_pages() {
        assert!(readable_protection(PAGE_READONLY));
        assert!(readable_protection(PAGE_EXECUTE_READ));
        assert!(!readable_protection(PAGE_READONLY | PAGE_GUARD));
        assert!(!readable_protection(PAGE_EXECUTE_READ | PAGE_NOACCESS));
        assert!(!readable_protection(0x10));
    }
}
