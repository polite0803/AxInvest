# AUDIT — 决策档位展示一致性：双轴正交与派生面同步

> 日期：2026-09-21｜范围：股票分析决策链（Rust 风控门 → Harness DTO → 前端展示）+ i18n 值域收敛
> 触发：用户报告「历史卡片挂角标签显示**观望**，结论文本显示**持有**」（同一条记录自相矛盾）
> 说明：行号为 2026-09-21 快照，后续改动会漂移；定位以**函数名 + 文件名**为准。

---

## 1. 根因

### 1.1 数据模型是**双轴正交**的

| 轴 | 字段 | 值域 |
|---|---|---|
| 方向强度 | `decision_action` | 买入 / 增持 / 持有 / 观望 / 减持 / 卖出 |
| 持仓状态 | `decision_position_state` | `EMPTY` / `OPENING` / `HOLDING` / `TRIMMING` |

「持有 vs 观望」**不是两个独立语义**，而是中性档（`HOLD` / `WAIT`）按持仓状态**单向派生**的结果：

```
非中性档        → 原样返回
中性档 + EMPTY  → 观望
中性档 + 非 EMPTY → 持有
```

派生单点：前端 `resolveDisplayAction(action, positionState, positionPct)`
（`src/lib/stock-analysis-utils.ts`）。

### 1.2 缺陷形态：**局部覆盖写入只改了源字段，派生轴滞留原值**

`portfolio-risk-gate.rhai` 作为后处理门控，会覆盖 `action` 与 `positionPct`，但同一条记录里
**三个派生态**没跟着重建：`positionState`、`reasoning` 里的 `决策=<档位名>`、`stopLossPct/takeProfitPct`。

**为什么比原 bug 更隐蔽**：前端 `resolveDisplayAction` 以 `positionState` **优先** —— 门控把
`action` 改成「观望」，`positionState` 却仍是 `HOLDING` ⇒ 派生结果**又变回「持有」**。
表现为「改完之后界面还是老样子」，极易误判成"改动没生效 / 没重种"而查错方向。

### 1.3 同根因扩散（5 处，各自独立存活）

| # | 位置 | 形态 |
|---|---|---|
| 1 | `DailyReviewPanel.tsx` | **真根因在 DTO**：`review.rs` 的 `DecisionComparison` 整条**缺持仓状态轴**，前端无从派生；兜底还自造私有值 |
| 2 | `TradePanel.tsx` | 裸展示 + `as unknown as Record<string, unknown>` 双重强转绕过类型 |
| 3 | `ExperimentTrail.tsx` | 两处 `${decision.action}` 裸插值 |
| 4 | `DecisionBanner.tsx` | 「问 AI」prompt 直接拼 raw action |
| 5 | `WorkflowAgentCard.tsx` | `data.action === t("stockAnalysis.actionBuy")` —— **拿本地化展示文案当协议值比较**，en-US 下恒 false |

第 5 处的形态与其余四处不同：它不裸展示，而是**跨语言字符串契约失效**。同一症状的两种根因
（未派生 vs 拿译文比协议值），修法完全不同。

---

## 2. 修复清单

### 2.1 Rust（5 文件）

| 文件 | 改动 |
|---|---|
| `commands/portfolio-risk-gate.rhai` | 新增全局函数 `is_known_action_label` / `align_decision_label`；输出段重算 `positionState`（三态）+ `stopLossPct`/`takeProfitPct` 归零 + `reasoning` 结论名对齐 |
| `crates/analysis-engine/src/review.rs` | DTO 补 `position_state: Option<String>` / `position_pct: Option<f64>`；兜底改权威哨兵 `ACTION_UNAVAILABLE`；`action` 注释纠正（实际是中文存储形态，原注释写的 `BUY/HOLD/SELL` 是错的） |
| `commands/stock_analysis.rs` | `QuickBacktestSample.decision_confidence: f64 → Option<f64>`；采样构造从**反推**改为 `ACTION_UNAVAILABLE` + `None` |
| `commands/stock_analysis_setup/seed_stock_analysis.rs` | `TEMPLATE_VERSION` **69 → 70** |
| `crates/rt-workflow/tests/rhai_syntax_check.rs` | 新增 `portfolio_risk_gate_syncs_derived_fields()`，含 5 个变异点 |

