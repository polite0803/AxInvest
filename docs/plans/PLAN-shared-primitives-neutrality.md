# PLAN · 共享基元的逐符号中性判定（B8-4 前置）

> 状态：proposed ｜ 日期：2026-11-05 ｜ 对应版本：v3.0.0+（未定版）
> 上游：`PLAN-shell-plus-plugins.md` §91.4-2（「20 处共享基元的逐函数中性判定」未做）与 §91.3 的 **B8-4**（共享基元下沉，预期 −8～−12）。
> 本文件**只做调研与判定，不改任何代码**；`docs/INDEX.md` 待 leader 登记。

---

## 0. 口径与复算（读数可复算，不手抄）

| 项 | 值 |
|---|---|
| 判据来源 | `scripts/audit-l1-blockers.mjs:69` 的 `feRe = /stock\|quote\|kline\|technical\|portfolio\|trading\|advis\|invest\|quant\|market/i`，按**文件路径名**判域归属，「非域文件 import 域文件」计 1 处 |
| 本次命令 | `node scripts/audit-l1-blockers.mjs` |
| 快照落点（脚本正常副作用，未删未提交） | `docs/audits/AUDIT-l1-blockers-2026-11-05-1126.md` |
| 前端股票反向阻塞 | **51 处 / 37 文件**（与任务给的数一致） |
| 同快照其余读数 | 前端 OPC 28 处 / 11 文件；Rust 待改写 99 处 / 16 文件；H 桶 3 处 / 3 文件 |

**复算片段**（与快照逐个对上，51 = 51；未落盘，直接 `node -e` 跑）：
对每个 `src/**/*.{ts,tsx}` 且路径不含 `feRe` 的文件，逐行跑 `/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g`，命中 specifier 再套 `feRe`。下表每一行的行号均出自该复算输出（给号引擎：脚本内 `i + 1`；人工核对用 `awk 'NR==<行>{print}'`）。

**判定三分**：

- **中性 → 可下沉**：符号本身不含股票值域（无 stock 枚举、无 `stockAnalysis.*` i18n 键、无「分析师/档位/动作/涨跌」语义），搬进中性目录后语义不变。
- **域特定 → 随插件迁走**：符号值域是股票业务（后端 `Style` / `ActionKind` / 四档持有期 / 分析师节点 id / 投资中心路由 tab）。**单纯换目录是洗绿**。
- **判据误报**：命中 `feRe` 的关键词与股市无关（quote=引用回复、market=插件市场/人才市场），或导入方与目标**同属另一个域**。

---

## 1. 结论先行：所谓「20 处共享基元」实测只有 2 处

| 类 | 处数 | 文件数 | 处置 | 归入批次 |
|---|---|---|---|---|
| ① 判据假阳性（`feRe` 命中关键词但非股市） | **7** | 6 | 不改代码、不改判据，只在审计判读口径登记 | — |
| ② **中性 → 可下沉** | **2** | 2 | `PageHeader`、`clearVendorCheckCache` | B8-4a |
| ③ 域特定但**导入方也是域自有**（迁出后自然消失，不需要「沉」） | **20** | 17 | 跟随股票前端打包迁出；其中 2 处是**死代码** | B8-2（2 处可先删） |
| ④ 域特定且**宿主文件内联域逻辑**（真耦合，不是基元问题） | **22** | 12 | 页面/任务中心走插件贡献面；barrel 走 B8-5 | B8-1/B8-2/B8-5 |
| **合计** | **51** | **37** | | |

> 文件数不按类相加（`src/components/time-travel/ReplayWorkbench.tsx` 同时出现在 ② 与 ③）：6 + 2 + 17 + 12 = 37。

**直接后果**：B8-4「共享基元下沉 −8～−12」在这个桶里**不成立** —— 真基元只有 2 处（−2）。§91.2 ② 列的 20 处里，13 处是 `@/lib/stock-analysis-utils`，而该文件的每个被引符号都带股票值域（见 §3 的反面判据），整文件搬或改名都是把耦合藏起来，读数下降但语义没变。把 −8～−12 记到 B8-4 账上会让后人对不齐；建议把该批的预期改写为 **−2（下沉）+ 可选的 −3（删死代码/改测试夹具）**，其余计入 B8-2/B8-5。

---

## 2. 逐符号判定表（51 行，一行一处；同一行引用多个符号则并列）

### 2.1 ① 判据假阳性（7 处 / 6 文件）—— 不改代码

