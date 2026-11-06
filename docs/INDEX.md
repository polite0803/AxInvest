# AxInvest 文档索引

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0+（未定版，待确认）

本目录是 `docs/` 的唯一入口：登记全部文档、命名规则与新增登记方法。

- 命名规则：方案用 `PROPOSAL-<主题>.md`，计划用 `PLAN-<主题>.md`，审计用 `AUDIT-<主题>-<YYYY-MM-DD>.md`；`marketing/` 独立；功能说明书用 `docs/features/<slug>.md`。
- **登记顺序：新增文档先归入一个功能点（下方「功能视图」16 个 slug），再按文件类型进入「归档视图」对应类别；两处都要登记。** 两视图由同一份条目数据生成，不存在两份手写清单。
- 功能说明书登记：先按统一模板在 `docs/features/<slug>.md` 成稿（含 `功能点：<slug>` 元信息），再在该功能点节内以「说明书」一行登记；未撰写时台账标「待撰写」。
- 来源标注：`来源=本地原创` 为本仓文档；`来源=导入` 为上游 AxAgent 一次性归档（基准 99c9b96f，2026-09-07，2026-11-04 导入）。导入文档正文逐字未改，仅在文件头保留一行来源块；除「来源」字段外，导入文档与本地文档同等登记、同等计入统计。
- 参考类文档（`docs/reference/`、`architecture/dependency-*.md`、`gate-coverage-ledger.md`、`workflow-schema.md`）归入「归档视图 · 参考」。

## 功能视图（按功能点）

> 一级结构固定为 16 个功能点（slug 不可自行增删）。台账的「当前版本」记本仓当前发布版本 `v3.0.0`；节内摘要的「最新版本」取该功能点挂靠文档的最新版本。每个功能点节内：说明书（未撰写标「待撰写」）+ 挂靠文档（按日期倒序，带状态、来源与对应版本）+ 摘要（文档数 / 最新版本 / 最近变更日期）。

### 功能域台账（16 个功能点）

| slug | 功能点 | 边界（含 / 不含） | 主要代码根 | 当前版本 | 文档数 | 导入数 | 说明书 |
|---|---|---|---|---|---|---|---|
| `stock-decision-chain` | 股票决策链 | 含四周期独立决策、分析师卡片解析、方向分歧合流、估值/胜率口径与决策落库；不含候选池构造（归趋势智选）与回放编排（归时间旅行）。 | `src-tauri/src/commands/stock_workflow/`<br>`src-tauri/src/commands/stock_analysis_setup/`<br>`axagent-core/crates/harness/src/holding_period.rs`<br>`src/components/stock-analysis/` | v3.0.0 | 51 | 0 | 有 |
| `serenity-screening` | 趋势智选 | 含候选池构造/去重、快速链、来源优先级、定时筛选与智能荐股；不含单票决策（归股票决策链）与回放批次（归时间旅行）。 | `src-tauri/src/commands/pool_scan.rs`<br>`src-tauri/src/commands/stock_workflow/serenity.rs`<br>`src-tauri/src/commands/stock_analysis_setup/seed_serenity*.rs`<br>`src/components/stock-analysis/` | v3.0.0 | 14 | 0 | 有 |
| `time-travel-replay` | 时间旅行回放 | 含 as-of 封闭世界、批次编排、live 窗口互斥、准入快照校验、四档反思触发；不含策略验证指标口径（归量化回测）与反思算法本体（归反思进化）。 | `src-tauri/src/commands/time_travel_batch.rs`<br>`axagent-core/crates/harness/src/as_of.rs`<br>`src/components/time-travel/`<br>`axagent-core/crates/entities/src/time_travel_batch_runs.rs` | v3.0.0 | 5 | 0 | 有 |
| `market-data` | 行情与数据源 | 含行情/个股数据面、多数据源适配、跨市场与抓取降级、K 线缓存与板块/行业数据；不含数据质量评分与门禁（归数据质量门禁）。 | `axagent-quant/crates/astock-data/`<br>`src-tauri/src/commands/market_mainline.rs`<br>`src-tauri/src/commands/market_close_sweep.rs`<br>`src-tauri/src/commands/stock_pipeline/` | v3.0.0 | 9 | 0 | 有 |
| `opc-capability-packs` | OPC 能力包 | 含 OPC 域包/行业适配、能力集封闭、办公室场景与需求发现；不含插件装载（归插件体系）与股票决策（归股票决策链）。 | `axagent-quant/crates/analysis-engine/`<br>`axagent-quant/crates/company-runtime/`<br>`src-tauri/src/commands/opc_capability_pack_actions.rs`<br>`src-tauri/src/commands/opc_setup/` | v3.0.0 | 31 | 6 | 有 |
| `quant-backtest` | 量化回测 | 含量化策略/回测、因子信号契约、策略基线与市场仿真；不含生产决策落库（归股票决策链）。 | `axagent-quant/crates/quant/`<br>`axagent-quant/crates/market-sim/`<br>`src-tauri/src/commands/quant_backtest.rs`<br>`src-tauri/src/commands/backtest_validation.rs` | v3.0.0 | 11 | 4 | 有 |
| `reflection-evolution` | 反思与进化 | 含批次反思、T+N 归因、进化/自学习与优化闭环；不含四档持有期定义（归股票决策链/harness）。 | `src-tauri/src/commands/stock_workflow/reflection.rs`<br>`axagent-ai/crates/trajectory/`<br>`src-tauri/src/commands/evolution_engine.rs`<br>`src-tauri/src/commands/workflow_reflection.rs` | v3.0.0 | 14 | 2 | 有 |
| `task-center` | 任务中心与调度 | 含任务中心、cron/定时调度、后台任务、无头调度与监控告警；不含具体业务链实现。 | `src-tauri/src/commands/background_tasks.rs`<br>`src-tauri/src/commands/scheduled_task.rs`<br>`src-tauri/src/commands/recommendation_cron.rs`<br>`src/components/tasks/` | v3.0.0 | 11 | 1 | 有 |
| `paper-portfolio` | 模拟持仓与交易桥 | 含模拟持仓、组合管理器、风控门、交易桥与订单；不含回测仿真（归量化回测）。 | `src-tauri/src/commands/paper_portfolio.rs`<br>`src-tauri/src/commands/portfolio-mgr*.rhai`<br>`src-tauri/src/commands/portfolio-risk-gate.rhai` | v3.0.0 | 暂无（仅说明书） | 0 | 有 |
| `plugin-system` | 插件体系 | 含插件清单/worker 帧协议、L1-L2 通信与环检测、动态 UI 装载、生命周期与能力面；不含 OPC 业务能力包（归 OPC 能力包）。 | `axagent-plugins/crates/plugins/`<br>`axagent-core/crates/plugin-proto/`<br>`src-tauri/src/commands/plugin.rs`<br>`src/components/plugin/` | v3.0.0 | 20 | 5 | 有 |
| `data-quality-gates` | 数据质量与门禁 | 含 DQI 分级熔断、权重坍缩三档、字段级校验、数据修复与静态门禁口径；不含 CI 构建门（归构建与 CI）。 | `axagent-quant/crates/astock-data/src/scoring.rs`<br>`src-tauri/src/commands/stock_pipeline/`<br>`scripts/check-*.mjs`<br>`src-tauri/src/commands/stock_workflow/decision.rs` | v3.0.0 | 4 | 1 | 有 |
| `agent-runtime` | 运行时与智能体 | 含智能体引擎、工具/审批/沙箱、上下文与知识库、工作流引擎/节点、技能与 MCP；不含模型供应商适配（归模型提供商与流式）。 | `axagent-ai/crates/agent/`<br>`axagent-runtime/crates/runtime-core/`<br>`axagent-runtime/crates/tools/`<br>`src-tauri/src/commands/` | v3.0.0 | 43 | 29 | 有 |
| `provider-models` | 模型提供商与流式 | 含 LLM 提供商适配、流式输出、采样参数与 token 计量；不含智能体编排（归运行时与智能体）。 | `axagent-ai/crates/providers/`<br>`src/stores/feature/providerStore.ts`<br>`src/components/settings/` | v3.0.0 | 8 | 6 | 有 |
| `storage-migration` | 存储与数据库迁移 | 含实体/DAO/迁移、SQLite/PG 方言、缓存、CRDT 与知识图谱存储；不含业务查询语义。 | `axagent-infra/crates/storage/`<br>`axagent-infra/crates/migration/`<br>`axagent-infra/crates/dao/`<br>`axagent-core/crates/entities/` | v3.0.0 | 9 | 1 | 有 |
| `ui-shell` | 前端外壳 | 含前端外壳/路由/布局/信息架构、i18n、组件规范、滚动与主题；不含后端业务命令。 | `src/components/layout/`<br>`src/i18n/locales/`<br>`src/pages/`<br>`src/components/` | v3.0.0 | 13 | 6 | 有 |
| `build-ci` | 构建与 CI | 含 8 workspace 构建、CI/发布、依赖与分层门禁、格式化/审计脚本与仓库拆分；不含业务数据质量门禁（归数据质量与门禁）。 | `.github/workflows/`<br>`scripts/`<br>`axagent-core/Cargo.toml`<br>`dprint.json` | v3.0.0 | 16 | 3 | 有 |

### 股票决策链（`stock-decision-chain`）

- 摘要：文档 51 篇（本地 51 / 导入 0）｜ 最新版本 v3.0.0+（未定版，待确认） ｜ 最近变更 2026-11-04
- 说明书：`docs/features/stock-decision-chain.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `stock-decision-chain.md` | 说明书 | 一次股票分析在同一轮工作流里产出超短 / 短 / 中 / 长四档相互独立的决策结论，并把分析师卡片的方向分歧合流成一个主档结论。入口是投资工作台的股票分析与后端命令 run_sto… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-four-horizon-workflow-alignment.md` | 计划 | 数据源四周期满足度（这是需求①后半的答案，也是需求⑦的输入） | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-r3h-long-sample-calibration.md` | 计划 | 用户裁决：门槛（30）不变 ⇒ 必须从「样本供给」侧解。 | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-p0-p1-implementation-2026-10-26.md` | 审计 | P0 三项全部落地（R-3h 核心机制已实跑验证：long 档首次产出 T+90 成熟验证行）， | done | 2026-10-26 | v2.9.9 | 本地原创 |
| `AUDIT-stock-business-profitability-2026-10-26-1440.md` | 审计 | ⇒ 结论：不仅「还没火候」，而是「形态上产不出可执行买入 + 已观测的 α 为负 + 学习闭环仍恒为 0」。 | active | 2026-10-26 | v2.9.9 | 本地原创 |
| `AUDIT-stock-business-profitability-2026-10-26.md` | 审计 | ⇒ 结论不是「还没火候」，而是「当前形态即使有 α 也无法执行」+「α 本身为负」。 | active | 2026-10-26 | v2.9.9 | 本地原创 |
| `AUDIT-factor-evidence-2026-10-25.md` | 审计 | B-3 的前提是「因子能从本地数据算出来」。实测三个硬约束： | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-limitup-entry-2026-10-25.md` | 审计 | 一句话：这不是「价设得太近」，是「价根本成交不了」。 | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-signal-efficacy-2026-10-25.md` | 审计 | 没有。confidence 与 5 日收益的相关系数 r = 0.010（n=240），统计上等于零。 | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-stock-business-profitability-2026-10-25.md` | 审计 | 311 条有效样本（买入、期窗匹配、已成熟），已扣 0.25% 成本： | superseded | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-stock-profitability-verdict-2026-10-25.md` | 审计 | 不能。以当前状态，「不断迭代」不是通向盈利的路径，它本身是阻碍盈利的结构性原因。 | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `PLAN-stock-analysis-latency.md` | 计划 | 日期：2026-10-25 | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `PLAN-validation-fill-model.md` | 计划 | 采用方案 ②：判定与收益都用「触发价成交」模型。 | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-stock-accuracy-2026-10-24.md` | 审计 | 针对三条业务线的目标（智能荐股准确荐股且不遗漏 / 股票分析工作流给出准确决策 / | active | 2026-10-24 | v2.9.9 | 本地原创 |
| `PLAN-hitrate-cost-basis.md` | 计划 | 三处相互放大的缺陷，合起来让所有胜率/均值系统性虚高： | done | 2026-10-24 | v2.9.9 | 本地原创 |
| `AUDIT-verdict-shape-normalization-2026-10-23.md` | 审计 | 日期：2026-10-23 | active | 2026-10-23 | v2.9.9 | 本地原创 |
| `PLAN-decision-action-single-source.md` | 计划 | 状态：待批（未开工）。落此文档于 2026-10-22。 | active | 2026-10-22 | v2.9.9 | 本地原创 |
| `PLAN-llm-horizon-decision-and-parity.md` | 计划 | 四档权威只有一套：src-tauri/crates/harness/src/holding_period.rs:31 的 enum Period | active | 2026-10-21 | v2.9.9 | 本地原创 |
| `PLAN-capital-long-evidence.md` | 计划 | 1. 002696 案例：四连板题材股以 trend-long 身份进了长线候选，分析端长线档判卖出。复盘发现 trend×long 证据（价格动量）与 90 交易日持有期之间零兑… | active | 2026-10-18 | v2.9.9 | 本地原创 |
| `PLAN-horizon-scale-alignment.md` | 计划 | 规则 R（定阶）：每根 bar 覆盖的交易日 d 必须 ≤ 本档持有期 h / 3 | active | 2026-10-18 | v2.9.9 | 本地原创 |
| `AUDIT-horizon-naming-2026-10-16.md` | 审计 | 日期：2026-10-16 | active | 2026-10-16 | v2.9.9 | 本地原创 |
| `PLAN-evidence-factor-audit.md` | 计划 | 立档：2026-10-16　执行人：Qoder　状态：待批（未开工） | active | 2026-10-16 | v2.9.9 | 本地原创 |
| `PLAN-stock-decision-reliability.md` | 计划 | 决策后 4 个交易日累计约 -4.6%；30.8 上方正是 9 月高点 31.77 一线的套牢区，大阳线次日即遇阻。超短档（持有 2 日）、短期档（5 日）当即失效。 | active | 2026-10-15 | v2.9.9 | 本地原创 |
| `PLAN-horizon-decision-repair.md` | 计划 | 以下读数均已在 2026-10-14 取自现网（PG，v154 代）： | active | 2026-10-14 | v2.9.9 | 本地原创 |
| `PLAN-dcf-growth-single-point.md` | 计划 | 同一次运行（stock_analyses.id=21cdd00e-f4a4-4780-aaac-4793516938b4，2026-10-02 16:06）落库的 dcf.assu… | active | 2026-10-02 | v2.9.9 | 本地原创 |
| `AUDIT-analyst-low-confidence-2026-10-01.md` | 审计 | 1. stock_analyses.id = a7e590a4-6239-4ffc-805a-c80aa1b9dec5（600406 国电南瑞，11:29 运行， | active | 2026-10-01 | v2.9.9 | 本地原创 |
| `PLAN-multi-horizon-decision-science.md` | 计划 | 一句话：现在是「一套算法 + 四个阈值 + 一组固定百分比」，不是「四套按尺度成立的算法」。 | active | 2026-09-29 | v2.9.9 | 本地原创 |
| `PLAN-dcf-inapplicable-panel-gate.md` | 计划 | 0.69-0.94元（历史代理锚…DCF模型前提不成立，仅供参考）」 | active | 2026-09-28 | v2.9.9 | 本地原创 |
| `PLAN-horizon-four-cycle-closure.md` | 计划 | 四档周期 ultra_short / short / mid / long（2 / 5 / 28 / 90 交易日）要在四条链路上做到「输入有周期 → 结论标周期 → 验证按周期 … | active | 2026-09-28 | v2.9.9 | 本地原创 |
| `PLAN-dcf-fallback-anchor-applicability.md` | 计划 | 301302（华如科技）16:54 运行 2bb5bb9d：巴菲特估值结论「21.17–71.86 元」，现价 21.72 ⇒ | active | 2026-09-27 | v2.9.9 | 本地原创 |
| `AUDIT-300642-valuation-leg-cancellation-2026-09-23.md` | 审计 | 样本：stock_analyses / 300642 透景生命 / created_at=1790104067295（2026-09-22）/ template_version=7… | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `AUDIT-decision-chain-structural-bias-2026-09-23.md` | 审计 | 买入档可达性：不是结构性不可达 —— 见 §3 的生产公式复算（合成场景 ⇒ 买入）。 | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` | 审计 | 前几轮修的都是接线（DB 旧值覆盖、字段级死链、判据失明、展示层派生）。 | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `PLAN-decision-action-convergence.md` | 计划 | 用户提问「减持/卖出/观望/买入这些概念是不是同义却分成了不同标记」——是，且不止是命名问题。 | done | 2026-09-23 | v2.9.9 | 本地原创 |
| `PLAN-stock-analysis-fast-workflow.md` | 计划 | 日期：2026-09-24 | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `PLAN-stock-decision-hitrate-validation.md` | 计划 | 在反思工作流已产出的验证数据上，补齐确定性判定与聚合统计，让「单周期决策准不准」有可复算、有基线的后市实证： | done | 2026-09-23 | v2.9.9 | 本地原创 |
| `PROPOSAL-stock-decision-four-horizon.md` | 方案 | 把股票分析决策从「单决策 + 单 timeHorizon」升级为「四周期独立决策能力」： | done | 2026-09-23 | v2.9.9 | 本地原创 |
| `AUDIT-300642-run-variance-2026-09-22.md` | 审计 | 三条独立缺陷叠加，共同制造了用户看到的现象： | active | 2026-09-22 | v2.9.7 | 本地原创 |
| `PLAN-stock-audit-fixes.md` | 计划 | 来源：AUDIT-stock-business-2026-09-12.md | active | 2026-09-22 | v2.9.7 | 本地原创 |
| `AUDIT-analyst-low-confidence-2026-09-21.md` | 审计 | 日期：2026-09-21 | active | 2026-09-21 | v2.9.7 | 本地原创 |
| `AUDIT-dcf-cashflow-anchor-2026-09-21.md` | 审计 | direct_fcf 在生产上恒为 None —— 因为 vendor 从来没有取过现金流量表。 | active | 2026-09-21 | v2.9.7 | 本地原创 |
| `AUDIT-decision-action-consistency-2026-09-21.md` | 审计 | 「持有 vs 观望」不是两个独立语义，而是中性档（HOLD / WAIT）按持仓状态单向派生的结果： | active | 2026-09-21 | v2.9.7 | 本地原创 |
| `AUDIT-pledge-attribution-2026-09-21.md` | 审计 | 日期：2026-09-21 | done | 2026-09-21 | v2.9.7 | 本地原创 |
| `AUDIT-valuation-param-db-override-2026-09-21.md` | 审计 | 用户报「华大智造现价 73.11 元，而估值结论 16.33–38.84 元」，并明确指出 | active | 2026-09-21 | v2.9.7 | 本地原创 |
| `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` | 审计 | 日期：2026-09-20 | active | 2026-09-20 | v2.9.7 | 本地原创 |
| `AUDIT-601166-chain-break-2026-09-14.md` | 审计 | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-601166-valuation-decision-2026-09-14.md` | 审计 | ⇒ 「启动失败 / 节点不存在」类故障已闭环。本轮问题在下游语义层。 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-sany-600031-decision-2026-09-13.md` | 审计 | 界面那张卡片是对的（公式链产出），库里那条是错的。根因不在业务逻辑，而在工作流引擎： | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `AUDIT-xingye-601166-decision-2026-09-13.md` | 审计 | 审计日期：2026-09-13 | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `AUDIT-stock-business-2026-09-12.md` | 审计 | 日期：2026-09-12 | active | 2026-09-12 | v2.9.7 | 本地原创 |
| `PLAN-VALUATION-CONFIG-WIRING.md` | 计划 | astock-data/src/mcp_tools.rs:1918-1920 三常量已与 analysis-engine::decision::ValueConfig | active | 2026-09-12 | v2.9.7 | 本地原创 |

