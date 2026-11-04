use std::path::Path;

use super::{
    FPS_SIGNATURE,
    protection::{PAGE_READWRITE, PAGE_WRITECOPY, PageProtectionGuard, PageProtector},
};

pub(crate) const FPS_SLOT_ORIGINAL: [u8; 8] = *b"%i FPS\0\0";
pub(crate) const FPS_SLOT_PATCHED: [u8; 8] = *b"%i FPS+\0";
pub(crate) const FPS_PATCH_INDEX: usize = 6;

pub(crate) trait MemoryAccess: PageProtector {
    fn read_exact(&self, address: u64, output: &mut [u8]) -> Result<(), String>;
    fn write_exact(&self, address: u64, input: &[u8]) -> Result<(), String>;
    fn protection_at(&self, address: u64) -> Result<u32, String>;
}

#[derive(Debug, thiserror::Error)]
#[error("{operation}: {message}")]
pub(crate) struct PatchTransactionError {
    pub(crate) operation: &'static str,
    pub(crate) message: String,
    pub(crate) recovery_attempted: bool,
    pub(crate) recovered: bool,
}

pub(crate) fn apply_fps_patch<T: MemoryAccess>(
    memory: &T,
    patch_address: u64,
    original_protection: u32,
    persist_snapshot: impl FnOnce() -> Result<(), String>,
) -> Result<(), PatchTransactionError> {
    let slot_address = patch_address
        .checked_sub(FPS_PATCH_INDEX as u64)
        .ok_or_else(|| failure("validate target", "patch address precedes FPS slot"))?;
    let mut observed = [0; FPS_SLOT_ORIGINAL.len()];
    memory
        .read_exact(slot_address, &mut observed)
        .map_err(|error| failure("read original slot", error))?;
    if observed != FPS_SLOT_ORIGINAL {
        return Err(failure(
            "validate original slot",
            "the complete FPS slot differs from the confirmed original bytes",
        ));
    }
    let protection = memory
        .protection_at(patch_address)
        .map_err(|error| failure("read original protection", error))?;
    if protection != original_protection {
        return Err(failure(
            "validate original protection",
            format!("expected 0x{original_protection:X}, observed 0x{protection:X}"),
        ));
    }
    persist_snapshot().map_err(|error| failure("persist recovery snapshot", error))?;

    let mut protection = PageProtectionGuard::make_writable(
        memory,
        patch_address,
        1,
        original_protection,
        original_protection,
    )
    .map_err(|error| failure("make target writable", error))?;
    if let Err(error) = memory.write_exact(patch_address, &[FPS_SLOT_PATCHED[FPS_PATCH_INDEX]]) {
        return Err(recover_patch_failure(
            memory,
            &mut protection,
            slot_address,
            patch_address,
            "write marker",
            error,
        ));
    }

    let mut actual = [0; FPS_SLOT_PATCHED.len()];
    if let Err(error) = memory.read_exact(slot_address, &mut actual) {
        return Err(recover_patch_failure(
            memory,
            &mut protection,
            slot_address,
            patch_address,
            "verify marker",
            error,
        ));
    }
    if actual != FPS_SLOT_PATCHED {
        return Err(recover_patch_failure(
            memory,
            &mut protection,
            slot_address,
            patch_address,
            "verify marker",
            "read-back bytes differ from the expected FPS slot".to_owned(),
        ));
    }

    if let Err(error) = protection.restore() {
        return Err(recover_patch_failure(
            memory,
            &mut protection,
            slot_address,
            patch_address,
            "restore original protection",
            error,
        ));
    }
    Ok(())
}

