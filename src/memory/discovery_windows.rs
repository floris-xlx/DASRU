use std::{
    collections::HashSet,
    ffi::c_void,
    mem::size_of,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
const ROW_HISTORY_HEADER_SIZE: usize = 16;
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

#[derive(Clone, Copy)]
struct RemoteSpanSegment {
    address: u64,
    length: u64,
}

struct RowHistoryRead {
    header: Option<RowHistoryHeader>,
    status: &'static str,
    data_allocation_base: Option<u64>,
    capacity_bytes: Option<u64>,
    elements: Vec<Vec<u8>>,
    error: Option<ReportError>,
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

fn inspect_row_history(
    handle: HANDLE,
    slot_address: u64,
    row_address: u64,
    slot_allocation_base: u64,
    row_allocation_base: u64,
) -> MetricsRowHistory {
    let read = read_row_history_state(
        handle,
        slot_address,
        row_address,
        slot_allocation_base,
        row_allocation_base,
    );
    let data_address = read.header.map(|header| header.data_address);
    let count = read.header.map(|header| header.count);
    let capacity = read.header.map(|header| header.capacity);
    let elements = read
        .elements
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let offset = (index as u64) * u64::from(ROW_HISTORY_ELEMENT_STRIDE);
            MetricsRowHistoryElement {
                index: index as u32,
                address: data_address
                    .and_then(|base| base.checked_add(offset))
                    .expect("validated history span contains every reported element"),
                bytes_hex: hex(bytes),
            }
        })
        .collect::<Vec<_>>();
    let observed_at_unix_ms = now_unix_ms();
    let sample = MetricsRowHistorySample {
        observed_at_unix_ms,
        status: read.status,
        data_address,
        count,
        capacity,
        capacity_bytes: read.capacity_bytes,
        data_allocation_base: read.data_allocation_base,
        data_pointer_changed: None,
        count_changed: None,
        capacity_changed: None,
        changed_elements: Vec::new(),
        removed_element_indices: Vec::new(),
        read_error: read.error.clone(),
    };
    let inspected_element_count = read.elements.len() as u32;
    MetricsRowHistory {
        row_address,
        status: read.status,
        data_address,
        count,
        capacity,
        data_allocation_base: read.data_allocation_base,
        capacity_bytes: read.capacity_bytes,
        allocation_validation: "COMMITTED_READABLE_VIRTUAL_ALLOCATION; HEAP_BLOCK_BOUNDARY_UNOBSERVED",
        element_stride_bytes: ROW_HISTORY_ELEMENT_STRIDE,
        interpretation: "RAW_BYTES_AND_ALIGNED_4_BYTE_VIEWS; FIELD_SEMANTICS_UNCONFIRMED",
        inspected_element_count,
        omitted_element_count: count
            .map_or(0, |count| count.saturating_sub(inspected_element_count)),
        elements,
        samples: vec![sample],
        last_header: read.header,
        last_elements: read.elements,
    }
}

