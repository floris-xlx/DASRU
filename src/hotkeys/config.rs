use std::{collections::HashSet, fs::File, io::Read, path::Path};

use serde::Deserialize;

use super::binding::{BindingError, HotkeyBinding};

pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
pub const MAX_ENABLED_BINDINGS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyDefinition {
    pub id: String,
    pub binding: HotkeyBinding,
    pub action_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyConfig {
    bindings: Vec<HotkeyDefinition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyConfigError {
    ReadFailed,
    TooLarge,
    Invalid,
    UnsupportedVersion,
    DuplicateId,
    DuplicateShortcut,
    UnknownAction,
    Empty,
    TooManyBindings,
    Binding(BindingError),
}

impl HotkeyConfigError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ReadFailed => "HOTKEY_CONFIG_READ_FAILED",
            Self::TooLarge => "HOTKEY_CONFIG_TOO_LARGE",
            Self::Invalid => "HOTKEY_CONFIG_INVALID",
            Self::UnsupportedVersion => "HOTKEY_CONFIG_UNSUPPORTED_VERSION",
            Self::DuplicateId => "HOTKEY_CONFIG_DUPLICATE_ID",
            Self::DuplicateShortcut => "HOTKEY_CONFIG_DUPLICATE_SHORTCUT",
            Self::UnknownAction => "HOTKEY_CONFIG_UNKNOWN_ACTION",
            Self::Empty => "HOTKEY_CONFIG_EMPTY",
            Self::TooManyBindings => "HOTKEY_CONFIG_TOO_MANY_BINDINGS",
            Self::Binding(error) => error.code(),
        }
    }
}

impl std::fmt::Display for HotkeyConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for HotkeyConfigError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    schema_version: u32,
    bindings: Vec<RawBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    id: String,
    shortcut: String,
    action: String,
    enabled: bool,
}

impl HotkeyConfig {
    pub fn parse(source: &str) -> Result<Self, HotkeyConfigError> {
        if source.len() > MAX_CONFIG_BYTES {
            return Err(HotkeyConfigError::TooLarge);
        }
        let raw: RawConfig = toml::from_str(source).map_err(|_| HotkeyConfigError::Invalid)?;
        if raw.schema_version != 1 {
            return Err(HotkeyConfigError::UnsupportedVersion);
        }

        let mut ids = HashSet::with_capacity(raw.bindings.len());
        let mut shortcuts = HashSet::new();
        let mut bindings = Vec::new();
        for raw_binding in raw.bindings {
            if raw_binding.id.is_empty() || !ids.insert(raw_binding.id.clone()) {
                return Err(HotkeyConfigError::DuplicateId);
            }
            if raw_binding.action != "observe" {
                return Err(HotkeyConfigError::UnknownAction);
            }
            let binding =
                HotkeyBinding::parse(&raw_binding.shortcut).map_err(HotkeyConfigError::Binding)?;
            if raw_binding.enabled {
                if bindings.len() == MAX_ENABLED_BINDINGS {
                    return Err(HotkeyConfigError::TooManyBindings);
                }
                if !shortcuts.insert(binding) {
                    return Err(HotkeyConfigError::DuplicateShortcut);
                }
                bindings.push(HotkeyDefinition {
                    id: raw_binding.id,
                    binding,
                    action_name: raw_binding.action,
                });
            }
        }
        if bindings.is_empty() {
            return Err(HotkeyConfigError::Empty);
        }
        Ok(Self { bindings })
    }

    pub fn load(path: &Path) -> Result<Self, HotkeyConfigError> {
        let file = File::open(path).map_err(|_| HotkeyConfigError::ReadFailed)?;
        let mut source = String::new();
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut source)
            .map_err(|_| HotkeyConfigError::Invalid)?;
        Self::parse(&source)
    }

    pub fn bindings(&self) -> &[HotkeyDefinition] {
        &self.bindings
    }
}
