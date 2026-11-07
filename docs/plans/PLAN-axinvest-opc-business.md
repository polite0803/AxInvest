# AxInvest OPC 实施：一人公司（行业数据资产包 + 台账 + 公司运行时内核）

> 状态：active ｜ 日期：2026-08-02 ｜ 对应版本：v2.8.7


> 版本：v3.0　日期：2026-08-02　状态：待评审
> 仓库：`D:\OneManager\AxInvest`（upstream=polite0803/AxAgent，origin=polite0803/AxInvest）
> 定位：把 AxOPC 成果（`D:\OneManager\AxOPC`）合并进 AxInvest，实现 9 大垂直行业的「行业 = 数据资产包」架构，**并在 AxInvest 本地实现公司运行时内核**。**AxAgent 零改动**——公司运行时是"一人公司"领域机制，非所有 fork 需要的横切能力（AxSim 做模拟推演不需要），故留在 AxInvest 本地 crate，与投资 crate 同策略（上游无 → 合并永不冲突）。

---

## 1. 背景与目标

### 1.1 背景

- `D:\OneManager\AxOPC` 是 AxAgent 的 fork（2.8.6），已有 372 个独有文件：opc 三 crate（~2400 行）、19 个 OPC 工具、9 大行业 + 18 领域 + 2 production 工作流 seed、6 角色专家、约 200 专家模板、OpcPage 前端。
- 用户决策链：
  1. AxOPC 职能**合并进 AxInvest**（单仓库/单运行时/单 PG）；
  2. 9 大行业以**「数据资产包」**组织（各自独立启用/禁用/导出/导入/剥离）——AxOPC 现状（行业硬编码在 seed 函数）不满足；
  3. **公司运行时不进 AxAgent**（后续 fork AxAgent 做 AxSim 不需要该功能）→ 全部在 AxInvest 本地实现。

### 1.2 目标

1. 搬入 AxOPC 的 OPC 业务成果（台账/工具/专家/前端）。
2. 重构行业组织方式为 **Industry Pack**（数据资产包），9 行业各自独立。
3. 在 AxInvest 本地实现**公司运行时内核**（WorkItem 状态机/组织抽象/招聘/升级/经验闭环），依赖基座（rt-workflow/trajectory/rt-messaging）既有能力，不新增上游迁移。
4. 迁移改号 v209/v210/v211（避开 AxInvest 已到 v208 的序列）。
5. 不破坏 AxInvest 投资域功能（preset_templates.rs 保留股票版）。

### 1.3 边界（重要）

| 归 AxInvest 本地 | 归 AxAgent（上游，不动） |
|---|---|
| 公司运行时内核（新 crate `company-runtime`） | rt-workflow / rt-messaging / trajectory / harness / agent 等基座 crate |
| 行业数据资产包、台账、OPC 工具、专家、OpcPage | 基座已有能力（Debate/Swarm 节点、RL 经验管道、8 渠道消息、SKILL.md 技能） |
| 本地薄补丁（session_router 加 role 字段等，标记注释，merge 上游时重放） | — |

---

## 2. 行业数据资产包（Industry Pack）——核心架构

**原则：行业是数据，不是代码。**

```
config/opc/industries/
├── finance_invest/
│   ├── manifest.yaml            # id / name / icon / version / enabled
│   ├── roles.yaml               # 行业角色映射（opc-cfo 等 → 专家/工具白名单）
│   ├── workflows/               # 工作流模板（原 seed_finance_investment 节点/边，纯数据）
│   │   └── finance-invest.yaml
│   ├── experts/                 # 行业专属专家提示词（可选）
│   └── tools.yaml               # 工具白名单
├── software_dev/  ├── ai_research/  ├── sales_growth/  ├── content_media/
├── industry_consulting/  ├── accounting/  ├── ecommerce/  ├── education/
```

