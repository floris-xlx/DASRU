use std::{
    path::{Path, PathBuf},
    ptr::null,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crate::{
    capability::{
        self, CapabilityEvidence, CapabilityReason, FeatureId, FeatureStatus, LayoutEvidence,
        MemoryEvidence,
    },
    identity::{BuildRegistry, ExecutableIdentity},
    memory::{
        FPS_SIGNATURE, MetricsDryRunStatus, MetricsScanStatus,
        transaction::{
            FPS_PATCH_INDEX, FPS_SLOT_ORIGINAL, apply_fps_patch, restore_fps_patch,
            validate_after_write_or_rollback,
        },
        writer::ExperimentalWritePermit,
    },
    process::{
        ProcessFailureKind, ProcessIdentity, ProcessInspection, WritableProcess,
        enumerate_pinned_game_processes, open_recovery_process, open_writable_process,
        recheck_pinned_process,
    },
    session::{PatchReservation, SessionCoordinator, SessionEvent, SessionLifecycle},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Console::{
            CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT,
            CTRL_SHUTDOWN_EVENT, SetConsoleCtrlHandler,
        },
        Memory::{MEM_COMMIT, MEM_IMAGE, PAGE_GUARD, PAGE_READONLY, PAGE_READWRITE},
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
};

use super::snapshot::RecoverySnapshot;

static RESTORE_REQUESTED: AtomicBool = AtomicBool::new(false);
static SESSION_COORDINATOR: OnceLock<SessionCoordinator> = OnceLock::new();

pub struct FpsMarkerSession {
    process: WritableProcess,
    snapshot: RecoverySnapshot,
    snapshot_path: PathBuf,
    lifecycle: SessionLifecycle,
    reservation: Option<PatchReservation>,
    lock: Option<ExperimentalSessionLock>,
    active: bool,
}

impl FpsMarkerSession {
    pub fn fps_slot(&mut self) -> Result<[u8; 8], String> {
        self.revalidate_identity()?;
        let mut bytes = [0; 8];
        self.process
            .read_exact(self.patch_address() - FPS_PATCH_INDEX as u64, &mut bytes)?;
        Ok(bytes)
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        self.lifecycle
            .transition(SessionEvent::BeginStopping)
            .map_err(|error| format!("invalid session stop transition: {error:?}"))?;
        let restored = match self.restore_active_patch() {
            Ok(restored) => restored,
            Err(error) => {
                let _ = self.lifecycle.transition(SessionEvent::RestorationFailed);
                return Err(error);
            }
        };
        self.lifecycle
            .transition(if restored {
                SessionEvent::RestorationVerified
            } else {
                SessionEvent::ProcessExited
            })
            .map_err(|error| format!("invalid session completion transition: {error:?}"))?;
        self.active = false;
        if let Some(reservation) = self.reservation.take() {
            reservation.release();
        }
        self.lock.take();
        Ok(())
    }

    fn restore_active_patch(&mut self) -> Result<bool, String> {
        if !self.process.is_running()? {
            self.remove_snapshot()?;
            return Ok(false);
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
        Ok(true)
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

    fn revalidate_identity(&mut self) -> Result<(), String> {
        if let Err(error) = self.process.revalidate() {
            let _ = self.lifecycle.transition(SessionEvent::IdentityChanged);
            return Err(error);
        }
        if self.process.identity().creation_time_filetime != self.snapshot.creation_time_filetime
            || self.process.identity().module_base != self.snapshot.module_base
            || self.process.identity().image_size != self.snapshot.module_size
            || self.process.identity().executable_path != self.snapshot.executable_path
        {
            let _ = self.lifecycle.transition(SessionEvent::IdentityChanged);
            return Err("process identity differs from the recovery snapshot".to_owned());
        }
        Ok(())
    }

    fn validate_process(&mut self) -> Result<(), String> {
        self.revalidate_identity()?;
        let executable =
            ExecutableIdentity::inspect(&self.snapshot.executable_path).map_err(|error| {
                let _ = self.lifecycle.transition(SessionEvent::IdentityChanged);
                format!("re-read target executable hash: {error}")
            })?;
        if executable.sha256 != self.snapshot.executable_sha256 {
            let _ = self.lifecycle.transition(SessionEvent::IdentityChanged);
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
        RecoverySnapshot::remove(&self.snapshot_path)
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

struct ExperimentalSessionLock {
    handle: HANDLE,
}

impl ExperimentalSessionLock {
    fn acquire() -> Result<Self, String> {
        let name = "Local\\DASRU.ExperimentalFpsMarker\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "could not create experimental session mutex (Win32 error {})",
                unsafe { GetLastError() }
            ));
        }
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self { handle }),
            WAIT_TIMEOUT => {
                unsafe { CloseHandle(handle) };
                Err("another experimental FPS marker session is active".to_owned())
            }
            _ => {
                let error = unsafe { GetLastError() };
                unsafe { CloseHandle(handle) };
                Err(format!(
                    "could not acquire experimental session mutex (Win32 error {error})"
                ))
            }
        }
    }
}

impl Drop for ExperimentalSessionLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

fn capability_failure(reason: CapabilityReason) -> &'static str {
    match reason {
        CapabilityReason::Available => "experimental FPS marker requirements passed",
        CapabilityReason::NoProcess => "no verified game process is available",
        CapabilityReason::AmbiguousProcess => {
            "multiple game processes were found; refusing a write"
        }
        CapabilityReason::AccessDenied => "required process memory access was denied",
        CapabilityReason::UnknownBuild => {
            "experimental writes require an exact registered executable build"
        }
        CapabilityReason::IdentityChanged => "process identity changed before the write",
        CapabilityReason::UnsupportedPlatform => {
            "experimental game-memory writing is supported on Windows only"
        }
        CapabilityReason::UnsupportedArchitecture => {
            "experimental game-memory writing requires an x86_64 host"
        }
        CapabilityReason::NotImplemented => "experimental FPS marker writing is not implemented",
        CapabilityReason::NotEvaluated => {
            "required process, memory, or layout evidence was not evaluated"
        }
        CapabilityReason::ExplicitOptInRequired => {
            "experimental memory writing was not explicitly enabled"
        }
        CapabilityReason::UnsupportedLayout => "complete unique FPS layout validation did not pass",
        CapabilityReason::PartialCoverage => "full-module memory coverage is required",
        CapabilityReason::CollectionFailed => "support diagnostics could not be collected",
        CapabilityReason::RegistrationConflict => "a requested global hotkey is already registered",
        CapabilityReason::RegistrationFailed => "global hotkey registration failed",
    }
}

