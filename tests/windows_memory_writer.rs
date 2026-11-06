#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dasru::{
    identity::{BuildRegistry, ExecutableIdentity},
    memory::inspect_metrics_layout_with_registry_and_replacement,
    osd::{apply_fps_marker_with_registry, restore_fps_marker_from},
};

const FPS_SLOT_ORIGINAL: [u8; 8] = *b"%i FPS\0\0";
const FPS_SLOT_PATCHED: [u8; 8] = *b"%i FPS+\0";
static TEST_LOCK: Mutex<()> = Mutex::new(());

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "dasru-writer-{}-{}",
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
        .expect("rustc is available");
    assert!(
        compilation.status.success(),
        "simulator compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    (root, executable)
}

fn registry_for(executable: &Path) -> BuildRegistry {
    let identity = ExecutableIdentity::inspect(executable).unwrap();
    BuildRegistry::from_toml(&format!(
        "[[builds]]\nid = 'simulated'\nname = 'Synthetic game'\nsha256 = '{}'\n",
        identity.sha256
    ))
    .unwrap()
}

fn start(executable: &Path) -> ChildGuard {
    let mut child = ChildGuard(
        Command::new(executable)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("synthetic game starts"),
    );
    for _ in 0..50 {
        assert!(child.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    child
}

#[test]
fn simulated_game_marker_is_patched_and_restored() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (root, executable) = fixture();
    let registry = registry_for(&executable);
    let _child = start(&executable);
    let snapshot_path = root.join("recovery.json");

    let read_only = inspect_metrics_layout_with_registry_and_replacement(&registry, "%i FPS+");
    assert_eq!(
        read_only.dry_run.status,
        dasru::memory::MetricsDryRunStatus::Safe
    );
    assert_eq!(read_only.dry_run.replacement, "%i FPS+");
    assert_eq!(read_only.dry_run.diff.len(), 1);
    assert_eq!(read_only.dry_run.diff[0].after, b'+');
    assert!(!snapshot_path.exists());

    let mut session = apply_fps_marker_with_registry(&registry, "+", true, &snapshot_path).unwrap();
    assert_eq!(session.fps_slot().unwrap(), FPS_SLOT_PATCHED);

    session.restore().unwrap();
    assert_eq!(session.fps_slot().unwrap(), FPS_SLOT_ORIGINAL);
    assert!(!snapshot_path.exists());
    session.restore().unwrap();
    assert_eq!(
        restore_fps_marker_from(&snapshot_path).unwrap(),
        "No pending FPS marker recovery snapshot"
    );

    drop(_child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_build_is_refused_without_creating_a_snapshot() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (root, executable) = fixture();
    let _child = start(&executable);
    let snapshot_path = root.join("recovery.json");

    let error = match apply_fps_marker_with_registry(
        &BuildRegistry::from_toml("builds = []").unwrap(),
        "+",
        true,
        &snapshot_path,
    ) {
        Err(error) => error,
        Ok(_) => panic!("unknown build must be refused"),
    };

    assert!(error.contains("registered executable build"));
    assert!(!snapshot_path.exists());
    drop(_child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_existing_recovery_snapshot_is_never_deleted_by_a_failed_attach() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (root, executable) = fixture();
    let registry = registry_for(&executable);
    let _child = start(&executable);
    let snapshot_path = root.join("recovery.json");
    let original = b"recovery data from another session";
    fs::write(&snapshot_path, original).unwrap();

    let error = match apply_fps_marker_with_registry(&registry, "+", true, &snapshot_path) {
        Err(error) => error,
        Ok(_) => panic!("an existing recovery snapshot must block a new patch"),
    };

    assert!(!error.is_empty());
    assert_eq!(fs::read(&snapshot_path).unwrap(), original);
    drop(_child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_game_processes_are_refused_before_snapshot_creation() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (root, executable) = fixture();
    let registry = registry_for(&executable);
    let _first = start(&executable);
    let _second = start(&executable);
    let snapshot_path = root.join("recovery.json");

    let error = match apply_fps_marker_with_registry(&registry, "+", true, &snapshot_path) {
        Err(error) => error,
        Ok(_) => panic!("duplicate game processes must be refused"),
    };

    assert!(error.contains("multiple game processes were found"));
    assert!(!snapshot_path.exists());
    drop(_second);
    drop(_first);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_refuses_a_snapshot_bound_to_another_process_creation_time() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (root, executable) = fixture();
    let registry = registry_for(&executable);
    let _child = start(&executable);
    let snapshot_path = root.join("recovery.json");
    let mut session = apply_fps_marker_with_registry(&registry, "+", true, &snapshot_path).unwrap();
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
    snapshot["creation_time_filetime"] =
        serde_json::Value::from(snapshot["creation_time_filetime"].as_u64().unwrap() + 1);
    fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();

    let error = restore_fps_marker_from(&snapshot_path).unwrap_err();
    assert!(error.contains("writable process handle identity differs"));
    assert!(snapshot_path.exists());
    assert_eq!(session.fps_slot().unwrap(), FPS_SLOT_PATCHED);

    session.restore().unwrap();
    assert_eq!(session.fps_slot().unwrap(), FPS_SLOT_ORIGINAL);
    drop(_child);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminated_target_is_not_reopened_by_pid_during_session_cleanup() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (root, executable) = fixture();
    let registry = registry_for(&executable);
    let mut child = start(&executable);
    let snapshot_path = root.join("recovery.json");
    let mut session = apply_fps_marker_with_registry(&registry, "+", true, &snapshot_path).unwrap();

    child.0.kill().unwrap();
    child.0.wait().unwrap();
    session.restore().unwrap();

    assert!(!snapshot_path.exists());
    drop(child);
    fs::remove_dir_all(root).unwrap();
}
