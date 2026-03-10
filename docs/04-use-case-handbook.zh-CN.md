> [English](./04-use-case-handbook.md)

---

# OmniSim — 应用场景手册

> **文档编号**：DOC-04 &nbsp;&nbsp; **版本**：v1.0 &nbsp;&nbsp; **日期**：2026-Q1 &nbsp;&nbsp; **作者**：OpenFunTech

---

| 文档编号 | DOC-04 |
| --- | --- |
| 文档版本 | v1.0 |
| 适用读者 | 外部用户（使用 OmniSim 搭建平台）· 新入贡献者 |
| 前置阅读 | DOC-01 架构设计（推荐） |
| 关联文档 | DOC-02 接口规范 · DOC-03 组件开发指南 |
| 上次更新 | 2026-Q1 |

## 1  如何使用本手册

本手册按场景组织，每个场景独立成节，互不依赖。直接跳到你关心的场景即可。

### 1.1  Manifest 文件结构速查

```toml
# platform.toml 顶层结构
[platform]
name    = "my-platform"    # 平台名称
version = "1.0"
[[units]]                  # 一个 Unit 一个 [[units]] 块
id      = "cpu0"           # 唯一 ID，可被 Bus 引用
kind    = "arm-cortex-a78-core"  # 组件类型（Registry 查找）
backend = "kvm"            # 执行维度：functional/kvm/timing/mock
timing  = "untimed"        # 时序维度：untimed/loose/cycle_accurate
[units.config]             # 组件特定配置
num_cores = 4
[[units.sub_units]]      # Composite Unit：子 Unit 嵌套
id   = "cpu0.l1-cache"
kind = "arm-l1-cache"
[[buses]]                  # 一个 Bus 一个 [[buses]] 块
kind = "mem"               # Bus 类型：mem/irq/pcie/reset/power/interconnect
id   = "main-mem-bus"
[[buses.targets]]
unit = "dram0"; base = "0x80000000"
[[boot]]                   # 启动序列（可选）
[[boot.sequence]]
action = "deassert_reset"; bus = "soc-reset-bus"
```

### 1.2  正交维度说明

```
三个正交维度，与场景拓扑无关
Host ISA：x86-64 / AArch64 / RISC-V，任意 Unit 均可组合
Host Backend：kvm（同架构硬件加速）/ functional（跨架构软件仿真）
仿真精度：functional（验证正确性）/ timing（性能建模）/ cycle_accurate（RTL 协同仿真）
本手册场景示例均使用 functional 精度，替换 backend/timing 字段即可切换
```

### 1.3  场景索引

| 场景 | 类别 | 核心特性 | 章节 |
| --- | --- | --- | --- |
| 片上异构双核（ARM + RISC-V） | 基础异构 | ResetBus · visible_to | §2.1 |
| PCIe GPGPU（两种 SM 架构） | 基础异构 | SimtHartBackend / SimtWarpBackend | §2.2 |
| PCIe DPU（I/O 卸载） | 基础异构 | EP DMA · NetworkBackend | §2.3 |
| 多卡互联 | 基础异构 | InterconnectBus · Windowed/Unified | §2.4 |
| 嵌入式 SoC（多异构核） | 基础异构 | PowerBus · DSP · NPU | §2.5 |
| 网络处理器 | 网络存储 | DPU 框架 + 硬件转发表 | §3.1 |
| SmartSSD / 计算存储 | 网络存储 | DPU 框架 + NAND 控制器 | §3.2 |
| CXL 设备 | 网络存储 | InterconnectBus MemoryDevice | §3.3 |
| 多路服务器 NUMA | NUMA | InterconnectBus CpuSocket · Unified | §4.1 |
| 大内存 SoC NUMA | NUMA | InterconnectBus 片上 NoC | §4.2 |
| 嵌套虚拟化 | 虚拟化 | Stage-2 MMU · TCG EL2 | §5.1 |
| TrustZone / TEE | 安全 | MemSecurity · SecurityWorld | §6.1 |
| 机密虚拟化（P2） | 安全 | SEV-SNP PSP · ARM CCA RMM | §6.2 |
| AI 服务器全栈 | 综合平台 | 多卡 + DPU + 片上 NPU | §7.1 |
| 异构内存平台 | 综合平台 | DDR + CXL + NVMe-oF 分层 | §7.2 |
| NUMA + 多卡混合 | 综合平台 | 两层 InterconnectBus 级联 | §7.3 |

## 2  基础异构计算场景

### 2.1  场景一：片上异构双核（ARM + RISC-V 协处理器）

ARM 主核运行 Linux，RISC-V 协处理器运行 RTOS，两核通过 Mailbox 通信。ARM 控制 RISC-V 的复位和启动顺序。

#### 关键配置要点

ResetBus：ARM 的 Reset Controller 控制 RISC-V 的 reset_in，RISC-V 在 deassert 后才开始执行

MemBus visible_to：ITCM（RISC-V 专属 SRAM）只对 RISC-V Core 可见，ARM 无法访问

IrqBus：跨子系统中断，RISC-V 完成任务后通过 Mailbox 中断通知 ARM

BootSequencer：先 deassert ARM 复位，ARM 初始化完成后再 deassert RISC-V 复位

#### 完整 Manifest

