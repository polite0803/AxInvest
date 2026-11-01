# PLAN: 域包「金融投资」与投顾经营语义收编

日期：2026-09-28　版本基线：2.9.9　状态：**待批**

## 0. 缘起

侧栏「金融域」下出现两个都叫「金融投资」的入口，且第二个页面对外宣称的四项能力
（个股分析 / 财报解读 / 估值计算 / 风险评估）全部与投资中心 `/invest`（InvestHub）重合。
本计划先经实测确认重合的性质，再据此收编名目、拆除重复执行入口。

## 1. 已实证结论（含证据）

| # | 结论 | 证据 |
|---|---|---|
| 1 | 两项导航同名：`/invest` 用 `nav.financeInvestment`「金融投资」，`/opc/domain/finance-invest` 用 `opc.domains.finance_invest`「金融投资」，图标同为 Dollar 系 | `src/lib/navRegistry.tsx:62-76`、`src/i18n/locales/zh-CN.json:3834,11511`、`src/components/layout/AppHeader.tsx:25,28` |
| 2 | 同一页面四个名字：侧栏「金融投资」/ 页面大标题「投资中心」/ 顶栏「金融投资」/ nav key `finance-investment` | `src/components/invest/InvestHub.tsx:173`、`zh-CN.json:11365` |
| 3 | 域包本尊是 CFO 视角的经营财务，不是股票分析：`FinancialReport` 由 `opc_invoices / opc_projects / opc_customers` 聚合，字段含 `investable_amount` | `config/opc/domain_packs/finance_invest/manifest.yaml:6`、`src-tauri/crates/analysis-engine/src/opc/finance.rs:9,17-28,97` |
| 4 | 「估值计算 / 风险评估」两个动作执行的是同一条 OPC 经营链，与股票财务无关 | `src/pages/opc/domains/DomainPages.tsx:718,741`、`src-tauri/src/commands/opc_workflows/seed_capability_pack_finance_invest.rs:59,96` |
| 5 | 「个股分析」是**重复执行入口**：`template_id: "stock-analysis"` 就是投资中心工作区/管道跑的同一模板；因该 tab `workflows: []`，动作落到通用工作流编辑器而非投资中心 | `src/pages/opc/domains/DomainPages.tsx:694-706`、`src/pages/opc/domains/DomainTabContent.tsx:95-101`、`src/pages/PipelinePage/PipelinePage.tsx:54` |
| 6 | 「财报解读」是纯对话 prompt，不接任何数据 | `src-tauri/src/commands/opc_capability_pack_actions.rs:369-379`、`src/pages/opc/domains/DomainTabContent.tsx:108-132` |
| 7 | 域包工作流的 5 个节点引用的专家 profile **在库里不存在**，专家 persona 从未生效 | `seed_capability_pack_finance_invest.rs:30,40,60,85,97`（`opc-finance_invest_lead-finance-*`）vs `src-tauri/src/commands/opc_setup/capability_pack_agents.rs:121`（产出 `opc-{expert_key}`）；缺失时 `agent_executor.rs:460-480` 静默退回节点内联 prompt |
| 8 | 结论 7 是**跨域包的系统性缺陷**，非 finance_invest 独有 | `seed_capability_pack_{accounting,ai_research,content_media,finance_invest,game_dev}.rs` 共 19 个节点引用不存在的 profile id |
| 9 | 包名三源漂移：manifest「金融投资分析」/ i18n「金融投资」/ Rust「金融投资」 | `config/opc/domain_packs/finance_invest/manifest.yaml:4`、`zh-CN.json:11511`、`opc_capability_pack_actions.rs:354` |
| 10 | 门禁只锁 **key**，不锁 **value** ⇒ 改文案安全，改键名必红 | `scripts/check-office-scene-align.mjs:211-252,284-295`（断言 `opc.domains.finance_invest`+`_desc` 键在 11 语言存在）、`scripts/check-i18n-key-parity.mjs`（只比叶子路径） |
| 11 | 前端动作清单来自本地 `DomainPages.tsx` 配置，后端 `opc_list_capability_pack_actions` **零调用方** | 已 grep `src/` 全树确认；仅 `opc_build_capability_pack_prompt` 被 3 处组件调用 |
| 12 | 改工作流模板正文后，**必须 bump `TEMPLATE_VERSION`**，否则已入库的行永不更新 | `src-tauri/src/commands/opc_workflows/mod.rs:635-648`（`existing.version >= version ⇒ 跳过`） |

