use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use dasru::{
    identity::{BuildRegistry, BuildResolution},
    probe::{ExecutableStatus, ProbeStatus, SteamStatus, inspect_executable_with_registry},
};

fn temporary_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "dasru-offline-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn inspect_json_reports_offline_identity_without_a_process() {
    let root = temporary_root();
    fs::create_dir_all(&root).unwrap();
    let executable = root.join("DOOMTheDarkAges.exe");
    fs::write(&executable, b"abc").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["inspect", "--exe"])
        .arg(&executable)
        .arg("--json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert!(report["observed_at_unix_ms"].as_u64().is_some());
    assert_eq!(report["inspection_mode"], "OFFLINE");
    assert_eq!(report["process_status"], "NOT_EVALUATED");
    assert_eq!(report["executable_status"], "UNKNOWN_BUILD");
    assert_eq!(report["status"], "UNKNOWN_BUILD");
    assert_eq!(report["run_integrity"], "NOT_EVALUATED");
    assert_eq!(
        report["executable"]["sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(report["processes"], serde_json::json!([]));
    assert!(
        report["processes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("pid").is_none())
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_offline_inspection_returns_a_nonzero_code_and_structured_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["inspect", "--exe"])
        .arg(temporary_root().join("missing.exe"))
        .arg("--json")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["process_status"], "NOT_EVALUATED");
    assert_eq!(report["executable_status"], "IDENTITY_INCONSISTENT");
    assert_eq!(report["errors"][0]["field"], "executable");
}

#[test]
fn invalid_cli_usage_returns_exit_code_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .arg("inspect")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
}

#[cfg(not(windows))]
#[test]
fn memory_layout_cli_reports_unsupported_platform_without_a_process() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["memory", "inspect", "--metrics", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["scan"]["status"], "UNSUPPORTED_PLATFORM");
    assert_eq!(report["dry_run"]["status"], "UNSUPPORTED");
}

#[cfg(not(windows))]
#[test]
fn memory_layout_cli_accepts_a_parameterized_fps_label() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args([
            "memory",
            "inspect",
            "--metrics",
            "--fps-label",
            "%i FPS+",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dry_run"]["replacement"], "%i FPS+");
    assert_eq!(report["dry_run"]["status"], "UNSUPPORTED");
}

#[test]
fn memory_layout_cli_rejects_labels_that_change_the_fps_format() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["memory", "inspect", "--metrics", "--fps-label", "FPS+é"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ASCII label beginning with %i FPS"));
}

#[test]
fn steam_build_mismatch_does_not_downgrade_executable_identity() {
    const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let root = temporary_root();
    let install = root.join("steamapps/common/DOOMTheDarkAges");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("DOOMTheDarkAges.exe");
    fs::write(&executable, b"abc").unwrap();
    fs::write(
        root.join("steamapps/appmanifest_3017860.acf"),
        r#""AppState" { "appid" "3017860" "installdir" "DOOMTheDarkAges" "buildid" "99999" }"#,
    )
    .unwrap();
    let registry = BuildRegistry::from_toml(&format!(
        "[[builds]]\nid='known'\nname='Known'\nsha256='{HASH}'\nsteam_build_id='12345'\n"
    ))
    .unwrap();

    let report = inspect_executable_with_registry(&executable, &registry);

    assert_eq!(report.status, ProbeStatus::ExactMatch);
    assert_eq!(report.executable_status, ExecutableStatus::ExactMatch);
    assert_eq!(report.steam_status, SteamStatus::Mismatch);
    assert_eq!(report.steam_build_id.as_deref(), Some("99999"));
    assert_eq!(report.expected_steam_build_id.as_deref(), Some("12345"));
    assert!(matches!(
        report.build_resolution,
        Some(BuildResolution::ExactMatch { .. })
    ));
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.errors[0].field, "steam.build_id");

    fs::write(
        root.join("steamapps/appmanifest_3017860.acf"),
        r#""AppState" { "appid" "3017860" "installdir" "OtherGame" "buildid" "99999" }"#,
    )
    .unwrap();
    let install_mismatch = inspect_executable_with_registry(&executable, &registry);
    assert_eq!(install_mismatch.status, ProbeStatus::ExactMatch);
    assert_eq!(install_mismatch.steam_status, SteamStatus::Mismatch);
    assert_eq!(
        install_mismatch.steam_install_dir.as_deref(),
        Some("OtherGame")
    );
    assert_eq!(install_mismatch.steam_build_id.as_deref(), Some("99999"));
    assert_eq!(install_mismatch.errors[0].field, "steam.install_dir");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unreadable_optional_steam_metadata_is_reported_separately_from_executable_identity() {
    const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let root = temporary_root();
    let install = root.join("steamapps/common/DOOMTheDarkAges");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("DOOMTheDarkAges.exe");
    fs::write(&executable, b"abc").unwrap();
    fs::write(
        root.join("steamapps/appmanifest_3017860.acf"),
        "corrupted manifest",
    )
    .unwrap();
    let registry = BuildRegistry::from_toml(&format!(
        "[[builds]]\nid='known'\nname='Known'\nsha256='{HASH}'\n"
    ))
    .unwrap();

    let report = inspect_executable_with_registry(&executable, &registry);

    assert_eq!(report.status, ProbeStatus::ExactMatch);
    assert_eq!(report.steam_status, SteamStatus::ReadError);
    assert_eq!(
        report.run_integrity,
        dasru::probe::RunIntegrityStatus::NotEvaluated
    );
    assert_eq!(report.errors[0].field, "steam.manifest");
    assert_eq!(report.exit_code(), 0);

    fs::remove_dir_all(root).unwrap();
}
