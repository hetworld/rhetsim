//! RV64I instruction decoding

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    Lui    { rd: u8, imm: i64 },
    Auipc  { rd: u8, imm: i64 },
    Jal    { rd: u8, offset: i64 },
    Jalr   { rd: u8, rs1: u8, offset: i64 },
    Branch { funct3: u8, rs1: u8, rs2: u8, offset: i64 },
    Load   { funct3: u8, rd: u8, rs1: u8, offset: i64 },
    Store  { funct3: u8, rs1: u8, rs2: u8, offset: i64 },
    OpImm  { funct3: u8, rd: u8, rs1: u8, imm: i64, funct7: u8 },
    Op     { funct3: u8, funct7: u8, rd: u8, rs1: u8, rs2: u8 },
    OpImm32 { funct3: u8, rd: u8, rs1: u8, imm: i64, funct7: u8 },
    Op32   { funct3: u8, funct7: u8, rd: u8, rs1: u8, rs2: u8 },
    Ecall,
    Ebreak,
    Fence,
    Unknown(u32),
}

fn rd(insn: u32) -> u8 { ((insn >> 7) & 0x1F) as u8 }
fn rs1(insn: u32) -> u8 { ((insn >> 15) & 0x1F) as u8 }
fn rs2(insn: u32) -> u8 { ((insn >> 20) & 0x1F) as u8 }
fn funct3(insn: u32) -> u8 { ((insn >> 12) & 0x7) as u8 }
fn funct7(insn: u32) -> u8 { ((insn >> 25) & 0x7F) as u8 }

fn i_imm(insn: u32) -> i64 { (insn as i32 >> 20) as i64 }

fn s_imm(insn: u32) -> i64 {
    let lo = (insn >> 7) & 0x1F;
    let hi = (insn >> 25) & 0x7F;
    let val = (hi << 5) | lo;
    ((val as i32) << 20 >> 20) as i64
}

fn b_imm(insn: u32) -> i64 {
    let b11  = (insn >> 7) & 1;
    let b4_1 = (insn >> 8) & 0xF;
    let b10_5 = (insn >> 25) & 0x3F;
    let b12  = (insn >> 31) & 1;
    let val = (b12 << 12) | (b11 << 11) | (b10_5 << 5) | (b4_1 << 1);
    ((val as i32) << 19 >> 19) as i64
}

fn u_imm(insn: u32) -> i64 { (insn & 0xFFFFF000) as i32 as i64 }

fn j_imm(insn: u32) -> i64 {
    let b19_12 = (insn >> 12) & 0xFF;
    let b11    = (insn >> 20) & 1;
    let b10_1  = (insn >> 21) & 0x3FF;
    let b20    = (insn >> 31) & 1;
    let val = (b20 << 20) | (b19_12 << 12) | (b11 << 11) | (b10_1 << 1);
    ((val as i32) << 11 >> 11) as i64
}

pub fn decode(insn: u32) -> Instruction {
    let opcode = insn & 0x7F;
    match opcode {
        0x37 => Instruction::Lui { rd: rd(insn), imm: u_imm(insn) },
        0x17 => Instruction::Auipc { rd: rd(insn), imm: u_imm(insn) },
        0x6F => Instruction::Jal { rd: rd(insn), offset: j_imm(insn) },
        0x67 => Instruction::Jalr { rd: rd(insn), rs1: rs1(insn), offset: i_imm(insn) },
        0x63 => Instruction::Branch {
            funct3: funct3(insn), rs1: rs1(insn), rs2: rs2(insn), offset: b_imm(insn),
        },
        0x03 => Instruction::Load {
            funct3: funct3(insn), rd: rd(insn), rs1: rs1(insn), offset: i_imm(insn),
        },
        0x23 => Instruction::Store {
            funct3: funct3(insn), rs1: rs1(insn), rs2: rs2(insn), offset: s_imm(insn),
        },
        0x13 => Instruction::OpImm {
            funct3: funct3(insn), rd: rd(insn), rs1: rs1(insn), imm: i_imm(insn),
            funct7: funct7(insn),
        },
        0x33 => Instruction::Op {
            funct3: funct3(insn), funct7: funct7(insn),
            rd: rd(insn), rs1: rs1(insn), rs2: rs2(insn),
        },
        0x1B => Instruction::OpImm32 {
            funct3: funct3(insn), rd: rd(insn), rs1: rs1(insn), imm: i_imm(insn),
            funct7: funct7(insn),
        },
        0x3B => Instruction::Op32 {
            funct3: funct3(insn), funct7: funct7(insn),
            rd: rd(insn), rs1: rs1(insn), rs2: rs2(insn),
        },
        0x73 => {
            let imm = (insn >> 20) & 0xFFF;
            match imm {
                0 => Instruction::Ecall,
                1 => Instruction::Ebreak,
                _ => Instruction::Unknown(insn),
            }
        }
        0x0F => Instruction::Fence,
        _ => Instruction::Unknown(insn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_addi() {
        // ADDI x1, x0, 42 => 0x02A00093
        let insn = decode(0x02A00093);
        // funct7 overlaps with imm for I-type; only matters for shifts
        match insn {
            Instruction::OpImm { funct3: 0, rd: 1, rs1: 0, imm: 42, .. } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn decode_lui() {
        // LUI x5, 0x12345 => 0x12345_2B7
        let insn = decode(0x123452B7);
        assert_eq!(insn, Instruction::Lui { rd: 5, imm: 0x12345000 });
    }

    #[test]
    fn decode_sw() {
        // SW x1, 0(x2) => 0x00112023
        let insn = decode(0x00112023);
        assert_eq!(insn, Instruction::Store { funct3: 2, rs1: 2, rs2: 1, offset: 0 });
    }

    #[test]
    fn decode_beq() {
        // BEQ x1, x0, +8:
        // B-type: imm[12|10:5] | rs2 | rs1 | funct3 | imm[4:1|11] | opcode
        // offset=8: b4_1=0100, rest 0
        // 0_000000_00000_00001_000_0100_0_1100011 = 0x00008463
        let insn = decode(0x00008463);
        // funct3=000 (BEQ), rs1=1, rs2=0, offset=8
        assert_eq!(insn, Instruction::Branch { funct3: 0, rs1: 1, rs2: 0, offset: 8 });
    }

    #[test]
    fn decode_ecall() {
        assert_eq!(decode(0x00000073), Instruction::Ecall);
    }

    #[test]
    fn decode_jal() {
        // JAL x1, +0 => 0x000000EF
        let insn = decode(0x000000EF);
        assert_eq!(insn, Instruction::Jal { rd: 1, offset: 0 });
    }

    #[test]
    fn decode_negative_imm() {
        // ADDI x1, x0, -1 => imm = 0xFFF => sign-extended to -1
        // 111111111111_00000_000_00001_0010011 = 0xFFF00093
        let insn = decode(0xFFF00093);
        assert_eq!(insn, Instruction::OpImm {
            funct3: 0, rd: 1, rs1: 0, imm: -1, funct7: 0x7F,
        });
    }
}