```toml
[platform]
name = "arm-riscv-soc"
# ARM 主核子系统
[[units]]
id = "arm-ss"; kind = "composite"; backend = "functional"
[[units.sub_units]]
id = "arm-ss.cpu"; kind = "arm-cortex-a78-core"; backend = "kvm"
[[units.sub_units]]
id = "arm-ss.gicv3"; kind = "arm-gicv3"; backend = "functional"
# RISC-V 协处理器子系统
[[units]]
id = "riscv-ss"; kind = "composite"; backend = "functional"
[[units.sub_units]]
id = "riscv-ss.rv64"; kind = "riscv-rv64-core"; backend = "functional"
[[units.sub_units]]
id = "riscv-ss.plic"; kind = "riscv-plic"; backend = "functional"
[[units.sub_units]]
id = "riscv-ss.itcm"; kind = "sram"; backend = "functional"
[units.sub_units.config]
size_kb = 64
# 片上共享资源
[[units]]
id = "ddr"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
size_gb = 4
[[units]]
id = "mailbox"; kind = "hw-mailbox"; backend = "functional"
[[units]]
id = "reset-ctrl"; kind = "soc-reset-ctrl"; backend = "functional"
# ARM MemBus（可见所有共享设备）
[[buses]]
kind = "mem"; id = "arm-mem-bus"
[[buses.targets]]
unit = "ddr";     base = "0x80000000"
[[buses.targets]]
unit = "mailbox"; base = "0x09000000"
[[buses.targets]]
unit = "arm-ss.gicv3.gicd"; base = "0x08000000"
# RISC-V MemBus（可见 ITCM + 共享设备，不可见 GIC）
[[buses]]
kind = "mem"; id = "riscv-mem-bus"
[[buses.targets]]
unit = "ddr";               base = "0x80000000"
[[buses.targets]]
unit = "mailbox";           base = "0x02000000"
[[buses.targets]]
unit = "riscv-ss.plic";     base = "0x0c000000"
[[buses.targets]]
unit = "riscv-ss.itcm"
base = "0x00000000"
visible_to = ["riscv-ss.rv64"]   # 只对 RISC-V Core 可见
# 中断总线
[[buses]]
kind = "irq"; id = "irq-bus"
[[buses.connections]]
from = "mailbox.irq_to_arm";   to = "arm-ss.gicv3.spi[32]"
[[buses.connections]]
from = "mailbox.irq_to_riscv"; to = "riscv-ss.plic.irq[1]"
# 复位总线
[[buses]]
kind = "reset"; id = "soc-reset-bus"
[[buses.connections]]
from = "reset-ctrl.riscv_reset"
to   = "riscv-ss.rv64.reset_in"
# 启动序列
[[boot.sequence]]
action = "deassert_reset"; bus = "soc-reset-bus"
[[boot.sequence]]
action = "wait_ready"; unit = "arm-ss.cpu"; timeout_ns = 5_000_000_000
```

### 2.2  场景二：PCIe GPGPU

Host 通过 PCIe 连接 GPGPU 加速卡。GPGPU 内部是 Composite Unit，包含 ARM 管理核、SM 阵列、HBM。

#### SM 架构选择

| 路线 | Backend | 适用场景 | 配置字段 |
| --- | --- | --- | --- |
| 路线 A | SimtHartBackend | 需要跑 RISC-V 二进制（如 PTX 编译到 RVV） | sm_backend = "simt_hart" |
| 路线 B | SimtWarpBackend | 自定义 ISA，Warp 调度器语义精确 | sm_backend = "simt_warp" |

#### 完整 Manifest（路线 A）

```toml
[platform]
name = "host-gpgpu"
# Host
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
# GPGPU（Composite PCIe EP）
[[units]]
id = "gpgpu0"; kind = "gpgpu-card"; backend = "functional"
[units.config]
sm_count        = 128
threads_per_warp = 32
sm_backend      = "simt_hart"   # 路线 A：RISC-V hart 阵列
hbm_size_gb     = 16
# PCIe 总线
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "gpgpu0"
# PCIe 复位总线（PERST#）
[[buses]]
kind = "reset"; id = "pcie-reset"
[[buses.connections]]
from = "host.pcie-rc.perst#"
to   = "gpgpu0.reset_in"
# 启动序列
[[boot.sequence]]
action = "deassert_reset"; bus = "pcie-reset"
[[boot.sequence]]
action = "wait_links_up"; bus = "pcie0"; timeout_ns = 10_000_000_000
[[boot.sequence]]
action = "wait_ready"; unit = "gpgpu0.arm-ctrl"; timeout_ns = 5_000_000_000
```

### 2.3  场景三：PCIe DPU

DPU 卸载 Host 的网络和存储 I/O，对 Host 暴露标准 virtio 接口。DPU 内部通过 nvme-of 访问远端存储，通过内部二层交换转发网络流量。

```toml
[platform]
name = "host-dpu"
[[units]]
id = "host"; kind = "arm-host"; backend = "kvm"
[[units]]
id = "dpu0"; kind = "dpu-card"; backend = "functional"
[units.config]
virtio_net_queues = 4
virtio_blk_queues = 2
nvmeof_target     = "192.168.1.100:4420"   # 远端存储（HostSocketBackend）
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "dpu0"
[[buses]]
kind = "reset"; id = "pcie-reset"
[[buses.connections]]
from = "host.pcie-rc.perst#"; to = "dpu0.reset_in"
```

### 2.4  场景四：多卡互联

多块 GPGPU 通过 InterconnectBus 实现卡间直接访问，无需经过 Host CPU。

```toml
[platform]
name = "multi-gpu"
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
[[units]]
id = "gpgpu0"; kind = "gpgpu-card"; backend = "functional"
[units.config]
vram_base_addr = "0x0000000000000000"
vram_size_gb   = 16
[[units]]
id = "gpgpu1"; kind = "gpgpu-card"; backend = "functional"
[units.config]
vram_base_addr = "0x0000000400000000"
vram_size_gb   = 16
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "gpgpu0"
[[buses.endpoints]]
unit = "gpgpu1"
# NVLink 风格卡间互联
[[buses]]
kind = "interconnect"; id = "gpu-fabric"
[buses.config]
addr_mode      = "windowed"
backend        = "direct_fabric"
bandwidth_gbps = 600
latency_ns     = 100
[[buses.nodes]]
unit = "gpgpu0"; node_kind = "accelerator"
base_addr = "0x0000000000000000"; size = "0x400000000"
[[buses.nodes]]
unit = "gpgpu1"; node_kind = "accelerator"
base_addr = "0x0000000400000000"; size = "0x400000000"
```

### 2.5  场景五：嵌入式 SoC（多异构核）

包含 ARM 主核、RISC-V 小核、DSP、NPU、FPGA 的完整嵌入式 SoC，支持电源域管理。