### 趋势智选（`serenity-screening`）

- 摘要：文档 14 篇（本地 14 / 导入 0）｜ 最新版本 v3.0.0+（未定版，待确认） ｜ 最近变更 2026-11-04
- 说明书：`docs/features/serenity-screening.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `serenity-screening.md` | 说明书 | 从趋势 / 瓶颈候选池出发，用确定性评分加轻量链路筛出当日可分析的标的，并把结果写回候选池供工作流与分析使用。入口是投资工作台的选股页签（ScreenerPage / Sereni… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-reco-systemic-rebuild.md` | 计划 | 1. 证据与风格健康分离：confidence 是「这只票的证据有多强」，不是「这个风格最近赚不赚钱」。 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-serenity-full-coverage-predictive.md` | 计划 | 数据基础已具备。缺口在三处——候选池构造（锁死 20 只领涨股 + 每趋势 ≤2 候选）、 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-a2-backfill-validation.md` | 计划 | 去重后 synthetic=0 的真实候选：598 条 / 18 个交易日。 | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-mover-miss-closure-2026-10-23.md` | 审计 | 日期：2026-10-23 | active | 2026-10-23 | v2.9.9 | 本地原创 |
| `PLAN-mover-recall-defect-closure.md` | 计划 | 1. 先止血再扩面：先修让补分析出不了货的单位错配，再动核查频率与事件域。 | active | 2026-10-22 | v2.9.9 | 本地原创 |
| `PLAN-reco-signal-quality.md` | 计划 | 对同一批 as-of 回放候选（先固定 as_of=2026-09-28），做前后两次后验对拍： | active | 2026-10-20 | v2.9.9 | 本地原创 |
| `PLAN-batch-automation-mover-recall.md` | 计划 | 当前用户手动完成选股与分析、反思，系统侧已存在大量自动化零件但闭环不完整。本方案的目标： | active | 2026-10-13 | v2.9.9 | 本地原创 |
| `PLAN-reco-asof-generation.md` | 计划 | 智能荐股与趋势智选在 as-of（回放）下取到的候选股票必须落库持久化，并基于这些 | active | 2026-10-12 | v2.9.9 | 本地原创 |
| `AUDIT-mover-universe-feasibility-2026-09-30.md` | 审计 | 方案一成立，但主通道必须换成腾讯批量行情：全市场 5921 只的「当日涨跌幅」用 qt.gtimg.cn/q= 批量（80 码/请求）只要 74 个请求 ≈ 42 秒，实测零失败；… | active | 2026-09-30 | v2.9.9 | 本地原创 |
| `PLAN-mover-recall-attribution.md` | 计划 | 原文：在最近四个周期区间（超短/短/中/长）内分别出现超过「单日涨停 / 20% / 30% / 40%」的股票，检查是否被对应时间档位的智能荐股 / 趋势智选推荐；未推荐者分析原… | done | 2026-09-30 | v2.9.9 | 本地原创 |
| `PLAN-serenity-horizon-adaptation.md` | 计划 | 趋势智选的四周期适配不等于把它改成四档出票。既有裁定（serenityPanel.tierScopeHint、 | active | 2026-09-30 | v2.9.9 | 本地原创 |
| `PLAN-reco-horizon-science-alignment.md` | 计划 | 本仓有两套「四周期」实现，互不消费： | active | 2026-09-29 | v2.9.9 | 本地原创 |
| `PLAN-serenity-fast-workflow.md` | 计划 | 日期：2026-09-24 | active | 2026-09-23 | v2.9.9 | 本地原创 |

### 时间旅行回放（`time-travel-replay`）

- 摘要：文档 5 篇（本地 5 / 导入 0）｜ 最新版本 v3.0.0+（未定版，待确认） ｜ 最近变更 2026-11-04
- 说明书：`docs/features/time-travel-replay.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `time-travel-replay.md` | 说明书 | 把分析锚到历史某个交易日 as_of，用当时可见的数据重跑选股链并产出对照结论，用来验证策略在历史上是否成立。入口是 /invest/replay 的回放工作台（ReplayWor… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-r2-daily-replay.md` | 计划 | R-1 接通回放验证后发现回放样本价格错位（详见 AUDIT-stock-business-profitability-2026-10-25.md §11）。 | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-replay-sample-feasibility-2026-10-25.md` | 审计 | 可行，但结论要修正三处。 回放基建比你预期的成熟得多（这点你可能低估了自己已建成的东西）； | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `PLAN-asof-replay-quality-attribution.md` | 计划 | 提交链：04b24a17d(S1) → e585cb489(S2) → 0123b51ba(S3) → f8e9fc568(S2 lint) → c18cba4e1(S3 判据) | active | 2026-09-26 | v2.9.9 | 本地原创 |
| `PLAN-asof-gaps.md` | 计划 | D 桶优先级高于 A/B：「取不到」会在降级面板留痕并让分析师写"数据缺失"；「取错」什么都不说。 | active | 2026-09-25 | v2.9.9 | 本地原创 |

### 行情与数据源（`market-data`）

- 摘要：文档 9 篇（本地 9 / 导入 0）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/market-data.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `market-data.md` | 说明书 | 覆盖「把行情与个股数据取进来」这一层：多 vendor 适配与路由、跨市场（A 股 / 美股 / 港股）分流、抓取失败后的降级与健康回退、gzip 与编码异常处理、K 线缓存与复权… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `AUDIT-data-outcome-coverage-2026-10-28.md` | 审计 | get_index_quotes 在三条通道全部失败时曾返回 Ok(vec![])，LLM 只能看到「今天没有指数行情」， | active | 2026-10-28 | v2.9.9 | 本地原创 |
| `AUDIT-stock-live-data-2026-10-24.md` | 审计 | X-4 的「读不到生产库」已消除；三条模板都处于「代码待落库」状态，不存在 R-6 的 | done | 2026-10-24 | v2.9.9 | 本地原创 |
| `PLAN-concept-block-change-pct-null.md` | 计划 | 「概念涨幅数据缺失：获取的概念板块未附带当日涨跌幅」（运行 414a926c，长川科技 300604） | active | 2026-09-28 | v2.9.9 | 本地原创 |
| `PLAN-emweb-gzip-decode-failure.md` | 计划 | 5 源全灭（eastmoney/akshare 同 URL error decoding response body、browser_eastmoney | active | 2026-09-28 | v2.9.9 | 本地原创 |
| `PLAN-industry-ranking-asof-gap.md` | 计划 | get_industry_ranking … 板块指数日 K 全部取数失败 HTTP request failed（eastmoney 与 | active | 2026-09-27 | v2.9.9 | 本地原创 |
| `PLAN-live-kline-cache-calendar-storm.md` | 计划 | 截图：301302 live 运行「决策信息缺失」，证据审计指认 t-risk 超时 → portfolio-mgr 级联跳过。 | active | 2026-09-27 | v2.9.9 | 本地原创 |
| `PLAN-margin-designified-absent-semantics.md` | 计划 | 对 18:22 运行 c6399466（粤海饲料 001313，74/B，工具可信度 58.2）的逐层核验： | active | 2026-09-27 | v2.9.9 | 本地原创 |
| `AUDIT-stock-data-horizons-2026-09-23.md` | 审计 | 多周期 K 线 / 指标可用。astock-data 数据层已具备按 period 参数拉取任意周期 K 线的能力， | active | 2026-09-23 | v2.9.9 | 本地原创 |

### OPC 能力包（`opc-capability-packs`）

- 摘要：文档 31 篇（本地 25 / 导入 6）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/opc-capability-packs.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `opc-capability-packs.md` | 说明书 | OPC（业务标签 axopc）的「域包」是纯数据资产包而非代码：每个包一个目录，内含 manifest.yaml（承诺能力契约 + 归属域）、roles.yaml（专家/工具白名单… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-domain-pack-consolidation.md` | 计划 | 两者无字段级对应：能力护照的 domain 是 CapabilityDomain 枚举；域包 manifest 无 domain 字段（不声明归属本体域）；roles.yaml 不… | active | 2026-10-11 | v2.9.9 | 本地原创 |
| `PLAN-domain-single-source.md` | 计划 | 已收敛的两个先例（正确形态，勿重新分裂）： | active | 2026-10-11 | v2.9.9 | 本地原创 |
| `PLAN-industry-to-domain-migration.md` | 计划 | 结论：上次「域改造」是加了一套新轨（domain）却没删旧轨（industry），且新增的 config/opc/domains/ 配置包无人读取。前端只把「域」做成了侧栏分组标题… | active | 2026-10-11 | v2.9.9 | 本地原创 |
| `PLAN-finance-pack-naming.md` | 计划 | 侧栏「金融域」下出现两个都叫「金融投资」的入口，且第二个页面对外宣称的四项能力 | active | 2026-09-28 | v2.9.9 | 本地原创 |
| `PLAN-office-auto-provision.md` | 计划 | 1. 创建房间时自动创建/绑定本体域（OPC 行业域包）所属智能体——现状要一个个手工加； | active | 2026-09-26 | v2.9.9 | 本地原创 |
| `PLAN-office-scene-domain-align.md` | 计划 | 根因定性：同一概念（行业显示名）存在两套平行键体系（office.scene.* vs opc.domains.*），且后者已完备——这是 AGENTS.md 禁区 12「禁止重复… | active | 2026-09-25 | v2.9.9 | 本地原创 |
| `PLAN-domain-ontology-wiring.md` | 计划 | 把 src-tauri/crates/harness/src/domain_ontology.rs 从「只被自身单测 + JS 门禁守护」推进到「被生产链路线性消费」，理顺「能力 … | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-ontology-vs-palantir-2026-09-15.md` | 审计 | 核心命题：传统平台建模"名词"，Ontology 建模"决策"。 Action 是独立一等类型，不是实体类上的方法（与 DDD 富血模型有本质区别）。 | active | 2026-09-15 | v2.9.7 | 本地原创 |
| `PLAN-semantica-borrowings.md` | 计划 | 配套：AUDIT-semantica-vs-axinvest-2026-09-13.md（总报告）+ AUDIT-semantica-ontology-deep-2026-09-1… | active | 2026-09-15 | v2.9.7 | 本地原创 |
| `AUDIT-domain-ontology-bottleneck-2026-09-14.md` | 审计 | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-domain-semantics-fixes-2026-09-14.md` | 审计 | 上游报告：AUDIT-domain-semantics-o1-2026-09-14.md（O1 注册表 + O2 扫描器） | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-domain-semantics-o1-2026-09-14.md` | 审计 | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-industry-label-removal-2026-09-14.md` | 审计 | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-ontology-abc-implementation-2026-09-14.md` | 审计 | 病灶：graph_dtos.rs 原 compute_type_affinity 把一条不相交兼容公理 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-ontology-p0123-execution-2026-09-14.md` | 审计 | 最重要的一个发现（P2-b）：边侧的问题不是「前端配色没跟上」，而是 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-ontology-recap-adaptive-wiring-2026-09-14.md` | 审计 | DB 侧当轮实测（原样）： | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-ontology-vs-existing-graph-structures-2026-09-14.md` | 审计 | 这个代码库里已经有三套类型系统在跑，但它们全部只描述「载体/过程」（这是哪种节点、怎么流转）， | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-semantica-layering-deep-2026-09-13.md` | 审计 | core/__init__.py:51-63 导出 Config, ConfigManager, LifecycleManager, PluginRegistry, Semanti… | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `AUDIT-semantica-ontology-deep-2026-09-13.md` | 审计 | 用户在上一轮指出「分析不够深入，其 ontology 应该是明显优于本地的」。这个判断成立，我上一轮的结论是错的。 | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `AUDIT-semantica-vs-axinvest-2026-09-13.md` | 审计 | 结论：这是一个「设计意图优秀、推理与管道层实现大量悬空」的项目 —— 但本体层是显著例外，它是完整的、可发布、且被系统性治理过的。可以借它的「机制设计」，绝不能借它的「推理/管道实… | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `PLAN-capability-discovery-orchestration-gap.md` | 计划 | 能力发现与认知编排的架构差距分析与实施方案。 | 已评审 | 2026-08-30 | v2.9.7 | 导入 |
| `PROPOSAL-unified-capability-model.md` | 方案 | 统一能力建模：让 LLM、编排器与开发者一致地发现与执行能力。 | 已实施 | 2026-08-30 | v2.9.7 | 导入 |
| `PLAN-opc-workflow-refactor.md` | 计划 | OPC 单人公司工作流与股票分析工作流不一致的重构方案。 | 未标注 | 2026-08-28 | v2.9.7 | 导入 |
| `PLAN-capability-supplementation.md` | 计划 | 能力补齐链路补全方案（cognitive_query 提议通道等）。 | 未标注 | 2026-08-17 | v2.9.5 | 导入 |
| `PROPOSAL-capability-discovery-design.md` | 方案 | 能力发现机制设计（按用户输入自动匹配工具与能力）。 | 已评审 | 2026-08-13 | v2.9.3 | 导入 |
| `PLAN-demand-discovery.md` | 计划 | OPC（一人公司）业务域已具备项目、客户、发票、看板、站点等能力，且工作流引擎（rt-workflow + WorkflowTemplateData + NodeKind 标准节点… | active | 2026-08-12 | v2.9.3 | 本地原创 |
| `PLAN-narrative-structure-enhancement.md` | 计划 | 当前文字创作工作流（workflow-cm-literary-creation）已具备基础的“生成式写作流水线”（元认知→大纲→循环写章→评审→归档），但其核心叙事逻辑（弧线、交汇… | active | 2026-08-10 | v2.9.2 | 本地原创 |
| `PLAN-axagent-company-runtime.md` | 计划 | 在 AxAgent 中引入 AI 公司运行时内核（组织自建、招聘决策、工作项状态机）。 | 待评审 | 2026-08-02 | v2.8.7 | 导入 |
| `PLAN-axinvest-opc-business.md` | 计划 | D:\OneManager\AxOPC 是 AxAgent 的 fork（2.8.6），已有 372 个独有文件：opc 三 crate（~2400 行）、19 个 OPC 工具、… | active | 2026-08-02 | v2.8.7 | 本地原创 |
| `PLAN-axopc-merge.md` | 计划 | 1. AxInvest 迁移改号 v209/v210/v211（AxOPC 用 v200/v201 与 v200_axinvest_stock_tables 冲突，已到 v208）… | active | 2026-08-02 | v2.8.7 | 本地原创 |

### 量化回测（`quant-backtest`）

- 摘要：文档 11 篇（本地 7 / 导入 4）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/quant-backtest.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `quant-backtest.md` | 说明书 | 量化能力由两条互不依赖的引擎组成，均由 wiring 层（src-tauri/src/commands/）对外暴露： | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `AUDIT-baseline-comparison-2026-10-25.md` | 审计 | 前面所有结论都基于「系统的净收益是 −0.72%」。但那个数字没有对照组： | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-factor-strategy-counterfactual-2026-10-25.md` | 审计 | 前序所有结论都卡在同一个地方：系统每天只产 6~7 条可验证的真实票， | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-sample-validity-2026-10-25.md` | 审计 | 「打板」意味着：以涨停封板价排队买入。它不可自动化、不可回测（成交与否取决于盘口排队与撤单速度， | active | 2026-10-25 | v2.9.9 | 本地原创 |
| `AUDIT-f-factor-signal-contract-2026-09-23.md` | 审计 | f 因子层没有「信号契约」。 每个因子各自就地决定三件事，三件事之间无统一约束、 | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `AUDIT-metrics-definition-2026-09-20.md` | 审计 | 审计日期：2026-09-20 | active | 2026-09-20 | v2.9.7 | 本地原创 |
| `PLAN-market-sim-value.md` | 计划 | 「把模拟仿真接入股票分析工作流」这个要求本身太粗 —— 项目里有四条互相独立的模拟线，它们与工作流的关系完全不同，混为一谈会得出相反结论。 | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `PLAN-refactor.md` | 计划 | 从「外围防御」转向「核心改造」的实质性重构方案（魔法字符串、隐式状态机）。 | 未标注 | 2026-08-19 | v2.9.5 | 导入 |
| `PLAN-unity-p2-runtime-permission-and-hybrid-classifier.md` | 计划 | Unity P2：运行时权限覆盖 + 分类策略 + LLM 增强实施计划。 | 未标注 | 2026-08-17 | v2.9.5 | 导入 |
| `PROPOSAL-cognitive-router-design.md` | 方案 | 认知编排器（V2 分层路由）设计方案：意图识别、能力匹配与分流策略。 | 未标注 | 2026-08-13 | v2.9.3 | 导入 |
| `PROPOSAL-afsim-axagent-distributed-architecture.md` | 方案 | AFSIM 与 AxAgent 的分布式集成架构设计（模块对接与协同仿真）。 | 未标注 | 2026-05-09 | v1.4.4 | 导入 |

### 反思与进化（`reflection-evolution`）

