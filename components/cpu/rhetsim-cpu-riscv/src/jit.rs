use std::collections::HashMap;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::*;
use cranelift_codegen::ir::{AbiParam, BlockArg, Function, InstBuilder, MemFlagsData, Signature, UserFuncName};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::decode::{decode, decode_compressed, Instruction};

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
}

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

pub const FAST_TLB_SIZE: usize = 256;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FastTlbEntry {
    pub tag: u64,
    pub addend: u64,
}

impl FastTlbEntry {
    pub const fn invalid() -> Self {
        Self { tag: u64::MAX, addend: 0 }
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
    fb_ctx: FunctionBuilderContext,
    tb_counter: u64,
    compiles: u64,
    executions: u64,
    invalidations: u64,
}

unsafe impl Send for JitEngine {}
unsafe impl Sync for JitEngine {}

fn memflags() -> MemFlagsData {
    MemFlagsData::new()
}

impl JitEngine {
    pub fn new() -> Self {
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
        let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu")
            .unwrap();
        let isa = isa_builder.finish(settings::Flags::new(flag_builder)).unwrap();

        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("helper_load", helper_load as *const u8);
        builder.symbol("helper_store", helper_store as *const u8);

        let mut module = JITModule::new(builder);

        let load_id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module.declare_function("helper_load", Linkage::Import, &sig).unwrap()
        };

        let store_id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
            module.declare_function("helper_store", Linkage::Import, &sig).unwrap()
        };

