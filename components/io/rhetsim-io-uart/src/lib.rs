//! Minimal 16550-style UART component.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use rhetsim_core::error::OmniResult;
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::mem::{MemTarget, MemAccessSize};

const THR: u64 = 0x00;
const RBR: u64 = 0x00;
const LSR: u64 = 0x05;

const LSR_THRE: u8 = 1 << 5;
const LSR_TEMT: u8 = 1 << 6;
const LSR_RESET: u8 = LSR_THRE | LSR_TEMT;

pub struct Uart {
    id: UnitId,
    output: Mutex<Vec<u8>>,
    lsr: AtomicU8,
}

impl Uart {
    pub fn new(id: &str) -> Self {
        Self {
            id: UnitId::new(id),
            output: Mutex::new(Vec::new()),
            lsr: AtomicU8::new(LSR_RESET),
        }
    }

    pub fn take_output(&self) -> Vec<u8> {
        let mut buf = self.output.lock().expect("uart output lock");
        std::mem::take(&mut *buf)
    }
}

impl Unit for Uart {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "uart-16550" }
    fn caps(&self) -> UnitCaps { UnitCaps::ADDRESSABLE | UnitCaps::IRQ_SOURCE }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }

    fn reset(&mut self) -> OmniResult<()> {
        self.output.lock().expect("uart output lock").clear();
        self.lsr.store(LSR_RESET, Ordering::Relaxed);
        Ok(())
    }
}

impl MemTarget for Uart {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { 0 }
    fn size(&self) -> u64 { 8 }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        match offset {
            RBR => Ok(0),
            LSR => Ok(self.lsr.load(Ordering::Relaxed) as u64),
            _ => Ok(0),
        }
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        if offset == THR {
            let ch = val as u8;
            self.output.lock().expect("uart output lock").push(ch);
            eprint!("{}", ch as char);
        }
        Ok(())
    }
}

pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uart_write_and_read() {
        let uart = Uart::new("uart0");

        uart.write(THR, MemAccessSize::U8, b'H' as u64).unwrap();
        uart.write(THR, MemAccessSize::U8, b'i' as u64).unwrap();

        let lsr = uart.read(LSR, MemAccessSize::U8).unwrap();
        assert_eq!(lsr, LSR_RESET as u64);

        let out = uart.take_output();
        assert_eq!(out, b"Hi");
    }

    #[test]
    fn uart_reset_clears_output() {
        let mut uart = Uart::new("uart0");
        uart.write(THR, MemAccessSize::U8, b'X' as u64).unwrap();
        assert_eq!(uart.take_output(), b"X");

        Unit::reset(&mut uart).unwrap();
        assert!(uart.take_output().is_empty());
    }
}
