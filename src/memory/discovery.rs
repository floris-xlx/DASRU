use serde::Serialize;
#[cfg(any(windows, test))]
use std::collections::HashSet;

use crate::{probe::ProbeReport, process::ProcessIdentity};

// shortcut: these RVAs match only the registered Steam build; add per-build layouts before supporting others.
pub(crate) const SUPPORTED_LAYOUT_BUILD_ID: &str = "steam-25755602";
const SUPPORTED_LAYOUT_STEAM_BUILD_ID: &str = "25755602";
pub(crate) const SUPPORTED_LAYOUT_SHA256: &str =
    "b7aece5fe2842035efd9036a1049edd53ee5efa25564811e20c39ca18362c018";
pub(crate) const OWNER_VTABLE_RVA: u64 = 0x335_1940;
pub(crate) const OWNER_SETUP_RVA: u64 = 0x4A1FE0;
pub(crate) const RENDER_FUNCTION_RVAS: [u64; 3] = [0x49B280, 0x49C303, 0x49C52E];
#[cfg(any(windows, test))]
pub(crate) const SCAN_SCOPE: &str = "COMMITTED_READABLE_MEM_IMAGE_MEM_PRIVATE_MEM_MAPPED";
// These are [RDI + offset] fields after setup copies the RCX `this` pointer into RDI.
pub(crate) const ROW_POINTERS: [(&str, u64); 8] = [
    ("FPS", 0x61E310),
    ("Total", 0x61E318),
    ("Render", 0x61E320),
    ("Game", 0x61E328),
    ("GPU", 0x61E330),
    ("CPU", 0x61E338),
    ("Session", 0x61E340),
    ("RS", 0x61E348),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetricsObjectStatus {
    CandidatesFound,
    NotFound,
    PartialScan,
    AmbiguousCandidate,
    NoProcess,
    AmbiguousProcess,
    IdentityInconsistent,
    UnknownBuild,
    UnsupportedLayout,
    AccessDenied,
    UnsupportedPlatform,
    ProbeError,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsObjectRegion {
    pub allocation_base: u64,
    pub region_base: u64,
    pub region_size: u64,
    pub state: u32,
    pub protection: u32,
    pub protection_name: &'static str,
    pub region_type: &'static str,
    pub module_offset: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsRowPointer {
    pub label: &'static str,
    pub owner_offset: u64,
    pub slot_address: u64,
    pub slot_module_offset: Option<u64>,
    pub slot_region: MetricsObjectRegion,
    pub target_address: Option<u64>,
    pub target_module_offset: Option<u64>,
    pub target_region: Option<MetricsObjectRegion>,
    pub pointer_chain: Vec<u64>,
    pub target_classification: &'static str,
    pub expected_label_match: bool,
    pub target_preview_hex: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsObjectCandidate {
    pub owner_address: u64,
    pub owner_module_offset: Option<u64>,
    pub owner_region: MetricsObjectRegion,
    pub vtable_address: u64,
    pub vtable_module_offset: u64,
    pub valid_row_pointers: u8,
    pub null_row_pointers: u8,
    pub expected_label_matches: u8,
    pub static_setup_reference_confirmed: bool,
    pub owner_fields_share_allocation: bool,
    pub confidence: &'static str,
    pub renderer_consumption: &'static str,
    pub rows: Vec<MetricsRowPointer>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsObjectScanStats {
    pub scan_scope: &'static str,
    pub address_range_start: u64,
    pub address_range_end: u64,
    pub regions_visited: u64,
    pub readable_regions_scanned: u64,
    pub image_regions_scanned: u64,
    pub private_regions_scanned: u64,
    pub mapped_regions_scanned: u64,
    pub bytes_read: u64,
    pub failed_reads: u64,
    pub coverage: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsObjectReport {
    pub schema_version: u8,
    pub observed_at_unix_ms: u64,
    pub status: MetricsObjectStatus,
    pub inspection_mode: &'static str,
    pub memory_modified: bool,
    pub build_status: &'static str,
    pub layout_status: &'static str,
    pub metrics_display_state: &'static str,
    pub update_frequency: &'static str,
    pub candidate_validation_status: &'static str,
    pub probe: ProbeReport,
    pub process: Option<ProcessIdentity>,
    pub owner_setup_module_offset: Option<u64>,
    pub owner_base_register: Option<&'static str>,
    pub static_render_function_module_offsets: Vec<u64>,
    pub owner_vtable_module_offset: Option<u64>,
    pub row_pointer_offsets: Vec<(&'static str, u64)>,
    pub scan_stats: Option<MetricsObjectScanStats>,
    pub candidates: Vec<MetricsObjectCandidate>,
    pub errors: Vec<crate::probe::ReportError>,
}

impl MetricsObjectReport {
    pub(crate) fn new(
        status: MetricsObjectStatus,
        build_status: &'static str,
        probe: ProbeReport,
    ) -> Self {
        let layout_known = is_supported_metrics_layout(&probe);
        let layout_status = if probe.status != crate::probe::ProbeStatus::ExactMatch {
            "NOT_EVALUATED"
        } else if layout_known {
            "SUPPORTED"
        } else {
            "UNSUPPORTED"
        };
        let process = probe.processes.first().and_then(|candidate| {
            if let crate::process::ProcessInspection::Available { identity } = &candidate.inspection
            {
                Some(identity.clone())
            } else {
                None
            }
        });
        Self {
            schema_version: 1,
            observed_at_unix_ms: probe.observed_at_unix_ms,
            status,
            inspection_mode: "PROCESS",
            memory_modified: false,
            build_status,
            layout_status,
            metrics_display_state: "NOT_OBSERVED",
            update_frequency: "NOT_MEASURED",
            candidate_validation_status: "NOT_EVALUATED",
            probe,
            process,
            owner_setup_module_offset: layout_known.then_some(OWNER_SETUP_RVA),
            owner_base_register: layout_known
                .then_some("RCX (`this`), copied to RDI at setup entry"),
            static_render_function_module_offsets: if layout_known {
                RENDER_FUNCTION_RVAS.to_vec()
            } else {
                Vec::new()
            },
            owner_vtable_module_offset: layout_known.then_some(OWNER_VTABLE_RVA),
            row_pointer_offsets: if layout_known {
                ROW_POINTERS.to_vec()
            } else {
                Vec::new()
            },
            scan_stats: None,
            candidates: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self.status {
            MetricsObjectStatus::CandidatesFound | MetricsObjectStatus::NotFound => 0,
            _ => 1,
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match self.status {
            MetricsObjectStatus::CandidatesFound => "CANDIDATES_FOUND",
            MetricsObjectStatus::NotFound => "NOT_FOUND",
            MetricsObjectStatus::PartialScan => "PARTIAL_SCAN",
            MetricsObjectStatus::AmbiguousCandidate => "AMBIGUOUS_CANDIDATE",
            MetricsObjectStatus::NoProcess => "NO_PROCESS",
            MetricsObjectStatus::AmbiguousProcess => "AMBIGUOUS_PROCESS",
            MetricsObjectStatus::IdentityInconsistent => "IDENTITY_INCONSISTENT",
            MetricsObjectStatus::UnknownBuild => "UNKNOWN_BUILD",
            MetricsObjectStatus::UnsupportedLayout => "UNSUPPORTED_LAYOUT",
            MetricsObjectStatus::AccessDenied => "ACCESS_DENIED",
            MetricsObjectStatus::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
            MetricsObjectStatus::ProbeError => "PROBE_ERROR",
        }
    }

    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            "Metrics object discovery: {}\nBuild: {}\nLayout: {}\nMetrics display state: {}\nUpdate frequency: {}\nCandidate validation: {}\nMemory modified: NO\n",
            self.status_label(),
            self.build_status,
            self.layout_status,
            self.metrics_display_state,
            self.update_frequency,
            self.candidate_validation_status
        );
        if let Some(process) = &self.process {
            output.push_str(&format!(
                "Process: PID {} (module 0x{:X}, {} bytes)\n",
                self.probe.processes.first().map_or(0, |p| p.pid),
                process.module_base,
                process.image_size
            ));
        }
        if let Some(stats) = &self.scan_stats {
            output.push_str(&format!(
                "Scan scope: {}\nAddress range: 0x{:X}-0x{:X}\nRegions visited: {}\nReadable regions scanned: {}\nMEM_IMAGE regions: {}\nMEM_PRIVATE regions: {}\nMEM_MAPPED regions: {}\nBytes read: {}\nFailed reads: {}\nCoverage: {}\n",
                stats.scan_scope,
                stats.address_range_start,
                stats.address_range_end,
                stats.regions_visited,
                stats.readable_regions_scanned,
                stats.image_regions_scanned,
                stats.private_regions_scanned,
                stats.mapped_regions_scanned,
                stats.bytes_read,
                stats.failed_reads,
                stats.coverage
            ));
        }
        for candidate in &self.candidates {
            output.push_str(&format!(
                "Owner: 0x{:X} ({} valid row pointers; {})\n",
                candidate.owner_address, candidate.valid_row_pointers, candidate.confidence
            ));
            for row in &candidate.rows {
                output.push_str(&format!(
                    "  {}: {}\n",
                    row.label,
                    row.target_address
                        .map(|address| format!("0x{address:X} ({})", row.target_classification))
                        .unwrap_or_else(|| "NULL".to_owned())
                ));
            }
            output.push_str(&format!(
                "  Renderer consumption: {}\n",
                candidate.renderer_consumption
            ));
        }
        for error in &self.errors {
            output.push_str(&format!(
                "Error [{}] {}: {}\n",
                error.code, error.field, error.message
            ));
        }
        output
    }
}

pub(crate) fn is_supported_metrics_layout(probe: &ProbeReport) -> bool {
    probe.status == crate::probe::ProbeStatus::ExactMatch
        && matches!(
            probe.build_resolution.as_ref(),
            Some(crate::identity::BuildResolution::ExactMatch { build })
                if build.id == SUPPORTED_LAYOUT_BUILD_ID
                    && build.steam_build_id.as_deref() == Some(SUPPORTED_LAYOUT_STEAM_BUILD_ID)
                    && build.sha256.eq_ignore_ascii_case(SUPPORTED_LAYOUT_SHA256)
        )
}

#[cfg(any(windows, test))]
pub(crate) fn scan_coverage_label(complete: bool) -> &'static str {
    if complete {
        "FULL_COMMITTED_READABLE_IMAGE_PRIVATE_MAPPED"
    } else {
        "PARTIAL"
    }
}

#[cfg(any(windows, test))]
pub(crate) fn candidate_validation_status(errors: &[crate::probe::ReportError]) -> &'static str {
    if errors.is_empty() {
        "COMPLETE"
    } else {
        "INCOMPLETE"
    }
}

#[cfg(any(windows, test))]
pub(crate) fn classify_scan(candidate_count: usize, complete: bool) -> MetricsObjectStatus {
    match (candidate_count, complete) {
        (0, true) => MetricsObjectStatus::NotFound,
        (0, false) => MetricsObjectStatus::PartialScan,
        (1, true) => MetricsObjectStatus::CandidatesFound,
        (_, _) if candidate_count > 1 => MetricsObjectStatus::AmbiguousCandidate,
        _ => MetricsObjectStatus::PartialScan,
    }
}

#[cfg(any(windows, test))]
pub(crate) fn candidate_confidence(valid_pointers: u8, label_matches: u8) -> &'static str {
    if valid_pointers == 8 && label_matches >= 6 {
        "HIGH_STRUCTURAL"
    } else if valid_pointers >= 6 {
        "MEDIUM_STRUCTURAL"
    } else {
        "LOW_STRUCTURAL"
    }
}

#[cfg(any(windows, test))]
pub(crate) fn find_aligned_pointer_matches(bytes: &[u8], base: u64, value: u64) -> Vec<u64> {
    bytes
        .windows(size_of::<u64>())
        .enumerate()
        .filter_map(|(offset, window)| {
            let address = base.checked_add(offset as u64)?;
            (address % size_of::<u64>() as u64 == 0
                && u64::from_ne_bytes(window.try_into().ok()?) == value)
                .then_some(address)
        })
        .collect()
}

#[cfg(any(windows, test))]
pub(crate) fn ascii_c_string_length(bytes: &[u8]) -> Option<usize> {
    let end = bytes.iter().position(|byte| *byte == 0)?;
    (end > 0 && bytes[..end].iter().all(|byte| (0x20..=0x7E).contains(byte))).then_some(end)
}

#[cfg(any(windows, test))]
pub(crate) fn is_expected_label(bytes: &[u8], expected: &str) -> bool {
    ascii_c_string_length(bytes).is_some_and(|length| &bytes[..length] == expected.as_bytes())
}

#[cfg(any(windows, test))]
pub(crate) fn scan_pointer_chunk(
    bytes: &[u8],
    address: u64,
    read_result: (usize, usize, bool),
    value: u64,
    tail: &mut Vec<u8>,
    matches: &mut HashSet<u64>,
) -> bool {
    let (requested, returned, succeeded) = read_result;
    if !succeeded || returned != requested || returned > bytes.len() {
        tail.clear();
        return false;
    }
    let Some(combined_base) = address.checked_sub(tail.len() as u64) else {
        tail.clear();
        return false;
    };
    tail.extend_from_slice(&bytes[..returned]);
    matches.extend(find_aligned_pointer_matches(tail, combined_base, value));
    *tail = tail[tail.len().saturating_sub(size_of::<u64>() - 1)..].to_vec();
    true
}

#[cfg(any(windows, test))]
const fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn expected_label_match_requires_the_exact_bounded_string() {
        assert!(is_expected_label(b"FPS\0padding", "FPS"));
        assert!(!is_expected_label(b"FPS row\0", "FPS"));
        assert!(!is_expected_label(b"FPS", "FPS"));
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
}
