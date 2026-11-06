use crate::process::ProcessIdentity;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SessionState {
    #[default]
    Detached,
    Attaching,
    Attached {
        identity: ProcessIdentity,
    },
    Degraded {
        identity: ProcessIdentity,
        recovery_retained: bool,
    },
    Stopping {
        identity: ProcessIdentity,
        recovery_retained: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    BeginAttachment,
    AttachmentVerified(ProcessIdentity),
    AttachmentFailed,
    IdentityChanged,
    BeginStopping,
    RestorationVerified,
    RestorationFailed,
    ProcessExited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub state: SessionState,
    pub event: SessionEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionLifecycle {
    state: SessionState,
}

impl SessionLifecycle {
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn transition(&mut self, event: SessionEvent) -> Result<(), TransitionError> {
        use SessionEvent as Event;
        use SessionState as State;

        let next = match (&self.state, &event) {
            (State::Detached, Event::BeginAttachment) => State::Attaching,
            (State::Attaching, Event::AttachmentVerified(identity)) => State::Attached {
                identity: identity.clone(),
            },
            (State::Attaching, Event::AttachmentFailed) => State::Detached,
            (State::Attaching, Event::IdentityChanged) => State::Detached,
            (State::Attached { identity }, Event::IdentityChanged) => State::Degraded {
                identity: identity.clone(),
                recovery_retained: true,
            },
            (State::Attached { identity }, Event::BeginStopping)
            | (State::Degraded { identity, .. }, Event::BeginStopping) => State::Stopping {
                identity: identity.clone(),
                recovery_retained: true,
            },
            (State::Stopping { identity, .. }, Event::BeginStopping) => State::Stopping {
                identity: identity.clone(),
                recovery_retained: true,
            },
            (State::Stopping { .. }, Event::RestorationVerified | Event::ProcessExited)
            | (State::Degraded { .. }, Event::ProcessExited) => State::Detached,
            (State::Stopping { identity, .. }, Event::RestorationFailed)
            | (State::Degraded { identity, .. }, Event::RestorationFailed) => State::Degraded {
                identity: identity.clone(),
                recovery_retained: true,
            },
            (State::Detached, Event::BeginStopping | Event::ProcessExited) => State::Detached,
            (State::Detached, Event::RestorationVerified) => State::Detached,
            _ => {
                return Err(TransitionError {
                    state: self.state.clone(),
                    event,
                });
            }
        };
        self.state = next;
        Ok(())
    }

    pub fn begin_recovery(identity: ProcessIdentity) -> Self {
        Self {
            state: SessionState::Degraded {
                identity,
                recovery_retained: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{SessionEvent, SessionLifecycle, SessionState};
    use crate::process::ProcessIdentity;

    fn identity() -> ProcessIdentity {
        ProcessIdentity {
            creation_time_filetime: 1234,
            executable_path: PathBuf::from("C:/Games/DOOMTheDarkAges.exe"),
            module_base: 0x1000,
            image_size: 0x10000,
        }
    }

    #[test]
    fn attachment_and_verified_stop_follow_the_lifecycle() {
        let identity = identity();
        let mut lifecycle = SessionLifecycle::default();
        lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
        lifecycle
            .transition(SessionEvent::AttachmentVerified(identity.clone()))
            .unwrap();
        lifecycle.transition(SessionEvent::BeginStopping).unwrap();
        lifecycle
            .transition(SessionEvent::RestorationVerified)
            .unwrap();
        assert_eq!(*lifecycle.state(), SessionState::Detached);
    }

    #[test]
    fn failed_attachment_returns_to_detached_and_invalid_events_are_typed() {
        let mut lifecycle = SessionLifecycle::default();
        let error = lifecycle
            .transition(SessionEvent::AttachmentVerified(identity()))
            .unwrap_err();
        assert_eq!(error.state, SessionState::Detached);
        lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
        lifecycle
            .transition(SessionEvent::AttachmentFailed)
            .unwrap();
        assert_eq!(*lifecycle.state(), SessionState::Detached);
    }

    #[test]
    fn identity_change_during_attachment_returns_to_detached() {
        let mut lifecycle = SessionLifecycle::default();
        lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
        lifecycle.transition(SessionEvent::IdentityChanged).unwrap();
        assert_eq!(*lifecycle.state(), SessionState::Detached);
    }

    #[test]
    fn identity_or_restore_failures_retain_recovery_information() {
        let identity = identity();
        let mut lifecycle = SessionLifecycle::default();
        lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
        lifecycle
            .transition(SessionEvent::AttachmentVerified(identity.clone()))
            .unwrap();
        lifecycle.transition(SessionEvent::IdentityChanged).unwrap();
        assert_eq!(
            *lifecycle.state(),
            SessionState::Degraded {
                identity: identity.clone(),
                recovery_retained: true,
            }
        );
        lifecycle.transition(SessionEvent::BeginStopping).unwrap();
        lifecycle
            .transition(SessionEvent::RestorationFailed)
            .unwrap();
        assert_eq!(
            *lifecycle.state(),
            SessionState::Degraded {
                identity,
                recovery_retained: true,
            }
        );
    }

    #[test]
    fn exited_process_discards_its_obsolete_recovery_state() {
        let identity = identity();
        let mut lifecycle = SessionLifecycle::begin_recovery(identity);
        lifecycle.transition(SessionEvent::ProcessExited).unwrap();
        assert_eq!(*lifecycle.state(), SessionState::Detached);
    }

    #[test]
    fn repeated_stop_and_restore_requests_are_safe() {
        let identity = identity();
        let mut lifecycle = SessionLifecycle::default();
        lifecycle.transition(SessionEvent::BeginAttachment).unwrap();
        lifecycle
            .transition(SessionEvent::AttachmentVerified(identity))
            .unwrap();
        lifecycle.transition(SessionEvent::BeginStopping).unwrap();
        lifecycle.transition(SessionEvent::BeginStopping).unwrap();
        lifecycle
            .transition(SessionEvent::RestorationVerified)
            .unwrap();
        lifecycle
            .transition(SessionEvent::RestorationVerified)
            .unwrap();
        assert_eq!(*lifecycle.state(), SessionState::Detached);
    }
}
