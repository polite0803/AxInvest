# ACCEPTANCE-P3 — P3 门禁验收实测台账

> 状态：active ｜ 日期：2026-11-24 ｜ 验收人：Bot 门禁与验证工程师 ｜ 基线：`8a7fbfbc0`（门禁执行时 HEAD）+ `21f9b8cdf`（e2e 收口提交）。
> 环境：`ssh -o BatchMode=yes wsl 'bash -s'` → `/home/polit/projects/AxInvest`（WSL ext4，不在 Z:/9P 构建）。
> 单条命令上限 1500s（Playwright 1800s）；未 push。所有读数为本轮实跑，不写「应已通过」。

## 0. 提交链与工作树归属

```
$ git log --oneline -14
8a7fbfbc0 feat(bots): P3 T5 IPC 审计、事件枚举与生成物对齐
5efe40097 feat(bots): P3 T4 编排定时命令与 cron 执行分支
d632f507f feat(bots): P3 T3 编排 runner 与多层委派链落地
2dbcce1bf feat(bots): P3 T2 编排命令层补齐 update_bot_workflow 与运行控制命令
767557bb1 fix(bot-workflow): align P3 frontend IPC and stabilize workflow page test
bb90d664e feat(bots): T2 P3 编排定义/DAG 校验与命令层（12 错误码 + 删部门转实例）
0b06e6c7d feat(bots): P3 编排页与 /bots/workflows 路由入口
958b35c76 feat(bots): P3 编排编辑器、运行面板与多层链视图
1088e54c1 feat(bots): P3 编排 DTO、DAG 预校验与 workflow slice
5f7263ea1 test(bots): P3 T7''' e2e 骨架落盘（可达 + 空态，不覆盖纵向场景）
15e9c80d9 feat(bots): T1 P3 编排实体与多层委派树列落库
f3e8abbe4 i18n(bots): P3 locale 12 错误码与 49 UI 键落齐 11 语言
59f8644c5 docs(bots): P3 多 Bot 协作/例程编排规格冻结（GAP/MODEL/IPC/PLAN/ADR/I18N + INDEX）
2e8db65d6 docs(bots): P2-b T7 验收台账与索引登记（15 项门禁实测）
```

