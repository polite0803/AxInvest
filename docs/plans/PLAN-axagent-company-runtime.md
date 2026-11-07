> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# AxAgent 通用能力实施：AI 公司运行时内核（Company Runtime Core）

> 版本：v1.0　日期：2026-08-02　状态：待评审
> 仓库：`D:\OneManager\AxAgent`（upstream=polite0803/AxAgent）
> 定位：补齐 AxAgent 基座的「组织运营」通用能力（公司运行时内核），供 AxInvest OPC、未来 AxOPC/AxSim 等业务复用。**机制进基座，内容进业务**——本仓库只实现"怎么跑"，不实现"跑什么行业"。

---

## 1. 背景与目标

### 1.1 背景

- AxInvest 合并 AxOPC 时发现：OpenOPC 的核心机制（工作项状态机、组织自建、招聘决策、阻塞升级、经验归因、Playbook 晋升）是**跨行业通用能力**，不属于任何具体业务。
- 若这些机制落在 AxInvest 本地 crate，将导致：① 上游 AxAgent 永远缺这块能力；② 未来 AxOPC/AxSim 无法复用；③ 与 Harness 架构"基座做能力宽度"原则冲突。
- 决策：**通用机制在 AxAgent 实施（本文档），OPC 业务内容在 AxInvest 实施（见 `docs/axinvest-opc-business-plan.md`）**。

### 1.2 目标

1. 在 AxAgent 新增「公司运行时内核」：WorkItem 状态机、组织抽象、招聘决策、阻塞升级、经验归因与 Playbook 晋升、角色感知路由。
2. 全部以通用命名实现（`company_`/`work_items`/`orgs` 等），**不出现任何行业/台账语义**（opc_invoices 等留 AxInvest）。
3. 遵循 Harness 分层：DTO/trait 进 `axagent-harness`，实体进 `axagent-entities`，逻辑进新 crate `company-core`，路由增强进 `rt-messaging`。
4. 迁移从 v115 起编号（AxAgent 当前到 v114）。

### 1.3 与 AxInvest 的关系

| 层 | 归属 | 内容 |
|---|---|---|
| 通用机制 | **AxAgent（本文档）** | WorkItem 状态机、Org/Role/Employee 抽象、招聘、升级、经验/Playbook、角色路由 |
| OPC 业务 | **AxInvest**（`docs/axinvest-opc-business-plan.md`） | 9 行业数据资产包、台账（发票/客户/项目）、OPC 工具、6 角色专家、OpcPage |
| 依赖方向 | AxInvest → AxAgent | AxInvest merge 上游获得公司运行时，再在其上构建业务 |

---

## 2. 通用能力清单与落点

| 能力 | 内容（OpenOPC 语义） | 落点 | 分层 |
|---|---|---|---|
| C1 WorkItem 状态机 | Phase 流转（§3.1）+ 依赖传播 + 五种管理模式 | 新 crate `company-core` | implementor |
| C2 组织抽象 | Org / Role / Employee / Talent 实体与关系 | `axagent-entities` 新表 + `company-core` | implementor |
| C3 招聘决策 | Recruiter：复用员工（携带历史）vs 人才库招聘，失败降级 fallback | `company-core` | implementor |
| C4 阻塞升级链 | 团队内升级 → 人工升级（经 ApprovalNode + 渠道通知） | `company-core` + `rt-messaging` | hybrid |
| C5 角色感知路由 | session_router 加 role 字段，角色感知消息分发 | `rt-messaging` | hybrid |
| C6 经验归因 + Playbook | 反馈归因到角色 → 私有档案 → 晋升共享手册 → 新人继承 | `company-core` + `trajectory` 扩展 | implementor |
| C7 SelfImprovingRound 契约 | harness trait：`evaluate_round` 桥接 ReAct 循环与 RL 经验池 | `axagent-harness` | foundation |
| C8 市场包格式 | 公司/行业包解析、导出、导入（.opcpkg 等价物） | `company-core` | implementor |

---

## 3. 关键设计

### 3.1 WorkItem 状态机（C1，参考 OpenOPC Phase）

```
QUEUED → IN_PROGRESS ⇄ BLOCKED → REVIEW → APPROVED → DONE
    ↘ WAITING_FOR_CHILDREN（依赖项未完成）   ↘ FAILED / CANCELLED（终态，触发依赖传播）
```

- 终态 `DONE_PHASES = {APPROVED, FAILED, CANCELLED}`；依赖传播：FAILED/CANCELLED 依赖 → 下游 doomed（不可认领）；soft/info 依赖不 gate 认领。
- 五种管理模式：execute / delegate / review / integrate / rework（Manager 驱动）。
- 阻塞升级链：阻塞消息暂停发送方 → 激活最合适角色 → 超权限升级人工。

