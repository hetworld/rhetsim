//! Minimal 16550-style UART component.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use rhetsim_core::error::OmniResult;
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::mem::{MemTarget, MemAccessSize};

const THR: u64 = 0x00;
const RBR: u64 = 0x00;
const IER: u64 = 0x01;
const IIR: u64 = 0x02;
const LCR: u64 = 0x03;
const MCR: u64 = 0x04;
const LSR: u64 = 0x05;
const MSR: u64 = 0x06;
const SCR: u64 = 0x07;

const LSR_DR:   u8 = 1 << 0;
const LSR_THRE: u8 = 1 << 5;
const LSR_TEMT: u8 = 1 << 6;
const LSR_RESET: u8 = LSR_THRE | LSR_TEMT;

const LCR_DLAB: u8 = 0x80;

pub struct Uart {
    id: UnitId,
    output: Mutex<Vec<u8>>,
    input: Mutex<VecDeque<u8>>,
    console: AtomicBool,
    lsr: AtomicU8,
    ier: AtomicU8,
    lcr: AtomicU8,
    mcr: AtomicU8,
    scr: AtomicU8,
    dll: AtomicU8,
    dlm: AtomicU8,
}

impl Uart {
    pub fn new(id: &str) -> Self {
        Self {
            id: UnitId::new(id),
            output: Mutex::new(Vec::new()),
            input: Mutex::new(VecDeque::new()),
            console: AtomicBool::new(false),
            lsr: AtomicU8::new(LSR_RESET),
            ier: AtomicU8::new(0),
            lcr: AtomicU8::new(0),
            mcr: AtomicU8::new(0),
            scr: AtomicU8::new(0),
            dll: AtomicU8::new(0),
            dlm: AtomicU8::new(0),
        }
    }

    pub fn take_output(&self) -> Vec<u8> {
        let mut buf = self.output.lock().expect("uart output lock");
        std::mem::take(&mut *buf)
    }

    pub fn push_input(&self, data: &[u8]) {
        let mut fifo = self.input.lock().expect("uart input lock");
        fifo.extend(data);
    }

    pub fn has_input(&self) -> bool {
        !self.input.lock().expect("uart input lock").is_empty()
    }

    pub fn set_console(&self, enable: bool) {
        self.console.store(enable, Ordering::Relaxed);
    }
}

impl Unit for Uart {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "uart-16550" }
    fn caps(&self) -> UnitCaps { UnitCaps::ADDRESSABLE | UnitCaps::IRQ_SOURCE }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }

    fn reset(&mut self) -> OmniResult<()> {
        self.output.lock().expect("uart output lock").clear();
        self.input.lock().expect("uart input lock").clear();
        self.lsr.store(LSR_RESET, Ordering::Relaxed);
        self.ier.store(0, Ordering::Relaxed);
        self.lcr.store(0, Ordering::Relaxed);
        self.mcr.store(0, Ordering::Relaxed);
        self.scr.store(0, Ordering::Relaxed);
        self.dll.store(0, Ordering::Relaxed);
        self.dlm.store(0, Ordering::Relaxed);
        Ok(())
    }
}

impl MemTarget for Uart {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { 0 }
    fn size(&self) -> u64 { 0x100 }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        let dlab = self.lcr.load(Ordering::Relaxed) & LCR_DLAB != 0;
        match offset {
            RBR if dlab  => Ok(self.dll.load(Ordering::Relaxed) as u64),
            RBR          => {
                let mut fifo = self.input.lock().expect("uart input lock");
                Ok(fifo.pop_front().unwrap_or(0) as u64)
            }
            IER if dlab  => Ok(self.dlm.load(Ordering::Relaxed) as u64),
            IER          => Ok(self.ier.load(Ordering::Relaxed) as u64),
            IIR          => Ok(0x01),
            LCR          => Ok(self.lcr.load(Ordering::Relaxed) as u64),
            MCR          => Ok(self.mcr.load(Ordering::Relaxed) as u64),
            LSR          => {
                let base = self.lsr.load(Ordering::Relaxed);
                let dr = if self.has_input() { LSR_DR } else { 0 };
                Ok((base | dr) as u64)
            }
            MSR          => Ok(0),
            SCR          => Ok(self.scr.load(Ordering::Relaxed) as u64),
            _            => Ok(0),
        }
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        let v = val as u8;
        let dlab = self.lcr.load(Ordering::Relaxed) & LCR_DLAB != 0;
        match offset {
            THR if dlab  => { self.dll.store(v, Ordering::Relaxed); }
            THR          => {
                if self.console.load(Ordering::Relaxed) {
                    let _ = std::io::stdout().write_all(&[v]);
                    let _ = std::io::stdout().flush();
                } else {
                    self.output.lock().expect("uart output lock").push(v);
                    eprint!("{}", v as char);
                }
            }
            IER if dlab  => { self.dlm.store(v, Ordering::Relaxed); }
            IER          => { self.ier.store(v, Ordering::Relaxed); }
            IIR          => {} // FCR write, ignore
            LCR          => { self.lcr.store(v, Ordering::Relaxed); }
            MCR          => { self.mcr.store(v, Ordering::Relaxed); }
            SCR          => { self.scr.store(v, Ordering::Relaxed); }
            _            => {}
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
