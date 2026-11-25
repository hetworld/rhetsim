//! End-to-end: CPU offloads vector-add to GPGPU via shared MemBus.
//!
//! Memory map:
//!   0x1000_0000 — UART (8 bytes)
//!   0x4000_0000 — GPGPU control registers (256 bytes)
//!   0x4000_1000 — GPGPU VRAM (64 KiB)
//!   0x8000_0000 — DRAM (64 KiB, CPU code + stack)

use std::sync::{Arc, Mutex};
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemAccessSize};
use rhetsim_core::unit::Unit;
use rhetsim_cpu_riscv::RiscvCpu;
use rhetsim_mem_dram::Dram;
use rhetsim_io_uart::Uart;
use rhetsim_accel_gpgpu::Gpgpu;
use rhetsim_accel_gpgpu::cmd_proc;
use rhetsim_runtime::Scheduler;

const DRAM_BASE: u64     = 0x8000_0000;
const DRAM_SIZE: usize   = 64 * 1024;
const UART_BASE: u64     = 0x1000_0000;
const GPU_CTRL_BASE: u64 = 0x4000_0000;
const GPU_VRAM_BASE: u64 = 0x4000_1000;
const GPU_VRAM_SIZE: usize = 64 * 1024;

fn build_vector_add_program() -> Vec<u8> {
    // RV64I program that:
    // 1. Writes two u32 vectors [1,2,3,4] to GPGPU VRAM via MemBus
    // 2. Configures GPGPU command registers (op=VECTOR_ADD, len=4, src/dst offsets)
    // 3. Rings the doorbell
    // 4. Writes "OK" to UART
    // 5. Halts via ECALL
    //
    // Register allocation:
    //   x1 = temp value
    //   x2 = GPU VRAM base (0x40001000)
    //   x3 = GPU CTRL base (0x40000000)
    //   x4 = UART base (0x10000000)
    //   x5 = temp

    let mut prog: Vec<u32> = Vec::new();

    // --- Setup base addresses ---
    // LUI x2, 0x40001  → x2 = 0x40001000 (GPU VRAM)
    prog.push(0x40001137);
    // LUI x3, 0x40000  → x3 = 0x40000000 (GPU CTRL)
    prog.push(0x400001B7);
    // LUI x4, 0x10000  → x4 = 0x10000000 (UART)
    prog.push(0x10000237);

    // --- Write vector A [1,2,3,4] to VRAM offset 0x0000 ---
    // src_a at VRAM+0x0000
    // ADDI x1, x0, 1; SW x1, 0(x2)
    prog.push(0x00100093); // ADDI x1, x0, 1
    prog.push(0x00112023); // SW x1, 0(x2)
    // ADDI x1, x0, 2; SW x1, 4(x2)
    prog.push(0x00200093); // ADDI x1, x0, 2
    prog.push(0x00112223); // SW x1, 4(x2)
    // ADDI x1, x0, 3; SW x1, 8(x2)
    prog.push(0x00300093); // ADDI x1, x0, 3
    prog.push(0x00112423); // SW x1, 8(x2)
    // ADDI x1, x0, 4; SW x1, 12(x2)
    prog.push(0x00400093); // ADDI x1, x0, 4
    prog.push(0x00112623); // SW x1, 12(x2)

    // --- Write vector B [10,20,30,40] to VRAM offset 0x1000 ---
    // Need x5 = VRAM + 0x1000 = 0x40002000
    // LUI x5, 0x40002
    prog.push(0x400022B7); // LUI x5, 0x40002

    // ADDI x1, x0, 10; SW x1, 0(x5)
    prog.push(0x00A00093); // ADDI x1, x0, 10
    prog.push(0x0012A023); // SW x1, 0(x5)
    // ADDI x1, x0, 20; SW x1, 4(x5)
    prog.push(0x01400093); // ADDI x1, x0, 20
    prog.push(0x0012A223); // SW x1, 4(x5)
    // ADDI x1, x0, 30; SW x1, 8(x5)
    prog.push(0x01E00093); // ADDI x1, x0, 30
    prog.push(0x0012A423); // SW x1, 8(x5)
    // ADDI x1, x0, 40; SW x1, 12(x5)
    prog.push(0x02800093); // ADDI x1, x0, 40
    prog.push(0x0012A623); // SW x1, 12(x5)

    // --- Configure GPGPU command registers ---
    // CMD_OP (offset 0x08) = 1 (VECTOR_ADD)
    prog.push(0x00100093); // ADDI x1, x0, 1
    prog.push(0x0011A423); // SW x1, 8(x3)   CMD_OP

    // CMD_LEN (offset 0x0C) = 4
    prog.push(0x00400093); // ADDI x1, x0, 4
    prog.push(0x0011A623); // SW x1, 12(x3)  CMD_LEN

    // CMD_SRC_A (offset 0x10) = 0x0000 (VRAM-relative offset)
    prog.push(0x0001A823); // SW x0, 16(x3)  CMD_SRC_A = 0

    // CMD_SRC_B (offset 0x14) = 0x1000
    // Need to build 0x1000 in x1
    prog.push(0x000010B7); // LUI x1, 0x1    → x1 = 0x1000
    prog.push(0x0011AA23); // SW x1, 20(x3)  CMD_SRC_B

    // CMD_DST (offset 0x18) = 0x2000
    prog.push(0x000020B7); // LUI x1, 0x2    → x1 = 0x2000
    prog.push(0x0011AC23); // SW x1, 24(x3)  CMD_DST

    // Ring doorbell: CMD_DOORBELL (offset 0x00) = 1
    prog.push(0x00100093); // ADDI x1, x0, 1
    prog.push(0x0011A023); // SW x1, 0(x3)   DOORBELL

    // --- Write "OK" to UART ---
    prog.push(0x04F00093); // ADDI x1, x0, 'O' (79)
    prog.push(0x00120023); // SB x1, 0(x4)
    prog.push(0x04B00093); // ADDI x1, x0, 'K' (75)
    prog.push(0x00120023); // SB x1, 0(x4)

    // ECALL (halt)
    prog.push(0x00000073);

    prog.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn cpu_offloads_vector_add_to_gpgpu() {
    // 1. Build MemBus with all devices
    let mut bus = MemBus::new("system-bus");

    let dram = Arc::new(Dram::with_base("dram0", DRAM_SIZE, DRAM_BASE));
    let uart = Arc::new(Uart::new("uart0"));
    let gpu = Gpgpu::new("gpu0", GPU_VRAM_SIZE);
    let gpu_ctrl = gpu.ctrl_arc();
    let gpu_vram = gpu.vram_arc();

    bus.map(dram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            DRAM_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(uart.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            UART_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(gpu_ctrl.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            GPU_CTRL_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(gpu_vram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            GPU_VRAM_BASE, MemAttrs::rw(), None).unwrap();

    let bus = Arc::new(bus);

    // 2. Create bus-backed CPU, load program into DRAM
    let mut cpu = RiscvCpu::with_bus("rv-core0", Arc::clone(&bus), DRAM_BASE);
    let program = build_vector_add_program();
    cpu.load_binary(&program, 0);

    // 3. Run CPU until it halts (writes vectors, configures GPU, rings doorbell, prints "OK")
    use rhetsim_core::backend::execution::ExecutionBackend;
    let cpu_consumed = cpu.exec(1000).expect("CPU exec");
    assert!(cpu.is_halted(), "CPU should halt after ECALL");

    // 36 instructions total (counted from program)
    assert_eq!(cpu_consumed, 36);

    // Verify UART received "OK"
    let uart_out = uart.take_output();
    assert_eq!(std::str::from_utf8(&uart_out).unwrap(), "OK");

    // Verify doorbell was rung (pending command)
    assert!(gpu.ctrl().has_pending(), "GPU should have pending command");

    // 4. Run GPGPU to process the command
    let gpu_arc: Arc<Mutex<Box<dyn Unit>>> = Arc::new(Mutex::new(Box::new(gpu)));
    let mut scheduler = Scheduler::new(100);
    scheduler.add_unit(gpu_arc.clone());
    let stats = scheduler.run(100, 100).expect("scheduler run");
    assert_eq!(stats.total_ns_elapsed, 4); // 4 elements processed

    // 5. Verify result vector in VRAM: [1+10, 2+20, 3+30, 4+40] = [11, 22, 33, 44]
    let result_base = GPU_VRAM_BASE + 0x2000;
    let expected: [u32; 4] = [11, 22, 33, 44];
    for (i, &exp) in expected.iter().enumerate() {
        let val = bus.read(result_base + (i as u64) * 4, MemAccessSize::U32, 0)
            .expect("read result");
        assert_eq!(val as u32, exp, "result mismatch at index {i}");
    }

    // Verify GPU status is DONE
    let status = bus.read(GPU_CTRL_BASE + 0x04, MemAccessSize::U32, 0).unwrap();
    assert_eq!(status as u32, cmd_proc::STATUS_DONE);
}

#[test]
fn cpu_and_gpgpu_on_same_scheduler() {
    // Both CPU and GPGPU on the same scheduler, interleaved execution.
    let mut bus = MemBus::new("system-bus");

    let dram = Arc::new(Dram::with_base("dram0", DRAM_SIZE, DRAM_BASE));
    let uart = Arc::new(Uart::new("uart0"));
    let gpu = Gpgpu::new("gpu0", GPU_VRAM_SIZE);
    let gpu_ctrl = gpu.ctrl_arc();
    let gpu_vram = gpu.vram_arc();

    bus.map(dram as Arc<dyn rhetsim_core::mem::MemTarget>,
            DRAM_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(uart.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            UART_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(gpu_ctrl as Arc<dyn rhetsim_core::mem::MemTarget>,
            GPU_CTRL_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(gpu_vram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            GPU_VRAM_BASE, MemAttrs::rw(), None).unwrap();

    let bus = Arc::new(bus);

    let mut cpu = RiscvCpu::with_bus("rv-core0", Arc::clone(&bus), DRAM_BASE);
    cpu.load_binary(&build_vector_add_program(), 0);

    let cpu_unit: Arc<Mutex<Box<dyn Unit>>> = Arc::new(Mutex::new(Box::new(cpu)));
    let gpu_unit: Arc<Mutex<Box<dyn Unit>>> = Arc::new(Mutex::new(Box::new(gpu)));

    let mut scheduler = Scheduler::new(100);
    scheduler.add_unit(cpu_unit);
    scheduler.add_unit(gpu_unit);

    // Run enough quanta for both CPU and GPGPU to complete
    let stats = scheduler.run(500, 100).expect("scheduler run");

    // CPU halts at 36 instructions, GPGPU processes 4 elements
    assert_eq!(stats.total_ns_elapsed, 36 + 4);

    // Verify result via bus
    let expected: [u32; 4] = [11, 22, 33, 44];
    for (i, &exp) in expected.iter().enumerate() {
        let val = bus.read(GPU_VRAM_BASE + 0x2000 + (i as u64) * 4, MemAccessSize::U32, 0)
            .unwrap();
        assert_eq!(val as u32, exp, "result mismatch at index {i}");
    }

    // UART output
    let uart_out = uart.take_output();
    assert_eq!(std::str::from_utf8(&uart_out).unwrap(), "OK");
}