T1''' ~ T5''' 与 T6''' 前端卡已由实现/前端线串行提交；T7''' 骨架为 `5f7263ea1`，纵向与多层链实测由门禁线提交 `21f9b8cdf` 补齐；T8''' 即本台账提交。后端线的 `update_bot_workflow` / T4 cron 分支在本轮开工时已落地为 `2dbcce1bf` / `5efe40097`，门禁线只核对实际命令与读数，未重复实现。

工作树仍有插件线/前端线/文档线在途脏文件（paper portfolio、screenshot diagnosis、layer allowlist、P3 规格文档等），全程未改、未提交。`docs/INDEX.md` 原有并行改动不纳入本次提交，只追加本台账两行。

## 1. P3 8 命令审计覆盖（IPC-P3 §2.4）

```
$ node --input-type=module - <<'JS'
import fs from "node:fs";
const spec = fs.readFileSync("docs/bot-integration/P3/IPC-P3.md", "utf8");
const audit = fs.readFileSync("scripts/lib/bot-ipc-audit.mjs", "utf8");
const doc = [...new Set([...spec.matchAll(/^\| `(create_bot_workflow\w*|update_bot_workflow|list_bot_workflows|list_bot_workflow_runs|delete_bot_workflow|run_bot_workflow|cancel_bot_workflow_run)` \|/gm)].map(m => m[1]))];
const block = audit.match(/const P3_COMMANDS = \[([\s\S]*?)\];/)[1];
const coded = [...new Set([...block.matchAll(/\[\s*"(\w+)"\s*,/g)].map(m => m[1]))];
console.log("doc=", JSON.stringify(doc));
console.log("audit=", JSON.stringify(coded));
console.log("missing=", JSON.stringify(doc.filter(x => !coded.includes(x))));
console.log("extra=", JSON.stringify(coded.filter(x => !doc.includes(x))));
JS
doc= ["create_bot_workflow","update_bot_workflow","list_bot_workflows","delete_bot_workflow","run_bot_workflow","list_bot_workflow_runs","cancel_bot_workflow_run","create_bot_workflow_schedule"]
audit= ["create_bot_workflow","update_bot_workflow","list_bot_workflows","delete_bot_workflow","run_bot_workflow","list_bot_workflow_runs","cancel_bot_workflow_run","create_bot_workflow_schedule"]
missing= []
extra= []
EXIT=0
```

审计脚本现已覆盖冻结稿全部 8 条新命令；门禁线补的 `root_id` / 工具 schema 不泄露隐藏字段 / `decorateWithP3` 正控与 4 条单点破坏自检，已随后端 T5 提交 `8a7fbfbc0` 落地，`botSelftest()` 读数 20 通过 / 0 失败。

## 2. e2e 原始输出（P3 纵向 + 多层链）

```
$ npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --timeout=90000 --reporter=list

Running 6 tests using 6 workers

  ✓  2 [chromium] › e2e/bot-workflow.spec.ts:65:3 › 骨架：编排页签可达且无编排空态保留 (9.2s)
  ✓  3 [chromium] › e2e/bot-delegation-chain.spec.ts:64:3 › 骨架：编排页签下委派链面板可达且空链不丢 (9.2s)
  ✓  1 [chromium] › e2e/bot-delegation-chain.spec.ts:58:3 › 骨架：/bots 可达且无 Bot 时 P1 空态保留 (9.7s)
  ✓  5 [chromium] › e2e/bot-workflow.spec.ts:59:3 › 骨架：/bots 可达且无 Bot 时 P1 空态保留 (9.7s)
  ✓  4 [chromium] › e2e/bot-delegation-chain.spec.ts:86:1 › P3 多层链：depth 1→2→3、root/parent 字段、L 前缀回执、cycle/maxDepth/busy 拒绝 (10.2s)
  ✓  6 [chromium] › e2e/bot-workflow.spec.ts:90:1 › P3 纵向：建编排 → 运行 → 步骤输出/回执 → 取消 → cron 触发登记 (11.7s)

  6 passed (13.1s)
EXIT=0
```

**边界（不做假绿）**：浏览器面没有 P3 命令的数据面，测试经 `page.addInitScript` 注入 `e2e/helpers/p3-tauri-shim.ts`，在 Tauri IPC 入口实现冻结 DTO 与 8 命令语义；真实运行时状态机（跳过判据、错误码继承、并发门、总超时、成员变更）由 `cargo test` 8+7 条测试锁定。e2e 证明「UI ↔ IPC 契约」链路与真实页面断言，不冒充 Rust 运行时证据。

## 3. T8''' 全量门禁逐项 PASS/FAIL 表

