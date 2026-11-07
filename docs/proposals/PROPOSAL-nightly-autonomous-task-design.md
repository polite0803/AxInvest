> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 夜间长时自主任务运行 · 详细设计文档

> 版本：v1.0（设计稿，未实施）
> 状态：待评审
> 范围：第一层（任务可靠/记忆/调度）+ 第二层（决策自主/风险/回退）+ 第三层（透明/成本/择时）

---

## 0. 设计原则（前置约束）

1. **复用优先，禁止重造轮子**：项目已存在成熟调度设施与执行引擎，本次改造**不新建 crate、不新增调度线程**，全部为「扩展既有结构 + 接线」。当需要新能力时，优先给现有类型加字段/方法，而非新建同义体系。
2. **规划与执行分离**：不写"巨型指令"。规划（拆解步骤）与执行（调用工具）是两个独立角色，规划只产步骤，执行只按步骤干。
3. **结构化输出**：日志、报告、审批请求、成本记录统一格式，机器可解析、人可读，严禁自由文本混杂指令。
4. **异常分级**：临时性（可重试）与永久性（停摆+报警）必须显式区分。

---

## 1. 现状盘点与复用基线

以下设施**已存在且被直接复用**，改造不触碰其核心逻辑，只做扩展/接线。

| # | 复用设施 | 位置 | 已具备能力 | 改造方式 |
|---|---------|------|-----------|---------|
| E1 | 执行引擎 | `crates/rt-workflow/src/work_engine` | `run/resume/cancel/loop_checkpoint/resume_breakpoints`；40+ 节点执行器（agent/tool/llm/loop/parallel/fallback/approval/email/notification/delay/logging…） | **直接作为执行核心**，新增能力用节点接入 |
| E2 | 工作流审批(HITL) | `work_engine` + `dao/repo/workflow_approval.rs` + `commands/work_engine.rs` | 审批持久化、`list_pending_approvals`(含超时自动裁决)、`resume_approval`(通过恢复/拒绝取消) | **人工安全阀底座**，补消息推送 + 超时默认拒 |
| E3 | 定时调度 | `runtime-core/cron_job.rs` | `CronJobStore`：`cron_jobs`表持久化、`list_due`、`calculate_next_run`、`recurring`(一次性)、三态、`run_on_startup`、`cron_job_history`表 | **作为夜间调度泵**，扩展字段 |
| E4 | 触发器恢复 | `src/init/trigger_recovery.rs` | 启动时从 `workflow_templates.trigger_config` 恢复非 manual 触发器 | **断点恢复的启动钩子**，扩展后台任务恢复 |
| E5 | 后台任务 | `commands/background_tasks.rs` + `entities/background_tasks.rs` | bash 任务提交/输出追加/状态机/Drop guard 兜底/事件 | 作为任务持久化载体，扩展状态与字段 |
| E6 | 记忆分层 | `runtime-core/compact.rs`(smart_compact) + `harness/memory.rs`/`rar_recaller.rs` | 四层阈值、记忆压缩→LLM摘要回退、分层召回 | **直接复用**，仅补任务日志压缩 |
| E7 | 自检回退 | `agent/self_verifier.rs` + `harness/reversible_effect.rs` + `consistency_check.rs` | 前后状态对比、多层验证、逆序回滚 | **接线**到关键节点执行后 |
| E8 | 成本定价 | `harness/usage_pricing.rs`(`ModelPricing.cost_for`) | 模型单价→成本折算 | **复用**，做成本估算/报告折算 |
| E9 | 双层决策 | `harness/business_rules.rs`+`rhai_engine.rs`+`domain_router.rs` | 规则契约(RuleResult/RequiresApproval)、规则引擎、路由 | **补闭环**三段流水线 |

---

## 2. 总体架构

**不再新建调度中枢 crate，改为对 E3 + E5 的"调度协调器"增强层**，落在 `src-tauri/src/scheduler/`（应用层，wiring 角色），通过 harness trait 与 work_engine 交互。

```
用户/CLI/定时                      cron_jobs 表 (持久化)
   │                                    ▲
   ▼                                    │ register recurring=false 一次性
 CronJobStore (E3, 复用, 扩展字段)──────┘
   │ list_due 轮询触发
   ▼
 Scheduler 协调器 (新增: src-tauri/src/scheduler/)
   ├─ 恢复引导  (启动扫描 background_tasks 未完成 → 重新入队+续跑)
   ├─ 优先级排序 (submitted_at + priority + 预估耗时加权)
   ├─ 成本门控   (E8 估算 → 时段择时 + max_budget 熔断 + 通知管理员)
   ├─ 审批闸门   (E2 复用 → 高风险挂起 + IM/邮件推送 + 超时默认拒)
   ├─ 回退收口   (E7 复用 → 失败 rollback + 标 needs_investigation)
   └─ 汇报触发   (report_generator + E8 成本 + E2 审批汇总)
   │
   ▼ 编排为工作流蓝图
 work_engine (E1, 复用): run/resume/cancel/loop_checkpoint
```

---

## 3. 逐需求改造方案

> 每个需求给出：复用什么 / 新增什么 / 关键改动 / 涉及文件。

