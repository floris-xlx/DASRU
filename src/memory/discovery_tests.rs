use super::*;

#[test]
fn row_history_header_decodes_pointer_count_and_capacity() {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&0x1234_5000_u64.to_le_bytes());
    bytes[8..12].copy_from_slice(&3_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&5_u32.to_le_bytes());

    assert_eq!(
        parse_row_history_header(&bytes),
        Ok(RowHistoryHeader {
            data_address: 0x1234_5000,
            count: 3,
            capacity: 5,
        })
    );
    assert_eq!(
        parse_row_history_header(&bytes[..15]),
        Err(RowHistoryHeaderError::IncompleteHeader)
    );
}

#[test]
fn row_history_validation_bounds_count_capacity_and_address_span() {
    assert_eq!(
        row_history_capacity_bytes(RowHistoryHeader {
            data_address: 0x1000,
            count: 2,
            capacity: 2,
        }),
        Ok(0x90)
    );
    assert_eq!(
        row_history_capacity_bytes(RowHistoryHeader {
            data_address: 0x1000,
            count: 2,
            capacity: 1,
        }),
        Err(RowHistoryHeaderError::CountExceedsCapacity)
    );
    assert_eq!(
        row_history_capacity_bytes(RowHistoryHeader {
            data_address: 0x1000,
            count: 0,
            capacity: 257,
        }),
        Err(RowHistoryHeaderError::CapacityExceedsLimit)
    );
    assert_eq!(
        row_history_capacity_bytes(RowHistoryHeader {
            data_address: 0,
            count: 0,
            capacity: 1,
        }),
        Err(RowHistoryHeaderError::NullDataAddress)
    );
    assert_eq!(
        row_history_capacity_bytes(RowHistoryHeader {
            data_address: 0x1001,
            count: 0,
            capacity: 1,
        }),
        Err(RowHistoryHeaderError::MisalignedDataAddress)
    );
    assert_eq!(
        row_history_capacity_bytes(RowHistoryHeader {
            data_address: u64::MAX - 7,
            count: 1,
            capacity: 1,
        }),
        Err(RowHistoryHeaderError::SpanOverflow)
    );
}

#[test]
fn row_history_diff_separates_changed_added_and_removed_elements() {
    let unchanged = vec![0u8; 0x48];
    let mut before = vec![0u8; 0x48];
    before[4..8].copy_from_slice(&10u32.to_le_bytes());
    let mut after = before.clone();
    after[4..8].copy_from_slice(&11u32.to_le_bytes());
    let added = vec![0xA5u8; 0x48];

    let (changes, removed) = compare_row_history_elements(
        &[unchanged.clone(), before.clone()],
        &[unchanged.clone(), after, added],
        0x2000,
    )
    .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].index, 1);
    assert_eq!(changes[0].address, 0x2048);
    assert!(changes[0].before_hex.is_some());
    assert_eq!(changes[0].changed_u32_fields.len(), 1);
    assert_eq!(changes[0].changed_u32_fields[0].offset, 4);
    assert_eq!(changes[0].changed_u32_fields[0].before_u32_le, 10);
    assert_eq!(changes[0].changed_u32_fields[0].after_u32_le, 11);
    assert_eq!(changes[1].index, 2);
    assert_eq!(changes[1].before_hex, None);
    assert!(removed.is_empty());

    let (_, removed) =
        compare_row_history_elements(&[unchanged.clone(), before], &[unchanged], 0x2000).unwrap();
    assert_eq!(removed, vec![1]);
}

#[test]
fn pointer_match_requires_natural_alignment_and_reports_absolute_address() {
    let value = 0x1122_3344_5566_7788u64;
    let mut bytes = vec![0; 24];
    bytes[1..9].copy_from_slice(&value.to_ne_bytes());
    bytes[16..24].copy_from_slice(&value.to_ne_bytes());

    assert_eq!(
        find_aligned_pointer_matches(&bytes, 0x1000, value),
        vec![0x1010]
    );
    assert_eq!(
        find_aligned_pointer_matches(&bytes[1..], 0x1001, value),
        vec![0x1010]
    );
}

