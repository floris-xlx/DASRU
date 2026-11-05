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
pub(crate) const ROW_UPDATE_HELPER_RVA: u64 = 0x4A61A0;
pub(crate) const ROW_DATA_POINTER_OFFSET: u16 = 0x88;
pub(crate) const ROW_DATA_COUNT_OFFSET: u16 = 0x90;
pub(crate) const ROW_DATA_CAPACITY_OFFSET: u16 = 0x94;
pub(crate) const ROW_HISTORY_ELEMENT_STRIDE: u16 = 0x48;
#[cfg(any(windows, test))]
pub(crate) const MAX_ROW_HISTORY_ELEMENTS: u32 = 256;
pub(crate) const RENDER_FUNCTION_RVAS: [u64; 3] = [0x49B280, 0x49C303, 0x49C52E];
#[cfg(windows)]
pub(crate) const TARGET_OBJECT_SIZE: usize = 256;
#[cfg(windows)]
pub(crate) const NESTED_TARGET_PREVIEW_SIZE: usize = 64;
#[cfg(windows)]
pub(crate) const SAMPLE_COUNT: u8 = 5;
#[cfg(windows)]
pub(crate) const SAMPLE_INTERVAL_MS: u64 = 250;
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
    pub target_object: Option<MetricsTargetObject>,
    pub row_history: Option<MetricsRowHistory>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(windows, test))]
pub(crate) struct RowHistoryHeader {
    pub data_address: u64,
    pub count: u32,
    pub capacity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(windows, test))]
