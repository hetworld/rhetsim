> [English](./01-architecture.md)

---

# OmniSim — 架构设计

> **文档编号**：DOC-01 &nbsp;&nbsp; **版本**：v1.0 &nbsp;&nbsp; **日期**：2026-Q1 &nbsp;&nbsp; **作者**：OpenFunTech

---

| 文档编号 | DOC-01 |
| --- | --- |
| 文档版本 | v1.0 |
| 适用读者 | 所有团队成员（入口文档） |
| 关联文档 | DOC-02 接口规范 · DOC-03 组件开发指南 · DOC-04 应用场景手册 |
| 上次更新 | 2026-Q1 |

## 1  背景与动机

### 1.1  硬件异构化的现实

现代计算平台已不再是单一架构的 CPU 系统。一张 AI 服务器主板上可能同时存在 x86 主机、ARM 管理核、RISC-V 控制核、GPU、DPU、NPU，以及通过 PCIe / CXL 连接的各类加速卡。软件栈需要在这些异构单元之间协同工作，而验证这种协同的唯一可靠手段就是仿真。

### 1.2  QEMU 的世界观与局限

QEMU 是目前最主流的开源仿真器，但它诞生于 CPU 仿真的时代，设计世界观是以 CPU 为中心的：

```
QEMU 的根本局限
CPU 是特权对象：拥有专属执行循环，Device 只能被动响应
连接关系隐藏在 C 初始化代码中：无法查询，无法序列化，无法动态修改
单体架构：组件之间高度耦合，无法独立测试和发布
Device 内部不能是完整 SoC：不支持 Composite Device 递归
仿真精度单一：只有功能仿真，无法建模性能
```

这些局限在仿真单一 CPU 平台时问题不大，但在异构平台场景下会导致严重的架构阻抗：GPU 的 SM 阵列、DPU 的网络处理核、NPU 的计算阵列，都无法在 QEMU 框架内优雅表达。

### 1.3  rust-vmm 的价值与边界

rust-vmm 是 Rust 生态下的虚拟化组件库，提供了优质的模块化基础（kvm-ioctls、vm-memory、virtio-queue 等），但它本身不是框架：

```
rust-vmm 的定位
价值：高质量的 Rust 安全虚拟化原语，OmniSim 直接复用
边界：只是组件库，没有组合框架，使用者自行管理连接关系
缺失：没有 Bus 抽象，没有 Manifest，没有调度器
```

### 1.4  OmniSim 要解决的问题

```
OmniSim 的定位
OmniSim = rust-vmm 的组合哲学 + 异构平台的拓扑描述能力 + libvirt/QEMU 的生态兼容性
介于 QEMU（单体）和 rust-vmm（纯组件库）之间：提供最小组合框架，保持组件独立性
```

| 维度 | QEMU | rust-vmm | OmniSim |
| --- | --- | --- | --- |
| 架构风格 | 单体，内部高度耦合 | 纯组件库，无框架 | 组件库 + 最小组合框架 |
| CPU 地位 | 特权对象，专属执行循环 | 无统一抽象 | Unit，与 NPU/DPU 完全对等 |
| 连接关系 | 藏在 C 初始化代码 | 使用者自行管理 | Bus 是一等公民，显式可查询 |
| Device 内 SoC | 不支持 | Composite Unit 递归嵌套 |  |
| 仿真精度 | 功能仿真 | 无定义 | Functional/Timing/RTL 可混用 |
| libvirt 兼容 | 原生支持 | 不涉及 | L4 兼容层，对 libvirt 透明 |
| 组件复用 | 不可 | rust-vmm crate | 直接复用 rust-vmm crate |

## 2  核心抽象

OmniSim 的整个设计建立在五个核心概念上。理解这五个概念，就理解了 OmniSim 的本质。

### 2.1  五个核心概念

```
五个核心概念
Unit    — 平台中可独立替换的最小硬件仿真单元
Bus     — Unit 之间连接关系的显式表达（一等公民）
Backend — Unit 的执行实现（执行维度 × 时序维度）
Manifest — 平台的完整声明式描述，可序列化为 JSON/TOML
Profile — Manifest 的具体实例（嵌入式/服务器/异构等）
```

### 2.2  Unit：粒度原则

Unit 的粒度判断标准只有一条：可以独立替换的最小硬件功能单元 = 一个 Unit。

