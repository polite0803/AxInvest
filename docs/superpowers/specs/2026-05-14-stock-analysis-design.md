# 股票分析模块移植设计

## 概述

从 [TradingAgents-astock](https://github.com/simonlin1212/TradingAgents-astock) 移植 A 股多智能体分析能力到 AxInvest，使用本项目原生架构（Expert + Role → AgentProfile、SharedBlackboard、SessionManager、Workflow Engine）承载，不做架构搬运。

## 核心原则

- **复用 AxInvest 原生架构**：Expert/Role/AgentProfile 模式，不引入 Python 依赖
- **纯 Rust 数据层**：HTTP 公开 API 替代 mootdx/akshare，零外部依赖
- **多 Agent + SharedBlackboard**：每个分析师是独立 AgentSession，通过 Blackboard 交换数据
- **双入口前端**：独立看板（侧重查看）+ 聊天集成（侧重交互）

---

## 1. 数据层 — crate `astock-data`

### 数据源映射

| Python 源 | Rust 替代 | 数据内容 |
|---|---|---|
| mootdx (TCP 7709) | 腾讯财经 `qt.gtimg.cn` | 实时行情、K线 |
| akshare (东方财富) | `push2his.eastmoney.com` | 历史K线、资金流向 |
| akshare (财务) | `emweb.securities.eastmoney.com` | 三表数据 |
| akshare (新闻) | 东方财富新闻 API + 新浪财经 | 新闻、公告、研报 |
| akshare (龙虎榜) | 东方财富龙虎榜 API | 龙虎榜、大宗交易 |
| akshare (解禁) | 东方财富限售解禁 API | 解禁日程、股东减持 |

### Crate 结构

```
crates/astock-data/
├── Cargo.toml          # reqwest, serde, tokio, chrono, thiserror
├── src/
│   ├── lib.rs          # AStockClient 统一入口 + vendor routing
│   ├── types.rs        # StockQuote, KLine, FinancialReport, NewsItem, MoneyFlow, DragonTigerEntry, LockupSchedule
│   ├── error.rs        # DataError
│   └── vendors/
│       ├── mod.rs      # Vendor trait
│       ├── tencent.rs  # 腾讯财经
│       ├── eastmoney.rs # 东方财富
│       └── sina.rs     # 新浪财经
```

---

## 2. Agent 架构 — Expert + Role → AgentProfile

### 模型

遵循上游 `agency_experts → agent_profiles → AgentSession` 三层模式：

- **agency_expert**：14 个股票分析专家，每个携带领域知识的 system_prompt
- **agent_profile**：Expert + agent_role 组合，关联 recommended_tools
- **AgentSession**：运行时实例，通过 SharedBlackboard 协作

### 14 个 Expert 定义

| # | Expert Name | agent_role | 领域知识 |
|---|---|---|---|
| 1 | 市场技术分析师 | analyst | K线形态、均线、MACD/RSI/KDJ、量价关系、支撑压力 |
| 2 | 情绪面分析师 | analyst | 社交媒体情绪、散户舆情、股吧热帖 |
| 3 | 消息面分析师 | analyst | 公司公告、行业新闻、宏观事件、研报观点 |
| 4 | 基本面分析师 | analyst | 三表解读、ROE/毛利率、PE/PB/PS、CAGR |
| 5 | 政策面分析师 | analyst | 产业政策、窗口指导、监管风向、税收/补贴 |
| 6 | 资金面追踪者 | analyst | 龙虎榜、主力资金、北向资金、大宗交易 |
| 7 | 筹码面观察者 | analyst | 解禁、增减持、股权质押、筹码集中度 |
| 8 | 多方研究员 | debator | 从分析报告中提炼看多论据 |
| 9 | 空方研究员 | debator | 挖掘风险、挑战多头假设 |
| 10 | 激进风险评估师 | risk_evaluator | 高收益导向风险评估 |
| 11 | 保守风险评估师 | risk_evaluator | 本金安全导向风险评估 |
| 12 | 中性风险评估师 | risk_evaluator | 平衡视角风险收益比 |
| 13 | 研究经理 | manager | 综合辩论+风控，制定投资计划 |
| 14 | 投资组合经理 | manager | 最终决策+仓位建议 |

### SharedBlackboard 数据模型

```
stock_code, stock_name, analysis_date

raw.kline        ← Vec<KLine>
raw.financials   ← FinancialReport
raw.news         ← Vec<NewsItem>
raw.money_flow   ← MoneyFlow
raw.dragon_tiger ← Vec<DragonTigerEntry>
raw.lockup       ← Vec<LockupSchedule>

report.market
report.sentiment
report.news
report.fundamentals
report.policy
report.hot_money
report.lockup

debate.round_N.bull
debate.round_N.bear

risk.aggressive
risk.conservative
risk.neutral

plan.investment
decision.final   ← StockDecision { action, position_pct, reasoning }
```

---

## 3. 编排层 — crate `stock-analysis`

### Crate 结构

```
crates/stock-analysis/
├── Cargo.toml
├── src/
│   ├── lib.rs           # pub mod orchestrator, pipeline, decision
│   ├── orchestrator.rs  # StockAnalysisOrchestrator::run()
│   ├── pipeline.rs      # run_analyst / run_debator / run_manager 通用执行器
│   └── decision.rs      # StockDecision, AnalysisConfig
```

### 5 阶段执行流程

```
阶段 1: 数据加载
  astock-data 拉取 → 写入 Blackboard raw.*

阶段 2: 并行分析 (7 Agent 同时)
  tokio::try_join! 执行 7 个 run_analyst()
  每个从 Blackboard 读数据 → LLM 生成报告 → 写回 report.*

阶段 3: 多空辩论 (循环 N 轮)
  for round in 0..max_rounds:
    tokio::join!(bull, bear) 并行
    读对方上轮论点 → 生成反驳 → 写回 debate.round_N.*
    收敛检测 → break

阶段 4: 风险评估
  tokio::try_join!(aggressive, conservative, neutral) 并行
  → run_manager("research-manager") 生成投资计划

阶段 5: 最终决策
  run_manager("portfolio-manager") → StockDecision
```

### AnalysisEvent 广播

```rust
enum AnalysisEvent {
    Started { stock_code, stock_name, date },
    DataLoaded { quote, kline_count, news_count },
    AnalystProgress { expert_id, status, progress_pct },
    AnalystReport { expert_id, report_text },
    DebateRound { round, bull_argument, bear_argument },
    RiskAssessment { risk_type, report },
    Decision { action, position_pct, reasoning },
    Error { stage, message },
}
```

---

## 4. 前端

### Store — `stockAnalysisStore.ts`

```typescript
interface StockAnalysisStore {
  // State
  analysisId, stockCode, stockName, status, progress
  quote, klineData, analystReports[], debateRounds[], riskAssessments[], decision

  // Actions
  startAnalysis(stockCode, date, providerId): Promise<void>
  cancelAnalysis(): Promise<void>
  fetchHistory(limit, offset): Promise<AnalysisSummary[]>
  loadAnalysis(analysisId): Promise<StockAnalysisResult>
  searchStock(keyword): Promise<StockSearchResult[]>
  getStockQuote(stockCode): Promise<StockQuote>
  getStockKline(stockCode, period, limit): Promise<KLine[]>
}
```

### 入口 A: 看板页面 `/stock-analysis`

组件树：
- `StockAnalysisPage` — 页面容器
- `StockSearchBar` — 代码输入 + 日期 + 启动按钮
- `AnalysisProgress` — 5 阶段进度条
- `StockQuoteCard` — 实时行情卡片
- `KLineChart` — K 线图（ECharts/AntV）
- `AnalystReportGrid` → `AnalystReportCard × 7` — 2 行 4 列
- `DebatePanel` — 多空左右对比
- `RiskMatrix` — 三方风险
- `DecisionBanner` — 最终决策横幅

### 入口 B: 聊天集成

- `agentStore` 注册 `stock-analysis` agent 类型
- `InputArea` 支持 `@股票代码` 或 `/analyze` 命令
- 复用 `AgentExecutionPanel` 展示各阶段 Agent 消息卡片
- 卡片底部"查看完整报告"链接跳转看板页面

### 路由

- `/stock-analysis` → 新建分析
- `/stock-analysis/:id` → 查看历史分析

---

## 5. Tauri Commands

```rust
// 数据查询
get_stock_quote(stock_code) -> StockQuote
get_stock_kline(stock_code, period, limit) -> Vec<KLine>
search_stock(keyword) -> Vec<StockSearchResult>

// 分析生命周期
start_stock_analysis(stock_code, date, provider_id) -> AnalysisSession
cancel_stock_analysis(analysis_id)

// 历史
list_stock_analyses(limit, offset) -> Vec<AnalysisSummary>
get_stock_analysis(analysis_id) -> StockAnalysisResult
```

---

## 6. 文件清单

### 新增：后端 (14 files)
- `src-tauri/crates/astock-data/Cargo.toml`, `lib.rs`, `types.rs`, `error.rs`
- `src-tauri/crates/astock-data/src/vendors/mod.rs`, `tencent.rs`, `eastmoney.rs`, `sina.rs`
- `src-tauri/crates/stock-analysis/Cargo.toml`, `lib.rs`, `orchestrator.rs`, `pipeline.rs`, `decision.rs`
- `src-tauri/crates/core/src/entity/stock_analyses.rs`
- `src-tauri/crates/migration/src/m20250514_000001_stock_analysis.rs`
- `src-tauri/src/commands/stock_analysis.rs`

### 新增：前端 (12 files)
- `src/components/stock-analysis/` — 10 组件文件
- `src/stores/feature/stockAnalysisStore.ts`
- `src/types/stock-analysis.ts`

### 新增：Expert 定义 (14 files)
- `agency_experts/stock-analysis/*.md` — 14 个 markdown 文件

### 修改 (13 files)
- `src-tauri/Cargo.toml`, `src-tauri/src/{commands/mod.rs, lib.rs}`
- `src-tauri/crates/{core/src/entity/mod.rs, migration/src/lib.rs}`
- `src/{router.tsx, stores/index.ts, types/index.ts}`
- `src/components/layout/Sidebar.tsx`
- `src/stores/feature/agentStore.ts`
- `src/components/chat/InputArea.tsx`
- `locales/` 下 11 种语言文件

### 总计：53 文件（40 新增 + 13 修改）

---

## 7. 上游依赖

上游 AxAgent 无需任何改动。所有能力均已具备：
- Expert + Role → AgentProfile 模式完整
- `run_turn_with_tools` 支持 `system_prompt: Vec<String>` 注入
- `SharedBlackboard` 通过 `Arc<RwLock<>>` 支持并行读写
- Agency expert import 流程完整

---

## 8. 阶段交付计划

| 阶段 | 内容 | 产出 |
|---|---|---|
| 1 | `astock-data` crate + vendor 实现 | 可独立测试的数据获取 |
| 2 | 14 个 Expert markdown + import → 单 Agent 分析验证 | 一个分析师可生成报告 |
| 3 | `stock-analysis` crate + 7 分析师并行 | 并行分析 Blackboard 协作 |
| 4 | 辩论 + 风险评估 + 决策 | 完整 5 阶段 pipeline |
| 5 | Tauri commands + 前端看板 | 可交互的完整看板页面 |
| 6 | 聊天集成 | 聊天中触发分析 + 卡片展示 |
| 7 | i18n + 测试 + 文档 | 11 种语言 + Vitest + Rust test |