- 摘要：文档 14 篇（本地 12 / 导入 2）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/reflection-evolution.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `reflection-evolution.md` | 说明书 | 覆盖「决策做完之后回头看」这一层：四档持有期（T+2 / T+5 / T+28 / T+90）的到期反思、horizon_results_json 的消费与落库、未到期档进入每日待… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-missed-mover-reflection-closure.md` | 计划 | 立项 2026-10-20。触发需求：发现→分析→反思主干已闭环，但没被任何一条链发现的强势股（妖股）要能被系统性补漏，并用它来优化三条链。 | active | 2026-10-21 | v2.9.9 | 本地原创 |
| `PLAN-chain-closure-remediation.md` | 计划 | 来源：一次三链路闭环核查（智能荐股→反思 / 趋势智选→反思 / 荐股与智选→股票分析→反思）， | active | 2026-10-20 | v2.9.9 | 本地原创 |
| `PLAN-mover-coverage-and-reflection-exit.md` | 计划 | 根因侧硬事实（同轮取证，不再复述推导）：回放 seed pool 在 as-of 下两真实源按设计返回空（as_of_capability.rs:18 NoHistoricalSe… | active | 2026-10-18 | v2.9.9 | 本地原创 |
| `PLAN-reflection-optimization-closure.md` | 计划 | 范围来源：本轮「全面分析股票业务反思/优化缺陷」评估后用户按编号点完的六条拍板。 | active | 2026-10-14 | v2.9.9 | 本地原创 |
| `PLAN-reflection-per-horizon-row.md` | 计划 | 目标： 把股票反思链（stock_reflections / reflection_lessons）从「一次分析只复盘主档一行」改为每个周期档各产出一行反思，并修掉因此暴露的四处档… | active | 2026-09-30 | v2.9.9 | 本地原创 |
| `PLAN-reco-reflection-closure.md` | 计划 | 荐股链是纯规则打分（recommender/strategies/ 5 风格×4 周期），反思对它的优化形态不是改 prompt / | active | 2026-09-29 | v2.9.9 | 本地原创 |
| `AUDIT-stock-reflection-four-horizon-baseline-2026-09-23.md` | 审计 | 日期： 2026-09-23 | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `PLAN-stock-reflection-four-horizon.md` | 计划 | 目标： 在四周期股票决策已经落地后，让反思、客观绩效判定、持久化、命中率统计和前端展示按 ultra_short / short / mid / long 分别运行，同时保持历史单… | active | 2026-09-23 | v2.9.9 | 本地原创 |
| `AUDIT-evolution-stats-backfill-2026-09-17.md` | 审计 | 日期：2026-09-17 | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `PLAN-evoflow-borrowings.md` | 计划 | 日期：2026-09-12 | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-evoflow-vs-axinvest-2026-09-12.md` | 审计 | 日期：2026-09-12 | active | 2026-09-12 | v2.9.7 | 本地原创 |
| `PLAN-self-evolution-implementation.md` | 计划 | 自进化能力的分阶段实施计划（P0 双通道闭环优先）。 | 未标注 | 2026-08-16 | v2.9.5 | 导入 |
| `AUDIT-self-evolution-gap-2026-08-13.md` | 审计 | 对标 DeepSeek Harness 的自进化能力差距分析与改进建议。 | 未标注 | 2026-08-13 | v2.9.3 | 导入 |

### 任务中心与调度（`task-center`）

- 摘要：文档 11 篇（本地 10 / 导入 1）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/task-center.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `task-center.md` | 说明书 | 覆盖「定时/手动把任务跑起来」这一层：任务中心统一运行视图、cron 表达式解析与档期计算、本地时区逐分钟扫描、无头调度（无 GUI 进程也能派发）、recommendation_… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `AUDIT-gate-and-daemon-impact-2026-10-26.md` | 审计 | ⇒ 上一版裁决里「G5 仍为 0」这条要撤销。 | active | 2026-10-26 | v2.9.9 | 本地原创 |
| `AUDIT-gates-and-headless-scheduler-2026-10-26.md` | 审计 | 默认全关 ⇒ 与引入前逐位一致（改动不改变既有行为，除非显式打开）。 | done | 2026-10-26 | v2.9.9 | 本地原创 |
| `PLAN-b1-daily-pipeline.md` | 计划 | cron_jobs 里 decision-backtest-daily 已 active： | done | 2026-10-25 | v2.9.9 | 本地原创 |
| `PLAN-async-tracked-slow-commands.md` | 计划 | 三个慢命令已接任务中心纳管（运行可见），但仍是同步 IPC：前端 invoke 会 await 到 | active | 2026-10-23 | v2.9.9 | 本地原创 |
| `AUDIT-tasks-center-status-2026-10-22.md` | 审计 | 证据：cron.adapter.ts:32-40 只把每个任务的 lastResult 合成一条运行；execute_scheduled_task 对内置任务走 scheduler… | active | 2026-10-22 | v2.9.9 | 本地原创 |
| `PLAN-monitor-chain-closure.md` | 计划 | 价格告警引擎在跑，但它的产出没有任何消费者：事件名两头对不上、触发状态永不回写、通知层从未装配。同时同一只股票只能存活一条告警 —— 现网 56 条告警里，实际进入实时监控的最多 … | active | 2026-10-14 | v2.9.9 | 本地原创 |
| `PLAN-daily-automation-pipeline.md` | 计划 | 1. 每个交易日 00:00 智能荐股 → 产生真实候选股票池 | active | 2026-09-30 | v2.9.9 | 本地原创 |
| `AUDIT-task-ledger-monotonic-2026-09-17.md` | 审计 | task_ledger.rs 的 mtime 前进到 00:53:13 已解释：worker 为逼 cargo 失效指纹而 | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-scheduled-tasks-2026-09-13.md` | 审计 | 关键的「声明 vs 运行」对照（修后）：with_task_type 全集（10 个）现已全部有落点 —— 6 个走专属分支，3 个（opc-demand-discovery / … | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `PROPOSAL-nightly-autonomous-task-design.md` | 方案 | 夜间长时自主任务的详细设计（优先复用既有调度与执行设施）。 | 待评审 | 2026-08-17 | v2.9.5 | 导入 |

### 模拟持仓与交易桥（`paper-portfolio`）

- 摘要：文档 1 篇（本地 1 / 导入 0）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/paper-portfolio.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `paper-portfolio.md` | 说明书 | 「模拟持仓」是一套虚拟组合簿记：把决策/资讯/截图诊断产生的标的按事件日价格记入 paper_positions，用最新行情价计算浮动与已实现盈亏，供观察后市表现。它不连接真实账户… | active | 2026-11-04 | v3.0.0 | 本地原创 |

### 插件体系（`plugin-system`）

- 摘要：文档 20 篇（本地 15 / 导入 5）｜ 最新版本 v3.0.0+（未定版，待确认） ｜ 最近变更 2026-11-04
- 说明书：`docs/features/plugin-system.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `AUDIT-l1-blockers-2026-11-04-0322.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-04 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-04-0422.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | active | 2026-11-04 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-04.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | active | 2026-11-04 | v3.0.0+（未定版，待确认） | 本地原创 |
| `plugin-system.md` | 说明书 | 插件体系把「内置功能」与「外部扩展」统一成同一套装载形态：一份 plugin.json 清单 + 可选的长驻 worker 子进程 + 可选的动态 UI 资产。目标是让扩展能力与内… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `AUDIT-l1-blockers-2026-11-03-1421.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-03-1557.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-03-1632.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-03.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-shell-plus-plugins.md` | 计划 | 对齐性自检：口径 1 与既有 §1.1 的主路径（独立二进制 + 长驻 worker + 长度前缀 JSON 帧）同形， | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-shared-primitives-neutrality.md` | 计划 | 51 处前端反向阻塞逐符号判定：真中性只有 2 处、判据误报 7 处、域内错位 20 处、宿主内联域逻辑 22 处；「20 处共享基元」原判据不成立。 | proposed | 2026-11-05 | v3.0.0+（未定版） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-02.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-02 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-capability-inventory-2026-11-01.md` | 审计 | 「跨面共享消费方」= 同一个前端文件（组件目录 / 页面 / store）既 invoke 了本域的某个命令，又 invoke 了别的域的命令。 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `AUDIT-l1-blockers-2026-11-01.md` | 审计 | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-everything-is-plugin.md` | 计划 | 关键判据：A 与 B 不是二选一。B 层插件在注册表里必须与 A 层内置实现是同一个类型， | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-plugin-gap-closure.md` | 计划 | 1. B 层零真实用户：仓内无任何插件声明 worker 或 capabilities（bundled 仅 2 个 shell hook 脚手架）；真实子进程集成测试默认门控跳过（… | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-dsh-plugin-gap.md` | 计划 | 这些是 AxInvest 里“通用基座”的部分。迁移时应删除自研、改用 DSH 实现，只保留配置与少量接线。 | done | 2026-10-27 | v2.9.9 | 本地原创 |
| `AUDIT-plugin-refactor-2026-08-14.md` | 审计 | 「一切皆插件」重构 P0–P3 完成度与缺陷核查报告。 | 已实施 | 2026-08-14 | v2.9.4 | 导入 |
| `PROPOSAL-everything-is-plugin-architecture.md` | 方案 | DeepSeek Harness 将「Everything is a Plugin（一切皆插件）」作为核心哲学：模型、工具、技能、会话、沙箱、存储、循环、UI，所有 Agent 能… | 未标注 | 2026-08-14 | v2.9.4 | 导入 |
| `comparison-hermes-agent.md` | 参考 | Hermes-Agent 与 AxAgent 的全面对比及可借鉴能力清单。 | 未标注 | 2026-07-11 | v2.8.3 | 导入 |
| `PLAN-openclaw-plugin-compat.md` | 计划 | OpenClaw 插件生态全面兼容实施计划。 | 未标注 | 2026-05-16 | v1.6.0 | 导入 |
| `PROPOSAL-openclaw-plugin-compat-design.md` | 方案 | OpenClaw 插件生态兼容设计（以同花顺数据插件为驱动示例）。 | 未标注 | 2026-05-16 | v1.6.0 | 导入 |

### 数据质量与门禁（`data-quality-gates`）

- 摘要：文档 4 篇（本地 3 / 导入 1）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/data-quality-gates.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `data-quality-gates.md` | 说明书 | 这条链路解决「分析结果看起来正常、其实是拿不到数据 / 上游分析师不可信 / 因子大量缺失」的静默失效：先由一个独立节点给每轮分析打数据质量等级（DQI），再把等级折算成仓位与置信… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-analysis-data-repair.md` | 计划 | 用户提出的问题是「工作流算法不断改进 ⇒ 分析数据失效」，诉求两项：快速修复失效数据 + 快速积累分析与反思数据。 | active | 2026-10-11 | v2.9.9 | 本地原创 |
| `AUDIT-data-quality-dual-algorithm-2026-09-14.md` | 审计 | 方法论差异不是「参数不同」，是「测的对象不同」：前者测「这批证据能不能信」，后者测「这个 JSON 长得规不规范」。 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `PROPOSAL-loop-engineering.md` | 方案 | 在 AxAgent 基座层实现 Loop Engineering（自改进循环）基础设施。这是 AI 智能体的最新范式：Agent 生成输出后自我评估，根据反馈迭代改进，直到达到质量… | 未标注 | 2026-07-28 | v2.8.6 | 导入 |

### 运行时与智能体（`agent-runtime`）

- 摘要：文档 43 篇（本地 14 / 导入 29）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/agent-runtime.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `agent-runtime.md` | 说明书 | 这条链路把「一次对话 / 一次工作流节点」落到可执行动作：会话持有上下文与事件，工具注册表把工具统一成可校验、可审计、可审批的执行单元，编排器决定哪些工具能并行；上下文超预算时由压… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `workflow-schema.md` | 参考 | 工作流节点 Schema 定义（供编辑器与校验共用） | active | 2026-10-30 | v2.9.9 | 本地原创 |
| `AUDIT-run-warnings-2026-10-23.md` | 审计 | 输入：C:\Users\polit\Documents\PowerShell.txt（1238 行，单次运行 10:25:15 ~ 10:35:xx，含多个 execution） | active | 2026-10-23 | v2.9.9 | 本地原创 |
| `PLAN-codex-parity-adoption.md` | 计划 | 日期：2026-09-25 | active | 2026-09-25 | v2.9.9 | 本地原创 |
| `AUDIT-codex-parity-gap-2026-09-24.md` | 审计 | 日期：2026-09-24 | active | 2026-09-24 | v2.9.9 | 本地原创 |
| `PLAN-jev-integration.md` | 计划 | 日期：2026-09-20 | active | 2026-09-20 | v2.9.7 | 本地原创 |
| `PLAN-cognitive-orchestration-fixes.md` | 计划 | 按 implement → review 循环逐项复查，发现 3 个新缺陷（1 个自引入、1 个既有、1 个口径问题）： | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `PLAN-weknora-borrowings.md` | 计划 | 日期：2026-09-14 | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-goink-vs-axinvest-2026-09-15.md` | 审计 | AGPL-3.0 是传染性许可。本审计的产出边界： | active | 2026-09-15 | v2.9.7 | 本地原创 |
| `PLAN-agent-workbench-enhancement.md` | 计划 | 工作台目前是「单路由 + 内存态单选 Tab」：/chat 是唯一真实路由，其余 7 个功能页被 redirectToChat 压成它的 Tab，Tab 状态只活在内存 store… | active | 2026-09-15 | v2.9.7 | 本地原创 |
| `PLAN-goink-borrowings.md` | 计划 | 性质：🔴 我方自有缺陷 ｜ 🟡 借鉴对方机制 ｜ 🟢 元机制 ｜ ⛔ 反面教材（产出护栏规则） | active | 2026-09-15 | v2.9.7 | 本地原创 |
| `AUDIT-explainer-completion-and-profile-maps-2026-09-14.md` | 审计 | 两层都必须修，只修①等于「让说明书从勉强能读变成写得更好」，用户仍看不到。 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-weknora-vs-axinvest-2026-09-14.md` | 审计 | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `PROPOSAL-EXEC-OBSERVABILITY.md` | 方案 | 不是「分析慢」，而是「长跑不可观测 + 卡死不自愈」： | active | 2026-09-12 | v2.9.7 | 本地原创 |
| `laap-agi-移植评估.md` | 参考 | laap-AGI 认知分析能力的移植可行性评估。 | 未标注 | 2026-09-04 | v2.9.7 | 导入 |
| `PLAN-awareness-saliency.md` | 计划 | 自主觉知生成 + 统一显著性仲裁器的实施方案。 | 设计定稿 | 2026-09-04 | v2.9.7 | 导入 |
| `PLAN-codex-parity.md` | 计划 | 对标 Codex 的 Agent 框架能力补齐方案，按 P0–P3 依赖排序。 | 未标注 | 2026-09-03 | v2.9.7 | 导入 |
| `PLAN-cognitive-orchestration-fixes-2026-09-07-snapshot.md` | 计划 | 认知编排分支缺陷的修复实施方案（含 3 个新缺陷与自引入回归）。 | 未标注 | 2026-09-01 | v2.9.7 | 导入 |
| `PROPOSAL-workflow-optimizer-design.md` | 方案 | WorkflowOptimizer（trajectory crate）的算法方案与阶段进度。 | 进行中 | 2026-08-31 | v2.9.7 | 导入 |
| `PLAN-approval-timeout-default-reject.md` | 计划 | 审批超时按 timeout_action 裁决（默认拒绝）的改造清单，待评审。 | 待评审 | 2026-08-17 | v2.9.5 | 导入 |
| `AUDIT-code-review-2026-08-14.md` | 审计 | 本次审查以项目 AGENTS.md 定义的规范与架构铁律为基准，结合静态检查与模块级源码走查。 | 已审查 | 2026-08-14 | v2.9.4 | 导入 |
| `PLAN-ai-research-agent.md` | 计划 | AI 研究智能体 KPI 真实性、status 契约与 DAG 冗余节点修复（2026-08-09 审查后遗留）。 | 未标注 | 2026-08-09 | v2.9.1 | 导入 |
| `PLAN-phase2-activate-backend-adapters.md` | 计划 | Phase 2：激活 8 个消息渠道后端适配器。 | 未标注 | 2026-07-17 | v2.8.5 | 导入 |
| `PLAN-phase3-ai-agent-message-flow.md` | 计划 | Phase 3：接入 AI Agent 消息流。 | 未标注 | 2026-07-17 | v2.8.5 | 导入 |
| `PROPOSAL-approval-hitl-implementation.md` | 方案 | ApprovalNode 可挂起人工介入（HITL）门的具体实现方案。 | 未标注 | 2026-07-06 | v2.8.0 | 导入 |
| `workflow-vs-n8n.md` | 参考 | AxAgent 工作流与 n8n 的能力对比分析。 | 未标注 | 2026-07-06 | v2.8.0 | 导入 |
| `AUDIT-architecture-2026-07-04.md` | 审计 | 按架构核心理念与审计标准对 AxAgent 的架构审计结论。 | 未标注 | 2026-07-04 | v2.7.0 | 导入 |
| `AUDIT-full-feature-defect-2026-07-04.md` | 审计 | 本次全链路审计发现 139 个功能缺陷（含上次审计残留和新增），按严重程度分布： | 未标注 | 2026-07-04 | v2.7.0 | 导入 |
| `AUDIT-file-git-tools-2026-07-02.md` | 审计 | 发现 8 个缺陷：4 个 P0（高危）、2 个 P1（中危）、2 个 P2（低危）。 | 未标注 | 2026-07-02 | v2.7.0 | 导入 |
| `AUDIT-full-feature-defect-2026-07-02.md` | 审计 | 本次全链路审计发现 78 个功能缺陷，按严重程度分布： | 未标注 | 2026-07-02 | v2.7.0 | 导入 |
| `architecture_report.md` | 参考 | AxAgent 项目架构分析报告（模块划分、依赖关系与数据流梳理）。 | 未标注 | 2026-07-01 | v2.7.0 | 导入 |
| `AUDIT-harness-engineering-2026-06-24.md` | 审计 | Harness 工程符合度审计报告（全栈符合度评分与差距）。 | 未标注 | 2026-06-24 | v2.6.0 | 导入 |
| `PROPOSAL-multi-agent-orchestration-design.md` | 方案 | 复杂长任务在 AI 执行过程中面临的核心挑战： | 草案 | 2026-06-21 | v2.6.0 | 导入 |
| `AUDIT-workflow-editor-defect-2026-06-15.md` | 审计 | 工作流编辑器缺陷审查报告（节点类型、连线与状态同步）。 | 未标注 | 2026-06-15 | v2.6.0 | 导入 |
| `AUDIT-full-code-2026-06-11.md` | 审计 | AxAgent 全面代码审计报告（Harness 剥离、缺陷分级与修复状态）。 | 未标注 | 2026-06-11 | v2.6.0 | 导入 |
| `PLAN-batch1-security-hardening.md` | 计划 | 安全加固实现计划（批次 1/3）。 | 未标注 | 2026-05-13 | v1.5.3 | 导入 |
| `PLAN-batch2-sandbox-hardening.md` | 计划 | 沙箱加固实现计划（批次 2/3）。 | 未标注 | 2026-05-13 | v1.5.3 | 导入 |
| `PLAN-batch3-multiagent-workflow.md` | 计划 | 多智能体协作 + DAG 工作流鲁棒性实现计划（批次 3/3）。 | 未标注 | 2026-05-13 | v1.5.3 | 导入 |
| `PROPOSAL-batch1-security-hardening-design.md` | 方案 | AxAgent 安全审计发现 5 类缺陷，按优先级分 3 批修复。第 1 批聚焦攻击面最广的两个领域： | 待实现 | 2026-05-13 | v1.5.3 | 导入 |
| `PROPOSAL-batch2-sandbox-hardening-design.md` | 方案 | AxAgent 安全审计发现沙箱为纯逻辑沙箱，无系统级隔离： | 待实现 | 2026-05-13 | v1.5.3 | 导入 |
| `PROPOSAL-batch3-multiagent-workflow-design.md` | 方案 | 审计发现两个子系统的架构缺陷： | 待实现 | 2026-05-13 | v1.5.3 | 导入 |
| `PROPOSAL-advanced-rag-design.md` | 方案 | AxAgent 当前 RAG 实现处于中级偏上水平（6.5/10），具备统一 RAG 抽象层、sqlite-vec 向量存储、混合搜索（向量+BM25）、语义缓存、基础重排序等能力… | 已确认 | 2026-05-12 | v1.5.2 | 导入 |
| `PLAN-phase5-complete-message-receiving.md` | 计划 | Phase 5：完善消息接收能力。 | 未标注 | 2026-04-30 | v1.3.0 | 导入 |

### 模型提供商与流式（`provider-models`）

- 摘要：文档 8 篇（本地 2 / 导入 6）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/provider-models.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `provider-models.md` | 说明书 | 这条链路把「一次模型调用」从 UI 的模型选择落到具体供应商请求：适配器把统一请求翻译成各家协议，流式增量再翻译回统一事件；tool_calls 在流式 / 非流式 / 历史序列化… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-llm-param-single-source.md` | 计划 | 同一执行体的同一 LLM 参数（max_tokens / temperature / top_p / frequency_penalty） | active | 2026-10-26 | v2.9.9 | 本地原创 |
| `PLAN-unity-p3-llm-classifier-and-approval-gate.md` | 计划 | P3：LLM 分类器实装 + ApprovalGate 真实阻断 + 端到端串联。 | 未标注 | 2026-08-17 | v2.9.5 | 导入 |
| `PLAN-knowledge-source-ingest.md` | 计划 | AxAgent 已有成熟的知识底座：Wiki 双体系（经典 + LLM）、RAG 导入管道（10+ 格式 + sqlite-vec）、WebFetchTool、持久化索引队列、cr… | 已完成 | 2026-08-01 | v2.8.6 | 导入 |
| `PLAN-phase1-platform-config-unification.md` | 计划 | Phase 1：消息渠道统一数据模型。 | 未标注 | 2026-07-17 | v2.8.5 | 导入 |
| `PROPOSAL-rfc-0001-stream-runtime.md` | 方案 | 引入流式执行运行时（Workflow Stream Runtime, WorSR）的 RFC。 | 未标注 | 2026-06-06 | v2.6.0 | 导入 |
| `PROPOSAL-stream-architecture.md` | 方案 | 工作流能力基座的视角反转方案（流式执行架构）。 | 未标注 | 2026-06-06 | v2.6.0 | 导入 |
| `PROPOSAL-layered-specialized-agents.md` | 方案 | 分层专用智能体的研发技术方案（轻量化架构、性能与 Token 管控）。 | 未标注 | 2026-04-30 | v1.3.0 | 导入 |

