> [中文版](./02-interface-spec.zh-CN.md)

---

# OmniSim — Interface Specification

> **Document ID**: DOC-02 &nbsp;&nbsp; **Version**: v1.0 &nbsp;&nbsp; **Date**: 2026-Q1 &nbsp;&nbsp; **Author**: OpenFunTech

---

| Document ID | DOC-02 |
| --- | --- |
| Version | v1.0 |
| Audience | Framework developers (implementing omnisim-core) · Component developers (depending on interfaces) |
| Prerequisites | DOC-01 Architecture Design |
| Related Docs | DOC-03 Component Dev Guide · DOC-04 Use Case Handbook |
| Last Updated | 2026-Q1 |

## 1  omnisim-core Interface Definitions

```
omnisim-core Design Conventions
Zero business dependencies (only std + thiserror + bitflags are permitted)
All traits require Send + Sync + 'static
All interface methods must not panic; errors are returned via OmniResult
This crate is the sole shared dependency for all other crates
```

### 1.1  ExecutionBackend — Execution Backend

The unified interface for all hardware units that "do work". CPUs, NPUs, DPUs, and DMA engines all implement this trait; the framework knows nothing about what happens internally.

```rust
// omnisim-core/src/backend/execution.rs
```

```
/// Core execution backend trait
/// The framework drives Unit execution only through this interface
pub trait ExecutionBackend: Send + Sync {
    /// Drive execution with a given time budget (nanoseconds)
    /// Return value: actual simulation time consumed (nanoseconds)
    ///   - return value <= budget_ns (must not exceed budget)
    ///   - return 0: nothing to do right now (Unit is waiting for an external event)
    ///   - return ExecFault: an exception that requires upper-layer handling
    fn exec(&mut self, budget_ns: u64) -> OmniResult<u64>;

    /// Reset to initial state
    /// Must be idempotent: multiple calls yield the same result
    fn reset(&mut self) -> OmniResult<()>;

    /// Save execution state (for migration / snapshot)
    fn save_state(&self) -> OmniResult<Vec<u8>>;

    /// Restore execution state
    fn load_state(&mut self, data: &[u8]) -> OmniResult<()>;

    /// Backend type name (for debugging)
    fn backend_name(&self) -> &'static str;

    /// Whether pause/resume is supported (required by KVM; TCG always supports it)
    fn supports_pause(&self) -> bool { true }
    fn pause(&mut self)  -> OmniResult<()> { Ok(()) }
    fn resume(&mut self) -> OmniResult<()> { Ok(()) }
}

/// Debug interface (optional — not all Backends implement this)
pub trait DebugBackend: ExecutionBackend {
    /// Read a register (reg_id defined by each Backend)
    fn read_reg(&self, reg_id: u64) -> OmniResult<u64>;
    fn write_reg(&mut self, reg_id: u64, val: u64) -> OmniResult<()>;

    /// Read the current PC
    fn pc(&self) -> u64;

    /// Insert a software breakpoint
    fn insert_breakpoint(&mut self, addr: u64) -> OmniResult<()>;
    fn remove_breakpoint(&mut self, addr: u64) -> OmniResult<()>;

    /// Single-step (execute one instruction and return)
    fn single_step(&mut self) -> OmniResult<StepResult>;
}

#[derive(Debug)]
pub enum StepResult {
    Ok { pc_after: u64, insn_bytes: Vec<u8> },
    Fault(FaultKind),
}
```

### 1.2  TimingModel — Timing Model

Orthogonal to ExecutionBackend. Describes how a Unit participates in global time advancement; it is the foundation for SyncBus to coordinate mixed-fidelity simulation.

```rust
// omnisim-core/src/backend/timing.rs
```

```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimingKind {
    /// Does not care about time; requests are handled immediately when exec() is called
    /// Suitable for: functional simulation CPUs, UARTs, simple peripherals
    Untimed,

    /// Transaction-level timing; exec() processes a batch of transactions and reports time consumed
    /// Suitable for: Cache, DDR, NPU performance models
    LooseTiming,

    /// Cycle-accurate; must synchronize with SyncBus every clock cycle
    /// Suitable for: RTL on FPGA — acts as the SyncBus timekeeper
    CycleAccurate,
}

pub trait TimingModel: Send + Sync {
    fn timing_kind(&self) -> TimingKind;

    /// Current simulation time (nanoseconds)
    fn now_ns(&self) -> u64;

    /// Called by SyncBus: please advance to target_ns
    /// Untimed: directly update internal clock
    /// LooseTiming: process backlogged transactions, update clock, return actual time reached
    /// CycleAccurate: drive FPGA to the corresponding cycle count
    fn advance_to(&mut self, target_ns: u64) -> OmniResult<u64>;

    /// Time of the next event that SyncBus should attend to
    /// None means no pending events
    fn next_event_ns(&self) -> Option<u64>;

    /// Clock frequency (Hz); CycleAccurate must implement this
    fn clock_hz(&self) -> Option<u64> { None }
}
```

### 1.3  MemTarget / MemSpace — Memory Subsystem

```rust
// omnisim-core/src/mem.rs
```

```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemAccessSize { U8 = 1, U16 = 2, U32 = 4, U64 = 8, U128 = 16 }

#[derive(Debug, Clone, Copy)]
pub struct MemAccess {
    pub addr:        u64,
    pub size:        MemAccessSize,
    pub is_write:    bool,
    /// Initiator Unit id (used for MemBus visible_to filtering)
    pub initiator_id: u32,
}

/// A unit accessible by address
/// Must be thread-safe; read must not modify observable state (pure-read semantics)
pub trait MemTarget: Send + Sync {
    /// Unique ID assigned by MemBus at realize time
    fn target_id(&self) -> u32;

    /// Address range covered by this Target ([base, base+size))
    fn base(&self) -> u64;
    fn size(&self) -> u64;

    fn read (&self,  offset: u64, size: MemAccessSize) -> OmniResult<u64>;
    fn write(&self,  offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()>;

    /// Bulk access (DMA / TCG block operation optimized path)
    fn read_bulk (&self,  offset: u64, buf: &mut [u8]) -> OmniResult<()> {
        // Default implementation: byte-by-byte calls to read()
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.read(offset + i as u64, MemAccessSize::U8)? as u8;
        }
        Ok(())
    }
    fn write_bulk(&self, offset: u64, buf: &[u8]) -> OmniResult<()> {
        for (i, b) in buf.iter().enumerate() {
            self.write(offset + i as u64, MemAccessSize::U8, *b as u64)?;
        }
        Ok(())
    }

    /// Direct memory access (RAM optimization: return a pointer to avoid copying)
    /// Only RAM-class Targets implement this method
    fn as_raw_ptr(&self) -> Option<*mut u8> { None }
}

/// Address space: routing table for MemTargets
/// Each Initiator Unit holds its own independent MemSpace instance
/// The same MemTarget can be registered in multiple MemSpaces (shared memory)
pub struct MemSpace {
    id:         String,
    mappings:   Vec<Mapping>,
    /// Address translation hook (MMU integration point)
    translator: Option<Box<dyn AddressTranslator>>,
}

struct Mapping {
    target:     Arc<dyn MemTarget>,
    host_base:  u64,    // base address in this MemSpace
    guest_base: u64,    // starting offset within the Target
    size:       u64,
    attrs:      MemAttrs,
}

#[derive(Clone, Copy, Default)]
pub struct MemAttrs {
    pub readable:   bool,
    pub writable:   bool,
    pub executable: bool,
    pub cacheable:  bool,
}

impl MemSpace {
    pub fn new(id: impl Into<String>) -> Self;

    /// Register a MemTarget
    pub fn map(
        &mut self,
        target:    Arc<dyn MemTarget>,
        host_base: u64,
        attrs:     MemAttrs,
    ) -> OmniResult<()>;

    /// Unregister
    pub fn unmap(&mut self, host_base: u64) -> OmniResult<()>;

    /// Physical address read/write (with translator conversion)
    pub fn read (&self, addr: u64, size: MemAccessSize, access: &MemAccess) -> OmniResult<u64>;
    pub fn write(&self, addr: u64, size: MemAccessSize, val: u64, access: &MemAccess) -> OmniResult<()>;

    /// Install an MMU/IOMMU address translator
    pub fn set_translator(&mut self, t: Box<dyn AddressTranslator>);

    /// Query address mapping (for debugging)
    pub fn query_mapping(&self, addr: u64) -> Option<MappingInfo>;
}

/// MMU/IOMMU address translation interface
pub trait AddressTranslator: Send + Sync {
    /// Virtual address → Physical address
    /// Returns None if address translation fails (page fault)
    fn translate(&self, va: u64, access: &MemAccess) -> OmniResult<u64>;

    /// TLB invalidation notification (called when CR3 / SATP is written)
    fn flush_tlb(&mut self, asid: Option<u64>);
}
```

### 1.4  IrqLine / IrqSink / IrqSource — Interrupt Subsystem

```rust
// omnisim-core/src/irq.rs
```

```
/// Interrupt receive side
pub trait IrqSink: Send + Sync {
    /// Assert interrupt line (level-triggered: active high)
    fn raise(&self, irq_num: u32) -> OmniResult<()>;
    /// Deassert interrupt line
    fn lower(&self, irq_num: u32) -> OmniResult<()>;
    /// Edge-triggered (positive pulse)
    fn pulse(&self, irq_num: u32) -> OmniResult<()> {
        self.raise(irq_num)?;
        self.lower(irq_num)
    }
    /// Range of interrupt numbers accepted by this Sink
    fn irq_count(&self) -> u32;
}

/// Interrupt send side (holds a reference to a Sink)
#[derive(Clone)]
pub struct IrqLine {
    sink:    Arc<dyn IrqSink>,
    irq_num: u32,
}

impl IrqLine {
    pub fn new(sink: Arc<dyn IrqSink>, irq_num: u32) -> Self;
    pub fn raise(&self) -> OmniResult<()> { self.sink.raise(self.irq_num) }
    pub fn lower(&self) -> OmniResult<()> { self.sink.lower(self.irq_num) }
    pub fn pulse(&self) -> OmniResult<()> { self.sink.pulse(self.irq_num) }
}

/// MSI descriptor (PCIe MSI/MSI-X)
#[derive(Debug, Clone)]
pub struct MsiDesc {
    pub addr:   u64,   // MSI write address
    pub data:   u32,   // MSI write data
    pub vector: u32,
}

/// MSI send interface (used by PCIe EPs)
pub trait MsiSender: Send + Sync {
    fn send_msi(&self, desc: &MsiDesc) -> OmniResult<()>;
}
```

### 1.5  Unit Trait — Core Composition Interface

The unified abstraction for all hardware entities. ARM CPUs, NPUs, DPUs, and UARTs are fully equal at the framework level through this trait.

```rust
// omnisim-core/src/unit.rs
```

```
/// Unit ID: globally unique, hierarchically named
/// Format: "soc0.arm_ss.core0", "gpgpu0.sm[3].alu"
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitId(String);

impl UnitId {
    pub fn new(s: impl Into<String>) -> Self;
    pub fn as_str(&self) -> &str;
    /// Parent ID: "soc0.arm_ss.core0" → Some("soc0.arm_ss")
    pub fn parent(&self) -> Option<UnitId>;
    /// Child ID: "soc0" + "arm_ss" → "soc0.arm_ss"
    pub fn child(&self, name: &str) -> UnitId;
}

/// Unit capability flags (bitmask)
bitflags::bitflags! {
    pub struct UnitCaps: u32 {
        /// Executable: implements ExecutionBackend
        const EXECUTABLE    = 0x01;
        /// Addressable: implements MemTarget (can be mounted on MemBus)
        const ADDRESSABLE   = 0x02;
        /// Interrupt source: can emit IrqLine
        const IRQ_SOURCE    = 0x04;
        /// Interrupt sink: implements IrqSink
        const IRQ_SINK      = 0x08;
        /// Composite unit: contains sub_units
        const COMPOSITE     = 0x10;
        /// Debuggable: implements DebugBackend
        const DEBUGGABLE    = 0x20;
        /// Supports snapshots
        const SNAPSHOTTABLE = 0x40;
    }
}

/// Unified interface for all hardware entities
pub trait Unit: Send + Sync {
    // ── Identity ────────────────────────────────────────────
    fn id(&self)   -> &UnitId;
    fn kind(&self) -> &'static str;
    fn caps(&self) -> UnitCaps;

    // ── Lifecycle ───────────────────────────────────────────
    /// realize: connect Buses, initialize internal state
    /// Called after all sub_unit realizes complete
    fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()>;

    /// unrealize: disconnect, release external resources
    fn unrealize(&mut self) -> OmniResult<()>;

    /// reset: reset to power-on initial state (connections preserved)
    fn reset(&mut self) -> OmniResult<()>;

    // ── Capability interfaces (return implementations matching caps()) ──
    fn execution_backend(&mut self)
        -> Option<&mut dyn ExecutionBackend> { None }

    fn timing_model(&mut self)
        -> Option<&mut dyn TimingModel> { None }

    fn as_mem_target(&self)
        -> Option<&dyn MemTarget> { None }

    fn as_irq_sink(&self)
        -> Option<Arc<dyn IrqSink>> { None }

    /// Descriptors of IrqLines this Unit can emit
    fn irq_source_descs(&self) -> &[IrqSourceDesc] { &[] }

    // ── Composite Unit ─────────────────────────────────────
    fn sub_units(&self)
        -> Option<&[Box<dyn Unit>]> { None }
    fn sub_units_mut(&mut self)
        -> Option<&mut Vec<Box<dyn Unit>>> { None }

    // ── State persistence ───────────────────────────────────
    fn save_snapshot(&self)  -> OmniResult<UnitSnapshot> {
        Err(OmniError::Internal("snapshot not supported".into()))
    }
    fn load_snapshot(&mut self, _snap: UnitSnapshot) -> OmniResult<()> {
        Err(OmniError::Internal("snapshot not supported".into()))
    }

    // ── Property system (for QMP queries) ──────────────────
    fn get_property(&self, _name: &str) -> Option<PropertyValue> { None }
    fn set_property(&mut self, _name: &str, _val: PropertyValue)
        -> OmniResult<()> {
        Err(OmniError::Internal("property not supported".into()))
    }
}

/// realize context: provides Bus connectivity capabilities
pub struct RealizeCtx<'a> {
    /// MemSpace for this Unit (in Initiator role)
    pub mem_space:  Option<&'a mut MemSpace>,
    /// Global IrqRouter (for connecting interrupt lines)
    pub irq_router: &'a mut IrqRouter,
    /// Configuration parameters for this Unit
    pub config:     &'a UnitConfig,
    /// Parent Unit ID (used when realizing inside a Composite Unit)
    pub parent_id:  Option<&'a UnitId>,
    /// Global Unit registry (for looking up other Units)
    pub unit_reg:   &'a UnitRegistry,
}

#[derive(Debug, Clone)]
pub struct IrqSourceDesc {
    /// Name within the Unit, e.g. "irq_out", "msi[0]"
    pub name: String,
    pub kind: IrqKind,
}

#[derive(Debug, Clone, Copy)]
pub enum IrqKind { LevelSensitive, EdgeTriggered, Msi }
```

