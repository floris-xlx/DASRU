use crate::probe::ProbeReport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureId {
    ProcessProbe,
    ExecutableInspection,
    MemoryScan,
    MetricsDiscovery,
    ExperimentalFpsMarker,
    SupportExport,
    GlobalHotkeys,
    KeyboardInput,
    Clipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureStatus {
    Supported,
    Unsupported,
    Unknown,
    Experimental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityReason {
    Available,
    NoProcess,
    AmbiguousProcess,
    AccessDenied,
    UnknownBuild,
    IdentityChanged,
    UnsupportedPlatform,
    NotImplemented,
    NotEvaluated,
    ExplicitOptInRequired,
    UnsupportedLayout,
    PartialCoverage,
    UnsupportedArchitecture,
    CollectionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationStatus {
    Implemented,
    Experimental,
    NotImplemented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accessibility {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    NotRequired,
    Required,
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X86,
    X86_64,
    Aarch64,
    Other,
}

impl Architecture {
    pub const fn current() -> Self {
        #[cfg(target_arch = "x86")]
        {
            Self::X86
        }
        #[cfg(target_arch = "x86_64")]
        {
            Self::X86_64
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::Aarch64
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::Other
        }
    }
}

impl Platform {
    pub const fn current() -> Self {
        #[cfg(windows)]
        {
            Self::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessEvidence {
    NotEvaluated,
    None,
    Ambiguous,
    AccessDenied,
    IdentityChanged,
    Verified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildEvidence {
    NotEvaluated,
    Unknown,
    Inconsistent,
    ExactMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEvidence {
    NotEvaluated,
    AccessDenied,
    Partial,
    Readable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutEvidence {
    NotEvaluated,
    Unsupported,
    Safe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityEvidence {
    pub platform: Platform,
    pub architecture: Architecture,
    pub process: ProcessEvidence,
    pub build: BuildEvidence,
    pub memory: MemoryEvidence,
    pub layout: LayoutEvidence,
    pub executable_readable: Option<bool>,
    pub report_collected: Option<bool>,
    pub destination_writable: Option<bool>,
    pub experimental_opt_in: bool,
}

impl Default for CapabilityEvidence {
    fn default() -> Self {
        Self {
            platform: Platform::current(),
            architecture: Architecture::current(),
            process: ProcessEvidence::NotEvaluated,
            build: BuildEvidence::NotEvaluated,
            memory: MemoryEvidence::NotEvaluated,
            layout: LayoutEvidence::NotEvaluated,
            executable_readable: None,
            report_collected: None,
            destination_writable: None,
            experimental_opt_in: false,
        }
    }
}

impl CapabilityEvidence {
    pub fn from_probe(report: &ProbeReport) -> Self {
        let process = match report.process_status {
            crate::probe::ProcessStatus::Verified => ProcessEvidence::Verified,
            crate::probe::ProcessStatus::Ambiguous => ProcessEvidence::Ambiguous,
            crate::probe::ProcessStatus::AccessDenied => ProcessEvidence::AccessDenied,
            crate::probe::ProcessStatus::IdentityInconsistent => ProcessEvidence::IdentityChanged,
            crate::probe::ProcessStatus::NotFound => ProcessEvidence::None,
            crate::probe::ProcessStatus::NotEvaluated
            | crate::probe::ProcessStatus::Unsupported
            | crate::probe::ProcessStatus::Error => ProcessEvidence::NotEvaluated,
        };
        let build = match &report.build_resolution {
            Some(crate::identity::BuildResolution::ExactMatch { .. }) => BuildEvidence::ExactMatch,
            Some(crate::identity::BuildResolution::UnknownBuild) => BuildEvidence::Unknown,
            Some(crate::identity::BuildResolution::IdentityInconsistent { .. }) => {
                BuildEvidence::Inconsistent
            }
            None => BuildEvidence::NotEvaluated,
        };
        Self {
            process,
            build,
            executable_readable: match report.executable_status {
                crate::probe::ExecutableStatus::ExactMatch
                | crate::probe::ExecutableStatus::UnknownBuild
                | crate::probe::ExecutableStatus::IdentityInconsistent => Some(true),
                crate::probe::ExecutableStatus::AccessDenied
                | crate::probe::ExecutableStatus::Error => Some(false),
                crate::probe::ExecutableStatus::NotEvaluated => None,
            },
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDecision {
    pub status: FeatureStatus,
    pub reason: CapabilityReason,
    pub implementation: ImplementationStatus,
    pub accessibility: Accessibility,
    pub authorization: Authorization,
}

impl CapabilityDecision {
    pub const fn implementation_status(self) -> &'static str {
        match self.implementation {
            ImplementationStatus::Implemented => "IMPLEMENTED",
            ImplementationStatus::Experimental => "EXPERIMENTAL",
            ImplementationStatus::NotImplemented => "NOT_IMPLEMENTED",
        }
    }

    pub const fn is_authorized(self) -> bool {
        matches!(
            self.status,
            FeatureStatus::Supported | FeatureStatus::Experimental
        ) && matches!(
            self.authorization,
            Authorization::NotRequired | Authorization::Granted
        )
    }

    pub const fn diagnostic_readiness(self) -> &'static str {
        match (self.implementation, self.reason, self.status) {
            (
                _,
                CapabilityReason::UnsupportedPlatform | CapabilityReason::UnsupportedArchitecture,
                _,
            ) => "UNSUPPORTED_PLATFORM",
            (ImplementationStatus::NotImplemented, _, _) => "NOT_IMPLEMENTED",
            (_, CapabilityReason::PartialCoverage, _) => "UNKNOWN",
            (
                ImplementationStatus::Experimental,
                CapabilityReason::NotEvaluated,
                FeatureStatus::Unknown,
            ) => "EXPERIMENTAL_OPT_IN",
            (_, CapabilityReason::NotEvaluated, FeatureStatus::Unknown) => "NOT_PROBED",
            (ImplementationStatus::Experimental, _, FeatureStatus::Experimental) => {
                "EXPERIMENTAL_OPT_IN"
            }
            (_, _, FeatureStatus::Supported) => "READY",
            (_, _, FeatureStatus::Unsupported) => "UNSUPPORTED",
            (_, _, FeatureStatus::Unknown) => "UNKNOWN",
            (_, _, FeatureStatus::Experimental) => "EXPERIMENTAL_OPT_IN",
        }
    }
}

pub fn evaluate(feature: FeatureId, evidence: &CapabilityEvidence) -> CapabilityDecision {
    use Accessibility as Access;
    use Authorization as Auth;
    use CapabilityReason as Reason;
    use FeatureStatus as Status;
    use ImplementationStatus as Impl;

    match feature {
        FeatureId::ProcessProbe => process_probe_implemented(evidence.platform),
        FeatureId::ExecutableInspection => match evidence.executable_readable {
            Some(true) => decision(
                Status::Supported,
                Reason::Available,
                Impl::Implemented,
                Access::Available,
                Auth::NotRequired,
            ),
            Some(false) => decision(
                Status::Unsupported,
                Reason::AccessDenied,
                Impl::Implemented,
                Access::Unavailable,
                Auth::NotRequired,
            ),
            None => decision(
                Status::Unknown,
                Reason::NotEvaluated,
                Impl::Implemented,
                Access::Unknown,
                Auth::NotRequired,
            ),
        },
        FeatureId::MemoryScan => {
            if evidence.platform != Platform::Windows {
                unsupported_platform()
            } else if evidence.architecture != Architecture::X86_64 {
                unsupported_architecture(Impl::Implemented)
            } else if let Some(failure) = process_failure(evidence.process, Impl::Implemented) {
                failure
            } else if evidence.process == ProcessEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Implemented,
                    Access::Unknown,
                    Auth::NotRequired,
                )
            } else {
                match evidence.memory {
                    MemoryEvidence::Readable => decision(
                        Status::Supported,
                        Reason::Available,
                        Impl::Implemented,
                        Access::Available,
                        Auth::NotRequired,
                    ),
                    MemoryEvidence::Partial => decision(
                        Status::Supported,
                        Reason::PartialCoverage,
                        Impl::Implemented,
                        Access::Available,
                        Auth::NotRequired,
                    ),
                    MemoryEvidence::AccessDenied => decision(
                        Status::Unsupported,
                        Reason::AccessDenied,
                        Impl::Implemented,
                        Access::Unavailable,
                        Auth::NotRequired,
                    ),
                    MemoryEvidence::NotEvaluated => decision(
                        Status::Unknown,
                        Reason::NotEvaluated,
                        Impl::Implemented,
                        Access::Unknown,
                        Auth::NotRequired,
                    ),
                }
            }
        }
        FeatureId::MetricsDiscovery => {
            if evidence.platform != Platform::Windows {
                unsupported_platform()
            } else if evidence.architecture != Architecture::X86_64 {
                unsupported_architecture(Impl::Implemented)
            } else if let Some(failure) = process_failure(evidence.process, Impl::Implemented) {
                failure
            } else if evidence.process == ProcessEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Implemented,
                    Access::Unknown,
                    Auth::NotRequired,
                )
            } else if evidence.process != ProcessEvidence::Verified {
                decision(
                    Status::Unsupported,
                    Reason::NoProcess,
                    Impl::Implemented,
                    Access::Unavailable,
                    Auth::NotRequired,
                )
            } else if evidence.build == BuildEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Implemented,
                    Access::Unknown,
                    Auth::NotRequired,
                )
            } else if evidence.build != BuildEvidence::ExactMatch {
                decision(
                    Status::Unsupported,
                    Reason::UnknownBuild,
                    Impl::Implemented,
                    Access::Available,
                    Auth::NotRequired,
                )
            } else if evidence.memory == MemoryEvidence::NotEvaluated
                || evidence.layout == LayoutEvidence::NotEvaluated
            {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Implemented,
                    Access::Unknown,
                    Auth::NotRequired,
                )
            } else if evidence.memory == MemoryEvidence::AccessDenied {
                decision(
                    Status::Unsupported,
                    Reason::AccessDenied,
                    Impl::Implemented,
                    Access::Unavailable,
                    Auth::NotRequired,
                )
            } else if evidence.memory == MemoryEvidence::Partial {
                decision(
                    Status::Unsupported,
                    Reason::PartialCoverage,
                    Impl::Implemented,
                    Access::Available,
                    Auth::NotRequired,
                )
            } else if evidence.layout == LayoutEvidence::Unsupported {
                decision(
                    Status::Unsupported,
                    Reason::UnsupportedLayout,
                    Impl::Implemented,
                    Access::Available,
                    Auth::NotRequired,
                )
            } else if evidence.memory == MemoryEvidence::Readable
                && evidence.layout == LayoutEvidence::Safe
            {
                decision(
                    Status::Supported,
                    Reason::Available,
                    Impl::Implemented,
                    Access::Available,
                    Auth::NotRequired,
                )
            } else {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Implemented,
                    Access::Unknown,
                    Auth::NotRequired,
                )
            }
        }
        FeatureId::ExperimentalFpsMarker => {
            if evidence.platform != Platform::Windows {
                unsupported_platform()
            } else if evidence.architecture != Architecture::X86_64 {
                unsupported_architecture(Impl::Experimental)
            } else if let Some(failure) = process_failure(evidence.process, Impl::Experimental) {
                failure
            } else if evidence.process == ProcessEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Experimental,
                    Access::Unknown,
                    if evidence.experimental_opt_in {
                        Auth::Denied
                    } else {
                        Auth::Required
                    },
                )
            } else if evidence.process != ProcessEvidence::Verified {
                decision(
                    Status::Unsupported,
                    Reason::NoProcess,
                    Impl::Experimental,
                    Access::Unavailable,
                    Auth::Denied,
                )
            } else if evidence.build == BuildEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Experimental,
                    Access::Unknown,
                    if evidence.experimental_opt_in {
                        Auth::Denied
                    } else {
                        Auth::Required
                    },
                )
            } else if evidence.build != BuildEvidence::ExactMatch {
                decision(
                    Status::Unsupported,
                    Reason::UnknownBuild,
                    Impl::Experimental,
                    Access::Available,
                    Auth::Denied,
                )
            } else if evidence.memory == MemoryEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Experimental,
                    Access::Unknown,
                    if evidence.experimental_opt_in {
                        Auth::Denied
                    } else {
                        Auth::Required
                    },
                )
            } else if evidence.memory == MemoryEvidence::Partial {
                decision(
                    Status::Unsupported,
                    Reason::PartialCoverage,
                    Impl::Experimental,
                    Access::Available,
                    Auth::Denied,
                )
            } else if evidence.memory == MemoryEvidence::AccessDenied {
                decision(
                    Status::Unsupported,
                    Reason::AccessDenied,
                    Impl::Experimental,
                    Access::Unavailable,
                    Auth::Denied,
                )
            } else if evidence.layout == LayoutEvidence::NotEvaluated {
                decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Experimental,
                    Access::Unknown,
                    if evidence.experimental_opt_in {
                        Auth::Denied
                    } else {
                        Auth::Required
                    },
                )
            } else if evidence.layout != LayoutEvidence::Safe {
                decision(
                    Status::Unsupported,
                    Reason::UnsupportedLayout,
                    Impl::Experimental,
                    Access::Available,
                    Auth::Denied,
                )
            } else if !evidence.experimental_opt_in {
                decision(
                    Status::Experimental,
                    Reason::ExplicitOptInRequired,
                    Impl::Experimental,
                    Access::Available,
                    Auth::Required,
                )
            } else {
                decision(
                    Status::Experimental,
                    Reason::Available,
                    Impl::Experimental,
                    Access::Available,
                    Auth::Granted,
                )
            }
        }
        FeatureId::SupportExport => {
            match (evidence.report_collected, evidence.destination_writable) {
                (Some(false), _) => decision(
                    Status::Unsupported,
                    Reason::CollectionFailed,
                    Impl::Implemented,
                    Access::Unavailable,
                    Auth::NotRequired,
                ),
                (Some(true), Some(true)) => decision(
                    Status::Supported,
                    Reason::Available,
                    Impl::Implemented,
                    Access::Available,
                    Auth::NotRequired,
                ),
                (Some(true), Some(false)) => decision(
                    Status::Unsupported,
                    Reason::AccessDenied,
                    Impl::Implemented,
                    Access::Unavailable,
                    Auth::NotRequired,
                ),
                (None, _) | (Some(true), None) => decision(
                    Status::Unknown,
                    Reason::NotEvaluated,
                    Impl::Implemented,
                    Access::Unknown,
                    Auth::NotRequired,
                ),
            }
        }
        FeatureId::GlobalHotkeys | FeatureId::KeyboardInput | FeatureId::Clipboard => decision(
            Status::Unsupported,
            Reason::NotImplemented,
            Impl::NotImplemented,
            Access::Unavailable,
            Auth::Denied,
        ),
    }
}

