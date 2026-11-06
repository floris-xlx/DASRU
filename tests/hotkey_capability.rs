use dasru::capability::{
    self, Accessibility, Architecture, Authorization, BuildEvidence, CapabilityEvidence,
    CapabilityReason, FeatureId, FeatureStatus, HotkeyRegistrationEvidence, ImplementationStatus,
    Platform, ProcessEvidence,
};
use dasru::hotkeys::{HotkeyErrorCode, manager::HotkeyState};

#[test]
fn hotkeys_are_implemented_but_unprobed_without_a_live_registration() {
    let evidence = CapabilityEvidence {
        platform: Platform::Windows,
        architecture: Architecture::X86_64,
        ..CapabilityEvidence::default()
    };

    let decision = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);

    assert_eq!(decision.status, FeatureStatus::Unknown);
    assert_eq!(decision.reason, CapabilityReason::NotEvaluated);
    assert_eq!(decision.implementation, ImplementationStatus::Implemented);
    assert_eq!(decision.accessibility, Accessibility::Unknown);
    assert_eq!(decision.authorization, Authorization::NotRequired);
    assert_eq!(decision.diagnostic_readiness(), "NOT_PROBED");
}

#[test]
fn hotkeys_remain_unsupported_off_windows() {
    let evidence = CapabilityEvidence {
        platform: Platform::Linux,
        ..CapabilityEvidence::default()
    };

    let decision = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);

    assert_eq!(decision.status, FeatureStatus::Unsupported);
    assert_eq!(decision.reason, CapabilityReason::UnsupportedPlatform);
    assert_eq!(decision.implementation, ImplementationStatus::Implemented);
}

#[test]
fn keyboard_input_remains_unimplemented() {
    let evidence = CapabilityEvidence {
        platform: Platform::Windows,
        ..CapabilityEvidence::default()
    };

    let decision = capability::evaluate(FeatureId::KeyboardInput, &evidence);

    assert_eq!(
        decision.implementation,
        ImplementationStatus::NotImplemented
    );
    assert_eq!(decision.diagnostic_readiness(), "NOT_IMPLEMENTED");
}

#[test]
fn capability_is_supported_only_after_hotkey_registration_succeeds() {
    let evidence = CapabilityEvidence {
        platform: Platform::Windows,
        process: ProcessEvidence::Verified,
        build: BuildEvidence::ExactMatch,
        hotkey_registration: HotkeyRegistrationEvidence::Active,
        ..CapabilityEvidence::default()
    };

    let decision = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);

    assert_eq!(decision.status, FeatureStatus::Supported);
    assert_eq!(decision.reason, CapabilityReason::Available);
    assert_eq!(decision.implementation, ImplementationStatus::Implemented);
    assert!(decision.is_authorized());
}

#[test]
fn active_registration_without_verified_process_is_not_supported() {
    let evidence = CapabilityEvidence {
        platform: Platform::Windows,
        hotkey_registration: HotkeyRegistrationEvidence::Active,
        ..CapabilityEvidence::default()
    };

    let decision = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);

    assert_eq!(decision.status, FeatureStatus::Unknown);
    assert_eq!(decision.reason, CapabilityReason::NotEvaluated);
    assert!(!decision.is_authorized());
}

#[test]
fn active_registration_for_unknown_build_is_unsupported() {
    let evidence = CapabilityEvidence {
        platform: Platform::Windows,
        process: ProcessEvidence::Verified,
        build: BuildEvidence::Unknown,
        hotkey_registration: HotkeyRegistrationEvidence::Active,
        ..CapabilityEvidence::default()
    };

    let decision = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);

    assert_eq!(decision.status, FeatureStatus::Unsupported);
    assert_eq!(decision.reason, CapabilityReason::UnknownBuild);
    assert!(!decision.is_authorized());
}

#[test]
fn failed_process_evidence_blocks_hotkey_support() {
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
            platform: Platform::Windows,
            process,
            build: BuildEvidence::ExactMatch,
            hotkey_registration: HotkeyRegistrationEvidence::Active,
            ..CapabilityEvidence::default()
        };

        let decision = capability::evaluate(FeatureId::GlobalHotkeys, &evidence);

        assert_eq!(decision.status, FeatureStatus::Unsupported);
        assert_eq!(decision.reason, reason);
        assert!(!decision.is_authorized());
    }
}

#[test]
fn hotkey_capability_tracks_registration_lifecycle_state() {
    use dasru::capability::HotkeyRegistrationEvidence as Registration;

    assert_eq!(
        HotkeyState::Active.registration_evidence(),
        Registration::Active
    );
    assert_eq!(
        HotkeyState::Inactive.registration_evidence(),
        Registration::Inactive
    );
    assert_eq!(
        HotkeyState::Conflict.registration_evidence(),
        Registration::Conflict
    );
    assert_eq!(
        HotkeyState::Error.registration_evidence(),
        Registration::Failed
    );
    assert_eq!(
        HotkeyState::Disabled.registration_evidence(),
        Registration::Inactive
    );
    assert_eq!(HotkeyErrorCode::NoProcess.as_str(), "HOTKEY_NO_PROCESS");
}