| # | 命令（原文） | Exit | 关键输出摘要 | 判定 |
|---|---|---:|---|---|
| 1 | `npm run typecheck` | 0 | `[check-tsc] tsc 发现 src 源文件 1599 个，开始 --noEmit 类型检查…`；无 error TS | PASS |
| 2 | `node scripts/vitest-run.mjs run` | 0 | `Test Files 326 passed (326)`；`Tests 3411 passed (3411)`；Duration 92.72s | PASS |
| 3 | `npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --timeout=90000 --reporter=list` | 0 | `6 passed (13.1s)`（详见 §2） | PASS |
| 4 | `npm run contracts` | 0 | `[BOT] Bot P1 IPC 契约：errors=0 warnings=0`；`结果: PASS` | PASS |
| 5 | `node scripts/check-contracts.mjs --selftest=bot` | 0 | `自检：20 通过 / 0 失败`；含 P3 缺命令/root_id/schema 泄露/事件变体 4 条单点破坏 | PASS |
| 6 | `node scripts/check-contracts.mjs --only=bot` | 0 | `errors=0 warnings=0`；`结果: PASS` | PASS |
| 7 | `npm run check:i18n` | 0 | 11 个 locale JSON 语法正常；`代码中 t() 调用唯一 key: 10490`，全部在 zh-CN 有定义 | PASS |
| 8 | `npm run check:i18n:key-parity` | 0 | 11 语言 × 16471 叶子路径一致；正控有效 | PASS |
| 9 | `npm run check:i18n:bots` | 0 | `语言: 11 必需 key: 151`；`结果: PASS（11 语言 × 全部必需 key 齐备）` | PASS |
| 10 | `npm run check:routes` | 0 | 路由 41 条、入口 34 个；2 条仅有页面内跳转（存量） | PASS |
| 11 | `npm run check:layers` | 0 | `✅ 无超基线违规`；`模式: ratchet ｜ 结论: ✅ 通过` | PASS |
| 12 | `npm run check:serde` | 1 | `axagent-core/crates/harness/src/vector_store.rs`：EmbeddingRecord / VectorSearchResult / HnswConfig 缺 camelCase 注解 | **存量红**（与 P1/P2-a/P2-b 台账一致，非 Bot 文件） |
| 13 | `node scripts/check-errorcode-alignment.mjs` | 0 | `后端 485 个错误码 ↔ 前端 11 语言 error 段对齐一致，无孤儿码` | PASS |
| 14 | `npm run test:rust` | 0 | 全 workspace 测试完成；doc-tests 4 ignored，无 failed | PASS |
| 15 | `cargo test --manifest-path src-tauri/Cargo.toml --test bot_delegation_chain --test bot_workflow` | 0 | 链 7 passed（depth/层内与全局预算/cycle/maxDepth/根完成子在途/run 取消整树/错误继承）；编排 7 passed（三步替换/并发/环/abort/continue/超时/成员变更） | PASS |
| 16 | `cargo test --manifest-path src-tauri/Cargo.toml --test bot_assignment --test bot_routine` | 0 | `bot_assignment` 4 passed；`bot_routine` 4 passed；0 failed（P2-b 回归组） | PASS |
| 17 | `cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate` | 0 | 4 passed（schema、consult、runner error、per-turn gate）；472 filtered | PASS |
| 18 | `npx playwright test --project=chromium --timeout=90000 --reporter=list` | 1 | `138 passed` / `3 skipped` / `3 failed`，红项均在 `e2e/stock-reflection-four-horizon.spec.ts` | **非 Bot 红**（见 §5） |

## 4. P2-b 四组回归零新增红

- `bot_assignment`：`cargo test --manifest-path src-tauri/Cargo.toml --test bot_assignment --test bot_routine` → 4+4 passed（§3 #16）。
- i18n：`npm run check:i18n` + `npm run check:i18n:key-parity` + `npm run check:i18n:bots` → 全 PASS（§3 #7–9），11 语言键集合齐备。
- 契约：`npm run contracts` + `node scripts/check-contracts.mjs --selftest=bot` → errors=0 / 自检 20/0（§3 #4–6）。
- `bot_routine` 与 `bot_delegate` 同轮绿（§3 #16–17）；P2-b 基线存量红 `check:serde` 未新增、未扩大。

## 5. 红项归属、复现与影响面

