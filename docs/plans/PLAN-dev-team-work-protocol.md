# PLAN · 开发团队工作协议（2026-10-30 建团）

> 状态：active ｜ 日期：2026-11-03 ｜ 对应版本：v3.0.0+（未定版，待确认）
> 建团背景：项目已有 47 个 workspace crate、268 个命令文件、164 份 docs、175+ 条记忆判据。
> 规模已经不是「一个人能靠上下文记住」的量级 ⇒ 需要显式分工与验证契约。

---

## 0. 一句话

> **分工按「谁能独立验证自己」划，而不是按「目录是谁的」划。**
> 本会话存在一条硬约束：**成员无法执行 shell**（见 §1），因此**验证权集中在 Lead**。
> 协议的全部设计都围绕这一点：成员产出「可验证的改动 + 精确的验收判据」，Lead 负责实跑。

---

## 1. 环境事实（建团时实测，非推测）

| 事实 | 证据 | 影响 |
|---|---|---|
| 成员子会话 **无法运行 shell** | 一律 `[exit code: 3221225794]`（= 0xC0000142 `STATUS_DLL_INIT_FAILED`）；申请 `danger-full-access` 被系统拒绝且**子会话 escalation 在启动时固定、无法内部放宽** | 成员只能静态阅读 + 写入；**所有验证归 Lead** |
| Lead 需 `danger-full-access` 才能跑 shell | 默认沙箱注入导致同样 0xC0000142；提权后正常 | Lead 每次调 `pwsh` 必须显式带该参数 |
| 本机**无 bash** | `bash --version` → 未识别 | CI 里所有 `bash scripts/*.sh` 本地跑不了；用 `.mjs` 等价物替代（已验证 `check-line-endings.mjs` 可用） |
| `git` 命令**全禁**（含只读 status/log/diff） | 本仓第一铁律（`.workbuddy/memory/MEMORY-RULES.md` 纪律 1） | 改动清单自己记；回滚用 `cp` 到 `output/backup-<日期>/` |
| Windows 跑 Rust 测试需 `__TAURI_WORKSPACE__=true` | 否则测试 exe 未嵌 Common Controls v6 manifest，启动即 `0xC0000139` | 所有 Rust 测试命令必须带 |

---

## 2. 团队分工与写入范围

| 成员 | 领域 | 写入范围 | 不碰 |
|---|---|---|---|
| **Lead** | 验证、裁决、跨域协调 | 全部（但优先只做裁决与验证） | — |
| **backend-core** | 基座：命令层 / 分层 / crate / 迁移 / 门禁脚本 | `src-tauri/src/**`、`src-tauri/crates/{harness,dao,entities,storage,migration,runtime}/**`、`AGENTS.md`、`scripts/ci-check.mjs` | 股票业务引擎、`src/**` |
| **engine-stock** | 股票引擎：决策链 / 回放 / 反射 / 校准 | `src-tauri/crates/{analysis-engine,astock-data,quant,market-sim}/**`、`src-tauri/src/commands/{stock_*,portfolio-mgr*}`、`*.rhai`、`horizon_tier_template.rs` | 通用基座、`src/**` |
| **frontend** | 前端：页面 / 路由 / i18n / store | `src/**`、`scripts/check-route-reachability.mjs` | `src-tauri/**` |
| **gatekeeper** | 门禁与验证基础设施 | `scripts/check-*.mjs`、`scripts/ci-check.mjs`、`.github/workflows/*.yml`、`CHECKS.md` | 业务代码 |

**写冲突规则**：
- 同一文件**同一时刻只允许一个成员写**。跨成员共享的文件（`package.json`、`ci.yml`、`AGENTS.md`）由任务 `write_scopes` 声明，Lead 派单时保证互斥。
- 同一文件多次 `edit` **必须串行**（记忆纪律 2/139：并行 edit 各自读旧快照，后写者覆盖先写者，实测导致「半改名状态」）。

---

## 3. 「我改好了」的定义（分三档）

> 这是本协议最核心的一条。**没有验收判据的完成声明不算完成。**

### 3.1 改前端（`src/**`）

