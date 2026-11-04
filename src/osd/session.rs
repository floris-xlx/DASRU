use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use crate::{
    identity::{BuildRegistry, ExecutableIdentity},
    memory::{
        FPS_SIGNATURE, MetricsDryRunStatus, MetricsScanStatus,
        transaction::{FPS_PATCH_INDEX, FPS_SLOT_ORIGINAL, apply_fps_patch, restore_fps_patch},
        writer::ExperimentalWritePermit,
    },
    probe::ProbeStatus,
    process::{
        ProcessIdentity, ProcessInspection, WritableProcess, enumerate_pinned_game_processes,
        open_recovery_process, open_writable_process, recheck_pinned_process,
    },
};
use windows_sys::Win32::{
    Foundation::GetLastError,
    System::{
        Console::{
            CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT,
            CTRL_SHUTDOWN_EVENT, SetConsoleCtrlHandler,
        },
        Memory::{MEM_COMMIT, MEM_IMAGE, PAGE_GUARD, PAGE_READONLY, PAGE_READWRITE},
    },
};

use super::snapshot::RecoverySnapshot;

static RESTORE_REQUESTED: AtomicBool = AtomicBool::new(false);

pub struct FpsMarkerSession {
    process: WritableProcess,
    snapshot: RecoverySnapshot,
    snapshot_path: PathBuf,
    active: bool,
}

impl FpsMarkerSession {
    pub fn fps_slot(&self) -> Result<[u8; 8], String> {
        self.process.revalidate()?;
        let mut bytes = [0; 8];
        self.process
            .read_exact(self.patch_address() - FPS_PATCH_INDEX as u64, &mut bytes)?;
        Ok(bytes)
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        if !self.process.is_running()? {
            self.remove_snapshot()?;
            self.active = false;
            return Ok(());
        }
        self.validate_process()?;
        self.validate_target_region()?;
        let current_signature = self.read_signature()?;
        let patched_signature = patched_signature();
        if current_signature != patched_signature && current_signature != FPS_SIGNATURE {
            return Err(
                "refusing restoration because adjacent FPS signature bytes changed".to_owned(),
            );
        }

        restore_fps_patch(
            &self.process,
            self.patch_address(),
            self.snapshot.original_protection,
        )
        .map_err(|error| {
            format!(
                "{}; recovery snapshot retained at {}",
                error,
                self.snapshot_path.display()
            )
        })?;
        if self.read_signature()? != FPS_SIGNATURE {
            return Err(format!(
                "restored FPS signature did not verify; recovery snapshot retained at {}",
                self.snapshot_path.display()
            ));
        }
        let region = self.process.query_region(self.patch_address())?;
        if region.state != MEM_COMMIT
            || region.region_type != MEM_IMAGE
            || region.protection != self.snapshot.original_protection
        {
            return Err(format!(
                "page protection restoration did not verify (0x{:X}); recovery snapshot retained at {}",
                region.protection,
                self.snapshot_path.display()
            ));
        }
        self.remove_snapshot()?;
        self.active = false;
        Ok(())
    }

    fn patch_address(&self) -> u64 {
        self.snapshot.module_base + self.snapshot.patch_module_offset
    }

    fn read_signature(&self) -> Result<Vec<u8>, String> {
        let signature_offset = self
            .snapshot
            .patch_module_offset
            .checked_sub(FPS_PATCH_INDEX as u64)
            .ok_or_else(|| "snapshot patch offset precedes FPS signature".to_owned())?;
        let address = self
            .snapshot
            .module_base
            .checked_add(signature_offset)
            .ok_or_else(|| "snapshot signature address overflows".to_owned())?;
        let mut signature = vec![0; FPS_SIGNATURE.len()];
        self.process.read_exact(address, &mut signature)?;
        Ok(signature)
    }