fn read_row_history_state(
    handle: HANDLE,
    slot_address: u64,
    row_address: u64,
    slot_allocation_base: u64,
    row_allocation_base: u64,
) -> RowHistoryRead {
    let Some(slot_region) = query_region(handle, slot_address) else {
        return history_read_error(
            None,
            "ROW_SLOT_UNMAPPED",
            "HISTORY_ROW_SLOT_UNMAPPED",
            format!("row pointer slot 0x{slot_address:X} is not mapped"),
        );
    };
    if slot_region.state != MEM_COMMIT
        || !readable_protection(slot_region.protection)
        || slot_region.allocation_base != slot_allocation_base
    {
        return history_read_error(
            None,
            "ROW_SLOT_REGION_CHANGED",
            "HISTORY_ROW_SLOT_REGION_CHANGED",
            format!("row pointer slot 0x{slot_address:X} changed region"),
        );
    }
    let mut row_pointer_bytes = [0u8; size_of::<u64>()];
    if !read_exact(handle, slot_address, &mut row_pointer_bytes)
        || u64::from_le_bytes(row_pointer_bytes) != row_address
    {
        return history_read_error(
            None,
            "ROW_POINTER_CHANGED",
            "HISTORY_ROW_POINTER_CHANGED",
            format!("row pointer slot 0x{slot_address:X} no longer targets 0x{row_address:X}"),
        );
    }

    let Some(row_region) = query_region(handle, row_address) else {
        return history_read_error(
            None,
            "ROW_TARGET_UNMAPPED",
            "HISTORY_ROW_TARGET_UNMAPPED",
            format!("row object 0x{row_address:X} is not mapped"),
        );
    };
    if row_region.state != MEM_COMMIT
        || !readable_protection(row_region.protection)
        || row_region.allocation_base != row_allocation_base
    {
        return history_read_error(
            None,
            "ROW_TARGET_REGION_CHANGED",
            "HISTORY_ROW_TARGET_REGION_CHANGED",
            format!("row object 0x{row_address:X} changed region"),
        );
    }
    let Some(header_address) = row_address.checked_add(u64::from(ROW_DATA_POINTER_OFFSET)) else {
        return history_read_error(
            None,
            "ROW_HEADER_ADDRESS_OVERFLOW",
            "HISTORY_ROW_HEADER_ADDRESS_OVERFLOW",
            format!("row object 0x{row_address:X} has an overflowing history-header address"),
        );
    };
    let Some(header_region) = query_region(handle, header_address) else {
        return history_read_error(
            None,
            "ROW_HEADER_UNMAPPED",
            "HISTORY_ROW_HEADER_UNMAPPED",
            format!("history header at 0x{header_address:X} is not mapped"),
        );
    };
    if header_region.state != MEM_COMMIT
        || !readable_protection(header_region.protection)
        || header_region.allocation_base != row_allocation_base
        || bounded_region_length(header_address, header_region, ROW_HISTORY_HEADER_SIZE)
            != ROW_HISTORY_HEADER_SIZE
    {
        return history_read_error(
            None,
            "ROW_HEADER_REGION_INVALID",
            "HISTORY_ROW_HEADER_REGION_INVALID",
            format!(
                "history header at 0x{header_address:X} is outside its readable row allocation"
            ),
        );
    }
    let mut header_bytes = [0u8; ROW_HISTORY_HEADER_SIZE];
    if !read_exact(handle, header_address, &mut header_bytes) {
        return history_read_error(
            None,
            "ROW_HEADER_READ_FAILED",
            "HISTORY_ROW_HEADER_READ_FAILED",
            format!("could not read history header at 0x{header_address:X}"),
        );
    }
    let header = match parse_row_history_header(&header_bytes) {
        Ok(header) => header,
        Err(error) => {
            return history_read_error(
                None,
                "INCOMPLETE_HEADER",
                "HISTORY_HEADER_INCOMPLETE",
                format!("could not decode row history header: {error:?}"),
            );
        }
    };
    let capacity_bytes = match row_history_capacity_bytes(header) {
        Ok(bytes) => bytes,
        Err(error) => {
            let (status, code) = history_header_error_status(error);
            return history_read_error(
                Some(header),
                status,
                code,
                format!("row history header failed validation: {error:?}"),
            );
        }
    };
    if capacity_bytes == 0 {
        return RowHistoryRead {
            header: Some(header),
            status: "EMPTY",
            data_allocation_base: None,
            capacity_bytes: Some(0),
            elements: Vec::new(),
            error: None,
        };
    }
    if !is_aligned_pointer_candidate(header.data_address, MIN_USER_ADDRESS, MAX_USER_ADDRESS) {
        return history_read_error(
            Some(header),
            "INVALID_DATA_POINTER",
            "HISTORY_DATA_POINTER_INVALID",
            format!(
                "history data pointer 0x{:X} is outside the aligned user range",
                header.data_address
            ),
        );
    }
    let (data_allocation_base, segments) =
        match readable_span_segments(handle, header.data_address, capacity_bytes) {
            Ok(span) => span,
            Err(error) => {
                return RowHistoryRead {
                    header: Some(header),
                    status: "ARRAY_ALLOCATION_INVALID",
                    data_allocation_base: None,
                    capacity_bytes: Some(capacity_bytes),
                    elements: Vec::new(),
                    error: Some(error),
                };
            }
        };
    let element_bytes = (header.count as usize) * usize::from(ROW_HISTORY_ELEMENT_STRIDE);
    let bytes = match read_remote_segments(handle, &segments, element_bytes) {
        Ok(bytes) => bytes,
        Err(error) => {
            return RowHistoryRead {
                header: Some(header),
                status: "ARRAY_READ_FAILED",
                data_allocation_base: Some(data_allocation_base),
                capacity_bytes: Some(capacity_bytes),
                elements: Vec::new(),
                error: Some(error),
            };
        }
    };
    let elements = bytes
        .chunks_exact(usize::from(ROW_HISTORY_ELEMENT_STRIDE))
        .map(<[u8]>::to_vec)
        .collect();
    RowHistoryRead {
        header: Some(header),
        status: if header.count == 0 {
            "EMPTY"
        } else {
            "READ_OK"
        },
        data_allocation_base: Some(data_allocation_base),
        capacity_bytes: Some(capacity_bytes),
        elements,
        error: None,
    }
}

fn history_header_error_status(error: RowHistoryHeaderError) -> (&'static str, &'static str) {
    match error {
        RowHistoryHeaderError::IncompleteHeader => {
            ("INCOMPLETE_HEADER", "HISTORY_HEADER_INCOMPLETE")
        }
        RowHistoryHeaderError::CountExceedsCapacity => {
            ("COUNT_EXCEEDS_CAPACITY", "HISTORY_COUNT_EXCEEDS_CAPACITY")
        }
        RowHistoryHeaderError::CapacityExceedsLimit => {
            ("CAPACITY_LIMIT_EXCEEDED", "HISTORY_CAPACITY_LIMIT_EXCEEDED")
        }
        RowHistoryHeaderError::NullDataAddress => {
            ("NULL_DATA_POINTER", "HISTORY_DATA_POINTER_NULL")
        }
        RowHistoryHeaderError::MisalignedDataAddress => {
            ("MISALIGNED_DATA_POINTER", "HISTORY_DATA_POINTER_MISALIGNED")
        }
        RowHistoryHeaderError::SpanOverflow => {
            ("ARRAY_SPAN_OVERFLOW", "HISTORY_ARRAY_SPAN_OVERFLOW")
        }
        RowHistoryHeaderError::MalformedElement => {
            ("ELEMENT_SIZE_INVALID", "HISTORY_ELEMENT_SIZE_INVALID")
        }
    }
}

fn history_read_error(
    header: Option<RowHistoryHeader>,
    status: &'static str,
    code: &'static str,
    message: String,
) -> RowHistoryRead {
    RowHistoryRead {
        header,
        status,
        data_allocation_base: None,
        capacity_bytes: None,
        elements: Vec::new(),
        error: Some(ReportError {
            field: "row_history",
            code,
            message,
        }),
    }
}

