// Copy-on-write keeps the patch private to the mapped game process.
pub(crate) const PAGE_WRITECOPY: u32 = 0x08;
pub(crate) const PAGE_READWRITE: u32 = 0x04;

pub(crate) trait PageProtector {
    fn set_protection(&self, address: u64, length: usize, protection: u32) -> Result<u32, String>;
}

pub(crate) struct PageProtectionGuard<'a, T: PageProtector> {
    target: &'a T,
    address: u64,
    length: usize,
    restore_to: u32,
    active: bool,
}

impl<'a, T: PageProtector> PageProtectionGuard<'a, T> {
    pub(crate) fn make_writable(
        target: &'a T,
        address: u64,
        length: usize,
        expected_current: u32,
        restore_to: u32,
    ) -> Result<Self, String> {
        let previous = target.set_protection(address, length, PAGE_WRITECOPY)?;
        let mut guard = Self {
            target,
            address,
            length,
            restore_to: if previous == expected_current {
                restore_to
            } else {
                previous
            },
            active: true,
        };
        if previous != expected_current {
            let restore = guard.restore();
            return Err(match restore {
                Ok(()) => format!(
                    "memory protection changed unexpectedly (expected 0x{expected_current:X}, observed 0x{previous:X})"
                ),
                Err(error) => format!(
                    "memory protection changed unexpectedly (expected 0x{expected_current:X}, observed 0x{previous:X}); restoration failed: {error}"
                ),
            });
        }
        Ok(guard)
    }

    pub(crate) fn ensure_writable(&mut self) -> Result<(), String> {
        let previous = self
            .target
            .set_protection(self.address, self.length, PAGE_WRITECOPY)?;
        if previous == self.restore_to || previous == PAGE_WRITECOPY || previous == PAGE_READWRITE {
            self.active = true;
            return Ok(());
        }

        let restore = self
            .target
            .set_protection(self.address, self.length, previous);
        self.active = restore.is_err();
        Err(match restore {
            Ok(observed) if observed == PAGE_WRITECOPY || observed == PAGE_READWRITE => format!(
                "unexpected page protection 0x{previous:X} during recovery; observed protection was reapplied"
            ),
            Ok(observed) => format!(
                "unexpected page protection 0x{previous:X} during recovery; protection changed from 0x{observed:X} while restoring"
            ),
            Err(error) => format!(
                "unexpected page protection 0x{previous:X} during recovery; could not reapply it: {error}"
            ),
        })
    }

    pub(crate) fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let previous = self
            .target
            .set_protection(self.address, self.length, self.restore_to)?;
        self.active = false;
        if previous != PAGE_WRITECOPY && previous != PAGE_READWRITE {
            return Err(format!(
                "page protection before restoration was 0x{previous:X}, expected PAGE_WRITECOPY or PAGE_READWRITE"
            ));
        }
        Ok(())
    }
}

impl<T: PageProtector> Drop for PageProtectionGuard<'_, T> {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            tracing::error!(%error, "failed to restore remote memory protection in guard drop");
        }
    }
}
