//! RV64IMAG instruction decoding

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
    // CSR instructions
    Csr    { funct3: u8, rd: u8, rs1: u8, csr: u16 },
    // System
    Ecall,
    Ebreak,
    Mret,
    Sret,
    Wfi,
    Fence,
    SfenceVma { rs1: u8, rs2: u8 },
    // Atomics (A extension)
    Amo    { funct5: u8, aq: bool, rl: bool, funct3: u8, rd: u8, rs1: u8, rs2: u8 },
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
        0x33 => {
            let f7 = funct7(insn);
            if f7 == 0x01 {
                // M extension (RV64M)
                Instruction::Op {
                    funct3: funct3(insn), funct7: f7,
                    rd: rd(insn), rs1: rs1(insn), rs2: rs2(insn),
                }
            } else {
                Instruction::Op {
                    funct3: funct3(insn), funct7: f7,
                    rd: rd(insn), rs1: rs1(insn), rs2: rs2(insn),
                }
            }
        }
        0x1B => Instruction::OpImm32 {
            funct3: funct3(insn), rd: rd(insn), rs1: rs1(insn), imm: i_imm(insn),
            funct7: funct7(insn),
        },
        0x3B => Instruction::Op32 {
            funct3: funct3(insn), funct7: funct7(insn),
            rd: rd(insn), rs1: rs1(insn), rs2: rs2(insn),
        },
        0x73 => {
            let f3 = funct3(insn);
            if f3 == 0 {
                let imm = insn >> 20;
                match imm {
                    0x000 => Instruction::Ecall,
                    0x001 => Instruction::Ebreak,
                    0x102 => Instruction::Sret,
                    0x302 => Instruction::Mret,
                    0x105 => Instruction::Wfi,
                    _ => {
                        if (insn >> 25) == 0x09 {
                            Instruction::SfenceVma { rs1: rs1(insn), rs2: rs2(insn) }
                        } else {
                            Instruction::Unknown(insn)
                        }
                    }
                }
            } else {
                // CSR instructions: funct3 1-3 = CSRRW/CSRRS/CSRRC
                //                   funct3 5-7 = CSRRWI/CSRRSI/CSRRCI
                let csr_addr = ((insn >> 20) & 0xFFF) as u16;
                Instruction::Csr {
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn), // for Imm variants, this is uimm
                    csr: csr_addr,
                }
            }
        }
        0x0F => Instruction::Fence,
        0x2F => {
            // A extension (atomics)
            let f5 = (insn >> 27) & 0x1F;
            let aq = (insn >> 26) & 1 != 0;
            let rl = (insn >> 25) & 1 != 0;
            Instruction::Amo {
                funct5: f5 as u8, aq, rl,
                funct3: funct3(insn),
                rd: rd(insn), rs1: rs1(insn), rs2: rs2(insn),
            }
        }
        _ => Instruction::Unknown(insn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_addi() {
        let insn = decode(0x02A00093);
        match insn {
            Instruction::OpImm { funct3: 0, rd: 1, rs1: 0, imm: 42, .. } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn decode_lui() {
        let insn = decode(0x123452B7);
        assert_eq!(insn, Instruction::Lui { rd: 5, imm: 0x12345000 });
    }

    #[test]
    fn decode_sw() {
        let insn = decode(0x00112023);
        assert_eq!(insn, Instruction::Store { funct3: 2, rs1: 2, rs2: 1, offset: 0 });
    }

    #[test]
    fn decode_beq() {
        let insn = decode(0x00008463);
        assert_eq!(insn, Instruction::Branch { funct3: 0, rs1: 1, rs2: 0, offset: 8 });
    }

    #[test]
    fn decode_ecall() {
        assert_eq!(decode(0x00000073), Instruction::Ecall);
    }

    #[test]
    fn decode_jal() {
        let insn = decode(0x000000EF);
        assert_eq!(insn, Instruction::Jal { rd: 1, offset: 0 });
    }

    #[test]
    fn decode_negative_imm() {
        let insn = decode(0xFFF00093);
        assert_eq!(insn, Instruction::OpImm {
            funct3: 0, rd: 1, rs1: 0, imm: -1, funct7: 0x7F,
        });
    }

    #[test]
    fn decode_csrrw() {
        // CSRRW x1, mstatus(0x300), x2
        // funct3=001, csr=0x300, rs1=x2, rd=x1
        // 0011_0000_0000_00010_001_00001_1110011
        let raw = 0x300_11_0_73u32;
        // Let me construct properly:
        // [31:20]=0x300, [19:15]=rs1=2, [14:12]=001, [11:7]=rd=1, [6:0]=0x73
        let raw = (0x300 << 20) | (2 << 15) | (1 << 12) | (1 << 7) | 0x73;
        let insn = decode(raw);
        match insn {
            Instruction::Csr { funct3: 1, rd: 1, rs1: 2, csr: 0x300 } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn decode_mret() {
        // MRET: 0x30200073
        assert_eq!(decode(0x30200073), Instruction::Mret);
    }

    #[test]
    fn decode_amo_add_w() {
        // AMOADD.W rd=1, rs1=2, rs2=3, aq=0, rl=0
        // funct5=00000, aq=0, rl=0, rs2=3, rs1=2, funct3=010, rd=1, opcode=0101111
        let raw = (0b00000 << 27) | (3 << 20) | (2 << 15) | (0b010 << 12) | (1 << 7) | 0x2F;
        let insn = decode(raw);
        match insn {
            Instruction::Amo { funct5: 0, aq: false, rl: false, funct3: 2, rd: 1, rs1: 2, rs2: 3 } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }
}
