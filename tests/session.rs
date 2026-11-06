use std::path::PathBuf;

use dasru::{
    capability::{
        self, Architecture, BuildEvidence, CapabilityEvidence, CapabilityReason, FeatureId,
        FeatureStatus, LayoutEvidence, MemoryEvidence, Platform, ProcessEvidence,
    },
    process::ProcessIdentity,
    session::{SessionCoordinator, SessionEvent, SessionLifecycle, SessionState},
};

fn identity() -> ProcessIdentity {
    ProcessIdentity {
        creation_time_filetime: 1234,
        executable_path: PathBuf::from("C:/Games/DOOMTheDarkAges.exe"),
        module_base: 0x1000,
        image_size: 0x10000,
    }
}

fn ready_write_evidence() -> CapabilityEvidence {
    CapabilityEvidence {
        platform: Platform::Windows,
        architecture: Architecture::X86_64,
        process: ProcessEvidence::Verified,
        build: BuildEvidence::ExactMatch,
        memory: MemoryEvidence::Readable,
        layout: LayoutEvidence::Safe,
        experimental_opt_in: true,
        ..CapabilityEvidence::default()
    }
}

#[test]
fn absent_ambiguous_denied_and_changed_processes_cannot_enable_memory_features() {
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
            ..ready_write_evidence()
        };
        let decision = capability::evaluate(FeatureId::MemoryScan, &evidence);
        assert_eq!(decision.status, FeatureStatus::Unsupported);
        assert_eq!(decision.reason, reason);
        assert!(!decision.is_authorized());
    }
}

#[test]
fn unknown_build_layout_and_missing_opt_in_fail_closed() {
    for build in [BuildEvidence::Unknown, BuildEvidence::Inconsistent] {
        let evidence = CapabilityEvidence {
            build,
            ..ready_write_evidence()
        };
        let decision = capability::evaluate(FeatureId::ExperimentalFpsMarker, &evidence);
        assert_eq!(decision.reason, CapabilityReason::UnknownBuild);
        assert!(!decision.is_authorized());
    }

    let evidence = CapabilityEvidence {
        layout: LayoutEvidence::Unsupported,
        ..ready_write_evidence()
    };
    assert_eq!(
        capability::evaluate(FeatureId::ExperimentalFpsMarker, &evidence).reason,
        CapabilityReason::UnsupportedLayout
    );

    let evidence = CapabilityEvidence {
        experimental_opt_in: false,
        ..ready_write_evidence()
    };
    let decision = capability::evaluate(FeatureId::ExperimentalFpsMarker, &evidence);
    assert_eq!(decision.status, FeatureStatus::Experimental);
    assert_eq!(decision.reason, CapabilityReason::ExplicitOptInRequired);
    assert!(!decision.is_authorized());
}

#[test]
fn identity_change_and_failed_restore_keep_the_lifecycle_degraded() {
    let original = identity();
    let mut lifecycle = SessionLifecycle::default();
    lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
    lifecycle
        .transition(SessionEvent::AttachmentVerified(original.clone()))
        .unwrap();
    lifecycle.transition(SessionEvent::IdentityChanged).unwrap();
    lifecycle.transition(SessionEvent::BeginStopping).unwrap();
    lifecycle
        .transition(SessionEvent::RestorationFailed)
        .unwrap();
    assert_eq!(
        lifecycle.state(),
        &SessionState::Degraded {
            identity: original,
            recovery_retained: true,
        }
    );
}

#[test]
fn process_exit_clears_recovery_and_patch_reservation_is_recoverable() {
    let identity = identity();
    let coordinator = SessionCoordinator::default();
    drop(coordinator.reserve_patch(&identity, 0x2006).unwrap());
    assert!(coordinator.reserve_patch(&identity, 0x2006).is_err());
    let recovery = coordinator.resume_recovery(&identity, 0x2006).unwrap();

    let mut lifecycle = SessionLifecycle::begin_recovery(identity.clone());
    lifecycle.transition(SessionEvent::ProcessExited).unwrap();
    assert_eq!(lifecycle.state(), &SessionState::Detached);
    recovery.release();
}

#[test]
fn attached_process_exit_passes_through_stopping() {
    let mut lifecycle = SessionLifecycle::default();
    lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
    lifecycle
        .transition(SessionEvent::AttachmentVerified(identity()))
        .unwrap();
    lifecycle.transition(SessionEvent::BeginStopping).unwrap();
    lifecycle.transition(SessionEvent::ProcessExited).unwrap();
    assert_eq!(lifecycle.state(), &SessionState::Detached);
}