## 2. 已定决策

- **D1** 域包新名目：**投顾经营**（2026-09-28 已确认）。
- **D2** 域包页的「个股分析」动作位**不保留**（整体删除，不做深链替代）。
- **D3** 结论 8 的跨域包坏 profile id 单列阶段 C，**排入本计划但不与 A/B 混验**。

## 3. 阶段 A — 名目收编（改值不改键）

目标：侧栏不再出现两个「金融投资」；域包对外名目与其真实数据源一致。

**A1 前端 i18n（11 语言，只改 value）** — `src/i18n/locales/{ar,de,en-US,es,fr,hi,ja,ko,ru,zh-CN,zh-TW}.json`

- `opc.domains.finance_invest`：金融投资 → **投顾经营**
- `opc.domains.finance_invest_desc`：改为「基于实际经营数据的财务分析与投资建议」（去掉「风险评估」股票话术）
- `nav.financeAnalysis`（现值「金融分析」，当前零消费方）→ **投顾经营**，作为 `finance-analysis` 导航项的新标签；**不新增键**（避免触碰 11 语言增键）
- `opc.domain.finance_invest.tabs.*`（`zh-CN.json:12190-12226`）三个 tab 与其下动作措辞按 CFO 语义改写：
  - 市场分析 → **经营概览**；`fi_financial` 财报解读 → **经营报表**
  - 估值建模 → **可投金额**；`fi_valuation` 估值计算 → **可投金额测算**
  - 风险管理 → **投顾风控**；`fi_risk` 风险评估 → **风险等级评估**
- `opc.domain.actions["finance-invest"].*`（`zh-CN.json:11582-11598`）与 `opc.domain.workflows["finance-invest"].*`（约 `:11944-11955`）两族同步改写（分别由 `DomainComponents.tsx:604,733` 动态前缀消费）

**A2 Rust 同步** — 硬编码中文不在 i18n 门禁扫描范围（`scripts/i18n-scan.mjs` 仅收 `.ts/.tsx`），但必须与前端一致：

- `src-tauri/src/commands/opc_capability_pack_actions.rs:354-356`（`name` / `description`）
- 同文件 `:2831,:2891` 两处 `("finance-invest", "金融投资")` 映射
- `src-tauri/crates/analysis-engine/src/opc/capability_pack_config.rs:202` `capability_pack_name`
- `src-tauri/src/commands/opc_workflows/seed_capability_pack_finance_invest.rs:155-157` 模板 `name`/`description`，并 **`TEMPLATE_VERSION` 4 → 5**（依据结论 12）

**A3 消除三源漂移（结论 9）** — `config/opc/domain_packs/finance_invest/manifest.yaml:4` 的 `name` 对齐「投顾经营」；`learning.yaml:3` 的 `domain_pack_name` 同步。

**A4 换图标** — `src/lib/navRegistry.tsx:72` 的 `FundFilled` 与首项 `DollarCircleFilled` 视觉同族，`finance-analysis` 改用非 Dollar 系图标（候选 `TeamOutlined` 已被 automation 占用，取 `SolutionOutlined` 一类未用项）。

**A5 检查清单** — 上述改写覆盖：`src/pages/opc/domains/DomainPages.tsx:682-767`（tab 标签与 `i18n.t` 键不变，只由 locale 值生效）；`config/opc/industries/finance_invest/`（旧目录孪生，若仍被读取需同改，动手前先确认读取方）。

**A6 防新歧义** — 同一金融域下第三项 `finance-accounting` 显示「财务会计」（`src/lib/navRegistry.tsx:77-84`）。「投顾经营」与它同为「经营/财务」话术，A1 措辞时须让两项一眼可分：本包一律带「投顾」前缀（投顾经营 / 可投金额 / 投顾风控），财务会计侧的措辞与键**不动**，避免把撞名从左移到右。改完后金融域三项应为：金融投资（投资中心）/ 投顾经营 / 财务会计。

## 4. 阶段 B — 拆除重复执行入口（决策 D2）

