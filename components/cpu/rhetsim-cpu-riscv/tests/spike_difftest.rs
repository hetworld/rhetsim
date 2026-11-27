//! Instruction-level difftest against Spike.
//!
//! Set `RHETSIM_SPIKE` to a Spike binary and `RHETSIM_ISA_TESTS` to the
//! directory containing the prebuilt `rv64*-p-*` ELF files (the same layout
//! `./run.sh test isa` produces). The test is a no-op when either variable is
//! absent, matching the other integration tests.
//!
//! Spike runs each test with `--log-commits`; its commit log records the
//! privilege level, PC, raw instruction and every architectural write of each
//! retired instruction. The test replays the same ELF in the interpreter one
//! instruction at a time and compares that state before every commit, stopping
//! at the first instruction of the `write_tohost` pass/fail loop.

mod common;

use common::{is_riscv_ptest, parse_image, Image, MEM_BASE, MEM_SIZE};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_cpu_riscv::csr::{MCAUSE_INTERRUPT, PRV_S};
use rhetsim_cpu_riscv::{RiscvCpu, RiscvCpuType};
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_COMMITS: usize = 400_000;
const SPIKE_TIMEOUT_SECS: u64 = 60;
// `smrnmi` matters: riscv-tests' reset vector writes `mnstatus`, and without
// the extension Spike takes an illegal-instruction trap that skips the write
// (its temporary mtvec points at the following instruction), while our CPU
// accepts the write. Enabling it keeps both traces aligned. `zicclsm` makes
// Spike execute misaligned data accesses in hardware like RhetSim does.
const DEFAULT_ISA: &str = "rv64gc_zba_zbb_zbc_zbs_svnapot_smrnmi_zicclsm";

/// Tests that are known not to be directly comparable with Spike. Each entry
/// needs a reason; the list is intentionally short and the test reports
/// skipped entries.
const SPIKE_SKIP: &[(&str, &str)] = &[
    (
        "rv64mi-p-breakpoint",
        "Spike has no Sdtrig: its tcontrol write traps, ours is accepted",
    ),
    (
        "rv64mi-p-illegal",
        "illegal-instruction mtval: Spike stores the insn word, RhetSim stores 0",
    ),
    (
        "rv64mi-p-ma_fetch",
        "Spike honours writes that clear C in misa; RhetSim's misa is read-only",
    ),
    (
        "rv64mi-p-zicntr",
        "Spike reports the test as failed: its time CSR does not advance by default",
    ),
];

#[derive(Debug)]
struct Commit {
    pc: u64,
    priv_mode: u8,
    insn: u32,
    x_writes: Vec<(usize, u64)>,
    f_writes: Vec<(usize, u64)>,
    csr_writes: Vec<(u16, u64)>,
}

fn hex(token: &str) -> Result<u64, String> {
    let digits = token
        .strip_prefix("0x")
        .ok_or_else(|| format!("expected hex value, got {token:?}"))?;
    u64::from_str_radix(digits, 16).map_err(|e| format!("bad hex {token:?}: {e}"))
}

fn parse_commit(line: &str) -> Result<Option<Commit>, String> {
    let Some(rest) = line.strip_prefix("core") else {
        return Ok(None);
    };
    let mut parts = rest.split_whitespace();
    // "0:" "3" "0xpc" "(0xinsn)" then the written state.
    let _hart = parts.next().ok_or("missing hart")?;
    let priv_mode: u8 = parts
        .next()
        .ok_or("missing privilege")?
        .parse()
        .map_err(|e| format!("bad privilege: {e}"))?;
    let pc = hex(parts.next().ok_or("missing pc")?)?;
    let insn_token = parts.next().ok_or("missing instruction")?;
    let insn_digits = insn_token
        .strip_prefix("(0x")
        .and_then(|t| t.strip_suffix(')'))
        .ok_or_else(|| format!("bad instruction token {insn_token:?}"))?;
    let insn = u32::from_str_radix(insn_digits, 16)
        .map_err(|e| format!("bad instruction {insn_token:?}: {e}"))?;

    let mut commit = Commit {
        pc,
        priv_mode,
        insn,
        x_writes: Vec::new(),
        f_writes: Vec::new(),
        csr_writes: Vec::new(),
    };

    while let Some(token) = parts.next() {
        if token == "mem" {
            // Loads log the address; stores log the address and the value.
            let _addr = parts.next().ok_or("mem without address")?;
            if parts.clone().next().is_some_and(|t| t.starts_with("0x")) {
                let _value = parts.next();
            }
            continue;
        }
        if let Some(idx) = token.strip_prefix('x') {
            if let Ok(idx) = idx.parse::<usize>() {
                let value = hex(parts.next().ok_or("x write without value")?)?;
                commit.x_writes.push((idx, value));
                continue;
            }
        }
        if let Some(idx) = token.strip_prefix('f') {
            if let Ok(idx) = idx.parse::<usize>() {
                let value = hex(parts.next().ok_or("f write without value")?)?;
                commit.f_writes.push((idx, value));
                continue;
            }
        }
        if let Some(csr) = token.strip_prefix('c') {
            if let Some((num, _name)) = csr.split_once('_') {
                let addr: u16 = num.parse().map_err(|e| format!("bad csr {token:?}: {e}"))?;
                let value = hex(parts.next().ok_or("csr write without value")?)?;
                commit.csr_writes.push((addr, value));
                continue;
            }
        }
        return Err(format!("unrecognised commit token {token:?} in {line:?}"));
    }
    Ok(Some(commit))
}