    fn validate_process(&self) -> Result<(), String> {
        self.process.revalidate()?;
        if self.process.identity().creation_time_filetime != self.snapshot.creation_time_filetime
            || self.process.identity().module_base != self.snapshot.module_base
            || self.process.identity().image_size != self.snapshot.module_size
            || self.process.identity().executable_path != self.snapshot.executable_path
        {
            return Err("process identity differs from the recovery snapshot".to_owned());
        }
        let executable = ExecutableIdentity::inspect(&self.snapshot.executable_path)
            .map_err(|error| format!("re-read target executable hash: {error}"))?;
        if executable.sha256 != self.snapshot.executable_sha256 {
            return Err("target executable hash differs from the recovery snapshot".to_owned());
        }
        Ok(())
    }

    fn validate_target_region(&self) -> Result<(), String> {
        let address = self.patch_address();
        let region = self.process.query_region(address)?;
        let region_end = region
            .base
            .checked_add(region.size)
            .ok_or_else(|| "recovery memory region overflows address space".to_owned())?;
        let module_end = self
            .snapshot
            .module_base
            .checked_add(u64::from(self.snapshot.module_size))
            .ok_or_else(|| "recovery module range overflows address space".to_owned())?;
        if region.state != MEM_COMMIT
            || region.region_type != MEM_IMAGE
            || (region.protection != self.snapshot.original_protection
                && region.protection != crate::memory::protection::PAGE_WRITECOPY
                && region.protection != PAGE_READWRITE)
            || address < region.base
            || address >= region_end
            || address < self.snapshot.module_base
            || address >= module_end
        {
            return Err(
                "recovery target is outside its expected committed image region".to_owned(),
            );
        }
        Ok(())
    }

    fn remove_snapshot(&self) -> Result<(), String> {
        if self.snapshot_path.exists() {
            RecoverySnapshot::remove(&self.snapshot_path)?;
        }
        Ok(())
    }
}

impl Drop for FpsMarkerSession {
    fn drop(&mut self) {
        if self.active
            && let Err(error) = self.restore()
        {
            tracing::error!(%error, snapshot = %self.snapshot_path.display(), "FPS marker drop restoration failed");
        }
    }
}