- 删 `src/pages/opc/domains/DomainPages.tsx:691-705` 中的 `fi-stock` 动作项（保留同 tab 的 `fi-financial`），使 `analysis` tab actions 只剩一项。
- 删 `src-tauri/src/commands/opc_capability_pack_actions.rs:358-368` 的 `fi-stock-analysis` 动作配置。
- 删 i18n 两族下的对应键（11 语言 × 2 处）：`opc.domain.finance_invest.tabs.analysis.actions.fi_stock`、`opc.domain.actions["finance-invest"]["fi-stock"]`。⚠ 这是**删键**，必须 11 语言同步，否则 `check-i18n-key-parity` 红。
- 删后核查：全树 grep `fi-stock` / `fi_stock` 归零（排除 `output/`、`.worktrees/`）。

## 5. 阶段 C — 坏专家 profile id（决策 D3，排期但独立验收）

现象：19 个 `Some("opc-…")` 引用的 id 形态与 seeder 产出不一致，运行期查不到 ⇒ 5 个专家的 md persona 从未生效，节点退回通用 prompt。

| 文件 | 坏引用数 | 现形态 | seeder 实际产出 |
|---|---|---|---|
| `src-tauri/src/commands/opc_workflows/seed_capability_pack_finance_invest.rs` | 5 | `opc-finance_invest_lead-finance-*` | `opc-finance-*` |
| `…/seed_capability_pack_accounting.rs` | 3 | `opc-accounting_lead-accounting-financial-*` | `opc-accounting-*` |
| `…/seed_capability_pack_game_dev.rs` | 4 | `opc-game_dev_lead-game-*` | `opc-game-*` |
| `…/seed_capability_pack_ai_research.rs` | 3 | `opc-ai_researcher-ai-*` | `opc-ai-*` |
| `…/seed_capability_pack_content_media.rs` | 4 | `opc-cmo-cmo-*` | 见 `capability_pack_experts.rs` 对应 `*_EXPERTS` 键 |

修法（二选一，实施前先用 `capability_pack_experts.rs` 的键清单核对每一条目标 id 真实存在）：
① 把种子里的引用改成 `opc-{expert_key}`；② 让 seeder 额外产出 `<pack>_lead-` 形态。默认取 ① —— 改引用面小、且与既有 6 个正常域包一致。

约束：每个被改动的 seed 文件都要 bump 自己的 `TEMPLATE_VERSION`（结论 12），并同步 `src-tauri/src/commands/opc_workflows/mod.rs:785,859-866` 中断言「金融投资」的用例。

## 6. 阶段 D — 记录不处理（另案）

- `src/i18n/locales/zh-CN.json:3833` `nav.invest`、`:6739-6744` `sidebar.sectionInvest*` 全仓零引用，属旧侧栏分组死键 —— 死键清理应走统一的 i18n 瘦身，不夹带进本计划。
- `src/components/stock-workspace/views/BacktestView.tsx:25` 跳 `BUILTIN_PAGE_PATH["replay-workbench"]`，该键不在 `src/lib/pageRegistry.ts:30-83` ⇒ 解析为 `undefined`，按钮为坏链。独立缺陷，建议另开。
- 域包动作 `fi-financial` 前端 key（`DomainPages.tsx:700`）与后端 key（`fi-financial-report`，`opc_capability_pack_actions.rs:370`）不一致 ⇒ 走 `DomainTabContent.tsx:134-147` 的通用兜底 persona。与阶段 C 同族（key 对齐），届时一并处理。

## 7. 验证门禁

阶段 A/B 各自完成后按 CI 口径执行；**两阶段全部完成后**再跑一次全量（AGENTS.md「逐阶段不跑全量」）。

```
node scripts/check-office-scene-align.mjs --selftest
node scripts/check-office-scene-align.mjs
node scripts/check-i18n-key-parity.mjs
node scripts/check-i18n-untranslated.mjs
python3 scripts/check_i18n.py --strict
bash scripts/check-hardcoded-i18n.sh --strict
npm run typecheck && npm run test:run
npm run lint:eslint          # oxlint，非 ESLint
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
```

⚠ 参数本身是门禁的一部分：`fmt` 少 `--all` 则 36 个 workspace crate 不被检查；`clippy` 少 `--all-features` 则 feature-gated 路径不被 lint。

## 8. 验收（需人工）

重启应用（改 md / seed 即 UPSERT 入库，见既有惯例），核对三条：

