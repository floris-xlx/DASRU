use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExecutableIdentity {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub file_version: Option<String>,
    pub product_version: Option<String>,
}

#[derive(Debug, Error)]
pub enum ExecutableError {
    #[error("could not inspect executable {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("executable changed while it was being inspected: {0}")]
    ChangedDuringInspection(PathBuf),
}

impl ExecutableIdentity {
    pub fn inspect(path: &Path) -> Result<Self, ExecutableError> {
        let mut file = File::open(path).map_err(|source| io_error(path, source))?;
        let before = file_snapshot(&file).map_err(|source| io_error(path, source))?;
        let path_before = File::open(path)
            .and_then(|path_file| file_snapshot(&path_file))
            .map_err(|source| io_error(path, source))?;
        if before != path_before {
            return Err(ExecutableError::ChangedDuringInspection(path.to_owned()));
        }

        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|source| io_error(path, source))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        #[cfg(windows)]
        let versions = windows_version_info(path).map_err(|source| io_error(path, source))?;
        #[cfg(not(windows))]
        let versions = (None, None);

        let after = file_snapshot(&file).map_err(|source| io_error(path, source))?;
        let path_after = File::open(path)
            .and_then(|path_file| file_snapshot(&path_file))
            .map_err(|source| io_error(path, source))?;
        // shortcut: metadata checks catch normal races; require immutable snapshots before attestation.
        if before != after || before != path_after {
            return Err(ExecutableError::ChangedDuringInspection(path.to_owned()));
        }

        Ok(Self {
            path: path.to_owned(),
            sha256: format!("{:x}", hasher.finalize()),
            size: before.size,
            file_version: versions.0,
            product_version: versions.1,
        })
    }
}

fn io_error(path: &Path, source: io::Error) -> ExecutableError {
    ExecutableError::Io {
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FileSnapshot {
    size: u64,
    modified: SystemTime,
    identity: PlatformFileIdentity,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct PlatformFileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct PlatformFileIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, PartialEq, Eq)]
struct PlatformFileIdentity;

fn file_snapshot(file: &File) -> io::Result<FileSnapshot> {
    let metadata = file.metadata()?;
    Ok(FileSnapshot {
        size: metadata.len(),
        modified: metadata.modified()?,
        identity: platform_file_identity(file, &metadata)?,
    })
}

#[cfg(unix)]
fn platform_file_identity(
    _file: &File,
    metadata: &std::fs::Metadata,
) -> io::Result<PlatformFileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Ok(PlatformFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_file_identity(
    file: &File,
    _metadata: &std::fs::Metadata,
) -> io::Result<PlatformFileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the raw handle comes from a live File and info is writable for the call.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(PlatformFileIdentity {
        volume_serial_number: info.dwVolumeSerialNumber,
        file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn platform_file_identity(
    _file: &File,
    _metadata: &std::fs::Metadata,
) -> io::Result<PlatformFileIdentity> {
    Ok(PlatformFileIdentity)
}

#[cfg(windows)]
fn windows_version_info(path: &Path) -> io::Result<(Option<String>, Option<String>)> {
    use std::{mem::size_of, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut ignored = 0;
    // SAFETY: wide_path is null-terminated and remains alive for this call.
    let size = unsafe { GetFileVersionInfoSizeW(wide_path.as_ptr(), &mut ignored) };
    if size == 0 {
        return Ok((None, None));
    }

    let mut version_data = Vec::new();
    version_data
        .try_reserve_exact(size as usize)
        .map_err(|error| io::Error::other(error.to_string()))?;
    version_data.resize(size as usize, 0);
    // SAFETY: version_data has the exact writable length requested by the API.
    let succeeded = unsafe {
        GetFileVersionInfoW(
            wide_path.as_ptr(),
            ignored,
            size,
            version_data.as_mut_ptr().cast(),
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    let root = [b'\\' as u16, 0];
    let mut fixed_info = ptr::null_mut();
    let mut length = 0;
    // SAFETY: version_data is the buffer returned by GetFileVersionInfoW; outputs are writable.
    let found = unsafe {
        VerQueryValueW(
            version_data.as_ptr().cast(),
            root.as_ptr(),
            &mut fixed_info,
            &mut length,
        )
    };
    if found == 0 || fixed_info.is_null() || length < size_of::<VS_FIXEDFILEINFO>() as u32 {
        return Ok((None, None));
    }

    // SAFETY: VerQueryValueW returned a pointer to at least one VS_FIXEDFILEINFO.
    let fixed_info = unsafe { fixed_info.cast::<VS_FIXEDFILEINFO>().read_unaligned() };
    if fixed_info.dwSignature != 0xFEEF04BD {
        return Ok((None, None));
    }

    Ok((
        Some(format_version(
            fixed_info.dwFileVersionMS,
            fixed_info.dwFileVersionLS,
        )),
        Some(format_version(
            fixed_info.dwProductVersionMS,
            fixed_info.dwProductVersionLS,
        )),
    ))
}

#[cfg(windows)]
fn format_version(high: u32, low: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        high >> 16,
        high & 0xffff,
        low >> 16,
        low & 0xffff
    )
}