fn spike_trace(
    spike: &Path,
    isa: &str,
    elf: &Path,
    timeout_secs: u64,
) -> Result<Vec<Commit>, String> {
    let output = Command::new("timeout")
        .arg(timeout_secs.to_string())
        .arg(spike)
        .arg(format!("--isa={isa}"))
        .arg("--log-commits")
        .arg(elf)
        .output()
        .map_err(|e| format!("spawn {}: {e}", spike.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
        return Err(format!(
            "spike exited with {}: {}",
            output.status,
            tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
        ));
    }

    let log = String::from_utf8_lossy(&output.stderr);
    let mut commits = Vec::new();
    for line in log.lines() {
        if let Some(commit) = parse_commit(line)? {
            commits.push(commit);
        }
    }
    if commits.is_empty() {
        return Err("spike produced no commit records".to_string());
    }
    Ok(commits)
}

fn fp_bits_equal(a: u64, b: u64) -> bool {
    if a == b {
        return true;
    }
    // NaN results may carry implementation-chosen payloads; compare the NaN
    // class (and NaN-boxing) instead of the payload.
    let both_f64 = f64::from_bits(a).is_nan() && f64::from_bits(b).is_nan();
    let both_f32 = f32::from_bits(a as u32).is_nan()
        && f32::from_bits(b as u32).is_nan()
        && (a >> 32) == (b >> 32);
    both_f64 || both_f32
}

fn csr_value(cpu: &RiscvCpu, addr: u16) -> Option<u64> {
    let fcsr = cpu.csr().fcsr;
    match addr {
        0x001 => Some((fcsr & 0x1f) as u64),
        0x002 => Some(((fcsr >> 5) & 0x7) as u64),
        0x003 => Some(fcsr as u64),
        _ => None,
    }
}

/// Spike only logs retired instructions, so an instruction that traps is absent
/// from its commit log. Detect a trap in our CPU by the trap CSRs it updates.
fn trap_signature(cpu: &RiscvCpu) -> (u64, u64, u64, u64) {
    let csr = cpu.csr();
    (csr.mcause, csr.mepc, csr.scause, csr.sepc)
}

/// Address the CPU will trap to for the highest-priority pending interrupt.
fn pending_interrupt_target(cpu: &RiscvCpu) -> Option<u64> {
    let cause = cpu.csr().pending_interrupt(cpu.priv_mode())?;
    let delegated = cpu.priv_mode() <= PRV_S && cpu.csr().is_delegated(MCAUSE_INTERRUPT | cause);
    let tvec = if delegated {
        cpu.csr().stvec
    } else {
        cpu.csr().mtvec
    };
    let base = tvec & !0x3;
    Some(if tvec & 0x3 == 1 {
        base + cause * 4
    } else {
        base
    })
}

fn check_insn(cpu: &RiscvCpu, commit: &Commit, index: usize, name: &str) -> Result<(), String> {
    let word = cpu.read_phys(commit.pc, 4) as u32;
    let mask = if word & 0x3 == 0x3 {
        0xffff_ffff
    } else {
        0xffff
    };
    if word & mask != commit.insn & mask {
        return Err(format!(
            "{name}: commit {index} at pc {:#x}: insn {:#010x} != spike {:#010x}",
            commit.pc, word, commit.insn
        ));
    }
    Ok(())
}

fn check_writes(cpu: &RiscvCpu, commit: &Commit, index: usize, name: &str) -> Result<(), String> {
    // mvendorid/marchid/mimpid are implementation-defined: Spike and RhetSim
    // report different IDs by design, so skip the destination of reads of them.
    let id_read = commit.insn & 0x7f == 0x73
        && matches!((commit.insn >> 12) & 0x7, 2 | 3)
        && matches!(commit.insn >> 20, 0xF11..=0xF13);
    let id_rd = ((commit.insn >> 7) & 0x1f) as usize;

    for &(reg, expected) in &commit.x_writes {
        if id_read && reg == id_rd {
            continue;
        }
        let actual = cpu.reg(reg);
        if actual != expected {
            return Err(format!(
                "{name}: commit {index} at pc {:#x} ({:#010x}): x{reg} {:#x} != spike {:#x}",
                commit.pc, commit.insn, actual, expected
            ));
        }
    }
    for &(reg, expected) in &commit.f_writes {
        let actual = cpu.freg(reg);
        if !fp_bits_equal(actual, expected) {
            return Err(format!(
                "{name}: commit {index} at pc {:#x} ({:#010x}): f{reg} {:#x} != spike {:#x}",
                commit.pc, commit.insn, actual, expected
            ));
        }
    }
    for &(addr, expected) in &commit.csr_writes {
        if let Some(actual) = csr_value(cpu, addr) {
            if actual != expected {
                return Err(format!(
                    "{name}: commit {index} at pc {:#x} ({:#010x}): csr {:#x} {:#x} != spike {:#x}",
                    commit.pc, commit.insn, addr, actual, expected
                ));
            }
        }
    }
    Ok(())
}

fn replay(image: &Image, trace: &[Commit], name: &str) -> Result<(), String> {
    let write_tohost = image
        .write_tohost
        .ok_or_else(|| format!("{name}: ELF has no write_tohost symbol"))?;

    let mut cpu = RiscvCpu::new_with_type(
        "spike-difftest",
        MEM_SIZE,
        MEM_BASE,
        RiscvCpuType::THeadC908Full,
    );
    cpu.set_jit_enabled(false);
    for (offset, bytes) in &image.segments {
        cpu.load_binary(bytes, *offset);
    }
    cpu.set_pc(image.entry);
    cpu.set_reg(10, 0); // a0 = hartid
    cpu.set_reg(11, 0); // a1 = DTB

    let mut hidden_steps = 0usize;
    let mut last_trapped: Option<(u64, u32)> = None;
    for (index, commit) in trace.iter().enumerate() {
        // An interrupt is taken before the instruction Spike logs as the first
        // handler instruction. `exec(1)` performs the pending-interrupt check
        // and then retires that handler instruction in the same call.
        if let Some(target) = pending_interrupt_target(&cpu) {
            if target != commit.pc {
                return Err(format!(
                    "{name}: commit {index}: pending interrupt vectors to {target:#x}, \
                     spike commits {:#x}",
                    commit.pc
                ));
            }
            check_insn(&cpu, commit, index, name)?;
            cpu.exec(1)
                .map_err(|e| format!("{name}: commit {index}: {e:?}"))?;
            if cpu.priv_mode() != commit.priv_mode {
                return Err(format!(
                    "{name}: commit {index} at pc {:#x}: priv {} != spike {}",
                    commit.pc,
                    cpu.priv_mode(),
                    commit.priv_mode
                ));
            }
            check_writes(&cpu, commit, index, name)?;
            last_trapped = None;
            continue;
        }

        // A trapping instruction is not retired by Spike and has no commit of
        // its own; consume those before executing the next committed
        // instruction. Every step we take here must be a trap.
        let mut consumed = 0;
        while cpu.pc() != commit.pc {
            if let Some((pc, insn)) = last_trapped {
                return Err(format!(
                    "{name}: commit {index}: our CPU trapped on {insn:#010x} at {pc:#x} \
                     (now at {:#x}), but Spike retired it and continued at {:#x}",
                    cpu.pc(),
                    commit.pc
                ));
            }
            if consumed == 16 {
                return Err(format!(
                    "{name}: commit {index}: our CPU is at {:#x} after 16 untraced \
                     instructions, spike expects {:#x}",
                    cpu.pc(),
                    commit.pc
                ));
            }
            consumed += 1;
            hidden_steps += 1;
            let pc = cpu.pc();
            let insn = cpu.read_phys(pc, 4) as u32;
            let size = if insn & 0x3 == 0x3 { 4 } else { 2 };
            let before = trap_signature(&cpu);
            cpu.exec(1)
                .map_err(|e| format!("{name}: commit {index}: {e:?}"))?;
            // A trap redirects the PC away from the fall-through address. The
            // trap-CSR comparison alone misses a second trap with identical
            // cause and EPC, and a handler that happens to be the next
            // instruction would look like a fall-through.
            if trap_signature(&cpu) == before && cpu.pc() == pc.wrapping_add(size) {
                return Err(format!(
                    "{name}: commit {index}: Spike did not retire {insn:#010x} at {pc:#x}, \
                     but our CPU executed it without trapping (now at {:#x})",
                    cpu.pc()
                ));
            }
        }

        if cpu.priv_mode() != commit.priv_mode {
            return Err(format!(
                "{name}: commit {index} at pc {:#x}: priv {} != spike {}",
                commit.pc,
                cpu.priv_mode(),
                commit.priv_mode
            ));
        }
        check_insn(&cpu, commit, index, name)?;

        let before = trap_signature(&cpu);
        cpu.exec(1)
            .map_err(|e| format!("{name}: commit {index}: {e:?}"))?;
        last_trapped = (trap_signature(&cpu) != before).then_some((commit.pc, commit.insn));
        check_writes(&cpu, commit, index, name)?;
    }

    if cpu.pc() != write_tohost {
        return Err(format!(
            "{name}: replay ended at {:#x}, expected write_tohost {:#x} ({} untraced steps)",
            cpu.pc(),
            write_tohost,
            hidden_steps
        ));
    }
    Ok(())
}

fn check_test(spike: &Path, isa: &str, path: &Path) -> Result<usize, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let image = parse_image(path);
    let write_tohost = image
        .write_tohost
        .ok_or_else(|| format!("{name}: ELF has no write_tohost symbol"))?;

    let commits =
        spike_trace(spike, isa, path, SPIKE_TIMEOUT_SECS).map_err(|e| format!("{name}: {e}"))?;
    if commits.len() > MAX_COMMITS {
        return Err(format!(
            "{name}: spike log has {} commits (limit {MAX_COMMITS})",
            commits.len()
        ));
    }

    // Spike boots from its ROM at 0x1000 and jumps to the ELF entry; skip the
    // ROM prefix and stop before the `write_tohost` loop.
    let start = commits
        .iter()
        .position(|c| c.pc == image.entry)
        .ok_or_else(|| format!("{name}: spike never reached entry {:#x}", image.entry))?;
    let stop = commits[start..]
        .iter()
        .position(|c| c.pc == write_tohost)
        .map(|i| start + i)
        .ok_or_else(|| format!("{name}: spike never reached write_tohost {write_tohost:#x}"))?;

    let trace = &commits[start..stop];
    if trace.is_empty() {
        return Err(format!("{name}: empty trace"));
    }
    replay(&image, trace, &name)?;
    Ok(trace.len())
}