fn readable_span_segments(
    handle: HANDLE,
    address: u64,
    length: u64,
) -> Result<(u64, Vec<RemoteSpanSegment>), ReportError> {
    let Some(end) = address.checked_add(length) else {
        return Err(ReportError {
            field: "row_history",
            code: "HISTORY_ARRAY_SPAN_OVERFLOW",
            message: "history array end address overflowed".to_owned(),
        });
    };
    let mut cursor = address;
    let mut allocation_base = None;
    let mut segments = Vec::new();
    while cursor < end {
        let Some(region) = query_region(handle, cursor) else {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_REGION_QUERY_FAILED",
                message: format!("could not query history array at 0x{cursor:X}"),
            });
        };
        let Some(region_end) = region.region_base.checked_add(region.region_size) else {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_REGION_OVERFLOW",
                message: format!("history array region at 0x{cursor:X} has invalid bounds"),
            });
        };
        if cursor < region.region_base
            || cursor >= region_end
            || region_end <= cursor
            || region.state != MEM_COMMIT
            || !readable_protection(region.protection)
        {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_REGION_UNREADABLE",
                message: format!("history array at 0x{cursor:X} crosses an unreadable region"),
            });
        }
        if allocation_base.is_some_and(|base| base != region.allocation_base) {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_ALLOCATION_MISMATCH",
                message: "history array capacity crosses allocation boundaries".to_owned(),
            });
        }
        allocation_base.get_or_insert(region.allocation_base);
        let segment_end = end.min(region_end);
        segments.push(RemoteSpanSegment {
            address: cursor,
            length: segment_end - cursor,
        });
        cursor = segment_end;
    }
    Ok((allocation_base.unwrap_or_default(), segments))
}

fn read_remote_segments(
    handle: HANDLE,
    segments: &[RemoteSpanSegment],
    length: usize,
) -> Result<Vec<u8>, ReportError> {
    let mut bytes = Vec::with_capacity(length);
    for segment in segments {
        let remaining = length.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        let segment_length = usize::try_from(segment.length)
            .unwrap_or(usize::MAX)
            .min(remaining);
        let mut part = vec![0; segment_length];
        if !read_exact(handle, segment.address, &mut part) {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_READ_FAILED",
                message: format!(
                    "ReadProcessMemory failed at 0x{:X} for {segment_length} bytes (Win32 error {})",
                    segment.address,
                    unsafe { GetLastError() }
                ),
            });
        }
        bytes.extend_from_slice(&part);
    }
    if bytes.len() != length {
        return Err(ReportError {
            field: "row_history",
            code: "HISTORY_ARRAY_READ_SHORT",
            message: format!("read {} of {length} requested history bytes", bytes.len()),
        });
    }
    Ok(bytes)
}

fn sample_row_history_once(
    handle: HANDLE,
    slot_address: u64,
    slot_allocation_base: u64,
    row_allocation_base: u64,
    history: &mut MetricsRowHistory,
    observed_at_unix_ms: u64,
) -> bool {
    let previous_header = history.last_header;
    let read = read_row_history_state(
        handle,
        slot_address,
        history.row_address,
        slot_allocation_base,
        row_allocation_base,
    );
    let current_header = read.header;
    let mut sample = MetricsRowHistorySample {
        observed_at_unix_ms,
        status: read.status,
        data_address: current_header.map(|header| header.data_address),
        count: current_header.map(|header| header.count),
        capacity: current_header.map(|header| header.capacity),
        capacity_bytes: read.capacity_bytes,
        data_allocation_base: read.data_allocation_base,
        data_pointer_changed: previous_header
            .zip(current_header)
            .map(|(before, after)| before.data_address != after.data_address),
        count_changed: previous_header
            .zip(current_header)
            .map(|(before, after)| before.count != after.count),
        capacity_changed: previous_header
            .zip(current_header)
            .map(|(before, after)| before.capacity != after.capacity),
        changed_elements: Vec::new(),
        removed_element_indices: Vec::new(),
        read_error: read.error.clone(),
    };
    let mut complete = matches!(read.status, "READ_OK" | "EMPTY") && read.error.is_none();
    if complete {
        let header = current_header.expect("successful history reads have a decoded header");
        match compare_row_history_elements(
            &history.last_elements,
            &read.elements,
            header.data_address,
        ) {
            Ok((changed, removed)) => {
                sample.changed_elements = changed;
                sample.removed_element_indices = removed;
                history.last_elements = read.elements;
            }
            Err(error) => {
                complete = false;
                sample.status = "ELEMENT_DIFF_INVALID";
                sample.read_error = Some(ReportError {
                    field: "row_history",
                    code: "HISTORY_ELEMENT_DIFF_INVALID",
                    message: format!("could not compare row history samples: {error:?}"),
                });
            }
        }
    }
    if let Some(header) = current_header {
        history.last_header = Some(header);
    }
    history.samples.push(sample);
    complete
}

fn inspect_target_object(
    handle: HANDLE,
    address: u64,
    region: RemoteRegion,
    identity: &ProcessIdentity,
    owner_address: u64,
    row_slot_address: u64,
    errors: &mut Vec<ReportError>,
) -> MetricsTargetObject {
    let prefix = read_region_prefix(handle, address, region, TARGET_OBJECT_SIZE);
    let status = read_status(&prefix);
    if !prefix.complete {
        errors.push(ReportError {
            field: "target_object",
            code: "TARGET_OBJECT_READ_INCOMPLETE",
            message: format!(
                "read {} of {} bounded bytes from row target 0x{address:X}",
                prefix.bytes.len(),
                prefix.requested
            ),
        });
    }
    let (address_shaped_values, unreadable_or_unmapped_address_shaped_values, nested_pointers) =
        inspect_nested_pointers(
            handle,
            &prefix.bytes,
            address,
            identity,
            owner_address,
            row_slot_address,
            errors,
        );
    let ascii_runs = find_ascii_runs(&prefix.bytes);
    let sample = memory_sample(now_unix_ms(), status, &prefix.bytes, &[]);
    MetricsTargetObject {
        status,
        observed_bytes: prefix.bytes.len().min(u16::MAX as usize) as u16,
        region_limited: prefix.requested < TARGET_OBJECT_SIZE,
        header_hex: (!prefix.bytes.is_empty())
            .then(|| hex(&prefix.bytes[..32.min(prefix.bytes.len())])),
        raw_prefix_hex: (!prefix.bytes.is_empty()).then(|| hex(&prefix.bytes)),
        ascii_runs,
        nul_terminated_ascii: nul_terminated_ascii(&prefix.bytes),
        address_shaped_values,
        unreadable_or_unmapped_address_shaped_values,
        nested_pointers,
        interpretation: "OBSERVED_BYTES; OBJECT_SEMANTICS_UNCONFIRMED",
        samples: vec![sample],
        baseline_bytes: prefix.bytes,
    }
}