### ① 任务"不丢失、不重复、可追溯" — 补恢复引导 + 幂等

- **复用**：E5(background_tasks 实体，已有唯一 id + 状态 + 时间戳 + output)、E1(loop_checkpoint)、E4(trigger_recovery)。
- **新增**：
  1. `background_tasks` 表加字段（migration）：`idempotency_key`(唯一键，防重复提交)、`attempt`(重试计数)、`resume_from`(断点位置，冗余到 checkpoint)。
  2. `Scheduler::restore()`：启动时 `list where status in (running, pending)` → 重新入队，`agent` 任务从其 checkpoint 续跑，`bash` 任务标记 `attempt+1` 并重跑。
  3. Tauri 命令 `submit_background_task` 支持可选 `idempotency_key`，重复 key 返回已有任务。
- **涉及文件**：`entities/background_tasks.rs`、`migration`、`src-tauri/src/scheduler/restore.rs`、`commands/background_tasks.rs`、`src/init/services.rs`(启动调用 restore)。

### ② 长对话记忆分层管理 — 判定已达要求，仅补任务日志压缩

- **复用**：E6 全部。
- **新增（可选）**：任务日志按「关键结论 / 全量流水」分级压缩；达到 token 上限自动压缩并保留结论引用。
- **涉及文件**：`crates/agent/src/content_synthesizer.rs` 或复用 `smart_compact` 对任务日志调用。
- **结论**：会话记忆分层已满足核心诉求，此条为低优先级补充。

### ③ 异步排队与优先级调度 — 扩展 CronJobStore，不加线程

- **复用**：E3 的 `CronJobStore::add/list_due` 轮询机制作为调度泵。
- **新增**：
  1. `CronJob` 扩展字段：`priority`(低/中/高/批量)、`epoch_cost_estimate`(预估成本，由 E8 估)、`queue_position`(可选)。
  2. 调度排序策略：`list_due` 返回后按 `priority desc, estimate asc, created_at asc` 排序 → 已满足"提交时间+紧急度+预估耗时自动化排列"。
  3. 闲时批量：低优先级任务加入每日批处理窗口（见 ⑧ 择时），高峰期不取。
- **涉及文件**：`runtime-core/cron_job.rs`(扩展 CronJob 字段+排序)、`src-tauri/src/scheduler/queue.rs`。

### ④ 双层决策机制（规则优先，模型辅助）— 补三段闭环

- **复用**：E9 的 business_rules(含 `RequiresApproval`) + rhai_engine。
- **新增**：在 `harness/domain_router.rs` 明确三段流水线：
  1. 规则能否覆盖该 (节点类型, 输入)？→ **能**则直接用 `RuleAction` 结果，不调模型；
  2. **不能**才交给 `reasoning_router` 调模型；
  3. 模型输出**再经规则校验**，违规则降级/阻断/转审批。
- **涉及文件**：`harness/domain_router.rs`、`kit/src/command_validator.rs`(补校验)。
- **原则**：低风险高频走规则，模糊才走模型，模型结果必须过规则关。

### ⑤ 风险分级与人工"安全阀" — 复用审批，补推送与超时默认拒

- **复用**：E2 完整审批基础设施（挂起 `resume_breakpoints` + 持久化 + 投票）。
- **新增**：
  1. **审批请求推送**：`workflow_approvals.channels` 已有字段但未接发送；审批记录创建时通过 `notification_executor` / `email_executor` / 飞书 IM 推送请求与审批链接。
  2. **超时默认拒绝**：修正 `workflow_approval.rs::auto_resolve_timeouts`（当前仅标 `expired`），改为按 `timeout_action` 缺省 **deny**，并确保 `resume_approval` 拒绝分支真正 `engine.cancel`。
  3. **风险等级策略落地**：低→纯自主；中→执行后记日志，次日复核（沿用 `operation_audit`）；高→执行前挂起等授权。
- **涉及文件**：`dao/repo/workflow_approval.rs`、`commands/work_engine.rs`、`work_engine/executors/approval_executor.rs`、`vite` 前端 `approvalStore.ts`(展示推送链接)。

### ⑥ 执行后自检与回退 — 接线 E7 到关键节点

- **复用**：E7（self_verifier 前后状态对比、reversible_effect 逆序回滚、consistency_check）。
- **新增**：
  1. 数据修改/发消息类关键节点执行后自动 `verify`（对比执行前后状态快照）。
  2. 验证失败 → `EffectScope.rollback_all()` 回退到操作前状态。
  3. 回退后任务置 `status = needs_investigation`（`background_tasks` 新增状态值），汇入 ⑦ 报告。
- **涉及文件**：`work_engine/executors/tool_executor.rs`(接入验证+回滚钩子)、`self_verifier.rs`(暴露对节点的通用入口)。

### ⑦ 长时工作汇报 — 补触发 + 两字段并入

- **复用**：`report_generator.rs`(MD/HTML/JSON)、thought_chain、insight_generator。
- **新增**：
  1. 任务完成事件 → `Scheduler` 判定超过阈值（时长/成本）→ 自动触发 `report_generator`。
  2. 报告中并入：**成本折算**(E8 `cost_for`) + **审批记录汇总**(查 `workflow_approvals` 的 resolved/approver/comment)。