#[test]
fn riscv_tests_match_spike() {
    let spike = match std::env::var_os("RHETSIM_SPIKE") {
        Some(path) => PathBuf::from(path),
        None => {
            eprintln!("RHETSIM_SPIKE is not set; skipping Spike difftest");
            return;
        }
    };
    if !spike.is_file() {
        eprintln!("RHETSIM_SPIKE={} is not a file; skipping", spike.display());
        return;
    }
    let dir = match std::env::var_os("RHETSIM_ISA_TESTS") {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("RHETSIM_ISA_TESTS is not set; skipping Spike difftest");
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
    let isa = std::env::var("RHETSIM_SPIKE_ISA").unwrap_or_else(|_| DEFAULT_ISA.to_string());

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
    let mut skipped = Vec::new();
    let mut ran = 0usize;
    let mut commits = 0usize;
    for path in &entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Some((_, reason)) = SPIKE_SKIP.iter().find(|(skip, _)| *skip == name) {
            skipped.push(format!("{name} ({reason})"));
            continue;
        }
        match check_test(&spike, &isa, path) {
            Ok(count) => {
                ran += 1;
                commits += count;
            }
            Err(message) => failures.push(message),
        }
    }

    eprintln!(
        "spike difftest: {ran}/{} tests, {commits} instructions{}",
        entries.len(),
        if skipped.is_empty() {
            String::new()
        } else {
            format!(", skipped: {}", skipped.join(", "))
        }
    );
    if !failures.is_empty() {
        panic!(
            "{} of {} riscv-tests diverged from Spike:\n{}",
            failures.len(),
            entries.len(),
            failures.join("\n")
        );
    }
}
