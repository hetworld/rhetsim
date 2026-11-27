//! DMA engine — bulk memory transfer via MemBus
//!
//! Register map (offsets from base):
//!   0x00  SRC_ADDR  (u64, r/w)  — source address
//!   0x08  DST_ADDR  (u64, r/w)  — destination address
//!   0x10  LENGTH    (u32, r/w)  — transfer length in bytes
//!   0x14  CTRL      (u32, w)    — bit 0: start
//!   0x18  STATUS    (u32, r)    — bit 0: done, bit 1: error
//!   0x1C  IRQ_EN    (u32, r/w)  — bit 0: raise IRQ on completion

pub fn register() {}

use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::error::OmniResult;
use rhetsim_core::irq::IrqLine;
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::sync::{Arc, RwLock};

const REG_SIZE: u64 = 0x20;

struct DmaState {
    src_addr: u64,
    dst_addr: u64,
    length: u32,
    status: u32,
    irq_en: u32,
}

pub struct DmaEngine {
    id: UnitId,
    base: u64,
    bus: Arc<MemBus>,
    state: RwLock<DmaState>,
    irq_line: Option<IrqLine>,
}

impl DmaEngine {
    pub fn new(id: &str, base: u64, bus: Arc<MemBus>) -> Self {
        Self {
            id: UnitId::new(id),
            base,
            bus,
            state: RwLock::new(DmaState {
                src_addr: 0,
                dst_addr: 0,
                length: 0,
                status: 0,
                irq_en: 0,
            }),
            irq_line: None,
        }
    }

    pub fn attach_irq(&mut self, line: IrqLine) {
        self.irq_line = Some(line);
    }

    pub fn is_done(&self) -> bool {
        self.state.read().unwrap().status & 1 != 0
    }

    fn do_transfer(&self) {
        let (src, dst, len) = {
            let mut s = self.state.write().unwrap();
            s.status = 0;
            (s.src_addr, s.dst_addr, s.length as u64)
        };

        let mut error = false;
        let full_words = len / 8;
        let tail = len % 8;

        for i in 0..full_words {
            let off = i * 8;
            match self.bus.read(src + off, MemAccessSize::U64, 0) {
                Ok(val) => {
                    if self
                        .bus
                        .write(dst + off, MemAccessSize::U64, val, 0)
                        .is_err()
                    {
                        error = true;
                        break;
                    }
                }
                Err(_) => {
                    error = true;
                    break;
                }
            }
        }

        if !error {
            let base_off = full_words * 8;
            for i in 0..tail {
                match self.bus.read(src + base_off + i, MemAccessSize::U8, 0) {
                    Ok(val) => {
                        if self
                            .bus
                            .write(dst + base_off + i, MemAccessSize::U8, val, 0)
                            .is_err()
                        {
                            error = true;
                            break;
                        }
                    }
                    Err(_) => {
                        error = true;
                        break;
                    }
                }
            }
        }

        let irq_en = {
            let mut s = self.state.write().unwrap();
            s.status = 1 | if error { 2 } else { 0 };
            s.irq_en
        };

        if irq_en & 1 != 0 {
            if let Some(ref line) = self.irq_line {
                let _ = line.raise();
            }
        }
    }
}

impl Unit for DmaEngine {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "dma-engine"
    }
    fn caps(&self) -> UnitCaps {
        UnitCaps::ADDRESSABLE
    }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> {
        Some(self)
    }

    fn reset(&mut self) -> OmniResult<()> {
        let mut s = self.state.write().unwrap();
        s.src_addr = 0;
        s.dst_addr = 0;
        s.length = 0;
        s.status = 0;
        s.irq_en = 0;
        Ok(())
    }
}