```toml
[platform]
name = "embedded-soc"
[[units]]
id = "arm-core"; kind = "arm-cortex-a55-core"; backend = "kvm"
[[units]]
id = "riscv-core"; kind = "riscv-rv32-core"; backend = "functional"
[[units]]
id = "dsp0"; kind = "dsp-core"; backend = "functional"
[units.config]
isa = "custom-dsp-v2"
[[units]]
id = "npu"; kind = "npu-accel"; backend = "mock"
[[units]]
id = "power-ctrl"; kind = "soc-power-ctrl"; backend = "functional"
[[units]]
id = "reset-ctrl"; kind = "soc-reset-ctrl"; backend = "functional"
# 电源总线
[[buses]]
kind = "power"; id = "soc-power-bus"
[[buses.connections]]
controller = "power-ctrl"; target = "npu";       domain = "PD_NPU"
[[buses.connections]]
controller = "power-ctrl"; target = "dsp0";      domain = "PD_DSP0"
[[buses.connections]]
controller = "power-ctrl"; target = "riscv-core"; domain = "PD_RISCV"
# 复位总线
[[buses]]
kind = "reset"; id = "soc-reset-bus"
[[buses.connections]]
from = "reset-ctrl.dsp_reset";   to = "dsp0.reset_in"
[[buses.connections]]
from = "reset-ctrl.riscv_reset"; to = "riscv-core.reset_in"
[[buses.connections]]
from = "reset-ctrl.npu_reset";   to = "npu.reset_in"
```

## 3  网络与存储场景

### 3.1  场景六：网络处理器

多核 MIPS/ARM 处理器，配合硬件转发表、包分类引擎、加解密引擎。

```toml
[platform]
name = "network-processor"
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
[[units]]
id = "np0"; kind = "net-processor-card"; backend = "functional"
[units.config]
core_count  = 8
core_isa    = "mips64"
hw_fwd_table = true   # omnisim-net-hwfwd 组件
crypto_engine = true  # MockBackend
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "np0"
```

### 3.2  场景七：SmartSSD / 计算存储

```toml
[platform]
name = "smart-ssd"
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
[[units]]
id = "ssd0"; kind = "smart-ssd-card"; backend = "functional"
[units.config]
capacity_tb    = 4
compute_engine = "compress+encrypt"   # MockBackend
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "ssd0"
```

### 3.3  场景八：CXL 设备

CXL.io 通过 PcieBus 覆盖（设备发现），CXL.mem 通过 InterconnectBus Unified 覆盖（Host 直接寻址设备内存）。

```toml
[platform]
name = "cxl-memory-device"
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
[[units]]
id = "cxl-mem0"; kind = "cxl-memory-expander"; backend = "functional"
[units.config]
capacity_gb = 512
latency_ns  = 200
# CXL.io：设备发现和配置（PcieBus 覆盖）
[[buses]]
kind = "pcie"; id = "cxl0"
root_complex = "host.cxl-rc"
[[buses.endpoints]]
unit = "cxl-mem0"
# CXL.mem：Host 直接寻址设备内存（InterconnectBus Unified）
[[buses]]
kind = "interconnect"; id = "cxl-mem-bus"
[buses.config]
addr_mode  = "unified"
backend    = "direct_fabric"
latency_ns = 200
[[buses.nodes]]
unit = "host"; node_kind = "cpu_socket"
base_addr = "0x000000000000"; size = "0x4000000000"   # 256GB DDR
[[buses.nodes]]
unit = "cxl-mem0"; node_kind = "memory_device"
base_addr = "0x400000000000"; size = "0x8000000000"   # 512GB CXL
```

## 4  NUMA 场景

```
两种 NUMA 形态共用相同框架
多路服务器 NUMA：Socket 间通过 UPI / Infinity Fabric 互联，latency ~150ns
大内存 SoC NUMA：计算 Cluster 通过片上 NoC 互联，latency ~30ns
两种形态 Manifest 结构完全相同，只有 latency_ns / bandwidth_gbps 参数不同
```

### 4.1  场景九：多路服务器 NUMA

```toml
[platform]
name = "dual-socket-server"
[[units]]
id = "socket0"; kind = "x86-cpu-socket"; backend = "kvm"
[units.config]
num_cores = 32
local_dram_gb = 256
[[units]]
id = "socket1"; kind = "x86-cpu-socket"; backend = "kvm"
[units.config]
num_cores = 32
local_dram_gb = 256
[[buses]]
kind = "interconnect"; id = "upi-fabric"
[buses.config]
addr_mode      = "unified"
backend        = "direct_fabric"
bandwidth_gbps = 400
latency_ns     = 150
[[buses.nodes]]
unit = "socket0"; node_kind = "cpu_socket"
base_addr = "0x000000000000"; size = "0x4000000000"
[[buses.nodes]]
unit = "socket1"; node_kind = "cpu_socket"
base_addr = "0x400000000000"; size = "0x4000000000"
```

### 4.2  场景十：大内存 SoC NUMA

```toml
[platform]
name = "large-mem-soc-numa"
[[units]]
id = "cluster0"; kind = "arm-compute-cluster"; backend = "kvm"
[units.config]
num_cores    = 8
local_hbm_gb = 32
[[units]]
id = "cluster1"; kind = "arm-compute-cluster"; backend = "kvm"
[units.config]
num_cores    = 8
local_hbm_gb = 32
# 片上 NoC：比服务器 NUMA 延迟低 5x，带宽高 5x
[[buses]]
kind = "interconnect"; id = "noc-fabric"
[buses.config]
addr_mode      = "unified"
backend        = "direct_fabric"
bandwidth_gbps = 2000
latency_ns     = 30
[[buses.nodes]]
unit = "cluster0"; node_kind = "cpu_socket"
base_addr = "0x000000000000"; size = "0x800000000"
[[buses.nodes]]
unit = "cluster1"; node_kind = "cpu_socket"
base_addr = "0x800000000000"; size = "0x800000000"
```

## 5  虚拟化场景

### 5.1  场景十一：嵌套虚拟化

OmniSim 仿真完整物理机（TCG 全系统仿真），物理机上运行真实的 Hypervisor 软件（KVM / LeoVirt-S），Hypervisor 再管理多个 VM。框架零改动，嵌套能力完全由 omnisim-tcg 的 EL2/VMX/H扩展前端提供。

