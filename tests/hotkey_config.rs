use dasru::hotkeys::config::HotkeyConfig;

fn binding(id: &str, shortcut: &str, action: &str, enabled: bool) -> String {
    format!(
        "[[bindings]]\nid = \"{id}\"\nshortcut = \"{shortcut}\"\naction = \"{action}\"\nenabled = {enabled}\n"
    )
}

fn config(bindings: &[String]) -> String {
    format!("schema_version = 1\n\n{}", bindings.join("\n"))
}

#[test]
fn parses_valid_versioned_config() {
    let source = config(&[
        binding("primary", "CTRL+ALT+F9", "observe", true),
        binding("secondary", "CTRL+ALT+F10", "observe", true),
    ]);

    let parsed = HotkeyConfig::parse(&source).unwrap();

    assert_eq!(parsed.bindings().len(), 2);
    assert_eq!(parsed.bindings()[0].id, "primary");
    assert_eq!(parsed.bindings()[1].id, "secondary");
}

#[test]
fn rejects_unknown_schema_version() {
    let source = config(&[binding("primary", "CTRL+ALT+F9", "observe", true)])
        .replace("schema_version = 1", "schema_version = 2");

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_duplicate_binding_ids_including_disabled_entries() {
    let source = config(&[
        binding("primary", "CTRL+ALT+F9", "observe", true),
        binding("primary", "CTRL+ALT+F10", "observe", false),
    ]);

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_duplicate_active_shortcuts_after_normalization() {
    let source = config(&[
        binding("primary", "CTRL+ALT+F9", "observe", true),
        binding("secondary", "ALT+CTRL+F9", "observe", true),
    ]);

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_unknown_actions() {
    let source = config(&[binding("primary", "CTRL+ALT+F9", "launch_game", true)]);

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_reserved_shortcuts() {
    let source = config(&[binding("primary", "CTRL+ALT+F12", "observe", true)]);

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_oversized_config() {
    let source = format!("schema_version = 1\n{}", " ".repeat(64 * 1024));

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn ignores_disabled_bindings() {
    let source = config(&[
        binding("primary", "CTRL+ALT+F9", "observe", true),
        binding("disabled", "CTRL+ALT+F10", "observe", false),
    ]);

    let parsed = HotkeyConfig::parse(&source).unwrap();

    assert_eq!(parsed.bindings().len(), 1);
    assert_eq!(parsed.bindings()[0].id, "primary");
}

#[test]
fn rejects_empty_active_configuration() {
    let source = config(&[binding("disabled", "CTRL+ALT+F9", "observe", false)]);

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_unknown_schema_fields() {
    let source = format!(
        "schema_version = 1\nextra = true\n\n{}",
        binding("primary", "CTRL+ALT+F9", "observe", true)
    );

    assert!(HotkeyConfig::parse(&source).is_err());
}

#[test]
fn rejects_more_than_sixteen_enabled_bindings() {
    let bindings = (0..17)
        .map(|index| {
            binding(
                &format!("binding-{index}"),
                &format!("CTRL+ALT+{}", char::from(b'A' + index)),
                "observe",
                true,
            )
        })
        .collect::<Vec<_>>();

    assert!(HotkeyConfig::parse(&config(&bindings)).is_err());
}