fn inspect_nested_pointers(
    handle: HANDLE,
    bytes: &[u8],
    parent_address: u64,
    identity: &ProcessIdentity,
    owner_address: u64,
    row_slot_address: u64,
    errors: &mut Vec<ReportError>,
) -> (u16, u16, Vec<MetricsNestedPointer>) {
    const MIN_USER_ADDRESS: u64 = 0x1_0000;
    const MAX_USER_ADDRESS: u64 = 0x0000_8000_0000_0000;

    let mut address_shaped_values = 0u16;
    let mut unmapped_values = 0u16;
    let mut targets = HashSet::new();
    let mut nested = Vec::new();
    let module_end = identity
        .module_base
        .checked_add(u64::from(identity.image_size));
    for offset in (0..bytes.len().saturating_sub(7)).step_by(size_of::<u64>()) {
        let Ok(offset_u16) = u16::try_from(offset) else {
            continue;
        };
        let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if !is_aligned_pointer_candidate(value, MIN_USER_ADDRESS, MAX_USER_ADDRESS) {
            continue;
        }
        address_shaped_values = address_shaped_values.saturating_add(1);
        if !targets.insert(value) {
            continue;
        }
        let Some(region) = query_region(handle, value) else {
            unmapped_values = unmapped_values.saturating_add(1);
            continue;
        };
        if region.state != MEM_COMMIT || !readable_protection(region.protection) {
            unmapped_values = unmapped_values.saturating_add(1);
            continue;
        }
        let prefix = read_region_prefix(handle, value, region, NESTED_TARGET_PREVIEW_SIZE);
        let status = read_status(&prefix);
        if !prefix.complete {
            errors.push(ReportError {
                field: "nested_pointer",
                code: "NESTED_TARGET_READ_INCOMPLETE",
                message: format!(
                    "read {} of {} bounded bytes from nested target 0x{value:X}",
                    prefix.bytes.len(),
                    prefix.requested
                ),
            });
        }
        let Some(source_address) = parent_address.checked_add(u64::from(offset_u16)) else {
            unmapped_values = unmapped_values.saturating_add(1);
            continue;
        };
        let sample = memory_sample(now_unix_ms(), status, &prefix.bytes, &[]);
        let static_access_module_offset = static_nested_pointer_access(offset_u16);
        nested.push(MetricsNestedPointer {
            source_offset: offset_u16,
            address: value,
            module_offset: module_offset(value, identity.module_base, module_end),
            static_access_module_offset,
            region: public_region(region, identity),
            pointer_chain: vec![
                owner_address,
                row_slot_address,
                parent_address,
                source_address,
                value,
            ],
            status,
            interpretation: if static_access_module_offset.is_some() {
                "STATIC_DYNAMIC_ARRAY_DATA_POINTER; HELPER_USE_CONFIRMED; RUNTIME_UPDATE_UNOBSERVED"
            } else {
                "OBSERVED_POINTER_VALUE; TARGET_RELATIONSHIP_UNCONFIRMED"
            },
            observed_bytes: prefix.bytes.len().min(u16::MAX as usize) as u16,
            region_limited: prefix.requested < NESTED_TARGET_PREVIEW_SIZE,
            bytes_hex: (!prefix.bytes.is_empty()).then(|| hex(&prefix.bytes)),
            ascii_runs: find_ascii_runs(&prefix.bytes),
            nul_terminated_ascii: nul_terminated_ascii(&prefix.bytes),
            samples: vec![sample],
            baseline_bytes: prefix.bytes,
        });
    }
    (address_shaped_values, unmapped_values, nested)
}

fn sample_candidates(
    handle: HANDLE,
    candidates: &mut [MetricsObjectCandidate],
    mut process_identity_is_current: impl FnMut() -> bool,
) -> (usize, usize, bool, bool) {
    let mut inspected_objects = 0usize;
    let mut inspected_row_histories = 0usize;
    let mut complete = true;
    for candidate in candidates.iter() {
        for row in &candidate.rows {
            if let Some(object) = &row.target_object {
                inspected_objects += usize::from(!object.baseline_bytes.is_empty());
                inspected_objects += object
                    .nested_pointers
                    .iter()
                    .filter(|nested| !nested.baseline_bytes.is_empty())
                    .count();
            }
            if let Some(history) = &row.row_history {
                inspected_row_histories += 1;
                complete &= matches!(history.status, "READ_OK" | "EMPTY");
            }
        }
    }
    if inspected_objects == 0 && inspected_row_histories == 0 {
        return (0, 0, true, true);
    }

    for _ in 1..SAMPLE_COUNT {
        thread::sleep(Duration::from_millis(SAMPLE_INTERVAL_MS));
        if !process_identity_is_current() {
            return (inspected_objects, inspected_row_histories, false, false);
        }
        let observed_at = now_unix_ms();
        for candidate in candidates.iter_mut() {
            for row in &mut candidate.rows {
                if let Some(history) = &mut row.row_history {
                    let Some(row_region) = &row.target_region else {
                        complete = false;
                        continue;
                    };
                    complete &= sample_row_history_once(
                        handle,
                        row.slot_address,
                        row.slot_region.allocation_base,
                        row_region.allocation_base,
                        history,
                        observed_at,
                    );
                }
                let Some(target_address) = row.target_address else {
                    continue;
                };
                let Some(object) = &mut row.target_object else {
                    continue;
                };
                let Some(initial_region) = &row.target_region else {
                    continue;
                };
                if !object.baseline_bytes.is_empty() {
                    let sample = sample_remote_object(
                        handle,
                        &[(row.slot_address, target_address)],
                        target_address,
                        initial_region.allocation_base,
                        &object.baseline_bytes,
                        observed_at,
                    );
                    complete &= sample.status == "READ_OK";
                    object.samples.push(sample);
                }
                for nested in &mut object.nested_pointers {
                    if nested.baseline_bytes.is_empty() {
                        continue;
                    }
                    let Some(source_address) =
                        target_address.checked_add(u64::from(nested.source_offset))
                    else {
                        complete = false;
                        continue;
                    };
                    let sample = sample_remote_object(
                        handle,
                        &[
                            (row.slot_address, target_address),
                            (source_address, nested.address),
                        ],
                        nested.address,
                        nested.region.allocation_base,
                        &nested.baseline_bytes,
                        observed_at,
                    );
                    complete &= sample.status == "READ_OK";
                    nested.samples.push(sample);
                }
            }
        }
    }
    (inspected_objects, inspected_row_histories, complete, true)
}