fn process_failure(
    process: ProcessEvidence,
    implementation: ImplementationStatus,
) -> Option<CapabilityDecision> {
    use Accessibility as Access;
    use Authorization as Auth;
    use CapabilityReason as Reason;
    use FeatureStatus as Status;
    use ImplementationStatus as Impl;

    let reason = match process {
        ProcessEvidence::None => Reason::NoProcess,
        ProcessEvidence::Ambiguous => Reason::AmbiguousProcess,
        ProcessEvidence::AccessDenied => Reason::AccessDenied,
        ProcessEvidence::IdentityChanged => Reason::IdentityChanged,
        ProcessEvidence::NotEvaluated | ProcessEvidence::Verified => return None,
    };
    Some(decision(
        Status::Unsupported,
        reason,
        implementation,
        Access::Unavailable,
        if implementation == Impl::Experimental {
            Auth::Denied
        } else {
            Auth::NotRequired
        },
    ))
}

fn process_probe_implemented(platform: Platform) -> CapabilityDecision {
    use Accessibility as Access;
    use Authorization as Auth;
    use FeatureStatus as Status;
    use ImplementationStatus as Impl;

    if platform == Platform::Windows {
        decision(
            Status::Supported,
            CapabilityReason::Available,
            Impl::Implemented,
            Access::Available,
            Auth::NotRequired,
        )
    } else {
        unsupported_platform()
    }
}

