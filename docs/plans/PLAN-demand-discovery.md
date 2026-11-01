# OPC 需求发现智能体 · 方案设计

> **状态**: 规划中
> **创建日期**: 2026-08-12
> **关联模块**: `opc` (一人公司业务域), `rt-workflow` (工作流引擎), `axagent-harness` (数据契约), `analysis-engine` (能力聚合), `tools` (平台扫描工具)
> **目标**: 在 OPC 域内打通「能力集 → 需求发现 → 对接交易 → 下发实施 → 交付」闭环。前期半自动（人工确认），预留全自动演进位。

---

## 1. 背景与现状

OPC（一人公司）业务域已具备项目、客户、发票、看板、站点等能力，且工作流引擎（`rt-workflow` + `WorkflowTemplateData` + `NodeKind` 标准节点）已成熟；多个业务（股票分析、文学创作、代码重构）已通过「工作流模板 + 配置面板」方式落地，**行业包/行业适配器机制正在被工作流替代**。

当前缺少一个**需求侧入口**：无人持续从外部平台发现潜在商机、自动评估自身能力是否可接单、将需求结构化后下发给对应业务工作流实施并最终交付。

本方案不新建行业适配器，而是**复用现有工作流引擎**，新增一个 `demand-discovery` 工作流模板 + 一个 OPC tab 页面，将「需求发现」作为 OPC 域内一条独立业务线。

---

## 2. 核心设计思路

1. **复用工作流引擎**：用 `axagent-harness::workflow_types` 的标准节点（`NodeKind`）编排，不新建执行器。
2. **业务逻辑与编排分离**：编排在工作流模板内（Agent/Tool/Condition/Loop/Container 节点），平台对接、能力扫描作为独立 `Tool` 节点或 service。
3. **动态性**：能力清单由扫描动态生成，需求匹配由 Agent 节点推理，不硬编码接单规则。
4. **半自动 → 全自动演进**：通过模板变量 `auto_mode` 控制，仅切换确认分支，不改变节点拓扑。

---

## 3. 总体架构

```
┌──────────────────────────────────────────────────────────────┐
│                    demand-discovery 工作流模板                 │
│                                                              │
│  Trigger ──► 能力集扫描 ──► 平台需求发现 ──► 需求提炼与匹配      │
│ (手动/定时)   (Tool)        (Tool)          (Agent, 焦点)      │
│                                                              │
│   ✅ 阶段 A · 需求发现与对接                                    │
└──────────────────────────────┬───────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────┐
│   ✅ 阶段 B · 实施与交付                                       │
│                                                              │
│  下发工作流实施 ──► 交付客户                                   │
│  (workflow_execute)  (交付记录 + 通知)                          │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

### 3.1 节点类型映射

| 工作流节点 | NodeKind | 职责 |
|---|---|---|
| 触发 | `Input` (Trigger) | 手动 / Schedule 定时 / Event |
| 能力集扫描 | `Tool` | 自动扫描 tools / 技能 / MCP / 工作流模板 |
| 平台需求发现 | `Tool` + `Container` | 闲鱼 / 猪八戒 线索采集（并行多平台） |
| 需求提炼与匹配 | `Agent` | 需求结构化 + 能力-需求商机评估（焦点） |
| 半自动确认 | `Condition` | `auto_mode` 分支：人工确认 vs 自动入库 |
| 下单对接 | `Tool` | 复用 OPC 创建客户 / 项目 / 发票 |
| 下发实施 | `Tool` + `Loop` | 映射到目标业务工作流并 `workflow_execute` |
| 交付 | `Tool` | 产出写入交付目录 + 交付记录 + 通知 |

---

## 4. 能力集搜集（仅自动扫描）

新增 `capability` service（建议放置于 `analysis-engine` crate），自动聚合四类来源：

| 来源 | 获取方式（已有） |
|---|---|
| 工具 Tools | `UnifiedToolRegistry.list_all()` / `tools::mod.rs` 注册表 |
| 技能 Skills | skill 注册与启用状态 |
| MCP 服务器/工具 | `list_mcp_servers` / `list_tools_for_server` |
| 工作流模板 | `workflow_list`（模板即“可交付能力”） |

- 结果归一化为 `CapabilityInventory`，缓存入库（新表 `opc_capability`）。
- 供 Agent 节点作为 `context_source` 注入，生成结构化能力清单（name / type / 适用场景 / 示例交付物）。
- 仅自动扫描，不做手动硬编码清单。

```rust
// analysis-engine/src/opc/capability.rs（建议新建）
pub struct CapabilityInventory {
    pub tools: Vec<CapabilityEntry>,
    pub skills: Vec<CapabilityEntry>,
    pub mcp_tools: Vec<CapabilityEntry>,
    pub workflows: Vec<CapabilityEntry>,
    pub scanned_at: chrono::DateTime<chrono::Utc>,
}
```

---

## 5. 平台需求发现（闲鱼 / 猪八戒）

前期**半自动**：一条 `discover_leads` 链路，Agent 节点负责定向检索与提炼，人工确认后入库；后期**全自动**（定时触发 + 自动入库）。

封装一个 `MarketplaceScanner` 工具（`tools` crate 内新增，复用已有 `HttpRequestTool` / `WebFetchTool` / `Browser` 能力），抽象统一接口：

```rust
// tools/src/tools/marketplace.rs（建议新建）
pub trait MarketplaceScanner {
    fn platform(&self) -> &'static str;          // "xianyu" / "zhubajie"
    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String>;
}
```

- 结果统一归一化为 `DemandLead`（来源平台 / 标题 / 描述 / 预算 / 发布者 / 链接 / 原始快照）。
- **半自动**：命中 → 人工确认 → 入库；**全自动**：按置信度阈值自动入库。
- 反爬 / 合规：请求走现有代理池，限频、只读公开信息；平台连接器做成可插拔，初期可仅实现“手动补录 + 浏览器辅助”最轻路径。

---

## 6. 需求对接与交易

复用 OPC 现有能力：`opc_create_project`、`customers`、`invoices`。

1. 需求确认后由 Agent 生成「需求任务书」（结构化需求 + 匹配到的能力 + 报价建议）。
2. 人工在面板确认 → 创建 `customer` + `project`（复用现有表）。
3. 交易 / 发票走现有 `invoices` 流程。

---

## 7. 下发工作流实施

核心的**动态下发**：需求任务书 → 选择对应业务工作流模板 → `workflow_execute`（`src/commands/workflows/mod.rs`）触发。

- 匹配逻辑由 Agent 节点完成（需求分类 → 映射到 `stock-analysis` / `literary-creation` / `code-refactor` 等已有模板）。
- 用 `NodeKind::Loop` / `Container` 支持一个需求拆分多个子工作流并行。
- 实施进度通过现有 `workflow_get_status` 回读。

---

## 8. 交付

- 工作流产出（文件 / 文档）→ 写入交付目录 → 生成交付记录（新表 `opc_delivery`）。
- 通知客户：复用现有 notification 能力；交付物可对接 `opc_sites`（站点）发布。

---

## 9. 数据模型（新增 migration，从 v200 起递增）

| 表 | 说明 |
|---|---|
| `opc_capability` | 能力清单快照（扫描来源 / 类型 / 描述） |
| `opc_demand_lead` | 需求线索（平台 / 内容 / 预算 / 状态=新/待确认/已对接/已成交/已交付） |
| `opc_delivery` | 交付记录（关联 project + 产出路径） |

**migration 编号**：遵循项目版本号策略，AxInvest 独有迁移从 v200 起单调递增；新增时累加 `dao/src/migrations/mod.rs` 的 `CURRENT_VERSION` 并在 `MIGRATIONS` 数组追加条目。

---

## 10. 前端（OPC 新增 tab）

`OpcPage.tsx` 新增 `demand` tab，页面 `DemandDiscoveryTab.tsx`：

- **能力集面板**：展示扫描到的能力清单（只读 + 刷新）。
- **需求池**：线索列表（平台 / 状态 / 预算），支持人工确认 / 补录。
- **对接与交易**：需求 → 任务书 → 建客户 / 项目。
- **下发实施**：选择目标工作流 → `workflow_execute` → 展示进度。
- **配置面板**：沿用 `StockAnalysisConfigPanel` 模式（`get/update_workflow_template` 读写变量），管理平台开关、扫描关键词、自动/半自动模式、匹配阈值。

---

## 11. 后端命令（新增，均带 `#[agent_command]`）

