# BASELINE-P4 — P4 启动前门禁基线读数

> 状态：已测（只记录，不修）｜日期：2026-11-25 ｜冻结提交：`cfaa35473`（分支 `feat/p3-bot-workflow-frontend`）｜执行：Bot 门禁与验证工程师
>
> 用途：P4 收口「P1–P3 回归零新增红」（PLAN-P4 §9.1 ⑥ / §9.3）的对照基线。命令原文取自 PLAN-P4 §9.2（与 IPC-P4 §7 同源），共 18 条。

## 1. 测量口径

- 执行通道：`ssh -o BatchMode=yes wsl 'bash -s'`，仓库在 WSL ext4，未在 `/mnt/z`（9P）上构建。
- 环境：Node v24.22.2 / npm 11.19.1 / cargo 1.99.1 / rustc 1.99.1；内存 13 GiB。
- 测量载体：**独立 git worktree** `/home/polit/projects/AxInvest-p4-baseline`（`git worktree add --detach … cfaa35473`），`node_modules` 软链到主检出，`CARGO_TARGET_DIR=/home/polit/projects/AxInvest/src-tauri/target` 复用主检出 88 GB 热缓存；`__TAURI_WORKSPACE__=true`（src-tauri `build.rs` 的测试构建要求）。
- 为什么不在主检出上测：测量期间主检出存在并行工作线（invest 重构 + 后端 P4 A1/A2 WIP）的未提交改动，且**测量中途仍在变化**（07:40 与 08:00 两次快照分别为 36 文件 +1631/−922、50 文件 +1806/−951）。在主检出上读到的绿，不能证明 `cfaa35473` 本身是绿。主检出对照读数见 §5。
- 原始日志：`/home/polit/p4-baseline-logs/clean/*.log`（WSL，未入仓库）；主检出首轮日志在 `/home/polit/p4-baseline-logs/*.log`。

## 2. 18 条命令逐条读数（clean worktree @ cfaa35473）

