# AxOPC 合并进 AxInvest —— 实施方案索引

> 版本：v3.0　日期：2026-08-02　状态：待评审

---

## 决策摘要

| 决策 | 内容 |
|---|---|
| 代码来源 | 直接用 AxOPC 成果（Rust 同栈），OpenOPC 仅作三机制设计蓝本（Python 异构不移植） |
| 合并形态 | AxOPC 职能合并进 AxInvest（单仓库/单运行时/单 PG） |
| 行业组织 | 行业 = 数据资产包（Industry Pack），各自独立启用/禁用/导出/导入/剥离 |
| **机制归属** | **全部在 AxInvest 本地实现**（含公司运行时内核）。**AxAgent 零改动**——公司运行时是"一人公司"领域机制，非所有 fork 需要的横切能力（后续 fork AxAgent 做 AxSim 不需要） |
| 分层 | 公司运行时 = AxInvest 本地新 crate `company-runtime`（implementor），依赖基座既有能力（rt-workflow/trajectory/rt-messaging/harness） |

## 唯一实施文档

| 文档 | 仓库 | 内容 |
|---|---|---|
| **`PLAN-axinvest-opc-business.md`**（v3.0） | `D:\OneManager\AxInvest` | 全部实施：行业数据资产包（9 行业）+ 台账 + OPC 工具 + 专家 + OpcPage + 公司运行时内核（WorkItem 状态机/组织抽象/招聘/升级/经验闭环/角色路由/市场包）。迁移 v209/v210/v211。 |

## 关键约束（详见子文档）

1. AxInvest 迁移改号 v209/v210/v211（AxOPC 用 v200/v201 与 `v200_axinvest_stock_tables` 冲突，已到 v208）。
2. `preset_templates.rs` 不得带入 AxOPC 的删除（保留 AxInvest 股票版 `build_stock_analysis_nodes`）。
3. `personality.rs` 丢弃 AxOPC 重构，保留 AxInvest 版。
4. `v100_consolidated.rs` 白名单增量（opc-company/opc-experts）+ AxInvest ALTER 清单两处都保留。
5. 搬入方式：选择性移植，不用 git merge 全量。
6. `opc_*` 前缀表只由 AxInvest OPC 业务层读写。
7. rt-messaging `session_router` 角色路由为**本地薄补丁**：注释标记 + merge 上游时重放，不 push 上游。

## 与 AxSim 的关系

AxAgent 保持纯净（不含公司运行时）→ 后续 fork AxAgent 做 AxSim 时不会带入任何 OPC 依赖；AxSim 如需公司运行时（如模拟组织推演），可从 AxInvest 反向移植或抽独立 crate。

---

*执行从 `PLAN-axinvest-opc-business.md` P0 开始。*