### 存储与数据库迁移（`storage-migration`）

- 摘要：文档 9 篇（本地 8 / 导入 1）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/storage-migration.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `storage-migration.md` | 说明书 | 存储层横跨两个 foundation / implementor crate： | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-memory-kb-reflow-id-space.md` | 计划 | src-tauri/src/init/services.rs:270-274： | active | 2026-10-11 | v2.9.9 | 本地原创 |
| `AUDIT-db-upsert-dialect-2026-09-17.md` | 审计 | 1. 报错原文 Query Error: error returned from database: 字段关联 "communities_json" 是不明确的 at line 9… | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-kg-cross-kb-merge-2026-09-17.md` | 审计 | wikis 绑定关系： | done | 2026-09-17 | v2.9.7 | 本地原创 |
| `PLAN-tombstone-code-wiring.md` | 计划 | 日期：2026-09-17 | done | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-tombstone-code-2026-09-16.md` | 审计 | 日期：2026-09-16 | done | 2026-09-16 | v2.9.7 | 本地原创 |
| `AUDIT-wiki-graph-edges-2026-09-15.md` | 审计 | 对标对象：WeKnora（output/tmp-weknora-src/WeKnora-main，MIT，仅作设计参照，不搬运代码） | active | 2026-09-15 | v2.9.7 | 本地原创 |
| `AUDIT-crdt-integration-2026-09-13.md` | 审计 | 这次接入是必要的 —— 一接入就暴露 crates/crdt 根本无法编译（D1），另有三个语义缺陷（D2–D4）。 | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `PLAN-causal-edge-layer.md` | 计划 | 为 knowledge_relations 增加因果语义的因果边层实施方案。 | 未标注 | 2026-09-04 | v2.9.7 | 导入 |

### 前端外壳（`ui-shell`）

- 摘要：文档 13 篇（本地 7 / 导入 6）｜ 最新版本 v3.0.0 ｜ 最近变更 2026-11-04
- 说明书：`docs/features/ui-shell.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `ui-shell.md` | 说明书 | 这条链路定义「用户看到的外壳」：页面从哪来、路由在哪注册、顶栏多高、文案从哪种语言来、组件之间允许怎样的依赖、浮层触发器如何定位、主题与语法高亮如何收口。它不是某个业务页面，而是所… | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-frontend-information-architecture.md` | 计划 | 两套导航模型并存： | active | 2026-10-27 | v2.9.9 | 本地原创 |
| `PLAN-url-as-truth.md` | 计划 | 当前 /chat 的 ?ws= 是半真相：有它就用它，没有它就用 localStorage。会话身份（conversationId）完全不进 URL，只在挂载时被读一次随即抹掉。于… | active | 2026-10-27 | v2.9.9 | 本地原创 |
| `PLAN-dashboard-consolidation.md` | 计划 | 1. src/lib/dynamicUI/dashboardSchema.ts + src/lib/__tests__/dashboardSchema.test.ts | active | 2026-09-30 | v2.9.9 | 本地原创 |
| `AUDIT-wiki-graph-ux-2026-09-17.md` | 审计 | 1. 「看不清」的根因是标签路径分叉：大图自动聚类（aggActive=true）时节点全走 | active | 2026-09-17 | v2.9.7 | 本地原创 |
| `AUDIT-invest-scroll-2026-09-14.md` | 审计 | 用户报告：投资中心页（/invest）无垂直滚动条，工作区等视图下部内容被折叠不可见（截图：工作区分析视图底部「看空/看多」比例条被硬裁）。 | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-scrollbar-visibility-2026-09-14.md` | 审计 | 投资中心内容区滚动生效后，滚动指示器在多种主题下都难以辨认（用户原话： | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `PLAN-phase4-frontend-ui-completion.md` | 计划 | Phase 4：消息渠道前端 UI 完善。 | 未标注 | 2026-07-17 | v2.8.5 | 导入 |
| `PLAN-hermes-borrow.md` | 计划 | 实施前对现有代码的盘点——基础设施已大量存在，核心工作是"接线 + 前端可视化"。 | 未标注 | 2026-07-11 | v2.8.3 | 导入 |
| `companion-window.md` | 参考 | nomifun-tauri 的 nomifun-companion 实现了桌面悬浮 AI 助手窗口（类似 Claude Desktop 的悬浮助手），macOS 用 macos-p… | 未标注 | 2026-07-06 | v2.8.0 | 导入 |
| `PLAN-i18n-hardcoded-strings-cleanup.md` | 计划 | i18n 硬编码字符串清理实施计划。 | 未标注 | 2026-05-15 | v1.5.8 | 导入 |
| `PROPOSAL-i18n-hardcoded-strings-cleanup-design.md` | 方案 | 项目存在系统性的 i18n 合规问题，总计约 1500 处违规，分三类： | 待审批 | 2026-05-15 | v1.5.8 | 导入 |
| `AUDIT-frontend-backend-2026-04-29.md` | 审计 | 前后端 IPC 契约批量不一致检测报告（invoke 调用 vs 命令注册）。 | 未标注 | 2026-04-29 | v1.2.7 | 导入 |

### 构建与 CI（`build-ci`）

