use std::{fs::OpenOptions, io::Write, path::Path, time::SystemTime};

use serde::Serialize;
use thiserror::Error;

use crate::{
    doctor::{self, DoctorReport, FeatureReadiness, HostCapabilities, HostReport, RuntimeReport},
    identity::BuildResolution,
    probe::{
        ExecutableStatus, ProbeReport, ProbeStatus, ProcessStatus, RunIntegrityStatus, SteamStatus,
        run_probe,
    },
};

#[derive(Debug, Serialize)]
pub struct SupportReport {
    pub schema_version: u8,
    pub observed_at_unix_ms: u64,
    pub host: HostReport,
    pub runtime: RuntimeReport,
    pub host_capabilities: HostCapabilities,
    pub feature_readiness: Vec<FeatureReadiness>,
    pub executable_compatibility: ExecutableCompatibility,
    pub errors: Vec<SupportError>,
}

#[derive(Debug, Serialize)]
pub struct ExecutableCompatibility {
    pub status: ProbeStatus,
    pub process_status: ProcessStatus,
    pub executable_status: ExecutableStatus,
    pub steam_status: SteamStatus,
    pub run_integrity: RunIntegrityStatus,
    pub candidate_count: usize,
    pub recognized_build_id: Option<String>,
    pub executable_sha256: Option<String>,
    pub steam_build_id: Option<String>,
    pub expected_steam_build_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SupportError {
    pub component: &'static str,
    pub field: &'static str,
    pub code: &'static str,
}

#[derive(Debug, Error)]
pub enum SupportExportError {
    #[error("support report serialization failed")]
    Serialize(#[source] serde_json::Error),
    #[error("support report destination already exists")]
    AlreadyExists,
    #[error("support report write failed")]
    Write(#[source] std::io::Error),
}

impl SupportExportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Serialize(_) => "SUPPORT_EXPORT_SERIALIZE_FAILED",
            Self::AlreadyExists => "SUPPORT_EXPORT_PATH_EXISTS",
            Self::Write(_) => "SUPPORT_EXPORT_WRITE_FAILED",
        }
    }
}

pub fn export_json(path: &Path) -> Result<(), SupportExportError> {
    let json = serde_json::to_vec_pretty(&collect()).map_err(SupportExportError::Serialize)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                SupportExportError::AlreadyExists
            } else {
                SupportExportError::Write(error)
            }
        })?;
    // shortcut: a failed write may leave a partial new file; use temp-and-publish if atomic export becomes necessary.
    file.write_all(&json).map_err(SupportExportError::Write)
}

fn collect() -> SupportReport {
    from_reports(doctor::inspect(), run_probe())
}

fn from_reports(doctor: DoctorReport, probe: ProbeReport) -> SupportReport {
    let DoctorReport {
        host,
        runtime,
        host_capabilities,
        feature_readiness,
        errors: doctor_errors,
        ..
    } = doctor;
    let executable_compatibility = ExecutableCompatibility {
        status: probe.status,
        process_status: probe.process_status,
        executable_status: probe.executable_status,
        steam_status: probe.steam_status,
        run_integrity: probe.run_integrity,
        candidate_count: probe.processes.len(),
        recognized_build_id: match &probe.build_resolution {
            Some(BuildResolution::ExactMatch { build }) => Some(build.id.clone()),
            Some(BuildResolution::UnknownBuild | BuildResolution::IdentityInconsistent { .. })
            | None => None,
        },
        executable_sha256: probe
            .executable
            .as_ref()
            .map(|executable| executable.sha256.clone()),
        steam_build_id: probe.steam_build_id,
        expected_steam_build_id: probe.expected_steam_build_id,
    };
    let errors = doctor_errors
        .into_iter()
        .map(|error| SupportError {
            component: "DOCTOR",
            field: error.field,
            code: error.code,
        })
        .chain(probe.errors.into_iter().map(|error| SupportError {
            component: "PROBE",
            field: error.field,
            code: error.code,
        }))
        .collect();

    SupportReport {
        schema_version: 1,
        observed_at_unix_ms: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        host,
        runtime,
        host_capabilities,
        feature_readiness,
        executable_compatibility,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        doctor::DoctorError,
        identity::{BuildRecord, ExecutableIdentity},
        probe::ReportError,
    };

    use super::*;

    #[test]
    fn support_report_preserves_compatibility_and_redacts_paths_and_error_messages() {
        let sentinel = "private-support-sentinel";
        let mut doctor = doctor::inspect();
        doctor.errors.push(DoctorError {
            field: "test.host",
            code: "TEST_DOCTOR_ERROR",
            message: sentinel.to_owned(),
        });

        let mut probe = ProbeReport::empty(ProbeStatus::ExactMatch, None);
        probe.executable_status = ExecutableStatus::ExactMatch;
        probe.steam_status = SteamStatus::Match;
        probe.executable = Some(ExecutableIdentity {
            path: PathBuf::from(sentinel),
            sha256: "a".repeat(64),
            size: 1024,
            file_version: None,
            product_version: None,
            version_info_error: Some(sentinel.to_owned()),
        });
        probe.steam_install_dir = Some(sentinel.to_owned());
        probe.steam_build_id = Some("25755602".to_owned());
        probe.expected_steam_build_id = Some("25755602".to_owned());
        probe.errors.push(ReportError {
            field: "executable.path",
            code: "TEST_PROBE_ERROR",
            message: sentinel.to_owned(),
        });
        probe.build_resolution = Some(BuildResolution::ExactMatch {
            build: BuildRecord {
                id: "steam-25755602".to_owned(),
                name: "DOOM: The Dark Ages test build".to_owned(),
                sha256: "a".repeat(64),
                file_version: None,
                product_version: None,
                steam_build_id: Some("25755602".to_owned()),
            },
        });

        let report = from_reports(doctor, probe);
        let json = serde_json::to_string(&report).expect("support report serializes");
        assert!(!json.contains(sentinel));
        assert!(json.contains(&"a".repeat(64)));
        assert!(json.contains("steam-25755602"));
        assert_eq!(
            report.errors,
            vec![
                SupportError {
                    component: "DOCTOR",
                    field: "test.host",
                    code: "TEST_DOCTOR_ERROR",
                },
                SupportError {
                    component: "PROBE",
                    field: "executable.path",
                    code: "TEST_PROBE_ERROR",
                },
            ]
        );
    }
}
