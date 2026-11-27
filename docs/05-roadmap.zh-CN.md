> [English](./05-roadmap.md)

---

# RhetSim — 路线图

本文档记录项目级别的实现路线图。每个 Phase 是一个独立可交付的阶段，包含明确的准入条件、目标、验收标准与能力边界。

## 状态说明

| 标记 | 含义 |
|------|------|
| 已完成 | 已在 `dev` 分支实现并通过对应验收测试 |
| 进行中 | 在 `dev` 分支开发中 |
| 计划中 | 尚未开始 |

> 分支说明：`dev` 是当前开发主线，`main` 仍停留在初始工程骨架；阶段性成果在稳定后统一合入 `main`。下文状态以 `dev` 为准。

## 已完成阶段

### Phase 0：工程骨架（已完成）

**目标：** 核心 trait 定型；两个异构 Unit 在同一 Platform 中实例化。

**交付件：**
- `rhetsim-core`：Unit、Bus（MemBus/IrqBus/ResetBus 等）、Backend、MemTarget trait
- `rhetsim-runtime`：调度器、启动流程、时钟、Unit 线程
- `rhetsim-manifest`：加载器、验证器、展开器、实例化管线
- MockCPU + 端到端集成测试

**验收：** MockCPU + 端到端集成测试通过。

### Phase 1：RV64I 功能解释器（已完成）

**目标：** 最小可运行的 RISC-V 仿真，含内存和 UART。

**交付件：**
- `rhetsim-cpu-riscv`：RV64I 基础整数 ISA 解释器（译码 + 执行）
- `rhetsim-mem-dram`：DRAM 组件（Vec 后端，MemTarget）
- `rhetsim-mem-sram`：SRAM 组件
- `rhetsim-io-uart`：UART 组件
- `hello_riscv` 集成测试

**验收：** `hello_riscv` 在参考平台上输出预期字符串。

### Phase 2：异构 CPU+GPGPU 平台（已完成）

**目标：** CPU 和 GPGPU 共享 MemBus；GPGPU 可接收命令并访问 VRAM。

**交付件：**
- `rhetsim-accel-gpgpu`：CmdProcessor、VramController、复合 Unit
- MemBus `visible_to` 过滤，实现每发起者地址空间隔离
- `hetero_cpu_gpgpu` 集成测试
- 参考平台清单（`platforms/reference/`）

**能力边界：** GPGPU 为功能级模型，不含周期精确时序与完整指令级模拟。

### Phase 3：中断子系统 & RISC-V 补全（已完成）

**目标：** CPU 能响应中断；RISC-V 支持更广泛的真实程序。

**交付件：**
- `rhetsim-intc-clint`：CLINT 实现（mtime/mtimecmp、每 hart 的 MSIP）
- `rhetsim-intc-plic`：PLIC 实现（优先级、pending、enable、claim/complete）
- RISC-V CPU：CSR 基础设施（mstatus、mie、mip、mtvec、mcause、mepc、mtval）
- RISC-V CPU：陷阱交付（执行循环中的中断检查、向量化入口）
- RV64M 扩展：MUL、MULH、MULHSU、MULHU、DIV、DIVU、REM、REMU（含 W 变体）
- RV64A 扩展：LR.W/D、SC.W/D、AMO{SWAP,ADD,AND,OR,XOR,MIN,MAX,MINU,MAXU}.W/D
- RV64C 压缩指令译码（解释器与 JIT）
- Sstc（`stimecmp`）计时器支持（Linux 启动路径使用）
- 端到端测试：定时器中断触发 → 陷阱处理程序执行 → 返回用户代码

**验收：** 定时器 / PLIC / UART 中断端到端测试通过；Linux 启动中 `/proc/interrupts` 计数增长。

**能力边界：** F/D 浮点扩展尚未实现；DTS 与内核配置已对齐 C 扩展（见 Phase 6.5）。

### Phase 4：GPGPU 计算通路（已完成）

**目标：** GPGPU 能实际执行 CPU 分发的计算 kernel。