```
版本要求
ARM 嵌套虚拟化：需要 omnisim-tcg P1（EL2 特权级仿真 + vGIC），非 P0 开源版本功能
x86 嵌套虚拟化：需要 omnisim-tcg P1（VMX 指令集 + EPT），非 P0 开源版本功能
RISC-V 嵌套虚拟化：需要 omnisim-tcg P1（H 扩展），非 P0 开源版本功能
```

```toml
[platform]
name = "nested-virt"
# 仿真物理机（不是 VM！是 OmniSim 仿真的硬件）
[[units]]
id = "phys-cpu"; kind = "arm-cortex-a78-core"; backend = "functional"
[units.config]
# 关键：使能 EL2 特权级仿真（P1 特性）
enable_el2 = true
enable_vhe = true   # ARM Virtualization Host Extensions
[[units]]
id = "phys-gicv3"; kind = "arm-gicv3"; backend = "functional"
[units.config]
enable_virt = true  # GICv3 虚拟化扩展（vGIC）
[[units]]
id = "phys-dram"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
size_gb = 64
# Stage-2 MMU 由 omnisim-tcg EL2 前端自动管理
# 不需要在 Manifest 中显式配置
# Hypervisor 软件（KVM / LeoVirt-S）以普通 ELF 加载
[[boot.sequence]]
action = "load_elf"
unit   = "phys-cpu"
path   = "/path/to/hypervisor.elf"
```

## 6  安全场景

### 6.1  场景十二：TrustZone / TEE

```toml
[platform]
name = "trustzone-soc"
[[units]]
id = "cpu"; kind = "arm-cortex-a78-core"; backend = "kvm"
[units.config]
enable_el3 = true   # TrustZone 需要 EL3
# 安全内存：只有 Secure World 可访问
[[units]]
id = "secure-sram"; kind = "sram"; backend = "functional"
[units.config]
size_kb  = 256
security = "secure"    # MemSecurity::Secure
# 普通内存
[[units]]
id = "normal-dram"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
size_gb  = 8
security = "non_secure"
[[buses]]
kind = "mem"; id = "mem-bus"
[[buses.targets]]
unit = "normal-dram"; base = "0x80000000"
[[buses.targets]]
unit = "secure-sram"; base = "0x0e000000"
# P0：安全检查为 no-op，Secure World 软件正确性可验证
# P2：MemBus 实施真正隔离，Normal World 访问 Secure 内存返回错误
```

### 6.2  场景十三：机密虚拟化（P2）

```
P2 特性，当前版本未实现
AMD SEV-SNP：需要 omnisim-security-psp 组件（P2）
ARM CCA：需要 omnisim-security-rmm 组件（P2）+ omnisim-tcg Realm World 支持
P0/P1 版本：MemSecurity 枚举已预留，接口已定义，但隔离检查为 no-op
```

```toml
# P2 预留 Manifest 结构（当前版本不生效）
# AMD SEV-SNP
[platform]
name = "sev-snp-platform"
[[units]]
id = "psp"; kind = "amd-psp"; backend = "mock"   # P2：functional
[[units]]
id = "vm0-mem"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
security = "encrypted_private"
asid     = 1
# ARM CCA
[platform]
name = "arm-cca-platform"
[[units]]
id = "rmm"; kind = "arm-rmm"; backend = "mock"   # P2：functional
[[units]]
id = "realm-mem"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
security = "realm"
```

## 7  综合异构平台场景

### 7.1  场景十四：AI 服务器全栈

ARM SoC 主控 + 片上 NPU + PCIe GPGPU × 4（NVLink 风格互联）+ DPU × 1。

```toml
[platform]
name = "ai-server"
[[units]]
id = "soc"; kind = "arm-soc"; backend = "kvm"
[units.config]
num_cores = 64
on_chip_npu = true   # 片上 NPU（MockBackend）
[[units]]
id = "gpgpu0"; kind = "gpgpu-card"; backend = "functional"
[units.config]
sm_count = 128; hbm_size_gb = 80; vram_base = "0x00000000000"
[[units]]
id = "gpgpu1"; kind = "gpgpu-card"; backend = "functional"
[units.config]
sm_count = 128; hbm_size_gb = 80; vram_base = "0x01400000000"
[[units]]
id = "gpgpu2"; kind = "gpgpu-card"; backend = "functional"
[units.config]
sm_count = 128; hbm_size_gb = 80; vram_base = "0x02800000000"
[[units]]
id = "gpgpu3"; kind = "gpgpu-card"; backend = "functional"
[units.config]
sm_count = 128; hbm_size_gb = 80; vram_base = "0x03c00000000"
[[units]]
id = "dpu0"; kind = "dpu-card"; backend = "functional"
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "soc.pcie-rc"
[[buses.endpoints]]
unit = "gpgpu0"
[[buses.endpoints]]
unit = "gpgpu1"
[[buses.endpoints]]
unit = "gpgpu2"
[[buses.endpoints]]
unit = "gpgpu3"
[[buses.endpoints]]
unit = "dpu0"
# GPU 卡间 NVLink 风格互联
[[buses]]
kind = "interconnect"; id = "gpu-fabric"
[buses.config]
addr_mode = "windowed"; backend = "direct_fabric"
bandwidth_gbps = 600; latency_ns = 100
[[buses.nodes]]
unit = "gpgpu0"; node_kind = "accelerator"
base_addr = "0x00000000000"; size = "0x1400000000"
[[buses.nodes]]
unit = "gpgpu1"; node_kind = "accelerator"
base_addr = "0x01400000000"; size = "0x1400000000"
[[buses.nodes]]
unit = "gpgpu2"; node_kind = "accelerator"
base_addr = "0x02800000000"; size = "0x1400000000"
[[buses.nodes]]
unit = "gpgpu3"; node_kind = "accelerator"
base_addr = "0x03c00000000"; size = "0x1400000000"
```

### 7.2  场景十五：异构内存平台

本地 DDR（低延迟）+ CXL 内存扩展（中等延迟）+ 远端 NVMe-oF（高延迟），构成三层内存层次。

