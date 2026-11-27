//! ARM SoC integration test — full platform with six peripherals
//!
//! Memory map:
//!   0x0000_0000 — DRAM (128 KiB, code + data + results)
//!   0x0800_0000 — GICv3 (Distributor + Redistributor)
//!   0x0900_0000 — UART0 (16550)
//!   0x0A00_0000 — DMA engine
//!   0x0B00_0000 — VirtIO console
//!   0x1000_0000 — PCIe ECAM config space
//!   0x2000_0000 — PCIe device BAR0 (4 KiB NIC)
//!
//! The ARM CPU:
//!   1. Writes "SoC\n" to UART
//!   2. Writes test pattern to DRAM, programs DMA to copy it
//!   3. Verifies DMA status and reads back copied data
//!   4. Reads VirtIO console magic register
//!   5. Writes "OK" to VirtIO console TX port
//!   6. Enumerates PCIe device via ECAM, reads vendor/device ID
//!   7. Writes 0xCAFEBABE to PCIe BAR0, reads back
//!   8. Stores all results to DRAM, halts via SVC

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAccessSize, MemAttrs, MemTarget};
use rhetsim_cpu_arm::cpu::ArmCpu;
use rhetsim_intc_gicv3::Gicv3;
use rhetsim_io_dma::DmaEngine;
use rhetsim_io_pcie::{Bdf, PcieEndpoint, PcieRootComplex};
use rhetsim_io_uart::Uart;
use rhetsim_io_virtio::VirtioConsole;
use rhetsim_mem_dram::Dram;
use std::sync::Arc;

const DRAM_BASE: u64 = 0x0000_0000;
const DRAM_SIZE: usize = 128 * 1024;
const GIC_BASE: u64 = 0x0800_0000;
const UART_BASE: u64 = 0x0900_0000;
const DMA_BASE: u64 = 0x0A00_0000;
const VIRTIO_BASE: u64 = 0x0B00_0000;
const ECAM_BASE: u64 = 0x1000_0000;
const PCIE_BAR: u64 = 0x2000_0000;
const BAR_SIZE: usize = 4096;

const SRC_OFF: u64 = 0x4000;
const DST_OFF: u64 = 0x5000;
const RESULT_OFF: u64 = 0x1_0000;

// ── AArch64 instruction encoding helpers ──────────────────────

fn movz(rd: u32, imm16: u32, shift: u32) -> u32 {
    let hw = shift / 16;
    0xD2800000 | (hw << 21) | ((imm16 & 0xFFFF) << 5) | rd
}

fn movk(rd: u32, imm16: u32, shift: u32) -> u32 {
    let hw = shift / 16;
    0xF2800000 | (hw << 21) | ((imm16 & 0xFFFF) << 5) | rd
}

fn strb_uoff(rt: u32, rn: u32, offset: u32) -> u32 {
    0x39000000 | ((offset & 0xFFF) << 10) | (rn << 5) | rt
}

fn str_w_uoff(rt: u32, rn: u32, byte_offset: u32) -> u32 {
    0xB9000000 | (((byte_offset / 4) & 0xFFF) << 10) | (rn << 5) | rt
}

fn ldr_w_uoff(rt: u32, rn: u32, byte_offset: u32) -> u32 {
    0xB9400000 | (((byte_offset / 4) & 0xFFF) << 10) | (rn << 5) | rt
}

fn str_x_uoff(rt: u32, rn: u32, byte_offset: u32) -> u32 {
    0xF9000000 | (((byte_offset / 8) & 0xFFF) << 10) | (rn << 5) | rt
}

fn svc0() -> u32 {
    0xD4000001
}

