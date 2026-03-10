> [English](./02-interface-spec.md)

---

# OmniSim — 接口规范

> **文档编号**：DOC-02 &nbsp;&nbsp; **版本**：v1.0 &nbsp;&nbsp; **日期**：2026-Q1 &nbsp;&nbsp; **作者**：OpenFunTech

---

| 文档编号 | DOC-02 |
| --- | --- |
| 文档版本 | v1.0 |
| 适用读者 | 框架开发者（实现 omnisim-core）· 组件开发者（依赖接口） |
| 前置阅读 | DOC-01 架构设计 |
| 关联文档 | DOC-03 组件开发指南 · DOC-04 应用场景手册 |
| 上次更新 | 2026-Q1 |

## 4  omnisim-core 接口定义

```
omnisim-core 设计约定
零业务依赖（仅允许 std + thiserror + bitflags）
所有 trait 要求 Send + Sync + 'static
所有接口方法不 panic，通过 OmniResult 返回错误
此 crate 是所有其他 crate 的唯一共同依赖
```

### 4.1  ExecutionBackend — 执行后端

所有"会动"的硬件单元的统一接口。CPU、NPU、DPU、DMA 引擎都实现此 trait，框架完全不知道内部做了什么。

```rust
// omnisim-core/src/backend/execution.rs
```

```
/// 执行后端核心 trait
/// 框架只通过此接口驱动 Unit 执行
pub trait ExecutionBackend: Send + Sync {
/// 驱动执行，给定时间预算（纳秒）
/// 返回值：实际消耗的仿真时间（纳秒）
///   - 返回值 <= budget_ns（不能超出预算）
///   - 返回 0 表示当前无事可做（Unit 在等待外部事件）
///   - 返回 ExecFault 表示遇到需要上层处理的异常
fn exec(&mut self, budget_ns: u64) -> OmniResult<u64>;

/// 复位到初始状态
/// 必须幂等：多次调用结果相同
fn reset(&mut self) -> OmniResult<()>;

/// 保存执行状态（用于迁移/快照）
fn save_state(&self) -> OmniResult<Vec<u8>>;

/// 恢复执行状态
fn load_state(&mut self, data: &[u8]) -> OmniResult<()>;

/// 后端类型名（调试用）
fn backend_name(&self) -> &'static str;

/// 是否支持暂停/继续（KVM 需要，TCG 总是支持）
fn supports_pause(&self) -> bool { true }
fn pause(&mut self)  -> OmniResult<()> { Ok(()) }
fn resume(&mut self) -> OmniResult<()> { Ok(()) }
}

/// 调试接口（可选，不是所有 Backend 都实现）
pub trait DebugBackend: ExecutionBackend {
/// 读取寄存器（reg_id 由各 Backend 定义）
fn read_reg(&self, reg_id: u64) -> OmniResult<u64>;
fn write_reg(&mut self, reg_id: u64, val: u64) -> OmniResult<()>;

/// 读取当前 PC
fn pc(&self) -> u64;

/// 插入软件断点
fn insert_breakpoint(&mut self, addr: u64) -> OmniResult<()>;
fn remove_breakpoint(&mut self, addr: u64) -> OmniResult<()>;

/// 单步执行（执行一条指令后返回）
fn single_step(&mut self) -> OmniResult<StepResult>;
}

#[derive(Debug)]
pub enum StepResult {
Ok { pc_after: u64, insn_bytes: Vec<u8> },
Fault(FaultKind),
}
```

### 4.2  TimingModel — 时序模型

与 ExecutionBackend 正交。描述 Unit 如何参与全局时间推进，是 SyncBus 协调混合精度仿真的基础。

```rust
// omnisim-core/src/backend/timing.rs
```

```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimingKind {
/// 不关心时间；exec() 被调用时立即处理
/// 适用：功能仿真 CPU、UART、简单外设
Untimed,

/// 事务级时序；exec() 处理一批事务后上报消耗时间
/// 适用：Cache、DDR、NPU 性能模型
LooseTiming,

/// 周期精确；每个时钟周期都必须与 SyncBus 同步
/// 适用：RTL on FPGA，是 SyncBus 的节拍器
CycleAccurate,
}

pub trait TimingModel: Send + Sync {
fn timing_kind(&self) -> TimingKind;

/// 当前仿真时间（纳秒）
fn now_ns(&self) -> u64;

/// SyncBus 调用：请推进到 target_ns
/// Untimed: 直接更新内部时钟
/// LooseTiming: 处理积压事务，更新时钟，返回实际到达时间
/// CycleAccurate: 驱动 FPGA 执行到对应周期数
fn advance_to(&mut self, target_ns: u64) -> OmniResult<u64>;

/// 下一个需要 SyncBus 关注的事件时间
/// None 表示无待处理事件
fn next_event_ns(&self) -> Option<u64>;

/// 时钟频率（Hz），CycleAccurate 必须实现
fn clock_hz(&self) -> Option<u64> { None }
}
```

### 4.3  MemTarget / MemSpace — 内存子系统

```rust
// omnisim-core/src/mem.rs
```

```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemAccessSize { U8 = 1, U16 = 2, U32 = 4, U64 = 8, U128 = 16 }

#[derive(Debug, Clone, Copy)]
pub struct MemAccess {
pub addr:    u64,
pub size:    MemAccessSize,
pub is_write:bool,
/// 发起者 Unit id（用于 MemBus visible_to 过滤）
pub initiator_id: u32,
}

/// 可被地址访问的单元
/// 必须线程安全；read 不修改可观察状态（纯读语义）
pub trait MemTarget: Send + Sync {
/// 在 realize 时由 MemBus 分配的唯一 ID
fn target_id(&self) -> u32;

/// 此 Target 覆盖的地址范围（[base, base+size)）
fn base(&self) -> u64;
fn size(&self) -> u64;

fn read (&self,  offset: u64, size: MemAccessSize) -> OmniResult<u64>;
fn write(&self,  offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()>;

/// 批量访问（DMA / TCG 块操作优化路径）
fn read_bulk (&self,  offset: u64, buf: &mut [u8]) -> OmniResult<()> {
// 默认实现：逐字节调用 read()
for (i, b) in buf.iter_mut().enumerate() {
*b = self.read(offset + i as u64, MemAccessSize::U8)? as u8;
}
Ok(())
}
fn write_bulk(&self, offset: u64, buf: &[u8])      -> OmniResult<()> {
for (i, b) in buf.iter().enumerate() {
self.write(offset + i as u64, MemAccessSize::U8, *b as u64)?;
}
Ok(())
}

/// 直接内存访问（RAM 优化：返回指针，避免拷贝）
/// 只有 RAM 类 Target 实现此方法
fn as_raw_ptr(&self) -> Option<*mut u8> { None }
}

/// 地址空间：MemTarget 的路由表
/// 每个 Initiator Unit 持有自己独立的 MemSpace 实例
/// 同一个 MemTarget 可以注册到多个 MemSpace（共享内存）
pub struct MemSpace {
id:        String,
mappings:  Vec<Mapping>,
/// 地址转换钩子（MMU 接入点）
translator: Option<Box<dyn AddressTranslator>>,
}

struct Mapping {
target:     Arc<dyn MemTarget>,
host_base:  u64,         // 在此 MemSpace 中的基地址
guest_base: u64,         // Target 内部偏移起点
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

/// 注册 MemTarget
pub fn map(
&mut self,
target:    Arc<dyn MemTarget>,
host_base: u64,
attrs:     MemAttrs,
) -> OmniResult<()>;

/// 取消注册
pub fn unmap(&mut self, host_base: u64) -> OmniResult<()>;

/// 物理地址读写（经过 translator 转换）
pub fn read (&self, addr: u64, size: MemAccessSize, access: &MemAccess) -> OmniResult<u64>;
pub fn write(&self, addr: u64, size: MemAccessSize, val: u64, access: &MemAccess) -> OmniResult<()>;

/// 安装 MMU/IOMMU 地址转换器
pub fn set_translator(&mut self, t: Box<dyn AddressTranslator>);

/// 查询地址映射（调试用）
pub fn query_mapping(&self, addr: u64) -> Option<MappingInfo>;
}

/// MMU/IOMMU 地址转换接口
pub trait AddressTranslator: Send + Sync {
/// 虚拟地址 → 物理地址
/// 返回 None 表示地址转换失败（page fault）
fn translate(&self, va: u64, access: &MemAccess) -> OmniResult<u64>;

/// TLB 失效通知（写 CR3 / SATP 时调用）
fn flush_tlb(&mut self, asid: Option<u64>);
}
```

### 4.4  IrqLine / IrqSink / IrqSource — 中断子系统

```rust
// omnisim-core/src/irq.rs
```

```
/// 中断接收端
pub trait IrqSink: Send + Sync {
/// 拉高中断线（电平触发：高电平有效）
fn raise(&self, irq_num: u32) -> OmniResult<()>;
/// 拉低中断线
fn lower(&self, irq_num: u32) -> OmniResult<()>;
/// 边沿触发（正脉冲）
fn pulse(&self, irq_num: u32) -> OmniResult<()> {
self.raise(irq_num)?;
self.lower(irq_num)
}
/// 此 Sink 接受的中断号范围
fn irq_count(&self) -> u32;
}

/// 中断发送端（持有对 Sink 的引用）
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

/// MSI 描述（PCIe MSI/MSI-X）
#[derive(Debug, Clone)]
pub struct MsiDesc {
pub addr:    u64,   // MSI 写入地址
pub data:    u32,   // MSI 写入数据
pub vector:  u32,
}

/// MSI 发送接口（PCIe EP 使用）
pub trait MsiSender: Send + Sync {
fn send_msi(&self, desc: &MsiDesc) -> OmniResult<()>;
}
```

### 4.5  Unit Trait — 核心组合接口

所有硬件实体的统一抽象。ARM CPU、NPU、DPU、UART 在框架层面通过此 trait 完全对等。

```rust
// omnisim-core/src/unit.rs
```