fn sample_remote_object(
    handle: HANDLE,
    pointer_chain: &[(u64, u64)],
    address: u64,
    allocation_base: u64,
    baseline: &[u8],
    observed_at: u64,
) -> MetricsObjectSample {
    for &(slot_address, expected) in pointer_chain {
        let mut bytes = [0u8; size_of::<u64>()];
        if !read_exact(handle, slot_address, &mut bytes) {
            return memory_sample(observed_at, "POINTER_READ_FAILED", &[], baseline);
        }
        if u64::from_le_bytes(bytes) != expected {
            return memory_sample(observed_at, "POINTER_CHANGED", &[], baseline);
        }
    }
    let Some(region) = query_region(handle, address) else {
        return memory_sample(observed_at, "TARGET_UNMAPPED", &[], baseline);
    };
    if region.state != MEM_COMMIT
        || !readable_protection(region.protection)
        || region.allocation_base != allocation_base
    {
        return memory_sample(observed_at, "TARGET_REGION_CHANGED", &[], baseline);
    }
    let prefix = read_region_prefix(handle, address, region, baseline.len());
    let status = if prefix.requested != baseline.len() {
        "REGION_BOUNDS_CHANGED"
    } else {
        read_status(&prefix)
    };
    memory_sample(observed_at, status, &prefix.bytes, baseline)
}

fn memory_sample(
    observed_at: u64,
    status: &'static str,
    bytes: &[u8],
    baseline: &[u8],
) -> MetricsObjectSample {
    MetricsObjectSample {
        observed_at_unix_ms: observed_at,
        status,
        bytes_hex: (!bytes.is_empty()).then(|| hex(bytes)),
        changed_u32_fields: if status == "READ_OK" {
            changed_u32_fields(baseline, bytes)
        } else {
            Vec::<MetricsChangedU32Field>::new()
        },
    }
}

struct BoundedRead {
    requested: usize,
    bytes: Vec<u8>,
    complete: bool,
}

fn read_region_prefix(
    handle: HANDLE,
    address: u64,
    region: RemoteRegion,
    limit: usize,
) -> BoundedRead {
    let requested = bounded_region_length(address, region, limit);
    let Ok(remote_address) = usize::try_from(address) else {
        return BoundedRead {
            requested,
            bytes: Vec::new(),
            complete: false,
        };
    };
    let mut bytes = vec![0u8; requested];
    let mut received = 0usize;
    // SAFETY: the caller bounds the read to a committed readable VirtualQueryEx region.
    let succeeded = unsafe {
        ReadProcessMemory(
            handle,
            remote_address as *const c_void,
            bytes.as_mut_ptr().cast(),
            requested,
            &mut received,
        )
    };
    let actual = received.min(requested);
    bytes.truncate(actual);
    BoundedRead {
        requested,
        complete: requested > 0 && succeeded != 0 && received == requested,
        bytes,
    }
}

fn bounded_region_length(address: u64, region: RemoteRegion, limit: usize) -> usize {
    let Some(end) = region.region_base.checked_add(region.region_size) else {
        return 0;
    };
    if address < region.region_base || address >= end {
        return 0;
    }
    usize::try_from(end - address).map_or(0, |available| limit.min(available))
}

