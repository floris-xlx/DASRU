use serde::Serialize;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    identity::{
        BuildRegistry, BuildRegistryError, BuildResolution, ExecutableError, ExecutableIdentity,
    },
    process::{
        ProcessCandidate, ProcessFailure, ProcessFailureKind, ProcessInspection, ProcessScanError,
        enumerate_game_processes, enumerate_pinned_game_processes, recheck_pinned_process,
    },
};

const BUILDS_TOML: &str = include_str!("../data/builds.toml");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProbeStatus {
    ExactMatch,
    UnknownBuild,
    AmbiguousProcess,
    AccessDenied,
    IdentityInconsistent,
    NoProcess,
    UnsupportedPlatform,
    ProbeError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InspectionMode {
    Offline,
    Process,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProcessStatus {
    NotEvaluated,
    Verified,
    Ambiguous,
    AccessDenied,
    IdentityInconsistent,
    NotFound,
    Unsupported,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutableStatus {
    NotEvaluated,
    ExactMatch,
    UnknownBuild,
    IdentityInconsistent,
    AccessDenied,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SteamStatus {
    NotEvaluated,
    Match,
    Mismatch,
    Unavailable,
    ReadError,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReportError {
    pub field: &'static str,
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldProvenance {
    pub field: &'static str,
    pub source: &'static str,
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
    pub schema_version: u8,
    pub observed_at_unix_ms: u64,
    pub inspection_mode: InspectionMode,
    pub process_status: ProcessStatus,
    pub executable_status: ExecutableStatus,
    pub steam_status: SteamStatus,
    pub status: ProbeStatus,
    pub run_integrity: RunIntegrityStatus,
    pub processes: Vec<ProcessCandidate>,
    pub executable: Option<ExecutableIdentity>,
    pub steam_build_id: Option<String>,
    pub expected_steam_build_id: Option<String>,
    pub steam_install_dir: Option<String>,
    pub steam_build_id_error: Option<String>,
    pub build_resolution: Option<BuildResolution>,
    pub evidence_provenance: Vec<&'static str>,
    pub field_provenance: Vec<FieldProvenance>,
    pub errors: Vec<ReportError>,
    pub reason: Option<String>,
}

impl ProbeReport {
    pub(crate) fn empty(status: ProbeStatus, reason: Option<String>) -> Self {
        Self {
            schema_version: 1,
            observed_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            inspection_mode: InspectionMode::Process,
            process_status: process_status(status),
            executable_status: ExecutableStatus::NotEvaluated,
            steam_status: SteamStatus::NotEvaluated,
            status,
            run_integrity: RunIntegrityStatus::NotEvaluated,
            processes: Vec::new(),
            executable: None,
            steam_build_id: None,
            expected_steam_build_id: None,
            steam_install_dir: None,
            steam_build_id_error: None,
            build_resolution: None,
            evidence_provenance: Vec::new(),
            field_provenance: Vec::new(),
            errors: reason
                .as_ref()
                .map(|message| {
                    vec![ReportError {
                        field: "probe",
                        code: status.label(),
                        message: message.clone(),
                    }]
                })
                .unwrap_or_default(),
            reason,
        }
    }

    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            "Status: {}\nRun integrity: NOT_EVALUATED\n",
            self.status.label()
        );
        output.insert_str(
            0,
            &format!(
                "Inspection mode: {}\nProcess status: {}\nExecutable status: {}\nSteam metadata status: {}\n",
                self.inspection_mode.label(),
                self.process_status.label(),
                self.executable_status.label(),
                self.steam_status.label(),
            ),
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
            if let Some(error) = &executable.version_info_error {
                output.push_str(&format!("Version metadata read error: {error}\n"));
            }
        }
        if let Some(build_id) = &self.steam_build_id {
            output.push_str(&format!("Steam Build ID (local install): {build_id}\n"));
        }
        if let Some(install_dir) = &self.steam_install_dir {
            output.push_str(&format!("Steam install directory: {install_dir}\n"));
        }
        if let Some(build_id) = &self.expected_steam_build_id {
            output.push_str(&format!("Registered Steam Build ID: {build_id}\n"));
        }
        if let Some(error) = &self.steam_build_id_error {
            output.push_str(&format!("Steam Build ID read error: {error}\n"));
        } else if self.steam_build_id.is_none() && self.steam_status == SteamStatus::Unavailable {
            output.push_str("Steam Build ID: unavailable\n");
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
                BuildResolution::IdentityInconsistent { reason } => {
                    output.push_str(&format!("Identity inconsistency: {reason}\n"));
                }
            }
        }

        for source in &self.evidence_provenance {
            output.push_str(&format!("Evidence: {source}\n"));
        }
        for error in &self.errors {
            output.push_str(&format!(
                "Error [{}] {}: {}\n",
                error.code, error.field, error.message
            ));
        }
        if self.errors.is_empty()
            && let Some(reason) = &self.reason
        {
            output.push_str(&format!("Reason: {reason}\n"));
        }
        output
    }

    pub fn exit_code(&self) -> i32 {
        match self.status {
            ProbeStatus::ExactMatch | ProbeStatus::UnknownBuild => 0,
            _ => 1,
        }
    }
}

impl InspectionMode {
    fn label(self) -> &'static str {
        match self {
            Self::Offline => "OFFLINE",
            Self::Process => "PROCESS",
        }
    }
}

impl ProcessStatus {
    fn label(self) -> &'static str {
        match self {
            Self::NotEvaluated => "NOT_EVALUATED",
            Self::Verified => "VERIFIED",
            Self::Ambiguous => "AMBIGUOUS",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::IdentityInconsistent => "IDENTITY_INCONSISTENT",
            Self::NotFound => "NOT_FOUND",
            Self::Unsupported => "UNSUPPORTED",
            Self::Error => "ERROR",
        }
    }
}

impl ExecutableStatus {
    fn label(self) -> &'static str {
        match self {
            Self::NotEvaluated => "NOT_EVALUATED",
            Self::ExactMatch => "EXACT_MATCH",
            Self::UnknownBuild => "UNKNOWN_BUILD",
            Self::IdentityInconsistent => "IDENTITY_INCONSISTENT",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::Error => "ERROR",
        }
    }
}

