> [中文版](./04-use-case-handbook.zh-CN.md)

---

# OmniSim — Use Case Handbook

> **Document ID**: DOC-04 &nbsp;&nbsp; **Version**: v1.0 &nbsp;&nbsp; **Date**: 2026-Q1 &nbsp;&nbsp; **Author**: OpenFunTech

---

| Document ID | DOC-04 |
| --- | --- |
| Version | v1.0 |
| Audience | External users (building platforms with OmniSim) · New contributors |
| Prerequisites | DOC-01 Architecture Design (recommended) |
| Related Docs | DOC-02 Interface Spec · DOC-03 Component Dev Guide |
| Last Updated | 2026-Q1 |

## 1  How to Use This Handbook

This handbook is organized by use case; each scenario is self-contained and independent of the others. Jump directly to the scenario you care about.

### 1.1  Manifest File Structure Quick Reference

```toml
# platform.toml top-level structure
[platform]
name    = "my-platform"    # platform name
version = "1.0"
[[units]]                  # one [[units]] block per Unit
id      = "cpu0"           # unique ID, referenceable by Buses
kind    = "arm-cortex-a78-core"  # component type (Registry lookup)
backend = "kvm"            # execution dimension: functional/kvm/timing/mock
timing  = "untimed"        # timing dimension: untimed/loose/cycle_accurate
[units.config]             # component-specific configuration
num_cores = 4
[[units.sub_units]]      # Composite Unit: nested child Units
id   = "cpu0.l1-cache"
kind = "arm-l1-cache"
[[buses]]                  # one [[buses]] block per Bus
kind = "mem"               # Bus type: mem/irq/pcie/reset/power/interconnect
id   = "main-mem-bus"
[[buses.targets]]
unit = "dram0"; base = "0x80000000"
[[boot]]                   # boot sequence (optional)
[[boot.sequence]]
action = "deassert_reset"; bus = "soc-reset-bus"
```

### 1.2  Orthogonal Dimension Explanation

```
Three orthogonal dimensions, independent of platform topology
Host ISA: x86-64 / AArch64 / RISC-V — any Unit can be combined
Host Backend: kvm (same-architecture hardware acceleration) / functional (cross-architecture software simulation)
Simulation fidelity: functional (correctness verification) / timing (performance modeling) / cycle_accurate (RTL co-simulation)
All scenario examples in this handbook use functional fidelity; switching backend/timing fields changes the mode
```

### 1.3  Scenario Index

| Scenario | Category | Core Features | Section |
| --- | --- | --- | --- |
| On-chip heterogeneous dual-core (ARM + RISC-V) | Basic heterogeneous | ResetBus · visible_to | §2.1 |
| PCIe GPGPU (two SM architectures) | Basic heterogeneous | SimtHartBackend / SimtWarpBackend | §2.2 |
| PCIe DPU (I/O offload) | Basic heterogeneous | EP DMA · NetworkBackend | §2.3 |
| Multi-card interconnect | Basic heterogeneous | InterconnectBus · Windowed/Unified | §2.4 |
| Embedded SoC (multi-heterogeneous-core) | Basic heterogeneous | PowerBus · DSP · NPU | §2.5 |
| Network processor | Network & storage | DPU framework + hardware forwarding table | §3.1 |
| SmartSSD / computational storage | Network & storage | DPU framework + NAND controller | §3.2 |
| CXL device | Network & storage | InterconnectBus MemoryDevice | §3.3 |
| Multi-socket server NUMA | NUMA | InterconnectBus CpuSocket · Unified | §4.1 |
| Large-memory SoC NUMA | NUMA | InterconnectBus on-chip NoC | §4.2 |
| Nested virtualization | Virtualization | Stage-2 MMU · TCG EL2 | §5.1 |
| TrustZone / TEE | Security | MemSecurity · SecurityWorld | §6.1 |
| Confidential virtualization (P2) | Security | SEV-SNP PSP · ARM CCA RMM | §6.2 |
| AI server full stack | Comprehensive platform | Multi-card + DPU + on-chip NPU | §7.1 |
| Heterogeneous memory platform | Comprehensive platform | DDR + CXL + NVMe-oF tiering | §7.2 |
| NUMA + multi-card hybrid | Comprehensive platform | Two-tier InterconnectBus cascade | §7.3 |