| 命令 | domain | safety | call_mode | 说明 |
|---|---|---|---|---|
| `opc_scan_capabilities` | opc | Safe | StateInput | 扫描并更新能力清单 |
| `opc_discover_leads` | opc | Caution | StateInput | 触发平台需求扫描 |
| `opc_confirm_lead` | opc | Caution | StateInput | 人工确认需求线索 |
| `opc_create_lead` | opc | Caution | StateInput | 手动补录需求 |
| `opc_execute_demand_workflow` | opc | Caution | StateInput | 下发需求到目标工作流实施 |
| `opc_list_deliveries` | opc | Safe | StateOnly | 列出交付记录 |

**命令注册三步**：`commands/mod.rs` 声明模块 → `generate_handler![]`（`register_commands.rs`）注册 → 添加 `#[agent_command]` 宏标签。

---

## 12. 演进路径（半自动 → 全自动）

| 阶段 | 平台采集 | 确认方式 | 触发 |
|---|---|---|---|
| 一期 | 手动补录 + 浏览器辅助 | 人工确认 | 手动 |
| 二期 | 定向检索（API / 代理池） | Agent 置信度预筛 + 人工兜底 | 手动 / Schedule |
| 三期 | 全自动抓取 | 自动入库 | `TriggerType::Schedule` 定时 |

通过模板变量 `auto_mode`（半自动 / 全自动）控制，不改变节点拓扑，仅切换确认分支。

---

## 13. 实施步骤（建议顺序）

1. **数据模型**：新增 migration 建 `opc_capability` / `opc_demand_lead` / `opc_delivery` 三表。
2. **能力集扫描**：`capability` service + `opc_scan_capabilities` 命令。
3. **需求发现**：`MarketplaceScanner` 工具接口 + 手动补录最轻路径 + `opc_discover_leads` / `opc_create_lead` / `opc_confirm_lead` 命令。
4. **下发实施**：`opc_execute_demand_workflow` 命令（映射目标工作流 + `workflow_execute`）。
5. **前端**：`OpcPage` 新增 `demand` tab + `DemandDiscoveryTab` + 配置面板（沿用 `StockAnalysisConfigPanel` 模式）。
6. **交付闭环**：`opc_delivery` 记录 + 通知 + `opc_list_deliveries`。
7. **演进**：二期/三期接入真实平台连接器与定时触发。