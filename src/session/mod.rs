mod state;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::process::ProcessIdentity;

pub use state::{SessionEvent, SessionLifecycle, SessionState, TransitionError};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PatchKey {
    identity: ProcessIdentity,
    patch_module_offset: u64,
}

#[derive(Debug, Default, Clone)]
pub struct SessionCoordinator {
    active_patches: Arc<Mutex<HashMap<PatchKey, bool>>>,
}

impl SessionCoordinator {
    pub fn reserve_patch(
        &self,
        identity: &ProcessIdentity,
        patch_module_offset: u64,
    ) -> Result<PatchReservation, &'static str> {
        let key = PatchKey {
            identity: identity.clone(),
            patch_module_offset,
        };
        let mut active = self
            .active_patches
            .lock()
            .map_err(|_| "session coordinator is unavailable")?;
        if active.contains_key(&key) {
            return Err("a session already owns this process patch");
        }
        active.insert(key.clone(), true);
        drop(active);
        Ok(PatchReservation {
            active_patches: self.active_patches.clone(),
            key: Some(key),
        })
    }

    pub fn resume_recovery(
        &self,
        identity: &ProcessIdentity,
        patch_module_offset: u64,
    ) -> Result<PatchReservation, &'static str> {
        let key = PatchKey {
            identity: identity.clone(),
            patch_module_offset,
        };
        let mut active = self
            .active_patches
            .lock()
            .map_err(|_| "session coordinator is unavailable")?;
        match active.get_mut(&key) {
            Some(is_active) if *is_active => {
                return Err("a session already owns this process patch");
            }
            Some(is_active) => *is_active = true,
            None => {
                active.insert(key.clone(), true);
            }
        }
        drop(active);
        Ok(PatchReservation {
            active_patches: self.active_patches.clone(),
            key: Some(key),
        })
    }

    pub fn discard_recovery(&self, identity: &ProcessIdentity, patch_module_offset: u64) {
        let key = PatchKey {
            identity: identity.clone(),
            patch_module_offset,
        };
        if let Ok(mut active) = self.active_patches.lock()
            && active.get(&key) == Some(&false)
        {
            active.remove(&key);
        }
    }
}

pub struct PatchReservation {
    active_patches: Arc<Mutex<HashMap<PatchKey, bool>>>,
    key: Option<PatchKey>,
}

impl PatchReservation {
    pub fn release(mut self) {
        self.remove();
    }

    fn remove(&mut self) {
        if let Some(key) = self.key.take()
            && let Ok(mut active) = self.active_patches.lock()
        {
            active.remove(&key);
        }
    }
}

impl Drop for PatchReservation {
    fn drop(&mut self) {
        if let Some(key) = &self.key
            && let Ok(mut active) = self.active_patches.lock()
            && let Some(is_active) = active.get_mut(key)
        {
            *is_active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::SessionCoordinator;
    use crate::process::ProcessIdentity;

    fn identity() -> ProcessIdentity {
        ProcessIdentity {
            creation_time_filetime: 1234,
            executable_path: PathBuf::from("C:/Games/DOOMTheDarkAges.exe"),
            module_base: 0x1000,
            image_size: 0x10000,
        }
    }

    #[test]
    fn concurrent_sessions_cannot_reserve_the_same_patch() {
        let coordinator = SessionCoordinator::default();
        let first = coordinator.reserve_patch(&identity(), 0x2006).unwrap();
        assert!(coordinator.reserve_patch(&identity(), 0x2006).is_err());

        first.release();
        assert!(coordinator.reserve_patch(&identity(), 0x2006).is_ok());
    }

    #[test]
    fn unresolved_reservation_blocks_new_attach_but_allows_recovery() {
        let coordinator = SessionCoordinator::default();
        drop(coordinator.reserve_patch(&identity(), 0x2006).unwrap());
        assert!(coordinator.reserve_patch(&identity(), 0x2006).is_err());
        let recovery = coordinator.resume_recovery(&identity(), 0x2006).unwrap();
        assert!(coordinator.resume_recovery(&identity(), 0x2006).is_err());
        recovery.release();
    }

    #[test]
    fn obsolete_recovery_can_be_discarded_after_process_exit() {
        let coordinator = SessionCoordinator::default();
        drop(coordinator.reserve_patch(&identity(), 0x2006).unwrap());
        coordinator.discard_recovery(&identity(), 0x2006);
        assert!(coordinator.reserve_patch(&identity(), 0x2006).is_ok());
    }

    #[test]
    fn reservations_are_bound_to_creation_identity_and_patch_offset() {
        let coordinator = SessionCoordinator::default();
        let first = coordinator.reserve_patch(&identity(), 0x2006).unwrap();
        assert!(coordinator.reserve_patch(&identity(), 0x3006).is_ok());

        let mut replacement = identity();
        replacement.creation_time_filetime += 1;
        assert!(coordinator.reserve_patch(&replacement, 0x2006).is_ok());
        drop(first);
    }
}
