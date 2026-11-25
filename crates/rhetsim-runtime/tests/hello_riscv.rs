//! End-to-end test: RV64I CPU writes "Hello from RhetSim!\n" to UART

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_cpu_riscv::RiscvCpu;

const MEM_BASE: u64 = 0x8000_0000;
const MEM_SIZE: usize = 64 * 1024;
const UART_ADDR: u64 = 0x1000_0000;

fn hello_program() -> Vec<u8> {
    let mut program: Vec<u32> = Vec::new();
    // LUI a1(x11), 0x10000 → a1 = 0x10000000
    program.push(0x10000_5B7);
    for &ch in b"Hello from RhetSim!\n" {
        // ADDI a0(x10), x0, ch
        program.push(((ch as u32) << 20) | 0x0000_0513);
        // SB a0(x10), 0(a1(x11))
        program.push(0x00A5_8023);
    }
    // ECALL (halt)
    program.push(0x0000_0073);
    program.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn riscv_hello_world() {
    let mut cpu = RiscvCpu::new("rv-core0", MEM_SIZE, MEM_BASE);
    cpu.set_uart_addr(UART_ADDR);

    let program = hello_program();
    cpu.load_binary(&program, 0);

    let consumed = cpu.exec(1000).expect("exec should succeed");

    assert!(cpu.is_halted(), "CPU should halt after ECALL");
    assert_eq!(
        std::str::from_utf8(cpu.uart_output()).unwrap(),
        "Hello from RhetSim!\n"
    );

    // 42 instructions: 1 LUI + 20*(ADDI+SB) + 1 ECALL
    assert_eq!(consumed, 42);
    assert_eq!(cpu.insn_count(), 42);
}

#[test]
fn riscv_hello_via_scheduler() {
    use std::sync::{Arc, Mutex};
    use rhetsim_core::unit::Unit;
    use rhetsim_runtime::Scheduler;

    let mut cpu = RiscvCpu::new("rv-core0", MEM_SIZE, MEM_BASE);
    cpu.set_uart_addr(UART_ADDR);
    cpu.load_binary(&hello_program(), 0);

    let cpu_arc: Arc<Mutex<Box<dyn Unit>>> = Arc::new(Mutex::new(Box::new(cpu)));

    let mut scheduler = Scheduler::new(100);
    scheduler.add_unit(cpu_arc.clone());

    let stats = scheduler.run(1000, 100).expect("scheduler run");

    // CPU halts after 42 instructions, so total consumed = 42 ns.
    assert_eq!(stats.total_ns_elapsed, 42);
}
