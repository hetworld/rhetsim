> [中文版](./05-roadmap.zh-CN.md)

---

# RhetSim — Roadmap

This document tracks the project-level implementation roadmap. Each Phase is a self-contained deliverable with explicit entry criteria, goals, acceptance tests, and capability boundaries.

## Status Legend

| Symbol | Meaning |
|--------|---------|
| Done | Implemented on `dev` and passes its acceptance tests |
| Active | In progress on `dev` |
| Planned | Not yet started |

> Branch note: `dev` is the current development line; `main` still holds the initial workspace skeleton. Stable milestones are merged back into `main` periodically. Status below reflects `dev`.

## Completed Phases

### Phase 0: Workspace Compiles (Done)

**Goal:** Core traits finalized; two heterogeneous Units instantiated in the same Platform.

**Deliverables:**
- `rhetsim-core`: Unit, Bus (MemBus/IrqBus/ResetBus/etc.), Backend, MemTarget traits
- `rhetsim-runtime`: scheduler, boot, clock, unit_thread
- `rhetsim-manifest`: loader, validator, expander, realizer pipeline
- MockCPU + end-to-end integration test

**Acceptance:** MockCPU + end-to-end integration test passes.

### Phase 1: RV64I Functional Interpreter (Done)

**Goal:** Minimal runnable RISC-V simulation with memory and UART.

**Deliverables:**
- `rhetsim-cpu-riscv`: RV64I base integer ISA interpreter (decode + execute)
- `rhetsim-mem-dram`: DRAM component (Vec-backed, MemTarget)
- `rhetsim-mem-sram`: SRAM component
- `rhetsim-io-uart`: UART component
- `hello_riscv` integration test

**Acceptance:** `hello_riscv` prints the expected output on the reference platform.

### Phase 2: Heterogeneous CPU+GPGPU Platform (Done)

**Goal:** CPU and GPGPU share a MemBus; GPGPU can receive commands and access VRAM.

**Deliverables:**
- `rhetsim-accel-gpgpu`: CmdProcessor, VramController, composite Unit
- MemBus `visible_to` filtering for per-initiator address space isolation
- `hetero_cpu_gpgpu` integration test
- Reference platform manifests (`platforms/reference/`)

**Capability boundary:** GPGPU is a functional model without cycle-accurate timing or a complete ISA.

### Phase 3: Interrupt Subsystem & RISC-V Completion (Done)

**Goal:** CPU can respond to interrupts; RISC-V supports a broader set of real-world binaries.

**Deliverables:**
- `rhetsim-intc-clint`: CLINT implementation (mtime/mtimecmp, MSIP per hart)
- `rhetsim-intc-plic`: PLIC implementation (priority, pending, enable, claim/complete)
- RISC-V CPU: CSR infrastructure (mstatus, mie, mip, mtvec, mcause, mepc, mtval)
- RISC-V CPU: trap delivery (interrupt check in exec loop, vectored entry)
- RV64M extension: MUL, MULH, MULHSU, MULHU, DIV, DIVU, REM, REMU (+ W variants)
- RV64A extension: LR.W/D, SC.W/D, AMO{SWAP,ADD,AND,OR,XOR,MIN,MAX,MINU,MAXU}.W/D
- RV64C compressed instruction decoding (interpreter and JIT)
- Sstc (`stimecmp`) timer support (used by the Linux boot path)
- End-to-end test: timer interrupt fires, trap handler executes, returns to user code

**Acceptance:** Timer / PLIC / UART interrupt end-to-end tests pass; `/proc/interrupts` counters increase during Linux boot.

**Capability boundary:** F/D floating-point extensions are not implemented; the DTS and kernel config now agree on the C extension (see Phase 6.5).

### Phase 4: GPGPU Compute Path (Done)

**Goal:** GPGPU can actually execute compute kernels dispatched by CPU.

**Deliverables:**
- SimtHartBackend (Route A): RISC-V hart array for SM SIMT execution ✓
- Warp scheduling: round-robin across warps ✓
- `rhetsim-mem-gddr`: GDDR memory backing for VRAM ✓
- `rhetsim-io-pcie`: PCIe bus basic implementation (BAR mapping, config space, TLP routing) ✓
- End-to-end test: CPU dispatches kernel → GPGPU executes → results in shared memory ✓

**Capability boundary:** PCIe covers basic TLP/BAR paths, not a full protocol stack or device enumeration.

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

**Capability boundary:** AArch64 is a core integer subset; GICv3 covers the minimal interrupt path; no EL2/EL3 virtualization extensions.

### Phase 6: Ecosystem & Usability (Done)

**Goal:** Users can load manifests via CLI, run simulations, and perform basic debugging.

**Deliverables:**
- `rhetsim-cli`: functional CLI (load manifest → instantiate platform → run) ✓
- `rhetsim-compat` QMP server: minimal VM lifecycle subset (query-status, quit, stop, cont) ✓
- `rhetsim-io-virtio`: VirtIO console device (standardized alternative to bare UART) ✓
- `rhetsim-dbg`: basic debugger (single-step, register/memory dump, breakpoints) ✓

**Capability boundary:** VirtIO implements console only; QMP covers a lifecycle subset; debugger is basic.

## Active Phase

### Phase 6.5: Linux System-Software Bring-up (Active)

**Goal:** Boot real system software reliably on RhetSim and provide an interactive serial shell.

