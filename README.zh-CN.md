> [English](./README.md)

---

# OmniSim

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](./Cargo.toml)
[![Build Status](https://img.shields.io/badge/build-pending-lightgrey.svg)](#)

OmniSim 是一个用 Rust 编写的开源异构硬件仿真框架。通过声明式 Manifest 系统、可插拔执行后端以及 QEMU 兼容接口层，实现多 ISA 混合平台仿真。

## 核心特性

- **异构多 ISA** — 在同一平台中并行仿真 ARM、RISC-V、x86 处理器核心
- **声明式平台描述** — 用 JSON/TOML Manifest 文件定义完整硬件拓扑，替换组件无需修改代码
- **可插拔执行后端** — 每个组件可独立选择 Functional、Timing、RTL 协同或 KVM 加速后端
- **八种总线类型** — Memory、IRQ、PCIe、Sync、Data、Reset、Power、Interconnect 总线均为一等公民
- **QEMU 兼容接口** — 通过 CLI 和 QMP 协议实现透明替换，开箱即用于 libvirt 和 virt-manager
- **开放可扩展** — 组件 crate 自注册，新增硬件模型无需改动框架核心

## 快速上手

> 🚧 建设中 — 工程骨架已就绪，核心 crate 达到功能完整后将提供可运行示例。

**环境要求**

```bash
# Rust toolchain 1.75 或更高版本
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable
```

**构建**

```bash
git clone https://github.com/openfuntech/omnisim.git
cd omnisim
cargo build --workspace
```

**运行**

```bash
# TODO: 补充示例 profile 运行命令
```

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
omnisim/
├── crates/                  # 框架核心（严格分层）
│   ├── omnisim-core/        # L1+L2：trait 与抽象，零业务依赖
│   ├── omnisim-manifest/    # L3：声明式平台描述与 Realizer
│   ├── omnisim-runtime/     # 调度器、UnitThread、SimClock、BootSequencer
│   ├── omnisim-tcg/         # TCG 执行引擎（ARM / RISC-V / x86 前端）
│   └── omnisim-compat/      # L4：QEMU CLI 解析器与 QMP Server
├── components/              # 硬件组件库
│   ├── cpu/                 # ARM Cortex-A、RISC-V、x86、Mock
│   ├── intc/                # GICv3、PLIC、CLINT
│   ├── memory/              # DRAM、SRAM、GDDR
│   ├── io/                  # UART、PCIe、DMA、Mailbox、VirtIO
│   └── accel/               # GPGPU、DPU
├── profiles/                # 预置平台 Profile（TOML/JSON）
├── tools/                   # omni-cli、omni-dbg（GDB stub）
├── tests/                   # 集成与系统测试（16 个场景）
└── docs/                    # 技术文档
```

## 参与贡献

> 🚧 贡献指南建设中，详见 [CONTRIBUTING.md](./CONTRIBUTING.md)。

欢迎提交 issue、参与讨论和发起 pull request。项目遵循标准 Rust 规范，通过 `cargo fmt`、`cargo clippy` 和 `cargo deny` 强制执行。

## 开源协议

OmniSim 采用 [Apache License 2.0](./LICENSE) 开源协议。
