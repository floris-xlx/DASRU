#[cfg(any(windows, test))]
pub(crate) mod snapshot;

#[cfg(windows)]
mod session;

#[cfg(windows)]
pub use session::{FpsMarkerSession, apply_fps_marker_with_registry};

#[cfg(windows)]
pub use session::restore_from_snapshot as restore_fps_marker_from;

#[cfg(windows)]
pub use session::run_experimental_session;

#[cfg(not(windows))]
pub fn run_experimental_session(
    _marker: &str,
    _experimental_opt_in: bool,
) -> Result<String, String> {
    Err("experimental game-memory writing is supported on Windows only".to_owned())
}

pub fn restore_fps_marker() -> Result<String, String> {
    #[cfg(windows)]
    {
        let path = snapshot::default_path()?;
        if !path.exists() {
            return Ok("No pending FPS marker recovery snapshot".to_owned());
        }
        restore_fps_marker_from(&path)
    }
    #[cfg(not(windows))]
    {
        Err("game-memory recovery is supported on Windows only".to_owned())
    }
}

#[cfg(not(windows))]
pub fn restore_fps_marker_from(_path: &std::path::Path) -> Result<String, String> {
    Err("game-memory recovery is supported on Windows only".to_owned())
}