impl SteamStatus {
    fn label(self) -> &'static str {
        match self {
            Self::NotEvaluated => "NOT_EVALUATED",
            Self::Match => "MATCH",
            Self::Mismatch => "MISMATCH",
            Self::Unavailable => "UNAVAILABLE",
            Self::ReadError => "READ_ERROR",
        }
    }
}

fn resolution_status(resolution: &BuildResolution) -> ProbeStatus {
    match resolution {
        BuildResolution::ExactMatch { .. } => ProbeStatus::ExactMatch,
        BuildResolution::UnknownBuild => ProbeStatus::UnknownBuild,
        BuildResolution::IdentityInconsistent { .. } => ProbeStatus::IdentityInconsistent,
    }
}

fn executable_status(resolution: &BuildResolution) -> ExecutableStatus {
    match resolution {
        BuildResolution::ExactMatch { .. } => ExecutableStatus::ExactMatch,
        BuildResolution::UnknownBuild => ExecutableStatus::UnknownBuild,
        BuildResolution::IdentityInconsistent { .. } => ExecutableStatus::IdentityInconsistent,
    }
}

fn executable_error_status(status: ProbeStatus) -> ExecutableStatus {
    match status {
        ProbeStatus::AccessDenied => ExecutableStatus::AccessDenied,
        ProbeStatus::IdentityInconsistent => ExecutableStatus::IdentityInconsistent,
        _ => ExecutableStatus::Error,
    }
}

fn process_status(status: ProbeStatus) -> ProcessStatus {
    match status {
        ProbeStatus::AmbiguousProcess => ProcessStatus::Ambiguous,
        ProbeStatus::AccessDenied => ProcessStatus::AccessDenied,
        ProbeStatus::IdentityInconsistent => ProcessStatus::IdentityInconsistent,
        ProbeStatus::NoProcess => ProcessStatus::NotFound,
        ProbeStatus::UnsupportedPlatform => ProcessStatus::Unsupported,
        ProbeStatus::ProbeError => ProcessStatus::Error,
        ProbeStatus::ExactMatch | ProbeStatus::UnknownBuild => ProcessStatus::NotEvaluated,
    }
}