### 1.6  Dataflow Interface — High-Bandwidth Data Streams

Used for large data transfers between accelerators (NPU/DPU) and memory; kept separate from MemBus to avoid MMIO path overhead.

```rust
// omnisim-core/src/dataflow.rs
```

```
/// Data stream endpoint (producer or consumer)
pub trait DataEndpoint: Send + Sync {
    fn endpoint_id(&self) -> u32;

    /// Producer: submit a batch of data
    fn produce(&self, desc: DataDesc) -> OmniResult<DataToken>;

    /// Consumer: retrieve processing results
    fn consume(&self, token: DataToken) -> OmniResult<DataResult>;

    /// Poll: whether data/results are ready
    fn poll_ready(&self) -> bool;
}

#[derive(Debug)]
pub struct DataDesc {
    pub src_addr: u64,       // source address (in Initiator's MemSpace)
    pub dst_addr: u64,       // destination address
    pub len:      usize,
    pub op:       DataOp,    // operation type
    pub metadata: Vec<u8>,   // operator parameters (NPU) / packet header (DPU)
}

#[derive(Debug, Clone, Copy)]
pub enum DataOp {
    DmaRead, DmaWrite, DmaCopy,
    NpuInference,          // NPU operator inference
    DpuPacketProcess,      // DPU packet processing
    Custom(u32),
}
```

### 1.7  UnitRegistry — Global Unit Registry

```rust
// omnisim-core/src/registry.rs
```

```
/// Runtime Unit registry
/// Supports lookup by ID, by kind, and filtered by caps
pub struct UnitRegistry {
    units: HashMap<UnitId, Arc<Mutex<Box<dyn Unit>>>>,
}

impl UnitRegistry {
    pub fn register(&mut self, unit: Box<dyn Unit>) -> OmniResult<()>;
    pub fn unregister(&mut self, id: &UnitId) -> OmniResult<()>;

    pub fn get(&self, id: &UnitId)
        -> Option<Arc<Mutex<Box<dyn Unit>>>>;

    /// Find all Units implementing specific caps
    pub fn find_by_caps(&self, caps: UnitCaps)
        -> Vec<Arc<Mutex<Box<dyn Unit>>>>;

    /// Find all Units of a specific kind
    pub fn find_by_kind(&self, kind: &str)
        -> Vec<Arc<Mutex<Box<dyn Unit>>>>;

    /// Get all executable Units (used by the scheduler)
    pub fn executable_units(&self)
        -> Vec<Arc<Mutex<Box<dyn Unit>>>>;
}
```

### 1.8  Security Execution Interface (Reserved in P0, Implemented in P2)

To support TrustZone (Scenario 12) and confidential virtualization (Scenario 13, P2), security-related interfaces are reserved in omnisim-core. In P0 the interfaces exist but produce no actual isolation (no-ops); P2 implements real security domain checks.

```
P0/P2 Phased Notes
P0: enums and interfaces defined; MemBus security checks are no-ops; Units return Normal World by default
P1: TrustZone SMC/HVC path, EL2/EL3 switching (implemented in omnisim-tcg)
P2: MemBus actual security domain isolation checks; ARM CCA RMM; AMD SEV-SNP PSP
```

```rust
// omnisim-core/src/security.rs

/// Memory security attribute
/// Attached to MemAttrs; describes which security domain a memory page belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemSecurity {
    #[default]
    NonSecure,              // Normal memory (default)
    Secure,                 // TrustZone Secure World
    Realm,                  // ARM CCA Realm World
    EncryptedPrivate(u32),  // AMD SEV: ASID-bound encryption
    EncryptedShared,        // AMD SEV: shared encrypted memory
}

/// CPU security world state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityWorld {
    #[default]
    Normal,   // Non-Secure EL0/1/2 (default)
    Secure,   // TrustZone Secure EL0/1/3
    Realm,    // ARM CCA Realm EL0/1/2
    Root,     // ARM CCA Root EL3
}
```

#### Unit Trait Security Interface Extension

```rust
pub trait Unit: Send + Sync {
    // Existing methods unchanged...

    /// Current security world of this CPU (default Normal; non-CPU Units return Normal)
    fn security_world(&self) -> SecurityWorld {
        SecurityWorld::Normal
    }

    /// Switch security world (triggered by SMC/RMI calls)
    /// P0 default returns NotSupported; P1/P2 implemented in TCG frontend
    fn switch_world(&mut self, _target: SecurityWorld) -> OmniResult<()> {
        Err(OmniError::NotSupported("security world switch"))
    }
}
```

#### MemAttrs Extension

```rust
/// Memory access attributes (new security field added)
pub struct MemAttrs {
    pub readable:   bool,
    pub writable:   bool,
    pub executable: bool,
    /// Security attribute, defaults to NonSecure
    pub security:   MemSecurity,
}

impl Default for MemAttrs {
    fn default() -> Self {
        Self {
            readable:   true,
            writable:   true,
            executable: false,
            security:   MemSecurity::NonSecure,
        }
    }
}

// MemBus security check (no-op until P2)
impl MemBus {
    fn check_security(
        &self,
        _initiator_world: SecurityWorld,
        _target_security: MemSecurity,
    ) -> OmniResult<()> {
        Ok(())   // P2 implements real isolation checks
    }
}
```

### 1.9  CoherenceDomain — Cache Coherence Domain (P2 Interface Placeholder)

To support CXL.cache (Scenario 8, P2), cache coherence domain interfaces are reserved. CXL.cache allows devices to cache Host memory and requires coherence protocol (snoop/invalidate) support. Before P2, only the interface is defined; it is not connected to MemBus.

```rust
// omnisim-core/src/coherence.rs
// P2 interface placeholder: defines traits, not yet implemented

#[derive(Debug, Clone, Copy)]
pub enum CoherenceProtocol {
    Ace,    // ARM ACE / CHI
    Moesi,  // Intel MESIF / AMD MOESI
    CxlBi,  // CXL 3.0 Back-Invalidate
}

#[derive(Debug, Clone, Copy)]
pub enum SnoopOp { ReadShared, ReadUnique, Invalidate }

#[derive(Debug, Clone, Copy)]
pub enum SnoopResp { Hit, Miss, HitDirty }

#[derive(Debug, Clone, Copy)]
pub enum CacheOp { Read, Write, Flush }

/// Cache coherence participant (implemented by CXL devices, LLC, etc.)
pub trait CoherenceParticipant: Send + Sync {
    fn on_snoop(&self, addr: u64, op: SnoopOp) -> OmniResult<SnoopResp>;
    fn request(&self, addr: u64, op: CacheOp)  -> OmniResult<()>;
}

pub struct CoherenceDomain {
    pub id:       String,
    pub protocol: CoherenceProtocol,
    pub members:  Vec<UnitId>,
    // Before P2, members is empty; no actual effect
}
```

### 1.10  AddressTranslator Stage-2 Extension (P1)

To support nested virtualization (Scenario 11), AddressTranslator is extended with a Stage-2 address translation interface (IPA → PA). Stage-2 is controlled by the Hypervisor (EL2) to isolate the physical address spaces of different VMs.

```
Nested Simulation Architecture Notes
OmniSim simulates a complete physical machine (TCG); a real Hypervisor (KVM / LeoVirt-S) runs on the physical machine
The Hypervisor manages multiple VMs; Stage-2 MMU is the core mechanism for VM isolation
omnisim-core only needs to provide the interface; the concrete implementation is in omnisim-tcg's EL2/VMX frontend (P1)
```

```rust
// omnisim-core/src/mem.rs extension

pub trait AddressTranslator: Send + Sync {
    /// Stage-1: VA → IPA (Guest OS page tables, EL1-controlled)
    /// Existing interface; signature updated to add asid parameter
    fn translate_s1(
        &self,
        va:     u64,
        asid:   u16,
        access: MemAccessKind,
    ) -> OmniResult<u64>;

    /// Stage-2: IPA → PA (Hypervisor page tables, EL2-controlled)
    /// New in P1: vmid distinguishes the address spaces of different VMs
    fn translate_s2(
        &self,
        ipa:    u64,
        vmid:   u16,
        access: MemAccessKind,
    ) -> OmniResult<u64>;

    /// Two-stage cascade: VA → IPA → PA
    /// Normal memory access path for Guest OS; default implementation chains s1 + s2
    fn translate_s1s2(
        &self,
        va:     u64,
        asid:   u16,
        vmid:   u16,
        access: MemAccessKind,
    ) -> OmniResult<u64> {
        let ipa = self.translate_s1(va, asid, access)?;
        self.translate_s2(ipa, vmid, access)
    }

    /// TLB flush: called when a VM is destroyed or a VMID is reclaimed
    fn flush_tlb_vmid(&self, vmid: u16);

    /// TLB flush: called on process context switch
    fn flush_tlb_asid(&self, asid: u16);
}
```

#### PrivilegeLevel Enum (Reserved in P0)

```rust
/// CPU privilege level (defined in P0; used by TCG frontend in P1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
    // ARM
    EL0, EL1, EL2, EL3,
    // x86
    Ring0, Ring1, Ring2, Ring3,
    RingMinus1,   // VMX non-root (Guest)
    // RISC-V
    U, S, H, M,
}

pub trait Unit: Send + Sync {
    // Existing methods unchanged...

    /// Current privilege level of this CPU (non-CPU Units return None)
    fn current_privilege(&self) -> Option<PrivilegeLevel> { None }
}
```

## 2  Bus Interface Detailed Design

```
Bus Design Principles
Bus is the explicit expression of connection relationships — a first-class citizen, never buried inside Units
Each Bus type is independently implemented with a single responsibility
Buses support dynamically adding/removing connections at runtime (hot-plug)
Bus connections can cross Composite Unit boundaries
```

### 2.1  MemBus — Address Bus

Responsible for address routing: given an Initiator's access address, it finds the corresponding MemTarget and forwards the read/write operation. Supports partial visibility (`visible_to`) to implement address space isolation between subsystems.

```rust
// omnisim-core/src/bus/mem_bus.rs
```

```
pub struct MemBus {
    id:       String,
    mappings: Vec<MemMapping>,
    /// Address overlap detection: checked at realize time, skipped at runtime
    overlap_policy: OverlapPolicy,
}

struct MemMapping {
    target:      Arc<dyn MemTarget>,
    /// Base address of this mapping in the MemBus address space
    base:        u64,
    size:        u64,
    attrs:       MemAttrs,
    /// Access control: which Initiators can see this mapping
    /// None = all Initiators can see it
    /// Some([id1,id2]) = only specified Initiators can see it
    /// Used to implement: ITCM visible only to RISC-V Core
    ///                    GIC registers visible only to ARM Core
    visible_to:  Option<Vec<u32>>, // initiator_id
}

#[derive(Clone, Copy)]
pub enum OverlapPolicy {
    /// Error if overlap detected at realize time
    Strict,
    /// Allow overlap; later registrations shadow earlier ones
    Shadow,
}

impl MemBus {
    pub fn new(id: impl Into<String>) -> Self;

    /// Register a Target (realize phase)
    pub fn map(
        &mut self,
        target:     Arc<dyn MemTarget>,
        base:       u64,
        attrs:      MemAttrs,
        visible_to: Option<Vec<u32>>,
    ) -> OmniResult<()>;

    /// Dynamic unregister (hot-unplug)
    pub fn unmap(&mut self, base: u64) -> OmniResult<()>;

    /// Route and execute a read
    pub fn read(
        &self,
        addr:         u64,
        size:         MemAccessSize,
        initiator_id: u32,
    ) -> OmniResult<u64>;

    /// Route and execute a write
    pub fn write(
        &self,
        addr:         u64,
        size:         MemAccessSize,
        val:          u64,
        initiator_id: u32,
    ) -> OmniResult<()>;

    /// Query address mappings (debugging / QMP query-memmap)
    pub fn dump_mappings(&self, initiator_id: Option<u32>)
        -> Vec<MappingInfo>;

    /// Build an independent MemSpace view for an Initiator
    /// The MemSpace holds a reference to MemBus; reads/writes are routed through MemBus
    pub fn make_space(&self, initiator_id: u32) -> MemSpace;
}

#[derive(Debug)]
pub struct MappingInfo {
    pub base:        u64,
    pub size:        u64,
    pub target_kind: String,
    pub attrs:       MemAttrs,
    pub visible_to:  Option<Vec<u32>>,
}
```

