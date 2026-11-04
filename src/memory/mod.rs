use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::{
    identity::BuildRegistry,
    probe::{ProbeReport, ProbeStatus, ReportError, RunIntegrityStatus, run_probe},
};

#[cfg(any(windows, test))]
mod layout;
#[cfg(any(windows, test))]
pub(crate) mod protection;
#[cfg(any(windows, test))]
mod scan;
#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))]
pub(crate) mod transaction;
#[cfg(any(windows, test))]
pub(crate) mod writer;

const BUILDS_TOML: &str = include_str!("../../data/builds.toml");
pub const FPS_SIGNATURE: &[u8] = b"%i FPS\0\0%.2fms\0\0Frame : %u";
const FPS_MARKER_REPLACEMENT: &str = "%i FPS*";
const FPS_SIGNATURE_HEX: &str = "2569204650530000252E32666D7300004672616D65203A202575";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetricsScanStatus {
    FoundUniqueMatch,
    NoMatch,
    AmbiguousMatch,
    PartialScan,
    NoProcess,
    AmbiguousProcess,
    IdentityInconsistent,
    AccessDenied,
    UnsupportedPlatform,
    ProbeError,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsMatch {
    pub address: u64,
    pub module_offset: u64,
    pub region_base: u64,
    pub region_size: u64,
    pub state: u32,
    pub protection: u32,
    pub protection_name: &'static str,
    pub region_type: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryScanStats {
    pub module_base: u64,
    pub module_size: u64,
    pub readable_regions: u64,
    pub skipped_regions: u64,
    pub bytes_read: u64,
    pub bytes_scanned: u64,
    pub bytes_untrusted: u64,
    pub bytes_unreadable: u64,
    pub failed_reads: u64,
    pub coverage: &'static str,
}

impl MemoryScanStats {
    #[cfg(any(windows, test))]
    pub(crate) fn record_read(&mut self, requested: u64, returned: u64, scanned: bool) {
        debug_assert!(returned <= requested);
        self.bytes_read += returned;
        self.bytes_unreadable += requested - returned;
        if scanned {
            self.bytes_scanned += requested;
        } else {
            self.bytes_untrusted += returned;
            self.failed_reads += 1;
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsScanReport {
    pub schema_version: u8,
    pub observed_at_unix_ms: u64,
    pub status: MetricsScanStatus,
    pub signature_name: &'static str,
    pub signature_hex: &'static str,
    pub run_integrity: RunIntegrityStatus,
    pub probe: ProbeReport,
    pub scan_stats: Option<MemoryScanStats>,
    pub matches: Vec<MetricsMatch>,
    pub errors: Vec<ReportError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetricsDryRunStatus {
    Safe,
    Unsupported,
    Ambiguous,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsStringBoundary {
    pub name: &'static str,
    pub start_address: u64,
    pub module_offset: u64,
    pub offset_from_signature: u64,
    pub terminator_address: u64,
    pub terminator_offset_from_signature: u64,
    pub capacity_bytes: u64,
    pub max_payload_bytes: u64,
    pub capacity_basis: &'static str,
    pub original_value: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsByteDiff {
    pub offset_from_signature: u64,
    pub before: u8,
    pub after: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsDryRun {
    pub status: MetricsDryRunStatus,
    pub target_string: &'static str,
    pub replacement: String,
    pub replacement_length_including_nul: u64,
    pub patch_bytes: u64,
    pub fits: Option<bool>,
    pub reason: Option<String>,
    pub diff: Vec<MetricsByteDiff>,
    pub write_plan: Vec<MetricsByteDiff>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricsLayoutReport {
    pub schema_version: u8,
    pub memory_modified: bool,
    pub scan: MetricsScanReport,
    pub dry_run: MetricsDryRun,
    pub region_type: Option<&'static str>,
    pub context_address: Option<u64>,
    pub context_module_offset: Option<u64>,
    pub signature_offset_in_context: Option<u64>,
    pub original_bytes_hex: Option<String>,
    pub strings: Vec<MetricsStringBoundary>,
    pub errors: Vec<ReportError>,
}

impl MetricsScanReport {
    fn new(status: MetricsScanStatus, probe: ProbeReport) -> Self {
        Self {
            schema_version: 1,
            observed_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            status,
            signature_name: "FPS_SIGNATURE",
            signature_hex: FPS_SIGNATURE_HEX,
            run_integrity: RunIntegrityStatus::NotEvaluated,
            probe,
            scan_stats: None,
            matches: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self.status {
            MetricsScanStatus::FoundUniqueMatch | MetricsScanStatus::NoMatch => 0,
            _ => 1,
        }
    }

    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            "Memory scan: {}\nRun integrity: NOT_EVALUATED\nSignature: {}\n",
            self.status.label(),
            self.signature_name
        );
        if let Some(candidate) = self.probe.processes.first() {
            output.push_str(&format!(
                "Process candidate: {} (PID {})\n",
                candidate.reported_name, candidate.pid
            ));
        }
        if let Some(stats) = &self.scan_stats {
            output.push_str(&format!(
                "Module: 0x{:X} ({} bytes)\nReadable regions: {}\nSkipped regions: {}\nBytes returned: {}\nBytes scanned: {}\nUntrusted returned bytes: {}\nUnreadable bytes: {}\n",
                stats.module_base,
                stats.module_size,
                stats.readable_regions,
                stats.skipped_regions,
                stats.bytes_read,
                stats.bytes_scanned,
                stats.bytes_untrusted,
                stats.bytes_unreadable
            ));
        }
        for found in &self.matches {
            output.push_str(&format!(
                "FPS signature: 0x{:X} (module +0x{:X}; {}; {}; protection 0x{:X})\n",
                found.address,
                found.module_offset,
                found.region_type,
                found.protection_name,
                found.protection
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

impl MetricsLayoutReport {
    fn new(scan: MetricsScanReport) -> Self {
        let ambiguous = matches!(
            scan.status,
            MetricsScanStatus::AmbiguousMatch | MetricsScanStatus::AmbiguousProcess
        ) || (scan.status == MetricsScanStatus::PartialScan
            && scan.matches.len() > 1);
        let (dry_run_status, reason) = if ambiguous {
            (
                MetricsDryRunStatus::Ambiguous,
                "more than one process or signature candidate was found".to_owned(),
            )
        } else {
            (
                MetricsDryRunStatus::Unsupported,
                "layout inspection was not completed".to_owned(),
            )
        };
        Self {
            schema_version: 1,
            memory_modified: false,
            scan,
            dry_run: MetricsDryRun {
                status: dry_run_status,
                target_string: "fps_label",
                replacement: FPS_MARKER_REPLACEMENT.to_owned(),
                replacement_length_including_nul: FPS_MARKER_REPLACEMENT.len() as u64 + 1,
                patch_bytes: 0,
                fits: None,
                reason: Some(reason),
                diff: Vec::new(),
                write_plan: Vec::new(),
            },
            region_type: None,
            context_address: None,
            context_module_offset: None,
            signature_offset_in_context: None,
            original_bytes_hex: None,
            strings: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn with_replacement(mut self, replacement: &str) -> Self {
        self.dry_run.set_replacement(replacement);
        self
    }

    pub fn exit_code(&self) -> i32 {
        self.scan.exit_code()
    }

    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            "Memory layout inspection: {}\n",
            self.dry_run.status.label()
        );
        output.push_str(&self.scan.to_human_text());
        if let Some(offset) = self.scan.matches.first().map(|found| found.module_offset) {
            output.push_str(&format!("Module-relative signature offset: 0x{offset:X}\n"));
        }
        if let Some(region_type) = self.region_type {
            output.push_str(&format!("Memory region type: {region_type}\n"));
        }
        if let (Some(address), Some(bytes)) = (self.context_address, &self.original_bytes_hex) {
            output.push_str(&format!("Original bytes at 0x{address:X}: {bytes}\n"));
        }
        for string in &self.strings {
            output.push_str(&format!(
                "String {} {:?}: start 0x{:X} (module +0x{:X}), terminator 0x{:X}, capacity {} bytes ({} payload bytes)\n",
                string.name,
                string.original_value,
                string.start_address,
                string.module_offset,
                string.terminator_address,
                string.capacity_bytes,
                string.max_payload_bytes
            ));
        }
        if let Some(reason) = &self.dry_run.reason {
            output.push_str(&format!("Dry-run reason: {reason}\n"));
        }
        output.push_str(&format!("Target: {}\n", self.dry_run.target_string));
        output.push_str(&format!("Replacement: {}\n", self.dry_run.replacement));
        output.push_str(&format!(
            "Replacement length: {} bytes including NUL\n",
            self.dry_run.replacement_length_including_nul
        ));
        if let Some(capacity) = self.strings.first().map(|string| string.capacity_bytes) {
            output.push_str(&format!("Capacity: {capacity} bytes\n"));
        }
        output.push_str(&format!(
            "Changed bytes: {}\nPatch span: {} bytes\nMemory modified: NO\n",
            self.dry_run.diff.len(),
            self.dry_run.patch_bytes
        ));
        for change in &self.dry_run.write_plan {
            output.push_str(&format!(
                "Patch byte +0x{:X}: {:02X} -> {:02X}\n",
                change.offset_from_signature, change.before, change.after
            ));
        }
        for change in &self.dry_run.diff {
            output.push_str(&format!(
                "Changed byte +0x{:X}: {:02X} -> {:02X}\n",
                change.offset_from_signature, change.before, change.after
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

impl MetricsDryRun {
    fn set_replacement(&mut self, replacement: &str) {
        self.replacement = replacement.to_owned();
        self.replacement_length_including_nul = replacement.len() as u64 + 1;
    }
}

impl MetricsScanStatus {
    fn label(self) -> &'static str {
        match self {
            Self::FoundUniqueMatch => "FOUND_UNIQUE_MATCH",
            Self::NoMatch => "NO_MATCH",
            Self::AmbiguousMatch => "AMBIGUOUS_MATCH",
            Self::PartialScan => "PARTIAL_SCAN",
            Self::NoProcess => "NO_PROCESS",
            Self::AmbiguousProcess => "AMBIGUOUS_PROCESS",
            Self::IdentityInconsistent => "IDENTITY_INCONSISTENT",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
            Self::ProbeError => "PROBE_ERROR",
        }
    }
}

impl MetricsDryRunStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Safe => "SAFE",
            Self::Unsupported => "UNSUPPORTED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

pub fn scan_metrics() -> MetricsScanReport {
    match BuildRegistry::from_toml(BUILDS_TOML) {
        Ok(registry) => scan_metrics_with_registry(&registry),
        Err(error) => registry_error_report(error, run_probe()),
    }
}

pub fn scan_metrics_with_registry(registry: &BuildRegistry) -> MetricsScanReport {
    metrics_with_registry(registry, false, FPS_MARKER_REPLACEMENT).scan
}

pub fn inspect_metrics_layout() -> MetricsLayoutReport {
    inspect_metrics_layout_with_replacement(FPS_MARKER_REPLACEMENT)
}

pub fn inspect_metrics_layout_with_replacement(replacement: &str) -> MetricsLayoutReport {
    match BuildRegistry::from_toml(BUILDS_TOML) {
        Ok(registry) => metrics_with_registry(&registry, true, replacement),
        Err(error) => MetricsLayoutReport::new(registry_error_report(error, run_probe()))
            .with_replacement(replacement),
    }
}

pub fn inspect_metrics_layout_with_registry(registry: &BuildRegistry) -> MetricsLayoutReport {
    metrics_with_registry(registry, true, FPS_MARKER_REPLACEMENT)
}

fn metrics_with_registry(
    registry: &BuildRegistry,
    inspect_layout: bool,
    replacement: &str,
) -> MetricsLayoutReport {
    let probe = crate::probe::run_probe_with_registry(registry);
    if !matches!(
        probe.status,
        ProbeStatus::ExactMatch | ProbeStatus::UnknownBuild
    ) {
        return MetricsLayoutReport::new(MetricsScanReport::new(
            status_from_probe(probe.status),
            probe,
        ))
        .with_replacement(replacement);
    }

    #[cfg(windows)]
    {
        if inspect_layout {
            windows::inspect_metrics_layout(probe, replacement)
        } else {
            MetricsLayoutReport::new(windows::scan_metrics(probe)).with_replacement(replacement)
        }
    }
    #[cfg(not(windows))]
    {
        let mut report = MetricsLayoutReport::new(MetricsScanReport::new(
            MetricsScanStatus::UnsupportedPlatform,
            probe,
        ))
        .with_replacement(replacement);
        if inspect_layout {
            report.dry_run.reason =
                Some("process memory inspection is supported on Windows only".to_owned());
        }
        report
    }
}

fn registry_error_report(
    error: crate::identity::BuildRegistryError,
    probe: ProbeReport,
) -> MetricsScanReport {
    let mut report = MetricsScanReport::new(MetricsScanStatus::ProbeError, probe);
    report.errors.push(ReportError {
        field: "build_registry",
        code: "BUILD_REGISTRY_INVALID",
        message: error.to_string(),
    });
    report
}

#[cfg(any(windows, test))]
fn classify_scan(match_count: usize, full_module: bool) -> MetricsScanStatus {
    if !full_module {
        return MetricsScanStatus::PartialScan;
    }
    match match_count {
        0 => MetricsScanStatus::NoMatch,
        1 => MetricsScanStatus::FoundUniqueMatch,
        _ => MetricsScanStatus::AmbiguousMatch,
    }
}

fn status_from_probe(status: ProbeStatus) -> MetricsScanStatus {
    match status {
        ProbeStatus::NoProcess => MetricsScanStatus::NoProcess,
        ProbeStatus::AmbiguousProcess => MetricsScanStatus::AmbiguousProcess,
        ProbeStatus::AccessDenied => MetricsScanStatus::AccessDenied,
        ProbeStatus::IdentityInconsistent => MetricsScanStatus::IdentityInconsistent,
        ProbeStatus::UnsupportedPlatform => MetricsScanStatus::UnsupportedPlatform,
        ProbeStatus::ExactMatch | ProbeStatus::UnknownBuild | ProbeStatus::ProbeError => {
            MetricsScanStatus::ProbeError
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_scans_are_never_classified_as_unique_or_empty() {
        assert_eq!(classify_scan(0, false), MetricsScanStatus::PartialScan);
        assert_eq!(classify_scan(1, false), MetricsScanStatus::PartialScan);
        assert_eq!(classify_scan(2, false), MetricsScanStatus::PartialScan);
    }

    #[test]
    fn complete_scans_classify_match_counts() {
        assert_eq!(classify_scan(0, true), MetricsScanStatus::NoMatch);
        assert_eq!(classify_scan(1, true), MetricsScanStatus::FoundUniqueMatch);
        assert_eq!(classify_scan(2, true), MetricsScanStatus::AmbiguousMatch);
    }

    #[test]
    fn invalid_registry_is_reported_as_a_structured_scanner_error() {
        let error = BuildRegistry::from_toml("not valid TOML = [").unwrap_err();
        let probe = ProbeReport::empty(ProbeStatus::ProbeError, None);
        let report = registry_error_report(error, probe);

        assert_eq!(report.status, MetricsScanStatus::ProbeError);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].field, "build_registry");
        assert_eq!(report.errors[0].code, "BUILD_REGISTRY_INVALID");
    }

    #[test]
    fn partial_inspection_with_multiple_observed_matches_is_ambiguous() {
        let probe = ProbeReport::empty(ProbeStatus::UnknownBuild, None);
        let mut scan = MetricsScanReport::new(MetricsScanStatus::PartialScan, probe);
        scan.matches = (0..2)
            .map(|address| MetricsMatch {
                address,
                module_offset: address,
                region_base: 0,
                region_size: 1,
                state: 0,
                protection: 0,
                protection_name: "UNKNOWN",
                region_type: "UNKNOWN",
            })
            .collect();
        let report = MetricsLayoutReport::new(scan);

        assert_eq!(report.scan.status, MetricsScanStatus::PartialScan);
        assert_eq!(report.dry_run.status, MetricsDryRunStatus::Ambiguous);
        assert_eq!(report.scan.matches.len(), 2);
    }

    #[test]
    fn partial_read_accounting_counts_missing_and_untrusted_bytes_once() {
        let mut stats = MemoryScanStats {
            module_base: 0,
            module_size: 1024,
            readable_regions: 0,
            skipped_regions: 0,
            bytes_read: 0,
            bytes_scanned: 0,
            bytes_untrusted: 0,
            bytes_unreadable: 0,
            failed_reads: 0,
            coverage: "READABLE_REGIONS_ONLY",
        };

        stats.record_read(1024, 512, false);

        assert_eq!(stats.bytes_read, 512);
        assert_eq!(stats.bytes_scanned, 0);
        assert_eq!(stats.bytes_untrusted, 512);
        assert_eq!(stats.bytes_unreadable, 512);
        assert_eq!(stats.failed_reads, 1);
        assert_eq!(
            stats.bytes_scanned + stats.bytes_untrusted + stats.bytes_unreadable,
            1024
        );
    }

    #[test]
    fn successful_read_bytes_are_the_only_bytes_counted_as_scanned() {
        let mut stats = MemoryScanStats {
            module_base: 0,
            module_size: 1024,
            readable_regions: 0,
            skipped_regions: 0,
            bytes_read: 0,
            bytes_scanned: 0,
            bytes_untrusted: 0,
            bytes_unreadable: 0,
            failed_reads: 0,
            coverage: "READABLE_REGIONS_ONLY",
        };

        stats.record_read(1024, 1024, true);

        assert_eq!(stats.bytes_read, 1024);
        assert_eq!(stats.bytes_scanned, 1024);
        assert_eq!(stats.bytes_untrusted, 0);
        assert_eq!(stats.bytes_unreadable, 0);
        assert_eq!(stats.failed_reads, 0);
    }

    #[test]
    fn fps_signature_matches_the_known_tdaunlocker_bytes() {
        let expected = [
            0x25, 0x69, 0x20, 0x46, 0x50, 0x53, 0x00, 0x00, 0x25, 0x2e, 0x32, 0x66, 0x6d, 0x73,
            0x00, 0x00, 0x46, 0x72, 0x61, 0x6d, 0x65, 0x20, 0x3a, 0x20, 0x25, 0x75,
        ];
        assert_eq!(FPS_SIGNATURE, expected);
    }
}
