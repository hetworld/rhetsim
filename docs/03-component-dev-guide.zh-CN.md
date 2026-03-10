> [English](./03-component-dev-guide.md)

---

# OmniSim — 组件开发指南

> **文档编号**：DOC-03 &nbsp;&nbsp; **版本**：v1.0 &nbsp;&nbsp; **日期**：2026-Q1 &nbsp;&nbsp; **作者**：OpenFunTech

---

| 文档编号 | DOC-03 |
| --- | --- |
| 文档版本 | v1.0 |
| 适用读者 | 组件开发者（内部 / 外部）· 新入贡献者 |
| 前置阅读 | DOC-01 架构设计 · DOC-02 接口规范 |
| 上次更新 | 2026-Q1 |

## 1  概览

本指南面向组件开发者，回答"怎么写一个新组件"。阅读前请先完成 DOC-01（架构设计）和 DOC-02（接口规范）的阅读，本文不重复接口定义。

### 1.1  组件在 OmniSim 中的位置

```
omnisim/components/         ← 组件库根目录
cpu/                      ← CPU 类组件
omnisim-cpu-arm/        ← 一个组件 = 一个 crate
omnisim-cpu-riscv/
accel/                    ← 加速器类组件
omnisim-accel-gpgpu/    ← GPGPU Composite Unit
omnisim-accel-dpu/
io/                       ← I/O 类组件
omnisim-io-uart/
omnisim-io-pcie/
intc/                     ← 中断控制器
memory/                   ← 内存组件
soc/                      ← SoC 级组件
omnisim-soc-core/       ← AXI Stream 等片上 Bus
net/                      ← 网络组件
omnisim-dpu-core/
omnisim-net-hwfwd/
storage/                  ← 存储组件
omnisim-storage-nand/
```

### 1.2  组件类型速查

| 组件类型 | 实现的核心 trait | 典型示例 | 本文章节 |
| --- | --- | --- | --- |
| 简单外设 | Unit + MemTarget | UART、Timer、GPIO | §2 |
| 可执行单元 | Unit + ExecutionBackend | CPU Core、DMA Engine | §3 |
| Composite Unit | Unit（含 sub_units） | GPGPU、DPU、ARM SoC | §4 |
| SIMT 执行单元 | Unit + SimtHartBackend/SimtWarpBackend | GPGPU SM | §5 |
| 固定功能加速器 | Unit + MemTarget + MockBackend | NPU（寄存器接口） | §6 |
| 互联类组件 | Unit + InterconnectBackend | CXL 设备、NVLink 桥接 | §7 |

## 2  简单外设开发

简单外设是最常见的组件类型：有一组 MMIO 寄存器，响应 CPU 读写，可能产生中断。以 UART 为例完整演示开发流程。

### 2.1  创建 crate

```toml
# 在 components/io/ 目录下新建 crate
cargo new --lib omnisim-io-uart
cd omnisim-io-uart
# Cargo.toml
[package]
name    = "omnisim-io-uart"
version = "0.1.0"
edition = "2021"
[dependencies]
omnisim-core = { path = "../../crates/omnisim-core" }
# 只允许依赖 omnisim-core，不允许依赖其他组件
```

### 2.2  实现 MemTarget

