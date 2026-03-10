> [中文版](./01-architecture.zh-CN.md)

---

# OmniSim — Architecture Design

> **Document ID**: DOC-01 &nbsp;&nbsp; **Version**: v1.0 &nbsp;&nbsp; **Date**: 2026-Q1 &nbsp;&nbsp; **Author**: OpenFunTech

---

| Document ID | DOC-01 |
| --- | --- |
| Version | v1.0 |
| Audience | All team members (entry-level document) |
| Related Docs | DOC-02 Interface Spec · DOC-03 Component Dev Guide · DOC-04 Use Case Handbook |
| Last Updated | 2026-Q1 |

## 1  Background & Motivation

### 1.1  The Reality of Hardware Heterogeneity

Modern compute platforms are no longer single-architecture CPU systems. A single AI server motherboard may host an x86 main processor, an ARM management core, a RISC-V control core, a GPU, a DPU, and an NPU, along with various accelerator cards connected via PCIe / CXL. The software stack must coordinate across all these heterogeneous units, and the only reliable means of verifying that coordination is simulation.

### 1.2  QEMU's World View and Its Limitations

QEMU is currently the most widely-used open-source emulator, but it was born in the era of CPU emulation and its design is CPU-centric:

```
QEMU's Fundamental Limitations
CPU is a privileged object: it owns a dedicated execution loop; Devices can only respond passively
Connection relationships are buried in C initialization code: not queryable, not serializable, not dynamically modifiable
Monolithic architecture: components are tightly coupled, cannot be tested or released independently
A Device cannot contain a full SoC: Composite Device recursion is not supported
Single simulation fidelity: functional simulation only, cannot model performance
```

These limitations are tolerable when simulating a single-CPU platform, but they cause severe architectural impedance in heterogeneous platform scenarios: GPU SM arrays, DPU network processing cores, and NPU compute arrays simply cannot be expressed elegantly within the QEMU framework.

### 1.3  rust-vmm's Value and Boundary

rust-vmm is a virtualization component library in the Rust ecosystem, providing high-quality modular primitives (kvm-ioctls, vm-memory, virtio-queue, etc.), but it is not a framework in itself:

```
rust-vmm's Role
Value: high-quality Rust safe virtualization primitives, reused directly by OmniSim
Boundary: just a component library; no composition framework; users manage connections themselves
Missing: no Bus abstraction, no Manifest, no scheduler
```

### 1.4  What OmniSim Solves

```
OmniSim's Positioning
OmniSim = rust-vmm's composition philosophy + heterogeneous platform topology description + libvirt/QEMU ecosystem compatibility
Sits between QEMU (monolithic) and rust-vmm (pure component library): provides a minimal composition framework while preserving component independence
```

| Dimension | QEMU | rust-vmm | OmniSim |
| --- | --- | --- | --- |
| Architecture Style | Monolithic, internally tightly coupled | Pure component library, no framework | Component library + minimal composition framework |
| CPU Status | Privileged object with dedicated execution loop | No unified abstraction | Unit, fully equal to NPU/DPU |
| Connection Relationships | Hidden in C initialization code | Managed by users | Bus is a first-class citizen, explicit and queryable |
| Device-internal SoC | Not supported | | Composite Unit recursive nesting |
| Simulation Fidelity | Functional only | Undefined | Functional/Timing/RTL can be mixed |
| libvirt Compatibility | Native | Not applicable | L4 compatibility layer, transparent to libvirt |
| Component Reuse | Not possible | rust-vmm crates | Directly reuses rust-vmm crates |

## 2  Core Abstractions

OmniSim's entire design is built on five core concepts. Understanding these five concepts means understanding the essence of OmniSim.

### 2.1  Five Core Concepts

```
Five Core Concepts
Unit     — the smallest independently-replaceable hardware simulation unit in a platform
Bus      — an explicit expression of the connection relationship between Units (first-class citizen)
Backend  — the execution implementation of a Unit (execution dimension × timing dimension)
Manifest — a complete declarative description of a platform, serializable to JSON/TOML
Profile  — a concrete instance of a Manifest (embedded / server / hetero / etc.)
```

### 2.2  Unit: The Granularity Principle

There is only one criterion for determining Unit granularity: the smallest hardware functional unit that can be independently replaced = one Unit.

| Type | Criterion | Examples |
| --- | --- | --- |
| Leaf Unit | Contains no child Units; has concrete execution or response logic | CPU Core, MMU, GIC GICD, L1 Cache, UART |
| Composite Unit | Composed internally of multiple child Units; exposes boundary interface externally | ARM SoC, GICv3, GPGPU, DPU, RISC-V subsystem |
| Executable Unit | Implements ExecutionBackend; can be driven by the scheduler | CPU Core, NPU, DPU, DMA Engine, Timer |
| Addressable Unit | Implements MemTarget; responds to address read/write | DRAM, SRAM, UART registers, GIC registers |

