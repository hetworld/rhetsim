//! VirtIO MMIO console device — simplified transport v2 with direct data ports.
//!
//! Register map (VirtIO MMIO transport v2):
//!   0x000  MagicValue      (R)   0x74726976
//!   0x004  Version         (R)   2
//!   0x008  DeviceID        (R)   3 (console)
//!   0x00C  VendorID        (R)   0x554D4551
//!   0x010  DeviceFeatures  (R)
//!   0x020  DeviceFeaturesSel (W)
//!   0x030  DriverFeatures  (W)
//!   0x034  DriverFeaturesSel (W)
//!   0x044  QueueSel        (W)
//!   0x050  QueueReady      (RW)
//!   0x060  QueueNotify     (W)
//!   0x064  InterruptStatus (R)
//!   0x068  InterruptACK    (W)
//!   0x070  Status          (RW)
//!   0x100  TX data port    (W)   byte appended to tx_buf
//!   0x108  RX data port    (R)   byte popped from rx_buf (0 if empty)

pub fn register() {}

use rhetsim_core::error::OmniResult;
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::collections::VecDeque;
use std::sync::RwLock;

const VIRTIO_MAGIC: u32 = 0x74726976;
const VIRTIO_VERSION: u32 = 2;
const VIRTIO_DEVICE_ID_CONSOLE: u32 = 3;
const VIRTIO_VENDOR_ID: u32 = 0x554D4551;

const REG_MAGIC: u64 = 0x000;
const REG_VERSION: u64 = 0x004;
const REG_DEVICE_ID: u64 = 0x008;
const REG_VENDOR_ID: u64 = 0x00C;
const REG_DEVICE_FEATURES: u64 = 0x010;
const REG_DEVICE_FEATURES_SEL: u64 = 0x020;
const REG_DRIVER_FEATURES: u64 = 0x030;
const REG_DRIVER_FEATURES_SEL: u64 = 0x034;
const REG_QUEUE_SEL: u64 = 0x044;
const REG_QUEUE_READY: u64 = 0x050;
const REG_QUEUE_NOTIFY: u64 = 0x060;
const REG_INTERRUPT_STATUS: u64 = 0x064;
const REG_INTERRUPT_ACK: u64 = 0x068;
const REG_STATUS: u64 = 0x070;
const REG_TX_DATA: u64 = 0x100;
const REG_RX_DATA: u64 = 0x108;

const REG_SIZE: u64 = 0x200;

struct VirtioState {
    status: u32,
    interrupt_status: u32,
    queue_sel: u32,
    queue_ready: u32,
    driver_features: u32,
}

pub struct VirtioConsole {
    id: UnitId,
    base: u64,
    state: RwLock<VirtioState>,
    tx_buf: RwLock<Vec<u8>>,
    rx_buf: RwLock<VecDeque<u8>>,
}

impl VirtioConsole {
    pub fn new(id: &str, base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            base,
            state: RwLock::new(VirtioState {
                status: 0,
                interrupt_status: 0,
                queue_sel: 0,
                queue_ready: 0,
                driver_features: 0,
            }),
            tx_buf: RwLock::new(Vec::new()),
            rx_buf: RwLock::new(VecDeque::new()),
        }
    }

    pub fn tx_output(&self) -> Vec<u8> {
        self.tx_buf.read().unwrap().clone()
    }

    pub fn push_rx(&self, data: &[u8]) {
        let mut rx = self.rx_buf.write().unwrap();
        rx.extend(data);
        let mut st = self.state.write().unwrap();
        st.interrupt_status |= 1;
    }
}

impl Unit for VirtioConsole {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "virtio-console"
    }
    fn caps(&self) -> UnitCaps {
        UnitCaps::ADDRESSABLE
    }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> {
        Some(self)
    }

    fn reset(&mut self) -> OmniResult<()> {
        let mut st = self.state.write().unwrap();
        st.status = 0;
        st.interrupt_status = 0;
        st.queue_sel = 0;
        st.queue_ready = 0;
        st.driver_features = 0;
        self.tx_buf.write().unwrap().clear();
        self.rx_buf.write().unwrap().clear();
        Ok(())
    }
}

