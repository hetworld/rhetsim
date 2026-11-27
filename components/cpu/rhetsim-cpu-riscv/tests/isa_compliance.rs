//! Runs prebuilt `riscv-tests` p-tests under the JIT and the interpreter.
//!
//! Set `RHETSIM_ISA_TESTS` to the directory containing the `rv64*-p-*` ELF
//! files (built by `./run.sh test isa` in rhetsim-dev-platform). The test is a
//! no-op when the variable is absent, matching the `linux_boot` integration
//! test's behaviour.

mod common;

use common::{is_riscv_ptest, parse_image, Image, MEM_BASE, MEM_SIZE};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_cpu_riscv::{RiscvCpu, RiscvCpuType};
use std::path::PathBuf;

const MAX_STEPS: u64 = 2_000_000;
const SLICE: u64 = 20_000;

/// Escape hatch for tests with a known, documented compliance gap. The list is
/// currently empty: all selected riscv-tests pass in both engines. A listed
/// test that starts passing fails the run so the list cannot go stale.
const KNOWN_FP_GAPS: &[&str] = &[];

struct Outcome {
    tohost: u64,
    pc: u64,
    a0: u64,
    a1: u64,
    a3: u64,
    fregs: [u64; 32],
    fcsr: u32,
    mstatus: u64,
    mcause: u64,
    signature: Option<Vec<u8>>,
}

fn run(image: &Image, jit_enabled: bool) -> Outcome {
    let mut cpu =
        RiscvCpu::new_with_type("isa-test", MEM_SIZE, MEM_BASE, RiscvCpuType::THeadC908Full);
    cpu.set_jit_enabled(jit_enabled);
    for (offset, bytes) in &image.segments {
        cpu.load_binary(bytes, *offset);
    }
    cpu.set_pc(image.entry);
    cpu.set_reg(10, 0); // a0 = hartid
    cpu.set_reg(11, 0); // a1 = DTB (unused by p-tests)

    let mut steps = 0;
    let mut tohost = 0;
    while steps < MAX_STEPS {
        cpu.exec(SLICE).unwrap();
        steps += SLICE;
        tohost = cpu.read_phys(image.tohost, 8);
        if tohost != 0 || cpu.is_halted() {
            break;
        }
    }

    let signature = image.signature.map(|(begin, end)| {
        let mut bytes = Vec::new();
        let mut addr = begin;
        while addr < end {
            bytes.extend_from_slice(&cpu.read_phys(addr, 8).to_le_bytes());
            addr += 8;
        }
        bytes
    });

    let mut fregs = [0u64; 32];
    for (i, reg) in fregs.iter_mut().enumerate() {
        *reg = cpu.freg(i);
    }

    Outcome {
        tohost,
        pc: cpu.pc(),
        a0: cpu.reg(10),
        a1: cpu.reg(11),
        a3: cpu.reg(13),
        fregs,
        fcsr: cpu.csr().fcsr,
        mstatus: cpu.csr().mstatus,
        mcause: cpu.csr().mcause,
        signature,
    }
}

fn same_fp_bits(x: u64, y: u64) -> bool {
    if x == y {
        return true;
    }
    // RISC-V canonical-NaN normalisation is still pending; compare NaNs by
    // class rather than payload.
    let both_f64 = f64::from_bits(x).is_nan() && f64::from_bits(y).is_nan();
    let both_f32 = f32::from_bits(x as u32).is_nan() && f32::from_bits(y as u32).is_nan();
    both_f64 || both_f32
}

fn same_signature(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.chunks(8).zip(b.chunks(8)).all(|(x, y)| {
        if x == y {
            return true;
        }
        if x.len() != 8 || y.len() != 8 {
            return false;
        }
        let mut xb = [0u8; 8];
        let mut yb = [0u8; 8];
        xb.copy_from_slice(x);
        yb.copy_from_slice(y);
        same_fp_bits(u64::from_le_bytes(xb), u64::from_le_bytes(yb))
    })
}

fn check_parity(name: &str, jit: &Outcome, interp: &Outcome) -> Option<String> {
    // The p-test environment ends in an infinite `write_tohost` loop that
    // rewrites t5/pc each iteration, so the stop position is not comparable.
    // The test's own PASS code and the signature cover the computed values.
    for i in 0..32 {
        if !same_fp_bits(jit.fregs[i], interp.fregs[i]) {
            return Some(format!(
                "{name}: f{i} {:#x} != {:#x}",
                jit.fregs[i], interp.fregs[i]
            ));
        }
    }
    if jit.fcsr != interp.fcsr {
        return Some(format!(
            "{name}: fcsr {:#x} != {:#x}",
            jit.fcsr, interp.fcsr
        ));
    }
    if jit.mstatus != interp.mstatus {
        return Some(format!(
            "{name}: mstatus {:#x} != {:#x}",
            jit.mstatus, interp.mstatus
        ));
    }
    if jit.mcause != interp.mcause {
        return Some(format!(
            "{name}: mcause {:#x} != {:#x}",
            jit.mcause, interp.mcause
        ));
    }
    if let (Some(a), Some(b)) = (&jit.signature, &interp.signature) {
        if !same_signature(a, b) {
            return Some(format!("{name}: signature differs"));
        }
    }
    None
}

#[test]
fn riscv_tests_match_the_interpreter() {
    let dir = match std::env::var_os("RHETSIM_ISA_TESTS") {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("RHETSIM_ISA_TESTS is not set; skipping riscv-tests compliance run");
            return;
        }
    };
    if !dir.is_dir() {
        eprintln!(
            "RHETSIM_ISA_TESTS={} is not a directory; skipping",
            dir.display()
        );
        return;
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read ISA test directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            is_riscv_ptest(&path).then_some(path)
        })
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "no rv64*-p-* tests under {}",
        dir.display()
    );

    let mut failures = Vec::new();
    let mut gaps = Vec::new();
    for path in &entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let known_gap = KNOWN_FP_GAPS.contains(&name.as_str());
        let image = parse_image(path);
        let jit = run(&image, true);
        let interp = run(&image, false);

        if known_gap {
            if jit.tohost == 1 && interp.tohost == 1 {
                failures.push(format!("{name}: passed; remove it from KNOWN_FP_GAPS"));
            } else {
                gaps.push(name);
            }
            continue;
        }

        if interp.tohost != 1 {
            failures.push(format!(
                "{name}: interpreter failed (tohost={:#x}, pc={:#x}, mcause={:#x}, a0={:#x}, a1={:#x}, a3={:#x}, fcsr={:#x})",
                interp.tohost, interp.pc, interp.mcause, interp.a0, interp.a1, interp.a3, interp.fcsr
            ));
            continue;
        }
        if jit.tohost != 1 {
            failures.push(format!(
                "{name}: JIT failed (tohost={:#x}, pc={:#x}, mcause={:#x}, a0={:#x}, a1={:#x}, a3={:#x}, fcsr={:#x})",
                jit.tohost, jit.pc, jit.mcause, jit.a0, jit.a1, jit.a3, jit.fcsr
            ));
            continue;
        }
        if let Some(diff) = check_parity(&name, &jit, &interp) {
            failures.push(diff);
        }
    }

    if gaps.is_empty() {
        eprintln!("ran {} riscv-tests p-tests", entries.len());
    } else {
        eprintln!(
            "ran {} riscv-tests p-tests ({} known gaps: {})",
            entries.len(),
            gaps.len(),
            gaps.join(" ")
        );
    }
    if !failures.is_empty() {
        panic!(
            "{} of {} riscv-tests failed:\n{}",
            failures.len(),
            entries.len(),
            failures.join("\n")
        );
    }
}