        JitEngine {
            module,
            tb_cache: HashMap::new(),
            load_id,
            store_id,
            fb_ctx: FunctionBuilderContext::new(),
            tb_counter: 0,
            compiles: 0,
            executions: 0,
            invalidations: 0,
        }
    }

    /// (compiled blocks, block executions, cache invalidations)
    pub fn stats(&self) -> (u64, u64, u64) {
        (self.compiles, self.executions, self.invalidations)
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
        let key = TbKey { pc: state.pc, priv_mode, satp };

        if !self.tb_cache.contains_key(&key) {
            match self.compile_block(state.pc, fetch_fn) {
                Some(tb) => {
                    self.compiles += 1;
                    self.tb_cache.insert(key, tb);
                }
                None => return false,
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

        loop {
            let raw = match fetch_fn(pc) {
                Ok(v) => v,
                Err(_) => break,
            };

            let half = raw as u16;
            if half == 0 { break; }

            let (insn, size) = if half & 0x3 != 0x3 {
                (decode_compressed(half), 2u64)
            } else {
                (decode(raw as u32), 4u64)
            };

            let is_block_end = matches!(insn,
                Instruction::Jal { .. } |
                Instruction::Jalr { .. } |
                Instruction::Branch { .. } |
                Instruction::Ecall |
                Instruction::Ebreak |
                Instruction::Mret |
                Instruction::Sret |
                Instruction::Wfi |
                Instruction::Csr { .. } |
                Instruction::SfenceVma { .. } |
                Instruction::Amo { .. } |
                Instruction::FenceI |
                Instruction::Unknown(_)
            );

            insns.push((insn, pc, size, raw as u32));
            pc += size;

            if is_block_end || insns.len() >= max_insns { break; }
        }

        if insns.is_empty() { return None; }

        let end_pc = pc;
        let insn_count = insns.len() as u64;

        self.tb_counter += 1;
        let func_name = format!("tb_{}", self.tb_counter);

        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(I64));
        sig.params.push(AbiParam::new(I64));

        let func_id = self.module
            .declare_function(&func_name, Linkage::Local, &sig)
            .unwrap();

        let mut func = Function::with_name_signature(
            UserFuncName::testcase(&func_name), sig.clone(),
        );

        let load_ref = self.module.declare_func_in_func(self.load_id, &mut func);
        let store_ref = self.module.declare_func_in_func(self.store_id, &mut func);
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

            let exit_block = builder.create_block();

            for (idx, &(ref insn, insn_pc, insn_size, raw)) in insns.iter().enumerate() {
                match insn {
                    Instruction::Lui { rd, imm } => {
                        if *rd != 0 {
                            let val = builder.ins().iconst(I64, *imm);
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Auipc { rd, imm } => {
                        if *rd != 0 {
                            let result = insn_pc.wrapping_add(*imm as u64);
                            let val = builder.ins().iconst(I64, result as i64);
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::OpImm { funct3, rd, rs1, imm, funct7 } => {
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
                                    if funct7 & 0x20 != 0 { builder.ins().sshr(src, sh) }
                                    else { builder.ins().ushr(src, sh) }
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
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Op { funct3, funct7, rd, rs1, rs2 } => {
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
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::OpImm32 { funct3, rd, rs1, imm, funct7 } => {
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
                                    if funct7 & 0x20 != 0 { builder.ins().sshr(src, sh32) }
                                    else { builder.ins().ushr(src, sh32) }
                                }
                                _ => src,
                            };
                            let val = builder.ins().sextend(I64, val32);
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Op32 { funct3, funct7, rd, rs1, rs2 } => {
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
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Load { funct3, rd, rs1, offset } => {
                        let base = load_reg(&mut builder, state_ptr, *rs1);
                        let off_val = builder.ins().iconst(I64, *offset);
                        let va = builder.ins().iadd(base, off_val);
                        let size_val = match funct3 { 0|4 => 1i64, 1|5 => 2, 2|6 => 4, 3 => 8, _ => 1 };

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
                            2 => { let v = builder.ins().load(I16, memflags(), host_addr, 0); builder.ins().uextend(I64, v) }
                            4 => { let v = builder.ins().load(I32, memflags(), host_addr, 0); builder.ins().uextend(I64, v) }
                            8 => builder.ins().load(I64, memflags(), host_addr, 0),
                            _ => { let v = builder.ins().load(I8, memflags(), host_addr, 0); builder.ins().uextend(I64, v) }
                        };
                        let arg_fast = BlockArg::Value(raw_fast);
                        builder.ins().jump(merge_block, &[arg_fast]);

                        builder.switch_to_block(slow_block);
                        builder.seal_block(slow_block);
                        let size_arg = builder.ins().iconst(I64, size_val);
                        let call = builder.ins().call(load_ref, &[mem_ctx, va, size_arg]);
                        let raw_slow = builder.inst_results(call)[0];
                        let fault_ptr = builder.ins().iadd_imm(mem_ctx, 40);
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
                                0 => { let v = builder.ins().ireduce(I8, raw_val); builder.ins().sextend(I64, v) }
                                1 => { let v = builder.ins().ireduce(I16, raw_val); builder.ins().sextend(I64, v) }
                                2 => { let v = builder.ins().ireduce(I32, raw_val); builder.ins().sextend(I64, v) }
                                3 => raw_val,
                                4 => { let m = builder.ins().iconst(I64, 0xFF); builder.ins().band(raw_val, m) }
                                5 => { let m = builder.ins().iconst(I64, 0xFFFF); builder.ins().band(raw_val, m) }
                                6 => { let m = builder.ins().iconst(I64, 0xFFFF_FFFFi64); builder.ins().band(raw_val, m) }
                                _ => raw_val,
                            };
                            builder.ins().store(memflags(), val, state_ptr, (*rd as i32) * 8);
                        }
                    }

                    Instruction::Store { funct3, rs1, rs2, offset } => {
                        let base = load_reg(&mut builder, state_ptr, *rs1);
                        let off_val = builder.ins().iconst(I64, *offset);
                        let va = builder.ins().iadd(base, off_val);
                        let val = load_reg(&mut builder, state_ptr, *rs2);
                        let size_val = match funct3 { 0 => 1i64, 1 => 2, 2 => 4, 3 => 8, _ => 1 };

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
                            1 => { let v = builder.ins().ireduce(I16, val); builder.ins().store(memflags(), v, host_addr, 0); }
                            2 => { let v = builder.ins().ireduce(I32, val); builder.ins().store(memflags(), v, host_addr, 0); }
                            3 => { builder.ins().store(memflags(), val, host_addr, 0); }
                            _ => { let v = builder.ins().ireduce(I8, val); builder.ins().store(memflags(), v, host_addr, 0); }
                        };
                        builder.ins().jump(cont_block, &[]);

                        builder.switch_to_block(slow_block);
                        builder.seal_block(slow_block);
                        let size_arg = builder.ins().iconst(I64, size_val);
                        let _ = builder.ins().call(store_ref, &[mem_ctx, va, size_arg, val]);
                        let fault_ptr = builder.ins().iadd_imm(mem_ctx, 40);
                        let fault = builder.ins().load(I64, memflags(), fault_ptr, 0);
                        let zero = builder.ins().iconst(I64, 0);
                        let faulted = builder.ins().icmp(IntCC::NotEqual, fault, zero);
                        let fault_block = builder.create_block();
                        builder.ins().brif(faulted, fault_block, &[], cont_block, &[]);
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
                            builder.ins().store(memflags(), link, state_ptr, (*rd as i32) * 8);
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
                            builder.ins().store(memflags(), link, state_ptr, (*rd as i32) * 8);
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

                    Instruction::Branch { funct3, rs1, rs2, offset } => {
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
                        builder.ins().brif(cond, taken_block, &[], not_taken_block, &[]);

                        builder.switch_to_block(taken_block);
                        builder.seal_block(taken_block);
                        let target = insn_pc.wrapping_add(*offset as u64);
                        emit_exit_with_pc(&mut builder, state_ptr, target, EXIT_BLOCK_END, idx + 1);
                        builder.ins().jump(exit_block, &[]);

                        builder.switch_to_block(not_taken_block);
                        builder.seal_block(not_taken_block);
                        emit_exit_with_pc(&mut builder, state_ptr, insn_pc + insn_size, EXIT_BLOCK_END, idx + 1);
                        builder.ins().jump(exit_block, &[]);

                        let nb = builder.create_block();
                        builder.switch_to_block(nb);
                        builder.seal_block(nb);
                    }

                    Instruction::Ecall => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_ECALL, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Ebreak => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_EBREAK, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Mret => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_MRET, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Sret => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_SRET, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Wfi => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_WFI, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::SfenceVma { .. } => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_SFENCE, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Csr { .. } => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_CSR, idx + 1, raw);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Amo { .. } => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_AMO, idx + 1, raw);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::FenceI => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_FENCEI, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                    Instruction::Fence => { /* no-op */ }
                    Instruction::Unknown(_) => {
                        emit_exit_insn(&mut builder, state_ptr, insn_pc, EXIT_TRAP, idx + 1, 0);
                        builder.ins().jump(exit_block, &[]);
                        let nb = builder.create_block();
                        builder.switch_to_block(nb); builder.seal_block(nb);
                    }
                }
            }

            // Fallthrough: set PC past end of block
            emit_exit_with_pc(&mut builder, state_ptr, end_pc, EXIT_BLOCK_END, insn_count as usize);
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
            code: unsafe { std::mem::transmute(code_ptr) },
            guest_pc: start_pc,
            guest_end_pc: end_pc,
            insn_count,
        })
    }
}