1. 侧栏「金融域」下不再出现两个「金融投资」，第二项显示「投顾经营」且图标与首项不同形。
2. `/invest` 页面标题仍「投资中心」，6 个 tab 未受影响；域包页 `analysis` tab 只剩 1 个动作，页面上不再出现「个股分析」。
3. 域包页执行「估值/风险」类动作，输出为经营数据口径（发票/项目/客户聚合），不再让人误读为个股结论 —— 这是名目收编是否真正生效的判据。

阶段 C 完成后另加一条：4. 运行日志中不再出现 profile 查空退回 inline 的 debug 记录，专家 persona 生效。

## 9. 提交切分

- commit 1（📦 文案）：阶段 A 的 i18n + Rust 字符串同步 + manifest 对齐 + 换图标
- commit 2（🔨 功能）：阶段 B 的动作位删除与键清理
- commit 3+（🐛 修复）：阶段 C 按域包分条

## 10. 实施记录（2026-09-28，A/B/C/D 已落地，门禁见 §11）

**与本文原稿的四处偏差，均为实施时新查明的事实：**

1. **侧栏标签的真实来源是 `nav.*`，不是 `opc.domains.*`。** 原稿 A1 把两件事混写了。实际改法是：`navRegistry.tsx` 的 `labelKey` 本来就指向 `nav.financeAnalysis`（A1 猜测的「改指向」无需执行），只把它的 11 语言值改成「投顾经营」；`nav.financeInvestment`（`/invest` 用）**保持「金融投资」不动**。`opc.domains.finance_invest` 另改，它驱动域包页大标题与顶栏。
2. **除 zh-CN 外，其余 10 语言的 `opc.domain.*` / `opc.domains.finance_invest` 整块值是英文**（zh-TW 也显示 "Finance & Investment"）。属存量翻译陈旧，与「去撞名」正交 ⇒ 本次只改**语义冲突键**（包显示名 + 导航标签）共 11 语言，股票话术的 tab/动作/工作流长文案仅在 zh-CN 收编。全量重译另案。
3. **阶段 C 的 19 条坏引用里，4 条无法安全映射**（原稿按 19 条全可修写）。普查脚本实测：`opc-cmo-cmo-content-creator / -seo-expert / -social-manager` 在全仓任何专家表里都没有同名键，`opc-cmo-cmo-content-strategist` 只作为公司路径 `cmo` 角色绑定存在（可解析，非坏）。⇒ 只修了 15 条（finance_invest 5 / accounting 3 / game_dev 4 / ai_research 3，其中 ai_research 的键定义在 `capability_pack_agents.rs` 而非 `capability_pack_experts.rs`），content_media 的 3 条**保持原样、未 bump 其模板版本** —— 不为过门禁猜目标。修后全量普查：21 个不同引用 id ⇒ 18 可解析 / 3 明确不可解析。
4. **D2「不处理」改为「补注册」。** `ReplayWorkbenchPage` 组件、其测试、`AppHeader.tsx:72` 的路径判断、`types/index.ts:933` 的 PageKey 全都已存在，只缺 `BUILTIN_PAGE_PATH` 的键与一条 `<Route>` —— 即该页是**被接线遗漏**而非被移除。故补 `pageRegistry.ts` 的 `"replay-workbench": "/replay-workbench"` + `ContentArea` 懒加载路由，使 `BacktestView.tsx:25` 的按钮成立。**若产品意图是删掉回放入口，请回退这一处。**

**阶段 D 其余两项**：死键 `nav.invest` + `sidebar.sectionInvest*`（6 个）11 语言共 77 键已删（删前核验：src 全树对这两模式零引用，且无 `nav.${` / `sidebar.section${` 动态拼键）；`fi-financial` 前端动作 key 已对齐后端 `fi-financial-report`，使 `opc_build_capability_pack_prompt` 不再落到通用兜底 persona。

## 11. 门禁实测状态

见会话内逐条退出码；⚠ 工作区在本次开工前已有他人的在途改动（`blackboard.rs`、`mcp_tools.rs`、`agent_executor.rs`、`code_executor.rs`、`AnalystReportCard.tsx`、`DebatePanel.tsx`、`agentOutput.ts`、`stockAnalysisStore.ts`、`seed_stock_analysis.rs`、`seed_consistency_tests.rs`、部分 agency_experts md），**全量 clippy / test 的红条须先分辨归属，不得由我代为收口**。