### 2.3  Bus: Connections Are First-Class Citizens

This is the most fundamental difference between OmniSim and QEMU. In QEMU, connections between Units are buried in C initialization code — not queryable, not serializable. OmniSim elevates connections to first-class citizens, expressing them explicitly as Bus objects.

OmniSim defines 10 types of Bus, covering all connection scenarios:

| Bus Type | Responsibility | Typical Use Case |
| --- | --- | --- |
| MemBus | Address routing, read/write forwarding, partial visibility | CPU→DRAM, CPU→peripheral registers |
| IrqBus | Interrupt signal routing | UART irq → GIC → CPU |
| PcieBus | PCIe device discovery, BAR, MSI-X | Host → GPU, Host → DPU |
| SyncBus | Mixed-precision timing coordination | Functional + Timing mixed simulation |
| DataBus | High-bandwidth data flow (separate from MemBus) | NPU → DRAM, DMA bulk transfer |
| ResetBus | Reset signal modeling | PERST#, on-chip Reset Controller |
| PowerBus | Power domain management | SoC multi-core power on/off, DVFS |
| InterconnectBus | Peer-to-peer node interconnect (Windowed/Unified) | NUMA, NVLink, CXL.mem |
| NetworkBackend | Unified external network access interface | DPU nvme-of, network processor |
| CoherenceDomain | Cache coherence domain (P2) | CXL.cache, ACE/CHI |

### 2.4  Backend: Two Orthogonal Dimensions

A Backend describes how a Unit executes, along two completely orthogonal dimensions:

| Execution Dimension | Timing Dimension | Combined Use Case | Product Phase |
| --- | --- | --- | --- |
| Functional (TCG) | Untimed | Functional simulation, verifying software correctness, fastest | OmniSim P0 open source |
| Timing (performance model) | LooseTiming | Performance simulation, modeling latency/bandwidth/throughput | P1 |
| Rtl (FPGA bridge) | CycleAccurate | RTL co-verification, cycle-accurate | P2 |
| Kvm (hardware acceleration) | Untimed | Hypervisor, real hardware execution | LeoVirt-S |
| Mock (test stub) | Untimed | Unit testing, CI verification | Test infrastructure |
| SimtHart (RISC-V hart array) | Untimed/Loose | GPGPU SM route A | OmniSim P0 |
| SimtWarp (SIMT Warp scheduler) | Untimed/Loose | GPGPU SM route B | OmniSim P1 |

Within the same platform, different Units can use different Backends and different TimingModels; SyncBus coordinates the timing.

### 2.5  Manifest: Declarative Platform Description

A Manifest is the complete declaration of a platform's topology, serializable to JSON or TOML. It is the language OmniSim uses to interact with the outside world. A Manifest describes "what Units exist" and "how they are connected" — it contains no execution logic.

```toml
# A minimal Manifest example
[platform]
name = "arm-riscv-coprocessor"
[[units]]
id      = "arm-core"
kind    = "arm-cortex-a78-core"
backend = "kvm"
[[units]]
id      = "riscv-ctrl"
kind    = "riscv-rv64-core"
backend = "functional"
[[buses]]
kind = "reset"
id   = "soc-reset"
[[buses.connections]]
from = "arm-core.reset_ctrl"
to   = "riscv-ctrl.reset_in"
```

## 3  Four-Layer Architecture