**配套机制**：
1. `opc_industries` 注册表：`id / name / icon / version / enabled / pack_path / installed_at`；启动扫描 `config/opc/industries/*/manifest.yaml` 对比 DB 版本决定 seed/升级。
2. **行业级版本号**：manifest `version` 取代全局 `OPC_TEMPLATE_VERSION`。
3. **按包 seed**：`ensure_opc_workflows_seeded` 遍历 `opc_industries WHERE enabled=1` → 读 yaml → upsert `workflow-{industry}-*` 模板；禁用 = enabled=0 + 不再 seed。
4. **导出/导入**：`opc industry export/install` → `.opcip` 包。
5. **工具解耦**：opc.rs 19 工具按「行业工具」（OpcGetFinancialReport）与「通用台账工具」（OpcListInvoices）拆分。

---

## 3. 公司运行时内核（新 crate `company-runtime`，AxInvest 本地）

### 3.1 定位

- 依赖基座**既有**能力（rt-workflow DAG/节点、trajectory RL 经验管道、rt-messaging 渠道、harness DTO），在其上实现 OpenOPC 三机制（Self-Built / Self-Run / Self-Grown）。
- 分层：implementor（可依赖 harness + entities + dao + trajectory + rt-workflow + rt-messaging；**不依赖 consumer**）。
- **AxAgent 零改动**：所有实现都在本地 crate + 本地迁移（v211+）+ 本地薄补丁。

### 3.2 WorkItem 状态机（Self-Run）

```
QUEUED → IN_PROGRESS ⇄ BLOCKED → REVIEW → APPROVED → DONE
    ↘ WAITING_FOR_CHILDREN（依赖项未完成）   ↘ FAILED / CANCELLED（终态，触发依赖传播）
```

- 终态 `DONE_PHASES = {APPROVED, FAILED, CANCELLED}`；依赖传播：FAILED/CANCELLED 依赖 → 下游 doomed（不可认领）；soft/info 依赖不 gate 认领。
- 五种管理模式：execute / delegate / review / integrate / rework（Manager 驱动）。
- 复用 rt-workflow 的 DAG 引擎做依赖解析与并行调度；`opc_work_items` 表对接执行。

**表 `opc_work_items`**：`id / run_id / phase / owner_role_id / deps_json / assignee_agent_id / manager_role_id / last_error / created_at / updated_at`

### 3.3 组织抽象（Self-Built）

**表**：
- `opc_orgs`：`id / name / company_profile / topology / final_decider_role_id`
- `opc_org_roles`：`id / org_id / role_id / name / responsibility / reports_to / seniority`
- `opc_org_employees`：`id / org_id / employee_id / role_id / expert_id / status / experience_ref`
- `opc_talent_templates`：`id / category / name / description / source_repo / prompt_refs / skill_refs / tags`

复用基座已有 `agent_roles`/`agent_profiles`/`agency_experts` 表做映射，新增 `opc_org*` 系列承载组织结构。

**招聘决策规则**（Recruiter）：复用已有员工（携带历史经验）→ 人才库招聘 → 降级 fallback 空员工（占位不阻塞）。

### 3.4 经验归因与 Playbook（Self-Grown）

- **归因铁律**：用户反馈解析为逐员工评估，只更新拥有相关工作项的角色档案（防互相污染）。
- **晋升机制**：单角色经验档案按阈值晋升到共享 Playbook，新员工入职继承。
- **对接 trajectory**：`evaluate_round` 输出进 RL ExperiencePool（补 Loop Engineering 缺口；`SelfImprovingRound` trait 定义在本地 crate，不进 harness）。

**表**：
- `opc_experience_records`：`id / role_id / work_item_id / signal / content / created_at`
- `opc_playbooks`：`id / role_id / title / content / promoted_from / version`

### 3.5 角色感知路由（本地薄补丁）

- rt-messaging `session_router` 会话模型加 `role_id` 字段（Option，向后兼容）——**本地薄补丁，注释标记，merge 上游时重放**，不 push 上游。