- 摘要：文档 16 篇（本地 13 / 导入 3）｜ 最新版本 v3.0.0+（未定版，待确认） ｜ 最近变更 2026-11-04
- 说明书：`docs/features/build-ci.md`

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `build-ci.md` | 说明书 | 仓库由 8 个 Cargo workspace 根（7 个 axagent-* 技术层 + src-tauri）加一个 Vite 前端组成。构建体系要解决三件事： | active | 2026-11-04 | v3.0.0 | 本地原创 |
| `PLAN-repo-split.md` | 计划 | 1. 修改等待长 —— 改一行前端代码也要等 tsc 扫完 40+ 目录 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 本地原创 |
| `dependency-graph.md` | 参考 | 仓库依赖图与按技术层拆分可行性分析（参考） | active | 2026-10-31 | v3.0.0 | 本地原创 |
| `dependency-matrix.md` | 参考 | 逐 crate 读取 Cargo.toml 得到的精确依赖矩阵（2026-10-31 实测） | active | 2026-10-31 | v3.0.0 | 本地原创 |
| `gate-coverage-ledger.md` | 参考 | CI / 本地门禁覆盖面台账与补齐进度 | active | 2026-10-31 | v3.0.0 | 本地原创 |
| `PLAN-MIGRATION-GUIDE.md` | 计划 | 主仓库保留 .github/workflows/ci-split.yml，为每个子仓库触发独立 CI 作业： | active | 2026-10-31 | v3.0.0 | 本地原创 |
| `PLAN-zip-advisory-closure.md` | 计划 | 触发：CI Run cargo audit 红 —— error: 9 vulnerabilities found! + 14 条 allowed unmaintained。 | active | 2026-10-20 | v2.9.9 | 本地原创 |
| `AUDIT-codebase-review-roadmap-2026-09-19.md` | 审计 | 本仓的工程质量不是瓶颈，「不变量」才是。 | active | 2026-09-19 | v2.9.7 | 本地原创 |
| `PLAN-declarative-schema-sync.md` | 计划 | SeaORM 2.0.1 原生支持列级改名声明，且官方 sync 就在消费它。 | done | 2026-09-16 | v2.9.7 | 本地原创 |
| `AUDIT-ci-drift-skill-desc-2026-09-14.md` | 审计 | 改动文件： | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-layer-discipline-l1-2026-09-14.md` | 审计 | 上游计划：PLAN-semantica-borrowings.md 的 L1 节（借鉴清单里最后一条未执行的 P0） | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `AUDIT-duplication-2026-09-13.md` | 审计 | 日期：2026-09-13 | active | 2026-09-13 | v2.9.7 | 本地原创 |
| `PLAN-axinvest-type-driven-refactor.md` | 计划 | 上游 AxAgent 已完成类型驱动设计重构（阶段 A-D），核心框架层已实现： | active | 2026-08-19 | v2.9.5 | 本地原创 |
| `PROPOSAL-type-driven-design.md` | 方案 | 类型驱动设计的长期实施计划（以类型约束替代隐式状态机与魔法字符串）。 | 计划中 | 2026-08-19 | v2.9.5 | 导入 |
| `RELEASE.md` | 参考 | AxAgent 版本号规范、发布节奏与冻结/观察窗口流程。 | 未标注 | 2026-07-02 | v2.7.0 | 导入 |
| `AUDIT-harness-architecture-2026-06-04.md` | 审计 | Harness 架构全面审查报告（RAG、工具与分层合规）。 | 未标注 | 2026-06-04 | v2.6.0 | 导入 |

### 通用（跨域文档）

- 摘要：文档 5 篇（本地 3 / 导入 2）｜ 最新版本 v3.0.0+（未定版，待确认） ｜ 最近变更 2026-11-03
- 收录口径：确实跨多个功能点的文档（团队协作协议、跨域裁决、对外产品介绍），不强行归入单一功能点。

| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |
|---|---|---|---|---|---|---|
| `PLAN-dev-team-work-protocol.md` | 计划 | 写冲突规则： | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 本地原创 |
| `PLAN-p0-adjudication.md` | 计划 | 日期：2026-09-14（第二版，推翻第一版的核心结论） | active | 2026-09-14 | v2.9.7 | 本地原创 |
| `PROMO.md` | marketing | 产品对外介绍（内容停留在 v2.5.0 / 32 crates，已过期，待重写） | active | 2026-09-05 | v2.9.7 | 本地原创 |
| `PROPOSAL-rfc-0001-review-guide.md` | 方案 | RFC-0001 评审指南（评审前需达成的共识与检查点）。 | 未标注 | 2026-06-06 | v2.6.0 | 导入 |
| `PROPOSAL-rfc-0001-review-materials.md` | 方案 | RFC-0001 评审材料汇编（含关键架构纠偏）。 | 未标注 | 2026-06-06 | v2.6.0 | 导入 |

## 归档视图（按类别）

> 与功能视图共享同一份条目；本节只做类别追溯，不重复登记说明书（`docs/features/*.md` 仅在功能视图出现）。

### 方案（PROPOSAL）

| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 功能点 | 来源 | 关联 |
|---|---|---|---|---|---|---|---|---|
| `PROPOSAL-stock-decision-four-horizon.md` | PROPOSAL：股票分析决策的四周期化（超短线 / 短线 / 中期 / 长期） | 把股票分析决策从「单决策 + 单 timeHorizon」升级为「四周期独立决策能力」： | done | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-stock-data-horizons-2026-09-23.md` `PLAN-decision-action-convergence.md` `PLAN-stock-decision-hitrate-validation.md` `PLAN-llm-horizon-decision-and-parity.md` |
| `PROPOSAL-EXEC-OBSERVABILITY.md` | PROPOSAL — 执行可观测性与超时治理（T-1 立项书） | 不是「分析慢」，而是「长跑不可观测 + 卡死不自愈」： | active | 2026-09-12 | v2.9.7 | 运行时与智能体 | 本地原创 | `PLAN-VALUATION-CONFIG-WIRING.md` |
| `PROPOSAL-workflow-optimizer-design.md` | WorkflowOptimizer 算法方案 | WorkflowOptimizer（trajectory crate）的算法方案与阶段进度。 | 进行中 | 2026-08-31 | v2.9.7 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-unified-capability-model.md` | 统一能力建模（Unified Capability Model） | 统一能力建模：让 LLM、编排器与开发者一致地发现与执行能力。 | 已实施 | 2026-08-30 | v2.9.7 | OPC 能力包 | 导入 |  |
| `PROPOSAL-type-driven-design.md` | AxAgent 类型驱动设计（Type-Driven Design）长期实施计划 | 类型驱动设计的长期实施计划（以类型约束替代隐式状态机与魔法字符串）。 | 计划中 | 2026-08-19 | v2.9.5 | 构建与 CI | 导入 |  |
| `PROPOSAL-nightly-autonomous-task-design.md` | 夜间长时自主任务运行 · 详细设计文档 | 夜间长时自主任务的详细设计（优先复用既有调度与执行设施）。 | 待评审 | 2026-08-17 | v2.9.5 | 任务中心与调度 | 导入 |  |
| `PROPOSAL-everything-is-plugin-architecture.md` | 「一切皆插件」的 Agent 架构范式：DeepSeek Harness 解读与 AxAgent 重构路径 | DeepSeek Harness 将「Everything is a Plugin（一切皆插件）」作为核心哲学：模型、工具、技能、会话、沙箱、存储、循环、UI，所有 Agent 能… | 未标注 | 2026-08-14 | v2.9.4 | 插件体系 | 导入 |  |
| `PROPOSAL-capability-discovery-design.md` | 能力发现（Capability Discovery）设计方案 | 能力发现机制设计（按用户输入自动匹配工具与能力）。 | 已评审 | 2026-08-13 | v2.9.3 | OPC 能力包 | 导入 |  |
| `PROPOSAL-cognitive-router-design.md` | 认知编排器（Cognitive Router）设计方案 | 认知编排器（V2 分层路由）设计方案：意图识别、能力匹配与分流策略。 | 未标注 | 2026-08-13 | v2.9.3 | 量化回测 | 导入 |  |
| `PROPOSAL-loop-engineering.md` | Loop Engineering 基础设施 | 在 AxAgent 基座层实现 Loop Engineering（自改进循环）基础设施。这是 AI 智能体的最新范式：Agent 生成输出后自我评估，根据反馈迭代改进，直到达到质量… | 未标注 | 2026-07-28 | v2.8.6 | 数据质量与门禁 | 导入 |  |
| `PROPOSAL-approval-hitl-implementation.md` | ApprovalNode 可挂起人工介入（HITL）门 — 具体实现方案 | ApprovalNode 可挂起人工介入（HITL）门的具体实现方案。 | 未标注 | 2026-07-06 | v2.8.0 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-multi-agent-orchestration-design.md` | 多 Agent 无人值守编排系统设计方案 | 复杂长任务在 AI 执行过程中面临的核心挑战： | 草案 | 2026-06-21 | v2.6.0 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-rfc-0001-review-guide.md` | RFC-0001 评审指南 | RFC-0001 评审指南（评审前需达成的共识与检查点）。 | 未标注 | 2026-06-06 | v2.6.0 | 通用 | 导入 |  |
| `PROPOSAL-rfc-0001-review-materials.md` | RFC-0001 评审材料 | RFC-0001 评审材料汇编（含关键架构纠偏）。 | 未标注 | 2026-06-06 | v2.6.0 | 通用 | 导入 |  |
| `PROPOSAL-rfc-0001-stream-runtime.md` | RFC-0001: Workflow Stream Runtime（WorSR） | 引入流式执行运行时（Workflow Stream Runtime, WorSR）的 RFC。 | 未标注 | 2026-06-06 | v2.6.0 | 模型提供商与流式 | 导入 |  |
| `PROPOSAL-stream-architecture.md` | Design: AxAgent 工作流能力基座 — 视角反转方案 | 工作流能力基座的视角反转方案（流式执行架构）。 | 未标注 | 2026-06-06 | v2.6.0 | 模型提供商与流式 | 导入 |  |
| `PROPOSAL-openclaw-plugin-compat-design.md` | OpenClaw 插件生态全面兼容设计 | OpenClaw 插件生态兼容设计（以同花顺数据插件为驱动示例）。 | 未标注 | 2026-05-16 | v1.6.0 | 插件体系 | 导入 |  |
| `PROPOSAL-i18n-hardcoded-strings-cleanup-design.md` | i18n 硬编码字符串清理 — 设计文档 | 项目存在系统性的 i18n 合规问题，总计约 1500 处违规，分三类： | 待审批 | 2026-05-15 | v1.5.8 | 前端外壳 | 导入 |  |
| `PROPOSAL-batch1-security-hardening-design.md` | Batch 1: 安全加固设计文档 | AxAgent 安全审计发现 5 类缺陷，按优先级分 3 批修复。第 1 批聚焦攻击面最广的两个领域： | 待实现 | 2026-05-13 | v1.5.3 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-batch2-sandbox-hardening-design.md` | Batch 2: 沙箱加固设计文档 | AxAgent 安全审计发现沙箱为纯逻辑沙箱，无系统级隔离： | 待实现 | 2026-05-13 | v1.5.3 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-batch3-multiagent-workflow-design.md` | Batch 3: 多智能体协作 + DAG 工作流鲁棒性 设计文档 | 审计发现两个子系统的架构缺陷： | 待实现 | 2026-05-13 | v1.5.3 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-advanced-rag-design.md` | 高级 RAG 扩展设计方案 | AxAgent 当前 RAG 实现处于中级偏上水平（6.5/10），具备统一 RAG 抽象层、sqlite-vec 向量存储、混合搜索（向量+BM25）、语义缓存、基础重排序等能力… | 已确认 | 2026-05-12 | v1.5.2 | 运行时与智能体 | 导入 |  |
| `PROPOSAL-afsim-axagent-distributed-architecture.md` | AFSIM-AxAgent 分布式集成架构设计 | AFSIM 与 AxAgent 的分布式集成架构设计（模块对接与协同仿真）。 | 未标注 | 2026-05-09 | v1.4.4 | 量化回测 | 导入 |  |
| `PROPOSAL-layered-specialized-agents.md` | 分层专用智能体软件研发技术方案 | 分层专用智能体的研发技术方案（轻量化架构、性能与 Token 管控）。 | 未标注 | 2026-04-30 | v1.3.0 | 模型提供商与流式 | 导入 |  |

### 计划（PLAN）

| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 功能点 | 来源 | 关联 |
|---|---|---|---|---|---|---|---|---|
| `PLAN-dev-team-work-protocol.md` | PLAN · 开发团队工作协议（2026-10-30 建团） | 写冲突规则： | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 通用 | 本地原创 | `PLAN-frontend-information-architecture.md` `AUDIT-gates-and-headless-scheduler-2026-10-26.md` `PLAN-r3h-long-sample-calibration.md` `PLAN-url-as-truth.md` |
| `PLAN-four-horizon-workflow-alignment.md` | PLAN：四周期工作流对齐 —— 十项需求差距分析与实现方案 | 数据源四周期满足度（这是需求①后半的答案，也是需求⑦的输入） | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 股票决策链 | 本地原创 | `PLAN-multi-horizon-decision-science.md` `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` `PLAN-analysis-data-repair.md` |
| `PLAN-r2-daily-replay.md` | PLAN-r2-daily-replay —— 每日时间旅行回放（R-2） | R-1 接通回放验证后发现回放样本价格错位（详见 AUDIT-stock-business-profitability-2026-10-25.md §11）。 | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 时间旅行回放 | 本地原创 | `AUDIT-stock-business-profitability-2026-10-25.md` |
| `PLAN-r3h-long-sample-calibration.md` | PLAN-r3h-long-sample-calibration —— 打破「0 买入」死循环（门槛不变，增加 long/mid 样本） | 用户裁决：门槛（30）不变 ⇒ 必须从「样本供给」侧解。 | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 股票决策链 | 本地原创 | `AUDIT-stock-business-profitability-2026-10-26.md` |
| `PLAN-shell-plus-plugins.md` | PLAN · 主壳 + 按能力插件（= `PLAN-everything-is-plugin.md` 的 P5 阶段） | 对齐性自检：口径 1 与既有 §1.1 的主路径（独立二进制 + 长驻 worker + 长度前缀 JSON 帧）同形， | active | 2026-11-03 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 | `PLAN-everything-is-plugin.md` `PLAN-plugin-gap-closure.md` `AUDIT-capability-inventory-2026-11-01.md` `AUDIT-l1-blockers-2026-11-01.md` `PLAN-repo-split.md` `PLAN-asof-gaps.md` |
| `PLAN-shared-primitives-neutrality.md` | PLAN · 共享基元的逐符号中性判定（B8-4 前置） | 51 处前端反向阻塞逐符号判定：真中性只有 2 处、判据误报 7 处、域内错位 20 处、宿主内联域逻辑 22 处；「20 处共享基元」原判据不成立。 | proposed | 2026-11-05 | v3.0.0+（未定版） | 插件体系 | 本地原创 | `PLAN-shell-plus-plugins.md` |
| `PLAN-everything-is-plugin.md` | PLAN · 一切皆插件 + 动态插件 | 关键判据：A 与 B 不是二选一。B 层插件在注册表里必须与 A 层内置实现是同一个类型， | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `PLAN-plugin-gap-closure.md` | PLAN-plugin-gap-closure — 「一切皆为插件」缺口收编计划 | 1. B 层零真实用户：仓内无任何插件声明 worker 或 capabilities（bundled 仅 2 个 shell hook 脚手架）；真实子进程集成测试默认门控跳过（… | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 | `PLAN-everything-is-plugin.md` |
| `PLAN-reco-systemic-rebuild.md` | PLAN — 智能荐股系统性重建（2026-11-01） | 1. 证据与风格健康分离：confidence 是「这只票的证据有多强」，不是「这个风格最近赚不赚钱」。 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 趋势智选 | 本地原创 | `PLAN-reco-signal-quality.md` |
| `PLAN-repo-split.md` | PLAN: AxAgent 仓库拆分方案 | 1. 修改等待长 —— 改一行前端代码也要等 tsc 扫完 40+ 目录 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 构建与 CI | 本地原创 | `PLAN-industry-to-domain-migration.md` |
| `PLAN-serenity-full-coverage-predictive.md` | PLAN-趋势智选：候选池改「趋势→全关联股票」+ 打分预测力 | 数据基础已具备。缺口在三处——候选池构造（锁死 20 只领涨股 + 每趋势 ≤2 候选）、 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 趋势智选 | 本地原创 |  |
| `PLAN-MIGRATION-GUIDE.md` | AxAgent 仓库拆分迁移手册 | 主仓库保留 .github/workflows/ci-split.yml，为每个子仓库触发独立 CI 作业： | active | 2026-10-31 | v3.0.0 | 构建与 CI | 本地原创 | `PLAN-repo-split.md` |
| `PLAN-dsh-plugin-gap.md` | PLAN · DSH 插件能力缺口表（Gap Analysis） | 这些是 AxInvest 里“通用基座”的部分。迁移时应删除自研、改用 DSH 实现，只保留配置与少量接线。 | done | 2026-10-27 | v2.9.9 | 插件体系 | 本地原创 |  |
| `PLAN-frontend-information-architecture.md` | PLAN · 前端信息架构重整 | 两套导航模型并存： | active | 2026-10-27 | v2.9.9 | 前端外壳 | 本地原创 |  |
| `PLAN-url-as-truth.md` | PLAN — URL 作为真相（路由 / 深链收敛） | 当前 /chat 的 ?ws= 是半真相：有它就用它，没有它就用 localStorage。会话身份（conversationId）完全不进 URL，只在挂载时被读一次随即抹掉。于… | active | 2026-10-27 | v2.9.9 | 前端外壳 | 本地原创 | `PLAN-agent-workbench-enhancement.md` |
| `PLAN-llm-param-single-source.md` | PLAN — LLM 采样参数单一权威 | 同一执行体的同一 LLM 参数（max_tokens / temperature / top_p / frequency_penalty） | active | 2026-10-26 | v2.9.9 | 模型提供商与流式 | 本地原创 | `PLAN-stock-analysis-latency.md` |
| `PLAN-a2-backfill-validation.md` | PLAN：A-2 回补验证执行手册 | 去重后 synthetic=0 的真实候选：598 条 / 18 个交易日。 | active | 2026-10-25 | v2.9.9 | 趋势智选 | 本地原创 | `AUDIT-stock-profitability-verdict-2026-10-25.md` `PLAN-stock-decision-reliability.md` `AUDIT-decision-chain-structural-bias-2026-09-23.md` `PLAN-hitrate-cost-basis.md` |
| `PLAN-b1-daily-pipeline.md` | B-1 执行方案：每日出票 + 自动验证 + 因子基线台账 | cron_jobs 里 decision-backtest-daily 已 active： | done | 2026-10-25 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `PLAN-stock-analysis-latency.md` | PLAN — 股票分析流耗时优化（质量优先） | 日期：2026-10-25 | active | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-validation-fill-model.md` | PLAN：N-10 裁决 —— 验证模型统一为「触发价成交」 | 采用方案 ②：判定与收益都用「触发价成交」模型。 | active | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-hitrate-cost-basis.md` `AUDIT-limitup-entry-2026-10-25.md` `AUDIT-replay-sample-feasibility-2026-10-25.md` `AUDIT-stock-profitability-verdict-2026-10-25.md` |
| `PLAN-hitrate-cost-basis.md` | PLAN：命中率链成本口径修复（G4）— 2026-10-25 | 三处相互放大的缺陷，合起来让所有胜率/均值系统性虚高： | done | 2026-10-24 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-stock-profitability-verdict-2026-10-25.md` `AUDIT-replay-sample-feasibility-2026-10-25.md` `PLAN-stock-decision-reliability.md` `AUDIT-stock-live-data-2026-10-24.md` `PLAN-validation-fill-model.md` |
| `PLAN-async-tracked-slow-commands.md` | PLAN — 三个慢命令异步化（面板不冻等 + 报告落库可回取 + 可取消） | 三个慢命令已接任务中心纳管（运行可见），但仍是同步 IPC：前端 invoke 会 await 到 | active | 2026-10-23 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `PLAN-decision-action-single-source.md` | PLAN：决策结论档收归单一权威 + 分档互斥 | 状态：待批（未开工）。落此文档于 2026-10-22。 | active | 2026-10-22 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-mover-recall-defect-closure.md` | 妖股漏检链缺陷收口计划 | 1. 先止血再扩面：先修让补分析出不了货的单位错配，再动核查频率与事件域。 | active | 2026-10-22 | v2.9.9 | 趋势智选 | 本地原创 | `PLAN-mover-recall-attribution.md` `PLAN-missed-mover-reflection-closure.md` |
| `PLAN-llm-horizon-decision-and-parity.md` | PLAN：LLM 决策四档化 + 公式/LLM 逐档对照验证 | 四档权威只有一套：src-tauri/crates/harness/src/holding_period.rs:31 的 enum Period | active | 2026-10-21 | v2.9.9 | 股票决策链 | 本地原创 | `PROPOSAL-stock-decision-four-horizon.md` `PLAN-four-horizon-workflow-alignment.md` |
| `PLAN-missed-mover-reflection-closure.md` | PLAN：补漏未发现的妖股 → 三条链反思优化（智能荐股 / 趋势智选 / 分析工作流） | 立项 2026-10-20。触发需求：发现→分析→反思主干已闭环，但没被任何一条链发现的强势股（妖股）要能被系统性补漏，并用它来优化三条链。 | active | 2026-10-21 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `PLAN-chain-closure-remediation.md` | PLAN：股票业务链路闭环补强（2026-10-20） | 来源：一次三链路闭环核查（智能荐股→反思 / 趋势智选→反思 / 荐股与智选→股票分析→反思）， | active | 2026-10-20 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `PLAN-reco-signal-quality.md` | PLAN — 智能荐股信号质量（1→2→3） | 对同一批 as-of 回放候选（先固定 as_of=2026-09-28），做前后两次后验对拍： | active | 2026-10-20 | v2.9.9 | 趋势智选 | 本地原创 |  |
| `PLAN-zip-advisory-closure.md` | PLAN：cargo audit 的 zip 三族红收口 | 触发：CI Run cargo audit 红 —— error: 9 vulnerabilities found! + 14 条 allowed unmaintained。 | active | 2026-10-20 | v2.9.9 | 构建与 CI | 本地原创 |  |
| `PLAN-capital-long-evidence.md` | PLAN-capital-long-evidence — capital×long 证据腿换血（真正实现） | 1. 002696 案例：四连板题材股以 trend-long 身份进了长线候选，分析端长线档判卖出。复盘发现 trend×long 证据（价格动量）与 90 交易日持有期之间零兑… | active | 2026-10-18 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-horizon-scale-alignment.md` | PLAN-horizon-scale-alignment —— 四档「持有交易日 → 尺度 → 窗口」全库单源 | 规则 R（定阶）：每根 bar 覆盖的交易日 d 必须 ≤ 本档持有期 h / 3 | active | 2026-10-18 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-serenity-horizon-adaptation.md` |
| `PLAN-mover-coverage-and-reflection-exit.md` | PLAN — 题材/连板股覆盖 + 回放样本宇宙 + 反思结构出口 | 根因侧硬事实（同轮取证，不再复述推导）：回放 seed pool 在 as-of 下两真实源按设计返回空（as_of_capability.rs:18 NoHistoricalSe… | active | 2026-10-18 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `PLAN-evidence-factor-audit.md` | PLAN-evidence-factor-audit —— 证据引用审计改判据：从文本相似度换成决策因子对账 | 立档：2026-10-16　执行人：Qoder　状态：待批（未开工） | active | 2026-10-16 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-stock-decision-reliability.md` | PLAN：股票决策可靠性修复（紫金矿业 10-09 误判 + 大盘下行宏观链路 + 反思调度失效） | 决策后 4 个交易日累计约 -4.6%；30.8 上方正是 9 月高点 31.77 一线的套牢区，大阳线次日即遇阻。超短档（持有 2 日）、短期档（5 日）当即失效。 | active | 2026-10-15 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-horizon-naming-2026-10-16.md` |
| `PLAN-horizon-decision-repair.md` | PLAN-horizon-decision-repair —— 四档决策链修复（超短/短档） | 以下读数均已在 2026-10-14 取自现网（PG，v154 代）： | active | 2026-10-14 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-monitor-chain-closure.md` | PLAN · 股票「监控」链收口（A→B(+C)→E→F→G→D→H） | 价格告警引擎在跑，但它的产出没有任何消费者：事件名两头对不上、触发状态永不回写、通知层从未装配。同时同一只股票只能存活一条告警 —— 现网 56 条告警里，实际进入实时监控的最多 … | active | 2026-10-14 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `PLAN-reflection-optimization-closure.md` | PLAN — 反思与优化闭环六项收口（2026-10-14） | 范围来源：本轮「全面分析股票业务反思/优化缺陷」评估后用户按编号点完的六条拍板。 | active | 2026-10-14 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `PLAN-batch-automation-mover-recall.md` | PLAN：股票批量自动化闭环（选股→分析→反思→优化）+ 妖股反推 | 当前用户手动完成选股与分析、反思，系统侧已存在大量自动化零件但闭环不完整。本方案的目标： | active | 2026-10-13 | v2.9.9 | 趋势智选 | 本地原创 |  |
| `PLAN-reco-asof-generation.md` | PLAN-reco-asof-generation —— as-of 候选持久化与代际反思闭环 | 智能荐股与趋势智选在 as-of（回放）下取到的候选股票必须落库持久化，并基于这些 | active | 2026-10-12 | v2.9.9 | 趋势智选 | 本地原创 |  |
| `PLAN-analysis-data-repair.md` | PLAN — 分析数据修复与反思数据回填 | 用户提出的问题是「工作流算法不断改进 ⇒ 分析数据失效」，诉求两项：快速修复失效数据 + 快速积累分析与反思数据。 | active | 2026-10-11 | v2.9.9 | 数据质量与门禁 | 本地原创 | `PLAN-four-horizon-workflow-alignment.md` |
| `PLAN-domain-pack-consolidation.md` | PLAN：域包（Domain Pack）收敛到本体域 | 两者无字段级对应：能力护照的 domain 是 CapabilityDomain 枚举；域包 manifest 无 domain 字段（不声明归属本体域）；roles.yaml 不… | active | 2026-10-11 | v2.9.9 | OPC 能力包 | 本地原创 |  |
| `PLAN-domain-single-source.md` | PLAN-domain-single-source — 「域」多载体收敛为一套 | 已收敛的两个先例（正确形态，勿重新分裂）： | active | 2026-10-11 | v2.9.9 | OPC 能力包 | 本地原创 |  |
| `PLAN-industry-to-domain-migration.md` | industry → domain 迁移方案（执行前定稿） | 结论：上次「域改造」是加了一套新轨（domain）却没删旧轨（industry），且新增的 config/opc/domains/ 配置包无人读取。前端只把「域」做成了侧栏分组标题… | active | 2026-10-11 | v2.9.9 | OPC 能力包 | 本地原创 | `AUDIT-industry-label-removal-2026-09-14.md` |
| `PLAN-memory-kb-reflow-id-space.md` | PLAN — Memory → Knowledge 回流：**错 id 空间**与静默失败（单独立项） | src-tauri/src/init/services.rs:270-274： | active | 2026-10-11 | v2.9.9 | 存储与数据库迁移 | 本地原创 | `PLAN-declarative-schema-sync.md` |
| `PLAN-dcf-growth-single-point.md` | PLAN：DCF 增长率取数稳健化与主口径判据收口（K1→K4） | 同一次运行（stock_analyses.id=21cdd00e-f4a4-4780-aaac-4793516938b4，2026-10-02 16:06）落库的 dcf.assu… | active | 2026-10-02 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-daily-automation-pipeline.md` | PLAN: 每日自动化闭环四任务（荐股 / 趋势智选 / 候选池分析 / 四周期到期反思） | 1. 每个交易日 00:00 智能荐股 → 产生真实候选股票池 | active | 2026-09-30 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `PLAN-dashboard-consolidation.md` | PLAN — 仪表盘收编（v3：接线 + 清尸 + 真 React 插件贡献点） | 1. src/lib/dynamicUI/dashboardSchema.ts + src/lib/__tests__/dashboardSchema.test.ts | active | 2026-09-30 | v2.9.9 | 前端外壳 | 本地原创 |  |
| `PLAN-mover-recall-attribution.md` | PLAN — 窗口涨幅达标漏检核查（Mover Recall） | 原文：在最近四个周期区间（超短/短/中/长）内分别出现超过「单日涨停 / 20% / 30% / 40%」的股票，检查是否被对应时间档位的智能荐股 / 趋势智选推荐；未推荐者分析原… | done | 2026-09-30 | v2.9.9 | 趋势智选 | 本地原创 | `AUDIT-mover-universe-feasibility-2026-09-30.md` |
| `PLAN-reflection-per-horizon-row.md` | 股票反思「每档一行」收口计划 | 目标： 把股票反思链（stock_reflections / reflection_lessons）从「一次分析只复盘主档一行」改为每个周期档各产出一行反思，并修掉因此暴露的四处档… | active | 2026-09-30 | v2.9.9 | 反思与进化 | 本地原创 | `PLAN-horizon-four-cycle-closure.md` `PLAN-stock-reflection-four-horizon.md` `PLAN-multi-horizon-decision-science.md` |
| `PLAN-serenity-horizon-adaptation.md` | PLAN — 趋势智选（serenity）四周期适配收口 | 趋势智选的四周期适配不等于把它改成四档出票。既有裁定（serenityPanel.tierScopeHint、 | active | 2026-09-30 | v2.9.9 | 趋势智选 | 本地原创 | `PLAN-multi-horizon-decision-science.md` `PLAN-reco-horizon-science-alignment.md` |
| `PLAN-multi-horizon-decision-science.md` | PLAN：四周期决策的科学化重做（替换「共用主链 + 只换技术分」） | 一句话：现在是「一套算法 + 四个阈值 + 一组固定百分比」，不是「四套按尺度成立的算法」。 | active | 2026-09-29 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-horizon-four-cycle-closure.md` |
| `PLAN-reco-horizon-science-alignment.md` | PLAN：智能荐股四周期口径收编 + 前端呈现缺陷修复 | 本仓有两套「四周期」实现，互不消费： | active | 2026-09-29 | v2.9.9 | 趋势智选 | 本地原创 | `PLAN-multi-horizon-decision-science.md` |
| `PLAN-reco-reflection-closure.md` | PLAN：荐股链反思优化闭环（R-E 从「只报数」升级为「回写通路」） | 荐股链是纯规则打分（recommender/strategies/ 5 风格×4 周期），反思对它的优化形态不是改 prompt / | active | 2026-09-29 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `PLAN-concept-block-change-pct-null.md` | PLAN：概念板块涨跌幅恒为 null——F10 主题报告只给成分、榜单拼接缺失（K 轮，300604 实证） | 「概念涨幅数据缺失：获取的概念板块未附带当日涨跌幅」（运行 414a926c，长川科技 300604） | active | 2026-09-28 | v2.9.9 | 行情与数据源 | 本地原创 |  |
| `PLAN-dcf-inapplicable-panel-gate.md` | PLAN：DCF 判「不适用」后区间仍进面板——确定性闸口缺失（I 轮，601399 实证） | 0.69-0.94元（历史代理锚…DCF模型前提不成立，仅供参考）」 | active | 2026-09-28 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-emweb-gzip-decode-failure.md` | PLAN：emweb 财报接口 gzip 响应炸穿解码——reqwest 未开自动解压（L 轮，002371 实证） | 5 源全灭（eastmoney/akshare 同 URL error decoding response body、browser_eastmoney | active | 2026-09-28 | v2.9.9 | 行情与数据源 | 本地原创 |  |
| `PLAN-finance-pack-naming.md` | PLAN: 域包「金融投资」与投顾经营语义收编 | 侧栏「金融域」下出现两个都叫「金融投资」的入口，且第二个页面对外宣称的四项能力 | active | 2026-09-28 | v2.9.9 | OPC 能力包 | 本地原创 |  |
| `PLAN-horizon-four-cycle-closure.md` | PLAN：四周期（超短/短/中/长）全链路闭环收编 | 四档周期 ultra_short / short / mid / long（2 / 5 / 28 / 90 交易日）要在四条链路上做到「输入有周期 → 结论标周期 → 验证按周期 … | active | 2026-09-28 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-dcf-fallback-anchor-applicability.md` |
| `PLAN-dcf-fallback-anchor-applicability.md` | PLAN：DCF 适用性门被代理锚「自我论证」——亏损公司三档全是利好（301302 实证） | 301302（华如科技）16:54 运行 2bb5bb9d：巴菲特估值结论「21.17–71.86 元」，现价 21.72 ⇒ | active | 2026-09-27 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-live-kline-cache-calendar-storm.md` |
| `PLAN-industry-ranking-asof-gap.md` | PLAN：行业排名 as-of 合成全灭——push2\* 族本机被阻断 + 快照归属日按「今天」判致周末永不回填（G 轮） | get_industry_ranking … 板块指数日 K 全部取数失败 HTTP request failed（eastmoney 与 | active | 2026-09-27 | v2.9.9 | 行情与数据源 | 本地原创 |  |
| `PLAN-live-kline-cache-calendar-storm.md` | PLAN：live K 线缓存风暴 + t-risk 超时（301302「决策信息缺失」根因修复） | 截图：301302 live 运行「决策信息缺失」，证据审计指认 t-risk 超时 → portfolio-mgr 级联跳过。 | active | 2026-09-27 | v2.9.9 | 行情与数据源 | 本地原创 |  |
| `PLAN-margin-designified-absent-semantics.md` | PLAN：两融「设计上没有」被压成裸 null——F 轮语义分层（粤海饲料 001313 实证） | 对 18:22 运行 c6399466（粤海饲料 001313，74/B，工具可信度 58.2）的逐层核验： | active | 2026-09-27 | v2.9.9 | 行情与数据源 | 本地原创 | `PLAN-dcf-fallback-anchor-applicability.md` |
| `PLAN-asof-replay-quality-attribution.md` | PLAN：回放运行数据质量误判修复（截图 52.1/C 级归因） | 提交链：04b24a17d(S1) → e585cb489(S2) → 0123b51ba(S3) → f8e9fc568(S2 lint) → c18cba4e1(S3 判据) | active | 2026-09-26 | v2.9.9 | 时间旅行回放 | 本地原创 |  |
| `PLAN-office-auto-provision.md` | PLAN-office-auto-provision — 建房即成队：按域包自动初始化办公室成员与场景 | 1. 创建房间时自动创建/绑定本体域（OPC 行业域包）所属智能体——现状要一个个手工加； | active | 2026-09-26 | v2.9.9 | OPC 能力包 | 本地原创 |  |
| `PLAN-asof-gaps.md` | PLAN：as-of（时间旅行/回放）模式剩余取数缺口 | D 桶优先级高于 A/B：「取不到」会在降级面板留痕并让分析师写"数据缺失"；「取错」什么都不说。 | active | 2026-09-25 | v2.9.9 | 时间旅行回放 | 本地原创 | `PLAN-analysis-data-repair.md` `PLAN-stock-audit-fixes.md` `AUDIT-codebase-review-roadmap-2026-09-19.md` `AUDIT-duplication-2026-09-13.md` |
| `PLAN-codex-parity-adoption.md` | PLAN：codex 后端对齐借鉴项落地 | 日期：2026-09-25 | active | 2026-09-25 | v2.9.9 | 运行时与智能体 | 本地原创 | `AUDIT-codex-parity-gap-2026-09-24.md` |
| `PLAN-office-scene-domain-align.md` | PLAN-office-scene-domain-align — 办公室场景模板与 OPC 域包本体对齐 | 根因定性：同一概念（行业显示名）存在两套平行键体系（office.scene.* vs opc.domains.*），且后者已完备——这是 AGENTS.md 禁区 12「禁止重复… | active | 2026-09-25 | v2.9.9 | OPC 能力包 | 本地原创 | `PLAN-domain-single-source.md` `PLAN-industry-to-domain-migration.md` |
| `PLAN-decision-action-convergence.md` | 决策标签 action 语义收敛方案 | 用户提问「减持/卖出/观望/买入这些概念是不是同义却分成了不同标记」——是，且不止是命名问题。 | done | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `PLAN-serenity-fast-workflow.md` | PLAN — 趋势智选「快速趋势智选」（Jev 判定链路） | 日期：2026-09-24 | active | 2026-09-23 | v2.9.9 | 趋势智选 | 本地原创 | `PLAN-stock-analysis-fast-workflow.md` `PLAN-jev-integration.md` `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` |
| `PLAN-stock-analysis-fast-workflow.md` | PLAN — 股票分析「快速分析」（Jev 决策链路） | 日期：2026-09-24 | active | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-jev-integration.md` `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` |
| `PLAN-stock-decision-hitrate-validation.md` | PLAN：阶段 0.5 —— 单周期决策命中率验证（反思工作流增强版） | 在反思工作流已产出的验证数据上，补齐确定性判定与聚合统计，让「单周期决策准不准」有可复算、有基线的后市实证： | done | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 | `PROPOSAL-stock-decision-four-horizon.md` |
| `PLAN-stock-reflection-four-horizon.md` | 股票反思工作流四周期化调整实施计划 | 目标： 在四周期股票决策已经落地后，让反思、客观绩效判定、持久化、命中率统计和前端展示按 ultra_short / short / mid / long 分别运行，同时保持历史单… | active | 2026-09-23 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `PLAN-stock-audit-fixes.md` | PLAN — 股票业务审计缺陷修复计划 | 来源：AUDIT-stock-business-2026-09-12.md | active | 2026-09-22 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-stock-business-2026-09-12.md` `AUDIT-300642-run-variance-2026-09-22.md` |
| `PLAN-jev-integration.md` | PLAN — JEV（TypeSafe AI）接入方案 | 日期：2026-09-20 | active | 2026-09-20 | v2.9.7 | 运行时与智能体 | 本地原创 | `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` |
| `PLAN-cognitive-orchestration-fixes.md` | 认知编排分支缺陷 — 修复实施方案 | 按 implement → review 循环逐项复查，发现 3 个新缺陷（1 个自引入、1 个既有、1 个口径问题）： | active | 2026-09-17 | v2.9.7 | 运行时与智能体 | 本地原创 |  |
| `PLAN-domain-ontology-wiring.md` | PLAN：领域本体接入智能体（domain-ontology-wiring） | 把 src-tauri/crates/harness/src/domain_ontology.rs 从「只被自身单测 + JS 门禁守护」推进到「被生产链路线性消费」，理顺「能力 … | active | 2026-09-17 | v2.9.7 | OPC 能力包 | 本地原创 | `PLAN-domain-single-source.md` |
| `PLAN-evoflow-borrowings.md` | PLAN — EvoFlow 借鉴清单与落地方案 | 日期：2026-09-12 | active | 2026-09-17 | v2.9.7 | 反思与进化 | 本地原创 | `AUDIT-evoflow-vs-axinvest-2026-09-12.md` |
| `PLAN-market-sim-value.md` | PLAN — market-sim 在股票业务中的价值定位与兑现路径 | 「把模拟仿真接入股票分析工作流」这个要求本身太粗 —— 项目里有四条互相独立的模拟线，它们与工作流的关系完全不同，混为一谈会得出相反结论。 | active | 2026-09-17 | v2.9.7 | 量化回测 | 本地原创 |  |
| `PLAN-tombstone-code-wiring.md` | PLAN：墓碑代码「实现并接入」任务清单 | 日期：2026-09-17 | done | 2026-09-17 | v2.9.7 | 存储与数据库迁移 | 本地原创 | `AUDIT-tombstone-code-2026-09-16.md` |
| `PLAN-weknora-borrowings.md` | PLAN — WeKnora 借鉴清单与落地方案 | 日期：2026-09-14 | active | 2026-09-17 | v2.9.7 | 运行时与智能体 | 本地原创 | `AUDIT-weknora-vs-axinvest-2026-09-14.md` |
| `PLAN-declarative-schema-sync.md` | PLAN-declarative-schema-sync | SeaORM 2.0.1 原生支持列级改名声明，且官方 sync 就在消费它。 | done | 2026-09-16 | v2.9.7 | 构建与 CI | 本地原创 | `PLAN-domain-single-source.md` `PLAN-memory-kb-reflow-id-space.md` |
| `PLAN-agent-workbench-enhancement.md` | PLAN — 智能体工作台完善建议 | 工作台目前是「单路由 + 内存态单选 Tab」：/chat 是唯一真实路由，其余 7 个功能页被 redirectToChat 压成它的 Tab，Tab 状态只活在内存 store… | active | 2026-09-15 | v2.9.7 | 运行时与智能体 | 本地原创 |  |
| `PLAN-goink-borrowings.md` | PLAN：goink 可借鉴项落地计划 | 性质：🔴 我方自有缺陷 ｜ 🟡 借鉴对方机制 ｜ 🟢 元机制 ｜ ⛔ 反面教材（产出护栏规则） | active | 2026-09-15 | v2.9.7 | 运行时与智能体 | 本地原创 | `AUDIT-goink-vs-axinvest-2026-09-15.md` `PLAN-narrative-structure-enhancement.md` |
| `PLAN-semantica-borrowings.md` | PLAN — Semantica 借鉴清单 | 配套：AUDIT-semantica-vs-axinvest-2026-09-13.md（总报告）+ AUDIT-semantica-ontology-deep-2026-09-1… | active | 2026-09-15 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-semantica-vs-axinvest-2026-09-13.md` `AUDIT-semantica-ontology-deep-2026-09-13.md` `AUDIT-domain-semantics-fixes-2026-09-14.md` `AUDIT-layer-discipline-l1-2026-09-14.md` `AUDIT-domain-ontology-bottleneck-2026-09-14.md` `AUDIT-ontology-vs-palantir-2026-09-15.md` |
| `PLAN-p0-adjudication.md` | PLAN — 4 项 P0 待裁决项的处置建议（含一轮自我纠错） | 日期：2026-09-14（第二版，推翻第一版的核心结论） | active | 2026-09-14 | v2.9.7 | 通用 | 本地原创 | `PLAN-evoflow-borrowings.md` `AUDIT-xingye-601166-decision-2026-09-13.md` `AUDIT-sany-600031-decision-2026-09-13.md` `AUDIT-evoflow-vs-axinvest-2026-09-12.md` |
| `PLAN-VALUATION-CONFIG-WIRING.md` | PLAN — 估值配置接线（C2 路径 Z） | astock-data/src/mcp_tools.rs:1918-1920 三常量已与 analysis-engine::decision::ValueConfig | active | 2026-09-12 | v2.9.7 | 股票决策链 | 本地原创 |  |
| `PLAN-awareness-saliency.md` | 自主觉知生成 + 统一显著性仲裁器 实施方案 | 自主觉知生成 + 统一显著性仲裁器的实施方案。 | 设计定稿 | 2026-09-04 | v2.9.7 | 运行时与智能体 | 导入 |  |
| `PLAN-causal-edge-layer.md` | 因果边层（Causal Edge Layer）实施方案 | 为 knowledge_relations 增加因果语义的因果边层实施方案。 | 未标注 | 2026-09-04 | v2.9.7 | 存储与数据库迁移 | 导入 |  |
| `PLAN-codex-parity.md` | PLAN-codex-parity — Agent 框架对标 Codex 补齐方案 | 对标 Codex 的 Agent 框架能力补齐方案，按 P0–P3 依赖排序。 | 未标注 | 2026-09-03 | v2.9.7 | 运行时与智能体 | 导入 |  |
| `PLAN-cognitive-orchestration-fixes-2026-09-07-snapshot.md` | 认知编排分支缺陷 — 修复实施方案 | 认知编排分支缺陷的修复实施方案（含 3 个新缺陷与自引入回归）。 | 未标注 | 2026-09-01 | v2.9.7 | 运行时与智能体 | 导入 | `PLAN-cognitive-orchestration-fixes.md` |
| `PLAN-capability-discovery-orchestration-gap.md` | 能力发现 × 认知编排 架构差距分析与实施方案 | 能力发现与认知编排的架构差距分析与实施方案。 | 已评审 | 2026-08-30 | v2.9.7 | OPC 能力包 | 导入 |  |
| `PLAN-opc-workflow-refactor.md` | OPC 工作流重构方案 | OPC 单人公司工作流与股票分析工作流不一致的重构方案。 | 未标注 | 2026-08-28 | v2.9.7 | OPC 能力包 | 导入 |  |
| `PLAN-axinvest-type-driven-refactor.md` | AxInvest 类型驱动设计重构方案 | 上游 AxAgent 已完成类型驱动设计重构（阶段 A-D），核心框架层已实现： | active | 2026-08-19 | v2.9.5 | 构建与 CI | 本地原创 |  |
| `PLAN-refactor.md` | 实质性重构方案：从“外围防御”到“核心改造” | 从「外围防御」转向「核心改造」的实质性重构方案（魔法字符串、隐式状态机）。 | 未标注 | 2026-08-19 | v2.9.5 | 量化回测 | 导入 |  |
| `PLAN-approval-timeout-default-reject.md` | 改造清单 · 审批超时默认拒绝 | 审批超时按 timeout_action 裁决（默认拒绝）的改造清单，待评审。 | 待评审 | 2026-08-17 | v2.9.5 | 运行时与智能体 | 导入 |  |
| `PLAN-capability-supplementation.md` | 能力补齐链路补全方案 | 能力补齐链路补全方案（cognitive_query 提议通道等）。 | 未标注 | 2026-08-17 | v2.9.5 | OPC 能力包 | 导入 |  |
| `PLAN-unity-p2-runtime-permission-and-hybrid-classifier.md` | Unity P2：运行时权限覆盖 + 分类策略实装 + LLM 增强实施计划 | Unity P2：运行时权限覆盖 + 分类策略 + LLM 增强实施计划。 | 未标注 | 2026-08-17 | v2.9.5 | 量化回测 | 导入 |  |
| `PLAN-unity-p3-llm-classifier-and-approval-gate.md` | P3：LLM 分类器实装 + ApprovalGate 真实阻断 + 端到端串联 | P3：LLM 分类器实装 + ApprovalGate 真实阻断 + 端到端串联。 | 未标注 | 2026-08-17 | v2.9.5 | 模型提供商与流式 | 导入 |  |
| `PLAN-self-evolution-implementation.md` | AxAgent 自进化实施计划 | 自进化能力的分阶段实施计划（P0 双通道闭环优先）。 | 未标注 | 2026-08-16 | v2.9.5 | 反思与进化 | 导入 |  |
| `PLAN-demand-discovery.md` | OPC 需求发现智能体 · 方案设计 | OPC（一人公司）业务域已具备项目、客户、发票、看板、站点等能力，且工作流引擎（rt-workflow + WorkflowTemplateData + NodeKind 标准节点… | active | 2026-08-12 | v2.9.3 | OPC 能力包 | 本地原创 |  |
| `PLAN-narrative-structure-enhancement.md` | 文字创作工作流叙事结构增强方案 | 当前文字创作工作流（workflow-cm-literary-creation）已具备基础的“生成式写作流水线”（元认知→大纲→循环写章→评审→归档），但其核心叙事逻辑（弧线、交汇… | active | 2026-08-10 | v2.9.2 | OPC 能力包 | 本地原创 |  |
| `PLAN-ai-research-agent.md` | PLAN-ai-research-agent — AI 研究智能体 KPI 与 DAG 修复 | AI 研究智能体 KPI 真实性、status 契约与 DAG 冗余节点修复（2026-08-09 审查后遗留）。 | 未标注 | 2026-08-09 | v2.9.1 | 运行时与智能体 | 导入 |  |
| `PLAN-axagent-company-runtime.md` | AxAgent 通用能力实施：AI 公司运行时内核（Company Runtime Core） | 在 AxAgent 中引入 AI 公司运行时内核（组织自建、招聘决策、工作项状态机）。 | 待评审 | 2026-08-02 | v2.8.7 | OPC 能力包 | 导入 |  |
| `PLAN-axinvest-opc-business.md` | AxInvest OPC 实施：一人公司（行业数据资产包 + 台账 + 公司运行时内核） | D:\OneManager\AxOPC 是 AxAgent 的 fork（2.8.6），已有 372 个独有文件：opc 三 crate（~2400 行）、19 个 OPC 工具、… | active | 2026-08-02 | v2.8.7 | OPC 能力包 | 本地原创 |  |
| `PLAN-axopc-merge.md` | AxOPC 合并进 AxInvest —— 实施方案索引 | 1. AxInvest 迁移改号 v209/v210/v211（AxOPC 用 v200/v201 与 v200_axinvest_stock_tables 冲突，已到 v208）… | active | 2026-08-02 | v2.8.7 | OPC 能力包 | 本地原创 | `PLAN-axinvest-opc-business.md` |
| `PLAN-knowledge-source-ingest.md` | 知识库增长更新入口 — 实施计划 | AxAgent 已有成熟的知识底座：Wiki 双体系（经典 + LLM）、RAG 导入管道（10+ 格式 + sqlite-vec）、WebFetchTool、持久化索引队列、cr… | 已完成 | 2026-08-01 | v2.8.6 | 模型提供商与流式 | 导入 |  |
| `PLAN-phase1-platform-config-unification.md` | Phase 1: 消息渠道统一数据模型 | Phase 1：消息渠道统一数据模型。 | 未标注 | 2026-07-17 | v2.8.5 | 模型提供商与流式 | 导入 |  |
| `PLAN-phase2-activate-backend-adapters.md` | Phase 2: 激活后端适配器 | Phase 2：激活 8 个消息渠道后端适配器。 | 未标注 | 2026-07-17 | v2.8.5 | 运行时与智能体 | 导入 |  |
| `PLAN-phase3-ai-agent-message-flow.md` | Phase 3: 接入 AI Agent 消息流 | Phase 3：接入 AI Agent 消息流。 | 未标注 | 2026-07-17 | v2.8.5 | 运行时与智能体 | 导入 |  |
| `PLAN-phase4-frontend-ui-completion.md` | Phase 4: 前端完善 | Phase 4：消息渠道前端 UI 完善。 | 未标注 | 2026-07-17 | v2.8.5 | 前端外壳 | 导入 |  |
| `PLAN-hermes-borrow.md` | Hermes-Agent 借鉴功能实施计划 | 实施前对现有代码的盘点——基础设施已大量存在，核心工作是"接线 + 前端可视化"。 | 未标注 | 2026-07-11 | v2.8.3 | 前端外壳 | 导入 |  |
| `PLAN-openclaw-plugin-compat.md` | OpenClaw 插件生态全面兼容 — 实施计划 | OpenClaw 插件生态全面兼容实施计划。 | 未标注 | 2026-05-16 | v1.6.0 | 插件体系 | 导入 |  |
| `PLAN-i18n-hardcoded-strings-cleanup.md` | i18n 硬编码字符串清理 — 实施计划 | i18n 硬编码字符串清理实施计划。 | 未标注 | 2026-05-15 | v1.5.8 | 前端外壳 | 导入 |  |
| `PLAN-batch1-security-hardening.md` | Batch 1: 安全加固实现计划 | 安全加固实现计划（批次 1/3）。 | 未标注 | 2026-05-13 | v1.5.3 | 运行时与智能体 | 导入 |  |
| `PLAN-batch2-sandbox-hardening.md` | Batch 2: 沙箱加固实现计划 | 沙箱加固实现计划（批次 2/3）。 | 未标注 | 2026-05-13 | v1.5.3 | 运行时与智能体 | 导入 |  |
| `PLAN-batch3-multiagent-workflow.md` | Batch 3: 多智能体协作 + DAG 工作流鲁棒性 实现计划 | 多智能体协作 + DAG 工作流鲁棒性实现计划（批次 3/3）。 | 未标注 | 2026-05-13 | v1.5.3 | 运行时与智能体 | 导入 |  |
| `PLAN-phase5-complete-message-receiving.md` | Phase 5: 完善消息接收能力 | Phase 5：完善消息接收能力。 | 未标注 | 2026-04-30 | v1.3.0 | 运行时与智能体 | 导入 |  |

### 审计（AUDIT）

| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 功能点 | 来源 | 关联 |
|---|---|---|---|---|---|---|---|---|
| `AUDIT-l1-blockers-2026-11-04-0322.md` | L1 功能包拆分阻塞面实测（2026-11-04-0322） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-04 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-06.md` | L1 功能包拆分阻塞面实测（2026-11-06） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。**本轮两处读数值得注意**：前端反向阻塞股票侧 = **56 处 / 43 文件**、OPC 侧 = **29 处 / 11 文件**（本索引原登记的 47/34 与 28/11 是 11-05 21:52 快照的读数，差额已按提交逐条归因，见 PLAN §125.7）；而 `docs/audits/` 下 2026-11-05 的 **18 份同族快照没有登记进本索引**（脚本每次跑都无条件落一份带时间戳的文档 ⇒ 本轮验证复跑又落了一份 `…-1257.md`，内容与本行逐字相同，已删）⇒ 见任务 #207。 | active | 2026-11-06 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-04-0422.md` | L1 功能包拆分阻塞面实测（2026-11-04-0422） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-04 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-04.md` | L1 功能包拆分阻塞面实测（2026-11-04） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-04 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-03-1421.md` | L1 功能包拆分阻塞面实测（2026-11-03-1421） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-03-1557.md` | L1 功能包拆分阻塞面实测（2026-11-03-1557） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-03-1632.md` | L1 功能包拆分阻塞面实测（2026-11-03-1632） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-03.md` | L1 功能包拆分阻塞面实测（2026-11-03） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-03 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-l1-blockers-2026-11-02.md` | L1 功能包拆分阻塞面实测（2026-11-02） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-02 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-capability-inventory-2026-11-01.md` | AUDIT-capability-inventory-2026-11-01 — AxInvest 功能点全集清点（一次扫全） | 「跨面共享消费方」= 同一个前端文件（组件目录 / 页面 / store）既 invoke 了本域的某个命令，又 invoke 了别的域的命令。 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 | `PLAN-shell-plus-plugins.md` |
| `AUDIT-l1-blockers-2026-11-01.md` | L1 功能包拆分阻塞面实测（2026-11-01） | 本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。 | superseded | 2026-11-01 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |
| `AUDIT-data-outcome-coverage-2026-10-28.md` | 取数状态信封：覆盖扩展（2026-10-28） | get_index_quotes 在三条通道全部失败时曾返回 Ok(vec![])，LLM 只能看到「今天没有指数行情」， | active | 2026-10-28 | v2.9.9 | 行情与数据源 | 本地原创 |  |
| `AUDIT-gate-and-daemon-impact-2026-10-26.md` | 裁决补充：①三门放开的价值量化　②剥离常驻调度的影响面（2026-10-26 15:10） | ⇒ 上一版裁决里「G5 仍为 0」这条要撤销。 | active | 2026-10-26 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `AUDIT-gates-and-headless-scheduler-2026-10-26.md` | 实施结果：开放档内买入 + 三门可配置　/　cron 剥离为常驻调度（2026-10-26） | 默认全关 ⇒ 与引入前逐位一致（改动不改变既有行为，除非显式打开）。 | done | 2026-10-26 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `AUDIT-p0-p1-implementation-2026-10-26.md` | 交付：P0 / P1 实施结果（2026-10-26 06:40~07:30） | P0 三项全部落地（R-3h 核心机制已实跑验证：long 档首次产出 T+90 成熟验证行）， | done | 2026-10-26 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-stock-business-profitability-2026-10-26.md` `PLAN-r3h-long-sample-calibration.md` |
| `AUDIT-stock-business-profitability-2026-10-26-1440.md` | 裁决（再答）：股票业务能不能用来赚钱（2026-10-26 14:40 重新取证） | ⇒ 结论：不仅「还没火候」，而是「形态上产不出可执行买入 + 已观测的 α 为负 + 学习闭环仍恒为 0」。 | active | 2026-10-26 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-stock-business-profitability-2026-10-26.md` |
| `AUDIT-stock-business-profitability-2026-10-26.md` | 裁决（重答）：股票业务现在是否真正可以用来赚钱（2026-10-26 06:30） | ⇒ 结论不是「还没火候」，而是「当前形态即使有 α 也无法执行」+「α 本身为负」。 | active | 2026-10-26 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-stock-business-profitability-2026-10-25.md` `PLAN-r3h-long-sample-calibration.md` |
| `AUDIT-baseline-comparison-2026-10-25.md` | 基线对照审计：选择性是否创造了任何价值（2026-10-25） | 前面所有结论都基于「系统的净收益是 −0.72%」。但那个数字没有对照组： | active | 2026-10-25 | v2.9.9 | 量化回测 | 本地原创 | `AUDIT-limitup-entry-2026-10-25.md` |
| `AUDIT-factor-evidence-2026-10-25.md` | B-3 因子实证：价量双因子有真实横截面预测力（2026-10-25） | B-3 的前提是「因子能从本地数据算出来」。实测三个硬约束： | active | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `AUDIT-factor-strategy-counterfactual-2026-10-25.md` | 全市场因子选股 · 反事实回测（2026-10-25） | 前序所有结论都卡在同一个地方：系统每天只产 6~7 条可验证的真实票， | active | 2026-10-25 | v2.9.9 | 量化回测 | 本地原创 | `PLAN-b1-daily-pipeline.md` |
| `AUDIT-limitup-entry-2026-10-25.md` | AUDIT：入场价的实盘可执行性 — 2026-10-25 | 一句话：这不是「价设得太近」，是「价根本成交不了」。 | active | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-validation-fill-model.md` `AUDIT-stock-profitability-verdict-2026-10-25.md` |
| `AUDIT-replay-sample-feasibility-2026-10-25.md` | 时间旅行生成样本方案可行性评估 — 2026-10-25 | 可行，但结论要修正三处。 回放基建比你预期的成熟得多（这点你可能低估了自己已建成的东西）； | active | 2026-10-25 | v2.9.9 | 时间旅行回放 | 本地原创 | `AUDIT-stock-profitability-verdict-2026-10-25.md` `PLAN-asof-gaps.md` `AUDIT-stock-accuracy-2026-10-24.md` |
| `AUDIT-sample-validity-2026-10-25.md` | AUDIT：样本有效性 — Q-1/Q-2/Q-3 裁决 + 样本可扩的真实路径 | 「打板」意味着：以涨停封板价排队买入。它不可自动化、不可回测（成交与否取决于盘口排队与撤单速度， | active | 2026-10-25 | v2.9.9 | 量化回测 | 本地原创 | `AUDIT-limitup-entry-2026-10-25.md` `PLAN-validation-fill-model.md` |
| `AUDIT-signal-efficacy-2026-10-25.md` | AUDIT：选股信号有效性 — 2026-10-25 | 没有。confidence 与 5 日收益的相关系数 r = 0.010（n=240），统计上等于零。 | active | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-baseline-comparison-2026-10-25.md` `PLAN-a2-backfill-validation.md` |
| `AUDIT-stock-business-profitability-2026-10-25.md` | 裁决：这套系统能否在「不断迭代」中赚钱（2026-10-25 终版） | 311 条有效样本（买入、期窗匹配、已成熟），已扣 0.25% 成本： | superseded | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `AUDIT-stock-profitability-verdict-2026-10-25.md` | 股票业务盈利性裁决 — 2026-10-25 | 不能。以当前状态，「不断迭代」不是通向盈利的路径，它本身是阻碍盈利的结构性原因。 | active | 2026-10-25 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-stock-live-data-2026-10-24.md` `PLAN-capital-long-evidence.md` `PLAN-stock-decision-reliability.md` |
| `AUDIT-stock-accuracy-2026-10-24.md` | 股票业务「准 与 全」四项修复审计 — 2026-10-24 | 针对三条业务线的目标（智能荐股准确荐股且不遗漏 / 股票分析工作流给出准确决策 / | active | 2026-10-24 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `AUDIT-stock-live-data-2026-10-24.md` | 股票业务生产库核验（2026-10-24） | X-4 的「读不到生产库」已消除；三条模板都处于「代码待落库」状态，不存在 R-6 的 | done | 2026-10-24 | v2.9.9 | 行情与数据源 | 本地原创 | `AUDIT-stock-accuracy-2026-10-24.md` |
| `AUDIT-mover-miss-closure-2026-10-23.md` | AUDIT：妖股漏检后的 as-of 分析—反思—荐股反思闭环 | 日期：2026-10-23 | active | 2026-10-23 | v2.9.9 | 趋势智选 | 本地原创 | `PLAN-mover-recall-attribution.md` `PLAN-reco-reflection-closure.md` `PLAN-missed-mover-reflection-closure.md` |
| `AUDIT-run-warnings-2026-10-23.md` | 审计：10-23 运行日志 WARN 分类与定性 | 输入：C:\Users\polit\Documents\PowerShell.txt（1238 行，单次运行 10:25:15 ~ 10:35:xx，含多个 execution） | active | 2026-10-23 | v2.9.9 | 运行时与智能体 | 本地原创 |  |
| `AUDIT-verdict-shape-normalization-2026-10-23.md` | 审计：`verdict` 形态归一（value-investor 修复 + 全仓未归一面清点） | 日期：2026-10-23 | active | 2026-10-23 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `AUDIT-tasks-center-status-2026-10-22.md` | AUDIT：任务中心状态模型（2026-10-22） | 证据：cron.adapter.ts:32-40 只把每个任务的 lastResult 合成一条运行；execute_scheduled_task 对内置任务走 scheduler… | active | 2026-10-22 | v2.9.9 | 任务中心与调度 | 本地原创 |  |
| `AUDIT-horizon-naming-2026-10-16.md` | AUDIT — 股票业务四周期（超短/短/中/长）命名与时间口径一致性 | 日期：2026-10-16 | active | 2026-10-16 | v2.9.9 | 股票决策链 | 本地原创 | `PLAN-stock-decision-reliability.md` |
| `AUDIT-analyst-low-confidence-2026-10-01.md` | 分析师节点「低置信」复核（2026-10-01，两轮：600406 → 修复后复跑 600887） | 1. stock_analyses.id = a7e590a4-6239-4ffc-805a-c80aa1b9dec5（600406 国电南瑞，11:29 运行， | active | 2026-10-01 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-analyst-low-confidence-2026-09-21.md` `PLAN-live-kline-cache-calendar-storm.md` |
| `AUDIT-mover-universe-feasibility-2026-09-30.md` | AUDIT — Mover universe 可行性（Phase 0 演练）2026-09-30 | 方案一成立，但主通道必须换成腾讯批量行情：全市场 5921 只的「当日涨跌幅」用 qt.gtimg.cn/q= 批量（80 码/请求）只要 74 个请求 ≈ 42 秒，实测零失败；… | active | 2026-09-30 | v2.9.9 | 趋势智选 | 本地原创 | `PLAN-mover-recall-attribution.md` |
| `AUDIT-codex-parity-gap-2026-09-24.md` | AUDIT：openai/codex 与本项目后端的机制级差距对照 | 日期：2026-09-24 | active | 2026-09-24 | v2.9.9 | 运行时与智能体 | 本地原创 |  |
| `AUDIT-300642-valuation-leg-cancellation-2026-09-23.md` | AUDIT — 300642 透景生命「观望 + 置信 39 + 目标价缺失」根因定位 | 样本：stock_analyses / 300642 透景生命 / created_at=1790104067295（2026-09-22）/ template_version=7… | active | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 |  |
| `AUDIT-decision-chain-structural-bias-2026-09-23.md` | AUDIT — 决策链的结构性偏置（2026-09-23） | 买入档可达性：不是结构性不可达 —— 见 §3 的生产公式复算（合成场景 ⇒ 买入）。 | active | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` |
| `AUDIT-f-factor-signal-contract-2026-09-23.md` | AUDIT — f 因子链的根本原因：**信号契约缺失**（2026-09-23） | f 因子层没有「信号契约」。 每个因子各自就地决定三件事，三件事之间无统一约束、 | active | 2026-09-23 | v2.9.9 | 量化回测 | 本地原创 | `AUDIT-300642-valuation-leg-cancellation-2026-09-23.md` `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` `AUDIT-dcf-cashflow-anchor-2026-09-21.md` |
| `AUDIT-stock-data-horizons-2026-09-23.md` | AUDIT：股票数据源多周期可用性（阶段0 结论 A） | 多周期 K 线 / 指标可用。astock-data 数据层已具备按 period 参数拉取任意周期 K 线的能力， | active | 2026-09-23 | v2.9.9 | 行情与数据源 | 本地原创 | `PROPOSAL-stock-decision-four-horizon.md` |
| `AUDIT-stock-reflection-four-horizon-baseline-2026-09-23.md` | 股票反思工作流四周期化基线审计 | 日期： 2026-09-23 | active | 2026-09-23 | v2.9.9 | 反思与进化 | 本地原创 |  |
| `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` | 估值模型的计算科学性审计：参数取值、口径归属与判据作用域 | 前几轮修的都是接线（DB 旧值覆盖、字段级死链、判据失明、展示层派生）。 | active | 2026-09-23 | v2.9.9 | 股票决策链 | 本地原创 | `AUDIT-decision-chain-structural-bias-2026-09-23.md` `AUDIT-valuation-param-db-override-2026-09-21.md` `AUDIT-f-factor-signal-contract-2026-09-23.md` |
| `AUDIT-300642-run-variance-2026-09-22.md` | AUDIT — 300642 同日两次分析结论不一致 + 估值区间语义 — 2026-09-22 | 三条独立缺陷叠加，共同制造了用户看到的现象： | active | 2026-09-22 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-dcf-cashflow-anchor-2026-09-21.md` |
| `AUDIT-analyst-low-confidence-2026-09-21.md` | 股票分析「低置信」根因审计（分析师数据质量面板） | 日期：2026-09-21 | active | 2026-09-21 | v2.9.7 | 股票决策链 | 本地原创 |  |
| `AUDIT-dcf-cashflow-anchor-2026-09-21.md` | AUDIT — 「巴菲特估值」为何修了多次仍然错误（2026-09-21） | direct_fcf 在生产上恒为 None —— 因为 vendor 从来没有取过现金流量表。 | active | 2026-09-21 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-601166-valuation-decision-2026-09-14.md` |
| `AUDIT-decision-action-consistency-2026-09-21.md` | AUDIT — 决策档位展示一致性：双轴正交与派生面同步 | 「持有 vs 观望」不是两个独立语义，而是中性档（HOLD / WAIT）按持仓状态单向派生的结果： | active | 2026-09-21 | v2.9.7 | 股票决策链 | 本地原创 |  |
| `AUDIT-pledge-attribution-2026-09-21.md` | 「质押数据获取失败（工具调用被拒绝）」归因审计 | 日期：2026-09-21 | done | 2026-09-21 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-analyst-low-confidence-2026-09-21.md` |
| `AUDIT-valuation-param-db-override-2026-09-21.md` | 估值结论严重偏离现价：参数被 DB 旧值覆盖 + 适用性判据对亏损公司失明 | 用户报「华大智造现价 73.11 元，而估值结论 16.33–38.84 元」，并明确指出 | active | 2026-09-21 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-dcf-cashflow-anchor-2026-09-21.md` `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` |
| `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` | AUDIT — 股票分析工作流 Jev 适配度与耗时归因 | 日期：2026-09-20 | active | 2026-09-20 | v2.9.7 | 股票决策链 | 本地原创 | `PLAN-jev-integration.md` `AUDIT-codebase-review-roadmap-2026-09-19.md` |
| `AUDIT-metrics-definition-2026-09-20.md` | AUDIT — Metrics 定义关系核查（`MetricType` 是否真重复？） | 审计日期：2026-09-20 | active | 2026-09-20 | v2.9.7 | 量化回测 | 本地原创 |  |
| `AUDIT-codebase-review-roadmap-2026-09-19.md` | AUDIT — 全仓代码体检与发展建议（2026-09-19） | 本仓的工程质量不是瓶颈，「不变量」才是。 | active | 2026-09-19 | v2.9.7 | 构建与 CI | 本地原创 | `PLAN-declarative-schema-sync.md` `PLAN-stock-audit-fixes.md` `PLAN-decision-action-convergence.md` `PLAN-tombstone-code-wiring.md` `PLAN-weknora-borrowings.md` `PLAN-semantica-borrowings.md` |
| `AUDIT-db-upsert-dialect-2026-09-17.md` | 审计：UPSERT 裸列名的方言差异缺陷（PG 42702） | 1. 报错原文 Query Error: error returned from database: 字段关联 "communities_json" 是不明确的 at line 9… | active | 2026-09-17 | v2.9.7 | 存储与数据库迁移 | 本地原创 |  |
| `AUDIT-evolution-stats-backfill-2026-09-17.md` | 审计：`evolution_execution_stats` 历史缺口「回填」可行性 —— 结论：**无对象可回填，且物理上不可回填** | 日期：2026-09-17 | active | 2026-09-17 | v2.9.7 | 反思与进化 | 本地原创 | `AUDIT-db-upsert-dialect-2026-09-17.md` |
| `AUDIT-kg-cross-kb-merge-2026-09-17.md` | 审计：为什么「公司 / 个人 / 行业之间没有任何关联关系」（2026-09-17） | wikis 绑定关系： | done | 2026-09-17 | v2.9.7 | 存储与数据库迁移 | 本地原创 | `PLAN-declarative-schema-sync.md` `PLAN-weknora-borrowings.md` `PLAN-cognitive-orchestration-fixes.md` |
| `AUDIT-task-ledger-monotonic-2026-09-17.md` | AUDIT — task_ledger 时间戳全序化 与 disk-cache 缩进修复 的验收取证 | task_ledger.rs 的 mtime 前进到 00:53:13 已解释：worker 为逼 cargo 失效指纹而 | active | 2026-09-17 | v2.9.7 | 任务中心与调度 | 本地原创 |  |
| `AUDIT-wiki-graph-ux-2026-09-17.md` | Wiki 图谱「放大后看不清 / 卡 / 跳」的定位与修复（2026-09-17） | 1. 「看不清」的根因是标签路径分叉：大图自动聚类（aggActive=true）时节点全走 | active | 2026-09-17 | v2.9.7 | 前端外壳 | 本地原创 |  |
| `AUDIT-tombstone-code-2026-09-16.md` | AUDIT：墓碑代码全仓扫描 | 日期：2026-09-16 | done | 2026-09-16 | v2.9.7 | 存储与数据库迁移 | 本地原创 |  |
| `AUDIT-goink-vs-axinvest-2026-09-15.md` | goink 对比审计：小说书写工作流可借鉴什么 | AGPL-3.0 是传染性许可。本审计的产出边界： | active | 2026-09-15 | v2.9.7 | 运行时与智能体 | 本地原创 | `PLAN-goink-borrowings.md` `PLAN-narrative-structure-enhancement.md` |
| `AUDIT-ontology-vs-palantir-2026-09-15.md` | AUDIT — 项目本体实现 vs Palantir Ontology 方法论 | 核心命题：传统平台建模"名词"，Ontology 建模"决策"。 Action 是独立一等类型，不是实体类上的方法（与 DDD 富血模型有本质区别）。 | active | 2026-09-15 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-semantica-ontology-deep-2026-09-13.md` `PLAN-semantica-borrowings.md` `AUDIT-domain-ontology-bottleneck-2026-09-14.md` |
| `AUDIT-wiki-graph-edges-2026-09-15.md` | AUDIT — Wiki 图谱「没有边 / 没有分组 / 一片无关系的点」 | 对标对象：WeKnora（output/tmp-weknora-src/WeKnora-main，MIT，仅作设计参照，不搬运代码） | active | 2026-09-15 | v2.9.7 | 存储与数据库迁移 | 本地原创 |  |
| `AUDIT-601166-chain-break-2026-09-14.md` | AUDIT — 601166 决策链断裂（`bear-r3` 单点失败掐断全链） | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 股票决策链 | 本地原创 |  |
| `AUDIT-601166-valuation-decision-2026-09-14.md` | AUDIT — 601166 估值/决策/分析师产出偏离根因（2026-09-14） | ⇒ 「启动失败 / 节点不存在」类故障已闭环。本轮问题在下游语义层。 | active | 2026-09-14 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-industry-label-removal-2026-09-14.md` |
| `AUDIT-ci-drift-skill-desc-2026-09-14.md` | 元设施维护：CI 门禁清单漂移修复 + 技能 description 拆分（2026-09-14） | 改动文件： | active | 2026-09-14 | v2.9.7 | 构建与 CI | 本地原创 |  |
| `AUDIT-data-quality-dual-algorithm-2026-09-14.md` | 数据质量双算法审计与统一（2026-09-14） | 方法论差异不是「参数不同」，是「测的对象不同」：前者测「这批证据能不能信」，后者测「这个 JSON 长得规不规范」。 | active | 2026-09-14 | v2.9.7 | 数据质量与门禁 | 本地原创 |  |
| `AUDIT-domain-ontology-bottleneck-2026-09-14.md` | AUDIT — 瓶颈掘金领域本体（真 O1：类 / 关系 / 公理 + 副本收敛 + CI 一致性门） | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `PLAN-semantica-borrowings.md` |
| `AUDIT-domain-semantics-fixes-2026-09-14.md` | AUDIT — 领域语义四项裁决执行（2026-09-14） | 上游报告：AUDIT-domain-semantics-o1-2026-09-14.md（O1 注册表 + O2 扫描器） | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-domain-semantics-o1-2026-09-14.md` `PLAN-semantica-borrowings.md` `AUDIT-layer-discipline-l1-2026-09-14.md` |
| `AUDIT-domain-semantics-o1-2026-09-14.md` | AUDIT — 领域语义层（O1 概念注册表 + O2 扫描器 v2） | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `PLAN-semantica-borrowings.md` |
| `AUDIT-explainer-completion-and-profile-maps-2026-09-14.md` | 审计：explainer 节点补全 + 前端 profile 映射表补齐（2026-09-14） | 两层都必须修，只修①等于「让说明书从勉强能读变成写得更好」，用户仍看不到。 | active | 2026-09-14 | v2.9.7 | 运行时与智能体 | 本地原创 |  |
| `AUDIT-industry-label-removal-2026-09-14.md` | AUDIT — 判据去行业标签化（601166 争议的第二轮） | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-601166-valuation-decision-2026-09-14.md` |
| `AUDIT-invest-scroll-2026-09-14.md` | AUDIT — 投资中心缺少垂直滚动条 / 下部内容被折叠（2026-09-14） | 用户报告：投资中心页（/invest）无垂直滚动条，工作区等视图下部内容被折叠不可见（截图：工作区分析视图底部「看空/看多」比例条被硬裁）。 | active | 2026-09-14 | v2.9.7 | 前端外壳 | 本地原创 |  |
| `AUDIT-layer-discipline-l1-2026-09-14.md` | AUDIT — 模块级分层护栏（L1）落地与实测（2026-09-14） | 上游计划：PLAN-semantica-borrowings.md 的 L1 节（借鉴清单里最后一条未执行的 P0） | active | 2026-09-14 | v2.9.7 | 构建与 CI | 本地原创 | `PLAN-semantica-borrowings.md` `AUDIT-domain-semantics-fixes-2026-09-14.md` |
| `AUDIT-ontology-abc-implementation-2026-09-14.md` | A / B / C 三组本体接线实施报告（2026-09-14） | 病灶：graph_dtos.rs 原 compute_type_affinity 把一条不相交兼容公理 | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-ontology-vs-existing-graph-structures-2026-09-14.md` `AUDIT-ci-drift-skill-desc-2026-09-14.md` |
| `AUDIT-ontology-p0123-execution-2026-09-14.md` | 执行记录：本体适应性接线 P0–P3（2026-09-14） | 最重要的一个发现（P2-b）：边侧的问题不是「前端配色没跟上」，而是 | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-ontology-recap-adaptive-wiring-2026-09-14.md` `AUDIT-ontology-abc-implementation-2026-09-14.md` |
| `AUDIT-ontology-recap-adaptive-wiring-2026-09-14.md` | 复盘：本体落地后的**业务层 / 基础能力层适应性接线**审计（2026-09-14） | DB 侧当轮实测（原样）： | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-ontology-p0123-execution-2026-09-14.md` |
| `AUDIT-ontology-vs-existing-graph-structures-2026-09-14.md` | AUDIT — 本体 vs 现有 DAG / 知识图谱 / WIKI 的关系 | 这个代码库里已经有三套类型系统在跑，但它们全部只描述「载体/过程」（这是哪种节点、怎么流转）， | active | 2026-09-14 | v2.9.7 | OPC 能力包 | 本地原创 |  |
| `AUDIT-scrollbar-visibility-2026-09-14.md` | AUDIT — 滚动指示器跨主题不可见（2026-09-14） | 投资中心内容区滚动生效后，滚动指示器在多种主题下都难以辨认（用户原话： | active | 2026-09-14 | v2.9.7 | 前端外壳 | 本地原创 | `AUDIT-invest-scroll-2026-09-14.md` |
| `AUDIT-weknora-vs-axinvest-2026-09-14.md` | AUDIT — WeKnora vs AxInvest 源码级对比审计（RAG / Wiki 专项） | 日期：2026-09-14 | active | 2026-09-14 | v2.9.7 | 运行时与智能体 | 本地原创 |  |
| `AUDIT-crdt-integration-2026-09-13.md` | CRDT 接入与缺陷修复 — 2026-09-13 | 这次接入是必要的 —— 一接入就暴露 crates/crdt 根本无法编译（D1），另有三个语义缺陷（D2–D4）。 | active | 2026-09-13 | v2.9.7 | 存储与数据库迁移 | 本地原创 |  |
| `AUDIT-duplication-2026-09-13.md` | AxInvest 重复功能 / 重复代码 全项目审计 | 日期：2026-09-13 | active | 2026-09-13 | v2.9.7 | 构建与 CI | 本地原创 |  |
| `AUDIT-sany-600031-decision-2026-09-13.md` | 三一重工 600031 决策复核 + Switch 默认分支断链修复 | 界面那张卡片是对的（公式链产出），库里那条是错的。根因不在业务逻辑，而在工作流引擎： | active | 2026-09-13 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-xingye-601166-decision-2026-09-13.md` |
| `AUDIT-scheduled-tasks-2026-09-13.md` | 定时任务体系审计 — AxInvest（2026-09-13） | 关键的「声明 vs 运行」对照（修后）：with_task_type 全集（10 个）现已全部有落点 —— 6 个走专属分支，3 个（opc-demand-discovery / … | active | 2026-09-13 | v2.9.7 | 任务中心与调度 | 本地原创 |  |
| `AUDIT-semantica-layering-deep-2026-09-13.md` | AUDIT — Semantica 分层维度专项深度对比（2026-09-13 · 三轮） | core/__init__.py:51-63 导出 Config, ConfigManager, LifecycleManager, PluginRegistry, Semanti… | active | 2026-09-13 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-semantica-vs-axinvest-2026-09-13.md` |
| `AUDIT-semantica-ontology-deep-2026-09-13.md` | AUDIT（专项）— Semantica 的 Ontology 层 vs AxInvest | 用户在上一轮指出「分析不够深入，其 ontology 应该是明显优于本地的」。这个判断成立，我上一轮的结论是错的。 | active | 2026-09-13 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-semantica-vs-axinvest-2026-09-13.md` `PLAN-semantica-borrowings.md` |
| `AUDIT-semantica-vs-axinvest-2026-09-13.md` | AUDIT — Semantica vs AxInvest（源码级对比审计） | 结论：这是一个「设计意图优秀、推理与管道层实现大量悬空」的项目 —— 但本体层是显著例外，它是完整的、可发布、且被系统性治理过的。可以借它的「机制设计」，绝不能借它的「推理/管道实… | active | 2026-09-13 | v2.9.7 | OPC 能力包 | 本地原创 | `AUDIT-semantica-ontology-deep-2026-09-13.md` `AUDIT-semantica-layering-deep-2026-09-13.md` |
| `AUDIT-xingye-601166-decision-2026-09-13.md` | 兴业银行（601166）决策一致性审计 | 审计日期：2026-09-13 | active | 2026-09-13 | v2.9.7 | 股票决策链 | 本地原创 | `AUDIT-stock-business-2026-09-12.md` |
| `AUDIT-evoflow-vs-axinvest-2026-09-12.md` | AUDIT — EvoFlow vs AxInvest 源码级对比审计 | 日期：2026-09-12 | active | 2026-09-12 | v2.9.7 | 反思与进化 | 本地原创 | `PLAN-evoflow-borrowings.md` |
| `AUDIT-stock-business-2026-09-12.md` | AxInvest 股票业务系统性审计报告 | 日期：2026-09-12 | active | 2026-09-12 | v2.9.7 | 股票决策链 | 本地原创 | `PLAN-stock-audit-fixes.md` |
| `AUDIT-code-review-2026-08-14.md` | AxAgent 项目缺陷全面审查报告 | 本次审查以项目 AGENTS.md 定义的规范与架构铁律为基准，结合静态检查与模块级源码走查。 | 已审查 | 2026-08-14 | v2.9.4 | 运行时与智能体 | 导入 |  |
| `AUDIT-plugin-refactor-2026-08-14.md` | 「一切皆插件」重构 P0–P3 完成度与缺陷核查报告 | 「一切皆插件」重构 P0–P3 完成度与缺陷核查报告。 | 已实施 | 2026-08-14 | v2.9.4 | 插件体系 | 导入 |  |
| `AUDIT-self-evolution-gap-2026-08-13.md` | 自进化差距分析与改进方案 | 对标 DeepSeek Harness 的自进化能力差距分析与改进建议。 | 未标注 | 2026-08-13 | v2.9.3 | 反思与进化 | 导入 |  |
| `AUDIT-architecture-2026-07-04.md` | AxAgent 架构审计报告 | 按架构核心理念与审计标准对 AxAgent 的架构审计结论。 | 未标注 | 2026-07-04 | v2.7.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-full-feature-defect-2026-07-04.md` | AxAgent 全面功能缺陷审计报告（更新版） | 本次全链路审计发现 139 个功能缺陷（含上次审计残留和新增），按严重程度分布： | 未标注 | 2026-07-04 | v2.7.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-file-git-tools-2026-07-02.md` | 文件操作 & Git 工具缺陷审计报告 | 发现 8 个缺陷：4 个 P0（高危）、2 个 P1（中危）、2 个 P2（低危）。 | 未标注 | 2026-07-02 | v2.7.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-full-feature-defect-2026-07-02.md` | AxAgent 全面功能缺陷审计报告 | 本次全链路审计发现 78 个功能缺陷，按严重程度分布： | 未标注 | 2026-07-02 | v2.7.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-harness-engineering-2026-06-24.md` | AxAgent Harness 工程符合度审计报告 | Harness 工程符合度审计报告（全栈符合度评分与差距）。 | 未标注 | 2026-06-24 | v2.6.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-workflow-editor-defect-2026-06-15.md` | 工作流编辑器缺陷审查报告 | 工作流编辑器缺陷审查报告（节点类型、连线与状态同步）。 | 未标注 | 2026-06-15 | v2.6.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-full-code-2026-06-11.md` | AxAgent 全面代码审计报告 | AxAgent 全面代码审计报告（Harness 剥离、缺陷分级与修复状态）。 | 未标注 | 2026-06-11 | v2.6.0 | 运行时与智能体 | 导入 |  |
| `AUDIT-harness-architecture-2026-06-04.md` | Harness 架构全面审查报告 | Harness 架构全面审查报告（RAG、工具与分层合规）。 | 未标注 | 2026-06-04 | v2.6.0 | 构建与 CI | 导入 |  |
| `AUDIT-frontend-backend-2026-04-29.md` | 前后端代码批量不一致检测报告 | 前后端 IPC 契约批量不一致检测报告（invoke 调用 vs 命令注册）。 | 未标注 | 2026-04-29 | v1.2.7 | 前端外壳 | 导入 |  |

### 参考（REFERENCE）

| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 功能点 | 来源 | 关联 |
|---|---|---|---|---|---|---|---|---|
| `dependency-graph.md` | AxAgent 仓库依赖图与拆分分析 | 仓库依赖图与按技术层拆分可行性分析（参考） | active | 2026-10-31 | v3.0.0 | 构建与 CI | 本地原创 |  |
| `dependency-matrix.md` | AxAgent Crate 精确依赖矩阵（实测） | 逐 crate 读取 Cargo.toml 得到的精确依赖矩阵（2026-10-31 实测） | active | 2026-10-31 | v3.0.0 | 构建与 CI | 本地原创 |  |
| `gate-coverage-ledger.md` | 门禁覆盖台账（gate-coverage-ledger） | CI / 本地门禁覆盖面台账与补齐进度 | active | 2026-10-31 | v3.0.0 | 构建与 CI | 本地原创 | `PLAN-declarative-schema-sync.md` |
| `workflow-schema.md` | 工作流 Schema 文档 | 工作流节点 Schema 定义（供编辑器与校验共用） | active | 2026-10-30 | v2.9.9 | 运行时与智能体 | 本地原创 |  |
| `laap-agi-移植评估.md` | laap-AGI 认知分析报告 | laap-AGI 认知分析能力的移植可行性评估。 | 未标注 | 2026-09-04 | v2.9.7 | 运行时与智能体 | 导入 |  |
| `comparison-hermes-agent.md` | Hermes-Agent vs AxAgent 全面对比报告 | Hermes-Agent 与 AxAgent 的全面对比及可借鉴能力清单。 | 未标注 | 2026-07-11 | v2.8.3 | 插件体系 | 导入 |  |
| `companion-window.md` | Companion 悬浮窗可行性调研 | nomifun-tauri 的 nomifun-companion 实现了桌面悬浮 AI 助手窗口（类似 Claude Desktop 的悬浮助手），macOS 用 macos-p… | 未标注 | 2026-07-06 | v2.8.0 | 前端外壳 | 导入 |  |
| `workflow-vs-n8n.md` | AxAgent 工作流 vs n8n 对比分析 | AxAgent 工作流与 n8n 的能力对比分析。 | 未标注 | 2026-07-06 | v2.8.0 | 运行时与智能体 | 导入 |  |
| `RELEASE.md` | AxAgent 版本发布流程 | AxAgent 版本号规范、发布节奏与冻结/观察窗口流程。 | 未标注 | 2026-07-02 | v2.7.0 | 构建与 CI | 导入 |  |
| `architecture_report.md` | AxAgent 项目架构分析报告 | AxAgent 项目架构分析报告（模块划分、依赖关系与数据流梳理）。 | 未标注 | 2026-07-01 | v2.7.0 | 运行时与智能体 | 导入 |  |

附：`docs/reference/` 含 2 个 schema 样例（`workflow-schema.json`、`skill-manifest-v2-example.json`）。

### marketing

| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 功能点 | 来源 | 关联 |
|---|---|---|---|---|---|---|---|---|
| `PROMO.md` | AxAgent ⚡ AI Native OS | 产品对外介绍（内容停留在 v2.5.0 / 32 crates，已过期，待重写） | active | 2026-09-05 | v2.9.7 | 通用 | 本地原创 |  |
