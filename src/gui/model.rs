use std::time::{Duration, Instant};

use crate::{
    capability::{CapabilityDecision, CapabilityEvidence, FeatureId},
    identity::BuildResolution,
    probe::{ExecutableStatus, ProbeReport, ProbeStatus, ProcessStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    NotRunning,
    Detecting,
    Verifying,
    Verified,
    UnknownBuild,
    Ambiguous,
    AccessDenied,
    ReverificationRequired,
    Error,
    Stale,
    Unsupported,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub status: ProbeStatus,
    pub process: ProcessStatus,
    pub build: Option<String>,
    pub evidence: CapabilityEvidence,
    pub errors: Vec<&'static str>,
}

impl Observation {
    /// Construct a projection of an authoritative probe status (also useful to service fakes).
    pub fn new(status: ProbeStatus) -> Self {
        Self {
            status,
            process: ProcessStatus::NotEvaluated,
            build: None,
            evidence: CapabilityEvidence::default(),
            errors: Vec::new(),
        }
    }

    pub fn from_probe(report: &ProbeReport) -> Self {
        let mut observation = Self::new(report.status);
        observation.process = report.process_status;
        observation.evidence = CapabilityEvidence::from_probe(report);
        observation.build = match &report.build_resolution {
            Some(BuildResolution::ExactMatch { build }) => Some(build.id.clone()),
            _ => None,
        };
        // A presentation must never turn partial or contradictory evidence into verification.
        if observation.status == ProbeStatus::ExactMatch
            && (observation.process != ProcessStatus::Verified
                || observation.build.is_none()
                || report.executable_status != ExecutableStatus::ExactMatch)
        {
            observation.status = ProbeStatus::IdentityInconsistent;
        }
        observation.errors = report.errors.iter().map(|error| error.code).collect();
        observation
    }

    pub fn capability(&self, feature: FeatureId) -> CapabilityDecision {
        crate::capability::evaluate(feature, &self.evidence)
    }
}

#[derive(Default, Clone)]
pub struct Model {
    generation: u64,
    refreshing: bool,
    stopped: bool,
    failed: bool,
    phase: Option<Connection>,
    pub observation: Option<Observation>,
    pub updated: Option<Instant>,
    pub last_error: Option<&'static str>,
    pub stale_after: Duration,
}

impl Model {
    pub fn with_stale_after(stale_after: Duration) -> Self {
        Self {
            stale_after,
            ..Self::default()
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn progress(&mut self, generation: u64, phase: Connection) {
        if !self.stopped
            && generation == self.generation
            && matches!(phase, Connection::Detecting | Connection::Verifying)
        {
            self.phase = Some(phase);
            self.observation = None;
        }
    }
    pub fn begin_refresh(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.refreshing = true;
        self.generation
    }

    pub fn apply(
        &mut self,
        generation: u64,
        result: Result<Observation, &'static str>,
        now: Instant,
    ) {
        if self.stopped || generation != self.generation {
            return;
        }
        self.refreshing = false;
        self.phase = None;
        match result {
            Ok(observation) => {
                self.failed = false;
                self.last_error = observation.errors.first().copied();
                self.observation = Some(observation);
                self.updated = Some(now);
            }
            Err(code) => {
                self.failed = true;
                self.last_error = Some(code);
            }
        }
    }

    pub fn connection(&self, now: Instant) -> Connection {
        if self.stopped {
            return Connection::Stopped;
        }
        if self.failed {
            return Connection::Error;
        }
        if let Some(phase) = self.phase {
            return phase;
        }
        if self.updated.is_some_and(|updated| {
            now.saturating_duration_since(updated) > self.stale_after.max(Duration::from_secs(3))
        }) {
            return Connection::Stale;
        }
        if self.refreshing && self.observation.is_none() {
            return Connection::Detecting;
        }
        match self
            .observation
            .as_ref()
            .map(|observation| observation.status)
        {
            None | Some(ProbeStatus::NoProcess) => Connection::NotRunning,
            Some(ProbeStatus::ExactMatch) => Connection::Verified,
            Some(ProbeStatus::UnknownBuild) => Connection::UnknownBuild,
            Some(ProbeStatus::AmbiguousProcess) => Connection::Ambiguous,
            Some(ProbeStatus::AccessDenied) => Connection::AccessDenied,
            Some(ProbeStatus::IdentityInconsistent) => Connection::ReverificationRequired,
            Some(ProbeStatus::UnsupportedPlatform) => Connection::Unsupported,
            Some(ProbeStatus::ProbeError) => Connection::Error,
        }
    }

    pub fn shutdown(&mut self) {
        self.stopped = true;
        self.generation = self.generation.saturating_add(1);
    }
}