| # | 引用位置 | 被引用符号 | 判定 | 依据（原文 / 调用点） | 难度 | 批次 |
|---|---|---|---|---|---|---|
| A1 | `src/components/chat/ChatViewMessages.tsx:86` | `QuoteBlock` | 误报 | `src/components/chat/CiteBlock.tsx:3`「引用回复：消息气泡内显示被引用消息的预览块」——`quote` 是**消息引用**，与行情无关（原锚 QuoteBlock.tsx 第 3 行，2026-11-11 #263 改名后随迁，见下方注记） | — | 不改 |
| A2 | `src/components/chat/InputArea.tsx:69` | `QuotePreviewBar` | 误报 | `src/components/chat/CitePreviewBar.tsx:17`「引用回复预览条：显示当前被引用的消息内容，可一键取消引用」（原锚 QuotePreviewBar.tsx 第 17 行，同上） | — | 不改 |
| A3 | `src/components/settings/index.ts:30` | `../chat/PluginMarketplace` | 误报 | `src/components/chat/PluginGallery.tsx:112`（原锚 `PluginMarketplace.tsx`，2026-11-12 §159 改名刀随迁）取键 `chat.plugins.marketplace.title`——`market` = **插件市场** | — | 不改 |
| A4 | `src/pages/SettingsPage.tsx:71` | `@/components/chat/PluginMarketplace`（动态 `import()`） | 误报 | 同上（同一组件，另一处装载） | — | 不改 |
| A5 | `src/components/settings/WorkflowSettings.tsx:5` | `@/pages/WorkflowMarketplace` | 误报 | `src/pages/WorkflowGallery.tsx:18`（原锚 `WorkflowMarketplace.tsx`，2026-11-12 §159 改名刀随迁）「目录条目类型，与后端 `axagent_harness::marketplace::CatalogItem` 对齐」= 工作流模板市场 | — | 不改 |
| A6 | `src/pages/opc/OpcPage.tsx:22` | `./components/MarketPackTab` | 误报（**两域判据不同源**） | `src/pages/opc/components/MarketPackTab.tsx:20` 调 `invoke("opc_market_list")`——`market` 是 OPC 的**能力市场**；导入方与目标同属 OPC | — | 不改（前端补 `cross` 桶另行裁定） |
| A7 | `src/pages/opc/OpcPage.tsx:25` | `./components/TalentMarketTab` | 误报（同上） | `src/pages/opc/components/TalentMarketTab.tsx:10` + `TALENT_CATEGORIES`——人才市场 | — | 不改 |

> 与 §91.2 ① 的差异：§91 记 6 处，漏了 A5（`WorkflowMarketplace` 同样是 marketplace 假阳性）。今天实为 **7 处 / 6 文件**。

> **2026-11-11 回写注记（快照保留原样，本注记消腐烂）**：#263 前端刀（提交 `e5e22ba97`）已将 `QuoteBlock`/`QuotePreviewBar` 改名 `CiteBlock`/`CitePreviewBar` —— **A1/A2 的「不改」判定被推翻**：改名后两文件不再命中股票域词面，前端反向 STOCK −2/−2（33/27 → 30/24）。A1/A2 行内证据锚已随改名改指新文件（`CiteBlock.tsx:3` / `CitePreviewBar.tsx:17`，头部注释文字与行号未变，锚点仍成立）；「引用位置」列的 86 / 69 是 2026-11-05 时点行号，现 import 行在 `ChatViewMessages.tsx:82` / `InputArea.tsx:56`。

### 2.2 ② 中性 → 可下沉（2 处 / 2 文件）

| # | 引用位置 | 被引用符号 | 判定 | 依据（原文 / 调用点） | 难度 | 批次 |
|---|---|---|---|---|---|---|
| B1 | `src/components/time-travel/ReplayWorkbench.tsx:1` | `PageHeader` | **中性→可下沉**（**B8-4a 已下沉**） | `src/components/common/PageHeader.tsx` 只渲染「返回键 + `t(titleKey)` 标题 + `meta` + `actions` + 可选锚点」，无任何域语义；被 4 个 stock 页 + `ReplayWorkbench` + `src/components/quant/QuantLab.tsx:13` 共用 | 中（锚点槽 + CSS 寄生，见 §4.1） | B8-4a |
| B2 | `src/components/settings/DataVendorsTab.tsx:446` | `clearVendorCheckCache` | **中性（机制中性，宿主调用）** | `src/components/stock-analysis/vendorCheck.ts:59-61` 函数体只有 `enabledCache = null;`；调用点 `src/components/settings/DataVendorsTab.tsx:444-448` 的语义是「保存 vendor 配置后失效缓存」，与股票无关 | 低-中（改成中性失效事件，见 §4.2） | B8-4a |

### 2.3 ③ 域特定，但导入方也是域自有（20 处 / 17 文件）—— 迁出后自然消失

> 这 20 处的共同特征：**`feRe` 只认路径名，没认出这些文件本身也是股票域**（`src/lib/`、`src/components/time-travel/`、`src/components/office/panels/`、`src/stores/feature/` 这些路径没有股票关键词）。它们的 import 是**域内引用**，跟着股票前端打包一起搬走即可，不是「基元放错地方」。

