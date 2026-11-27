//! Physical Memory Protection (PMP) with the Smepmp security enhancements.
//!
//! PMP is checked on the physical address of every fetch, load and store.
//! Entries with `L=0` only apply to S/U-mode; locked entries (`L=1`) also
//! restrict M-mode. The Smepmp `mseccfg.MML`/`MMWP` bits switch M-mode to a
//! deny-by-default policy with the extended permission table from the
//! Smepmp specification.

use crate::csr::CsrFile;
use crate::mmu::AccessType;

/// PMP entry permission bits in a `pmpcfg` byte.
pub const PMP_R: u8 = 1 << 0;
/// Write permission.
pub const PMP_W: u8 = 1 << 1;
/// Execute permission.
pub const PMP_X: u8 = 1 << 2;
/// Address-matching mode field (R=0, W/O=0 on RV64): OFF/na4/NAPOT/TOR.
pub const PMP_A_MASK: u8 = 3 << 3;
/// Lock bit: the entry also applies to M-mode and resists writes.
pub const PMP_L: u8 = 1 << 7;

/// Number of implemented PMP entries (16, like the C908).
pub const NUM_ENTRIES: usize = 16;

/// `pmpaddr` holds address bits [55:2].
pub const PMPADDR_MASK: u64 = (1 << 54) - 1;

/// Smepmp `mseccfg` bits.
pub const MSECCFG_MML: u64 = 1 << 0;
pub const MSECCFG_MMWP: u64 = 1 << 1;
pub const MSECCFG_RLB: u64 = 1 << 2;

/// Address matching modes.
const A_OFF: u8 = 0;
const A_TOR: u8 = 1;
const A_NA4: u8 = 2;
const A_NAPOT: u8 = 3;

const PRV_M: u8 = 3;

/// Read one PMP configuration byte out of the packed `pmpcfg` registers.
pub fn cfg_byte(pmpcfg: &[u64; 2], index: usize) -> u8 {
    ((pmpcfg[index / 8] >> ((index % 8) * 8)) & 0xFF) as u8
}

fn set_cfg_byte(pmpcfg: &mut [u64; 2], index: usize, value: u8) {
    let shift = (index % 8) * 8;
    let mask = 0xFFu64 << shift;
    pmpcfg[index / 8] = (pmpcfg[index / 8] & !mask) | ((value as u64) << shift);
}

fn a_field(cfg: u8) -> u8 {
    (cfg & PMP_A_MASK) >> 3
}

/// True when writes to this PMP entry are ignored.
pub fn entry_locked(mseccfg: u64, pmpcfg: &[u64; 2], index: usize) -> bool {
    if mseccfg & MSECCFG_RLB != 0 {
        return false;
    }
    cfg_byte(pmpcfg, index) & PMP_L != 0
}

/// Write one PMP configuration byte, honouring the Smepmp locking rules.
///
/// Returns true when the stored value changed (the caller must then flush
/// translation caches).
pub fn write_cfg(mseccfg: u64, pmpcfg: &mut [u64; 2], index: usize, value: u8) -> bool {
    let mml = mseccfg & MSECCFG_MML != 0;
    let rlb = mseccfg & MSECCFG_RLB != 0;

    let mut locked = true;
    if rlb {
        locked = false;
    }
    if !mml && !entry_locked(mseccfg, pmpcfg, index) {
        locked = false;
    }
    if mml {
        // A locked entry may be created only as execute-only (M-mode
        // execute region) ...
        if value & PMP_L != 0 && value & PMP_X != PMP_X {
            locked = false;
        }
        // ... or as the shared RWX region (no L, R=0, W=1, X=1).
        if value & PMP_L != PMP_L && (value & (PMP_R | PMP_W | PMP_X)) != (PMP_W | PMP_X) {
            locked = false;
        }
    }

    if locked {
        return false;
    }
    if cfg_byte(pmpcfg, index) == value {
        return false;
    }
    // Without MML the R=0, W=1 encoding is reserved and ignored.
    if value & PMP_W != 0 && value & PMP_R == 0 && !mml {
        return false;
    }
    set_cfg_byte(pmpcfg, index, value);
    true
}

