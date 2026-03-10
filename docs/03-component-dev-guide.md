> [中文版](./03-component-dev-guide.zh-CN.md)

---

# OmniSim — Component Development Guide

> **Document ID**: DOC-03 &nbsp;&nbsp; **Version**: v1.0 &nbsp;&nbsp; **Date**: 2026-Q1 &nbsp;&nbsp; **Author**: OpenFunTech

---

| Document ID | DOC-03 |
| --- | --- |
| Version | v1.0 |
| Audience | Component developers (internal / external) · New contributors |
| Prerequisites | DOC-01 Architecture Design · DOC-02 Interface Spec |
| Last Updated | 2026-Q1 |

## 1  Overview

This guide is for component developers and answers the question "how do I write a new component?" Please complete DOC-01 (Architecture Design) and DOC-02 (Interface Spec) before reading this guide; interface definitions are not repeated here.

### 1.1  Where Components Fit in OmniSim

```
omnisim/components/         ← component library root
  cpu/                      ← CPU-class components
    omnisim-cpu-arm/        ← one component = one crate
    omnisim-cpu-riscv/
  accel/                    ← accelerator-class components
    omnisim-accel-gpgpu/    ← GPGPU Composite Unit
    omnisim-accel-dpu/
  io/                       ← I/O-class components
    omnisim-io-uart/
    omnisim-io-pcie/
  intc/                     ← interrupt controllers
  memory/                   ← memory components
  soc/                      ← SoC-level components
    omnisim-soc-core/       ← AXI Stream and other on-chip Buses
  net/                      ← network components
    omnisim-dpu-core/
    omnisim-net-hwfwd/
  storage/                  ← storage components
    omnisim-storage-nand/
```

### 1.2  Component Type Quick Reference

| Component Type | Core Traits to Implement | Typical Examples | Section |
| --- | --- | --- | --- |
| Simple peripheral | Unit + MemTarget | UART, Timer, GPIO | §2 |
| Executable unit | Unit + ExecutionBackend | CPU Core, DMA Engine | §3 |
| Composite Unit | Unit (with sub_units) | GPGPU, DPU, ARM SoC | §4 |
| SIMT execution unit | Unit + SimtHartBackend/SimtWarpBackend | GPGPU SM | §5 |
| Fixed-function accelerator | Unit + MemTarget + MockBackend | NPU (register interface) | §6 |
| Interconnect component | Unit + InterconnectBackend | CXL device, NVLink bridge | §7 |

## 2  Simple Peripheral Development

Simple peripherals are the most common component type: they have a set of MMIO registers, respond to CPU reads and writes, and may generate interrupts. A UART is used as a complete demonstration of the development workflow.

### 2.1  Create the Crate

```toml
# Create a new crate under components/io/
cargo new --lib omnisim-io-uart
cd omnisim-io-uart
# Cargo.toml
[package]
name    = "omnisim-io-uart"
version = "0.1.0"
edition = "2021"
[dependencies]
omnisim-core = { path = "../../crates/omnisim-core" }
# Only omnisim-core dependency is allowed; other component crates are forbidden
```

### 2.2  Implement MemTarget

```rust
// src/lib.rs
use omnisim_core::prelude::*;
pub struct Uart {
// Register state
tx_buf:  VecDeque<u8>,
rx_buf:  VecDeque<u8>,
ier:     u8,    // Interrupt Enable Register
lsr:     u8,    // Line Status Register
// Connected IrqLine (injected at realize time)
irq:     Option<IrqLine>,
}
impl MemTarget for Uart {
fn read(&self, offset: u64, buf: &mut [u8]) -> OmniResult<()> {
match offset {
0x00 => buf[0] = self.rx_buf.front().copied().unwrap_or(0),
0x01 => buf[0] = self.ier,
0x05 => buf[0] = self.lsr,
_ => buf.fill(0),
}
Ok(())
}
fn write(&mut self, offset: u64, buf: &[u8]) -> OmniResult<()> {
match offset {
0x00 => {
self.tx_buf.push_back(buf[0]);
self.flush_tx();
}
0x01 => self.ier = buf[0],
_ => {}
}
Ok(())
}
fn size(&self) -> u64 { 0x100 }
}
```

### 2.3  Implement the Unit Trait

