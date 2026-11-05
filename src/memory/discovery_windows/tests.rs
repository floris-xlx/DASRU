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

    let invalid_slot = unsafe { owner.add((ROW_POINTERS[0].1 / size_of::<u64>() as u64) as usize) };
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