## 2  Basic Heterogeneous Computing Scenarios

### 2.1  Scenario 1: On-Chip Heterogeneous Dual-Core (ARM + RISC-V Coprocessor)

The ARM main core runs Linux; the RISC-V coprocessor runs an RTOS; the two cores communicate via Mailbox. ARM controls the RISC-V reset and startup sequence.

#### Key Configuration Points

ResetBus: ARM's Reset Controller controls RISC-V's reset_in; RISC-V only begins executing after deassert

MemBus visible_to: ITCM (RISC-V private SRAM) is only visible to the RISC-V Core; ARM cannot access it

IrqBus: cross-subsystem interrupt; RISC-V notifies ARM via Mailbox interrupt when a task completes

BootSequencer: deassert ARM reset first; then deassert RISC-V reset after ARM initialization completes

#### Complete Manifest

```toml
[platform]
name = "arm-riscv-soc"
# ARM main core subsystem
[[units]]
id = "arm-ss"; kind = "composite"; backend = "functional"
[[units.sub_units]]
id = "arm-ss.cpu"; kind = "arm-cortex-a78-core"; backend = "kvm"
[[units.sub_units]]
id = "arm-ss.gicv3"; kind = "arm-gicv3"; backend = "functional"
# RISC-V coprocessor subsystem
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
# On-chip shared resources
[[units]]
id = "ddr"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
size_gb = 4
[[units]]
id = "mailbox"; kind = "hw-mailbox"; backend = "functional"
[[units]]
id = "reset-ctrl"; kind = "soc-reset-ctrl"; backend = "functional"
# ARM MemBus (all shared devices visible)
[[buses]]
kind = "mem"; id = "arm-mem-bus"
[[buses.targets]]
unit = "ddr";     base = "0x80000000"
[[buses.targets]]
unit = "mailbox"; base = "0x09000000"
[[buses.targets]]
unit = "arm-ss.gicv3.gicd"; base = "0x08000000"
# RISC-V MemBus (ITCM + shared devices visible, GIC not visible)
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
visible_to = ["riscv-ss.rv64"]   # only visible to RISC-V Core
# Interrupt bus
[[buses]]
kind = "irq"; id = "irq-bus"
[[buses.connections]]
from = "mailbox.irq_to_arm";   to = "arm-ss.gicv3.spi[32]"
[[buses.connections]]
from = "mailbox.irq_to_riscv"; to = "riscv-ss.plic.irq[1]"
# Reset bus
[[buses]]
kind = "reset"; id = "soc-reset-bus"
[[buses.connections]]
from = "reset-ctrl.riscv_reset"
to   = "riscv-ss.rv64.reset_in"
# Boot sequence
[[boot.sequence]]
action = "deassert_reset"; bus = "soc-reset-bus"
[[boot.sequence]]
action = "wait_ready"; unit = "arm-ss.cpu"; timeout_ns = 5_000_000_000
```

### 2.2  Scenario 2: PCIe GPGPU

The Host connects to a GPGPU accelerator card via PCIe. The GPGPU is internally a Composite Unit containing an ARM management core, SM array, and HBM.

#### SM Architecture Selection

| Route | Backend | Use Case | Config Field |
| --- | --- | --- | --- |
| Route A | SimtHartBackend | Need to run RISC-V binaries (e.g., PTX compiled to RVV) | sm_backend = "simt_hart" |
| Route B | SimtWarpBackend | Custom ISA, precise Warp scheduler semantics | sm_backend = "simt_warp" |

#### Complete Manifest (Route A)

```toml
[platform]
name = "host-gpgpu"
# Host
[[units]]
id = "host"; kind = "x86-host"; backend = "kvm"
# GPGPU (Composite PCIe EP)
[[units]]
id = "gpgpu0"; kind = "gpgpu-card"; backend = "functional"
[units.config]
sm_count        = 128
threads_per_warp = 32
sm_backend      = "simt_hart"   # Route A: RISC-V hart array
hbm_size_gb     = 16
# PCIe bus
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "gpgpu0"
# PCIe reset bus (PERST#)
[[buses]]
kind = "reset"; id = "pcie-reset"
[[buses.connections]]
from = "host.pcie-rc.perst#"
to   = "gpgpu0.reset_in"
# Boot sequence
[[boot.sequence]]
action = "deassert_reset"; bus = "pcie-reset"
[[boot.sequence]]
action = "wait_links_up"; bus = "pcie0"; timeout_ns = 10_000_000_000
[[boot.sequence]]
action = "wait_ready"; unit = "gpgpu0.arm-ctrl"; timeout_ns = 5_000_000_000
```