```rust
impl Unit for Uart {
fn id(&self)   -> &UnitId  { &self.id }
fn kind(&self) -> &str     { "uart-16550" }
fn mem_target(&self) -> Option<Arc<dyn MemTarget>> {
Some(Arc::new(self.clone()))  // or return an Arc of self
}
fn irq_source_descs(&self) -> &[IrqSourceDesc] {
&[IrqSourceDesc { signal: "irq_out".into() }]
}
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// Obtain the connected interrupt line from IrqRouter
self.irq = Some(ctx.irq_router.get_line(&self.id, "irq_out")?);
Ok(())
}
}
```

### 2.4  Register with the Factory

```rust
// src/lib.rs
pub fn register(registry: &mut ComponentRegistry) {
registry.register_unit("uart-16550", |config| {
Box::new(Uart::from_config(config)?) as Box<dyn Unit>
});
}
```

### 2.5  Unit Tests

```rust
#[cfg(test)]
mod tests {
use super::*;
use omnisim_core::test_utils::MockIrqLine;
#[test]
fn test_uart_tx() {
let mut uart = Uart::new("uart0");
uart.irq = Some(MockIrqLine::new());
// Write TX register
uart.write(0x00, &[b'H']).unwrap();
assert_eq!(uart.tx_buf.len(), 1);
}
#[test]
fn test_uart_rx_irq() {
let mut uart = Uart::new("uart0");
let irq = MockIrqLine::new();
uart.ier = 0x01;  // Enable RX interrupt
uart.irq = Some(irq.clone());
uart.inject_rx(b'A');
assert!(irq.was_asserted());
}
}
```

## 3  Executable Unit Development

An executable unit implements ExecutionBackend and is periodically driven by the OmniSim scheduler. A typical example is a DMA Engine (no full CPU TCG required, but it must actively transfer data).

### 3.1  Implement ExecutionBackend

```rust
pub struct DmaEngine {
id:       UnitId,
// Control registers
src_addr: u64,
dst_addr: u64,
length:   u64,
ctrl:     u8,
// Injected at runtime
mem_bus:  Option<Arc<MemBus>>,
irq:      Option<IrqLine>,
}
impl ExecutionBackend for DmaEngine {
fn exec(&mut self, max_cycles: u64) -> ExecResult {
if self.ctrl & 0x01 == 0 {
return ExecResult { cycles: 1, stop: false };
}
// Execute one DMA transfer
let mem = self.mem_bus.as_ref().unwrap();
let mut buf = vec![0u8; self.length as usize];
mem.read_bulk(self.src_addr, &mut buf).unwrap();
mem.write_bulk(self.dst_addr, &buf).unwrap();
// Transfer complete, trigger interrupt
self.ctrl &= !0x01;
if let Some(irq) = &self.irq { irq.assert(); }
ExecResult { cycles: self.length, stop: false }
}
fn backend_kind(&self) -> BackendKind { BackendKind::Functional }
}
```

### 3.2  Obtain MemBus Reference at realize Time

```rust
impl Unit for DmaEngine {
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
self.mem_bus = Some(ctx.mem_bus("main-mem-bus")?);
self.irq     = Some(ctx.irq_router.get_line(&self.id, "irq_done")?);
Ok(())
}
}
```

## 4  Composite Unit Development

A Composite Unit contains multiple child Units internally and presents as a single whole externally (e.g., a GPGPU card appears as a single PCIe EP to the Host).

### 4.1  Structural Design Principles

```
Three Elements of a Composite Unit
Child Unit list (sub_units): all internal constituent units
Internal Buses: connections between child Units, not visible externally
Boundary interfaces (Ports): externally-exposed MemTarget / IrqSource / ResetSink, etc.
```

### 4.2  DPU Example Skeleton