### 2.2  IrqBus — Interrupt Bus

Manages interrupt line connections. Supports one-to-one, one-to-many (interrupt fan-out), and many-to-one (interrupt aggregation) connections.

```rust
// omnisim-core/src/bus/irq_bus.rs
```

```
pub struct IrqBus {
    id:          String,
    connections: Vec<IrqConnection>,
}

/// An interrupt connection: from source endpoint to sink endpoint
pub struct IrqConnection {
    id:     u32,
    source: IrqEndpoint,
    sink:   IrqEndpoint,
}

/// Interrupt line endpoint descriptor
#[derive(Debug, Clone)]
pub struct IrqEndpoint {
    /// Unit ID
    pub unit_id: UnitId,
    /// Interrupt signal name within the Unit
    /// Format: "irq_out", "msi[0]", "irq[33]"
    pub signal:  String,
}

impl IrqBus {
    pub fn new(id: impl Into<String>) -> Self;

    /// realize: resolve IrqEndpoints and establish actual IrqLine connections
    pub fn realize(&mut self, reg: &UnitRegistry) -> OmniResult<()>;

    /// Dynamically add a connection (for hot-plugged device interrupt connections)
    pub fn connect(
        &mut self,
        source: IrqEndpoint,
        sink:   IrqEndpoint,
        reg:    &UnitRegistry,
    ) -> OmniResult<u32>;  // returns connection_id

    /// Disconnect
    pub fn disconnect(&mut self, connection_id: u32) -> OmniResult<()>;

    /// Query all connections (for debugging)
    pub fn dump_connections(&self) -> Vec<IrqConnectionInfo>;
}
```

### 2.3  PcieBus — PCIe Bus

Handles PCIe topology: Root Complex to Endpoint connections, BAR address mapping, MSI routing, and configuration space access.

```rust
// omnisim-core/src/bus/pcie_bus.rs
```

```
pub struct PcieBus {
    id:           String,
    root_complex: UnitId,
    endpoints:    Vec<PcieEndpointEntry>,
    /// BAR mappings: EP BAR address → Host physical address
    bar_mappings: Vec<BarMapping>,
}

struct PcieEndpointEntry {
    unit_id:  UnitId,
    bus:      u8,
    device:   u8,
    function: u8,
    bars:     Vec<BarConfig>,
}

pub struct BarConfig {
    pub bar_index: u8,
    pub size:      u64,
    /// Mapping base address of this EP BAR in the Host address space
    pub host_base: u64,
    pub bar_kind:  BarKind,
}

#[derive(Clone, Copy)]
pub enum BarKind {
    Mem32, Mem64, Io,
}

/// Interface that PCIe EPs must implement (extension trait of EP Units)
pub trait PcieEndpoint: Unit {
    /// Configuration space read/write (BDF addressing)
    fn cfg_read (&self,  offset: u16, size: MemAccessSize) -> OmniResult<u64>;
    fn cfg_write(&self,  offset: u16, size: MemAccessSize, val: u64) -> OmniResult<()>;

    /// BAR window access (Host accesses EP internals via BAR)
    fn bar_read (&self,  bar: u8, offset: u64, size: MemAccessSize) -> OmniResult<u64>;
    fn bar_write(&self,  bar: u8, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()>;

    /// MSI/MSI-X send capability
    fn msi_sender(&self) -> Option<Arc<dyn MsiSender>>;

    /// Internal MemSpace of the EP (reached when Host writes via BAR into EP's internal bus)
    fn internal_mem_space(&mut self) -> &mut MemSpace;
}

/// Interface that PCIe RCs must implement
pub trait PcieRootComplex: Unit {
    /// ECAM configuration space access
    fn ecam_read (&self,  bdf: Bdf, offset: u16, size: MemAccessSize) -> OmniResult<u64>;
    fn ecam_write(&self,  bdf: Bdf, offset: u16, size: MemAccessSize, val: u64) -> OmniResult<()>;

    /// DMA Remapping (IOMMU)
    fn iommu(&self) -> Option<Arc<dyn AddressTranslator>>;
}

#[derive(Debug, Clone, Copy)]
pub struct Bdf { pub bus: u8, pub device: u8, pub function: u8 }
```

### 2.4  SyncBus — Timing Synchronization Bus

Coordinates global timing across different TimingModels in mixed-fidelity simulation, ensuring correct causality between LooseTiming and CycleAccurate Units.

```rust
// omnisim-core/src/bus/sync_bus.rs
```

```
pub struct SyncBus {
    id:           String,
    participants: Vec<SyncParticipant>,
    /// Global advance step size (nanoseconds)
    /// Each tick advances all LooseTiming Units to now + quantum
    time_quantum_ns: u64,
}

struct SyncParticipant {
    unit_id:       UnitId,
    timing_model:  Arc<Mutex<dyn TimingModel>>,
    /// true = CycleAccurate, acts as the timekeeper
    /// SyncBus uses the timekeeper's time as the reference
    is_timekeeper: bool,
}

impl SyncBus {
    pub fn new(id: impl Into<String>, quantum_ns: u64) -> Self;

    pub fn add_participant(
        &mut self,
        unit_id:       UnitId,
        timing_model:  Arc<Mutex<dyn TimingModel>>,
        is_timekeeper: bool,
    ) -> OmniResult<()>;

    /// Advance by one quantum step
    /// Call order:
    ///   1. Advance CycleAccurate timekeeper to target
    ///   2. Advance all LooseTiming Units to the same target
    ///   3. Untimed Units do not participate (driven directly by other Units' exec())
    pub fn tick(&mut self) -> OmniResult<SyncStats>;

    /// Current global simulation time
    pub fn global_time_ns(&self) -> u64;
}

#[derive(Debug)]
pub struct SyncStats {
    pub global_time_ns: u64,
    pub lagging_units:  Vec<UnitId>,  // Units that failed to keep up with timing
}
```

### 2.5  IrqRouter — Global Interrupt Router

Centrally manages interrupt connections across all IrqBuses, providing name-based lookup and dynamic connection capabilities.

```rust
// omnisim-core/src/bus/irq_router.rs (continued in §5.6)
```

```
pub struct IrqRouter {
    /// bus_id → IrqBus
    buses: HashMap<String, IrqBus>,
    /// (unit_id, signal_name) → IrqLine (cache, populated after realize)
    line_cache: HashMap<(UnitId, String), IrqLine>,
}

impl IrqRouter {
    /// Connect interrupt lines (realize phase)
    pub fn connect(
        &mut self,
        source: IrqEndpoint,
        sink:   IrqEndpoint,
        reg:    &UnitRegistry,
    ) -> OmniResult<()>;

    /// Get an IrqLine (called by Units at realize time to store in their own fields)
    pub fn get_line(
        &self,
        unit_id: &UnitId,
        signal:  &str,
    ) -> OmniResult<IrqLine>;

    /// Register an IrqSink (called by IrqSink Units at realize time)
    pub fn register_sink(
        &mut self,
        unit_id: UnitId,
        signal:  String,
        sink:    Arc<dyn IrqSink>,
    );
}
```

### 2.6  ResetBus — Reset Signal Bus

ResetBus is fully symmetrical with IrqBus and uniformly models all cross-Unit reset relationships. It covers two physical scenarios: PCIe PERST# (RC controls EP reset) and on-chip Reset Controller (controls multi-core reset sequences).

```
Design Principles
ResetBus models the connection relationship of reset signals; it does not care whether they are PCIe PERST# or on-chip GPIO
PcieBus automatically establishes ResetLines between the RC and all EPs at realize time; no manual configuration needed
On-chip scenario: Reset Controller Unit acts as reset_source; controlled cores act as reset_sink
```

```rust
// omnisim-core/src/bus/reset_bus.rs

/// Reset signal receive side (implemented by the Unit being reset)
pub trait ResetSink: Send + Sync {
    fn assert_reset(&self)   -> OmniResult<()>;
    fn deassert_reset(&self) -> OmniResult<()>;
    fn is_in_reset(&self)    -> bool;
}

/// One reset line (from source to sink)
pub struct ResetLine {
    pub sink:    Arc<dyn ResetSink>,
    pub line_id: String,
}

/// Ready signal: notifies waiters when a Unit has powered up and finished initialization
pub struct ReadySignal {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl ReadySignal {
    pub fn set(&self);
    pub fn wait(&self, timeout_ns: Option<u64>) -> OmniResult<()>;
}

pub struct ResetBus {
    pub id:          String,
    pub connections: Vec<ResetConnection>,
}

pub struct ResetConnection {
    pub from: UnitId,   // Reset Controller or PCIe RC (PERST#)
    pub to:   UnitId,   // Unit being reset
}

impl ResetBus {
    pub fn assert_all(&self)   -> OmniResult<()>;
    pub fn deassert(&self, target: &UnitId) -> OmniResult<()>;
    pub fn deassert_all(&self) -> OmniResult<()>;
}
```

#### Unit Trait Extension (ResetBus-related)

```rust
// New methods added to Unit trait (default implementations are None / empty slices)
pub trait Unit: Send + Sync {
    // Existing methods unchanged...

    /// Unit as a reset signal receive side
    fn reset_sink(&self) -> Option<Arc<dyn ResetSink>> { None }

    /// Unit as a reset signal source (describes which reset lines it can control)
    fn reset_source_descs(&self) -> &[ResetSourceDesc] { &[] }

    /// Ready signal: set() after the Unit completes initialization; BootSequencer can wait()
    fn ready_signal(&self) -> Option<Arc<ReadySignal>> { None }
}

pub struct ResetSourceDesc {
    pub signal_name: String,   // e.g. "perst#" / "riscv_reset"
}
```

#### BootStep New Variants

```rust
pub enum BootStep {
    // Existing variants unchanged...
    WriteReg  { unit: String, reg: String, val: String },
    WaitIrq   { unit: String, irq: String },

    // New: reset control
    AssertReset   { bus: String },
    DeassertReset { bus: String },

    // New: wait for PCIe link to come up (after PERST# is deasserted)
    WaitLinksUp { bus: String, timeout_ns: Option<u64> },

    // New: wait for Unit ready signal (firmware initialization complete)
    WaitReady   { unit: String, timeout_ns: Option<u64> },
}
```

#### Manifest Description Example

```toml
# PCIe scenario: PERST# controls GPGPU reset
[[buses]]
kind = "reset"
id   = "pcie-reset-bus"
[[buses.connections]]
from = "host.pcie-rc.perst#"
to   = "gpgpu0.reset_in"

# On-chip scenario: Reset Controller controls RISC-V core
[[buses]]
kind = "reset"
id   = "soc-reset-bus"
[[buses.connections]]
from = "soc.reset-ctrl.riscv_reset"
to   = "soc.riscv-ss.rv64-core.reset_in"

# BootDesc usage example
[[boot.sequence]]
action = "deassert_reset"
bus    = "pcie-reset-bus"

[[boot.sequence]]
action     = "wait_links_up"
bus        = "pcie-reset-bus"
timeout_ns = 10_000_000_000

[[boot.sequence]]
action     = "wait_ready"
unit       = "gpgpu0.arm-ctrl"
timeout_ns = 5_000_000_000
```

### 2.7  PowerBus — Power Domain Management Bus

PowerBus is fully symmetrical with ResetBus and uniformly models power domain management in embedded SoCs. The Power Controller controls power on/off and low-power state transitions for compute cores via PowerBus.

```
Applicable Scenarios
Embedded SoC (Scenario 5): ARM main core dynamically manages power domains for DSP, NPU, and RISC-V small core
Power-sensitive devices: verifying DVFS / CPUIdle / runtime power management (RTOS/Linux PM)
Server scenarios: PowerBus is typically not needed (components do not implement power_sink; framework ignores)
```

```rust
// omnisim-core/src/bus/power_bus.rs

/// Power state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    On,
    Off,
    /// Powered off but register state preserved (low-power standby)
    Retention,
}

/// Power signal receive side (implemented by the controlled Unit)
pub trait PowerSink: Send + Sync {
    fn power_on(&self)        -> OmniResult<()>;
    fn power_off(&self)       -> OmniResult<()>;
    fn enter_retention(&self) -> OmniResult<()>;
    fn power_state(&self)     -> PowerState;
}

pub struct PowerBus {
    pub id:          String,
    pub connections: Vec<PowerConnection>,
}

pub struct PowerConnection {
    pub controller: UnitId,   // Power Controller Unit
    pub target:     UnitId,   // Controlled Unit
    pub domain:     String,   // Power domain name, e.g. "PD_NPU"
}

impl PowerBus {
    pub fn power_on(&self, target: &UnitId)        -> OmniResult<()>;
    pub fn power_off(&self, target: &UnitId)       -> OmniResult<()>;
    pub fn enter_retention(&self, target: &UnitId) -> OmniResult<()>;
    pub fn state(&self, target: &UnitId)           -> OmniResult<PowerState>;
}
```

#### Unit Trait Extension (PowerBus-related)

```rust
pub trait Unit: Send + Sync {
    // Existing methods unchanged...

    /// Unit as a power signal receive side (default None = power management not supported)
    fn power_sink(&self) -> Option<Arc<dyn PowerSink>> { None }
}
```

