# GAP-P5 — P5 差距分析（费用/Token 配额 + 每 Bot 技能归属）

> 状态：frozen-for-P5 ｜ 日期：2026-11-25 ｜ 基线：`fedf00c17`（tag `v3.1.0`）+ 本地三提交（`b0a43360e` / `1c7e75b63` / `afe255c40`）｜ 分支：`feat/p3-bot-workflow-frontend` ｜ 对应版本：3.1.0（若落地）｜ 来源：本地原创
>
> 前置：`GAP.md`、`GAP-P2.md`、`GAP-P2B.md`、`GAP-P3.md`、`GAP-P4.md`、`SCOPE-OPTIONS-P5.md`（候选稿）、`ADR-P5.md`
>
> 方法沿用 P1–P4：A（已有可复用）/ B（需改造或接线）/ C（缺失需新建），逐条给「P5 做 / 不做 + 理由」。
> **本轮未重新联网复核 OMB 原文**：`presets.md` / `runtime-contract.md` / `team-sharing.md` 未取回原文，C1 的 preset 导入格式以本仓既有 `SKILL.md` frontmatter 约定为真源（见 §4 C1-6），OMB 侧格式差异留作后续核对项，不臆造细节。

## 0. P5 范围冻结结论（承接 `SCOPE-OPTIONS-P5.md` §5–§6 的 D1–D6 裁决）

| 裁决 | 冻结值 | 本文落点 |
|---|---|---|
| D1 切片数 | **2 条**（B 费用/Token 配额、C1 每 Bot 技能归属 + preset 导入） | §2 / §3 |
| D2 B 的拦截语义 | **事后熔断**：超限只拦「新建 run / 新建轮」，已启动的跑完 | §2 B-5 / ADR-P5-3 |
| D3 A 是否进本批 | **不进**（多 Bot 协作运行时单独立项） | §5 |
| D4 C3 记忆写入审批档 | **不进本批**（属 P5-b / P6 候选；本批不做记忆写入面） | §5 |
| D5 G-B13 共享模板白名单 | **并入 C1**（preset 导入的过滤规则） | §3 C1-6 |
| D6 INDEX 登记 | 冻结稿落盘后**两视图各登记一行** | `docs/INDEX.md`（本轮已登记） |

**P5 = B + C1 两条互不依赖的纵向切片**，各自可独立上线、各自独立签字（同 ADR-P4-7 的体例）。

## 1. 需求与现状映射

