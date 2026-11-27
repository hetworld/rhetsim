//! RISC-V Sv39 page table walk

const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;

const LEVELS: usize = 3;
const PTESIZE: u64 = 8;
const PAGESIZE: u64 = 4096;

const MSTATUS_SUM: u64 = 1 << 18;
const MSTATUS_MXR: u64 = 1 << 19;

#[derive(Debug, Clone, Copy)]
pub enum AccessType {
    Fetch,
    Load,
    Store,
}

#[derive(Debug, Clone, Copy)]
pub struct PageFault {
    pub cause: u64,
    pub tval: u64,
}

impl PageFault {
    fn new(access: AccessType, va: u64) -> Self {
        let cause = match access {
            AccessType::Fetch => 12,
            AccessType::Load => 13,
            AccessType::Store => 15,
        };
        Self { cause, tval: va }
    }
}

pub fn translate_sv39(
    va: u64,
    satp: u64,
    access: AccessType,
    priv_mode: u8,
    mstatus: u64,
    phys_read_u64: &dyn Fn(u64) -> u64,
) -> Result<u64, PageFault> {
    let mode = (satp >> 60) & 0xF;
    if mode == 0 || priv_mode == 3 {
        return Ok(va);
    }

    let root_ppn = satp & 0x0FFF_FFFF_FFFF;
    let vpn = [(va >> 12) & 0x1FF, (va >> 21) & 0x1FF, (va >> 30) & 0x1FF];
    let page_offset = va & 0xFFF;

    let mut a = root_ppn * PAGESIZE;

    for level in (0..LEVELS).rev() {
        let pte_addr = a + vpn[level] * PTESIZE;
        let pte = phys_read_u64(pte_addr);

        if pte & PTE_V == 0 || (pte & PTE_R == 0 && pte & PTE_W != 0) {
            return Err(PageFault::new(access, va));
        }

        if pte & PTE_R != 0 || pte & PTE_X != 0 {
            let ppn = (pte >> 10) & 0x0FFF_FFFF_FFFF;

            for l in 0..level {
                if (ppn >> (l * 9)) & 0x1FF != 0 {
                    return Err(PageFault::new(access, va));
                }
            }

            check_perm(pte, access, priv_mode, mstatus, va)?;

            if pte & PTE_A == 0 {
                return Err(PageFault::new(access, va));
            }
            if matches!(access, AccessType::Store) && pte & PTE_D == 0 {
                return Err(PageFault::new(access, va));
            }

            let mut pa = page_offset;
            for (l, &vpn_l) in vpn.iter().enumerate() {
                let bits = if l < level {
                    vpn_l
                } else {
                    (ppn >> (l * 9)) & 0x1FF
                };
                pa |= bits << (12 + l * 9);
            }
            return Ok(pa);
        }

        // Non-leaf: descend
        let ppn = (pte >> 10) & 0x0FFF_FFFF_FFFF;
        a = ppn * PAGESIZE;
    }

    Err(PageFault::new(access, va))
}

fn check_perm(
    pte: u64,
    access: AccessType,
    priv_mode: u8,
    mstatus: u64,
    va: u64,
) -> Result<(), PageFault> {
    let u_bit = pte & PTE_U != 0;
    let sum = mstatus & MSTATUS_SUM != 0;
    let mxr = mstatus & MSTATUS_MXR != 0;

    if priv_mode == 1 && u_bit && !sum {
        return Err(PageFault::new(access, va));
    }
    if priv_mode == 0 && !u_bit {
        return Err(PageFault::new(access, va));
    }

    match access {
        AccessType::Fetch => {
            if pte & PTE_X == 0 {
                return Err(PageFault::new(access, va));
            }
            if priv_mode == 1 && u_bit {
                return Err(PageFault::new(access, va));
            }
        }
        AccessType::Load => {
            if pte & PTE_R == 0 && !(mxr && pte & PTE_X != 0) {
                return Err(PageFault::new(access, va));
            }
        }
        AccessType::Store => {
            if pte & PTE_W == 0 {
                return Err(PageFault::new(access, va));
            }
        }
    }
    Ok(())
}

const TLB_SIZE: usize = 256;

#[derive(Clone, Copy)]
struct TlbEntry {
    va_page: u64,
    pa_page: u64,
    perm: u8,
    valid: bool,
}

impl TlbEntry {
    const fn empty() -> Self {
        Self {
            va_page: 0,
            pa_page: 0,
            perm: 0,
            valid: false,
        }
    }
}

pub struct Tlb {
    entries: [TlbEntry; TLB_SIZE],
}

impl Default for Tlb {
    fn default() -> Self {
        Self::new()
    }
}

impl Tlb {
    pub fn new() -> Self {
        Self {
            entries: [TlbEntry::empty(); TLB_SIZE],
        }
    }

    pub fn flush(&mut self) {
        for e in &mut self.entries {
            e.valid = false;
        }
    }

    pub fn lookup(&self, va: u64, access: AccessType) -> Option<u64> {
        let va_page = va >> 12;
        let idx = (va_page as usize) & (TLB_SIZE - 1);
        let e = &self.entries[idx];
        if !e.valid || e.va_page != va_page {
            return None;
        }
        if e.perm & access_mask(access) == 0 {
            return None;
        }
        Some((e.pa_page << 12) | (va & 0xFFF))
    }