/// Write one `pmpaddr` register, honouring the locking rules.
///
/// Returns true when the stored value changed.
pub fn write_addr(
    mseccfg: u64,
    pmpcfg: &[u64; 2],
    pmpaddr: &mut [u64; 16],
    index: usize,
    value: u64,
) -> bool {
    // A locked next entry in TOR mode also protects this address register.
    if index + 1 < NUM_ENTRIES {
        let next = cfg_byte(pmpcfg, index + 1);
        if a_field(next) == A_TOR && next & PMP_L != 0 && mseccfg & MSECCFG_RLB == 0 {
            return false;
        }
    }
    if entry_locked(mseccfg, pmpcfg, index) {
        return false;
    }
    let value = value & PMPADDR_MASK;
    if pmpaddr[index] == value {
        return false;
    }
    pmpaddr[index] = value;
    true
}

/// Write `mseccfg`, applying the Smepmp sticky-bit and RLB rules.
///
/// Returns true when a policy bit changed (translation caches must be
/// flushed).
pub fn write_mseccfg(mseccfg: &mut u64, cfg: &[u64; 2], value: u64) -> bool {
    let old = *mseccfg;
    let mut value = value;

    // RLB cannot be enabled once any entry is locked.
    if old & MSECCFG_RLB == 0 && (0..NUM_ENTRIES).any(|i| entry_locked(old, cfg, i)) {
        value &= !MSECCFG_RLB;
    }
    // MML and MMWP are sticky: once enabled they cannot be cleared.
    let sticky = old & (MSECCFG_MML | MSECCFG_MMWP);
    *mseccfg = (value & (MSECCFG_MML | MSECCFG_MMWP | MSECCFG_RLB)) | sticky;
    *mseccfg != old
}

fn pmp_access_bit(access: AccessType) -> u8 {
    match access {
        AccessType::Fetch => PMP_X,
        AccessType::Load => PMP_R,
        AccessType::Store => PMP_W,
    }
}

/// Decode the address range of entry `index`; `None` for disabled entries.
fn entry_range(cfg: u8, addr: u64, prev_addr: u64) -> Option<(u64, u64)> {
    match a_field(cfg) {
        A_OFF => None,
        A_TOR => {
            let start = prev_addr << 2;
            let end = (addr << 2).wrapping_sub(1);
            if start > end {
                None
            } else {
                Some((start, end))
            }
        }
        A_NA4 => {
            let start = addr << 2;
            Some((start, start + 3))
        }
        A_NAPOT => {
            let a = (addr << 2) | 0x3;
            Some((a & (a + 1), a | (a + 1)))
        }
        _ => None,
    }
}

/// Permission mask granted by a matching entry.
fn entry_perms(cfg: u8, mode: u8, mml: bool) -> u8 {
    let rwx = PMP_R | PMP_W | PMP_X;
    if !mml {
        let mut allowed = rwx;
        if mode != PRV_M || cfg & PMP_L != 0 {
            allowed &= cfg;
        }
        return allowed;
    }

    // Smepmp extended truth table, keyed by the 4-bit operation
    // {L, R, W, X}.
    let op = ((cfg & PMP_L) >> 4) | ((cfg & PMP_R) << 2) | (cfg & PMP_W) | ((cfg & PMP_X) >> 2);
    if mode == PRV_M {
        match op {
            2 | 3 | 14 => PMP_R | PMP_W,
            9 | 10 => PMP_X,
            11 | 13 => PMP_R | PMP_X,
            12 | 15 => PMP_R,
            _ => 0,
        }
    } else {
        match op {
            1 | 10 | 11 => PMP_X,
            2 | 4 | 15 => PMP_R,
            3 | 6 => PMP_R | PMP_W,
            5 => PMP_R | PMP_X,
            7 => PMP_R | PMP_W | PMP_X,
            _ => 0,
        }
    }
}

/// Privileges allowed when no PMP entry matches.
fn default_perms(mode: u8, access: AccessType, mseccfg: u64) -> u8 {
    if mseccfg & MSECCFG_MMWP != 0 {
        return 0;
    }
    if mseccfg & MSECCFG_MML != 0 {
        // MML: M-mode keeps read/write access but needs an explicit rule
        // to execute; every other mode is denied.
        if mode == PRV_M && !matches!(access, AccessType::Fetch) {
            return PMP_R | PMP_W;
        }
        return 0;
    }
    if mode == PRV_M {
        PMP_R | PMP_W | PMP_X
    } else {
        0
    }
}