| 类型 | 判断标准 | 示例 |
| --- | --- | --- |
| Leaf Unit | 不再包含子 Unit；有具体执行或响应逻辑 | CPU Core、MMU、GIC GICD、L1 Cache、UART |
| Composite Unit | 内部由多个子 Unit 组成；对外暴露边界接口 | ARM SoC、GICv3、GPGPU、DPU、RISC-V 子系统 |
| 可执行 Unit | 实现 ExecutionBackend；能被调度器驱动 | CPU Core、NPU、DPU、DMA Engine、Timer |
| 可寻址 Unit | 实现 MemTarget；响应地址读写 | DRAM、SRAM、UART 寄存器、GIC 寄存器 |

### 2.3  Bus：连接关系是一等公民

这是 OmniSim 与 QEMU 最根本的区别。在 QEMU 中，Unit 之间的连接关系藏在 C 初始化代码里，无法查询、无法序列化。OmniSim 把连接关系提升为一等公民，以 Bus 对象显式表达。

OmniSim 定义了 10 种 Bus，覆盖所有连接场景：

| Bus 类型 | 职责 | 典型使用场景 |
| --- | --- | --- |
| MemBus | 地址路由，读写转发，partial visible | CPU→DRAM、CPU→外设寄存器 |
| IrqBus | 中断信号路由 | UART irq → GIC → CPU |
| PcieBus | PCIe 设备发现、BAR、MSI-X | Host → GPU、Host → DPU |
| SyncBus | 混合精度时序协调 | Functional + Timing 混合仿真 |
| DataBus | 高带宽数据流（与 MemBus 分离） | NPU → DRAM、DMA 批量传输 |
| ResetBus | 复位信号建模 | PERST#、片上 Reset Controller |
| PowerBus | 电源域管理 | SoC 多核上下电、DVFS |
| InterconnectBus | 对等节点互联（Windowed/Unified） | NUMA、NVLink、CXL.mem |
| NetworkBackend | 外部网络接入统一接口 | DPU nvme-of、网络处理器 |
| CoherenceDomain | 缓存一致性域（P2） | CXL.cache、ACE/CHI |

### 2.4  Backend：两个正交维度

Backend 描述 Unit 如何执行，有两个完全正交的维度：

| 执行维度 | 时序维度 | 组合场景 | 产品阶段 |
| --- | --- | --- | --- |
| Functional（TCG） | Untimed | 功能仿真，验证软件正确性，速度最快 | OmniSim P0 开源 |
| Timing（性能模型） | LooseTiming | 性能仿真，建模延迟/带宽/吞吐 | P1 |
| Rtl（FPGA 桥接） | CycleAccurate | RTL 协同验证，周期精确 | P2 |
| Kvm（硬件加速） | Untimed | Hypervisor，真实硬件执行 | LeoVirt-S |
| Mock（测试桩） | Untimed | 单元测试，CI 验证 | 测试基础设施 |
| SimtHart（RISC-V hart 阵列） | Untimed/Loose | GPGPU SM 路线 A | OmniSim P0 |
| SimtWarp（SIMT Warp 调度器） | Untimed/Loose | GPGPU SM 路线 B | OmniSim P1 |

同一个平台内，不同 Unit 可以使用不同 Backend 和不同 TimingModel，SyncBus 负责协调时序。

### 2.5  Manifest：声明式平台描述

Manifest 是平台拓扑的完整声明，可序列化为 JSON 或 TOML，是 OmniSim 和外部世界交互的语言。Manifest 描述"有什么 Unit"和"它们怎么连接"，不包含任何执行逻辑。