```
/// Unit ID：全局唯一，层次化命名
/// 格式："soc0.arm_ss.core0"、"gpgpu0.sm[3].alu"
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitId(String);

impl UnitId {
pub fn new(s: impl Into<String>) -> Self;
pub fn as_str(&self) -> &str;
/// 父 ID："soc0.arm_ss.core0" → Some("soc0.arm_ss")
pub fn parent(&self) -> Option<UnitId>;
/// 子 ID："soc0" + "arm_ss" → "soc0.arm_ss"
pub fn child(&self, name: &str) -> UnitId;
}

/// Unit 能力标志（位掩码）
bitflags::bitflags! {
pub struct UnitCaps: u32 {
/// 可执行：实现 ExecutionBackend
const EXECUTABLE  = 0x01;
/// 可寻址：实现 MemTarget（可被 MemBus 挂载）
const ADDRESSABLE = 0x02;
/// 中断源：可发出 IrqLine
const IRQ_SOURCE  = 0x04;
/// 中断汇：实现 IrqSink
const IRQ_SINK    = 0x08;
/// 复合单元：包含 sub_units
const COMPOSITE   = 0x10;
/// 可调试：实现 DebugBackend
const DEBUGGABLE  = 0x20;
/// 支持快照
const SNAPSHOTTABLE = 0x40;
}
}

/// 所有硬件实体的统一接口
pub trait Unit: Send + Sync {
// ── 身份 ───────────────────────────────────────────
fn id(&self)   -> &UnitId;
fn kind(&self) -> &'static str;
fn caps(&self) -> UnitCaps;

// ── 生命周期 ────────────────────────────────────────
/// realize：连接 Bus、初始化内部状态
/// 在所有 sub_unit realize 之后调用
fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()>;

/// unrealize：断开连接、释放外部资源
fn unrealize(&mut self) -> OmniResult<()>;

/// reset：复位到上电初始状态（保留连接）
fn reset(&mut self) -> OmniResult<()>;

// ── 能力接口（按 caps() 返回相应实现）────────────────
fn execution_backend(&mut self)
-> Option<&mut dyn ExecutionBackend> { None }

fn timing_model(&mut self)
-> Option<&mut dyn TimingModel> { None }

fn as_mem_target(&self)
-> Option<&dyn MemTarget> { None }

fn as_irq_sink(&self)
-> Option<Arc<dyn IrqSink>> { None }

/// 此 Unit 可发出的 IrqLine 描述
fn irq_source_descs(&self) -> &[IrqSourceDesc] { &[] }

// ── Composite Unit ──────────────────────────────────
fn sub_units(&self)
-> Option<&[Box<dyn Unit>]> { None }
fn sub_units_mut(&mut self)
-> Option<&mut Vec<Box<dyn Unit>>> { None }

// ── 状态持久化 ──────────────────────────────────────
fn save_snapshot(&self)  -> OmniResult<UnitSnapshot> {
Err(OmniError::Internal("snapshot not supported".into()))
}
fn load_snapshot(&mut self, _snap: UnitSnapshot) -> OmniResult<()> {
Err(OmniError::Internal("snapshot not supported".into()))
}

// ── 属性系统（QMP query 用）─────────────────────────
fn get_property(&self, _name: &str) -> Option<PropertyValue> { None }
fn set_property(&mut self, _name: &str, _val: PropertyValue)
-> OmniResult<()> {
Err(OmniError::Internal("property not supported".into()))
}
}

/// realize 上下文：provide Bus 连接能力
pub struct RealizeCtx<'a> {
/// 此 Unit 所属的 MemSpace（Initiator 角色）
pub mem_space:   Option<&'a mut MemSpace>,
/// 全局 IrqRouter（用于连接中断线）
pub irq_router:  &'a mut IrqRouter,
/// 此 Unit 的配置参数
pub config:      &'a UnitConfig,
/// 父 Unit ID（Composite Unit 内部 realize 时使用）
pub parent_id:   Option<&'a UnitId>,
/// 全局 Unit 注册表（用于查找其他 Unit）
pub unit_reg:    &'a UnitRegistry,
}

#[derive(Debug, Clone)]
pub struct IrqSourceDesc {
/// 在 Unit 内部的名字，如 "irq_out"、"msi[0]"
pub name:    String,
pub kind:    IrqKind,
}

#[derive(Debug, Clone, Copy)]
pub enum IrqKind { LevelSensitive, EdgeTriggered, Msi }
```

### 4.6  Dataflow 接口 — 高带宽数据流

用于 NPU/DPU 等加速器与内存之间的大块数据传输，与 MemBus 分离以避免 MMIO 路径的开销。

```rust
// omnisim-core/src/dataflow.rs
```

```
/// 数据流端点（生产者或消费者）
pub trait DataEndpoint: Send + Sync {
fn endpoint_id(&self) -> u32;

/// 生产者：提交一批数据
fn produce(&self, desc: DataDesc) -> OmniResult<DataToken>;

/// 消费者：取回处理结果
fn consume(&self, token: DataToken) -> OmniResult<DataResult>;

/// 轮询：是否有数据/结果就绪
fn poll_ready(&self) -> bool;
}

#[derive(Debug)]
pub struct DataDesc {
pub src_addr:  u64,        // 源地址（在 Initiator 的 MemSpace 中）
pub dst_addr:  u64,        // 目标地址
pub len:       usize,
pub op:        DataOp,     // 操作类型
pub metadata:  Vec<u8>,    // 算子参数（NPU）/ 数据包头（DPU）
}

#[derive(Debug, Clone, Copy)]
pub enum DataOp {
DmaRead, DmaWrite, DmaCopy,
NpuInference,              // NPU 算子推理
DpuPacketProcess,          // DPU 数据包处理
Custom(u32),
}
```

### 4.7  UnitRegistry — 全局单元注册表

```rust
// omnisim-core/src/registry.rs
```

```
/// 运行时 Unit 注册表
/// 支持按 ID 查找、按 kind 查找、按 caps 过滤
pub struct UnitRegistry {
units: HashMap<UnitId, Arc<Mutex<Box<dyn Unit>>>>,
}

impl UnitRegistry {
pub fn register(&mut self, unit: Box<dyn Unit>) -> OmniResult<()>;
pub fn unregister(&mut self, id: &UnitId) -> OmniResult<()>;

pub fn get(&self, id: &UnitId)
-> Option<Arc<Mutex<Box<dyn Unit>>>>;

/// 查找所有实现特定 caps 的 Unit
pub fn find_by_caps(&self, caps: UnitCaps)
-> Vec<Arc<Mutex<Box<dyn Unit>>>>;

/// 查找所有特定 kind 的 Unit
pub fn find_by_kind(&self, kind: &str)
-> Vec<Arc<Mutex<Box<dyn Unit>>>>;

/// 获取所有可执行 Unit（调度器使用）
pub fn executable_units(&self)
-> Vec<Arc<Mutex<Box<dyn Unit>>>>;
}
```

### 4.8 安全执行接口（P0 预留，P2 实现）

为支持 TrustZone（场景十二）和机密虚拟化（场景十三，P2），在 omnisim-core 预留安全相关接口。P0 阶段接口存在但不产生实际隔离效果（no-op），P2 阶段实现真正的安全域检查。

```
P0/P2 分阶段说明
P0：定义枚举和接口，MemBus 安全检查为 no-op，Unit 默认返回 Normal World
P1：TrustZone SMC/HVC 路径，EL2/EL3 切换（在 omnisim-tcg 实现）
P2：MemBus 实际安全域隔离检查，ARM CCA RMM，AMD SEV-SNP PSP
```

```rust
// omnisim-core/src/security.rs

/// 内存安全属性
/// 附加在 MemAttrs 上，描述内存页属于哪个安全域
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemSecurity {
#[default]
NonSecure,              // 普通内存（默认）
Secure,                 // TrustZone Secure World
Realm,                  // ARM CCA Realm World
EncryptedPrivate(u32),  // AMD SEV：ASID 绑定加密
EncryptedShared,        // AMD SEV：共享加密内存
}

/// CPU 安全世界状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityWorld {
#[default]
Normal,   // Non-Secure EL0/1/2（默认）
Secure,   // TrustZone Secure EL0/1/3
Realm,    // ARM CCA Realm EL0/1/2
Root,     // ARM CCA Root EL3
}
```

#### Unit trait 安全接口扩展

```rust
pub trait Unit: Send + Sync {
// 现有方法不变...

/// CPU 当前所在安全世界（默认 Normal，非 CPU Unit 返回 Normal）
fn security_world(&self) -> SecurityWorld {
SecurityWorld::Normal
}

/// 切换安全世界（SMC/RMI 调用时触发）
/// P0 默认返回 NotSupported，P1/P2 在 TCG 前端实现
fn switch_world(&mut self, _target: SecurityWorld) -> OmniResult<()> {
Err(OmniError::NotSupported("security world switch"))
}
}
```

#### MemAttrs 扩展

```rust
/// 内存访问属性（新增 security 字段）
pub struct MemAttrs {
pub readable:   bool,
pub writable:   bool,
pub executable: bool,
/// 安全属性，默认 NonSecure
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

// MemBus 安全检查（P2 之前 no-op）
impl MemBus {
fn check_security(
&self,
_initiator_world: SecurityWorld,
_target_security: MemSecurity,
) -> OmniResult<()> {
Ok(())   // P2 实现真正的隔离检查
}
}
```

### 4.9 CoherenceDomain — 缓存一致性域（P2 接口占位）

为支持 CXL.cache（场景八，P2），预留缓存一致性域接口。CXL.cache 允许设备缓存 Host 内存，需要一致性协议（snoop/invalidate）支持。P2 之前只定义接口，不接入 MemBus。

```rust
// omnisim-core/src/coherence.rs
// P2 接口占位：定义 trait，暂不实现

#[derive(Debug, Clone, Copy)]
pub enum CoherenceProtocol {
Ace,     // ARM ACE / CHI
Moesi,   // Intel MESIF / AMD MOESI
CxlBi,   // CXL 3.0 Back-Invalidate
}

#[derive(Debug, Clone, Copy)]
pub enum SnoopOp { ReadShared, ReadUnique, Invalidate }

#[derive(Debug, Clone, Copy)]
pub enum SnoopResp { Hit, Miss, HitDirty }

#[derive(Debug, Clone, Copy)]
pub enum CacheOp { Read, Write, Flush }

/// 缓存一致性参与者（CXL 设备、LLC 等实现此 trait）
pub trait CoherenceParticipant: Send + Sync {
fn on_snoop(&self, addr: u64, op: SnoopOp) -> OmniResult<SnoopResp>;
fn request(&self, addr: u64, op: CacheOp)  -> OmniResult<()>;
}

pub struct CoherenceDomain {
pub id:       String,
pub protocol: CoherenceProtocol,
pub members:  Vec<UnitId>,
// P2 之前 members 为空，不产生实际效果
}
```

### 4.10 AddressTranslator Stage-2 扩展（P1）

为支持嵌套虚拟化（场景十一），AddressTranslator 扩展 Stage-2 地址转换接口（IPA → PA）。Stage-2 由 Hypervisor（EL2）控制，用于隔离不同 VM 的物理地址空间。

```
嵌套仿真架构说明
OmniSim 仿真完整物理机（TCG），物理机上跑真实 Hypervisor 软件（KVM / LeoVirt-S）
Hypervisor 管理多个 VM，Stage-2 MMU 是 Hypervisor 隔离 VM 的核心机制
omnisim-core 只需提供接口，具体实现在 omnisim-tcg 的 EL2/VMX 前端（P1）
```

```rust
// omnisim-core/src/mem.rs 扩展

pub trait AddressTranslator: Send + Sync {
/// Stage-1：VA → IPA（Guest OS 页表，EL1 控制）
/// 现有接口，签名调整增加 asid 参数
fn translate_s1(
&self,
va:     u64,
asid:   u16,
access: MemAccessKind,
) -> OmniResult<u64>;

/// Stage-2：IPA → PA（Hypervisor 页表，EL2 控制）
/// P1 新增：vmid 区分不同 VM 的地址空间
fn translate_s2(
&self,
ipa:    u64,
vmid:   u16,
access: MemAccessKind,
) -> OmniResult<u64>;

/// 两阶段级联：VA → IPA → PA
/// Guest OS 正常内存访问路径，默认实现串联 s1 + s2
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

/// TLB flush：VM 销毁或 VMID 回收时调用
fn flush_tlb_vmid(&self, vmid: u16);

/// TLB flush：进程切换时调用
fn flush_tlb_asid(&self, asid: u16);
}
```

#### PrivilegeLevel 枚举（P0 预留）

```rust
/// CPU 特权级（P0 定义，P1 在 TCG 前端使用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
// ARM
EL0, EL1, EL2, EL3,
// x86
Ring0, Ring1, Ring2, Ring3,
RingMinus1,   // VMX non-root（Guest）
// RISC-V
U, S, H, M,
}

pub trait Unit: Send + Sync {
// 现有方法不变...

/// CPU 当前特权级（非 CPU Unit 返回 None）
fn current_privilege(&self) -> Option<PrivilegeLevel> { None }
}
```

## 5  Bus 接口详细设计

```
Bus 设计原则
Bus 是连接关系的显式表达，是一等公民，不藏在 Unit 内部
每种 Bus 独立实现，职责单一
Bus 支持运行时动态添加/移除连接（热插拔）
Bus 连接可以跨越 Composite Unit 边界
```

### 5.1  MemBus — 地址总线

负责地址路由：根据 Initiator 的访问地址找到对应的 MemTarget，并转发读写操作。支持部分可见（visible_to）实现不同子系统的地址空间隔离。

```rust
// omnisim-core/src/bus/mem_bus.rs
```