```toml
[platform]
name = "heterogeneous-memory"
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
[units.config]
local_dram_gb = 128
[[units]]
id = "cxl-mem0"; kind = "cxl-memory-expander"; backend = "functional"
[units.config]
capacity_gb = 512; latency_ns = 200
[[units]]
id = "nvmeof0"; kind = "nvmeof-initiator"; backend = "functional"
[units.config]
target = "192.168.1.100:4420"   # HostSocketBackend
# CXL.mem 接入
[[buses]]
kind = "interconnect"; id = "cxl-bus"
[buses.config]
addr_mode = "unified"; backend = "direct_fabric"; latency_ns = 200
[[buses.nodes]]
unit = "host"; node_kind = "cpu_socket"
base_addr = "0x000000000000"; size = "0x2000000000"   # 128GB DDR
[[buses.nodes]]
unit = "cxl-mem0"; node_kind = "memory_device"
base_addr = "0x200000000000"; size = "0x8000000000"   # 512GB CXL
```

### 7.3  场景十六：NUMA + 多卡混合

双 Socket NUMA，每个 Socket 下挂 2 块 GPU，构成两层互联拓扑。GPU 跨 Socket 访问需经两层 InterconnectBus 级联路由。

```toml
[platform]
name = "numa-multi-gpu"
[[units]]
id = "socket0"; kind = "x86-cpu-socket"; backend = "kvm"
[units.config]
num_cores = 32; local_dram_gb = 256
[[units]]
id = "socket1"; kind = "x86-cpu-socket"; backend = "kvm"
[units.config]
num_cores = 32; local_dram_gb = 256
[[units]]
id = "gpgpu0"; kind = "gpgpu-card"; backend = "functional"
[[units]]
id = "gpgpu1"; kind = "gpgpu-card"; backend = "functional"
[[units]]
id = "gpgpu2"; kind = "gpgpu-card"; backend = "functional"
[[units]]
id = "gpgpu3"; kind = "gpgpu-card"; backend = "functional"
# PCIe：Socket0 下挂 GPU0/1，Socket1 下挂 GPU2/3
[[buses]]
kind = "pcie"; id = "pcie-s0"
root_complex = "socket0.pcie-rc"
[[buses.endpoints]]
unit = "gpgpu0"
[[buses.endpoints]]
unit = "gpgpu1"
[[buses]]
kind = "pcie"; id = "pcie-s1"
root_complex = "socket1.pcie-rc"
[[buses.endpoints]]
unit = "gpgpu2"
[[buses.endpoints]]
unit = "gpgpu3"
# L1：Socket 间 NUMA 互联
[[buses]]
kind = "interconnect"; id = "upi-fabric"
[buses.config]
addr_mode = "unified"; backend = "direct_fabric"
bandwidth_gbps = 400; latency_ns = 150
[[buses.nodes]]
unit = "socket0"; node_kind = "cpu_socket"
base_addr = "0x000000000000"; size = "0x4000000000"
[[buses.nodes]]
unit = "socket1"; node_kind = "cpu_socket"
base_addr = "0x400000000000"; size = "0x4000000000"
# L2：GPU 卡间互联（跨 Socket 访问时两层级联）
[[buses]]
kind = "interconnect"; id = "gpu-fabric"
[buses.config]
addr_mode = "windowed"; backend = "direct_fabric"
bandwidth_gbps = 400; latency_ns = 500
[[buses.nodes]]
unit = "gpgpu0"; node_kind = "accelerator"
base_addr = "0x000000000"; size = "0x400000000"
[[buses.nodes]]
unit = "gpgpu1"; node_kind = "accelerator"
base_addr = "0x400000000"; size = "0x400000000"
[[buses.nodes]]
unit = "gpgpu2"; node_kind = "accelerator"
base_addr = "0x800000000"; size = "0x400000000"
[[buses.nodes]]
unit = "gpgpu3"; node_kind = "accelerator"
base_addr = "0xc00000000"; size = "0x400000000"
```

## 12  验证场景与完整 Manifest

### 12.1  场景一：x86 KVM Host + PCIe GPGPU

#### 12.1.1  拓扑描述

```
host（Composite）
├── x86-core0 [KVM]              KVM 加速的 x86-64 vCPU
├── iommu                        Intel VT-d IOMMU
├── pcie-rc                      PCIe Root Complex
├── lapic0                       Local APIC（CPU0）
├── ioapic                       I/O APIC（外设中断控制器）
└── dram0                        Host 主内存 [8 GB]
│
│  [PCIe Bus]
│
gpgpu0（Composite, PCIe EP）
├── pcie-ep                      两面性：对 Host 是 MemTarget，对内部是 Initiator
├── riscv-ctrl [TCG]             GPU 控制器固件 CPU（RV64）
├── cmd-proc                     命令处理器（解析 GPU 命令流）
├── sm[0..127]                   128 个 SIMT 计算单元（无独立 CPU core）
├── l2-cache [Timing]            L2 Cache 性能模型
└── vram-ctrl [Timing]           VRAM 控制器性能模型 → GDDR6 [16 GB]
```

#### 12.1.2  关键设计点

| 设计点 | 实现方式 | 验证目标 |
| --- | --- | --- |
| KVM + TCG 共存 | x86-core0: backend=kvm; riscv-ctrl: backend=tcg 两个 Unit 在不同线程各自执行 | Backend 是 Unit 级属性，互不干扰 |
| PCIe EP 两面性 | pcie-ep 同时注册到： ① host.mem_bus（作为 MemTarget，BAR 窗口） ② gpgpu0.internal_mem_bus（作为 Initiator，访问内部） | MemBus 的 visible_to 实现地址空间隔离 |
| MSI 中断路径 | riscv-ctrl 写 MSI 地址 → pcie-ep 转发 → host.ioapic → lapic0 | IrqBus 跨 Composite Unit 边界 |
| SM 模板展开 | sm[0..127] 通过 count 展开为 128 个独立 Unit 每个 SM 有私有寄存器文件 | 验证 Manifest 模板展开机制 |
| VRAM 性能建模 | vram-ctrl: backend=timing, timing=loose 建模 GDDR6 带宽/延迟 | Functional + Timing 混合精度 |