```rust
pub struct Dpu {
id:       UnitId,
// Child Units
arm_ctrl:    ArmCore,
virtio_net:  VirtioNet,
virtio_blk:  VirtioBlk,
nvmeof:      NvmeOfInitiator,
net_switch:  NetSwitch,
dma_engine:  DmaEngine,
// Internal Buses
internal_mem_bus: MemBus,
packet_bus:       PacketBus,    // omnisim-dpu-core component layer
// External interface (PCIe EP)
bars:     Vec<BarDesc>,
}
impl Unit for Dpu {
fn sub_units(&self) -> Vec<&dyn Unit> {
vec![&self.arm_ctrl, &self.virtio_net,
&self.virtio_blk, &self.nvmeof,
&self.net_switch, &self.dma_engine]
}
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 1. Realize internal MemBus
self.internal_mem_bus.realize()?;
// 2. Realize child Units in order, injecting internal Bus references
self.arm_ctrl.realize_with_bus(&self.internal_mem_bus)?;
self.virtio_net.realize_with_bus(&self.internal_mem_bus)?;
// 3. Register with external PcieBus
ctx.pcie_bus.register_endpoint(self.id.clone(), self)?;
Ok(())
}
}
impl PcieEndpoint for Dpu {
fn bar_descs(&self) -> &[BarDesc] { &self.bars }
fn on_bar_write(&self, bar_idx: u8, offset: u64, data: &[u8]) -> OmniResult<()> {
// Host writes DPU control registers
self.internal_mem_bus.write(offset, data)
}
// Reverse DMA: DPU proactively reads Host vring
fn dma_read(&self, host_addr: u64, buf: &mut [u8]) -> OmniResult<()> {
self.pcie_bus_ref.ep_dma_read(&self.id, host_addr, buf)
}
}
```

## 5  SIMT Execution Unit Development (GPGPU SM)

A GPGPU's SM (Streaming Multiprocessor) is a special executable unit that must implement SIMT parallel execution semantics. OmniSim provides two routes; component developers choose based on their needs.

### 5.1  Route A: SimtHartBackend (RISC-V Hart Array)

Simulates threads within an SM using a RISC-V hart array, leveraging TCG's existing RVV extension. Suitable for scenarios that require executable RISC-V binaries.

```rust
pub struct SmUnitHart {
id:              UnitId,
threads_per_warp: u32,     // configurable: 32 / 64 / custom
warp_count:      u32,
// Each thread corresponds to one RISC-V hart
harts:           Vec<RiscvHart>,
// Shared memory (visible to all warps in the same SM)
shared_mem:      SharedSram,
}
impl ExecutionBackend for SmUnitHart {
fn backend_kind(&self) -> BackendKind { BackendKind::SimtHart }
fn exec(&mut self, max_cycles: u64) -> ExecResult {
// Schedule one warp: all threads execute the same instruction
let warp = self.next_warp();
let instr = warp.fetch_instruction();
// RVV vector instruction → execute all active threads in parallel
for hart in warp.active_threads() {
hart.exec_one(instr);
}
ExecResult { cycles: 1, stop: false }
}
}
```

### 5.2  Route B: SimtWarpBackend (Warp Scheduler)

Directly simulates SIMT semantics using a Warp scheduler + vector execution unit; supports custom ISAs (independent of RISC-V) via the pluggable GpuFrontend trait.

```rust
/// GPU ISA frontend interface: pluggable ISA decoder
pub trait GpuFrontend: Send + Sync {
fn decode(&self, pc: u64, mem: &[u8]) -> GpuInstr;
fn isa_name(&self) -> &'static str;
}
pub struct SmUnitWarp {
id:              UnitId,
threads_per_warp: u32,
warp_count:      u32,
frontend:        Box<dyn GpuFrontend>,  // pluggable ISA
warps:           Vec<WarpState>,
vec_exec:        VectorExecUnit,
tensor_exec:     TensorExecUnit,
}
impl ExecutionBackend for SmUnitWarp {
fn backend_kind(&self) -> BackendKind { BackendKind::SimtWarp }
fn exec(&mut self, _max_cycles: u64) -> ExecResult {
let warp = self.scheduler.pick_warp(&self.warps);
let instr = self.frontend.decode(warp.pc, &warp.instr_cache);
match instr.kind {
GpuInstrKind::Vec    => self.vec_exec.execute(warp, &instr),
GpuInstrKind::Tensor => self.tensor_exec.execute(warp, &instr),
GpuInstrKind::Mem    => self.exec_mem(warp, &instr),
GpuInstrKind::Ctrl   => self.exec_ctrl(warp, &instr),
}
ExecResult { cycles: 1, stop: false }
}
}
```

## 6  Common Integration Patterns

### 6.1  Connecting to ResetBus

```rust
// Component needs to accept an external reset signal
impl ResetSink for MyUnit {
fn assert_reset(&self) -> OmniResult<()> {
// Stop execution, save/clear register state
self.state.lock().reset();
Ok(())
}
fn deassert_reset(&self) -> OmniResult<()> {
// Resume execution
self.start_execution();
Ok(())
}
fn is_in_reset(&self) -> bool { self.state.lock().in_reset }
}
impl Unit for MyUnit {
fn reset_sink(&self) -> Option<Arc<dyn ResetSink>> {
Some(Arc::new(self.reset_sink_impl.clone()))
}
// If this component can also control downstream resets
fn reset_source_descs(&self) -> &[ResetSourceDesc] {
&[ResetSourceDesc { signal_name: "sub_reset".into() }]
}
}
```

