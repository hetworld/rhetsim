> [English](./05-roadmap.md)

---

# RhetSim — 路线图

本文档记录项目级别的实现路线图。每个 Phase 是一个独立可交付的阶段，包含明确的准入条件、目标和验证测试。

## 状态说明

| 标记 | 含义 |
|------|------|
| 已完成 | 已合入 `main` |
| 进行中 | 在 `dev` 分支开发 |
| 计划中 | 尚未开始 |

## 已完成阶段

### Phase 0：工程骨架（已完成）

**目标：** 核心 trait 定型；两个异构 Unit 在同一 Platform 中实例化。

**交付件：**
- `rhetsim-core`：Unit、Bus（MemBus/IrqBus/ResetBus 等）、Backend、MemTarget trait
- `rhetsim-runtime`：调度器、启动流程、时钟、Unit 线程
- `rhetsim-manifest`：加载器、验证器、展开器、实例化管线
- MockCPU + 端到端集成测试

### Phase 1：RV64I 功能解释器（已完成）

**目标：** 最小可运行的 RISC-V 仿真，含内存和 UART。

**交付件：**
- `rhetsim-cpu-riscv`：RV64I 基础整数 ISA 解释器（译码 + 执行）
- `rhetsim-mem-dram`：DRAM 组件（Vec 后端，MemTarget）
- `rhetsim-mem-sram`：SRAM 组件
- `rhetsim-io-uart`：UART 组件
- `hello_riscv` 集成测试

### Phase 2：异构 CPU+GPGPU 平台（已完成）

**目标：** CPU 和 GPGPU 共享 MemBus；GPGPU 可接收命令并访问 VRAM。

**交付件：**
- `rhetsim-accel-gpgpu`：CmdProcessor、VramController、复合 Unit
- MemBus `visible_to` 过滤，实现每发起者地址空间隔离
- `hetero_cpu_gpgpu` 集成测试
- 参考平台清单（`platforms/reference/`）

---

## 计划阶段

### Phase 3：中断子系统 & RISC-V 补全（已完成）

**目标：** CPU 能响应中断；RISC-V 支持更广泛的裸机程序。

**交付件：**
- `rhetsim-intc-clint`：CLINT 实现（mtime/mtimecmp、每 hart 的 MSIP）
- `rhetsim-intc-plic`：PLIC 实现（优先级、pending、enable、claim/complete）
- RISC-V CPU：CSR 基础设施（mstatus、mie、mip、mtvec、mcause、mepc、mtval）
- RISC-V CPU：陷阱交付（执行循环中的中断检查、向量化入口）
- RV64M 扩展：MUL、MULH、MULHSU、MULHU、DIV、DIVU、REM、REMU（含 W 变体）
- RV64A 扩展：LR.W/D、SC.W/D、AMO{SWAP,ADD,AND,OR,XOR,MIN,MAX,MINU,MAXU}.W/D
- 端到端测试：定时器中断触发 → 陷阱处理程序执行 → 返回用户代码

### Phase 4：GPGPU 计算通路（计划中）

**目标：** GPGPU 能实际执行 CPU 分发的计算 kernel。

**交付件：**
- SimtHartBackend（路线 A）：RISC-V hart 阵列 SIMT 执行
- Warp 调度：round-robin 或 greedy-then-oldest
- `rhetsim-mem-gddr`：GDDR 内存后端
- `rhetsim-io-pcie`：PCIe 总线基础实现（BAR 映射、配置空间、TLP 路由）
- 端到端测试：CPU 分发 kernel → GPGPU 执行 → 结果写回共享内存

### Phase 5：ARM & 多 ISA 异构（进行中）

**目标：** 第二个真实 CPU 架构；三体异构平台可运行。

**交付件：**
- `rhetsim-cpu-arm`：AArch64 解释器（核心整数子集，参照 RV64I 实现模式）✓
- 端到端测试：ARM 主机 + RISC-V 协处理器共享 MemBus ✓
- `rhetsim-intc-gicv3`：GICv3 中断控制器（Distributor、Redistributor、CPU 接口）✓
- AArch64 CPU ↔ GICv3 对接（ICC 系统寄存器、IRQ 异常向量化、ERET）✓
- 端到端测试：ARM CPU + GICv3 中断驱动程序 ✓
- `rhetsim-io-dma`：DMA 控制器，支持跨设备批量内存搬运
- 端到端测试：ARM + RISC-V + GPGPU 同平台运行

### Phase 6：生态 & 可用性（计划中）

**目标：** 用户可通过 CLI 加载 manifest、运行仿真、进行基础调试。

**交付件：**
- `rhetsim-cli`：功能化 CLI（加载 manifest → 实例化平台 → 运行）
- `rhetsim-compat` QMP 服务器：最小 VM 生命周期子集（query-status、quit、stop、cont）
- `rhetsim-io-virtio`：VirtIO 控制台设备（UART 的标准化替代）
- `rhetsim-dbg`：基础调试器（单步、寄存器/内存 dump、断点）

### Phase 7：性能建模（计划中）

**目标：** 从功能仿真进入时序仿真。

**交付件：**
- TimingBackend 实现 + LooseTiming 模式
- SyncBus 时序协调（多 Unit 时钟同步）
- `rhetsim-cache`：L1/L2 缓存层级，可配置替换策略与延迟
- DDR 控制器时序模型（bank conflict、refresh、tCAS/tRCD/tRP）

### Phase 8：TCG & RTL 协仿真（计划中）

**目标：** 动态二进制翻译管线 + RTL 验证桥。

**交付件：**
- `rhetsim-tcg`：IR 实现、基础优化 pass
- x86-64 Host Backend（JIT 到宿主机）
- RISC-V / ARM 前端迁移到 TCG 管线
- RtlBackend 接口 + FPGA bridge driver
- 混合精度协仿真（功能级 + 周期精确）

### Phase 9：安全扩展（计划中）

**目标：** 机密计算 / TrustZone 建模。

**交付件：**
- ARM EL2/EL3 TCG 前端
- MemSecurity 检查层（地址空间隔离）
- ARM CCA RMM 建模
- AMD SEV-SNP PSP 建模

---

## 依赖关系

```
Phase 3 (中断) ──┬── Phase 4 (GPGPU) ──┐
                 │                      ├── Phase 7 (性能建模) ── Phase 8 (TCG/RTL)
                 └── Phase 5 (ARM) ─────┘        │
                                                  └── Phase 9 (安全)
                      Phase 6 (生态) 可与 4/5 并行
```

## 里程碑 ↔ Phase 对照

原架构文档（01-architecture.md §5.1）定义了 M0–M6 里程碑。Phase 计划是基于实际实现进展的细化：

| 原始里程碑 | 对应 Phase |
|---|---|
| M0 架构 | Phase 0 |
| M1 功能仿真 | Phase 1 + Phase 3 + Phase 5 |
| M2 生态集成 | Phase 6 |
| M3 异构平台 | Phase 2 + Phase 4 |
| M4 性能仿真 | Phase 7 |
| M5 RTL 协仿真 | Phase 8 |
| M6 安全扩展 | Phase 9 |
