use std::{fs, process::Command, time::SystemTime};

use serde_json::Value;

#[test]
fn support_export_writes_privacy_safe_compatibility_json() {
    let secret = "dasru-support-private-sentinel";
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "dasru-support-{}-{unique}.json",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["support", "export"])
        .arg(&path)
        .env("SteamAppId", secret)
        .env("STEAM_COMPAT_DATA_PATH", secret)
        .env("WINEPREFIX", secret)
        .output()
        .expect("support export command starts");

    assert!(
        output.status.success(),
        "support export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(&path).expect("support report was written");
    let text = String::from_utf8(bytes).expect("support JSON is UTF-8");
    assert!(!text.contains(secret));
    assert!(!text.contains(path.to_string_lossy().as_ref()));
    let report: Value = serde_json::from_str(&text).expect("support JSON is valid");
    assert_object_keys(
        &report,
        &[
            "errors",
            "executable_compatibility",
            "feature_readiness",
            "host",
            "host_capabilities",
            "observed_at_unix_ms",
            "runtime",
            "schema_version",
        ],
    );
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["runtime"]["steam"]["status"], "DETECTED");
    assert_object_keys(
        &report["executable_compatibility"],
        &[
            "candidate_count",
            "executable_sha256",
            "executable_status",
            "expected_steam_build_id",
            "process_status",
            "recognized_build_id",
            "run_integrity",
            "status",
            "steam_build_id",
            "steam_status",
        ],
    );
    assert_eq!(
        report["executable_compatibility"]["run_integrity"],
        "NOT_EVALUATED"
    );
    assert!(report["executable_compatibility"]["status"].is_string());
    assert!(report["executable_compatibility"]["process_status"].is_string());
    assert!(report["executable_compatibility"]["executable_status"].is_string());
    assert!(report["executable_compatibility"]["steam_status"].is_string());
    assert!(report["executable_compatibility"]["candidate_count"].is_u64());
    assert_eq!(
        report["host_capabilities"]["keyboard_input"]["accessible"],
        "UNKNOWN"
    );
    assert_eq!(
        report["host_capabilities"]["keyboard_input"]["implementation_status"],
        "NOT_IMPLEMENTED"
    );
    for error in report["errors"].as_array().expect("structured errors") {
        assert_object_keys(error, &["code", "component", "field"]);
    }
    let compatibility = report["executable_compatibility"]
        .as_object()
        .expect("compatibility object");
    assert!(!compatibility.contains_key("path"));
    assert!(!compatibility.contains_key("pid"));
    assert!(!compatibility.contains_key("processes"));
    assert!(!compatibility.contains_key("steam_install_dir"));
    fs::remove_file(path).expect("remove test report");
}

#[test]
fn support_export_write_failure_has_a_stable_code_without_echoing_path() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir()
        .join(format!(
            "dasru-support-missing-{}-{unique}",
            std::process::id()
        ))
        .join("support.json");
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["support", "export"])
        .arg(&path)
        .output()
        .expect("support export command starts");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("SUPPORT_EXPORT_WRITE_FAILED"));
    assert!(!stderr.contains(path.to_string_lossy().as_ref()));
}

#[test]
fn support_export_does_not_replace_an_existing_file() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "dasru-support-existing-{}-{unique}.json",
        std::process::id()
    ));
    let original = b"keep this file";
    fs::write(&path, original).expect("create existing output file");
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["support", "export"])
        .arg(&path)
        .output()
        .expect("support export command starts");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("SUPPORT_EXPORT_PATH_EXISTS"));
    assert_eq!(fs::read(&path).expect("existing file remains"), original);
    fs::remove_file(path).expect("remove test file");
}

#[test]
fn support_export_requires_an_output_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["support", "export"])
        .output()
        .expect("support export command starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
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
