//! Runs prebuilt `riscv-tests` p-tests under the JIT and the interpreter.
//!
//! Set `RHETSIM_ISA_TESTS` to the directory containing the `rv64*-p-*` ELF
//! files (built by `./run.sh test isa` in rhetsim-dev-platform). The test is a
//! no-op when the variable is absent, matching the `linux_boot` integration
//! test's behaviour.

use object::{Object, ObjectSegment, ObjectSymbol};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_cpu_riscv::{RiscvCpu, RiscvCpuType};
use std::path::{Path, PathBuf};

const MEM_BASE: u64 = 0x8000_0000;
const MEM_SIZE: usize = 16 * 1024 * 1024;
const MAX_STEPS: u64 = 2_000_000;
const SLICE: u64 = 20_000;

/// FP tests that depend on IEEE flag accumulation, canonical-NaN output or
/// `fcsr` rounding modes, which the interpreter and JIT do not model yet.
/// They are reported as expected gaps; remove entries as the cases are fixed.
const KNOWN_FP_GAPS: &[&str] = &[
    "rv64uf-p-fadd",
    "rv64uf-p-fcmp",
    "rv64uf-p-fcvt_w",
    "rv64uf-p-fdiv",
    "rv64uf-p-fmadd",
    "rv64uf-p-fmin",
    "rv64ud-p-fadd",
    "rv64ud-p-fcmp",
    "rv64ud-p-fcvt",
    "rv64ud-p-fcvt_w",
    "rv64ud-p-fdiv",
    "rv64ud-p-fmadd",
    "rv64ud-p-fmin",
    "rv64ud-p-move",
];

struct Image {
    entry: u64,
    segments: Vec<(u64, Vec<u8>)>,
    tohost: u64,
    signature: Option<(u64, u64)>,
}

fn parse_image(path: &Path) -> Image {
    let data = std::fs::read(path).expect("read ELF");
    let file = object::File::parse(&*data).expect("parse ELF");

    let mut segments = Vec::new();
    for segment in file.segments() {
        let bytes = segment.data().unwrap_or(&[]);
        if bytes.is_empty() {
            continue;
        }
        let addr = segment.address();
        assert!(addr >= MEM_BASE, "segment below DRAM: {addr:#x}");
        segments.push((addr - MEM_BASE, bytes.to_vec()));
    }

    let mut tohost = None;
    let mut begin_signature = None;
    let mut end_signature = None;
    for symbol in file.symbols() {
        match symbol.name() {
            Ok("tohost") => tohost = Some(symbol.address()),
            Ok("begin_signature") => begin_signature = Some(symbol.address()),
            Ok("end_signature") => end_signature = Some(symbol.address()),
            _ => {}
        }
    }

    Image {
        entry: file.entry(),
        segments,
        tohost: tohost.expect("tohost symbol"),
        signature: begin_signature.zip(end_signature),
    }
}

struct Outcome {
    tohost: u64,
    pc: u64,
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
            let name = path.file_name()?.to_string_lossy();
            if path.is_file()
                && name.starts_with("rv64")
                && name.contains("-p-")
                && !name.contains('.')
            {
                Some(path)
            } else {
                None
            }
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
                "{name}: interpreter failed (tohost={:#x}, pc={:#x}, mcause={:#x})",
                interp.tohost, interp.pc, interp.mcause
            ));
            continue;
        }
        if jit.tohost != 1 {
            failures.push(format!(
                "{name}: JIT failed (tohost={:#x}, pc={:#x}, mcause={:#x})",
                jit.tohost, jit.pc, jit.mcause
            ));
            continue;
        }
        if let Some(diff) = check_parity(&name, &jit, &interp) {
            failures.push(diff);
        }
    }

    eprintln!(
        "ran {} riscv-tests p-tests ({} known FP gaps: {})",
        entries.len(),
        gaps.len(),
        gaps.join(" ")
    );
    if !failures.is_empty() {
        panic!(
            "{} of {} riscv-tests failed:\n{}",
            failures.len(),
            entries.len(),
            failures.join("\n")
        );
    }
}
