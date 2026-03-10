//! RISC-V MMU — Sv39 / Sv48 / Sv57

use omnisim_core::error::OmniResult;
use omnisim_core::mem::{AddressTranslator, MemAccessKind};

pub struct RiscvMmu;

impl AddressTranslator for RiscvMmu {
    fn translate_s1(&self, va: u64, _asid: u16, _access: MemAccessKind) -> OmniResult<u64> {
        Ok(va)
    }
    fn translate_s2(&self, ipa: u64, _vmid: u16, _access: MemAccessKind) -> OmniResult<u64> {
        Ok(ipa)
    }
    fn flush_tlb_vmid(&self, _vmid: u16) {}
    fn flush_tlb_asid(&self, _asid: u16) {}
}