```
pub struct MemBus {
id:       String,
mappings: Vec<MemMapping>,
/// 地址冲突检测：realize 时检查，运行时跳过
overlap_policy: OverlapPolicy,
}

struct MemMapping {
target:      Arc<dyn MemTarget>,
/// 此映射在 MemBus 的地址空间中的基地址
base:        u64,
size:        u64,
attrs:       MemAttrs,
/// 访问控制：哪些 Initiator 可见此映射
/// None = 所有 Initiator 可见
/// Some([id1,id2]) = 只有指定 Initiator 可见
/// 用于实现：ITCM 只对 RISC-V Core 可见
///           GIC 寄存器只对 ARM Core 可见
visible_to:  Option<Vec<u32>>, // initiator_id
}

#[derive(Clone, Copy)]
pub enum OverlapPolicy {
/// realize 时若有重叠则报错
Strict,
/// 允许重叠，后注册的覆盖先注册的（shadow）
Shadow,
}

impl MemBus {
pub fn new(id: impl Into<String>) -> Self;

/// 注册 Target（realize 阶段）
pub fn map(
&mut self,
target:     Arc<dyn MemTarget>,
base:       u64,
attrs:      MemAttrs,
visible_to: Option<Vec<u32>>,
) -> OmniResult<()>;

/// 动态取消（热拔出）
pub fn unmap(&mut self, base: u64) -> OmniResult<()>;

/// 路由并执行读操作
pub fn read(
&self,
addr:     u64,
size:     MemAccessSize,
initiator_id: u32,
) -> OmniResult<u64>;

/// 路由并执行写操作
pub fn write(
&self,
addr:     u64,
size:     MemAccessSize,
val:      u64,
initiator_id: u32,
) -> OmniResult<()>;

/// 查询地址映射（调试/QMP query-memmap）
pub fn dump_mappings(&self, initiator_id: Option<u32>)
-> Vec<MappingInfo>;

/// 为 Initiator 构建独立的 MemSpace 视图
/// MemSpace 持有对 MemBus 的引用，读写通过 MemBus 路由
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

### 5.2  IrqBus — 中断总线

管理中断线的连接关系。支持一对一、一对多（中断扇出）、多对一（中断汇聚）连接。

```rust
// omnisim-core/src/bus/irq_bus.rs
```

```
pub struct IrqBus {
id:          String,
connections: Vec<IrqConnection>,
}

/// 中断连接：从 source 端点到 sink 端点
pub struct IrqConnection {
id:     u32,
source: IrqEndpoint,
sink:   IrqEndpoint,
}

/// 中断线端点描述
#[derive(Debug, Clone)]
pub struct IrqEndpoint {
/// Unit ID
pub unit_id:  UnitId,
/// Unit 内部的中断信号名
/// 格式："irq_out"、"msi[0]"、"irq[33]"
pub signal:   String,
}

impl IrqBus {
pub fn new(id: impl Into<String>) -> Self;

/// realize：解析 IrqEndpoint，建立实际的 IrqLine 连接
pub fn realize(&mut self, reg: &UnitRegistry) -> OmniResult<()>;

/// 动态添加连接（热插拔设备的中断连接）
pub fn connect(
&mut self,
source: IrqEndpoint,
sink:   IrqEndpoint,
reg:    &UnitRegistry,
) -> OmniResult<u32>;  // 返回 connection_id

/// 断开连接
pub fn disconnect(&mut self, connection_id: u32) -> OmniResult<()>;

/// 查询所有连接（调试用）
pub fn dump_connections(&self) -> Vec<IrqConnectionInfo>;
}
```

### 5.3  PcieBus — PCIe 总线

处理 PCIe 拓扑：Root Complex 与 Endpoint 的连接，BAR 地址映射，MSI 路由，配置空间访问。

```rust
// omnisim-core/src/bus/pcie_bus.rs
```

```
pub struct PcieBus {
id:           String,
root_complex: UnitId,
endpoints:    Vec<PcieEndpointEntry>,
/// BAR 映射：EP 的 BAR 地址 → Host 物理地址
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
pub bar_index:    u8,
pub size:         u64,
/// EP BAR 在 Host 地址空间的映射基地址
pub host_base:    u64,
pub bar_kind:     BarKind,
}

#[derive(Clone, Copy)]
pub enum BarKind {
Mem32, Mem64, Io,
}

/// PCIe EP 需实现的接口（EP Unit 的扩展 trait）
pub trait PcieEndpoint: Unit {
/// 配置空间读写（BDF 寻址）
fn cfg_read (&self,  offset: u16, size: MemAccessSize) -> OmniResult<u64>;
fn cfg_write(&self,  offset: u16, size: MemAccessSize, val: u64) -> OmniResult<()>;

/// BAR 窗口访问（Host 通过 BAR 访问 EP 内部）
fn bar_read (&self,  bar: u8, offset: u64, size: MemAccessSize) -> OmniResult<u64>;
fn bar_write(&self,  bar: u8, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()>;

/// MSI/MSI-X 发送能力
fn msi_sender(&self) -> Option<Arc<dyn MsiSender>>;

/// EP 内部的 MemSpace（Host 通过 BAR 写入时触达 EP 内部总线）
fn internal_mem_space(&mut self) -> &mut MemSpace;
}

/// PCIe RC 需实现的接口
pub trait PcieRootComplex: Unit {
/// ECAM 配置空间访问
fn ecam_read (&self,  bdf: Bdf, offset: u16, size: MemAccessSize) -> OmniResult<u64>;
fn ecam_write(&self,  bdf: Bdf, offset: u16, size: MemAccessSize, val: u64) -> OmniResult<()>;

/// DMA Remapping（IOMMU）
fn iommu(&self) -> Option<Arc<dyn AddressTranslator>>;
}

#[derive(Debug, Clone, Copy)]
pub struct Bdf { pub bus: u8, pub device: u8, pub function: u8 }
```

### 5.4  SyncBus — 时序同步总线

协调混合精度仿真中不同 TimingModel 的全局时序，确保 LooseTiming 和 CycleAccurate Unit 之间的因果关系正确。

```rust
// omnisim-core/src/bus/sync_bus.rs
```

```
pub struct SyncBus {
id:           String,
participants: Vec<SyncParticipant>,
/// 全局推进步长（纳秒）
/// 每次 tick 推进所有 LooseTiming Unit 到 now + quantum
time_quantum_ns: u64,
}

struct SyncParticipant {
unit_id:       UnitId,
timing_model:  Arc<Mutex<dyn TimingModel>>,
/// true = CycleAccurate，是节拍器
/// SyncBus 以节拍器的时间为基准
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

/// 推进一个量子步长
/// 调用顺序：
///   1. 推进 CycleAccurate timekeeper 到 target
///   2. 推进所有 LooseTiming 到同一 target
///   3. Untimed 不参与（由其他 Unit 的 exec() 直接驱动）
pub fn tick(&mut self) -> OmniResult<SyncStats>;

/// 当前全局仿真时间
pub fn global_time_ns(&self) -> u64;
}

#[derive(Debug)]
pub struct SyncStats {
pub global_time_ns: u64,
pub lagging_units:  Vec<UnitId>,  // 没有跟上时序的 Unit
}
```

### 5.5  IrqRouter — 全局中断路由

集中管理所有 IrqBus 的中断连接，提供按名字查找和动态连接功能。

```rust
// omnisim-core/src/bus/irq_router.rs
```

```
pub struct IrqRouter {
/// bus_id → IrqBus
buses: HashMap<String, IrqBus>,
/// (unit_id, signal_name) → IrqLine（缓存，realize 后填充）
line_cache: HashMap<(UnitId, String), IrqLine>,
}

impl IrqRouter {
/// 连接中断线（realize 阶段）
pub fn connect(
&mut self,
source: IrqEndpoint,
sink:   IrqEndpoint,
reg:    &UnitRegistry,
) -> OmniResult<()>;

/// 获取 IrqLine（Unit 在 realize 时调用，保存到自身字段）
pub fn get_line(
&self,
unit_id: &UnitId,
signal:  &str,
) -> OmniResult<IrqLine>;

/// 注册 IrqSink（realize 时由 IrqSink Unit 调用）
pub fn register_sink(
&mut self,
unit_id: UnitId,
signal:  String,
sink:    Arc<dyn IrqSink>,
);
}
```

### 5.6 ResetBus — 复位信号总线

ResetBus 与 IrqBus 完全对称，统一建模所有跨 Unit 的复位关系。覆盖两种物理场景：PCIe PERST#（RC 控制 EP 复位）和片上 Reset Controller（控制多核复位序列）。

```
设计原则
ResetBus 建模的是复位信号的连接关系，不关心是 PCIe PERST# 还是片上 GPIO
PcieBus realize 时自动在 RC 和所有 EP 之间建立 ResetLine，无需手动配置
片上场景：Reset Controller Unit 作为 reset_source，被控核作为 reset_sink
```

```rust
// omnisim-core/src/bus/reset_bus.rs

/// 复位信号接收端（被复位的 Unit 实现此 trait）
pub trait ResetSink: Send + Sync {
fn assert_reset(&self)   -> OmniResult<()>;
fn deassert_reset(&self) -> OmniResult<()>;
fn is_in_reset(&self)    -> bool;
}

/// 一条复位线（从 source 到 sink）
pub struct ResetLine {
pub sink:    Arc<dyn ResetSink>,
pub line_id: String,
}

/// 就绪信号：Unit 上电并初始化完成后通知等待方
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
pub from: UnitId,   // Reset Controller 或 PCIe RC（PERST#）
pub to:   UnitId,   // 被复位的 Unit
}

impl ResetBus {
pub fn assert_all(&self)   -> OmniResult<()>;
pub fn deassert(&self, target: &UnitId) -> OmniResult<()>;
pub fn deassert_all(&self) -> OmniResult<()>;
}
```

#### Unit trait 扩展（ResetBus 相关）

```rust
// Unit trait 新增方法（默认实现为 None / 空切片）
pub trait Unit: Send + Sync {
// 现有方法不变...

/// Unit 作为复位信号接收端
fn reset_sink(&self) -> Option<Arc<dyn ResetSink>> { None }

/// Unit 作为复位信号发出端（描述它能控制哪些复位线）
fn reset_source_descs(&self) -> &[ResetSourceDesc] { &[] }

/// 就绪信号：Unit 完成初始化后 set()，BootSequencer 可以 wait()
fn ready_signal(&self) -> Option<Arc<ReadySignal>> { None }
}

pub struct ResetSourceDesc {
pub signal_name: String,   // 例如 "perst#" / "riscv_reset"
}
```

#### BootStep 新增变体

```rust
pub enum BootStep {
// 现有变体不变...
WriteReg  { unit: String, reg: String, val: String },
WaitIrq   { unit: String, irq: String },

// 新增：复位控制
AssertReset   { bus: String },
DeassertReset { bus: String },

// 新增：等待 PCIe link 建立（PERST# 释放后）
WaitLinksUp { bus: String, timeout_ns: Option<u64> },

// 新增：等待 Unit 就绪信号（固件初始化完成）
WaitReady   { unit: String, timeout_ns: Option<u64> },
}
```

#### Manifest 描述示例

```toml
# PCIe 场景：PERST# 控制 GPGPU 复位
[[buses]]
kind = "reset"
id   = "pcie-reset-bus"
[[buses.connections]]
from = "host.pcie-rc.perst#"
to   = "gpgpu0.reset_in"

# 片上场景：Reset Controller 控制 RISC-V 核
[[buses]]
kind = "reset"
id   = "soc-reset-bus"
[[buses.connections]]
from = "soc.reset-ctrl.riscv_reset"
to   = "soc.riscv-ss.rv64-core.reset_in"

# BootDesc 使用示例
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

### 5.7 PowerBus — 电源域管理总线