#### Manifest Description Example

```toml
[[buses]]
kind = "power"
id   = "soc-power-bus"

[[buses.connections]]
controller = "soc.power-ctrl"
target     = "soc.npu"
domain     = "PD_NPU"

[[buses.connections]]
controller = "soc.power-ctrl"
target     = "soc.dsp[0]"
domain     = "PD_DSP0"

[[buses.connections]]
controller = "soc.power-ctrl"
target     = "soc.riscv-core"
domain     = "PD_RISCV"
```

### 2.8  InterconnectBus — Peer-to-Peer Node Interconnect Bus

InterconnectBus models peer-to-peer interconnects between nodes, complementing PcieBus (which models hierarchical Host-Device relationships). It covers multi-card GPU interconnects, NUMA multi-socket servers, large-memory SoC compute cluster interconnects, and CXL memory devices.

```
InterconnectBus vs PcieBus
PcieBus: RC (Host) to EP (Device), with master-slave hierarchy; Host controls device discovery and reset
InterconnectBus: peer-to-peer between nodes, no master-slave; nodes can be GPU / CPU Socket / CXL device
Not mutually exclusive: a GPU card connects to Host via PcieBus and to other cards via InterconnectBus
```

```rust
// omnisim-core/src/bus/interconnect_bus.rs

/// Addressing mode: determines the fundamental semantics of memory access
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddrMode {
    /// Each node has an independent address space; nodes access each other via mapped windows
    /// Suitable for: PCIe P2P, NVLink-style, custom card-to-card interconnects
    Windowed,
    /// All nodes share a unified physical address space
    /// Suitable for: Infinity Fabric, NUMA, CXL.mem, on-chip NoC
    Unified,
}

/// Node type
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// CPU Socket / NUMA Cluster / Compute cluster
    CpuSocket,
    /// GPU / DPU and other accelerator cards
    Accelerator,
    /// CXL memory expansion device
    MemoryDevice,
}

pub struct InterconnectNode {
    pub unit_id:         UnitId,
    pub node_kind:       NodeKind,
    pub base_addr:       u64,
    pub size:            u64,
    /// Access control whitelist (None = all nodes can access)
    pub accessible_from: Option<Vec<UnitId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterconnectPerf {
    /// Bandwidth (GB/s); used in Timing simulation; can be omitted for functional simulation
    pub bandwidth_gbps: f64,
    /// One-way latency (ns); used in Timing simulation
    pub latency_ns:     u64,
}

pub struct InterconnectBus {
    pub id:        String,
    pub addr_mode: AddrMode,
    pub nodes:     Vec<InterconnectNode>,
    pub perf:      Option<InterconnectPerf>,
    /// Pluggable protocol backend
    pub backend:   Box<dyn InterconnectBackend>,
}
```

#### InterconnectBackend — Pluggable Protocol Interface

```rust
/// Protocol backend interface: different interconnect protocols implement different Backends
pub trait InterconnectBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn read(
        &self,
        src:  &UnitId,
        dst:  &UnitId,
        addr: u64,
        buf:  &mut [u8],
    ) -> OmniResult<()>;

    fn write(
        &self,
        src:  &UnitId,
        dst:  &UnitId,
        addr: u64,
        buf:  &[u8],
    ) -> OmniResult<()>;

    /// Cross-node atomic operation (card-to-card barrier / atomic add)
    fn atomic(
        &self,
        src:  &UnitId,
        dst:  &UnitId,
        addr: u64,
        op:   AtomicOp,
    ) -> OmniResult<u64>;
}

/// PCIe P2P implementation: routed through PcieBus
pub struct PciePeerBackend {
    pcie_bus: Arc<PcieBus>,
}

/// High-speed direct-connect implementation: NVLink / Infinity Fabric / on-chip NoC / custom
/// Functional simulation semantics are identical; differences are only in perf parameters (bandwidth/latency)
pub struct DirectFabricBackend {
    shared_space: Arc<MemBus>,
}

impl InterconnectBackend for PciePeerBackend    { /* ... */ }
impl InterconnectBackend for DirectFabricBackend { /* ... */ }
```

#### Manifest Description Example

```toml
# Multi-card GPU interconnect: NVLink-style (Windowed, high-speed private)
[[buses]]
kind = "interconnect"
id   = "gpu-fabric"
[buses.config]
addr_mode      = "windowed"
bandwidth_gbps = 600
latency_ns     = 100
backend        = "direct_fabric"
[[buses.nodes]]
unit      = "gpgpu0"
node_kind = "accelerator"
base_addr = "0x0000000000000000"
size      = "0x400000000"   # 16GB VRAM
[[buses.nodes]]
unit      = "gpgpu1"
node_kind = "accelerator"
base_addr = "0x0000000400000000"
size      = "0x400000000"

# NUMA multi-socket server (Unified, single address space)
[[buses]]
kind = "interconnect"
id   = "upi-fabric"
[buses.config]
addr_mode      = "unified"
bandwidth_gbps = 400
latency_ns     = 150
backend        = "direct_fabric"
[[buses.nodes]]
unit      = "socket0"
node_kind = "cpu_socket"
base_addr = "0x0000000000000000"
size      = "0x4000000000"   # 256GB
[[buses.nodes]]
unit      = "socket1"
node_kind = "cpu_socket"
base_addr = "0x4000000000000"
size      = "0x4000000000"

# Large-memory SoC on-chip NoC (Unified, low latency)
[[buses]]
kind = "interconnect"
id   = "noc-fabric"
[buses.config]
addr_mode      = "unified"
bandwidth_gbps = 2000
latency_ns     = 30
backend        = "direct_fabric"
[[buses.nodes]]
unit      = "soc.cluster0"
node_kind = "cpu_socket"
base_addr = "0x000000000000"
size      = "0x800000000"   # 32GB HBM
[[buses.nodes]]
unit      = "soc.cluster1"
node_kind = "cpu_socket"
base_addr = "0x008000000000"
size      = "0x800000000"
```

### 2.9  PcieEndpoint Reverse DMA Extension

The DPU scenario (Scenario 3) introduces the need for an EP to proactively access Host memory: the DPU's DMA engine needs to read/write virtio rings in Host memory directly, without going through BAR registers. This is EP→RC direction DMA, the reverse of the usual RC→EP direction.

```rust
// omnisim-core/src/bus/pcie_bus.rs extension

pub trait PcieEndpoint: Unit {
    // Existing interfaces unchanged...
    fn bar_descs(&self)      -> &[BarDesc];
    fn on_bar_write(&self, bar_idx: u8, offset: u64, data: &[u8]) -> OmniResult<()>;
    fn on_bar_read(&self, bar_idx: u8, offset: u64, buf: &mut [u8]) -> OmniResult<()>;

    // New: EP proactively accesses Host memory (reverse DMA)
    /// Read data from Host memory (DPU reads vring avail ring)
    fn dma_read(
        &self,
        host_addr: u64,
        buf:       &mut [u8],
    ) -> OmniResult<()>;

    /// Write data to Host memory (DPU writes vring used ring)
    fn dma_write(
        &self,
        host_addr: u64,
        buf:       &[u8],
    ) -> OmniResult<()>;
}

impl PcieBus {
    /// EP initiates a reverse DMA read (internally subject to IOMMU permission check)
    pub fn ep_dma_read(
        &self,
        ep_id:     &UnitId,
        host_addr: u64,
        buf:       &mut [u8],
    ) -> OmniResult<()> {
        if let Some(iommu) = &self.iommu {
            iommu.check_dma_permission(ep_id, host_addr, buf.len())?;
        }
        self.host_mem_bus.read_bulk(host_addr, buf)
    }

    pub fn ep_dma_write(
        &self,
        ep_id:     &UnitId,
        host_addr: u64,
        buf:       &[u8],
    ) -> OmniResult<()>;
}
```

### 2.10  NetworkBackend — External Network Access Interface

NetworkBackend provides a unified external network access interface for DPU nvme-of initiators, network processors, and similar components, abstracting away the differences between host sockets and simulated networks.

```rust
// omnisim-core/src/network.rs

pub trait NetworkBackend: Send + Sync {
    /// Send a packet to the external network
    fn send_packet(&self, pkt: Packet) -> OmniResult<()>;

    /// Register a packet receive callback
    fn on_receive(&self, cb: Box<dyn Fn(Packet) + Send + Sync>);

    /// RDMA read (nvme-of over RDMA)
    fn rdma_read(&self, addr: u64, buf: &mut [u8]) -> OmniResult<()>;

    /// RDMA write
    fn rdma_write(&self, addr: u64, buf: &[u8]) -> OmniResult<()>;
}

/// Host socket implementation
/// nvme-of / RDMA access to real remote storage servers
pub struct HostSocketBackend {
    fd: std::os::unix::io::RawFd,
}

/// Simulated network implementation
/// Connects to another simulated node's PacketBus (component layer)
pub struct SimNetworkBackend {
    // PacketBus reference, defined in the omnisim-dpu-core component layer
    tx: std::sync::mpsc::Sender<Packet>,
}

impl NetworkBackend for HostSocketBackend { /* ... */ }
impl NetworkBackend for SimNetworkBackend  { /* ... */ }
```

## 3  omnisim-manifest — L3 Platform Description Layer

```
L3 Layer Responsibilities
The sole responsibility of L3 is to translate a declarative Manifest (JSON/TOML) into runtime Unit + Bus instances (a Platform). L3 knows the configuration descriptions of Units, but knows nothing about Backend implementation details; the Realizer uses ComponentRegistry to locate factories.
```

### 3.1  Manifest Top-Level Structure

```rust
// omnisim-manifest/src/manifest.rs
```

```
/// Complete declarative description of a platform
/// Serializable to JSON / TOML; this is the target format for CLI translation and tooling
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub platform:  PlatformDesc,
    /// Template library (referenceable by UnitDesc.template; merged with config after expansion)
    #[serde(default)]
    pub templates: HashMap<String, UnitDesc>,
    pub units:     Vec<UnitDesc>,
    pub buses:     Vec<BusDesc>,
    #[serde(default)]
    pub boot:      BootDesc,
    /// Metadata (version, origin, signature)
    #[serde(default)]
    pub meta:      ManifestMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformDesc {
    pub name:        String,
    pub description: Option<String>,
    /// Target architecture (used by CLI for automatic parameter inference)
    pub arch:        Option<String>,   // "aarch64", "x86_64", "riscv64"
    /// Global simulation mode
    pub sim_mode:    SimMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum SimMode {
    #[default]
    Functional,   // Pure functional simulation (Untimed)
    Performance,  // Performance simulation (LooseTiming mixed)
    Rtl,          // RTL co-simulation (CycleAccurate mixed)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestMeta {
    pub version:         String,
    pub omnisim_version: String,
    pub created_by:      Option<String>,
    pub created_at:      Option<String>,
    /// SHA256 of content before this field
    pub signature:       Option<String>,
}
```

### 3.2  UnitDesc — Unit Declaration

```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDesc {
    pub id:   String,
    pub kind: String,      // component registry name, e.g. "arm-cortex-a78-core"

    /// Template reference: expand the template first, then override with fields in this desc
    #[serde(default)]
    pub template: Option<String>,

    // ── Backend configuration ────────────────────────────────
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default)]
    pub timing:  TimingKind,

    // ── Component-specific configuration (schema defined by each component) ──
    #[serde(default)]
    pub config: serde_json::Value,

    // ── Composite Unit sub-units ─────────────────────────────
    #[serde(default)]
    pub sub_units: Vec<UnitDesc>,

    // ── Count expansion: id = "sm[0..127]" → 128 instances sm0..sm127 ──
    /// ${index} variable available in each instance's config during expansion
    #[serde(default)]
    pub count: Option<CountSpec>,

    // ── Boot dependencies ────────────────────────────────────
    /// Units that must be realized before this Unit
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CountSpec {
    /// Fixed count: count: 128
    Fixed(u32),
    /// Range expansion: count: { from: 0, to: 127 }
    Range { from: u32, to: u32 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    #[default] Functional,
    Timing, Rtl, Kvm, Vhost, Mock,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TimingKind {
    #[default] Untimed,
    Loose,
    CycleAccurate,
}
```

### 3.3  BusDesc — Bus Declaration

```
/// All Bus types distinguished by tag
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusDesc {
    /// Address bus
    Mem {
        id:         String,
        /// Allow address overlap (shadow mode)
        #[serde(default)]
        overlap:    bool,
        /// List of Initiator Unit ids (who may initiate accesses)
        initiators: Vec<String>,
        targets:    Vec<MemTargetDesc>,
    },

    /// Interrupt bus
    Irq {
        id:          String,
        connections: Vec<IrqConnectionDesc>,
    },

    /// Dataflow bus (large transfers for NPU/DPU)
    Data {
        id:        String,
        producers: Vec<String>,
        consumers: Vec<String>,
    },

    /// Timing synchronization bus
    Sync {
        id:           String,
        participants: Vec<String>,
        /// Timing quantum step size (nanoseconds)
        #[serde(default = "default_quantum")]
        time_quantum_ns: u64,
    },

    /// PCIe bus
    Pcie {
        id:           String,
        root_complex: String,
        endpoints:    Vec<PcieEndpointDesc>,
        #[serde(default)]
        config:       PcieBusConfig,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemTargetDesc {
    /// Target Unit id
    pub unit:       String,
    /// Base address in this Bus's address space
    pub base:       HexU64,
    /// Override the Unit's default size (optional)
    pub size:       Option<HexU64>,
    /// Access attributes
    #[serde(default = "MemAttrs::rw")]
    pub attrs:      MemAttrsDesc,
    /// Access control: visible only to which Initiators
    pub visible_to: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrqConnectionDesc {
    /// "uart0.irq_out"
    pub from: String,
    /// "gicv3.gicd.irq[33]"
    pub to:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PcieEndpointDesc {
    pub unit:     String,
    pub bus:      u8,
    pub device:   u8,
    pub function: u8,
    pub bars:     Vec<BarDesc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarDesc {
    pub index:     u8,
    pub size:      HexU64,
    pub host_base: HexU64,
    #[serde(default)]
    pub kind:      BarKindDesc,
}

/// Hex u64 — accepts both "0x80000000" and 2147483648
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HexU64(#[serde(with = "hex_or_dec")] pub u64);
```

