# ADR-P5 — P5 架构决策记录（8 条）

> 状态：frozen-for-P5 ｜ 日期：2026-11-25 ｜ 基线：`fedf00c17` + 本地三提交（`b0a43360e` / `1c7e75b63` / `afe255c40`）｜ 前置：`ADR.md`、`ADR-P2.md`、`ADR-P2B.md`、`ADR-P3.md`、`ADR-P4.md`、`GAP-P5.md`
>
> **裁决来源**：`SCOPE-OPTIONS-P5.md` §6 的 D1–D6 由用户于 2026-11-25 采纳建议值（D1=2 条切片、D2=事后熔断、D3=A 不进本批、D4=C3 不进本批、D5=G-B13 并入 C1、D6=冻结后两视图登记 INDEX）。本文即该裁决的架构落点，每条附「理由 / 后果 / 反转成本」。
>
> 与 ADR-P4 的关系：**不 supersede P4 任何条款**。P4 的运行配额（2/900/12 等六个上限、只收紧、拒绝不夹紧）继续有效；P5 只在**费用/Token 维度**上加一层。

## ADR-P5-1 — 两条独立纵向切片：B（费用/Token 配额）与 C1（每 Bot 技能归属）同批交付、分别签字（D1）

**决策**：P5 = 两条互不依赖的切片。
- **P5-b** = 费用/Token 配额（用量账本 + 费用配额 + 事后熔断 + 用量 UI）；
- **P5-c1** = 每 Bot 技能归属 + preset 导入（含 G-B13 白名单）+ 技能页签。

两条各有独立的实体、命令、前端与测试；同批提交但在收口判定中**分别签字**。任一条延期，另一条可单独收口并把延期项标 `deferred`。

**理由**：
1. B 的边际成本最低 —— P4 刚落地「只收紧」的配额解析层与 task-local 下发路径，B 只是在这条路上**加一个费用维度**，不是新造一层（`SCOPE-OPTIONS-P5` §5.1 第 1 条）。
2. B 的用户价值是「可见」而非「精确」—— 8–12 人日换「哪个 Bot 在烧钱」的可见性与超限熔断，对 8–12 人的团队是刚需。
3. C1 与 B **零交集**（一个动运行时记账面，一个动技能索引面），可并行；C1 是本批唯一的新用户可见面，且**没有运行时风险**。
4. A 的三个子项都不满足「一个可独立上线的纵向切片」（ADR-P5-7）。

**后果**：收口判定分 B/C1 两节签字；INDEX 登记同一批文档；任一节的 FAIL 不影响另一节 PASS。

**反转成本**：若必须整批原子上线，只需把收口判定从「分节签字」改为「全绿才签」，零代码成本。

## ADR-P5-2 — 用量账本 append-only，写入时固化归属（不做事后 join）

**决策**：新增 `bot_usage_ledger` 表，**只 INSERT 不 UPDATE 不 DELETE**。每次 Bot 相关模型轮结束时写一行，写入时固化：`bot_id`、`department_id`（写入时快照）、`conversation_id` / `execution_id`、`workflow_run_id`、`assignment_id`、`model_id`、`origin`（`interactive` / `routine` / `assignment`）、四类 token 数、`estimated_cost_usd`、`pricing_source`、`created_at`。

**理由**：
1. **会话会被清理** —— P4 `PLAN-P4.md:419` 原文「例程会话还会被清理」。事后 join `messages` / `agent_sessions` 会**越查越少**，账本必须写入时固化。
2. **跨 Bot 归因无法事后还原** —— `bot_assignments` 无 token/cost 列；一轮委派花的钱记在哪个 Bot 名下，事后再也说不清。
3. append-only 让**并发写是单条 INSERT**（无锁、无更新冲突），汇总全部走查询；且账本可审计、可追责。

**后果**：
- 账本**只写不改**，与会话生命周期解耦；清理任务不触账本（`GAP-P5` B-8）。
- `department_id` 是**写入时快照**：Bot 事后换部门，历史用量仍记在当时的部门（账本是历史事实，不随组织结构改写）。这条口径写进 UI 说明。
- 账本体积随轮次线性增长；本批**不做归档/清理**（超长期留作 P6 候选）。

**反转成本**：若改为可更新（按 run 归并成一行），需引入 upsert 与并发冲突处理，账本从「事实」变成「聚合」，审计价值下降；不划算。

## ADR-P5-3 — 费用配额只收紧 + 事后熔断：超限只拦新建 run/轮，已启动的跑完（D2）