impl MemTarget for DmaEngine {
    fn target_id(&self) -> u32 {
        0
    }
    fn base(&self) -> u64 {
        self.base
    }
    fn size(&self) -> u64 {
        REG_SIZE
    }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        let s = self.state.read().unwrap();
        Ok(match offset {
            0x00 => s.src_addr,
            0x08 => s.dst_addr,
            0x10 => s.length as u64,
            0x18 => s.status as u64,
            0x1C => s.irq_en as u64,
            _ => 0,
        })
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        match offset {
            0x00 => {
                self.state.write().unwrap().src_addr = val;
            }
            0x08 => {
                self.state.write().unwrap().dst_addr = val;
            }
            0x10 => {
                self.state.write().unwrap().length = val as u32;
            }
            0x14 => {
                if val & 1 != 0 {
                    self.do_transfer();
                }
            }
            0x1C => {
                self.state.write().unwrap().irq_en = val as u32;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhetsim_core::mem::MemAttrs;

    fn make_bus_with_dram(base: u64, size: usize) -> Arc<MemBus> {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram", size, base));
        let mut bus = MemBus::new("test-bus");
        bus.map(dram as Arc<dyn MemTarget>, base, MemAttrs::rw(), None)
            .unwrap();
        Arc::new(bus)
    }

    #[test]
    fn test_register_readback() {
        let bus = make_bus_with_dram(0x1000_0000, 4096);
        let dma = DmaEngine::new("dma0", 0x0, bus);

        dma.write(0x00, MemAccessSize::U64, 0xDEAD_BEEF).unwrap();
        dma.write(0x08, MemAccessSize::U64, 0xCAFE_BABE).unwrap();
        dma.write(0x10, MemAccessSize::U32, 256).unwrap();
        dma.write(0x1C, MemAccessSize::U32, 1).unwrap();

        assert_eq!(dma.read(0x00, MemAccessSize::U64).unwrap(), 0xDEAD_BEEF);
        assert_eq!(dma.read(0x08, MemAccessSize::U64).unwrap(), 0xCAFE_BABE);
        assert_eq!(dma.read(0x10, MemAccessSize::U32).unwrap(), 256);
        assert_eq!(dma.read(0x1C, MemAccessSize::U32).unwrap(), 1);
    }

    #[test]
    fn test_dma_transfer() {
        let dram_base: u64 = 0;
        let bus = make_bus_with_dram(dram_base, 0x1000);

        // Write source data at offset 0x000
        let src_data: Vec<u64> = vec![0x1111, 0x2222, 0x3333, 0x4444];
        for (i, &val) in src_data.iter().enumerate() {
            bus.write(dram_base + (i * 8) as u64, MemAccessSize::U64, val, 0)
                .unwrap();
        }

        let dma = DmaEngine::new("dma0", 0xF000_0000, bus.clone());

        // Configure: copy 32 bytes from 0x000 to 0x100
        dma.write(0x00, MemAccessSize::U64, dram_base).unwrap();
        dma.write(0x08, MemAccessSize::U64, dram_base + 0x100)
            .unwrap();
        dma.write(0x10, MemAccessSize::U32, 32).unwrap();

        // Trigger
        dma.write(0x14, MemAccessSize::U32, 1).unwrap();

        // Verify done
        assert!(dma.is_done());
        assert_eq!(dma.read(0x18, MemAccessSize::U32).unwrap(), 1); // done=1, error=0

        // Verify destination data
        for (i, &expected) in src_data.iter().enumerate() {
            let val = bus
                .read(dram_base + 0x100 + (i * 8) as u64, MemAccessSize::U64, 0)
                .unwrap();
            assert_eq!(val, expected, "mismatch at word {}", i);
        }
    }

    #[test]
    fn test_dma_transfer_non_aligned() {
        let dram_base: u64 = 0;
        let bus = make_bus_with_dram(dram_base, 0x1000);

        // Write 5 bytes of source data
        for i in 0u64..5 {
            bus.write(dram_base + i, MemAccessSize::U8, 0x10 + i, 0)
                .unwrap();
        }

        let dma = DmaEngine::new("dma0", 0xF000_0000, bus.clone());
        dma.write(0x00, MemAccessSize::U64, dram_base).unwrap();
        dma.write(0x08, MemAccessSize::U64, dram_base + 0x200)
            .unwrap();
        dma.write(0x10, MemAccessSize::U32, 5).unwrap();
        dma.write(0x14, MemAccessSize::U32, 1).unwrap();

        assert!(dma.is_done());
        for i in 0u64..5 {
            let val = bus
                .read(dram_base + 0x200 + i, MemAccessSize::U8, 0)
                .unwrap();
            assert_eq!(val, 0x10 + i, "byte {} mismatch", i);
        }
    }

    #[test]
    fn test_irq_on_completion() {
        use std::sync::atomic::{AtomicU32, Ordering};

        struct MockSink {
            raised: AtomicU32,
        }
        impl rhetsim_core::irq::IrqSink for MockSink {
            fn raise(&self, irq_num: u32) -> OmniResult<()> {
                self.raised.store(irq_num + 1, Ordering::SeqCst);
                Ok(())
            }
            fn lower(&self, _: u32) -> OmniResult<()> {
                Ok(())
            }
            fn irq_count(&self) -> u32 {
                1
            }
        }

        let bus = make_bus_with_dram(0, 0x1000);
        let sink = Arc::new(MockSink {
            raised: AtomicU32::new(0),
        });
        let line = IrqLine::new(sink.clone(), 7);

        let mut dma = DmaEngine::new("dma0", 0xF000_0000, bus.clone());
        dma.attach_irq(line);

        // Write a byte of source data
        bus.write(0, MemAccessSize::U8, 0xAB, 0).unwrap();

        // Enable IRQ, configure 1-byte transfer, trigger
        dma.write(0x1C, MemAccessSize::U32, 1).unwrap();
        dma.write(0x00, MemAccessSize::U64, 0).unwrap();
        dma.write(0x08, MemAccessSize::U64, 0x100).unwrap();
        dma.write(0x10, MemAccessSize::U32, 1).unwrap();
        dma.write(0x14, MemAccessSize::U32, 1).unwrap();

        assert!(dma.is_done());
        // IRQ 7 was raised → stored as 7+1=8
        assert_eq!(sink.raised.load(Ordering::SeqCst), 8);
    }
}