### 2.3  Scenario 3: PCIe DPU

The DPU offloads the Host's network and storage I/O, exposing standard virtio interfaces to the Host. The DPU internally accesses remote storage via nvme-of and forwards network traffic via an internal Layer 2 switch.

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
nvmeof_target     = "192.168.1.100:4420"   # remote storage (HostSocketBackend)
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

### 2.4  Scenario 4: Multi-Card Interconnect

Multiple GPGPUs communicate directly via InterconnectBus without going through the Host CPU.

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
# NVLink-style card-to-card interconnect
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

### 2.5  Scenario 5: Embedded SoC (Multi-Heterogeneous-Core)

A complete embedded SoC with ARM main core, RISC-V small core, DSP, NPU, and FPGA, with power domain management.

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
# Power bus
[[buses]]
kind = "power"; id = "soc-power-bus"
[[buses.connections]]
controller = "power-ctrl"; target = "npu";       domain = "PD_NPU"
[[buses.connections]]
controller = "power-ctrl"; target = "dsp0";      domain = "PD_DSP0"
[[buses.connections]]
controller = "power-ctrl"; target = "riscv-core"; domain = "PD_RISCV"
# Reset bus
[[buses]]
kind = "reset"; id = "soc-reset-bus"
[[buses.connections]]
from = "reset-ctrl.dsp_reset";   to = "dsp0.reset_in"
[[buses.connections]]
from = "reset-ctrl.riscv_reset"; to = "riscv-core.reset_in"
[[buses.connections]]
from = "reset-ctrl.npu_reset";   to = "npu.reset_in"
```

## 3  Network and Storage Scenarios

### 3.1  Scenario 6: Network Processor

Multi-core MIPS/ARM processor with hardware forwarding table, packet classification engine, and crypto engine.

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
hw_fwd_table = true   # omnisim-net-hwfwd component
crypto_engine = true  # MockBackend
[[buses]]
kind = "pcie"; id = "pcie0"
root_complex = "host.pcie-rc"
[[buses.endpoints]]
unit = "np0"
```

### 3.2  Scenario 7: SmartSSD / Computational Storage

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

### 3.3  Scenario 8: CXL Device

CXL.io is covered by PcieBus (device discovery); CXL.mem is covered by InterconnectBus Unified (Host directly addresses device memory).

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
# CXL.io: device discovery and configuration (PcieBus coverage)
[[buses]]
kind = "pcie"; id = "cxl0"
root_complex = "host.cxl-rc"
[[buses.endpoints]]
unit = "cxl-mem0"
# CXL.mem: Host directly addresses device memory (InterconnectBus Unified)
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

## 4  NUMA Scenarios

```
Two NUMA Forms Using the Same Framework
Multi-socket server NUMA: sockets interconnected via UPI / Infinity Fabric, latency ~150ns
Large-memory SoC NUMA: compute clusters interconnected via on-chip NoC, latency ~30ns
Both forms have identical Manifest structure; only latency_ns / bandwidth_gbps parameters differ
```

### 4.1  Scenario 9: Multi-Socket Server NUMA

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

### 4.2  Scenario 10: Large-Memory SoC NUMA

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
# On-chip NoC: ~5x lower latency, ~5x higher bandwidth than server NUMA
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

## 5  Virtualization Scenarios

### 5.1  Scenario 11: Nested Virtualization

OmniSim simulates a complete physical machine (TCG full-system simulation); a real Hypervisor (KVM / LeoVirt-S) runs on the physical machine; the Hypervisor then manages multiple VMs. The framework requires zero changes; nested capability is entirely provided by omnisim-tcg's EL2/VMX/H-extension frontend.

```
Version Requirements
ARM nested virtualization: requires omnisim-tcg P1 (EL2 privilege-level simulation + vGIC) — not available in P0 open-source version
x86 nested virtualization: requires omnisim-tcg P1 (VMX instruction set + EPT) — not available in P0 open-source version
RISC-V nested virtualization: requires omnisim-tcg P1 (H extension) — not available in P0 open-source version
```

