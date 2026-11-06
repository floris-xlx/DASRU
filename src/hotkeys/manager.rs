use crate::process::ProcessIdentity;

use super::{
    binding::{BindingError, HotkeyBinding, validate_bindings},
    keyboard::KeyboardEventTracker,
    state::{HotkeyLifecycle, LifecycleEvent},
};

pub use super::HotkeyErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyState {
    Disabled,
    Inactive,
    Registering,
    Active,
    Conflict,
    Error,
    Stopping,
}

impl HotkeyState {
    pub const fn registration_evidence(self) -> crate::capability::HotkeyRegistrationEvidence {
        use crate::capability::HotkeyRegistrationEvidence as Registration;

        match self {
            Self::Active => Registration::Active,
            Self::Conflict => Registration::Conflict,
            Self::Error => Registration::Failed,
            Self::Disabled | Self::Inactive | Self::Registering | Self::Stopping => {
                Registration::Inactive
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGameProcess {
    pub pid: u32,
    pub identity: ProcessIdentity,
    pub build_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationFailure {
    Conflict,
    Failed,
    RawInputRegistrationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationOperation {
    Register(u32, HotkeyBinding),
    Unregister(u32),
}

pub trait RegistrationBackend {
    fn register(&mut self, id: u32, binding: HotkeyBinding) -> Result<(), RegistrationFailure>;

    fn unregister(&mut self, id: u32) -> Result<(), RegistrationFailure>;
}

pub trait ForegroundBackend {
    fn verify_game(
        &mut self,
        expected_pid: u32,
        expected_identity: &ProcessIdentity,
    ) -> Result<(), HotkeyErrorCode>;

    fn foreground_pid(&mut self) -> Result<Option<u32>, HotkeyErrorCode>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyFailure {
    pub code: HotkeyErrorCode,
    pub state: HotkeyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Accepted(HotkeyBinding),
    Rejected {
        binding: HotkeyBinding,
        code: HotkeyErrorCode,
    },
}

pub struct HotkeyManager<R, F> {
    bindings: Vec<HotkeyBinding>,
    game: Option<VerifiedGameProcess>,
    registrar: R,
    foreground: F,
    lifecycle: HotkeyLifecycle,
    keyboard: KeyboardEventTracker,
    registered: Vec<u32>,
    enabled: bool,
    terminal: bool,
}

impl<R: RegistrationBackend, F: ForegroundBackend> HotkeyManager<R, F> {
    pub fn new(
        bindings: Vec<HotkeyBinding>,
        game: VerifiedGameProcess,
        registrar: R,
        foreground: F,
    ) -> Result<Self, BindingError> {
        validate_bindings(&bindings)?;
        Ok(Self {
            bindings,
            game: Some(game),
            registrar,
            foreground,
            lifecycle: HotkeyLifecycle::default(),
            keyboard: KeyboardEventTracker::default(),
            registered: Vec::new(),
            enabled: true,
            terminal: false,
        })
    }

    pub fn new_disabled(registrar: R, foreground: F) -> Self {
        Self {
            bindings: Vec::new(),
            game: None,
            registrar,
            foreground,
            lifecycle: HotkeyLifecycle::default(),
            keyboard: KeyboardEventTracker::default(),
            registered: Vec::new(),
            enabled: false,
            terminal: false,
        }
    }

    pub fn state(&self) -> HotkeyState {
        self.lifecycle.state()
    }

    pub fn start(&mut self) -> Result<(), HotkeyFailure> {
        if !self.enabled {
            return Ok(());
        }
        if self.terminal {
            return Err(self.failure(HotkeyErrorCode::Disabled));
        }
        let Some(game) = &self.game else {
            self.terminal = true;
            return Err(self.failure(HotkeyErrorCode::NoProcess));
        };
        if game.build_id.is_empty() {
            self.terminal = true;
            return Err(self.failure(HotkeyErrorCode::UnknownBuild));
        }
        if let Err(code) = self.foreground.verify_game(game.pid, &game.identity) {
            self.terminal = true;
            return Err(self.failure(code));
        }
        match self.foreground.foreground_pid() {
            Ok(Some(pid)) if pid == game.pid => self.register_all(),
            Ok(_) => {
                self.lifecycle
                    .transition(LifecycleEvent::VerifiedInactive)
                    .map_err(|code| self.failure(code))?;
                Ok(())
            }
            Err(code) => {
                self.terminal = true;
                Err(self.failure(code))
            }
        }
    }

    pub fn reconcile(&mut self) -> Result<(), HotkeyFailure> {
        if !self.enabled {
            return Ok(());
        }
        if self.terminal {
            return Err(self.failure(HotkeyErrorCode::Disabled));
        }
        let Some(game) = &self.game else {
            self.invalidate();
            return Err(self.failure(HotkeyErrorCode::NoProcess));
        };
        if let Err(code) = self.foreground.verify_game(game.pid, &game.identity) {
            self.invalidate();
            return Err(self.failure(code));
        }
        let foreground_pid = match self.foreground.foreground_pid() {
            Ok(foreground_pid) => foreground_pid,
            Err(code) => {
                self.fail_runtime();
                return Err(self.failure(code));
            }
        };
        if foreground_pid == Some(game.pid) {
            match self.state() {
                HotkeyState::Inactive => self.register_all(),
                HotkeyState::Conflict => Ok(()),
                HotkeyState::Active => Ok(()),
                HotkeyState::Disabled => Err(self.failure(HotkeyErrorCode::Disabled)),
                HotkeyState::Error | HotkeyState::Stopping => {
                    Err(self.failure(HotkeyErrorCode::NotActive))
                }
                HotkeyState::Registering => Ok(()),
            }
        } else {
            self.deactivate()
        }
    }

    pub fn handle_hotkey(&mut self, id: u32) -> Option<HotkeyEvent> {
        let binding = *self.bindings.get(id.checked_sub(1)? as usize)?;
        if self.state() != HotkeyState::Active {
            return Some(HotkeyEvent::Rejected {
                binding,
                code: HotkeyErrorCode::NotActive,
            });
        }
        let Some(game) = &self.game else {
            self.invalidate();
            return Some(HotkeyEvent::Rejected {
                binding,
                code: HotkeyErrorCode::NoProcess,
            });
        };
        if let Err(code) = self.foreground.verify_game(game.pid, &game.identity) {
            self.invalidate();
            return Some(HotkeyEvent::Rejected { binding, code });
        }
        match self.foreground.foreground_pid() {
            Ok(Some(pid)) if pid == game.pid => Some(HotkeyEvent::Accepted(binding)),
            Ok(Some(_)) => {
                let _ = self.deactivate();
                Some(HotkeyEvent::Rejected {
                    binding,
                    code: HotkeyErrorCode::ForegroundMismatch,
                })
            }
            Ok(None) | Err(HotkeyErrorCode::ForegroundUnavailable) => {
                let _ = self.deactivate();
                Some(HotkeyEvent::Rejected {
                    binding,
                    code: HotkeyErrorCode::ForegroundUnavailable,
                })
            }
            Err(code) => {
                self.fail_runtime();
                Some(HotkeyEvent::Rejected { binding, code })
            }
        }
    }

    pub fn handle_keyboard_input(
        &mut self,
        virtual_key: u16,
        is_down: bool,
    ) -> Option<HotkeyEvent> {
        let event = self.keyboard.observe(virtual_key, is_down)?;
        if !event.is_down {
            return None;
        }
        let id = self
            .bindings
            .iter()
            .position(|binding| {
                binding.virtual_key() as u16 == event.virtual_key
                    && binding.modifiers() as u8 == event.modifiers
            })
            .map(|index| index as u32 + 1)?;
        self.handle_hotkey(id)
    }

    pub fn shutdown(&mut self) -> Result<(), HotkeyFailure> {
        self.keyboard.focus_lost();
        if self.state() == HotkeyState::Disabled {
            return Ok(());
        }
        self.lifecycle
            .transition(LifecycleEvent::BeginShutdown)
            .map_err(|code| self.failure(code))?;
        let unregister_failed = self.unregister_all().is_err();
        self.lifecycle
            .transition(LifecycleEvent::ShutdownComplete)
            .map_err(|code| self.failure(code))?;
        self.terminal = true;
        if unregister_failed {
            Err(self.failure(HotkeyErrorCode::ShutdownFailed))
        } else {
            Ok(())
        }
    }

    fn register_all(&mut self) -> Result<(), HotkeyFailure> {
        self.keyboard.focus_lost();
        self.lifecycle
            .transition(LifecycleEvent::BeginRegistration)
            .map_err(|code| self.failure(code))?;
        for (index, binding) in self.bindings.iter().copied().enumerate() {
            let id = index as u32 + 1;
            match self.registrar.register(id, binding) {
                Ok(()) => self.registered.push(id),
                Err(failure) => {
                    let rollback_failed = self.unregister_all().is_err();
                    let (event, code) = if rollback_failed {
                        (
                            LifecycleEvent::RegistrationFailed,
                            HotkeyErrorCode::RegistrationFailed,
                        )
                    } else {
                        match failure {
                            RegistrationFailure::Conflict => (
                                LifecycleEvent::RegistrationConflict,
                                HotkeyErrorCode::RegistrationConflict,
                            ),
                            RegistrationFailure::Failed => (
                                LifecycleEvent::RegistrationFailed,
                                HotkeyErrorCode::RegistrationFailed,
                            ),
                            RegistrationFailure::RawInputRegistrationFailed => (
                                LifecycleEvent::RegistrationFailed,
                                HotkeyErrorCode::RawInputRegistrationFailed,
                            ),
                        }
                    };
                    self.lifecycle.transition(event).ok();
                    if event == LifecycleEvent::RegistrationFailed {
                        self.terminal = true;
                    }
                    return Err(self.failure(code));
                }
            }
        }
        self.lifecycle
            .transition(LifecycleEvent::RegistrationSucceeded)
            .map_err(|code| self.failure(code))?;
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), HotkeyFailure> {
        self.keyboard.focus_lost();
        let unregister_failed = self.unregister_all().is_err();
        if unregister_failed {
            self.fail_runtime();
            return Err(self.failure(HotkeyErrorCode::RegistrationFailed));
        }
        match self.state() {
            HotkeyState::Active | HotkeyState::Registering | HotkeyState::Conflict => {
                self.lifecycle
                    .transition(LifecycleEvent::FocusLost)
                    .map_err(|code| self.failure(code))?;
            }
            HotkeyState::Inactive => {}
            _ => return Err(self.failure(HotkeyErrorCode::InvalidTransition)),
        }
        Ok(())
    }

    fn invalidate(&mut self) {
        self.keyboard.focus_lost();
        let _ = self.unregister_all();
        self.lifecycle
            .transition(LifecycleEvent::ProcessInvalidated)
            .ok();
        self.terminal = true;
    }

    fn fail_runtime(&mut self) {
        self.keyboard.focus_lost();
        let _ = self.unregister_all();
        self.lifecycle
            .transition(LifecycleEvent::OperationFailed)
            .ok();
        self.terminal = true;
    }

    fn unregister_all(&mut self) -> Result<(), HotkeyFailure> {
        let mut failed = false;
        for id in self.registered.drain(..).rev() {
            if self.registrar.unregister(id).is_err() {
                failed = true;
            }
        }
        if failed {
            Err(self.failure(HotkeyErrorCode::RegistrationFailed))
        } else {
            Ok(())
        }
    }

    fn failure(&self, code: HotkeyErrorCode) -> HotkeyFailure {
        HotkeyFailure {
            code,
            state: self.state(),
        }
    }
}
