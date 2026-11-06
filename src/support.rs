use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

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
    #[error("support temporary file creation failed")]
    TempCreate(#[source] io::Error),
    #[error("support report write failed")]
    Write(#[source] io::Error),
    #[error("support report publication failed")]
    Publish(#[source] io::Error),
}

impl SupportExportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Serialize(_) => "SUPPORT_EXPORT_SERIALIZE_FAILED",
            Self::AlreadyExists => "SUPPORT_EXPORT_PATH_EXISTS",
            Self::TempCreate(_) => "SUPPORT_EXPORT_TEMP_CREATE_FAILED",
            Self::Write(_) => "SUPPORT_EXPORT_WRITE_FAILED",
            Self::Publish(_) => "SUPPORT_EXPORT_PUBLISH_FAILED",
        }
    }
}

pub fn export_json(path: &Path) -> Result<(), SupportExportError> {
    let json = serde_json::to_vec_pretty(&collect()).map_err(SupportExportError::Serialize)?;
    let (temporary, mut file) = create_temporary_export(path)?;
    write_temporary(&mut file, &json)?;
    file.sync_all().map_err(SupportExportError::Write)?;
    drop(file);

    // shortcut: destination volumes must support hard links; use platform no-replace rename if this limits installs.
    // A same-directory hard link publishes atomically and fails if the destination already exists.
    match fs::hard_link(&temporary.path, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(SupportExportError::AlreadyExists)
        }
        Err(error) => Err(SupportExportError::Publish(error)),
    }
}

fn write_temporary(file: &mut impl Write, json: &[u8]) -> Result<(), SupportExportError> {
    file.write_all(json).map_err(SupportExportError::Write)?;
    file.flush().map_err(SupportExportError::Write)
}

struct TemporaryExport {
    path: PathBuf,
}

impl Drop for TemporaryExport {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_temporary_export(
    destination: &Path,
) -> Result<(TemporaryExport, File), SupportExportError> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..128 {
        let path = parent.join(format!(
            ".dasru-support-{}-{nonce}-{attempt}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(SupportExportError::TempCreate(error)),
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) = file.set_permissions(fs::Permissions::from_mode(0o600)) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(SupportExportError::TempCreate(error));
            }
        }

        return Ok((TemporaryExport { path }, file));
    }

    Err(SupportExportError::TempCreate(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary filename",
    )))
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
        process::{ProcessCandidate, ProcessIdentity, ProcessInspection},
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
            file_version: Some(sentinel.to_owned()),
            product_version: Some(sentinel.to_owned()),
            version_info_error: Some(sentinel.to_owned()),
        });
        probe.steam_install_dir = Some(sentinel.to_owned());
        probe.steam_build_id = Some("25755602".to_owned());
        probe.expected_steam_build_id = Some("25755602".to_owned());
        probe.steam_build_id_error = Some(sentinel.to_owned());
        probe.reason = Some(sentinel.to_owned());
        probe.processes.push(ProcessCandidate {
            pid: 123456,
            reported_name: sentinel.to_owned(),
            inspection: ProcessInspection::Available {
                identity: ProcessIdentity {
                    creation_time_filetime: 1,
                    executable_path: PathBuf::from(sentinel),
                    module_base: 2,
                    image_size: 3,
                },
            },
        });
        probe.errors.push(ReportError {
            field: "executable.path",
            code: "TEST_PROBE_ERROR",
            message: sentinel.to_owned(),
        });
        probe.build_resolution = Some(BuildResolution::ExactMatch {
            build: BuildRecord {
                id: "steam-25755602".to_owned(),
                name: sentinel.to_owned(),
                sha256: "a".repeat(64),
                file_version: Some(sentinel.to_owned()),
                product_version: Some(sentinel.to_owned()),
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

    #[test]
    fn write_failure_removes_the_staged_file() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("private write failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let directory = std::env::temp_dir().join(format!(
            "dasru-support-write-failure-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("report.json");
        let temporary_path;
        let result = {
            let (temporary, _file) = create_temporary_export(&destination).unwrap();
            temporary_path = temporary.path.clone();
            write_temporary(&mut FailingWriter, b"report")
        };

        assert!(matches!(result, Err(SupportExportError::Write(_))));
        assert!(!temporary_path.exists());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir(directory).unwrap();
    }
}