```
① node scripts/check-line-endings.mjs          # 或 bash 版本（本机无 bash）
② npx dprint check --incremental=false
③ npx oxlint src --max-warnings=0              # ← 必须手动，ci-check 曾整段注释掉（已修）
④ npm run typecheck
⑤ npm run build                                # ← 必须手动，本地镜像原本没有
⑥ npm run test:run
若触碰 i18n 另加：
⑦ node scripts/check-i18n-key-parity.mjs
⑧ node scripts/check-i18n-untranslated.mjs
⑨ node scripts/check-i18n-placeholder-usage.mjs
⑩ node scripts/check-errorcode-alignment.mjs
```

### 3.2 改 Rust（`src-tauri/**`）

阶段内（每次改完，快）：
```
cargo clippy -p <crate> --all-targets --all-features -- -D warnings
```
阶段收口（提交前，慢，10–40 min）：
```
cargo fmt --all --check                       # 必须 --all（不带只覆盖根 package）
cargo clippy --workspace --all-targets --all-features -- -D warnings   # 必须 --workspace
cargo check --workspace
__TAURI_WORKSPACE__=true cargo test --workspace
```
改 `*.rhai` 额外硬要求：**必须升 `TEMPLATE_VERSION`**（`seed_stock_analysis.rs`），否则存量库照跑旧脚本，改了等于没改（判据 166/171）。

### 3.3 改文档（`docs/**`、`AGENTS.md`）

```
node scripts/check-single-source-facts.mjs     # 行号引用腐烂（注意历史 274 条误报，只看新增 BROKEN）
node scripts/check-decl-evidence.mjs           # 软判据，仅看有无新增
```
`AGENTS.md` 额外铁律：**禁止在中段插入行**（该文件被记忆机制按行号广泛引用，中段插入会让其后每一行整体移位，实测一次插 14 行导致 B 栏 +4 / C 栏 +14 漂移）。新增内容一律追加到文件末尾。

---

## 4. 门禁基线（2026-10-30 实跑，全部 EXIT=0）

> 这是团队开工的**零点**。谁改坏了，就对照这张表找差异。

| 门禁 | 结果 | 备注 |
|---|---|---|
| `dprint check --incremental=false` | ✅ 0 | |
| `npx oxlint src --max-warnings=0` | ✅ 0 | 1421 文件 / 96 规则 / 87ms |
| `check-i18n-key-parity.mjs` | ✅ 0 | 11 语言 16271 条路径一致，**正控有效** |
| `check-i18n-untranslated.mjs` | ✅ 0 | 豁免 7 项 |
| `check-i18n-placeholder-usage.mjs` | ✅ 0 | R2/R3 有豁免清单 |
| `check-line-endings.mjs` | ✅ 0 | |
| `check-layer-discipline.mjs` | ✅ 0 | ratchet 模式 |
| `check-domain-single-source.mjs` | ✅ 0 | 扫 17 文件 / 10 站点 |
| `check-orphan-modules.mjs` | ✅ 0 | 1983 个 .rs；**基线有 11 条既存未处置** |
| `check-errorcode-alignment.mjs` | ✅ 0 | 448 个错误码 ↔ 11 语言对齐 |
| `check-route-reachability.mjs --selftest` | ✅ 0 | 7 条 PASS |
| `check-route-reachability.mjs --strict` | ✅ 0 | ⚠ **但判据过宽**（见 §5.1），绿不代表真可达 |
| `ci-check.mjs --quick` | ✅ 0 | 112.9s，含 oxlint（修复后） |
| `check-single-source-facts.mjs` | ✅ BROKEN **0** | LOCATED 582 / NONLOC 9（未检查）/ EXTERNAL 43。exit 3 由 NONLOC 展示触发，**不是腐烂** |

**另：`[lints] workspace = true` 的真实继承面（实测，两轮交叉核对一致）**

| 分类 | 数量 | 明细 |
|---|---|---|
| 真继承 | **7** | `plugins` · `plugin-proto` · `rt-messaging` · `rt-theme` · `rt-webhook` · `runtime` · `telemetry` |
| **假继承** | **3** | `notification`（`workspace = true` 被 `#` 注释）· `rt-workflow`（空表）· `runtime-core`（空表） |
| 无 `[lints]` 段 | 31 | 含 `harness` / `dao` / `entities` / `agent` / `gateway` 等热点 |
| 合计 | 41 | |

> ⚠ **判据形态**：`grep '\[lints\]'` 会数出 10，但正确判据是「**`[lints]` 段下存在 `workspace = true` 这个键**」。
> 这条差点被写错进 AGENTS.md（Lead 首轮用正则统计得 10/31，成员读原文纠正为 7/34，Lead 独立复算后确认成员正确）。
> ⇒ **本仓的「数字判据」必须剥注释后按结构取，不能按文本出现次数取。**