| 需求 | 现状证据 | 判定 |
|---|---|---|
| Token / 费用计价 | `axagent-core/crates/harness/src/usage_pricing.rs`：`ModelPricing`（:22-28）、`pricing_for_model`（:87）、`UsageCostEstimate::total_cost_usd`（:72）、`cost_for_tokens`（:443） | **A 复用** |
| 计价来源二选一 | `src-tauri/pricing.toml`（`[[models]]` + `[budget]`）+ `src-tauri/src/commands/agent/pricing.rs`：`lookup_pricing_from_config`（:50）、`heuristic_pricing`（:78）、`estimate_cost_usd`（:186） | **A 复用** |
| 单轮用量可得 | `axagent-core/crates/harness/src/conversation_model.rs:154-165` `TokenUsage`（input/output/cache_creation/cache_read + `total_tokens()` :173） | **A 复用** |
| 费用落库 | 仅 `agent_sessions.total_cost_usd`（`entities/src/agent_sessions.rs:23`），写点唯一：`commands/agent/mod.rs:2590-2616` `estimate_cost_usd` → `agent_session::update_agent_session_after_query` | **B 接线**：需 Bot 维度账本 |
| Bot 维度归因 | `bot_assignments` / `bot_workflow_runs` **无 token/cost 列**（`entities/src/bot_assignments.rs:16-59`） | **C 新建**：append-only 账本表 |
| 委派轮用量 | `AssignmentTurnExecutor::execute_with_context`（`bot_assignment_runner.rs:45-57`）返回 `Result<String, String>` —— **执行器契约里没有 token 通道** | **B 改造**：执行器回传用量 |
| 例程轮用量 | `bot_routine_runner.rs:269-281` `summary.usage` 全量在手（`run_turn_with_tools` 返回 `TurnSummary`） | **A 复用**：就地写账本 |
| 编排步骤用量 | `bot_workflow_runner.rs` `orchestrate`（:538-665）只建 step assignment，**不做模型调用**；`on_assignment_completed`（:244+）只收 `&BotAssignment` | **B 接线**：经 assignment 回执上送 |
| 配额解析层 | `src-tauri/src/bot_quota.rs`：`QuotaLimits`（:28-35）、`AssignmentContext`（task-local）、`check_run_allowed`（:249）、`tension`/`tighten` 形态 | **A 复用**：加一个费用维度而非重造层 |
| 配额只收紧原则 | P4 `ADR-P4-2`：`min(定义值, 配额)`，超额**拒绝**不静默夹紧 | **A 复用**：P5 沿用同一原则 |
| 技能目录 | `axagent-kit/crates/kit/src/skill_dirs.rs`：`SKILL_DIR_PRIORITY`（:6）、`skill_dirs()`（:114）、`all_skills_dirs()`（:123）、`reload_skill_dirs()`（:106） | **A 复用** |
| 技能索引与提示词 | `axagent-runtime/crates/tools/src/tools/skill/mod.rs`：`SKILL_INDEX`（:18）、`SkillIndex::rebuild`（:125）、`list_skills`（:418）、`SkillsListTool`（:483）、`SkillViewTool`（:561） | **A 复用**：只加归属过滤层 |
| 技能启停 | `entities/src/skill_states.rs`（name/enabled/updated_at）、`dao/repo/skill.rs`、`commands/skills/install.rs`（`list_skills` :109 / `toggle_skill` :344） | **A 复用**：全局启停不动 |
| Bot 工作区 | `axagent-ai/crates/agent/src/bot_workspace.rs`：`BotWorkspace::for_bot`（:46-52）= `~/.axagent/bots/<botId>/`，管 SOUL.md / MEMORY.md / memory/** | **A 复用**：每 Bot 技能落在同根下的 `skills/` |
| Bot 侧栏页签 | `src/components/bot/BotDetail.tsx:178-236` Tabs（threads/routines/assignments）；`BotDepartmentModal.tsx:33` `initialTab?: "basic" \| "quota"` + `:149-155` Tabs 接线 | **A 复用**：照此加 skills 页签 |
| 错误码 / i18n / 审计 | P4 12 码、504 码实测基线（11 语言各 504）、`BOT_I18N_KEYS` 185 条、`bot-ipc-audit.mjs` P3/P4 段 | **A 复用**机制 |

## 2. P5-b（B）逐条 A/B/C 判定 — 费用 / Token 配额

| # | 差距 | 现状 | 判定 | P5 做 / 不做 | 理由 |
|---|---|---|---|---|---|
| B-1 | 用量账本存储 | 无 Bot 维度账本表 | **C** | **做**：新表 `bot_usage_ledger`，append-only | 会话会被清理（P4 `PLAN-P4.md:419`「例程会话还会被清理」）；事后 join 越查越少，账本必须写入时固化 |
| B-2 | 写入点缺失 | 例程轮有 `summary.usage` 但无处写；委派轮执行器**不回传 token** | **B** | **做**：① 例程轮就地写；② 委派轮把用量并入执行器返回值 | 委派/编排是「谁花的」的关键路径，不写则跨 Bot 无法归因 |
| B-3 | 费用维度配额 | `bot_department_quotas` 只有 6 个**运行**上限，无 token/cost 维度 | **B** | **做**：新增 `bot_cost_quotas` 表（Bot/部门维度） | P4 配额是纯策略覆盖、无记账链路；费用配额需要记账，两者正交（`GAP-P4` G-Q6 明写「另立 P5」） |
| B-4 | 配额解析层 | `bot_quota.rs` 已有一套「只收紧」解析 + task-local 下发 | **A** | **复用**：加费用维度，不新造层 | P4 刚落地这层上下文，换批次做就凉了（`SCOPE-OPTIONS-P5` §5.1 第 1 条） |
| B-5 | 超限拦截语义 | 无 | — | **做**：**事后熔断**（D2） | 用量要等一轮结束才知道，无法在触发前精确判断；「超限停新建 run/轮，已启动的跑完」是唯一可解释的语义 |
| B-6 | 计价口径 | `pricing.toml` + `heuristic_pricing` 兜底 | **A** | **复用**：只做本地估算，UI 标「估算」 | 跨 provider 计价不统一，不做对账承诺（`SCOPE-OPTIONS-P5` §3.4） |
| B-7 | 缓存 token 计价 | `estimate_cost_usd`（`pricing.rs:186`）**只算 input/output**，忽略 cache token | **B** | **做**：账本写入时用 `usage_pricing` 的四类分价（含 cache_creation / cache_read） | 缓存读写单价与普通 token 不同，漏算会让账本系统性偏低 |
| B-8 | 清理任务与账本 | 会话清理任务存在 | — | **做**：账本只写不改，与会话生命周期解耦 | 账本若随会话清理，「这周花了多少」会随时间消失 |
| B-9 | 交互轮（人在场）记账 | `commands/agent/mod.rs:2590` 已在写 `agent_sessions.total_cost_usd` | **B** | **做**：同处**增写一条账本行**（带 botId / conversationId） | 交互轮是 Bot 线程，成本归因最清晰；复用既有计价结果，零重复计算 |
| B-10 | 编排 run 级汇总 | `bot_workflow_runs` 无 token/cost 列 | **B** | **做**：run 终态时按 run 内账本行汇总（派生值，不新表） | run 是用户最关心的「这一次编排花了多少」 |
| B-11 | 预算（会话/日级）拦截 | `BudgetConfig.max_cost_per_day_usd` / `max_cost_per_session_usd` 字段存在但**无消费点** | **不做** | **不做**（留 P6 候选） | 这两个字段是**全局**口径，与本批的 Bot/部门口径正交；改它会牵动通用聊天路径 |
| B-12 | 精确扣费 / 预留 | 无 | **不做** | **不做** | 无法预扣，事后口径已是本批上限 |
| B-13 | 配额 UI | 无 | **C** | **做**：Bot 详情页「用量」页签 + 部门弹窗「用量」页签 | 无 UI 的配额等于不可用（同 `GAP-P4` G-Q5） |
| B-14 | 成本预警通知 | 无 | **不做** | **不做** | 通知面属 P2-a 既有管线，扩展需另立口径；本批只做「可见 + 拦截」 |

## 3. P5-c1（C1）逐条 A/B/C 判定 — 每 Bot 技能归属 + preset 导入

| # | 差距 | 现状 | 判定 | P5 做 / 不做 | 理由 |
|---|---|---|---|---|---|
| C1-1 | 技能按 botId 归属 | 技能目录全是全局（`skill_dirs.rs` 六类源目录 + 外部目录），**无 botId 维度** | **C** | **做**：每 Bot 技能落在 `~/.axagent/bots/<botId>/skills/<skillName>/` | P1 只做了 SOUL/MEMORY 注入；技能仍是全局库 ⇒ 「这个 Bot 会什么」不可配 |
| C1-2 | 归属与全局库的共存 | `SkillIndex::rebuild`（`skill/mod.rs:125`）按目录优先级取同名首个 | **B** | **做**：索引期加 botId 作用域过滤（全局技能永远可见，Bot 技能仅对属主 Bot 可见） | 「先加归属维度、保留全局兜底」（`SCOPE-OPTIONS-P5` §4.4）；直接把全局库改成 per-bot 会破坏全部既有技能 |
| C1-3 | 同名冲突 | 全局库内同名按优先级取首个 | **B** | **做**：Bot 技能与全局同名时**Bot 内优先**；Bot 之间互不可见 | 「我的 Bot 有个同名技能」是正常诉求，不该被全局目录压掉 |
| C1-4 | preset 导入格式 | 无 preset 概念 | **C** | **做**：定义 `bot-skill-preset.v1` 导入格式（JSON） | P2-c 原文要求「OMB preset 导入格式」；本轮未取回 OMB 原文，格式以本仓 `SKILL.md` frontmatter 约定为真源（见下） |
| C1-5 | preset 导入的落点 | 无 | **B** | **做**：导入落到 `~/.axagent/bots/<botId>/skills/`，**不污染全局库** | 导入是用户显式动作；污染全局库会让「哪个 Bot 装的」不可追溯 |
| C1-6 | G-B13 共享模板白名单 | `GAP-P2B.md:34` 判「留 P2-c」；`PLAN-P2.md:87` 的 P2-c 清单未含白名单 | **B** | **做**：并入 C1 —— preset 导入时按白名单过滤可导入技能 | D5 裁决。白名单本质是导入过滤规则，分开做要改两次导入路径（`SCOPE-OPTIONS-P5` §4.5） |
| C1-7 | 导入的原子性 | 无 | — | **做**：整包校验通过才落盘；失败不留半包 | 半包状态会让「Bot 的技能列表」不可解释 |
| C1-8 | 技能启停语义 | `skill_states` 是**全局**启停 | **A** | **复用**：全局启停不动；Bot 技能另设「本 Bot 停用」标记 | 全局停用一个技能会影响所有 Bot，属另一个语义；不混 |
| C1-9 | 归属写入的持久化 | 无 | **C** | **做**：新表 `bot_skill_bindings`（bot_id ↔ skill_name，append 归属声明） | 目录即真相源不足以回答「哪些技能是**这个 Bot 显式装的**」；需要显式归属记录才能做 UI 列表与审计 |
| C1-10 | 运行时注入 | `build_bot_system_prompt`（`bot_routine_runner.rs:42-45`）注入 SOUL/MEMORY | **B** | **做**：Bot 技能名在提示词里以 `<bot-skills>` 段列出（**只列名字，不注入全文**） | 全文注入会撑爆上下文；技能正文仍走既有 `SkillView` 渐进式披露路径 |
| C1-11 | 迁移既有全局技能 | 全部技能都在全局库 | **不做** | **不做** | 不搬：全局技能对所有 Bot 天然可见，无需迁移 |
| C1-12 | Bot API key（C2） | 无对外调用面 | **不做** | **不做** | 属对外攻击面，需 key/速率/审计/吊销四件同批，另立批次 |
| C1-13 | 记忆写入（C3） | Bot 不能写自己的 MEMORY | **不做** | **不做**（D4） | 需先裁决「记忆写入属哪一档审批」，是待裁决项，不塞进关键路径 |
| C1-14 | 技能编辑器 | 只能改 SKILL.md 原文 | **不做** | **不做** | 属于「能力面补全」的后续项，本批只做归属 + 导入 |

## 4. 两处需在实现前核对的外部事实（如实标注为未核实）

| # | 事实 | 本轮状态 | 处置 |
|---|---|---|---|
| F-1 | OMB `presets.md` 的 preset 原文格式 | **未取回**（本轮离线） | 以本仓 `SKILL.md` frontmatter 约定（`name` / `description` / `version` / `category` / `domain` / `platforms` / `tags` / `requires_toolsets` / `fallback_for_toolsets`，见 `src-tauri/agency_experts/skills/risk-management/SKILL.md:1-13`）定义 `bot-skill-preset.v1`；后续取回原文后只做**格式对齐**，不改语义 |
| F-2 | OMB `team-sharing.md` / `runtime-contract.md` 的多 Bot 协作语义 | **未取回** | 本批不做 A（候选 A 已 D3 判不进本批），无依赖 |

## 5. P5 做 / 不做清单

**做（P5-b 费用/Token 配额）**：
`bot_usage_ledger` 账本表与 DAO；`bot_cost_quotas` 配额表与 DAO；例程轮 / 委派轮 / 交互轮三个写入点；委派执行器回传用量；费用配额解析（只收紧）+ 事后熔断（拦新建 run/轮）；`get/set/reset_bot_cost_quota` + `list_bot_usage` + `get_bot_usage_summary` 命令；Bot 详情页「用量」页签 + 部门弹窗「用量」页签（标「估算」）；Rust 测试 + e2e + 台账。

**做（P5-c1 每 Bot 技能归属）**：
`bot_skill_bindings` 归属表与 DAO；`BotWorkspace` 增 `skills/` 根；`SkillIndex` 加 botId 作用域过滤（Bot 内优先）；`bot-skill-preset.v1` 导入格式 + 白名单过滤 + 原子落盘；Bot 详情页「技能」页签；`<bot-skills>` 提示词段（只列名）；Rust 测试 + e2e + 台账。

**不做（P5 范围外）**：
多 Bot 协作运行时（事故线程 / ≤2 重试 / `openedBy` / rooms，D3）；Bot API key（C2）；记忆写入与维护（C3，D4）；会话/日级全局预算拦截（B-11）；精确扣费与预扣（B-12）；成本预警通知（B-14）；Bot 技能正文编辑器（C1-14）；既有全局技能迁移到 per-bot（C1-11）；P1–P4 冻结口径的任何变更。

## 6. 复用边界（实现线禁止重造）

- **计价**：只用 `usage_pricing`（四类分价）+ `commands/agent/pricing.rs` 的 `lookup_pricing_from_config` / `heuristic_pricing`；不新造价格表、不写死 provider 单价。
- **配额解析**：只在 `bot_quota.rs` 旁**加一个维度**（费用），沿用 `min` 只收紧与 task-local 下发；不新造第二套解析器。
- **执行器**：`AssignmentTurnExecutor` 的**签名扩展必须向后兼容**（`execute` / `execute_with_context` 保持现有签名，新增 usage 回传用**新方法 + 默认实现**，P2-b 的 Fake 执行器零改动）。
- **技能索引**：只在 `SkillIndex` 加**作用域过滤**；不重写扫描器、不改 `SKILL.md` 解析、不动 `skill_states` 全局启停。
- **Bot 工作区**：只在 `BotWorkspace` 根下加 `skills/` 子目录；不新建第二套 per-bot 路径约定。
- **提示词注入**：`<bot-skills>` 只列名字；技能正文仍走 `SkillsList` / `SkillView` 渐进式披露。
- **错误/i18n/审计**：沿用 `error_code.rs`、11 语言 locale、`bot-ipc-audit.mjs` 既有结构；新增码先 locale 后代码。