### 3.4  BootDesc — Boot Configuration

```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BootDesc {
    /// Boot sequence: realize + reset + exec each Unit in order
    /// If omitted, the Realizer auto-topologically-sorts by depends_on
    #[serde(default)]
    pub sequence: Vec<BootStep>,

    /// Kernel image
    pub kernel:   Option<ImageDesc>,
    /// initrd
    pub initrd:   Option<ImageDesc>,
    /// DTB
    pub dtb:      Option<ImageDesc>,
    /// Firmware (BIOS/UEFI/SBI)
    pub firmware: Option<ImageDesc>,

    /// Kernel command line
    pub cmdline:  Option<String>,

    /// Initial register values per Unit
    #[serde(default)]
    pub registers: HashMap<String, HashMap<String, HexU64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BootStep {
    /// Wait for a Unit to write a specific address (poll a flag bit)
    WaitWrite { unit: String, addr: HexU64, val: HexU64, mask: Option<HexU64> },
    /// Release the reset signal of a Unit
    ReleaseReset { unit: String },
    /// Delay N nanoseconds
    DelayNs { ns: u64 },
    /// Write a register
    WriteReg { unit: String, reg: String, val: HexU64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageDesc {
    pub path:      String,
    pub load_addr: Option<HexU64>,
    pub format:    ImageFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    #[default] Raw, Elf, Uimage, Pe,
}
```

### 3.5  Validator — Manifest Validation

Static validation is performed before realize to pinpoint errors to specific fields, avoiding problems that would only surface at runtime.

```rust
// omnisim-manifest/src/validator.rs
```

```
pub struct Validator {
    /// Known Unit kinds (obtained from ComponentRegistry)
    known_kinds: HashSet<String>,
}

impl Validator {
    pub fn validate(&self, m: &Manifest) -> ValidationReport;
}

#[derive(Debug)]
pub struct ValidationReport {
    pub errors:   Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Debug)]
pub struct ValidationError {
    pub path:    String,   // JSON pointer: "/units/2/config/size"
    pub message: String,
    pub kind:    ErrorKind,
}

#[derive(Debug)]
pub enum ErrorKind {
    UnknownUnitKind,
    DuplicateId,
    UnresolvedRef,         // Bus references a non-existent Unit id
    CircularDependency,
    MissingRequiredField,
    InvalidAddressRange,   // base + size overflows or overlaps
    IrqEndpointNotFound,
    TemplateNotFound,
    ConfigSchemaViolation, // does not conform to the component's JSON Schema
}

/// Validation rules (executed in order):
/// 1. All unit.id values are unique (including instances expanded from count)
/// 2. All unit.kind values are registered in ComponentRegistry
/// 3. All template references are resolvable
/// 4. All depends_on references exist and are acyclic
/// 5. All Unit ids referenced by Buses exist
/// 6. Mem Bus: address ranges do not overlap (unless overlap=true)
/// 7. Irq Bus: IrqEndpoint format is valid; Unit declares corresponding irq_source/sink
/// 8. PCIe: BAR addresses do not conflict with regular MemBus addresses
/// 9. Each UnitDesc.config conforms to the component's registered JSON Schema
```

### 3.6  Realizer — Manifest to Platform

```rust
// omnisim-manifest/src/realizer.rs
```

```
/// Component factory function type
pub type UnitFactory = Box<dyn Fn(&UnitDesc) -> OmniResult<Box<dyn Unit>> + Send + Sync>;

/// Component registry: kind → factory
pub struct ComponentRegistry {
    factories: HashMap<String, UnitFactory>,
    /// JSON Schema per kind (used by Validator)
    schemas:   HashMap<String, serde_json::Value>,
    /// Set of supported BackendKinds per kind
    backends:  HashMap<String, HashSet<BackendKind>>,
}

impl ComponentRegistry {
    pub fn new() -> Self;

    /// Register a component (called by each component crate's register() function)
    pub fn register(
        &mut self,
        kind:     &'static str,
        factory:  UnitFactory,
        schema:   Option<serde_json::Value>,
        backends: &[BackendKind],
    );

    pub fn known_kinds(&self) -> impl Iterator<Item = &str>;
    pub fn get_factory(&self, kind: &str) -> Option<&UnitFactory>;
    pub fn get_schema (&self, kind: &str) -> Option<&serde_json::Value>;
}

/// Realizer: converts a Manifest into a runtime Platform
pub struct Realizer {
    registry: Arc<ComponentRegistry>,
}

impl Realizer {
    pub fn new(registry: Arc<ComponentRegistry>) -> Self;

    /// Main entry point: validate + expand templates + instantiate all Units + connect all Buses
    pub fn realize(&self, manifest: &Manifest) -> OmniResult<Platform>;

    // ── Internal steps (pub for testing) ────────────────────

    /// Step 1: expand template references + count expansion
    pub fn expand(&self, manifest: &Manifest) -> OmniResult<Manifest>;

    /// Step 2: topological sort (by depends_on)
    pub fn topo_sort(units: &[UnitDesc]) -> OmniResult<Vec<usize>>;

    /// Step 3: instantiate Units in topological order
    pub fn instantiate(
        &self,
        units: &[UnitDesc],
        order: &[usize],
    ) -> OmniResult<Vec<Box<dyn Unit>>>;

    /// Step 4: realize each Unit (inject RealizeCtx)
    pub fn wire_units(
        units:      &mut Vec<Box<dyn Unit>>,
        buses:      &[BusDesc],
        irq_router: &mut IrqRouter,
        registry:   &UnitRegistry,
    ) -> OmniResult<()>;

    /// Step 5: build Bus instances and connect them
    pub fn build_buses(
        buses:    &[BusDesc],
        registry: &UnitRegistry,
    ) -> OmniResult<BusSet>;
}

/// Runtime platform: the product of realize
pub struct Platform {
    pub name:       String,
    pub unit_reg:   UnitRegistry,
    pub buses:      BusSet,
    pub irq_router: IrqRouter,
    pub boot:       BootDesc,
    /// Original Manifest (for incremental realize during hot-plug)
    pub manifest:   Manifest,
}

impl Platform {
    /// Hot-plug: realize new UnitDesc + BusDesc and attach to the runtime
    pub fn hot_add(
        &mut self,
        unit:     UnitDesc,
        buses:    Vec<BusDesc>,
        realizer: &Realizer,
    ) -> OmniResult<UnitId>;

    /// Hot-unplug: unrealize a Unit and remove it from Buses
    pub fn hot_remove(&mut self, unit_id: &UnitId) -> OmniResult<()>;

    /// Serialize current state (for migration)
    pub fn snapshot(&self) -> OmniResult<PlatformSnapshot>;
    pub fn restore(snap: PlatformSnapshot, realizer: &Realizer)
        -> OmniResult<Platform>;
}

pub struct BusSet {
    pub mem_buses:  HashMap<String, MemBus>,
    pub irq_buses:  HashMap<String, IrqBus>,
    pub data_buses: HashMap<String, DataBus>,
    pub sync_buses: HashMap<String, SyncBus>,
    pub pcie_buses: HashMap<String, PcieBus>,
}
```

### 3.7  Component Self-Registration Pattern

Each component crate provides a `register()` function, called at binary startup, achieving zero-intrusion registration:

```rust
// components/cpu/omnisim-cpu-arm/src/lib.rs
```

```
/// ARM CPU component registration entry point
/// Called in omnisim-runtime's main():
///   omnisim_cpu_arm::register(&mut registry);
pub fn register(reg: &mut ComponentRegistry) {
    reg.register(
        "arm-cortex-a78-core",
        Box::new(|desc| Ok(Box::new(CortexA78Core::from_desc(desc)?))),
        Some(serde_json::from_str(SCHEMA_JSON).unwrap()),
        &[BackendKind::Functional, BackendKind::Kvm],
    );
    reg.register(
        "arm-cortex-a55-core",
        Box::new(|desc| Ok(Box::new(CortexA55Core::from_desc(desc)?))),
        Some(serde_json::from_str(SCHEMA_JSON).unwrap()),
        &[BackendKind::Functional, BackendKind::Kvm],
    );
    // ... other ARM variants
}

static SCHEMA_JSON: &str = r#"{
    "type": "object",
    "properties": {
        "num_cores":  { "type": "integer", "minimum": 1, "maximum": 256 },
        "el2_enable": { "type": "boolean", "default": false },
        "sve_vl":     { "type": "integer", "enum": [128, 256, 512, 1024] }
    },
    "required": ["num_cores"]
}"#;
```

## 4  omnisim-runtime — Runtime and Scheduling Layer

```
Runtime Layer Responsibilities
omnisim-runtime is the "engine" of the entire simulation platform: it holds the Platform instance, drives exec() on all executable Units, coordinates SyncBus timing, handles VM exits, and responds to external control commands (pause / resume / snapshot).
```

### 4.1  Overall Execution Model

```
┌────────────────────────────────┐
│         Scheduler              │
│  ┌──────────────────────────┐  │
│  │   RunLoop (main thread)  │  │
│  │   - handle control cmds  │  │
│  │   - drive SyncBus tick   │  │
│  │   - monitor Unit threads │  │
│  └──────────────────────────┘  │
┌──────────┐      │  ┌─────┐ ┌─────┐ ┌─────────┐  │
│ QMP/CLI  │─────▶│  │cpu0 │ │cpu1 │ │ npu0    │  │
│ control  │      │  │thread│ │thread│ │ thread  │  │
└──────────┘      │  └─────┘ └─────┘ └─────────┘  │
│         UnitThread × N         │
└────────────────────────────────┘

Each executable Unit runs in its own dedicated thread (UnitThread)
RunLoop sends control commands (Pause/Resume/Reset) via channel
UnitThread reports events (VmExit/Fault/Halt) via channel
```

### 4.2  Scheduler

```rust
// omnisim-runtime/src/scheduler.rs
```

```
pub struct Scheduler {
    platform:     Arc<RwLock<Platform>>,
    unit_threads: HashMap<UnitId, UnitThreadHandle>,
    ctrl_tx:      mpsc::Sender<CtrlCmd>,
    event_rx:     mpsc::Receiver<UnitEvent>,
    state:        SchedulerState,
    config:       SchedulerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    Idle,       // realize complete, not yet started
    Running,    // executing normally
    Paused,     // paused (all Unit threads waiting)
    Stepping,   // single-step debug mode
    Terminated, // stopped
}

#[derive(Debug)]
pub struct SchedulerConfig {
    /// Time budget per exec() call per Unit (nanoseconds)
    /// Smaller = lower response latency, higher overhead
    pub exec_budget_ns: u64,        // default 1_000_000 (1 ms)
    /// SyncBus timing quantum (nanoseconds)
    pub sync_quantum_ns: u64,       // default 1_000_000
    /// Unit thread scheduling policy
    pub thread_policy: ThreadPolicy,
    /// Maximum VM exit retry count
    pub max_vm_exit_retries: u32,   // default 3
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ThreadPolicy {
    /// Each Unit gets its own OS thread (default)
    #[default] OneThreadPerUnit,
    /// All Units share a thread pool (suitable when Unit count is very large)
    ThreadPool { size: usize },
}

impl Scheduler {
    pub fn new(platform: Arc<RwLock<Platform>>, config: SchedulerConfig) -> Self;

    /// Start all Unit threads and enter Running state
    pub fn start(&mut self) -> OmniResult<()>;

    /// Pause all Units (waits for all threads to confirm pause before returning)
    pub fn pause(&mut self) -> OmniResult<()>;

    /// Resume execution
    pub fn resume(&mut self) -> OmniResult<()>;

    /// Reset all Units (pause → reset → resume)
    pub fn reset(&mut self) -> OmniResult<()>;

    /// Stop all Unit threads and enter Terminated state
    pub fn stop(&mut self) -> OmniResult<()>;

    /// Hot-plug: add a new Unit and start its thread
    pub fn hot_add_unit(&mut self, desc: UnitDesc) -> OmniResult<UnitId>;
    pub fn hot_remove_unit(&mut self, id: &UnitId) -> OmniResult<()>;

    /// RunLoop: core loop of the main control thread (started inside start())
    fn run_loop(&mut self);

    /// Handle events from Unit threads
    fn handle_event(&mut self, event: UnitEvent) -> OmniResult<()>;
}
```

### 4.3  UnitThread — Unit Execution Thread

```rust
// omnisim-runtime/src/unit_thread.rs
```