| # | 引用位置 | 被引用符号 | 判定 | 依据（原文 / 调用点） | 难度 | 批次 |
|---|---|---|---|---|---|---|
| C1 | 原 `src/components/office/panels/` 轨迹面板（**2026-11-08 已删**，刀 A） | `TimelineNode` / `TimelinePhase` | 域特定（**死代码**） | 文件头「agent 协作轨迹 + **股票业务**时间线合并面板」，数据源 `stockAnalysisStore.timeline` 的四阶段 `scan/diagnose/debate/decide`；`git grep -n "TrajectoryPanel" -- src` 只命中自身 → **全仓零消费**（删前 2026-11-08 复验仍只命中注释） | 低（判死后删除） | B8-4a |
| C2 | 原 `src/components/office/panels/` 策略迷你条（**2026-11-08 已删**，刀 A） | `investPath` | 域特定（**死代码**） | 文件头「仅在投研办公室场景…数据源 strategyStore（quant）…跳转到 quant 页面」；`git grep` 只命中自身 → 零消费 | 低（判死后删除） | B8-4a |
| C3 | `src/components/time-travel/batchTables.tsx:3` | `horizonLabelKey` | 域特定（导入方亦域） | 列定义含 `stockCode/stockName/confidence`（同文件 `:20-41`）；`horizonLabelKey` 返回 **`stockAnalysis.timeHorizon*`**（`src/lib/stock-analysis-utils.ts:1930-1934`） | 低（随包） | B8-2 |
| C4 | `src/components/time-travel/TimeTravelBatchCard.tsx:4` | `horizonLabelKey` | 域特定（导入方亦域） | 同文件 `:224` `t(horizonLabelKey(p) ?? p)`；类型 `TimeTravelBatchAnalysis`（`src/types/timeTravel.ts:26`）逐股一行 | 低（随包） | B8-2 |
| C5 | `src/components/time-travel/ReplayWorkbench.tsx:6` | `investPath` | 域特定（导入方亦域） | `investPath` 值域 = `InvestTab` 联合类型（`src/lib/investRoutes.ts:26-32`：`screener / workspace / cross-market / screenshot-diagnosis / paper-portfolio / quant`） | 低（随包） | B8-2 |
| C6 | `src/lib/agentOutput.ts:20` | `alignReasoningDecisionLabel`、`analystBaseOf`、`HORIZON_CAMEL_TO_SNAKE`、`parseAction`、`parsePositionState`、`parseRiskLevel`、`resolveDisplayAction` | 域特定（**整文件域**） | `git grep -n "agentOutput" -- src` 的消费方全部是域文件：`src/components/stock-analysis/**`、`src/components/stock-workspace/DecisionHeroBar.tsx`、`src/stores/feature/stockAnalysisStore.ts`、`src/stores/feature/stockWorkflowChatBridge.ts`、`src/lib/{decision-agreement,risk-assessment,stock-analysis-utils}.ts`、本文件测试。文件内通用名函数（`:61 cleanToolCallTags`、`:115 tryBeautifyJson`、`:208 extractContent`、`:648 parseJsonLoose`）**也没有任何非域消费方** | 中（随包 + 拆通用半区可选） | B8-2 |
| C7 | `src/lib/agentOutput.ts:29` | `F7ReferenceSnapshot`、`HorizonAgreement`、`HorizonDivergence`、`LlmDecisionsByHorizon`、`LlmHorizonDecision`、`LlmHorizonReadState`、`StockDecision` | 域特定 | `normalizeDecision(): StockDecision`（同文件 `:247`）、`normalizeHorizonPriceMap(): StockDecision["horizonPriceMap"]`（`:46`） | 中 | B8-2 |
| C8 | `src/lib/dataQualityDiagnosis.ts:15` | `analystBaseOf` | 域特定 | 同文件 `:28-39` 的 `EXPERT_ID_TO_DQ_ABBR` 逐条列 10 个分析师槽位（`a-market-analyst` / `a-sentiment` / …）；消费方仅 `src/components/stock-analysis/**` | 低 | B8-2 |
| C9 | `src/lib/decision-agreement.ts:37` | `parseActionStrict`、`parseRiskLevel`、`StockAction`、`StockRiskLevel` | 域特定 | 模块头 `:34`「方向判据交给权威归一化器 `parseActionStrict`」；`:44 AGREEMENT_WEIGHTS` 逐维复刻后端股票决策一致性分值；消费方仅 `stockAnalysisStore` + 本文件测试 | 低 | B8-2 |
| C10 | `src/lib/decision-agreement.ts:37` | `StockActionType` | 域特定 | 同上 | 低 | B8-2 |
| C11 | `src/lib/decisionInputDiagnosis.ts:21` | `analystBaseOf`、`analystNodeId` | 域特定 | 模块头 `:2-3`「把 **portfolio-mgr** 实际消费的 16 个上游节点输出…」；`:103` `analystNodeId(nodeId, "mid")` 用四档之一的 `mid` | 低 | B8-2 |
| C12 | `src/lib/__tests__/agentOutput.test.ts:16` | `StockDecision` | 域特定（域测试） | 被测模块 `@/lib/agentOutput` 为域文件（见 C6） | 低 | B8-2 |
| C13 | `src/lib/__tests__/analysisHistoryFilter.test.ts:12` | `filterAnalysisHistory`、`getActionTKey`、`horizonOfRecord`、`mergeAnalysisHistoryPages`、`provenanceOfRecord`、`StockAction`、`UNKNOWN_HORIZON` | 域特定（域测试） | 同文件 `:14-18`「历史分析弹窗『全量加载 + 筛选』的判据层」 | 低 | B8-2 |
| C14 | `src/lib/__tests__/analystSentiment.test.ts:11` | `classifyAnalystReport`、`computeStockConsensus`、`deriveAnalystConsensus`、`extractBullBearScores`、`groupAnalystReportsByTier`、`scopeAnalystReports`、`tallyAnalystSentiment` | 域特定（域测试） | 计票口径单测（`:1` i18n-exempt 头注释） | 低 | B8-2 |
| C15 | `src/lib/__tests__/analystTallyRealFixtures.test.ts:12` | `classifyAnalystReport`、`classifySentiment`、`extractBullBearScores`、`normalizeVerdictScore`、`tallyAnalystSentiment` | 域特定（域测试） | 夹具是 2026-10-10 立讯精密（002475）真实分析产物（`:3-4`） | 低 | B8-2 |
| C16 | `src/lib/__tests__/evidenceWeights.pluginRoute.test.ts:11` | `computeEvidenceWeights`、`EvidenceWeightRequest` | 域特定（域测试） | 断言走**股票插件命令**（`:4-9`「`computeEvidenceWeights` 走插件命令…」） | 低 | B8-2 |
| C17 | `src/theme/__tests__/tokenSingleSource.test.ts:14` | `RECO_STYLE_HUE`、`RECO_STYLE_ALIASES` | 域特定（**可夹具化**） | 值域 = 后端 `Style` 的 11 个变体（`src/lib/stock-analysis-utils.ts:867-882`）。测试只拿它当「不得出现第二张风格色表」的**样本**（`:211`、`:231`、`:250-257`），换合成常量同样能保住负控 | 低 | B8-4a（−1，非下沉） |
| C18 | `src/utils/__tests__/workflowNodeLabel.test.ts:1` | `HORIZON_T_SUFFIX` | 域特定 | 被测文件 `src/utils/workflowNodeLabel.ts:32-35` **自带第二张档位表**并硬编码 `stockAnalysis.*` 键；该 util 除本测试外**零消费**（`git grep -n "getWorkflowNodeLabel" -- src` 只命中本测试） | 低-中（判死或随包） | B8-4a（判死）或 B8-2 |
| C19 | `src/stores/feature/screenshotDiagnosisStore.ts:19` | `PaperPortfolio`（类型） | 域特定 | store 唯一消费方是 `src/components/stock-analysis/ScreenshotDiagnosisDashboard.tsx:15`；`PaperPortfolio` 对齐 `axagent_entities::paper_portfolios::Model`（`src/types/paper-portfolio.ts:5-7`） | 中（若要留壳须走 schema-gen） | B8-2 |
| C20 | `src/components/office/phaser/__tests__/sceneTemplates.labelKey.test.ts:4` | `INVESTMENT_OFFICE_TEMPLATE` | 域特定（**内容型**） | 模板 6 房间 = research / data_room / meeting / strategy / trading / risk（`src/components/office/phaser/investSceneTemplates.ts:33-95`）；注册口 `registerSceneTemplate` 本身中性（`:103`） | 低-中 | B8-0/未定（见 §7 第 4 条） |

