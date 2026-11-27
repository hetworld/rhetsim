> [English](./README.md)

---

# RhetSim

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](./Cargo.toml)
[![Build Status](https://img.shields.io/badge/build-pending-lightgrey.svg)](#)

RhetSim 是一个用 Rust 编写的开源异构硬件仿真框架。通过声明式 Manifest 系统、可插拔执行后端以及 QEMU 兼容接口层，实现多 ISA 混合平台仿真。

## 核心特性

- **异构多 ISA** — 在同一平台中并行仿真 ARM、RISC-V、x86 处理器核心
- **声明式平台描述** — 用 JSON/TOML Manifest 文件定义完整硬件拓扑，替换组件无需修改代码
- **可插拔执行后端** — 每个组件可独立选择 Functional、Timing、RTL 协同或 KVM 加速后端
- **八种总线类型** — Memory、IRQ、PCIe、Sync、Data、Reset、Power、Interconnect 总线均为一等公民
- **QEMU 兼容接口** — 通过 CLI 和 QMP 协议实现透明替换，开箱即用于 libvirt 和 virt-manager
- **开放可扩展** — 组件 crate 自注册，新增硬件模型无需改动框架核心

## 快速上手

**环境要求**

```bash
# Rust toolchain 1.75 或更高版本
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable
```

**构建**

```bash
git clone https://github.com/openfuntech/rhetsim.git
cd rhetsim
cargo build --workspace
```

**启动 RISC-V Linux**

安装 RISC-V 交叉编译工具链：

```bash
# Ubuntu/Debian
sudo apt install gcc-riscv64-linux-gnu

# Arch Linux
sudo pacman -S riscv64-linux-gnu-gcc
```

编译 OpenSBI 固件：

```bash
git clone https://github.com/riscv-software-src/opensbi.git ../opensbi
cd ../opensbi
make CROSS_COMPILE=riscv64-linux-gnu- PLATFORM=generic FW_JUMP_ADDR=0x80200000
cd -
```

编译 Linux 内核（仓库内提供了最小化配置）：

```bash
git clone --depth 1 --branch v6.1 https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git ../linux-6.1
cd ../linux-6.1
cp ../rhetsim/platforms/reference/server/linux-rv64imac.config .config
make ARCH=riscv CROSS_COMPILE=riscv64-linux-gnu- -j$(nproc)
cd -
```

设备树文件（`riscv-virt.dtb`）已包含在仓库中。

启动模拟器：

```bash
# 快速启动 — 默认使用 ../opensbi 和 ../linux-6.1
./run.sh

# 或显式指定路径
./run.sh --firmware /path/to/fw_jump.bin --kernel /path/to/Image

# 非交互批处理模式
./run.sh --batch --steps 100000000
```

也可以通过环境变量覆盖默认路径：

```bash
RHETSIM_FIRMWARE=/path/to/fw_jump.bin RHETSIM_KERNEL=/path/to/Image ./run.sh
```

CLI 选项：

| 选项 | 说明 |
|------|------|
| `--batch` | 非交互模式（UART 输出到 stdout） |
| `--steps <n>` | 批处理模式下最大指令步数（默认 100000000） |
| `--firmware <path>` | 覆盖固件路径 |
| `--kernel <path>` | 覆盖内核镜像路径 |
| `--dtb <path>` | 覆盖设备树路径 |
| `--uart-log <path>` | UART 输出写入文件，而不是 stdout |
| `--log-level <level>` | 模拟器日志级别：`error`、`warn`、`info`（默认）、`debug`、`trace` |
| `--log-file <path>` | 模拟器日志写入文件（默认 stderr） |
| `--trace <items>` | 执行 trace 分类：`exec,cpu,jit,int,plic,clint,uart,loader,all` |
| `--trace-file <path>` | 执行 trace 写入文件（默认 stderr） |
| `--list-units` | 列出可用的硬件单元类型 |

ELF 和 raw binary 格式自动检测。ELF 文件按 PT_LOAD 段加载，入口点自动提取。

### 输出通道

输出分为三条独立通道，与 QEMU 的模型一致：

| 通道 | 内容 | 去向 |
|------|------|------|
| Guest 控制台 | OpenSBI / Linux 的 UART 输出 | stdout，或 `--uart-log <path>` |
| 模拟器日志 | Boot layout、进度、警告、错误 | stderr，或 `--log-file <path>` |
| 执行 trace | 指令、trap、PLIC/CLINT/UART 事件 | 默认关闭；`--trace` + `--trace-file` |

```bash
# 指令 trace 写文件，Linux 控制台仍然留在终端
./run.sh --batch --trace exec,int --trace-file trace.log

# 更详细的模拟器日志
./run.sh --log-level debug
```

`--trace exec` 和 `--trace cpu` 会关闭 JIT，让每条指令都经过解释器，保证逐指令 trace，但速度会明显变慢；只想观察 JIT basic block 时用 `--trace jit`。

## 文档

完整文档位于 [`docs/`](./docs/INDEX.zh-CN.md)，每份文档均提供中英文版本。

| # | 主题 | 中文版 | 英文版 |
|---|------|--------|--------|
| 01 | 架构设计 | [01-architecture.zh-CN.md](./docs/01-architecture.zh-CN.md) | [EN](./docs/01-architecture.md) |
| 02 | 接口规范 | [02-interface-spec.zh-CN.md](./docs/02-interface-spec.zh-CN.md) | [EN](./docs/02-interface-spec.md) |
| 03 | 组件开发指南 | [03-component-dev-guide.zh-CN.md](./docs/03-component-dev-guide.zh-CN.md) | [EN](./docs/03-component-dev-guide.md) |
| 04 | 应用场景手册 | [04-use-case-handbook.zh-CN.md](./docs/04-use-case-handbook.zh-CN.md) | [EN](./docs/04-use-case-handbook.md) |

## 工程结构

```
rhetsim/
├── crates/                  # 框架核心（严格分层）
│   ├── rhetsim-core/        # L1+L2：trait 与抽象，零业务依赖
│   ├── rhetsim-manifest/    # L3：声明式平台描述与 Realizer
│   ├── rhetsim-runtime/     # 调度器、UnitThread、SimClock、BootSequencer
│   ├── rhetsim-tcg/         # TCG 执行引擎（ARM / RISC-V / x86 前端）
│   └── rhetsim-compat/      # L4：QEMU CLI 解析器与 QMP Server
├── components/              # 硬件组件库
│   ├── cpu/                 # ARM Cortex-A、RISC-V、x86、Mock
│   ├── intc/                # GICv3、PLIC、CLINT
│   ├── memory/              # DRAM、SRAM、GDDR
│   ├── io/                  # UART、PCIe、DMA、Mailbox、VirtIO
│   └── accel/               # GPGPU、DPU
├── platforms/                # 预置平台 Profile（TOML/JSON）
├── tools/                   # rhetsim-cli、rhetsim-dbg（GDB stub）
├── tests/                   # 集成与系统测试（16 个场景）
└── docs/                    # 技术文档
```

## 参与贡献

> 🚧 贡献指南建设中，详见 [CONTRIBUTING.md](./CONTRIBUTING.md)。

欢迎提交 issue、参与讨论和发起 pull request。项目遵循标准 Rust 规范，通过 `cargo fmt`、`cargo clippy` 和 `cargo deny` 强制执行。

## 开源协议

RhetSim 采用 [Apache License 2.0](./LICENSE) 开源协议。