### 3.6 市场包格式（.opcip）

- 行业包导出/导入解析（manifest + workflows + experts + tools），`opc industry export/install` 命令。

---

## 4. 搬入冲突处理清单（diff 体检实锤）

### 4.1 机械类（12 处，手并）

`Cargo.toml`、`migrations/mod.rs`、`commands/mod.rs`、`register_commands.rs`、`init/mod.rs`、`init/state.rs`、`lib.rs`、`tools/Cargo.toml`、`tools/mod.rs`、`ContentArea.tsx`、`Sidebar.tsx`、`AgentPropertyPanel.tsx`、`agent_profile.rs`

### 4.2 实质冲突（4 处）

| 文件 | AxOPC 改动 | 处理 |
|---|---|---|
| `v100_consolidated.rs` | category 白名单加 `opc-company`/`opc-experts` | ✅ 两处都保留（白名单 + AxInvest ALTER 清单） |
| `preset_templates.rs` | **删除了股票分析节点** | ⚠️ 不得带入其删除，保留 AxInvest 股票版 |
| `personality.rs` | 函数式 → struct+async trait 重构（558 行） | ⚠️ 丢弃其重构，保留 AxInvest 版 |
| `browserMock.ts` | 加 OPC 预设模板（502 行） | ✅ 保留 OPC 预设增量 |

---

## 5. 实施步骤（P0-P4，全部在 AxInvest）

### P0 骨架（0.5 天）

1. 复制 `opc-types` / `opc-entities` / `opc-dao` 三 crate 进 `src-tauri/crates/`。
2. **迁移改号**：`v200_opc_tables.rs` → `v209_opc_tables.rs`，`v201_opc_ext.rs` → `v210_opc_ext.rs`；migrations/mod.rs 注册；`CURRENT_VERSION` → 210。
3. Cargo.toml workspace 注册三 crate。
4. `v100_consolidated.rs` 合并（保留 ALTER 清单 + 加 category 白名单 + agent_profile.rs 同步）。
5. `cargo check` + `cargo clippy` 零警告。

### P0.5 行业包引擎（1 天，优先）

1. `opc_industries` 表 migration（v211）。
2. manifest.yaml schema + 包扫描/加载引擎（`company-runtime/src/industry_pack/`）。
3. seed 引擎重构：`ensure_opc_workflows_seeded` → 遍历启用行业包。
4. **一个行业包**（finance_invest）端到端跑通。
5. 反编译脚本：`seed_industries.rs` 9 函数 → yaml 包（半自动 + 人工校对 profile id）。

### P1 台账（1-2 天）

1. `tools/opc.rs` 搬入注册（行业/通用拆分）。
2. 命令 `opc.rs` / `opc_setup/` / `opc_knowledge.rs` 注册。
3. 前端：`OpcPage.tsx` 挂 tab + browserMock 预设 + Sidebar/ContentArea 入口。
4. 6 角色专家 + `config/agent_roles.yaml`。
5. 验收：发票/客户/项目 CRUD + 角色 AgentProfile 可用。

### P2 行业数据化（1-2 天）

1. 9 行业包全部落地，18 领域并入对应行业包或独立「领域包」。
2. 人才库（约 200 专家 + 75 workflow-templates json + 生成脚本）搬入。
3. `opc industry export/install` 实现（.opcip）。
4. 验收：任一行业可单独禁用/启用，导出包可装到另一实例。

### P3 公司运行时（2-3 周，依赖基座既有能力）

1. `company-runtime` crate 骨架 + `opc_work_items` 状态机（§3.2，纯函数 + 全流转单测）。
2. 组织抽象 CRUD + 招聘决策（§3.3）。
3. 看板投影：`opc_work_items` phase → Kanban 列，前端看板页。
4. 阻塞升级链：团队内升级 → 人工（ApprovalNode + rt-messaging 8 渠道）。
5. 经验闭环：`evaluate_round` → `opc_experience_records` → 晋升 `opc_playbooks` → 新人继承；对接 trajectory ExperiencePool。
6. 角色感知路由：rt-messaging `session_router` + role 字段（本地薄补丁）。
7. 验收：同一行业 brief 二次运行质量可测提升；阻塞升级到人工可用。