pub fn apply_fps_marker_with_registry(
    registry: &BuildRegistry,
    marker: &str,
    experimental_opt_in: bool,
    snapshot_path: &Path,
) -> Result<FpsMarkerSession, String> {
    if marker != "+" {
        return Err("the only supported FPS marker is +".to_owned());
    }
    let layout = crate::memory::inspect_metrics_layout_with_registry(registry);
    let probe = &layout.scan.probe;
    if probe.status != ProbeStatus::ExactMatch {
        return Err(format!(
            "experimental writes require an exact registered executable build; observed {:?}",
            probe.status
        ));
    }
    let resolution = probe
        .build_resolution
        .as_ref()
        .ok_or_else(|| "probe did not report executable build resolution".to_owned())?;
    let permit = ExperimentalWritePermit::authorize(experimental_opt_in, resolution)
        .map_err(str::to_owned)?;
    if layout.scan.status != MetricsScanStatus::FoundUniqueMatch
        || layout
            .scan
            .scan_stats
            .as_ref()
            .is_none_or(|stats| stats.coverage != "FULL_MODULE")
        || layout.dry_run.status != MetricsDryRunStatus::Safe
        || layout.strings.first().is_none_or(|string| {
            string.name != "fps_label"
                || string.capacity_bytes != 8
                || string.original_value != "%i FPS"
        })
    {
        return Err("complete unique FPS layout validation did not pass".to_owned());
    }
    let found = layout
        .scan
        .matches
        .first()
        .ok_or_else(|| "FPS scan did not retain its unique match".to_owned())?;
    let probe_process = layout
        .scan
        .probe
        .processes
        .first()
        .ok_or_else(|| "verified process identity is missing".to_owned())?;
    if layout.scan.probe.processes.len() != 1 {
        return Err("multiple game processes were found; refusing a write".to_owned());
    }
    let ProcessInspection::Available { identity } = &probe_process.inspection else {
        return Err("verified process identity is unavailable".to_owned());
    };
    let executable = layout
        .scan
        .probe
        .executable
        .as_ref()
        .ok_or_else(|| "verified executable fingerprint is missing".to_owned())?;
    let build_hash = match resolution {
        crate::identity::BuildResolution::ExactMatch { build } => &build.sha256,
        _ => return Err("executable build is not registered".to_owned()),
    };
    if executable.sha256 != *build_hash {
        return Err("reported executable hash differs from the registered build".to_owned());
    }

    let pinned = enumerate_pinned_game_processes().map_err(|error| error.to_string())?;
    if pinned.len() != 1
        || pinned[0].candidate.pid != probe_process.pid
        || !matches!(
            recheck_pinned_process(&pinned[0]),
            ProcessInspection::Available { identity: ref current } if current == identity
        )
    {
        return Err("process identity or process count changed before writable attach".to_owned());
    }
    let process = open_writable_process(probe_process.pid, identity, &permit)
        .map_err(|failure| format!("writable process attach failed: {failure:?}"))?;
    process.revalidate()?;
    if process.identity() != identity {
        return Err("writable handle identity differs from the read-only inspection".to_owned());
    }
    let current_processes = enumerate_pinned_game_processes().map_err(|error| error.to_string())?;
    if current_processes.len() != 1
        || current_processes[0].candidate.pid != probe_process.pid
        || !matches!(
            recheck_pinned_process(&current_processes[0]),
            ProcessInspection::Available { identity: ref current } if current == identity
        )
    {
        return Err("process identity or process count changed after writable attach".to_owned());
    }
    let current_executable = ExecutableIdentity::inspect(&identity.executable_path)
        .map_err(|error| format!("revalidate target executable fingerprint: {error}"))?;
    if current_executable.sha256 != executable.sha256 {
        return Err("target executable changed after read-only inspection".to_owned());
    }

    let patch_module_offset = found
        .module_offset
        .checked_add(FPS_PATCH_INDEX as u64)
        .ok_or_else(|| "FPS patch module offset overflows".to_owned())?;
    if found.module_offset + FPS_SIGNATURE.len() as u64 > u64::from(identity.image_size) {
        return Err("FPS signature extends past the executable module".to_owned());
    }
    let patch_address = identity
        .module_base
        .checked_add(patch_module_offset)
        .ok_or_else(|| "FPS patch address overflows".to_owned())?;
    let region = process.query_region(patch_address)?;
    let region_end = region
        .base
        .checked_add(region.size)
        .ok_or_else(|| "FPS memory region address overflows".to_owned())?;
    if region.state != MEM_COMMIT
        || region.region_type != MEM_IMAGE
        || region.protection != PAGE_READONLY
        || region.protection & PAGE_GUARD != 0
        || patch_address < region.base
        || patch_address >= region_end
    {
        return Err(
            "FPS target is not in the confirmed committed PAGE_READONLY MEM_IMAGE region"
                .to_owned(),
        );
    }
    let signature_address = identity
        .module_base
        .checked_add(found.module_offset)
        .ok_or_else(|| "FPS signature address overflows".to_owned())?;
    let mut signature = vec![0; FPS_SIGNATURE.len()];
    process.read_exact(signature_address, &mut signature)?;
    if signature != FPS_SIGNATURE {
        return Err("original FPS signature changed before the write transaction".to_owned());
    }
    let mut original_slot = [0; FPS_SLOT_ORIGINAL.len()];
    process.read_exact(signature_address, &mut original_slot)?;
    if original_slot != FPS_SLOT_ORIGINAL {
        return Err("original eight-byte FPS slot changed before the write transaction".to_owned());
    }

    let snapshot = RecoverySnapshot::new(
        probe_process.pid,
        identity,
        executable.sha256.clone(),
        patch_module_offset,
        region.protection,
    );
    if let Err(error) = apply_fps_patch(&process, patch_address, region.protection, || {
        snapshot.persist(snapshot_path)
    }) {
        let recovery = if error.recovered {
            RecoverySnapshot::remove(snapshot_path)
                .map(|()| "; automatic rollback verified and snapshot removed".to_owned())
                .unwrap_or_else(|remove_error| format!("; rollback verified; {remove_error}"))
        } else {
            format!(
                "; recovery snapshot retained at {}",
                snapshot_path.display()
            )
        };
        return Err(format!("{error}{recovery}"));
    }
    let expected_signature = patched_signature();
    let mut actual_signature = vec![0; FPS_SIGNATURE.len()];
    let signature_read = process
        .read_exact(signature_address, &mut actual_signature)
        .and_then(|()| {
            if actual_signature == expected_signature {
                Ok(())
            } else {
                Err("read-back signature differs from the expected patched signature".to_owned())
            }
        });
    if let Err(verification_error) = signature_read {
        let rollback = restore_fps_patch(&process, patch_address, region.protection);
        if rollback.is_ok() {
            let _ = RecoverySnapshot::remove(snapshot_path);
        }
        return Err(format!(
            "post-write FPS signature verification failed: {verification_error}; rollback result: {}; snapshot: {}",
            rollback.map_or_else(|error| error.to_string(), |_| "restored".to_owned()),
            snapshot_path.display()
        ));
    }
    process.revalidate()?;
    Ok(FpsMarkerSession {
        process,
        snapshot,
        snapshot_path: snapshot_path.to_owned(),
        active: true,
    })
}

