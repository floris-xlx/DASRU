use crate::{
    memory::{protection::PageProtector, transaction::MemoryAccess},
    process::WritableProcess,
};

use windows_sys::Win32::System::Memory::MEM_COMMIT;

impl PageProtector for WritableProcess {
    fn set_protection(&self, address: u64, length: usize, protection: u32) -> Result<u32, String> {
        WritableProcess::set_protection(self, address, length, protection)
    }
}

impl MemoryAccess for WritableProcess {
    fn read_exact(&self, address: u64, output: &mut [u8]) -> Result<(), String> {
        WritableProcess::read_exact(self, address, output)
    }

    fn write_exact(&self, address: u64, input: &[u8]) -> Result<(), String> {
        WritableProcess::write_exact(self, address, input)
    }

    fn protection_at(&self, address: u64) -> Result<u32, String> {
        let region = self.query_region(address)?;
        let end = region
            .base
            .checked_add(region.size)
            .ok_or_else(|| "queried memory region overflows address space".to_owned())?;
        if address < region.base || address >= end || region.state != MEM_COMMIT {
            return Err("target byte is outside committed remote memory".to_owned());
        }
        Ok(region.protection)
    }
}
