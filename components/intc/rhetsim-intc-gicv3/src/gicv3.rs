//! GICv3 Composite Unit
//!
//! Memory map (combined GICD + GICR regions):
//!   base + 0x0000_0000  GICD (64 KiB)
//!   base + 0x0001_0000  GICR PE0 RD_base (64 KiB)
//!   base + 0x0002_0000  GICR PE0 SGI_base (64 KiB)
//!   base + 0x0003_0000  GICR PE1 RD_base (64 KiB)
//!   base + 0x0004_0000  GICR PE1 SGI_base (64 KiB)
//!   ... (2 × 64 KiB per PE)
//!
//! Total size: 0x10000 + num_cpus * 0x20000

use crate::gicc::Gicc;
use crate::gicd::Gicd;
use crate::gicr::Gicr;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::irq::IrqSink;
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::sync::RwLock;

const GICD_SIZE: u64 = 0x10000;
const GICR_FRAME_SIZE: u64 = 0x20000; // RD_base + SGI_base, each 64 KiB

pub struct Gicv3 {
    id: UnitId,
    base: u64,
    num_cpus: usize,
    num_spis: usize,
    gicd: RwLock<Gicd>,
    gicrs: Vec<Gicr>,
    giccs: RwLock<Vec<Gicc>>,
}

impl Gicv3 {
    pub fn new(id: &str, num_cpus: usize, num_spis: usize, base: u64) -> Self {
        assert!(num_cpus > 0 && num_cpus <= 256);
        let gicrs: Vec<Gicr> = (0..num_cpus).map(|i| Gicr::new(i as u8)).collect();
        let giccs: Vec<Gicc> = (0..num_cpus).map(|i| Gicc::new(i as u8)).collect();
        Self {
            id: UnitId::new(id),
            base,
            num_cpus,
            num_spis,
            gicd: RwLock::new(Gicd::new(num_spis)),
            gicrs,
            giccs: RwLock::new(giccs),
        }
    }

    fn total_size(&self) -> u64 {
        GICD_SIZE + self.num_cpus as u64 * GICR_FRAME_SIZE
    }

    /// Check if CPU `cpu` has a pending interrupt that should be signaled.
    /// Returns the INTID if an interrupt is pending with priority < PMR.
    pub fn pending_irq(&self, cpu: usize) -> Option<u32> {
        if cpu >= self.num_cpus {
            return None;
        }
        let giccs = self.giccs.read().unwrap();
        if !giccs[cpu].group1_enabled() {
            return None;
        }
        let pmr = giccs[cpu].pmr();

        // Check redistributor (SGI/PPI) first
        if let Some((intid, prio)) = self.gicrs[cpu].highest_pending() {
            if prio < pmr {
                return Some(intid as u32);
            }
        }
        // Then check distributor (SPI)
        let gicd = self.gicd.read().unwrap();
        if let Some((intid, prio)) = gicd.highest_pending_spi(cpu as u8) {
            if prio < pmr {
                return Some(intid as u32);
            }
        }
        None
    }

    /// Acknowledge interrupt (ICC_IAR1 read): returns INTID, marks active, clears pending.
    pub fn acknowledge(&self, cpu: usize) -> u32 {
        if cpu >= self.num_cpus {
            return 1023;
        } // spurious
        let giccs = self.giccs.read().unwrap();
        if !giccs[cpu].group1_enabled() {
            return 1023;
        }
        let pmr = giccs[cpu].pmr();
        drop(giccs);

        // SGI/PPI
        if let Some((intid, prio)) = self.gicrs[cpu].highest_pending() {
            if prio < pmr {
                self.gicrs[cpu].set_pending(intid, false);
                self.gicrs[cpu].set_active(intid, true);
                return intid as u32;
            }
        }
        // SPI
        let gicd = self.gicd.read().unwrap();
        if let Some((intid, prio)) = gicd.highest_pending_spi(cpu as u8) {
            if prio < pmr {
                gicd.set_spi_pending(intid, false);
                gicd.set_spi_active(intid, true);
                return intid as u32;
            }
        }
        1023 // spurious
    }