/// Check whether `mode` may perform `access` to physical range
/// `[addr, addr + size)`.
pub fn check(csr: &CsrFile, mode: u8, addr: u64, size: u64, access: AccessType) -> bool {
    let mseccfg = csr.mseccfg;
    let mml = mseccfg & MSECCFG_MML != 0;
    let need = pmp_access_bit(access);
    let end = addr.wrapping_add(size.max(1) - 1);

    let mut num_rules = 0usize;
    for i in 0..NUM_ENTRIES {
        if a_field(cfg_byte(&csr.pmpcfg, i)) != A_OFF {
            num_rules += 1;
        }
    }
    if num_rules == 0 {
        return default_perms(mode, access, mseccfg) & need == need;
    }

    for i in 0..NUM_ENTRIES {
        let cfg = cfg_byte(&csr.pmpcfg, i);
        let prev = if i == 0 { 0 } else { csr.pmpaddr[i - 1] };
        let Some((start, range_end)) = entry_range(cfg, csr.pmpaddr[i], prev) else {
            continue;
        };
        let start_in = addr >= start && addr <= range_end;
        let end_in = end >= start && end <= range_end;
        if start_in != end_in {
            // Partially inside a rule: deny, as required by the PMP spec.
            return false;
        }
        if start_in && end_in {
            return entry_perms(cfg, mode, mml) & need == need;
        }
    }

    default_perms(mode, access, mseccfg) & need == need
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csr::{CsrFile, PRV_M, PRV_S, PRV_U};

    fn csr_with_region(base: u64, size: u64, cfg: u8) -> CsrFile {
        let mut csr = CsrFile::new(0);
        csr.pmpaddr[0] = (base >> 2) | (size / 8 - 1);
        csr.pmpcfg[0] = cfg as u64;
        csr
    }

    #[test]
    fn no_rules_deny_su_allow_m() {
        let csr = CsrFile::new(0);
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Load));
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Fetch));
        assert!(!check(&csr, PRV_S, 0x1000, 4, AccessType::Load));
        assert!(!check(&csr, PRV_U, 0x1000, 4, AccessType::Load));
    }

    #[test]
    fn napot_region_permissions() {
        // 4 KiB at 0x1000, NAPOT, R/W, unlocked: applies to S/U only.
        let csr = csr_with_region(0x1000, 4096, 0x1B);
        assert!(check(&csr, PRV_S, 0x1000, 8, AccessType::Load));
        assert!(check(&csr, PRV_S, 0x1FF8, 8, AccessType::Store));
        assert!(!check(&csr, PRV_S, 0x1FFC, 8, AccessType::Load)); // partial
        assert!(!check(&csr, PRV_S, 0x2000, 8, AccessType::Load));
        assert!(check(&csr, PRV_M, 0x2000, 8, AccessType::Load));
        // Execute is not granted by R/W.
        assert!(!check(&csr, PRV_S, 0x1000, 2, AccessType::Fetch));
    }

    #[test]
    fn locked_entry_restricts_m_mode() {
        // A locked R-only region at 0x2000 limits M-mode stores.
        let mut csr = CsrFile::new(0);
        csr.pmpaddr[1] = ((0x2000 >> 2) | (4096 / 8 - 1)) as u64;
        csr.pmpcfg[0] = 0x99u64 << 8; // entry 1: NAPOT | R | L
        assert!(check(&csr, PRV_M, 0x2000, 8, AccessType::Load));
        assert!(!check(&csr, PRV_M, 0x2000, 8, AccessType::Store));
        // L=0 entries do not apply to M-mode.
        let csr = csr_with_region(0x1000, 4096, 0x1B);
        assert!(check(&csr, PRV_M, 0x1000, 8, AccessType::Store));
        assert!(check(&csr, PRV_S, 0x1000, 8, AccessType::Store));
    }

    #[test]
    fn mml_execute_only_region() {
        let mut csr = csr_with_region(0x1000, 4096, 0x18 | 0x80 | 0x04); // L|X
        csr.mseccfg = MSECCFG_MML;
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Fetch));
        assert!(!check(&csr, PRV_M, 0x1000, 4, AccessType::Load));
        assert!(!check(&csr, PRV_S, 0x1000, 4, AccessType::Fetch));
    }

    #[test]
    fn mml_shared_region() {
        // L=1, R=0, W=1, X=1 shared RWX region.
        let mut csr = csr_with_region(0x1000, 4096, 0x18 | 0x80 | 0x02 | 0x04);
        csr.mseccfg = MSECCFG_MML;
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Fetch));
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Load));
        assert!(!check(&csr, PRV_M, 0x1000, 4, AccessType::Store));
        assert!(check(&csr, PRV_S, 0x1000, 4, AccessType::Fetch));
        assert!(!check(&csr, PRV_S, 0x1000, 4, AccessType::Load));
        assert!(!check(&csr, PRV_S, 0x1000, 4, AccessType::Store));
    }

    #[test]
    fn mml_default_policy() {
        let mut csr = CsrFile::new(0);
        csr.mseccfg = MSECCFG_MML;
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Load));
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Store));
        assert!(!check(&csr, PRV_M, 0x1000, 4, AccessType::Fetch));
        assert!(!check(&csr, PRV_S, 0x1000, 4, AccessType::Load));
    }

    #[test]
    fn mmwp_default_denies_everything() {
        let mut csr = CsrFile::new(0);
        csr.mseccfg = MSECCFG_MMWP;
        assert!(!check(&csr, PRV_M, 0x1000, 4, AccessType::Load));
        assert!(!check(&csr, PRV_M, 0x1000, 4, AccessType::Fetch));
        assert!(!check(&csr, PRV_S, 0x1000, 4, AccessType::Load));

        // A locked rule still grants access inside its range.
        let mut csr = csr_with_region(0x1000, 4096, 0x9F);
        csr.mseccfg = MSECCFG_MMWP;
        assert!(check(&csr, PRV_M, 0x1000, 4, AccessType::Load));
        assert!(!check(&csr, PRV_M, 0x2000, 4, AccessType::Load));
    }

    #[test]
    fn locked_entries_resist_writes() {
        let mut csr = csr_with_region(0x1000, 4096, 0x9F);
        let old = csr.pmpcfg[0];
        assert!(!write_cfg(csr.mseccfg, &mut csr.pmpcfg, 0, 0x00));
        assert_eq!(csr.pmpcfg[0], old);
        assert!(!write_addr(
            csr.mseccfg,
            &csr.pmpcfg,
            &mut csr.pmpaddr,
            0,
            0
        ));
    }

    #[test]
    fn rlb_bypasses_locked_entries() {
        let mut csr = CsrFile::new(0);
        assert!(write_mseccfg(&mut csr.mseccfg, &csr.pmpcfg, MSECCFG_RLB));
        assert!(write_cfg(csr.mseccfg, &mut csr.pmpcfg, 0, 0x9F));
        assert_eq!(cfg_byte(&csr.pmpcfg, 0), 0x9F);
        assert!(write_cfg(csr.mseccfg, &mut csr.pmpcfg, 0, 0x00));
        assert_eq!(cfg_byte(&csr.pmpcfg, 0), 0);
    }

    #[test]
    fn mml_mmwp_are_sticky() {
        let mut csr = CsrFile::new(0);
        assert!(write_mseccfg(&mut csr.mseccfg, &csr.pmpcfg, MSECCFG_MML));
        assert!(!write_mseccfg(&mut csr.mseccfg, &csr.pmpcfg, 0));
        assert_eq!(csr.mseccfg & MSECCFG_MML, MSECCFG_MML);
        assert!(write_mseccfg(&mut csr.mseccfg, &csr.pmpcfg, MSECCFG_MMWP));
        assert!(!write_mseccfg(&mut csr.mseccfg, &csr.pmpcfg, 0));
        assert_eq!(
            csr.mseccfg & (MSECCFG_MML | MSECCFG_MMWP),
            MSECCFG_MML | MSECCFG_MMWP
        );
    }

    #[test]
    fn rlb_cannot_be_enabled_after_locking() {
        let mut csr = csr_with_region(0x1000, 4096, 0x9F);
        assert!(!write_mseccfg(&mut csr.mseccfg, &csr.pmpcfg, MSECCFG_RLB));
        assert_eq!(csr.mseccfg & MSECCFG_RLB, 0);
    }

    #[test]
    fn reserved_rw_encoding_ignored_without_mml() {
        let mut csr = CsrFile::new(0);
        // A=NAPOT, R=0, W=1 is reserved unless MML is set.
        assert!(!write_cfg(csr.mseccfg, &mut csr.pmpcfg, 0, 0x1A));
        assert_eq!(cfg_byte(&csr.pmpcfg, 0), 0);
        csr.mseccfg = MSECCFG_MML;
        assert!(write_cfg(csr.mseccfg, &mut csr.pmpcfg, 0, 0x1A));
        assert_eq!(cfg_byte(&csr.pmpcfg, 0), 0x1A);
    }
}
