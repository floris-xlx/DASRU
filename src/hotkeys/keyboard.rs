use std::collections::HashSet;

pub const MOD_ALT: u8 = 0x01;
pub const MOD_CONTROL: u8 = 0x02;
pub const MOD_SHIFT: u8 = 0x04;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub virtual_key: u16,
    pub is_down: bool,
    pub modifiers: u8,
}

#[derive(Debug, Default)]
pub struct KeyboardEventTracker {
    pressed: HashSet<u16>,
}

impl KeyboardEventTracker {
    pub fn observe(&mut self, virtual_key: u16, is_down: bool) -> Option<KeyboardEvent> {
        let changed = if is_down {
            self.pressed.insert(virtual_key)
        } else {
            self.pressed.remove(&virtual_key)
        };
        if !changed {
            return None;
        }

        Some(KeyboardEvent {
            virtual_key,
            is_down,
            modifiers: self.modifiers(),
        })
    }

    pub fn modifiers(&self) -> u8 {
        let mut modifiers = 0;
        if self.any_pressed(&[0x10, 0xa0, 0xa1]) {
            modifiers |= MOD_SHIFT;
        }
        if self.any_pressed(&[0x11, 0xa2, 0xa3]) {
            modifiers |= MOD_CONTROL;
        }
        if self.any_pressed(&[0x12, 0xa4, 0xa5]) {
            modifiers |= MOD_ALT;
        }
        modifiers
    }

    pub fn focus_lost(&mut self) {
        self.pressed.clear();
    }

    fn any_pressed(&self, keys: &[u16]) -> bool {
        keys.iter().any(|key| self.pressed.contains(key))
    }
}