#[test]
fn string_classifier_requires_printable_ascii_and_terminator() {
    assert_eq!(ascii_c_string_length(b"FPS row\0padding"), Some(7));
    assert_eq!(ascii_c_string_length(b"FPS\x01\0"), None);
    assert_eq!(ascii_c_string_length(b"unterminated"), None);
    assert_eq!(
        nul_terminated_ascii(b"FPS:\0padding"),
        Some("FPS:".to_owned())
    );
    assert_eq!(nul_terminated_ascii(b"FPS:\x01\0"), None);
    assert_eq!(nul_terminated_ascii(b"unterminated"), None);
}

#[test]
fn expected_label_match_requires_the_exact_bounded_string() {
    assert!(is_expected_label(b"FPS\0padding", "FPS"));
    assert!(!is_expected_label(b"FPS row\0", "FPS"));
    assert!(!is_expected_label(b"FPS", "FPS"));
}

#[test]
fn nested_label_matches_do_not_change_direct_label_confidence() {
    assert!(is_expected_nested_label(Some("FPS:"), "FPS"));
    assert!(!is_expected_nested_label(Some("FPS"), "FPS"));
    assert!(!is_expected_nested_label(Some("FPS extra:"), "FPS"));
    assert_eq!(candidate_confidence(8, 0), "MEDIUM_STRUCTURAL");
}

#[test]
fn partial_read_clears_overlap_and_never_completes_a_pointer_match() {
    let value = 0x1122_3344_5566_7788u64;
    let bytes = value.to_ne_bytes();
    let mut matches = HashSet::new();
    let mut tail = Vec::new();

    assert!(scan_pointer_chunk(
        &bytes[..4],
        0x1000,
        (4, 4, true),
        value,
        &mut tail,
        &mut matches,
    ));
    assert!(!scan_pointer_chunk(
        &bytes[4..],
        0x1004,
        (8, 4, false),
        value,
        &mut tail,
        &mut matches,
    ));

    assert!(matches.is_empty());
    assert!(tail.is_empty());
}

#[test]
fn no_match_is_distinct_from_an_incomplete_scan() {
    assert_eq!(classify_scan(0, true), MetricsObjectStatus::NotFound);
    assert_eq!(classify_scan(0, false), MetricsObjectStatus::PartialScan);
    assert_eq!(classify_scan(1, false), MetricsObjectStatus::PartialScan);
    assert_eq!(
        classify_scan(2, false),
        MetricsObjectStatus::AmbiguousCandidate
    );
    assert_eq!(scan_coverage_label(false), "PARTIAL");
    assert_eq!(candidate_validation_status(&[]), "COMPLETE");
    assert_eq!(
        candidate_validation_status(&[crate::probe::ReportError {
            field: "rows",
            code: "ROW_TARGET_UNMAPPED",
            message: "invalid pointer".to_owned(),
        }]),
        "INCOMPLETE"
    );
}

#[test]
fn build_specific_layout_requires_the_verified_id_and_executable_hash() {
    use crate::identity::{BuildRecord, BuildResolution};

    let mut probe = ProbeReport::empty(crate::probe::ProbeStatus::ExactMatch, None);
    probe.build_resolution = Some(BuildResolution::ExactMatch {
        build: BuildRecord {
            id: SUPPORTED_LAYOUT_BUILD_ID.to_owned(),
            name: "verified layout".to_owned(),
            sha256: SUPPORTED_LAYOUT_SHA256.to_owned(),
            file_version: None,
            product_version: None,
            steam_build_id: Some("25755602".to_owned()),
        },
    });
    assert!(is_supported_metrics_layout(&probe));

    probe.status = crate::probe::ProbeStatus::UnknownBuild;
    assert!(!is_supported_metrics_layout(&probe));
    probe.status = crate::probe::ProbeStatus::ExactMatch;

    if let Some(BuildResolution::ExactMatch { build }) = &mut probe.build_resolution {
        build.sha256 = "00".repeat(32);
    }
    assert!(!is_supported_metrics_layout(&probe));

    if let Some(BuildResolution::ExactMatch { build }) = &mut probe.build_resolution {
        build.id = "another-build".to_owned();
        build.sha256 = SUPPORTED_LAYOUT_SHA256.to_owned();
    }
    assert!(!is_supported_metrics_layout(&probe));

    if let Some(BuildResolution::ExactMatch { build }) = &mut probe.build_resolution {
        build.id = SUPPORTED_LAYOUT_BUILD_ID.to_owned();
        build.steam_build_id = Some("25755603".to_owned());
    }
    assert!(!is_supported_metrics_layout(&probe));
}