### 2.2 前端（8 文件 + 1 测试）

| 文件 | 改动 |
|---|---|
| `lib/stock-analysis-utils.ts` | 新增 6 张值域表 + 7 个 `getXxxTKey` 函数（dashboard 专属，含 override 表保住「强烈买入/强烈卖出」两个档位） |
| `components/stock-analysis/DailyReviewPanel.tsx` | 接口补两轴；两处裸展示改走派生 |
| `components/stock-analysis/TradePanel.tsx` | 走派生，**删除双重强转** |
| `components/stock-analysis/ExperimentTrail.tsx` | 局部 `actionLabel()` 替换裸插值 |
| `components/stock-analysis/DecisionBanner.tsx` | prompt 走派生 |
| `components/stock-analysis/QuickBacktestPanel.tsx` | 两列 render 改 `getActionTKey` / `null` 安全 |
| `components/chat/WorkflowAgentCard.tsx` | 改 `parseAction` / `parseRiskLevel` 归一化后再比较；`EXTREME` 补入红色分支 |
| `components/stock-analysis/DashboardReportPreview.tsx` | 5 处裸展示 i18n 化；3 个颜色 case 函数收敛到 utils；**移除文件级 `i18n-exempt`** ⇒ 文件零 CJK、零违规 |
| `lib/__tests__/stockAnalysisAction.test.ts` | 新增 3 个 describe / 9 条断言（含变异检验点、跨 locale 契约） |

### 2.3 i18n（11 locale × 18 新 key）

新增 key：`actionStrongBuy` `actionStrongSell` `trendBullish/Bearish/Sideways`
`catalystBullish/Bearish` `catalystShortTerm/MidTerm/LongTerm`
`checklistEntry/Add/Reduce/StopLoss/TakeProfit` `severityHigh/Mid/Low`。

同时修正：
- **zh-TW 8 档 action 标签全部繁体化**（`买入→買入`、`卖出→賣出`、`减持→減持`；原先与已繁体化的 `actionWait="觀望"` 同组混用）
- zh-TW / ja 的 `dashboard` 段补正（含半繁半简形态：`每日復盘`、`記录交易`、`投資仪表盘`）

### 2.4 仓库卫生

- 11 个 `.json.bak-b3`（2026-09-20 AI 批量改 locale 的残留，生成脚本 `output/_i18n-insert-b3.mjs` 尚在，无代码引用）
  ⇒ **移动**（非删除）至 `output/backup-20260920-locales-b3/`，逐文件 sha256 复制校验后删源。

---

## 3. 验证矩阵

| 门禁 | 结果 |
|---|---|
| `cargo check -p axagent` | **Finished dev profile in 9m50s，error=0 / warning=0** |
| `cargo test -p axagent-rt-workflow --test rhai_syntax_check` | **8 passed / 0 failed** |
| `cargo fmt --check` | EXIT=0 |
| `tsc --noEmit` | EXIT=0 / 0 错误 |
| `oxlint src --max-warnings=0` | EXIT=0 / **0 warnings 0 errors**（1261 files / 96 rules）|
| `dprint check` | EXIT=0 |
| `scripts/i18n-scan.mjs --strict` | EXIT=0，`DashboardReportPreview` 违规 **0**，全项目 All passed |
| `scripts/check_i18n.py --strict` | EXIT=0 |
| `scripts/check-i18n-untranslated.mjs` | EXIT=0，新增 0 |
| `vitest run`（全量） | **171 files / 1955 tests passed**（原 1946 + 新增 9，差值自洽）|
| 变异检验（禁用 dashboard override 表） | `EXIT=1`，**恰好**「强烈买入/强烈卖出」那一条变红 ⇒ 断言有真实区分力 |
| 回滚点有效性 | 11 文件 sha256 与工作区**逐文件不同** ⇒ 备份成立（`output/backup-20260921-locales/`）|

