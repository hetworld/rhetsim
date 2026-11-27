/// Hand-assembled RV64I program: writes "Hello from RhetSim!\n" to UART at 0x10000000, then halts.
/// Assumes PC starts at 0x80000000.
///
/// Instruction sequence:
///   LUI  a1, 0x10000        ; a1 = 0x10000000 (UART base)
///   For each char c:
///     ADDI a0, x0, c        ; a0 = c
///     SB   a0, 0(a1)        ; UART THR write
///   ECALL                   ; halt
pub fn hello_program() -> Vec<u8> {
    let mut program: Vec<u32> = Vec::new();

    // LUI a1(x11), 0x10000 → a1 = 0x10000000
    // imm[31:12]=0x10000 | rd=11 | opcode=0x37
    program.push(0x10000_5B7);

    for &ch in b"Hello from RhetSim!\n" {
        // ADDI a0(x10), x0, ch
        // imm[11:0]=ch | rs1=0 | funct3=0 | rd=10 | opcode=0x13
        program.push(((ch as u32) << 20) | 0x0000_0513);
        // SB a0(x10), 0(a1(x11))
        // imm[11:5]=0 | rs2=10 | rs1=11 | funct3=0 | imm[4:0]=0 | opcode=0x23
        program.push(0x00A5_8023);
    }

    // ECALL (halt)
    program.push(0x0000_0073);

    program.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_structure() {
        let bin = hello_program();
        let msg = b"Hello from RhetSim!\n";
        // 1 (LUI) + 2*len (ADDI+SB per char) + 1 (ECALL) = 2 + 2*20 = 42 instructions
        assert_eq!(bin.len(), (1 + 2 * msg.len() + 1) * 4);

        // First instruction: LUI a1, 0x10000
        assert_eq!(&bin[0..4], &0x100005B7u32.to_le_bytes());

        // Second instruction: ADDI a0, x0, 'H' (0x48)
        assert_eq!(&bin[4..8], &0x04800513u32.to_le_bytes());

        // Third instruction: SB a0, 0(a1)
        assert_eq!(&bin[8..12], &0x00A58023u32.to_le_bytes());

        // Last instruction: ECALL
        assert_eq!(&bin[bin.len()-4..], &0x00000073u32.to_le_bytes());
    }
}
