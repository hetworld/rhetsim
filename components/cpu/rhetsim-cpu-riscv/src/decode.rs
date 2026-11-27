//! RV64IMAC instruction decoding

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    Lui {
        rd: u8,
        imm: i64,
    },
    Auipc {
        rd: u8,
        imm: i64,
    },
    Jal {
        rd: u8,
        offset: i64,
    },
    Jalr {
        rd: u8,
        rs1: u8,
        offset: i64,
    },
    Branch {
        funct3: u8,
        rs1: u8,
        rs2: u8,
        offset: i64,
    },
    Load {
        funct3: u8,
        rd: u8,
        rs1: u8,
        offset: i64,
    },
    Store {
        funct3: u8,
        rs1: u8,
        rs2: u8,
        offset: i64,
    },
    // Floating-point load/store
    FpLoad {
        width: u8,
        rd: u8,
        rs1: u8,
        offset: i64,
    },
    FpStore {
        width: u8,
        rs1: u8,
        rs2: u8,
        offset: i64,
    },
    // Floating-point arithmetic/compare/convert
    FpOp {
        funct7: u8,
        funct3: u8,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    // Fused multiply-add family
    FpFma {
        opcode: u8,
        funct7: u8,
        rd: u8,
        rs1: u8,
        rs2: u8,
        rs3: u8,
    },
    // Zba/Zbb register-register operations
    Bitmanip {
        opcode: u8,
        funct7: u8,
        funct3: u8,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    // Zba/Zbb immediate operations (funct12 encodes the operation/shamt)
    BitmanipImm {
        opcode: u8,
        funct3: u8,
        rd: u8,
        rs1: u8,
        funct12: u16,
    },
    OpImm {
        funct3: u8,
        rd: u8,
        rs1: u8,
        imm: i64,
        funct7: u8,
    },
    Op {
        funct3: u8,
        funct7: u8,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    OpImm32 {
        funct3: u8,
        rd: u8,
        rs1: u8,
        imm: i64,
        funct7: u8,
    },
    Op32 {
        funct3: u8,
        funct7: u8,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    // CSR instructions
    Csr {
        funct3: u8,
        rd: u8,
        rs1: u8,
        csr: u16,
    },
    // System
    Ecall,
    Ebreak,
    Mret,
    Sret,
    Wfi,
    Fence,
    FenceI,
    SfenceVma {
        rs1: u8,
        rs2: u8,
    },
    Cbo {
        op: u8,
        rs1: u8,
    },
    /// T-Head XTheadSync barrier (no architectural state).
    TheadSync,
    // Atomics (A extension)
    Amo {
        funct5: u8,
        aq: bool,
        rl: bool,
        funct3: u8,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    Unknown(u32),
}

fn rd(insn: u32) -> u8 {
    ((insn >> 7) & 0x1F) as u8
}
fn rs1(insn: u32) -> u8 {
    ((insn >> 15) & 0x1F) as u8
}
fn rs2(insn: u32) -> u8 {
    ((insn >> 20) & 0x1F) as u8
}
fn funct3(insn: u32) -> u8 {
    ((insn >> 12) & 0x7) as u8
}
fn funct7(insn: u32) -> u8 {
    ((insn >> 25) & 0x7F) as u8
}

fn i_imm(insn: u32) -> i64 {
    (insn as i32 >> 20) as i64
}

fn s_imm(insn: u32) -> i64 {
    let lo = (insn >> 7) & 0x1F;
    let hi = (insn >> 25) & 0x7F;
    let val = (hi << 5) | lo;
    ((val as i32) << 20 >> 20) as i64
}

fn b_imm(insn: u32) -> i64 {
    let b11 = (insn >> 7) & 1;
    let b4_1 = (insn >> 8) & 0xF;
    let b10_5 = (insn >> 25) & 0x3F;
    let b12 = (insn >> 31) & 1;
    let val = (b12 << 12) | (b11 << 11) | (b10_5 << 5) | (b4_1 << 1);
    ((val as i32) << 19 >> 19) as i64
}

fn u_imm(insn: u32) -> i64 {
    (insn & 0xFFFFF000) as i32 as i64
}

fn j_imm(insn: u32) -> i64 {
    let b19_12 = (insn >> 12) & 0xFF;
    let b11 = (insn >> 20) & 1;
    let b10_1 = (insn >> 21) & 0x3FF;
    let b20 = (insn >> 31) & 1;
    let val = (b20 << 20) | (b19_12 << 12) | (b11 << 11) | (b10_1 << 1);
    ((val as i32) << 11 >> 11) as i64
}

pub fn decode(insn: u32) -> Instruction {
    let opcode = insn & 0x7F;
    match opcode {
        0x37 => Instruction::Lui {
            rd: rd(insn),
            imm: u_imm(insn),
        },
        0x17 => Instruction::Auipc {
            rd: rd(insn),
            imm: u_imm(insn),
        },
        0x6F => Instruction::Jal {
            rd: rd(insn),
            offset: j_imm(insn),
        },
        0x67 => Instruction::Jalr {
            rd: rd(insn),
            rs1: rs1(insn),
            offset: i_imm(insn),
        },
        0x63 => Instruction::Branch {
            funct3: funct3(insn),
            rs1: rs1(insn),
            rs2: rs2(insn),
            offset: b_imm(insn),
        },
        0x03 => Instruction::Load {
            funct3: funct3(insn),
            rd: rd(insn),
            rs1: rs1(insn),
            offset: i_imm(insn),
        },
        0x07 => match funct3(insn) {
            2 => Instruction::FpLoad {
                width: 4,
                rd: rd(insn),
                rs1: rs1(insn),
                offset: i_imm(insn),
            },
            3 => Instruction::FpLoad {
                width: 8,
                rd: rd(insn),
                rs1: rs1(insn),
                offset: i_imm(insn),
            },
            _ => Instruction::Unknown(insn),
        },
        0x23 => Instruction::Store {
            funct3: funct3(insn),
            rs1: rs1(insn),
            rs2: rs2(insn),
            offset: s_imm(insn),
        },
        0x27 => match funct3(insn) {
            2 => Instruction::FpStore {
                width: 4,
                rs1: rs1(insn),
                rs2: rs2(insn),
                offset: s_imm(insn),
            },
            3 => Instruction::FpStore {
                width: 8,
                rs1: rs1(insn),
                rs2: rs2(insn),
                offset: s_imm(insn),
            },
            _ => Instruction::Unknown(insn),
        },
        0x53 => Instruction::FpOp {
            funct7: funct7(insn),
            funct3: funct3(insn),
            rd: rd(insn),
            rs1: rs1(insn),
            rs2: rs2(insn),
        },
        0x43 | 0x47 | 0x4B | 0x4F => Instruction::FpFma {
            opcode: (insn & 0x7F) as u8,
            funct7: funct7(insn),
            rd: rd(insn),
            rs1: rs1(insn),
            rs2: rs2(insn),
            rs3: ((insn >> 27) & 0x1F) as u8,
        },
        0x13 => {
            let f3 = funct3(insn);
            let f7 = funct7(insn);
            let funct12 = ((insn >> 20) & 0xFFF) as u16;
            let bitmanip = (f3 == 1 && (f7 == 0x30 || f7 == 0x14 || f7 == 0x24 || f7 == 0x34))
                || (f3 == 5 && (f7 == 0x30 || f7 == 0x35 || f7 == 0x14 || f7 == 0x24));
            if bitmanip {
                Instruction::BitmanipImm {
                    opcode: 0x13,
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    funct12,
                }
            } else {
                Instruction::OpImm {
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    imm: i_imm(insn),
                    funct7: f7,
                }
            }
        }
        0x33 => {
            let f7 = funct7(insn);
            let f3 = funct3(insn);
            let bitmanip = matches!(
                (f7, f3),
                (0x10, 2 | 4 | 6)
                    | (0x20, 4 | 6 | 7)
                    | (0x05, 1..=3)
                    | (0x05, 4..=7)
                    | (0x14, 1)
                    | (0x24, 1 | 5)
                    | (0x34, 1)
                    | (0x30, 1 | 5)
            );
            if bitmanip {
                Instruction::Bitmanip {
                    opcode: 0x33,
                    funct7: f7,
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    rs2: rs2(insn),
                }
            } else {
                Instruction::Op {
                    funct3: f3,
                    funct7: f7,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    rs2: rs2(insn),
                }
            }
        }
        0x1B => {
            let f3 = funct3(insn);
            let f7 = funct7(insn);
            let funct12 = ((insn >> 20) & 0xFFF) as u16;
            if (f3 == 1 && (f7 == 0x04 || f7 == 0x05 || f7 == 0x30)) || (f3 == 5 && f7 == 0x30) {
                Instruction::BitmanipImm {
                    opcode: 0x1B,
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    funct12,
                }
            } else {
                Instruction::OpImm32 {
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    imm: i_imm(insn),
                    funct7: f7,
                }
            }
        }
        0x3B => {
            let f3 = funct3(insn);
            let f7 = funct7(insn);
            let bitmanip = matches!((f7, f3), (0x04, 0 | 4) | (0x10, 2 | 4 | 6) | (0x30, 1 | 5));
            if bitmanip {
                Instruction::Bitmanip {
                    opcode: 0x3B,
                    funct7: f7,
                    funct3: f3,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    rs2: rs2(insn),
                }
            } else {
                Instruction::Op32 {
                    funct3: f3,
                    funct7: f7,
                    rd: rd(insn),
                    rs1: rs1(insn),
                    rs2: rs2(insn),
                }
            }
        }
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
                    // Svinval fine-grained fences.
                    0x180 | 0x1A0 => Instruction::SfenceVma { rs1: 0, rs2: 0 },
                    _ => {
                        if (insn >> 25) == 0x09 {
                            Instruction::SfenceVma {
                                rs1: rs1(insn),
                                rs2: rs2(insn),
                            }
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
        0x0B => match insn {
            0x0180_000B | 0x0190_000B | 0x01A0_000B | 0x01B0_000B => Instruction::TheadSync,
            _ if insn & 0xFE00_7FFF == 0x0400_000B => Instruction::SfenceVma {
                rs1: rs1(insn),
                rs2: rs2(insn),
            },
            _ => Instruction::Unknown(insn),
        },
        0x0F => match funct3(insn) {
            1 => Instruction::FenceI,
            2 => Instruction::Cbo {
                op: ((insn >> 20) & 0xF) as u8,
                rs1: rs1(insn),
            },
            _ => Instruction::Fence,
        },
        0x2F => {
            // A extension (atomics)
            let f5 = (insn >> 27) & 0x1F;
            let aq = (insn >> 26) & 1 != 0;
            let rl = (insn >> 25) & 1 != 0;
            Instruction::Amo {
                funct5: f5 as u8,
                aq,
                rl,
                funct3: funct3(insn),
                rd: rd(insn),
                rs1: rs1(insn),
                rs2: rs2(insn),
            }
        }
        _ => Instruction::Unknown(insn),
    }
}

pub fn decode_compressed(half: u16) -> Instruction {
    let op = half & 0x3;
    let funct3_c = (half >> 13) & 0x7;

    match op {
        0b00 => {
            let rd_p = (((half >> 2) & 0x7) + 8) as u8;
            let rs1_p = (((half >> 7) & 0x7) + 8) as u8;
            match funct3_c {
                0b000 => {
                    // C.ADDI4SPN: addi rd', sp, nzuimm
                    let nzuimm = ((half >> 6) & 0x1) << 2
                        | ((half >> 5) & 0x1) << 3
                        | ((half >> 11) & 0x3) << 4
                        | ((half >> 7) & 0xF) << 6;
                    if nzuimm == 0 {
                        return Instruction::Unknown(half as u32);
                    }
                    Instruction::OpImm {
                        funct3: 0,
                        rd: rd_p,
                        rs1: 2,
                        imm: nzuimm as i64,
                        funct7: 0,
                    }
                }
                0b010 => {
                    // C.LW: lw rd', offset(rs1')
                    let off = ((half >> 6) & 0x1) << 2
                        | ((half >> 10) & 0x7) << 3
                        | ((half >> 5) & 0x1) << 6;
                    Instruction::Load {
                        funct3: 2,
                        rd: rd_p,
                        rs1: rs1_p,
                        offset: off as i64,
                    }
                }
                0b011 => {
                    // C.LD: ld rd', offset(rs1')
                    let off = ((half >> 10) & 0x7) << 3 | ((half >> 5) & 0x3) << 6;
                    Instruction::Load {
                        funct3: 3,
                        rd: rd_p,
                        rs1: rs1_p,
                        offset: off as i64,
                    }
                }
                0b110 => {
                    // C.SW: sw rs2', offset(rs1')
                    let rs2_p = rd_p; // same bit field
                    let off = ((half >> 6) & 0x1) << 2
                        | ((half >> 10) & 0x7) << 3
                        | ((half >> 5) & 0x1) << 6;
                    Instruction::Store {
                        funct3: 2,
                        rs1: rs1_p,
                        rs2: rs2_p,
                        offset: off as i64,
                    }
                }
                0b111 => {
                    // C.SD: sd rs2', offset(rs1')
                    let rs2_p = rd_p;
                    let off = ((half >> 10) & 0x7) << 3 | ((half >> 5) & 0x3) << 6;
                    Instruction::Store {
                        funct3: 3,
                        rs1: rs1_p,
                        rs2: rs2_p,
                        offset: off as i64,
                    }
                }
                _ => Instruction::Unknown(half as u32),
            }
        }
        0b01 => {
            match funct3_c {
                0b000 => {
                    // C.NOP / C.ADDI: addi rd, rd, nzimm
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let imm = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                    let imm = ((imm as i16) << 10 >> 10) as i64; // sign-extend 6-bit
                    Instruction::OpImm {
                        funct3: 0,
                        rd,
                        rs1: rd,
                        imm,
                        funct7: 0,
                    }
                }
                0b001 => {
                    // C.ADDIW: addiw rd, rd, imm
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let imm = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                    let imm = ((imm as i16) << 10 >> 10) as i64;
                    Instruction::OpImm32 {
                        funct3: 0,
                        rd,
                        rs1: rd,
                        imm,
                        funct7: 0,
                    }
                }
                0b010 => {
                    // C.LI: addi rd, x0, imm
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let imm = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                    let imm = ((imm as i16) << 10 >> 10) as i64;
                    Instruction::OpImm {
                        funct3: 0,
                        rd,
                        rs1: 0,
                        imm,
                        funct7: 0,
                    }
                }
                0b011 => {
                    let rd = ((half >> 7) & 0x1F) as u8;
                    if rd == 2 {
                        // C.ADDI16SP: addi sp, sp, nzimm (scaled by 16)
                        let nzimm = (((half >> 6) & 0x1) << 4)
                            | (((half >> 2) & 0x1) << 5)
                            | (((half >> 5) & 0x1) << 6)
                            | (((half >> 3) & 0x3) << 7)
                            | (((half >> 12) & 0x1) << 9);
                        let nzimm = ((nzimm as i16) << 6 >> 6) as i64; // sign-extend 10-bit
                        if nzimm == 0 {
                            return Instruction::Unknown(half as u32);
                        }
                        Instruction::OpImm {
                            funct3: 0,
                            rd: 2,
                            rs1: 2,
                            imm: nzimm,
                            funct7: 0,
                        }
                    } else {
                        // C.LUI: lui rd, nzimm
                        let nzimm = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                        let nzimm = ((nzimm as i32) << 26 >> 26) as i64; // sign-extend 6-bit
                        if nzimm == 0 {
                            return Instruction::Unknown(half as u32);
                        }
                        let imm = nzimm << 12;
                        Instruction::Lui { rd, imm }
                    }
                }
                0b100 => {
                    let rd_p = (((half >> 7) & 0x7) + 8) as u8;
                    let funct2 = (half >> 10) & 0x3;
                    match funct2 {
                        0b00 => {
                            // C.SRLI: srli rd', rd', shamt
                            let shamt = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                            Instruction::OpImm {
                                funct3: 5,
                                rd: rd_p,
                                rs1: rd_p,
                                imm: shamt as i64,
                                funct7: 0,
                            }
                        }
                        0b01 => {
                            // C.SRAI: srai rd', rd', shamt
                            let shamt = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                            Instruction::OpImm {
                                funct3: 5,
                                rd: rd_p,
                                rs1: rd_p,
                                imm: shamt as i64,
                                funct7: 0x20,
                            }
                        }
                        0b10 => {
                            // C.ANDI: andi rd', rd', imm
                            let imm = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                            let imm = ((imm as i16) << 10 >> 10) as i64;
                            Instruction::OpImm {
                                funct3: 7,
                                rd: rd_p,
                                rs1: rd_p,
                                imm,
                                funct7: 0,
                            }
                        }
                        0b11 => {
                            let rs2_p = (((half >> 2) & 0x7) + 8) as u8;
                            let funct1 = (half >> 12) & 0x1;
                            let funct2b = (half >> 5) & 0x3;
                            match (funct1, funct2b) {
                                (0, 0b00) => Instruction::Op {
                                    funct3: 0,
                                    funct7: 0x20,
                                    rd: rd_p,
                                    rs1: rd_p,
                                    rs2: rs2_p,
                                }, // C.SUB
                                (0, 0b01) => Instruction::Op {
                                    funct3: 4,
                                    funct7: 0x00,
                                    rd: rd_p,
                                    rs1: rd_p,
                                    rs2: rs2_p,
                                }, // C.XOR
                                (0, 0b10) => Instruction::Op {
                                    funct3: 6,
                                    funct7: 0x00,
                                    rd: rd_p,
                                    rs1: rd_p,
                                    rs2: rs2_p,
                                }, // C.OR
                                (0, 0b11) => Instruction::Op {
                                    funct3: 7,
                                    funct7: 0x00,
                                    rd: rd_p,
                                    rs1: rd_p,
                                    rs2: rs2_p,
                                }, // C.AND
                                (1, 0b00) => Instruction::Op32 {
                                    funct3: 0,
                                    funct7: 0x20,
                                    rd: rd_p,
                                    rs1: rd_p,
                                    rs2: rs2_p,
                                }, // C.SUBW
                                (1, 0b01) => Instruction::Op32 {
                                    funct3: 0,
                                    funct7: 0x00,
                                    rd: rd_p,
                                    rs1: rd_p,
                                    rs2: rs2_p,
                                }, // C.ADDW
                                _ => Instruction::Unknown(half as u32),
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                0b101 => {
                    // C.J: jal x0, offset
                    let off = ((half >> 3) & 0x7) << 1
                        | ((half >> 11) & 0x1) << 4
                        | ((half >> 2) & 0x1) << 5
                        | ((half >> 7) & 0x1) << 6
                        | ((half >> 6) & 0x1) << 7
                        | ((half >> 9) & 0x3) << 8
                        | ((half >> 8) & 0x1) << 10
                        | ((half >> 12) & 0x1) << 11;
                    let off = ((off as i16) << 4 >> 4) as i64; // sign-extend 12-bit
                    Instruction::Jal { rd: 0, offset: off }
                }
                0b110 => {
                    // C.BEQZ: beq rs1', x0, offset
                    let rs1_p = (((half >> 7) & 0x7) + 8) as u8;
                    let off = ((half >> 3) & 0x3) << 1
                        | ((half >> 10) & 0x3) << 3
                        | ((half >> 2) & 0x1) << 5
                        | ((half >> 5) & 0x3) << 6
                        | ((half >> 12) & 0x1) << 8;
                    let off = ((off as i16) << 7 >> 7) as i64; // sign-extend 9-bit
                    Instruction::Branch {
                        funct3: 0,
                        rs1: rs1_p,
                        rs2: 0,
                        offset: off,
                    }
                }
                0b111 => {
                    // C.BNEZ: bne rs1', x0, offset
                    let rs1_p = (((half >> 7) & 0x7) + 8) as u8;
                    let off = ((half >> 3) & 0x3) << 1
                        | ((half >> 10) & 0x3) << 3
                        | ((half >> 2) & 0x1) << 5
                        | ((half >> 5) & 0x3) << 6
                        | ((half >> 12) & 0x1) << 8;
                    let off = ((off as i16) << 7 >> 7) as i64;
                    Instruction::Branch {
                        funct3: 1,
                        rs1: rs1_p,
                        rs2: 0,
                        offset: off,
                    }
                }
                _ => Instruction::Unknown(half as u32),
            }
        }
        0b10 => {
            match funct3_c {
                0b000 => {
                    // C.SLLI: slli rd, rd, shamt
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let shamt = ((half >> 2) & 0x1F) | (((half >> 12) & 0x1) << 5);
                    Instruction::OpImm {
                        funct3: 1,
                        rd,
                        rs1: rd,
                        imm: shamt as i64,
                        funct7: 0,
                    }
                }
                0b010 => {
                    // C.LWSP: lw rd, offset(sp)
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let off = ((half >> 4) & 0x7) << 2
                        | ((half >> 12) & 0x1) << 5
                        | ((half >> 2) & 0x3) << 6;
                    Instruction::Load {
                        funct3: 2,
                        rd,
                        rs1: 2,
                        offset: off as i64,
                    }
                }
                0b011 => {
                    // C.LDSP: ld rd, offset(sp)
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let off = ((half >> 5) & 0x3) << 3
                        | ((half >> 12) & 0x1) << 5
                        | ((half >> 2) & 0x7) << 6;
                    Instruction::Load {
                        funct3: 3,
                        rd,
                        rs1: 2,
                        offset: off as i64,
                    }
                }
                0b100 => {
                    let bit12 = (half >> 12) & 0x1;
                    let rd = ((half >> 7) & 0x1F) as u8;
                    let rs2 = ((half >> 2) & 0x1F) as u8;
                    if bit12 == 0 {
                        if rs2 == 0 {
                            // C.JR: jalr x0, 0(rs1)
                            Instruction::Jalr {
                                rd: 0,
                                rs1: rd,
                                offset: 0,
                            }
                        } else {
                            // C.MV: add rd, x0, rs2
                            Instruction::Op {
                                funct3: 0,
                                funct7: 0,
                                rd,
                                rs1: 0,
                                rs2,
                            }
                        }
                    } else {
                        if rd == 0 && rs2 == 0 {
                            // C.EBREAK
                            Instruction::Ebreak
                        } else if rs2 == 0 {
                            // C.JALR: jalr x1, 0(rs1)
                            Instruction::Jalr {
                                rd: 1,
                                rs1: rd,
                                offset: 0,
                            }
                        } else {
                            // C.ADD: add rd, rd, rs2
                            Instruction::Op {
                                funct3: 0,
                                funct7: 0,
                                rd,
                                rs1: rd,
                                rs2,
                            }
                        }
                    }
                }
                0b110 => {
                    // C.SWSP: sw rs2, offset(sp)
                    let rs2 = ((half >> 2) & 0x1F) as u8;
                    let off = ((half >> 9) & 0xF) << 2 | ((half >> 7) & 0x3) << 6;
                    Instruction::Store {
                        funct3: 2,
                        rs1: 2,
                        rs2,
                        offset: off as i64,
                    }
                }
                0b111 => {
                    // C.SDSP: sd rs2, offset(sp)
                    let rs2 = ((half >> 2) & 0x1F) as u8;
                    let off = ((half >> 10) & 0x7) << 3 | ((half >> 7) & 0x7) << 6;
                    Instruction::Store {
                        funct3: 3,
                        rs1: 2,
                        rs2,
                        offset: off as i64,
                    }
                }
                _ => Instruction::Unknown(half as u32),
            }
        }
        _ => Instruction::Unknown(half as u32),
    }
}

#[cfg(test)]
mod tests {
    // Instruction encodings are written with explicit zero fields for clarity.
    #![allow(clippy::identity_op)]

    use super::*;

    #[test]
    fn decode_addi() {
        let insn = decode(0x02A00093);
        match insn {
            Instruction::OpImm {
                funct3: 0,
                rd: 1,
                rs1: 0,
                imm: 42,
                ..
            } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn decode_lui() {
        let insn = decode(0x123452B7);
        assert_eq!(
            insn,
            Instruction::Lui {
                rd: 5,
                imm: 0x12345000
            }
        );
    }

    #[test]
    fn decode_sw() {
        let insn = decode(0x00112023);
        assert_eq!(
            insn,
            Instruction::Store {
                funct3: 2,
                rs1: 2,
                rs2: 1,
                offset: 0
            }
        );
    }

    #[test]
    fn decode_beq() {
        let insn = decode(0x00008463);
        assert_eq!(
            insn,
            Instruction::Branch {
                funct3: 0,
                rs1: 1,
                rs2: 0,
                offset: 8
            }
        );
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
        assert_eq!(
            insn,
            Instruction::OpImm {
                funct3: 0,
                rd: 1,
                rs1: 0,
                imm: -1,
                funct7: 0x7F,
            }
        );
    }

    #[test]
    fn decode_csrrw() {
        // CSRRW x1, mstatus(0x300), x2
        // funct3=001, csr=0x300, rs1=x2, rd=x1
        // 0011_0000_0000_00010_001_00001_1110011
        // [31:20]=0x300, [19:15]=rs1=2, [14:12]=001, [11:7]=rd=1, [6:0]=0x73
        let raw = (0x300 << 20) | (2 << 15) | (1 << 12) | (1 << 7) | 0x73;
        let insn = decode(raw);
        match insn {
            Instruction::Csr {
                funct3: 1,
                rd: 1,
                rs1: 2,
                csr: 0x300,
            } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn decode_mret() {
        // MRET: 0x30200073
        assert_eq!(decode(0x30200073), Instruction::Mret);
    }

    #[test]
    fn decode_c_addi() {
        // C.ADDI x15, x15, -1 → 0x17FD (bit12=1, imm[4:0]=0x1F, rd=15)
        // bits: 000 1 01111 11101 01
        let insn = decode_compressed(0x17FD);
        assert_eq!(
            insn,
            Instruction::OpImm {
                funct3: 0,
                rd: 15,
                rs1: 15,
                imm: -1,
                funct7: 0
            }
        );
    }

    #[test]
    fn decode_c_li() {
        // C.LI x10, 7 → addi x10, x0, 7
        // funct3=010, imm[5]=0, rd=10, imm[4:0]=00111, op=01
        let half: u16 = (0b010 << 13) | (0 << 12) | (10 << 7) | (0b00111 << 2) | 0b01;
        let insn = decode_compressed(half);
        assert_eq!(
            insn,
            Instruction::OpImm {
                funct3: 0,
                rd: 10,
                rs1: 0,
                imm: 7,
                funct7: 0
            }
        );
    }

    #[test]
    fn decode_c_mv() {
        // C.MV x1, x2 → add x1, x0, x2
        // funct3=100, bit12=0, rd=1, rs2=2, op=10
        let half: u16 = (0b100 << 13) | (0 << 12) | (1 << 7) | (2 << 2) | 0b10;
        let insn = decode_compressed(half);
        assert_eq!(
            insn,
            Instruction::Op {
                funct3: 0,
                funct7: 0,
                rd: 1,
                rs1: 0,
                rs2: 2
            }
        );
    }

    #[test]
    fn decode_c_jr() {
        // C.JR x1 → jalr x0, 0(x1)
        // funct3=100, bit12=0, rs1=1, rs2=0, op=10
        let half: u16 = (0b100 << 13) | (0 << 12) | (1 << 7) | (0 << 2) | 0b10;
        let insn = decode_compressed(half);
        assert_eq!(
            insn,
            Instruction::Jalr {
                rd: 0,
                rs1: 1,
                offset: 0
            }
        );
    }

    #[test]
    fn decode_c_jalr() {
        // C.JALR x5 → jalr x1, 0(x5)
        // funct3=100, bit12=1, rs1=5, rs2=0, op=10
        let half: u16 = (0b100 << 13) | (1 << 12) | (5 << 7) | (0 << 2) | 0b10;
        let insn = decode_compressed(half);
        assert_eq!(
            insn,
            Instruction::Jalr {
                rd: 1,
                rs1: 5,
                offset: 0
            }
        );
    }

    #[test]
    fn decode_c_add() {
        // C.ADD x1, x1, x2 → add x1, x1, x2
        // funct3=100, bit12=1, rd=1, rs2=2, op=10
        let half: u16 = (0b100 << 13) | (1 << 12) | (1 << 7) | (2 << 2) | 0b10;
        let insn = decode_compressed(half);
        assert_eq!(
            insn,
            Instruction::Op {
                funct3: 0,
                funct7: 0,
                rd: 1,
                rs1: 1,
                rs2: 2
            }
        );
    }

    #[test]
    fn decode_c_lwsp() {
        // C.LWSP x10, 4(sp) → lw x10, 4(x2)
        // offset=4: offset[4:2]=001 → bits[6:4]=001, offset[5]=0 → bit12=0, offset[7:6]=00 → bits[3:2]=00
        let half: u16 = (0b010 << 13) | (0 << 12) | (10 << 7) | (0b001 << 4) | (0b00 << 2) | 0b10;
        let insn = decode_compressed(half);
        assert_eq!(
            insn,
            Instruction::Load {
                funct3: 2,
                rd: 10,
                rs1: 2,
                offset: 4
            }
        );
    }

    #[test]
    fn decode_c_ebreak() {
        // C.EBREAK: funct3=100, bit12=1, rd=0, rs2=0, op=10
        let half: u16 = (0b100 << 13) | (1 << 12) | (0 << 7) | (0 << 2) | 0b10;
        let insn = decode_compressed(half);
        assert_eq!(insn, Instruction::Ebreak);
    }

    #[test]
    fn decode_amo_add_w() {
        // AMOADD.W rd=1, rs1=2, rs2=3, aq=0, rl=0
        // funct5=00000, aq=0, rl=0, rs2=3, rs1=2, funct3=010, rd=1, opcode=0101111
        let raw = (0b00000 << 27) | (3 << 20) | (2 << 15) | (0b010 << 12) | (1 << 7) | 0x2F;
        let insn = decode(raw);
        match insn {
            Instruction::Amo {
                funct5: 0,
                aq: false,
                rl: false,
                funct3: 2,
                rd: 1,
                rs1: 2,
                rs2: 3,
            } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }
}
