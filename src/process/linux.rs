use super::{ProcessCandidate, ProcessScanError};

pub fn enumerate_game_processes() -> Result<Vec<ProcessCandidate>, ProcessScanError> {
    // shortcut: Linux discovery is unsupported; implement it before Phase 3 Proton support.
    Err(ProcessScanError::UnsupportedPlatform)
}
