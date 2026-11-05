mod history;
mod objects;

use history::inspect_row_history;
#[cfg(test)]
use history::sample_row_history_once;
#[cfg(test)]
use objects::{bounded_region_length, sample_remote_object};
use objects::{inspect_target_object, now_unix_ms, sample_candidates};

use std::{collections::HashSet, ffi::c_void, mem::size_of};

use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE},
    System::{
        Diagnostics::Debug::ReadProcessMemory,
        Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQueryEx},
        SystemInformation::{GetSystemInfo, SYSTEM_INFO},
    },
};

use crate::{
    probe::{ProbeReport, ProbeStatus, ReportError},
    process::{
        ProcessIdentity, ProcessInspection, ProcessScanError, enumerate_pinned_game_processes,
        open_readonly_process, recheck_pinned_process,
    },
};

use super::{
    discovery::{
        MetricsChangedU32Field, MetricsNestedPointer, MetricsObjectCandidate, MetricsObjectRegion,
        MetricsObjectReport, MetricsObjectSample, MetricsObjectScanStats, MetricsObjectStatus,
        MetricsRowHistory, MetricsRowHistoryElement, MetricsRowHistorySample, MetricsRowPointer,
        MetricsTargetObject, NESTED_TARGET_PREVIEW_SIZE, OWNER_VTABLE_RVA, ROW_DATA_POINTER_OFFSET,
        ROW_HISTORY_ELEMENT_STRIDE, ROW_POINTERS, RowHistoryHeader, RowHistoryHeaderError,
        SAMPLE_COUNT, SAMPLE_INTERVAL_MS, SCAN_SCOPE, TARGET_OBJECT_SIZE, ascii_c_string_length,
        candidate_confidence, candidate_validation_status, changed_u32_fields, classify_scan,
        compare_row_history_elements, find_ascii_runs, is_aligned_pointer_candidate,
        is_expected_label, is_expected_nested_label, is_supported_metrics_layout,
        nul_terminated_ascii, parse_row_history_header, row_history_capacity_bytes,
        scan_coverage_label, scan_pointer_chunk, static_nested_pointer_access,
    },
    windows::{protection_name, readable_protection, region_type_name},
};

const CHUNK_SIZE: usize = 1024 * 1024;
const MAX_CANDIDATES: usize = 32;
const TARGET_PREVIEW_SIZE: usize = 64;
const MIN_USER_ADDRESS: u64 = 0x1_0000;
const MAX_USER_ADDRESS: u64 = 0x0000_8000_0000_0000;

struct ScanStats {
    start: u64,
    end: u64,
    regions: u64,
    readable_regions: u64,
    image_regions: u64,
    private_regions: u64,
    mapped_regions: u64,
    bytes_read: u64,
    failed_reads: u64,
    complete: bool,
}