pub(crate) fn restore_fps_patch<T: MemoryAccess>(
    memory: &T,
    patch_address: u64,
    original_protection: u32,
) -> Result<(), PatchTransactionError> {
    let slot_address = patch_address
        .checked_sub(FPS_PATCH_INDEX as u64)
        .ok_or_else(|| {
            failure(
                "validate recovery target",
                "patch address precedes FPS slot",
            )
        })?;
    let mut observed = [0; FPS_SLOT_ORIGINAL.len()];
    memory
        .read_exact(slot_address, &mut observed)
        .map_err(|error| failure("read recovery target", error))?;
    let current_protection = memory
        .protection_at(patch_address)
        .map_err(|error| failure("read recovery protection", error))?;
    if current_protection != original_protection
        && current_protection != PAGE_WRITECOPY
        && current_protection != PAGE_READWRITE
    {
        return Err(failure(
            "validate recovery protection",
            format!("unexpected page protection 0x{current_protection:X}"),
        ));
    }
    if observed == FPS_SLOT_ORIGINAL {
        if current_protection == original_protection {
            return Ok(());
        }
    } else if observed != FPS_SLOT_PATCHED {
        return Err(failure(
            "validate recovery bytes",
            "target bytes differ from both the original and expected patched FPS slot",
        ));
    }

    let mut protection = PageProtectionGuard::make_writable(
        memory,
        patch_address,
        1,
        current_protection,
        original_protection,
    )
    .map_err(|error| failure("make recovery target writable", error))?;
    if observed == FPS_SLOT_PATCHED {
        memory
            .write_exact(patch_address, &[FPS_SLOT_ORIGINAL[FPS_PATCH_INDEX]])
            .map_err(|error| failure("restore original marker byte", error))?;
        let mut restored = [0; FPS_SLOT_ORIGINAL.len()];
        memory
            .read_exact(slot_address, &mut restored)
            .map_err(|error| failure("verify restored bytes", error))?;
        if restored != FPS_SLOT_ORIGINAL {
            return Err(failure(
                "verify restored bytes",
                "read-back bytes differ from the original FPS slot",
            ));
        }
    }
    protection
        .restore()
        .map_err(|error| failure("restore original protection", error))
}

pub(crate) fn validate_after_write_or_rollback<T: MemoryAccess>(
    memory: &T,
    signature_address: u64,
    patch_address: u64,
    original_protection: u32,
    snapshot_path: &Path,
    validate_after_write: impl FnOnce() -> Result<(), String>,
    remove_snapshot: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let Err(validation_error) = validate_after_write() else {
        return Ok(());
    };

    let rollback = (|| {
        let mut observed_signature = vec![0; FPS_SIGNATURE.len()];
        memory.read_exact(signature_address, &mut observed_signature)?;
        let mut expected_patched_signature = FPS_SIGNATURE.to_vec();
        expected_patched_signature[FPS_PATCH_INDEX] = FPS_SLOT_PATCHED[FPS_PATCH_INDEX];
        if observed_signature != expected_patched_signature && observed_signature != FPS_SIGNATURE {
            return Err(
                "current FPS signature differs from the original and expected patched bytes"
                    .to_owned(),
            );
        }

        restore_fps_patch(memory, patch_address, original_protection)
            .map_err(|error| error.to_string())?;
        let mut restored_signature = vec![0; FPS_SIGNATURE.len()];
        memory.read_exact(signature_address, &mut restored_signature)?;
        if restored_signature != FPS_SIGNATURE {
            return Err("restored FPS signature did not verify".to_owned());
        }
        if memory.protection_at(patch_address)? != original_protection {
            return Err("restored page protection did not verify".to_owned());
        }
        Ok(())
    })();

    if let Err(rollback_error) = rollback {
        return Err(format!(
            "post-write validation failed: {validation_error}; rollback was not verified: {rollback_error}; recovery snapshot retained at {}",
            snapshot_path.display()
        ));
    }

    match remove_snapshot() {
        Ok(()) => Err(format!(
            "post-write validation failed: {validation_error}; rollback verified and recovery snapshot removed"
        )),
        Err(snapshot_error) => Err(format!(
            "post-write validation failed: {validation_error}; rollback verified but snapshot removal failed: {snapshot_error}; recovery snapshot retained at {}",
            snapshot_path.display()
        )),
    }
}

fn recover_patch_failure<T: MemoryAccess>(
    memory: &T,
    protection: &mut PageProtectionGuard<'_, T>,
    slot_address: u64,
    patch_address: u64,
    operation: &'static str,
    cause: String,
) -> PatchTransactionError {
    let mut recovery_errors = Vec::new();
    let rollback = protection
        .ensure_writable()
        .and_then(|()| memory.write_exact(patch_address, &[FPS_SLOT_ORIGINAL[FPS_PATCH_INDEX]]))
        .and_then(|()| {
            let mut restored = [0; FPS_SLOT_ORIGINAL.len()];
            memory.read_exact(slot_address, &mut restored)?;
            if restored == FPS_SLOT_ORIGINAL {
                Ok(())
            } else {
                Err("read-back bytes differ from the original FPS slot".to_owned())
            }
        });
    if let Err(error) = rollback {
        recovery_errors.push(format!("byte recovery failed: {error}"));
    }
    let protection_restore = protection.restore();
    if let Err(error) = protection_restore {
        recovery_errors.push(format!("protection recovery failed: {error}"));
    }
    PatchTransactionError {
        operation,
        message: if recovery_errors.is_empty() {
            format!("{cause}; original bytes and protection were restored")
        } else {
            format!("{cause}; {}", recovery_errors.join("; "))
        },
        recovery_attempted: true,
        recovered: recovery_errors.is_empty(),
    }
}