### 2.4 ④ 域特定且宿主文件内联域逻辑（22 处 / 12 文件）—— 真耦合，走贡献面

| # | 引用位置 | 被引用符号 | 判定 | 依据（原文 / 调用点） | 难度 | 批次 |
|---|---|---|---|---|---|---|
| D1 | `src/components/layout/ContentArea.tsx:367` | `InvestPage`（历史注释） | **已收口（2026-11-18 复核）** | 投资主页装载权已交 `src/pages/invest/routes.tsx` 的域声明；宿主只剩 `/invest/replay` 子路由与这行历史注释，已无 `InvestPage` lazy import/路由 | — | 已收口 |
| D2 | ~~`src/pages/ScreenerPage.tsx`~~ **已注销** | `stock-analysis/ScreenerPage` | **已收口（2026-11-08）** | 5 行纯转发薄壳已删除，改由 `src/components/invest/InvestHub.tsx:15` 直取域组件（无行为变化）⇒ 该处反向 import 归零；文件名不含 `feRe` 词，删除不动 `frontendFileCaliber` | — | 已收口 |
| D3 | ~~`src/pages/ScreenshotDiagnosisPage.tsx`~~ **已注销** | `ScreenshotDiagnosisDashboard` | **已收口（2026-11-08）** | 同上：薄壳已删除，`src/components/invest/InvestHub.tsx:22` 直取域组件 | — | 已收口 |
| D4 | `src/pages/TasksCenterPage/OverviewTab.tsx:3` | `EvolutionApprovalDrawer` | 域特定 | 任务中心概览内联**股票进化审批**抽屉 | 中 | B8-2 |
| D5 | `src/pages/TasksCenterPage/OverviewTab.tsx:6` | `EvolutionHistoryEntry` | 域特定 | 同上 | 中 | B8-2 |
| D6 | `src/pages/TasksCenterPage/OverviewTab.tsx:7` | `useStockAnalysisStore` | 域特定 | 同上（读 `evolutionHistory`） | 中 | B8-2 |
| D7 | `src/pages/TasksCenterPage/forms/TaskCreateModal.tsx:25` | `getInvestKindDef`、`INVEST_KIND_DEFS`、`InvestTaskKind` | 域特定 | `src/pages/TasksCenterPage/forms/investTaskDefs.ts:15-22` 的 7 个种类（`stock-analysis`/`watchlist-scan`/…）→ 专用命令映射（`:8-9`） | 中 | B8-2 |
| D8 | `src/pages/TasksCenterPage/forms/TaskCreateModal.tsx:26` | `InvestTaskFields` | 域特定 | 投资任务表单字段组件 | 中 | B8-2 |
| D9 | `src/pages/TasksCenterPage/adapters/registry.ts:12` | `stockAnalysisAdapter` | 域特定 | 适配器注册表（同文件 `:24 REGISTRY`）；`:20-22` 已声明「插件贡献的源走同一条 `REGISTRY`」= 现成接缝 | 中 | B8-2 |
| D10 | `src/pages/TasksCenterPage/__tests__/adapters.test.ts:17` | `mapStockAnalysisRun`、`stockAnalysisAdapter` | 域特定（域测试） | 断言股票源的 `UnifiedRun` 映射 | 低 | B8-2 |
| D11 | `src/pages/TasksCenterPage/__tests__/OverviewTab.evolutionApproval.test.tsx:2` | `EvolutionHistoryEntry` | 域特定（域测试） | 进化审批抽屉测试 | 低 | B8-2 |
| D12 | `src/pages/TasksCenterPage/__tests__/OverviewTab.evolutionApproval.test.tsx:3` | `useStockAnalysisStore` | 域特定（域测试） | 同上 | 低 | B8-2 |
| D13 | `src/hooks/useRightPanel.ts:1` | `useStockAnalysisStore` | 域特定（**真跨域：chat → stock store**） | 消费方 `src/components/chat/ChatBubbleExpandButton.tsx:1`；hook 的 `tabKey` 值域 = `"market" \| "analyze" \| "execute"`（`src/hooks/useRightPanel.ts:8`），并写 `timelineJump` query（`:27`）给 `StockAnalysisPage` 消费 | 中（改为事件/注入或渲染器注册表） | B8-1 |
| D14 | `src/main.tsx:44` | `investSceneTemplates`（动态 `import()`） | 域特定（内容型注册） | 启动期 `queueMicrotask` 注入场景模板（`src/components/office/phaser/investSceneTemplates.ts:12` 注释） | 中 | B8-0/未定 |
| D15 | `src/stores/index.ts:102` | `useCrossMarketStore`（re-export） | 域特定（barrel） | 同文件 `:101` 分节注释「── AxInvest 投资域 ──」 | 高（须先改道消费方） | B8-5 |
| D16 | `src/stores/index.ts:103` | `usePaperPortfolioStore` | 域特定（barrel） | 同上 | 高 | B8-5 |
| D17 | `src/stores/index.ts:105` | `useBacktestStore`、`useStrategyStore` | 域特定（barrel） | 同上 | 高 | B8-5 |
| D18 | `src/stores/index.ts:106` | `useStockAnalysisStore` | 域特定（barrel） | 同上 | 高 | B8-5 |
| D19 | `src/stores/index.ts:104` | `PortfolioStressResult`、`SimulationScenarioRow`、`SimulationSnapshot`（类型 re-export） | 域特定（barrel） | 同上 | 高 | B8-5 |
| D20 | `src/types/index.ts:2420` | `export * from "./quant"` | 域特定（barrel） | 同文件 `:2419` 分节注释「=== AxInvest 投资域（stock/quant/pipeline）===」 | 高 | B8-5 |
| D21 | `src/types/index.ts:2425` | `BacktestResult`（类型 re-export） | 域特定（barrel） | 同文件 `:2421-2424` 注释：`BacktestResult` 与 `stock-analysis` 同名不同义，barrel 权威取 quant 版 | 高 | B8-5 |
| D22 | `src/types/index.ts:2426` | `export * from "./stock-analysis"` | 域特定（barrel） | 同上 | 高 | B8-5 |