### 关键技法：变异检验

「写了对的断言」≠「断言有区分力」。本轮对两处判据做了破坏性验证：
① 前端 override 表（上述）；② rhai 门禁的 5 个变异点（对齐函数改名 / 状态轴赋值摘除 /
结论名未走对齐 / 止损未归零 / 止盈未归零）—— 每条破坏都必须让对应断言变红，测不出来即判据空转。

---

## 4. 生效前置条件

| 面 | 条件 |
|---|---|
| 前端改动 | 即时生效（HMR） |
| 后端 `portfolio-risk-gate.rhai` + 新 DTO | **须重启 app 触发模板重种**（`TEMPLATE_VERSION=70`）。DB 里旧库存是 v69 ⇒ 若不升号，改了等于没改 |

---

## 5. 登记（本轮不擅改 —— 范围与风险超单批，需决策）

### 5.1 翻译存量（大工程，非本轮引入）

| 项 | 量级 | 判定依据 | 为何不顺手改 |
|---|---|---|---|
| 9 种语言的 `stockAnalysis` 段英文占位 | 每语言 **922~1052 key** 与 en-US 逐字相同（全库 18~22%）| 逐字比对 en-US | 需真翻译，非机械替换 |
| zh-TW 简体泄漏 | **3206 条**（全库）；`stockAnalysis` 段 **945 条** | 「zh-TW 值 === zh-CN 值且含汉字」 | 手写字表有假阳性（简体同形字如「性」被误收）⇒ 建议 OpenCC 类工具 |
| ja 值与 zh-CN 相同 | **2320 条** | 同上 | 同上 |

**现有门禁查不出来**：`check-i18n-untranslated.mjs:68` 的 `CJK_LOCALES = {zh-CN, zh-TW, ja}` —— ja/zh-TW
被豁免「含汉字」判据（脚本 L65-66 **自陈**了这个边界），而英文占位天然不含汉字 ⇒ 全绿。
这是**判据口径缺口**，不是被测对象清白。

### 5.2 `i18n-exempt` 文件级豁免

`scripts/i18n-scan.mjs:168` 是 `if (content.includes("i18n-exempt")) { return []; }` ——
**整个文件直接失效**。全仓 **65 个文件**带此注释，其中 **30 个含 `t()` 调用的真 UI 组件**。

本轮只摘了 `DashboardReportPreview` 一个（摘掉后暴露 5 处真裸展示，修复后 0 违规 ⇒ **豁免彻底不需要**）。
其余 30 个每个都可能是同类缺陷的藏身处。建议把豁免从**文件级**降为**行级 allowlist**，
判据：`grep -n 'useTranslation\|t("' <文件>` 有命中 ⇒ 不能用文件级豁免。

### 5.3 同类未改（各自独立根因）

- `AnalystReportCard.tsx:227` —— `action` 来自 LLM 自由文本，无值域可收敛
- `HistoricalAnalysisPanel.tsx:282` —— 回测 DTO 无持仓状态轴，无法派生

---

## 6. 沉淀

- **技能修正** `i18n-hardcoded-fix`：原第 2 节表格写「mock/NLP 字符串 → 文件头加 `i18n-exempt`」
  **未提作用域**，是「文件级豁免被滥用」的**源头**。已补作用域专节 + 唯一判据 + 既有豁免审计命令。
- **技能修正** `rust-consistency-audit` 方法一：「三个必查的坑」→ **四个**，新增
  「局部覆盖写入的派生面漂移」（含逐派生态 grep 检法、下游重派生把覆盖值派生回去的机制、
  单点函数根治、变异检验门禁）。症状分流表 + description 同步补召回词。