```
/// Each executable Unit corresponds to one UnitThread
pub struct UnitThread {
    unit_id:   UnitId,
    unit:      Arc<Mutex<Box<dyn Unit>>>,
    ctrl_rx:   mpsc::Receiver<CtrlCmd>,
    event_tx:  mpsc::Sender<UnitEvent>,
    budget_ns: u64,
}

impl UnitThread {
    /// Thread main function
    pub fn run(mut self) {
        loop {
            // Check control commands (non-blocking)
            match self.ctrl_rx.try_recv() {
                Ok(CtrlCmd::Pause)     => { self.wait_resume(); continue; }
                Ok(CtrlCmd::Reset)     => { self.do_reset(); }
                Ok(CtrlCmd::Terminate) => break,
                _                      => {}
            }

            // Drive execution
            let result = self.unit.lock().unwrap()
                .execution_backend()
                .map(|b| b.exec(self.budget_ns));

            match result {
                Some(Ok(consumed_ns)) => {
                    // Normal: report execution progress
                    self.event_tx.send(UnitEvent::Progress {
                        unit_id: self.unit_id.clone(),
                        consumed_ns,
                    }).ok();
                }
                Some(Err(OmniError::ExecFault { kind, pc })) => {
                    self.event_tx.send(UnitEvent::Fault {
                        unit_id: self.unit_id.clone(),
                        kind, pc,
                    }).ok();
                    self.wait_resume(); // wait for Scheduler to handle
                }
                None => {
                    // This Unit has no ExecutionBackend (leaf peripheral); thread exits
                    break;
                }
                _ => {}
            }
        }
    }
}

/// Control commands (Scheduler → UnitThread)
#[derive(Debug)]
pub enum CtrlCmd {
    Pause,
    Resume,
    Reset,
    Terminate,
    /// Single-step (debug mode)
    Step { count: u32 },
}

/// Events (UnitThread → Scheduler)
#[derive(Debug)]
pub enum UnitEvent {
    Progress { unit_id: UnitId, consumed_ns: u64 },
    Fault    { unit_id: UnitId, kind: FaultKind, pc: u64 },
    Halted   { unit_id: UnitId },
    VmExit   { unit_id: UnitId, exit: VmExitReason },
    StepDone { unit_id: UnitId, result: StepResult },
}

#[derive(Debug)]
pub enum VmExitReason {
    Mmio   { addr: u64, size: u8, is_write: bool, data: u64 },
    IoPort { port: u16, size: u8, is_write: bool, data: u32 },
    Shutdown,
    Debug,
    Hlt,
    Unknown(u64),
}
```

### 4.4  VM Exit Handling Flow

When a KVM Unit triggers a VM exit, the UnitThread reports the event and the RunLoop routes it to the corresponding MemTarget or IO handler:

```rust
// omnisim-runtime/src/vm_exit.rs
```

```
pub struct VmExitDispatcher {
    /// MMIO routing: queries MemBus directly
    mem_bus: Arc<MemBus>,
    /// IO port routing (x86)
    io_bus:  Arc<IoBus>,
}

impl VmExitDispatcher {
    pub fn dispatch(&self, exit: VmExitReason) -> OmniResult<DispatchResult>;
}

pub enum DispatchResult {
    /// Handled; execution may continue (return value written to Unit registers)
    Handled { data: Option<u64> },
    /// Requires upper-layer handling (e.g. Shutdown → Scheduler stops)
    Escalate(VmExitReason),
    /// Handling failed
    Error(OmniError),
}

// MMIO handling flow:
// 1. VmExit { MMIO, addr=0x09000000, is_write=false, size=1 }
// 2. VmExitDispatcher::dispatch()
// 3. mem_bus.read(0x09000000, U8, initiator_id=KVM_INITIATOR)
// 4. Routed to uart0.MemTarget (registered at realize time)
// 5. uart0.read(offset=0, U8) → returns rx register value
// 6. Written back to KVM vcpu's mmio data register
// 7. KVM_RUN continues
```

### 4.5  SimClock — Global Simulation Clock

```rust
// omnisim-runtime/src/clock.rs
```

```
/// Global monotonic simulation clock
/// Untimed mode:       incremented directly (advanced by exec budget)
/// LooseTiming mode:   advanced by SyncBus tick
/// CycleAccurate mode: driven by FPGA timekeeper
pub struct SimClock {
    now_ns: AtomicU64,
}

impl SimClock {
    pub fn now_ns(&self) -> u64 {
        self.now_ns.load(Ordering::Acquire)
    }

    /// Advance the clock (only Scheduler/SyncBus may call this)
    pub(crate) fn advance(&self, delta_ns: u64) -> u64 {
        self.now_ns.fetch_add(delta_ns, Ordering::AcqRel) + delta_ns
    }

    /// Register a clock event: callback fires at target_ns
    pub fn register_event(
        &self,
        target_ns: u64,
        cb: Box<dyn FnOnce(u64) + Send>,
    ) -> EventHandle;

    /// Cancel an event
    pub fn cancel_event(&self, handle: EventHandle);
}
```

### 4.6  BootSequencer — Boot Sequence Executor

```rust
// omnisim-runtime/src/boot.rs
```

```
/// Starts Units in order according to BootDesc.sequence
pub struct BootSequencer {
    desc:      BootDesc,
    platform:  Arc<RwLock<Platform>>,
    scheduler: Arc<Mutex<Scheduler>>,
}

impl BootSequencer {
    /// Execute the full boot sequence (called after Scheduler::start())
    pub fn run(&mut self) -> OmniResult<()>;

    /// Load an image file to the specified address
    pub fn load_image(&self, img: &ImageDesc) -> OmniResult<u64>;

    /// Execute a single BootStep
    fn execute_step(&mut self, step: &BootStep) -> OmniResult<()>;
}

// Boot sequence example (ARM host + RISC-V co-processor):
// 1. load_image(kernel) → 0x80000000
// 2. load_image(dtb)    → 0x84000000
// 3. WriteReg(cpu0, "x0", 0)            // arg0: hartid
// 4. WriteReg(cpu0, "x1", 0x84000000)   // arg1: dtb addr
// 5. WriteReg(cpu0, "pc", 0x80000000)   // entry point
// 6. WaitWrite(reset_ctrl, 0x0, 0x1)    // wait for ARM firmware to write release signal
// 7. ReleaseReset(riscv_ctrl)            // release RISC-V co-processor
```

## 5  omnisim-tcg — TCG Execution Engine Interface

```
TCG Boundary Design Principles
TCG is one ExecutionBackend implementation of omnisim-core — it is not the framework core
TCG accesses memory via the MemSpace interface; it holds no direct reference to MemBus
TCG emits interrupts via IrqLine; it never calls IrqSink directly
TCG IR is an internal implementation detail; it is not exposed to other crates
Ported from QEMU TCG, but interface boundaries are completely redefined
```

### 5.1  TcgEngine — TCG Engine Main Interface

```rust
// omnisim-tcg/src/engine.rs
```

```
/// TCG engine: translates and executes guest instructions
/// Implements ExecutionBackend + DebugBackend
pub struct TcgEngine<F: TcgFrontend, B: TcgHostBackend> {
    frontend:  F,                  // ISA frontend (ARM / RISC-V / x86)
    host_back: B,                  // host code generation backend (x86_64 / aarch64)
    tb_cache:  TbCache,            // Translation Block cache
    mem_space: Arc<MemSpace>,      // memory access entry point
    cpu_state: Box<dyn CpuState>,  // general-purpose register state
}

impl<F: TcgFrontend, B: TcgHostBackend> ExecutionBackend
for TcgEngine<F, B>
{
    fn exec(&mut self, budget_ns: u64) -> OmniResult<u64> {
        let budget_insns = self.ns_to_insns(budget_ns);
        let mut executed = 0u64;

        while executed < budget_insns {
            // 1. Look up TB cache
            let pc = self.cpu_state.pc();
            let tb = match self.tb_cache.lookup(pc) {
                Some(tb) => tb,
                None => {
                    // 2. Translate: guest instructions → TCG IR → host machine code
                    let ir   = self.frontend.translate(pc, &self.mem_space)?;
                    let code = self.host_back.compile(&ir)?;
                    self.tb_cache.insert(pc, code)
                }
            };
            // 3. Execute TB
            match tb.execute(&mut self.cpu_state, &self.mem_space)? {
                TbResult::Continue  => { executed += tb.insn_count(); }
                TbResult::Exit(r)   => { return self.handle_exit(r); }
                TbResult::Fault(f)  => {
                    return Err(OmniError::ExecFault {
                        kind: f, pc: self.cpu_state.pc(),
                    });
                }
            }
        }
        Ok(self.insns_to_ns(executed))
    }
}

#[derive(Debug)]
pub enum TbResult {
    /// TB executed normally; continue to the next TB
    Continue,
    /// TB encountered an event requiring exit from the translation loop
    Exit(TbExitReason),
    /// Exception / error
    Fault(FaultKind),
}

#[derive(Debug)]
pub enum TbExitReason {
    /// Interrupt encountered (IRQ / FIQ etc.)
    Interrupt,
    /// WFI/WFE (ARM) / WFI (RISC-V)
    WaitForInterrupt,
    /// System call / trap
    Syscall,
    /// TB ended (jump to next TB)
    ChainEnd { next_pc: u64 },
}
```

### 5.2  TcgFrontend Trait — ISA Frontend

```
/// ISA decoding + TCG IR generation
/// Each ISA implements one TcgFrontend
pub trait TcgFrontend: Send + Sync {
    type State: CpuState;

    /// Translate a Basic Block starting at pc → TCG IR
    /// Stops at branches / interrupt check-points / privileged instructions
    fn translate(
        &self,
        pc:        u64,
        mem_space: &MemSpace,
    ) -> OmniResult<TcgIrBlock>;

    /// ISA name (for debugging)
    fn isa_name(&self) -> &'static str;

    /// Maximum instruction byte length (used for TB cache key computation)
    fn max_insn_len(&self) -> usize;
}

/// Register state interface (opaque to TcgHostBackend)
pub trait CpuState: Send + Sync {
    fn pc(&self) -> u64;
    fn set_pc(&mut self, pc: u64);

    /// Read / write general-purpose registers (reg_idx defined by the ISA)
    fn read_reg(&self, idx: usize) -> u64;
    fn write_reg(&mut self, idx: usize, val: u64);

    /// Read / write system registers (sysreg_id defined by the ISA)
    fn read_sysreg(&self, id: u64) -> OmniResult<u64>;
    fn write_sysreg(&mut self, id: u64, val: u64) -> OmniResult<()>;

    /// Save / restore complete state (for snapshots)
    fn save(&self) -> Vec<u8>;
    fn load(&mut self, data: &[u8]) -> OmniResult<()>;
}
```

### 5.3  TcgHostBackend Trait — Host Code Generation Backend

```
/// TCG IR → host machine code
pub trait TcgHostBackend: Send + Sync {
    /// Compile a TCG IR Block → executable machine code
    fn compile(&self, ir: &TcgIrBlock) -> OmniResult<CompiledTb>;

    /// Host architecture name
    fn host_arch(&self) -> &'static str;

    /// Code buffer management
    fn alloc_code_buf(&self, size: usize) -> OmniResult<CodeBuffer>;
    fn free_code_buf(&self, buf: CodeBuffer);
}

/// A compiled Translation Block
pub struct CompiledTb {
    code_ptr:   *const u8,  // pointer to executable code
    code_size:  usize,
    insn_count: u32,        // number of corresponding guest instructions
    guest_pc:   u64,        // corresponding guest PC
    exit_pcs:   Vec<u64>,   // all possible exit points within the TB
}

impl CompiledTb {
    pub fn execute(
        &self,
        cpu: &mut dyn CpuState,
        mem: &MemSpace,
    ) -> OmniResult<TbResult>;

    pub fn insn_count(&self) -> u64 { self.insn_count as u64 }
}
```

### 5.4  TbCache — Translation Block Cache

```
/// TB cache: guest PC → CompiledTb
pub struct TbCache {
    /// Hot cache (direct-mapped, ultra-fast lookup)
    fast:       Box<[Option<CachedTb>; FAST_CACHE_SIZE]>,
    /// Slow cache (HashMap, covers the full address space)
    slow:       HashMap<u64, Arc<CompiledTb>>,
    /// Maximum capacity (bytes); LRU eviction when exceeded
    max_bytes:  usize,
    used_bytes: usize,
}

const FAST_CACHE_SIZE: usize = 1 << 14; // 16 K slots

impl TbCache {
    pub fn lookup(&self, pc: u64) -> Option<Arc<CompiledTb>>;
    pub fn insert(&mut self, pc: u64, tb: CompiledTb) -> Arc<CompiledTb>;
    /// Invalidate an address range (SMC / self-modifying code)
    pub fn invalidate_range(&mut self, start: u64, end: u64);
    /// Full invalidation (ASID switch / TLB flush)
    pub fn flush_all(&mut self);
    pub fn stats(&self) -> TbCacheStats;
}
```

### 5.5  Frontend / Backend Combination Matrix

| Guest ISA (frontend) | Host x86_64 | Host AArch64 | Notes |
| --- | --- | --- | --- |
| ARM AArch64 | ✅ Primary | ✅ Supported | |
| ARM AArch32 | ✅ Supported | Compatible with 32-bit images | |
| RISC-V RV64 | ✅ Primary | ✅ Supported | |
| RISC-V RV32 | ✅ Supported | Planned | Embedded scenarios |
| x86-64 | KVM preferred | ✅ TCG | KVM recommended on x86 hosts |

## 6  omnisim-compat — L4 Compatibility Layer

```
L4 Layer Responsibilities
omnisim-compat makes the outside world (libvirt / OpenStack / virt-manager) see OmniSim as a standard QEMU process. It implements QEMU command-line parsing and the core subset of the QMP protocol, internally translating them into L3 Manifest operations and L2 Platform control.
```

