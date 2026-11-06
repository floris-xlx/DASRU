#![cfg(feature = "desktop-ui")]
use dasru::{
    gui::model::{Connection, Model, Observation},
    probe::ProbeStatus,
};
use std::time::{Duration, Instant};

#[test]
fn starts_disconnected_and_only_exact_evidence_verifies() {
    let mut model = Model::default();
    assert_eq!(model.connection(Instant::now()), Connection::NotRunning);
    for (status, expected) in [
        (ProbeStatus::UnknownBuild, Connection::UnknownBuild),
        (ProbeStatus::AmbiguousProcess, Connection::Ambiguous),
        (ProbeStatus::AccessDenied, Connection::AccessDenied),
        (
            ProbeStatus::IdentityInconsistent,
            Connection::ReverificationRequired,
        ),
        (ProbeStatus::ProbeError, Connection::Error),
        (ProbeStatus::NoProcess, Connection::NotRunning),
        (ProbeStatus::ExactMatch, Connection::Verified),
    ] {
        let generation = model.begin_refresh();
        model.apply(generation, Ok(Observation::new(status)), Instant::now());
        assert_eq!(model.connection(Instant::now()), expected);
    }
}

#[test]
fn stale_results_cannot_replace_newer_state_and_failures_remain_visible() {
    let mut model = Model::default();
    let old = model.begin_refresh();
    let new = model.begin_refresh();
    let now = Instant::now();
    model.apply(new, Err("WORKER_FAILED"), now);
    model.apply(old, Ok(Observation::new(ProbeStatus::ExactMatch)), now);
    assert_eq!(model.connection(now), Connection::Error);
    assert_eq!(model.last_error, Some("WORKER_FAILED"));
    model.begin_refresh();
    assert_eq!(model.last_error, Some("WORKER_FAILED"));
}

#[test]
fn expired_verified_data_is_stale_and_shutdown_rejects_pending_results() {
    let mut model = Model::default();
    let now = Instant::now();
    let generation = model.begin_refresh();
    model.apply(
        generation,
        Ok(Observation::new(ProbeStatus::ExactMatch)),
        now,
    );
    assert_eq!(
        model.connection(now + Duration::from_secs(10)),
        Connection::Stale
    );
    model.shutdown();
    model.apply(
        generation,
        Ok(Observation::new(ProbeStatus::ExactMatch)),
        now,
    );
    assert_eq!(model.connection(now), Connection::Stopped);
}

#[test]
fn worker_error_never_keeps_verified_connection() {
    let mut model = Model::default();
    let generation = model.begin_refresh();
    model.apply(
        generation,
        Ok(Observation::new(ProbeStatus::ExactMatch)),
        Instant::now(),
    );
    let generation = model.begin_refresh();
    model.apply(generation, Err("SERVICE_UNAVAILABLE"), Instant::now());
    assert_eq!(model.connection(Instant::now()), Connection::Error);
}

#[test]
fn process_change_clears_old_verification_while_hashing() {
    let mut model = Model::default();
    let generation = model.begin_refresh();
    model.apply(
        generation,
        Ok(Observation::new(ProbeStatus::ExactMatch)),
        Instant::now(),
    );
    let next = model.begin_refresh();
    model.progress(next, Connection::Verifying);
    assert_eq!(model.connection(Instant::now()), Connection::Verifying);
    model.progress(generation, Connection::Verified);
    assert_eq!(model.connection(Instant::now()), Connection::Verifying);
}