pub(crate) enum RowHistoryHeaderError {
    IncompleteHeader,
    CountExceedsCapacity,
    CapacityExceedsLimit,
    NullDataAddress,
    MisalignedDataAddress,
    SpanOverflow,
    MalformedElement,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsRowHistoryElement {
    pub index: u32,
    pub address: u64,
    pub bytes_hex: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsRowHistoryElementChange {
    pub index: u32,
    pub address: u64,
    pub before_hex: Option<String>,
    pub after_hex: String,
    pub changed_u32_fields: Vec<MetricsChangedU32Field>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsRowHistorySample {
    pub observed_at_unix_ms: u64,
    pub status: &'static str,
    pub data_address: Option<u64>,
    pub count: Option<u32>,
    pub capacity: Option<u32>,
    pub capacity_bytes: Option<u64>,
    pub data_allocation_base: Option<u64>,
    pub data_pointer_changed: Option<bool>,
    pub count_changed: Option<bool>,
    pub capacity_changed: Option<bool>,
    pub changed_elements: Vec<MetricsRowHistoryElementChange>,
    pub removed_element_indices: Vec<u32>,
    pub read_error: Option<crate::probe::ReportError>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsRowHistory {
    pub row_address: u64,
    pub status: &'static str,
    pub data_address: Option<u64>,
    pub count: Option<u32>,
    pub capacity: Option<u32>,
    pub data_allocation_base: Option<u64>,
    pub capacity_bytes: Option<u64>,
    pub allocation_validation: &'static str,
    pub element_stride_bytes: u16,
    pub interpretation: &'static str,
    pub inspected_element_count: u32,
    pub omitted_element_count: u32,
    pub elements: Vec<MetricsRowHistoryElement>,
    pub samples: Vec<MetricsRowHistorySample>,
    #[cfg(windows)]
    #[serde(skip)]
    pub(crate) last_header: Option<RowHistoryHeader>,
    #[cfg(windows)]
    #[serde(skip)]
    pub(crate) last_elements: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsAsciiRun {
    pub offset: u16,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsChangedU32Field {
    pub offset: u16,
    pub interpretation: &'static str,
    pub before_hex: String,
    pub after_hex: String,
    pub before_u32_le: u32,
    pub after_u32_le: u32,
    pub before_f32_le: Option<f32>,
    pub after_f32_le: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsObjectSample {
    pub observed_at_unix_ms: u64,
    pub status: &'static str,
    pub bytes_hex: Option<String>,
    pub changed_u32_fields: Vec<MetricsChangedU32Field>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsNestedPointer {
    pub source_offset: u16,
    pub address: u64,
    pub module_offset: Option<u64>,
    pub static_access_module_offset: Option<u64>,
    pub region: MetricsObjectRegion,
    pub pointer_chain: Vec<u64>,
    pub status: &'static str,
    pub interpretation: &'static str,
    pub observed_bytes: u16,
    pub region_limited: bool,
    pub bytes_hex: Option<String>,
    pub ascii_runs: Vec<MetricsAsciiRun>,
    pub nul_terminated_ascii: Option<String>,
    pub samples: Vec<MetricsObjectSample>,
    #[serde(skip)]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) baseline_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsTargetObject {
    pub status: &'static str,
    pub observed_bytes: u16,
    pub region_limited: bool,
    pub header_hex: Option<String>,
    pub raw_prefix_hex: Option<String>,
    pub ascii_runs: Vec<MetricsAsciiRun>,
    pub nul_terminated_ascii: Option<String>,
    pub address_shaped_values: u16,
    pub unreadable_or_unmapped_address_shaped_values: u16,
    pub nested_pointers: Vec<MetricsNestedPointer>,
    pub interpretation: &'static str,
    pub samples: Vec<MetricsObjectSample>,
    #[serde(skip)]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) baseline_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsStaticSlotRead {
    pub owner_offset: u64,
    pub function_module_offset: u64,
    pub instruction_module_offset: u64,
    pub instruction_bytes: &'static str,
    pub base_register: &'static str,
    pub destination_register: &'static str,
    pub evidence: &'static str,
}

// Verified with x86-64 disassembly of the exact registered executable fingerprint.
pub(crate) const STATIC_RENDER_SLOT_READS: [MetricsStaticSlotRead; 8] = [
    MetricsStaticSlotRead {
        owner_offset: 0x61E310,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49C1EC,
        instruction_bytes: "49 8B 8E 10 E3 61 00",
        base_register: "R14",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E318,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49B8E9,
        instruction_bytes: "48 8B 8E 18 E3 61 00",
        base_register: "RSI",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E320,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49BBB1,
        instruction_bytes: "48 8B 8E 20 E3 61 00",
        base_register: "RSI",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E328,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49B938,
        instruction_bytes: "48 8B 8E 28 E3 61 00",
        base_register: "RSI",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E330,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49BC23,
        instruction_bytes: "48 8B 8E 30 E3 61 00",
        base_register: "RSI",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E338,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49BC89,
        instruction_bytes: "48 8B 8E 38 E3 61 00",
        base_register: "RSI",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E340,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49B9C6,
        instruction_bytes: "48 8B 8E 40 E3 61 00",
        base_register: "RSI",
        destination_register: "RCX",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
    MetricsStaticSlotRead {
        owner_offset: 0x61E348,
        function_module_offset: 0x49B280,
        instruction_module_offset: 0x49B9E7,
        instruction_bytes: "4C 8B AE 48 E3 61 00",
        base_register: "RSI",
        destination_register: "R13",
        evidence: "DECODED_X86_64_MOV_LOAD; RUNTIME_CALL_PATH_UNCONFIRMED",
    },
];

#[derive(Clone, Debug, Serialize)]
pub struct MetricsStaticSlotReadSummary {
    pub status: &'static str,
    pub reads: Vec<MetricsStaticSlotRead>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct MetricsStaticDataFlow {
    pub evidence_class: &'static str,
    pub row_update_helper_module_offset: u64,
    pub row_data_pointer_offset: u16,
    pub row_data_count_offset: u16,
    pub row_data_capacity_offset: u16,
    pub row_sample_stride_bytes: u16,
    pub helper_metric_value_register: &'static str,
    pub format_template_module_offset: u64,
    pub formatting_wrapper_module_offsets: [u64; 2],
    pub bounded_formatter_module_offset: u64,
    pub temporary_buffer_stack_offset: u16,
    pub temporary_buffer_capacity_bytes: u16,
    pub text_measure_module_offset: u64,
    pub text_glyph_path_module_offsets: [u64; 5],
    pub row_series_to_formatted_text_link: &'static str,
    pub runtime_invocation: &'static str,
    pub renderer_submission: &'static str,
    pub hypothesis: &'static str,
}

const STATIC_METRICS_DATA_FLOW: MetricsStaticDataFlow = MetricsStaticDataFlow {
    evidence_class: "STATIC_DISASSEMBLY",
    row_update_helper_module_offset: ROW_UPDATE_HELPER_RVA,
    row_data_pointer_offset: ROW_DATA_POINTER_OFFSET,
    row_data_count_offset: ROW_DATA_COUNT_OFFSET,
    row_data_capacity_offset: ROW_DATA_CAPACITY_OFFSET,
    row_sample_stride_bytes: ROW_HISTORY_ELEMENT_STRIDE,
    helper_metric_value_register: "XMM2",
    format_template_module_offset: 0x3351DF8,
    formatting_wrapper_module_offsets: [0x4A0180, 0x4A0290],
    bounded_formatter_module_offset: 0x2148F80,
    temporary_buffer_stack_offset: 0x40,
    temporary_buffer_capacity_bytes: 0x800,
    text_measure_module_offset: 0x90FDE0,
    text_glyph_path_module_offsets: [0x90FB00, 0x90E5A0, 0x90F770, 0x90F0C0, 0x90EB50],
    row_series_to_formatted_text_link: "NOT_ESTABLISHED",
    runtime_invocation: "NOT_OBSERVED",
    renderer_submission: "GLYPH_BATCH_PATH_FOUND; FINAL_GPU_SUBMISSION_UNIDENTIFIED",
    hypothesis: "ROW_SERIES_MAY_BE_METRIC_HISTORY_SEPARATE_FROM_TEMPORARY_FORMATTED_TEXT",
};

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
    pub direct_label_matches: u8,
    pub nested_label_matches: u8,
    pub confidence_basis: &'static str,
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
    pub sampling_status: &'static str,
    pub sampled_object_count: usize,
    pub requested_sample_count: u8,
    pub sample_interval_ms: u64,
    pub sampled_row_history_count: usize,
    pub requested_row_history_sample_count: u8,
    pub probe: ProbeReport,
    pub process: Option<ProcessIdentity>,
    pub owner_setup_module_offset: Option<u64>,
    pub owner_base_register: Option<&'static str>,
    pub static_render_function_module_offsets: Vec<u64>,
    pub owner_vtable_module_offset: Option<u64>,
    pub row_pointer_offsets: Vec<(&'static str, u64)>,
    pub static_render_slot_reads: MetricsStaticSlotReadSummary,
    pub static_data_flow: Option<MetricsStaticDataFlow>,
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
            sampling_status: "NOT_EVALUATED",
            sampled_object_count: 0,
            requested_sample_count: 0,
            sample_interval_ms: 0,
            sampled_row_history_count: 0,
            requested_row_history_sample_count: 0,
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
            static_render_slot_reads: MetricsStaticSlotReadSummary {
                status: if layout_known {
                    "VERIFIED_STATIC_READS; RUNTIME_CALL_PATH_UNCONFIRMED"
                } else {
                    "NOT_EVALUATED"
                },
                reads: if layout_known {
                    STATIC_RENDER_SLOT_READS.to_vec()
                } else {
                    Vec::new()
                },
            },
            static_data_flow: layout_known.then_some(STATIC_METRICS_DATA_FLOW),
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
            "Metrics object discovery: {}\nBuild: {}\nLayout: {}\nMetrics display state: {}\nUpdate frequency: {}\nCandidate validation: {}\nSampling: {} ({} objects, {} samples, {} ms interval; {} row histories x {} samples)\nStatic render slot reads: {}\nMemory modified: NO\n",
            self.status_label(),
            self.build_status,
            self.layout_status,
            self.metrics_display_state,
            self.update_frequency,
            self.candidate_validation_status,
            self.sampling_status,
            self.sampled_object_count,
            self.requested_sample_count,
            self.sample_interval_ms,
            self.sampled_row_history_count,
            self.requested_row_history_sample_count,
            self.static_render_slot_reads.reads.len()
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
                "Owner: 0x{:X} ({} valid row pointers; {} direct and {} nested label matches; {}; {})\n",
                candidate.owner_address,
                candidate.valid_row_pointers,
                candidate.direct_label_matches,
                candidate.nested_label_matches,
                candidate.confidence,
                candidate.confidence_basis
            ));
            for row in &candidate.rows {
                output.push_str(&format!(
                    "  {}: {}\n",
                    row.label,
                    row.target_address
                        .map(|address| format!("0x{address:X} ({})", row.target_classification))
                        .unwrap_or_else(|| "NULL".to_owned())
                ));
                if let Some(history) = &row.row_history {
                    let changed_elements = history
                        .samples
                        .iter()
                        .map(|sample| sample.changed_elements.len())
                        .sum::<usize>();
                    let read_errors = history
                        .samples
                        .iter()
                        .filter(|sample| sample.read_error.is_some())
                        .count();
                    output.push_str(&format!(
                        "    History: {} (count {:?}, capacity {:?}, {} elements read, {} samples, {} changed elements, {} read errors)\n",
                        history.status,
                        history.count,
                        history.capacity,
                        history.inspected_element_count,
                        history.samples.len(),
                        changed_elements,
                        read_errors
                    ));
                }
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
pub(crate) fn find_ascii_runs(bytes: &[u8]) -> Vec<MetricsAsciiRun> {
    let mut runs = Vec::new();
    let mut start = None;
    for (offset, byte) in bytes.iter().copied().chain(std::iter::once(0)).enumerate() {
        if (0x20..=0x7E).contains(&byte) {
            start.get_or_insert(offset);
        } else if let Some(run_start) = start.take() {
            if offset - run_start >= 4 {
                runs.push(MetricsAsciiRun {
                    offset: u16::try_from(run_start).unwrap_or(u16::MAX),
                    text: String::from_utf8_lossy(&bytes[run_start..offset]).into_owned(),
                });
            }
        }
    }
    runs
}

#[cfg(any(windows, test))]
pub(crate) fn changed_u32_fields(before: &[u8], after: &[u8]) -> Vec<MetricsChangedU32Field> {
    let common_length = before.len().min(after.len());
    let mut changes = Vec::new();
    for offset in (0..common_length.saturating_sub(3)).step_by(4) {
        let before_bytes: [u8; 4] = before[offset..offset + 4].try_into().unwrap();
        let after_bytes: [u8; 4] = after[offset..offset + 4].try_into().unwrap();
        if before_bytes != after_bytes {
            let before_f32 = f32::from_le_bytes(before_bytes);
            let after_f32 = f32::from_le_bytes(after_bytes);
            changes.push(MetricsChangedU32Field {
                offset: u16::try_from(offset).unwrap_or(u16::MAX),
                interpretation: "RAW_U32_AND_F32_VIEWS; FIELD_SEMANTICS_UNCONFIRMED",
                before_hex: hex_bytes(&before_bytes),
                after_hex: hex_bytes(&after_bytes),
                before_u32_le: u32::from_le_bytes(before_bytes),
                after_u32_le: u32::from_le_bytes(after_bytes),
                before_f32_le: before_f32.is_finite().then_some(before_f32),
                after_f32_le: after_f32.is_finite().then_some(after_f32),
            });
        }
    }
    changes
}

#[cfg(any(windows, test))]
fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

#[cfg(any(windows, test))]
pub(crate) fn is_aligned_pointer_candidate(value: u64, minimum: u64, maximum: u64) -> bool {
    value >= minimum && value < maximum && value % 8 == 0
}

#[cfg(any(windows, test))]
pub(crate) fn static_nested_pointer_access(source_offset: u16) -> Option<u64> {
    (source_offset == ROW_DATA_POINTER_OFFSET).then_some(ROW_UPDATE_HELPER_RVA)
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
pub(crate) fn nul_terminated_ascii(bytes: &[u8]) -> Option<String> {
    let length = ascii_c_string_length(bytes)?;
    std::str::from_utf8(&bytes[..length])
        .ok()
        .map(str::to_owned)
}

#[cfg(any(windows, test))]
pub(crate) fn is_expected_label(bytes: &[u8], expected: &str) -> bool {
    ascii_c_string_length(bytes).is_some_and(|length| &bytes[..length] == expected.as_bytes())
}

#[cfg(any(windows, test))]
pub(crate) fn is_expected_nested_label(observed: Option<&str>, expected: &str) -> bool {
    observed.is_some_and(|observed| {
        observed
            .strip_prefix(expected)
            .is_some_and(|suffix| suffix == ":")
    })
}

#[cfg(any(windows, test))]
pub(crate) fn parse_row_history_header(
    bytes: &[u8],
) -> Result<RowHistoryHeader, RowHistoryHeaderError> {
    let header = bytes
        .get(..16)
        .ok_or(RowHistoryHeaderError::IncompleteHeader)?;
    Ok(RowHistoryHeader {
        data_address: u64::from_le_bytes(header[..8].try_into().unwrap()),
        count: u32::from_le_bytes(header[8..12].try_into().unwrap()),
        capacity: u32::from_le_bytes(header[12..16].try_into().unwrap()),
    })
}

#[cfg(any(windows, test))]
pub(crate) fn row_history_capacity_bytes(
    header: RowHistoryHeader,
) -> Result<u64, RowHistoryHeaderError> {
    if header.count > header.capacity {
        return Err(RowHistoryHeaderError::CountExceedsCapacity);
    }
    if header.capacity > MAX_ROW_HISTORY_ELEMENTS {
        return Err(RowHistoryHeaderError::CapacityExceedsLimit);
    }
    if header.capacity == 0 {
        return Ok(0);
    }
    if header.data_address == 0 {
        return Err(RowHistoryHeaderError::NullDataAddress);
    }
    if header.data_address % 8 != 0 {
        return Err(RowHistoryHeaderError::MisalignedDataAddress);
    }
    let length = u64::from(header.capacity)
        .checked_mul(u64::from(ROW_HISTORY_ELEMENT_STRIDE))
        .ok_or(RowHistoryHeaderError::SpanOverflow)?;
    header
        .data_address
        .checked_add(length)
        .ok_or(RowHistoryHeaderError::SpanOverflow)?;
    Ok(length)
}

#[cfg(any(windows, test))]
pub(crate) fn compare_row_history_elements(
    previous: &[Vec<u8>],
    current: &[Vec<u8>],
    data_address: u64,
) -> Result<(Vec<MetricsRowHistoryElementChange>, Vec<u32>), RowHistoryHeaderError> {
    let element_size = usize::from(ROW_HISTORY_ELEMENT_STRIDE);
    if previous
        .iter()
        .chain(current)
        .any(|element| element.len() != element_size)
    {
        return Err(RowHistoryHeaderError::MalformedElement);
    }

    let mut changes = Vec::new();
    for (index, after) in current.iter().enumerate() {
        let before = previous.get(index);
        if before.is_some_and(|before| before == after) {
            continue;
        }
        let offset = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(u64::from(ROW_HISTORY_ELEMENT_STRIDE)))
            .ok_or(RowHistoryHeaderError::SpanOverflow)?;
        let address = data_address
            .checked_add(offset)
            .ok_or(RowHistoryHeaderError::SpanOverflow)?;
        changes.push(MetricsRowHistoryElementChange {
            index: u32::try_from(index).map_err(|_| RowHistoryHeaderError::SpanOverflow)?,
            address,
            before_hex: before.map(|bytes| hex_bytes(bytes)),
            after_hex: hex_bytes(after),
            changed_u32_fields: before
                .map_or_else(Vec::new, |before| changed_u32_fields(before, after)),
        });
    }
    let removed = (current.len()..previous.len())
        .map(|index| u32::try_from(index).map_err(|_| RowHistoryHeaderError::SpanOverflow))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((changes, removed))
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
            compare_row_history_elements(&[unchanged.clone(), before], &[unchanged], 0x2000)
                .unwrap();
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
}