PowerBus 与 ResetBus 完全对称，统一建模嵌入式 SoC 中的电源域管理。Power Controller 通过 PowerBus 控制各计算核的上下电和低功耗状态切换。

```
适用场景
嵌入式 SoC（场景五）：ARM 主核动态管理 DSP、NPU、RISC-V 小核的电源域
功耗敏感设备：验证 DVFS / CPUIdle / 运行时电源管理（RTOS/Linux PM）
服务器场景：通常不需要 PowerBus（组件不实现 power_sink，框架忽略）
```

```rust
// omnisim-core/src/bus/power_bus.rs

/// 电源状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
On,
Off,
/// 断电但寄存器状态保留（低功耗待机）
Retention,
}

/// 电源信号接收端（被控制的 Unit 实现此 trait）
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
pub target:     UnitId,   // 被控制的 Unit
pub domain:     String,   // 电源域名称，如 "PD_NPU"
}

impl PowerBus {
pub fn power_on(&self, target: &UnitId)        -> OmniResult<()>;
pub fn power_off(&self, target: &UnitId)       -> OmniResult<()>;
pub fn enter_retention(&self, target: &UnitId) -> OmniResult<()>;
pub fn state(&self, target: &UnitId)           -> OmniResult<PowerState>;
}
```

#### Unit trait 扩展（PowerBus 相关）

```rust
pub trait Unit: Send + Sync {
// 现有方法不变...

/// Unit 作为电源信号接收端（默认 None = 不支持电源管理）
fn power_sink(&self) -> Option<Arc<dyn PowerSink>> { None }
}
```

#### Manifest 描述示例

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

### 5.8 InterconnectBus — 对等节点互联总线

InterconnectBus 建模对等节点之间的互联关系，补充 PcieBus（Host-Device 层次关系）的不足。覆盖多卡 GPU 卡间互联、NUMA 多路服务器、大内存 SoC 计算簇互联、CXL 内存设备等场景。

```
InterconnectBus vs PcieBus
PcieBus：RC（Host）和 EP（Device）之间，有主从层次，Host 控制设备发现和复位
InterconnectBus：节点之间对等互联，没有主从，节点可以是 GPU / CPU Socket / CXL 设备
两者不互斥：GPU 卡通过 PcieBus 连接 Host，同时通过 InterconnectBus 卡间直连
```

```rust
// omnisim-core/src/bus/interconnect_bus.rs

/// 寻址模式：决定内存访问的根本语义
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddrMode {
/// 各节点独立地址空间，通过映射窗口互访
/// 适用：PCIe P2P、NVLink 风格、自定义卡间互联
Windowed,
/// 所有节点共享统一物理地址空间
/// 适用：Infinity Fabric、NUMA、CXL.mem、片上 NoC
Unified,
}

/// 节点类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
/// CPU Socket / NUMA Cluster / 计算簇
CpuSocket,
/// GPU / DPU 等加速卡
Accelerator,
/// CXL 内存扩展设备
MemoryDevice,
}

pub struct InterconnectNode {
pub unit_id:         UnitId,
pub node_kind:       NodeKind,
pub base_addr:       u64,
pub size:            u64,
/// 访问控制白名单（None = 所有节点可访问）
pub accessible_from: Option<Vec<UnitId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterconnectPerf {
/// 带宽（GB/s），Timing 仿真用，功能仿真可省略
pub bandwidth_gbps: f64,
/// 单向延迟（ns），Timing 仿真用
pub latency_ns:     u64,
}

pub struct InterconnectBus {
pub id:        String,
pub addr_mode: AddrMode,
pub nodes:     Vec<InterconnectNode>,
pub perf:      Option<InterconnectPerf>,
/// 可插拔协议后端
pub backend:   Box<dyn InterconnectBackend>,
}
```

#### InterconnectBackend — 可插拔协议接口

```rust
/// 协议后端接口：不同互联协议实现不同 Backend
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

/// 跨节点原子操作（卡间 barrier / 原子加）
fn atomic(
&self,
src:  &UnitId,
dst:  &UnitId,
addr: u64,
op:   AtomicOp,
) -> OmniResult<u64>;
}

/// PCIe P2P 实现：通过 PcieBus 路由
pub struct PciePeerBackend {
pcie_bus: Arc<PcieBus>,
}

/// 高速直连实现：NVLink / Infinity Fabric / 片上 NoC / 自定义
/// 功能仿真语义完全相同，差异只在 perf 参数（带宽/延迟）
pub struct DirectFabricBackend {
shared_space: Arc<MemBus>,
}

impl InterconnectBackend for PciePeerBackend    { /* ... */ }
impl InterconnectBackend for DirectFabricBackend { /* ... */ }
```

#### Manifest 描述示例

```toml
# 多卡 GPU 互联：NVLink 风格（Windowed，高速私有）
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

# NUMA 多路服务器（Unified，统一地址空间）
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

# 大内存 SoC 片上 NoC（Unified，低延迟）
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

### 5.9 PcieEndpoint 反向 DMA 扩展

DPU 场景（场景三）引入了 EP 主动访问 Host 内存的需求：DPU 的 DMA 引擎需要直接读写 Host 内存里的 virtio ring，不经过 BAR 寄存器。这是 PCIe EP→RC 方向的 DMA，与通常的 RC→EP 方向相反。

```rust
// omnisim-core/src/bus/pcie_bus.rs 扩展

pub trait PcieEndpoint: Unit {
// 现有接口不变...
fn bar_descs(&self)      -> &[BarDesc];
fn on_bar_write(&self, bar_idx: u8, offset: u64, data: &[u8]) -> OmniResult<()>;
fn on_bar_read(&self, bar_idx: u8, offset: u64, buf: &mut [u8]) -> OmniResult<()>;

// 新增：EP 主动访问 Host 内存（反向 DMA）
/// 从 Host 内存读取数据（DPU 读 vring avail ring）
fn dma_read(
&self,
host_addr: u64,
buf:       &mut [u8],
) -> OmniResult<()>;

/// 向 Host 内存写入数据（DPU 写 vring used ring）
fn dma_write(
&self,
host_addr: u64,
buf:       &[u8],
) -> OmniResult<()>;
}