### P4 生态（后续）

1. 市场包（.opcip 市场），人才库导入（agency-agents 格式）。
2. 9 行业档案持续打磨，**金融行业吃 astock-data 工具链**（AxInvest 独有协同）。

---

## 6. 表设计（草案，全 opc_ 前缀）

| 表 | 阶段 | 关键列 |
|---|---|---|
| `opc_industries` | P0.5 | id / name / icon / version / enabled / pack_path / installed_at |
| `opc_invoices` | P0 | customer_id / invoice_number / status / line_items_json / total / currency / due_at / paid_at |
| `opc_customers` | P0 | name / email / status / tags_json / total_revenue / invoice_count |
| `opc_projects` | P0 | customer_id / title / status / milestones_json / budget / deadline |
| `opc_landing_pages` / `opc_blog_posts` / `opc_kpi_records` / `opc_revenue_records` / `opc_contact_submissions` | P0 | — |
| `opc_work_items` | P3 | run_id / phase（§3.2 枚举）/ owner_role_id / deps_json / assignee_agent_id / manager_role_id / last_error |
| `opc_orgs` / `opc_org_roles` / `opc_org_employees` / `opc_talent_templates` | P3 | §3.3 |
| `opc_experience_records` / `opc_playbooks` | P3 | §3.4 |

**前缀纪律**：`opc_*` 表只由 AxInvest OPC 业务层读写，投资域/基座不碰。

---

## 7. 风险与验收

### 7.1 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| 迁移改号遗漏（v209/v210） | 高 | P0 第一步改号，migrations 全量 diff 复查 |
| preset_templates.rs 误带入 AxOPC 删除 | 高 | 合并后 grep `build_stock_analysis_nodes` 必须存在 |
| 反编译行业包 profile id 断裂 | 中 | 转换后全量 seed + 校验 AgentProfile 绑定 |
| 角色感知路由触及 rt-messaging 共享文件（本地薄补丁） | 中 | 注释标记 + merge 上游时重放；不 push 上游 |
| 行业包引擎与 seed 重复执行 | 中 | 行业级版本号 + `check_template_version` 按包判断 |

### 7.2 验收标准

- **P0**：`cargo check` + `cargo clippy` 零警告；迁移幂等。
- **P0.5**：禁用 finance_invest 包后模板不再 seed；重新启用恢复。
- **P1**：发票/客户/项目 CRUD + 角色 AgentProfile 可用。
- **P2**：任一行业导出 .opcip → 另一实例 install → 模板/角色/工具完整。
- **P3**：同行业 brief 二次运行质量可测提升（Playbook 生效）；升级到人工可用。

---

## 8. 纪律提醒

1. **AxAgent 零改动**：公司运行时全部实现于本地 crate + 本地迁移（v211+）+ 本地薄补丁；上游共享文件改动（rt-messaging session_router）一律注释标记 + merge 时重放，**不 push 上游**。
2. **表前缀即边界**：`opc_*` 只由 OPC 业务层读写。
3. **搬入方式**：选择性移植（逐个文件/补丁提取），不用 git merge 全量（1991 文件差异噪音大）。
4. **行业包是数据不是代码**：新增行业 = 新增目录 + manifest，不碰 Rust 编译。
5. **依赖基座既有能力**：复用 rt-workflow（Debate/Swarm/DAG）、trajectory（RL 经验管道）、rt-messaging（8 渠道）、SKILL.md 技能系统，不重复造轮子。

---

*文档结束。执行顺序：P0 → P0.5 → P1 → P2 → P3 → P4。*