---

## 3. 反面判据：为什么 `@/lib/stock-analysis-utils` 的 13 处**不能**算基元

§91.2 ② 把这一族列进「共享基元下沉」，并留了一句「其中 stock-analysis-utils 的函数是股票语义…必须先逐函数判中性」。本次复算为 **13 处 / 12 文件**（§91.2 的「13 处…余 12 处」分别对应处数与文件数，不矛盾）。判定结果：**逐个都带股票值域**，无一中性。

| 符号（定义处） | 值域证据 | 判定 |
|---|---|---|
| `horizonLabelKey`（`src/lib/stock-analysis-utils.ts:1932`） | 返回字面量 `stockAnalysis.timeHorizon${suffix}`（`:1934`），该 i18n 键族属股票插件 | 域特定 |
| `HORIZON_T_SUFFIX`（`:1713`） | 四档 `ultra_short/short/mid/long`（`:1714-1717`），与 Rust `holding_period` / `analyst_node_id` 同源 | 域特定 |
| `analystBaseOf`（`:1735`）/ `analystNodeId`（`:1724`） | 分析师节点 id `base--tier`、分隔符 `ANALYST_TIER_SEP`（`:1721`）来自 `harness::holding_period` | 域特定 |
| `parseActionStrict`（`:397`）/ `StockAction`（`:305`）/ `StockRiskLevel`（`:362`） | 值域是「买入/增持/持有/减持/卖出/观望」与「低/中/高/极高/无法评级」（`:330-359`、`:379-387`） | 域特定 |
| `parseRiskLevel`（`:681`） | 同上（中文风险档 → 英文枚举） | 域特定 |
| `RECO_STYLE_HUE`（`:825`）/ `RECO_STYLE_ALIASES`（`:843`） | 11 个后端 `Style` 变体 → 类目色相（`:818-823` 注释「按后端 `Style` 枚举的 11 个变体登记一次色相」） | 域特定 |
| `computeEvidenceWeights`（`:283`） | 走**股票插件命令** `EVIDENCE_WEIGHTS_COMMAND`（`:290-299`） | 域特定 |