**表 `work_items`**：`id / run_id / phase / owner_role_id / deps_json / assignee_agent_id / manager_role_id / last_error / created_at / updated_at`

### 3.2 组织抽象（C2）

**表**：
- `orgs`：`id / name / company_profile / topology / final_decider_role_id`
- `org_roles`：`id / org_id / role_id / name / responsibility / reports_to / seniority`
- `org_employees`：`id / org_id / employee_id / role_id / expert_id / status / experience_ref`
- `talent_templates`：`id / category / name / description / source_repo / prompt_refs / skill_refs / tags`

（复用 AxAgent 已有 `agent_roles`/`agent_profiles`/`agency_experts` 表做映射，新增 org 系列表承载组织结构。）

### 3.3 经验归因与 Playbook（C6）

- **归因铁律**：用户反馈解析为逐员工评估，只更新拥有相关工作项的角色档案（防互相污染）。
- **晋升机制**：单角色经验档案按阈值晋升到共享 Playbook，新员工入职继承。
- **对接 trajectory**：`evaluate_round` 输出进 RL ExperiencePool（补 Loop Engineering 缺口）。

**表**：
- `experience_records`：`id / role_id / work_item_id / signal / content / created_at`
- `playbooks`：`id / role_id / title / content / promoted_from / version`

### 3.4 SelfImprovingRound 契约（C7，harness）

```rust
// axagent-harness
pub trait SelfImprovingRound {
    /// 评估一轮生成结果，返回结构化反馈（归因到角色的经验信号）
    async fn evaluate_round(&self, ctx: &RoundContext) -> Result<RoundEvaluation, RoundError>;
}
```
业务层（AxInvest OPC / 未来 AxOPC）实现 `evaluate_round`；`company-core` 负责把 `RoundEvaluation` 写入 `experience_records` 并驱动晋升。

### 3.5 角色感知路由（C5，rt-messaging 薄补丁）

- `session_router` 的会话模型加 `role_id` 字段（Option，向后兼容）。
- 消息分发按 `role_id` 过滤/定向；不破坏现有 8 渠道路由。
- **上游共享文件改动纪律**：本地验证 → 由用户在 AxAgent 正式提交 → 合并后不复盖。

---

## 4. 实施步骤（P0-P4）

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P0 契约与实体** | harness 定义 `SelfImprovingRound` trait + WorkItem/Org DTO；entities 新增 6 表（work_items/orgs/org_roles/org_employees/talent_templates/experience_records/playbooks）；迁移 v115 起 | `cargo check` 通过 |
| **P1 状态机内核** | `company-core` crate：WorkItem Phase 状态机 + 依赖传播 + 五种管理模式 + 看板投影（纯函数，无 IO） | 单元测试覆盖全部合法/非法流转 |
| **P2 组织与招聘** | Org/Role/Employee CRUD + 招聘决策（复用优先/招聘/fallback） | 招聘决策单测通过 |
| **P3 经验闭环** | 经验归因 + Playbook 晋升 + trajectory 对接（`evaluate_round` → ExperiencePool） | 二次运行质量可测提升 |
| **P4 路由与市场包** | rt-messaging role 路由 + .opcpkg 导出/导入 | 消息按角色定向；包可导出导入 |

---

## 5. 分层合规检查

| 落点 | 可依赖 | 检查 |
|---|---|---|
| `axagent-harness`（C7） | 零 axagent-* 依赖 | ✅ foundation |
| `axagent-entities`（C2 表） | 仅 harness | ✅ foundation |
| `company-core`（C1/C2/C3/C6/C8） | harness + entities + dao + trajectory | ✅ implementor（不依赖 consumer） |
| `rt-messaging`（C4/C5） | harness + 按需 implementor | ✅ hybrid（禁依赖 consumer/entities） |
| `trajectory`（C6 扩展） | harness + entities | ✅ implementor |

---

## 6. 风险与纪律

| 风险 | 缓解 |
|---|---|
| rt-messaging 角色路由触及上游共享文件 | 本地临时补丁 + 注释标记，验证后由用户正式提交上游 |
| 状态机语义翻译失真（OpenOPC Python → Rust） | P1 纯函数实现 + 全流转单测，不依赖运行时 |
| 与 AxInvest 迁移编号冲突 | AxAgent 侧从 v115 起，AxInvest 侧 v209/v210（各自独立仓库无冲突） |
| 招聘决策过度复杂 | 先实现确定性规则（复用→招聘→fallback），LLM 决策后续增强 |

---

*文档结束。完成后 AxInvest 通过 merge 上游获得 C1-C8，再实施 `docs/axinvest-opc-business-plan.md`。*