**交付件：**
- SimtHartBackend（路线 A）：RISC-V hart 阵列 SIMT 执行 ✓
- Warp 调度：round-robin ✓
- `rhetsim-mem-gddr`：GDDR 内存后端 ✓
- `rhetsim-io-pcie`：PCIe 总线基础实现（BAR 映射、配置空间、TLP 路由）✓
- 端到端测试：CPU 分发 kernel → GPGPU 执行 → 结果写回共享内存 ✓

**能力边界：** PCIe 为基础 TLP/BAR 通路，不含完整协议栈与设备枚举。

### Phase 5：ARM & 多 ISA 异构（已完成）

**目标：** 第二个真实 CPU 架构；三体异构平台可运行。

**交付件：**
- `rhetsim-cpu-arm`：AArch64 解释器（核心整数子集，参照 RV64I 实现模式）✓
- 端到端测试：ARM 主机 + RISC-V 协处理器共享 MemBus ✓
- `rhetsim-intc-gicv3`：GICv3 中断控制器（Distributor、Redistributor、CPU 接口）✓
- AArch64 CPU ↔ GICv3 对接（ICC 系统寄存器、IRQ 异常向量化、ERET）✓
- 端到端测试：ARM CPU + GICv3 中断驱动程序 ✓
- `rhetsim-io-dma`：DMA 控制器，支持跨设备批量内存搬运 ✓
- 端到端测试：ARM + RISC-V + GPGPU 同平台运行 ✓

**能力边界：** AArch64 为核心整数子集；GICv3 覆盖最小中断通路；不含 EL2/EL3 虚拟化扩展。

### Phase 6：生态 & 可用性（已完成）

**目标：** 用户可通过 CLI 加载 manifest、运行仿真、进行基础调试。

**交付件：**
- `rhetsim-cli`：功能化 CLI（加载 manifest → 实例化平台 → 运行）✓
- `rhetsim-compat` QMP 服务器：最小 VM 生命周期子集（query-status、quit、stop、cont）✓
- `rhetsim-io-virtio`：VirtIO 控制台设备（UART 的标准化替代）✓
- `rhetsim-dbg`：基础调试器（单步、寄存器/内存 dump、断点）✓

**能力边界：** VirtIO 仅实现 console；QMP 仅生命周期子集；调试器为基础功能。

## 进行中阶段

### Phase 6.5：Linux 系统软件 bring-up（进行中）

**目标：** 在 RhetSim 上稳定启动真实系统软件，并提供可交互的串口 shell。

**交付件：**
- OpenSBI（`fw_jump`）+ Linux 6.1 双 hart SMP 启动至内嵌 initramfs ✓
- PLIC/UART 中断、Sstc 计时器、per-hart CLINT/PLIC 上下文 ✓
- 内核 printk 时间戳（`printk.time=1`）✓
- 基于 BusyBox 的根文件系统（rv64ima 软浮点静态构建）✓
- `/init` 交互式 shell（ash）与基础命令集 ✓
- JIT 数据访问缺页异常交付（用户态按需分页 / A 位处理）✓
- 内核配置 ↔ DTS 扩展声明一致性修复（C 扩展）✓

**验收标准：**
- batch 模式输出包含 `Run /init as init process` 与 `RHETSIM_INIT_OK`，进程退出码 0
- `/proc/interrupts` 中 PLIC `ttyS0` 与 `riscv-timer` 计数在 1 秒休眠后增长
- 交互模式可执行 `ls`/`cat` 等命令，`Ctrl-A X` 正常退出

**非目标：** F/D 浮点用户态、virtio-blk 根文件系统（见 Phase 6.6 / 6.7）。

## 计划阶段

### Phase 6.6：ISA 完整性 & 合规（计划中）

**目标：** 让模拟器的 ISA 能力真实、完整、可验证。

**准入：** Phase 6.5 完成，Linux 可稳定启动到用户态。