**Deliverables:**
- OpenSBI (`fw_jump`) + Linux 6.1 2-hart SMP boot to embedded initramfs ✓
- PLIC/UART interrupts, Sstc timer, per-hart CLINT/PLIC contexts ✓
- Kernel printk timestamps (`printk.time=1`) ✓
- BusyBox-based root filesystem (rv64ima soft-float static build) ✓
- `/init` interactive shell (ash) and basic command set ✓
- JIT data-access page-fault delivery (userspace demand paging / A-bit handling) ✓
- Align kernel config and DTS extension declarations (C extension) ✓

**Acceptance:**
- Batch mode output contains `Run /init as init process` and `RHETSIM_INIT_OK`, process exit code 0
- PLIC `ttyS0` and `riscv-timer` counters in `/proc/interrupts` grow after a 1-second sleep
- Interactive mode runs `ls`/`cat`, and `Ctrl-A X` exits cleanly

**Non-goals:** F/D floating-point userspace and virtio-blk rootfs (see Phase 6.6 / 6.7).

## Planned Phases

### Phase 6.6: ISA Completeness & Compliance (Planned)

**Goal:** Make the simulator's ISA capability truthful, complete, and verifiable.

**Entry criteria:** Phase 6.5 complete; Linux boots reliably into userspace.

**Deliverables:**
- RV64F/D floating-point extensions (interpreter + JIT, CSR and `mstatus.FS` state management)
- DTS faithfully declares C/F/D extensions; kernel rebuilt with `CONFIG_FPU`
- riscv-tests / riscv-arch-test compliance suites integrated
- Instruction-level difftest framework against Spike/QEMU

**Acceptance:** arch-test pass rate meets the agreed threshold; difftest shows no divergence on randomized instruction streams.

**Capability boundary:** Vector (V) extension is out of scope.

### Phase 6.7: Device Models & Root Filesystem (Planned)

**Goal:** Support a real rootfs, graceful shutdown, and basic networking.

**Deliverables:**
- virtio-mmio block (and optionally virtio-net)
- RTC and syscon poweroff/reboot (corresponding OpenSBI devices become visible)
- Buildroot/BusyBox rootfs image and boot scripts
- CI: OpenSBI/Linux build caching and boot regression (golden log)

**Acceptance:** ext4 rootfs mounts from virtio-blk and boots to a shell; `poweroff` terminates the simulator cleanly.

**Capability boundary:** Not a complete PCIe device ecosystem.

### Phase 7: Performance Modeling (Planned)

**Goal:** Transition from functional simulation to timing simulation.

**Entry criteria:** Phase 6.6 compliance tests and the difftest baseline are complete (functional correctness is demonstrable).

**Deliverables:**
- TimingBackend implementation + LooseTiming mode
- SyncBus timing coordination (multi-Unit clock synchronization)
- `rhetsim-cache`: L1/L2 cache hierarchy (currently a placeholder), configurable replacement policies and latencies
- DDR controller timing model (bank conflicts, refresh, tCAS/tRCD/tRP)

**Non-goals:** Cycle-accurate CPU pipeline modeling (to be evaluated separately).

### Phase 8: JIT/TCG & RTL Co-simulation (Planned)

**Goal:** Dynamic binary translation pipeline + RTL verification bridge.

**Prerequisite decision:** Whether the existing Cranelift JIT in `rhetsim-cpu-riscv` is upgraded in place or replaced by a custom TCG pipeline.

**Deliverables:**
- `rhetsim-tcg`: IR implementation, basic optimizer passes, TB cache (skeleton exists)
- x86-64 host backend (JIT to host machine)
- RISC-V / ARM frontend migration to TCG pipeline
- RtlBackend interface + FPGA bridge driver
- Mixed-precision co-simulation (functional + cycle-accurate)

**Capability boundary:** Mixed-precision co-simulation is experimental.

### Phase 9: Security Extensions (Planned)

**Goal:** Confidential computing and TrustZone modeling.

**Prerequisite decision:** Whether to include the RISC-V H extension / CoVE track.

**Deliverables:**
- ARM EL2/EL3 TCG frontend
- MemSecurity enforcement layer (address space isolation checks)
- ARM CCA RMM modeling
- AMD SEV-SNP PSP modeling

---

## Dependency Graph

```text
Phase 6.5 (system software) ─┐
Phase 6.6 (ISA/compliance) ──┼── Phase 7 (performance) ── Phase 8 (TCG/RTL) ── Phase 9 (security)
Phase 6.7 (devices/rootfs) ──┘
```

Phase 6.6 gates Phase 7; the JIT strategy decision must land before Phase 8 starts.

## Milestone ↔ Phase Mapping

The original architecture document (01-architecture.md §5.1) defined milestones M0–M6. The Phase plan above is a refinement based on actual implementation progress:

| Original Milestone | Covered By |
|---|---|
| M0 Architecture | Phase 0 |
| M1 Functional Simulation | Phase 1 + Phase 3 + Phase 5 |
| M2 Ecosystem Integration | Phase 6 + Phase 6.5 |
| M3 Heterogeneous Platform | Phase 2 + Phase 4 |
| M4 Performance Simulation | Phase 7 |
| M5 RTL Co-simulation | Phase 8 |
| M6 Security Extensions | Phase 9 |

> Phases 6.6 / 6.7 are new ISA-correctness and system-usability phases beyond the original M0–M6 milestone set.