pub fn run_probe() -> ProbeReport {
    run_probe_cancellable(&std::sync::atomic::AtomicBool::new(false))
}
pub fn run_probe_cancellable(cancel: &std::sync::atomic::AtomicBool) -> ProbeReport {
    let registry = match BuildRegistry::from_toml(BUILDS_TOML) {
        Ok(registry) => registry,
        Err(error) => return registry_error(error),
    };
    run_probe_with_registry_cancellable(&registry, cancel)
}

pub fn inspect_executable(path: &Path) -> ProbeReport {
    let registry = match BuildRegistry::from_toml(BUILDS_TOML) {
        Ok(registry) => registry,
        Err(error) => return registry_error(error),
    };
    inspect_executable_with_registry(path, &registry)
}

pub fn inspect_executable_with_registry(path: &Path, registry: &BuildRegistry) -> ProbeReport {
    match inspect_executable_identity(path, registry) {
        Ok(inspection) => {
            let mut report = ProbeReport::empty(resolution_status(&inspection.resolution), None);
            report.inspection_mode = InspectionMode::Offline;
            report.process_status = ProcessStatus::NotEvaluated;
            report.attach_executable_inspection(inspection);
            report
                .evidence_provenance
                .push("offline executable inspection");
            report
        }
        Err((status, reason)) => {
            let mut report = ProbeReport::empty(status, Some(reason.clone()));
            report.inspection_mode = InspectionMode::Offline;
            report.process_status = ProcessStatus::NotEvaluated;
            report.executable_status = executable_error_status(status);
            if let Some(error) = report.errors.first_mut() {
                error.field = "executable";
                error.code = "EXECUTABLE_INSPECTION_FAILED";
            }
            report
                .evidence_provenance
                .push("offline executable inspection");
            report
        }
    }
}

#[derive(Clone)]
struct ExecutableInspection {
    executable: ExecutableIdentity,
    resolution: BuildResolution,
    steam_status: SteamStatus,
    steam_build_id: Option<String>,
    expected_steam_build_id: Option<String>,
    steam_install_dir: Option<String>,
    steam_build_id_error: Option<String>,
    steam_error: Option<ReportError>,
}

impl ProbeReport {
    fn attach_executable_inspection(&mut self, inspection: ExecutableInspection) {
        if let BuildResolution::IdentityInconsistent { reason } = &inspection.resolution {
            self.errors.push(ReportError {
                field: "executable.version_metadata",
                code: "REGISTERED_METADATA_MISMATCH",
                message: reason.clone(),
            });
        }
        if let Some(message) = &inspection.executable.version_info_error {
            self.errors.push(ReportError {
                field: "executable.version_info",
                code: "VERSION_METADATA_READ_FAILED",
                message: message.clone(),
            });
        }
        self.executable_status = executable_status(&inspection.resolution);
        self.steam_status = inspection.steam_status;
        self.executable = Some(inspection.executable);
        self.build_resolution = Some(inspection.resolution);
        self.steam_build_id = inspection.steam_build_id;
        self.expected_steam_build_id = inspection.expected_steam_build_id;
        self.steam_install_dir = inspection.steam_install_dir;
        self.steam_build_id_error = inspection.steam_build_id_error;
        if let Some(error) = inspection.steam_error {
            self.errors.push(error);
        }
        self.field_provenance.extend([
            FieldProvenance {
                field: "executable.sha256",
                source: "streaming SHA-256 of executable file",
            },
            FieldProvenance {
                field: "executable.size",
                source: "executable file metadata and byte count",
            },
            FieldProvenance {
                field: "executable.file_version",
                source: "Windows PE version resource",
            },
            FieldProvenance {
                field: "executable.product_version",
                source: "Windows PE version resource",
            },
        ]);
        if self.steam_status != SteamStatus::NotEvaluated
            && self.steam_status != SteamStatus::Unavailable
        {
            self.field_provenance.extend([
                FieldProvenance {
                    field: "steam.install_dir",
                    source: "local Steam appmanifest",
                },
                FieldProvenance {
                    field: "steam.build_id",
                    source: "local Steam appmanifest",
                },
            ]);
        }
        self.evidence_provenance.extend([
            "streaming SHA-256 of the executable file",
            "bundled data/builds.toml compatibility registry",
        ]);
        if self.steam_status != SteamStatus::NotEvaluated
            && self.steam_status != SteamStatus::Unavailable
        {
            self.evidence_provenance.push("local Steam appmanifest");
        }
    }
}