---

## 5. 已确认的「假绿」与「过宽判据」（不得当作验收证据）

### 5.1 `check-route-reachability.mjs` 的父路径吞子树（**实测坐实**）

```js
// :303
if (route.startsWith(entry + "/") && !route.includes(":")) { return true; }
```
`/chat` 是侧栏首项 ⇒ **所有 `/chat/*` 恒绿**。实测：
```
isReachable('/chat/workflow',    new Set(['/chat']))    → true   ← 应为 false
isReachable('/invest/screener',  new Set(['/invest']))  → true   ← 应为 false
isReachable('/knowledge/memory', new Set(['/knowledge'])) → true ← 应为 false
isReachable('/chat/workflow',    new Set())             → false  ← 正对照有效
isReachable('/opc/:tab',         new Set(['/opc']))     → true   ← 设计意图，须保留
```
且 `--selftest`（`:454-476`）**只测 `parseRegistryAndDeadPaths`，完全没测** `parseRoutes` / `parseNavEntries` / `isReachable`。
**另**：该门**不在 CI 也不在 ci-check**（`.github/workflows/*.yml` 零命中）⇒ 当前无任何流水线执行它。

### 5.2 `ci-check.mjs` 曾有三处假绿（**已修**）

| 位置 | 原缺陷 | 现状 |
|---|---|---|
| `:589` | clippy 缺 `--workspace` ⇒ 只 lint 根 package，31+ crate 不检查 | 已补 `--workspace` |
| `:608` | `cargo test ... \|\| cargo test ...` ⇒ 前半失败被后半成功掩盖 | 已删兜底 + 补 `__TAURI_WORKSPACE__` |
| `:384` | oxlint 整段注释 ⇒ 本地**零 lint** | 已恢复（用 oxlint 非 eslint） |

**但 `ci-check` 仍不等于 CI**（尚缺：`npm run build` / `contracts` / 行尾 / nextest 全量 / PG / E2E / audit / deny）。**它绿 ≠ 可提交。**

### 5.3 软判据清单（跑但不会红 ⇒ 别当验收）

- `check-decl-evidence --ci`、`check-single-source-facts --ci` 的软命中通道（`--ci` 把 exit 3 降成 0）
- `check-sql-dialect` B 段
- `check-id-validation` 的 38 条 WARNING
- `check-cron-job-authority` R2–R4（无 PG 时恒「未检查」，CI 不带 `--require-live`）

### 5.4 禁区 13 的唯一执行者没进流水线

`scripts/check-serde-annotations.ts`（规则：有 `Serialize/Deserialize` 但缺 `rename_all = "camelCase"`）只挂在 `package.json:86` 的 `check:serde`，**不在 CI 也不在 ci-check**。同类：`check-paired-fields.ts`、`type-consistency.mjs`。

---

## 6. 待裁决项（成员已发现，Lead 不擅自决定）

| # | 事项 | 发现者 | 为何必须裁决 |
|---|---|---|---|
| D-1 | **主链 R-212 是否受 `gate.valuation_enabled` 管辖** | engine-stock | `portfolio-mgr.rhai:3430` 无条件封顶，而长档 `portfolio-mgr-h-long.rhai:36-37` 用同一对阈值（−35% / 80 分位）且受开关控制 ⇒ **开开关对长档实际无效**。但 R-212 来历是用户对央企新材料案例的实证拍板（`:3396-3411`），与 v188 三门目的不同，谁盖谁须用户定 |
| D-2 | `astock-data` 的 crate 角色 | backend-core | 依赖形态像 consumer，但被 implementor（analysis-engine）依赖 ⇒ 判 consumer 会违「implementor 不得依赖 consumer」。**Lead 已定 implementor**，此处备案 |
| D-3 | 命令模块横向互调豁免（`background_tasks` 5 个命令 / `analyst_feedback` 6 个命令） | backend-core | 豁免理由写「不承载业务」与事实不符。两条路：(a) 抽 service 层（1.5–2 人日）(b) 改判据措辞（0.3 人日） |
| D-4 | `portfolio_backtest_run` 是无实现桩 + 前端零调用 | engine-stock | 删（纯收益）还是补实现 |
| D-5 | `BacktestResult` 同名冲突的深路径例外 | frontend | 改名 vs 登记白名单；当前例外只写在注释里，无机器在看 |
| D-6 | `platforms/*.rs` 8 个 stub 删除 | backend-core | 宿主注释已预授权，但删除动作需授权（且须用 Node 删，非 `rm -rf`） |

