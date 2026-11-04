use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::{
    identity::BuildRegistry,
    probe::{ProbeReport, ProbeStatus, ReportError, RunIntegrityStatus, run_probe},
};

#[cfg(any(windows, test))]
mod scan;
#[cfg(windows)]
mod windows;

const BUILDS_TOML: &str = include_str!("../../data/builds.toml");
pub const FPS_SIGNATURE: &[u8] = b"%i FPS\0\0%.2fms\0\0Frame : %u";
const FPS_SIGNATURE_HEX: &str = "2569204650530000252E32666D7300004672616D65203A202575";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetricsScanStatus {
    FoundUniqueMatch,
    NoMatch,
    AmbiguousMatch,
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
    pub region_base: u64,
    pub region_size: u64,
    pub state: u32,
    pub protection: u32,
    pub protection_name: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryScanStats {
    pub module_base: u64,
    pub module_size: u64,
    pub readable_regions: u64,
    pub skipped_regions: u64,
    pub bytes_read: u64,
    pub bytes_unreadable: u64,
    pub failed_reads: u64,
    pub coverage: &'static str,
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
                "Module: 0x{:X} ({} bytes)\nReadable regions: {}\nSkipped regions: {}\nBytes read: {}\nUnreadable bytes: {}\n",
                stats.module_base,
                stats.module_size,
                stats.readable_regions,
                stats.skipped_regions,
                stats.bytes_read,
                stats.bytes_unreadable
            ));
        }
        for found in &self.matches {
            output.push_str(&format!(
                "FPS signature: 0x{:X} ({}; protection 0x{:X})\n",
                found.address, found.protection_name, found.protection
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

impl MetricsScanStatus {
    fn label(self) -> &'static str {
        match self {
            Self::FoundUniqueMatch => "FOUND_UNIQUE_MATCH",
            Self::NoMatch => "NO_MATCH",
            Self::AmbiguousMatch => "AMBIGUOUS_MATCH",
            Self::NoProcess => "NO_PROCESS",
            Self::AmbiguousProcess => "AMBIGUOUS_PROCESS",
            Self::IdentityInconsistent => "IDENTITY_INCONSISTENT",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
            Self::ProbeError => "PROBE_ERROR",
        }
    }
}

pub fn scan_metrics() -> MetricsScanReport {
    match BuildRegistry::from_toml(BUILDS_TOML) {
        Ok(registry) => scan_metrics_with_registry(&registry),
        Err(_) => MetricsScanReport::new(MetricsScanStatus::ProbeError, run_probe()),
    }
}

pub fn scan_metrics_with_registry(registry: &BuildRegistry) -> MetricsScanReport {
    let probe = crate::probe::run_probe_with_registry(registry);
    if !matches!(
        probe.status,
        ProbeStatus::ExactMatch | ProbeStatus::UnknownBuild
    ) {
        return MetricsScanReport::new(status_from_probe(probe.status), probe);
    }

    #[cfg(windows)]
    {
        windows::scan_metrics(probe)
    }
    #[cfg(not(windows))]
    {
        MetricsScanReport::new(MetricsScanStatus::UnsupportedPlatform, probe)
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
    fn fps_signature_matches_the_known_tdaunlocker_bytes() {
        let expected = [
            0x25, 0x69, 0x20, 0x46, 0x50, 0x53, 0x00, 0x00, 0x25, 0x2e, 0x32, 0x66, 0x6d, 0x73,
            0x00, 0x00, 0x46, 0x72, 0x61, 0x6d, 0x65, 0x20, 0x3a, 0x20, 0x25, 0x75,
        ];
        assert_eq!(FPS_SIGNATURE, expected);
    }
}
