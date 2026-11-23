# ACCEPTANCE-P2B — P2-b 门禁验收实测台账

> 状态：active ｜ 日期：2026-11-23 ｜ 验收人：Bot 门禁与验证工程师 ｜ 基线：`ee6690c3a`
> 环境：`ssh -o BatchMode=yes wsl 'bash -s'` → `/home/polit/projects/AxInvest`（WSL ext4，不在 Z:/9P 构建）。
> 单条命令上限 25 分钟（`timeout 1500s`）；本次无卡住。未 push。

## 0. 最终树与提交链（原文）

```
$ git log --oneline -12            # 验收开始时（T6'' 提交前）
1000a71cf feat(bots): P2-b T3'' 委派执行器 + consult 工具（dispatch 全链 + 一层防环）
c9681d923 feat(bots): P2-b T4'' IPC 委派命令层（3 命令 + 错误码 ×4 + 事件枚举 + 审计扩展）
92b2883f7 feat(bots): P2-b T2'' 部门命令层 + delete_bot 级联（5 命令 + 错误码 ×2）
1fba0e84b feat(bots): P2-b T5'' 前端部门管理与委派区（i18n 11 语言 + mock + 事件刷新）
e0407212b feat(bots): P2-b T1'' 三实体 + DAO + SCHEMA 登记（bot_departments / members / assignments）
066d7f6ff docs(bot-integration): P2-b spec package (GAP/ADR/MODEL/IPC/PLAN/I18N-KEYS) + INDEX registration
ee6690c3a test(bots): P2-a T6' 例程 e2e 收口（browserMock 例程命令 + 纵向/负路径 spec）
7870ee91e feat(bots): P2-a T2' Bot 例程执行分支（cron executor botId 路由 + headless 轮 + 删除级联）
f1651d66a feat(bots): P2-a T5' IPC 命令层（Bot 例程命令 + botId/timezone 透传 + 新错误码）
f4cd1c00a feat(bots): P2-a T4' Bot 例程 UI 与通知 botId 维度
b2769ed67 feat(bots): T1' CronJob 加 botId/timezone JSON 字段 + 时区化档期计算
b93d4b7c4 docs(bot-integration): P2-a spec package (GAP/MODEL/IPC/PLAN/ADR) + INDEX registration
```

五卡顺序与实现线报告一致；T5''（`1fba0e84b`）确在后端 T2''（`92b2883f7`）之前，与契约冻结后并行开工的计划相符。

```
$ git status --short                # 验收开始时（摘录 Bot 相关 2 行，完整 14 行见会话报告）
 M src/lib/browserMock.ts           # 插件线：paper_portfolio_* / screenshot_diagnosis_* 浏览器 mock
 M src/pages/__tests__/BotsPage.test.tsx  # 前端线：P2-b 部门数据源 mock 补齐
（其余 12 项均为插件线/其他线脏文件：paper portfolio、screenshot diagnosis、layer allowlist 等）
```
工作树脏文件不属门禁线，全程未碰、未提交。

```
$ git log --oneline -3              # T6'' 门禁线独立提交后
1fc4689f9 test(bots): P2-b T6 委派 e2e 收口（部门归属/dispatch/取消/删 Bot 级联）
1000a71cf feat(bots): P2-b T3'' 委派执行器 + consult 工具（dispatch 全链 + 一层防环）
c9681d923 feat(bots): P2-b T4'' IPC 委派命令层（3 命令 + 错误码 ×4 + 事件枚举 + 审计扩展）
```
提交 `1fc4689f9` 只含 `e2e/bot-delegation.spec.ts`（197 行），提交前 `git diff --cached --name-only` 为空。

## 1. 时序矛盾核查：前端报的 6 红

前端线报告 `check-contracts.mjs` 因后端错误码文档未登记报 6 红；在最终树 HEAD（`1000a71cf`）重跑：

```
$ node scripts/check-contracts.mjs
[BOT] Bot P1 IPC 契约：errors=0 warnings=0
[汇总] errors=0 warnings=0
结果: PASS
EXIT=0
```
结论：**已转绿**，非遗留红。同轮 `node scripts/check-errorcode-alignment.mjs` → `OK: 后端 473 个错误码 ↔ 前端 11 语言 error 段对齐一致，无孤儿码。`（EXIT=0）。

## 2. T6'' e2e：真实文件与原始输出

文件存在性（验收开始时）：`MISSING e2e/bot-assignment.spec.ts`；PLAN-P2B §6 指定落点为 `e2e/bot-delegation.spec.ts`，该文件在原树中也不存在，已由门禁线补齐并提交（`1fc4689f9`）。

```
$ npx playwright test e2e/bot-delegation.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list

Running 2 tests using 2 workers

  ✓  2 [chromium] › e2e/bot-delegation.spec.ts:180:3 › Bot 部门归属与委派（P2-b T6″） › 负路径：空部门名被表单校验拒绝，模态不关闭且不落库 (7.2s)
  ✓  1 [chromium] › e2e/bot-delegation.spec.ts:102:3 › Bot 部门归属与委派（P2-b T6″） › 纵向：建部门 → dispatch 委派 → 取消 → 删 Bot 级联 → Chief/成员保留 (13.0s)

  2 passed (14.2s)
EXIT=0
```