pub fn restore_from_snapshot(path: &Path) -> Result<String, String> {
    let snapshot = RecoverySnapshot::load(path)?;
    let expected = ProcessIdentity {
        creation_time_filetime: snapshot.creation_time_filetime,
        executable_path: snapshot.executable_path.clone(),
        module_base: snapshot.module_base,
        image_size: snapshot.module_size,
    };
    let process = open_recovery_process(snapshot.pid, &expected)
        .map_err(|failure| format!("recovery process attach refused: {failure:?}"))?;
    let mut session = FpsMarkerSession {
        process,
        snapshot,
        snapshot_path: path.to_owned(),
        active: true,
    };
    session.restore()?;
    Ok("FPS marker restored and recovery snapshot removed".to_owned())
}

pub fn run_experimental_session(marker: &str, experimental_opt_in: bool) -> Result<String, String> {
    let _handler = ConsoleHandler::install()?;
    let path = super::snapshot::default_path()?;
    let registry = BuildRegistry::from_toml(include_str!("../../data/builds.toml"))
        .map_err(|error| format!("build registry is invalid: {error}"))?;
    let mut session =
        apply_fps_marker_with_registry(&registry, marker, experimental_opt_in, &path)?;
    println!(
        "FPS marker + active for PID {}; press Ctrl+C to restore.",
        session.process.pid()
    );
    loop {
        if RESTORE_REQUESTED.swap(false, Ordering::SeqCst) {
            session.restore()?;
            return Ok("FPS marker restored; original bytes and protection verified".to_owned());
        }
        if !session.process.is_running()? {
            session.remove_snapshot()?;
            session.active = false;
            return Ok(
                "target process exited; its memory was released and the snapshot was removed"
                    .to_owned(),
            );
        }
        session.process.revalidate()?;
        thread::sleep(Duration::from_millis(100));
    }
}

fn patched_signature() -> Vec<u8> {
    let mut signature = FPS_SIGNATURE.to_vec();
    signature[FPS_PATCH_INDEX] = b'+';
    signature
}

struct ConsoleHandler;

impl ConsoleHandler {
    fn install() -> Result<Self, String> {
        RESTORE_REQUESTED.store(false, Ordering::SeqCst);
        // SAFETY: handler has the required system ABI and only sets an atomic flag.
        if unsafe { SetConsoleCtrlHandler(Some(console_control), 1) } == 0 {
            return Err(format!(
                "SetConsoleCtrlHandler failed (Win32 error {})",
                unsafe { GetLastError() }
            ));
        }
        Ok(Self)
    }
}

impl Drop for ConsoleHandler {
    fn drop(&mut self) {
        // SAFETY: unregisters the same process-wide callback registered by install.
        unsafe { SetConsoleCtrlHandler(Some(console_control), 0) };
    }
}

unsafe extern "system" fn console_control(event: u32) -> i32 {
    if matches!(
        event,
        CTRL_C_EVENT
            | CTRL_BREAK_EVENT
            | CTRL_CLOSE_EVENT
            | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT
    ) {
        RESTORE_REQUESTED.store(true, Ordering::SeqCst);
        1
    } else {
        0
    }
}
