use std::{ffi::c_void, mem::size_of};

use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE},
    System::{
        Diagnostics::Debug::ReadProcessMemory,
        Memory::{
            MEM_COMMIT, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE, MEMORY_BASIC_INFORMATION,
            PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD,
            PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQueryEx,
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
    FPS_SIGNATURE, MemoryScanStats, MetricsByteDiff, MetricsDryRunStatus, MetricsLayoutReport,
    MetricsMatch, MetricsScanReport, MetricsScanStatus, MetricsStringBoundary, classify_scan,
    layout, scan,
};

const CHUNK_SIZE: usize = 1024 * 1024;

struct ReadableRegion {
    base: u64,
    size: u64,
    state: u32,
    protection: u32,
    region_type: u32,
}

pub(super) fn scan_metrics(probe: ProbeReport) -> MetricsScanReport {
    scan_metrics_inner(probe, false, super::FPS_MARKER_REPLACEMENT).scan
}

pub(super) fn inspect_metrics_layout(probe: ProbeReport, replacement: &str) -> MetricsLayoutReport {
    let mut report = scan_metrics_inner(probe, true, replacement);
    report.dry_run.set_replacement(replacement);
    report
}

fn scan_metrics_inner(
    probe: ProbeReport,
    inspect_layout: bool,
    replacement: &str,
) -> MetricsLayoutReport {
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

    let (matches, stats, scan_error) = match scan_module(
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
    let status = classify_scan(matches.len(), stats.coverage == "FULL_MODULE");
    let mut scan_report = MetricsScanReport::new(status, probe);
    scan_report.scan_stats = Some(stats);
    scan_report.matches = matches;
    if let Some(message) = scan_error {
        scan_report.errors.push(ReportError {
            field: "memory.scan",
            code: "MEMORY_SCAN_INCOMPLETE",
            message,
        });
    } else if let Some(stats) = &scan_report.scan_stats
        && stats.coverage != "FULL_MODULE"
    {
        let message = if stats.failed_reads > 0 {
            format!("{} memory reads failed or were short", stats.failed_reads)
        } else {
            format!("{} module bytes were not readable", stats.bytes_unreadable)
        };
        scan_report.errors.push(ReportError {
            field: "memory.scan",
            code: "MEMORY_SCAN_INCOMPLETE",
            message,
        });
    }
    let mut report = MetricsLayoutReport::new(scan_report);
    report.dry_run.replacement = replacement.to_owned();
    if inspect_layout {
        inspect_unique_layout(
            process_handle.raw(),
            expected.module_base,
            expected.image_size,
            &mut report,
        );
    }

    if !same_process_identity(&recheck_pinned_process(&pinned[0]), &expected) {
        return fail(
            report.scan.probe,
            MetricsScanStatus::IdentityInconsistent,
            "process",
            "PROCESS_IDENTITY_CHANGED_DURING_SCAN",
            "the retained process identity changed while memory was scanned",
        );
    }
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
) -> MetricsLayoutReport {
    let mut report = MetricsLayoutReport::new(MetricsScanReport::new(status, probe));
    report.scan.errors.push(ReportError {
        field,
        code,
        message: message.to_owned(),
    });
    report
}

fn inspect_unique_layout(
    process: HANDLE,
    module_base: u64,
    module_size: u32,
    report: &mut MetricsLayoutReport,
) {
    if report.scan.matches.len() > 1 {
        report.dry_run.status = MetricsDryRunStatus::Ambiguous;
        report.dry_run.reason = Some("multiple signature candidates were observed".to_owned());
        return;
    }
    match report.scan.status {
        MetricsScanStatus::AmbiguousMatch | MetricsScanStatus::AmbiguousProcess => {
            report.dry_run.status = MetricsDryRunStatus::Ambiguous;
            report.dry_run.reason =
                Some("more than one process or signature candidate was found".to_owned());
            return;
        }
        MetricsScanStatus::FoundUniqueMatch => {}
        _ => {
            report.dry_run.status = MetricsDryRunStatus::Unsupported;
            report.dry_run.reason = Some(format!(
                "layout requires FOUND_UNIQUE_MATCH with FULL_MODULE coverage; scan status is {}",
                report.scan.status.label()
            ));
            return;
        }
    }
    if report
        .scan
        .scan_stats
        .as_ref()
        .is_none_or(|stats| stats.coverage != "FULL_MODULE")
    {
        report.dry_run.status = MetricsDryRunStatus::Unsupported;
        report.dry_run.reason = Some("layout requires complete module coverage".to_owned());
        return;
    }
    let Some(found) = report.scan.matches.first() else {
        report.dry_run.status = MetricsDryRunStatus::Unsupported;
        report.dry_run.reason = Some("the scan report contained no signature match".to_owned());
        return;
    };
    let region_end = match found.region_base.checked_add(found.region_size) {
        Some(end) => end,
        None => {
            layout_error(
                report,
                "LAYOUT_REGION_OVERFLOW",
                "match region range overflows",
            );
            return;
        }
    };
    let module_end = match module_base.checked_add(u64::from(module_size)) {
        Some(end) => end,
        None => {
            layout_error(report, "LAYOUT_MODULE_OVERFLOW", "module range overflows");
            return;
        }
    };
    let context_start = found
        .address
        .saturating_sub(16)
        .max(found.region_base)
        .max(module_base);
    let context_end = found
        .address
        .saturating_add(128)
        .min(region_end)
        .min(module_end);
    if context_end <= context_start {
        layout_error(
            report,
            "LAYOUT_CONTEXT_EMPTY",
            "no readable bytes surround the signature",
        );
        return;
    }
    let Ok(start) = usize::try_from(context_start) else {
        layout_error(
            report,
            "LAYOUT_ADDRESS_INVALID",
            "context address does not fit this process",
        );
        return;
    };
    let Ok(length) = usize::try_from(context_end - context_start) else {
        layout_error(
            report,
            "LAYOUT_LENGTH_INVALID",
            "context length does not fit this process",
        );
        return;
    };
    let mut bytes = vec![0u8; length];
    let mut bytes_read = 0usize;
    // SAFETY: the retained handle is read-only and this range lies within the matched readable region.
    let succeeded = unsafe {
        ReadProcessMemory(
            process,
            start as *const c_void,
            bytes.as_mut_ptr().cast(),
            length,
            &mut bytes_read,
        )
    };
    if succeeded == 0 || bytes_read != length {
        layout_error(
            report,
            "LAYOUT_CONTEXT_READ_FAILED",
            &format!(
                "could not read complete layout context (requested {length}, received {bytes_read}; Win32 error {})",
                unsafe { GetLastError() }
            ),
        );
        return;
    }
    report.region_type = Some(found.region_type);
    report.context_address = Some(context_start);
    report.context_module_offset = context_start.checked_sub(module_base);
    report.signature_offset_in_context = Some(found.address - context_start);
    report.original_bytes_hex = Some(hex(&bytes));
    let Ok(signature_index) = usize::try_from(found.address - context_start) else {
        layout_error(
            report,
            "LAYOUT_SIGNATURE_OFFSET_INVALID",
            "signature offset is invalid",
        );
        return;
    };
    let strings = match layout::parse_boundaries(&bytes, signature_index) {
        Ok(strings) => strings,
        Err(message) => {
            layout_error(report, "LAYOUT_BOUNDARIES_UNSUPPORTED", message);
            return;
        }
    };

    report.strings = strings
        .iter()
        .map(|string| {
            let start_address = found.address + string.start_offset as u64;
            let terminator_address = found.address + string.terminator_offset as u64;
            MetricsStringBoundary {
                name: string.name,
                start_address,
                module_offset: start_address - module_base,
                offset_from_signature: string.start_offset as u64,
                terminator_address,
                terminator_offset_from_signature: string.terminator_offset as u64,
                capacity_bytes: string.capacity_bytes as u64,
                max_payload_bytes: string.capacity_bytes.saturating_sub(1) as u64,
                capacity_basis: string.capacity_basis,
                original_value: string.original_value.clone(),
            }
        })
        .collect();

    let target = &strings[0];
    let slot_start = signature_index + target.start_offset;
    let replacement = report.dry_run.replacement.as_bytes();
    match layout::plan_patch(&bytes, slot_start, target.capacity_bytes, replacement) {
        Ok(plan) => {
            report.dry_run.status = MetricsDryRunStatus::Safe;
            report.dry_run.fits = Some(true);
            report.dry_run.reason = None;
            report.dry_run.patch_bytes = plan.write_plan.len() as u64;
            report.dry_run.diff = plan
                .diff
                .into_iter()
                .map(|diff| MetricsByteDiff {
                    offset_from_signature: diff.offset.saturating_sub(signature_index) as u64,
                    before: diff.before,
                    after: diff.after,
                })
                .collect();
            report.dry_run.write_plan = plan
                .write_plan
                .into_iter()
                .map(|diff| MetricsByteDiff {
                    offset_from_signature: diff.offset.saturating_sub(signature_index) as u64,
                    before: diff.before,
                    after: diff.after,
                })
                .collect();
        }
        Err(layout::PatchError::DoesNotFit) => {
            report.dry_run.status = MetricsDryRunStatus::Unsupported;
            report.dry_run.fits = Some(false);
            report.dry_run.reason = Some(format!(
                "replacement needs {} bytes including its terminator; the FPS label slot has {} bytes",
                replacement.len() + 1,
                target.capacity_bytes
            ));
        }
        Err(error) => {
            let message = match error {
                layout::PatchError::InvalidBoundary => "patch boundary is invalid",
                layout::PatchError::ContainsNul => "replacement contains a NUL byte",
                layout::PatchError::DoesNotFit => unreachable!(),
            };
            layout_error(report, "LAYOUT_PATCH_UNSUPPORTED", message);
        }
    }
}

fn layout_error(report: &mut MetricsLayoutReport, code: &'static str, message: &str) {
    report.dry_run.status = MetricsDryRunStatus::Unsupported;
    report.dry_run.fits = None;
    report.dry_run.reason = Some(message.to_owned());
    report.errors.push(ReportError {
        field: "layout",
        code,
        message: message.to_owned(),
    });
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
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
) -> Result<(Vec<MetricsMatch>, MemoryScanStats, Option<String>), String> {
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
        bytes_scanned: 0,
        bytes_untrusted: 0,
        bytes_unreadable: 0,
        failed_reads: 0,
        coverage: "READABLE_REGIONS_ONLY",
    };
    let mut scanner = scan::ChunkScanner::new(FPS_SIGNATURE);
    let mut readable_regions = Vec::new();
    let mut cursor = start;
    let mut scan_error = None;

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
            scan_error = Some(format!(
                "VirtualQueryEx failed at 0x{cursor:X} (Win32 error {})",
                unsafe { GetLastError() }
            ));
            stats.skipped_regions += 1;
            stats.bytes_unreadable += (end - cursor) as u64;
            scanner.unreadable_gap();
            break;
        }
        if received < size_of::<MEMORY_BASIC_INFORMATION>() {
            scan_error = Some("VirtualQueryEx returned a truncated memory region".to_owned());
            stats.skipped_regions += 1;
            stats.bytes_unreadable += (end - cursor) as u64;
            scanner.unreadable_gap();
            break;
        }
        let region_base = info.BaseAddress as usize;
        let region_size = info.RegionSize;
        let Some(region_end) = region_base.checked_add(region_size) else {
            scan_error = Some("memory region address range overflows".to_owned());
            stats.skipped_regions += 1;
            stats.bytes_unreadable += (end - cursor) as u64;
            scanner.unreadable_gap();
            break;
        };
        if region_end <= cursor {
            scan_error = Some("VirtualQueryEx did not advance through the module".to_owned());
            stats.skipped_regions += 1;
            stats.bytes_unreadable += (end - cursor) as u64;
            scanner.unreadable_gap();
            break;
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
                region_type: info.Type,
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
                let scanned = scanner.push_read(
                    address as u64,
                    &buffer[..bytes_read],
                    length,
                    succeeded != 0,
                );
                stats.record_read(length as u64, bytes_read as u64, scanned);
                address += length;
            }
        } else {
            stats.skipped_regions += 1;
            stats.bytes_unreadable += (range_end - range_start) as u64;
            scanner.unreadable_gap();
        }
        cursor = region_end;
    }

    stats.coverage = if stats.bytes_scanned == module_size
        && stats.bytes_untrusted == 0
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
                    module_offset: address - module_base,
                    region_type: region_type_name(region.region_type),
                })
        })
        .collect();
    Ok((matches, stats, scan_error))
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

fn region_type_name(region_type: u32) -> &'static str {
    match region_type {
        MEM_IMAGE => "MEM_IMAGE",
        MEM_PRIVATE => "MEM_PRIVATE",
        MEM_MAPPED => "MEM_MAPPED",
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

    #[test]
    fn reports_win32_memory_region_types() {
        assert_eq!(region_type_name(MEM_IMAGE), "MEM_IMAGE");
        assert_eq!(region_type_name(MEM_PRIVATE), "MEM_PRIVATE");
        assert_eq!(region_type_name(MEM_MAPPED), "MEM_MAPPED");
        assert_eq!(region_type_name(0), "UNKNOWN");
    }
}