**决策**：
- **作用域**：Bot 维度与部门维度**两个 scope**。Bot 配额管该 Bot 自己的用量；部门配额管该部门**全部 Bot** 的合计用量（部门是现有唯一组织维度，`bot_departments` + `bot_department_members`；不引入第三种 scope 体系）。
- **只收紧**：费用配额是**上限**，`min(定义值, 配额)` 语义与 P4 `ADR-P4-2` 一致；不设「默认值可被配额放大」的路径。
- **事后熔断**：用量要等一轮结束才知道 ⇒ **超限时只拦「新建的 run / 新建的轮」**：
  - 已启动的编排 run **跑完**（不中断、不改状态、不回滚）；
  - 新建的 run / 新的例程轮 / 新的委派轮 **在创建前被拒**，返回 `BOT_COST_QUOTA_EXCEEDED`（detail 带 scope、当前用量、上限）。
- **不做预扣**：不预扣、不预留、不按预算削减模型输出。

**理由**：
1. 用量只有在一轮结束、拿到 `summary.usage` 之后才知道 —— **事前无法精确判断**，任何「精确扣费」都是假的。
2. 事后熔断是唯一可解释、可测试的语义：用户看到的是「上一轮结束时已超限 ⇒ 下一轮被拒」，因果链清楚。
3. 与 P4 一致：P4 的运行配额也是「只收紧 + 超额拒绝」（`BOT_QUOTA_EXCEEDED`），不夹紧、不静默。
4. 中断已启动的 run 需要引入「取消 / 回滚 / 部分计费」三套语义，成本与副作用都放大，且与 P3 的 run 终态机冲突。

**后果**：
- 熔断有**滞后性**：超限当轮照常跑完，下一轮才被拒。UI 必须写明这一点。
- 一次超限后 Bot 进入「已被熔断」状态，直到用量窗口滚动或配额被调高。熔断状态**由用量查询派生**（当前窗口用量 ≥ 上限），不另存状态位 ⇒ 无状态一致性问题。
- 熔断只挡**新建**，不挡人在场的交互轮（交互轮是用户主动发起的，且 P5-b 的交互记账只是**可见性**，不做拦截）—— 这条边界写进 UI 与文档，避免用户困惑。

**反转成本**：若改为事前拦截（按预算预测 token），需要模型输出长度预测，误差大且不可解释；不采纳。若改为中断已启动的 run，需引入 run 取消语义与部分计费口径，约 +2–3 人日。

## ADR-P5-4 — 费用是「估算」，不做对账承诺

**决策**：账本的 `estimated_cost_usd` 是**按本仓 `pricing.toml` + `usage_pricing` 四类分价算出的本地估算值**，UI **必须标注「估算」**；不承诺与 provider 账单一致。

计价来源优先级（沿用 `commands/agent/pricing.rs` `estimate_cost_usd` 的既有优先级）：
1. 调用方传入的模型单价（`resolved_model.input_price_per_mtok` / `output_price_per_mtok`）；
2. `src-tauri/pricing.toml` 的 `[[models]]`（按 `model_id` 或 `aliases` 匹配）；
3. `heuristic_pricing` 的档位兜底（`nano`/`mini`/`pro`/`opus`/`deepseek`/`qwen`，默认 `(2.50, 10.00)` / Mtok）；
4. `usage_pricing::pricing_for_model` 的硬编码表（`axagent-core/crates/harness/src/usage_pricing.rs:87-436`）。

`pricing_source` 字段落库，标明该行用的是哪一档来源。**缓存 token 单价独立**：`usage_pricing` 的 `cache_creation_cost_per_million` / `cache_read_cost_per_million` 与普通 token 不同，账本必须四类分价分别计（`GAP-P5` B-7）。

**理由**：跨 provider 计价口径不统一（缓存读写计价、套餐、区域差价都不同）；本地估算的用途是「相对比较谁在烧钱」，不是「报销单」。写死 provider 单价会在价格调整时全线失真。

**后果**：
- UI 的费用列必须带「估算」标识（i18n 键 `bot.usage.estimated`），不得写成「账单」「费用」这种对账语义。
- `pricing_source` 让用户能判断某行的可信度：命中 `pricing.toml` 比落到 heuristic 更可信。
- 价格表更新后历史账本**不重算**（append-only，ADR-P5-2）⇒ 历史值反映当时的估算口径。

**反转成本**：若要接 provider 真实用量 API，需要按 provider 各写一套适配 + 时区/口径对齐，约 +5 人日；本批不做。

