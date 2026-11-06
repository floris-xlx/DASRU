use dasru::hotkeys::keyboard::{KeyboardEvent, KeyboardEventTracker, MOD_ALT, MOD_CONTROL};

const VK_F9: u16 = 0x78;
const VK_LCONTROL: u16 = 0xa2;
const VK_RCONTROL: u16 = 0xa3;
const VK_LMENU: u16 = 0xa4;

#[test]
fn reports_keyboard_make_and_break_events() {
    let mut tracker = KeyboardEventTracker::default();

    assert_eq!(
        tracker.observe(VK_F9, true),
        Some(KeyboardEvent {
            virtual_key: VK_F9,
            is_down: true,
            modifiers: 0,
        })
    );
    assert_eq!(
        tracker.observe(VK_F9, false),
        Some(KeyboardEvent {
            virtual_key: VK_F9,
            is_down: false,
            modifiers: 0,
        })
    );
}

#[test]
fn tracks_modifier_state_across_left_and_right_keys() {
    let mut tracker = KeyboardEventTracker::default();

    assert_eq!(
        tracker.observe(VK_LCONTROL, true).unwrap().modifiers,
        MOD_CONTROL
    );
    assert_eq!(
        tracker.observe(VK_RCONTROL, true).unwrap().modifiers,
        MOD_CONTROL
    );
    assert_eq!(
        tracker.observe(VK_LMENU, true).unwrap().modifiers,
        MOD_CONTROL | MOD_ALT
    );
    assert_eq!(
        tracker.observe(VK_LCONTROL, false).unwrap().modifiers,
        MOD_CONTROL | MOD_ALT
    );
    assert_eq!(
        tracker.observe(VK_RCONTROL, false).unwrap().modifiers,
        MOD_ALT
    );
    assert_eq!(tracker.observe(VK_LMENU, false).unwrap().modifiers, 0);
}

#[test]
fn suppresses_auto_repeat_and_unmatched_break_events() {
    let mut tracker = KeyboardEventTracker::default();

    assert!(tracker.observe(VK_F9, true).is_some());
    assert_eq!(tracker.observe(VK_F9, true), None);
    assert!(tracker.observe(VK_F9, false).is_some());
    assert_eq!(tracker.observe(VK_F9, false), None);
}

#[test]
fn focus_loss_clears_pressed_keys_and_modifiers() {
    let mut tracker = KeyboardEventTracker::default();
    tracker.observe(VK_LCONTROL, true);
    tracker.observe(VK_LMENU, true);
    tracker.observe(VK_F9, true);

    tracker.focus_lost();

    assert_eq!(tracker.modifiers(), 0);
    assert_eq!(tracker.observe(VK_F9, false), None);
    assert_eq!(
        tracker.observe(VK_LCONTROL, true).unwrap().modifiers,
        MOD_CONTROL
    );
}