#[test]
fn reports_the_committed_readable_region_scope() {
    assert_eq!(
        SCAN_SCOPE,
        "COMMITTED_READABLE_MEM_IMAGE_MEM_PRIVATE_MEM_MAPPED"
    );
}

#[test]
fn high_confidence_requires_valid_slots_and_expected_labels() {
    assert_eq!(candidate_confidence(8, 0), "MEDIUM_STRUCTURAL");
    assert_eq!(candidate_confidence(7, 7), "MEDIUM_STRUCTURAL");
    assert_eq!(candidate_confidence(8, 6), "HIGH_STRUCTURAL");
    assert_eq!(candidate_confidence(4, 4), "LOW_STRUCTURAL");
}

#[test]
fn second_level_analysis_finds_bounded_ascii_runs() {
    let runs = find_ascii_runs(b"\x01FPS 144\0\x01CPU temp");

    assert_eq!(
        runs.iter()
            .map(|run| (run.offset, run.text.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "FPS 144"), (10, "CPU temp")]
    );
}

#[test]
fn sample_comparison_reports_changed_aligned_words_as_raw_views() {
    let before = [0u8, 0, 0, 0, 0, 0, 0, 0x3F];
    let after = [0u8, 0, 0, 0, 0, 0, 0x80, 0x3F];

    let changes = changed_u32_fields(&before, &after);

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].offset, 4);
    assert_eq!(changes[0].before_u32_le, 0x3F00_0000);
    assert_eq!(changes[0].after_u32_le, 0x3F80_0000);
    assert_eq!(changes[0].before_f32_le, Some(0.5));
    assert_eq!(changes[0].after_f32_le, Some(1.0));
}

#[test]
fn nested_pointer_candidates_must_be_aligned_and_inside_user_range() {
    assert!(is_aligned_pointer_candidate(
        0x10000,
        0x10000,
        0x8000_0000_0000
    ));
    assert!(!is_aligned_pointer_candidate(
        0x10001,
        0x10000,
        0x8000_0000_0000
    ));
    assert!(!is_aligned_pointer_candidate(
        0x8000_0000_0000,
        0x10000,
        0x8000_0000_0000
    ));
}

#[test]
fn row_data_pointer_has_exact_build_static_helper_evidence() {
    assert_eq!(
        static_nested_pointer_access(ROW_DATA_POINTER_OFFSET),
        Some(ROW_UPDATE_HELPER_RVA)
    );
    assert_eq!(static_nested_pointer_access(0x50), None);
}

#[test]
fn static_data_flow_keeps_runtime_and_renderer_claims_unconfirmed() {
    assert_eq!(STATIC_METRICS_DATA_FLOW.row_data_pointer_offset, 0x88);
    assert_eq!(
        STATIC_METRICS_DATA_FLOW.row_series_to_formatted_text_link,
        "NOT_ESTABLISHED"
    );
    assert_eq!(STATIC_METRICS_DATA_FLOW.runtime_invocation, "NOT_OBSERVED");
    assert!(
        STATIC_METRICS_DATA_FLOW
            .renderer_submission
            .contains("FINAL_GPU_SUBMISSION_UNIDENTIFIED")
    );
}

#[test]
fn verified_render_disassembly_reads_every_owner_slot() {
    for (_, slot_offset) in ROW_POINTERS {
        let read = STATIC_RENDER_SLOT_READS
            .iter()
            .find(|read| read.owner_offset == slot_offset)
            .expect("each row has a verified disassembly read");
        let bytes = read
            .instruction_bytes
            .split_ascii_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).expect("valid instruction byte"))
            .collect::<Vec<_>>();
        assert_eq!(bytes.len(), 7);
        assert_eq!(bytes[0] & 0x08, 0x08, "REX.W pointer load");
        assert_eq!(bytes[1], 0x8B, "MOV reg, r/m instruction");
        assert_eq!(bytes[2] & 0xC0, 0x80, "32-bit memory displacement");
        assert_eq!(
            u32::from_le_bytes(bytes[3..7].try_into().unwrap()) as u64,
            slot_offset
        );
        assert!(read.instruction_module_offset > read.function_module_offset);
    }
}