impl PcieBus {
/// EP 发起反向 DMA 读（内部经过 IOMMU 权限校验）
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

### 5.10 NetworkBackend — 外部网络接入接口

NetworkBackend 为 DPU 的 nvme-of initiator、网络处理器等组件提供统一的外部网络接入接口，屏蔽宿主机 socket 和仿真网络的差异。

```rust
// omnisim-core/src/network.rs

pub trait NetworkBackend: Send + Sync {
/// 发送数据包到外部网络
fn send_packet(&self, pkt: Packet) -> OmniResult<()>;

/// 注册数据包接收回调
fn on_receive(&self, cb: Box<dyn Fn(Packet) + Send + Sync>);

/// RDMA 读（nvme-of over RDMA）
fn rdma_read(&self, addr: u64, buf: &mut [u8]) -> OmniResult<()>;

/// RDMA 写
fn rdma_write(&self, addr: u64, buf: &[u8]) -> OmniResult<()>;
}

/// 宿主机 socket 实现
/// nvme-of / RDMA 访问真实远端存储服务器
pub struct HostSocketBackend {
fd: std::os::unix::io::RawFd,
}

/// 仿真网络实现
/// 连接到另一个仿真节点的 PacketBus（组件层）
pub struct SimNetworkBackend {
// PacketBus 引用，在 omnisim-dpu-core 组件层定义
tx: std::sync::mpsc::Sender<Packet>,
}

impl NetworkBackend for HostSocketBackend { /* ... */ }
impl NetworkBackend for SimNetworkBackend  { /* ... */ }
```

## 6  omnisim-manifest — L3 平台描述层

```
L3 层职责
L3 的唯一职责：把声明式的 Manifest（JSON/TOML）翻译成运行时的 Unit + Bus 实例（Platform）。L3 知道 Unit 的配置描述，但不知道 Backend 的实现细节；Realizer 通过 ComponentRegistry 找工厂。
```

### 6.1  Manifest 顶层结构

```rust
// omnisim-manifest/src/manifest.rs
```

```
/// 平台完整声明式描述
/// 可序列化为 JSON / TOML，是 CLI 翻译和工具链的目标格式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
pub platform:  PlatformDesc,
/// 模板库（可被 UnitDesc.template 引用，展开后合并 config）
#[serde(default)]
pub templates: HashMap<String, UnitDesc>,
pub units:     Vec<UnitDesc>,
pub buses:     Vec<BusDesc>,
#[serde(default)]
pub boot:      BootDesc,
/// 元信息（版本、来源、签名）
#[serde(default)]
pub meta:      ManifestMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformDesc {
pub name:        String,
pub description: Option<String>,
/// 目标体系结构（用于 CLI 自动推断参数）
pub arch:        Option<String>,   // "aarch64", "x86_64", "riscv64"
/// 全局仿真模式
pub sim_mode:    SimMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum SimMode {
#[default]
Functional,   // 纯功能仿真（Untimed）
Performance,  // 性能仿真（LooseTiming 混合）
Rtl,          // RTL 协同（CycleAccurate 混合）
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestMeta {
pub version:    String,
pub omnisim_version: String,
pub created_by: Option<String>,
pub created_at: Option<String>,
/// SHA256 of content before this field
pub signature:  Option<String>,
}
```

### 6.2  UnitDesc — Unit 声明

```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDesc {
pub id:      String,
pub kind:    String,      // 组件注册名，如 "arm-cortex-a78-core"

/// 引用模板：先展开模板，再用本 desc 的字段覆盖
#[serde(default)]
pub template: Option<String>,

// ── Backend 配置 ──────────────────────────────────────
#[serde(default)]
pub backend:  BackendKind,
#[serde(default)]
pub timing:   TimingKind,

// ── 组件特定配置（由各组件定义 schema）───────────────
#[serde(default)]
pub config:   serde_json::Value,

// ── Composite Unit 子单元 ──────────────────────────────
#[serde(default)]
pub sub_units: Vec<UnitDesc>,

// ── 数量展开：id = "sm[0..127]" → 128 个 sm0..sm127 ──
/// 展开时每个 instance 的 config 中可用 ${index} 变量
#[serde(default)]
pub count:    Option<CountSpec>,

// ── 启动依赖 ──────────────────────────────────────────
/// 此 Unit realize 之前必须先 realize 的 Unit
#[serde(default)]
pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CountSpec {
/// 固定数量：count: 128
Fixed(u32),
/// 范围展开：count: { from: 0, to: 127 }
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

### 6.3  BusDesc — Bus 声明

```
/// 所有 Bus 类型用 tag 区分
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusDesc {
/// 地址总线
Mem {
id:         String,
/// 允许地址重叠（shadow 模式）
#[serde(default)]
overlap:    bool,
/// Initiator Unit id 列表（谁可以发起访问）
initiators: Vec<String>,
targets:    Vec<MemTargetDesc>,
},

/// 中断总线
Irq {
id:          String,
connections: Vec<IrqConnectionDesc>,
},

/// 数据流总线（NPU/DPU 大块传输）
Data {
id:          String,
producers:   Vec<String>,
consumers:   Vec<String>,
},

/// 时序同步总线
Sync {
id:              String,
participants:    Vec<String>,
/// 时序量子步长（纳秒）
#[serde(default = "default_quantum")]
time_quantum_ns: u64,
},

/// PCIe 总线
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
/// 在此 Bus 的地址空间中的基地址
pub base:       HexU64,
/// 覆盖 Unit 默认 size（可选）
pub size:       Option<HexU64>,
/// 访问属性
#[serde(default = "MemAttrs::rw")]
pub attrs:      MemAttrsDesc,
/// 访问控制：只对哪些 Initiator 可见
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

/// 十六进制 u64，支持 "0x80000000" 和 2147483648 两种写法
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HexU64(#[serde(with = "hex_or_dec")] pub u64);
```

### 6.4  BootDesc — 启动配置

```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BootDesc {
/// 启动顺序：按序 realize + reset + exec 各 Unit
/// 省略则由 Realizer 根据 depends_on 自动拓扑排序
#[serde(default)]
pub sequence: Vec<BootStep>,

/// 内核镜像
pub kernel:   Option<ImageDesc>,
/// initrd
pub initrd:   Option<ImageDesc>,
/// DTB
pub dtb:      Option<ImageDesc>,
/// 固件（BIOS/UEFI/SBI）
pub firmware: Option<ImageDesc>,

/// 内核命令行
pub cmdline:  Option<String>,

/// 各 Unit 的初始寄存器值
#[serde(default)]
pub registers: HashMap<String, HashMap<String, HexU64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BootStep {
/// 等待某 Unit 写入特定地址（轮询标志位）
WaitWrite { unit: String, addr: HexU64, val: HexU64, mask: Option<HexU64> },
/// 释放某 Unit 的复位信号
ReleaseReset { unit: String },
/// 延迟 N 纳秒
DelayNs { ns: u64 },
/// 写寄存器
WriteReg { unit: String, reg: String, val: HexU64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageDesc {
pub path:     String,
pub load_addr: Option<HexU64>,
pub format:   ImageFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
#[default] Raw, Elf, Uimage, Pe,
}
```

### 6.5  Validator — Manifest 校验

realize 之前先做静态校验，将错误精确到字段，避免运行时才发现配置问题。

```rust
// omnisim-manifest/src/validator.rs
```

```
pub struct Validator {
/// 已知的 Unit kind（从 ComponentRegistry 获取）
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
UnresolvedRef,         // Bus 引用了不存在的 Unit id
CircularDependency,
MissingRequiredField,
InvalidAddressRange,   // Base + size 溢出或重叠
IrqEndpointNotFound,
TemplateNotFound,
ConfigSchemaViolation, // 与组件 JSON Schema 不符
}

/// 校验规则（按顺序执行）：
/// 1. 所有 unit.id 唯一（含展开后的 count 实例）
/// 2. 所有 unit.kind 已在 ComponentRegistry 注册
/// 3. 所有 template 引用可解析
/// 4. 所有 depends_on 引用存在，无环
/// 5. 所有 Bus 引用的 Unit id 存在
/// 6. Mem Bus：地址区间不重叠（除非 overlap=true）
/// 7. Irq Bus：IrqEndpoint 格式合法，Unit 声明了对应 irq_source/sink
/// 8. PCIe：BAR 地址不与普通 MemBus 冲突
/// 9. 各 UnitDesc.config 满足组件注册的 JSON Schema
```

### 6.6  Realizer — Manifest 到 Platform

```rust
// omnisim-manifest/src/realizer.rs
```

```
/// 组件工厂函数类型
pub type UnitFactory = Box<dyn Fn(&UnitDesc) -> OmniResult<Box<dyn Unit>> + Send + Sync>;

/// 组件注册表：kind → factory
pub struct ComponentRegistry {
factories:   HashMap<String, UnitFactory>,
/// 各 kind 的 JSON Schema（用于 Validator）
schemas:     HashMap<String, serde_json::Value>,
/// 各 kind 支持的 BackendKind 集合
backends:    HashMap<String, HashSet<BackendKind>>,
}

impl ComponentRegistry {
pub fn new() -> Self;

/// 注册组件（各 component crate 的 register() 函数调用此方法）
pub fn register(
&mut self,
kind:    &'static str,
factory: UnitFactory,
schema:  Option<serde_json::Value>,
backends: &[BackendKind],
);

pub fn known_kinds(&self) -> impl Iterator<Item = &str>;
pub fn get_factory(&self, kind: &str) -> Option<&UnitFactory>;
pub fn get_schema (&self, kind: &str) -> Option<&serde_json::Value>;
}

/// Realizer：把 Manifest 转换为运行时 Platform
pub struct Realizer {
registry: Arc<ComponentRegistry>,
}

impl Realizer {
pub fn new(registry: Arc<ComponentRegistry>) -> Self;

/// 主入口：验证 + 展开模板 + 实例化所有 Unit + 连接所有 Bus
pub fn realize(&self, manifest: &Manifest) -> OmniResult<Platform>;

// ── 内部步骤（pub 以供测试）──────────────────────────

/// 步骤 1：展开 template 引用 + count 数量展开
pub fn expand(&self, manifest: &Manifest) -> OmniResult<Manifest>;

/// 步骤 2：拓扑排序（按 depends_on）
pub fn topo_sort(units: &[UnitDesc]) -> OmniResult<Vec<usize>>;

/// 步骤 3：按拓扑顺序实例化 Unit
pub fn instantiate(
&self,
units: &[UnitDesc],
order: &[usize],
) -> OmniResult<Vec<Box<dyn Unit>>>;

/// 步骤 4：realize 每个 Unit（注入 RealizeCtx）
pub fn wire_units(
units:      &mut Vec<Box<dyn Unit>>,
buses:      &[BusDesc],
irq_router: &mut IrqRouter,
registry:   &UnitRegistry,
) -> OmniResult<()>;

/// 步骤 5：构建 Bus 实例并连接
pub fn build_buses(
buses:    &[BusDesc],
registry: &UnitRegistry,
) -> OmniResult<BusSet>;
}

/// 运行时平台：realize 的产物
pub struct Platform {
pub name:       String,
pub unit_reg:   UnitRegistry,
pub buses:      BusSet,
pub irq_router: IrqRouter,
pub boot:       BootDesc,
/// 原始 Manifest（用于热插拔时的增量 realize）
pub manifest:   Manifest,
}

impl Platform {
/// 热插拔：realize 新 UnitDesc + BusDesc 并接入运行时
pub fn hot_add(
&mut self,
unit: UnitDesc,
buses: Vec<BusDesc>,
realizer: &Realizer,
) -> OmniResult<UnitId>;

/// 热拔出：unrealize Unit，从 Bus 移除
pub fn hot_remove(&mut self, unit_id: &UnitId) -> OmniResult<()>;

/// 序列化当前状态（用于迁移）
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

### 6.7  组件自注册模式

每个 component crate 提供一个 register() 函数，在 binary 启动时调用，实现零侵入注册：

```rust
// components/cpu/omnisim-cpu-arm/src/lib.rs
```

```
/// ARM CPU 组件注册入口
/// 在 omnisim-runtime 的 main() 中调用：
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
// ... 其他 ARM 变体
}

static SCHEMA_JSON: &str = r#"{
"type": "object",
"properties": {
"num_cores": { "type": "integer", "minimum": 1, "maximum": 256 },
"el2_enable": { "type": "boolean", "default": false },
"sve_vl":   { "type": "integer", "enum": [128, 256, 512, 1024] }
},
"required": ["num_cores"]
}"#;
```

## 7  omnisim-runtime — 运行时与调度层

```
runtime 层职责
omnisim-runtime 是整个仿真平台的"发动机"：持有 Platform 实例，驱动所有可执行 Unit 的 exec()，协调 SyncBus 时序，处理 VM exit，响应外部控制命令（暂停/继续/快照）。
```

### 7.1  整体执行模型

```
┌────────────────────────────────┐
│         Scheduler              │
│  ┌──────────────────────────┐  │
│  │   RunLoop（主控线程）      │  │
│  │   - 处理控制命令           │  │
│  │   - 驱动 SyncBus tick     │  │
│  │   - 监控各 Unit 线程状态   │  │
│  └──────────────────────────┘  │
┌──────────┐      │  ┌─────┐ ┌─────┐ ┌─────────┐  │
│ QMP/CLI  │─────▶│  │cpu0 │ │cpu1 │ │ npu0    │  │
│ control  │      │  │thread│ │thread│ │ thread  │  │
└──────────┘      │  └─────┘ └─────┘ └─────────┘  │
│         UnitThread × N         │
└────────────────────────────────┘

每个可执行 Unit 运行在独立线程（UnitThread）
RunLoop 通过 channel 发控制命令（Pause/Resume/Reset）
UnitThread 通过 channel 上报事件（VmExit/Fault/Halt）
```

### 7.2  Scheduler — 调度器

```rust
// omnisim-runtime/src/scheduler.rs
```

```
pub struct Scheduler {
platform:    Arc<RwLock<Platform>>,
unit_threads: HashMap<UnitId, UnitThreadHandle>,
ctrl_tx:     mpsc::Sender<CtrlCmd>,
event_rx:    mpsc::Receiver<UnitEvent>,
state:       SchedulerState,
config:      SchedulerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
Idle,       // realize 完成，未启动
Running,    // 正常执行中
Paused,     // 已暂停（所有 Unit 线程等待）
Stepping,   // 单步调试模式
Terminated, // 已停止
}

#[derive(Debug)]
pub struct SchedulerConfig {
/// 单 Unit 单次 exec() 的时间预算（纳秒）
/// 越小响应延迟越低，开销越高
pub exec_budget_ns: u64,            // 默认 1_000_000（1ms）
/// SyncBus 时序量子（纳秒）
pub sync_quantum_ns: u64,           // 默认 1_000_000
/// Unit 线程调度策略
pub thread_policy: ThreadPolicy,
/// 最大 VM exit 重试次数
pub max_vm_exit_retries: u32,       // 默认 3
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ThreadPolicy {
/// 每个 Unit 独立 OS 线程（默认）
#[default] OneThreadPerUnit,
/// 所有 Unit 共享线程池（适合 Unit 数量极多的场景）
ThreadPool { size: usize },
}

impl Scheduler {
pub fn new(platform: Arc<RwLock<Platform>>, config: SchedulerConfig) -> Self;

/// 启动所有 Unit 线程，进入 Running 状态
pub fn start(&mut self) -> OmniResult<()>;

/// 暂停所有 Unit（等待所有线程确认暂停再返回）
pub fn pause(&mut self) -> OmniResult<()>;

/// 继续执行
pub fn resume(&mut self) -> OmniResult<()>;

/// 复位所有 Unit（先 pause，再 reset，再 resume）
pub fn reset(&mut self) -> OmniResult<()>;

/// 停止所有 Unit 线程，进入 Terminated 状态
pub fn stop(&mut self) -> OmniResult<()>;

/// 热插拔：添加新 Unit 并启动其线程
pub fn hot_add_unit(&mut self, desc: UnitDesc) -> OmniResult<UnitId>;
pub fn hot_remove_unit(&mut self, id: &UnitId) -> OmniResult<()>;

/// RunLoop：主控线程的核心循环（在 start() 内部启动）
fn run_loop(&mut self);

/// 处理来自 Unit 线程的事件
fn handle_event(&mut self, event: UnitEvent) -> OmniResult<()>;
}
```

### 7.3  UnitThread — 单元执行线程

```rust
// omnisim-runtime/src/unit_thread.rs
```

```
/// 每个可执行 Unit 对应一个 UnitThread
pub struct UnitThread {
unit_id:    UnitId,
unit:       Arc<Mutex<Box<dyn Unit>>>,
ctrl_rx:    mpsc::Receiver<CtrlCmd>,
event_tx:   mpsc::Sender<UnitEvent>,
budget_ns:  u64,
}

impl UnitThread {
/// 线程主函数
pub fn run(mut self) {
loop {
// 检查控制命令（非阻塞）
match self.ctrl_rx.try_recv() {
Ok(CtrlCmd::Pause)     => { self.wait_resume(); continue; }
Ok(CtrlCmd::Reset)     => { self.do_reset(); }
Ok(CtrlCmd::Terminate) => break,
_                      => {}
}

// 驱动执行
let result = self.unit.lock().unwrap()
.execution_backend()
.map(|b| b.exec(self.budget_ns));

match result {
Some(Ok(consumed_ns)) => {
// 正常：上报执行进度
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
self.wait_resume(); // 等待 Scheduler 处理
}
None => {
// 此 Unit 无 ExecutionBackend（Leaf 外设），线程退出
break;
}
_ => {}
}
}
}
}

/// 控制命令（Scheduler → UnitThread）
#[derive(Debug)]
pub enum CtrlCmd {
Pause,
Resume,
Reset,
Terminate,
/// 单步（调试模式）
Step { count: u32 },
}

/// 事件（UnitThread → Scheduler）
#[derive(Debug)]
pub enum UnitEvent {
Progress  { unit_id: UnitId, consumed_ns: u64 },
Fault     { unit_id: UnitId, kind: FaultKind, pc: u64 },
Halted    { unit_id: UnitId },
VmExit    { unit_id: UnitId, exit: VmExitReason },
StepDone  { unit_id: UnitId, result: StepResult },
}

#[derive(Debug)]
pub enum VmExitReason {
Mmio      { addr: u64, size: u8, is_write: bool, data: u64 },
IoPort    { port: u16, size: u8, is_write: bool, data: u32 },
Shutdown,
Debug,
Hlt,
Unknown(u64),
}
```

### 7.4  VM Exit 处理流程

KVM Unit 发生 VM exit 时，UnitThread 上报事件，RunLoop 路由到对应的 MemTarget 或 IO handler：

```rust
// omnisim-runtime/src/vm_exit.rs
```

```
pub struct VmExitDispatcher {
/// MMIO 路由：直接查 MemBus
mem_bus: Arc<MemBus>,
/// IO 端口路由（x86）
io_bus:  Arc<IoBus>,
}

impl VmExitDispatcher {
pub fn dispatch(&self, exit: VmExitReason) -> OmniResult<DispatchResult>;
}

pub enum DispatchResult {
/// 已处理，可继续执行（写入返回值到 Unit 寄存器）
Handled { data: Option<u64> },
/// 需要上层处理（如 Shutdown → Scheduler 停止）
Escalate(VmExitReason),
/// 处理失败
Error(OmniError),
}

// MMIO 处理流程：
// 1. VmExit { MMIO, addr=0x09000000, is_write=false, size=1 }
// 2. VmExitDispatcher::dispatch()
// 3. mem_bus.read(0x09000000, U8, initiator_id=KVM_INITIATOR)
// 4. 路由到 uart0.MemTarget（已在 realize 时注册）
// 5. uart0.read(offset=0, U8) → 返回 rx 寄存器值
// 6. 写回 KVM vcpu 的 mmio data 寄存器
// 7. KVM_RUN 继续
```

### 7.5  SimClock — 全局仿真时钟

```rust
// omnisim-runtime/src/clock.rs
```

```
/// 全局单调仿真时钟
/// 对 Untimed 模式：直接递增（按 exec budget 推进）
/// 对 LooseTiming：由 SyncBus tick 推进
/// 对 CycleAccurate：由 FPGA 节拍器驱动
pub struct SimClock {
now_ns: AtomicU64,
}

impl SimClock {
pub fn now_ns(&self) -> u64 {
self.now_ns.load(Ordering::Acquire)
}

/// 推进时钟（只允许 Scheduler/SyncBus 调用）
pub(crate) fn advance(&self, delta_ns: u64) -> u64 {
self.now_ns.fetch_add(delta_ns, Ordering::AcqRel) + delta_ns
}

/// 注册时钟事件：在 target_ns 时回调
pub fn register_event(
&self,
target_ns: u64,
cb: Box<dyn FnOnce(u64) + Send>,
) -> EventHandle;

/// 取消事件
pub fn cancel_event(&self, handle: EventHandle);
}
```

### 7.6  BootSequencer — 启动顺序执行器

```rust
// omnisim-runtime/src/boot.rs
```

```
/// 按 BootDesc 的 sequence 有序启动各 Unit
pub struct BootSequencer {
desc:      BootDesc,
platform:  Arc<RwLock<Platform>>,
scheduler: Arc<Mutex<Scheduler>>,
}

impl BootSequencer {
/// 执行完整启动序列（在 Scheduler::start() 之后调用）
pub fn run(&mut self) -> OmniResult<()>;

/// 加载镜像文件到指定地址
pub fn load_image(&self, img: &ImageDesc) -> OmniResult<u64>;

/// 执行单个 BootStep
fn execute_step(&mut self, step: &BootStep) -> OmniResult<()>;
}

// 启动序列示例（ARM Host + RISC-V 协处理器）：
// 1. load_image(kernel) → 0x80000000
// 2. load_image(dtb)    → 0x84000000
// 3. WriteReg(cpu0, "x0", 0)            // arg0: hartid
// 4. WriteReg(cpu0, "x1", 0x84000000)   // arg1: dtb addr
// 5. WriteReg(cpu0, "pc", 0x80000000)   // entry
// 6. WaitWrite(reset_ctrl, 0x0, 0x1)    // 等 ARM 固件写 release 信号
// 7. ReleaseReset(riscv_ctrl)            // 释放 RISC-V 协处理器
```

## 8  omnisim-tcg — TCG 执行引擎接口

```
TCG 边界设计原则
TCG 是 omnisim-core 的一个 ExecutionBackend 实现，不是框架核心
TCG 通过 MemSpace 接口访问内存，不持有任何 MemBus 的直接引用
TCG 通过 IrqLine 发出中断，不直接调用 IrqSink
TCG IR 是内部实现细节，不暴露给其他 crate
移植自 QEMU TCG，但接口边界完全重新定义
```

### 8.1  TcgEngine — TCG 引擎主接口

```rust
// omnisim-tcg/src/engine.rs
```

```
/// TCG 引擎：翻译 + 执行客户指令
/// 实现 ExecutionBackend + DebugBackend
pub struct TcgEngine<F: TcgFrontend, B: TcgHostBackend> {
frontend:   F,         // 指令集前端（ARM/RISC-V/x86）
host_back:  B,         // 宿主代码生成后端（x86_64/aarch64）
tb_cache:   TbCache,   // Translation Block 缓存
mem_space:  Arc<MemSpace>, // 内存访问入口
cpu_state:  Box<dyn CpuState>, // 通用寄存器状态
}

impl<F: TcgFrontend, B: TcgHostBackend> ExecutionBackend
for TcgEngine<F, B>
{
fn exec(&mut self, budget_ns: u64) -> OmniResult<u64> {
let budget_insns = self.ns_to_insns(budget_ns);
let mut executed = 0u64;

while executed < budget_insns {
// 1. 查 TB 缓存
let pc = self.cpu_state.pc();
let tb = match self.tb_cache.lookup(pc) {
Some(tb) => tb,
None => {
// 2. 翻译：guest 指令 → TCG IR → 宿主机器码
let ir = self.frontend.translate(pc, &self.mem_space)?;
let code = self.host_back.compile(&ir)?;
self.tb_cache.insert(pc, code)
}
};
// 3. 执行 TB
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
/// TB 正常执行完，继续下一个 TB
Continue,
/// TB 遇到需要退出翻译循环的事件
Exit(TbExitReason),
/// 异常/错误
Fault(FaultKind),
}

#[derive(Debug)]
pub enum TbExitReason {
/// 遇到中断（Irq/FIQ 等）
Interrupt,
/// WFI/WFE（ARM）/ WFI（RISC-V）
WaitForInterrupt,
/// 系统调用 / 陷阱
Syscall,
/// TB 结束（跳转到下一个 TB）
ChainEnd { next_pc: u64 },
}
```

### 8.2  TcgFrontend Trait — 指令集前端

```
/// 指令集解码 + TCG IR 生成
/// 每个 ISA 实现一个 TcgFrontend
pub trait TcgFrontend: Send + Sync {
type State: CpuState;

/// 从 pc 开始翻译一个 Basic Block → TCG IR
/// 遇到跳转/中断检查点/特权指令时停止
fn translate(
&self,
pc:        u64,
mem_space: &MemSpace,
) -> OmniResult<TcgIrBlock>;

/// 指令集名（调试用）
fn isa_name(&self) -> &'static str;

/// 指令最大字节数（用于 TB 缓存 key 计算）
fn max_insn_len(&self) -> usize;
}

/// 寄存器状态接口（对 TcgHostBackend 透明）
pub trait CpuState: Send + Sync {
fn pc(&self) -> u64;
fn set_pc(&mut self, pc: u64);

/// 读写通用寄存器（reg_idx 由 ISA 定义）
fn read_reg(&self, idx: usize) -> u64;
fn write_reg(&mut self, idx: usize, val: u64);

/// 读写系统寄存器（sysreg_id 由 ISA 定义）
fn read_sysreg(&self, id: u64) -> OmniResult<u64>;
fn write_sysreg(&mut self, id: u64, val: u64) -> OmniResult<()>;

/// 保存/恢复完整状态（快照）
fn save(&self) -> Vec<u8>;
fn load(&mut self, data: &[u8]) -> OmniResult<()>;
}
```

### 8.3  TcgHostBackend Trait — 宿主代码生成后端

```
/// TCG IR → 宿主机器码
pub trait TcgHostBackend: Send + Sync {
/// 编译 TCG IR Block → 可执行机器码
fn compile(&self, ir: &TcgIrBlock) -> OmniResult<CompiledTb>;

/// 宿主架构名
fn host_arch(&self) -> &'static str;

/// 代码缓冲区管理
fn alloc_code_buf(&self, size: usize) -> OmniResult<CodeBuffer>;
fn free_code_buf(&self, buf: CodeBuffer);
}

/// 编译后的 Translation Block
pub struct CompiledTb {
code_ptr:   *const u8,     // 可执行代码指针
code_size:  usize,
insn_count: u32,           // 对应的 guest 指令数
guest_pc:   u64,           // 对应的 guest PC
exit_pcs:   Vec<u64>,      // TB 内所有可能的 exit 点
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

### 8.4  TbCache — Translation Block 缓存

```
/// TB 缓存：guest PC → CompiledTb
pub struct TbCache {
/// 热缓存（直接映射，超快查找）
fast:    Box<[Option<CachedTb>; FAST_CACHE_SIZE]>,
/// 慢缓存（HashMap，覆盖全地址空间）
slow:    HashMap<u64, Arc<CompiledTb>>,
/// 最大容量（字节），超出时 LRU 淘汰
max_bytes: usize,
used_bytes: usize,
}

const FAST_CACHE_SIZE: usize = 1 << 14; // 16K 槽

impl TbCache {
pub fn lookup(&self, pc: u64) -> Option<Arc<CompiledTb>>;
pub fn insert(&mut self, pc: u64, tb: CompiledTb) -> Arc<CompiledTb>;
/// 地址范围失效（SMC / 代码自修改）
pub fn invalidate_range(&mut self, start: u64, end: u64);
/// 全量失效（ASID 切换 / TLB flush）
pub fn flush_all(&mut self);
pub fn stats(&self) -> TbCacheStats;
}
```

### 8.5  前后端组合矩阵

| 客户 ISA（前端） | 宿主 x86_64 | 宿主 AArch64 | 备注 |
| --- | --- | --- | --- |
| ARM AArch64 | ✅ 主力 | ✅ 支持 |  |
| ARM AArch32 | ✅ 支持 | 兼容 32-bit 镜像 |  |
| RISC-V RV64 | ✅ 主力 | ✅ 支持 |  |
| RISC-V RV32 | ✅ 支持 | 计划中 | 嵌入式场景 |
| x86-64 | KVM 优先 | ✅ TCG | x86 Host 推荐用 KVM |

## 9  omnisim-compat — L4 兼容层

```
L4 层职责
omnisim-compat 让外部世界（libvirt / OpenStack / virt-manager）看到的是一个标准 QEMU 进程。它实现 QEMU 命令行解析和 QMP 协议核心子集，内部翻译为 L3 Manifest 操作和 L2 Platform 控制。
```

### 9.1  CLI Parser — QEMU 命令行解析

```rust
// omnisim-compat/src/cli/parser.rs
```

```
/// 解析 QEMU 风格命令行，输出 Manifest
pub struct QemuCliParser {
/// 已知的 -machine 名 → 基础 Manifest 模板
machine_map: HashMap<String, Manifest>,
/// 已知的 -device driver 名 → UnitDesc 生成器
device_map:  HashMap<String, DeviceFactory>,
}

impl QemuCliParser {
/// 解析完整命令行（std::env::args() 或测试输入）
pub fn parse(&self, args: &[String]) -> OmniResult<Manifest>;
}

// 支持的主要参数组（按优先级）：
//   -machine <name>       选择基础平台模板
//   -cpu <model>          覆盖 CPU 类型
//   -m <size>[M|G]        内存大小
//   -smp <n>              CPU 数量
//   -kernel <path>        内核路径
//   -initrd <path>        initrd 路径
//   -append <cmdline>     内核参数
//   -dtb <path>           设备树
//   -device <driver>[,key=val...]  热插设备
//   -drive file=<path>[,if=virtio,...]
//   -netdev user[,id=...]
//   -chardev socket[,path=...]
//   -qmp unix:<path>,server[,nowait]
//   -qmp tcp:<host>:<port>
//   -enable-kvm           → backend=kvm
//   -nographic            → 关闭 framebuffer
//   -S                    → 启动后暂停（等待 QMP cont）
//   -s / -gdb tcp::1234   → 启动 GDB stub

/// -device 参数到 UnitDesc 的翻译示例
/// -device virtio-blk-pci,drive=d0,id=blk0
/// →
/// UnitDesc {
///   id:     "blk0",
///   kind:   "virtio-blk-pci",
///   config: { "drive": "d0" }
/// }
```

### 9.2  QMP Server — QMP 协议实现

```rust
// omnisim-compat/src/qmp/server.rs
```

```
pub struct QmpServer {
listener:   QmpListener,   // Unix socket 或 TCP
scheduler:  Arc<Mutex<Scheduler>>,
platform:   Arc<RwLock<Platform>>,
realizer:   Arc<Realizer>,
negotiated: bool,          // capability negotiation 完成标志
}

pub enum QmpListener {
UnixSocket(PathBuf),
Tcp(SocketAddr),
}

impl QmpServer {
/// 启动监听（独立线程）
pub fn start(self) -> OmniResult<JoinHandle<()>>;

/// 处理单个连接
async fn handle_conn(&self, conn: QmpConn);

/// 分发命令
async fn dispatch(&self, cmd: QmpCommand) -> QmpResponse;
}
```

### 9.3  QMP 命令完整列表

按 libvirt 实际使用频率排序，标注实现优先级：

| 命令 | 优先级 | 说明 | L2/L3 操作 |
| --- | --- | --- | --- |
| qmp_capabilities | P0 | 能力协商（握手第一步，必须实现） | 返回 OmniSim 支持的能力列表 |
| query-version | P0 | QEMU 版本信息（libvirt 检测用） | 返回硬编码的兼容版本号 |
| query-status | P0 | 查询运行状态 | Scheduler::state() |
| stop | P0 | 暂停所有 vCPU | Scheduler::pause() |
| cont | P0 | 继续执行 | Scheduler::resume() |
| system_reset | P0 | 系统复位 | Scheduler::reset() |
| system_powerdown | P0 | 优雅关机（发 ACPI 事件） | IrqLine → ACPI Unit |
| query-cpus-fast | P0 | 查询 CPU 状态列表 | UnitRegistry::find_by_kind("*-core") |
| query-pci | P1 | 查询 PCIe 拓扑 | Platform::buses.pcie_buses |
| query-block | P1 | 查询块设备列表 | UnitRegistry::find_by_kind("virtio-blk*") |
| device_add | P1 | 热插设备 | Platform::hot_add() + Scheduler::hot_add_unit() |
| device_del | P1 | 热拔设备 | Platform::hot_remove() |
| drive_add | P1 | 热插磁盘 | 转为 device_add(virtio-blk-pci) |
| netdev_add | P1 | 热插网卡 | 转为 device_add |
| migrate | P2 | 实时迁移 | Platform::snapshot() + 传输 + 目标 restore() |
| migrate-cancel | P2 | 取消迁移 |  |
| query-migrate | P2 | 查询迁移状态 |  |
| savevm | P2 | 保存快照 | Platform::snapshot() → 文件 |
| loadvm | P2 | 加载快照 | Platform::restore() |
| query-snapshots | P2 | 列出快照 |  |
| object_add | P2 | 添加 QEMU 对象（memory-backend-file 等） | Manifest 动态修改 |
| dump-guest-memory | P3 | 导出 guest 内存（crash dump） | MemSpace 批量读 |
| x-query-profile | P3 | 性能分析数据 | Scheduler + UnitThread 统计 |

### 9.4  QMP 消息格式

```rust
// omnisim-compat/src/qmp/types.rs
```

```
/// QMP greeting（连接后服务端主动发送）
/// { "QMP": { "version": {...}, "capabilities": [...] } }
#[derive(Serialize)]
pub struct QmpGreeting {
#[serde(rename = "QMP")]
pub qmp: QmpGreetingBody,
}

/// 客户端发送的命令
/// { "execute": "query-status", "arguments": {...}, "id": <any> }
#[derive(Deserialize)]
pub struct QmpRequest {
pub execute:   String,
#[serde(default)]
pub arguments: serde_json::Value,
/// 透传到 response，libvirt 用于关联请求
pub id:        Option<serde_json::Value>,
}

/// 成功响应
/// { "return": {...}, "id": <any> }
#[derive(Serialize)]
pub struct QmpSuccess {
#[serde(rename = "return")]
pub result: serde_json::Value,
#[serde(skip_serializing_if = "Option::is_none")]
pub id:     Option<serde_json::Value>,
}

/// 错误响应
/// { "error": { "class": "GenericError", "desc": "..." }, "id": <any> }
#[derive(Serialize)]
pub struct QmpError {
pub error: QmpErrorBody,
#[serde(skip_serializing_if = "Option::is_none")]
pub id:    Option<serde_json::Value>,
}

/// 异步事件（服务端主动推送，无 id）
/// { "event": "STOP", "data": {...}, "timestamp": {...} }
#[derive(Serialize)]
pub struct QmpEvent {
pub event:     String,
pub data:      serde_json::Value,
pub timestamp: QmpTimestamp,
}

/// OmniSim 发出的异步事件列表（libvirt 订阅）：
/// STOP         → Scheduler 进入 Paused
/// RESUME       → Scheduler 进入 Running
/// RESET        → 系统复位完成
/// SHUTDOWN     → guest 主动关机
/// POWERDOWN    → ACPI 关机
/// DEVICE_TRAY_MOVED → 光驱弹出
/// DEVICE_DELETED    → 热拔完成
/// GUEST_PANICKED    → guest kernel panic
/// MIGRATION_STATUS  → 迁移状态变化
/// BALLOON_CHANGE    → 内存气球变化
```

### 9.5  GDB Stub — omni-dbg

实现 GDB RSP（Remote Serial Protocol），让标准 GDB / LLDB 可以调试 OmniSim 内的 guest。

```rust
// tools/omni-dbg/src/gdb_stub.rs
```

```
pub struct GdbStub {
listen_addr: SocketAddr,
scheduler:   Arc<Mutex<Scheduler>>,
platform:    Arc<RwLock<Platform>>,
target_cpu:  UnitId,   // 当前调试的 CPU Unit
}

/// 实现的 GDB RSP 命令：
/// ?        → 查询 stop reason
/// g / G    → 读/写所有寄存器
/// p / P    → 读/写单个寄存器
/// m / M    → 读/写内存
/// c / C    → 继续执行
/// s / S    → 单步
/// z0/Z0    → 删除/插入软件断点
/// z1/Z1    → 删除/插入硬件断点
/// z2/Z2    → 删除/插入写观察点
/// z3/Z3    → 删除/插入读观察点
/// qXfer:features → 读取目标 XML（寄存器描述）
/// vCont    → 多线程控制（SMP 调试）
/// T        → 查询线程是否存活
/// H        → 选择操作线程（对应 CPU Unit）

/// GDB 与 OmniSim 的映射：
/// GDB thread id  →  CPU Unit id（soc0.arm_ss.core0 → tid=1）
/// GDB register   →  DebugBackend::read_reg()
/// GDB memory     →  MemSpace::read()（使用 target CPU 的视角）
```

### 9.6  可执行文件伪装

OmniSim 通过软链接伪装成 QEMU 可执行文件，libvirt 无感接入：

```
# 安装时创建软链接
ln -s /usr/bin/omnisim /usr/bin/qemu-system-aarch64
ln -s /usr/bin/omnisim /usr/bin/qemu-system-x86_64
ln -s /usr/bin/omnisim /usr/bin/qemu-system-riscv64

# OmniSim 读取 argv[0] 推断默认体系结构：
# qemu-system-aarch64 → arch = "aarch64"
# qemu-system-x86_64  → arch = "x86_64"

# libvirt 生成的完整调用（OmniSim 透明处理）：
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

## 10  组件实现约定

```
约定目的
所有 component crate 遵守同一套约定，保证可互换性：框架代码和测试代码不需要知道具体组件是哪个，只通过 trait 交互。
```

### 10.1  组件 crate 标准结构

```
// 以 omnisim-intc-gicv3 为例
omnisim-intc-gicv3/
├── Cargo.toml
├── src/
│   ├── lib.rs          // register() 入口 + pub re-exports
│   ├── gicv3.rs        // GicV3（Composite Unit）
│   ├── gicd.rs         // Distributor（Leaf Unit, MemTarget + IrqSink）
│   ├── gicr.rs         // Redistributor × N（Leaf Unit）
│   ├── gicc.rs         // CPU Interface（Leaf Unit）
│   └── schema.json     // 组件 config JSON Schema
└── tests/
├── unit_test.rs    // 单元测试（MockBackend + MockMemTarget）
└── integ_test.rs   // 集成测试（最小 Platform）
```

### 10.2  Leaf Unit 标准模式

```rust
// 以 UART（omnisim-io-uart）为例
```

```
/// Leaf Unit：只实现 MemTarget（响应地址读写），
/// 不实现 ExecutionBackend（无独立执行逻辑）
pub struct PL011Uart {
id:        UnitId,
/// realize 后填充（从 IrqRouter 获取）
irq_line:  Option<IrqLine>,
/// 寄存器状态
regs:      Mutex<UartRegs>,
}

impl Unit for PL011Uart {
fn id(&self)   -> &UnitId    { &self.id }
fn kind(&self) -> &'static str { "pl011-uart" }
fn caps(&self) -> UnitCaps {
UnitCaps::ADDRESSABLE | UnitCaps::IRQ_SOURCE
}

fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 从 IrqRouter 获取出口中断线
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
fn base(&self)      -> u64 { 0 }  // offset 0
fn size(&self)      -> u64 { 0x1000 }

fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
let regs = self.regs.lock().unwrap();
match offset {
0x00 => Ok(regs.dr as u64),   // Data Register
0x18 => Ok(regs.fr as u64),   // Flag Register
0x24 => Ok(regs.ibrd as u64), // Integer Baud Rate
_    => Ok(0),
}
}

fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
let mut regs = self.regs.lock().unwrap();
match offset {
0x00 => {
regs.dr = val as u8;
// 触发 TX 完成中断
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

### 10.3  Executable Unit 标准模式

```rust
// 以 ARM Cortex-A78 Core 为例
```

```
pub struct CortexA78Core {
id:      UnitId,
/// TCG 引擎（Functional Backend）或 KvmBackend
backend: Box<dyn ExecutionBackend>,
/// ARM 系统寄存器状态（SCTLR/TCR/TTBR...）
sysregs: ArmSysRegs,
}

impl Unit for CortexA78Core {
fn caps(&self) -> UnitCaps {
UnitCaps::EXECUTABLE | UnitCaps::DEBUGGABLE | UnitCaps::SNAPSHOTTABLE
}

fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 将 MemBus 视图注入 Backend（TCG 通过 MemSpace 访问内存）
// KvmBackend：设置 KVM memslot
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

### 10.4  Composite Unit 标准模式

```rust
// 以 GICv3（含 GICD + GICR×N + GICC×N）为例
```

```
pub struct GicV3 {
id:        UnitId,
num_cpus:  u32,
sub_units: Vec<Box<dyn Unit>>,   // [gicd, gicr0..N, gicc0..N]
/// 内部 MemBus（gicd/gicr 的地址映射）
internal_mem_bus: MemBus,
}

impl Unit for GicV3 {
fn caps(&self) -> UnitCaps {
UnitCaps::COMPOSITE | UnitCaps::IRQ_SINK | UnitCaps::ADDRESSABLE
}

fn realize(&mut self, ctx: &mut RealizeCtx) -> OmniResult<()> {
// 1. realize 所有子 Unit
for sub in &mut self.sub_units {
let mut sub_ctx = ctx.child_ctx(sub.id());
sub.realize(&mut sub_ctx)?;
}
// 2. 在外部 MemBus 注册 GIC 地址窗口
//    GICD: 0x08000000, GICR: 0x08010000×N
//    实际地址由 Manifest 的 MemBus 配置决定
Ok(())
}

fn sub_units(&self) -> Option<&[Box<dyn Unit>]> {
Some(&self.sub_units)
}
fn sub_units_mut(&mut self) -> Option<&mut Vec<Box<dyn Unit>>> {
Some(&mut self.sub_units)
}

/// GICv3 作为 IrqSink：外设中断 → GICD.SETSPI
fn as_irq_sink(&self) -> Option<Arc<dyn IrqSink>> {
// 返回 GICD 的 IrqSink 实现
Some(self.gicd().as_irq_sink().unwrap())
}
}
```

### 10.5  组件配置 Schema 约定

每个组件必须提供 JSON Schema，Validator 在 realize 前校验 UnitDesc.config：

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
"description": "GICD base address (override, usually from MemBus)"
},
"gicr_stride": {
"type": "integer",
"default": 131072,
"description": "GICR stride per CPU (default 128KB)"
},
"its_enable": {
"type": "boolean",
"default": false
}
}
}
```

### 10.6  单元测试约定

每个组件 crate 必须包含：

| 测试类型 | 使用工具 | 测试内容 |
| --- | --- | --- |
| 寄存器读写 | MockMemAccess + MemTarget 直接调用 | 所有寄存器的读写逻辑，边界值，重置值 |
| 中断触发 | MockIrqSink（记录 raise/lower 次数） | 触发条件，电平/边沿语义，mask 逻辑 |
| Unit 生命周期 | MinimalPlatform（helper） | realize → reset → unrealize 幂等性 |
| Snapshot/Restore | save_snapshot + load_snapshot | 状态完整性，跨版本兼容性（可选） |
| 集成：最小平台 | Manifest + Realizer + MockCpu | 组件在完整 Bus 连接下的行为 |

## 11  工程结构与构建约定

### 11.1  Workspace 完整结构

```
omnisim/
├── Cargo.toml                      # workspace 根
├── Cargo.lock
├── deny.toml                       # cargo-deny：许可证 + 漏洞检查
├── .cargo/config.toml              # target / linker / env 配置
│
├── crates/                         # 框架核心 crate（严格分层）
│   ├── omnisim-core/               # L1+L2 接口，零业务依赖（std+thiserror+bitflags）
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
│   │       ├── security.rs         # MemSecurity / SecurityWorld（P0 预留，P2 实施）
│   │       ├── coherence.rs        # CoherenceDomain / CoherenceParticipant（P2 占位）
│   │       ├── network.rs          # NetworkBackend / Packet
│   │       ├── registry.rs         # UnitRegistry
│   │       └── snapshot.rs         # UnitSnapshot / PlatformSnapshot
│   │
│   ├── omnisim-manifest/           # L3 平台描述
│   │   └── src/
│   │       ├── manifest.rs         # Manifest / UnitDesc / BusDesc / BootDesc
│   │       ├── validator.rs        # Validator / ValidationReport
│   │       ├── expander.rs         # 模板展开 / count 展开
│   │       ├── realizer.rs         # Realizer / ComponentRegistry / Platform
│   │       └── loader.rs           # from_json / from_toml / to_json / to_toml
│   │
│   ├── omnisim-runtime/            # 运行时调度
│   │   └── src/
│   │       ├── scheduler.rs        # Scheduler / SchedulerConfig
│   │       ├── unit_thread.rs      # UnitThread / CtrlCmd / UnitEvent
│   │       ├── vm_exit.rs          # VmExitDispatcher
│   │       ├── clock.rs            # SimClock
│   │       ├── boot.rs             # BootSequencer
│   │       └── main.rs             # 程序入口（注册所有组件，解析 CLI，启动）
│   │
│   ├── omnisim-tcg/                # TCG 执行引擎
│   │   └── src/
│   │       ├── engine.rs           # TcgEngine<F,B>
│   │       ├── frontend.rs         # TcgFrontend trait / CpuState trait
│   │       ├── host_backend.rs     # TcgHostBackend trait
│   │       ├── ir.rs               # TcgIrBlock / TcgOp
│   │       ├── optimizer.rs        # 常量折叠 / 死代码消除
│   │       ├── tb_cache.rs         # TbCache
│   │       ├── frontends/
│   │       │   ├── arm/            # ARM AArch64 + AArch32 前端
│   │       │   ├── riscv/          # RISC-V RV64/RV32 前端
│   │       │   └── x86/            # x86-64 前端
│   │       └── host_backends/
│   │           ├── x86_64/         # 生成 x86_64 机器码
│   │           └── aarch64/        # 生成 AArch64 机器码
│   │
│   └── omnisim-compat/             # L4 兼容层
│       └── src/
│           ├── lib.rs
│           ├── cli/
│           │   ├── parser.rs       # QemuCliParser
│           │   └── machine_map.rs  # -machine 名 → 基础 Manifest
│           └── qmp/
│               ├── server.rs       # QmpServer
│               ├── types.rs        # QmpRequest / QmpResponse / QmpEvent
│               └── handlers.rs     # 每个 QMP 命令的 handler
│
├── components/                     # 硬件组件库
│   ├── cpu/
│   │   ├── omnisim-cpu-arm/        # ARM Cortex-A 系列
│   │   ├── omnisim-cpu-riscv/      # RISC-V RV64/RV32
│   │   ├── omnisim-cpu-x86/        # x86-64（TCG + KVM）
│   │   └── omnisim-cpu-mock/       # Mock CPU（测试）
│   │   # MMU 并入各 CPU crate 内部，不独立成组件
│   │   # omnisim-cpu-arm/src/mmu.rs    ARMv8/v9 MMU（Stage1 + Stage2）
│   │   # omnisim-cpu-riscv/src/mmu.rs  Sv39/Sv48/Sv57
│   ├── intc/
│   │   ├── omnisim-intc-gicv3/     # ARM GICv3（GICD+GICR+GICC+ITS）
│   │   ├── omnisim-intc-plic/      # RISC-V PLIC
│   │   └── omnisim-intc-clint/     # RISC-V CLINT（软中断+定时器）
│   ├── cache/
│   │   └── omnisim-cache/          # Cache 统一模型（L1/L2/L3 由容量/延迟参数区分）
│   │   #    P2 RTL 协同时如差异过大可再拆分为独立 crate

│   ├── memory/
│   │   ├── omnisim-mem-dram/       # DRAM（含 TimingBackend）
│   │   ├── omnisim-mem-sram/       # SRAM（无 timing，低延迟）
│   │   └── omnisim-mem-gddr/       # GDDR6（GPU 显存，含 TimingBackend）
│   ├── io/
│   │   ├── omnisim-io-uart/        # PL011 UART
│   │   ├── omnisim-io-pcie/        # PCIe Root Complex + EP 框架
│   │   ├── omnisim-io-dma/         # DMA 引擎（含 Dataflow 接口）
│   │   ├── omnisim-io-mailbox/     # 片上 Mailbox（双向中断）
│   │   └── omnisim-io-virtio/      # Virtio 总线 + 设备（blk/net/console）
│   └── accel/
│       ├── omnisim-accel-gpgpu/    # GPGPU（SM + L2 + VRAM，Composite）
│       └── omnisim-accel-dpu/      # DPU（网络处理器，Composite）
│
├── profiles/                       # 预置 Platform Profile
│   ├── embedded/                   # 嵌入式 SoC（RISC-V）
│   ├── server/                     # ARM 服务器
│   ├── hetero/                     # 异构加速平台
│   └── extended/                   # 扩展版本功能接口占位（TimingBackend / RTL 协同等）
│
├── tools/
│   ├── omni-cli/                   # 命令行入口（注册所有组件）
│   └── omni-dbg/                   # GDB stub
│
└── tests/
├── common/                     # 测试工具（MinimalPlatform / MockBackend）
├── system/                     # 系统级测试入口（调用各子 crate 测试）
├── integration/
│   ├── scenario01_arm_riscv.rs    # 场景一：片上异构双核
│   ├── scenario02_pcie_gpgpu.rs   # 场景二：PCIe GPGPU
│   ├── scenario03_pcie_dpu.rs     # 场景三：PCIe DPU
│   ├── scenario04_multi_gpu.rs    # 场景四：多卡互联
│   ├── scenario05_embedded_soc.rs # 场景五：嵌入式 SoC
│   ├── scenario06_network_proc.rs # 场景六：网络处理器
│   ├── scenario07_smart_ssd.rs    # 场景七：SmartSSD
│   ├── scenario08_cxl.rs          # 场景八：CXL 设备
│   ├── scenario09_numa_server.rs  # 场景九：多路服务器 NUMA
│   ├── scenario10_soc_numa.rs     # 场景十：大内存 SoC NUMA
│   ├── scenario11_nested_virt.rs  # 场景十一：嵌套虚拟化
│   ├── scenario12_trustzone.rs    # 场景十二：TrustZone / TEE
│   ├── scenario13_cvm.rs          # 场景十三：机密虚拟化（P2）
│   ├── scenario14_ai_server.rs    # 场景十四：AI 服务器全栈
│   ├── scenario15_hetero_mem.rs   # 场景十五：异构内存平台
│   └── scenario16_numa_gpu.rs     # 场景十六：NUMA + 多卡混合
└── fixtures/                   # 测试用 Manifest JSON/TOML
```

### 11.2  Cargo.toml 约定

```toml
// 根 Cargo.toml
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
# ... 所有 components
"tools/omni-cli",
"tools/omni-dbg",
]
resolver = "2"

[workspace.dependencies]
# 统一管理版本，各 crate 用 { workspace = true }
omnisim-core     = { path = "crates/omnisim-core" }
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"
thiserror        = "1"
log              = "0.4"
tracing          = "0.1"
tokio            = { version = "1", features = ["full"] }
bitflags         = "2"
kvm-ioctls       = "0.16"
vm-memory        = "0.12"

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1

[profile.bench]
inherits = "release"
debug = true
```

### 11.3  依赖约束（cargo-deny）

| 规则 | 说明 |
| --- | --- |
| 组件 crate 只能依赖 omnisim-core | 不允许组件之间相互依赖（通过 deny.toml 中 bans 检查） |
| omnisim-core 零业务依赖 | 只允许 std + thiserror + bitflags，无业务逻辑依赖 |
| 许可证白名单 | MIT / Apache-2.0 / BSD-2/3 / ISC（不允许 GPL/LGPL 进入核心） |
| 禁止 unwrap() 在库代码中 | clippy::unwrap_used = deny（通过 .clippy.toml） |
| 所有 pub 接口有文档 | missing_docs = warn（核心 crate 为 deny） |
