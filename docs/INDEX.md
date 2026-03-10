> [中文版](./INDEX.zh-CN.md)

---

# OmniSim — Documentation Index

This index covers all OmniSim technical documents. Each document is available in both English and Chinese.

## Documents

| # | Topic | English | Chinese |
|---|-------|---------|---------|
| 01 | Architecture Design | [01-architecture.md](./01-architecture.md) | [01-architecture.zh-CN.md](./01-architecture.zh-CN.md) |
| 02 | Interface Specification | [02-interface-spec.md](./02-interface-spec.md) | [02-interface-spec.zh-CN.md](./02-interface-spec.zh-CN.md) |
| 03 | Component Development Guide | [03-component-dev-guide.md](./03-component-dev-guide.md) | [03-component-dev-guide.zh-CN.md](./03-component-dev-guide.zh-CN.md) |
| 04 | Use Case Handbook | [04-use-case-handbook.md](./04-use-case-handbook.md) | [04-use-case-handbook.zh-CN.md](./04-use-case-handbook.zh-CN.md) |

## Document Descriptions

**01 — Architecture Design**
Top-level design of OmniSim: the four-layer architecture (L1–L4), five core abstractions (Unit, Bus, Backend, Manifest, Profile), and the overall design philosophy. Start here if you are new to the project.

**02 — Interface Specification**
Complete Rust trait and type definitions for all public interfaces across `omnisim-core`, `omnisim-manifest`, `omnisim-runtime`, `omnisim-tcg`, and `omnisim-compat`. The primary reference for framework and component developers.

**03 — Component Development Guide**
Step-by-step guide to implementing a hardware component: project layout, trait implementation patterns, self-registration, config schema, and testing conventions.

**04 — Use Case Handbook**
Sixteen end-to-end simulation scenarios covering heterogeneous SoC, PCIe accelerators, embedded systems, AI servers, nested virtualization, and more. Each scenario includes a Manifest example and notes on key configuration points.

## Recommended Reading Order

| Role | Recommended Order |
|------|-------------------|
| New contributor / evaluator | 01 → 04 → 03 → 02 |
| Framework developer | 01 → 02 → 03 |
| Component developer | 01 → 03 → 02 |
| Integration / DevOps | 01 → 04 |
