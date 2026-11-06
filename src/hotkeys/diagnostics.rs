use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy)]
#[repr(usize)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) enum Stage {
    RawInputObserved,
    InputForwarded,
    ForwardFailed,
    RawInputReadFailed,
    QueueDiscarded,
    StaleDropped,
    InputProcessed,
    ChordCandidate,
    Accepted,
    Rejected,
    IdentityRejected,
    FocusChanged,
    Activated,
    Deactivated,
}

impl Stage {
    const COUNT: usize = 14;
}

pub(super) struct HotkeyDiagnostics {
    counters: [AtomicU64; Stage::COUNT],
}

impl Default for HotkeyDiagnostics {
    fn default() -> Self {
        Self {
            counters: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl HotkeyDiagnostics {
    pub(super) fn increment(&self, stage: Stage) {
        self.increment_by(stage, 1);
    }

    pub(super) fn increment_by(&self, stage: Stage, amount: u64) {
        let counter = &self.counters[stage as usize];
        let mut value = counter.load(Ordering::Relaxed);
        while value < u64::MAX {
            match counter.compare_exchange_weak(
                value,
                value.saturating_add(amount),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => value = observed,
            }
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn count(&self, stage: Stage) -> u64 {
        self.counters[stage as usize].load(Ordering::Relaxed)
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub(super) fn summary(&self) -> String {
        format!(
            "HOTKEY_DIAGNOSTICS raw_observed={} forwarded={} forward_failed={} read_failed={} queue_discarded={} stale_dropped={} processed={} chord_candidates={} accepted={} rejected={} identity_rejected={} focus_changes={} activations={} deactivations={}",
            self.count(Stage::RawInputObserved),
            self.count(Stage::InputForwarded),
            self.count(Stage::ForwardFailed),
            self.count(Stage::RawInputReadFailed),
            self.count(Stage::QueueDiscarded),
            self.count(Stage::StaleDropped),
            self.count(Stage::InputProcessed),
            self.count(Stage::ChordCandidate),
            self.count(Stage::Accepted),
            self.count(Stage::Rejected),
            self.count(Stage::IdentityRejected),
            self.count(Stage::FocusChanged),
            self.count(Stage::Activated),
            self.count(Stage::Deactivated),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{HotkeyDiagnostics, Stage};

    #[test]
    fn diagnostic_stage_counters_are_independent_and_saturating() {
        let diagnostics = HotkeyDiagnostics::default();
        diagnostics.increment(Stage::RawInputObserved);
        diagnostics.increment(Stage::Accepted);
        diagnostics.counters[Stage::Accepted as usize]
            .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
        diagnostics.increment(Stage::Accepted);

        assert_eq!(diagnostics.count(Stage::RawInputObserved), 1);
        assert_eq!(diagnostics.count(Stage::Accepted), u64::MAX);
        assert_eq!(diagnostics.count(Stage::Rejected), 0);
    }

    #[test]
    fn summary_reports_only_bounded_stage_counts() {
        let summary = HotkeyDiagnostics::default().summary();

        for stage in [
            "raw_observed=",
            "forwarded=",
            "forward_failed=",
            "read_failed=",
            "queue_discarded=",
            "stale_dropped=",
            "processed=",
            "chord_candidates=",
            "accepted=",
            "rejected=",
            "identity_rejected=",
            "focus_changes=",
            "activations=",
            "deactivations=",
        ] {
            assert!(summary.contains(stage), "missing stage {stage}");
        }
        assert!(!summary.contains("0x"));
    }
}
