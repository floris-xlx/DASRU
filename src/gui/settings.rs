pub use super::settings_writer::{SettingsWriter, WriteStatus};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const WINDOW_WIDTH: f32 = 720.0;
pub const MIN_HEIGHT: f32 = 480.0;
pub const MAX_HEIGHT: f32 = 1200.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub schema_version: u32,
    pub dark: bool,
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
    pub text_scale: f32,
    pub binding: String,
    pub refresh_seconds: u64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            dark: true,
            size: [WINDOW_WIDTH, 700.0],
            position: None,
            text_scale: 1.0,
            binding: "Ctrl+Alt+F9".into(),
            refresh_seconds: 1,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("SETTINGS_SCHEMA_UNSUPPORTED");
        }
        if !self.text_scale.is_finite() || !(0.8..=1.6).contains(&self.text_scale) {
            return Err("SETTINGS_TEXT_SCALE_INVALID");
        }
        if !(1..=30).contains(&self.refresh_seconds) {
            return Err("SETTINGS_INTERVAL_INVALID");
        }
        if !self.size.iter().all(|v| v.is_finite())
            || self.size[0] < 650.0
            || self.size[1] < MIN_HEIGHT
            || self.size.iter().any(|v| *v > 16384.0)
        {
            return Err("SETTINGS_GEOMETRY_INVALID");
        }
        if self
            .position
            .is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 100000.0))
        {
            return Err("SETTINGS_GEOMETRY_INVALID");
        }
        if self.binding.len() > 80
            || crate::hotkeys::binding::HotkeyBinding::parse(&self.binding).is_err()
        {
            return Err("HOTKEY_INVALID_BINDING");
        }
        Ok(())
    }
    pub fn clamp_geometry_with_frame(
        &mut self,
        origin: [f32; 2],
        work_area: [f32; 2],
        frame: [f32; 2],
    ) {
        if frame.iter().any(|value| !value.is_finite() || *value < 0.0) {
            self.position = None;
            return;
        }
        self.clamp_geometry(origin, [work_area[0] - frame[0], work_area[1] - frame[1]]);
    }
    pub fn clamp_geometry(&mut self, origin: [f32; 2], size: [f32; 2]) {
        if !origin.iter().chain(size.iter()).all(|v| v.is_finite())
            || size.iter().any(|v| *v <= 0.0)
        {
            self.position = None;
            return;
        }
        self.size[0] = self.size[0].min(size[0]);
        self.size[1] = self.size[1].min(size[1]);
        if let Some(position) = &mut self.position {
            for axis in 0..2 {
                position[axis] =
                    position[axis].clamp(origin[axis], origin[axis] + size[axis] - self.size[axis]);
            }
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    original: Option<Vec<u8>>,
    writable: bool,
}
impl SettingsStore {
    pub fn load(path: PathBuf) -> (Self, Preferences, Option<&'static str>) {
        let mut store = Self {
            path,
            original: None,
            writable: true,
        };
        match read_settings(&store.path) {
            Ok(None) => (store, Preferences::default(), None),
            Ok(Some(bytes)) => {
                let parsed = serde_json::from_slice::<Preferences>(&bytes)
                    .map_err(|_| "SETTINGS_CORRUPT")
                    .and_then(|mut p| {
                        p.validate()?;
                        p.size = [WINDOW_WIDTH, p.size[1].clamp(MIN_HEIGHT, MAX_HEIGHT)];
                        Ok(p)
                    });
                store.original = Some(bytes);
                match parsed {
                    Ok(p) => (store, p, None),
                    Err(error) => {
                        store.writable = false;
                        (store, Preferences::default(), Some(error))
                    }
                }
            }
            Err(error) => {
                store.writable = false;
                (store, Preferences::default(), Some(error))
            }
        }
    }
    pub fn save(&mut self, preferences: &Preferences) -> Result<(), &'static str> {
        if !self.writable {
            return Err("SETTINGS_PRESERVED_INVALID_FILE");
        }
        preferences.validate()?;
        if read_settings(&self.path)? != self.original {
            return Err("SETTINGS_CHANGED_EXTERNALLY");
        }
        let bytes =
            serde_json::to_vec_pretty(preferences).map_err(|_| "SETTINGS_SERIALIZE_FAILED")?;
        if self.original.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|_| "SETTINGS_DIRECTORY_FAILED")?;
        let temp = parent.join(format!(
            ".preferences-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        publish(&temp, &self.path, &bytes)?;
        self.original = Some(bytes);
        Ok(())
    }
}
fn publish(temp: &Path, destination: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(temp).map_err(|_| "SETTINGS_TEMP_FAILED")?;
    let result = (|| {
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "SETTINGS_WRITE_FAILED")?;
        drop(file);
        replace(temp, destination).map_err(|_| "SETTINGS_PUBLISH_FAILED")
    })();
    let _ = fs::remove_file(temp);
    result
}

fn read_settings(path: &Path) -> Result<Option<Vec<u8>>, &'static str> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("SETTINGS_READ_FAILED"),
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "SETTINGS_READ_FAILED")?;
    if bytes.len() > 65536 {
        return Err("SETTINGS_TOO_LARGE");
    }
    Ok(Some(bytes))
}
#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}
#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: null-terminated paths remain alive throughout this same-volume atomic replacement.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub fn user_path() -> Result<PathBuf, &'static str> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(|p| PathBuf::from(p).join("DASRU").join("preferences.json"))
        .ok_or("SETTINGS_DIRECTORY_UNAVAILABLE")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_collision_never_removes_someone_elses_file() {
        let temp =
            std::env::temp_dir().join(format!("dasru-settings-collision-{}", std::process::id()));
        let destination = temp.with_extension("json");
        fs::write(&temp, b"preserve").unwrap();
        assert_eq!(
            publish(&temp, &destination, b"replacement"),
            Err("SETTINGS_TEMP_FAILED")
        );
        assert_eq!(fs::read(&temp).unwrap(), b"preserve");
        fs::remove_file(temp).unwrap();
    }
}