---

## 7. 协作纪律（从本仓历史教训提炼，违反过一次的都标了日期）

1. **禁止 `git`** —— 含只读。改动清单自记，回滚 `cp` 到 `output/backup-<日期>/`。
2. **同文件 `edit` 串行**；每轮 edit 后回读被改的那行（2026-09-15 实测：import 改名被同名 edit 覆盖回旧值，文件进入半改名状态，运行报 not defined）。
3. **「编译通过」有七型假阳性 ⇒ 修完必全量重跑**（判据 #72/#633）。`cargo check` + `cargo test` 全绿**不跑** clippy 规则。
4. **改生成物 = 假修复**（`register_commands.rs` 由 `build.rs` 生成，手改会被下次构建静默覆盖）。
5. **报告不擅改**：需产品语义决策的缺陷只登记 + 单列告知；纯 bug 可顺手修但须单列。
6. **「写进产物」≠「有人消费」**：判字段/函数是否有用，必须 grep 两跳。
7. **「0 命中」前先做正对照**：用一个已知存在的引用验证 grep 方法本身有效。
8. **不要盲跑 `--update-baseline`**（判据 #281：全量重写会锁死并发中间态，且可能覆盖手工调整）。
9. **不要为过门禁改语义**：先 grep 既有先例照抄形态（例：`std::sync::Mutex` 换 `tokio::sync::Mutex` 属改语义，测试替身不需要）。

---

## 8. 已知的「文档滞后于代码」清单（本仓高频模式）

> 本仓反复出现「做完了没回写」与「声明了机制但没人接线」，两者方向相反但后果一样：**读者被骗**。

| 文档 | 声称 | 实际 |
|---|---|---|
| `PLAN-frontend-information-architecture.md:3` | 「方案定稿，待实施（2026-10-28）」 | 正文 `:195-419` 已记录阶段 1–5 全部落地 ⇒ **头部状态行是假的** |
| `AUDIT-gates-and-headless-scheduler-2026-10-26.md` | `TEMPLATE_VERSION` = 188 | 实际 **193**（v189–v193 五个版本全部未回写） |
| 同上 §7 限制 5 | 「Windows 服务化未做」 | `scripts/install-scheduler-task.ps1` 已完整实现 |
| `PLAN-r3h-long-sample-calibration.md:84` | 「放宽到至少一源」 | `time_travel_batch.rs:487-499` 已实证推翻该前提 |
| `PLAN-url-as-truth.md` | §1 事实基线逐条核对 | `redirectToChat` 已被删除（`ContentArea.tsx:181` 留墓碑），`:305` 行号已漂移到 `:327` |
| `CHECKS.md` | 「12 步全部通过 = 可发版」 | 4 条口径不符、1 条（eslint）必崩、漏 ~45 条真实门禁 |
| `AGENTS.md:342` | implementor 含 `scanner` | `crates/scanner` **不存在** |
| `AGENTS.md:47/157` | 36 个 crate（+schema-gen = 37） | 实际 **41**（+schema-gen = 42） |
| `.workbuddy/memory/MEMORY.md:46` | 「进行中 = 声明式 schema 同步 §十一·八⑧」 | 该节已标 ✅ 全部落地（2026-09-16） |

---

## 9. 本协议的局限（如实登记）

1. **§4 基线是在本机跑的，不是 CI 跑的。** CI 有真 PG service、E2E（macOS）、cargo-audit / deny，本地全无。⇒ 基线绿**不能推出** CI 绿。
2. **`cargo clippy --workspace --all-targets --all-features` 的全量结果尚未取得**（冷缓存 10–40 min，建团时只跑到分包级）。§3.2 的「阶段收口」一档**未被本协议自身验证过**。
3. **无 Windows CI runner**（4 个 job 分别 ubuntu / macos）⇒ Windows 专属链接问题（0xC0000139 一族）在 CI **结构性不可见**。
4. **成员无法自验**是本协议最大的结构性弱点。它靠「Lead 实跑 + 成员给精确判据」补偿，但**如果 Lead 成为瓶颈，吞吐会受限于我一个人**。若后续能解决成员 shell 权限，§3 应改为成员自验。