fn inspect_executable_identity(
    path: &Path,
    registry: &BuildRegistry,
) -> Result<ExecutableInspection, (ProbeStatus, String)> {
    inspect_executable_identity_cancellable(
        path,
        registry,
        &std::sync::atomic::AtomicBool::new(false),
    )
}
fn inspect_executable_identity_cancellable(
    path: &Path,
    registry: &BuildRegistry,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ExecutableInspection, (ProbeStatus, String)> {
    let executable =
        ExecutableIdentity::inspect_cancellable(path, cancel).map_err(executable_error)?;
    let resolution = registry.resolve(&executable);
    let expected_steam_build_id = match &resolution {
        BuildResolution::ExactMatch { build } => build.steam_build_id.clone(),
        BuildResolution::UnknownBuild | BuildResolution::IdentityInconsistent { .. } => None,
    };
    let (steam_status, steam_build_id, steam_install_dir, steam_build_id_error, steam_error) =
        match crate::steam::read_install_metadata(path) {
            Ok(None) => (SteamStatus::Unavailable, None, None, None, None),
            Ok(Some(metadata)) => {
                if let Some(expected) = expected_steam_build_id.as_deref()
                    && expected != metadata.build_id
                {
                    let message = format!(
                        "Steam Build ID {} differs from registered Build ID {expected}",
                        metadata.build_id
                    );
                    (
                        SteamStatus::Mismatch,
                        Some(metadata.build_id),
                        Some(metadata.install_dir),
                        None,
                        Some(ReportError {
                            field: "steam.build_id",
                            code: "STEAM_BUILD_ID_MISMATCH",
                            message,
                        }),
                    )
                } else {
                    (
                        SteamStatus::Match,
                        Some(metadata.build_id),
                        Some(metadata.install_dir),
                        None,
                        None,
                    )
                }
            }
            Err(crate::steam::SteamReadError::InstallationMismatch { metadata }) => (
                SteamStatus::Mismatch,
                Some(metadata.build_id),
                Some(metadata.install_dir),
                None,
                Some(ReportError {
                    field: "steam.install_dir",
                    code: "STEAM_INSTALL_DIR_MISMATCH",
                    message: "manifest installdir does not contain the inspected executable"
                        .to_owned(),
                }),
            ),
            Err(crate::steam::SteamReadError::Manifest(message)) => (
                SteamStatus::ReadError,
                None,
                None,
                Some(message.clone()),
                Some(ReportError {
                    field: "steam.manifest",
                    code: "STEAM_MANIFEST_READ_FAILED",
                    message,
                }),
            ),
        };
    Ok(ExecutableInspection {
        executable,
        resolution,
        steam_status,
        steam_build_id,
        expected_steam_build_id,
        steam_install_dir,
        steam_build_id_error,
        steam_error,
    })
}

pub fn run_probe_with_registry(registry: &BuildRegistry) -> ProbeReport {
    run_probe_with_registry_cancellable(registry, &std::sync::atomic::AtomicBool::new(false))
}
fn run_probe_with_registry_cancellable(
    registry: &BuildRegistry,
    cancel: &std::sync::atomic::AtomicBool,
) -> ProbeReport {
    let pinned_candidates = match enumerate_pinned_game_processes() {
        Ok(candidates) => candidates,
        Err(error) => return process_scan_error(error),
    };
    let candidates: Vec<_> = pinned_candidates
        .iter()
        .map(|candidate| candidate.candidate.clone())
        .collect();

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

    let candidate = &pinned_candidates[0].candidate;
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

    let executable_inspection = match inspect_executable_identity_cancellable(
        &process_identity.executable_path,
        registry,
        cancel,
    ) {
        Ok(inspection) => inspection,
        Err((status, reason)) => {
            let mut report = ProbeReport::empty(status, Some(reason));
            report.processes = candidates;
            report.process_status = ProcessStatus::Verified;
            report.executable_status = executable_error_status(status);
            if let Some(error) = report.errors.first_mut() {
                error.field = "executable";
                error.code = "EXECUTABLE_INSPECTION_FAILED";
            }
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
            report.process_status = process_status(status);
            report.attach_executable_inspection(executable_inspection.clone());
            report
                .evidence_provenance
                .extend(["Windows process path query"]);
            return report;
        }
    };

    if current_candidates.len() > 1 {
        let mut report = ambiguous_process_report(
            current_candidates,
            "multiple game process candidates existed during inspection; none was selected",
        );
        report.process_status = ProcessStatus::Ambiguous;
        report.attach_executable_inspection(executable_inspection.clone());
        report
            .evidence_provenance
            .extend(["Windows process snapshots before and after file inspection"]);
        return report;
    }

    let retained_process_inspection = recheck_pinned_process(&pinned_candidates[0]);

    let mut verification_failure = match current_candidates.as_slice() {
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
    if verification_failure.is_none() {
        verification_failure = match retained_process_inspection {
            ProcessInspection::Available { identity } if identity == *process_identity => None,
            ProcessInspection::Available { .. } => Some((
                ProbeStatus::IdentityInconsistent,
                "process identity changed on the retained process handle".to_owned(),
            )),
            ProcessInspection::Unavailable { failure } => Some(process_failure_details(
                &failure,
                "while rechecking the retained process handle",
            )),
        };
    }

    if let Some((status, reason)) = verification_failure {
        let mut report = ProbeReport::empty(status, Some(reason));
        report.processes = current_candidates;
        report.process_status = process_status(status);
        report.attach_executable_inspection(executable_inspection);
        report
            .evidence_provenance
            .extend(["Windows process snapshots before and after file inspection"]);
        return report;
    }

    let status = resolution_status(&executable_inspection.resolution);
    let mut report = ProbeReport::empty(status, None);
    report.process_status = ProcessStatus::Verified;
    report.processes = current_candidates;
    report.attach_executable_inspection(executable_inspection);
    report.evidence_provenance.extend([
        "Windows Toolhelp process and module snapshots",
        "QueryFullProcessImageNameW and GetProcessTimes",
    ]);
    report.field_provenance.push(FieldProvenance {
        field: "processes",
        source: "Windows Toolhelp process and module snapshots",
    });
    report
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
            version_info_error: Some("access denied".to_owned()),
        });
        report.steam_build_id = Some("25755602".to_owned());

        let json = serde_json::to_value(&report).expect("report serializes");
        let human = report.to_human_text();

        assert_eq!(json["status"], "EXACT_MATCH");
        assert_eq!(json["executable"]["sha256"], "a".repeat(64));
        assert_eq!(json["executable"]["version_info_error"], "access denied");
        assert_eq!(json["steam_build_id"], "25755602");
        assert!(human.contains("Status: EXACT_MATCH"));
        assert!(human.contains(&format!("SHA-256: {}", "a".repeat(64))));
        assert!(human.contains("Version metadata read error: access denied"));
        assert!(human.contains("Steam Build ID (local install): 25755602"));
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

    #[test]
    fn reports_unreadable_steam_manifest_without_claiming_a_build_id() {
        let mut report = ProbeReport::empty(ProbeStatus::UnknownBuild, None);
        report.steam_build_id_error = Some("invalid appmanifest".to_owned());

        let json = serde_json::to_value(&report).expect("report serializes");
        let human = report.to_human_text();

        assert!(json["steam_build_id"].is_null());
        assert_eq!(json["steam_build_id_error"], "invalid appmanifest");
        assert!(human.contains("Steam Build ID read error: invalid appmanifest"));
    }
}
