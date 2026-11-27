use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::{Ieee32, Ieee64};
use cranelift_codegen::ir::types::*;
use cranelift_codegen::ir::{
    AbiParam, Block, BlockArg, FuncRef, Function, InstBuilder, MemFlagsData, Signature,
    UserFuncName, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use crate::decode::{decode, decode_compressed, Instruction};
use crate::profile::RiscvCpuType;

const EXIT_ECALL: u64 = 1;
const EXIT_TRAP: u64 = 2;
const EXIT_BLOCK_END: u64 = 3;
const EXIT_CSR: u64 = 4;
const EXIT_MRET: u64 = 5;
const EXIT_SRET: u64 = 6;
const EXIT_WFI: u64 = 7;
const EXIT_SFENCE: u64 = 8;
const EXIT_AMO: u64 = 9;
const EXIT_FENCEI: u64 = 10;
const EXIT_EBREAK: u64 = 11;
const EXIT_FAULT: u64 = 12;

#[repr(C)]
pub struct JitCpuState {
    pub regs: [u64; 32],
    pub pc: u64,
    pub exit_reason: u64,
    pub exit_pc: u64,
    pub _pad: u64,
    pub insn_count: u64,
    pub raw_insn: u64,
    /// Pointer to the CPU's 32 floating-point register bit patterns.
    ///
    /// Keeping the FP registers behind a pointer avoids copying 256 bytes on
    /// every translation-block entry; only `fcsr` and `fp_dirty` travel by
    /// value.
    pub fregs: *mut u64,
    /// `fcsr` (fflags | frm | fcsr) as read at block entry.
    pub fcsr: u64,
    /// Set by translated code when an FP instruction retires.
    pub fp_dirty: u64,
    /// `mstatus.FS` (`MSTATUS_FS_MASK`); FP instructions trap when zero.
    pub fs: u64,
}

// The generated code accesses these fields at hard-coded offsets.
const _: () = {
    assert!(std::mem::offset_of!(JitCpuState, pc) == 256);
    assert!(std::mem::offset_of!(JitCpuState, exit_reason) == 264);
    assert!(std::mem::offset_of!(JitCpuState, exit_pc) == 272);
    assert!(std::mem::offset_of!(JitCpuState, insn_count) == 288);
    assert!(std::mem::offset_of!(JitCpuState, raw_insn) == 296);
    assert!(std::mem::offset_of!(JitCpuState, fregs) == 304);
    assert!(std::mem::offset_of!(JitCpuState, fcsr) == 312);
    assert!(std::mem::offset_of!(JitCpuState, fp_dirty) == 320);
    assert!(std::mem::offset_of!(JitCpuState, fs) == 328);
};

pub type JitBlockFn = unsafe extern "C" fn(*mut JitCpuState, *mut u8);

extern "C" fn helper_load(mem_ctx: *mut u8, addr: u64, size: u64) -> u64 {
    let ctx = unsafe { &mut *(mem_ctx as *mut HelperContext) };
    ctx.fault = 0;
    (ctx.load_fn)(ctx.cpu_ptr, addr, size as usize, &mut ctx.fault)
}

extern "C" fn helper_store(mem_ctx: *mut u8, addr: u64, size: u64, val: u64) -> u64 {
    let ctx = unsafe { &mut *(mem_ctx as *mut HelperContext) };
    ctx.fault = 0;
    (ctx.store_fn)(ctx.cpu_ptr, addr, size as usize, val, &mut ctx.fault)
}

extern "C" fn helper_cbo_zero(mem_ctx: *mut u8, base: u64) -> u64 {
    let ctx = unsafe { &mut *(mem_ctx as *mut HelperContext) };
    ctx.fault = 0;
    let mut fault = 0u64;
    let ret = (ctx.cbo_zero_fn)(ctx.cpu_ptr, base, &mut fault);
    ctx.fault = fault;
    ret
}

// Cranelift's x86-64 backend lowers `fma` to the libm `fma`/`fmaf` libcalls
// unless the native-FMA ISA flag is enabled. Route those libcalls back to
// Rust's fused multiply-add so JIT and interpreter share one implementation.
extern "C" fn helper_fma_f64(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

extern "C" fn helper_fma_f32(a: f32, b: f32, c: f32) -> f32 {
    a.mul_add(b, c)
}

fn clmul_bits(a: u64, b: u64) -> (u64, u64) {
    let mut lo = 0u64;
    let mut hi = 0u64;
    for i in 0..64 {
        if (b >> i) & 1 != 0 {
            lo ^= a << i;
            if i != 0 {
                hi ^= a >> (64 - i);
            }
        }
    }
    (lo, hi)
}

extern "C" fn helper_clmul(a: u64, b: u64) -> u64 {
    clmul_bits(a, b).0
}

extern "C" fn helper_clmulh(a: u64, b: u64) -> u64 {
    clmul_bits(a, b).1
}

extern "C" fn helper_clmulr(a: u64, b: u64) -> u64 {
    let (lo, hi) = clmul_bits(a, b);
    (lo >> 63) | (hi << 1)
}

// State offsets used by generated code.
const STATE_FREGS: i32 = 304;
const STATE_FCSR: i32 = 312;
const STATE_FP_DIRTY: i32 = 320;
const STATE_FS: i32 = 328;

pub const FAST_TLB_SIZE: usize = 256;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FastTlbEntry {
    pub tag: u64,
    pub addend: u64,
}

impl FastTlbEntry {
    pub const fn invalid() -> Self {
        Self {
            tag: u64::MAX,
            addend: 0,
        }
    }
}

#[repr(C)]
pub struct HelperContext {
    pub cpu_ptr: *mut u8,
    pub load_fn: fn(*mut u8, u64, usize, *mut u64) -> u64,
    pub store_fn: fn(*mut u8, u64, usize, u64, *mut u64) -> u64,
    pub read_tlb: *mut FastTlbEntry,
    pub write_tlb: *mut FastTlbEntry,
    /// Set by the helpers when the access raised a guest page fault; the
    /// translated block checks this and exits with `EXIT_FAULT`.
    pub fault: u64,
    /// Zero a 64-byte cache block (`cbo.zero`).
    pub cbo_zero_fn: fn(*mut u8, u64, *mut u64) -> u64,
}

// The generated code loads these fields at hard-coded offsets.
const _: () = {
    assert!(std::mem::offset_of!(HelperContext, read_tlb) == 24);
    assert!(std::mem::offset_of!(HelperContext, write_tlb) == 32);
    assert!(std::mem::offset_of!(HelperContext, fault) == 40);
};

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
struct TbKey {
    pc: u64,
    priv_mode: u8,
    satp: u64,
}

#[allow(dead_code)]
struct TranslationBlock {
    code: JitBlockFn,
    guest_pc: u64,
    guest_end_pc: u64,
    insn_count: u64,
}

pub struct JitEngine {
    module: JITModule,
    tb_cache: HashMap<TbKey, TranslationBlock>,
    load_id: FuncId,
    store_id: FuncId,
    cbo_zero_id: FuncId,
    /// clmul, clmulh, clmulr
    clmul_ids: [FuncId; 3],
    fb_ctx: FunctionBuilderContext,
    tb_counter: u64,
    compiles: u64,
    executions: u64,
    invalidations: u64,
    fallbacks: u64,
    compressed: bool,
}

unsafe impl Send for JitEngine {}
unsafe impl Sync for JitEngine {}

fn memflags() -> MemFlagsData {
    MemFlagsData::new()
}

impl Default for JitEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl JitEngine {
    pub fn new() -> Self {
        Self::new_for(RiscvCpuType::default())
    }

    pub fn new_for(cpu_type: RiscvCpuType) -> Self {
        let mut flag_builder = settings::builder();
        // Blocks average ~5 instructions, so compile time dominates; the
        // optimizer costs far more than the generated code saves. The IR
        // verifier and unwind info are compile-time overhead we don't need
        // for JIT blocks, and opt_level=none makes alias analysis useless.
        flag_builder.set("opt_level", "none").unwrap();
        flag_builder.set("enable_verifier", "false").unwrap();
        flag_builder.set("unwind_info", "false").unwrap();
        flag_builder.set("enable_alias_analysis", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        let isa_builder =
            cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("helper_load", helper_load as *const u8);
        builder.symbol("helper_store", helper_store as *const u8);
        builder.symbol("helper_cbo_zero", helper_cbo_zero as *const u8);
        builder.symbol("helper_clmul", helper_clmul as *const u8);
        builder.symbol("helper_clmulh", helper_clmulh as *const u8);
        builder.symbol("helper_clmulr", helper_clmulr as *const u8);
        builder.symbol("fma", helper_fma_f64 as *const u8);
        builder.symbol("fmaf", helper_fma_f32 as *const u8);

        let mut module = JITModule::new(builder);

        let load_id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("helper_load", Linkage::Import, &sig)
                .unwrap()
        };

        let store_id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("helper_store", Linkage::Import, &sig)
                .unwrap()
        };

        let cbo_zero_id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function("helper_cbo_zero", Linkage::Import, &sig)
                .unwrap()
        };

        let clmul_ids = ["helper_clmul", "helper_clmulh", "helper_clmulr"].map(|name| {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module
                .declare_function(name, Linkage::Import, &sig)
                .unwrap()
        });

        JitEngine {
            module,
            tb_cache: HashMap::new(),
            load_id,
            store_id,
            cbo_zero_id,
            clmul_ids,
            fb_ctx: FunctionBuilderContext::new(),
            tb_counter: 0,
            compiles: 0,
            executions: 0,
            invalidations: 0,
            fallbacks: 0,
            compressed: cpu_type.has_compressed(),
        }
    }

    /// (compiled blocks, block executions, cache invalidations)
    pub fn stats(&self) -> (u64, u64, u64) {
        (self.compiles, self.executions, self.invalidations)
    }

    /// Number of block-compilation attempts that had to fall back to the
    /// interpreter (for example, on an unsupported XThead instruction).
    pub fn fallback_count(&self) -> u64 {
        self.fallbacks
    }

    pub fn invalidate_all(&mut self) {
        self.invalidations += 1;
        self.tb_cache.clear();
    }

    pub fn execute(
        &mut self,
        state: &mut JitCpuState,
        helper_ctx: &mut HelperContext,
        priv_mode: u8,
        satp: u64,
        fetch_fn: &dyn Fn(u64) -> Result<u64, (u64, u64)>,
    ) -> bool {
        let key = TbKey {
            pc: state.pc,
            priv_mode,
            satp,
        };

        if !self.tb_cache.contains_key(&key) {
            match self.compile_block(state.pc, fetch_fn) {
                Some(tb) => {
                    self.compiles += 1;
                    self.tb_cache.insert(key, tb);
                }
                None => {
                    self.fallbacks += 1;
                    return false;
                }
            }
        }

        self.executions += 1;
        let tb = self.tb_cache.get(&key).unwrap();
        state.insn_count = 0;
        unsafe {
            (tb.code)(state, helper_ctx as *mut HelperContext as *mut u8);
        }
        state.regs[0] = 0;
        true
    }

    fn compile_block(
        &mut self,
        start_pc: u64,
        fetch_fn: &dyn Fn(u64) -> Result<u64, (u64, u64)>,
    ) -> Option<TranslationBlock> {
        let mut insns = Vec::new();
        let mut pc = start_pc;
        let max_insns = 128;

        while let Ok(raw) = fetch_fn(pc) {
            let half = raw as u16;
            if half == 0 {
                break;
            }

            let (insn, size) = if half & 0x3 != 0x3 {
                if !self.compressed {
                    return None;
                }
                (decode_compressed(half), 2u64)
            } else {
                (decode(raw as u32), 4u64)
            };

            // T-Head vendor instructions other than the sync barrier still
            // execute in the interpreter.
            if matches!(insn, Instruction::Thead { .. }) {
                return None;
            }

            let is_block_end = matches!(
                insn,
                Instruction::Jal { .. }
                    | Instruction::Jalr { .. }
                    | Instruction::Branch { .. }
                    | Instruction::Ecall
                    | Instruction::Ebreak
                    | Instruction::Mret
                    | Instruction::Sret
                    | Instruction::Wfi
                    | Instruction::Csr { .. }
                    | Instruction::SfenceVma { .. }
                    | Instruction::Amo { .. }
                    | Instruction::FenceI
                    | Instruction::Unknown(_)
            );

            // Start a fresh block at every CSR instruction: it is executed by
            // the interpreter after the block exits, and counter reads after
            // a minstret write must not observe pre-CSR increments applied
            // late.
            if matches!(insn, Instruction::Csr { .. }) && !insns.is_empty() {
                break;
            }

            insns.push((insn, pc, size, raw as u32));
            pc += size;

            if is_block_end || insns.len() >= max_insns {
                break;
            }
        }

        if insns.is_empty() {
            return None;
        }

        let end_pc = pc;
        let insn_count = insns.len() as u64;

        self.tb_counter += 1;
        let func_name = format!("tb_{}", self.tb_counter);

        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(I64));
        sig.params.push(AbiParam::new(I64));

        let func_id = self
            .module
            .declare_function(&func_name, Linkage::Local, &sig)
            .unwrap();

        let mut func =
            Function::with_name_signature(UserFuncName::testcase(&func_name), sig.clone());

        let load_ref = self.module.declare_func_in_func(self.load_id, &mut func);
        let store_ref = self.module.declare_func_in_func(self.store_id, &mut func);
        let cbo_zero_ref = self
            .module
            .declare_func_in_func(self.cbo_zero_id, &mut func);
        let clmul_refs = self
            .clmul_ids
            .map(|id| self.module.declare_func_in_func(id, &mut func));
        let frontend_config = self.module.target_config();

        {
            let mut builder = FunctionBuilder::new(&mut func, &mut self.fb_ctx);

            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let state_ptr = builder.block_params(entry_block)[0];
            let mem_ctx = builder.block_params(entry_block)[1];

            let read_tlb_base = builder.ins().load(I64, memflags(), mem_ctx, 24);
            let write_tlb_base = builder.ins().load(I64, memflags(), mem_ctx, 32);
            let fregs_base = builder.ins().load(I64, memflags(), state_ptr, STATE_FREGS);

            let exit_block = builder.create_block();

            let emit = EmitCtx {
                state_ptr,
                mem_ctx,
                read_tlb_base,
                write_tlb_base,
                fregs_base,
                exit_block,
                load_ref,
                store_ref,
                cbo_zero_ref,
                clmul_refs,
            };

            for (idx, &(ref insn, insn_pc, insn_size, raw)) in insns.iter().enumerate() {
                match insn {
                    Instruction::Lui { rd, imm } => {
                        if *rd != 0 {
                            let val = builder.ins().iconst(I64, *imm);
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Auipc { rd, imm } => {
                        if *rd != 0 {
                            let result = insn_pc.wrapping_add(*imm as u64);
                            let val = builder.ins().iconst(I64, result as i64);
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::OpImm {
                        funct3,
                        rd,
                        rs1,
                        imm,
                        funct7,
                    } => {
                        if *rd != 0 {
                            let src = load_reg(&mut builder, state_ptr, *rs1);
                            let shamt = (*imm as u64) & 0x3F;
                            let val = match funct3 {
                                0 => {
                                    let iv = builder.ins().iconst(I64, *imm);
                                    builder.ins().iadd(src, iv)
                                }
                                1 => {
                                    let sh = builder.ins().iconst(I64, shamt as i64);
                                    builder.ins().ishl(src, sh)
                                }
                                2 => {
                                    let iv = builder.ins().iconst(I64, *imm);
                                    let cmp = builder.ins().icmp(IntCC::SignedLessThan, src, iv);
                                    builder.ins().uextend(I64, cmp)
                                }
                                3 => {
                                    let iv = builder.ins().iconst(I64, *imm);
                                    let cmp = builder.ins().icmp(IntCC::UnsignedLessThan, src, iv);
                                    builder.ins().uextend(I64, cmp)
                                }
                                4 => {
                                    let iv = builder.ins().iconst(I64, *imm);
                                    builder.ins().bxor(src, iv)
                                }
                                5 => {
                                    let sh = builder.ins().iconst(I64, shamt as i64);
                                    if funct7 & 0x20 != 0 {
                                        builder.ins().sshr(src, sh)
                                    } else {
                                        builder.ins().ushr(src, sh)
                                    }
                                }
                                6 => {
                                    let iv = builder.ins().iconst(I64, *imm);
                                    builder.ins().bor(src, iv)
                                }
                                7 => {
                                    let iv = builder.ins().iconst(I64, *imm);
                                    builder.ins().band(src, iv)
                                }
                                _ => src,
                            };
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Op {
                        funct3,
                        funct7,
                        rd,
                        rs1,
                        rs2,
                    } => {
                        if *rd != 0 {
                            let a = load_reg(&mut builder, state_ptr, *rs1);
                            let b = load_reg(&mut builder, state_ptr, *rs2);
                            let val = if *funct7 == 0x01 {
                                emit_m_op(&mut builder, *funct3, a, b)
                            } else {
                                match (funct3, funct7) {
                                    (0, 0x00) => builder.ins().iadd(a, b),
                                    (0, 0x20) => builder.ins().isub(a, b),
                                    (1, 0x00) => {
                                        let mask = builder.ins().iconst(I64, 0x3F);
                                        let sh = builder.ins().band(b, mask);
                                        builder.ins().ishl(a, sh)
                                    }
                                    (2, 0x00) => {
                                        let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
                                        builder.ins().uextend(I64, cmp)
                                    }
                                    (3, 0x00) => {
                                        let cmp = builder.ins().icmp(IntCC::UnsignedLessThan, a, b);
                                        builder.ins().uextend(I64, cmp)
                                    }
                                    (4, 0x00) => builder.ins().bxor(a, b),
                                    (5, 0x00) => {
                                        let mask = builder.ins().iconst(I64, 0x3F);
                                        let sh = builder.ins().band(b, mask);
                                        builder.ins().ushr(a, sh)
                                    }
                                    (5, 0x20) => {
                                        let mask = builder.ins().iconst(I64, 0x3F);
                                        let sh = builder.ins().band(b, mask);
                                        builder.ins().sshr(a, sh)
                                    }
                                    (6, 0x00) => builder.ins().bor(a, b),
                                    (7, 0x00) => builder.ins().band(a, b),
                                    _ => a,
                                }
                            };
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::OpImm32 {
                        funct3,
                        rd,
                        rs1,
                        imm,
                        funct7,
                    } => {
                        if *rd != 0 {
                            let src64 = load_reg(&mut builder, state_ptr, *rs1);
                            let src = builder.ins().ireduce(I32, src64);
                            let shamt = (*imm as u32) & 0x1F;
                            let val32 = match funct3 {
                                0 => {
                                    let iv = builder.ins().iconst(I64, *imm as i32 as i64);
                                    let iv32 = builder.ins().ireduce(I32, iv);
                                    builder.ins().iadd(src, iv32)
                                }
                                1 => {
                                    let sh = builder.ins().iconst(I64, shamt as i64);
                                    let sh32 = builder.ins().ireduce(I32, sh);
                                    builder.ins().ishl(src, sh32)
                                }
                                5 => {
                                    let sh = builder.ins().iconst(I64, shamt as i64);
                                    let sh32 = builder.ins().ireduce(I32, sh);
                                    if funct7 & 0x20 != 0 {
                                        builder.ins().sshr(src, sh32)
                                    } else {
                                        builder.ins().ushr(src, sh32)
                                    }
                                }
                                _ => src,
                            };
                            let val = builder.ins().sextend(I64, val32);
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Op32 {
                        funct3,
                        funct7,
                        rd,
                        rs1,
                        rs2,
                    } => {
                        if *rd != 0 {
                            let a64 = load_reg(&mut builder, state_ptr, *rs1);
                            let b64 = load_reg(&mut builder, state_ptr, *rs2);
                            let a = builder.ins().ireduce(I32, a64);
                            let b = builder.ins().ireduce(I32, b64);
                            let val32 = if *funct7 == 0x01 {
                                emit_m32_op(&mut builder, *funct3, a, b)
                            } else {
                                match (funct3, funct7) {
                                    (0, 0x00) => builder.ins().iadd(a, b),
                                    (0, 0x20) => builder.ins().isub(a, b),
                                    (1, 0x00) => {
                                        let mask = builder.ins().iconst(I64, 0x1F);
                                        let m32 = builder.ins().ireduce(I32, mask);
                                        let sh = builder.ins().band(b, m32);
                                        builder.ins().ishl(a, sh)
                                    }
                                    (5, 0x00) => {
                                        let mask = builder.ins().iconst(I64, 0x1F);
                                        let m32 = builder.ins().ireduce(I32, mask);
                                        let sh = builder.ins().band(b, m32);
                                        builder.ins().ushr(a, sh)
                                    }
                                    (5, 0x20) => {
                                        let mask = builder.ins().iconst(I64, 0x1F);
                                        let m32 = builder.ins().ireduce(I32, mask);
                                        let sh = builder.ins().band(b, m32);
                                        builder.ins().sshr(a, sh)
                                    }
                                    _ => a,
                                }
                            };
                            let val = builder.ins().sextend(I64, val32);
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Load {
                        funct3,
                        rd,
                        rs1,
                        offset,
                    } => {
                        let base = load_reg(&mut builder, state_ptr, *rs1);
                        let off_val = builder.ins().iconst(I64, *offset);
                        let va = builder.ins().iadd(base, off_val);
                        let size_val = match funct3 {
                            0 | 4 => 1i64,
                            1 | 5 => 2,
                            2 | 6 => 4,
                            3 => 8,
                            _ => 1,
                        };

                        let twelve = builder.ins().iconst(I64, 12);
                        let va_page = builder.ins().ushr(va, twelve);
                        let mask_ff = builder.ins().iconst(I64, 0xFF);
                        let tlb_idx = builder.ins().band(va_page, mask_ff);
                        let four = builder.ins().iconst(I64, 4);
                        let entry_offset = builder.ins().ishl(tlb_idx, four);
                        let entry_ptr = builder.ins().iadd(read_tlb_base, entry_offset);
                        let tag = builder.ins().load(I64, memflags(), entry_ptr, 0);
                        let hit = builder.ins().icmp(IntCC::Equal, tag, va_page);

                        let fast_block = builder.create_block();
                        let slow_block = builder.create_block();
                        let merge_block = builder.create_block();
                        builder.append_block_param(merge_block, I64);
                        builder.ins().brif(hit, fast_block, &[], slow_block, &[]);

                        builder.switch_to_block(fast_block);
                        builder.seal_block(fast_block);
                        let addend = builder.ins().load(I64, memflags(), entry_ptr, 8);
                        let host_addr = builder.ins().iadd(addend, va);
                        let raw_fast = match size_val {
                            2 => {
                                let v = builder.ins().load(I16, memflags(), host_addr, 0);
                                builder.ins().uextend(I64, v)
                            }
                            4 => {
                                let v = builder.ins().load(I32, memflags(), host_addr, 0);
                                builder.ins().uextend(I64, v)
                            }
                            8 => builder.ins().load(I64, memflags(), host_addr, 0),
                            _ => {
                                let v = builder.ins().load(I8, memflags(), host_addr, 0);
                                builder.ins().uextend(I64, v)
                            }
                        };
                        let arg_fast = BlockArg::Value(raw_fast);
                        builder.ins().jump(merge_block, &[arg_fast]);

                        builder.switch_to_block(slow_block);
                        builder.seal_block(slow_block);
                        let size_arg = builder.ins().iconst(I64, size_val);
                        let call = builder.ins().call(load_ref, &[mem_ctx, va, size_arg]);
                        let raw_slow = builder.inst_results(call)[0];
                        let fault_ptr = builder.ins().iadd_imm_s(mem_ctx, 40);
                        let fault = builder.ins().load(I64, memflags(), fault_ptr, 0);
                        let zero = builder.ins().iconst(I64, 0);
                        let faulted = builder.ins().icmp(IntCC::NotEqual, fault, zero);
                        let fault_block = builder.create_block();
                        let ok_block = builder.create_block();
                        builder.ins().brif(faulted, fault_block, &[], ok_block, &[]);
                        builder.switch_to_block(fault_block);
                        builder.seal_block(fault_block);
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_FAULT, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        builder.switch_to_block(ok_block);
                        builder.seal_block(ok_block);
                        let arg_slow = BlockArg::Value(raw_slow);
                        builder.ins().jump(merge_block, &[arg_slow]);

                        builder.switch_to_block(merge_block);
                        builder.seal_block(merge_block);
                        let raw_val = builder.block_params(merge_block)[0];

                        if *rd != 0 {
                            let val = match funct3 {
                                0 => {
                                    let v = builder.ins().ireduce(I8, raw_val);
                                    builder.ins().sextend(I64, v)
                                }
                                1 => {
                                    let v = builder.ins().ireduce(I16, raw_val);
                                    builder.ins().sextend(I64, v)
                                }
                                2 => {
                                    let v = builder.ins().ireduce(I32, raw_val);
                                    builder.ins().sextend(I64, v)
                                }
                                3 => raw_val,
                                4 => {
                                    let m = builder.ins().iconst(I64, 0xFF);
                                    builder.ins().band(raw_val, m)
                                }
                                5 => {
                                    let m = builder.ins().iconst(I64, 0xFFFF);
                                    builder.ins().band(raw_val, m)
                                }
                                6 => {
                                    let m = builder.ins().iconst(I64, 0xFFFF_FFFFi64);
                                    builder.ins().band(raw_val, m)
                                }
                                _ => raw_val,
                            };
                            builder
                                .ins()
                                .store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Store {
                        funct3,
                        rs1,
                        rs2,
                        offset,
                    } => {
                        let base = load_reg(&mut builder, state_ptr, *rs1);
                        let off_val = builder.ins().iconst(I64, *offset);
                        let va = builder.ins().iadd(base, off_val);
                        let val = load_reg(&mut builder, state_ptr, *rs2);
                        let size_val = match funct3 {
                            0 => 1i64,
                            1 => 2,
                            2 => 4,
                            3 => 8,
                            _ => 1,
                        };

                        let twelve = builder.ins().iconst(I64, 12);
                        let va_page = builder.ins().ushr(va, twelve);
                        let mask_ff = builder.ins().iconst(I64, 0xFF);
                        let tlb_idx = builder.ins().band(va_page, mask_ff);
                        let four = builder.ins().iconst(I64, 4);
                        let entry_offset = builder.ins().ishl(tlb_idx, four);
                        let entry_ptr = builder.ins().iadd(write_tlb_base, entry_offset);
                        let tag = builder.ins().load(I64, memflags(), entry_ptr, 0);
                        let hit = builder.ins().icmp(IntCC::Equal, tag, va_page);

                        let fast_block = builder.create_block();
                        let slow_block = builder.create_block();
                        let cont_block = builder.create_block();
                        builder.ins().brif(hit, fast_block, &[], slow_block, &[]);

                        builder.switch_to_block(fast_block);
                        builder.seal_block(fast_block);
                        let addend = builder.ins().load(I64, memflags(), entry_ptr, 8);
                        let host_addr = builder.ins().iadd(addend, va);
                        match funct3 {
                            1 => {
                                let v = builder.ins().ireduce(I16, val);
                                builder.ins().store(memflags(), v, host_addr, 0);
                            }
                            2 => {
                                let v = builder.ins().ireduce(I32, val);
                                builder.ins().store(memflags(), v, host_addr, 0);
                            }
                            3 => {
                                builder.ins().store(memflags(), val, host_addr, 0);
                            }
                            _ => {
                                let v = builder.ins().ireduce(I8, val);
                                builder.ins().store(memflags(), v, host_addr, 0);
                            }
                        };
                        builder.ins().jump(cont_block, &[]);

                        builder.switch_to_block(slow_block);
                        builder.seal_block(slow_block);
                        let size_arg = builder.ins().iconst(I64, size_val);
                        let _ = builder.ins().call(store_ref, &[mem_ctx, va, size_arg, val]);
                        let fault_ptr = builder.ins().iadd_imm_s(mem_ctx, 40);
                        let fault = builder.ins().load(I64, memflags(), fault_ptr, 0);
                        let zero = builder.ins().iconst(I64, 0);
                        let faulted = builder.ins().icmp(IntCC::NotEqual, fault, zero);
                        let fault_block = builder.create_block();
                        builder
                            .ins()
                            .brif(faulted, fault_block, &[], cont_block, &[]);
                        builder.switch_to_block(fault_block);
                        builder.seal_block(fault_block);
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_FAULT, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);

                        builder.switch_to_block(cont_block);
                        builder.seal_block(cont_block);
                    }

                    Instruction::Jal { rd, offset } => {
                        if *rd != 0 {
                            let link = builder.ins().iconst(I64, (insn_pc + insn_size) as i64);
                            builder
                                .ins()
                                .store(memflags(), link, state_ptr, (*rd as i32) * 8);
                        }
                        let target = insn_pc.wrapping_add(*offset as u64);
                        emit_exit_with_pc(&mut builder, state_ptr, target, EXIT_BLOCK_END, idx + 1);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }

                    Instruction::Jalr { rd, rs1, offset } => {
                        let base = load_reg(&mut builder, state_ptr, *rs1);
                        if *rd != 0 {
                            let link = builder.ins().iconst(I64, (insn_pc + insn_size) as i64);
                            builder
                                .ins()
                                .store(memflags(), link, state_ptr, (*rd as i32) * 8);
                        }
                        let off_val = builder.ins().iconst(I64, *offset);
                        let target = builder.ins().iadd(base, off_val);
                        let mask = builder.ins().iconst(I64, !1i64);
                        let target = builder.ins().band(target, mask);
                        // pc offset = 32*8 = 256
                        builder.ins().store(memflags(), target, state_ptr, 256);
                        let exit = builder.ins().iconst(I64, EXIT_BLOCK_END as i64);
                        builder.ins().store(memflags(), exit, state_ptr, 264);
                        let count = builder.ins().iconst(I64, (idx + 1) as i64);
                        builder.ins().store(memflags(), count, state_ptr, 288);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }

                    Instruction::Branch {
                        funct3,
                        rs1,
                        rs2,
                        offset,
                    } => {
                        let a = load_reg(&mut builder, state_ptr, *rs1);
                        let b = load_reg(&mut builder, state_ptr, *rs2);
                        let cc = match funct3 {
                            0 => IntCC::Equal,
                            1 => IntCC::NotEqual,
                            4 => IntCC::SignedLessThan,
                            5 => IntCC::SignedGreaterThanOrEqual,
                            6 => IntCC::UnsignedLessThan,
                            7 => IntCC::UnsignedGreaterThanOrEqual,
                            _ => IntCC::Equal,
                        };
                        let cond = builder.ins().icmp(cc, a, b);
                        let taken_block = builder.create_block();
                        let not_taken_block = builder.create_block();
                        builder
                            .ins()
                            .brif(cond, taken_block, &[], not_taken_block, &[]);

                        builder.switch_to_block(taken_block);
                        builder.seal_block(taken_block);
                        let target = insn_pc.wrapping_add(*offset as u64);
                        emit_exit_with_pc(&mut builder, state_ptr, target, EXIT_BLOCK_END, idx + 1);
                        builder.ins().jump(exit_block, &[]);

                        builder.switch_to_block(not_taken_block);
                        builder.seal_block(not_taken_block);
                        emit_exit_with_pc(
                            &mut builder,
                            state_ptr,
                            insn_pc + insn_size,
                            EXIT_BLOCK_END,
                            idx + 1,
                        );
                        builder.ins().jump(exit_block, &[]);

                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }

                    Instruction::Ecall => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_ECALL, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::Ebreak => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_EBREAK, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::Mret => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_MRET, idx, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::Sret => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_SRET, idx, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::Wfi => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_WFI, idx, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::SfenceVma { .. } => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_SFENCE, idx, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::Csr { .. } => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_CSR, idx, raw);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::Amo { .. } => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_AMO, idx, raw);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::FenceI => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_FENCEI, idx, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                    Instruction::FpLoad {
                        width,
                        rd,
                        rs1,
                        offset,
                    } => emit_fp_load(
                        &mut builder,
                        &emit,
                        *width,
                        *rd,
                        *rs1,
                        *offset,
                        insn_pc,
                        idx + 1,
                    ),

                    Instruction::FpStore {
                        width,
                        rs1,
                        rs2,
                        offset,
                    } => emit_fp_store(
                        &mut builder,
                        &emit,
                        *width,
                        *rs1,
                        *rs2,
                        *offset,
                        insn_pc,
                        idx + 1,
                    ),

                    Instruction::FpOp {
                        funct7,
                        funct3,
                        rd,
                        rs1,
                        rs2,
                    } => emit_fp_op(
                        &mut builder,
                        &emit,
                        *funct7,
                        *funct3,
                        *rd,
                        *rs1,
                        *rs2,
                        insn_pc,
                        idx + 1,
                    ),

                    Instruction::FpFma {
                        opcode,
                        funct7,
                        rd,
                        rs1,
                        rs2,
                        rs3,
                    } => emit_fp_fma(
                        &mut builder,
                        &emit,
                        *opcode,
                        *funct7,
                        *rd,
                        *rs1,
                        *rs2,
                        *rs3,
                        insn_pc,
                        idx + 1,
                    ),

                    Instruction::Bitmanip {
                        opcode,
                        funct7,
                        funct3,
                        rd,
                        rs1,
                        rs2,
                    } => emit_bitmanip(
                        &mut builder,
                        &emit,
                        *opcode,
                        *funct7,
                        *funct3,
                        *rd,
                        *rs1,
                        *rs2,
                        insn_pc,
                        idx + 1,
                    ),

                    Instruction::BitmanipImm {
                        opcode,
                        funct3,
                        rd,
                        rs1,
                        funct12,
                    } => emit_bitmanip_imm(
                        &mut builder,
                        &emit,
                        *opcode,
                        *funct3,
                        *rd,
                        *rs1,
                        *funct12,
                        insn_pc,
                        idx + 1,
                    ),

                    Instruction::Cbo { op, rs1 } => {
                        emit_cbo(&mut builder, &emit, *op, *rs1, insn_pc, idx + 1)
                    }

                    Instruction::TheadSync => { /* no architectural state */ }
                    Instruction::Thead { .. } => return None,
                    Instruction::Fence => { /* no-op */ }
                    Instruction::Unknown(_) => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_TRAP, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }
                }
            }

            // Fallthrough: set PC past end of block
            emit_exit_with_pc(
                &mut builder,
                state_ptr,
                end_pc,
                EXIT_BLOCK_END,
                insn_count as usize,
            );
            builder.ins().jump(exit_block, &[]);

            builder.switch_to_block(exit_block);
            builder.seal_block(exit_block);
            builder.ins().return_(&[]);

            builder.finalize(frontend_config);
        }

        let mut ctx = Context::for_function(func);
        self.module.define_function(func_id, &mut ctx).unwrap();
        self.module.finalize_definitions().unwrap();

        let code_ptr = self.module.get_finalized_function(func_id);

        Some(TranslationBlock {
            code: unsafe { std::mem::transmute::<*const u8, JitBlockFn>(code_ptr) },
            guest_pc: start_pc,
            guest_end_pc: end_pc,
            insn_count,
        })
    }
}

fn load_reg(
    builder: &mut FunctionBuilder,
    state_ptr: cranelift_codegen::ir::Value,
    reg: u8,
) -> cranelift_codegen::ir::Value {
    if reg == 0 {
        builder.ins().iconst(I64, 0)
    } else {
        builder
            .ins()
            .load(I64, memflags(), state_ptr, (reg as i32) * 8)
    }
}

fn emit_exit_with_pc(
    builder: &mut FunctionBuilder,
    state_ptr: cranelift_codegen::ir::Value,
    pc: u64,
    reason: u64,
    count: usize,
) {
    let pc_val = builder.ins().iconst(I64, pc as i64);
    builder.ins().store(memflags(), pc_val, state_ptr, 256); // offset 32*8
    let exit_val = builder.ins().iconst(I64, reason as i64);
    builder.ins().store(memflags(), exit_val, state_ptr, 264); // offset 33*8
    let cnt = builder.ins().iconst(I64, count as i64);
    builder.ins().store(memflags(), cnt, state_ptr, 288); // offset 36*8
}

fn emit_exit_insn(
    builder: &mut FunctionBuilder,
    state_ptr: cranelift_codegen::ir::Value,
    pc: u64,
    reason: u64,
    count: usize,
    raw_insn: u32,
) {
    let pc_val = builder.ins().iconst(I64, pc as i64);
    builder.ins().store(memflags(), pc_val, state_ptr, 256);
    let exit_val = builder.ins().iconst(I64, reason as i64);
    builder.ins().store(memflags(), exit_val, state_ptr, 264);
    // exit_pc at offset 34*8=272
    let exit_pc = builder.ins().iconst(I64, pc as i64);
    builder.ins().store(memflags(), exit_pc, state_ptr, 272);
    let cnt = builder.ins().iconst(I64, count as i64);
    builder.ins().store(memflags(), cnt, state_ptr, 288);
    if raw_insn != 0 {
        let raw_val = builder.ins().iconst(I64, raw_insn as i64);
        builder.ins().store(memflags(), raw_val, state_ptr, 296); // 37*8
    }
}

fn emit_m_op(
    builder: &mut FunctionBuilder,
    funct3: u8,
    a: cranelift_codegen::ir::Value,
    b: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    match funct3 {
        0 => builder.ins().imul(a, b),
        1 => builder.ins().smulhi(a, b),
        2 => {
            // MULHSU: signed(a) × unsigned(b), high 64
            let umulhi = builder.ins().umulhi(a, b);
            let s63 = builder.ins().iconst(I64, 63);
            let a_sign = builder.ins().sshr(a, s63);
            let correction = builder.ins().imul(a_sign, b);
            builder.ins().iadd(umulhi, correction)
        }
        3 => builder.ins().umulhi(a, b),
        4 => {
            // DIV signed: b==0 → -1, MIN/-1 → MIN. Host idiv faults on both,
            // so substitute a safe divisor and select the architectural result.
            let zero = builder.ins().iconst(I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
            let neg1 = builder.ins().iconst(I64, -1i64);
            let one = builder.ins().iconst(I64, 1);
            let min = builder.ins().iconst(I64, i64::MIN);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min);
            let b_neg1 = builder.ins().icmp(IntCC::Equal, b, neg1);
            let overflow = builder.ins().band(a_min, b_neg1);
            let safe_b = builder.ins().select(is_zero, one, b);
            let safe_b = builder.ins().select(overflow, one, safe_b);
            let result = builder.ins().sdiv(a, safe_b);
            let result = builder.ins().select(overflow, a, result);
            builder.ins().select(is_zero, neg1, result)
        }
        5 => {
            let zero = builder.ins().iconst(I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
            let neg1 = builder.ins().iconst(I64, -1i64);
            let one = builder.ins().iconst(I64, 1);
            let safe_b = builder.ins().select(is_zero, one, b);
            let result = builder.ins().udiv(a, safe_b);
            builder.ins().select(is_zero, neg1, result)
        }
        6 => {
            // REM signed: b==0 → a, MIN/-1 → 0.
            let zero = builder.ins().iconst(I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
            let neg1 = builder.ins().iconst(I64, -1i64);
            let one = builder.ins().iconst(I64, 1);
            let min = builder.ins().iconst(I64, i64::MIN);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min);
            let b_neg1 = builder.ins().icmp(IntCC::Equal, b, neg1);
            let overflow = builder.ins().band(a_min, b_neg1);
            let safe_b = builder.ins().select(is_zero, one, b);
            let safe_b = builder.ins().select(overflow, one, safe_b);
            let result = builder.ins().srem(a, safe_b);
            let result = builder.ins().select(overflow, zero, result);
            builder.ins().select(is_zero, a, result)
        }
        7 => {
            let zero = builder.ins().iconst(I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
            let one = builder.ins().iconst(I64, 1);
            let safe_b = builder.ins().select(is_zero, one, b);
            let result = builder.ins().urem(a, safe_b);
            builder.ins().select(is_zero, a, result)
        }
        _ => a,
    }
}

/// Values shared by the instruction emitters in this module.
#[derive(Clone, Copy)]
struct EmitCtx {
    state_ptr: Value,
    mem_ctx: Value,
    read_tlb_base: Value,
    write_tlb_base: Value,
    fregs_base: Value,
    exit_block: Block,
    load_ref: FuncRef,
    store_ref: FuncRef,
    cbo_zero_ref: FuncRef,
    clmul_refs: [FuncRef; 3],
}

fn store_rd(builder: &mut FunctionBuilder, state_ptr: Value, rd: u8, val: Value) {
    if rd != 0 {
        builder
            .ins()
            .store(memflags(), val, state_ptr, (rd as i32) * 8);
    }
}

fn load_fp_bits(builder: &mut FunctionBuilder, ctx: &EmitCtx, reg: u8) -> Value {
    builder
        .ins()
        .load(I64, memflags(), ctx.fregs_base, (reg as i32) * 8)
}

fn store_fp_bits(builder: &mut FunctionBuilder, ctx: &EmitCtx, reg: u8, val: Value) {
    builder
        .ins()
        .store(memflags(), val, ctx.fregs_base, (reg as i32) * 8);
}

/// Trap as an illegal instruction when `mstatus.FS` is Off, otherwise
/// continue in a fresh block.
fn emit_fp_guard(builder: &mut FunctionBuilder, ctx: &EmitCtx, insn_pc: u64, count: usize) {
    let fs = builder.ins().load(I64, memflags(), ctx.state_ptr, STATE_FS);
    let zero = builder.ins().iconst(I64, 0);
    let disabled = builder.ins().icmp(IntCC::Equal, fs, zero);
    let trap_block = builder.create_block();
    let cont_block = builder.create_block();
    builder
        .ins()
        .brif(disabled, trap_block, &[], cont_block, &[]);
    builder.switch_to_block(trap_block);
    builder.seal_block(trap_block);
    emit_exit_insn(builder, ctx.state_ptr, insn_pc, EXIT_TRAP, count, 0);
    builder.ins().jump(ctx.exit_block, &[]);
    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
}

fn mark_fp_dirty(builder: &mut FunctionBuilder, ctx: &EmitCtx) {
    let one = builder.ins().iconst(I64, 1);
    builder
        .ins()
        .store(memflags(), one, ctx.state_ptr, STATE_FP_DIRTY);
}

fn f32_of(builder: &mut FunctionBuilder, bits: Value) -> Value {
    let low = builder.ins().ireduce(I32, bits);
    builder.ins().bitcast(F32, memflags(), low)
}

fn f64_of(builder: &mut FunctionBuilder, bits: Value) -> Value {
    builder.ins().bitcast(F64, memflags(), bits)
}

fn bits_of_f32(builder: &mut FunctionBuilder, val: Value) -> Value {
    let bits = builder.ins().bitcast(I32, memflags(), val);
    let bits = builder.ins().uextend(I64, bits);
    box_f32(builder, bits)
}

fn bits_of_f64(builder: &mut FunctionBuilder, val: Value) -> Value {
    builder.ins().bitcast(I64, memflags(), val)
}

fn const_f32(builder: &mut FunctionBuilder, bits: u32) -> Value {
    builder.ins().f32const(Ieee32::with_bits(bits))
}

fn const_f64(builder: &mut FunctionBuilder, bits: u64) -> Value {
    builder.ins().f64const(Ieee64::with_bits(bits))
}

fn box_f32(builder: &mut FunctionBuilder, bits: Value) -> Value {
    let mask = builder.ins().iconst(I64, 0xFFFF_FFFF);
    let low = builder.ins().band(bits, mask);
    let upper = builder.ins().iconst(I64, 0xFFFF_FFFF_0000_0000u64 as i64);
    builder.ins().bor(low, upper)
}

/// OR `mask` into `fcsr` when `cond` is true.
fn or_fcsr_if(builder: &mut FunctionBuilder, ctx: &EmitCtx, cond: Value, mask: Value) {
    let zero = builder.ins().iconst(I64, 0);
    let selected = builder.ins().select(cond, mask, zero);
    let fcsr = builder
        .ins()
        .load(I64, memflags(), ctx.state_ptr, STATE_FCSR);
    let updated = builder.ins().bor(fcsr, selected);
    builder
        .ins()
        .store(memflags(), updated, ctx.state_ptr, STATE_FCSR);
}

fn call_clmul(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    which: usize,
    a: Value,
    b: Value,
) -> Value {
    let call = builder.ins().call(ctx.clmul_refs[which], &[a, b]);
    builder.inst_results(call)[0]
}

fn emit_trap(builder: &mut FunctionBuilder, ctx: &EmitCtx, insn_pc: u64, count: usize) {
    emit_exit_insn(builder, ctx.state_ptr, insn_pc, EXIT_TRAP, count, 0);
    builder.ins().jump(ctx.exit_block, &[]);
    let next = builder.create_block();
    builder.switch_to_block(next);
    builder.seal_block(next);
}

/// Emit an integer/fp-width guest load with the fast-TLB shortcut and a
/// helper fallback. Returns the zero-extended raw value.
fn emit_guest_load(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    va: Value,
    size_val: i64,
    insn_pc: u64,
    count: usize,
) -> Value {
    let twelve = builder.ins().iconst(I64, 12);
    let va_page = builder.ins().ushr(va, twelve);
    let mask_ff = builder.ins().iconst(I64, 0xFF);
    let tlb_idx = builder.ins().band(va_page, mask_ff);
    let four = builder.ins().iconst(I64, 4);
    let entry_offset = builder.ins().ishl(tlb_idx, four);
    let entry_ptr = builder.ins().iadd(ctx.read_tlb_base, entry_offset);
    let tag = builder.ins().load(I64, memflags(), entry_ptr, 0);
    let hit = builder.ins().icmp(IntCC::Equal, tag, va_page);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    let merge_block = builder.create_block();
    builder.append_block_param(merge_block, I64);
    builder.ins().brif(hit, fast_block, &[], slow_block, &[]);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addend = builder.ins().load(I64, memflags(), entry_ptr, 8);
    let host_addr = builder.ins().iadd(addend, va);
    let raw_fast = match size_val {
        2 => {
            let v = builder.ins().load(I16, memflags(), host_addr, 0);
            builder.ins().uextend(I64, v)
        }
        4 => {
            let v = builder.ins().load(I32, memflags(), host_addr, 0);
            builder.ins().uextend(I64, v)
        }
        8 => builder.ins().load(I64, memflags(), host_addr, 0),
        _ => {
            let v = builder.ins().load(I8, memflags(), host_addr, 0);
            builder.ins().uextend(I64, v)
        }
    };
    builder
        .ins()
        .jump(merge_block, &[BlockArg::Value(raw_fast)]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let size_arg = builder.ins().iconst(I64, size_val);
    let call = builder
        .ins()
        .call(ctx.load_ref, &[ctx.mem_ctx, va, size_arg]);
    let raw_slow = builder.inst_results(call)[0];
    let fault_ptr = builder.ins().iadd_imm_s(ctx.mem_ctx, 40);
    let fault = builder.ins().load(I64, memflags(), fault_ptr, 0);
    let zero = builder.ins().iconst(I64, 0);
    let faulted = builder.ins().icmp(IntCC::NotEqual, fault, zero);
    let fault_block = builder.create_block();
    let ok_block = builder.create_block();
    builder.ins().brif(faulted, fault_block, &[], ok_block, &[]);
    builder.switch_to_block(fault_block);
    builder.seal_block(fault_block);
    emit_exit_insn(builder, ctx.state_ptr, insn_pc, EXIT_FAULT, count, 0);
    builder.ins().jump(ctx.exit_block, &[]);
    builder.switch_to_block(ok_block);
    builder.seal_block(ok_block);
    builder
        .ins()
        .jump(merge_block, &[BlockArg::Value(raw_slow)]);

    builder.switch_to_block(merge_block);
    builder.seal_block(merge_block);
    builder.block_params(merge_block)[0]
}

/// Emit a guest store with the fast-TLB shortcut and a helper fallback.
fn emit_guest_store(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    va: Value,
    size_val: i64,
    val: Value,
    insn_pc: u64,
    count: usize,
) {
    let twelve = builder.ins().iconst(I64, 12);
    let va_page = builder.ins().ushr(va, twelve);
    let mask_ff = builder.ins().iconst(I64, 0xFF);
    let tlb_idx = builder.ins().band(va_page, mask_ff);
    let four = builder.ins().iconst(I64, 4);
    let entry_offset = builder.ins().ishl(tlb_idx, four);
    let entry_ptr = builder.ins().iadd(ctx.write_tlb_base, entry_offset);
    let tag = builder.ins().load(I64, memflags(), entry_ptr, 0);
    let hit = builder.ins().icmp(IntCC::Equal, tag, va_page);

    let fast_block = builder.create_block();
    let slow_block = builder.create_block();
    let cont_block = builder.create_block();
    builder.ins().brif(hit, fast_block, &[], slow_block, &[]);

    builder.switch_to_block(fast_block);
    builder.seal_block(fast_block);
    let addend = builder.ins().load(I64, memflags(), entry_ptr, 8);
    let host_addr = builder.ins().iadd(addend, va);
    match size_val {
        2 => {
            let v = builder.ins().ireduce(I16, val);
            builder.ins().store(memflags(), v, host_addr, 0);
        }
        4 => {
            let v = builder.ins().ireduce(I32, val);
            builder.ins().store(memflags(), v, host_addr, 0);
        }
        8 => {
            builder.ins().store(memflags(), val, host_addr, 0);
        }
        _ => {
            let v = builder.ins().ireduce(I8, val);
            builder.ins().store(memflags(), v, host_addr, 0);
        }
    };
    builder.ins().jump(cont_block, &[]);

    builder.switch_to_block(slow_block);
    builder.seal_block(slow_block);
    let size_arg = builder.ins().iconst(I64, size_val);
    let _ = builder
        .ins()
        .call(ctx.store_ref, &[ctx.mem_ctx, va, size_arg, val]);
    let fault_ptr = builder.ins().iadd_imm_s(ctx.mem_ctx, 40);
    let fault = builder.ins().load(I64, memflags(), fault_ptr, 0);
    let zero = builder.ins().iconst(I64, 0);
    let faulted = builder.ins().icmp(IntCC::NotEqual, fault, zero);
    let fault_block = builder.create_block();
    builder
        .ins()
        .brif(faulted, fault_block, &[], cont_block, &[]);
    builder.switch_to_block(fault_block);
    builder.seal_block(fault_block);
    emit_exit_insn(builder, ctx.state_ptr, insn_pc, EXIT_FAULT, count, 0);
    builder.ins().jump(ctx.exit_block, &[]);

    builder.switch_to_block(cont_block);
    builder.seal_block(cont_block);
}

#[allow(clippy::too_many_arguments)]
fn emit_fp_load(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    width: u8,
    rd: u8,
    rs1: u8,
    offset: i64,
    insn_pc: u64,
    count: usize,
) {
    emit_fp_guard(builder, ctx, insn_pc, count);
    let base = load_reg(builder, ctx.state_ptr, rs1);
    let off = builder.ins().iconst(I64, offset);
    let va = builder.ins().iadd(base, off);
    let raw = emit_guest_load(builder, ctx, va, width as i64, insn_pc, count);
    let bits = if width == 4 {
        box_f32(builder, raw)
    } else {
        raw
    };
    store_fp_bits(builder, ctx, rd, bits);
    mark_fp_dirty(builder, ctx);
}

#[allow(clippy::too_many_arguments)]
fn emit_fp_store(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    width: u8,
    rs1: u8,
    rs2: u8,
    offset: i64,
    insn_pc: u64,
    count: usize,
) {
    emit_fp_guard(builder, ctx, insn_pc, count);
    let base = load_reg(builder, ctx.state_ptr, rs1);
    let off = builder.ins().iconst(I64, offset);
    let va = builder.ins().iadd(base, off);
    let val = load_fp_bits(builder, ctx, rs2);
    emit_guest_store(builder, ctx, va, width as i64, val, insn_pc, count);
    mark_fp_dirty(builder, ctx);
}

/// RISC-V minimum/maximum: return the non-NaN operand, canonical NaN when
/// both are NaN, and the sign-mandated zero when both are zero.
fn emit_fp_minmax(
    builder: &mut FunctionBuilder,
    a: Value,
    b: Value,
    is_double: bool,
    take_max: bool,
) -> Value {
    let nan = if is_double {
        const_f64(builder, 0x7FF8_0000_0000_0000)
    } else {
        const_f32(builder, 0x7FC0_0000)
    };
    let zero = if is_double {
        let bits = if take_max { 0 } else { 1u64 << 63 };
        const_f64(builder, bits)
    } else {
        let bits = if take_max { 0 } else { 1u32 << 31 };
        const_f32(builder, bits)
    };
    let a_nan = builder.ins().fcmp(FloatCC::Unordered, a, a);
    let b_nan = builder.ins().fcmp(FloatCC::Unordered, b, b);
    let both_nan = builder.ins().band(a_nan, b_nan);
    let a_zero = builder.ins().fcmp(FloatCC::Equal, a, zero);
    let b_zero = builder.ins().fcmp(FloatCC::Equal, b, zero);
    let both_zero = builder.ins().band(a_zero, b_zero);
    let base = if take_max {
        builder.ins().fmax(a, b)
    } else {
        builder.ins().fmin(a, b)
    };
    let base = builder.ins().select(both_zero, zero, base);
    let only_a_nan = builder.ins().select(a_nan, b, base);
    let either_nan = builder.ins().select(b_nan, a, only_a_nan);
    builder.ins().select(both_nan, nan, either_nan)
}

fn emit_fp_class(builder: &mut FunctionBuilder, bits: Value, is_double: bool) -> Value {
    let (sign_shift, exp_shift, exp_mask, frac_mask, qnan_bit) = if is_double {
        (63u64, 52u64, 0x7FFu64, 0x000F_FFFF_FFFF_FFFFu64, 1u64 << 51)
    } else {
        (31, 23, 0xFF, 0x007F_FFFF, 1u64 << 22)
    };
    let bits = if is_double {
        bits
    } else {
        let mask = builder.ins().iconst(I64, 0xFFFF_FFFF);
        builder.ins().band(bits, mask)
    };
    let sign_shift_v = builder.ins().iconst(I64, sign_shift as i64);
    let sign = builder.ins().ushr(bits, sign_shift_v);
    let one = builder.ins().iconst(I64, 1);
    let sign = builder.ins().band(sign, one);
    let exp_shift_v = builder.ins().iconst(I64, exp_shift as i64);
    let exp = builder.ins().ushr(bits, exp_shift_v);
    let exp_mask_v = builder.ins().iconst(I64, exp_mask as i64);
    let exp = builder.ins().band(exp, exp_mask_v);
    let frac_mask_v = builder.ins().iconst(I64, frac_mask as i64);
    let frac = builder.ins().band(bits, frac_mask_v);
    let zero = builder.ins().iconst(I64, 0);

    let exp_all = builder.ins().icmp(IntCC::Equal, exp, exp_mask_v);
    let exp_zero = builder.ins().icmp(IntCC::Equal, exp, zero);
    let frac_zero = builder.ins().icmp(IntCC::Equal, frac, zero);
    let frac_nonzero = builder.ins().icmp(IntCC::NotEqual, frac, zero);
    let is_neg = builder.ins().icmp(IntCC::NotEqual, sign, zero);

    let is_nan = builder.ins().band(exp_all, frac_nonzero);
    let is_inf = builder.ins().band(exp_all, frac_zero);
    let is_zero = builder.ins().band(exp_zero, frac_zero);
    let is_subnormal = builder.ins().band(exp_zero, frac_nonzero);
    let qnan_bit_v = builder.ins().iconst(I64, qnan_bit as i64);
    let quiet = builder.ins().band(frac, qnan_bit_v);
    let is_quiet = builder.ins().icmp(IntCC::NotEqual, quiet, zero);
    let is_quiet = builder.ins().band(is_nan, is_quiet);

    let val_norm = if_true_else(builder, is_neg, 1 << 1, 1 << 6);
    let val_sub = if_true_else(builder, is_neg, 1 << 2, 1 << 5);
    let val_zero = if_true_else(builder, is_neg, 1 << 3, 1 << 4);
    let val_inf = if_true_else(builder, is_neg, 1 << 0, 1 << 7);
    let val_nan = if_true_else(builder, is_quiet, 1 << 9, 1 << 8);

    let mut result = builder.ins().select(is_subnormal, val_sub, val_norm);
    result = builder.ins().select(is_zero, val_zero, result);
    result = builder.ins().select(is_inf, val_inf, result);
    builder.ins().select(is_nan, val_nan, result)
}

fn if_true_else(builder: &mut FunctionBuilder, cond: Value, yes: u64, no: u64) -> Value {
    let yes_v = builder.ins().iconst(I64, yes as i64);
    let no_v = builder.ins().iconst(I64, no as i64);
    builder.ins().select(cond, yes_v, no_v)
}

#[allow(clippy::too_many_arguments)]
fn emit_fp_op(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    funct7: u8,
    funct3: u8,
    rd: u8,
    rs1: u8,
    rs2: u8,
    insn_pc: u64,
    count: usize,
) {
    emit_fp_guard(builder, ctx, insn_pc, count);
    let is_double = funct7 & 1 != 0;
    let a_bits = load_fp_bits(builder, ctx, rs1);
    let b_bits = load_fp_bits(builder, ctx, rs2);
    let mut fp_result: Option<Value> = None;
    let mut int_result: Option<Value> = None;
    let mut trap = false;

    match funct7 {
        0x00 | 0x01 => {
            let v = if is_double {
                let a = f64_of(builder, a_bits);
                let b = f64_of(builder, b_bits);
                let r = builder.ins().fadd(a, b);
                bits_of_f64(builder, r)
            } else {
                let a = f32_of(builder, a_bits);
                let b = f32_of(builder, b_bits);
                let r = builder.ins().fadd(a, b);
                bits_of_f32(builder, r)
            };
            fp_result = Some(v);
        }
        0x04 | 0x05 => {
            let v = if is_double {
                let a = f64_of(builder, a_bits);
                let b = f64_of(builder, b_bits);
                let r = builder.ins().fsub(a, b);
                bits_of_f64(builder, r)
            } else {
                let a = f32_of(builder, a_bits);
                let b = f32_of(builder, b_bits);
                let r = builder.ins().fsub(a, b);
                bits_of_f32(builder, r)
            };
            fp_result = Some(v);
        }
        0x08 | 0x09 => {
            let v = if is_double {
                let a = f64_of(builder, a_bits);
                let b = f64_of(builder, b_bits);
                let r = builder.ins().fmul(a, b);
                bits_of_f64(builder, r)
            } else {
                let a = f32_of(builder, a_bits);
                let b = f32_of(builder, b_bits);
                let r = builder.ins().fmul(a, b);
                bits_of_f32(builder, r)
            };
            fp_result = Some(v);
        }
        0x0C | 0x0D => {
            let (a, b) = if is_double {
                (f64_of(builder, a_bits), f64_of(builder, b_bits))
            } else {
                (f32_of(builder, a_bits), f32_of(builder, b_bits))
            };
            let zero = if is_double {
                const_f64(builder, 0)
            } else {
                const_f32(builder, 0)
            };
            let b_zero = builder.ins().fcmp(FloatCC::Equal, b, zero);
            let a_zero = builder.ins().fcmp(FloatCC::Equal, a, zero);
            let invalid = builder.ins().iconst(I64, 1 << 4);
            let div_zero = builder.ins().iconst(I64, 1 << 3);
            let flag = builder.ins().select(a_zero, invalid, div_zero);
            or_fcsr_if(builder, ctx, b_zero, flag);
            let v = builder.ins().fdiv(a, b);
            fp_result = Some(if is_double {
                bits_of_f64(builder, v)
            } else {
                bits_of_f32(builder, v)
            });
        }
        0x2C | 0x2D => {
            let (a, zero) = if is_double {
                (f64_of(builder, a_bits), const_f64(builder, 0))
            } else {
                (f32_of(builder, a_bits), const_f32(builder, 0))
            };
            let negative = builder.ins().fcmp(FloatCC::LessThan, a, zero);
            let invalid = builder.ins().iconst(I64, 1 << 4);
            or_fcsr_if(builder, ctx, negative, invalid);
            let v = builder.ins().sqrt(a);
            fp_result = Some(if is_double {
                bits_of_f64(builder, v)
            } else {
                bits_of_f32(builder, v)
            });
        }
        0x10 | 0x11 => {
            let (a, b, sign_mask) = if is_double {
                (a_bits, b_bits, 1u64 << 63)
            } else {
                let mask = builder.ins().iconst(I64, 0xFFFF_FFFF);
                (
                    builder.ins().band(a_bits, mask),
                    builder.ins().band(b_bits, mask),
                    1u64 << 31,
                )
            };
            let sign_mask_v = builder.ins().iconst(I64, sign_mask as i64);
            let sign = match funct3 {
                0 => builder.ins().band(b, sign_mask_v),
                1 => {
                    let not_b = builder.ins().bnot(b);
                    builder.ins().band(not_b, sign_mask_v)
                }
                _ => {
                    let x = builder.ins().bxor(a, b);
                    builder.ins().band(x, sign_mask_v)
                }
            };
            let not_mask = builder.ins().bnot(sign_mask_v);
            let keep = builder.ins().band(a, not_mask);
            let out = builder.ins().bor(keep, sign);
            fp_result = Some(if is_double {
                out
            } else {
                box_f32(builder, out)
            });
        }
        0x14 | 0x15 => {
            let take_max = funct3 != 0;
            let v = if is_double {
                let a = f64_of(builder, a_bits);
                let b = f64_of(builder, b_bits);
                let r = emit_fp_minmax(builder, a, b, true, take_max);
                bits_of_f64(builder, r)
            } else {
                let a = f32_of(builder, a_bits);
                let b = f32_of(builder, b_bits);
                let r = emit_fp_minmax(builder, a, b, false, take_max);
                bits_of_f32(builder, r)
            };
            fp_result = Some(v);
        }
        0x20 => {
            let a = f64_of(builder, a_bits);
            let v = builder.ins().fdemote(F32, a);
            fp_result = Some(bits_of_f32(builder, v));
        }
        0x21 => {
            let a = f32_of(builder, a_bits);
            let v = builder.ins().fpromote(F64, a);
            fp_result = Some(bits_of_f64(builder, v));
        }
        0x50 | 0x51 => {
            let cc = match funct3 {
                2 => FloatCC::Equal,
                1 => FloatCC::LessThan,
                _ => FloatCC::LessThanOrEqual,
            };
            let cmp = if is_double {
                let a = f64_of(builder, a_bits);
                let b = f64_of(builder, b_bits);
                builder.ins().fcmp(cc, a, b)
            } else {
                let a = f32_of(builder, a_bits);
                let b = f32_of(builder, b_bits);
                builder.ins().fcmp(cc, a, b)
            };
            int_result = Some(builder.ins().uextend(I64, cmp));
        }
        0x60 | 0x61 => {
            let value = if is_double {
                f64_of(builder, a_bits)
            } else {
                let a = f32_of(builder, a_bits);
                builder.ins().fpromote(F64, a)
            };
            let nan = builder.ins().fcmp(FloatCC::Unordered, value, value);
            let invalid = builder.ins().iconst(I64, 1 << 4);
            or_fcsr_if(builder, ctx, nan, invalid);
            let out = match rs2 {
                0 => {
                    let v = builder.ins().fcvt_to_sint_sat(I32, value);
                    builder.ins().sextend(I64, v)
                }
                1 => {
                    let v = builder.ins().fcvt_to_uint_sat(I32, value);
                    builder.ins().uextend(I64, v)
                }
                2 => builder.ins().fcvt_to_sint_sat(I64, value),
                _ => builder.ins().fcvt_to_uint_sat(I64, value),
            };
            int_result = Some(out);
        }
        0x68 | 0x69 => {
            let x = load_reg(builder, ctx.state_ptr, rs1);
            let (value64, unsigned) = match rs2 {
                0 => {
                    let v = builder.ins().ireduce(I32, x);
                    (builder.ins().sextend(I64, v), false)
                }
                1 => {
                    let v = builder.ins().ireduce(I32, x);
                    (builder.ins().uextend(I64, v), true)
                }
                2 => (x, false),
                _ => (x, true),
            };
            let value = if unsigned {
                builder.ins().fcvt_from_uint(F64, value64)
            } else {
                builder.ins().fcvt_from_sint(F64, value64)
            };
            let bits = if is_double {
                bits_of_f64(builder, value)
            } else {
                let v = builder.ins().fdemote(F32, value);
                bits_of_f32(builder, v)
            };
            fp_result = Some(bits);
        }
        0x70 => {
            if funct3 == 1 {
                int_result = Some(emit_fp_class(builder, a_bits, false));
            } else {
                let v = builder.ins().ireduce(I32, a_bits);
                int_result = Some(builder.ins().sextend(I64, v));
            }
        }
        0x71 => {
            if funct3 == 1 {
                int_result = Some(emit_fp_class(builder, a_bits, true));
            } else {
                int_result = Some(a_bits);
            }
        }
        0x78 => {
            let x = load_reg(builder, ctx.state_ptr, rs1);
            let v = builder.ins().ireduce(I32, x);
            let v = builder.ins().uextend(I64, v);
            fp_result = Some(box_f32(builder, v));
        }
        0x79 => {
            let x = load_reg(builder, ctx.state_ptr, rs1);
            fp_result = Some(x);
        }
        _ => trap = true,
    }

    if trap {
        emit_trap(builder, ctx, insn_pc, count);
        return;
    }
    if let Some(v) = int_result {
        store_rd(builder, ctx.state_ptr, rd, v);
    }
    if let Some(v) = fp_result {
        store_fp_bits(builder, ctx, rd, v);
    }
    mark_fp_dirty(builder, ctx);
}

#[allow(clippy::too_many_arguments)]
fn emit_fp_fma(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    opcode: u8,
    funct7: u8,
    rd: u8,
    rs1: u8,
    rs2: u8,
    rs3: u8,
    insn_pc: u64,
    count: usize,
) {
    emit_fp_guard(builder, ctx, insn_pc, count);
    let is_double = funct7 & 1 != 0;
    let a_bits = load_fp_bits(builder, ctx, rs1);
    let b_bits = load_fp_bits(builder, ctx, rs2);
    let c_bits = load_fp_bits(builder, ctx, rs3);
    let bits = if is_double {
        let a = f64_of(builder, a_bits);
        let b = f64_of(builder, b_bits);
        let c = f64_of(builder, c_bits);
        let neg_a = builder.ins().fneg(a);
        let neg_c = builder.ins().fneg(c);
        let v = match opcode {
            0x43 => builder.ins().fma(a, b, c),
            0x47 => builder.ins().fma(a, b, neg_c),
            0x4B => builder.ins().fma(neg_a, b, c),
            _ => builder.ins().fma(neg_a, b, neg_c),
        };
        bits_of_f64(builder, v)
    } else {
        let a = f32_of(builder, a_bits);
        let b = f32_of(builder, b_bits);
        let c = f32_of(builder, c_bits);
        let neg_a = builder.ins().fneg(a);
        let neg_c = builder.ins().fneg(c);
        let v = match opcode {
            0x43 => builder.ins().fma(a, b, c),
            0x47 => builder.ins().fma(a, b, neg_c),
            0x4B => builder.ins().fma(neg_a, b, c),
            _ => builder.ins().fma(neg_a, b, neg_c),
        };
        bits_of_f32(builder, v)
    };
    store_fp_bits(builder, ctx, rd, bits);
    mark_fp_dirty(builder, ctx);
}

fn emit_cbo(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    op: u8,
    rs1: u8,
    insn_pc: u64,
    count: usize,
) {
    match op {
        0..=2 => {}
        4 => {
            let base = load_reg(builder, ctx.state_ptr, rs1);
            let call = builder.ins().call(ctx.cbo_zero_ref, &[ctx.mem_ctx, base]);
            let status = builder.inst_results(call)[0];
            let zero = builder.ins().iconst(I64, 0);
            let faulted = builder.ins().icmp(IntCC::NotEqual, status, zero);
            let fault_block = builder.create_block();
            let cont_block = builder.create_block();
            builder
                .ins()
                .brif(faulted, fault_block, &[], cont_block, &[]);
            builder.switch_to_block(fault_block);
            builder.seal_block(fault_block);
            emit_exit_insn(builder, ctx.state_ptr, insn_pc, EXIT_FAULT, count, 0);
            builder.ins().jump(ctx.exit_block, &[]);
            builder.switch_to_block(cont_block);
            builder.seal_block(cont_block);
        }
        _ => emit_trap(builder, ctx, insn_pc, count),
    }
}

fn orc_b(builder: &mut FunctionBuilder, a: Value) -> Value {
    let mask = builder.ins().iconst(I64, 0x7F7F_7F7F_7F7F_7F7F);
    let high = builder.ins().iconst(I64, 0x8080_8080_8080_8080u64 as i64);
    let seven = builder.ins().iconst(I64, 7);
    let mult = builder.ins().iconst(I64, 0xFF);
    let low = builder.ins().band(a, mask);
    let sum = builder.ins().iadd(low, mask);
    let t = builder.ins().bor(sum, a);
    let bit7 = builder.ins().band(t, high);
    let ones = builder.ins().ushr(bit7, seven);
    builder.ins().imul(ones, mult)
}

#[allow(clippy::too_many_arguments)]
fn emit_bitmanip(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    opcode: u8,
    funct7: u8,
    funct3: u8,
    rd: u8,
    rs1: u8,
    rs2: u8,
    insn_pc: u64,
    count: usize,
) {
    let a = load_reg(builder, ctx.state_ptr, rs1);
    let b = load_reg(builder, ctx.state_ptr, rs2);
    let shift_mask = builder.ins().iconst(I64, 0x3F);
    let one = builder.ins().iconst(I64, 1);
    let val = match (opcode, funct7, funct3) {
        (0x33, 0x10, 2) => {
            let sh = builder.ins().ishl(a, one);
            Some(builder.ins().iadd(sh, b))
        }
        (0x33, 0x10, 4) => {
            let amt = builder.ins().iconst(I64, 2);
            let sh = builder.ins().ishl(a, amt);
            Some(builder.ins().iadd(sh, b))
        }
        (0x33, 0x10, 6) => {
            let amt = builder.ins().iconst(I64, 3);
            let sh = builder.ins().ishl(a, amt);
            Some(builder.ins().iadd(sh, b))
        }
        (0x33, 0x20, 4) => {
            let x = builder.ins().bxor(a, b);
            Some(builder.ins().bnot(x))
        }
        (0x33, 0x20, 6) => {
            let not_b = builder.ins().bnot(b);
            Some(builder.ins().bor(a, not_b))
        }
        (0x33, 0x20, 7) => {
            let not_b = builder.ins().bnot(b);
            Some(builder.ins().band(a, not_b))
        }
        (0x33, 0x05, 1) => Some(call_clmul(builder, ctx, 0, a, b)),
        (0x33, 0x05, 2) => Some(call_clmul(builder, ctx, 2, a, b)),
        (0x33, 0x05, 3) => Some(call_clmul(builder, ctx, 1, a, b)),
        (0x33, 0x05, 4) => Some(builder.ins().smin(a, b)),
        (0x33, 0x05, 5) => Some(builder.ins().umin(a, b)),
        (0x33, 0x05, 6) => Some(builder.ins().smax(a, b)),
        (0x33, 0x05, 7) => Some(builder.ins().umax(a, b)),
        (0x33, 0x14, 1) => {
            let sh = builder.ins().band(b, shift_mask);
            let bit = builder.ins().ishl(one, sh);
            Some(builder.ins().bor(a, bit))
        }
        (0x33, 0x24, 1) => {
            let sh = builder.ins().band(b, shift_mask);
            let bit = builder.ins().ishl(one, sh);
            let not_bit = builder.ins().bnot(bit);
            Some(builder.ins().band(a, not_bit))
        }
        (0x33, 0x24, 5) => {
            let sh = builder.ins().band(b, shift_mask);
            let shifted = builder.ins().ushr(a, sh);
            Some(builder.ins().band(shifted, one))
        }
        (0x33, 0x34, 1) => {
            let sh = builder.ins().band(b, shift_mask);
            let bit = builder.ins().ishl(one, sh);
            Some(builder.ins().bxor(a, bit))
        }
        (0x33, 0x30, 1) => {
            let sh = builder.ins().band(b, shift_mask);
            Some(builder.ins().rotl(a, sh))
        }
        (0x33, 0x30, 5) => {
            let sh = builder.ins().band(b, shift_mask);
            Some(builder.ins().rotr(a, sh))
        }
        (0x3B, 0x04, 0) => {
            let az = builder.ins().ireduce(I32, a);
            let az = builder.ins().uextend(I64, az);
            Some(builder.ins().iadd(az, b))
        }
        (0x3B, 0x04, 4) => {
            let mask = builder.ins().iconst(I64, 0xFFFF);
            Some(builder.ins().band(a, mask))
        }
        (0x3B, 0x10, 2) | (0x3B, 0x10, 4) | (0x3B, 0x10, 6) => {
            let shift = match funct3 {
                2 => 1,
                4 => 2,
                _ => 3,
            };
            let az = builder.ins().ireduce(I32, a);
            let az = builder.ins().uextend(I64, az);
            let amt = builder.ins().iconst(I64, shift);
            let sh = builder.ins().ishl(az, amt);
            Some(builder.ins().iadd(sh, b))
        }
        (0x3B, 0x30, 1) | (0x3B, 0x30, 5) => {
            let a32 = builder.ins().ireduce(I32, a);
            let b32 = builder.ins().ireduce(I32, b);
            let mask = builder.ins().iconst(I32, 0x1F);
            let sh = builder.ins().band(b32, mask);
            let v = if funct3 == 1 {
                builder.ins().rotl(a32, sh)
            } else {
                builder.ins().rotr(a32, sh)
            };
            Some(builder.ins().sextend(I64, v))
        }
        _ => None,
    };
    match val {
        Some(v) => store_rd(builder, ctx.state_ptr, rd, v),
        None => emit_trap(builder, ctx, insn_pc, count),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_bitmanip_imm(
    builder: &mut FunctionBuilder,
    ctx: &EmitCtx,
    opcode: u8,
    funct3: u8,
    rd: u8,
    rs1: u8,
    funct12: u16,
    insn_pc: u64,
    count: usize,
) {
    let a = load_reg(builder, ctx.state_ptr, rs1);
    let f = funct12;
    let one = builder.ins().iconst(I64, 1);
    let val = match (opcode, funct3, f) {
        (0x13, 5, 0x6B8) => Some(builder.ins().bswap(a)),
        (0x13, 5, 0x287) => Some(orc_b(builder, a)),
        (0x13, 5, f) if (0x600..0x640).contains(&f) => {
            let sh = builder.ins().iconst(I64, (f & 0x3F) as i64);
            Some(builder.ins().rotr(a, sh))
        }
        (0x13, 5, f) if (0x480..0x4C0).contains(&f) => {
            let sh = builder.ins().iconst(I64, (f & 0x3F) as i64);
            let shifted = builder.ins().ushr(a, sh);
            Some(builder.ins().band(shifted, one))
        }
        (0x1B, 1, 0x600) => {
            let v = builder.ins().ireduce(I32, a);
            let v = builder.ins().clz(v);
            Some(builder.ins().uextend(I64, v))
        }
        (0x1B, 1, 0x601) => {
            let v = builder.ins().ireduce(I32, a);
            let v = builder.ins().ctz(v);
            Some(builder.ins().uextend(I64, v))
        }
        (0x1B, 1, 0x602) => {
            let v = builder.ins().ireduce(I32, a);
            let v = builder.ins().popcnt(v);
            Some(builder.ins().uextend(I64, v))
        }
        (0x1B, 1, f) if f >> 5 == 0x04 || f >> 5 == 0x05 => {
            let az = builder.ins().ireduce(I32, a);
            let az = builder.ins().uextend(I64, az);
            let sh = builder.ins().iconst(I64, (f & 0x3F) as i64);
            Some(builder.ins().ishl(az, sh))
        }
        (0x1B, 5, f) if (0x600..0x640).contains(&f) => {
            let a32 = builder.ins().ireduce(I32, a);
            let sh = builder.ins().iconst(I32, (f & 0x1F) as i64);
            let v = builder.ins().rotr(a32, sh);
            Some(builder.ins().sextend(I64, v))
        }
        (0x13, 1, 0x600) => Some(builder.ins().clz(a)),
        (0x13, 1, 0x601) => Some(builder.ins().ctz(a)),
        (0x13, 1, 0x602) => Some(builder.ins().popcnt(a)),
        (0x13, 1, 0x604) => {
            let v = builder.ins().ireduce(I8, a);
            Some(builder.ins().sextend(I64, v))
        }
        (0x13, 1, 0x605) => {
            let v = builder.ins().ireduce(I16, a);
            Some(builder.ins().sextend(I64, v))
        }
        (0x13, 1, f) if (0x280..0x2C0).contains(&f) => {
            let sh = builder.ins().iconst(I64, (f & 0x3F) as i64);
            let bit = builder.ins().ishl(one, sh);
            Some(builder.ins().bor(a, bit))
        }
        (0x13, 1, f) if (0x480..0x4C0).contains(&f) => {
            let sh = builder.ins().iconst(I64, (f & 0x3F) as i64);
            let bit = builder.ins().ishl(one, sh);
            let not_bit = builder.ins().bnot(bit);
            Some(builder.ins().band(a, not_bit))
        }
        (0x13, 1, f) if (0x680..0x6C0).contains(&f) => {
            let sh = builder.ins().iconst(I64, (f & 0x3F) as i64);
            let bit = builder.ins().ishl(one, sh);
            Some(builder.ins().bxor(a, bit))
        }
        _ => None,
    };
    match val {
        Some(v) => store_rd(builder, ctx.state_ptr, rd, v),
        None => emit_trap(builder, ctx, insn_pc, count),
    }
}

fn emit_m32_op(
    builder: &mut FunctionBuilder,
    funct3: u8,
    a: cranelift_codegen::ir::Value,
    b: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    match funct3 {
        0 => builder.ins().imul(a, b),
        4 => {
            let zero = builder.ins().iconst(I64, 0);
            let z32 = builder.ins().ireduce(I32, zero);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, z32);
            let neg1 = builder.ins().iconst(I64, -1i64);
            let n32 = builder.ins().ireduce(I32, neg1);
            let one = builder.ins().iconst(I64, 1);
            let o32 = builder.ins().ireduce(I32, one);
            let min = builder.ins().iconst(I64, i32::MIN as i64);
            let min32 = builder.ins().ireduce(I32, min);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min32);
            let b_neg1 = builder.ins().icmp(IntCC::Equal, b, n32);
            let overflow = builder.ins().band(a_min, b_neg1);
            let safe_b = builder.ins().select(is_zero, o32, b);
            let safe_b = builder.ins().select(overflow, o32, safe_b);
            let result = builder.ins().sdiv(a, safe_b);
            let result = builder.ins().select(overflow, a, result);
            builder.ins().select(is_zero, n32, result)
        }
        5 => {
            let zero = builder.ins().iconst(I64, 0);
            let z32 = builder.ins().ireduce(I32, zero);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, z32);
            let neg1 = builder.ins().iconst(I64, -1i64);
            let n32 = builder.ins().ireduce(I32, neg1);
            let one = builder.ins().iconst(I64, 1);
            let o32 = builder.ins().ireduce(I32, one);
            let safe_b = builder.ins().select(is_zero, o32, b);
            let result = builder.ins().udiv(a, safe_b);
            builder.ins().select(is_zero, n32, result)
        }
        6 => {
            let zero = builder.ins().iconst(I64, 0);
            let z32 = builder.ins().ireduce(I32, zero);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, z32);
            let neg1 = builder.ins().iconst(I64, -1i64);
            let n32 = builder.ins().ireduce(I32, neg1);
            let one = builder.ins().iconst(I64, 1);
            let o32 = builder.ins().ireduce(I32, one);
            let min = builder.ins().iconst(I64, i32::MIN as i64);
            let min32 = builder.ins().ireduce(I32, min);
            let a_min = builder.ins().icmp(IntCC::Equal, a, min32);
            let b_neg1 = builder.ins().icmp(IntCC::Equal, b, n32);
            let overflow = builder.ins().band(a_min, b_neg1);
            let safe_b = builder.ins().select(is_zero, o32, b);
            let safe_b = builder.ins().select(overflow, o32, safe_b);
            let result = builder.ins().srem(a, safe_b);
            let result = builder.ins().select(overflow, z32, result);
            builder.ins().select(is_zero, a, result)
        }
        7 => {
            let zero = builder.ins().iconst(I64, 0);
            let z32 = builder.ins().ireduce(I32, zero);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, z32);
            let one = builder.ins().iconst(I64, 1);
            let o32 = builder.ins().ireduce(I32, one);
            let safe_b = builder.ins().select(is_zero, o32, b);
            let result = builder.ins().urem(a, safe_b);
            builder.ins().select(is_zero, a, result)
        }
        _ => a,
    }
}
