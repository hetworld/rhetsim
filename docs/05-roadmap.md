> [中文版](./05-roadmap.zh-CN.md)

---

# RhetSim — Roadmap

This document tracks the project-level implementation roadmap. Each Phase is a self-contained deliverable with clear entry criteria, goals, and verification tests.

## Status Legend

| Symbol | Meaning |
|--------|---------|
| Done | Merged to `main` |
| Active | In progress on `dev` |
| Planned | Not yet started |

## Completed Phases

### Phase 0: Workspace Compiles (Done)

**Goal:** Core traits finalized; two heterogeneous Units instantiated in the same Platform.

**Deliverables:**
- `rhetsim-core`: Unit, Bus (MemBus/IrqBus/ResetBus/etc.), Backend, MemTarget traits
- `rhetsim-runtime`: scheduler, boot, clock, unit_thread
- `rhetsim-manifest`: loader, validator, expander, realizer pipeline
- MockCPU + end-to-end integration test

### Phase 1: RV64I Functional Interpreter (Done)

**Goal:** Minimal runnable RISC-V simulation with memory and UART.

**Deliverables:**
- `rhetsim-cpu-riscv`: RV64I base integer ISA interpreter (decode + execute)
- `rhetsim-mem-dram`: DRAM component (Vec-backed, MemTarget)
- `rhetsim-mem-sram`: SRAM component
- `rhetsim-io-uart`: UART component
- `hello_riscv` integration test

### Phase 2: Heterogeneous CPU+GPGPU Platform (Done)

**Goal:** CPU and GPGPU share a MemBus; GPGPU can receive commands and access VRAM.

**Deliverables:**
- `rhetsim-accel-gpgpu`: CmdProcessor, VramController, composite Unit
- MemBus `visible_to` filtering for per-initiator address space isolation
- `hetero_cpu_gpgpu` integration test
- Reference platform manifests (`platforms/reference/`)

---

## Planned Phases

### Phase 3: Interrupt Subsystem & RISC-V Completion (Done)

**Goal:** CPU can respond to interrupts; RISC-V supports a broader set of real-world binaries.

**Deliverables:**
- `rhetsim-intc-clint`: CLINT implementation (mtime/mtimecmp, MSIP per hart)
- `rhetsim-intc-plic`: PLIC implementation (priority, pending, enable, claim/complete)
- RISC-V CPU: CSR infrastructure (mstatus, mie, mip, mtvec, mcause, mepc, mtval)
- RISC-V CPU: trap delivery (interrupt check in exec loop, vectored entry)
- RV64M extension: MUL, MULH, MULHSU, MULHU, DIV, DIVU, REM, REMU (+ W variants)
- RV64A extension: LR.W/D, SC.W/D, AMO{SWAP,ADD,AND,OR,XOR,MIN,MAX,MINU,MAXU}.W/D
- End-to-end test: timer interrupt fires, trap handler executes, returns to user code

### Phase 4: GPGPU Compute Path (Done)

**Goal:** GPGPU can actually execute compute kernels dispatched by CPU.

**Deliverables:**
- SimtHartBackend (Route A): RISC-V hart array for SM SIMT execution ✓
- Warp scheduling: round-robin across warps ✓
- `rhetsim-mem-gddr`: GDDR memory backing for VRAM ✓
- `rhetsim-io-pcie`: PCIe bus basic implementation (BAR mapping, config space, TLP routing) ✓
- End-to-end test: CPU dispatches kernel → GPGPU executes → results in shared memory ✓

### Phase 5: ARM & Multi-ISA Heterogeneous (Done)

**Goal:** Second real CPU architecture; three-way heterogeneous platform runs.

**Deliverables:**
- `rhetsim-cpu-arm`: AArch64 interpreter (core integer subset, following RV64I pattern) ✓
- End-to-end test: ARM host + RISC-V coprocessor on shared MemBus ✓
- `rhetsim-intc-gicv3`: GICv3 interrupt controller (Distributor, Redistributor, CPU interface) ✓
- AArch64 CPU ↔ GICv3 wiring (ICC system registers, IRQ exception vectoring, ERET) ✓
- End-to-end test: ARM CPU + GICv3 interrupt-driven program ✓
- `rhetsim-io-dma`: DMA controller for cross-device bulk memory transfer ✓
- End-to-end test: ARM + RISC-V + GPGPU on a single platform ✓

### Phase 6: Ecosystem & Usability (Done)

**Goal:** Users can load manifests via CLI, run simulations, and perform basic debugging.

**Deliverables:**
- `rhetsim-cli`: functional CLI (load manifest → instantiate platform → run) ✓
- `rhetsim-compat` QMP server: minimal VM lifecycle subset (query-status, quit, stop, cont) ✓
- `rhetsim-io-virtio`: VirtIO console device (standardized alternative to bare UART) ✓
- `rhetsim-dbg`: basic debugger (single-step, register/memory dump, breakpoints) ✓

### Phase 7: Performance Modeling

**Goal:** Transition from functional simulation to timing simulation.

**Deliverables:**
- TimingBackend implementation + LooseTiming mode
- SyncBus timing coordination (multi-Unit clock synchronization)
- `rhetsim-cache`: L1/L2 cache hierarchy with configurable replacement policies and latencies
- DDR controller timing model (bank conflicts, refresh, tCAS/tRCD/tRP)

### Phase 8: TCG & RTL Co-simulation

**Goal:** Dynamic binary translation pipeline + RTL verification bridge.

**Deliverables:**
- `rhetsim-tcg`: IR implementation, basic optimizer passes
- x86-64 host backend (JIT to host machine)
- RISC-V / ARM frontend migration to TCG pipeline
- RtlBackend interface + FPGA bridge driver
- Mixed-precision co-simulation (functional + cycle-accurate)

### Phase 9: Security Extensions

**Goal:** Confidential computing and TrustZone modeling.

**Deliverables:**
- ARM EL2/EL3 TCG frontend
- MemSecurity enforcement layer (address space isolation checks)
- ARM CCA RMM modeling
- AMD SEV-SNP PSP modeling

---

## Dependency Graph

```
Phase 3 (Interrupts) ──┬── Phase 4 (GPGPU) ──┐
                       │                      ├── Phase 7 (Performance) ── Phase 8 (TCG/RTL)
                       └── Phase 5 (ARM) ─────┘        │
                                                        └── Phase 9 (Security)
                            Phase 6 (Ecosystem) — parallel with 4/5
```

## Milestone ↔ Phase Mapping

The original architecture document (01-architecture.md §5.1) defined milestones M0–M6. The Phase plan above is a refinement based on actual implementation progress:

| Original Milestone | Covered By |
|---|---|
| M0 Architecture | Phase 0 |
| M1 Functional Simulation | Phase 1 + Phase 3 + Phase 5 |
| M2 Ecosystem Integration | Phase 6 |
| M3 Heterogeneous Platform | Phase 2 + Phase 4 |
| M4 Performance Simulation | Phase 7 |
| M5 RTL Co-simulation | Phase 8 |
| M6 Security Extensions | Phase 9 |
