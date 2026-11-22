# Bot 集成 P1 —— 验收清单与门禁口径

> 维护者：门禁与验证工程师
> 适用范围：`docs/bot-integration/PLAN.md` 的 P1（T1–T6）
> 最后实测：2026-11-22，基线 commit `6b9e5ea85`

本文件回答两个问题：

1. **P1 的每条验收点，证据在哪？**（下表「证据」列给出可复跑的命令/文件）
2. **一条命令怎么知道 P1 能不能收口？**（`npm run p1:accept`）

---

## 1. 一条命令验收

```console
$ npm run p1:accept            # 静态门禁 + typecheck + vitest 全量（默认，约 4–6 分钟）
$ npm run p1:accept -- --quick # 只跑静态门禁（秒级，改文案时用）
$ npm run p1:accept -- --e2e   # 再加 Bot e2e（浏览器模式 6 条）
$ npm run p1:accept -- --full  # 再加 cargo（migration / schema_compliance / agent）+ Bot e2e
```

脚本：`scripts/p1-acceptance.mjs`
行为：按固定顺序执行，记录**每条命令原文、exit code、耗时、输出尾部**，
最后打印 `PASS/FAIL` 判定表；完整日志写入
`docs/bot-integration/evidence/p1-acceptance-<时间戳>.log`。
退出码：`0` = 全绿；`1` = 任一红项。

### 当前判定（2026-11-22）

| 门禁 | 命令 | 结果 | 归属 |
| --- | --- | --- | --- |
| TypeScript | `npm run typecheck` | ✅ 1577 文件，exit 0 | — |
| 单测全量 | `node scripts/vitest-run.mjs run` | ✅ 311 文件 / 3323 用例 | — |
| 契约 A/C/D/F/G/H + BOT | `node scripts/check-contracts.mjs --only=a,c,d,f,g,h,bot` | ❌ **H 段 +1**：`src-tauri/src/commands/bot.rs:471` 的 `let _ = bot_repo::set_soul_drift(..).await;` | 后端（Bot 运行时） |
| Bot 契约（单段） | `node scripts/check-contracts.mjs --only=bot` | ✅ 0 error / 0 warning | — |
| 错误码 ↔ 11 语言 | `node scripts/check-errorcode-alignment.mjs` | ✅ 465 码对齐 | — |
| i18n 严格 | `python3 scripts/check_i18n.py --strict` | ✅ | — |
| i18n key 集合 | `node scripts/check-i18n-key-parity.mjs` | ✅ 11 语言一致（16351 条） | — |
| **Bot i18n 必需 key** | `node scripts/check-bot-i18n-keys.mjs` | ✅ 46 key × 11 语言 | — |
| 分层纪律 | `node scripts/check-layer-discipline.mjs` | ✅ | — |
| Serde 注解 | `node scripts/check-serde-annotations.ts` | ❌ `axagent-core/crates/harness/src/vector_store.rs`（**既有红，非 Bot**） | 架构/存量 |
| 路由可达性 | `npm run check:routes` | ✅（`--strict` 另有既有的 `/wiki` 孤儿键） | 存量 |
| Bot e2e | `npx playwright test e2e/bots.spec.ts --project=chromium --grep-invert "@heavy"` | ✅ 6/6 | — |
| e2e 全量 | `CI=1 npx playwright test --project=chromium --grep-invert "@heavy"` | ✅ 114 passed / 2 既有 `fixme` / 0 failed | — |

> 结论：**P1 代码面已可用，门禁剩 1 条 Bot 相关红项**（H 段 silent Result）。
> 修掉 `bot.rs:471` 后 `npm run p1:accept` 应只剩「Serde 既有红」——见 §4。

---

## 2. P1 验收点逐条对照

| # | 验收点 | 结果 | 证据 |
| --- | --- | --- | --- |
| 1 | 创建 Bot（列表即时出现，不刷新） | ✅ | `e2e/bots.spec.ts:143`；`src/pages/__tests__/BotsPage.test.tsx`；`src/stores/feature/__tests__/botStore.test.ts` |
| 2 | 切换 Bot（列表选中 → 详情/线程跟着切） | ✅ | `e2e/bots.spec.ts:133`、`:205`；`BotList`/`BotsPage` 单测 |
| 3 | 线程归属（新线程落到 Bot 下） | ✅ | `e2e/bots.spec.ts:154`（点击后查 `localStorage` 的 `botId`） |
| 4 | 按 Bot 过滤线程（Web 模式不恒空） | ✅ | `e2e/bots.spec.ts:183`；`src/lib/__tests__/conversationBotFilter.test.ts`；**mock 层回归锁** `src/lib/__tests__/browserMock.bots.test.ts`（create/update 双路径写 `botId`） |
| 5 | 记忆读取（只读，200 行 / 24 KB 预算） | ✅ | `e2e/bots.spec.ts:205`（断言 `2/200`）；`BotMemoryPanel` 单测 |
| 6 | 记忆写入 | — | P1 按 IPC.md 不提供写命令（`read_bot_memory` 只读）；不在本期验收 |
| 7 | 重启后持久化（刷新后线程仍归属） | ✅ | `e2e/bots.spec.ts:154` 的 `page.reload()` + localStorage 断言 |
| 8 | 删除 Bot 不删线程与记忆（ADR-009） | ✅ | `e2e/bots.spec.ts:223`；删除确认文案含「线程与记忆会保留」 |
| 9 | 11 语言文案齐备 | ✅ | `node scripts/check-bot-i18n-keys.mjs`（46 key × 11） |
| 10 | 9 个 Tauri 命令契约（名称/参数/返回/注册/DTO/错误码） | ✅ | `node scripts/check-contracts.mjs --only=bot` + `--selftest=bot`（16/16） |
| 11 | P1 不新增事件 | ✅ | Bot 契约第 6 节（禁止 `bot-*` 事件） |
| 12 | CI 不再静默跳过 Bot e2e | ✅ | `.github/workflows/ci.yml:952` 改为 `--grep-invert "@heavy"`；`src/__tests__/ci.workflow.test.ts` 守卫；证据 `evidence/e2e-filter-before-after.md` |