```toml
# 一个最小的 Manifest 示例
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

## 3  四层架构

### 3.1  架构总览

```
┌──────────────────────────────────────────────────────────────┐
│  L4  Compatibility Layer  兼容层                             │
│      CLI Parser（QEMU 命令行兼容）                           │
│      QMP Server（libvirt / OpenStack 协议）                  │
│      依赖：L3 Manifest API                                   │
├──────────────────────────────────────────────────────────────┤
│  L3  Manifest Layer  平台描述层                              │
│      UnitDesc · BusDesc · Realizer · Registry               │
│      依赖：L2 Unit/Bus trait                                 │
├──────────────────────────────────────────────────────────────┤
│  L2  Composition Layer  组合层                               │
│      Unit（含 sub_units 递归）· Bus 运行时实例               │
│      依赖：L1 Backend/TimingModel trait                      │
├──────────────────────────────────────────────────────────────┤
│  L1  Backend Layer  后端层                                   │
│      ExecutionBackend · TimingModel                          │
│      直接复用 rust-vmm crate                                 │
└──────────────────────────────────────────────────────────────┘
```

### 3.2  层间依赖原则

层间依赖严格单向，上层只通过接口访问下层，任何层都不能跨层调用：

L4 只知道 QEMU/libvirt 协议，通过 L3 接口操作，不直接操作 Unit

L3 只知道 Manifest 格式，通过 L2 接口 realize，不知道 Backend 细节

L2 只知道 Unit 和 Bus，通过 L1 接口执行，不知道内部实现

L1 只知道执行逻辑，不知道自己在哪个平台里

### 3.3  L1 — Backend Layer

Unit 的实际执行实现。每种 Backend 独立实现，可以替换，不影响上层。直接复用 rust-vmm crate 作为底层基础。

| Backend | 实现机制 | 时序模型 | 适用场景 |
| --- | --- | --- | --- |
| Functional | 移植 QEMU TCG（ARM/RISC-V/x86 前端，x86_64/AArch64 后端） | Untimed | 功能验证，速度优先 |
| Timing | 微架构性能模型，建模流水线、Cache miss、内存带宽 | LooseTiming | 性能仿真 |
| Rtl | FPGA 桥接驱动，将仿真请求转发到 FPGA 上的 RTL | CycleAccurate | RTL 协同验证 |
| Kvm | 复用 rust-vmm/kvm-ioctls，通过 /dev/kvm ioctl 驱动 | Untimed（真实时间） | 同架构虚拟化 |
| Mock | 记录所有调用，支持注入自定义返回值 | Untimed | 单元测试 / CI |
| SimtHart | RISC-V hart 阵列，RVV + custom Tensor 扩展 | Untimed/Loose | GPGPU SM 路线 A |
| SimtWarp | Warp 调度器 + 向量执行单元，GpuFrontend trait 插拔 | Untimed/Loose | GPGPU SM 路线 B |

### 3.4  L2 — Composition Layer

运行时 Unit 组合和连接。连接关系以 Bus 对象显式存在，可查询，可动态修改（热插拔）。

#### Unit 生命周期

```
创建 → realize（连接 Bus，初始化内部状态）
→ reset（复位到上电初始状态）
→ exec loop（Backend 驱动执行，调度器管理）
→ destroy（释放所有资源）
```

#### Composite Unit 递归

Unit 内部包含 sub_units，realize() 时递归 realize 所有子 Unit。子 Unit 连接内部 Bus，外部通过边界 Port 与父 Unit 交互，嵌套深度不限。这使得 GPGPU（内含 ARM ctrl + SM 阵列 + HBM）这样的复杂结构可以作为一个 Unit 挂在 PcieBus 上，外部完全透明。

#### MemSpace 隔离

每个 Initiator Unit 持有自己的 MemSpace（地址路由表）。ARM Core 的 MemSpace 和 RISC-V Core 的 MemSpace 相互独立，通过 MemBus 的 visible_to 机制实现按需共享，天然支持片上安全隔离和子系统地址空间隔离。

### 3.5  L3 — Manifest Layer

声明式平台描述，是 OmniSim 和外部世界交互的语言。Realizer 负责把 Manifest（静态描述）翻译成运行时 Platform（Unit + Bus 实例）。

L3 知道 Unit 的配置描述，但不知道 Backend 的实现细节；Realizer 通过 ComponentRegistry 按 kind 字段找到对应工厂函数，实例化 Unit。这使得新组件无需修改框架，只需注册工厂即可接入。

### 3.6  L4 — Compatibility Layer

对外表现为 QEMU 进程，使 libvirt / OpenStack 无感接入，现有运维工具零改动。

#### CLI Parser

解析 QEMU 命令行参数格式，翻译为内部 Manifest。libvirt 生成的 -machine / -cpu / -device 参数可直接传入 omnisim-aarch64 / omnisim-x86_64 可执行文件（名字伪装为 qemu-system-*）。

#### QMP Server

实现 libvirt 所需的 QMP 命令子集：生命周期（query-status / stop / cont / system_reset）、热插拔（device_add / device_del → L2 动态添加/移除 Unit）、迁移（序列化 Manifest + Unit 状态）、查询（query-cpus / query-pci）。

## 4  关键设计决策

本章记录每个重要设计决策及其理由。这些决策定义了 OmniSim 的边界和取舍。

### 4.1  Bus 是一等公民，不藏在 Unit 内部

```
决策
Bus 是独立对象，在 Manifest 中显式声明，在运行时可查询，可序列化。不允许 Unit 之间通过私有引用直接通信。
```

理由：连接关系的显式化是整个组合框架的基础。只有当连接关系成为一等公民，才能做到：Manifest 序列化（用于迁移和云调度）、热插拔（动态添加/移除 Unit）、可观测性（debug 时查询连接拓扑）、跨 Composite Unit 边界连接。

### 4.2  CPU 和 NPU/DPU/GPU 对框架完全对等

```
决策
Unit trait 对所有硬件单元统一，框架层没有 CPU 专属路径。CPU 只是一种恰好实现了 ExecutionBackend 的 Unit。
```

理由：异构平台仿真的核心诉求。如果框架层给 CPU 特权，就无法优雅支持 GPU SM 阵列、DPU 网络处理核等"非传统 CPU"的执行单元。Unit trait 统一后，调度器对所有可执行 Unit 一视同仁，Backend 替换对框架透明。

### 4.3  Manifest 声明式，不命令式

```
决策
平台描述是数据（TOML/JSON），不是代码。Realizer 负责把数据翻译成运行时实例。
```

理由：声明式描述使 Manifest 版本管理、diff、审计和自动化工具成为可能。将平台描述保持为数据（而非代码），外部工具无需运行仿真即可分析拓扑、做出决策。

### 4.4  Backend × TimingModel 正交

```
决策
执行维度（怎么执行）和时序维度（时间如何推进）完全独立，任意组合均合法。
```

理由：不同场景对执行和时序的需求独立变化。验证软件正确性时只需 Functional + Untimed（最快）。性能建模时需要 Timing + LooseTiming。RTL 协同时需要 Rtl + CycleAccurate。如果两者耦合，每新增一种组合就需要一个新的 Backend 类型，指数爆炸。

### 4.5  omnisim-core 零强制依赖

```
决策
omnisim-core 只依赖 Rust 标准库，不引入任何第三方 crate。所有组件只依赖 omnisim-core，不互相依赖。
```

理由：保证组件可以独立编译、独立测试、独立发布。如果 core 引入重型依赖，所有组件都会被传递依赖污染。零依赖也使 omnisim-core 的接口定义足够稳定，不因上游依赖升级而产生破坏性变更。

### 4.6  Composite Unit 递归嵌套

```
决策
Unit 可以包含 sub_units，嵌套深度不限，内外通过边界 Port 交互。
```

理由：硬件本身就是递归结构。一块 GPGPU 卡内部有 ARM ctrl、SM 阵列、HBM 控制器，它们之间有独立的内部总线。如果不支持递归，要么把 GPGPU 展平成一堆 Leaf Unit（失去内部结构），要么用特殊的"Card"概念（破坏 Unit 的统一性）。Composite Unit 递归是最自然的解法。

### 4.7  生态兼容优先于洁癖

```
决策
L4 兼容层完整实现 QEMU CLI 和 QMP 协议，即使这使 L4 代码复杂。
```

理由：现有 libvirt / OpenStack 生态是存量资产，不能要求用户重写运维工具。兼容层的复杂度被封装在 L4，不污染 L1-L3 的设计，是可接受的代价。

## 5  演进路径

### 5.1  里程碑计划

| 里程碑 | 目标 | 关键交付 |
| --- | --- | --- |
| M0 架构 | omnisim-core 接口定稿 | Unit/Bus/Backend trait；MockBackend；两个异构 Unit 在同一 Platform 实例化 |
| M1 功能仿真 | 最小可运行仿真 | 移植 QEMU TCG；ARM + RISC-V 前端；L3 Manifest 加载；最小 EmbeddedProfile |
| M2 生态接入 | libvirt 可调用 | L4 CLI Parser；QMP Server 核心子集；KvmBackend |
| M3 异构平台 | 场景一、二跑通 | PCIe Bus；GPGPU Composite Unit；ResetBus；MemBus visible_to 过滤 |
| M4 性能仿真 | Timing Backend | TimingBackend + LooseTiming；SyncBus 时序协调；Cache/DDR 性能建模 |
| M5 RTL 协同 | RTL 协同验证 | RtlBackend + CycleAccurate；FPGA 桥接驱动；混合精度协同仿真 |
| M6 安全扩展 | TrustZone / 机密虚拟化 | EL2/EL3 TCG 前端；MemSecurity 检查；ARM CCA RMM；AMD SEV-SNP PSP |

### 5.2  各阶段接口稳定性承诺

```
接口稳定性规则
P0：omnisim-core 接口进入 1.0 后不做破坏性变更，只允许新增
P1：Timing Backend 接口，稳定性承诺随里程碑进度明确
P2：RTL/安全接口，版本独立管理
跨层规则：上层变更不影响下层接口；下层接口变更时上层必须同步更新
```
