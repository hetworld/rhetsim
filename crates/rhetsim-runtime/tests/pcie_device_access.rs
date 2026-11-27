//! End-to-end: CPU enumerates PCIe devices via ECAM and accesses BAR memory.
//!
//! Memory map (all addresses < 0x80000000 to avoid RV64 LUI sign-extension):
//!   0x0000_0000 — DRAM (64 KiB, CPU code + results)
//!   0x3000_0000 — PCIe ECAM config space (RC as MemTarget)
//!   0x4000_0000 — PCIe device 0 BAR0 (4 KiB)
//!   0x5000_0000 — PCIe device 1 BAR0 (4 KiB)
//!
//! The CPU:
//!   1. Reads device 0 vendor/device ID via ECAM
//!   2. Reads device 1 vendor/device ID via ECAM
//!   3. Writes 0x1234_5678 to device 0 BAR0
//!   4. Writes 0xABCD_EF01 to device 1 BAR0
//!   5. Reads back both values and stores to DRAM results area
//!   6. ECALL to halt

use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAccessSize, MemAttrs, MemTarget};
use rhetsim_cpu_riscv::RiscvCpu;
use rhetsim_io_pcie::{Bdf, PcieEndpoint, PcieRootComplex};
use rhetsim_mem_dram::Dram;
use std::sync::Arc;

const DRAM_BASE: u64 = 0x0000_0000;
const DRAM_SIZE: usize = 64 * 1024;
const ECAM_BASE: u64 = 0x3000_0000;
const DEV0_BAR: u64 = 0x4000_0000;
const DEV1_BAR: u64 = 0x5000_0000;
const BAR_SIZE: usize = 4096;
const RESULT_OFFSET: u64 = 0x2000;