pub(super) fn discover_metrics_objects(probe: ProbeReport) -> MetricsObjectReport {
    if probe.status != ProbeStatus::ExactMatch {
        let status = match probe.status {
            ProbeStatus::UnknownBuild => MetricsObjectStatus::UnknownBuild,
            ProbeStatus::NoProcess => MetricsObjectStatus::NoProcess,
            ProbeStatus::AmbiguousProcess => MetricsObjectStatus::AmbiguousProcess,
            ProbeStatus::AccessDenied => MetricsObjectStatus::AccessDenied,
            ProbeStatus::IdentityInconsistent => MetricsObjectStatus::IdentityInconsistent,
            ProbeStatus::UnsupportedPlatform => MetricsObjectStatus::UnsupportedPlatform,
            ProbeStatus::ProbeError => MetricsObjectStatus::ProbeError,
            ProbeStatus::ExactMatch => unreachable!(),
        };
        return MetricsObjectReport::new(status, probe_status_label(probe.status), probe);
    }
    if !is_supported_metrics_layout(&probe) {
        return error_report(
            probe,
            MetricsObjectStatus::UnsupportedLayout,
            "METRICS_LAYOUT_UNSUPPORTED",
            "the executable is recognized, but no metrics-object layout is registered for its exact build fingerprint",
        );
    }
    if size_of::<usize>() != size_of::<u64>() {
        return error_report(
            probe,
            MetricsObjectStatus::UnsupportedPlatform,
            "PROCESS_POINTER_WIDTH_UNSUPPORTED",
            "the registered DOOM build requires a 64-bit DASRU process",
        );
    }

    if probe.processes.len() != 1 {
        return error_report(
            probe,
            MetricsObjectStatus::IdentityInconsistent,
            "PROCESS_IDENTITY_MISSING",
            "exactly one verified game process is required",
        );
    }
    let (pid, expected) = match &probe.processes[0].inspection {
        ProcessInspection::Available { identity } => (probe.processes[0].pid, identity.clone()),
        ProcessInspection::Unavailable { failure } => {
            let status = if failure.kind == crate::process::ProcessFailureKind::AccessDenied {
                MetricsObjectStatus::AccessDenied
            } else {
                MetricsObjectStatus::IdentityInconsistent
            };
            let message = format!("{} failed: {:?}", failure.operation, failure.kind);
            return error_report(probe, status, "PROCESS_IDENTITY_UNAVAILABLE", &message);
        }
    };
    let pinned = match enumerate_pinned_game_processes() {
        Ok(candidates) => candidates,
        Err(error) => return process_error_report(probe, error),
    };
    if pinned.len() != 1
        || pinned[0].candidate.pid != pid
        || !matches!(&pinned[0].candidate.inspection, ProcessInspection::Available { identity } if identity == &expected)
    {
        return error_report(
            probe,
            if pinned.len() > 1 {
                MetricsObjectStatus::AmbiguousProcess
            } else {
                MetricsObjectStatus::IdentityInconsistent
            },
            "PROCESS_SET_CHANGED",
            "the verified process set changed before memory discovery",
        );
    }
    let process = match open_readonly_process(pid, &expected) {
        Ok(process) => process,
        Err(ProcessInspection::Unavailable { failure }) => {
            let status = if failure.kind == crate::process::ProcessFailureKind::AccessDenied {
                MetricsObjectStatus::AccessDenied
            } else {
                MetricsObjectStatus::IdentityInconsistent
            };
            return error_report(
                probe,
                status,
                "READ_ONLY_ATTACH_FAILED",
                &format!("{} failed: {:?}", failure.operation, failure.kind),
            );
        }
        Err(ProcessInspection::Available { .. }) => {
            return error_report(
                probe,
                MetricsObjectStatus::IdentityInconsistent,
                "READ_ONLY_ATTACH_FAILED",
                "read-only process attachment returned no usable handle",
            );
        }
    };
    let Some(vtable_address) = expected.module_base.checked_add(OWNER_VTABLE_RVA) else {
        return error_report(
            probe,
            MetricsObjectStatus::IdentityInconsistent,
            "OWNER_VTABLE_OFFSET_INVALID",
            "registered build module base plus vtable offset overflowed",
        );
    };
    if OWNER_VTABLE_RVA >= u64::from(expected.image_size) {
        return error_report(
            probe,
            MetricsObjectStatus::IdentityInconsistent,
            "OWNER_VTABLE_OUTSIDE_MODULE",
            "registered build module is smaller than the expected metrics vtable offset",
        );
    }

    let (addresses, stats, mut scan_errors) =
        scan_process_for_pointer(process.raw(), vtable_address);
    let mut candidate_errors = Vec::new();
    if addresses.len() > MAX_CANDIDATES {
        candidate_errors.push(ReportError {
            field: "candidate_validation",
            code: "CANDIDATE_LIMIT_REACHED",
            message: format!(
                "more than {MAX_CANDIDATES} vtable references were found; results are truncated"
            ),
        });
    }
    let mut candidates = Vec::new();
    for address in addresses.into_iter().take(MAX_CANDIDATES) {
        if let Some(candidate) = inspect_owner(
            process.raw(),
            address,
            vtable_address,
            &expected,
            &mut candidate_errors,
        ) {
            candidates.push(candidate);
        }
    }
    let (
        sampled_object_count,
        sampled_row_history_count,
        sampling_complete,
        sampling_identity_consistent,
    ) = sample_candidates(
        process.raw(),
        &mut candidates,
        || matches!(recheck_pinned_process(&pinned[0]), ProcessInspection::Available { identity } if identity == expected),
    );
    let identity_consistent = sampling_identity_consistent
        && matches!(recheck_pinned_process(&pinned[0]), ProcessInspection::Available { identity } if identity == expected);
    let status = if identity_consistent {
        classify_scan(candidates.len(), stats.complete)
    } else {
        scan_errors.push(ReportError {
            field: "process",
            code: "PROCESS_IDENTITY_REVALIDATION_FAILED",
            message: if sampling_identity_consistent {
                "the retained process identity could not be verified after memory sampling"
                    .to_owned()
            } else {
                "the retained process identity could not be verified between memory samples"
                    .to_owned()
            },
        });
        MetricsObjectStatus::IdentityInconsistent
    };
    let mut report = MetricsObjectReport::new(status, "EXACT_MATCH", probe);
    report.process = Some(expected);
    report.scan_stats = Some(MetricsObjectScanStats {
        scan_scope: SCAN_SCOPE,
        address_range_start: stats.start,
        address_range_end: stats.end,
        regions_visited: stats.regions,
        readable_regions_scanned: stats.readable_regions,
        image_regions_scanned: stats.image_regions,
        private_regions_scanned: stats.private_regions,
        mapped_regions_scanned: stats.mapped_regions,
        bytes_read: stats.bytes_read,
        failed_reads: stats.failed_reads,
        coverage: scan_coverage_label(stats.complete),
    });
    report.candidates = candidates;
    report.candidate_validation_status = candidate_validation_status(&candidate_errors);
    report.sampling_status = if sampled_object_count == 0 && sampled_row_history_count == 0 {
        "NO_OBJECTS"
    } else if sampling_complete {
        "COMPLETE"
    } else {
        "PARTIAL"
    };
    report.sampled_object_count = sampled_object_count;
    report.requested_sample_count = if sampled_object_count == 0 {
        0
    } else {
        SAMPLE_COUNT
    };
    report.sample_interval_ms = if sampled_object_count == 0 && sampled_row_history_count == 0 {
        0
    } else {
        SAMPLE_INTERVAL_MS
    };
    report.sampled_row_history_count = sampled_row_history_count;
    report.requested_row_history_sample_count = if sampled_row_history_count == 0 {
        0
    } else {
        SAMPLE_COUNT
    };
    report.errors = scan_errors;
    report.errors.extend(candidate_errors);
    report
}

