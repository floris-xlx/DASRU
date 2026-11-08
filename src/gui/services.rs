use super::{controller::MonitorService, model::Observation};
use crate::{
    probe::{ProbeReport, ProbeStatus},
    process::{ProcessIdentity, ProcessInspection},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};

pub trait ProbeBackend: Send + 'static {
    type Key: Clone + PartialEq + Send;
    fn key(&mut self) -> Option<Self::Key>;
    fn probe(&mut self, cancel: &AtomicBool) -> ProbeReport;
    fn report_key(&mut self, report: &ProbeReport) -> Option<Self::Key>;
}

pub struct ProbeService<B: ProbeBackend> {
    backend: B,
    cache: Option<(B::Key, Observation, Instant)>,
}

impl<B: ProbeBackend> ProbeService<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            cache: None,
        }
    }
}

impl<B: ProbeBackend> MonitorService for ProbeService<B> {
    fn observe(&mut self, cancel: &AtomicBool) -> Result<Observation, &'static str> {
        self.observe_with_progress(cancel, &mut |_| {})
    }
    fn observe_with_progress(
        &mut self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(super::model::Connection),
    ) -> Result<Observation, &'static str> {
        if cancel.load(Ordering::Acquire) {
            return Err("WORKER_CANCELLED");
        }
        let before = self.backend.key();
        if let Some((key, observation, verified)) = &self.cache
            && before.as_ref() == Some(key)
            && verified.elapsed() < Duration::from_secs(30)
        {
            return Ok(observation.clone());
        }
        self.cache = None;
        progress(if before.is_some() {
            super::model::Connection::Verifying
        } else {
            super::model::Connection::Detecting
        });
        let report = self.backend.probe(cancel);
        if cancel.load(Ordering::Acquire) {
            return Err("WORKER_CANCELLED");
        }
        let mut observation = Observation::from_probe(&report);
        let report_key = self.backend.report_key(&report);
        let after = self.backend.key();
        if before.is_some() && (before != after || report_key != after) {
            return Ok(Observation::new(ProbeStatus::IdentityInconsistent));
        }
        if let Some(key) = after.filter(|key| report_key.as_ref() == Some(key)) {
            // shortcut: metadata is not proof against hostile rewrites restoring timestamps; rehash every 30s and never authorize from this cache.
            if matches!(
                observation.status,
                ProbeStatus::ExactMatch | ProbeStatus::UnknownBuild
            ) {
                self.cache = Some((key, observation.clone(), Instant::now()));
            }
        }
        if observation.errors.is_empty() && observation.status == ProbeStatus::ProbeError {
            observation.errors.push("PROBE_FAILED");
        }
        Ok(observation)
    }
}

#[derive(Clone, PartialEq)]
pub struct ProcessFileKey {
    pid: u32,
    identity: ProcessIdentity,
    size: u64,
    modified: SystemTime,
    created: SystemTime,
}

impl ProcessFileKey {
    fn from_candidate(candidate: &crate::process::ProcessCandidate) -> Option<Self> {
        let ProcessInspection::Available { identity } = &candidate.inspection else {
            return None;
        };
        let metadata = std::fs::metadata(&identity.executable_path).ok()?;
        Some(Self {
            pid: candidate.pid,
            identity: identity.clone(),
            size: metadata.len(),
            modified: metadata.modified().ok()?,
            created: metadata.created().ok()?,
        })
    }
}

pub struct NativeProbe;
impl ProbeBackend for NativeProbe {
    type Key = ProcessFileKey;
    fn key(&mut self) -> Option<Self::Key> {
        let candidates = crate::process::enumerate_pinned_game_processes().ok()?;
        let [pinned] = candidates.as_slice() else {
            return None;
        };
        if crate::process::recheck_pinned_process(pinned) != pinned.candidate.inspection {
            return None;
        }
        ProcessFileKey::from_candidate(&pinned.candidate)
    }
    fn probe(&mut self, cancel: &AtomicBool) -> ProbeReport {
        crate::probe::run_probe_cancellable(cancel)
    }
    fn report_key(&mut self, report: &ProbeReport) -> Option<Self::Key> {
        let [candidate] = report.processes.as_slice() else {
            return None;
        };
        ProcessFileKey::from_candidate(candidate)
    }
}