## ADR-P5-5 — 每 Bot 技能 = 目录即真相源 + 显式归属表；全局技能永远可见，Bot 技能仅属主可见

**决策**：
- **物理落点**：每 Bot 的技能放在 `~/.axagent/bots/<botId>/skills/<skillName>/SKILL.md`（沿用 `BotWorkspace::for_bot` 的既有 per-bot 根，**不新建第二套路径约定**）。
- **显式归属表** `bot_skill_bindings`（`bot_id` + `skill_name` + `source`（`workspace` / `preset`）+ `preset_id` + 时间戳）：回答「哪些技能是这个 Bot **显式装的**」（目录扫描只能回答「哪些文件在」）。
- **可见性规则**：
  - 全局技能（`skill_dirs.rs` 六类源目录 + `~/.axagent/config.yaml` 的 `skills.external_dirs`）对**所有 Bot 永远可见**，行为逐字节等于今天；
  - Bot 技能**仅对属主 Bot 可见**，Bot 之间互不可见；
  - **同名时 Bot 内优先**（Bot 自己的技能压过全局同名技能）。
- **启停分层**：`skill_states`（全局启停）**语义不变**；Bot 技能另有「本 Bot 停用」标记，不改全局表。

**理由**：
1. 「技能仍是全局库」是 P1–P4 一直未动的缺口（`GAP-P5` C1-1）；直接把全局库改成 per-bot 会破坏全部既有技能与 `SkillIndex` 的目录优先级语义。
2. **先加归属维度、保留全局兜底**（`SCOPE-OPTIONS-P5` §4.4）：全局技能天然对所有 Bot 可见，**无需迁移**（C1-11 不做），改动面最小。
3. 目录即真相源保证「删文件 = 删技能」；显式归属表保证「装了哪些」可审计。二者缺一不可。
4. 同名冲突必须按「Bot 内优先」裁决，否则「我的 Bot 有个同名技能」永远被全局目录压掉（C1-3）。

**后果**：
- `SkillIndex` 扫描期加作用域过滤：全局条目照旧，Bot 条目仅在属主 Bot 的会话里出现。
- `BotWorkspace` 增 `skills_root()`；不新增 per-bot 根目录。
- 归属表与目录可能不一致（文件被手删但归属行还在）⇒ 读取时以**目录为准**，归属表只作归属声明与审计；不一致在 UI 上标「缺失」。

**反转成本**：若改为纯 per-bot（无全局兜底），需要一次性迁移全部既有技能 + 改 `SkillIndex` 优先级语义，约 +2 人日且破坏既有用户配置；不采纳。

## ADR-P5-6 — preset 导入格式 `bot-skill-preset.v1`，整包校验后原子落盘，白名单过滤（G-B13 并入）

**决策**：
- **格式**：JSON，版本字段 `version: "bot-skill-preset.v1"`，含 `name` / `description` / `skills[]`（每项：`name` / `description` / `content`（SKILL.md 正文，含 frontmatter）/ `category` / `tags` / `requiresToolsets`）。
- **落点**：导入到 `~/.axagent/bots/<botId>/skills/` —— **不污染全局库**。
- **原子性**：**整包校验通过才落盘**（每项 `name` 合法、frontmatter 可解析、正文非空）；校验失败则**不留半包**。
- **白名单（G-B13，D5 裁决）**：preset 内可导入的技能名受**共享模板白名单**约束；不在白名单的条目**跳过并回报**（不整包失败）。
- **提示词注入**：Bot 的技能名以 `<bot-skills>` 段注入系统提示词，**只列名字与一行描述，不注入全文**；正文仍走既有 `SkillsList` / `SkillView` 渐进式披露。

**理由**：
1. 白名单本质是 preset 导入的**过滤规则**，分开做要改两次导入路径（`SCOPE-OPTIONS-P5` §4.5，D5 已裁决并入）。
2. **整包原子**是关键：`BotWorkspace::write_soul_atomic`（`bot_workspace.rs:85-96`）已有 tmp+rename 的原子写先例，沿用同一手法。
3. 白名单条目**跳过而非整包失败**：白名单是「哪些允许进」的政策，不在白名单的条目不是用户的错误；整包失败会让大 preset 永远导不进。
4. 只列名不注入全文：全文注入会撑爆上下文预算（与 ADR-005 的 MEMORY 注入预算同款考量）。

