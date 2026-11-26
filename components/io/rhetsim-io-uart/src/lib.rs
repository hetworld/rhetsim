//! Minimal 16550-style UART component.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use rhetsim_core::error::OmniResult;
use rhetsim_core::irq::IrqLine;
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

// IER interrupt-enable bits
const IER_RDI:  u8 = 1 << 0; // received data available
const IER_THRI: u8 = 1 << 1; // transmitter holding register empty
const IER_RLSI: u8 = 1 << 2; // receiver line status
const IER_MSI:  u8 = 1 << 3; // modem status

// IIR interrupt-identification values
const IIR_NONE: u8 = 0x01;
const IIR_THRE: u8 = 0x02;
const IIR_RDA:  u8 = 0x04;
const IIR_LSR:  u8 = 0x06;

// LSR error bits (not modeled, but kept for the interrupt condition)
const LSR_ERR_MASK: u8 = 0x1E;

pub struct Uart {
    id: UnitId,
    output: Mutex<Vec<u8>>,
    input: Mutex<VecDeque<u8>>,
    irq_line: Mutex<Option<IrqLine>>,
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
            irq_line: Mutex::new(None),
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
        {
            let mut fifo = self.input.lock().expect("uart input lock");
            fifo.extend(data);
        }
        self.update_irq();
    }

    pub fn has_input(&self) -> bool {
        !self.input.lock().expect("uart input lock").is_empty()
    }

    pub fn set_console(&self, enable: bool) {
        self.console.store(enable, Ordering::Relaxed);
    }

    /// Connect the UART interrupt output to an interrupt controller.
    pub fn set_irq_line(&self, line: IrqLine) {
        *self.irq_line.lock().expect("uart irq lock") = Some(line);
        self.update_irq();
    }

    fn irq_asserted(&self) -> bool {
        let ier = self.ier.load(Ordering::Relaxed);
        let lsr = self.lsr.load(Ordering::Relaxed);

        if ier & IER_RLSI != 0 && lsr & LSR_ERR_MASK != 0 {
            return true;
        }
        if ier & IER_RDI != 0 && self.has_input() {
            return true;
        }
        if ier & IER_THRI != 0 && lsr & LSR_THRE != 0 {
            return true;
        }
        // Modem status interrupts are not modeled.
        false
    }

    fn iir_value(&self) -> u8 {
        let ier = self.ier.load(Ordering::Relaxed);
        let lsr = self.lsr.load(Ordering::Relaxed);

        if ier & IER_RLSI != 0 && lsr & LSR_ERR_MASK != 0 {
            IIR_LSR
        } else if ier & IER_RDI != 0 && self.has_input() {
            IIR_RDA
        } else if ier & IER_THRI != 0 && lsr & LSR_THRE != 0 {
            IIR_THRE
        } else {
            IIR_NONE
        }
    }

    fn update_irq(&self) {
        let asserted = self.irq_asserted();
        let line = self.irq_line.lock().expect("uart irq lock").clone();
        if let Some(line) = line {
            let _ = if asserted { line.raise() } else { line.lower() };
        }
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
        self.update_irq();
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
                let byte = {
                    let mut fifo = self.input.lock().expect("uart input lock");
                    fifo.pop_front().unwrap_or(0)
                };
                self.update_irq();
                Ok(byte as u64)
            }
            IER if dlab  => Ok(self.dlm.load(Ordering::Relaxed) as u64),
            IER          => Ok(self.ier.load(Ordering::Relaxed) as u64),
            IIR          => Ok(self.iir_value() as u64),
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
                }
            }
            IER if dlab  => { self.dlm.store(v, Ordering::Relaxed); }
            IER          => {
                self.ier.store(v, Ordering::Relaxed);
                self.update_irq();
            }
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
    use std::sync::Arc;
    use rhetsim_core::irq::IrqSink;

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

    struct MockSink {
        asserted: AtomicBool,
    }

    impl MockSink {
        fn new() -> Self {
            Self { asserted: AtomicBool::new(false) }
        }
    }

    impl IrqSink for MockSink {
        fn raise(&self, _irq_num: u32) -> OmniResult<()> {
            self.asserted.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn lower(&self, _irq_num: u32) -> OmniResult<()> {
            self.asserted.store(false, Ordering::Relaxed);
            Ok(())
        }
        fn irq_count(&self) -> u32 { 1 }
    }

    #[test]
    fn uart_irq_line_rx() {
        let uart = Uart::new("uart0");
        let sink = Arc::new(MockSink::new());
        uart.set_irq_line(IrqLine::new(sink.clone(), 10));

        uart.write(IER, MemAccessSize::U8, IER_RDI as u64).unwrap();
        assert!(!sink.asserted.load(Ordering::Relaxed));

        uart.push_input(b"X");
        assert!(sink.asserted.load(Ordering::Relaxed));
        assert_eq!(uart.read(IIR, MemAccessSize::U8).unwrap(), IIR_RDA as u64);

        assert_eq!(uart.read(RBR, MemAccessSize::U8).unwrap(), b'X' as u64);
        assert!(!sink.asserted.load(Ordering::Relaxed));
    }

    #[test]
    fn uart_irq_line_thre() {
        let uart = Uart::new("uart0");
        let sink = Arc::new(MockSink::new());
        uart.set_irq_line(IrqLine::new(sink.clone(), 10));

        uart.write(IER, MemAccessSize::U8, IER_THRI as u64).unwrap();
        assert!(sink.asserted.load(Ordering::Relaxed));
        assert_eq!(uart.read(IIR, MemAccessSize::U8).unwrap(), IIR_THRE as u64);

        uart.write(IER, MemAccessSize::U8, 0).unwrap();
        assert!(!sink.asserted.load(Ordering::Relaxed));
        assert_eq!(uart.read(IIR, MemAccessSize::U8).unwrap(), IIR_NONE as u64);
    }
}
