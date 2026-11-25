//! TcgIrBlock / TcgOp — TCG 中间表示

#[derive(Debug, Clone)]
pub enum TcgOp {
    LoadReg  { dst: u32, src_reg: u32 },
    StoreReg { src: u32, dst_reg: u32 },
    Add      { dst: u32, a: u32, b: u32 },
    Sub      { dst: u32, a: u32, b: u32 },
    Load     { dst: u32, addr: u32, size: u8 },
    Store    { src: u32, addr: u32, size: u8 },
    Branch   { cond: u32, target: u64 },
    Call     { target: u64 },
    Exit,
}

#[derive(Debug, Clone)]
pub struct TcgIrBlock {
    pub guest_pc:  u64,
    pub ops:       Vec<TcgOp>,
    pub num_temps: u32,
}

impl TcgIrBlock {
    pub fn new(guest_pc: u64) -> Self {
        Self { guest_pc, ops: Vec::new(), num_temps: 0 }
    }
}
