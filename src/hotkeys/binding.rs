use std::{collections::HashSet, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    Observe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyBinding {
    modifiers: u32,
    virtual_key: u32,
    action: HotkeyAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingError {
    InvalidBinding,
    UnmodifiedKey,
    ReservedBinding,
    InvalidVirtualKey,
    DuplicateBinding,
}

impl BindingError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidBinding => "HOTKEY_INVALID_BINDING",
            Self::UnmodifiedKey => "HOTKEY_UNMODIFIED_KEY",
            Self::ReservedBinding => "HOTKEY_RESERVED_BINDING",
            Self::InvalidVirtualKey => "HOTKEY_INVALID_VIRTUAL_KEY",
            Self::DuplicateBinding => "HOTKEY_DUPLICATE_BINDING",
        }
    }
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: invalid shortcut", self.code())
    }
}

impl std::error::Error for BindingError {}

impl HotkeyBinding {
    pub fn parse(value: &str) -> Result<Self, BindingError> {
        let mut modifiers = 0;
        let mut windows_modifier = false;
        let mut virtual_key = None;

        for token in value.split('+').map(str::trim).map(str::to_ascii_uppercase) {
            if token.is_empty() {
                return Err(BindingError::InvalidBinding);
            }
            let modifier = match token.as_str() {
                "CTRL" | "CONTROL" => Some(0x0002),
                "ALT" => Some(0x0001),
                "SHIFT" => Some(0x0004),
                "WIN" | "WINDOWS" => {
                    windows_modifier = true;
                    Some(0x0008)
                }
                _ => None,
            };
            if let Some(modifier) = modifier {
                if modifiers & modifier != 0 {
                    return Err(BindingError::InvalidBinding);
                }
                modifiers |= modifier;
                continue;
            }
            if virtual_key.is_some() {
                return Err(BindingError::InvalidBinding);
            }
            virtual_key = Some(parse_virtual_key(&token).ok_or(BindingError::InvalidVirtualKey)?);
        }

        let virtual_key = virtual_key.ok_or(BindingError::InvalidBinding)?;
        if windows_modifier {
            return Err(BindingError::ReservedBinding);
        }
        if modifiers == 0 {
            return Err(BindingError::UnmodifiedKey);
        }
        if is_reserved(modifiers, virtual_key) {
            return Err(BindingError::ReservedBinding);
        }
        Ok(Self {
            modifiers,
            virtual_key,
            action: HotkeyAction::Observe,
        })
    }

    pub const fn modifiers(self) -> u32 {
        self.modifiers
    }

    pub const fn virtual_key(self) -> u32 {
        self.virtual_key
    }

    pub const fn action(self) -> HotkeyAction {
        self.action
    }
}

impl fmt::Display for HotkeyBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (flag, name) in [(0x0002, "CTRL"), (0x0001, "ALT"), (0x0004, "SHIFT")] {
            if self.modifiers & flag != 0 {
                if !first {
                    formatter.write_str("+")?;
                }
                formatter.write_str(name)?;
                first = false;
            }
        }
        if !first {
            formatter.write_str("+")?;
        }
        formatter.write_str(key_name(self.virtual_key))
    }
}

pub fn validate_bindings(bindings: &[HotkeyBinding]) -> Result<(), BindingError> {
    let mut unique = HashSet::with_capacity(bindings.len());
    if bindings.iter().any(|binding| !unique.insert(*binding)) {
        Err(BindingError::DuplicateBinding)
    } else {
        Ok(())
    }
}

fn parse_virtual_key(token: &str) -> Option<u32> {
    if token.len() == 1 {
        let key = token.as_bytes()[0];
        if key.is_ascii_alphanumeric() {
            return Some(u32::from(key));
        }
    }
    if let Some(number) = token
        .strip_prefix('F')
        .and_then(|suffix| suffix.parse::<u32>().ok())
    {
        if (1..=24).contains(&number) {
            return Some(0x70 + number - 1);
        }
    }
    Some(match token {
        "BACKSPACE" => 0x08,
        "TAB" => 0x09,
        "ENTER" | "RETURN" => 0x0d,
        "ESC" | "ESCAPE" => 0x1b,
        "SPACE" => 0x20,
        "PAGEUP" | "PGUP" => 0x21,
        "PAGEDOWN" | "PGDN" => 0x22,
        "END" => 0x23,
        "HOME" => 0x24,
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "INSERT" | "INS" => 0x2d,
        "DELETE" | "DEL" => 0x2e,
        _ => return None,
    })
}

fn key_name(virtual_key: u32) -> &'static str {
    match virtual_key {
        0x08 => "BACKSPACE",
        0x09 => "TAB",
        0x0d => "ENTER",
        0x1b => "ESC",
        0x20 => "SPACE",
        0x21 => "PAGEUP",
        0x22 => "PAGEDOWN",
        0x23 => "END",
        0x24 => "HOME",
        0x25 => "LEFT",
        0x26 => "UP",
        0x27 => "RIGHT",
        0x28 => "DOWN",
        0x2d => "INSERT",
        0x2e => "DELETE",
        0x70..=0x87 => FKEYS[(virtual_key - 0x70) as usize],
        0x30..=0x39 => DIGITS[(virtual_key - 0x30) as usize],
        0x41..=0x5a => LETTERS[(virtual_key - 0x41) as usize],
        _ => "UNKNOWN",
    }
}

const FKEYS: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const LETTERS: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];

fn is_reserved(modifiers: u32, virtual_key: u32) -> bool {
    let control = modifiers & 0x0002 != 0;
    let alt = modifiers & 0x0001 != 0;
    let shift = modifiers & 0x0004 != 0;
    virtual_key == 0x7b
        || (alt && virtual_key == 0x09)
        || (alt && virtual_key == 0x73)
        || (control && virtual_key == 0x1b)
        || (control && shift && virtual_key == 0x1b)
        || (control && alt && virtual_key == 0x2e)
}
