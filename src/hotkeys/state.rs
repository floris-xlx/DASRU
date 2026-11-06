use super::{HotkeyErrorCode, manager::HotkeyState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    VerifiedInactive,
    BeginRegistration,
    RegistrationSucceeded,
    RegistrationConflict,
    RegistrationFailed,
    FocusLost,
    ProcessInvalidated,
    OperationFailed,
    BeginShutdown,
    ShutdownComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyLifecycle {
    state: HotkeyState,
}

impl Default for HotkeyLifecycle {
    fn default() -> Self {
        Self {
            state: HotkeyState::Disabled,
        }
    }
}

impl HotkeyLifecycle {
    pub fn state(self) -> HotkeyState {
        self.state
    }

    pub fn transition(&mut self, event: LifecycleEvent) -> Result<HotkeyState, HotkeyErrorCode> {
        use HotkeyState as State;
        let next = match (self.state, event) {
            (State::Disabled, LifecycleEvent::VerifiedInactive) => State::Inactive,
            (State::Inactive, LifecycleEvent::VerifiedInactive) => State::Inactive,
            (
                State::Disabled | State::Inactive | State::Conflict,
                LifecycleEvent::BeginRegistration,
            ) => State::Registering,
            (State::Registering, LifecycleEvent::RegistrationSucceeded) => State::Active,
            (State::Registering, LifecycleEvent::RegistrationConflict) => State::Conflict,
            (State::Registering, LifecycleEvent::RegistrationFailed) => State::Error,
            (State::Active | State::Registering | State::Conflict, LifecycleEvent::FocusLost) => {
                State::Inactive
            }
            (
                State::Disabled
                | State::Inactive
                | State::Registering
                | State::Active
                | State::Conflict
                | State::Error,
                LifecycleEvent::ProcessInvalidated,
            ) => State::Disabled,
            (
                State::Inactive | State::Registering | State::Active | State::Conflict,
                LifecycleEvent::OperationFailed,
            ) => State::Error,
            (
                State::Disabled
                | State::Inactive
                | State::Registering
                | State::Active
                | State::Conflict
                | State::Error,
                LifecycleEvent::BeginShutdown,
            ) => State::Stopping,
            (State::Stopping, LifecycleEvent::BeginShutdown) => State::Stopping,
            (State::Stopping, LifecycleEvent::ShutdownComplete) => State::Disabled,
            _ => return Err(HotkeyErrorCode::InvalidTransition),
        };
        self.state = next;
        Ok(next)
    }
}