### 3.1  Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│  L4  Compatibility Layer                                     │
│      CLI Parser (QEMU command-line compatibility)            │
│      QMP Server (libvirt / OpenStack protocol)               │
│      Depends on: L3 Manifest API                             │
├──────────────────────────────────────────────────────────────┤
│  L3  Manifest Layer                                          │
│      UnitDesc · BusDesc · Realizer · Registry                │
│      Depends on: L2 Unit/Bus traits                          │
├──────────────────────────────────────────────────────────────┤
│  L2  Composition Layer                                       │
│      Unit (with sub_units recursion) · Bus runtime instances │
│      Depends on: L1 Backend/TimingModel traits               │
├──────────────────────────────────────────────────────────────┤
│  L1  Backend Layer                                           │
│      ExecutionBackend · TimingModel                          │
│      Directly reuses rust-vmm crates                         │
└──────────────────────────────────────────────────────────────┘
```

### 3.2  Inter-Layer Dependency Principles

Layer dependencies are strictly one-directional; upper layers only access lower layers through interfaces, and no layer may call across layers:

L4 only knows QEMU/libvirt protocols; it operates through L3 interfaces and never directly manipulates Units

L3 only knows the Manifest format; it realizes through L2 interfaces and knows nothing of Backend internals

L2 only knows Units and Buses; it executes through L1 interfaces and knows nothing of internal implementations

L1 only knows execution logic and has no knowledge of which platform it is part of

### 3.3  L1 — Backend Layer

The actual execution implementation of a Unit. Each Backend is independently implemented and replaceable without affecting upper layers. Directly reuses rust-vmm crates as its low-level foundation.

| Backend | Mechanism | Timing Model | Use Case |
| --- | --- | --- | --- |
| Functional | Ported QEMU TCG (ARM/RISC-V/x86 frontends, x86_64/AArch64 backends) | Untimed | Functional verification, speed-first |
| Timing | Microarchitecture performance model, modeling pipeline, cache miss, memory bandwidth | LooseTiming | Performance simulation |
| Rtl | FPGA bridge driver, forwarding simulation requests to RTL on FPGA | CycleAccurate | RTL co-verification |
| Kvm | Reuses rust-vmm/kvm-ioctls, driven via /dev/kvm ioctl | Untimed (real time) | Same-architecture virtualization |
| Mock | Records all calls, supports injecting custom return values | Untimed | Unit testing / CI |
| SimtHart | RISC-V hart array, RVV + custom Tensor extensions | Untimed/Loose | GPGPU SM route A |
| SimtWarp | Warp scheduler + vector execution unit, pluggable GpuFrontend trait | Untimed/Loose | GPGPU SM route B |

### 3.4  L2 — Composition Layer

Runtime Unit composition and connectivity. Connection relationships exist explicitly as Bus objects — queryable and dynamically modifiable (hot-plug).

#### Unit Lifecycle

```
Create → realize (connect Buses, initialize internal state)
→ reset (reset to power-on initial state)
→ exec loop (Backend-driven execution, managed by scheduler)
→ destroy (release all resources)
```

#### Composite Unit Recursion

A Unit contains sub_units internally; realize() recursively realizes all child Units. Child Units connect to internal Buses; the outside world interacts with the parent Unit through boundary Ports. Nesting depth is unlimited. This allows a complex structure like a GPGPU (containing an ARM ctrl + SM array + HBM) to be mounted on a PcieBus as a single Unit, completely transparent to the outside.

#### MemSpace Isolation

Each Initiator Unit holds its own MemSpace (address routing table). The ARM Core's MemSpace and the RISC-V Core's MemSpace are independent; on-demand sharing is achieved via MemBus's `visible_to` mechanism, providing natural support for on-chip security isolation and subsystem address space separation.

### 3.5  L3 — Manifest Layer

Declarative platform description — the language OmniSim uses to communicate with the outside world. The Realizer translates a Manifest (static description) into a runtime Platform (Unit + Bus instances).

L3 knows the configuration description of a Unit but knows nothing of Backend implementation details. The Realizer looks up the corresponding factory function by the `kind` field in the ComponentRegistry and instantiates the Unit. This allows new components to be integrated simply by registering a factory, with no changes to the framework.

### 3.6  L4 — Compatibility Layer

Presents itself externally as a QEMU process, enabling libvirt / OpenStack to integrate transparently, with zero changes to existing operations tooling.

#### CLI Parser

Parses QEMU command-line argument format and translates it into an internal Manifest. `-machine` / `-cpu` / `-device` arguments generated by libvirt can be passed directly to the `omnisim-aarch64` / `omnisim-x86_64` executables (whose names are disguised as `qemu-system-*`).

#### QMP Server

Implements the subset of QMP commands required by libvirt: lifecycle (query-status / stop / cont / system_reset), hot-plug (device_add / device_del → L2 dynamic Unit add/remove), migration (serialized Manifest + Unit state), and queries (query-cpus / query-pci).

## 4  Key Design Decisions

This chapter records each important design decision and its rationale. These decisions define OmniSim's boundaries and trade-offs.

### 4.1  Bus Is a First-Class Citizen, Not Hidden Inside Units

```
Decision
Bus is an independent object, declared explicitly in the Manifest, queryable at runtime, and serializable.
Units are not allowed to communicate directly via private references.
```

Rationale: Making connections explicit is the foundation of the entire composition framework. Only when connections are first-class citizens can we achieve: Manifest serialization (for migration and cloud scheduling), hot-plug (dynamically adding/removing Units), observability (querying connection topology during debugging), and cross-Composite-Unit boundary connections.

### 4.2  CPUs and NPUs/DPUs/GPUs Are Fully Equal in the Framework

```
Decision
The Unit trait is unified for all hardware units; the framework has no CPU-specific path.
A CPU is simply a Unit that happens to implement ExecutionBackend.
```

Rationale: This is the core requirement for heterogeneous platform simulation. If the framework granted CPUs special privilege, GPU SM arrays, DPU network processing cores, and other "non-traditional CPU" execution units could not be expressed elegantly. With a unified Unit trait, the scheduler treats all executable Units identically, and Backend replacement is transparent to the framework.

### 4.3  Manifest Is Declarative, Not Imperative

```
Decision
Platform description is data (TOML/JSON), not code. The Realizer is responsible for translating data into runtime instances.
```

Rationale: Declarative description makes Manifest version control, diffing, auditing, and automated tooling possible. Keeping the platform description as data (rather than code) allows external tools to analyze topology and make decisions without running a simulation.

### 4.4  Backend × TimingModel Are Orthogonal

```
Decision
Execution dimension (how to execute) and timing dimension (how time advances) are completely independent; any combination is valid.
```

Rationale: Different scenarios have independently varying requirements for execution and timing. Software correctness verification needs only Functional + Untimed (fastest). Performance modeling needs Timing + LooseTiming. RTL co-simulation needs Rtl + CycleAccurate. If the two are coupled, every new combination requires a new Backend type, causing combinatorial explosion.

### 4.5  omnisim-core Has Zero Forced Dependencies

```
Decision
omnisim-core depends only on the Rust standard library; no third-party crates are introduced.
All components depend only on omnisim-core; they do not depend on each other.
```

Rationale: Ensures components can be compiled, tested, and released independently. If core introduces heavy dependencies, all components will be contaminated by transitive dependencies. Zero dependencies also keeps omnisim-core's interface definitions stable enough to avoid breaking changes caused by upstream dependency upgrades.

### 4.6  Composite Unit Recursive Nesting

```
Decision
A Unit can contain sub_units, with unlimited nesting depth; interior and exterior interact through boundary Ports.
```

Rationale: Hardware is inherently a recursive structure. A GPGPU card contains an ARM ctrl, an SM array, and an HBM controller, with independent internal buses between them. Without recursion support, either GPGPU would be flattened into a pile of Leaf Units (losing internal structure), or a special "Card" concept would be needed (breaking the uniformity of Unit). Composite Unit recursion is the most natural solution.

### 4.7  Ecosystem Compatibility Over Purity

```
Decision
The L4 compatibility layer fully implements the QEMU CLI and QMP protocol, even if this makes L4 code complex.
```

Rationale: The existing libvirt / OpenStack ecosystem is a legacy asset; users cannot be asked to rewrite their operations tooling. The complexity of the compatibility layer is encapsulated in L4 and does not pollute the design of L1–L3 — an acceptable trade-off.

## 5  Evolution Path

### 5.1  Milestone Plan

| Milestone | Goal | Key Deliverables |
| --- | --- | --- |
| M0 Architecture | omnisim-core interfaces finalized | Unit/Bus/Backend traits; MockBackend; two heterogeneous Units instantiated in the same Platform |
| M1 Functional Simulation | Minimal runnable simulation | Ported QEMU TCG; ARM + RISC-V frontends; L3 Manifest loading; minimal EmbeddedProfile |
| M2 Ecosystem Integration | libvirt-callable | L4 CLI Parser; core QMP Server subset; KvmBackend |
| M3 Heterogeneous Platform | Scenarios 1 and 2 working | PCIe Bus; GPGPU Composite Unit; ResetBus; MemBus visible_to filtering |
| M4 Performance Simulation | Timing Backend | TimingBackend + LooseTiming; SyncBus timing coordination; Cache/DDR performance modeling |
| M5 RTL Co-simulation | RTL co-verification | RtlBackend + CycleAccurate; FPGA bridge driver; mixed-precision co-simulation |
| M6 Security Extensions | TrustZone / confidential virtualization | EL2/EL3 TCG frontend; MemSecurity checks; ARM CCA RMM; AMD SEV-SNP PSP |

### 5.2  Interface Stability Commitments per Phase

```
Interface Stability Rules
P0: omnisim-core interfaces are non-breaking after 1.0; only additions are allowed
P1: Timing Backend interfaces; stability commitment follows milestone progress
P2: RTL/security interfaces; versioned independently
Cross-layer rule: upper-layer changes must not affect lower-layer interfaces; lower-layer interface changes require synchronized updates to upper layers
```