**后果**：
- 导入命令需返回「成功 n 条 / 跳过 m 条（白名单）/ 失败原因」。
- 白名单内容与来源**可配置**（`~/.axagent/config.yaml` 的 `skills.preset_whitelist`），默认空 ⇒ 默认不过滤（不改变现状行为）。
- **未核实项**：OMB `presets.md` 原文格式本轮**未取回**（`GAP-P5` F-1）；本格式以本仓 `SKILL.md` frontmatter 约定为真源定义，后续取回原文后只做格式对齐，不改语义。

**反转成本**：若改为非原子逐条导入，实现更简单但会产生半包状态；不采纳。若白名单来源改为硬编码清单，改动 ≤0.5 人日。

## ADR-P5-7 — A（多 Bot 协作运行时）不进本批；先做 A-min，且只在真实协作需求出现时立项（D3）

**决策**：P5 **不做**候选 A 的三个子项：
- **A1** 事故线程 + 自动重试（≤2 次）—— 不做；
- **A2** 线程 `openedBy` 归属 —— 不做；
- **A3** Bot 群聊 / rooms —— 不做。

后续若立项，**先只做 A-min**（A1 + A2，7–10 人日），rooms 单独立项。

**理由**：
1. 三个子项都不满足「一个可独立上线的纵向切片」这条立项原则（`SCOPE-OPTIONS-P5` §5.1 第 4 条）。
2. **自动重试是成本放大器** —— P3 `GAP-P3.md` G-P13 已因「重试使成本与副作用翻倍」明写不做；P5 正在引入成本可见性，同批再上自动重试是自相矛盾。
3. rooms 等于第二套 IM（成员 / 可见性 / 未读 / 通知四件都要重做），且与 `fleet_messages`、`bot_threads` 语义重叠。
4. A2 改共享表 `conversations` 会波及全部会话列表查询与 DTO，回归面远超本批。

**后果**：
- `bot_assignments` 五态、`bot_workflow_runs` 六态**零变更**（与 P4 同款承诺）。
- 事故上报仍是 P2-b 口径：失败即 `failed` + `error_code`，前端可见即够（`GAP-P2B` G-B12）。
- 线程归属仍是 P1 口径（`conversations.bot_id`），不加 `opened_by` 列。

**反转成本**：A-min 单独立项约 7–10 人日，与 P5 无耦合（不碰账本、不碰配额），可随时起。

## ADR-P5-8 — 委派轮用量必须经执行器回传：不改 `execute` 签名，用新增方法 + 默认实现

**决策**：`AssignmentTurnExecutor`（`src-tauri/src/bot_assignment_runner.rs:45-57`）的现有两个方法
（`execute` / `execute_with_context`）**签名保持不变**；新增一个带回传用量的方法：

```
async fn execute_with_usage(
    &self,
    assignment: &BotAssignment,
    task_text: &str,
) -> Result<AssignmentTurnOutcome, String>   // 默认实现 = 现有 execute_with_context 包一层（用量为空）
```

`AssignmentTurnOutcome { text: String, usage: Option<BotTurnUsage> }`。P2-b 的 Fake 执行器（测试用）**零改动**（默认实现兜住）；真实执行器 `RoutineTurnExecutor` 覆写该方法，从 `run_bot_non_interactive_turn_with_context` 的 `summary.usage` 取数。

**理由**：
1. 现有契约 `Result<String, String>` **没有 token 通道**（`GAP-P5` §1「委派轮用量」行），不改就拿不到委派链的成本归因 —— 而委派/编排正是「谁花的」的关键路径。
2. **不能改 `execute` / `execute_with_context` 的签名**：它们被 P2-b 的 Fake 执行器与 `src-tauri/tests/bot_delegation_chain.rs` 等测试实现；改签名会让 P2-b/P3 全部测试跟着改，回归面失控。
3. 默认实现 = 旧行为，**旧实现者零改动**，符合 P4「不重造既有能力」的约束。

**后果**：
- `run_bot_non_interactive_turn_with_context` 需要把 `summary.usage` 一并回传（新增一个带回传的变体或改返回值为结构体）——这是**本批唯一必须改的既有执行路径**。
- 用量为 `Option`：`None` 表示「该执行路径不产生用量」（Fake / 无模型环境），账本不写行，**不写 0 行**（避免污染统计）。
- `ConsultRunner::consult`（`bot_assignment_runner.rs:359+`）同走新方法。

**反转成本**：若改为直接改 `execute_with_context` 签名，需同步改 P2-b Fake + `bot_delegation_chain.rs` + `bot_assignment.rs` 测试，约 0.5–1 人日，但会让 P2-b 的测试面被 P5 污染；不采纳。
