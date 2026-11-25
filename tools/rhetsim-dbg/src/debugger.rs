use rhetsim_core::error::OmniResult;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::mem::{MemTarget, MemAccessSize};
use std::collections::HashSet;

pub struct DebugSession {
    breakpoints: HashSet<u64>,
}

impl DebugSession {
    pub fn new() -> Self {
        Self { breakpoints: HashSet::new() }
    }

    pub fn add_breakpoint(&mut self, addr: u64) -> bool {
        self.breakpoints.insert(addr)
    }

    pub fn remove_breakpoint(&mut self, addr: u64) -> bool {
        self.breakpoints.remove(&addr)
    }

    pub fn has_breakpoint(&self, addr: u64) -> bool {
        self.breakpoints.contains(&addr)
    }

    pub fn breakpoints(&self) -> &HashSet<u64> {
        &self.breakpoints
    }

    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    pub fn step(&self, backend: &mut dyn ExecutionBackend) -> OmniResult<u64> {
        backend.exec(1)
    }

    pub fn continue_exec(
        &self,
        backend: &mut dyn ExecutionBackend,
        pc_fn: impl Fn() -> u64,
        max_steps: u64,
    ) -> OmniResult<(u64, Option<u64>)> {
        let mut total = 0u64;
        while total < max_steps {
            backend.exec(1)?;
            total += 1;
            let pc = pc_fn();
            if self.breakpoints.contains(&pc) {
                return Ok((total, Some(pc)));
            }
        }
        Ok((total, None))
    }

    pub fn dump_memory(
        &self,
        target: &dyn MemTarget,
        offset: u64,
        len: usize,
    ) -> OmniResult<Vec<u8>> {
        let mut buf = vec![0u8; len];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = target.read(offset + i as u64, MemAccessSize::U8)? as u8;
        }
        Ok(buf)
    }

    pub fn format_memory(addr: u64, data: &[u8]) -> String {
        let mut out = String::new();
        for (i, chunk) in data.chunks(16).enumerate() {
            let line_addr = addr + (i * 16) as u64;
            out.push_str(&format!("{:08x}: ", line_addr));
            for (j, &byte) in chunk.iter().enumerate() {
                if j == 8 { out.push(' '); }
                out.push_str(&format!("{:02x} ", byte));
            }
            out.push_str(" |");
            for &byte in chunk {
                let c = if byte.is_ascii_graphic() || byte == b' ' { byte as char } else { '.' };
                out.push(c);
            }
            out.push_str("|\n");
        }
        out
    }

    pub fn format_regs_rv64(regs: &[u64; 32], pc: u64) -> String {
        let names = [
            "zero","ra","sp","gp","tp","t0","t1","t2",
            "s0","s1","a0","a1","a2","a3","a4","a5",
            "a6","a7","s2","s3","s4","s5","s6","s7",
            "s8","s9","s10","s11","t3","t4","t5","t6",
        ];
        let mut out = format!("pc  = {:#018x}\n", pc);
        for (i, (name, val)) in names.iter().zip(regs.iter()).enumerate() {
            out.push_str(&format!("{:4} = {:#018x}", name, val));
            if i % 4 == 3 { out.push('\n'); } else { out.push_str("  "); }
        }
        out
    }
}

impl Default for DebugSession {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_breakpoint_add_remove() {
        let mut session = DebugSession::new();
        assert!(session.add_breakpoint(0x1000));
        assert!(session.add_breakpoint(0x2000));
        assert!(!session.add_breakpoint(0x1000)); // duplicate
        assert!(session.has_breakpoint(0x1000));
        assert!(session.has_breakpoint(0x2000));
        assert!(!session.has_breakpoint(0x3000));
        assert_eq!(session.breakpoints().len(), 2);

        assert!(session.remove_breakpoint(0x1000));
        assert!(!session.has_breakpoint(0x1000));
        assert!(!session.remove_breakpoint(0x9999));

        session.clear_breakpoints();
        assert!(session.breakpoints().is_empty());
    }

    #[test]
    fn test_format_memory() {
        let data: Vec<u8> = (0x41..=0x50).collect(); // 'A'..'P'
        let formatted = DebugSession::format_memory(0x1000, &data);
        assert!(formatted.contains("00001000:"));
        assert!(formatted.contains("41 "));
        assert!(formatted.contains("|ABCDEFGH"));
    }

    #[test]
    fn test_format_regs() {
        let mut regs = [0u64; 32];
        regs[1] = 0xDEAD;
        regs[10] = 42;
        let formatted = DebugSession::format_regs_rv64(&regs, 0x8000_0000);
        assert!(formatted.contains("pc  = 0x0000000080000000"));
        assert!(formatted.contains("ra"));
        assert!(formatted.contains("a0"));
        assert!(formatted.contains("0x000000000000dead"));
        assert!(formatted.contains("0x000000000000002a"));
    }
}