---

## 3. e2e 选择器契约（前端改动时请保持）

`e2e/bots.spec.ts` 不依赖文案（11 语言），只依赖下列稳定钩子：

| 钩子 | 位置 |
| --- | --- |
| `[data-testid="bot-list-item"]` / `bot-create-btn` / `bot-edit-btn` / `bot-delete-btn` | `BotList` / `BotDetail` |
| `[data-testid="bot-soul-panel"]` / `bot-memory-panel` / `bot-threads-panel` | `BotDetail` |
| `#bot-editor-name` | `BotEditor`（表单输入框 id） |
| `.conv-item`（带 `data-conv-id`） | `ChatSidebar` 会话条目 |
| `[data-testid="new-conversation-btn"]` | `ChatSidebar` |
| `.ant-modal-footer .ant-btn-primary`、`.ant-popconfirm-buttons .ant-btn-primary` | Ant Design 的确定按钮 |

规则：
- 这些钩子是**对前端实现的最小契约**，重命名会让 e2e 以「元素不存在」明确失败，**不会静默跳过**（此前 `--grep-invert` 标题正则的教训）。
- 新增 Bot 用例**不要**打 `@heavy`；只有确实依赖真实 Tauri/长时间运行的用例才打，且必须加进 `src/__tests__/ci.workflow.test.ts` 的 `HEAVY_E2E_FILES` 名单。

---

## 4. 已知红项与责任线

### 4.1 Bot 相关（必须修）

**H 段：静默丢弃 Result +1（246 → 247）**

- 位置：`src-tauri/src/commands/bot.rs:471`
  ```rust
  let _ = bot_repo::set_soul_drift(db, &bot_id, snapshot.drift).await;
  ```
- 为什么红：`contracts` 的棘轮只减不增；丢弃的 `Result` 必须显式处理。
- 建议改法（二选一）：
  1. `if let Err(err) = ... { tracing::warn!(bot_id = %bot_id, error = %err, "回写 soul_drift 失败"); }`
  2. 若确认可丢，加注释说明并加入 `SILENT_RESULT_EXEMPT_CALLEES` 的合理豁免（需评审）。
- 复现：
  ```console
  $ node scripts/check-contracts.mjs --only=h
  ```

### 4.2 既有红项（非 Bot，另行处理）

- `node scripts/check-serde-annotations.ts` 在
  `axagent-core/crates/harness/src/vector_store.rs` 报 `EmbeddingRecord` /
  `VectorSearchResult` / `HnswConfig` 缺 `rename_all = "camelCase"`。
  该文件自 `a30824476` 起未改动，属存量问题；CI 也未接这道门。
- `node scripts/check-route-reachability.mjs --strict` 报 `pageRegistry` 的
  `wiki: "/wiki"` 无对应 `<Route>`。`npm run check:routes`（非 strict）通过，
  strict 是更严口径，已确认非 Bot 引入。

---

## 5. 文档与实现的已知差异

- **PLAN.md 写「42 个 i18n key」，但 T3 表格逐行相加是 46**（1+7+14+9+6+9）。
  门禁以**逐行列出的 key** 为准（46），避免为了凑数字漏检 4 个。
  已按文档问题上报，待架构师确认后统一。
- `src/types/index.ts` 的 `Conversation.botId` 已由前端补齐（T4 落点）。
- 浏览器 mock 的 `title`/`description` 上限已与 `bot.rs` 对齐（120 / 500 字符）。

---

## 6. 失败时怎么复现（交给责任人）

```console
# 只复现 Bot 契约
$ node scripts/check-contracts.mjs --only=bot

# 只复现 H 段棘轮
$ node scripts/check-contracts.mjs --only=h

# 只复现 Bot e2e（6 条）
$ npx playwright test e2e/bots.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000

# 复现 CI 同等 e2e（串行 + 重试）
$ CI=1 npx playwright test --project=chromium --grep-invert "@heavy" --timeout=120000
```

所有命令均在 WSL ext4 内执行：

```console
$ ssh -o BatchMode=yes wsl 'bash -s'
$ cd /home/polit/projects/AxInvest
```