---

## 10. 首轮执行结果（2026-10-30 建团当日）

### 10.1 已交付并**经 Lead 实跑验收**

| # | 交付 | 验收结果 |
|---|---|---|
| 1 | `AGENTS.md` 事实订正（删 `scanner` / 删 `rt-dashboard` 残留 / 成员数 36→41 / 补登 5 个 crate 角色 / 登记 lint 继承缺口） | ✅ `scanner`=0、`rt-dashboard`=1（仅摘除声明）、41/42 已写入、**401 行且 :385 未漂移**、`check-single-source-facts` **BROKEN=0** |
| 2 | `scripts/ci-check.mjs` 修三处假绿（clippy 补 `--workspace`、test 删 `\|\|` 兜底 + 补 `__TAURI_WORKSPACE__`、oxlint 解注释） | ✅ `node --check` 通过；`--quick` **EXIT=0（112.9s）**，输出中**已出现 `OxLint 检查 ✓`** |

### 10.2 `check-route-reachability.mjs`：一条**根因级**解析缺陷（已修，有负对照）

成员交付的收紧判据本身是对的（`--selftest` 绿、`--strict` 能红）。但 `--strict` 报出的 3 条孤儿键里
有 2 条是我读源码判定为**误报**的。追根因后发现问题**不在收紧判据，而在解析器的注释剥离**：

```js
// 修复前（scripts/check-route-reachability.mjs 原 :54-58）
.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "));   // 块注释
```

`ContentArea.tsx:383` 的 `` path={`${BUILTIN_PAGE_PATH.settings}/*`} `` —— **splat 通配 `/*` 落在模板串内**，
被块注释正则当作起点，配到文件后面最近的「块注释结束符」，**吞掉其间全部内容**：

| 后果（实测） | 证据 |
|---|---|
| `settings` 的模板串被截断（闭合反引号被吃掉）⇒ 该 Route 解析不出 | `parseRoutes` 无 `/settings/*` |
| `llmWiki` 整段落在被吞区间内 ⇒ 4 条 llm-wiki 路由**全部未被解析** | `parseRoutes` 无 `/llm-wiki*` |
| ⇒ **路由数静默少 5 条**（47 而非 52） | 既不被查可达性，也不会报孤儿 |
| ⇒ `/settings`、`/llm-wiki` 被误报成孤儿键 | 与 `--strict` 输出逐字一致 |
| ⇒ **`/llm-wiki/:wikiId/ingest` 这条真缺陷因从未进 `routes` 而隐身** | 修复后才现形 |

**这不是新 bug，是第二次同族复发**：`:47-53` 的注释记录了 2026-10-28 的同一形态
（`navRegistry.tsx` 行注释里写了 `/opc/*`），当时的修法是**调换两个正则的顺序** ——
那个修法管得住「注释里的 `/*`」，管不住「模板串里的 `/*`」。

**修法**：把 `stripComments` 改成**逐字符状态机**（跟踪 code / 行注释 / 块注释 / `"` / `'` /
模板串），非注释区间原样复制、注释字符替换为空格以保住行号。模板串刻意不做 `${}` 嵌套解析 ——
把它整段当字面量更安全（模板串里的 `//` 与 `/*` 本来就只是文本）。

**验收（Lead 实跑）**：

| 判据 | 修前 | 修后 |
|---|---|---|
| `parseRoutes` 路由数 | 47 | **52** |
| `--strict` 孤儿键 | 3（`settings` / `wiki` / `llmWiki`） | **1（仅 `wiki`）** |
| `--strict` 无入口路由 | 0（真缺陷隐身） | **1（`/llm-wiki/:wikiId/ingest`）** ← 真缺陷现形 |
| `--selftest` | EXIT=0 | EXIT=0（新增 5 条断言） |

**新增 5 条回归断言，且用负对照证明了区分力 = 5/5**（旧实现在同一批断言上全 FAIL、
误报孤儿键恰为 `["settings","llmWiki"]`、解析出 0 条路由）。

> ⚠ **负对照抓到了一次「无效断言」**：第一版样本**没有**在末尾放块注释，于是旧正则
> 匹配不到结束符、什么都不做 ⇒ 5 条断言在两种实现下都 PASS（区分力 0/5，等于没写）。
> 这条教训值得记：**断言「修复后会过」不足以证明它有效，必须构造能复现缺陷的样本**。