**「参数化上收」能减读数吗**：不能。例如把 `horizonLabelKey` 拆成中性 `resolveEnumLabelKey(table, key)` + 域内表，调用点仍然 import 域内的表，specifier 仍含 `stock` ⇒ 读数不变，只是多一层间接。**不建议**在 B8-4 做这类改造（属结构改良，不是阻塞面收敛）。

同理，`src/lib/agentOutput.ts` 的通用名函数（`cleanToolCallTags` / `tryBeautifyJson` / `extractContent` / `parseJsonLoose`）今天**零非域消费方**（消费方全在 `src/components/stock-analysis/**`、`stockAnalysisStore`、`decision-agreement`、`risk-assessment`、`stock-analysis-utils`）。拆出「通用半区」不减任何一处读数；只有当**域半区整体随包迁出**时，C6/C7 两处才归零。

---

## 4. 下沉到哪（具体路径 + 门禁影响）

中性目录的白名单是 `scripts/check-component-neutrality.mjs:61` 的 `NEUTRAL_PREFIXES = ["layout","common","shared","dynamicUI"]`；`src/lib/` 不受该门管（它只扫 `src/components`）。棘轮基线 `scripts/component-neutrality-baseline.json` 只有 212 条、**只减不增**，本轮实跑 `node scripts/check-component-neutrality.mjs` 为 `PASS`（扫描 927 文件 · 原始 228 · 白名单后 225 条 · 基线 212，已修复 1 条）。

### 4.1 `PageHeader` → `src/components/common/PageHeader.tsx`

- **必须同时做的三件事**（少一件就变成新的跨 feature 引用或样式丢失）：
  1. **锚点改插槽**（**B8-4a 已按此落地**）：原 `showTimeAnchor?: boolean` 换成 `anchor?: ReactNode`（`src/components/common/PageHeader.tsx`），并删掉对 `@/components/time-travel/PageTimeAnchor` 的 import。否则 `common/PageHeader` → `time-travel/PageTimeAnchor` 会当场触发中立性门禁（`time-travel` 不在 `NEUTRAL_PREFIXES`），基线只减不增 ⇒ 直接红。
  2. **唯一传 `showTimeAnchor` 的调用点**：`src/components/time-travel/ReplayWorkbench.tsx:67-72`（`showTimeAnchor` 在 `:71`）改为 `anchor={<PageTimeAnchor />}`。
  3. **CSS 一起搬**：组件用的 `.sa-header / .sa-header-title / .sa-header-meta / .sa-header-back` 定义在 `src/components/stock-analysis/stock-analysis.css:49-90`，而该 CSS **只被** `src/components/stock-analysis/StockAnalysisPage.tsx:45` 引入 ⇒ 今天的 `ReplayWorkbench` / `QuantLab` 头部在独立路由下**根本没有样式**（既有隐患，非本轮引入）。下沉时把这 42 行迁到中性样式（或改 Tailwind 类）并让组件自带 import。