| # | 命令原文（PLAN-P4 §9.2） | exit | 耗时 | 关键输出摘要 |
|---|---|---|---|---|
| ① | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_` | 0 | 366 s | `12 passed; 0 failed; 402 filtered out`（含 bot_department / bot_assignment / bot_workflow_run） |
| ② | `cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance` | 0 | 360 s | `verify_all_entity_columns_exist_in_db … ok`，`1 passed` |
| ③ | `cargo test --manifest-path src-tauri/Cargo.toml --test bot_quota --test bot_input` | **101** | 0 s | `error: no test target named bot_quota`（目标尚未落盘，属预期） |
| ④ | `cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine --test bot_delegation_chain` | 0 | 450 s | bot_assignment 4 ✅ / bot_delegation_chain 7 ✅ / bot_routine 4 ✅ / bot_workflow 7 ✅，合计 22 passed |
| ⑤ | `cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate` | 0 | 461 s | `4 passed; 0 failed; 472 filtered out` |
| ⑥ | `npm run typecheck` | 0 | 4 s | `tsc 发现 src 源文件 1600 个，开始 --noEmit 类型检查…`（无错） |
| ⑦ | `node scripts/check-contracts.mjs --selftest=bot` | 0 | 0 s | `自检：20 通过 / 0 失败` |
| ⑧ | `node scripts/check-contracts.mjs` | **1** | 4 s | `[A] 已定义但未注册` 9 条（全部 `paper_portfolio_*`）；`[BOT] errors=0 warnings=0`；汇总 `errors=9` → FAIL |
| ⑨ | `node scripts/check-errorcode-alignment.mjs` | 0 | 0 s | `后端 485 个错误码 ↔ 前端 11 语言 error 段对齐一致，无孤儿码` |
| ⑩ | `node scripts/check-bot-i18n-keys.mjs` | 0 | 0 s | `语言: 11 必需 key: 151`，`PASS` |
| ⑪ | `npm run check:i18n` | 0 | 0 s | `t() 唯一 key 10490`；`zh-CN 唯一 key 17928`；`7441 个 key 未被代码引用`；全部通过 |
| ⑫ | `npm run check:i18n:key-parity` | 0 | 0 s | 11 语言各 16476 条，路径集合一致；正控有效 |
| ⑬ | `npm run check:routes` | 0 | 1 s | 路由 41 / 入口 34；⚠ `引用 pageRegistry 已删除键 1 处：BUILTIN_PAGE_PATH[botWorkflows]`（warning，不改变 exit） |
| ⑭ | `npm run check:layers` | 0 | 0 s | `模式: ratchet ｜ 结论: ✅ 通过` |
| ⑮ | `npm run check:serde` | **1** | 0 s | `axagent-core/crates/harness/src/vector_store.rs`：`EmbeddingRecord` / `VectorSearchResult` / `HnswConfig` 缺 camelCase（存量红，与 Bot 无关） |
| ⑯ | `node scripts/vitest-run.mjs run --grep-invert "@heavy"` | **1** | 0 s | `CACError: Unknown option --grepInvert` —— **冻结命令本身不可执行**（详见 §3.3） |
| ⑯ | 　　等价补测：`node scripts/vitest-run.mjs run` | **1** | 105 s | `Test Files 2 failed | 324 passed (326)`；`Tests 11 failed | 3402 passed (3413)`（明细见 §3.4） |
| ⑰ | `npx playwright test e2e/bot-quota.spec.ts e2e/bot-needs-input.spec.ts --project=chromium --grep-invert '@heavy' --timeout=120000 --reporter=list` | **1** | 2 s | `Error: No tests found.`（spec 尚未落盘，属预期） |
| ⑱ | `npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --grep-invert '@heavy' --timeout=120000 --reporter=list` | 0 | 21 s | `6 passed (16.9s)`（--workers=3；默认 6 workers 同样 6 passed） |

> 说明：③ 与 ⑰ 的 101/1 不是行为红，是「P4 产物尚未存在」。⑥–⑮ 的耗时是 worktree 冷启动下的观测值，主检出热缓存下更快。

## 3. 红项清单与归属

### 3.1 存量红（与 Bot 无关，P4 不得新增、不必修复）

| 项 | 读数 | 归属 |
|---|---|---|
| ⑮ `check:serde` | `vector_store.rs` 3 个结构体（EmbeddingRecord / VectorSearchResult / HnswConfig） | P1 前既有；P4 前后必须保持**恰好这一组** |
| ⑧ `check-contracts` | 9 条 `[A] 命令已定义但未注册`，全部为 `paper_portfolio_*` | invest 域（并行 invest 重构线）；同线未提交改动删除该模块后在主检出转绿 |

> ⑧ 首跑曾报 17 条（额外含 `screenshot_diagnosis_*` 8 条），同 worktree 复跑为 9 条且可稳定复现；以复现读数 9 为准。8 条差异对应并行线当刻正在改写的文件，按竞态记录，不作为独立红项。

### 3.2 Bot 域存量红（冻结 HEAD 上真实存在，修复不在本轮范围）

全量 vitest（⑯ 等价补测）的 11 条失败全部落在 Bot 前端既有用例：

| 文件 | 失败数 | 首因（原文） |
|---|---|---|
| `src/pages/__tests__/BotWorkflowsPage.test.tsx` | 4 | `AssertionError: expected undefined to be '/bots/workflows'` → `BUILTIN_PAGE_PATH.botWorkflows` 在 HEAD 的 `pageRegistry` 中缺失（⑬ 的 warning 同源） |
| `src/pages/__tests__/BotsPage.test.tsx` | 7 | `TypeError: listBotDepartments is not a function`（`BotsPage.tsx:112` 已调用，测试替身未提供） |

归因：两条都是**已提交代码与已提交测试之间的漂移**，与并行未提交改动无关（并行改动恰好包含这两处修复，见 §5）。P4 期间这两组用例必须保持红/绿形态**不因 P4 新增改动而变化**；若 P4 线顺手修复，须在 ACCEPTANCE-P4 注明「存量红转绿，非新增红」，不得悄悄改写基线。

### 3.3 冻结命令自身缺陷（需架构师裁定，不在本轮改 PLAN）

⑯ 的 `--grep-invert "@heavy"` 是 Playwright 语义，vitest CLI 不接受，任何工作树上都以 `CACError: Unknown option --grepInvert` 退出 1。仓库内 `@heavy` 只存在于 Playwright spec（`e2e/*.spec.ts` 的 `{ tag: "@heavy" }`），vitest 侧不存在该标签。
建议修订（待架构师写入 PLAN-P4 §9.2 后再采用）：`node scripts/vitest-run.mjs run`（全量即口径）。
在修订落笔前，本基线以「冻结原文 exit 1（不可执行）+ 等价补测读数」双记录，避免掩盖问题。

### 3.4 全量 vitest 失败明细（11 条）

```
FAIL src/pages/__tests__/BotWorkflowsPage.test.tsx (4 tests | 4 failed)
  × 入口：导航注册表与路由表都暴露 /bots/workflows（侧栏入口 + 独立路由）
  × 空态：无编排时显示空态与创建入口（不落空白页）
  × 空态：无 Bot 时创建入口禁用（无 Bot 空态不丢）
  × 关键流程：创建编排 → 运行 → steps 进度 → outputs 回执 → 委派链可见
FAIL src/pages/__tests__/BotsPage.test.tsx (7 tests | 7 failed)
  × 挂载时加载 Bot 列表与模型提供方
  × 没有选中 Bot 时自动选中第一个
  × 新建线程带 botId，并跳到 /chat?bot=<id>
  × 打开线程先设活动会话，再带 Bot 上下文进入对话页
  × 切换到另一个 Bot 时列表选中项与详情一起切换（props 注入）
  × 没有 Bot 时展示空状态，且不渲染详情
  × 新建线程失败时提示错误且不跳转
```

## 4. 「不新增红」对照阈值（P4 收口逐项比对）

| 命令 | P4 完成时的允许形态 |
|---|---|
| ① | exit 0；`bot_` 过滤用例数 ≥ 12，0 failed |
| ② | exit 0；1 passed |
| ③ | exit 0（骨架落盘后）；0 failed（`#[ignore]`/未点亮的 pending 不计红） |
| ④ | exit 0；22 passed（4+7+4+7），0 failed |
| ⑤ | exit 0；4 passed |
| ⑥–⑫、⑭ | exit 0；⑨ 码数 485 → 490（IPC-P4 §4 预期，以实测为准）；⑩ 必需 key 151 → 185 |
| ⑧ | **不得多于 9 条 `paper_portfolio_*`**；`[BOT]` 段必须 errors=0 |
| ⑬ | exit 0；`BUILTIN_PAGE_PATH[botWorkflows]` warning 允许保留（既有） |
| ⑮ | **恰好** vector_store.rs 的 3 个结构体，不新增、不换文件 |
| ⑯（等价全量） | 不得新增失败文件/用例；当前 2 文件 / 3411 用例中 11 红为上限 |
| ⑰ | exit 0；P4 两条 spec 全绿（fixme 未点亮项计 skipped，须在 ACCEPTANCE-P4 列出） |
| ⑱ | exit 0；6 passed |

## 5. 主检出（含并行未提交 WIP）对照读数 — 仅作参照

07:42–08:00 在主检出 `/home/polit/projects/AxInvest` 上按同一命令跑过一轮，与 clean HEAD 的差异全部由并行 WIP 引起，**不能替代基线**：

| 命令 | clean HEAD | 主检出（WIP） | 差异原因 |
|---|---|---|---|
| ⑧ contracts | exit 1（9 条 paper_portfolio） | exit 0 | WIP 删除 `commands/paper_portfolio.rs` |
| ⑯ 全量 vitest | 11 failed / 3402 passed / 3413 | 0 failed / 3411 passed / 326 文件 | WIP 修正 pageRegistry 与 BotsPage 测试替身 |
| ⑥ typecheck | 1600 文件 | 1599 文件 | WIP 删/增前端文件 |
| ⑪ i18n | 7441 未引用 key / 16476 条 | 7438 / 16471 | WIP 改 locale |
| ⑬ routes | 0（1 warning botWorkflows） | 0（无该 warning） | WIP 补 pageRegistry |
| ①/②/④/⑤ | 全绿（12/1/22/4） | 全绿（12/1/22/4） | 一致 |

结论：主检出上的「全绿」是并行 WIP 的读数，不是 `cfaa35473` 的读数；后续 P4 收口比对**必须**以本文件 §2/§4 的 clean HEAD 读数为基线，并在 ACCEPTANCE-P4 说明测量树。

## 6. 结论与交接

- 18 条命令在冻结 HEAD 上：**13 条绿**，5 条红（③⑧⑮⑯⑰），其中 ③⑰ 是 P4 产物缺失（本轮骨架落盘后应转绿），⑧⑮ 是非 Bot 存量红，⑯ 是冻结命令缺陷 + 11 条 Bot 前端存量红。
- 本轮随之落盘（独立提交）：`src-tauri/tests/bot_quota.rs`、`src-tauri/tests/bot_input.rs`（可编译，pending 场景以 `#[cfg(any())]` 屏蔽，点亮方式写在文件头）、`e2e/helpers/p3-tauri-shim.ts` 扩展（6 命令 + `bot-input-request-updated`）、`e2e/bot-quota.spec.ts`、`e2e/bot-needs-input.spec.ts`（场景 `test.fixme`，待 A4/B4 UI 落地点亮）。落盘后 ③ 预期 exit 0、⑰ 预期 exit 0（skipped），其余读数不得变化。
- 待实现转绿的断言清单见两份 Rust 骨架文件头与 e2e spec 头注释；未点亮项一律在 `ACCEPTANCE-P4.md` 显式列出，不冒充已覆盖。

## 7. 骨架落盘后复测（同日，主检出）

落盘内容（本轮，独立提交于本文件之后）：

- `src-tauri/tests/bot_input.rs`：骨架落盘时 B1 未提交（`#[cfg(any())]` 屏蔽）；B1 提交 `a6b4d11ff` 落盘后**当场点亮**，现为 8 条 DAO 生命周期用例（双路查询 / `UNIQUE(ask_id)` / 只从 pending 作答 / 终态不可回退 / 清扫 / 过滤排序 / 待答计数 / 预留列恒 NULL）；B2·B3 归属的场景写在文件头归属表；
- 追加 `src-tauri/tests/bot_quota.rs` 2 个用例（A2 原有 7 个用例由后端随 A2 落盘，本轮只补齐 A5 缺口）；
- 扩展 `e2e/helpers/p3-tauri-shim.ts`：6 条 P4 命令 + `bot-input-request-updated` + `agent_respond_ask` P4 分支 + 配额只收紧；
- 新增 `e2e/bot-quota.spec.ts`、`e2e/bot-needs-input.spec.ts`：契约自检 16 条可跑，A4/B4 页面证据 9 条 `test.fixme`。

| # | 命令原文 | exit | 关键输出 |
|---|---|---|---|
| ③ | `__TAURI_WORKSPACE__=true cargo test --manifest-path src-tauri/Cargo.toml --test bot_quota --test bot_input` | 0 | `bot_input`: `8 passed`（1 条 300s 冻结守卫 + 7 条 DAO 生命周期）；`bot_quota`: `9 passed`（后端 A2 的 7 条 + 本轮 2 条：`step_timeout_violation_names_the_offending_step`、`cron_gate_rejects_before_run_creation_across_workflows`），`0 failed` |
| ⑰ | `npx playwright test e2e/bot-quota.spec.ts e2e/bot-needs-input.spec.ts --project=chromium --grep-invert '@heavy' --timeout=120000 --reporter=list` | 0 | `16 passed; 9 skipped`（skipped 全部是 `test.fixme`：A4 面板 3 条、B4 卡片/徽章 6 条） |
| ⑥ | `npm run typecheck` | 0 | 无错（`src` 1599 文件） |

结论：③⑰ 从「P4 产物缺失」转为绿，§4 阈值表对应两行的允许形态已达成；⑧⑮⑯ 的存量红未因本轮改动变化（本轮只动 `e2e/`、`src-tauri/tests/`、`docs/`）。

过程中的一次真实红（记录以备复现）：两 spec 首跑 14 红，根因都在**替身自身**而非产品：
`page.addInitScript` 只序列化函数源码，模块级常量在页面上下文未定义（`P4_QUOTA_DEFAULTS is not defined`）；
事件 payload 直接 emit 了可变行引用，测试末尾读到的历史事件全被后续状态覆盖。
两者修复后才绿——说明这批契约自检确实能拦住替身与冻结契约不符，而不是写了就算通过。

尚未有证据的项（不得计入已覆盖）：
- A5「cron 触发超限 → history 失败」只有**门禁之前**的可观测部分（`cron_gate_rejects_before_run_creation_across_workflows`）；history 记 `failed` 走 `init/services.rs` 调度器闭包内的既有错误路径，需后端抽出 seam 或端到端启动调度器才能断言。
- B5 ①③⑤ 与 ⑥ 的「无活 sender」分支分别依赖 B2 交互桥与 B3 命令层，待其落盘后由各自测试锁定，`bot_input.rs` 文件头已列归属表。
- 复测窗口内后端 B2 正在落盘（`src-tauri/src/bot_input_request.rs` 等为未提交改动），首次 `cargo test --test bot_input` 因 `IpcEventName::BotInputRequestUpdated` 尚未加入枚举而 `exit 101`（后端 WIP 的瞬时红，非本轮产物）；待其枚举补齐后原样重跑即 `exit 0`。此类并行线瞬时红不作为基线。

