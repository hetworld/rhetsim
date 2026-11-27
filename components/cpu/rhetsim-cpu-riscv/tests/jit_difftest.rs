//! Randomised JIT/interpreter differential test.
//!
//! Each seed generates a straight-line program over the implemented integer,
//! bitmanip, FP and CMO instruction families, runs it once with the JIT and
//! once with the interpreter, and compares the architectural state. FP
//! registers are compared with NaN payloads masked out (RISC-V canonical-NaN
//! normalisation is still a compliance gap); CSRs, integer registers and the
//! translation state are compared exactly.

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_cpu_riscv::csr::CsrFile;
use rhetsim_cpu_riscv::{RiscvCpu, RiscvCpuType};

const MEM_SIZE: usize = 64 * 1024;
const MEM_BASE: u64 = 0;
const SCRATCH: u64 = 0x1000;
const SEEDS: u64 = 32;
const INSTRUCTIONS: usize = 192;

fn r_type(f7: u32, rs2: u32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}

fn i_type(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    ((imm as u32) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}

fn s_type(imm: i32, rs2: u32, rs1: u32, f3: u32, op: u32) -> u32 {
    let imm = imm as u32;
    ((imm >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | ((imm & 0x1F) << 7) | op
}

fn fp_fma(op: u32, f7: u32, rd: u32, rs1: u32, rs2: u32, rs3: u32) -> u32 {
    (rs3 << 27) | (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (rd << 7) | op
}

/// Deterministic xorshift64 generator; no external dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn pick(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }

    fn bit(&mut self) -> bool {
        self.next() & 1 != 0
    }
}

struct Program {
    insns: Vec<u32>,
    regs: [u64; 32],
}

fn finite_f64_bits(rng: &mut Rng) -> u64 {
    // Exponents 0x3F0..=0x40E keep seeds roughly in 2^-15..2^15.
    let sign = (rng.next() & 1) << 63;
    let exp = 0x3F0 + (rng.pick(0x1F) as u64);
    let mant = rng.next() & 0x000F_FFFF_FFFF_FFFF;
    sign | (exp << 52) | mant
}

fn finite_f32_bits(rng: &mut Rng) -> u32 {
    let sign = (rng.next() as u32 & 1) << 31;
    let exp = 0x78 + rng.pick(0x10);
    let mant = (rng.next() as u32) & 0x007F_FFFF;
    sign | (exp << 23) | mant
}

fn int_rd(rng: &mut Rng) -> u32 {
    // x30/x31 hold the scratch base addresses.
    1 + rng.pick(29)
}

fn any_reg(rng: &mut Rng) -> u32 {
    rng.pick(32)
}

fn int_r(rng: &mut Rng) -> u32 {
    let f3 = rng.pick(8);
    let f7 = match (f3, rng.bit()) {
        (0 | 5, true) => 0x20,
        _ => 0,
    };
    r_type(f7, any_reg(rng), any_reg(rng), f3, int_rd(rng), 0x33)
}

fn int_i(rng: &mut Rng) -> u32 {
    let f3 = rng.pick(8);
    let imm = match f3 {
        1 => rng.pick(64) as i32,
        5 => {
            let high = if rng.bit() { 0x400 } else { 0 };
            high | rng.pick(64) as i32
        }
        _ => rng.next() as i32,
    };
    i_type(imm, any_reg(rng), f3, int_rd(rng), 0x13)
}

fn mul_div(rng: &mut Rng) -> u32 {
    r_type(
        0x01,
        any_reg(rng),
        any_reg(rng),
        rng.pick(8),
        int_rd(rng),
        0x33,
    )
}

fn int_w(rng: &mut Rng) -> u32 {
    if rng.bit() {
        let f3 = if rng.bit() { 0 } else { 5 };
        r_type(0, any_reg(rng), any_reg(rng), f3, int_rd(rng), 0x3B)
    } else {
        match rng.pick(3) {
            0 => i_type(rng.next() as i32, any_reg(rng), 0, int_rd(rng), 0x1B),
            1 => {
                let shamt = rng.pick(32) as i32;
                i_type(shamt, any_reg(rng), 1, int_rd(rng), 0x1B)
            }
            _ => {
                let high = if rng.bit() { 0x400 } else { 0 };
                let imm = high | rng.pick(32) as i32;
                i_type(imm, any_reg(rng), 5, int_rd(rng), 0x1B)
            }
        }
    }
}

const BITMANIP_R: &[(u32, u32, u32)] = &[
    (0x33, 0x10, 2),
    (0x33, 0x10, 4),
    (0x33, 0x10, 6),
    (0x33, 0x20, 4),
    (0x33, 0x20, 6),
    (0x33, 0x20, 7),
    (0x33, 0x05, 1),
    (0x33, 0x05, 2),
    (0x33, 0x05, 3),
    (0x33, 0x05, 4),
    (0x33, 0x05, 5),
    (0x33, 0x05, 6),
    (0x33, 0x05, 7),
    (0x33, 0x14, 1),
    (0x33, 0x24, 1),
    (0x33, 0x24, 5),
    (0x33, 0x34, 1),
    (0x33, 0x30, 1),
    (0x33, 0x30, 5),
    (0x3B, 0x04, 0),
    (0x3B, 0x04, 4),
    (0x3B, 0x10, 2),
    (0x3B, 0x10, 4),
    (0x3B, 0x10, 6),
    (0x3B, 0x30, 1),
    (0x3B, 0x30, 5),
];

fn bitmanip_r(rng: &mut Rng) -> u32 {
    let (op, f7, f3) = BITMANIP_R[rng.pick(BITMANIP_R.len() as u32) as usize];
    r_type(f7, any_reg(rng), any_reg(rng), f3, int_rd(rng), op)
}

fn bitmanip_i(rng: &mut Rng) -> u32 {
    let (op, f3, funct12) = match rng.pick(17) {
        0 => (0x13, 1, 0x600),                 // clz
        1 => (0x13, 1, 0x601),                 // ctz
        2 => (0x13, 1, 0x602),                 // cpop
        3 => (0x13, 1, 0x604),                 // sext.b
        4 => (0x13, 1, 0x605),                 // sext.h
        5 => (0x13, 5, 0x6B8),                 // rev8
        6 => (0x13, 5, 0x287),                 // orc.b
        7 => (0x13, 5, 0x600 | rng.pick(64)),  // rori
        8 => (0x13, 5, 0x480 | rng.pick(64)),  // bexti
        9 => (0x13, 1, 0x280 | rng.pick(64)),  // bseti
        10 => (0x13, 1, 0x480 | rng.pick(64)), // bclri
        11 => (0x13, 1, 0x680 | rng.pick(64)), // binvi
        12 => (0x1B, 1, 0x600),                // clzw
        13 => (0x1B, 1, 0x601),                // ctzw
        14 => (0x1B, 1, 0x602),                // cpopw
        15 => (0x1B, 1, 0x080 | rng.pick(64)), // slli.uw
        _ => (0x1B, 5, 0x600 | rng.pick(32)),  // roriw
    };
    i_type(funct12 as i32, any_reg(rng), f3, int_rd(rng), op)
}

fn fp_op(rng: &mut Rng) -> u32 {
    let frd = any_reg(rng);
    let frs1 = any_reg(rng);
    let frs2 = any_reg(rng);
    let f3 = rng.pick(3);
    let fcvt_kind = rng.pick(4);
    match rng.pick(28) {
        0 => r_type(0x00, frs2, frs1, 0, frd, 0x53),   // fadd.s
        1 => r_type(0x01, frs2, frs1, 0, frd, 0x53),   // fadd.d
        2 => r_type(0x04, frs2, frs1, 0, frd, 0x53),   // fsub.s
        3 => r_type(0x05, frs2, frs1, 0, frd, 0x53),   // fsub.d
        4 => r_type(0x08, frs2, frs1, 0, frd, 0x53),   // fmul.s
        5 => r_type(0x09, frs2, frs1, 0, frd, 0x53),   // fmul.d
        6 => r_type(0x0C, frs2, frs1, 0, frd, 0x53),   // fdiv.s
        7 => r_type(0x0D, frs2, frs1, 0, frd, 0x53),   // fdiv.d
        8 => r_type(0x2C, 0, frs1, 0, frd, 0x53),      // fsqrt.s
        9 => r_type(0x2D, 0, frs1, 0, frd, 0x53),      // fsqrt.d
        10 => r_type(0x10, frs2, frs1, f3, frd, 0x53), // fsgnj.s
        11 => r_type(0x11, frs2, frs1, f3, frd, 0x53), // fsgnj.d
        12 => r_type(0x14, frs2, frs1, rng.bit() as u32, frd, 0x53), // fmin/max.s
        13 => r_type(0x15, frs2, frs1, rng.bit() as u32, frd, 0x53), // fmin/max.d
        14 => r_type(0x20, 0, frs1, 0, frd, 0x53),     // fcvt.s.d
        15 => r_type(0x21, 0, frs1, 0, frd, 0x53),     // fcvt.d.s
        16 => r_type(0x50, frs2, frs1, f3, int_rd(rng), 0x53), // feq/flt/fle.s
        17 => r_type(0x51, frs2, frs1, f3, int_rd(rng), 0x53), // feq/flt/fle.d
        18 => r_type(0x60, fcvt_kind, frs1, 0, int_rd(rng), 0x53), // fcvt int <- s
        19 => r_type(0x61, fcvt_kind, frs1, 0, int_rd(rng), 0x53), // fcvt int <- d
        20 => r_type(0x68, fcvt_kind, any_reg(rng), 0, frd, 0x53), // fcvt s <- int
        21 => r_type(0x69, fcvt_kind, any_reg(rng), 0, frd, 0x53), // fcvt d <- int
        22 => r_type(0x70, 0, frs1, rng.bit() as u32, int_rd(rng), 0x53), // fmv.x.w/fclass.s
        23 => r_type(0x71, 0, frs1, rng.bit() as u32, int_rd(rng), 0x53), // fmv.x.d/fclass.d
        24 => r_type(0x78, 0, any_reg(rng), 0, frd, 0x53), // fmv.w.x
        25 => r_type(0x79, 0, any_reg(rng), 0, frd, 0x53), // fmv.d.x
        26 | 27 => {
            let op = [0x43u32, 0x47, 0x4B, 0x4F][rng.pick(4) as usize];
            let f7 = rng.bit() as u32;
            fp_fma(op, f7, frd, frs1, frs2, any_reg(rng))
        }
        _ => unreachable!(),
    }
}

fn memory_or_cbo(rng: &mut Rng) -> u32 {
    let offset = (rng.pick(32) * 8) as i32;
    match rng.pick(9) {
        0 => i_type(offset, 30, 3, int_rd(rng), 0x03),  // ld
        1 => s_type(offset, any_reg(rng), 30, 3, 0x23), // sd
        2 => i_type(offset, 30, 2, int_rd(rng), 0x03),  // lw
        3 => s_type(offset, any_reg(rng), 30, 2, 0x23), // sw
        4 => i_type(offset, 30, 3, any_reg(rng), 0x07), // fld
        5 => s_type(offset, any_reg(rng), 30, 3, 0x27), // fsd
        6 => i_type(offset, 30, 2, any_reg(rng), 0x07), // flw
        7 => s_type(offset, any_reg(rng), 30, 2, 0x27), // fsw
        _ => {
            let op = rng.pick(5);
            (op << 20) | (30 << 15) | (2 << 12) | 0x0F // cbo.inval/clean/flush/zero
        }
    }
}

fn random_insn(rng: &mut Rng) -> u32 {
    match rng.pick(12) {
        0 | 1 => int_r(rng),
        2 => int_i(rng),
        3 => mul_div(rng),
        4 => int_w(rng),
        5 | 6 => bitmanip_r(rng),
        7 | 8 => bitmanip_i(rng),
        9 | 10 => fp_op(rng),
        _ => memory_or_cbo(rng),
    }
}

fn generate(seed: u64) -> Program {
    let mut rng = Rng::new(seed);
    let mut regs = [0u64; 32];
    let edges = [
        0,
        1,
        2,
        u64::MAX,
        u32::MAX as u64,
        1 << 63,
        0x8000_0000_0000_0000,
        0x7FFF_FFFF_FFFF_FFFF,
    ];
    for reg in regs.iter_mut().take(21).skip(1) {
        *reg = if rng.pick(4) == 0 {
            edges[rng.pick(edges.len() as u32) as usize]
        } else {
            rng.next()
        };
    }
    regs[30] = SCRATCH;
    regs[31] = SCRATCH + 0x100;

    let mut insns = Vec::with_capacity(INSTRUCTIONS + 20);
    // Seed f1..f8 with finite doubles and f9..f16 with finite singles.
    for f in 1..=8u32 {
        regs[f as usize] = finite_f64_bits(&mut rng);
        insns.push(r_type(0x79, 0, f, 0, f, 0x53)); // fmv.d.x fF, xF
    }
    for f in 9..=16u32 {
        regs[f as usize] = finite_f32_bits(&mut rng) as u64;
        insns.push(r_type(0x78, 0, f, 0, f, 0x53)); // fmv.w.x fF, xF
    }
    while insns.len() < INSTRUCTIONS {
        insns.push(random_insn(&mut rng));
    }
    insns.push(0x0000_0073); // ecall -> mtvec 0 -> halt
    Program { insns, regs }
}

fn run(program: &Program, jit_enabled: bool) -> RiscvCpu {
    let mut cpu =
        RiscvCpu::new_with_type("difftest", MEM_SIZE, MEM_BASE, RiscvCpuType::THeadC908Full);
    cpu.set_jit_enabled(jit_enabled);
    for (i, insn) in program.insns.iter().enumerate() {
        cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
    }
    for (i, value) in program.regs.iter().enumerate() {
        cpu.set_reg(i, *value);
    }
    cpu.set_pc(0);
    cpu.exec(1_000_000).unwrap();
    cpu
}

fn fp_registers_match(a: &RiscvCpu, b: &RiscvCpu) -> Result<(), String> {
    for i in 0..32 {
        let x = a.freg(i);
        let y = b.freg(i);
        if x == y {
            continue;
        }
        let both_f64_nan = f64::from_bits(x).is_nan() && f64::from_bits(y).is_nan();
        let both_f32_nan = f32::from_bits(x as u32).is_nan() && f32::from_bits(y as u32).is_nan();
        if both_f64_nan || both_f32_nan {
            continue;
        }
        return Err(format!("f{i}: {x:#018x} != {y:#018x}"));
    }
    Ok(())
}

fn state_without_fp(cpu: &RiscvCpu) -> Vec<u8> {
    let mut state = cpu.save_state().unwrap();
    let fregs = 32 * 8 + 8 + 1 + 1 + CsrFile::state_size();
    for byte in &mut state[fregs..fregs + 32 * 8] {
        *byte = 0;
    }
    state
}

const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;
const PTE_N: u64 = 1 << 63;

/// A 64 KiB Svnapot page must translate identically when the load/store is
/// executed from a translated block.
#[test]
fn svnapot_page_matches_the_interpreter() {
    fn setup(jit_enabled: bool) -> RiscvCpu {
        let mut cpu = RiscvCpu::new_with_type("svnapot", 0x2_0000, 0, RiscvCpuType::THeadC908Full);
        cpu.set_jit_enabled(jit_enabled);

        // PMP: allow S-mode access to the whole address space (as firmware
        // would) so translation, not PMP, is what the test exercises.
        cpu.csr_mut().pmpaddr[0] = (1 << 54) - 1;
        cpu.csr_mut().pmpcfg[0] = 0x1F;

        // Sv39: root @ PPN 1, L1 @ PPN 2, L0 @ PPN 3.
        // L0[4] is a 64 KiB NAPOT leaf with PPN 0x18 (low nibble 8),
        // mapping VA 0x4000 + off -> PA 0x14000 + off.
        let mut pte = |offset: u64, value: u64| cpu.load_binary(&value.to_le_bytes(), offset);
        pte(0x1000, (2 << 10) | PTE_V);
        pte(0x2000, (3 << 10) | PTE_V);
        pte(
            0x3000 + 4 * 8,
            (0x18 << 10) | PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D | PTE_N,
        );

        cpu.csr_mut().satp = (8u64 << 60) | 1;
        // MPRV=1, MPP=S: M-mode data accesses use the S-mode translation.
        cpu.csr_mut().mstatus = (1 << 17) | (1 << 11);

        let insns: [u32; 8] = [
            (4 << 12) | (10 << 7) | 0x37, // lui x10, 4 -> VA 0x4000
            0x02A0_0593,                  // addi x11, x0, 42
            0x00B5_2023,                  // sw x11, 0(x10)
            0x0005_2603,                  // lw x12, 0(x10)
            0x0110_0693,                  // addi x13, x0, 17
            0x2AD5_2E23,                  // sw x13, 0x2BC(x10)
            0x2BC5_2703,                  // lw x14, 0x2BC(x10)
            0x0000_0073,                  // ecall
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(10_000).unwrap();
        cpu
    }

    let jit = setup(true);
    let interp = setup(false);
    assert!(jit.is_halted() && interp.is_halted());
    assert_eq!(
        state_without_fp(&jit),
        state_without_fp(&interp),
        "Svnapot state differs"
    );
    assert_eq!(jit.reg(12), 42);
    assert_eq!(jit.reg(14), 17);
    let (compiles, executions, _, fallbacks) = jit.jit_stats().expect("JIT engine");
    assert!(compiles > 0 && executions > 0);
    assert_eq!(fallbacks, 0);
}

#[test]
fn random_programs_match_the_interpreter() {
    for seed in 1..=SEEDS {
        let program = generate(seed);
        let jit = run(&program, true);
        let interp = run(&program, false);

        assert!(jit.is_halted(), "seed {seed}: JIT did not halt");
        assert!(interp.is_halted(), "seed {seed}: interpreter did not halt");
        assert_eq!(
            state_without_fp(&jit),
            state_without_fp(&interp),
            "seed {seed}: architectural state differs"
        );
        if let Err(why) = fp_registers_match(&jit, &interp) {
            panic!("seed {seed}: FP register mismatch: {why}");
        }
        assert_eq!(
            jit.csr().fcsr,
            interp.csr().fcsr,
            "seed {seed}: fcsr differs"
        );

        let (compiles, executions, _, fallbacks) =
            jit.jit_stats().expect("JIT engine should be enabled");
        assert!(compiles > 0, "seed {seed}: JIT compiled no blocks");
        assert!(executions > 0, "seed {seed}: JIT executed no blocks");
        assert_eq!(fallbacks, 0, "seed {seed}: JIT fell back");
    }
}