```toml
[platform]
name = "nested-virt"
# Simulated physical machine (not a VM! this is hardware simulated by OmniSim)
[[units]]
id = "phys-cpu"; kind = "arm-cortex-a78-core"; backend = "functional"
[units.config]
# Key: enable EL2 privilege-level simulation (P1 feature)
enable_el2 = true
enable_vhe = true   # ARM Virtualization Host Extensions
[[units]]
id = "phys-gicv3"; kind = "arm-gicv3"; backend = "functional"
[units.config]
enable_virt = true  # GICv3 virtualization extension (vGIC)
[[units]]
id = "phys-dram"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
size_gb = 64
# Stage-2 MMU is automatically managed by the omnisim-tcg EL2 frontend
# No explicit configuration needed in the Manifest
# Hypervisor software (KVM / LeoVirt-S) is loaded as an ordinary ELF
[[boot.sequence]]
action = "load_elf"
unit   = "phys-cpu"
path   = "/path/to/hypervisor.elf"
```

## 6  Security Scenarios

### 6.1  Scenario 12: TrustZone / TEE

```toml
[platform]
name = "trustzone-soc"
[[units]]
id = "cpu"; kind = "arm-cortex-a78-core"; backend = "kvm"
[units.config]
enable_el3 = true   # TrustZone requires EL3
# Secure memory: accessible only from Secure World
[[units]]
id = "secure-sram"; kind = "sram"; backend = "functional"
[units.config]
size_kb  = 256
security = "secure"    # MemSecurity::Secure
# Normal memory
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
# P0: security checks are no-ops; Secure World software correctness is verifiable
# P2: MemBus enforces real isolation; Normal World access to Secure memory returns an error
```

### 6.2  Scenario 13: Confidential Virtualization (P2)

```
P2 Feature — Not Yet Implemented in Current Version
AMD SEV-SNP: requires omnisim-security-psp component (P2)
ARM CCA: requires omnisim-security-rmm component (P2) + omnisim-tcg Realm World support
P0/P1 versions: MemSecurity enum is reserved, interfaces are defined, but isolation checks are no-ops
```

```toml
# P2 reserved Manifest structure (no effect in current version)
# AMD SEV-SNP
[platform]
name = "sev-snp-platform"
[[units]]
id = "psp"; kind = "amd-psp"; backend = "mock"   # P2: functional
[[units]]
id = "vm0-mem"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
security = "encrypted_private"
asid     = 1
# ARM CCA
[platform]
name = "arm-cca-platform"
[[units]]
id = "rmm"; kind = "arm-rmm"; backend = "mock"   # P2: functional
[[units]]
id = "realm-mem"; kind = "ddr-ctrl"; backend = "functional"
[units.config]
security = "realm"
```

## 7  Comprehensive Heterogeneous Platform Scenarios

### 7.1  Scenario 14: AI Server Full Stack

ARM SoC main controller + on-chip NPU + PCIe GPGPU × 4 (NVLink-style interconnect) + DPU × 1.

