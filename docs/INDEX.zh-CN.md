> [English](./INDEX.md)

---

# RhetSim — 文档索引

本索引收录 RhetSim 全部技术文档，每份文档均提供中英文版本。

## 文档列表

| # | 主题 | 英文版 | 中文版 |
|---|------|--------|--------|
| 01 | 架构设计 | [01-architecture.md](./01-architecture.md) | [01-architecture.zh-CN.md](./01-architecture.zh-CN.md) |
| 02 | 接口规范 | [02-interface-spec.md](./02-interface-spec.md) | [02-interface-spec.zh-CN.md](./02-interface-spec.zh-CN.md) |
| 03 | 组件开发指南 | [03-component-dev-guide.md](./03-component-dev-guide.md) | [03-component-dev-guide.zh-CN.md](./03-component-dev-guide.zh-CN.md) |
| 04 | 应用场景手册 | [04-use-case-handbook.md](./04-use-case-handbook.md) | [04-use-case-handbook.zh-CN.md](./04-use-case-handbook.zh-CN.md) |
| 05 | 路线图 | [05-roadmap.md](./05-roadmap.md) | [05-roadmap.zh-CN.md](./05-roadmap.zh-CN.md) |

## 文档说明

**01 — 架构设计**
RhetSim 的顶层设计：四层架构（L1–L4）、五个核心抽象（Unit、Bus、Backend、Manifest、Profile）以及整体设计理念。初次了解项目从这里开始。

**02 — 接口规范**
`rhetsim-core`、`rhetsim-manifest`、`rhetsim-runtime`、`rhetsim-tcg`、`rhetsim-compat` 全部公共接口的 Rust trait 与类型定义。框架开发者和组件开发者的主要参考手册。

**03 — 组件开发指南**
实现一个硬件组件的完整流程：工程目录结构、trait 实现模式、自注册机制、配置 Schema 约定、测试规范。

**04 — 应用场景手册**
十六个端到端仿真场景，涵盖异构 SoC、PCIe 加速器、嵌入式系统、AI 服务器、嵌套虚拟化等。每个场景包含完整 Manifest 示例及关键配置说明。

**05 — 路线图**
逐阶段实现路线图：已完成阶段（0–2）、当前工作（Phase 3）、计划阶段（4–9）。包含依赖关系图和与原始 M0–M6 里程碑的对照表。

## 建议阅读顺序

| 角色 | 建议顺序 |
|------|----------|
| 初次了解 / 评估项目 | 01 → 04 → 03 → 02 |
| 框架开发者 | 01 → 02 → 03 |
| 组件开发者 | 01 → 03 → 02 |
| 集成 / DevOps | 01 → 04 |
