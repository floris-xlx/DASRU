use super::history::sample_row_history_once;
use super::*;
use std::{
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn inspect_target_object(
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

pub(super) fn sample_candidates(
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

pub(super) fn sample_remote_object(
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

pub(super) fn bounded_region_length(address: u64, region: RemoteRegion, limit: usize) -> usize {
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

pub(super) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