### 6.1  CLI Parser — QEMU Command-Line Parsing

```rust
// omnisim-compat/src/cli/parser.rs
```

```
/// Parses a QEMU-style command line and outputs a Manifest
pub struct QemuCliParser {
    /// Known -machine names → base Manifest templates
    machine_map: HashMap<String, Manifest>,
    /// Known -device driver names → UnitDesc generators
    device_map:  HashMap<String, DeviceFactory>,
}

impl QemuCliParser {
    /// Parse the full command line (std::env::args() or test input)
    pub fn parse(&self, args: &[String]) -> OmniResult<Manifest>;
}

// Supported argument groups (in priority order):
//   -machine <name>               select base platform template
//   -cpu <model>                  override CPU type
//   -m <size>[M|G]                memory size
//   -smp <n>                      CPU count
//   -kernel <path>                kernel path
//   -initrd <path>                initrd path
//   -append <cmdline>             kernel parameters
//   -dtb <path>                   device tree
//   -device <driver>[,key=val...] hot-plug device
//   -drive file=<path>[,if=virtio,...]
//   -netdev user[,id=...]
//   -chardev socket[,path=...]
//   -qmp unix:<path>,server[,nowait]
//   -qmp tcp:<host>:<port>
//   -enable-kvm                   → backend=kvm
//   -nographic                    → disable framebuffer
//   -S                            → pause after start (wait for QMP cont)
//   -s / -gdb tcp::1234           → start GDB stub

/// Example translation from -device argument to UnitDesc:
/// -device virtio-blk-pci,drive=d0,id=blk0
/// →
/// UnitDesc {
///   id:     "blk0",
///   kind:   "virtio-blk-pci",
///   config: { "drive": "d0" }
/// }
```

### 6.2  QMP Server — QMP Protocol Implementation

```rust
// omnisim-compat/src/qmp/server.rs
```

```
pub struct QmpServer {
    listener:   QmpListener,           // Unix socket or TCP
    scheduler:  Arc<Mutex<Scheduler>>,
    platform:   Arc<RwLock<Platform>>,
    realizer:   Arc<Realizer>,
    negotiated: bool,                  // capability negotiation complete flag
}

pub enum QmpListener {
    UnixSocket(PathBuf),
    Tcp(SocketAddr),
}

impl QmpServer {
    /// Start listening (in a dedicated thread)
    pub fn start(self) -> OmniResult<JoinHandle<()>>;

    /// Handle a single connection
    async fn handle_conn(&self, conn: QmpConn);

    /// Dispatch a command
    async fn dispatch(&self, cmd: QmpCommand) -> QmpResponse;
}
```

### 6.3  Complete QMP Command List

Sorted by actual libvirt usage frequency, with implementation priority noted:

| Command | Priority | Description | L2/L3 Operation |
| --- | --- | --- | --- |
| qmp_capabilities | P0 | Capability negotiation (first handshake step; must implement) | Return list of capabilities supported by OmniSim |
| query-version | P0 | QEMU version info (used by libvirt for detection) | Return hardcoded compatible version number |
| query-status | P0 | Query running state | Scheduler::state() |
| stop | P0 | Pause all vCPUs | Scheduler::pause() |
| cont | P0 | Resume execution | Scheduler::resume() |
| system_reset | P0 | System reset | Scheduler::reset() |
| system_powerdown | P0 | Graceful shutdown (send ACPI event) | IrqLine → ACPI Unit |
| query-cpus-fast | P0 | Query CPU state list | UnitRegistry::find_by_kind("*-core") |
| query-pci | P1 | Query PCIe topology | Platform::buses.pcie_buses |
| query-block | P1 | Query block device list | UnitRegistry::find_by_kind("virtio-blk*") |
| device_add | P1 | Hot-plug device | Platform::hot_add() + Scheduler::hot_add_unit() |
| device_del | P1 | Hot-unplug device | Platform::hot_remove() |
| drive_add | P1 | Hot-plug disk | Translated to device_add(virtio-blk-pci) |
| netdev_add | P1 | Hot-plug NIC | Translated to device_add |
| migrate | P2 | Live migration | Platform::snapshot() + transfer + target restore() |
| migrate-cancel | P2 | Cancel migration | |
| query-migrate | P2 | Query migration status | |
| savevm | P2 | Save snapshot | Platform::snapshot() → file |
| loadvm | P2 | Load snapshot | Platform::restore() |
| query-snapshots | P2 | List snapshots | |
| object_add | P2 | Add QEMU object (memory-backend-file etc.) | Dynamic Manifest modification |
| dump-guest-memory | P3 | Export guest memory (crash dump) | MemSpace bulk read |
| x-query-profile | P3 | Performance profiling data | Scheduler + UnitThread statistics |

### 6.4  QMP Message Format

```rust
// omnisim-compat/src/qmp/types.rs
```

```
/// QMP greeting (server sends proactively upon connection)
/// { "QMP": { "version": {...}, "capabilities": [...] } }
#[derive(Serialize)]
pub struct QmpGreeting {
    #[serde(rename = "QMP")]
    pub qmp: QmpGreetingBody,
}

/// Command sent by client
/// { "execute": "query-status", "arguments": {...}, "id": <any> }
#[derive(Deserialize)]
pub struct QmpRequest {
    pub execute:   String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// Passed through to response; used by libvirt to correlate requests
    pub id:        Option<serde_json::Value>,
}

/// Success response
/// { "return": {...}, "id": <any> }
#[derive(Serialize)]
pub struct QmpSuccess {
    #[serde(rename = "return")]
    pub result: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id:     Option<serde_json::Value>,
}

/// Error response
/// { "error": { "class": "GenericError", "desc": "..." }, "id": <any> }
#[derive(Serialize)]
pub struct QmpError {
    pub error: QmpErrorBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id:    Option<serde_json::Value>,
}

/// Async event (server pushes proactively; no id)
/// { "event": "STOP", "data": {...}, "timestamp": {...} }
#[derive(Serialize)]
pub struct QmpEvent {
    pub event:     String,
    pub data:      serde_json::Value,
    pub timestamp: QmpTimestamp,
}

/// Async events emitted by OmniSim (subscribed to by libvirt):
/// STOP              → Scheduler enters Paused
/// RESUME            → Scheduler enters Running
/// RESET             → system reset complete
/// SHUTDOWN          → guest initiated shutdown
/// POWERDOWN         → ACPI shutdown
/// DEVICE_TRAY_MOVED → optical drive ejected
/// DEVICE_DELETED    → hot-unplug complete
/// GUEST_PANICKED    → guest kernel panic
/// MIGRATION_STATUS  → migration state changed
/// BALLOON_CHANGE    → memory balloon changed
```

### 6.5  GDB Stub — omni-dbg

Implements GDB RSP (Remote Serial Protocol), enabling standard GDB / LLDB to debug the guest running inside OmniSim.

```rust
// tools/omni-dbg/src/gdb_stub.rs
```

```
pub struct GdbStub {
    listen_addr: SocketAddr,
    scheduler:   Arc<Mutex<Scheduler>>,
    platform:    Arc<RwLock<Platform>>,
    target_cpu:  UnitId,   // currently debugged CPU Unit
}

/// Implemented GDB RSP commands:
/// ?        → query stop reason
/// g / G    → read/write all registers
/// p / P    → read/write a single register
/// m / M    → read/write memory
/// c / C    → continue execution
/// s / S    → single step
/// z0/Z0    → remove/insert software breakpoint
/// z1/Z1    → remove/insert hardware breakpoint
/// z2/Z2    → remove/insert write watchpoint
/// z3/Z3    → remove/insert read watchpoint
/// qXfer:features → read target XML (register descriptions)
/// vCont    → multi-thread control (SMP debugging)
/// T        → query whether a thread is alive
/// H        → select operating thread (maps to CPU Unit)

/// GDB ↔ OmniSim mapping:
/// GDB thread id  →  CPU Unit id  (soc0.arm_ss.core0 → tid=1)
/// GDB register   →  DebugBackend::read_reg()
/// GDB memory     →  MemSpace::read() (from the perspective of the target CPU)
```

### 6.6  Executable Masquerade

OmniSim masquerades as QEMU executables via symlinks, allowing seamless libvirt integration:

```
# Create symlinks at install time
ln -s /usr/bin/omnisim /usr/bin/qemu-system-aarch64
ln -s /usr/bin/omnisim /usr/bin/qemu-system-x86_64
ln -s /usr/bin/omnisim /usr/bin/qemu-system-riscv64

# OmniSim reads argv[0] to infer the default architecture:
# qemu-system-aarch64 → arch = "aarch64"
# qemu-system-x86_64  → arch = "x86_64"

# Full invocation generated by libvirt (handled transparently by OmniSim):
qemu-system-aarch64 \
  -machine virt \
  -accel kvm \
  -cpu host \
  -m 4096 \
  -smp 4 \
  -kernel /path/to/Image \
  -append "root=/dev/vda console=ttyAMA0" \
  -drive file=/path/to/disk.qcow2,if=virtio \
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0 \
  -qmp unix:/var/run/omnisim-1234.sock,server,nowait \
  -daemonize -pidfile /var/run/omnisim-1234.pid
```

## 7  Component Implementation Conventions

```
Purpose of Conventions
All component crates follow the same set of conventions to guarantee interchangeability: framework code and test code never need to know which specific component is in use — they interact exclusively through traits.
```

### 7.1  Standard Component Crate Structure

```
// Using omnisim-intc-gicv3 as an example
omnisim-intc-gicv3/
├── Cargo.toml
├── src/
│   ├── lib.rs          // register() entry point + pub re-exports
│   ├── gicv3.rs        // GicV3 (Composite Unit)
│   ├── gicd.rs         // Distributor (Leaf Unit: MemTarget + IrqSink)
│   ├── gicr.rs         // Redistributor × N (Leaf Unit)
│   ├── gicc.rs         // CPU Interface (Leaf Unit)
│   └── schema.json     // component config JSON Schema
└── tests/
    ├── unit_test.rs    // unit tests (MockBackend + MockMemTarget)
    └── integ_test.rs   // integration tests (minimal Platform)
```

### 7.2  Leaf Unit Standard Pattern

```rust
// Using UART (omnisim-io-uart) as an example
```

```
/// Leaf Unit: implements only MemTarget (responds to address reads/writes);
/// does not implement ExecutionBackend (no independent execution logic)
pub struct PL011Uart {
    id:       UnitId,
    /// Populated after realize (obtained from IrqRouter)
    irq_line: Option<IrqLine>,
    /// Register state
    regs:     Mutex<UartRegs>,
}

impl Unit for PL011Uart {
    fn id(&self)   -> &UnitId      { &self.id }
    fn kind(&self) -> &'static str { "pl011-uart" }
    fn caps(&self) -> UnitCaps {
        UnitCaps::ADDRESSABLE | UnitCaps::IRQ_SOURCE
    }

    fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
        // Obtain outgoing interrupt line from IrqRouter
        self.irq_line = Some(
            ctx.irq_router.get_line(&self.id, "irq_out")?
        );
        Ok(())
    }

    fn unrealize(&mut self) -> OmniResult<()> {
        self.irq_line = None;
        Ok(())
    }

    fn reset(&mut self) -> OmniResult<()> {
        *self.regs.lock().unwrap() = UartRegs::default();
        Ok(())
    }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }

    fn irq_source_descs(&self) -> &[IrqSourceDesc] {
        &[IrqSourceDesc {
            name: "irq_out".into(),
            kind: IrqKind::LevelSensitive,
        }]
    }
}

impl MemTarget for PL011Uart {
    fn target_id(&self) -> u32 { /* assigned by MemBus */ 0 }
    fn base(&self)      -> u64 { 0 }     // offset 0
    fn size(&self)      -> u64 { 0x1000 }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        let regs = self.regs.lock().unwrap();
        match offset {
            0x00 => Ok(regs.dr   as u64),  // Data Register
            0x18 => Ok(regs.fr   as u64),  // Flag Register
            0x24 => Ok(regs.ibrd as u64),  // Integer Baud Rate
            _    => Ok(0),
        }
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        let mut regs = self.regs.lock().unwrap();
        match offset {
            0x00 => {
                regs.dr = val as u8;
                // Trigger TX-complete interrupt
                drop(regs);
                if let Some(irq) = &self.irq_line {
                    irq.pulse()?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
```

### 7.3  Executable Unit Standard Pattern

```rust
// Using ARM Cortex-A78 Core as an example
```

```
pub struct CortexA78Core {
    id:      UnitId,
    /// TCG engine (Functional Backend) or KvmBackend
    backend: Box<dyn ExecutionBackend>,
    /// ARM system register state (SCTLR/TCR/TTBR...)
    sysregs: ArmSysRegs,
}

impl Unit for CortexA78Core {
    fn caps(&self) -> UnitCaps {
        UnitCaps::EXECUTABLE | UnitCaps::DEBUGGABLE | UnitCaps::SNAPSHOTTABLE
    }

    fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
        // Inject MemBus view into Backend (TCG accesses memory via MemSpace)
        // KvmBackend: set up KVM memslots
        self.backend.attach_mem_space(ctx.mem_space.take().unwrap())?;
        Ok(())
    }

    fn execution_backend(&mut self)
        -> Option<&mut dyn ExecutionBackend> { Some(&mut *self.backend) }

    fn save_snapshot(&self) -> OmniResult<UnitSnapshot> {
        Ok(UnitSnapshot {
            unit_id: self.id.clone(),
            data: self.backend.save_state()?,
        })
    }
}
```