    /// End of interrupt (ICC_EOIR1 write): deactivates the interrupt.
    pub fn eoi(&self, cpu: usize, intid: u32) {
        let intid = intid as usize;
        if intid < 32 {
            if cpu < self.num_cpus {
                self.gicrs[cpu].set_active(intid, false);
            }
        } else if intid < 1020 {
            self.gicd.read().unwrap().set_spi_active(intid, false);
        }
    }

    /// Read ICC system register (called by CPU on MRS).
    /// sysreg encoding: op0=3, op1, CRn, CRm, op2 combined into a 16-bit key.
    pub fn read_icc_reg(&self, cpu: usize, reg: u16) -> u64 {
        if cpu >= self.num_cpus {
            return 0;
        }
        match reg {
            0xC660 => self.acknowledge(cpu) as u64, // ICC_IAR1_EL1
            0xC230 => self.giccs.read().unwrap()[cpu].pmr() as u64, // ICC_PMR_EL1
            0xC663 => self.giccs.read().unwrap()[cpu].bpr() as u64, // ICC_BPR1_EL1
            0xC667 => {
                // ICC_IGRPEN1_EL1
                if self.giccs.read().unwrap()[cpu].group1_enabled() {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// Write ICC system register (called by CPU on MSR).
    pub fn write_icc_reg(&self, cpu: usize, reg: u16, val: u64) {
        if cpu >= self.num_cpus {
            return;
        }
        match reg {
            0xC230 => self.giccs.write().unwrap()[cpu].set_pmr(val as u8),
            0xC663 => self.giccs.write().unwrap()[cpu].set_bpr(val as u8),
            0xC661 => self.eoi(cpu, val as u32), // ICC_EOIR1_EL1
            0xC667 => {
                // ICC_IGRPEN1_EL1
                self.giccs.write().unwrap()[cpu].set_group1_enabled(val & 1 != 0);
            }
            _ => {}
        }
    }

    pub fn configure_spi(&self, intid: usize, priority: u8, target_cpu: u8) {
        let mut gicd = self.gicd.write().unwrap();
        gicd.set_enabled(true);
        gicd.set_spi_enable(intid, true);
        gicd.set_spi_priority(intid, priority);
        gicd.set_spi_target(intid, target_cpu);
    }

    /// Trigger an SGI to a specific CPU.
    pub fn send_sgi(&self, target_cpu: usize, intid: usize) {
        if target_cpu < self.num_cpus && intid < 16 {
            self.gicrs[target_cpu].set_pending(intid, true);
        }
    }
}

impl Unit for Gicv3 {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "arm-gicv3"
    }
    fn caps(&self) -> UnitCaps {
        UnitCaps::ADDRESSABLE | UnitCaps::IRQ_SINK
    }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> {
        Some(self)
    }

    fn reset(&mut self) -> OmniResult<()> {
        self.gicd
            .write()
            .map_err(|e| OmniError::Internal(e.to_string()))?
            .reset();
        for r in &self.gicrs {
            r.reset();
        }
        for c in self
            .giccs
            .write()
            .map_err(|e| OmniError::Internal(e.to_string()))?
            .iter_mut()
        {
            c.reset();
        }
        Ok(())
    }
}

impl MemTarget for Gicv3 {
    fn target_id(&self) -> u32 {
        0
    }
    fn base(&self) -> u64 {
        self.base
    }
    fn size(&self) -> u64 {
        self.total_size()
    }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        if offset < GICD_SIZE {
            return Ok(self
                .gicd
                .read()
                .map_err(|e| OmniError::Internal(e.to_string()))?
                .read_reg(offset));
        }
        let gicr_offset = offset - GICD_SIZE;
        let cpu = (gicr_offset / GICR_FRAME_SIZE) as usize;
        let frame_offset = gicr_offset % GICR_FRAME_SIZE;
        if cpu < self.num_cpus {
            if frame_offset < 0x10000 {
                // RD_base frame
                match frame_offset {
                    0x0014 => {
                        // GICR_TYPER: last bit indicates if this is the last redistributor
                        let last = if cpu == self.num_cpus - 1 {
                            1u64 << 4
                        } else {
                            0
                        };
                        return Ok(last | ((cpu as u64) << 8));
                    }
                    _ => return Ok(0),
                }
            } else {
                // SGI_base frame
                let sgi_offset = frame_offset - 0x10000;
                return Ok(self.gicrs[cpu].read_sgi_reg(sgi_offset));
            }
        }
        Ok(0)
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        if offset < GICD_SIZE {
            self.gicd
                .write()
                .map_err(|e| OmniError::Internal(e.to_string()))?
                .write_reg(offset, val);
            return Ok(());
        }
        let gicr_offset = offset - GICD_SIZE;
        let cpu = (gicr_offset / GICR_FRAME_SIZE) as usize;
        let frame_offset = gicr_offset % GICR_FRAME_SIZE;
        if cpu < self.num_cpus && frame_offset >= 0x10000 {
            let sgi_offset = frame_offset - 0x10000;
            self.gicrs[cpu].write_sgi_reg(sgi_offset, val);
        }
        Ok(())
    }
}

impl IrqSink for Gicv3 {
    fn raise(&self, irq_num: u32) -> OmniResult<()> {
        let intid = irq_num as usize;
        if intid < 32 {
            return Err(OmniError::IrqNotFound(format!(
                "INTID {} is SGI/PPI, use send_sgi()",
                intid
            )));
        }
        if intid >= 32 + self.num_spis {
            return Err(OmniError::IrqNotFound(format!(
                "INTID {} out of range",
                intid
            )));
        }
        self.gicd
            .read()
            .map_err(|e| OmniError::Internal(e.to_string()))?
            .set_spi_pending(intid, true);
        Ok(())
    }

    fn lower(&self, irq_num: u32) -> OmniResult<()> {
        let intid = irq_num as usize;
        if intid < 32 || intid >= 32 + self.num_spis {
            return Err(OmniError::IrqNotFound(format!(
                "INTID {} out of range",
                intid
            )));
        }
        self.gicd
            .read()
            .map_err(|e| OmniError::Internal(e.to_string()))?
            .set_spi_pending(intid, false);
        Ok(())
    }

    fn irq_count(&self) -> u32 {
        self.num_spis as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spi_raise_and_acknowledge() {
        let gic = Gicv3::new("gic0", 1, 64, 0x0800_0000);
        // Enable distributor
        gic.gicd.write().unwrap().set_enabled(true);
        // Enable SPI 32, priority 0x40, routed to CPU 0
        gic.gicd.read().unwrap().set_spi_enable(32, true);
        gic.gicd.read().unwrap().set_spi_priority(32, 0x40);
        gic.gicd.read().unwrap().set_spi_target(32, 0);
        // Enable CPU interface, PMR = 0xFF
        gic.giccs.write().unwrap()[0].set_group1_enabled(true);
        gic.giccs.write().unwrap()[0].set_pmr(0xFF);
        // Raise SPI 32
        gic.raise(32).unwrap();
        // Should be pending
        assert_eq!(gic.pending_irq(0), Some(32));
        // Acknowledge
        assert_eq!(gic.acknowledge(0), 32);
        // Now active, not pending
        assert_eq!(gic.pending_irq(0), None);
        // EOI
        gic.eoi(0, 32);
        // Should be fully cleared
        assert_eq!(gic.pending_irq(0), None);
    }

    #[test]
    fn sgi_pending_and_ack() {
        let gic = Gicv3::new("gic0", 2, 32, 0x0800_0000);
        gic.giccs.write().unwrap()[0].set_group1_enabled(true);
        gic.giccs.write().unwrap()[0].set_pmr(0xFF);
        // Enable SGI 5 on CPU 0
        gic.gicrs[0].set_enable(5, true);
        gic.gicrs[0].set_priority(5, 0x20);
        // Send SGI
        gic.send_sgi(0, 5);
        assert_eq!(gic.pending_irq(0), Some(5));
        assert_eq!(gic.acknowledge(0), 5);
        assert_eq!(gic.pending_irq(0), None);
        gic.eoi(0, 5);
    }

    #[test]
    fn priority_ordering() {
        let gic = Gicv3::new("gic0", 1, 64, 0x0800_0000);
        gic.gicd.write().unwrap().set_enabled(true);
        gic.giccs.write().unwrap()[0].set_group1_enabled(true);
        gic.giccs.write().unwrap()[0].set_pmr(0xFF);
        // SPI 32 prio 0x80, SPI 33 prio 0x20
        for intid in [32, 33] {
            gic.gicd.read().unwrap().set_spi_enable(intid, true);
            gic.gicd.read().unwrap().set_spi_target(intid, 0);
        }
        gic.gicd.read().unwrap().set_spi_priority(32, 0x80);
        gic.gicd.read().unwrap().set_spi_priority(33, 0x20);
        gic.raise(32).unwrap();
        gic.raise(33).unwrap();
        // SPI 33 has lower priority value → higher priority
        assert_eq!(gic.acknowledge(0), 33);
        gic.eoi(0, 33);
        assert_eq!(gic.acknowledge(0), 32);
        gic.eoi(0, 32);
    }

    #[test]
    fn pmr_masks_low_priority() {
        let gic = Gicv3::new("gic0", 1, 32, 0x0800_0000);
        gic.gicd.write().unwrap().set_enabled(true);
        gic.giccs.write().unwrap()[0].set_group1_enabled(true);
        gic.giccs.write().unwrap()[0].set_pmr(0x40);
        gic.gicd.read().unwrap().set_spi_enable(32, true);
        gic.gicd.read().unwrap().set_spi_priority(32, 0x80);
        gic.gicd.read().unwrap().set_spi_target(32, 0);
        gic.raise(32).unwrap();
        // Priority 0x80 >= PMR 0x40 → masked
        assert_eq!(gic.pending_irq(0), None);
        assert_eq!(gic.acknowledge(0), 1023);
        // Lower priority value (higher priority)
        gic.gicd.read().unwrap().set_spi_priority(32, 0x20);
        assert_eq!(gic.pending_irq(0), Some(32));
    }

    #[test]
    fn mem_mapped_gicd_enable() {
        let gic = Gicv3::new("gic0", 1, 32, 0x0800_0000);
        // Write GICD_CTLR to enable
        gic.write(0x0000, MemAccessSize::U32, 1).unwrap();
        assert_eq!(gic.read(0x0000, MemAccessSize::U32).unwrap(), 1);
        // Write GICD_ISENABLER1 (SPI 32..63)
        gic.write(0x0104, MemAccessSize::U32, 0x1).unwrap(); // enable SPI 32
        assert!(gic.gicd.read().unwrap().is_spi_enabled(32));
    }

    #[test]
    fn mem_mapped_gicr_sgi() {
        let gic = Gicv3::new("gic0", 1, 32, 0x0800_0000);
        // GICR PE0 SGI_base = GICD_SIZE + 0x10000
        let sgi_base = GICD_SIZE + 0x10000;
        // Enable SGI 3 via GICR_ISENABLER0
        gic.write(sgi_base + 0x0100, MemAccessSize::U32, 1 << 3)
            .unwrap();
        assert!(gic.gicrs[0].is_enabled(3));
        // Set priority for SGI 3
        gic.write(sgi_base + 0x0400 + 3, MemAccessSize::U8, 0x30)
            .unwrap();
        assert_eq!(gic.gicrs[0].priority(3), 0x30);
    }

    #[test]
    fn icc_sysreg_read_write() {
        let gic = Gicv3::new("gic0", 1, 32, 0x0800_0000);
        // Write ICC_PMR_EL1
        gic.write_icc_reg(0, 0xC230, 0xF0);
        assert_eq!(gic.read_icc_reg(0, 0xC230), 0xF0);
        // Write ICC_IGRPEN1_EL1
        gic.write_icc_reg(0, 0xC667, 1);
        assert_eq!(gic.read_icc_reg(0, 0xC667), 1);
    }
}