### 10.3 修复后 `--strict` 剩下的 2 条：**均为真缺陷**（需产品裁决，见 §11 D-3 / D-9）

| 项 | 定性 | 证据 |
|---|---|---|
| `/llm-wiki/:wikiId/ingest` **无任何入口** | ✅ 真缺陷（此前隐身） | `ContentArea.tsx:399-406` 有路由，渲染 `SafeLazyPage Page={LazyIngestPage}`；但全仓 `navigate`/`to`/`href` 对 `/ingest` **零命中** ⇒ 该页面**没有任何可达路径** |
| `wiki: "/wiki"` **孤儿键** | ✅ 真缺陷 | `pageRegistry.ts:73` 有键、ContentArea 零 Route；`e2e/wiki.spec.ts:5` 的 `page.goto("/wiki")` 必 404 |
| `/llm-wiki/:wikiId/graph`、`…/edit/:noteId` | △ 仅页面内跳转（弱入口） | 有入口但仅来自页面内 `navigate`，导航与命令面板都到不了 —— 信息项，非失败 |

### 10.3b ⚠ 由此暴露的**门禁盲区**（本仓应记的通用教训）

`parseRoutes` 的解析盲区会让路由**彻底隐身**（既不报不可达、也不报孤儿）。
本次是「注释剥离吞掉模板串」触发的；**同一机制还可能有别的触发形态**。
⇒ 判据：**任何「从源码抽取对象集合」的门禁，都必须回答「抽取面是否完整」** ——
否则它的「0 问题」与「没抽取到」不可区分（与判据 #206「扫描面 ≠ 影响面」同族）。
本次修复顺带把 `/llm-wiki/:wikiId/ingest` 从「从未被检查」变成「已判定不可达」，
**这才是修复的真实价值**，而不只是消掉两条误报。

### 10.4 ✅ 已结案：p5 探针「会说谎」的根因（**受控对照实验**）

先在**同一条命令内**做「确认空库 → 跑探针 → 立即计数」，消除了此前跨命令观测的口径漂移：

| | A：不加前置 | B：加 `--with-pg-prereqs` |
|---|---|---|
| 跑前表数 | **0** | **0** |
| probe 退出码 | 1 | **0** |
| `aborted` 判据 | **`[✗] 中止于：notes 执行 CREATE TABLE "notes" … 失败：function ax_cjk_ngram(text) does not exist`** | `[✓] 无中止` |
| 建表 | 57 / 192，**缺 135** | **192 / 192，缺 0** |
| 收敛 | `[✗] 残余 {"CREATE TABLE": 135}` | **`[✓] 零残余`** |
| 跑后表数 | 61 | 196 |
| 结论 | 不符合预期 | **符合预期** |

**根因（两件事，叠加）**：

1. **引擎不建 PG 函数，且建函数排在「建表」之后** —— `extras::FUNCTIONS`（含 `ax_cjk_ngram`）的
   唯一消费点是 `expected.rs:500` 的指纹登记，`ChangeKind` 21 类里**没有 `CreateFunction`**；
   而 `notes` / `memory_items` 等表的生成列**内联在 `CREATE TABLE`** 里引用该函数
   ⇒ 全新空库建到 `notes` 即失败，**fail-open 后 135 张被 `AbortedAfterError` 跳过**。
   这条 **`crates/dao/tests/pg_cjk_fts.rs:79-86` 早就写明**，只是探针没照做。
2. **探针从不检查 `out.aborted`** ⇒ 明明中止了，报告里仍打印「未被拒（refusal 为空）」，
   两者自相矛盾。`pg_cjk_fts.rs:93-96` 也早就写明必须断言它。

**已修（`crates/dao/examples/p5_engine_takeover.rs`）**：
- 新增 `--with-pg-prereqs`：建 `vector` 扩展 + `ax_cjk_ngram` 函数（两步都幂等，
  与 `pg_cjk_fts.rs:76-89` 共用同一份 `cjk_ngram::render_function_sql`，不手抄）；
- 新增判据 **`[✓/✗] 无执行中止（aborted 为空）`** —— 让「中止」再也不能伪装成「未被拒」。