覆盖：建 Bot A(edits)/B(ask) → 建部门（Chief=A，成员 A+B）→ A 发 dispatch 给 B → assignee 面板 queued → UI 取消（cancelled，终态无取消按钮）→ 第二单在途 → 删 B → 两单级联 cancelled、Chief 保留、B 从成员消失；负路径空部门名被拒且不落库。

**mock 边界（不做假绿）**：`browserMockBots.ts` 无运行时/调度器，`create_bot_assignment` 只落 queued。queued→running→completed、ask 档写拒绝（failed/ERR_PERMISSION）、300s 超时、并行门、回执落线程由 Rust 测试锁：`src-tauri/tests/bot_assignment.rs` 4 测全绿（含 `dispatch_happy_path_completes_emits_and_appends_receipt`、`permission_denial_marks_failed_with_readable_detail`、`timeout_marks_failed_with_timeout_code`）。

**可见性口径**：MODEL-P2B §2.3 规定 person 发起时 `requester_bot_id=NULL`，面板按 `botId` 过滤只命中 assignee，故 UI 断言在 B 的「委派」区做，级联结果直读数据面 localStorage。首版 e2e 曾在 A 的面板断言失败（收到 0 条），据此按规格口径修正，不是把红项改绿。

## 3. T7'' 全量逐项 PASS/FAIL 表

| # | 命令（原文） | Exit | 关键输出摘要 | 判定 |
|---|---|---:|---|---|
| 1 | `npm run typecheck` | 0 | `[check-tsc] tsc 发现 src 源文件 1588 个，开始 --noEmit 类型检查…`；无 error TS | PASS |
| 2 | `node scripts/vitest-run.mjs run` | 0 | `Test Files 319 passed (319)`；`Tests 3383 passed (3383)`；Duration 100.06s | PASS |
| 3 | `node scripts/check-contracts.mjs` | 0 | `[汇总] errors=0 warnings=0`；`结果: PASS` | PASS |
| 4 | `node scripts/check-errorcode-alignment.mjs` | 0 | `后端 473 个错误码 ↔ 前端 11 语言 error 段对齐一致，无孤儿码` | PASS |
| 5 | `node scripts/check-bot-i18n-keys.mjs` | 0 | `语言: 11 必需 key: 102`；`结果: PASS` | PASS |
| 6 | `npm run check:layers` | 0 | `✅ 无超基线违规`；`模式: ratchet ｜ 结论: ✅ 通过` | PASS |
| 7 | `cargo test --manifest-path axagent-ai/Cargo.toml -p axagent-agent` | 0 | lib `1945 passed; 0 failed`；各集成测试全绿；doc-tests 5 ignored | PASS |
| 8 | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao` | 0 | lib `411 passed`；`tests/schema_compliance.rs` 1 passed；全套 0 failed | PASS |
| 8a | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_` | 0 | `9 passed; 402 filtered out` | PASS |
| 8b | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib reconcile` | 0 | `243 passed; 168 filtered out` | PASS |
| 8c | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib counts::` | 0 | `13 passed; 398 filtered out` | PASS |
| 8d | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance` | 0 | `1 passed; 0 failed` | PASS |
| 9 | `cargo test --manifest-path axagent-ai/Cargo.toml --test bot_assignment --test bot_routine` | 101 | `error: no test target named bot_assignment in default-run packages`（命令路径有误：测试不在 axagent-ai workspace） | **命令路径错误**，非代码红 |
| 9a | `cargo test --manifest-path src-tauri/Cargo.toml --test bot_assignment --test bot_routine` | 0 | `bot_assignment` 4 passed；`bot_routine` 4 passed；0 failed | PASS（正确路径） |
| 10 | `cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate` | 0 | 4 passed（tool_name/schema、consult、runner error、per_turn_gate）；472 filtered out | PASS |
| 11 | `npm run check:i18n` | 0 | 11 locale 语法正常；`代码中 t() 唯一 key 10452`，全部在 zh-CN 有定义 | PASS |
| 12 | `npm run check:i18n:key-parity` | 0 | 11 语言 × 16415 叶子路径一致；正控有效 | PASS |
| 13 | `npm run check:routes` | 0 | 路由 40 条、入口 34 个；无新增不可达路由 | PASS |
| 14 | `npm run check:serde` | 1 | `axagent-core/crates/harness/src/vector_store.rs`：EmbeddingRecord / VectorSearchResult / HnswConfig 缺 camelCase 注解 | **存量红**（与 P1/P2-a 台账一致，未修） |
| 15 | `npx playwright test e2e/bot-delegation.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list` | 0 | `2 passed (14.2s)` | PASS |

## 4. 归属与剩余项

- Bot 线真红：**0**。唯一 FAIL 为存量 `check:serde`（vector_store.rs，非 Bot 文件）；命令 #9 是 manifest 路径写错，按 #9a 正确路径全绿。
- 工作树 14 项未提交改动属插件线/前端线，门禁线未改、未提交、未 push。
- 未验证：真实 Tauri 桌面前端的 UI 渲染与真人操作（e2e 在 Vite + browserMock 面）；到点自动执行与 ask 档写拒绝以 Rust 集成测试为准。
