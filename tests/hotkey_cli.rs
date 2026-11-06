use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct ConfigFile(PathBuf);

impl ConfigFile {
    fn write(source: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dasru-hotkeys-{unique}.toml"));
        fs::write(&path, source).unwrap();
        Self(path)
    }
}

impl Drop for ConfigFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["hotkeys"])
        .args(arguments)
        .output()
        .expect("DASRU CLI starts")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn top_level_usage_lists_configurable_hotkey_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .output()
        .expect("DASRU CLI starts");
    let output_text = text(&output);

    assert_eq!(output.status.code(), Some(2), "{output_text}");
    assert!(output_text.contains("hotkeys validate --config <path>"));
    assert!(output_text.contains("hotkeys listen --config <path>"));
}

const VALID_CONFIG: &str = r#"
schema_version = 1

[[bindings]]
id = "primary"
shortcut = "CTRL+ALT+F9"
action = "observe"
enabled = true

[[bindings]]
id = "secondary"
shortcut = "CTRL+ALT+F10"
action = "observe"
enabled = true
"#;

#[test]
fn validates_config_and_exits_cleanly() {
    let config = ConfigFile::write(VALID_CONFIG);
    let path = config.0.to_str().unwrap();
    let output = cli(&["validate", "--config", path]);

    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert!(text(&output).contains("HOTKEY_CONFIG_VALID bindings=2"));
}

#[test]
fn validate_rejects_unknown_action_before_startup() {
    let config =
        ConfigFile::write(&VALID_CONFIG.replace("action = \"observe\"", "action = \"launch\""));
    let output = cli(&["validate", "--config", config.0.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("HOTKEY_CONFIG_UNKNOWN_ACTION"));
}

#[test]
fn config_and_legacy_binding_are_ambiguous() {
    let config = ConfigFile::write(VALID_CONFIG);
    let output = cli(&[
        "listen",
        "--config",
        config.0.to_str().unwrap(),
        "--bind",
        "CTRL+ALT+F9",
    ]);

    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(text(&output).contains("HOTKEY_INVALID_ARGUMENTS"));
}

#[test]
fn configured_listener_validates_before_backend_startup() {
    let config = ConfigFile::write(VALID_CONFIG);
    let output = cli(&["listen", "--config", config.0.to_str().unwrap()]);
    let output_text = text(&output);

    assert_eq!(output.status.code(), Some(1), "{output_text}");
    if cfg!(windows) {
        assert!(output_text.contains("HOTKEY_NO_PROCESS"), "{output_text}");
    } else {
        assert!(
            output_text.contains("HOTKEY_UNSUPPORTED_PLATFORM"),
            "{output_text}"
        );
    }
}

#[test]
fn config_read_error_does_not_print_local_path() {
    let output = cli(&["validate", "--config", "private-path-sentinel.toml"]);
    let output_text = text(&output);

    assert_eq!(output.status.code(), Some(2), "{output_text}");
    assert!(output_text.contains("HOTKEY_CONFIG_READ_FAILED"));
    assert!(!output_text.contains("private-path-sentinel"));
}
