use serde::Serialize;

use crate::{
    identity::{
        BuildRegistry, BuildRegistryError, BuildResolution, ExecutableError, ExecutableIdentity,
    },
    process::{
        ProcessCandidate, ProcessFailure, ProcessFailureKind, ProcessInspection, ProcessScanError,
        enumerate_game_processes,
    },
};

const BUILDS_TOML: &str = include_str!("../data/builds.toml");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProbeStatus {
    ExactMatch,
    UnknownBuild,
    InstallMismatch,
    AmbiguousProcess,
    AccessDenied,
    IdentityInconsistent,
    NoProcess,
    UnsupportedPlatform,
    ProbeError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunIntegrityStatus {
    NotEvaluated,
}

impl ProbeStatus {
    fn label(self) -> &'static str {
        match self {
            Self::ExactMatch => "EXACT_MATCH",
            Self::UnknownBuild => "UNKNOWN_BUILD",
            Self::InstallMismatch => "INSTALL_MISMATCH",
            Self::AmbiguousProcess => "AMBIGUOUS_PROCESS",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::IdentityInconsistent => "IDENTITY_INCONSISTENT",
            Self::NoProcess => "NO_PROCESS",
            Self::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
            Self::ProbeError => "PROBE_ERROR",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProbeReport {
    pub status: ProbeStatus,
    pub run_integrity: RunIntegrityStatus,
    pub processes: Vec<ProcessCandidate>,
    pub executable: Option<ExecutableIdentity>,
    pub build_resolution: Option<BuildResolution>,
    pub evidence_provenance: Vec<&'static str>,
    pub reason: Option<String>,
}

impl ProbeReport {
    fn empty(status: ProbeStatus, reason: Option<String>) -> Self {
        Self {
            status,
            run_integrity: RunIntegrityStatus::NotEvaluated,
            processes: Vec::new(),
            executable: None,
            build_resolution: None,
            evidence_provenance: Vec::new(),
            reason,
        }
    }

    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            "Status: {}\nRun integrity: NOT_EVALUATED\n",
            self.status.label()
        );

        for process in &self.processes {
            output.push_str(&format!(
                "Process candidate: {} (PID {})\n",
                process.reported_name, process.pid
            ));
            if let ProcessInspection::Available { identity } = &process.inspection {
                output.push_str(&format!(
                    "  Creation time (FILETIME): {}\n  Executable: {}\n  Module base: 0x{:X}\n  Image size: {} bytes\n",
                    identity.creation_time_filetime,
                    identity.executable_path.display(),
                    identity.module_base,
                    identity.image_size
                ));
            } else if let ProcessInspection::Unavailable { failure } = &process.inspection {
                output.push_str(&format!(
                    "  Inspection: {:?} during {} (Win32 error: {})\n",
                    failure.kind,
                    failure.operation,
                    failure
                        .win32_error
                        .map_or_else(|| "none".to_owned(), |code| code.to_string())
                ));
            }
        }

        if let Some(executable) = &self.executable {
            output.push_str(&format!(
                "SHA-256: {}\nFile version: {}\nProduct version: {}\n",
                executable.sha256,
                executable.file_version.as_deref().unwrap_or("unavailable"),
                executable
                    .product_version
                    .as_deref()
                    .unwrap_or("unavailable")
            ));
        }
        if let Some(resolution) = &self.build_resolution {
            match resolution {
                BuildResolution::ExactMatch { build } => {
                    output.push_str(&format!(
                        "Registered build: {} ({})\n",
                        build.name, build.id
                    ));
                }
                BuildResolution::UnknownBuild => output.push_str("Registered build: unknown\n"),
                BuildResolution::InstallMismatch { expected_build_id } => {
                    output.push_str(&format!(
                        "Installation mismatch; expected Steam Build ID: {expected_build_id}\n"
                    ))
                }
                BuildResolution::IdentityInconsistent { reason } => {
                    output.push_str(&format!("Identity inconsistency: {reason}\n"));
                }
            }
        }

        for source in &self.evidence_provenance {
            output.push_str(&format!("Evidence: {source}\n"));
        }
        if let Some(reason) = &self.reason {
            output.push_str(&format!("Reason: {reason}\n"));
        }
        output
    }
}

pub fn run_probe() -> ProbeReport {
    let registry = match BuildRegistry::from_toml(BUILDS_TOML) {
        Ok(registry) => registry,
        Err(error) => return registry_error(error),
    };
    run_probe_with_registry(&registry)
}

pub fn run_probe_with_registry(registry: &BuildRegistry) -> ProbeReport {
    let candidates = match enumerate_game_processes() {
        Ok(candidates) => candidates,
        Err(error) => return process_scan_error(error),
    };

    if candidates.is_empty() {
        let mut report = ProbeReport::empty(
            ProbeStatus::NoProcess,
            Some("game process was not found".into()),
        );
        report
            .evidence_provenance
            .push("Windows Toolhelp process snapshot");
        return report;
    }
    if candidates.len() > 1 {
        return ambiguous_process_report(
            candidates,
            "multiple game process candidates were found; none was selected",
        );
    }

    let candidate = &candidates[0];
    let process_identity = match &candidate.inspection {
        ProcessInspection::Available { identity } => identity,
        ProcessInspection::Unavailable { failure } => {
            let (status, reason) = process_failure_details(failure, "during process inspection");
            let mut report = ProbeReport::empty(status, Some(reason));
            report.processes = candidates;
            report
                .evidence_provenance
                .push("Windows Toolhelp process snapshot");
            return report;
        }
    };

    let executable = match ExecutableIdentity::inspect(&process_identity.executable_path) {
        Ok(identity) => identity,
        Err(error) => {
            let (status, reason) = executable_error(error);
            let mut report = ProbeReport::empty(status, Some(reason));
            report.processes = candidates;
            report.evidence_provenance.extend([
                "Windows process path query",
                "executable file open and metadata inspection",
            ]);
            return report;
        }
    };

    let current_candidates = match enumerate_game_processes() {
        Ok(candidates) => candidates,
        Err(error) => {
            let (status, reason) = process_error_parts(error);
            let mut report = ProbeReport::empty(status, Some(reason));
            report.processes = candidates;
            report.executable = Some(executable);
            report.evidence_provenance.extend([
                "Windows process path query",
                "streaming SHA-256 of the executable file",
            ]);
            return report;
        }
    };

    if current_candidates.len() > 1 {
        let mut report = ambiguous_process_report(
            current_candidates,
            "multiple game process candidates existed during inspection; none was selected",
        );
        report.executable = Some(executable);
        report.evidence_provenance.extend([
            "Windows process snapshots before and after file inspection",
            "streaming SHA-256 of the executable file",
        ]);
        return report;
    }

    let verification_failure = match current_candidates.as_slice() {
        [current] if current.pid == candidate.pid => match &current.inspection {
            ProcessInspection::Unavailable { failure } => Some(process_failure_details(
                failure,
                "while rechecking the process",
            )),
            ProcessInspection::Available { identity }
                if current
                    .reported_name
                    .eq_ignore_ascii_case(&candidate.reported_name)
                    && identity == process_identity =>
            {
                None
            }
            ProcessInspection::Available { .. } => Some((
                ProbeStatus::IdentityInconsistent,
                "process identity changed during executable inspection".to_owned(),
            )),
        },
        [] => Some((
            ProbeStatus::IdentityInconsistent,
            "process exited during executable inspection".to_owned(),
        )),
        [_] => Some((
            ProbeStatus::IdentityInconsistent,
            "the discovered process changed during executable inspection".to_owned(),
        )),
        _ => unreachable!("multiple candidates were handled above"),
    };

    if let Some((status, reason)) = verification_failure {
        let mut report = ProbeReport::empty(status, Some(reason));
        report.processes = current_candidates;
        report.executable = Some(executable);
        report.evidence_provenance.extend([
            "Windows process snapshots before and after file inspection",
            "streaming SHA-256 of the executable file",
        ]);
        return report;
    }

    let resolution = registry.resolve(&executable, None);
    let status = match &resolution {
        BuildResolution::ExactMatch { .. } => ProbeStatus::ExactMatch,
        BuildResolution::UnknownBuild => ProbeStatus::UnknownBuild,
        BuildResolution::InstallMismatch { .. } => ProbeStatus::InstallMismatch,
        BuildResolution::IdentityInconsistent { .. } => ProbeStatus::IdentityInconsistent,
    };
    ProbeReport {
        status,
        run_integrity: RunIntegrityStatus::NotEvaluated,
        processes: current_candidates,
        executable: Some(executable),
        build_resolution: Some(resolution),
        evidence_provenance: vec![
            "Windows Toolhelp process and module snapshots",
            "QueryFullProcessImageNameW and GetProcessTimes",
            "streaming SHA-256 of the executable file",
            "bundled data/builds.toml compatibility registry",
        ],
        reason: None,
    }
}

fn process_scan_error(error: ProcessScanError) -> ProbeReport {
    let source = match &error {
        ProcessScanError::UnsupportedPlatform => "native process discovery backend",
        ProcessScanError::Io { .. } => "Windows Toolhelp process snapshot",
    };
    let (status, reason) = process_error_parts(error);
    let mut report = ProbeReport::empty(status, Some(reason));
    report.evidence_provenance.push(source);
    report
}

fn ambiguous_process_report(candidates: Vec<ProcessCandidate>, reason: &str) -> ProbeReport {
    let mut report = ProbeReport::empty(ProbeStatus::AmbiguousProcess, Some(reason.to_owned()));
    report.processes = candidates;
    report
        .evidence_provenance
        .push("Windows Toolhelp process snapshot");
    report
}

fn process_error_parts(error: ProcessScanError) -> (ProbeStatus, String) {
    match error {
        ProcessScanError::UnsupportedPlatform => (
            ProbeStatus::UnsupportedPlatform,
            "native process discovery is currently supported on Windows only".to_owned(),
        ),
        ProcessScanError::Io { operation, source } => {
            let status = if source.kind() == std::io::ErrorKind::PermissionDenied {
                ProbeStatus::AccessDenied
            } else {
                ProbeStatus::ProbeError
            };
            (status, format!("{operation} failed: {source}"))
        }
    }
}

fn process_failure_details(failure: &ProcessFailure, context: &str) -> (ProbeStatus, String) {
    let status = match failure.kind {
        ProcessFailureKind::AccessDenied => ProbeStatus::AccessDenied,
        ProcessFailureKind::IdentityInconsistent | ProcessFailureKind::ProcessExited => {
            ProbeStatus::IdentityInconsistent
        }
        ProcessFailureKind::Unavailable => ProbeStatus::ProbeError,
    };
    let reason = if failure.kind == ProcessFailureKind::ProcessExited {
        format!("process exited {context}")
    } else {
        format!(
            "{} failed {context} (Win32 error: {})",
            failure.operation,
            failure
                .win32_error
                .map_or_else(|| "none".to_owned(), |code| code.to_string())
        )
    };
    (status, reason)
}

fn executable_error(error: ExecutableError) -> (ProbeStatus, String) {
    match error {
        ExecutableError::ChangedDuringInspection(path) => (
            ProbeStatus::IdentityInconsistent,
            format!("executable changed during inspection: {}", path.display()),
        ),
        ExecutableError::Io { path, source } => {
            let status = if source.kind() == std::io::ErrorKind::PermissionDenied {
                ProbeStatus::AccessDenied
            } else if source.kind() == std::io::ErrorKind::NotFound {
                ProbeStatus::IdentityInconsistent
            } else {
                ProbeStatus::ProbeError
            };
            (
                status,
                format!("could not inspect executable {}: {source}", path.display()),
            )
        }
    }
}

fn registry_error(error: BuildRegistryError) -> ProbeReport {
    ProbeReport::empty(
        ProbeStatus::ProbeError,
        Some(format!("build registry is invalid: {error}")),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn human_and_json_outputs_report_the_same_executable_identity() {
        let mut report = ProbeReport::empty(ProbeStatus::ExactMatch, None);
        report.executable = Some(ExecutableIdentity {
            path: PathBuf::from("DOOMTheDarkAges.exe"),
            sha256: "a".repeat(64),
            size: 42,
            file_version: Some("1.2.3.4".to_owned()),
            product_version: Some("1.2.3.4".to_owned()),
        });

        let json = serde_json::to_value(&report).expect("report serializes");
        let human = report.to_human_text();

        assert_eq!(json["status"], "EXACT_MATCH");
        assert_eq!(json["executable"]["sha256"], "a".repeat(64));
        assert!(human.contains("Status: EXACT_MATCH"));
        assert!(human.contains(&format!("SHA-256: {}", "a".repeat(64))));
        assert!(human.contains("Run integrity: NOT_EVALUATED"));
    }

    #[test]
    fn ambiguous_process_report_keeps_every_candidate_without_selecting_one() {
        let report = ambiguous_process_report(
            vec![
                ProcessCandidate {
                    pid: 11,
                    reported_name: "DOOMTheDarkAges.exe".to_owned(),
                    inspection: ProcessInspection::Unavailable {
                        failure: crate::process::ProcessFailure {
                            operation: "OpenProcess".to_owned(),
                            kind: ProcessFailureKind::Unavailable,
                            win32_error: Some(87),
                        },
                    },
                },
                ProcessCandidate {
                    pid: 22,
                    reported_name: "DOOMTheDarkAges.exe".to_owned(),
                    inspection: ProcessInspection::Unavailable {
                        failure: crate::process::ProcessFailure {
                            operation: "OpenProcess".to_owned(),
                            kind: ProcessFailureKind::AccessDenied,
                            win32_error: Some(5),
                        },
                    },
                },
            ],
            "multiple candidates",
        );

        assert_eq!(report.status, ProbeStatus::AmbiguousProcess);
        assert_eq!(report.processes.len(), 2);
        assert!(report.executable.is_none());
    }
}