#### 12.1.3  完整 Manifest（TOML）

```
[platform]
name        = "x86-kvm-host-pcie-gpgpu"
description = "x86 KVM Host with PCIe GPGPU (RISC-V controller + SM array)"
arch        = "x86_64"
sim_mode    = "performance"  # Functional CPU + Timing cache/vram
# ─── 模板：SM 单元（128 个展开）────────────────────────────
[templates.sm-unit]
kind    = "gpu-simt-sm"
backend = "mock"            # SM 暂无独立 backend，由 cmd-proc 驱动
timing  = "untimed"
[templates.sm-unit.config]
warp_count    = 32
thread_per_warp = 32
reg_file_size = 65536
# ─── Units ───────────────────────────────────────────────────
# Host Composite
[[units]]
id   = "host"
kind = "x86-soc"
[[units.sub_units]]
id      = "x86-core0"
kind    = "x86-core"
backend = "kvm"
timing  = "untimed"
[units.sub_units.config]
vendor = "Intel"
model  = "IceLake"
[[units.sub_units]]
id   = "iommu"
kind = "intel-vtd"
[units.sub_units.config]
iotlb_size = 1024
[[units.sub_units]]
id   = "pcie-rc"
kind = "x86-pcie-rc"
[[units.sub_units]]
id   = "lapic0"
kind = "x86-lapic"
[[units.sub_units]]
id   = "ioapic"
kind = "x86-ioapic"
[units.sub_units.config]
num_irqs = 24
[[units.sub_units]]
id   = "dram0"
kind = "dram"
[units.sub_units.config]
size_mb = 8192
# GPGPU Composite（PCIe Endpoint）
[[units]]
id   = "gpgpu0"
kind = "pcie-gpgpu"
[[units.sub_units]]
id   = "pcie-ep"
kind = "pcie-ep-bridge"
[units.sub_units.config]
# BAR0: 16MB 寄存器空间; BAR1: 16GB VRAM 窗口
bars = [
{ index=0, size_mb=16,    kind="mem64" },
{ index=2, size_gb=16,    kind="mem64" },
]
[[units.sub_units]]
id      = "riscv-ctrl"
kind    = "riscv-rv64-core"
backend = "tcg"
timing  = "untimed"
[units.sub_units.config]
isa_ext = ["m", "a", "c"]
[[units.sub_units]]
id   = "cmd-proc"
kind = "gpu-cmd-processor"
[[units.sub_units]]
id       = "sm"
template = "sm-unit"
[units.sub_units.count]
from = 0
to   = 127
[[units.sub_units]]
id      = "l2-cache"
kind    = "cache-l2"
backend = "timing"
timing  = "loose"
[units.sub_units.config]
size_kb        = 4096
assoc          = 16
line_size      = 128
latency_cycles = 40
[[units.sub_units]]
id      = "vram-ctrl"
kind    = "gddr6-ctrl"
backend = "timing"
timing  = "loose"
[units.sub_units.config]
size_gb         = 16
bandwidth_gbps  = 960
latency_ns      = 100
# ─── Buses ───────────────────────────────────────────────────
# Host 地址总线
[[buses]]
kind       = "mem"
id         = "host-membus"
initiators = ["host.x86-core0", "host.iommu"]
[[buses.targets]]
unit = "host.dram0"
base = "0x0000000000000000"
[[buses.targets]]
unit = "host.lapic0"
base = "0xFEE00000"
[[buses.targets]]
unit = "host.ioapic"
base = "0xFEC00000"
# GPGPU BAR0 窗口（Host 通过此地址访问 GPU 寄存器）
[[buses.targets]]
unit = "gpgpu0.pcie-ep"         # PCIe EP 作为 MemTarget
base = "0x00000000F0000000"     # BAR0 在 Host 地址空间的映射
# GPGPU 内部地址总线（riscv-ctrl 和 cmd-proc 的视角）
[[buses]]
kind       = "mem"
id         = "gpgpu-internal-bus"
initiators = ["gpgpu0.riscv-ctrl", "gpgpu0.cmd-proc"]
# riscv-ctrl 通过此总线访问 SM 控制寄存器
[[buses.targets]]
unit = "gpgpu0.cmd-proc"
base = "0x10000000"
[[buses.targets]]
unit = "gpgpu0.l2-cache"
base = "0x20000000"
[[buses.targets]]
unit = "gpgpu0.vram-ctrl"
base = "0x40000000"
# PCIe 总线
[[buses]]
kind         = "pcie"
id           = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit     = "gpgpu0"
bus      = 1
device   = 0
function = 0
[[buses.endpoints.bars]]
index = 0
size  = "0x1000000"      # 16 MB
host_base = "0xF0000000"
[[buses.endpoints.bars]]
index = 2
size  = "0x400000000"    # 16 GB
host_base = "0x100000000"
# 中断总线：GPU → Host
[[buses]]
kind = "irq"
id   = "gpgpu-irq-bus"
# riscv-ctrl 发 MSI → host ioapic
[[buses.connections]]
from = "gpgpu0.riscv-ctrl.irq_out"
to   = "gpgpu0.pcie-ep.msi_in"   # 先到 EP MSI 逻辑
[[buses.connections]]
from = "gpgpu0.pcie-ep.msi_out"
to   = "host.ioapic.irq[16]"
[[buses.connections]]
from = "host.ioapic.irq_out[16]"
to   = "host.lapic0.irq"
# SyncBus：协调 Timing 组件时序
[[buses]]
kind            = "sync"
id              = "perf-sync"
participants    = ["gpgpu0.l2-cache", "gpgpu0.vram-ctrl"]
time_quantum_ns = 1000000   # 1ms 量子步长
# ─── Boot ────────────────────────────────────────────────────
[boot]
[boot.kernel]
path      = "path/to/bzImage"
load_addr = "0x1000000"
format    = "elf"
[boot.registers]
"host.x86-core0" = { "rip" = "0x1000000" }
```

### 12.2  场景二：ARM KVM Server + RISC-V 协处理器

#### 12.2.1  拓扑描述