- **涉及文件**：`src-tauri/src/scheduler/report.rs`、`agent/report_generator.rs`(扩展成本/审批段)。

### ⑧ 动态择时执行（成本感知）— 复用定价零件，补门控逻辑

- **复用**：E8 `ModelPricing.cost_for` + `pricing.toml`。
- **新增**：
  1. `gate.rs`：取当前时段模型价格 → 低则提前启动排队任务，高则让非紧急任务进入延迟/闲时窗口。
  2. **成本上限**：用户设 `max_budget`；累计超限 → 停止新任务 + 通过 notification/email 通知管理员。
  3. 采用**分钟级二次采样**探价，不做实时常驻值守（复用 cron tick 频率）。
- **涉及文件**：`src-tauri/src/scheduler/gate.rs`、`kit/billing.rs`(补成本上限状态，当前只有余额查询)。

---

## 4. 关键设计决策：如何复用 CronJobStore 而不冲突

- **不加新调度线程**：夜间任务统一表示为 `recurring=false` 的一次性 CronJob 注册进 `CronJobStore`，由现有 `list_due` 轮询触发执行，通过 `TriggerManager` → `work_engine.run_workflow`。
- **调度协调器不做独立进程**：`Scheduler` 是纯函数式协调层（恢复/排序/门控/汇报），只被 cron tick 与任务完成事件调用，不持有自己的定时循环。
- **一次性任务续跑**：`run_on_startup` + `restore()` 保证重启后未完成任务重新入队。
- **破坏性覆盖检查**：修改 `CronJob` 结构加 `priority`/`epoch_cost_estimate` 字段为向后兼容新增 Option 字段，不破坏现有访问代码。

---

## 5. 数据模型变更（migrations）

| 表 | 变更 |
|----|------|
| `background_tasks` | + `idempotency_key TEXT UNIQUE`、+ `attempt INTEGER DEFAULT 0`、+ `resume_from TEXT`、status 新增 `needs_investigation` |
| `cron_jobs` | 结构内 `CronJob` 加 `priority`、`epoch_cost_estimate`(Option) |

> 遵守 AGENTS 铁律 #10：表结构变更必须写 migration，不直接改 entity。

---

## 6. 后端命令清单（命令注册三步：`commands/mod.rs` + `lib.rs`，缺一不可）

| 命令 | 动作 |
|------|------|
| `submit_background_task` | 能力扩展：支持 `idempotency_key`、`priority`、`budget` |
| `restore_pending_tasks` | 手动触发恢复引导（调试用） |
| `set_task_priority` | 运行中重排序 |
| `set_budget` / `get_budget_usage` | 成本上限管理 |
| `get_task_report` | 取 ⑦ 报告 |
| 复用：`resume_approval`、`cancel_approval`、`list_pending_approvals` | 审批门 |

> 全部走 `invoke<T>()`（AGENTS 铁律 #1），错误带 `ErrorResponse` 错误码并同步 11 语言 `error` 段翻译（AGENTS 铁律#5-7）。

---

## 7. 前端改造（camelCase 对齐，DTO 两步同步）

- 新增/扩展 store：后台任务面板轮询 `submit_background_task`/`list_background_tasks` 状态；审批面板（复用 `approvalStore`）展示推送的审批链接；成本仪表盘（复用 `CostChart`）。
- 新增字段类型消费 camelCase：`idempotencyKey`、`priority`、`epochCostEstimate`、`needsInvestigation`。
- 组件命名导出、UI 文本 11 语言同步、Monaco 白名单无新增。

---

## 8. 设计陷阱核对

| 陷阱 | 如何规避 |
|------|---------|
| 巨型指令 | 规划=workflow 蓝图节点，执行=work_engine 各 executor，二者接口分离 |
| 结构化输出 | 报告/审批/成本统一走 `report_generator` + `json_schema`，审批请求固定 schema |
| 异常分级 | `recovery_strategies` transient→retry；`error_classifier` permanent→停摆 + 通知管理员 |

---

## 9. 落地顺序（MVP 收敛）

| 阶段 | 交付 | 价值 |
|------|------|------|
| **P1** | 恢复引导 + 幂等键 + 审批推送 & 超时默认拒绝 | 补齐最高风险的安全阀与断点恢复 |
| **P2** | 优先级排序 + 成本门控 + 择时 + 汇报触发 | 成本可控 + 透明化 |
| **P3** | 自检回退接线 + 双层决策闭环 | 自愈与自主 |

---

## 10. 风险与待确认

- `CronJob` 扩展字段涉及 `runtime-core`（被大量引用），需谨慎做兼容；改动前先 `cargo check`。
- 审批"超时默认拒绝"是对 `auto_resolve_timeouts` 行为的语义变更，需与现有拒绝/取消语义对齐，避免误杀。
- 高风险审批**通过前绝不执行**是硬约束：需在 `approval_executor` 验证审批记录存在且非拒绝才放行。
- 成本探价频率、`max_budget` 粒度、审查复核窗口需产品确认。
