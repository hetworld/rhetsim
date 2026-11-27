//! AArch64 instruction decoding (core integer subset)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    X(u8), // X0-X30
    Sp,    // X31 as SP
    Zr,    // X31 as zero register
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftType {
    Lsl,
    Lsr,
    Asr,
    Ror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    Eq,
    Ne,
    Cs,
    Cc,
    Mi,
    Pl,
    Vs,
    Vc,
    Hi,
    Ls,
    Ge,
    Lt,
    Gt,
    Le,
    Al,
    Nv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    // Data processing — immediate
    AddImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u32,
        shift: bool,
    },
    SubImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u32,
        shift: bool,
    },
    AddsImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u32,
        shift: bool,
    },
    SubsImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u32,
        shift: bool,
    },
    MovZ {
        sf: bool,
        rd: u8,
        imm: u16,
        hw: u8,
    },
    MovK {
        sf: bool,
        rd: u8,
        imm: u16,
        hw: u8,
    },
    MovN {
        sf: bool,
        rd: u8,
        imm: u16,
        hw: u8,
    },
    // Logical — immediate
    AndImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u64,
    },
    OrrImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u64,
    },
    EorImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u64,
    },
    AndsImm {
        sf: bool,
        rd: u8,
        rn: u8,
        imm: u64,
    },
    // Data processing — register
    AddReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    SubReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    AddsReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    SubsReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    AndReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    OrrReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    EorReg {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: ShiftType,
        amount: u8,
    },
    // Multiply
    Madd {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    Msub {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    // Division
    Sdiv {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Udiv {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    // Shifts
    Lslv {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Lsrv {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Asrv {
        sf: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    // Loads and stores
    LdrImm {
        sf: bool,
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    StrImm {
        sf: bool,
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    LdrLit {
        sf: bool,
        rt: u8,
        offset: i64,
    },
    Ldp {
        sf: bool,
        rt: u8,
        rt2: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    Stp {
        sf: bool,
        rt: u8,
        rt2: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    LdrbImm {
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    StrbImm {
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    LdrhImm {
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    StrhImm {
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    LdrswImm {
        rt: u8,
        rn: u8,
        offset: i64,
        wb: WriteBack,
    },
    // Branches
    B {
        offset: i64,
    },
    Bl {
        offset: i64,
    },
    Br {
        rn: u8,
    },
    Blr {
        rn: u8,
    },
    Ret {
        rn: u8,
    },
    BCond {
        cond: Cond,
        offset: i64,
    },
    Cbz {
        sf: bool,
        rt: u8,
        offset: i64,
    },
    Cbnz {
        sf: bool,
        rt: u8,
        offset: i64,
    },
    // System
    Svc {
        imm: u16,
    },
    Mrs {
        rt: u8,
        sysreg: u16,
    },
    Msr {
        rt: u8,
        sysreg: u16,
    },
    Eret,
    Nop,
    Wfi,
    // Address generation
    Adr {
        rd: u8,
        offset: i64,
    },
    Adrp {
        rd: u8,
        offset: i64,
    },

    Unknown(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBack {
    None,
    Pre,
    Post,
}

fn rd(insn: u32) -> u8 {
    (insn & 0x1F) as u8
}
fn rn(insn: u32) -> u8 {
    ((insn >> 5) & 0x1F) as u8
}
fn rm(insn: u32) -> u8 {
    ((insn >> 16) & 0x1F) as u8
}
fn rt2(insn: u32) -> u8 {
    ((insn >> 10) & 0x1F) as u8
}
fn sf(insn: u32) -> bool {
    insn >> 31 != 0
}

fn decode_shift(sh: u32) -> ShiftType {
    match sh & 3 {
        0 => ShiftType::Lsl,
        1 => ShiftType::Lsr,
        2 => ShiftType::Asr,
        3 => ShiftType::Ror,
        _ => unreachable!(),
    }
}

fn decode_cond(c: u32) -> Cond {
    match c & 0xF {
        0x0 => Cond::Eq,
        0x1 => Cond::Ne,
        0x2 => Cond::Cs,
        0x3 => Cond::Cc,
        0x4 => Cond::Mi,
        0x5 => Cond::Pl,
        0x6 => Cond::Vs,
        0x7 => Cond::Vc,
        0x8 => Cond::Hi,
        0x9 => Cond::Ls,
        0xA => Cond::Ge,
        0xB => Cond::Lt,
        0xC => Cond::Gt,
        0xD => Cond::Le,
        0xE => Cond::Al,
        0xF => Cond::Nv,
        _ => unreachable!(),
    }
}

fn sign_extend(val: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((val as u64 as i64) << shift) >> shift
}

fn decode_bitmask_imm(sf: bool, n: u32, immr: u32, imms: u32) -> Option<u64> {
    let len = if n != 0 {
        6
    } else {
        let combined = (!imms) & 0x3F;
        if combined == 0 {
            return None;
        }
        63 - combined.leading_zeros()
    };
    if len == 0 {
        return None;
    }
    let size = 1u32 << len;
    let levels = size - 1;
    let s = imms & levels;
    let r = immr & levels;
    if s == levels {
        return None;
    }
    let welem: u64 = (1u64 << (s + 1)) - 1;
    let rotated = if r == 0 {
        welem
    } else {
        (welem >> r) | (welem << (size - r))
    } & ((1u64 << size) - 1);
    let mut result = 0u64;
    let mut pos = 0;
    while pos < 64 {
        result |= rotated << pos;
        pos += size;
    }
    if !sf {
        result &= 0xFFFF_FFFF;
    }
    Some(result)
}

pub fn decode(insn: u32) -> Instruction {
    // ═══ Branches ═════════════════════════════════════════════
    // B: 000101 imm26
    if insn >> 26 == 0b000101 {
        let imm26 = insn & 0x03FF_FFFF;
        return Instruction::B {
            offset: sign_extend(imm26 << 2, 28),
        };
    }
    // BL: 100101 imm26
    if insn >> 26 == 0b100101 {
        let imm26 = insn & 0x03FF_FFFF;
        return Instruction::Bl {
            offset: sign_extend(imm26 << 2, 28),
        };
    }
    // B.cond: 01010100 imm19 0 cond
    if (insn >> 24) & 0xFF == 0x54 {
        let imm19 = (insn >> 5) & 0x7FFFF;
        let cond = decode_cond(insn & 0xF);
        return Instruction::BCond {
            cond,
            offset: sign_extend(imm19 << 2, 21),
        };
    }
    // CBZ/CBNZ: sf 011010 op imm19 Rt  (bits[30:25] = 011010)
    if (insn >> 25) & 0x3F == 0b011010 {
        let is_sf = sf(insn);
        let op = (insn >> 24) & 1;
        let imm19 = (insn >> 5) & 0x7FFFF;
        let rt = rd(insn);
        let offset = sign_extend(imm19 << 2, 21);
        return if op == 0 {
            Instruction::Cbz {
                sf: is_sf,
                rt,
                offset,
            }
        } else {
            Instruction::Cbnz {
                sf: is_sf,
                rt,
                offset,
            }
        };
    }
    // BR/BLR/RET: 1101011 opc 11111 000000 Rn 00000
    if (insn >> 25) == 0b1101011
        && (insn >> 16) & 0x1F == 0b11111
        && (insn >> 10) & 0x3F == 0
        && insn & 0x1F == 0
    {
        let opc = (insn >> 21) & 0xF;
        let rn_val = rn(insn);
        return match opc {
            0b0000 => Instruction::Br { rn: rn_val },
            0b0001 => Instruction::Blr { rn: rn_val },
            0b0010 => Instruction::Ret { rn: rn_val },
            0b0100 => Instruction::Eret,
            _ => Instruction::Unknown(insn),
        };
    }

    // ═══ System instructions ══════════════════════════════════
    if insn == 0xD503201F {
        return Instruction::Nop;
    }
    if insn == 0xD503207F {
        return Instruction::Wfi;
    }
    // SVC: 11010100_000 imm16 000 01
    if insn >> 21 == 0b110_1010_0000 && (insn & 0x1F) == 1 {
        let imm16 = ((insn >> 5) & 0xFFFF) as u16;
        return Instruction::Svc { imm: imm16 };
    }
    // MRS: 1101010100 1 ...
    if (insn >> 20) & 0xFFF == 0xD53 {
        let sysreg = ((insn >> 5) & 0xFFFF) as u16;
        return Instruction::Mrs {
            rt: rd(insn),
            sysreg,
        };
    }
    // MSR: 1101010100 0 ...
    if (insn >> 20) & 0xFFF == 0xD51 {
        let sysreg = ((insn >> 5) & 0xFFFF) as u16;
        return Instruction::Msr {
            rt: rd(insn),
            sysreg,
        };
    }

    // ═══ ADR/ADRP ═════════════════════════════════════════════
    if (insn >> 24) & 0x1F == 0b10000 {
        let op = (insn >> 31) & 1;
        let immlo = (insn >> 29) & 3;
        let immhi = (insn >> 5) & 0x7FFFF;
        let imm = (immhi << 2) | immlo;
        let rd_val = rd(insn);
        if op == 0 {
            return Instruction::Adr {
                rd: rd_val,
                offset: sign_extend(imm, 21),
            };
        } else {
            return Instruction::Adrp {
                rd: rd_val,
                offset: sign_extend(imm << 12, 33),
            };
        }
    }

    // ═══ Data processing — immediate (bits[28:26] = 100) ═════
    if (insn >> 26) & 0x7 == 0b100 {
        // Add/Sub immediate: sf op S 100010 sh imm12 Rn Rd
        if (insn >> 23) & 0x3F == 0b100010 {
            let is_sf = sf(insn);
            let op = (insn >> 30) & 1;
            let s = (insn >> 29) & 1;
            let sh = (insn >> 22) & 1;
            let imm12 = (insn >> 10) & 0xFFF;
            let rn_val = rn(insn);
            let rd_val = rd(insn);
            return match (op, s) {
                (0, 0) => Instruction::AddImm {
                    sf: is_sf,
                    rd: rd_val,
                    rn: rn_val,
                    imm: imm12,
                    shift: sh != 0,
                },
                (0, 1) => Instruction::AddsImm {
                    sf: is_sf,
                    rd: rd_val,
                    rn: rn_val,
                    imm: imm12,
                    shift: sh != 0,
                },
                (1, 0) => Instruction::SubImm {
                    sf: is_sf,
                    rd: rd_val,
                    rn: rn_val,
                    imm: imm12,
                    shift: sh != 0,
                },
                (1, 1) => Instruction::SubsImm {
                    sf: is_sf,
                    rd: rd_val,
                    rn: rn_val,
                    imm: imm12,
                    shift: sh != 0,
                },
                _ => Instruction::Unknown(insn),
            };
        }
        // Move wide: sf opc 100101 hw imm16 Rd
        if (insn >> 23) & 0x3F == 0b100101 {
            let is_sf = sf(insn);
            let opc = (insn >> 29) & 0x3;
            let hw = ((insn >> 21) & 0x3) as u8;
            let imm16 = ((insn >> 5) & 0xFFFF) as u16;
            let rd_val = rd(insn);
            return match opc {
                0 => Instruction::MovN {
                    sf: is_sf,
                    rd: rd_val,
                    imm: imm16,
                    hw,
                },
                2 => Instruction::MovZ {
                    sf: is_sf,
                    rd: rd_val,
                    imm: imm16,
                    hw,
                },
                3 => Instruction::MovK {
                    sf: is_sf,
                    rd: rd_val,
                    imm: imm16,
                    hw,
                },
                _ => Instruction::Unknown(insn),
            };
        }
        // Logical immediate: sf opc 100100 N immr imms Rn Rd
        if (insn >> 23) & 0x3F == 0b100100 {
            let is_sf = sf(insn);
            let opc = (insn >> 29) & 0x3;
            let n = (insn >> 22) & 1;
            let immr = (insn >> 16) & 0x3F;
            let imms = (insn >> 10) & 0x3F;
            let rn_val = rn(insn);
            let rd_val = rd(insn);
            if let Some(imm) = decode_bitmask_imm(is_sf, n, immr, imms) {
                return match opc {
                    0 => Instruction::AndImm {
                        sf: is_sf,
                        rd: rd_val,
                        rn: rn_val,
                        imm,
                    },
                    1 => Instruction::OrrImm {
                        sf: is_sf,
                        rd: rd_val,
                        rn: rn_val,
                        imm,
                    },
                    2 => Instruction::EorImm {
                        sf: is_sf,
                        rd: rd_val,
                        rn: rn_val,
                        imm,
                    },
                    3 => Instruction::AndsImm {
                        sf: is_sf,
                        rd: rd_val,
                        rn: rn_val,
                        imm,
                    },
                    _ => Instruction::Unknown(insn),
                };
            }
            return Instruction::Unknown(insn);
        }
    }

    // ═══ Loads and stores ═════════════════════════════════════

    // LDR (literal): opc 011 V=0 00 imm19 Rt  (bits[29:24] = 011000)
    if (insn >> 24) & 0x3F == 0b011000 {
        let opc30 = (insn >> 30) & 3;
        let imm19 = (insn >> 5) & 0x7FFFF;
        let rt_val = rd(insn);
        return match opc30 {
            0 => Instruction::LdrLit {
                sf: false,
                rt: rt_val,
                offset: sign_extend(imm19 << 2, 21),
            },
            1 => Instruction::LdrLit {
                sf: true,
                rt: rt_val,
                offset: sign_extend(imm19 << 2, 21),
            },
            _ => Instruction::Unknown(insn),
        };
    }

    // LDP/STP: opc V=0 101 type L imm7 Rt2 Rn Rt  (bits[29:27]=101, bit[26]=0, bit[25]=0)
    if (insn >> 27) & 0x7 == 0b101 && (insn >> 26) & 1 == 0 && (insn >> 25) & 1 == 0 {
        let is_sf = sf(insn);
        let type_field = (insn >> 23) & 0x7;
        let l = (insn >> 22) & 1;
        let imm7 = (insn >> 15) & 0x7F;
        let scale: i64 = if is_sf { 8 } else { 4 };
        let offset = sign_extend(imm7, 7) * scale;
        let rt2_val = rt2(insn);
        let rn_val = rn(insn);
        let rt_val = rd(insn);
        let wb = match type_field {
            0b001 => WriteBack::Post,
            0b010 => WriteBack::None,
            0b011 => WriteBack::Pre,
            _ => return Instruction::Unknown(insn),
        };
        if l == 1 {
            return Instruction::Ldp {
                sf: is_sf,
                rt: rt_val,
                rt2: rt2_val,
                rn: rn_val,
                offset,
                wb,
            };
        } else {
            return Instruction::Stp {
                sf: is_sf,
                rt: rt_val,
                rt2: rt2_val,
                rn: rn_val,
                offset,
                wb,
            };
        }
    }

    // LDR/STR (unsigned offset): size 111 V=0 01 opc imm12 Rn Rt
    // bits[29:27]=111, bit[26]=0, bits[25:24]=01
    if (insn >> 27) & 0x7 == 0b111 && (insn >> 26) & 1 == 0 && (insn >> 24) & 0x3 == 0b01 {
        let size = (insn >> 30) & 3;
        let opc = (insn >> 22) & 3;
        let imm12 = ((insn >> 10) & 0xFFF) as i64;
        let rn_val = rn(insn);
        let rt_val = rd(insn);
        match (size, opc) {
            (3, 0) => {
                return Instruction::StrImm {
                    sf: true,
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 8,
                    wb: WriteBack::None,
                }
            }
            (3, 1) => {
                return Instruction::LdrImm {
                    sf: true,
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 8,
                    wb: WriteBack::None,
                }
            }
            (2, 0) => {
                return Instruction::StrImm {
                    sf: false,
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 4,
                    wb: WriteBack::None,
                }
            }
            (2, 1) => {
                return Instruction::LdrImm {
                    sf: false,
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 4,
                    wb: WriteBack::None,
                }
            }
            (0, 0) => {
                return Instruction::StrbImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12,
                    wb: WriteBack::None,
                }
            }
            (0, 1) => {
                return Instruction::LdrbImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12,
                    wb: WriteBack::None,
                }
            }
            (1, 0) => {
                return Instruction::StrhImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 2,
                    wb: WriteBack::None,
                }
            }
            (1, 1) => {
                return Instruction::LdrhImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 2,
                    wb: WriteBack::None,
                }
            }
            (2, 2) => {
                return Instruction::LdrswImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset: imm12 * 4,
                    wb: WriteBack::None,
                }
            }
            _ => {}
        }
    }

    // LDR/STR (pre/post-index): size 111 V=0 00 opc imm9 wb Rn Rt
    // bits[29:27]=111, bit[26]=0, bits[25:24]=00
    if (insn >> 27) & 0x7 == 0b111 && (insn >> 26) & 1 == 0 && (insn >> 24) & 0x3 == 0b00 {
        let size = (insn >> 30) & 3;
        let opc = (insn >> 22) & 3;
        let imm9 = (insn >> 12) & 0x1FF;
        let wb_type = (insn >> 10) & 3;
        let rn_val = rn(insn);
        let rt_val = rd(insn);
        let offset = sign_extend(imm9, 9);
        let wb = match wb_type {
            0b01 => WriteBack::Post,
            0b11 => WriteBack::Pre,
            _ => return Instruction::Unknown(insn),
        };
        match (size, opc) {
            (3, 0) => {
                return Instruction::StrImm {
                    sf: true,
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (3, 1) => {
                return Instruction::LdrImm {
                    sf: true,
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (2, 0) => {
                return Instruction::StrImm {
                    sf: false,
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (2, 1) => {
                return Instruction::LdrImm {
                    sf: false,
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (0, 0) => {
                return Instruction::StrbImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (0, 1) => {
                return Instruction::LdrbImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (1, 0) => {
                return Instruction::StrhImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (1, 1) => {
                return Instruction::LdrhImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            (2, 2) => {
                return Instruction::LdrswImm {
                    rt: rt_val,
                    rn: rn_val,
                    offset,
                    wb,
                }
            }
            _ => {}
        }
    }

    // ═══ Data processing — register ═══════════════════════════

    // Logical shifted register: sf opc 01010 shift N Rm imm6 Rn Rd
    if (insn >> 24) & 0x1F == 0b01010 {
        let is_sf = sf(insn);
        let opc = (insn >> 29) & 0x3;
        let sh = decode_shift((insn >> 22) & 3);
        let n = (insn >> 21) & 1;
        let rm_val = rm(insn);
        let amount = ((insn >> 10) & 0x3F) as u8;
        let rn_val = rn(insn);
        let rd_val = rd(insn);
        return match (opc, n) {
            (0, 0) => Instruction::AndReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            (1, 0) => Instruction::OrrReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            (2, 0) => Instruction::EorReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            _ => Instruction::Unknown(insn),
        };
    }

    // Add/Sub shifted register: sf op S 01011 shift 0 Rm imm6 Rn Rd
    if (insn >> 24) & 0x1F == 0b01011 {
        let is_sf = sf(insn);
        let op = (insn >> 30) & 1;
        let s = (insn >> 29) & 1;
        let sh = decode_shift((insn >> 22) & 3);
        let rm_val = rm(insn);
        let amount = ((insn >> 10) & 0x3F) as u8;
        let rn_val = rn(insn);
        let rd_val = rd(insn);
        return match (op, s) {
            (0, 0) => Instruction::AddReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            (0, 1) => Instruction::AddsReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            (1, 0) => Instruction::SubReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            (1, 1) => Instruction::SubsReg {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                shift: sh,
                amount,
            },
            _ => Instruction::Unknown(insn),
        };
    }

    // Data processing 2-source: sf 0 S 11010110 Rm opcode Rn Rd
    if (insn >> 21) & 0xFF == 0b11010110 {
        let is_sf = sf(insn);
        let rm_val = rm(insn);
        let opcode2 = (insn >> 10) & 0x3F;
        let rn_val = rn(insn);
        let rd_val = rd(insn);
        return match opcode2 {
            0b000010 => Instruction::Udiv {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
            },
            0b000011 => Instruction::Sdiv {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
            },
            0b001000 => Instruction::Lslv {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
            },
            0b001001 => Instruction::Lsrv {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
            },
            0b001010 => Instruction::Asrv {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
            },
            _ => Instruction::Unknown(insn),
        };
    }

    // Data processing 3-source (MADD/MSUB): sf op54 11011 op31 Rm o0 Ra Rn Rd
    if (insn >> 24) & 0x1F == 0b11011 {
        let is_sf = sf(insn);
        let o0 = (insn >> 15) & 1;
        let ra = ((insn >> 10) & 0x1F) as u8;
        let rm_val = rm(insn);
        let rn_val = rn(insn);
        let rd_val = rd(insn);
        return if o0 == 0 {
            Instruction::Madd {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                ra,
            }
        } else {
            Instruction::Msub {
                sf: is_sf,
                rd: rd_val,
                rn: rn_val,
                rm: rm_val,
                ra,
            }
        };
    }

    Instruction::Unknown(insn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_add_imm() {
        // ADD X1, X2, #42
        let insn = decode(0x9100A841);
        assert_eq!(
            insn,
            Instruction::AddImm {
                sf: true,
                rd: 1,
                rn: 2,
                imm: 42,
                shift: false
            }
        );
    }

    #[test]
    fn decode_sub_imm() {
        // SUB X1, X2, #10
        let insn = decode(0xD1002841);
        assert_eq!(
            insn,
            Instruction::SubImm {
                sf: true,
                rd: 1,
                rn: 2,
                imm: 10,
                shift: false
            }
        );
    }

    #[test]
    fn decode_movz() {
        // MOVZ X0, #0x1234
        let insn = decode(0xD2824680);
        assert_eq!(
            insn,
            Instruction::MovZ {
                sf: true,
                rd: 0,
                imm: 0x1234,
                hw: 0
            }
        );
    }

    #[test]
    fn decode_b() {
        // B +8
        let insn = decode(0x14000002);
        assert_eq!(insn, Instruction::B { offset: 8 });
    }

    #[test]
    fn decode_bl() {
        // BL +16
        let insn = decode(0x94000004);
        assert_eq!(insn, Instruction::Bl { offset: 16 });
    }

    #[test]
    fn decode_ret() {
        // RET (X30)
        let insn = decode(0xD65F03C0);
        assert_eq!(insn, Instruction::Ret { rn: 30 });
    }

    #[test]
    fn decode_nop() {
        assert_eq!(decode(0xD503201F), Instruction::Nop);
    }

    #[test]
    fn decode_add_reg() {
        // ADD X1, X2, X3
        let insn = decode(0x8B030041);
        assert_eq!(
            insn,
            Instruction::AddReg {
                sf: true,
                rd: 1,
                rn: 2,
                rm: 3,
                shift: ShiftType::Lsl,
                amount: 0
            }
        );
    }

    #[test]
    fn decode_subs_reg() {
        // SUBS X1, X2, X3
        let insn = decode(0xEB030041);
        assert_eq!(
            insn,
            Instruction::SubsReg {
                sf: true,
                rd: 1,
                rn: 2,
                rm: 3,
                shift: ShiftType::Lsl,
                amount: 0
            }
        );
    }

    #[test]
    fn decode_str_imm() {
        // STR X0, [X1]
        let insn = decode(0xF9000020);
        assert_eq!(
            insn,
            Instruction::StrImm {
                sf: true,
                rt: 0,
                rn: 1,
                offset: 0,
                wb: WriteBack::None
            }
        );
    }

    #[test]
    fn decode_ldr_imm() {
        // LDR X0, [X1, #8]
        let insn = decode(0xF9400420);
        assert_eq!(
            insn,
            Instruction::LdrImm {
                sf: true,
                rt: 0,
                rn: 1,
                offset: 8,
                wb: WriteBack::None
            }
        );
    }

    #[test]
    fn decode_svc() {
        // SVC #0
        let insn = decode(0xD4000001);
        assert_eq!(insn, Instruction::Svc { imm: 0 });
    }

    #[test]
    fn decode_cbz() {
        // CBZ X0, +12
        let insn = decode(0xB4000060);
        assert_eq!(
            insn,
            Instruction::Cbz {
                sf: true,
                rt: 0,
                offset: 12
            }
        );
    }

    #[test]
    fn decode_cbnz() {
        // CBNZ X0, -4
        let insn = decode(0xB5FFFFE0);
        assert_eq!(
            insn,
            Instruction::Cbnz {
                sf: true,
                rt: 0,
                offset: -4
            }
        );
    }

    #[test]
    fn decode_stp() {
        // STP X0, X1, [X2]
        let insn = decode(0xA9000440);
        assert_eq!(
            insn,
            Instruction::Stp {
                sf: true,
                rt: 0,
                rt2: 1,
                rn: 2,
                offset: 0,
                wb: WriteBack::None
            }
        );
    }

    #[test]
    fn decode_ldp() {
        // LDP X3, X4, [X2]
        let insn = decode(0xA9401043);
        assert_eq!(
            insn,
            Instruction::Ldp {
                sf: true,
                rt: 3,
                rt2: 4,
                rn: 2,
                offset: 0,
                wb: WriteBack::None
            }
        );
    }

    #[test]
    fn decode_strb() {
        // STRB W0, [X1]
        let insn = decode(0x39000020);
        assert_eq!(
            insn,
            Instruction::StrbImm {
                rt: 0,
                rn: 1,
                offset: 0,
                wb: WriteBack::None
            }
        );
    }

    #[test]
    fn decode_ldrb() {
        // LDRB W2, [X1]
        let insn = decode(0x39400022);
        assert_eq!(
            insn,
            Instruction::LdrbImm {
                rt: 2,
                rn: 1,
                offset: 0,
                wb: WriteBack::None
            }
        );
    }
}