```
soc（Composite）
├── arm-ss（Composite）                ARM 主处理器子系统
│     ├── core[0..7] [KVM]            8 × Cortex-A78 核
│     ├── mmu[0..7]                   ARM MMU（Stage1+Stage2）
│     ├── l1i/l1d[0..7]              每核私有 L1 Cache
│     ├── l2                          共享 L2 Cache [4MB, Timing]
│     └── gicv3（Composite）          GICv3 中断控制器
│           ├── gicd                  Distributor（共享）
│           ├── gicr[0..7]            Redistributor（每核一个）
│           └── gicc[0..7]            CPU Interface（每核一个）
│
├── riscv-ss（Composite）             RISC-V 协处理器子系统
│     ├── rv64-core [TCG]             RV64 控制核
│     ├── itcm                        指令紧耦合内存 [256KB，RISC-V 私有]
│     ├── dtcm                        数据紧耦合内存 [256KB，RISC-V 私有]
│     ├── clint                       Core-Local Timer + 软中断
│     ├── plic                        Platform-Level 中断控制器
│     └── reset-ctrl                  复位控制器（ARM 写此寄存器释放 RISC-V）
│
├── shared-sram                       共享 SRAM [2MB]（ARM 和 RISC-V 均可访问）
├── mailbox                            片上邮箱（双向中断 + 消息 FIFO）
└── ddr-ctrl [Timing]                 DDR5 控制器 → 32GB 内存
```

#### 12.2.2  关键设计点

| 设计点 | 实现方式 | 验证目标 |
| --- | --- | --- |
| 部分共享地址空间 | shared-sram 注册到两个 MemBus（arm-membus + riscv-membus） 两个子系统看到相同物理地址 | MemBus 的多 Initiator 共享 Target |
| 私有 TCM 地址空间 | itcm/dtcm 只注册到 riscv-membus，使用 visible_to 过滤 ARM 访问同地址 → UnmappedAddress fault | MemBus visible_to 过滤机制 |
| 跨子系统中断 | Mailbox 作为中介： RISC-V → mailbox.tx_irq → ARM GICv3 ARM → mailbox.rx_irq → RISC-V PLIC | IrqBus 跨 Composite Unit 边界 |
| 启动顺序依赖 | ARM 先启动，固件就绪后写 reset-ctrl 寄存器 BootDesc.sequence: WaitWrite → ReleaseReset | BootSequencer + reset-ctrl 设备 |
| GICv3 层次 | GICv3 是 Composite Unit，包含 GICD + 8×GICR + 8×GICC 对外是单个 IrqSink（接收外设中断） | Composite Unit 的 IrqSink 接口 |

#### 12.2.3  完整 Manifest（TOML）

