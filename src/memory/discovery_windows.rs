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
        MetricsObjectCandidate, MetricsObjectRegion, MetricsObjectReport, MetricsObjectScanStats,
        MetricsObjectStatus, MetricsRowPointer, OWNER_VTABLE_RVA, ROW_POINTERS, SCAN_SCOPE,
        ascii_c_string_length, candidate_confidence, classify_scan, is_expected_label,
        scan_pointer_chunk,
    },
    windows::{protection_name, readable_protection, region_type_name},
};

const CHUNK_SIZE: usize = 1024 * 1024;
const MAX_CANDIDATES: usize = 32;
const TARGET_PREVIEW_SIZE: usize = 64;

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

    let (addresses, mut stats, mut errors) =
        scan_process_for_pointer(process.raw(), vtable_address);
    if addresses.len() > MAX_CANDIDATES {
        stats.complete = false;
        errors.push(ReportError {
            field: "memory.scan",
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
            &mut errors,
        ) {
            candidates.push(candidate);
        }
    }
    if !errors.is_empty() {
        stats.complete = false;
    }
    if !matches!(recheck_pinned_process(&pinned[0]), ProcessInspection::Available { identity } if identity == expected)
    {
        errors.push(ReportError {
            field: "process",
            code: "PROCESS_IDENTITY_CHANGED_DURING_SCAN",
            message: "the retained process identity changed while memory was scanned".to_owned(),
        });
        stats.complete = false;
    }
    let status = classify_scan(candidates.len(), stats.complete);
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
        coverage: if stats.complete {
            "FULL_COMMITTED_READABLE_IMAGE_PRIVATE_MAPPED"
        } else {
            "PARTIAL"
        },
    });
    report.candidates = candidates;
    report.errors = errors;
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
            });
            continue;
        }
        valid += 1;
        let preview_length = region
            .region_base
            .checked_add(region.region_size)
            .and_then(|end| end.checked_sub(target))
            .and_then(|available| usize::try_from(available).ok())
            .map_or(0, |available| TARGET_PREVIEW_SIZE.min(available));
        let mut preview = vec![0u8; preview_length];
        let preview_ok = !preview.is_empty() && read_exact(handle, target, &mut preview);
        if !preview_ok {
            errors.push(ReportError {
                field: "rows",
                code: "ROW_TARGET_PREVIEW_FAILED",
                message: format!("could not read the bounded preview for {label} at 0x{target:X}"),
            });
            preview.clear();
        }
        let string_length = ascii_c_string_length(&preview);
        let label_match = is_expected_label(&preview, label);
        expected_label_matches += u8::from(label_match);
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
            target_preview_hex: preview_ok.then(|| hex(&preview)),
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
        static_setup_reference_confirmed: true,
        owner_fields_share_allocation: true,
        confidence: candidate_confidence(valid, expected_label_matches),
        renderer_consumption: "STATIC_SETUP_REFERENCE_CONFIRMED; LIVE_RENDERER_CONSUMPTION_UNCONFIRMED",
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
    static mut SYNTHETIC_OWNER: [u64; SYNTHETIC_OWNER_WORDS] = [0; SYNTHETIC_OWNER_WORDS];
    #[cfg(target_pointer_width = "64")]
    static mut SYNTHETIC_OWNER_TWO: [u64; SYNTHETIC_OWNER_WORDS] = [0; SYNTHETIC_OWNER_WORDS];

    #[test]
    fn module_offset_returns_none_for_addresses_outside_the_image() {
        assert_eq!(module_offset(0x1000, 0x2000, Some(0x3000)), None);
        assert_eq!(module_offset(0x2000, 0x2000, Some(0x3000)), Some(0));
        assert_eq!(module_offset(0x3000, 0x2000, Some(0x3000)), None);
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
        assert_eq!(
            classify_scan(candidates.len(), stats.complete),
            MetricsObjectStatus::AmbiguousCandidate
        );

        let invalid_slot =
            unsafe { owner.add((ROW_POINTERS[0].1 / size_of::<u64>() as u64) as usize) };
        // SAFETY: invalid_slot points into the synthetic owner created above.
        unsafe { invalid_slot.write(1) };
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
        assert_eq!(
            classify_scan(1, errors.is_empty()),
            MetricsObjectStatus::PartialScan
        );
        // SAFETY: restore the fixture slot to its original valid row string pointer.
        unsafe { invalid_slot.write(ROW_TEXTS[0].as_ptr() as usize as u64) };
    }
}