fn recovery_is_complete(
    process: &WritableProcess,
    snapshot_path: &Path,
    signature_address: u64,
    patch_address: u64,
    original_protection: u32,
) -> bool {
    match std::fs::symlink_metadata(snapshot_path) {
        Ok(_) => return false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    match process.is_running() {
        Ok(false) => return true,
        Ok(true) => {}
        Err(_) => return false,
    }
    let mut signature = vec![0; FPS_SIGNATURE.len()];
    if process
        .read_exact(signature_address, &mut signature)
        .is_err()
        || signature != FPS_SIGNATURE
    {
        return false;
    }
    process.query_region(patch_address).is_ok_and(|region| {
        region.state == MEM_COMMIT
            && region.region_type == MEM_IMAGE
            && region.protection == original_protection
    })
}

pub fn apply_fps_marker_with_registry(
    registry: &BuildRegistry,
    marker: &str,
    experimental_opt_in: bool,
    snapshot_path: &Path,
) -> Result<FpsMarkerSession, String> {
    let mut lifecycle = SessionLifecycle::default();
    lifecycle
        .transition(SessionEvent::BeginAttachment)
        .map_err(|error| format!("invalid session attachment transition: {error:?}"))?;
    let result = apply_fps_marker_attaching(
        registry,
        marker,
        experimental_opt_in,
        snapshot_path,
        &mut lifecycle,
    );
    if result.is_err() {
        let _ = lifecycle.transition(SessionEvent::AttachmentFailed);
    }
    result
}

fn apply_fps_marker_attaching(
    registry: &BuildRegistry,
    marker: &str,
    experimental_opt_in: bool,
    snapshot_path: &Path,
    lifecycle: &mut SessionLifecycle,
) -> Result<FpsMarkerSession, String> {
    if marker != "+" {
        return Err("the only supported FPS marker is +".to_owned());
    }
    let lock = ExperimentalSessionLock::acquire()?;
    match std::fs::symlink_metadata(snapshot_path) {
        Ok(_) => return Err("pending recovery snapshot blocks a new FPS marker session".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not inspect recovery snapshot: {error}")),
    }
    let layout =
        crate::memory::inspect_metrics_layout_with_registry_and_replacement(registry, "%i FPS+");
    let probe = &layout.scan.probe;
    let full_coverage = layout
        .scan
        .scan_stats
        .as_ref()
        .is_some_and(|stats| stats.coverage == "FULL_MODULE");
    let full_scan = layout.scan.status == MetricsScanStatus::FoundUniqueMatch && full_coverage;
    let safe_layout = full_scan
        && layout.dry_run.status == MetricsDryRunStatus::Safe
        && layout.dry_run.replacement == "%i FPS+"
        && layout.strings.first().is_some_and(|string| {
            string.name == "fps_label"
                && string.capacity_bytes == 8
                && string.original_value == "%i FPS"
        });
    let mut evidence = CapabilityEvidence::from_probe(probe);
    evidence.memory = if full_coverage {
        MemoryEvidence::Readable
    } else if layout.scan.status == MetricsScanStatus::AccessDenied {
        MemoryEvidence::AccessDenied
    } else if layout.scan.scan_stats.is_some() {
        MemoryEvidence::Partial
    } else {
        MemoryEvidence::NotEvaluated
    };
    evidence.layout = if safe_layout {
        LayoutEvidence::Safe
    } else if full_coverage {
        LayoutEvidence::Unsupported
    } else {
        LayoutEvidence::NotEvaluated
    };
    evidence.experimental_opt_in = experimental_opt_in;
    let decision = capability::evaluate(FeatureId::ExperimentalFpsMarker, &evidence);
    if decision.status != FeatureStatus::Experimental || !decision.is_authorized() {
        return Err(capability_failure(decision.reason).to_owned());
    }
    let resolution = probe
        .build_resolution
        .as_ref()
        .ok_or_else(|| "probe did not report executable build resolution".to_owned())?;
    let permit = ExperimentalWritePermit::authorize(experimental_opt_in, resolution, decision)
        .map_err(str::to_owned)?;
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
    let (build_hash, registered_build_id) = match resolution {
        crate::identity::BuildResolution::ExactMatch { build } => (&build.sha256, build.id.clone()),
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
        registered_build_id,
        patch_module_offset,
        region.protection,
    );
    let mut reservation = Some(
        SESSION_COORDINATOR
            .get_or_init(SessionCoordinator::default)
            .reserve_patch(identity, patch_module_offset)
            .map_err(str::to_owned)?,
    );
    let mut snapshot_persisted = false;
    if let Err(error) = apply_fps_patch(&process, patch_address, region.protection, || {
        snapshot.persist(snapshot_path)?;
        snapshot_persisted = true;
        Ok(())
    }) {
        let recovery = if snapshot_persisted && (!error.recovery_attempted || error.recovered) {
            match RecoverySnapshot::remove(snapshot_path) {
                Ok(()) => {
                    reservation.take().unwrap().release();
                    if error.recovered {
                        "; original bytes and protection were restored; recovery snapshot removed"
                            .to_owned()
                    } else {
                        "; memory was not changed; recovery snapshot removed".to_owned()
                    }
                }
                Err(remove_error) => {
                    format!("; recovery snapshot retained; {remove_error}")
                }
            }
        } else if !error.recovery_attempted && !snapshot_persisted {
            reservation.take().unwrap().release();
            "; memory was not changed; any preexisting snapshot was preserved".to_owned()
        } else {
            format!(
                "; recovery snapshot retained at {}",
                snapshot_path.display()
            )
        };
        return Err(format!("{error}{recovery}"));
    }
    let expected_signature = patched_signature();
    if let Err(error) = validate_after_write_or_rollback(
        &process,
        signature_address,
        patch_address,
        region.protection,
        snapshot_path,
        || {
            let mut actual_signature = vec![0; FPS_SIGNATURE.len()];
            process.read_exact(signature_address, &mut actual_signature)?;
            if actual_signature != expected_signature {
                return Err(
                    "read-back signature differs from the expected patched signature".to_owned(),
                );
            }
            process.revalidate()
        },
        || RecoverySnapshot::remove(snapshot_path),
    ) {
        if recovery_is_complete(
            &process,
            snapshot_path,
            signature_address,
            patch_address,
            region.protection,
        ) {
            reservation.take().unwrap().release();
        }
        return Err(error);
    }
    lifecycle
        .transition(SessionEvent::AttachmentVerified(identity.clone()))
        .map_err(|error| format!("invalid session attachment transition: {error:?}"))?;
    Ok(FpsMarkerSession {
        process,
        snapshot,
        snapshot_path: snapshot_path.to_owned(),
        lifecycle: lifecycle.clone(),
        reservation,
        lock: Some(lock),
        active: true,
    })
}

pub fn restore_from_snapshot(path: &Path) -> Result<String, String> {
    let lock = ExperimentalSessionLock::acquire()?;
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok("No pending FPS marker recovery snapshot".to_owned());
        }
        Err(error) => return Err(format!("could not inspect recovery snapshot: {error}")),
    }
    let snapshot = RecoverySnapshot::load(path)?;
    let expected = ProcessIdentity {
        creation_time_filetime: snapshot.creation_time_filetime,
        executable_path: snapshot.executable_path.clone(),
        module_base: snapshot.module_base,
        image_size: snapshot.module_size,
    };
    let process = match open_recovery_process(snapshot.pid, &expected) {
        Ok(process) => process,
        Err(ProcessInspection::Unavailable { failure })
            if failure.kind == ProcessFailureKind::ProcessExited =>
        {
            RecoverySnapshot::remove(path)?;
            SESSION_COORDINATOR
                .get_or_init(SessionCoordinator::default)
                .discard_recovery(&expected, snapshot.patch_module_offset);
            return Ok("target process exited; obsolete recovery snapshot removed".to_owned());
        }
        Err(failure) => return Err(format!("recovery process attach refused: {failure:?}")),
    };
    let reservation = SESSION_COORDINATOR
        .get_or_init(SessionCoordinator::default)
        .resume_recovery(&expected, snapshot.patch_module_offset)
        .map_err(str::to_owned)?;
    let mut session = FpsMarkerSession {
        process,
        snapshot,
        snapshot_path: path.to_owned(),
        lifecycle: SessionLifecycle::begin_recovery(expected),
        reservation: Some(reservation),
        lock: Some(lock),
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
            session.restore()?;
            return Ok(
                "target process exited; its memory was released and the snapshot was removed"
                    .to_owned(),
            );
        }
        session.revalidate_identity()?;
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

#[cfg(test)]
mod tests {
    use super::ExperimentalSessionLock;

    #[test]
    fn experimental_session_lock_rejects_a_concurrent_thread() {
        let lock = ExperimentalSessionLock::acquire().unwrap();
        let second = std::thread::spawn(|| ExperimentalSessionLock::acquire().is_err());
        assert!(second.join().unwrap());
        drop(lock);
        assert!(ExperimentalSessionLock::acquire().is_ok());
    }
}
