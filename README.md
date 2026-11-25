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

**Prerequisites**

```bash
# Rust toolchain 1.75 or later
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable
```

**Build**

```bash
git clone https://github.com/openfuntech/rhetsim.git
cd rhetsim
cargo build --workspace
```

**Boot Linux on RISC-V**

Install the RISC-V cross-compilation toolchain:

```bash
# Ubuntu/Debian
sudo apt install gcc-riscv64-linux-gnu

# Arch Linux
sudo pacman -S riscv64-linux-gnu-gcc
```

Build OpenSBI firmware:

```bash
git clone https://github.com/riscv-software-src/opensbi.git ../opensbi
cd ../opensbi
make CROSS_COMPILE=riscv64-linux-gnu- PLATFORM=generic FW_JUMP_ADDR=0x80200000
cd -
```

Build the Linux kernel (a minimal config is provided in the repo):

```bash
git clone --depth 1 --branch v6.1 https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git ../linux-6.1
cd ../linux-6.1
cp ../rhetsim/platforms/reference/server/linux-rv64ima.config .config
make ARCH=riscv CROSS_COMPILE=riscv64-linux-gnu- -j$(nproc)
cd -
```

The device tree blob (`riscv-virt.dtb`) is already included in the repo.

Start the simulator:

```bash
# Quick start — uses ../opensbi and ../linux-6.1 by default
./run.sh

# Or specify paths explicitly
./run.sh --firmware /path/to/fw_jump.bin --kernel /path/to/Image

# Non-interactive batch mode
./run.sh --batch --steps 100000000
```

You can also override default paths via environment variables:

```bash
RHETSIM_FIRMWARE=/path/to/fw_jump.bin RHETSIM_KERNEL=/path/to/Image ./run.sh
```

CLI options:

| Option | Description |
|--------|-------------|
| `--batch` | Non-interactive mode (no console I/O) |
| `--steps <n>` | Max instruction steps in batch mode (default: 100000000) |
| `--firmware <path>` | Override firmware binary path |
| `--kernel <path>` | Override kernel image path |
| `--dtb <path>` | Override device tree blob path |
| `--list-units` | List available hardware unit kinds |

Both ELF and raw binary formats are auto-detected. ELF files are loaded by their PT_LOAD segments with the entry point extracted automatically.

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
│   ├── rhetsim-core/        # L1+L2: traits and abstractions, zero business deps
│   ├── rhetsim-manifest/    # L3: declarative platform description and realizer
│   ├── rhetsim-runtime/     # Scheduler, UnitThread, SimClock, BootSequencer
│   ├── rhetsim-tcg/         # TCG execution engine (ARM / RISC-V / x86 frontends)
│   └── rhetsim-compat/      # L4: QEMU CLI parser and QMP server
├── components/              # Hardware component library
│   ├── cpu/                 # ARM Cortex-A, RISC-V, x86, Mock
│   ├── intc/                # GICv3, PLIC, CLINT
│   ├── memory/              # DRAM, SRAM, GDDR
│   ├── io/                  # UART, PCIe, DMA, Mailbox, VirtIO
│   └── accel/               # GPGPU, DPU
├── platforms/                # Pre-built platform profiles (TOML/JSON)
├── tools/                   # rhetsim-cli, rhetsim-dbg (GDB stub)
├── tests/                   # Integration and system tests (16 scenarios)
└── docs/                    # Technical documentation
```

## Contributing

> 🚧 Contributing guidelines are coming soon. See [CONTRIBUTING.md](./CONTRIBUTING.md) for the current placeholder.

We welcome issues, discussions, and pull requests. The project follows standard Rust conventions enforced by `cargo fmt`, `cargo clippy`, and `cargo deny`.

## License

RhetSim is licensed under the [Apache License 2.0](./LICENSE).