### 7.4  Composite Unit Standard Pattern

```rust
// Using GICv3 (with GICD + GICR×N + GICC×N) as an example
```

```
pub struct GicV3 {
    id:        UnitId,
    num_cpus:  u32,
    sub_units: Vec<Box<dyn Unit>>,   // [gicd, gicr0..N, gicc0..N]
    /// Internal MemBus (address mappings for gicd/gicr)
    internal_mem_bus: MemBus,
}

impl Unit for GicV3 {
    fn caps(&self) -> UnitCaps {
        UnitCaps::COMPOSITE | UnitCaps::IRQ_SINK | UnitCaps::ADDRESSABLE
    }

    fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
        // 1. realize all sub-Units
        for sub in &mut self.sub_units {
            let mut sub_ctx = ctx.child_ctx(sub.id());
            sub.realize(&mut sub_ctx)?;
        }
        // 2. Register the GIC address windows on the external MemBus
        //    GICD: 0x08000000, GICR: 0x08010000 × N
        //    Actual addresses are determined by the MemBus config in the Manifest
        Ok(())
    }

    fn sub_units(&self) -> Option<&[Box<dyn Unit>]> {
        Some(&self.sub_units)
    }
    fn sub_units_mut(&mut self) -> Option<&mut Vec<Box<dyn Unit>>> {
        Some(&mut self.sub_units)
    }

    /// GICv3 as IrqSink: peripheral interrupt → GICD.SETSPI
    fn as_irq_sink(&self) -> Option<Arc<dyn IrqSink>> {
        // Return the GICD's IrqSink implementation
        Some(self.gicd().as_irq_sink().unwrap())
    }
}
```

### 7.5  Component Config Schema Convention

Each component must provide a JSON Schema; the Validator checks UnitDesc.config before realize:

```
// components/intc/omnisim-intc-gicv3/src/schema.json
{
    "$schema": "http://json-schema.org/draft-07/schema",
    "type": "object",
    "required": ["num_cpus"],
    "additionalProperties": false,
    "properties": {
        "num_cpus": {
            "type": "integer",
            "minimum": 1,
            "maximum": 256,
            "description": "Number of CPU interfaces (GICRs)"
        },
        "gicd_base": {
            "type": "integer",
            "description": "GICD base address (override; usually from MemBus)"
        },
        "gicr_stride": {
            "type": "integer",
            "default": 131072,
            "description": "GICR stride per CPU (default 128 KB)"
        },
        "its_enable": {
            "type": "boolean",
            "default": false
        }
    }
}
```

### 7.6  Unit Test Conventions

Each component crate must include:

| Test Type | Tooling | What to Test |
| --- | --- | --- |
| Register read/write | MockMemAccess + direct MemTarget calls | Read/write logic for all registers; boundary values; reset values |
| Interrupt triggering | MockIrqSink (records raise/lower counts) | Trigger conditions; level/edge semantics; mask logic |
| Unit lifecycle | MinimalPlatform (helper) | Idempotency of realize → reset → unrealize |
| Snapshot/Restore | save_snapshot + load_snapshot | State completeness; cross-version compatibility (optional) |
| Integration: minimal platform | Manifest + Realizer + MockCpu | Component behavior under full Bus wiring |

## 8  Project Structure and Build Conventions

### 8.1  Complete Workspace Structure

```
omnisim/
├── Cargo.toml                      # workspace root
├── Cargo.lock
├── deny.toml                       # cargo-deny: license + vulnerability checks
├── .cargo/config.toml              # target / linker / env configuration
│
├── crates/                         # framework core crates (strict layering)
│   ├── omnisim-core/               # L1+L2 interfaces; zero business dependencies (std+thiserror+bitflags)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs            # OmniError / OmniResult
│   │       ├── unit.rs             # Unit trait / UnitId / UnitCaps
│   │       ├── backend/
│   │       │   ├── execution.rs    # ExecutionBackend / DebugBackend
│   │       │   └── timing.rs       # TimingModel / TimingKind
│   │       ├── mem.rs              # MemTarget / MemSpace / AddressTranslator
│   │       ├── irq.rs              # IrqSink / IrqLine / MsiSender
│   │       ├── dataflow.rs         # DataEndpoint / DataDesc / DataOp / DataToken
│   │       ├── bus/
│   │       │   ├── mem_bus.rs      # MemBus
│   │       │   ├── irq_bus.rs      # IrqBus / IrqRouter
│   │       │   ├── pcie_bus.rs     # PcieBus / PcieEndpoint / PcieRootComplex
│   │       │   ├── sync_bus.rs     # SyncBus
│   │       │   ├── data_bus.rs     # DataBus
│   │       │   ├── reset_bus.rs    # ResetBus / ResetSink / ResetLine / ReadySignal
│   │       │   ├── power_bus.rs    # PowerBus / PowerSink / PowerState
│   │       │   └── interconnect_bus.rs  # InterconnectBus / InterconnectBackend
│   │       ├── security.rs         # MemSecurity / SecurityWorld (reserved P0, implemented P2)
│   │       ├── coherence.rs        # CoherenceDomain / CoherenceParticipant (P2 placeholder)
│   │       ├── network.rs          # NetworkBackend / Packet
│   │       ├── registry.rs         # UnitRegistry
│   │       └── snapshot.rs         # UnitSnapshot / PlatformSnapshot
│   │
│   ├── omnisim-manifest/           # L3 platform description
│   │   └── src/
│   │       ├── manifest.rs         # Manifest / UnitDesc / BusDesc / BootDesc
│   │       ├── validator.rs        # Validator / ValidationReport
│   │       ├── expander.rs         # template expansion / count expansion
│   │       ├── realizer.rs         # Realizer / ComponentRegistry / Platform
│   │       └── loader.rs           # from_json / from_toml / to_json / to_toml
│   │
│   ├── omnisim-runtime/            # runtime scheduling
│   │   └── src/
│   │       ├── scheduler.rs        # Scheduler / SchedulerConfig
│   │       ├── unit_thread.rs      # UnitThread / CtrlCmd / UnitEvent
│   │       ├── vm_exit.rs          # VmExitDispatcher
│   │       ├── clock.rs            # SimClock
│   │       ├── boot.rs             # BootSequencer
│   │       └── main.rs             # program entry point (register all components, parse CLI, start)
│   │
│   ├── omnisim-tcg/                # TCG execution engine
│   │   └── src/
│   │       ├── engine.rs           # TcgEngine<F,B>
│   │       ├── frontend.rs         # TcgFrontend trait / CpuState trait
│   │       ├── host_backend.rs     # TcgHostBackend trait
│   │       ├── ir.rs               # TcgIrBlock / TcgOp
│   │       ├── optimizer.rs        # constant folding / dead code elimination
│   │       ├── tb_cache.rs         # TbCache
│   │       ├── frontends/
│   │       │   ├── arm/            # ARM AArch64 + AArch32 frontend
│   │       │   ├── riscv/          # RISC-V RV64/RV32 frontend
│   │       │   └── x86/            # x86-64 frontend
│   │       └── host_backends/
│   │           ├── x86_64/         # generates x86_64 machine code
│   │           └── aarch64/        # generates AArch64 machine code
│   │
│   └── omnisim-compat/             # L4 compatibility layer
│       └── src/
│           ├── lib.rs
│           ├── cli/
│           │   ├── parser.rs       # QemuCliParser
│           │   └── machine_map.rs  # -machine name → base Manifest
│           └── qmp/
│               ├── server.rs       # QmpServer
│               ├── types.rs        # QmpRequest / QmpResponse / QmpEvent
│               └── handlers.rs     # handler for each QMP command
│
├── components/                     # hardware component library
│   ├── cpu/
│   │   ├── omnisim-cpu-arm/        # ARM Cortex-A series
│   │   ├── omnisim-cpu-riscv/      # RISC-V RV64/RV32
│   │   ├── omnisim-cpu-x86/        # x86-64 (TCG + KVM)
│   │   └── omnisim-cpu-mock/       # Mock CPU (testing)
│   │   # MMU merged into each CPU crate; not a standalone component
│   │   # omnisim-cpu-arm/src/mmu.rs    ARMv8/v9 MMU (Stage1 + Stage2)
│   │   # omnisim-cpu-riscv/src/mmu.rs  Sv39/Sv48/Sv57
│   ├── intc/
│   │   ├── omnisim-intc-gicv3/     # ARM GICv3 (GICD+GICR+GICC+ITS)
│   │   ├── omnisim-intc-plic/      # RISC-V PLIC
│   │   └── omnisim-intc-clint/     # RISC-V CLINT (software interrupt + timer)
│   ├── cache/
│   │   └── omnisim-cache/          # Unified Cache model (L1/L2/L3 distinguished by capacity/latency params)
│   │   #    May be split into separate crates in P2 if RTL co-sim divergence is too large
│   ├── memory/
│   │   ├── omnisim-mem-dram/       # DRAM (with TimingBackend)
│   │   ├── omnisim-mem-sram/       # SRAM (no timing; low latency)
│   │   └── omnisim-mem-gddr/       # GDDR6 (GPU VRAM; with TimingBackend)
│   ├── io/
│   │   ├── omnisim-io-uart/        # PL011 UART
│   │   ├── omnisim-io-pcie/        # PCIe Root Complex + EP framework
│   │   ├── omnisim-io-dma/         # DMA engine (with Dataflow interface)
│   │   ├── omnisim-io-mailbox/     # on-chip Mailbox (bidirectional interrupt)
│   │   └── omnisim-io-virtio/      # Virtio bus + devices (blk/net/console)
│   └── accel/
│       ├── omnisim-accel-gpgpu/    # GPGPU (SM + L2 + VRAM; Composite)
│       └── omnisim-accel-dpu/      # DPU (network processor; Composite)
│
├── profiles/                       # pre-built Platform Profiles
│   ├── embedded/                   # embedded SoC (RISC-V)
│   ├── server/                     # ARM server
│   ├── hetero/                     # heterogeneous acceleration platform
│   └── extended/                   # extended feature interface placeholders (TimingBackend / RTL co-sim etc.)
│
├── tools/
│   ├── omni-cli/                   # CLI entry point (registers all components)
│   └── omni-dbg/                   # GDB stub
│
└── tests/
    ├── common/                     # test utilities (MinimalPlatform / MockBackend)
    ├── system/                     # system-level test entry (calls sub-crate tests)
    ├── integration/
    │   ├── scenario01_arm_riscv.rs    # Scenario 1:  on-chip heterogeneous dual-core
    │   ├── scenario02_pcie_gpgpu.rs   # Scenario 2:  PCIe GPGPU
    │   ├── scenario03_pcie_dpu.rs     # Scenario 3:  PCIe DPU
    │   ├── scenario04_multi_gpu.rs    # Scenario 4:  multi-card interconnect
    │   ├── scenario05_embedded_soc.rs # Scenario 5:  embedded SoC
    │   ├── scenario06_network_proc.rs # Scenario 6:  network processor
    │   ├── scenario07_smart_ssd.rs    # Scenario 7:  SmartSSD
    │   ├── scenario08_cxl.rs          # Scenario 8:  CXL device
    │   ├── scenario09_numa_server.rs  # Scenario 9:  multi-socket server NUMA
    │   ├── scenario10_soc_numa.rs     # Scenario 10: large-memory SoC NUMA
    │   ├── scenario11_nested_virt.rs  # Scenario 11: nested virtualization
    │   ├── scenario12_trustzone.rs    # Scenario 12: TrustZone / TEE
    │   ├── scenario13_cvm.rs          # Scenario 13: confidential virtualization (P2)
    │   ├── scenario14_ai_server.rs    # Scenario 14: AI server full-stack
    │   ├── scenario15_hetero_mem.rs   # Scenario 15: heterogeneous memory platform
    │   └── scenario16_numa_gpu.rs     # Scenario 16: NUMA + multi-card mixed
    └── fixtures/                   # test Manifest JSON/TOML files
```

### 8.2  Cargo.toml Conventions

```toml
// Root Cargo.toml
```

```
[workspace]
members = [
    "crates/omnisim-core",
    "crates/omnisim-manifest",
    "crates/omnisim-runtime",
    "crates/omnisim-tcg",
    "crates/omnisim-compat",
    "components/cpu/omnisim-cpu-arm",
    # ... all components
    "tools/omni-cli",
    "tools/omni-dbg",
]
resolver = "2"

[workspace.dependencies]
# Versions managed centrally; each crate uses { workspace = true }
omnisim-core = { path = "crates/omnisim-core" }
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
thiserror    = "1"
log          = "0.4"
tracing      = "0.1"
tokio        = { version = "1", features = ["full"] }
bitflags     = "2"
kvm-ioctls   = "0.16"
vm-memory    = "0.12"

[profile.release]
opt-level     = 3
lto           = "thin"
codegen-units = 1

[profile.bench]
inherits = "release"
debug    = true
```

### 8.3  Dependency Constraints (cargo-deny)

| Rule | Description |
| --- | --- |
| Component crates may only depend on omnisim-core | Cross-component dependencies are prohibited (checked via `bans` in deny.toml) |
| omnisim-core has zero business dependencies | Only std + thiserror + bitflags permitted; no business logic dependencies |
| License allowlist | MIT / Apache-2.0 / BSD-2/3 / ISC (GPL/LGPL must not enter the core) |
| No `unwrap()` in library code | `clippy::unwrap_used = deny` (enforced via .clippy.toml) |
| All `pub` interfaces documented | `missing_docs = warn` (core crates use `deny`) |
