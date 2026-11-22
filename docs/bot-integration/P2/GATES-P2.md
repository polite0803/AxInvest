# P2-a T7' 前置：5 项门禁缺口落地方案（成文不实现，T1'–T4' 合入后由 T6'/T7' 收口）

> 状态：方案稿 ｜ 日期：2026-11-23 ｜ 基线：`b93d4b7c4` ｜ 作者：Bot 门禁与验证工程师
>
> 来源：P1 验收（2026-11-23 01:05）列出的 5 项 P2 门禁缺口。**两处口径修正**（相对缺口原始清单）：
> ① 审批矩阵不再按「max(own, inherited) 全序」——冻结稿 ADR-P2-1 已改为会话覆盖/非交互取 Bot 档/custom→Prompt/只降不升四规则；
> ② 例程不要求「必须显式时区」——冻结稿 IPC-P2 §2.1 `timezone?: string|null`，null=宿主 Local 是合法值。
> 本方案按冻结稿落笔。

## 1. `scripts/check-prompt-slots.mjs` — 提示词 slot 注入门禁

**补的盲区**：P1 的 SOUL/MEMORY 注入（`bot_context.rs`）没有任何门禁对齐「渲染出的提示词」，注入点改动（如挪到 system 段之后）会静默失效；反向风险是 slot 泄漏进普通会话。

**断言清单**（全部硬失败）：

- A1 Bot 会话渲染出的完整提示词含 `<bot-soul>`、`<bot-memory>` 两个 slot，各**恰好出现一次**，顺序为 soul→memory（与 `resolve_bot_context` 注入顺序一致）。
- A2 slot 内容与 BotWorkspace 实际文件一致：把 fixture Bot 的 SOUL/MEMORY 写入临时 workspace，断言 slot 内文本逐段包含（不含包裹标签本身）。
- A3 非 Bot 会话（无 botId）提示词与基线快照**逐字节一致**——快照存 `docs/bot-integration/evidence/prompt-slots-nonbot.json`，diff 即红（防 slot 泄漏）。
- A4 存在 slot 时，上下文不得含空占位串（如空 SOUL 渲染出的 `<bot-soul></bot-soul>` ⇒ 红；空文件应整个省略 slot，与 P1 行为对齐）。

**实现要点**：纯函数入口——直接调 `resolve_bot_context` 的等价构造路径（或经 `agent_turn_adapter` headless 模式拿最终 prompt），不真跑模型；fixture 存 `scripts/fixtures/prompt-slots/`。若实现侧函数不导出，T7' 收口时要求后端补一个 `#[cfg(test)]` 导出或集成测试钩子，不在门禁侧 mock 字符串拼接（mock 了就守不住真链路）。

**落点**：`scripts/check-prompt-slots.mjs`；CI：`ci-check.mjs` Bot 段追加；本地：进 `p1:accept` 静态段。

## 2. `scripts/check-routine-bot-ownership.mjs` — 例程 botId 归属与 cron 校验

**补的盲区**：例程数据在 `cron_job` JSON 里（不可 SQL 索引，GAP G-10），孤儿 botId、坏 cron 只有运行时才炸。

**断言清单**（对 `store.list()` 全量反序列化逐条判定；实现落 T2' 后跑在真实数据上）：

- A1 每条 `task_type == "bot_routine"` 的记录携带非空 `botId`，且该 bot 存在——指向已删除 bot = FAIL（T2' 的 `delete_bot` 级联删例程后此断言应恒绿，本门禁防绕过）。
- A2 cron 表达式恰为 5 字段（复用 `check-cron-job-authority.mjs` 现有解析器，不另写语法判定）。
- A3 `timezone` 字段：`null`（=宿主 Local，合法）或能在 `chrono_tz` 等价清单（JS 侧用 `Intl.supportedValuesOf('timeZone')`）中解析；拒本地偏移串（如 `UTC+8`）。**不**要求非空——IPC-P2 §2.1 冻结口径 null=Local。
- A4 非 Bot 例程（无 `botId`）不受 A1 约束，仅要求 `botId` 字段类型合法（string|null）。
- A5 熔断：候选违规 >50 条降级为仅报告（沿用 `check-i18n-orphan-diff.mjs` 的显式降级惯例，禁止静默跳过）。

**落点**：`scripts/check-routine-bot-ownership.mjs`；CI：Bot 段追加。

## 3. 审批解析矩阵测试（T3' 单测的门禁登记，非新脚本）