- **调用点全量**（6 处 import）：`src/components/stock-analysis/{BacktestPage,ComparePage,TradePage,WatchlistPage}.tsx`、`src/components/time-travel/ReplayWorkbench.tsx:1`、`src/components/quant/QuantLab.tsx:13`。
- **测试面**：5 个 stock 单测用 `.sa-header-back` / `.sa-header-title` 选择器（如 `src/components/stock-analysis/__tests__/BacktestPage.test.tsx:30`），类名保持不变即不受影响。
- **门禁收益**：基线里 `src/components/quant/QuantLab.tsx → @/components/stock-analysis/_shared/PageHeader` 与 `src/components/time-travel/ReplayWorkbench.tsx → @/components/stock-analysis/_shared/PageHeader` 两条会一起变成「已修复」⇒ 跑 `--update-baseline` 可 212 → 210。
- **审计读数**：`ReplayWorkbench.tsx:1` 的 specifier 变成 `@/components/common/PageHeader`（不含 `feRe` 关键词）⇒ **−1**。

### 4.2 `clearVendorCheckCache` → 中性失效事件

- 新建 `src/lib/cacheInvalidation.ts`（中性，无域词）：`export type InvalidationTopic = "vendor-config"; onInvalidate(topic, fn): () => void; invalidate(topic): void`。
- `src/components/stock-analysis/vendorCheck.ts:59` 改为模块加载时 `onInvalidate("vendor-config", () => { enabledCache = null; })`；`src/components/settings/DataVendorsTab.tsx:446` 改 `const { invalidate } = await import("@/lib/cacheInvalidation"); invalidate("vendor-config");`。
- 审计读数：`DataVendorsTab.tsx:446` 的 specifier 变中性 ⇒ **−1**。组件中立性门禁不涉及（不在 `src/components`）。
- **更省的替代（需产品裁定）**：直接删掉这次调用，靠 `CACHE_TTL_MS = 30_000`（`src/components/stock-analysis/vendorCheck.ts:28`）自然过期 ⇒ 也是 −1，但保存配置后最长 30 s 内面板可能仍按旧 vendor 集判断（文案是「数据源未启用」）。**不推荐**，因为那是把「即时生效」换成「等 30 秒」，属于行为退化。

### 4.3 不要下沉的东西（明确列出，防后人重做一遍）

- `src/components/stock-analysis/_shared/SectionHeading.tsx`：虽同为中性组件，但**只有** `BacktestPage` 一个消费方，下沉读数不变 ⇒ 不进首刀。
- `src/lib/investRoutes.ts` 的 `investPath`：值域是投资中心 tab 联合类型，`/invest` 路由本身随插件走 ⇒ 不下沉。
- `src/lib/dualView.ts`：见 §6，它是「中性路径 + 域值域」的另一个坑，下沉只会把耦合洗白。

---

## 5. 可执行的最小首刀（5 个符号，预期 51 → 46）

> 前提：本文件不改代码。以下是给实现批的清单，按「先零风险、后碰公共件」排序。

| 序 | 动作 | 命中符号 | 预期读数 | 风险点 / 前置 |
|---|---|---|---|---|
| 1 | 删 `src/components/office/panels/TrajectoryPanel.tsx` | `TimelineNode`/`TimelinePhase`（C1） | −1 | 死代码已用 `git grep` 全仓核对（只命中自身）、`src/components/office` 内无 `import.meta.glob` 装配。但该文件是 `ffbab4f01`「恢复股票业务模块」**机械恢复**进来的 ⇒ 删前找 owner 确认不接线 |
| 2 | 删 `src/components/office/panels/StrategyRoomMiniBar.tsx` | `investPath`（C2） | −1 | 同上；注意它在 `677003c59` 还被改过 3 行（改了深链构造）却仍无消费方 ⇒ 说明「有人以为是活的」，删前同样要确认 |
| 3 | `PageHeader` → `src/components/common/PageHeader.tsx`（锚点插槽 + CSS 迁移） | `PageHeader`（B1） | −1 | 6 处调用点；`showTimeAnchor` 唯一使用者是 `ReplayWorkbench.tsx:71`；`.sa-header*` 迁出会动 `stock-analysis.css`；下沉后必须跑 `node scripts/check-component-neutrality.mjs` 并 `--update-baseline`（212 → 210） |
| 4 | `clearVendorCheckCache` → `src/lib/cacheInvalidation.ts` | `clearVendorCheckCache`（B2） | −1 | 订阅时序（vendorCheck 必须已被 import 才订阅；它是懒加载模块，未加载时 `/invest` 面板本就没缓存可失效，语义安全）；不要选「删调用靠 TTL」的退化方案 |
| 5 | `src/theme/__tests__/tokenSingleSource.test.ts` 去域依赖（换合成色表夹具） | `RECO_STYLE_HUE`/`RECO_STYLE_ALIASES`（C17） | −1 | 断言力度：`hasSecondStyleTable` 的负控必须继续用**非空表**喂（`:231`、`:250-257`）；真实色表的覆盖移交股票包自己的测试，别静默删断言 |