    pub fn fill(&mut self, va: u64, pa: u64, access: AccessType) {
        let va_page = va >> 12;
        let pa_page = pa >> 12;
        let idx = (va_page as usize) & (TLB_SIZE - 1);
        let mask = access_mask(access);
        let e = &mut self.entries[idx];
        if e.valid && e.va_page == va_page {
            e.perm |= mask;
        } else {
            *e = TlbEntry {
                va_page,
                pa_page,
                perm: mask,
                valid: true,
            };
        }
    }
}

fn access_mask(access: AccessType) -> u8 {
    match access {
        AccessType::Load => 1,
        AccessType::Store => 2,
        AccessType::Fetch => 4,
    }
}

#[cfg(test)]
mod tests {
    // Page-table test addresses spell out the zero offset for readability.
    #![allow(clippy::erasing_op)]

    use super::*;

    fn make_pte(ppn: u64, flags: u64) -> u64 {
        (ppn << 10) | flags
    }

    #[test]
    fn bare_mode_passthrough() {
        let satp = 0; // mode=0 (Bare)
        let result = translate_sv39(0x1234, satp, AccessType::Load, 1, 0, &|_| 0);
        assert_eq!(result.unwrap(), 0x1234);
    }

    #[test]
    fn m_mode_passthrough() {
        let satp = 8u64 << 60; // Sv39 enabled but M-mode
        let result = translate_sv39(0x1234, satp, AccessType::Load, 3, 0, &|_| 0);
        assert_eq!(result.unwrap(), 0x1234);
    }

    #[test]
    fn sv39_identity_superpage_1g() {
        // 1 GiB superpage: VA 0x8000_0000 → PA 0x8000_0000
        let root_ppn: u64 = 0x1000;
        let root_table_pa = root_ppn * PAGESIZE;
        let satp = (8u64 << 60) | root_ppn;

        // VPN[2] for 0x8000_0000 = 2
        let pte_offset = 2 * PTESIZE;
        let ppn = 0x8000_0000u64 >> 12; // = 0x80000
        let pte = make_pte(ppn, PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D);

        let phys_read = |addr: u64| -> u64 {
            if addr == root_table_pa + pte_offset {
                pte
            } else {
                0
            }
        };

        let pa = translate_sv39(0x8000_1234, satp, AccessType::Load, 1, 0, &phys_read).unwrap();
        assert_eq!(pa, 0x8000_1234);
    }

    #[test]
    fn sv39_4k_page() {
        // 3-level walk: VA 0x0000_0000_0040_0000 → PA 0x0000_0000_0050_0000
        // VPN[2]=0, VPN[1]=2, VPN[0]=0
        let l2_ppn: u64 = 0x100;
        let l1_ppn: u64 = 0x200;
        let l0_ppn: u64 = 0x300;
        let target_ppn: u64 = 0x500;

        let satp = (8u64 << 60) | l2_ppn;

        let l2_pte = make_pte(l1_ppn, PTE_V); // non-leaf
        let l1_pte = make_pte(l0_ppn, PTE_V); // non-leaf
        let l0_pte = make_pte(target_ppn, PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D);

        let phys_read = move |addr: u64| -> u64 {
            if addr == l2_ppn * PAGESIZE + 0 * PTESIZE {
                l2_pte
            }
            // VPN[2]=0
            else if addr == l1_ppn * PAGESIZE + 2 * PTESIZE {
                l1_pte
            }
            // VPN[1]=2
            else if addr == l0_ppn * PAGESIZE + 0 * PTESIZE {
                l0_pte
            }
            // VPN[0]=0
            else {
                0
            }
        };

        let va = 0x0000_0000_0040_0ABC;
        let pa = translate_sv39(va, satp, AccessType::Load, 1, 0, &phys_read).unwrap();
        assert_eq!(pa, target_ppn * PAGESIZE + 0xABC);
    }

    #[test]
    fn sv39_page_fault_invalid() {
        let satp = (8u64 << 60) | 0x100;
        let result = translate_sv39(0x1000, satp, AccessType::Load, 1, 0, &|_| 0);
        assert!(result.is_err());
        let fault = result.unwrap_err();
        assert_eq!(fault.cause, 13);
    }

    #[test]
    fn sv39_page_fault_no_write_perm() {
        let root_ppn: u64 = 0x100;
        let satp = (8u64 << 60) | root_ppn;
        let pte = make_pte(0x80000, PTE_V | PTE_R | PTE_X | PTE_A | PTE_D); // no W

        let phys_read = move |addr: u64| -> u64 {
            if addr == root_ppn * PAGESIZE + 2 * PTESIZE {
                pte
            } else {
                0
            }
        };

        let result = translate_sv39(0x8000_0000, satp, AccessType::Store, 1, 0, &phys_read);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().cause, 15);
    }

    #[test]
    fn sv39_page_fault_no_access_bit() {
        let root_ppn: u64 = 0x100;
        let satp = (8u64 << 60) | root_ppn;
        let pte = make_pte(0x80000, PTE_V | PTE_R | PTE_W | PTE_X | PTE_D); // no A

        let phys_read = move |addr: u64| -> u64 {
            if addr == root_ppn * PAGESIZE + 2 * PTESIZE {
                pte
            } else {
                0
            }
        };

        let result = translate_sv39(0x8000_0000, satp, AccessType::Load, 1, 0, &phys_read);
        assert!(result.is_err());
    }
}
