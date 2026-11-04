#![cfg(windows)]

use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dasru::{
    identity::BuildRegistry,
    memory::{MetricsScanStatus, scan_metrics_with_registry},
    probe::{ProbeStatus, run_probe_with_registry},
    process::ProcessInspection,
};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        let path = path.to_string_lossy().replace('/', "\\");
        path.strip_prefix("\\\\?\\")
            .or_else(|| path.strip_prefix("\\??\\"))
            .unwrap_or(&path)
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

fn wait_for_start(child: &mut Child) {
    for _ in 0..50 {
        assert!(
            child.try_wait().unwrap().is_none(),
            "simulator exited early"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn complete_probe_discovers_simulated_game_and_reports_ambiguity() {
    let root = std::env::temp_dir().join(format!(
        "dasru-windows-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("simulated_game.rs");
    let executable = root.join("DOOMTheDarkAges.exe");
    fs::write(&source, include_str!("fixtures/simulated_game.rs")).unwrap();
    let compilation = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("rustc is available to the Windows test job");
    assert!(
        compilation.status.success(),
        "simulator compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );

    let registry = BuildRegistry::from_toml("builds = []").unwrap();
    let first = Command::new(&executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("simulator starts");
    let mut first = ChildGuard(first);
    wait_for_start(&mut first.0);

    let single = run_probe_with_registry(&registry);
    assert_eq!(single.status, ProbeStatus::UnknownBuild);
    assert_eq!(single.process_status, dasru::probe::ProcessStatus::Verified);
    assert_eq!(single.processes.len(), 1);
    assert_eq!(single.processes[0].pid, first.0.id());
    let ProcessInspection::Available { identity } = &single.processes[0].inspection else {
        panic!("simulated game process identity is available");
    };
    let observed_path = fs::canonicalize(&identity.executable_path)
        .expect("the process-reported executable path resolves");
    let expected_path = fs::canonicalize(&executable).unwrap();
    assert!(
        same_path(&observed_path, &expected_path),
        "process path {:?} differs from expected {:?}",
        observed_path,
        expected_path
    );
    assert!(identity.creation_time_filetime > 0);
    assert!(identity.module_base > 0);
    assert!(identity.image_size > 0);

    let memory_scan = scan_metrics_with_registry(&registry);
    assert_eq!(memory_scan.status, MetricsScanStatus::FoundUniqueMatch);
    assert_eq!(memory_scan.probe.processes[0].pid, first.0.id());
    assert_eq!(memory_scan.matches.len(), 1);
    let found = &memory_scan.matches[0];
    assert!(found.address >= identity.module_base);
    assert!(found.address < identity.module_base + u64::from(identity.image_size));
    assert!(found.protection_name.starts_with("PAGE_"));

    let cli_scan = Command::new(env!("CARGO_BIN_EXE_dasru"))
        .args(["memory", "scan", "--metrics", "--json"])
        .output()
        .expect("DASRU memory scan command starts");
    assert!(
        cli_scan.status.success(),
        "memory scan failed: {}",
        String::from_utf8_lossy(&cli_scan.stderr)
    );
    let cli_report: serde_json::Value = serde_json::from_slice(&cli_scan.stdout).unwrap();
    assert_eq!(cli_report["status"], "FOUND_UNIQUE_MATCH");
    assert_eq!(cli_report["matches"].as_array().unwrap().len(), 1);

    let second = Command::new(&executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("second simulator starts");
    let mut second = ChildGuard(second);
    wait_for_start(&mut second.0);
    let ambiguous = run_probe_with_registry(&registry);
    assert_eq!(ambiguous.status, ProbeStatus::AmbiguousProcess);
    assert_eq!(ambiguous.processes.len(), 2);
    assert!(
        ambiguous
            .processes
            .iter()
            .any(|candidate| candidate.pid == first.0.id())
    );
    assert!(
        ambiguous
            .processes
            .iter()
            .any(|candidate| candidate.pid == second.0.id())
    );

    second.0.kill().unwrap();
    second.0.wait().unwrap();
    let after_exit = run_probe_with_registry(&registry);
    assert_eq!(after_exit.status, ProbeStatus::UnknownBuild);
    assert_eq!(after_exit.processes.len(), 1);
    assert_eq!(after_exit.processes[0].pid, first.0.id());

    first.0.kill().unwrap();
    first.0.wait().unwrap();
    let absent = run_probe_with_registry(&registry);
    assert_eq!(absent.status, ProbeStatus::NoProcess);
    assert_eq!(absent.process_status, dasru::probe::ProcessStatus::NotFound);

    drop(second);
    drop(first);
    fs::remove_dir_all(root).unwrap();
}