1. `npm run check:serde` → EXIT=1。复现：在 ext4 工作树直接跑该命令；输出固定指向 `axagent-core/crates/harness/src/vector_store.rs` 三个结构缺 camelCase 注解。归属：基础设施/搜索线，非 Bot 文件；P1/P2-a/P2-b 台账均登记为存量红。影响面：serde 注解门禁持续红，不阻塞 Bot IPC/契约/e2e 判定；不在本轮修他人实现。
2. `npx playwright test --project=chromium --timeout=90000 --reporter=list` → EXIT=1，3 红。复现：全量命令或 `npx playwright test e2e/stock-reflection-four-horizon.spec.ts --project=chromium --timeout=90000 --reporter=list`；均在 `openReflectionPanel()` 的 `page.locator(".ant-table-tbody tr.ant-table-row").first()` 等 20s 不可见（spec 第 186 行），测试点位于 241/269/302 行。归属：股票反思面板线（`e2e/stock-reflection-four-horizon.spec.ts` + stock-analysis 前端）；该文件与相关实现本轮未由我改动，`git status` 显示相关 store 为他人并行脏文件。影响面：全量 Playwright 从「全绿」降为 138/3/3；P3 Bot 两组 e2e 6/6 全绿不受影响，§10 ③ 的 P2-b 四组回归不含此 spec，故不阻塞 P3 收口，但需股票反思线领走。

## 6. PLAN-P3 §10 收口条件逐条签字

| 条件 | 判定 | 证据 | 签字 |
|---|---|---|---|
| ① T1'''–T8''' 全绿且提交链完整；多层链 e2e 覆盖 depth 1→2→3 / cycle / maxDepth / per-layer busy / 子树等待 / 整链取消 | PASS | §0 提交链 `59f8644c5`→`15e9c80d9`→`5f7263ea1`→`1088e54c1`→`958b35c76`/`0b06e6c7d`→`bb90d664e`→`2dbcce1bf`→`d632f507f`→`5efe40097`→`8a7fbfbc0`→`21f9b8cdf`→本台账；§2 链 e2e 6/6；§3 #15 链 7+编排 7 Rust 全绿 | Bot 门禁与验证工程师 ✅ |
| ② 一条 e2e 覆盖顺序/依赖/变量传递/单线程回执/失败跳过/取消/cron 触发 | PASS | `e2e/bot-workflow.spec.ts` 单测例：两步链 `dependsOn` + `{{collect.output}}`；步骤与 `[L2 ·` 回执同 root；failed + skipped 面板断言；终态取消错误码；`task_type="bot_workflow"` cron 登记。§2 `6 passed`；Rust #15 补运行时判据 | Bot 门禁与验证工程师 ✅ |
| ③ P2-b 既有四组回归（bot_assignment / bot_routine / i18n / 契约）零新增红 | PASS | §4 四组命令与读数；唯一新增非 Bot 红为 §5.2 股票反思 spec，不在四组内 | Bot 门禁与验证工程师 ✅ |
| ④ `ACCEPTANCE-P3.md` 记入 INDEX | PASS | 本文件落盘 `docs/bot-integration/P3/ACCEPTANCE-P3.md`；`docs/INDEX.md` 追加两表行（本提交） | Bot 门禁与验证工程师 ✅ |
| ⑤ P2-b 一层断言已按 ADR-P3-2「继承修订清单」改写，无遗留「一层不回归」表述 | PASS | `git grep -n '一层不回归\|assignee_round_tool_table_excludes_delegate' -- src src-tauri e2e docs/bot-integration/P2 scripts` 无输出；`src-tauri/src/bot_routine_runner.rs:350` 与 `src-tauri/tests/bot_delegation_chain.rs:159` 均为 `delegate_tool_respects_depth_cap`；本 Tc 17 Rust 4 passed | Bot 门禁与验证工程师 ✅ |
| ⑥ 未改实现代码之外的任何 P2 规格口径（若改动须先在 PLAN-P3 留修订记录） | PASS | `git diff --stat -- docs/bot-integration/P2/` 无输出；本轮仅改 e2e spec/helper、审计 fixture 与验收文档，P2 规格冻结稿未动 | Bot 门禁与验证工程师 ✅ |

**收口结论**：§10 六条全部 PASS，P3 可以收口。两项存量/他线红（§5）不阻塞 Bot 线，已定位责任线并留复现步骤；未 push。