fn scan_process_for_pointer(handle: HANDLE, value: u64) -> (Vec<u64>, ScanStats, Vec<ReportError>) {
    let mut system_info = SYSTEM_INFO::default();
    // SAFETY: GetSystemInfo fills the writable, correctly sized system information struct.
    unsafe { GetSystemInfo(&mut system_info) };
    let start = system_info.lpMinimumApplicationAddress as usize as u64;
    let maximum = system_info.lpMaximumApplicationAddress as usize as u64;
    let end = maximum.saturating_add(1);
    let mut stats = ScanStats {
        start,
        end,
        regions: 0,
        readable_regions: 0,
        image_regions: 0,
        private_regions: 0,
        mapped_regions: 0,
        bytes_read: 0,
        failed_reads: 0,
        complete: true,
    };
    let mut matches = HashSet::new();
    let mut errors = Vec::new();
    let mut cursor = start as usize;
    let end = end as usize;

    while cursor < end {
        // SAFETY: a zeroed output structure is valid for VirtualQueryEx to fill.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: handle is a retained read-only process handle and info is writable.
        let received = unsafe {
            VirtualQueryEx(
                handle,
                cursor as *const c_void,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if received < size_of::<MEMORY_BASIC_INFORMATION>() {
            stats.complete = false;
            errors.push(ReportError {
                field: "memory.scan",
                code: "VIRTUAL_QUERY_FAILED",
                message: format!(
                    "VirtualQueryEx failed at 0x{cursor:X} (Win32 error {})",
                    unsafe { GetLastError() }
                ),
            });
            break;
        }
        stats.regions += 1;
        let region_base = info.BaseAddress as usize;
        let Some(region_end) = region_base.checked_add(info.RegionSize) else {
            stats.complete = false;
            errors.push(ReportError {
                field: "memory.scan",
                code: "REGION_RANGE_OVERFLOW",
                message: format!("memory region at 0x{region_base:X} overflows"),
            });
            break;
        };
        if region_end <= cursor {
            stats.complete = false;
            errors.push(ReportError {
                field: "memory.scan",
                code: "REGION_WALK_STALLED",
                message: format!("VirtualQueryEx did not advance at 0x{cursor:X}"),
            });
            break;
        }
        if info.State == MEM_COMMIT
            && readable_protection(info.Protect)
            && relevant_region_type(info.Type)
        {
            stats.readable_regions += 1;
            match info.Type {
                windows_sys::Win32::System::Memory::MEM_IMAGE => stats.image_regions += 1,
                windows_sys::Win32::System::Memory::MEM_PRIVATE => stats.private_regions += 1,
                windows_sys::Win32::System::Memory::MEM_MAPPED => stats.mapped_regions += 1,
                _ => unreachable!(),
            }
            let mut address = cursor.max(region_base);
            let scan_end = end.min(region_end);
            let mut tail = Vec::new();
            while address < scan_end {
                let length = CHUNK_SIZE.min(scan_end - address);
                let mut buffer = vec![0u8; length];
                let mut bytes_read = 0usize;
                // SAFETY: this region is committed and readable; buffer is writable for length.
                let succeeded = unsafe {
                    ReadProcessMemory(
                        handle,
                        address as *const c_void,
                        buffer.as_mut_ptr().cast(),
                        length,
                        &mut bytes_read,
                    )
                };
                stats.bytes_read += bytes_read as u64;
                if !scan_pointer_chunk(
                    &buffer,
                    address as u64,
                    (length, bytes_read, succeeded != 0),
                    value,
                    &mut tail,
                    &mut matches,
                ) {
                    stats.failed_reads += 1;
                    stats.complete = false;
                    errors.push(ReportError {
                        field: "memory.scan",
                        code: "MEMORY_READ_INCOMPLETE",
                        message: format!(
                            "ReadProcessMemory returned {bytes_read} of {length} bytes at 0x{address:X} (Win32 error {})",
                            unsafe { GetLastError() }
                        ),
                    });
                }
                address += length;
            }
        }
        cursor = region_end;
    }

    let mut matches: Vec<_> = matches.into_iter().collect();
    matches.sort_unstable();
    (matches, stats, errors)
}

fn inspect_owner(
    handle: HANDLE,
    owner_address: u64,
    vtable_address: u64,
    identity: &ProcessIdentity,
    errors: &mut Vec<ReportError>,
) -> Option<MetricsObjectCandidate> {
    let Some(owner_region) = query_region(handle, owner_address) else {
        errors.push(ReportError {
            field: "owner",
            code: "OWNER_REGION_QUERY_FAILED",
            message: format!("could not query candidate owner at 0x{owner_address:X}"),
        });
        return None;
    };
    if owner_region.state != MEM_COMMIT || !readable_protection(owner_region.protection) {
        return None;
    }
    let mut vtable = [0u8; 8];
    if !read_exact(handle, owner_address, &mut vtable) {
        errors.push(ReportError {
            field: "owner",
            code: "OWNER_READ_FAILED",
            message: format!("could not read candidate owner at 0x{owner_address:X}"),
        });
        return None;
    }
    if u64::from_ne_bytes(vtable) != vtable_address {
        return None;
    }
    let mut rows = Vec::with_capacity(ROW_POINTERS.len());
    let mut valid = 0u8;
    let mut nulls = 0u8;
    let mut expected_label_matches = 0u8;
    let mut nested_label_matches = 0u8;
    let module_end = identity
        .module_base
        .checked_add(u64::from(identity.image_size));
    for (label, offset) in ROW_POINTERS {
        let slot_address = owner_address.checked_add(offset)?;
        let Some(slot_region) = query_region(handle, slot_address) else {
            errors.push(ReportError {
                field: "rows",
                code: "ROW_SLOT_REGION_QUERY_FAILED",
                message: format!("could not query {label} row slot at 0x{slot_address:X}"),
            });
            return None;
        };
        if slot_region.state != MEM_COMMIT
            || !readable_protection(slot_region.protection)
            || slot_region.allocation_base != owner_region.allocation_base
        {
            return None;
        }
        let mut bytes = [0u8; 8];
        if !read_exact(handle, slot_address, &mut bytes) {
            errors.push(ReportError {
                field: "rows",
                code: "ROW_POINTER_READ_FAILED",
                message: format!("could not read {label} row pointer at 0x{slot_address:X}"),
            });
            return None;
        }
        let target = u64::from_ne_bytes(bytes);
        if target == 0 {
            nulls += 1;
            rows.push(MetricsRowPointer {
                label,
                owner_offset: offset,
                slot_address,
                slot_module_offset: module_offset(slot_address, identity.module_base, module_end),
                slot_region: public_region(slot_region, identity),
                target_address: None,
                target_module_offset: None,
                target_region: None,
                pointer_chain: vec![owner_address, slot_address],
                target_classification: "NULL",
                expected_label_match: false,
                target_preview_hex: None,
                target_object: None,
                row_history: None,
            });
            continue;
        }
        let Some(region) = query_region(handle, target) else {
            errors.push(ReportError {
                field: "rows",
                code: "ROW_TARGET_UNMAPPED",
                message: format!("{label} row target 0x{target:X} is not mapped"),
            });
            rows.push(MetricsRowPointer {
                label,
                owner_offset: offset,
                slot_address,
                slot_module_offset: module_offset(slot_address, identity.module_base, module_end),
                slot_region: public_region(slot_region, identity),
                target_address: Some(target),
                target_module_offset: module_offset(target, identity.module_base, module_end),
                target_region: None,
                pointer_chain: vec![owner_address, slot_address, target],
                target_classification: "UNMAPPED",
                expected_label_match: false,
                target_preview_hex: None,
                target_object: None,
                row_history: None,
            });
            continue;
        };
        if region.state != MEM_COMMIT || !readable_protection(region.protection) {
            errors.push(ReportError {
                field: "rows",
                code: "ROW_TARGET_UNREADABLE",
                message: format!("{label} row target 0x{target:X} is not readable"),
            });
            rows.push(MetricsRowPointer {
                label,
                owner_offset: offset,
                slot_address,
                slot_module_offset: module_offset(slot_address, identity.module_base, module_end),
                slot_region: public_region(slot_region, identity),
                target_address: Some(target),
                target_module_offset: module_offset(target, identity.module_base, module_end),
                target_region: Some(public_region(region, identity)),
                pointer_chain: vec![owner_address, slot_address, target],
                target_classification: "UNREADABLE_REGION",
                expected_label_match: false,
                target_preview_hex: None,
                target_object: None,
                row_history: None,
            });
            continue;
        }
        valid += 1;
        let target_object = inspect_target_object(
            handle,
            target,
            region,
            identity,
            owner_address,
            slot_address,
            errors,
        );
        let preview = &target_object.baseline_bytes
            [..TARGET_PREVIEW_SIZE.min(target_object.baseline_bytes.len())];
        let string_length = ascii_c_string_length(preview);
        let row_history = string_length.is_none().then(|| {
            inspect_row_history(
                handle,
                slot_address,
                target,
                slot_region.allocation_base,
                region.allocation_base,
            )
        });
        let label_match = is_expected_label(preview, label);
        expected_label_matches += u8::from(label_match);
        nested_label_matches += u8::from(target_object.nested_pointers.iter().any(|pointer| {
            is_expected_nested_label(pointer.nul_terminated_ascii.as_deref(), label)
        }));
        let classification = if string_length.is_some() {
            "NULL_TERMINATED_ASCII_STRING"
        } else {
            match region.region_type {
                windows_sys::Win32::System::Memory::MEM_IMAGE => "IMAGE_RESIDENT_OBJECT_OR_DATA",
                windows_sys::Win32::System::Memory::MEM_PRIVATE => "PRIVATE_MEMORY_OBJECT",
                windows_sys::Win32::System::Memory::MEM_MAPPED => "MAPPED_MEMORY_OBJECT",
                _ => "UNCLASSIFIED_OBJECT",
            }
        };
        rows.push(MetricsRowPointer {
            label,
            owner_offset: offset,
            slot_address,
            slot_module_offset: module_offset(slot_address, identity.module_base, module_end),
            slot_region: public_region(slot_region, identity),
            target_address: Some(target),
            target_module_offset: module_offset(target, identity.module_base, module_end),
            target_region: Some(public_region(region, identity)),
            pointer_chain: vec![owner_address, slot_address, target],
            target_classification: classification,
            expected_label_match: label_match,
            target_preview_hex: (!preview.is_empty()).then(|| hex(preview)),
            target_object: Some(target_object),
            row_history,
        });
    }
    Some(MetricsObjectCandidate {
        owner_address,
        owner_module_offset: module_offset(owner_address, identity.module_base, module_end),
        owner_region: public_region(owner_region, identity),
        vtable_address,
        vtable_module_offset: OWNER_VTABLE_RVA,
        valid_row_pointers: valid,
        null_row_pointers: nulls,
        expected_label_matches,
        direct_label_matches: expected_label_matches,
        nested_label_matches,
        confidence_basis: "VALID_ROW_POINTERS_AND_DIRECT_LABEL_MATCHES_ONLY; NESTED_LABELS_DO_NOT_UPGRADE",
        static_setup_reference_confirmed: true,
        owner_fields_share_allocation: true,
        confidence: candidate_confidence(valid, expected_label_matches),
        renderer_consumption: "STATIC_FUNCTION_SLOT_READS_CONFIRMED; RUNTIME_RENDERER_CONSUMPTION_UNCONFIRMED",
        rows,
    })
}

fn query_region(handle: HANDLE, address: u64) -> Option<RemoteRegion> {
    let address = usize::try_from(address).ok()?;
    // SAFETY: a zeroed output structure is valid for VirtualQueryEx to fill.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: handle is valid and info is writable for the Win32 structure size.
    let received = unsafe {
        VirtualQueryEx(
            handle,
            address as *const c_void,
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    (received >= size_of::<MEMORY_BASIC_INFORMATION>()).then_some(RemoteRegion {
        allocation_base: info.AllocationBase as usize as u64,
        region_base: info.BaseAddress as usize as u64,
        region_size: info.RegionSize as u64,
        state: info.State,
        protection: info.Protect,
        region_type: info.Type,
    })
}

#[derive(Clone, Copy)]
struct RemoteRegion {
    allocation_base: u64,
    region_base: u64,
    region_size: u64,
    state: u32,
    protection: u32,
    region_type: u32,
}

fn public_region(region: RemoteRegion, identity: &ProcessIdentity) -> MetricsObjectRegion {
    let module_end = identity
        .module_base
        .checked_add(u64::from(identity.image_size));
    MetricsObjectRegion {
        allocation_base: region.allocation_base,
        region_base: region.region_base,
        region_size: region.region_size,
        state: region.state,
        protection: region.protection,
        protection_name: protection_name(region.protection),
        region_type: region_type_name(region.region_type),
        module_offset: module_offset(region.region_base, identity.module_base, module_end),
    }
}

fn module_offset(address: u64, base: u64, end: Option<u64>) -> Option<u64> {
    let end = end?;
    if address >= base && address < end {
        Some(address - base)
    } else {
        None
    }
}

fn read_exact(handle: HANDLE, address: u64, output: &mut [u8]) -> bool {
    let Ok(address) = usize::try_from(address) else {
        return false;
    };
    let mut received = 0usize;
    // SAFETY: output is writable for its length and the caller validates address readability.
    let succeeded = unsafe {
        ReadProcessMemory(
            handle,
            address as *const c_void,
            output.as_mut_ptr().cast(),
            output.len(),
            &mut received,
        )
    };
    succeeded != 0 && received == output.len()
}

fn relevant_region_type(region_type: u32) -> bool {
    matches!(
        region_type,
        windows_sys::Win32::System::Memory::MEM_IMAGE
            | windows_sys::Win32::System::Memory::MEM_PRIVATE
            | windows_sys::Win32::System::Memory::MEM_MAPPED
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn error_report(
    probe: ProbeReport,
    status: MetricsObjectStatus,
    code: &'static str,
    message: &str,
) -> MetricsObjectReport {
    let mut report = MetricsObjectReport::new(status, probe_status_label(probe.status), probe);
    report.errors.push(ReportError {
        field: "discovery",
        code,
        message: message.to_owned(),
    });
    report
}

fn process_error_report(probe: ProbeReport, error: ProcessScanError) -> MetricsObjectReport {
    let status = match &error {
        ProcessScanError::UnsupportedPlatform => MetricsObjectStatus::UnsupportedPlatform,
        ProcessScanError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            MetricsObjectStatus::AccessDenied
        }
        ProcessScanError::Io { .. } => MetricsObjectStatus::ProbeError,
    };
    error_report(
        probe,
        status,
        "PROCESS_REVALIDATION_FAILED",
        &error.to_string(),
    )
}

fn probe_status_label(status: ProbeStatus) -> &'static str {
    match status {
        ProbeStatus::ExactMatch => "EXACT_MATCH",
        ProbeStatus::UnknownBuild => "UNKNOWN_BUILD",
        ProbeStatus::AmbiguousProcess => "AMBIGUOUS_PROCESS",
        ProbeStatus::AccessDenied => "ACCESS_DENIED",
        ProbeStatus::IdentityInconsistent => "IDENTITY_INCONSISTENT",
        ProbeStatus::NoProcess => "NO_PROCESS",
        ProbeStatus::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
        ProbeStatus::ProbeError => "PROBE_ERROR",
    }
}

#[cfg(test)]
#[path = "discovery_windows/tests.rs"]
mod tests;