**交付件：**
- RV64F/D 浮点扩展（解释器 + JIT，CSR 与 `mstatus.FS` 状态管理）
- DTS 如实声明 C/F/D 扩展；内核启用 `CONFIG_FPU` 并重建
- riscv-tests / riscv-arch-test 合规测试套件接入
- 与 Spike/QEMU 的指令级 difftest 框架

**验收：** arch-test 通过率达到约定阈值；difftest 随机指令流无差异。

**能力边界：** 向量扩展（V）不在本阶段范围。

### Phase 6.7：设备模型 & 根文件系统（计划中）

**目标：** 支撑真实 rootfs、正常关机与基础网络。

**交付件：**
- virtio-mmio block（以及可选的 virtio-net）
- RTC 与 syscon poweroff/reboot（OpenSBI 相应设备可见）
- Buildroot/BusyBox rootfs 镜像与启动脚本
- CI：OpenSBI/Linux 构建缓存、启动回归（golden log）

**验收：** 从 virtio-blk 挂载 ext4 rootfs 启动到 shell；`poweroff` 可使模拟器正常退出。

**能力边界：** 不含完整 PCIe 设备生态。

### Phase 7：性能建模（计划中）

**目标：** 从功能仿真进入时序仿真。

**准入：** Phase 6.6 合规测试与 difftest 基线完成（功能正确性可证）。

**交付件：**
- TimingBackend 实现 + LooseTiming 模式
- SyncBus 时序协调（多 Unit 时钟同步）
- `rhetsim-cache`：L1/L2 缓存层级（当前为占位实现），可配置替换策略与延迟
- DDR 控制器时序模型（bank conflict、refresh、tCAS/tRCD/tRP）

**非目标：** 周期精确的 CPU 流水线模型（另行评估）。

### Phase 8：JIT/TCG & RTL 协仿真（计划中）

**目标：** 动态二进制翻译管线 + RTL 验证桥。

**前置决策：** 现有 `rhetsim-cpu-riscv` 的 Cranelift JIT 是保留升级还是被自研 TCG 替换。

**交付件：**
- `rhetsim-tcg`：IR 实现、基础优化 pass、TB cache（骨架已存在）
- x86-64 Host Backend（JIT 到宿主机）
- RISC-V / ARM 前端迁移到 TCG 管线
- RtlBackend 接口 + FPGA bridge driver
- 混合精度协仿真（功能级 + 周期精确）

**能力边界：** 混合精度协仿真为实验特性。

### Phase 9：安全扩展（计划中）

**目标：** 机密计算 / TrustZone 建模。

**前置决策：** 是否纳入 RISC-V H 扩展 / CoVE 路线。

**交付件：**
- ARM EL2/EL3 TCG 前端
- MemSecurity 检查层（地址空间隔离）
- ARM CCA RMM 建模
- AMD SEV-SNP PSP 建模

---

## 依赖关系

```text
Phase 6.5 (系统软件) ───┐
Phase 6.6 (ISA/合规) ───┼── Phase 7 (性能建模) ── Phase 8 (TCG/RTL) ── Phase 9 (安全)
Phase 6.7 (设备/rootfs) ┘
```

Phase 6.6 是 Phase 7 的准入条件；Phase 8 开始前需完成 JIT 路线决策。

## 里程碑 ↔ Phase 对照

原架构文档（01-architecture.md §5.1）定义了 M0–M6 里程碑。Phase 计划是基于实际实现进展的细化：

| 原始里程碑 | 对应 Phase |
|---|---|
| M0 架构 | Phase 0 |
| M1 功能仿真 | Phase 1 + Phase 3 + Phase 5 |
| M2 生态集成 | Phase 6 + Phase 6.5 |
| M3 异构平台 | Phase 2 + Phase 4 |
| M4 性能仿真 | Phase 7 |
| M5 RTL 协仿真 | Phase 8 |
| M6 安全扩展 | Phase 9 |

> Phase 6.6 / 6.7 为原 M0–M6 之外新增的 ISA 正确性与系统可用性阶段，不直接对应原始里程碑。
