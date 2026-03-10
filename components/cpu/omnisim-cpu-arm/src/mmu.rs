//! ARMv8/v9 MMU — Stage1 + Stage2 地址转换
//! AddressTranslator 实现，内部模块，不对外暴露为独立 Unit

use omnisim_core::error::OmniResult;
use omnisim_core::mem::{AddressTranslator, MemAccessKind};

pub struct ArmMmu;

impl AddressTranslator for ArmMmu {
    fn translate_s1(&self, va: u64, _asid: u16, _access: MemAccessKind) -> OmniResult<u64> {
        Ok(va) // 骨架：直接映射
    }
    fn translate_s2(&self, ipa: u64, _vmid: u16, _access: MemAccessKind) -> OmniResult<u64> {
        Ok(ipa)
    }
    fn flush_tlb_vmid(&self, _vmid: u16) {}
    fn flush_tlb_asid(&self, _asid: u16) {}
}