#[test]
fn arm_soc_full_platform() {
    // ── Build peripherals ─────────────────────────────────────
    let dram = Arc::new(Dram::with_base("dram0", DRAM_SIZE, DRAM_BASE));
    let gic = Arc::new(Gicv3::new("gic0", 1, 64, GIC_BASE));
    let uart = Arc::new(Uart::new("uart0"));
    let virtio = Arc::new(VirtioConsole::new("vcon0", VIRTIO_BASE));

    let nic_mem = Arc::new(Dram::with_base("nic-bar0", BAR_SIZE, 0));
    let mut ep = PcieEndpoint::new(Bdf::new(0, 0, 0), 0x8086, 0x15B8, 0x020000);
    ep.add_bar(
        0,
        PCIE_BAR,
        BAR_SIZE as u64,
        nic_mem.clone() as Arc<dyn MemTarget>,
    );
    let rc = PcieRootComplex::new("pcie0", ECAM_BASE);
    rc.attach(Box::new(ep));
    let rc = Arc::new(rc);

    // ── Build MemBus ──────────────────────────────────────────
    let mut bus = MemBus::new("sys-bus");
    bus.map(
        dram.clone() as Arc<dyn MemTarget>,
        DRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        gic.clone() as Arc<dyn MemTarget>,
        GIC_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        uart.clone() as Arc<dyn MemTarget>,
        UART_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        virtio.clone() as Arc<dyn MemTarget>,
        VIRTIO_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        rc.clone() as Arc<dyn MemTarget>,
        ECAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        nic_mem.clone() as Arc<dyn MemTarget>,
        PCIE_BAR,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus = Arc::new(bus);

    // DMA needs bus access
    let dma = Arc::new(DmaEngine::new("dma0", DMA_BASE, bus.clone()));
    // Re-open bus to add DMA (need mutable access — rebuild)
    let mut bus2 = MemBus::new("sys-bus");
    bus2.map(
        dram.clone() as Arc<dyn MemTarget>,
        DRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus2.map(
        gic.clone() as Arc<dyn MemTarget>,
        GIC_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus2.map(
        uart.clone() as Arc<dyn MemTarget>,
        UART_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus2.map(
        dma.clone() as Arc<dyn MemTarget>,
        DMA_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus2.map(
        virtio.clone() as Arc<dyn MemTarget>,
        VIRTIO_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus2.map(
        rc.clone() as Arc<dyn MemTarget>,
        ECAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus2.map(
        nic_mem.clone() as Arc<dyn MemTarget>,
        PCIE_BAR,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus2 = Arc::new(bus2);

    // ── Build ARM CPU ─────────────────────────────────────────
    let mut cpu = ArmCpu::with_bus("arm-core0", bus2.clone(), DRAM_BASE);
    cpu.attach_gic(gic.clone(), 0);
    cpu.set_sp(DRAM_BASE + DRAM_SIZE as u64);

    // Register assignments:
    //   X2  = UART base     (0x0900_0000)
    //   X3  = DMA base      (0x0A00_0000)
    //   X4  = ECAM base     (0x1000_0000)
    //   X5  = PCIe BAR base (0x2000_0000)
    //   X6  = results area  (0x0001_0000)
    //   X7  = VirtIO base   (0x0B00_0000)
    //   X8  = scratch addr
    //   X0, X1, X9 = temps
    let prog: Vec<u32> = vec![
        // ── Set up base addresses ─────────────────────────────
        movz(2, 0x0000, 0),
        movk(2, 0x0900, 16), // X2 = UART
        movz(3, 0x0000, 0),
        movk(3, 0x0A00, 16), // X3 = DMA
        movz(4, 0x0000, 0),
        movk(4, 0x1000, 16), // X4 = ECAM
        movz(5, 0x0000, 0),
        movk(5, 0x2000, 16), // X5 = PCIe BAR
        movz(6, 0x0000, 0),
        movk(6, 0x0001, 16), // X6 = results
        movz(7, 0x0000, 0),
        movk(7, 0x0B00, 16), // X7 = VirtIO
        // ── Part 1: Write "SoC\n" to UART ─────────────────────
        movz(0, b'S' as u32, 0),
        strb_uoff(0, 2, 0),
        movz(0, b'o' as u32, 0),
        strb_uoff(0, 2, 0),
        movz(0, b'C' as u32, 0),
        strb_uoff(0, 2, 0),
        movz(0, b'\n' as u32, 0),
        strb_uoff(0, 2, 0),
        // ── Part 2: Write test pattern to DRAM at SRC_OFF ─────
        movz(8, SRC_OFF as u32, 0), // X8 = 0x4000
        movz(0, 0xBEEF, 0),
        movk(0, 0xDEAD, 16), // X0 = 0xDEADBEEF
        str_w_uoff(0, 8, 0), // DRAM[0x4000] = 0xDEADBEEF
        movz(0, 0xFACE, 0),
        movk(0, 0xCAFE, 16), // X0 = 0xCAFEFACE
        str_w_uoff(0, 8, 4), // DRAM[0x4004] = 0xCAFEFACE
        // ── Part 3: Program DMA transfer ──────────────────────
        // SRC_ADDR (offset 0x00) = 0x4000
        movz(0, SRC_OFF as u32, 0),
        str_x_uoff(0, 3, 0), // DMA[0x00] = src
        // DST_ADDR (offset 0x08) = 0x5000
        movz(0, DST_OFF as u32, 0),
        str_x_uoff(0, 3, 8), // DMA[0x08] = dst
        // LENGTH (offset 0x10) = 8 bytes
        movz(0, 8, 0),
        str_w_uoff(0, 3, 0x10), // DMA[0x10] = 8
        // CTRL = 1 (start)
        movz(0, 1, 0),
        str_w_uoff(0, 3, 0x14), // DMA[0x14] = start
        // ── Part 4: Read DMA status → results[0] ─────────────
        ldr_w_uoff(0, 3, 0x18), // X0 = DMA STATUS
        str_w_uoff(0, 6, 0),    // results[0]
        // ── Part 5: Verify DMA copy ──────────────────────────
        movz(8, DST_OFF as u32, 0), // X8 = 0x5000
        ldr_w_uoff(0, 8, 0),
        str_w_uoff(0, 6, 4), // results[4] = DRAM[0x5000]
        ldr_w_uoff(0, 8, 4),
        str_w_uoff(0, 6, 8), // results[8] = DRAM[0x5004]
        // ── Part 6: VirtIO magic → results[12] ───────────────
        ldr_w_uoff(0, 7, 0),  // VirtIO magic
        str_w_uoff(0, 6, 12), // results[12]
        // ── Part 7: Write "OK" to VirtIO TX ──────────────────
        movz(0, b'O' as u32, 0),
        str_w_uoff(0, 7, 0x100), // VirtIO TX port
        movz(0, b'K' as u32, 0),
        str_w_uoff(0, 7, 0x100),
        // ── Part 8: PCIe ECAM read → results[16] ─────────────
        ldr_w_uoff(0, 4, 0),  // ECAM[0] = vendor|device
        str_w_uoff(0, 6, 16), // results[16]
        // ── Part 9: Write 0xCAFEBABE to PCIe BAR, readback ───
        movz(0, 0xBABE, 0),
        movk(0, 0xCAFE, 16),
        str_w_uoff(0, 5, 0),  // BAR[0] = 0xCAFEBABE
        ldr_w_uoff(0, 5, 0),  // readback
        str_w_uoff(0, 6, 20), // results[20]
        // ── Halt ──────────────────────────────────────────────
        svc0(),
    ];

    let binary: Vec<u8> = prog.iter().flat_map(|w| w.to_le_bytes()).collect();
    cpu.load_binary(&binary, 0);

    // ── Execute ───────────────────────────────────────────────
    let consumed = cpu.exec(500).expect("CPU exec");
    assert!(cpu.is_halted(), "CPU should halt via SVC");
    assert_eq!(consumed, prog.len() as u64);

    // ── Verify UART output ────────────────────────────────────
    let uart_out = uart.take_output();
    assert_eq!(uart_out, b"SoC\n", "UART should contain 'SoC\\n'");

    // ── Verify DMA ────────────────────────────────────────────
    let dma_status = dram.read(RESULT_OFF, MemAccessSize::U32).unwrap();
    assert_eq!(dma_status, 1, "DMA should be done (status=1)");

    let dma_word0 = dram.read(RESULT_OFF + 4, MemAccessSize::U32).unwrap();
    assert_eq!(dma_word0, 0xDEAD_BEEF, "DMA copy word 0 mismatch");

    let dma_word1 = dram.read(RESULT_OFF + 8, MemAccessSize::U32).unwrap();
    assert_eq!(dma_word1, 0xCAFE_FACE, "DMA copy word 1 mismatch");

    // ── Verify VirtIO ─────────────────────────────────────────
    let virtio_magic = dram.read(RESULT_OFF + 12, MemAccessSize::U32).unwrap();
    assert_eq!(virtio_magic, 0x74726976, "VirtIO magic mismatch");

    let vtx = virtio.tx_output();
    assert_eq!(vtx, b"OK", "VirtIO TX should contain 'OK'");

    // ── Verify PCIe ───────────────────────────────────────────
    let pcie_id = dram.read(RESULT_OFF + 16, MemAccessSize::U32).unwrap();
    assert_eq!(pcie_id & 0xFFFF, 0x8086, "PCIe vendor_id mismatch (Intel)");
    assert_eq!((pcie_id >> 16) & 0xFFFF, 0x15B8, "PCIe device_id mismatch");

    let pcie_rb = dram.read(RESULT_OFF + 20, MemAccessSize::U32).unwrap();
    assert_eq!(pcie_rb, 0xCAFE_BABE, "PCIe BAR readback mismatch");

    // Verify BAR memory directly
    assert_eq!(nic_mem.read(0, MemAccessSize::U32).unwrap(), 0xCAFE_BABE);
}
