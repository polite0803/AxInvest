# Changelog

## 2026-06-07 — Stock Investment Refactor + Decision Timeline + Dual View

### 重大变更

#### 抽离 5 个独立页面(Phase 1-7)
- **WatchlistPage** `/watchlist` — 自选股管理,支持分组、排序、批量分析
- **ScreenerPage** `/screener` — 选股器,推荐评分面板迁出
- **TradePage** `/trade` — 交易页,Tab 4 跳转入口
- **BacktestPage** `/backtest` — 回测独立页
- **ComparePage** `/compare` — 跨标对照
- 侧栏新增 5 个入口 + `投资` 分组,折叠组默认展开

#### StockAnalysisPage 侧栏瘦身(Phase 7)
- 22 个 sheet panels → 10 个(保留 4 核心 + 6 高频)
- 移走:HistoricalAnalysisPanel、AnalystReportGrid、RecommendationPanel 等
- 移走 13 个 panel 组件 import
- 移动端核心面板 9 → 4(index/screener/sectors/north)
- 顶栏新增 `→ 交易` 跳转按钮

#### Decision Timeline 决策时间线(Phase 8)
- 新类型:`TimelineNode` / `TimelinePhase` / `EvidenceRef`
- store 新增:`timeline` / `highlightedPanel` + `pushTimelineNode` / `updateTimelineNode` / `clearTimeline` / `setHighlightedPanel`
- 4 阶段:`scan` / `diagnose` / `debate` / `decide`,流式从 `workflow-step-done` 事件中累积
- `DecisionTimelinePanel` 替换 `AnalysisProgress` + `DecisionBanner`
- 节点证据芯片 EvidenceChip → 跨 tab 跳转 + 0.4s 高亮闪烁
- `useRightPanel` hook 桥接 store + query param
- 17 个 expert agency 节点自动归属
- i18n 8 个 key × 11 语言

#### Dual View 双向折叠(Phase 9)
- `dualViewRegistry`:支持 `compact` / `panel` / `defaultTab` / `noDualView`
- `DualViewRenderer` 适配器:模式切换 + 折叠按钮
- `ChatBubbleExpandButton`:对话端 `→` 展开为侧栏 panel
- `PanelCollapseButton`:侧栏端 `→` 降级为剪贴板(后续接入 chat 写入 API)
- 试点完成:估值评估(ValueAssessment) — 已接入对话流 + 侧栏
- 试点完成:辩论节点(DebatePanel) — 多空观点紧凑气泡
- 接入:`RiskMatrix` / `RecommendationPanel` / `AnalystReportGrid`
- 消息 metadata 扩展:`meta.bubbleMeta.{dualViewId, dataRef}`(可选字段,不破坏现有 chat 渲染)
- 黑名单:高频 ticker / 跨标对照 / 配置面板不出现 dual view
- i18n 5 个 key × 11 语言

#### 跨页跳转(Phase 10)
- WatchlistPanel 点击股票 → `navigate("/stock-analysis?code=...")` 跳转分析页
- StockAnalysisPage 顶栏 `→ 交易` 按钮跳转交易页
- StockAnalysisPage 监听 `timeline-jump` 事件,根据 `timelineJump` query param 切换 activeTab
  - `market:*` → market tab
  - `analyze:<panelKey>` → analysts/debate/value/risk/decision
  - `execute:*` → decision tab(交易计划归入决策 tab)

### 工程指标

- `npm run typecheck` → 0 错
- `npm run format` → 全绿
- `npm run test:run` → **553 tests passed, 1 skipped**(60 个测试文件)
- 新增测试:
  - `stockAnalysisStore` 时间线相关:5 个
  - `DecisionTimelinePanel`:4 个
  - `dualView` 注册表:6 个
  - `ChatBubbleExpandButton`:2 个
  - 5 个 page tests 至少 1 个快照测试:通过
- 11 语言 i18n 键齐备(zh-CN / zh-TW / en-US / ja / ko / de / fr / es / ru / hi / ar)

### 不在本次范围

- 不重写任何面板内部组件逻辑
- 不引入 ⌘K 命令面板(后续做)
- 不改 antd 主题或设计 token
- 不重构 store 整体结构
- 不动后端 Rust 代码
- 不做 17 个 expert agency 的 icon 定制化
- DualView 接入 6 个目标面板完成,其余面板留待后续迭代

### 验证清单(浏览器手动)

- [ ] 5 个新页面路由可访问
- [ ] 分析页侧栏瘦身生效(13 个 panel 移走,移动端 4 核心可见)
- [ ] 时间线 4 阶段折叠/展开正常
- [ ] 跨 tab 跳转 + 0.4s 高亮闪烁
- [ ] DualView 6 个面板可双向折叠
- [ ] 从自选股跳分析页、从分析页跳交易页状态正确