const fn unsupported_platform() -> CapabilityDecision {
    decision(
        FeatureStatus::Unsupported,
        CapabilityReason::UnsupportedPlatform,
        ImplementationStatus::NotImplemented,
        Accessibility::Unavailable,
        Authorization::Denied,
    )
}

const fn unsupported_architecture(implementation: ImplementationStatus) -> CapabilityDecision {
    decision(
        FeatureStatus::Unsupported,
        CapabilityReason::UnsupportedArchitecture,
        implementation,
        Accessibility::Unavailable,
        Authorization::Denied,
    )
}

const fn decision(
    status: FeatureStatus,
    reason: CapabilityReason,
    implementation: ImplementationStatus,
    accessibility: Accessibility,
    authorization: Authorization,
) -> CapabilityDecision {
    CapabilityDecision {
        status,
        reason,
        implementation,
        accessibility,
        authorization,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_fps_evidence() -> CapabilityEvidence {
        CapabilityEvidence {
            platform: Platform::Windows,
            architecture: Architecture::X86_64,
            process: ProcessEvidence::Verified,
            build: BuildEvidence::ExactMatch,
            memory: MemoryEvidence::Readable,
            layout: LayoutEvidence::Safe,
            ..CapabilityEvidence::default()
        }
    }

    #[test]
    fn process_probe_requires_a_supported_process_backend() {
        let evidence = CapabilityEvidence {
            platform: Platform::Windows,
            process: ProcessEvidence::None,
            build: BuildEvidence::Unknown,
            ..CapabilityEvidence::default()
        };
        let decision = evaluate(FeatureId::ProcessProbe, &evidence);
        assert_eq!(decision.status, FeatureStatus::Supported);
        assert!(decision.is_authorized());

        let linux = CapabilityEvidence {
            platform: Platform::Linux,
            ..evidence
        };
        let linux_decision = evaluate(FeatureId::ProcessProbe, &linux);
        assert_eq!(linux_decision.status, FeatureStatus::Unsupported);
        assert_eq!(linux_decision.reason, CapabilityReason::UnsupportedPlatform);
        let other = CapabilityEvidence {
            platform: Platform::Other,
            ..evidence
        };
        assert_eq!(
            evaluate(FeatureId::ProcessProbe, &other).status,
            FeatureStatus::Unsupported
        );
    }

    #[test]
    fn memory_features_require_the_supported_windows_architecture() {
        let evidence = CapabilityEvidence {
            architecture: Architecture::Aarch64,
            experimental_opt_in: true,
            ..ready_fps_evidence()
        };
        for feature in [
            FeatureId::MemoryScan,
            FeatureId::MetricsDiscovery,
            FeatureId::ExperimentalFpsMarker,
        ] {
            let decision = evaluate(feature, &evidence);
            assert_eq!(decision.status, FeatureStatus::Unsupported);
            assert_eq!(decision.reason, CapabilityReason::UnsupportedArchitecture);
            assert!(!decision.is_authorized());
        }
    }

    #[test]
    fn read_only_inspection_and_scanning_do_not_require_a_known_build() {
        let evidence = CapabilityEvidence {
            platform: Platform::Windows,
            process: ProcessEvidence::Verified,
            build: BuildEvidence::Unknown,
            memory: MemoryEvidence::Readable,
            executable_readable: Some(true),
            ..CapabilityEvidence::default()
        };
        assert_eq!(
            evaluate(FeatureId::ExecutableInspection, &evidence).status,
            FeatureStatus::Supported
        );
        assert_eq!(
            evaluate(FeatureId::MemoryScan, &evidence).status,
            FeatureStatus::Supported
        );
        assert_eq!(
            evaluate(FeatureId::MetricsDiscovery, &evidence).reason,
            CapabilityReason::UnknownBuild
        );
    }

    #[test]
    fn unknown_build_never_supports_build_specific_features() {
        let evidence = CapabilityEvidence {
            build: BuildEvidence::Unknown,
            ..ready_fps_evidence()
        };
        for feature in [
            FeatureId::MetricsDiscovery,
            FeatureId::ExperimentalFpsMarker,
        ] {
            let decision = evaluate(feature, &evidence);
            assert_eq!(decision.status, FeatureStatus::Unsupported);
            assert_eq!(decision.reason, CapabilityReason::UnknownBuild);
            assert_eq!(decision.diagnostic_readiness(), "UNSUPPORTED");
            assert!(!decision.is_authorized());
        }
    }

    #[test]
    fn experimental_status_is_separate_from_explicit_authorization() {
        let unprobed = evaluate(
            FeatureId::ExperimentalFpsMarker,
            &CapabilityEvidence {
                platform: Platform::Windows,
                architecture: Architecture::X86_64,
                ..CapabilityEvidence::default()
            },
        );
        assert_eq!(unprobed.status, FeatureStatus::Unknown);
        assert_eq!(unprobed.diagnostic_readiness(), "EXPERIMENTAL_OPT_IN");

        let without_opt_in = evaluate(FeatureId::ExperimentalFpsMarker, &ready_fps_evidence());
        assert_eq!(without_opt_in.status, FeatureStatus::Experimental);
        assert_eq!(
            without_opt_in.reason,
            CapabilityReason::ExplicitOptInRequired
        );
        assert_eq!(without_opt_in.diagnostic_readiness(), "EXPERIMENTAL_OPT_IN");
        assert!(!without_opt_in.is_authorized());

        let evidence = CapabilityEvidence {
            experimental_opt_in: true,
            ..ready_fps_evidence()
        };
        let authorized = evaluate(FeatureId::ExperimentalFpsMarker, &evidence);
        assert_eq!(authorized.status, FeatureStatus::Experimental);
        assert!(authorized.is_authorized());
    }

    #[test]
    fn process_failures_fail_closed_for_memory_features() {
        for (process, reason) in [
            (ProcessEvidence::None, CapabilityReason::NoProcess),
            (
                ProcessEvidence::Ambiguous,
                CapabilityReason::AmbiguousProcess,
            ),
            (
                ProcessEvidence::AccessDenied,
                CapabilityReason::AccessDenied,
            ),
            (
                ProcessEvidence::IdentityChanged,
                CapabilityReason::IdentityChanged,
            ),
        ] {
            let evidence = CapabilityEvidence {
                process,
                ..ready_fps_evidence()
            };
            let decision = evaluate(FeatureId::MemoryScan, &evidence);
            assert_eq!(decision.status, FeatureStatus::Unsupported);
            assert_eq!(decision.reason, reason);
            assert!(!decision.is_authorized());

            let experimental = evaluate(FeatureId::ExperimentalFpsMarker, &evidence);
            assert_eq!(experimental.status, FeatureStatus::Unsupported);
            assert_eq!(experimental.reason, reason);
            assert_eq!(
                experimental.implementation,
                ImplementationStatus::Experimental
            );
            assert!(!experimental.is_authorized());
        }
    }

    #[test]
    fn unsupported_layout_and_partial_memory_never_authorize_a_write() {
        let unsupported_layout = CapabilityEvidence {
            layout: LayoutEvidence::Unsupported,
            experimental_opt_in: true,
            ..ready_fps_evidence()
        };
        assert_eq!(
            evaluate(FeatureId::ExperimentalFpsMarker, &unsupported_layout).reason,
            CapabilityReason::UnsupportedLayout
        );
        let partial_memory = CapabilityEvidence {
            memory: MemoryEvidence::Partial,
            experimental_opt_in: true,
            ..ready_fps_evidence()
        };
        let decision = evaluate(FeatureId::MemoryScan, &partial_memory);
        assert_eq!(decision.status, FeatureStatus::Supported);
        assert_eq!(decision.reason, CapabilityReason::PartialCoverage);
        assert_eq!(decision.diagnostic_readiness(), "UNKNOWN");
        let decision = evaluate(FeatureId::ExperimentalFpsMarker, &partial_memory);
        assert_eq!(decision.reason, CapabilityReason::PartialCoverage);
        assert!(!decision.is_authorized());

        let partial_metrics = CapabilityEvidence {
            memory: MemoryEvidence::Partial,
            layout: LayoutEvidence::Safe,
            ..ready_fps_evidence()
        };
        let decision = evaluate(FeatureId::MetricsDiscovery, &partial_metrics);
        assert_eq!(decision.status, FeatureStatus::Unsupported);
        assert_eq!(decision.reason, CapabilityReason::PartialCoverage);
    }

    #[test]
    fn probe_translation_distinguishes_unknown_builds_from_unreadable_executables() {
        let mut report = ProbeReport::empty(crate::probe::ProbeStatus::UnknownBuild, None);
        report.process_status = crate::probe::ProcessStatus::Verified;
        report.executable_status = crate::probe::ExecutableStatus::UnknownBuild;
        report.build_resolution = Some(crate::identity::BuildResolution::UnknownBuild);
        let evidence = CapabilityEvidence::from_probe(&report);
        assert_eq!(evidence.process, ProcessEvidence::Verified);
        assert_eq!(evidence.build, BuildEvidence::Unknown);
        assert_eq!(evidence.executable_readable, Some(true));

        report.executable_status = crate::probe::ExecutableStatus::AccessDenied;
        assert_eq!(
            CapabilityEvidence::from_probe(&report).executable_readable,
            Some(false)
        );
    }

    #[test]
    fn os_apis_do_not_claim_unimplemented_inputs() {
        let windows = CapabilityEvidence {
            platform: Platform::Windows,
            ..CapabilityEvidence::default()
        };
        for feature in [
            FeatureId::GlobalHotkeys,
            FeatureId::KeyboardInput,
            FeatureId::Clipboard,
        ] {
            let decision = evaluate(feature, &windows);
            assert_eq!(decision.status, FeatureStatus::Unsupported);
            assert_eq!(decision.reason, CapabilityReason::NotImplemented);
            assert!(!decision.is_authorized());
        }
    }

    #[test]
    fn offline_inspection_and_export_require_their_inputs_to_be_evaluated() {
        let evidence = CapabilityEvidence {
            executable_readable: Some(true),
            report_collected: Some(true),
            destination_writable: Some(true),
            ..CapabilityEvidence::default()
        };
        assert_eq!(
            evaluate(FeatureId::ExecutableInspection, &evidence).status,
            FeatureStatus::Supported
        );
        assert_eq!(
            evaluate(FeatureId::SupportExport, &evidence).status,
            FeatureStatus::Supported
        );
        assert_eq!(
            evaluate(FeatureId::SupportExport, &CapabilityEvidence::default()).status,
            FeatureStatus::Unknown
        );
        let collection_failed = CapabilityEvidence {
            report_collected: Some(false),
            destination_writable: Some(true),
            ..CapabilityEvidence::default()
        };
        let decision = evaluate(FeatureId::SupportExport, &collection_failed);
        assert_eq!(decision.status, FeatureStatus::Unsupported);
        assert_eq!(decision.reason, CapabilityReason::CollectionFailed);

        let destination_unwritable = CapabilityEvidence {
            report_collected: Some(true),
            destination_writable: Some(false),
            ..CapabilityEvidence::default()
        };
        let decision = evaluate(FeatureId::SupportExport, &destination_unwritable);
        assert_eq!(decision.status, FeatureStatus::Unsupported);
        assert_eq!(decision.reason, CapabilityReason::AccessDenied);
    }
}