fn load_reg(builder: &mut FunctionBuilder, state_ptr: cranelift_codegen::ir::Value, reg: u8) -> cranelift_codegen::ir::Value {
    if reg == 0 {
        builder.ins().iconst(I64, 0)
    } else {
        builder.ins().load(I64, memflags(), state_ptr, (reg as i32) * 8)
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
            // DIV signed: b==0 → -1, MIN/-1 → MIN (but sdiv traps on that)
            // Use helper: check b==0 first
            let zero = builder.ins().iconst(I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
            let neg1 = builder.ins().iconst(I64, -1i64);
            // For b!=0 case, also check MIN/-1 overflow
            let one = builder.ins().iconst(I64, 1);
            let safe_b = builder.ins().select(is_zero, one, b);
            let result = builder.ins().sdiv(a, safe_b);
            // Check MIN/-1 case: if a==MIN && b==-1 → result=MIN (which sdiv gives us correctly)
            // Actually cranelift sdiv with safe_b=1 when b==0 gives a/1=a, then we select -1
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
            let zero = builder.ins().iconst(I64, 0);
            let is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
            let one = builder.ins().iconst(I64, 1);
            let safe_b = builder.ins().select(is_zero, one, b);
            let result = builder.ins().srem(a, safe_b);
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
            let safe_b = builder.ins().select(is_zero, o32, b);
            let result = builder.ins().sdiv(a, safe_b);
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
            let one = builder.ins().iconst(I64, 1);
            let o32 = builder.ins().ireduce(I32, one);
            let safe_b = builder.ins().select(is_zero, o32, b);
            let result = builder.ins().srem(a, safe_b);
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
