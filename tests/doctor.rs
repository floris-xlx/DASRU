use std::process::{Command, Output};

use serde_json::Value;

fn run_doctor(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(arguments)
        .output()
        .expect("doctor command starts")
}

#[test]
fn doctor_json_reports_host_signals_separately_from_feature_implementation() {
    let output = run_doctor(&["doctor", "--json"]);
    assert!(
        output.status.success(),
        "doctor should return a report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid doctor JSON");

    assert_object_keys(
        &report,
        &[
            "errors",
            "feature_readiness",
            "host",
            "host_capabilities",
            "observed_at_unix_ms",
            "runtime",
            "schema_version",
        ],
    );
    assert_object_keys(
        &report["host"],
        &[
            "architecture",
            "distribution",
            "environment",
            "kernel_version",
            "os",
            "os_version",
            "session_type",
            "xwayland_available",
        ],
    );
    assert_object_keys(
        &report["host_capabilities"],
        &[
            "clipboard",
            "global_hotkeys",
            "keyboard_input",
            "process_memory_read",
            "wayland_global_shortcuts_portal",
        ],
    );
    assert_eq!(report["schema_version"], 1);
    assert!(report["observed_at_unix_ms"].as_u64().is_some());
    assert!(
        report["host"]["os"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        report["host"]["architecture"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(report["host"]["session_type"].as_str().is_some());
    assert!(report["host"]["environment"].as_str().is_some());
    assert!(
        report["host"]["xwayland_available"].is_boolean()
            || report["host"]["xwayland_available"].is_null()
    );
    if report["host"]["os"] == "linux" {
        assert!(report["host"]["xwayland_available"].is_boolean());
        assert_ne!(report["host"]["session_type"], "XWAYLAND");
    }
    if report["host"]["os"] == "linux" {
        assert_eq!(
            report["host_capabilities"]["process_memory_read"]["accessible"],
            "UNKNOWN"
        );
        assert_eq!(
            report["host_capabilities"]["process_memory_read"]["implementation_status"],
            "NOT_IMPLEMENTED"
        );
    }

    for runtime in ["steam", "proton", "wine"] {
        assert!(report["runtime"][runtime]["detected"].is_boolean());
        assert!(report["runtime"][runtime]["detected_by"].is_array());
        let mut keys = report["runtime"][runtime]
            .as_object()
            .expect("runtime signal object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["detected", "detected_by", "status"]);
        assert!(matches!(
            report["runtime"][runtime]["status"].as_str(),
            Some("DETECTED" | "NOT_DETECTED")
        ));
    }
    for capability in [
        "process_memory_read",
        "clipboard",
        "global_hotkeys",
        "keyboard_input",
        "wayland_global_shortcuts_portal",
    ] {
        assert_object_keys(
            &report["host_capabilities"][capability],
            &["accessible", "detected", "implementation_status", "reason"],
        );
        for field in ["detected", "accessible", "implementation_status"] {
            assert!(
                report["host_capabilities"][capability][field]
                    .as_str()
                    .is_some()
            );
        }
        if report["host_capabilities"][capability]["implementation_status"] == "NOT_IMPLEMENTED" {
            assert_ne!(
                report["host_capabilities"][capability]["accessible"],
                "VERIFIED"
            );
        }
    }
    let portal = &report["host_capabilities"]["wayland_global_shortcuts_portal"];
    assert!(portal["detected"].as_str().is_some());
    assert_ne!(portal["accessible"], "VERIFIED");
    assert_eq!(portal["implementation_status"], "NOT_IMPLEMENTED");

    let features = report["feature_readiness"]
        .as_array()
        .expect("feature readiness list");
    for feature in [
        "process_probe",
        "memory_scan",
        "native_osd_write",
        "clipboard",
        "global_hotkeys",
        "wayland_global_shortcuts_portal",
    ] {
        assert!(features.iter().any(|entry| entry["feature"] == feature));
    }
    for entry in features {
        assert_object_keys(
            entry,
            &[
                "accessible",
                "detected",
                "failure_reasons",
                "feature",
                "implementation_status",
                "readiness",
            ],
        );
    }
    assert_eq!(
        features
            .iter()
            .find(|entry| entry["feature"] == "clipboard")
            .unwrap()["implementation_status"],
        "NOT_IMPLEMENTED"
    );
    assert_eq!(
        features
            .iter()
            .find(|entry| entry["feature"] == "global_hotkeys")
            .unwrap()["implementation_status"],
        "NOT_IMPLEMENTED"
    );
    assert_eq!(
        features
            .iter()
            .find(|entry| entry["feature"] == "keyboard_input")
            .unwrap()["implementation_status"],
        "NOT_IMPLEMENTED"
    );
    for feature in ["clipboard", "global_hotkeys"] {
        let entry = features
            .iter()
            .find(|entry| entry["feature"] == feature)
            .unwrap();
        assert_ne!(entry["readiness"], "AVAILABLE");
        assert_ne!(entry["accessible"], "VERIFIED");
    }
    let process_probe = features
        .iter()
        .find(|entry| entry["feature"] == "process_probe")
        .unwrap();
    if process_probe["accessible"] == "UNKNOWN" {
        assert_ne!(process_probe["readiness"], "AVAILABLE");
    }
    if report["host"]["os"] == "linux" {
        assert_eq!(process_probe["accessible"], "UNKNOWN");
        assert_eq!(process_probe["implementation_status"], "NOT_IMPLEMENTED");
    }
    assert!(report["errors"].is_array());
}

#[test]
fn doctor_reports_environment_signals_without_serializing_their_values() {
    let secret = "dasru-doctor-sensitive-value-sentinel";
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["doctor", "--json"])
        .env("SteamAppId", secret)
        .env("STEAM_COMPAT_DATA_PATH", secret)
        .env("WINEPREFIX", secret)
        .output()
        .expect("doctor command starts");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains(secret));
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid doctor JSON");
    for (runtime, variable) in [
        ("steam", "SteamAppId"),
        ("proton", "STEAM_COMPAT_DATA_PATH"),
        ("wine", "WINEPREFIX"),
    ] {
        assert_eq!(report["runtime"][runtime]["status"], "DETECTED");
        assert!(
            report["runtime"][runtime]["detected_by"]
                .as_array()
                .unwrap()
                .contains(&Value::String(variable.to_owned()))
        );
    }
}

#[test]
fn doctor_human_output_is_a_support_report() {
    let output = run_doctor(&["doctor"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("human output is UTF-8");
    assert!(text.starts_with("DASRU Doctor"));
    assert!(text.contains("Host"));
    assert!(text.contains("Environment"));
    assert!(text.contains("Feature readiness"));
    assert!(text.contains("Keyboard input"));
    assert!(text.contains("Wayland GlobalShortcuts portal"));
}

#[test]
fn doctor_rejects_unknown_options_with_usage_exit_code() {
    let output = run_doctor(&["doctor", "--unknown"]);
    assert_eq!(output.status.code(), Some(2));
}

fn assert_object_keys(value: &Value, expected: &[&str]) {
    let mut actual = value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}
