> [中文版](./README.zh-CN.md)

---

# RHetSim

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](./Cargo.toml)
[![Build Status](https://img.shields.io/badge/build-pending-lightgrey.svg)](#)

RHetSim is an open-source heterogeneous hardware simulation framework written in Rust. It enables mixed-ISA platform simulation through a declarative Manifest system, pluggable execution backends, and a QEMU-compatible interface layer.

## Features

- **Heterogeneous multi-ISA** — Simulate ARM, RISC-V, and x86 cores side by side in a single platform
- **Declarative platform description** — Define complete hardware topologies in JSON/TOML Manifest files; no code changes required to swap components
- **Pluggable execution backends** — Switch between Functional, Timing, RTL co-simulation, and KVM acceleration per component
- **Eight bus types** — Memory, IRQ, PCIe, Sync, Data, Reset, Power, and Interconnect buses as first-class citizens
- **QEMU-compatible interface** — Drop-in replacement via CLI and QMP protocol; works with libvirt and virt-manager out of the box
- **Open and extensible** — Component crates self-register; adding a new hardware model requires no changes to the framework core

## Getting Started

> 🚧 Coming soon — the project scaffold is in place; runnable examples will be added once core crates reach functional completeness.

**Prerequisites**

```bash
# Rust toolchain 1.75 or later
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable
```

**Build**

```bash
git clone https://github.com/openfuntech/omnisim.git
cd omnisim
cargo build --workspace
```

**Run**

```bash
# TODO: add example profile run command
```

## Documentation

Full documentation is maintained in [`docs/`](./docs/INDEX.md). Each document is available in English and Chinese.

| # | Topic | English | Chinese |
|---|-------|---------|---------|
| 01 | Architecture Design | [01-architecture.md](./docs/01-architecture.md) | [zh-CN](./docs/01-architecture.zh-CN.md) |
| 02 | Interface Specification | [02-interface-spec.md](./docs/02-interface-spec.md) | [zh-CN](./docs/02-interface-spec.zh-CN.md) |
| 03 | Component Development Guide | [03-component-dev-guide.md](./docs/03-component-dev-guide.md) | [zh-CN](./docs/03-component-dev-guide.zh-CN.md) |
| 04 | Use Case Handbook | [04-use-case-handbook.md](./docs/04-use-case-handbook.md) | [zh-CN](./docs/04-use-case-handbook.zh-CN.md) |

## Project Structure

```
rhetsim/
├── crates/                  # Framework core (strictly layered)
│   ├── omnisim-core/        # L1+L2: traits and abstractions, zero business deps
│   ├── omnisim-manifest/    # L3: declarative platform description and realizer
│   ├── omnisim-runtime/     # Scheduler, UnitThread, SimClock, BootSequencer
│   ├── omnisim-tcg/         # TCG execution engine (ARM / RISC-V / x86 frontends)
│   └── omnisim-compat/      # L4: QEMU CLI parser and QMP server
├── components/              # Hardware component library
│   ├── cpu/                 # ARM Cortex-A, RISC-V, x86, Mock
│   ├── intc/                # GICv3, PLIC, CLINT
│   ├── memory/              # DRAM, SRAM, GDDR
│   ├── io/                  # UART, PCIe, DMA, Mailbox, VirtIO
│   └── accel/               # GPGPU, DPU
├── profiles/                # Pre-built platform profiles (TOML/JSON)
├── tools/                   # omni-cli, omni-dbg (GDB stub)
├── tests/                   # Integration and system tests (16 scenarios)
└── docs/                    # Technical documentation
```

## Contributing

> 🚧 Contributing guidelines are coming soon. See [CONTRIBUTING.md](./CONTRIBUTING.md) for the current placeholder.

We welcome issues, discussions, and pull requests. The project follows standard Rust conventions enforced by `cargo fmt`, `cargo clippy`, and `cargo deny`.

## License

OmniSim is licensed under the [Apache License 2.0](./LICENSE).