**合计**：−5（51 → 46）。其中**真「下沉」只占 −2**（序 3、4），序 1/2 是删死代码、序 5 是测试夹具化 —— 引用这个数时必须这样拆开说，否则又变成「用读数代替语义」。

**首刀之后不做的事**（否则会拉高读数或制造假绿）：
- 不要为了 −8 去改 `feRe` 或给 `stock-analysis-utils` 换文件名（`PLAN-shell-plus-plugins.md` §91.1 已把「不改判据」定死，§64.13 有先例）。
- 不要先拆 `stores/index.ts` / `types/index.ts`（D15–D22）：消费方未改道前拆 barrel 只会把隐藏依赖变成显式计数，读数**上升**（§91.3 的 B8-5 已注明）。

---

## 6. 顺带发现：`feRe` 的盲区（只登记，不改判据）

`feRe` 按**路径名**判归属，两个方向都会错，51 只是「被抓到的那部分」：

1. **域文件取了中性名 ⇒ 从未计数**：`src/components/time-travel/**`、`src/lib/{agentOutput,decision-agreement,decisionInputDiagnosis,dataQualityDiagnosis,risk-assessment}.ts`、`src/utils/workflowNodeLabel.ts`、`src/stores/feature/screenshotDiagnosisStore.ts`。它们今天的跨域引用被算在「非域 import 域」里（本文件 C 类），而它们**自身**对其它域的引用（例如 `src/lib/risk-assessment.ts:16` 引 `agentOutput`）根本不进统计。
2. **中性路径 + 域值域 ⇒ 结构耦合被漏掉**：`src/lib/dualView.ts` 的 `defaultTab: "market" | "analyze" | "execute"`（`:21`），被 `src/components/chat/ChatBubbleExpandButton.tsx:2` 消费 —— 路径无关键词，从未计入 51，但性质与 D13（`useRightPanel`）完全同类。
3. **另一个方向的假阳性**已在 §2.1 单列（quote/market 两族）。

结论：**51 不是「前端跨域耦合总量」，只是「`feRe` 能看见的那部分」**。终态判据若写成「51 → 0」，会诱使后人把 C 类域文件改个名就算完成 —— 这正是本文件要挡的。

---

## 7. 未做清单（如实列）

1. **没改任何代码**：不改 `feRe`、不改 `docs/INDEX.md`、不改 `PLAN-shell-plus-plugins.md`、不 commit/push/checkout/stash。审计快照 `docs/audits/AUDIT-l1-blockers-2026-11-05-1126.md` 是脚本副作用，保留不提交。
2. **未做运行验证**：没跑 `npm run typecheck` / `vitest` / `cargo`（本批无代码改动）；门禁只实跑 `node scripts/check-component-neutrality.mjs`（PASS）与 `node scripts/audit-l1-blockers.mjs`。
3. **未裁决 `src/components/time-travel/**` 的归属**：本文件以「域自有（股票回放）」判定（依据：`src/types/timeTravel.ts:26` 的 `TimeTravelBatchAnalysis` 为逐股一行、Rust `ownedRe` 含 `time_travel`）。若 owner 裁定它留壳，则 C3–C5 会升级为真跨域，`horizonLabelKey` 一族需改成「宿主中性档位表 + 域 i18n 键映射」，本文件的 C 类结论**需重做**。
4. **未裁决「投研办公室场景模板」终局**（C20 + D14）：两条路 —— ① 作为**数据型**预设由插件贡献（符合 `PLAN-shell-plus-plugins.md` §93「插件不取主 origin React」）；② 留在宿主当普通场景包（i18n 键落在 `office.*` 命名空间：`src/components/office/phaser/__tests__/sceneTemplates.labelKey.test.ts:48` 断言其标签键为 `office.scene.investment_office`）。本轮不拍。
5. **未做 schema-gen 扩面**：C19（`PaperPortfolio`）与 6 处 DTO 类仍在 `src/types/`，未验证 `schema-gen` 能否覆盖（当前只出 18 个 IPC DTO）。
6. **未评估 barrel 拆除的消费方改道顺序**（D15–D22，8 处）。
7. **未统计 OPC 侧 28 处**（不在本任务范围；同批 `feRe` 会同样把 `market` / `pack` 类词吃进来，判定方法可复用本文件）。
8. **未逐条深挖 `src/lib/agentOutput.ts` 的通用半区拆分**：只核到「通用函数的非域消费方为 0」，拆分对读数的贡献为 0，故未给方案；若将来有非域调用方出现（如 chat 渲染器），再单独立项。
9. **`docs/INDEX.md` 未登记**（按硬约束留给 leader）。