### 6.2  Connecting to PowerBus (Embedded SoC)

```rust
impl PowerSink for MyUnit {
fn power_on(&self)        -> OmniResult<()> { self.resume();  Ok(()) }
fn power_off(&self)       -> OmniResult<()> { self.suspend(); Ok(()) }
fn enter_retention(&self) -> OmniResult<()> { self.freeze();  Ok(()) }
fn power_state(&self)     -> PowerState     { self.current_power_state }
}
impl Unit for MyUnit {
fn power_sink(&self) -> Option<Arc<dyn PowerSink>> {
Some(Arc::new(self.power_sink_impl.clone()))
}
}
```

### 6.3  Implementing PCIe EP (with Reverse DMA)

```rust
// BAR descriptor: exposes control register region externally
impl PcieEndpoint for MyDevice {
fn bar_descs(&self) -> &[BarDesc] {
&[BarDesc { index: 0, size: 0x10000, kind: BarKind::Mem64 }]
}
fn on_bar_write(&self, _bar: u8, offset: u64, data: &[u8]) -> OmniResult<()> {
self.regs.write(offset, data)
}
fn on_bar_read(&self, _bar: u8, offset: u64, buf: &mut [u8]) -> OmniResult<()> {
self.regs.read(offset, buf)
}
// Reverse DMA: device proactively accesses Host memory
fn dma_read(&self, host_addr: u64, buf: &mut [u8]) -> OmniResult<()> {
// Routed through PcieBus, subject to IOMMU check
self.pcie_bus.ep_dma_read(&self.id, host_addr, buf)
}
fn dma_write(&self, host_addr: u64, buf: &[u8]) -> OmniResult<()> {
self.pcie_bus.ep_dma_write(&self.id, host_addr, buf)
}
}
```

### 6.4  Connecting to InterconnectBus (Card-to-Card Interconnect)

```rust
// Register with InterconnectBus to allow access from other nodes
impl Unit for MyAccel {
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// Declare this node's address window on the InterconnectBus
if let Some(ic_bus) = ctx.interconnect_bus("gpu-fabric") {
ic_bus.register_node(InterconnectNode {
unit_id:   self.id.clone(),
node_kind: NodeKind::Accelerator,
base_addr: self.vram_base,
size:      self.vram_size,
accessible_from: None,  // accessible from all nodes
})?;
}
Ok(())
}
}
// Access another node's memory across cards
fn peer_read(ic_bus: &InterconnectBus, peer: &UnitId, addr: u64) -> Vec<u8> {
let mut buf = vec![0u8; 64];
ic_bus.backend.read(&my_id, peer, addr, &mut buf).unwrap();
buf
}
```

### 6.5  Connecting to NetworkBackend

```rust
// DPU's nvme-of initiator uses NetworkBackend to access remote storage
pub struct NvmeOfInitiator {
backend: Box<dyn NetworkBackend>,
}
impl NvmeOfInitiator {
pub fn read_block(&self, lba: u64, buf: &mut [u8]) -> OmniResult<()> {
// RDMA read: directly read block data from remote storage server
let remote_addr = lba * 512;
self.backend.rdma_read(remote_addr, buf)
}
}
```

## 7  Testing Framework and Release Conventions

### 7.1  Three-Tier Testing Strategy

| Tier | Scope | Tools | When to Run |
| --- | --- | --- | --- |
| Unit tests | Single Unit internal logic | MockBus + MockIrqLine + MockResetBus | cargo test (every commit) |
| Integration tests | Multiple Units + Bus connections | RealizeCtx + MockBackend + minimal Manifest | CI before PR merge |
| Scenario tests | Full Manifest load and run | omnisim-runtime + test fixture Manifest | Nightly / before Release |

### 7.2  Unit Test Template

