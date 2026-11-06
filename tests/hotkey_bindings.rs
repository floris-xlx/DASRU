use std::process::{Command, Output};

use dasru::hotkeys::binding::HotkeyBinding;

fn listen(bindings: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dasru"));
    command.args(["hotkeys", "listen"]);
    for binding in bindings {
        command.args(["--bind", binding]);
    }
    command.output().expect("DASRU hotkey CLI starts")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn accepts_valid_modifier_shortcut() {
    let binding = HotkeyBinding::parse("CTRL+ALT+F9").unwrap();

    assert_eq!(binding.to_string(), "CTRL+ALT+F9");
}

#[test]
fn rejects_duplicate_bindings_regardless_of_modifier_order() {
    let output = listen(&["CTRL+ALT+F9", "ALT+CTRL+F9"]);

    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    assert!(
        output_text(&output).contains("HOTKEY_DUPLICATE_BINDING"),
        "{}",
        output_text(&output)
    );
}

#[test]
fn rejects_unmodified_gameplay_keys() {
    let output = listen(&["F9"]);

    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    assert!(
        output_text(&output).contains("HOTKEY_UNMODIFIED_KEY"),
        "{}",
        output_text(&output)
    );
}

#[test]
fn rejects_reserved_windows_shortcuts() {
    let output = listen(&["WIN+L"]);

    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    assert!(
        output_text(&output).contains("HOTKEY_RESERVED_BINDING"),
        "{}",
        output_text(&output)
    );
}

#[test]
fn rejects_reserved_f12() {
    let output = listen(&["CTRL+ALT+F12"]);

    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    assert!(
        output_text(&output).contains("HOTKEY_RESERVED_BINDING"),
        "{}",
        output_text(&output)
    );
}

#[test]
fn rejects_invalid_virtual_key() {
    let output = listen(&["CTRL+ALT+F25"]);

    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    assert!(
        output_text(&output).contains("HOTKEY_INVALID_VIRTUAL_KEY"),
        "{}",
        output_text(&output)
    );
}

#[test]
fn rejects_duplicate_modifiers() {
    let output = listen(&["CTRL+CTRL+ALT+F9"]);

    assert_eq!(output.status.code(), Some(2), "{}", output_text(&output));
    assert!(
        output_text(&output).contains("HOTKEY_INVALID_BINDING"),
        "{}",
        output_text(&output)
    );
}

#[test]
fn rejects_known_reserved_shortcuts() {
    for binding in [
        "ALT+TAB",
        "ALT+F4",
        "CTRL+ESC",
        "CTRL+ALT+DELETE",
        "CTRL+SHIFT+ESC",
    ] {
        let output = listen(&[binding]);

        assert_eq!(
            output.status.code(),
            Some(2),
            "binding={binding}: {}",
            output_text(&output)
        );
        assert!(
            output_text(&output).contains("HOTKEY_RESERVED_BINDING"),
            "binding={binding}: {}",
            output_text(&output)
        );
    }
}

#[test]
fn normalizes_equivalent_bindings() {
    let reversed = HotkeyBinding::parse("ALT+CTRL+F9").unwrap();
    let canonical = HotkeyBinding::parse("CTRL+ALT+F9").unwrap();

    assert_eq!(reversed, canonical);
    assert_eq!(reversed.to_string(), "CTRL+ALT+F9");
}

#[test]
fn hotkeys_command_does_not_start_without_a_verified_process() {
    let output = listen(&["CTRL+ALT+F9"]);
    let text = output_text(&output);

    assert_eq!(output.status.code(), Some(1), "{text}");
    if cfg!(windows) {
        assert!(text.contains("HOTKEY_NO_PROCESS"), "{text}");
    } else {
        assert!(text.contains("HOTKEY_UNSUPPORTED_PLATFORM"), "{text}");
    }
}