```toml
[platform]
name = "ai-server"
[[units]]
id = "soc"; kind = "arm-soc"; backend = "kvm"
[units.config]
num_cores = 64
on_chip_npu = true   # on-chip NPU (MockBackend)
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
# GPU NVLink-style card-to-card interconnect
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

### 7.2  Scenario 15: Heterogeneous Memory Platform

Local DDR (low latency) + CXL memory expansion (medium latency) + remote NVMe-oF (high latency), forming a three-tier memory hierarchy.

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
# CXL.mem integration
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

### 7.3  Scenario 16: NUMA + Multi-Card Hybrid

Dual-socket NUMA with 2 GPUs under each socket, forming a two-tier interconnect topology. Cross-socket GPU access requires routing through two cascaded InterconnectBus tiers.

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
# PCIe: GPU0/1 under Socket0, GPU2/3 under Socket1
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
# L1: inter-socket NUMA interconnect
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
# L2: GPU card-to-card interconnect (two-tier cascade for cross-socket access)
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

## 12  Validation Scenarios and Complete Manifests

### 12.1  Scenario 1: x86 KVM Host + PCIe GPGPU

#### 12.1.1  Topology Description

```
host (Composite)
├── x86-core0 [KVM]              KVM-accelerated x86-64 vCPU
├── iommu                        Intel VT-d IOMMU
├── pcie-rc                      PCIe Root Complex
├── lapic0                       Local APIC (CPU0)
├── ioapic                       I/O APIC (peripheral interrupt controller)
└── dram0                        Host main memory [8 GB]
│
│  [PCIe Bus]
│
gpgpu0 (Composite, PCIe EP)
├── pcie-ep                      Dual-faced: MemTarget to Host, Initiator internally
├── riscv-ctrl [TCG]             GPU controller firmware CPU (RV64)
├── cmd-proc                     Command processor (parses GPU command stream)
├── sm[0..127]                   128 SIMT compute units (no independent CPU cores)
├── l2-cache [Timing]            L2 Cache performance model
└── vram-ctrl [Timing]           VRAM controller performance model → GDDR6 [16 GB]
```

#### 12.1.2  Key Design Points

| Design Point | Implementation | Verification Goal |
| --- | --- | --- |
| KVM + TCG coexistence | x86-core0: backend=kvm; riscv-ctrl: backend=tcg — two Units executing independently in separate threads | Backend is a Unit-level attribute; mutual independence |
| PCIe EP dual-faced | pcie-ep is simultaneously registered to: ① host.mem_bus (as MemTarget, BAR window) ② gpgpu0.internal_mem_bus (as Initiator, internal access) | MemBus visible_to implements address space isolation |
| MSI interrupt path | riscv-ctrl writes MSI address → pcie-ep forwards → host.ioapic → lapic0 | IrqBus crosses Composite Unit boundary |
| SM template expansion | sm[0..127] expanded via count into 128 independent Units; each SM has private register file | Validates Manifest template expansion mechanism |
| VRAM performance modeling | vram-ctrl: backend=timing, timing=loose — models GDDR6 bandwidth/latency | Functional + Timing mixed fidelity |

#### 12.1.3  Complete Manifest (TOML)

```
[platform]
name        = "x86-kvm-host-pcie-gpgpu"
description = "x86 KVM Host with PCIe GPGPU (RISC-V controller + SM array)"
arch        = "x86_64"
sim_mode    = "performance"  # Functional CPU + Timing cache/vram
# ─── Template: SM unit (expanded to 128) ─────────────────────
[templates.sm-unit]
kind    = "gpu-simt-sm"
backend = "mock"            # SM has no independent backend yet; driven by cmd-proc
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
# GPGPU Composite (PCIe Endpoint)
[[units]]
id   = "gpgpu0"
kind = "pcie-gpgpu"
[[units.sub_units]]
id   = "pcie-ep"
kind = "pcie-ep-bridge"
[units.sub_units.config]
# BAR0: 16MB register space; BAR1: 16GB VRAM window
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
# Host address bus
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
# GPGPU BAR0 window (Host accesses GPU registers via this address)
[[buses.targets]]
unit = "gpgpu0.pcie-ep"         # PCIe EP as MemTarget
base = "0x00000000F0000000"     # BAR0 mapping in Host address space
# GPGPU internal address bus (riscv-ctrl and cmd-proc perspective)
[[buses]]
kind       = "mem"
id         = "gpgpu-internal-bus"
initiators = ["gpgpu0.riscv-ctrl", "gpgpu0.cmd-proc"]
# riscv-ctrl accesses SM control registers via this bus
[[buses.targets]]
unit = "gpgpu0.cmd-proc"
base = "0x10000000"
[[buses.targets]]
unit = "gpgpu0.l2-cache"
base = "0x20000000"
[[buses.targets]]
unit = "gpgpu0.vram-ctrl"
base = "0x40000000"
# PCIe bus
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
# Interrupt bus: GPU → Host
[[buses]]
kind = "irq"
id   = "gpgpu-irq-bus"
# riscv-ctrl sends MSI → host ioapic
[[buses.connections]]
from = "gpgpu0.riscv-ctrl.irq_out"
to   = "gpgpu0.pcie-ep.msi_in"   # first to EP MSI logic
[[buses.connections]]
from = "gpgpu0.pcie-ep.msi_out"
to   = "host.ioapic.irq[16]"
[[buses.connections]]
from = "host.ioapic.irq_out[16]"
to   = "host.lapic0.irq"
# SyncBus: coordinate timing of Timing components
[[buses]]
kind            = "sync"
id              = "perf-sync"
participants    = ["gpgpu0.l2-cache", "gpgpu0.vram-ctrl"]
time_quantum_ns = 1000000   # 1ms quantum step
# ─── Boot ────────────────────────────────────────────────────
[boot]
[boot.kernel]
path      = "path/to/bzImage"
load_addr = "0x1000000"
format    = "elf"
[boot.registers]
"host.x86-core0" = { "rip" = "0x1000000" }
```

### 12.2  Scenario 2: ARM KVM Server + RISC-V Coprocessor

#### 12.2.1  Topology Description

```
soc (Composite)
├── arm-ss (Composite)                ARM main processor subsystem
│     ├── core[0..7] [KVM]            8 × Cortex-A78 cores
│     ├── mmu[0..7]                   ARM MMU (Stage1+Stage2)
│     ├── l1i/l1d[0..7]              per-core private L1 Cache
│     ├── l2                          shared L2 Cache [4MB, Timing]
│     └── gicv3 (Composite)           GICv3 interrupt controller
│           ├── gicd                  Distributor (shared)
│           ├── gicr[0..7]            Redistributor (one per core)
│           └── gicc[0..7]            CPU Interface (one per core)
│
├── riscv-ss (Composite)             RISC-V coprocessor subsystem
│     ├── rv64-core [TCG]             RV64 control core
│     ├── itcm                        Instruction TCM [256KB, RISC-V private]
│     ├── dtcm                        Data TCM [256KB, RISC-V private]
│     ├── clint                       Core-Local Timer + software interrupts
│     ├── plic                        Platform-Level Interrupt Controller
│     └── reset-ctrl                  Reset controller (ARM writes register to release RISC-V)
│
├── shared-sram                       Shared SRAM [2MB] (accessible to both ARM and RISC-V)
├── mailbox                            On-chip mailbox (bidirectional interrupt + message FIFO)
└── ddr-ctrl [Timing]                 DDR5 controller → 32GB memory
```

#### 12.2.2  Key Design Points

| Design Point | Implementation | Verification Goal |
| --- | --- | --- |
| Partially shared address space | shared-sram registered to both MemBuses (arm-membus + riscv-membus); both subsystems see the same physical address | MemBus multi-Initiator shared Target |
| Private TCM address space | itcm/dtcm registered only to riscv-membus with visible_to filter; ARM access to same address → UnmappedAddress fault | MemBus visible_to filter mechanism |
| Cross-subsystem interrupts | Mailbox as intermediary: RISC-V → mailbox.tx_irq → ARM GICv3; ARM → mailbox.rx_irq → RISC-V PLIC | IrqBus across Composite Unit boundary |
| Boot sequence dependency | ARM boots first; firmware writes reset-ctrl register when ready; BootDesc.sequence: WaitWrite → ReleaseReset | BootSequencer + reset-ctrl device |
| GICv3 hierarchy | GICv3 is a Composite Unit containing GICD + 8×GICR + 8×GICC; externally a single IrqSink (receives peripheral interrupts) | Composite Unit's IrqSink interface |

#### 12.2.3  Complete Manifest (TOML)

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
# ── ARM subsystem ─────────────────────────────────────────
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
# ── RISC-V subsystem ──────────────────────────────────────
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
# ── Shared resources ────────────────────────────────────────
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
# ARM subsystem address bus
[[buses]]
kind       = "mem"
id         = "arm-membus"
initiators = ["soc.arm-ss.core[0..7]"]   # expanded to core0..core7
# DDR: starting at 0x80000000, 32GB
[[buses.targets]]
unit = "soc.ddr-ctrl"
base = "0x80000000"
# GICv3 GICD (distributor)
[[buses.targets]]
unit = "soc.arm-ss.gicv3.gicd"
base = "0x08000000"
# GICv3 GICR (redistributor, 128KB per CPU)
[[buses.targets]]
unit = "soc.arm-ss.gicv3.gicr[0..7]"
base = "0x08080000"
# Shared SRAM (visible to both ARM and RISC-V at the same physical address)
[[buses.targets]]
unit = "soc.shared-sram"
base = "0x70000000"
# Mailbox registers (ARM access)
[[buses.targets]]
unit = "soc.mailbox"
base = "0x09000000"
# UART (for ARM use)
[[buses.targets]]
unit = "uart0"
base = "0x09010000"
# reset-ctrl (ARM writes this address to release RISC-V reset)
[[buses.targets]]
unit = "soc.riscv-ss.reset-ctrl"
base = "0x09020000"
# RISC-V subsystem address bus
[[buses]]
kind       = "mem"
id         = "riscv-membus"
initiators = ["soc.riscv-ss.rv64-core"]
# Private ITCM (visible to RISC-V only)
[[buses.targets]]
unit       = "soc.riscv-ss.itcm"
base       = "0x00000000"
visible_to = ["soc.riscv-ss.rv64-core"]
# Private DTCM
[[buses.targets]]
unit       = "soc.riscv-ss.dtcm"
base       = "0x10000000"
visible_to = ["soc.riscv-ss.rv64-core"]
# Shared SRAM (same physical address 0x70000000, shared with ARM)
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
# Mailbox registers (RISC-V access, same physical address as ARM)
[[buses.targets]]
unit = "soc.mailbox"
base = "0x09000000"
# Interrupt bus
[[buses]]
kind = "irq"
id   = "soc-irq-bus"
# Mailbox: RISC-V sends message → notify ARM
[[buses.connections]]
from = "soc.mailbox.irq_to_arm"
to   = "soc.arm-ss.gicv3.gicd.irq[100]"
# Mailbox: ARM sends message → notify RISC-V
[[buses.connections]]
from = "soc.mailbox.irq_to_riscv"
to   = "soc.riscv-ss.plic.irq[1]"
# CLINT Timer → RISC-V core
[[buses.connections]]
from = "soc.riscv-ss.clint.mtip"
to   = "soc.riscv-ss.rv64-core.mtip"
# CLINT Software → RISC-V core
[[buses.connections]]
from = "soc.riscv-ss.clint.msip"
to   = "soc.riscv-ss.rv64-core.msip"
# SyncBus (DDR + L2 timing coordination)
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
# ARM Trusted Firmware load address
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
# Boot sequence: ARM boots first, RISC-V released after
[[boot.sequence]]
action = "write_reg"
unit   = "soc.arm-ss.core0"
reg    = "pc"
val    = "0x9010000"        # ARM TF-A entry point
[[boot.sequence]]
action = "write_reg"
unit   = "soc.arm-ss.core0"
reg    = "x1"
val    = "0x84000000"       # DTB address
# Wait for ARM firmware to write reset-ctrl.release register (value=1)
[[boot.sequence]]
action = "wait_write"
unit   = "soc.riscv-ss.reset-ctrl"
addr   = "0x0"
val    = "0x1"
# Release RISC-V reset, begin execution from ITCM 0x0
[[boot.sequence]]
action = "release_reset"
unit   = "soc.riscv-ss.rv64-core"
[boot.registers]
"soc.riscv-ss.rv64-core" = { "pc" = "0x00000000" }  # ITCM start address
```

