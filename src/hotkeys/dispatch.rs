use super::{
    binding::{HotkeyAction, HotkeyBinding},
    config::{HotkeyConfig, HotkeyDefinition},
    manager::HotkeyEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionResolutionError {
    UnknownAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredBinding {
    pub id: String,
    pub binding: HotkeyBinding,
    pub action: HotkeyAction,
}

impl ConfiguredBinding {
    pub fn from_config(config: &HotkeyConfig) -> Result<Vec<Self>, ActionResolutionError> {
        config
            .bindings()
            .iter()
            .map(Self::from_definition)
            .collect()
    }

    pub fn from_definition(definition: &HotkeyDefinition) -> Result<Self, ActionResolutionError> {
        let action = match definition.action_name.as_str() {
            "observe" => HotkeyAction::Observe,
            _ => return Err(ActionResolutionError::UnknownAction),
        };
        Ok(Self {
            id: definition.id.clone(),
            binding: definition.binding,
            action,
        })
    }

    pub fn find(bindings: &[Self], shortcut: HotkeyBinding) -> Option<&Self> {
        bindings.iter().find(|binding| binding.binding == shortcut)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionEvent {
    pub id: String,
    pub action: HotkeyAction,
    pub binding: HotkeyBinding,
}

pub trait ActionEventSink {
    fn emit(&mut self, event: ActionEvent);
}

pub struct ActionDispatcher<S> {
    bindings: Vec<ConfiguredBinding>,
    sink: S,
    stopped: bool,
}

impl<S: ActionEventSink> ActionDispatcher<S> {
    pub fn new(bindings: Vec<ConfiguredBinding>, sink: S) -> Self {
        Self {
            bindings,
            sink,
            stopped: false,
        }
    }

    pub fn dispatch(
        &mut self,
        event: HotkeyEvent,
        captured_generation: u64,
        current_generation: u64,
    ) -> bool {
        if self.stopped || captured_generation != current_generation {
            return false;
        }
        let HotkeyEvent::Accepted(binding) = event else {
            return false;
        };
        let Some(configured) = ConfiguredBinding::find(&self.bindings, binding) else {
            return false;
        };
        self.sink.emit(ActionEvent {
            id: configured.id.clone(),
            action: configured.action,
            binding,
        });
        true
    }

    pub fn shutdown(&mut self) {
        self.stopped = true;
        self.bindings.clear();
    }
}