**判据按 ADR-P2-1/ADR-P2-4 冻结稿**，表驱动锁定六条（T3' 实现时逐条对应，T7' 验收核对覆盖面）：

- R1 会话档位非 `default` 时**双向覆盖** Bot 档（specific 覆盖 general）：`accept_edits` 会话 + `full` Bot ⇒ 会话档生效；`full` 会话 + `ask` Bot ⇒ 同理（不是只升不降）。
- R2 会话档位 `default`/未设 ⇒ 取 Bot 档映射。
- R3 非交互源（例程）**只取 Bot 档**，无会话合并对象。
- R4 `custom` ⇒ `Prompt`（最安全解释），且 `level_supported(任意 provider, "custom") == false`。
- R5 五档映射表全量：ask→Prompt、edits→WorkspaceWrite、auto→WorkspaceWrite（ADR-P2-4 建议值，Q1 裁决后只改此格）、full→Allow。
- R6 只降不升护栏：解析结果流经 `resolve_effective_permission` 后不得高于原上限（delegation 等强制上限不变）。

**落点**：T3' 在 `src-tauri/src/bot_approval.rs` 同文件 `#[test]` 矩阵（六条全覆盖）；门禁侧动作 = T7' 在 `ACCEPTANCE-P2.md` 登记 `cargo test -p axagent --lib bot_approval` 并核对矩阵条数 ≥ 六规则 × 组合（≈25 行表驱动用例）。前端如有重复解释（如设置页提示），vitest 侧补一条「文案与映射表一致」的轻断言即可，不另起矩阵。

## 4. 通知归属计数测试（T4' vitest 的门禁登记）

**断言清单**（落 `src/lib/__tests__/notification*.test.ts` 或 T4' 同目录）：

- A1 未读计数按 `botId` 严格隔离：构造 bot-A 的通知后，bot-B 视角 unread == 0（跨 bot 泄漏 = FAIL）。
- A2 旧通知（无 `botId`）在「全部 Bot」视图可见、计入总数（IPC-P2 §4：映射查不到不写字段、通知仍落）——回归保护：P1 通知零丢失。
- A3 计数口径唯一：同一条通知不得同时计入两个 bot 的未读；广播形态（无 botId）只计入「全部」。
- A4 标记已读只影响本 bot 的计数（bot-B markAllRead 后 bot-A unread 不变）。

**落点**：T4' vitest；门禁侧动作 = T7' 验收核对四条覆盖 + `npx vitest run --grep-invert "@heavy"` 绿。

## 5. schema_compliance P2 登记（扩展既有测试，非新脚本）

**现状**：`axagent-infra/crates/dao/tests/schema_compliance.rs:159,273` 只锁了 `bots` 表 + `bot_id` 列。

**方案**：T1'/T5' 落地后，把新列登记进实体定义清单（判定条件：实体 struct 字段 ↔ 迁移后 DB 列集合**双向相等**）：

- `cron_job` 新列 `bot_id`、`timezone`（T1'；注意 MODEL-P2 的 serde-default JSON 字段形态——若落 JSON 列内而非物理列，登记的是「JSON 键存在性」断言而非 DB 列，按实际 DDL 形态二选一，T1' 落地时定）。
- 新实体表（若 T2'/T5' 引入）逐表登记。
- **不做**通知 botId 的后端登记——IPC-P2 §4 冻结「通知是纯前端 localStorage 模型」，无后端表，登记反而暗示了不存在的契约。

**落点**：`schema_compliance.rs` 扩展由后端在 T1' 落（测试文件属后端改动面）；门禁侧动作 = T7' 验收跑 `npm run test:rust` 对应段并核对零 diff。

## 6. 附带：验收证据日志入库（*.log 被 .gitignore:9 忽略）

**方案**：`docs/bot-integration/evidence/*.log` 改为 **`.md` 后缀入库**（内容本就是命令原文 + exit code + 输出摘要的文本，改后缀即可读可 diff），不动 `.gitignore`（`*.log` 白名单会放过真正的构建日志）。`p1-acceptance.mjs` 写日志处的文件名常量同步改。已存在的 7 份 `.log`：改后缀迁入同一目录。随 T7' 一并执行（属 docs/scripts 面不撞前后端）。

## 7. 收口顺序

T1'–T4' 提交齐 → §1/§2/§3/§4/§5 按「实现者落测试、门禁核对覆盖」执行 → T7' 全量回归 + `ACCEPTANCE-P2.md` 登记 + §6 证据日志迁移。