fn read_status(read: &BoundedRead) -> &'static str {
    if read.complete {
        "READ_OK"
    } else if read.bytes.is_empty() {
        "READ_FAILED"
    } else {
        "PARTIAL_READ"
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
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
mod tests {
    use super::*;

    #[cfg(target_pointer_width = "64")]
    const SYNTHETIC_OWNER_WORDS: usize = 0x61E350 / size_of::<u64>();
    #[cfg(target_pointer_width = "64")]
    static SYNTHETIC_VTABLE: [u64; 1] = [0];
    #[cfg(target_pointer_width = "64")]
    static ROW_TEXTS: [&[u8]; 8] = [
        b"FPS\0",
        b"Total\0",
        b"Render\0",
        b"Game\0",
        b"GPU\0",
        b"CPU\0",
        b"Session\0",
        b"RS\0",
    ];
    #[cfg(target_pointer_width = "64")]
    static NESTED_POINTER_TEXT: &[u8] = b"Nested metric descriptor\0";
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_OWNER: [u64; SYNTHETIC_OWNER_WORDS] = [0; SYNTHETIC_OWNER_WORDS];
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_OWNER_TWO: [u64; SYNTHETIC_OWNER_WORDS] = [0; SYNTHETIC_OWNER_WORDS];
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_TARGET_OBJECT: [u64; TARGET_OBJECT_SIZE / size_of::<u64>()] =
        [0; TARGET_OBJECT_SIZE / size_of::<u64>()];
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_HISTORY_ROW: [u64; 0xA0 / size_of::<u64>()] = [0; 0xA0 / size_of::<u64>()];
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_HISTORY_SLOT: u64 = 0;
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_HISTORY_DATA: [u64; 3 * 0x48 / size_of::<u64>()] =
        [0; 3 * 0x48 / size_of::<u64>()];

    #[test]
    fn module_offset_returns_none_for_addresses_outside_the_image() {
        assert_eq!(module_offset(0x1000, 0x2000, Some(0x3000)), None);
        assert_eq!(module_offset(0x2000, 0x2000, Some(0x3000)), Some(0));
        assert_eq!(module_offset(0x3000, 0x2000, Some(0x3000)), None);
    }

    #[test]
    fn bounded_object_reads_stop_at_the_readable_region_end() {
        let region = RemoteRegion {
            allocation_base: 0x1000,
            region_base: 0x1000,
            region_size: 0x100,
            state: MEM_COMMIT,
            protection: windows_sys::Win32::System::Memory::PAGE_READONLY,
            region_type: windows_sys::Win32::System::Memory::MEM_PRIVATE,
        };

        assert_eq!(bounded_region_length(0x1080, region, 256), 0x80);
        assert_eq!(bounded_region_length(0x1100, region, 256), 0);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn target_object_inspection_reports_readable_nested_strings() {
        let process = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() };
        let object = std::ptr::addr_of_mut!(SYNTHETIC_TARGET_OBJECT).cast::<u64>();
        // SAFETY: this test-owned object has 256 bytes and is aligned for a pointer-sized field.
        unsafe { object.write(NESTED_POINTER_TEXT.as_ptr() as usize as u64) };
        let address = object as usize as u64;
        let region = query_region(process, address).expect("synthetic object is mapped");
        let identity = ProcessIdentity {
            creation_time_filetime: 1,
            executable_path: std::path::PathBuf::new(),
            module_base: 0,
            image_size: 0,
        };
        let mut errors = Vec::new();

        let inspected = inspect_target_object(
            process,
            address,
            region,
            &identity,
            address,
            address,
            &mut errors,
        );

        assert_eq!(inspected.observed_bytes, TARGET_OBJECT_SIZE as u16);
        assert!(inspected.nested_pointers.iter().any(|pointer| {
            pointer.address == NESTED_POINTER_TEXT.as_ptr() as usize as u64
                && pointer.nul_terminated_ascii.as_deref() == Some("Nested metric descriptor")
                && pointer
                    .ascii_runs
                    .iter()
                    .any(|run| run.text == "Nested metric descriptor")
        }));
        assert!(errors.is_empty());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn row_history_reader_bounds_remote_array_and_reports_changes_separately() {
        let process = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() };
        let row = std::ptr::addr_of_mut!(SYNTHETIC_HISTORY_ROW).cast::<u8>();
        let slot = std::ptr::addr_of_mut!(SYNTHETIC_HISTORY_SLOT);
        let data = std::ptr::addr_of_mut!(SYNTHETIC_HISTORY_DATA).cast::<u8>();
        let row_address = row as usize as u64;
        let slot_address = slot as usize as u64;
        let data_address = data as usize as u64;
        // SAFETY: these test-owned aligned buffers are sized for the synthetic row and three elements.
        unsafe {
            std::ptr::write_bytes(row, 0, 0xA0);
            std::ptr::write_bytes(data, 0, 3 * 0x48);
            row.add(0x88).cast::<u64>().write_unaligned(data_address);
            row.add(0x90).cast::<u32>().write_unaligned(2);
            row.add(0x94).cast::<u32>().write_unaligned(3);
            slot.write(row_address);
            data.add(4).cast::<u32>().write_unaligned(10);
            data.add(0x48 + 8).cast::<u32>().write_unaligned(20);
        }
        let row_region = query_region(process, row_address).expect("synthetic row is mapped");
        let slot_region = query_region(process, slot_address).expect("synthetic slot is mapped");
        let mut history = inspect_row_history(
            process,
            slot_address,
            row_address,
            slot_region.allocation_base,
            row_region.allocation_base,
        );

        assert_eq!(history.status, "READ_OK");
        assert_eq!(history.data_address, Some(data_address));
        assert_eq!(history.count, Some(2));
        assert_eq!(history.capacity, Some(3));
        assert_eq!(history.capacity_bytes, Some(0xD8));
        assert_eq!(history.element_stride_bytes, 0x48);
        assert!(
            history
                .allocation_validation
                .contains("HEAP_BLOCK_BOUNDARY_UNOBSERVED")
        );
        assert_eq!(history.inspected_element_count, 2);
        assert_eq!(history.elements.len(), 2);
        assert!(history.data_allocation_base.is_some());

        // SAFETY: the test mutates only its own two-element synthetic array and header.
        unsafe {
            row.add(0x90).cast::<u32>().write_unaligned(1);
            data.add(4).cast::<u32>().write_unaligned(11);
        }
        assert!(sample_row_history_once(
            process,
            slot_address,
            slot_region.allocation_base,
            row_region.allocation_base,
            &mut history,
            now_unix_ms(),
        ));
        let sample = history.samples.last().expect("second sample is recorded");
        assert_eq!(sample.status, "READ_OK");
        assert_eq!(sample.data_pointer_changed, Some(false));
        assert_eq!(sample.count_changed, Some(true));
        assert_eq!(sample.capacity_changed, Some(false));
        assert_eq!(sample.changed_elements.len(), 1);
        assert_eq!(sample.changed_elements[0].index, 0);
        assert_eq!(sample.changed_elements[0].changed_u32_fields[0].offset, 4);
        assert_eq!(
            sample.changed_elements[0].changed_u32_fields[0].before_u32_le,
            10
        );
        assert_eq!(
            sample.changed_elements[0].changed_u32_fields[0].after_u32_le,
            11
        );
        assert_eq!(sample.removed_element_indices, vec![1]);

        // SAFETY: shift the fixture to the second allocated element and shorten its capacity.
        unsafe {
            row.add(0x88)
                .cast::<u64>()
                .write_unaligned(data_address + 0x48);
            row.add(0x94).cast::<u32>().write_unaligned(2);
        }
        assert!(sample_row_history_once(
            process,
            slot_address,
            slot_region.allocation_base,
            row_region.allocation_base,
            &mut history,
            now_unix_ms(),
        ));
        let sample = history
            .samples
            .last()
            .expect("relocated array sample is recorded");
        assert_eq!(sample.data_pointer_changed, Some(true));
        assert_eq!(sample.count_changed, Some(false));
        assert_eq!(sample.capacity_changed, Some(true));
        assert_eq!(sample.changed_elements.len(), 1);
        assert_eq!(sample.changed_elements[0].address, data_address + 0x48);

        // SAFETY: replacing this fixture slot with another address tests refusal before dereference.
        unsafe { slot.write(data_address) };
        assert!(!sample_row_history_once(
            process,
            slot_address,
            slot_region.allocation_base,
            row_region.allocation_base,
            &mut history,
            now_unix_ms(),
        ));
        let sample = history
            .samples
            .last()
            .expect("stale pointer sample is recorded");
        assert_eq!(sample.status, "ROW_POINTER_CHANGED");
        assert_eq!(
            sample.read_error.as_ref().map(|error| error.code),
            Some("HISTORY_ROW_POINTER_CHANGED")
        );
        // SAFETY: restore the test-owned slot so concurrent tests see a valid fixture.
        unsafe { slot.write(row_address) };

        // SAFETY: this malformed header must be rejected before any history data is read.
        unsafe {
            row.add(0x90).cast::<u32>().write_unaligned(2);
            row.add(0x94).cast::<u32>().write_unaligned(1);
        }
        let invalid_count = inspect_row_history(
            process,
            slot_address,
            row_address,
            slot_region.allocation_base,
            row_region.allocation_base,
        );
        assert_eq!(invalid_count.status, "COUNT_EXCEEDS_CAPACITY");
        assert!(invalid_count.elements.is_empty());
        assert_eq!(
            invalid_count.samples[0]
                .read_error
                .as_ref()
                .map(|error| error.code),
            Some("HISTORY_COUNT_EXCEEDS_CAPACITY")
        );

        // SAFETY: allocate one committed page, then point a bounded capacity across its end.
        let allocation = unsafe {
            windows_sys::Win32::System::Memory::VirtualAlloc(
                std::ptr::null(),
                0x1000,
                windows_sys::Win32::System::Memory::MEM_COMMIT
                    | windows_sys::Win32::System::Memory::MEM_RESERVE,
                windows_sys::Win32::System::Memory::PAGE_READWRITE,
            )
        };
        assert!(!allocation.is_null(), "synthetic allocation succeeds");
        // SAFETY: this row is test-owned and the synthetic pointer is only an observed value.
        unsafe {
            row.add(0x88)
                .cast::<u64>()
                .write_unaligned(allocation as usize as u64 + 0x1000 - 0x48);
            row.add(0x90).cast::<u32>().write_unaligned(1);
            row.add(0x94).cast::<u32>().write_unaligned(2);
        }
        let boundary_crossing = inspect_row_history(
            process,
            slot_address,
            row_address,
            slot_region.allocation_base,
            row_region.allocation_base,
        );
        assert_eq!(boundary_crossing.status, "ARRAY_ALLOCATION_INVALID");
        assert!(boundary_crossing.elements.is_empty());
        assert_eq!(
            boundary_crossing.samples[0]
                .read_error
                .as_ref()
                .map(|error| error.code),
            Some("HISTORY_ARRAY_REGION_UNREADABLE")
        );
        // SAFETY: release the page allocated solely for this boundary test.
        assert_ne!(
            unsafe {
                windows_sys::Win32::System::Memory::VirtualFree(
                    allocation,
                    0,
                    windows_sys::Win32::System::Memory::MEM_RELEASE,
                )
            },
            0
        );
        // SAFETY: restore the static test fixture to its original bounded array.
        unsafe {
            row.add(0x88).cast::<u64>().write_unaligned(data_address);
            row.add(0x90).cast::<u32>().write_unaligned(2);
            row.add(0x94).cast::<u32>().write_unaligned(3);
        }
    }

    #[cfg(target_pointer_width = "64")]
    unsafe fn initialize_owner(owner: *mut u64, vtable_address: u64) {
        // SAFETY: caller provides one of the synthetic arrays sized for every configured slot.
        unsafe {
            owner.write(vtable_address);
            for (index, (_, offset)) in ROW_POINTERS.iter().enumerate() {
                owner
                    .add((*offset / size_of::<u64>() as u64) as usize)
                    .write(ROW_TEXTS[index].as_ptr() as usize as u64);
            }
        }
    }

    #[test]
    fn unknown_build_never_scans_process_memory() {
        let probe = ProbeReport::empty(ProbeStatus::UnknownBuild, Some("test".to_owned()));

        let report = discover_metrics_objects(probe);

        assert_eq!(report.status, MetricsObjectStatus::UnknownBuild);
        assert!(report.scan_stats.is_none());
        assert!(report.candidates.is_empty());
        assert!(report.owner_vtable_module_offset.is_none());
        assert!(report.row_pointer_offsets.is_empty());
        assert!(!report.memory_modified);
    }

    #[test]
    fn recognized_build_without_the_verified_layout_is_refused_before_scanning() {
        let mut probe = ProbeReport::empty(ProbeStatus::ExactMatch, None);
        probe.build_resolution = Some(crate::identity::BuildResolution::ExactMatch {
            build: crate::identity::BuildRecord {
                id: "steam-25755603".to_owned(),
                name: "future build".to_owned(),
                sha256: "00".repeat(32),
                file_version: None,
                product_version: None,
                steam_build_id: Some("25755603".to_owned()),
            },
        });

        let report = discover_metrics_objects(probe);

        assert_eq!(report.status, MetricsObjectStatus::UnsupportedLayout);
        assert_eq!(report.layout_status, "UNSUPPORTED");
        assert!(report.scan_stats.is_none());
        assert!(report.candidates.is_empty());
        assert!(report.owner_vtable_module_offset.is_none());
        assert!(report.row_pointer_offsets.is_empty());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn scans_and_validates_a_synthetic_metrics_owner_read_only() {
        let vtable_address = SYNTHETIC_VTABLE.as_ptr() as usize as u64;
        let owner = std::ptr::addr_of_mut!(SYNTHETIC_OWNER).cast::<u64>();
        let owner_two = std::ptr::addr_of_mut!(SYNTHETIC_OWNER_TWO).cast::<u64>();
        // SAFETY: these test-owned static structures have space for every configured row slot.
        unsafe {
            initialize_owner(owner, vtable_address);
            initialize_owner(owner_two, vtable_address);
        }
        let owner_address = owner as usize as u64;
        let owner_two_address = owner_two as usize as u64;
        let (matches, stats, errors) = scan_process_for_pointer(
            unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() },
            vtable_address,
        );
        assert!(
            stats.complete,
            "synthetic process scan should be complete: {errors:?}"
        );
        assert!(matches.contains(&owner_address));
        assert!(matches.contains(&owner_two_address));

        let identity = ProcessIdentity {
            creation_time_filetime: 1,
            executable_path: std::path::PathBuf::new(),
            module_base: vtable_address - OWNER_VTABLE_RVA,
            image_size: 0x0400_0000,
        };
        let process = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() };
        let mut candidates = Vec::new();
        for owner_address in [owner_address, owner_two_address] {
            let mut errors = Vec::new();
            let candidate = inspect_owner(
                process,
                owner_address,
                vtable_address,
                &identity,
                &mut errors,
            )
            .expect("synthetic owner validates");
            assert_eq!(candidate.valid_row_pointers, 8);
            assert_eq!(candidate.expected_label_matches, 8);
            assert_eq!(candidate.direct_label_matches, 8);
            assert_eq!(candidate.nested_label_matches, 0);
            assert_eq!(candidate.null_row_pointers, 0);
            assert!(candidate.rows.iter().all(|row| {
                row.expected_label_match
                    && row.target_classification == "NULL_TERMINATED_ASCII_STRING"
                    && row
                        .target_region
                        .as_ref()
                        .is_some_and(|region| region.protection_name.starts_with("PAGE_"))
            }));
            assert!(errors.is_empty());
            candidates.push(candidate);
        }
        let prior_sample_count = candidates[0].rows[0]
            .target_object
            .as_ref()
            .unwrap()
            .samples
            .len();
        let mut interrupted_candidates = candidates.clone();
        let mut identity_checks = 0;
        let (_, _, sampling_complete, identity_consistent) =
            sample_candidates(process, &mut interrupted_candidates, || {
                identity_checks += 1;
                identity_checks < 3
            });
        assert!(!sampling_complete);
        assert!(!identity_consistent);
        assert_eq!(identity_checks, 3);
        assert_eq!(
            interrupted_candidates[0].rows[0]
                .target_object
                .as_ref()
                .unwrap()
                .samples
                .len(),
            prior_sample_count + 2
        );
        let mut identity_checks = 0;
        let (sampled_objects, sampled_histories, sampling_complete, identity_consistent) =
            sample_candidates(process, &mut candidates, || {
                identity_checks += 1;
                true
            });
        assert!(sampling_complete);
        assert!(identity_consistent);
        assert_eq!(identity_checks, usize::from(SAMPLE_COUNT) - 1);
        assert_eq!(sampled_histories, 0);
        assert!(sampled_objects >= 16);
        assert!(
            candidates
                .iter()
                .flat_map(|candidate| &candidate.rows)
                .all(|row| {
                    row.target_object.as_ref().is_some_and(|object| {
                        object.samples.len() == usize::from(SAMPLE_COUNT)
                            && object
                                .samples
                                .iter()
                                .all(|sample| sample.status == "READ_OK")
                    })
                })
        );
        assert_eq!(
            classify_scan(candidates.len(), stats.complete),
            MetricsObjectStatus::AmbiguousCandidate
        );

        let invalid_slot =
            unsafe { owner.add((ROW_POINTERS[0].1 / size_of::<u64>() as u64) as usize) };
        // SAFETY: invalid_slot points into the synthetic owner created above.
        unsafe { invalid_slot.write(1) };
        let original_target = ROW_TEXTS[0].as_ptr() as usize as u64;
        let original_region = query_region(process, original_target).unwrap();
        let baseline = &candidates[0].rows[0]
            .target_object
            .as_ref()
            .unwrap()
            .baseline_bytes;
        let stale_pointer_sample = sample_remote_object(
            process,
            &[(invalid_slot as usize as u64, original_target)],
            original_target,
            original_region.allocation_base,
            baseline,
            now_unix_ms(),
        );
        assert_eq!(stale_pointer_sample.status, "POINTER_CHANGED");
        assert!(stale_pointer_sample.bytes_hex.is_none());
        let mut errors = Vec::new();
        let candidate = inspect_owner(
            process,
            owner_address,
            vtable_address,
            &identity,
            &mut errors,
        )
        .expect("invalid row target does not invalidate the owner observation");
        assert_eq!(candidate.valid_row_pointers, 7);
        assert!(!candidate.rows[0].expected_label_match);
        assert!(!errors.is_empty());
        assert!(
            stats.complete,
            "candidate validation must not change scan coverage"
        );
        assert_eq!(
            classify_scan(1, stats.complete),
            MetricsObjectStatus::CandidatesFound
        );
        assert_eq!(candidate_validation_status(&errors), "INCOMPLETE");
        // SAFETY: restore the fixture slot to its original valid row string pointer.
        unsafe { invalid_slot.write(ROW_TEXTS[0].as_ptr() as usize as u64) };
    }
}