> ⚠ **我上一轮的结论被自己推翻了，必须记清楚**：
> ① 我曾实测「未 abort、192 张全建成」据此**否定成员的「函数缺失致 abort」推断** —— 那次**测错了库**
> （跨命令、且有残余状态），成员是对的；
> ② 我又据「残余 ADD CHECK/ADD COLUMN/CREATE INDEX/SET DEFAULT」下结论「引擎不收敛」——
> 那批残余其实是**中止后残缺状态**的产物；加前置后 **192/192 + 零残余**，引擎本身正常。
>
> ⇒ 判据：**用「探针报出的红/绿」反推被测对象之前，必须先证明探针自己没说谎。**
> 本次的教训是「探针缺判据」与「被测对象有缺陷」长着同一张脸。

### 10.4b 由此暴露的真缺陷（待立项，不是探针问题）

引擎**无法从真空库独立建全**：`FUNCTIONS` 只进指纹不执行、`ChangeKind` 无 `CreateFunction`。
生产/CI 都靠外部先把函数建好（`pg_cjk_fts.rs` 手工建、本次 `--with-pg-prereqs` 手工建）。
⇒ 这是**架构级缺口**（补 `CreateFunction` 需动 plan/render/apply 与变更排序），
与 task-9 拆分一致：探针侧已修复（本轮），引擎侧待单独立项。

### 10.5 task-7（超短档 microstructure）：成员写完但**未验证**，Lead 接手修完并验收

**关键事实**：`engine-stock` 在失败前**已把 task-7 的代码全部写完**（5 个文件，12:36–12:38，
外加 `src/types/stock-analysis.ts` 的 TS 侧字段），但**从未跑过一次编译**。
⇒ 交付物是「写完但可能是坏的」——这正是本协议 §3「没有验收判据的完成声明不算完成」的实例。

Lead 接手后查出**两处编译/运行级缺陷**（都不是设计问题，是收尾遗漏）：

| # | 缺陷 | 后果 | 修法 |
|---|---|---|---|
| 1 | `seed_consistency_tests.rs:5452` 断言消息里写了裸 `{ pool_break_count }` | `format!` 把 `{` 当占位符 ⇒ **整个 `axagent` lib test 编译失败**（`invalid format string`） | 改成描述性措辞（避免在格式串里写裸花括号） |
| 2 | `.rhai` 消费端用**数组下标**取 map：`pool_row[0]` / `[1]` / `[2]` | `pm_pool_row_for` 返回的是 `#{ inPool / breakCount / limitUpStreak }`（**map**）⇒ Rhai 把 `0` 当 map 键、要求 string，运行时报 `Data type incorrect: i64 (expecting string)` | 改为 `.inPool` / `.breakCount` / `.limitUpStreak`，并在原处留下判据注释 |

> **缺陷 2 是被门禁抓住的，不是靠人眼**：`branch_node_scope_is_exactly_the_seed_mapping`
> 按「节点真实 scope」把 19 个键（含新映射 `pool_entries`）铺进 Rhai 引擎跑真脚本，
> 直接报出 `ultra_short 档按节点真实 scope 运行失败：Data type incorrect: i64`。
> ⇒ **这条门禁的价值在本轮被实证**：它拦住了「语法能过、生产脚本一跑就崩」的改动。

**验收结果（Lead 实跑）**：

| 判据 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ EXIT=0（含我改的探针文件；需逐文件 `rustfmt` 而非全量 `cargo fmt`，见记忆 #208） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib seed_consistency` | ✅ **55 passed / 0 failed**（首跑 54/55，唯一失败即上面的缺陷 2；修后全绿） |
| `TEMPLATE_VERSION` | ✅ = 194，注释首行明写「**⚠ P0-5 封顶未解除**」 |
| 保守方案是否被遵循 | ✅ `pm_cap_unwired_action`（`:161`）**一字未改**，仍无条件封顶 |
| 量纲修复是否落实 | ✅ `raw.breakCount` 取个股 `micro_breaks`；`pool_break_count` 退出 `raw`，仅作披露字段 `marketBreakCount` |
| 信号透出 | ✅ 新增 `microLegPresent` / `microBasis`（四态：`in_pool` / `not_in_pool` / `break_count_absent` / `*_missing`） |
| `cargo clippy -p axagent --all-targets --all-features -- -D warnings` | ✅ **EXIT=0**（12m35s，零告警） |