```rust
#[cfg(test)]
mod tests {
use omnisim_core::test_utils::*;
fn make_unit() -> MyUnit {
MyUnit::new("test-unit-0")
}
#[test]
fn test_realize() {
let mut unit = make_unit();
let mut ctx  = MockRealizeCtx::new();
unit.realize(&mut ctx).expect("realize should succeed");
}
#[test]
fn test_reset_cycle() {
let mut unit = make_unit();
let reset_sink = unit.reset_sink().unwrap();
reset_sink.assert_reset().unwrap();
assert!(reset_sink.is_in_reset());
reset_sink.deassert_reset().unwrap();
assert!(!reset_sink.is_in_reset());
}
#[test]
fn test_mem_read_write() {
let unit = make_unit();
let target = unit.mem_target().unwrap();
let mut buf = [0u8; 4];
target.read(0x00, &mut buf).unwrap();
// assert expected reset value
assert_eq!(buf, [0x00, 0x00, 0x00, 0x00]);
}
}
```

### 7.3  Manifest Integration Test

```rust
// tests/integration/uart_irq_test.rs
#[test]
fn test_uart_triggers_irq_to_gic() {
let manifest = Manifest::from_toml(r#"
[[units]]
id = "uart0"; kind = "uart-16550"; backend = "functional"
[[units]]
id = "gicv3"; kind = "arm-gicv3"; backend = "functional"
[[buses]]
kind = "irq"; id = "irq-bus"
[[buses.connections]]
from = "uart0.irq_out"
to   = "gicv3.spi[33]"
"#).unwrap();
let mut platform = Realizer::new().realize(manifest).unwrap();
// Inject RX data into UART, expect GIC SPI 33 to be asserted
let uart = platform.unit_mut("uart0").unwrap();
uart.inject_rx(b'A');
let gic = platform.unit("gicv3").unwrap();
assert!(gic.spi_pending(33));
}
```

### 7.4  Release Conventions

```
Component Release Rules
Version numbers follow SemVer: breaking interface changes → bump Major, new interfaces → Minor, fixes → Patch
Each component crate has an independent version, not tied to the framework version
Components may only depend on omnisim-core; cross-component-crate dependencies are forbidden (enforced via Cargo.toml tooling)
All public APIs must have doc comments (/// doc comment); CI checks with rustdoc --deny=missing_docs
Every PR must include corresponding tests; coverage must not drop below the current level
```

### 7.5  Component Naming Conventions

| Type | Naming Pattern | Examples |
| --- | --- | --- |
| CPU-class | omnisim-cpu-{isa} | omnisim-cpu-arm, omnisim-cpu-riscv |
| Interrupt controller | omnisim-intc-{name} | omnisim-intc-gicv3, omnisim-intc-plic |
| Memory-class | omnisim-mem-{type} | omnisim-mem-dram, omnisim-mem-sram |
| IO-class | omnisim-io-{name} | omnisim-io-uart, omnisim-io-pcie |
| Accelerator-class | omnisim-accel-{name} | omnisim-accel-gpgpu, omnisim-accel-dpu |
| SoC-level | omnisim-soc-{name} | omnisim-soc-core |
| Network-class | omnisim-net-{name} | omnisim-net-hwfwd |
| Storage-class | omnisim-storage-{name} | omnisim-storage-nand |
| Security-class | omnisim-security-{name} | omnisim-security-psp, omnisim-security-rmm |

## 8  CLI Quick Reference

OmniSim's primary configuration method is the Manifest file (see DOC-04 Use Case Handbook for details). The CLI is merely the startup entry point and has few options.

### 8.1  Basic Usage

```shell
# Start from a Manifest file
omni-cli --manifest platform.toml
# Specify a Profile (overrides the default Profile in the Manifest)
omni-cli --manifest platform.toml --profile server
# QEMU compatibility mode (libvirt invocation path)
omnisim-aarch64 -machine virt -cpu cortex-a57 -m 4096 \
-device virtio-blk-pci,drive=d0 \
-qmp unix:/var/run/omnisim.sock,server
# Debug mode: print the full topology after realize
omni-cli --manifest platform.toml --dump-topology
# Validate Manifest without running
omni-cli --manifest platform.toml --validate-only
```

### 8.2  Common Options

| Option | Description | Default |
| --- | --- | --- |
| --manifest \<path\> | Manifest file path (TOML or JSON) | None (required) |
| --profile \<n\> | Override the profile selection in the Manifest | Default profile declared in Manifest |
| --log-level \<lvl\> | Log level: error/warn/info/debug/trace | info |
| --qmp \<path\> | QMP socket path | Disabled |
| --dump-topology | Print the Unit/Bus topology after realize and exit | Disabled |
| --validate-only | Perform only Manifest syntax and semantic validation, do not start simulation | Disabled |