#[test]
fn cpu_enumerates_and_accesses_pcie_devices() {
    // --- Build PCIe topology ---
    let dev0_mem = Arc::new(Dram::with_base("dev0-bar0", BAR_SIZE, 0));
    let dev1_mem = Arc::new(Dram::with_base("dev1-bar0", BAR_SIZE, 0));

    let mut ep0 = PcieEndpoint::new(Bdf::new(0, 0, 0), 0x10DE, 0x2204, 0x030200);
    ep0.add_bar(
        0,
        DEV0_BAR,
        BAR_SIZE as u64,
        dev0_mem.clone() as Arc<dyn MemTarget>,
    );

    let mut ep1 = PcieEndpoint::new(Bdf::new(0, 1, 0), 0x1234, 0x5678, 0x020000);
    ep1.add_bar(
        0,
        DEV1_BAR,
        BAR_SIZE as u64,
        dev1_mem.clone() as Arc<dyn MemTarget>,
    );

    let rc = PcieRootComplex::new("pcie0", ECAM_BASE);
    rc.attach(Box::new(ep0));
    rc.attach(Box::new(ep1));

    // --- Build MemBus ---
    let mut bus = MemBus::new("system-bus");
    let dram = Arc::new(Dram::with_base("dram0", DRAM_SIZE, DRAM_BASE));
    bus.map(
        dram.clone() as Arc<dyn MemTarget>,
        DRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let rc = Arc::new(rc);
    bus.map(
        rc.clone() as Arc<dyn MemTarget>,
        ECAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        dev0_mem.clone() as Arc<dyn MemTarget>,
        DEV0_BAR,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        dev1_mem.clone() as Arc<dyn MemTarget>,
        DEV1_BAR,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus = Arc::new(bus);

    // --- Build RV64I program ---
    // x2 = ECAM base    (0x30000000)
    // x3 = DEV0 BAR     (0x40000000)
    // x4 = DEV1 BAR     (0x50000000)
    // x10 = results     (0x00002000)
    // x1, x5 = temps
    let prog: Vec<u32> = vec![
        // Setup base addresses (all bit 31 = 0, safe for RV64 LUI)
        0x30000137, // LUI  x2,  0x30000       ; x2  = 0x30000000 (ECAM)
        0x400001B7, // LUI  x3,  0x40000       ; x3  = 0x40000000 (DEV0 BAR)
        0x50000237, // LUI  x4,  0x50000       ; x4  = 0x50000000 (DEV1 BAR)
        0x00002537, // LUI  x10, 0x2           ; x10 = 0x00002000 (results)
        // --- Read device 0 config: BDF 00:00.0 → ECAM offset 0 ---
        0x00012283, // LW   x5, 0(x2)          ; x5 = vendor|device ID
        0x00552023, // SW   x5, 0(x10)         ; results[0] = dev0 ID
        // --- Read device 1 config: BDF 00:01.0 → ECAM offset 0x8000 ---
        0x000080B7, // LUI  x1, 0x8            ; x1 = 0x8000
        0x002080B3, // ADD  x1, x1, x2         ; x1 = ECAM + 0x8000
        0x0000A283, // LW   x5, 0(x1)          ; x5 = vendor|device ID
        0x00552223, // SW   x5, 4(x10)         ; results[4] = dev1 ID
        // --- Write 0x12345678 to device 0 BAR0 ---
        // LUI x1, 0x12345 → x1 = 0x12345000
        // ADDI x1, x1, 0x678 → x1 = 0x12345678
        0x123450B7, // LUI  x1, 0x12345
        0x67808093, // ADDI x1, x1, 0x678
        0x0011A023, // SW   x1, 0(x3)          ; dev0_bar[0] = 0x12345678
        // --- Write 0x0A0B0C0D to device 1 BAR0 ---
        // LUI x1, 0x0A0B1 → x1 = 0x0A0B1000
        // ADDI x1, x1, -0xFF3 → but that's tricky...
        // Simpler: use two smaller values
        // Actually, 0x0A0B0C0D:
        // LUI x1, 0x0A0B1 → 0x0A0B1000
        // ADDI x1, x1, -0xFF3... no.
        // 0x0A0B0C0D = 0x0A0B1000 + (-0x3F3)
        // -0x3F3 = 0xFFFF_FC0D? No.
        // 0x0A0B1000 - 0x0A0B0C0D = 0x3F3. So 0x0A0B0C0D = 0x0A0B1000 - 0x3F3 is wrong.
        // Let me just pick a simpler constant: 0x0ABC_0DEF
        // 0x0ABC0DEF = 0x0ABC1000 + (-0x211)? 0x0ABC1000 - 0x211 = 0x0ABC0DEF. Yes!
        // -0x211 as 12-bit signed: 0xDEF. Check: 0xDEF = 3567, as signed 12-bit = 3567-4096=-529? No.
        // Let me just use 0x0ABCDEF0 instead.
        // 0x0ABCDEF0: LUI 0x0ABCE → 0x0ABCE000, ADDI -0x110 → 0x0ABCE000 - 0x110 = 0x0ABCDEF0?
        // 0x0ABCE000 - 0x110 = 0x0ABCDEF0. Wait, E000 - 110 = DEF0? yes!
        // -0x110 as 12-bit = 0xEF0. Check: 0xEF0 = 3824, signed = 3824-4096=-272. So -272 = -0x110. Hmm, -0x110 = -272. ✓
        // So ADDI x1, x1, 0xEF0 (which is sign-extended to -272 = -0x110)
        // But actually in the instruction encoding, the immediate is the raw 12-bit value.
        // 0xEF0 as 12 bits = 0xEF0. As signed: bit 11 = 1, so negative = -(0x1000-0xEF0) = -0x110 = -272
        // 0x0ABCE000 + (-272) = 0x0ABCE000 - 0x110 = 0x0ABCDEF0 ✓
        0x0ABCE0B7, // LUI  x1, 0x0ABCE
        0xEF008093, // ADDI x1, x1, -272       ; x1 = 0x0ABCDEF0
        0x00122023, // SW   x1, 0(x4)          ; dev1_bar[0] = 0x0ABCDEF0
        // --- Read back from BARs ---
        0x0001A283, // LW   x5, 0(x3)          ; readback dev0
        0x00552423, // SW   x5, 8(x10)         ; results[8]
        0x00022283, // LW   x5, 0(x4)          ; readback dev1
        0x00552623, // SW   x5, 12(x10)        ; results[12]
        // ECALL (halt)
        0x00000073,
    ];

    let binary: Vec<u8> = prog.iter().flat_map(|w| w.to_le_bytes()).collect();

    // --- Create CPU and run ---
    let mut cpu = RiscvCpu::with_bus("rv-core0", Arc::clone(&bus), DRAM_BASE);
    cpu.load_binary(&binary, 0);

    use rhetsim_core::backend::execution::ExecutionBackend;
    let consumed = cpu.exec(1000).expect("CPU exec");
    assert!(cpu.is_halted(), "CPU should halt after ECALL");
    assert_eq!(consumed, prog.len() as u64);

    // --- Verify results in DRAM ---
    // Result[0]: device 0 vendor|device = 0x2204_10DE
    let id0 = dram.read(RESULT_OFFSET, MemAccessSize::U32).unwrap();
    assert_eq!(id0 & 0xFFFF, 0x10DE, "device 0 vendor_id mismatch");
    assert_eq!((id0 >> 16) & 0xFFFF, 0x2204, "device 0 device_id mismatch");

    // Result[4]: device 1 vendor|device = 0x5678_1234
    let id1 = dram.read(RESULT_OFFSET + 4, MemAccessSize::U32).unwrap();
    assert_eq!(id1 & 0xFFFF, 0x1234, "device 1 vendor_id mismatch");
    assert_eq!((id1 >> 16) & 0xFFFF, 0x5678, "device 1 device_id mismatch");

    // Result[8]: readback from device 0 BAR0 = 0x12345678
    let rb0 = dram.read(RESULT_OFFSET + 8, MemAccessSize::U32).unwrap();
    assert_eq!(rb0, 0x1234_5678, "device 0 BAR readback mismatch");

    // Result[12]: readback from device 1 BAR0 = 0x0ABCDEF0
    let rb1 = dram.read(RESULT_OFFSET + 12, MemAccessSize::U32).unwrap();
    assert_eq!(rb1, 0x0ABC_DEF0, "device 1 BAR readback mismatch");

    // Verify BAR memory directly
    assert_eq!(dev0_mem.read(0, MemAccessSize::U32).unwrap(), 0x1234_5678);
    assert_eq!(dev1_mem.read(0, MemAccessSize::U32).unwrap(), 0x0ABC_DEF0);
}