### 12.3  Validation Matrix

| Validation Point | Scenario 1 | Scenario 2 | Test Method |
| --- | --- | --- | --- |
| Backend mixing | KVM(x86) + TCG(RISC-V) | KVM(ARM) + TCG(RISC-V) | Integration test: both Backends execute independently and report progress |
| Address space isolation | Host cannot directly access GPU memory | ARM cannot access ITCM/DTCM | MemBus::read returns UnmappedAddress |
| Partial shared memory | — | shared-sram consistency across both sides | ARM writes → RISC-V reads to verify |
| Cross-hierarchy interrupts | GPU MSI → IOAPIC → LAPIC | Mailbox → GICv3/PLIC | IrqLine.pulse() triggers, MockIrqSink records |
| Hot-plug | QMP device_add new GPU | QMP device_del coprocessor | Platform::hot_add/remove + Scheduler |
| Boot ordering | — | ARM boots first, WaitWrite → ReleaseReset | BootSequencer single-step execution |
| Snapshot/migration | Full snapshot save | Platform::snapshot + restore | |
| GDB debugging | GDB connects to RISC-V ctrl | GDB connects to RV64 core | GdbStub + DebugBackend |
| Performance simulation | L2/VRAM Timing | DDR/L2 Timing | SyncBus tick + TimingModel |