```rust
// src/lib.rs
use omnisim_core::prelude::*;
pub struct Uart {
// 寄存器状态
tx_buf:  VecDeque<u8>,
rx_buf:  VecDeque<u8>,
ier:     u8,    // Interrupt Enable Register
lsr:     u8,    // Line Status Register
// 连接的 IrqLine（realize 时注入）
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

### 2.3  实现 Unit trait

```rust
impl Unit for Uart {
fn id(&self)   -> &UnitId  { &self.id }
fn kind(&self) -> &str     { "uart-16550" }
fn mem_target(&self) -> Option<Arc<dyn MemTarget>> {
Some(Arc::new(self.clone()))  // 或返回 self 的 Arc
}
fn irq_source_descs(&self) -> &[IrqSourceDesc] {
&[IrqSourceDesc { signal: "irq_out".into() }]
}
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 从 IrqRouter 获取已连接的中断线
self.irq = Some(ctx.irq_router.get_line(&self.id, "irq_out")?);
Ok(())
}
}
```

### 2.4  注册到工厂

```rust
// src/lib.rs
pub fn register(registry: &mut ComponentRegistry) {
registry.register_unit("uart-16550", |config| {
Box::new(Uart::from_config(config)?) as Box<dyn Unit>
});
}
```

### 2.5  单元测试

```rust
#[cfg(test)]
mod tests {
use super::*;
use omnisim_core::test_utils::MockIrqLine;
#[test]
fn test_uart_tx() {
let mut uart = Uart::new("uart0");
uart.irq = Some(MockIrqLine::new());
// 写 TX 寄存器
uart.write(0x00, &[b'H']).unwrap();
assert_eq!(uart.tx_buf.len(), 1);
}
#[test]
fn test_uart_rx_irq() {
let mut uart = Uart::new("uart0");
let irq = MockIrqLine::new();
uart.ier = 0x01;  // 使能 RX 中断
uart.irq = Some(irq.clone());
uart.inject_rx(b'A');
assert!(irq.was_asserted());
}
}
```

## 3  可执行单元开发

可执行单元实现 ExecutionBackend，会被 OmniSim 调度器周期性驱动执行。典型示例是 DMA Engine（不需要完整 CPU TCG，但需要主动搬移数据）。

### 3.1  实现 ExecutionBackend

```rust
pub struct DmaEngine {
id:       UnitId,
// 控制寄存器
src_addr: u64,
dst_addr: u64,
length:   u64,
ctrl:     u8,
// 运行时注入
mem_bus:  Option<Arc<MemBus>>,
irq:      Option<IrqLine>,
}
impl ExecutionBackend for DmaEngine {
fn exec(&mut self, max_cycles: u64) -> ExecResult {
if self.ctrl & 0x01 == 0 {
return ExecResult { cycles: 1, stop: false };
}
// 执行一次 DMA 传输
let mem = self.mem_bus.as_ref().unwrap();
let mut buf = vec![0u8; self.length as usize];
mem.read_bulk(self.src_addr, &mut buf).unwrap();
mem.write_bulk(self.dst_addr, &buf).unwrap();
// 传输完成，触发中断
self.ctrl &= !0x01;
if let Some(irq) = &self.irq { irq.assert(); }
ExecResult { cycles: self.length, stop: false }
}
fn backend_kind(&self) -> BackendKind { BackendKind::Functional }
}
```

### 3.2  realize 时获取 MemBus 引用

```rust
impl Unit for DmaEngine {
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
self.mem_bus = Some(ctx.mem_bus("main-mem-bus")?);
self.irq     = Some(ctx.irq_router.get_line(&self.id, "irq_done")?);
Ok(())
}
}
```

## 4  Composite Unit 开发

Composite Unit 内部包含多个子 Unit，对外是一个整体（如 GPGPU 卡对 Host 是一个 PCIe EP）。

### 4.1  结构设计原则

```
Composite Unit 三要素
子 Unit 列表（sub_units）：内部所有组成单元
内部 Bus：子 Unit 之间的连接，对外不可见
边界接口（Port）：对外暴露的 MemTarget / IrqSource / ResetSink 等
```

### 4.2  DPU 示例骨架

```rust
pub struct Dpu {
id:       UnitId,
// 子 Unit
arm_ctrl:    ArmCore,
virtio_net:  VirtioNet,
virtio_blk:  VirtioBlk,
nvmeof:      NvmeOfInitiator,
net_switch:  NetSwitch,
dma_engine:  DmaEngine,
// 内部 Bus
internal_mem_bus: MemBus,
packet_bus:       PacketBus,    // omnisim-dpu-core 组件层
// 对外接口（PCIe EP）
bars:     Vec<BarDesc>,
}
impl Unit for Dpu {
fn sub_units(&self) -> Vec<&dyn Unit> {
vec![&self.arm_ctrl, &self.virtio_net,
&self.virtio_blk, &self.nvmeof,
&self.net_switch, &self.dma_engine]
}
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 1. realize 内部 MemBus
self.internal_mem_bus.realize()?;
// 2. 按顺序 realize 子 Unit，注入内部 Bus 引用
self.arm_ctrl.realize_with_bus(&self.internal_mem_bus)?;
self.virtio_net.realize_with_bus(&self.internal_mem_bus)?;
// 3. 注册到外部 PcieBus
ctx.pcie_bus.register_endpoint(self.id.clone(), self)?;
Ok(())
}
}
impl PcieEndpoint for Dpu {
fn bar_descs(&self) -> &[BarDesc] { &self.bars }
fn on_bar_write(&self, bar_idx: u8, offset: u64, data: &[u8]) -> OmniResult<()> {
// Host 写 DPU 控制寄存器
self.internal_mem_bus.write(offset, data)
}
// 反向 DMA：DPU 主动读 Host vring
fn dma_read(&self, host_addr: u64, buf: &mut [u8]) -> OmniResult<()> {
self.pcie_bus_ref.ep_dma_read(&self.id, host_addr, buf)
}
}
```

## 5  SIMT 执行单元开发（GPGPU SM）

GPGPU 的 SM（Streaming Multiprocessor）是特殊的可执行单元，需要实现 SIMT 并行执行语义。OmniSim 提供两条路线，组件开发者按需选择。

### 5.1  路线 A：SimtHartBackend（RISC-V hart 阵列）

用 RISC-V hart 阵列模拟 SM 内的线程，依托 TCG 已有的 RVV 扩展，适合需要可执行 RISC-V 二进制的场景。

```rust
pub struct SmUnitHart {
id:              UnitId,
threads_per_warp: u32,     // 可配置：32 / 64 / 自定义
warp_count:      u32,
// 每个 thread 对应一个 RISC-V hart
harts:           Vec<RiscvHart>,
// 共享内存（同一 SM 内所有 warp 可见）
shared_mem:      SharedSram,
}
impl ExecutionBackend for SmUnitHart {
fn backend_kind(&self) -> BackendKind { BackendKind::SimtHart }
fn exec(&mut self, max_cycles: u64) -> ExecResult {
// 调度一个 warp：所有 thread 执行同一条指令
let warp = self.next_warp();
let instr = warp.fetch_instruction();
// RVV 向量指令 → 并行执行所有 active thread
for hart in warp.active_threads() {
hart.exec_one(instr);
}
ExecResult { cycles: 1, stop: false }
}
}
```

### 5.2  路线 B：SimtWarpBackend（Warp 调度器）

用 Warp 调度器 + 向量执行单元直接模拟 SIMT 语义，通过 GpuFrontend trait 支持自定义 ISA（不依赖 RISC-V）。

```rust
/// GPU ISA 前端接口：插拔不同 ISA 解码器
pub trait GpuFrontend: Send + Sync {
fn decode(&self, pc: u64, mem: &[u8]) -> GpuInstr;
fn isa_name(&self) -> &'static str;
}
pub struct SmUnitWarp {
id:              UnitId,
threads_per_warp: u32,
warp_count:      u32,
frontend:        Box<dyn GpuFrontend>,  // 插拔 ISA
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

## 6  常见集成模式

### 6.1  接入 ResetBus

```rust
// 组件需要接受外部复位信号
impl ResetSink for MyUnit {
fn assert_reset(&self) -> OmniResult<()> {
// 停止执行，保存/清零寄存器状态
self.state.lock().reset();
Ok(())
}
fn deassert_reset(&self) -> OmniResult<()> {
// 重新开始执行
self.start_execution();
Ok(())
}
fn is_in_reset(&self) -> bool { self.state.lock().in_reset }
}
impl Unit for MyUnit {
fn reset_sink(&self) -> Option<Arc<dyn ResetSink>> {
Some(Arc::new(self.reset_sink_impl.clone()))
}
// 如果本组件也能控制下级复位
fn reset_source_descs(&self) -> &[ResetSourceDesc] {
&[ResetSourceDesc { signal_name: "sub_reset".into() }]
}
}
```

### 6.2  接入 PowerBus（嵌入式 SoC）

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

### 6.3  实现 PCIe EP（含反向 DMA）

```rust
// BAR 描述：对外暴露控制寄存器区域
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
// 反向 DMA：设备主动访问 Host 内存
fn dma_read(&self, host_addr: u64, buf: &mut [u8]) -> OmniResult<()> {
// 通过 PcieBus 路由，经过 IOMMU 检查
self.pcie_bus.ep_dma_read(&self.id, host_addr, buf)
}
fn dma_write(&self, host_addr: u64, buf: &[u8]) -> OmniResult<()> {
self.pcie_bus.ep_dma_write(&self.id, host_addr, buf)
}
}
```

### 6.4  接入 InterconnectBus（卡间互联）

```rust
// 注册到 InterconnectBus，允许被其他节点访问
impl Unit for MyAccel {
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 声明本节点在 InterconnectBus 上的地址窗口
if let Some(ic_bus) = ctx.interconnect_bus("gpu-fabric") {
ic_bus.register_node(InterconnectNode {
unit_id:   self.id.clone(),
node_kind: NodeKind::Accelerator,
base_addr: self.vram_base,
size:      self.vram_size,
accessible_from: None,  // 所有节点可访问
})?;
}
Ok(())
}
}
// 跨卡访问另一节点的内存
fn peer_read(ic_bus: &InterconnectBus, peer: &UnitId, addr: u64) -> Vec<u8> {
let mut buf = vec![0u8; 64];
ic_bus.backend.read(&my_id, peer, addr, &mut buf).unwrap();
buf
}
```

### 6.5  接入 NetworkBackend

```rust
// DPU 的 nvme-of initiator 使用 NetworkBackend 访问远端存储
pub struct NvmeOfInitiator {
backend: Box<dyn NetworkBackend>,
}
impl NvmeOfInitiator {
pub fn read_block(&self, lba: u64, buf: &mut [u8]) -> OmniResult<()> {
// RDMA read：直接从远端存储服务器读取块数据
let remote_addr = lba * 512;
self.backend.rdma_read(remote_addr, buf)
}
}
```

## 7  测试框架与发布约定

### 7.1  三层测试策略

| 层次 | 测试范围 | 使用工具 | 运行时机 |
| --- | --- | --- | --- |
| 单元测试 | 单个 Unit 内部逻辑 | MockBus + MockIrqLine + MockResetBus | cargo test（每次 commit） |
| 集成测试 | 多个 Unit 组合 + Bus 连接 | RealizeCtx + MockBackend + 最小 Manifest | CI PR 合并前 |
| 场景测试 | 完整 Manifest 加载运行 | omnisim-runtime + 测试 fixture Manifest | Nightly / Release 前 |

### 7.2  单元测试模板

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

### 7.3  Manifest 集成测试

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
// 向 UART 注入 RX 数据，期望 GIC SPI 33 被拉高
let uart = platform.unit_mut("uart0").unwrap();
uart.inject_rx(b'A');
let gic = platform.unit("gicv3").unwrap();
assert!(gic.spi_pending(33));
}
```

### 7.4  发布约定

```
组件发布规则
版本号遵循 SemVer：破坏性接口变更 → Major 版本升，新增接口 → Minor，修复 → Patch
每个组件 crate 独立版本，不与框架版本绑定
组件只能依赖 omnisim-core，不能依赖其他组件 crate（通过 Cargo.toml 工具链强制检查）
公开 API 必须有文档注释（/// doc comment），CI 检查 rustdoc --deny=missing_docs
每个 PR 必须包含对应测试，覆盖率不低于现有水平
```

### 7.5  组件命名规范

| 类型 | 命名模式 | 示例 |
| --- | --- | --- |
| CPU 类 | omnisim-cpu-{isa} | omnisim-cpu-arm、omnisim-cpu-riscv |
| 中断控制器 | omnisim-intc-{name} | omnisim-intc-gicv3、omnisim-intc-plic |
| 内存类 | omnisim-mem-{type} | omnisim-mem-dram、omnisim-mem-sram |
| IO 类 | omnisim-io-{name} | omnisim-io-uart、omnisim-io-pcie |
| 加速器类 | omnisim-accel-{name} | omnisim-accel-gpgpu、omnisim-accel-dpu |
| SoC 级 | omnisim-soc-{name} | omnisim-soc-core |
| 网络类 | omnisim-net-{name} | omnisim-net-hwfwd |
| 存储类 | omnisim-storage-{name} | omnisim-storage-nand |
| 安全类 | omnisim-security-{name} | omnisim-security-psp、omnisim-security-rmm |

## 8  CLI 快速参考

OmniSim 的主要配置方式是 Manifest 文件（详见 DOC-04 应用场景手册）。CLI 只是启动入口，参数较少。

### 8.1  基本用法

```shell
# 从 Manifest 文件启动
omni-cli --manifest platform.toml
# 指定 Profile（覆盖 Manifest 中的默认 Profile）
omni-cli --manifest platform.toml --profile server
# QEMU 兼容模式（libvirt 调用路径）
omnisim-aarch64 -machine virt -cpu cortex-a57 -m 4096 \
-device virtio-blk-pci,drive=d0 \
-qmp unix:/var/run/omnisim.sock,server
# 调试模式：打印 realize 后的完整拓扑
omni-cli --manifest platform.toml --dump-topology
# 验证 Manifest 但不运行
omni-cli --manifest platform.toml --validate-only
```

### 8.2  常用选项

| 选项 | 说明 | 默认值 |
| --- | --- | --- |
| --manifest <path> | Manifest 文件路径（TOML 或 JSON） | 无（必填） |
| --profile <name> | 覆盖 Manifest 中的 profile 选择 | Manifest 中声明的默认 profile |
| --log-level <lvl> | 日志级别：error/warn/info/debug/trace | info |
| --qmp <path> | QMP socket 路径 | 不开启 |
| --dump-topology | 打印 realize 后的 Unit/Bus 拓扑并退出 | 关闭 |
| --validate-only | 只做 Manifest 语法和语义校验，不启动仿真 | 关闭 |
