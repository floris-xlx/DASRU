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
    pub(crate) fn observe_at_generation(
        &mut self,
        virtual_key: u16,
        is_down: bool,
        captured_generation: u64,
        current_generation: u64,
    ) -> Option<KeyboardEvent> {
        if captured_generation != current_generation {
            self.focus_lost();
            return None;
        }
        self.observe(virtual_key, is_down)
    }

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

#[cfg(test)]
mod tests {
    use super::{KeyboardEventTracker, MOD_ALT, MOD_CONTROL};

    fn chord_candidate(
        tracker: &mut KeyboardEventTracker,
        key: u16,
        is_down: bool,
        captured_generation: u64,
        current_generation: u64,
    ) -> bool {
        tracker
            .observe_at_generation(key, is_down, captured_generation, current_generation)
            .is_some_and(|event| {
                event.is_down
                    && event.virtual_key == 0x78
                    && event.modifiers == (MOD_CONTROL | MOD_ALT)
            })
    }

    #[test]
    fn replay_accepts_one_current_chord_and_discards_queued_old_generation() {
        let mut tracker = KeyboardEventTracker::default();
        let chord = [(0xa2, true), (0xa4, true), (0x78, true), (0x78, true)];
        let accepted = chord
            .iter()
            .filter(|(key, down)| chord_candidate(&mut tracker, *key, *down, 0, 0))
            .count();
        assert_eq!(accepted, 1);

        let queued_before_focus_loss = [(0xa2, true), (0xa4, true), (0x78, true)];
        let accepted_after_refocus = queued_before_focus_loss
            .iter()
            .filter(|(key, down)| chord_candidate(&mut tracker, *key, *down, 0, 2))
            .count();
        assert_eq!(accepted_after_refocus, 0);
        assert!(!chord_candidate(&mut tracker, 0x78, true, 2, 2));
    }
}