**仍未做（如实登记）**：`microstructure` 腿是否真进融合（`leg_signal` 是否返回 `Some`）、
以及「进涨停池 = 有开仓证据」的产品口径 —— 二者都待 D-2 裁决，本版只透出数据不改变动作。

### 10.6 未开工（已派单，因成员故障未执行）

| 任务 | 内容 |
|---|---|
| task-9 CI 接线 | 探针侧已修（§10.4），但**未接进 `ci.yml`**。接入前需先决定「是否在 CI 里新建独立空库」（`--fresh-pg` 要求空库，而现有 PG job 共用 `axagent_test`） |

### 10.6 ⚠ 建团当日出现的团队故障（必须记录）

**四个成员子会话在首轮后**陆续失败**（`frontend` / `backend-core` / `engine-stock` 的后续轮次均 `failed`，无收尾消息）。** 首轮盘点与首轮修复均成功，**第二轮起不可用**。

- 影响：task-5 的误报修复、task-7、task-9 均因此停摆（已派单未执行）。
- 这不是任务写得不好（首轮产出质量很高），而是**运行环境不稳定**。
- ⇒ **本协议的 §2 分工在成员不可用时退化为「Lead 独做」**，吞吐直接掉到单线程。选型时应把「成员可持续性」当作与前两条同级的约束。

---

## 11. 待用户裁决（成员发现的，Lead 不擅自决定）

| # | 事项 | 为何必须由用户定 |
|---|---|---|
| **D-1** | **主链 R-212 是否受 `gate.valuation_enabled` 管辖** | Lead 实测：`portfolio-mgr.rhai:3430` 的 `valuation_capped` **无条件**封顶；而长档 `portfolio-mgr-h-long.rhai:36-37` 用**同一对阈值**（−35% / PE 80 分位）且受开关控制 ⇒ **开开关对长档实际无效**。但 R-212 来历是用户对央企新材料案例的实证拍板（`:3396-3411`），与 v188 三门「避免 0 买入死循环」是两个决策 ⇒ 谁盖谁须用户定 |
| **D-2** | **P0-5 封顶是否解除**（即「进涨停池 = 有开仓证据」吗） | `portfolio-mgr-h-ultra-short.rhai:99-102` 原注释自陈「门通过只说明情绪+个股代理不否决，并不等于拿到微观结构开仓证据」。Lead 已裁定**暂不解除，仅透出信号**，需用户确认是否最终解除 |
| **D-3** | `/wiki` 孤儿键处置 | 删键 + 改 `e2e/wiki.spec.ts:5` 指向 `/knowledge/wiki`（成员倾向）vs 补 `/wiki → /knowledge/wiki` 重定向 |
| **D-4** | `portfolio_backtest_run` 是无实现桩 + 前端零调用 | 删（清注册）还是补实现 |
| **D-5** | `BacktestResult` 同名冲突的深路径例外 | 改名 vs 登记白名单（当前例外只写在注释里，无机器在看） |
| **D-6** | 命令模块横向互调豁免（`background_tasks` 5 命令 / `analyst_feedback` 6 命令） | 豁免理由写「不承载业务」与事实不符：(a) 抽 service 层（1.5–2 人日）(b) 改判据措辞（0.3 人日） |
| **D-7** | `platforms/*.rs` 8 个 stub 删除 | 宿主注释已预授权，但删除动作需授权（且须用 Node 删，非 `rm -rf`） |
| **D-8** | 6 条行级数据语义承接（v131/v137/v203/v225/v228/v230） | 「等价判据」的具体形态属测试设计决策 |
| **D-9** | `/llm-wiki/:wikiId/ingest` 无任何入口（`ContentArea.tsx:399-406`，渲染 `LazyIngestPage`） | 三选一：① 补入口（Wiki 页面内加「导入」按钮）② 删该路由 + 删 `LazyIngestPage`（若功能已废弃）③ 若属深链专用则显式登记为 internal 白名单。**修复前它连「不可达」都没被报出来**（解析盲区），故必须先决定它是否该存在 |
| **D-10** | `check-route-reachability` 是否接入 CI | 判据已修好，但它**至今不在 `ci.yml` 也不在 `ci-check.mjs`**（`package.json:105/106/142` 三条命令无任何 workflow 调用）⇒ 「判据修好了但没人跑」。接入需决定挂哪个 job + 是否带 `--strict` |