impl MemTarget for VirtioConsole {
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
        let st = self.state.read().unwrap();
        Ok(match offset {
            REG_MAGIC => VIRTIO_MAGIC as u64,
            REG_VERSION => VIRTIO_VERSION as u64,
            REG_DEVICE_ID => VIRTIO_DEVICE_ID_CONSOLE as u64,
            REG_VENDOR_ID => VIRTIO_VENDOR_ID as u64,
            REG_DEVICE_FEATURES => 0,
            REG_QUEUE_READY => st.queue_ready as u64,
            REG_INTERRUPT_STATUS => st.interrupt_status as u64,
            REG_STATUS => st.status as u64,
            REG_RX_DATA => {
                drop(st);
                let mut rx = self.rx_buf.write().unwrap();
                rx.pop_front().unwrap_or(0) as u64
            }
            _ => 0,
        })
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        match offset {
            REG_DEVICE_FEATURES_SEL | REG_DRIVER_FEATURES_SEL => {}
            REG_DRIVER_FEATURES => {
                self.state.write().unwrap().driver_features = val as u32;
            }
            REG_QUEUE_SEL => {
                self.state.write().unwrap().queue_sel = val as u32;
            }
            REG_QUEUE_READY => {
                self.state.write().unwrap().queue_ready = val as u32;
            }
            REG_QUEUE_NOTIFY => {}
            REG_INTERRUPT_ACK => {
                let mut st = self.state.write().unwrap();
                st.interrupt_status &= !(val as u32);
            }
            REG_STATUS => {
                self.state.write().unwrap().status = val as u32;
            }
            REG_TX_DATA => {
                self.tx_buf.write().unwrap().push(val as u8);
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_version_device_id() {
        let dev = VirtioConsole::new("vcon0", 0);
        assert_eq!(dev.read(REG_MAGIC, MemAccessSize::U32).unwrap(), 0x74726976);
        assert_eq!(dev.read(REG_VERSION, MemAccessSize::U32).unwrap(), 2);
        assert_eq!(dev.read(REG_DEVICE_ID, MemAccessSize::U32).unwrap(), 3);
        assert_eq!(
            dev.read(REG_VENDOR_ID, MemAccessSize::U32).unwrap(),
            0x554D4551
        );
    }

    #[test]
    fn test_tx_output() {
        let dev = VirtioConsole::new("vcon0", 0);
        dev.write(REG_TX_DATA, MemAccessSize::U8, b'H' as u64)
            .unwrap();
        dev.write(REG_TX_DATA, MemAccessSize::U8, b'i' as u64)
            .unwrap();
        assert_eq!(dev.tx_output(), b"Hi");
    }

    #[test]
    fn test_rx_input() {
        let dev = VirtioConsole::new("vcon0", 0);
        dev.push_rx(b"AB");
        assert_eq!(
            dev.read(REG_RX_DATA, MemAccessSize::U8).unwrap(),
            b'A' as u64
        );
        assert_eq!(
            dev.read(REG_RX_DATA, MemAccessSize::U8).unwrap(),
            b'B' as u64
        );
        assert_eq!(dev.read(REG_RX_DATA, MemAccessSize::U8).unwrap(), 0);
    }

    #[test]
    fn test_status_register() {
        let dev = VirtioConsole::new("vcon0", 0);
        assert_eq!(dev.read(REG_STATUS, MemAccessSize::U32).unwrap(), 0);
        dev.write(REG_STATUS, MemAccessSize::U32, 0x0F).unwrap();
        assert_eq!(dev.read(REG_STATUS, MemAccessSize::U32).unwrap(), 0x0F);
    }

    #[test]
    fn test_interrupt_ack_clears() {
        let dev = VirtioConsole::new("vcon0", 0);
        dev.push_rx(b"X");
        assert_eq!(
            dev.read(REG_INTERRUPT_STATUS, MemAccessSize::U32).unwrap(),
            1
        );
        dev.write(REG_INTERRUPT_ACK, MemAccessSize::U32, 1).unwrap();
        assert_eq!(
            dev.read(REG_INTERRUPT_STATUS, MemAccessSize::U32).unwrap(),
            0
        );
    }

    #[test]
    fn test_queue_ready() {
        let dev = VirtioConsole::new("vcon0", 0);
        dev.write(REG_QUEUE_READY, MemAccessSize::U32, 1).unwrap();
        assert_eq!(dev.read(REG_QUEUE_READY, MemAccessSize::U32).unwrap(), 1);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut dev = VirtioConsole::new("vcon0", 0);
        dev.write(REG_STATUS, MemAccessSize::U32, 0x0F).unwrap();
        dev.write(REG_TX_DATA, MemAccessSize::U8, b'Z' as u64)
            .unwrap();
        dev.push_rx(b"Q");
        dev.reset().unwrap();
        assert_eq!(dev.read(REG_STATUS, MemAccessSize::U32).unwrap(), 0);
        assert!(dev.tx_output().is_empty());
        assert_eq!(dev.read(REG_RX_DATA, MemAccessSize::U8).unwrap(), 0);
    }
}