```
[platform]
name        = "arm-server-riscv-coprocessor"
description = "ARM KVM Server SoC with on-chip RISC-V coprocessor"
arch        = "aarch64"
sim_mode    = "performance"
# ─── Units ───────────────────────────────────────────────────
[[units]]
id   = "soc"
kind = "composite-soc"
# ── ARM 子系统 ────────────────────────────────────────────
[[units.sub_units]]
id   = "arm-ss"
kind = "composite-subsystem"
[[units.sub_units.sub_units]]
id      = "core"
kind    = "arm-cortex-a78-core"
backend = "kvm"
timing  = "untimed"
[units.sub_units.sub_units.config]
el2_enable = true
[units.sub_units.sub_units.count]
from = 0
to   = 7
[[units.sub_units.sub_units]]
id   = "mmu"
kind = "arm-mmu-v8"
[units.sub_units.sub_units.count]
from = 0
to   = 7
[[units.sub_units.sub_units]]
id      = "l2"
kind    = "cache-l2"
backend = "timing"
timing  = "loose"
[units.sub_units.sub_units.config]
size_kb        = 4096
assoc          = 16
latency_cycles = 15
[[units.sub_units.sub_units]]
id   = "gicv3"
kind = "arm-gicv3"
[units.sub_units.sub_units.config]
num_cpus   = 8
its_enable = true
# ── RISC-V 子系统 ──────────────────────────────────────────
[[units.sub_units]]
id   = "riscv-ss"
kind = "composite-subsystem"
[[units.sub_units.sub_units]]
id      = "rv64-core"
kind    = "riscv-rv64-core"
backend = "tcg"
timing  = "untimed"
[units.sub_units.sub_units.config]
isa_ext = ["m","a","c","zicsr"]
[[units.sub_units.sub_units]]
id   = "itcm"
kind = "sram"
[units.sub_units.sub_units.config]
size_kb = 256
[[units.sub_units.sub_units]]
id   = "dtcm"
kind = "sram"
[units.sub_units.sub_units.config]
size_kb = 256
[[units.sub_units.sub_units]]
id   = "clint"
kind = "riscv-clint"
[units.sub_units.sub_units.config]
num_harts = 1
[[units.sub_units.sub_units]]
id   = "plic"
kind = "riscv-plic"
[units.sub_units.sub_units.config]
num_sources = 64
num_targets = 2
[[units.sub_units.sub_units]]
id   = "reset-ctrl"
kind = "reset-controller"
[units.sub_units.sub_units.config]
targets = ["soc.riscv-ss.rv64-core"]
# ── 共享资源 ────────────────────────────────────────────────
[[units.sub_units]]
id   = "shared-sram"
kind = "sram"
[units.sub_units.config]
size_mb = 2
[[units.sub_units]]
id   = "mailbox"
kind = "on-chip-mailbox"
[units.sub_units.config]
fifo_depth   = 32
msg_size     = 8
[[units.sub_units]]
id      = "ddr-ctrl"
kind    = "ddr5-ctrl"
backend = "timing"
timing  = "loose"
[units.sub_units.config]
size_gb        = 32
bandwidth_gbps = 204
latency_ns     = 70
# ─── Buses ───────────────────────────────────────────────────
# ARM 子系统地址总线
[[buses]]
kind       = "mem"
id         = "arm-membus"
initiators = ["soc.arm-ss.core[0..7]"]   # 展开为 core0..core7
# DDR：0x80000000 起，32GB
[[buses.targets]]
unit = "soc.ddr-ctrl"
base = "0x80000000"
# GICv3 GICD（分发器）
[[buses.targets]]
unit = "soc.arm-ss.gicv3.gicd"
base = "0x08000000"
# GICv3 GICR（重分配器，每 CPU 128KB）
[[buses.targets]]
unit = "soc.arm-ss.gicv3.gicr[0..7]"
base = "0x08080000"
# 共享 SRAM（ARM 和 RISC-V 共同可见，相同物理地址）
[[buses.targets]]
unit = "soc.shared-sram"
base = "0x70000000"
# Mailbox 寄存器（ARM 访问）
[[buses.targets]]
unit = "soc.mailbox"
base = "0x09000000"
# UART（供 ARM 使用）
[[buses.targets]]
unit = "uart0"
base = "0x09010000"
# reset-ctrl（ARM 写此地址释放 RISC-V 复位）
[[buses.targets]]
unit = "soc.riscv-ss.reset-ctrl"
base = "0x09020000"
# RISC-V 子系统地址总线
[[buses]]
kind       = "mem"
id         = "riscv-membus"
initiators = ["soc.riscv-ss.rv64-core"]
# 私有 ITCM（只有 RISC-V 可见）
[[buses.targets]]
unit       = "soc.riscv-ss.itcm"
base       = "0x00000000"
visible_to = ["soc.riscv-ss.rv64-core"]
# 私有 DTCM
[[buses.targets]]
unit       = "soc.riscv-ss.dtcm"
base       = "0x10000000"
visible_to = ["soc.riscv-ss.rv64-core"]
# 共享 SRAM（相同物理地址 0x70000000，与 ARM 共享）
[[buses.targets]]
unit = "soc.shared-sram"
base = "0x70000000"
# CLINT
[[buses.targets]]
unit = "soc.riscv-ss.clint"
base = "0x02000000"
# PLIC
[[buses.targets]]
unit = "soc.riscv-ss.plic"
base = "0x0C000000"
# Mailbox 寄存器（RISC-V 访问，与 ARM 相同物理地址）
[[buses.targets]]
unit = "soc.mailbox"
base = "0x09000000"
# 中断总线
[[buses]]
kind = "irq"
id   = "soc-irq-bus"
# Mailbox：RISC-V 发消息 → 通知 ARM
[[buses.connections]]
from = "soc.mailbox.irq_to_arm"
to   = "soc.arm-ss.gicv3.gicd.irq[100]"
# Mailbox：ARM 发消息 → 通知 RISC-V
[[buses.connections]]
from = "soc.mailbox.irq_to_riscv"
to   = "soc.riscv-ss.plic.irq[1]"
# CLINT Timer → RISC-V 核
[[buses.connections]]
from = "soc.riscv-ss.clint.mtip"
to   = "soc.riscv-ss.rv64-core.mtip"
# CLINT Software → RISC-V 核
[[buses.connections]]
from = "soc.riscv-ss.clint.msip"
to   = "soc.riscv-ss.rv64-core.msip"
# SyncBus（DDR + L2 时序协调）
[[buses]]
kind            = "sync"
id              = "ddr-sync"
participants    = ["soc.ddr-ctrl", "soc.arm-ss.l2"]
time_quantum_ns = 1000000
# ─── Boot ────────────────────────────────────────────────────
[boot]
[boot.firmware]
path   = "path/to/u-boot.bin"
format = "raw"
[boot.firmware.load_addr]
# ARM Trusted Firmware 加载地址
load_addr = "0x9010000"
[boot.kernel]
path      = "path/to/Image"
load_addr = "0x80080000"
format    = "raw"
[boot.dtb]
path      = "path/to/platform.dtb"
load_addr = "0x84000000"
format    = "raw"
[boot.cmdline]
cmdline = "earlycon console=ttyAMA0 root=/dev/vda"
# 启动顺序：ARM 先启动，RISC-V 后释放
[[boot.sequence]]
action = "write_reg"
unit   = "soc.arm-ss.core0"
reg    = "pc"
val    = "0x9010000"        # ARM TF-A 入口
[[boot.sequence]]
action = "write_reg"
unit   = "soc.arm-ss.core0"
reg    = "x1"
val    = "0x84000000"       # DTB 地址
# 等待 ARM 固件写 reset-ctrl.release 寄存器（值=1）
[[boot.sequence]]
action = "wait_write"
unit   = "soc.riscv-ss.reset-ctrl"
addr   = "0x0"
val    = "0x1"
# 释放 RISC-V 复位，从 ITCM 0x0 开始执行
[[boot.sequence]]
action = "release_reset"
unit   = "soc.riscv-ss.rv64-core"
[boot.registers]
"soc.riscv-ss.rv64-core" = { "pc" = "0x00000000" }  # ITCM 起始地址
```

### 12.3  验证矩阵

| 验证点 | 场景一 | 场景二 | 测试手段 |
| --- | --- | --- | --- |
| Backend 混合 | KVM(x86) + TCG(RISC-V) | KVM(ARM) + TCG(RISC-V) | 集成测试：两 Backend 各自执行并汇报进度 |
| 地址空间隔离 | Host 不可直接访问 GPU 内存 | ARM 不可访问 ITCM/DTCM | MemBus::read 返回 UnmappedAddress |
| 部分共享内存 | — | shared-sram 两侧一致性 | ARM 写 → RISC-V 读验证 |
| 跨层次中断 | GPU MSI → IOAPIC → LAPIC | Mailbox → GICv3/PLIC | IrqLine.pulse() 触发，MockIrqSink 记录 |
| 热插拔 | QMP device_add 新 GPU | QMP device_del 协处理器 | Platform::hot_add/remove + Scheduler |
| 启动顺序 | — | ARM 先启动，WaitWrite → ReleaseReset | BootSequencer 单步执行 |
| 快照/迁移 | 全量快照保存 | Platform::snapshot + restore |  |
| GDB 调试 | GDB 连接 RISC-V ctrl | GDB 连接 RV64 core | GdbStub + DebugBackend |
| 性能仿真 | L2/VRAM Timing | DDR/L2 Timing | SyncBus tick + TimingModel |
