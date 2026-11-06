use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    memory::{
        FPS_SIGNATURE,
        transaction::{FPS_PATCH_INDEX, FPS_SLOT_ORIGINAL, FPS_SLOT_PATCHED},
    },
    process::{GAME_EXECUTABLE, ProcessIdentity},
};

const SNAPSHOT_SCHEMA_VERSION: u8 = 1;
const PAGE_READONLY: u32 = 0x02;

#[cfg(windows)]
pub(crate) fn default_path() -> Result<PathBuf, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        "LOCALAPPDATA is unavailable; cannot persist a recovery snapshot".to_owned()
    })?;
    Ok(PathBuf::from(local_app_data)
        .join("DASRU")
        .join("osd-recovery.json"))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct RecoverySnapshot {
    schema_version: u8,
    pub(crate) pid: u32,
    pub(crate) creation_time_filetime: u64,
    pub(crate) executable_path: PathBuf,
    pub(crate) executable_sha256: String,
    #[serde(default)]
    pub(crate) registered_build_id: Option<String>,
    pub(crate) module_base: u64,
    pub(crate) module_size: u32,
    pub(crate) patch_module_offset: u64,
    pub(crate) original_bytes: [u8; 8],
    pub(crate) patched_bytes: [u8; 8],
    pub(crate) original_protection: u32,
}

impl RecoverySnapshot {
    pub(crate) fn new(
        pid: u32,
        process: &ProcessIdentity,
        executable_sha256: String,
        registered_build_id: String,
        patch_module_offset: u64,
        original_protection: u32,
    ) -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            pid,
            creation_time_filetime: process.creation_time_filetime,
            executable_path: process.executable_path.clone(),
            executable_sha256,
            registered_build_id: Some(registered_build_id),
            module_base: process.module_base,
            module_size: process.image_size,
            patch_module_offset,
            original_bytes: FPS_SLOT_ORIGINAL,
            patched_bytes: FPS_SLOT_PATCHED,
            original_protection,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let signature_end = self
            .patch_module_offset
            .checked_sub(FPS_PATCH_INDEX as u64)
            .and_then(|offset| offset.checked_add(FPS_SIGNATURE.len() as u64));
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION
            || self.pid == 0
            || self.creation_time_filetime == 0
            || self.module_base == 0
            || self.module_size == 0
            || self
                .module_base
                .checked_add(u64::from(self.module_size))
                .is_none()
            || self.patch_module_offset < FPS_PATCH_INDEX as u64
            || self.patch_module_offset >= u64::from(self.module_size)
            || signature_end.is_none_or(|end| end > u64::from(self.module_size))
        {
            return Err("snapshot has invalid process or module identity fields".to_owned());
        }
        if self.executable_sha256.len() != 64
            || !self
                .executable_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !self
                .executable_path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(GAME_EXECUTABLE))
        {
            return Err("snapshot has invalid executable identity".to_owned());
        }
        if self
            .registered_build_id
            .as_ref()
            .is_some_and(|build_id| build_id.is_empty() || build_id.len() > 256)
        {
            return Err("snapshot has an invalid registered build identity".to_owned());
        }
        if self.original_bytes != FPS_SLOT_ORIGINAL
            || self.patched_bytes != FPS_SLOT_PATCHED
            || self.original_protection != PAGE_READONLY
        {
            return Err("snapshot does not describe the confirmed FPS marker patch".to_owned());
        }
        Ok(())
    }

    pub(crate) fn persist(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let parent = path
            .parent()
            .ok_or_else(|| "snapshot path has no parent directory".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create snapshot directory: {error}"))?;
        let file_name = path
            .file_name()
            .ok_or_else(|| "snapshot path has no file name".to_owned())?
            .to_string_lossy();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));
        let result = persist_new_file(&temporary, path, self);
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_reader(
            File::open(path)
                .map_err(|error| format!("open recovery snapshot {}: {error}", path.display()))?,
        )
        .map_err(|error| format!("parse recovery snapshot {}: {error}", path.display()))?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    #[cfg(windows)]
    pub(crate) fn remove(path: &Path) -> Result<(), String> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "remove recovery snapshot {}: {error}",
                path.display()
            )),
        }
    }
}

fn persist_new_file(
    temporary: &Path,
    destination: &Path,
    snapshot: &RecoverySnapshot,
) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .map_err(|error| format!("create temporary recovery snapshot: {error}"))?;
    serde_json::to_writer(&mut file, snapshot)
        .map_err(|error| format!("serialize recovery snapshot: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("finish recovery snapshot: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("flush recovery snapshot: {error}"))?;
    drop(file);
    fs::hard_link(temporary, destination).map_err(|error| {
        format!(
            "publish recovery snapshot {} without overwriting an existing snapshot: {error}",
            destination.display()
        )
    })?;
    fs::remove_file(temporary)
        .map_err(|error| format!("remove temporary recovery snapshot: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::process::ProcessIdentity;

    use super::RecoverySnapshot;

    fn snapshot() -> RecoverySnapshot {
        RecoverySnapshot::new(
            44016,
            &ProcessIdentity {
                creation_time_filetime: 1234,
                executable_path: PathBuf::from("C:/Games/DOOMTheDarkAges.exe"),
                module_base: 0x1000,
                image_size: 0x10000,
            },
            "a".repeat(64),
            "fixture-build".to_owned(),
            0x2006,
            0x02,
        )
    }

    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "dasru-osd-snapshot-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn snapshot_round_trips_and_cannot_be_overwritten() {
        let path = path();
        let snapshot = snapshot();

        snapshot.persist(&path).unwrap();
        assert_eq!(RecoverySnapshot::load(&path).unwrap(), snapshot);
        assert!(snapshot.persist(&path).is_err());

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn schema_one_snapshots_without_registered_build_ids_remain_recoverable() {
        let mut value = serde_json::to_value(snapshot()).unwrap();
        value.as_object_mut().unwrap().remove("registered_build_id");
        let legacy: RecoverySnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.registered_build_id, None);
        legacy.validate().unwrap();
    }

    #[test]
    fn snapshot_validation_rejects_changed_expected_bytes_and_out_of_module_offsets() {
        let mut invalid_bytes = snapshot();
        invalid_bytes.patched_bytes[7] = b'!';
        assert!(invalid_bytes.validate().is_err());

        let mut invalid_offset = snapshot();
        invalid_offset.patch_module_offset = 0x10000;
        assert!(invalid_offset.validate().is_err());

        let mut truncated_signature = snapshot();
        truncated_signature.module_size = 0x2019;
        assert!(truncated_signature.validate().is_err());
    }
}