fn failure(operation: &'static str, message: impl Into<String>) -> PatchTransactionError {
    PatchTransactionError {
        operation,
        message: message.into(),
        recovery_attempted: false,
        recovered: false,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, cell::RefCell, path::Path};

    use super::{MemoryAccess, apply_fps_patch, restore_fps_patch};
    use crate::memory::protection::{PAGE_READWRITE, PAGE_WRITECOPY, PageProtector};

    const BASE: u64 = 0x1000;
    const PATCH_ADDRESS: u64 = BASE + 6;
    const READ_ONLY: u32 = 0x02;

    struct FakeMemory {
        bytes: RefCell<Vec<u8>>,
        protection: Cell<u32>,
        fail_next_write: Cell<bool>,
        fail_writable_protection: Cell<bool>,
        fail_restore_protection: Cell<bool>,
        corrupt_read_number: Cell<usize>,
        reads: Cell<usize>,
    }

    impl FakeMemory {
        fn new(slot: &[u8]) -> Self {
            let mut bytes = slot.to_vec();
            bytes.extend_from_slice(b"%.2fms\0\0Frame : %u\0adjacent");
            Self {
                bytes: RefCell::new(bytes),
                protection: Cell::new(READ_ONLY),
                fail_next_write: Cell::new(false),
                fail_writable_protection: Cell::new(false),
                fail_restore_protection: Cell::new(false),
                corrupt_read_number: Cell::new(0),
                reads: Cell::new(0),
            }
        }
    }

    impl PageProtector for FakeMemory {
        fn set_protection(
            &self,
            _address: u64,
            _length: usize,
            protection: u32,
        ) -> Result<u32, String> {
            if protection == PAGE_WRITECOPY && self.fail_writable_protection.replace(false) {
                return Err("simulated protection change failure".to_owned());
            }
            if protection == READ_ONLY && self.fail_restore_protection.replace(false) {
                return Err("simulated protection restoration failure".to_owned());
            }
            Ok(self.protection.replace(protection))
        }
    }

    impl MemoryAccess for FakeMemory {
        fn read_exact(&self, address: u64, output: &mut [u8]) -> Result<(), String> {
            let start = usize::try_from(address - BASE).map_err(|error| error.to_string())?;
            let end = start + output.len();
            output.copy_from_slice(
                self.bytes
                    .borrow()
                    .get(start..end)
                    .ok_or_else(|| "read outside fake memory".to_owned())?,
            );
            self.reads.set(self.reads.get() + 1);
            if self.corrupt_read_number.get() == self.reads.get() {
                output[0] ^= 0xFF;
            }
            Ok(())
        }

        fn write_exact(&self, address: u64, input: &[u8]) -> Result<(), String> {
            if self.fail_next_write.replace(false) {
                let start = usize::try_from(address - BASE).map_err(|error| error.to_string())?;
                self.bytes.borrow_mut()[start] = input[0];
                if self.protection.get() == PAGE_WRITECOPY {
                    self.protection.set(PAGE_READWRITE);
                }
                return Err("simulated short write".to_owned());
            }
            let start = usize::try_from(address - BASE).map_err(|error| error.to_string())?;
            self.bytes.borrow_mut()[start..start + input.len()].copy_from_slice(input);
            if self.protection.get() == PAGE_WRITECOPY {
                self.protection.set(PAGE_READWRITE);
            }
            Ok(())
        }

        fn protection_at(&self, _address: u64) -> Result<u32, String> {
            Ok(self.protection.get())
        }
    }

    #[test]
    fn patch_changes_only_the_marker_byte_and_recovery_restores_it() {
        let memory = FakeMemory::new(b"%i FPS\0\0");
        let mut persisted = false;

        apply_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY, || {
            persisted = true;
            Ok(())
        })
        .unwrap();

        assert!(persisted);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS+\0");
        assert_eq!(
            &memory.bytes.borrow()[8..],
            b"%.2fms\0\0Frame : %u\0adjacent"
        );
        assert_eq!(memory.protection.get(), READ_ONLY);

        restore_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY).unwrap();
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(
            &memory.bytes.borrow()[8..],
            b"%.2fms\0\0Frame : %u\0adjacent"
        );
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn incorrect_original_bytes_are_refused_before_snapshot_or_write() {
        let memory = FakeMemory::new(b"%i F?S\0\0");
        let mut persisted = false;

        let error = apply_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY, || {
            persisted = true;
            Ok(())
        })
        .unwrap_err();

        assert!(!persisted);
        assert!(!error.recovery_attempted);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i F?S\0\0");
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn page_protection_change_failure_does_not_modify_target_bytes() {
        let memory = FakeMemory::new(b"%i FPS\0\0");
        memory.fail_writable_protection.set(true);

        let error = apply_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY, || Ok(())).unwrap_err();

        assert!(!error.recovery_attempted);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn failed_partial_write_attempts_and_verifies_recovery() {
        let memory = FakeMemory::new(b"%i FPS\0\0");
        memory.fail_next_write.set(true);

        let error = apply_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY, || Ok(())).unwrap_err();

        assert!(error.recovery_attempted);
        assert!(error.recovered);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn readback_mismatch_attempts_and_verifies_recovery() {
        let memory = FakeMemory::new(b"%i FPS\0\0");
        memory.corrupt_read_number.set(2);

        let error = apply_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY, || Ok(())).unwrap_err();

        assert!(error.recovery_attempted);
        assert!(error.recovered);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn protection_restore_failure_rolls_back_the_byte_and_retries_protection() {
        let memory = FakeMemory::new(b"%i FPS\0\0");
        memory.fail_restore_protection.set(true);

        let error = apply_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY, || Ok(())).unwrap_err();

        assert!(error.recovery_attempted);
        assert!(error.recovered);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn recovery_refuses_unexpected_target_bytes_and_is_idempotent_after_restore() {
        let memory = FakeMemory::new(b"%i FPS!\0");

        assert!(restore_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY).is_err());
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS!\0");

        memory.bytes.borrow_mut()[..8].copy_from_slice(b"%i FPS\0\0");
        restore_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY).unwrap();
        restore_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY).unwrap();
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
    }

    #[test]
    fn recovery_accepts_readwrite_protection_after_copy_on_write() {
        let memory = FakeMemory::new(b"%i FPS+\0");
        memory.protection.set(PAGE_READWRITE);

        restore_fps_patch(&memory, PATCH_ADDRESS, READ_ONLY).unwrap();

        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn post_write_validation_failure_rolls_back_before_returning() {
        let memory = FakeMemory::new(b"%i FPS+\0");
        let mut snapshot_removed = false;

        let error = super::validate_after_write_or_rollback(
            &memory,
            BASE,
            PATCH_ADDRESS,
            READ_ONLY,
            Path::new("recovery.json"),
            || Err("injected process revalidation failure".to_owned()),
            || {
                snapshot_removed = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("injected process revalidation failure"));
        assert!(error.contains("rollback verified"));
        assert!(snapshot_removed);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS\0\0");
        assert_eq!(
            &memory.bytes.borrow()[8..],
            b"%.2fms\0\0Frame : %u\0adjacent"
        );
        assert_eq!(memory.protection.get(), READ_ONLY);
    }

    #[test]
    fn post_write_rollback_refuses_changed_signature_and_retains_snapshot() {
        let memory = FakeMemory::new(b"%i FPS+\0");
        memory.bytes.borrow_mut()[9] ^= 1;
        let mut snapshot_removed = false;

        let error = super::validate_after_write_or_rollback(
            &memory,
            BASE,
            PATCH_ADDRESS,
            READ_ONLY,
            Path::new("recovery.json"),
            || Err("injected process revalidation failure".to_owned()),
            || {
                snapshot_removed = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("rollback was not verified"));
        assert!(error.contains("recovery snapshot retained"));
        assert!(!snapshot_removed);
        assert_eq!(&memory.bytes.borrow()[..8], b"%i FPS+\0");
    }
}
