# PLAN：智能荐股四周期口径收编 + 前端呈现缺陷修复

> 状态：active ｜ 日期：2026-09-29 ｜ 对应版本：v2.9.9


日期：2026-09-29 · 状态：**待批准** · 前置：`PLAN-multi-horizon-decision-science.md`（Phase A~F 只落在分析决策链，本计划把它收编到荐股链）

## 〇、边界声明（先划清这两条链）

本仓有**两套**「四周期」实现，互不消费：

| 链 | 载体 | 四周期现状 |
|---|---|---|
| 分析决策链 | `commands/portfolio-mgr.rhai` + `analysis-engine`（后验融合、逐档先验、SNR、IC） | Phase A~F 已科学化（TEMPLATE_VERSION 105） |
| **智能荐股链** | `crates/analysis-engine/src/recommender/`（6 风格 × 4 档策略出候选票） | **本轮之前完全未动** |

实证：在 `recommender/` 目录 grep `decisions_by_horizon` / `horizon_prior` / `evidence_weight` /
`get_horizon_base_weights` ⇒ **零命中**。⇒ 分析链被判掉的构造性错误（E1/E3/E5/E6/E7），
在荐股链**原样存在**，而且荐股是用户直接看到「买哪只、买多少、止损在哪」的那一面。

---

## 一、荐股链诊断（R1~R12，均可静态实证）

| # | 缺陷 | 证据 | 后果 |
|---|---|---|---|
| R1 | **置信度与档位无关**：`calc_confidence` 签名里根本没有 `period`，入参是按**风格**固定的常量 | `recommender/scoring.rs:64-108`；调用侧 `strategies/trend.rs:194-196`（0.85/0.7/0.0）、`value.rs:197-201`、`capital.rs:281-283`、`watchlist.rs:130-132` | 同一票同一风格，超短与长线 **同一个 confidence 数字**。四档不是四个预测 |
| R2 | **档间唯一差别是一个经验乘数**：仓位 `base × conf × period.factor()` | `scoring.rs:147`；`harness/src/holding_period.rs:52-58`（0.4/0.6/0.8/1.0） | 即被判定为 E6 的那张表。分析链已换成「风险预算 + 凯利 + SNR」（Phase D-2），荐股链没换 |
| R3 | **两张周期仓位数字并存**：荐股 `factor()`(0.4/0.6/0.8/1.0) vs 决策链 `position_multiplier()`(0.6/0.8/1.0/1.2)，注释自述「语义不同」 | `holding_period.rs:52` 与 `:87` | 同一票同档在荐股面板与分析面板给出不同仓位，且无一致性门 ⇒ 用户看到两处矛盾数 |
| R4 | **全链只吃日线**：所有策略 `get_klines(code, "daily", limit)`，四档差别只是日线根数（20/60/150/300） | `trend.rs:69`、`capital.rs:41`、`value.rs:52/80/180`、`reversion.rs:40`、`mod.rs:408` | 分析链已做「超短吃 60 分钟、长档吃季度聚合 + `ScoreBands::scaled_for`」（`astock-data/src/scale.rs`）⇒ 两条链对同一档**取数口径互相矛盾** |
| R5 | **止损/目标仍是拍的固定百分比**（E5 本体） | `trend.rs:32-39`（2/5/8/15% − 5/10/20/30%）、`value.rs:144-176`、`capital.rs:77-84`、`watchlist.rs:57+` | 与标的波动率无关。整个 `recommender/` grep `vol_move\|realized_vol\|atr` **零命中**；也没有分析链的 `stopSource` 降级标注 |
| R6 | **持有天数绕开唯一权威源**：`capital.rs:77-84` 自带逐档天数（short=**7**）、`watchlist.rs:65/82` 读自己的变量 | 而 `holding_period.rs:63-68` 声明 `default_holding_days` 是全仓唯一天数源（short=**5**） | 短档荐股按 7 天判成熟、反思/回测按 5 天分档 ⇒ **错档**，且是「两处都自洽、两边打架」的形态。**为何没被门拦住**：权威源内的测试 `holding_period.rs:142-148` 只断言了 `m["mid"]["days"]==28` 一档 ⇒ 声明的「唯一来源」只约束了源自己，**消费方漂移零门禁**（本计划 Phase R-A 要补的就是这个） |
| R7 | **期限结构两端缺失**：`reversion` 在 ultra_short 与 long 直接 `return None` | `reversion.rs:79`「超短线不适用」/`:148`「v1 不做长线超跌」、`mod.rs:550/572` | PLAN §一.2 的「短期反转 / 中期动量 / 长期反转」在荐股链=中段有、两端无 |
| R8 | **档-因子错配**：`value`（PE/PB 估值）却做了 ultra_short | `value.rs:46-73`（PE≤60 + 价在 MA10 下方，持有 2 天） | 周月尺度才可兑现的估值信号接到 2 日博弈；与 R7 是同一件事的两面：** nobody 校验过「该档该不该有这个风格」** |
| R9 | **置信度被改写成本组相对分位后直接落库/展示** | `scoring.rs:195-232`（组内 min-max，top 恒 100），且发生在质量校准**之后**（`mod.rs:659-684`） | 面板上「置信度 92」不是 92% 胜率，而是「今天组内第二好」。跨档、跨次运行不可比 ⇒ 逐档标定的输入被污染 |
| R10 | **校准器忽略市场状态 + 基准硬编码**：调用传死 `"neutral"`，基准 0.50 | `mod.rs:662-666`；而被调方 `backtest_strategy.rs:1282-1290` 本身支持 bull/bear 先验、κ=20 收缩 | 有现成的分档/分市场能力不用；`sample>=5` 与分析链 `IC_MIN_SAMPLE=8`、命中率 5 三套门槛各写各的 |
| R11 | **自适应权重不分期**：`ADAPTIVE_WEIGHTS` 是全局单份四元组，回测 EWMA 调优 | `scoring.rs:22-49` | 超短与长线共用一套因子权重、且互相污染（E1 在荐股链的形态） |
| R12 | **反身性折扣只压超短且无推导** | `mod.rs:671-674`（UltraShort ×0.85，其余恒 1.0） | 拍的数；无标注、无门、无「为什么另外三档是 1.0」的可检性 |
| R13 | **一次只算一档** + 无跨档一致性 | `mod.rs:391-576`（`recommend_stocks_inner(client, period, …)`）、按 period 分键缓存 `mod.rs:227` | 四档要看 4 次全量扫描；Phase F 立的「同分必须互标 `sharesPosteriorWith`」在这条链**没有对应物** ⇒ 分不清收敛与复制 |

---

## 二、前端呈现诊断（F1~F9）

| # | 缺陷 | 证据 | 后果 |
|---|---|---|---|
| F1 | **超短档显示成「长线」**：三元只判 `short`/`mid`，`else` 落 `period.long` | `src/components/stock-analysis/dual-view/CompactRecommendation.tsx:76-83` | 直接造假：档位标签指向错误的档。**属「伪装成正常结果」，不可接受** |
| F2 | 策略矩阵**漏一整档**：`PERIOD_KEYS = ["short","mid","long"]` | `src/components/stock-analysis/RecoStrategyMatrix.tsx:8` | 超短档回测统计在矩阵里**静默不存在**，用户以为该档无数据 = 无缺陷 |
| F3 | **跨档简单平均把周期维度抹平**：按 style 聚合时对各 period 的胜率取算术平均、signals 直接相加 | `RecommendationPanel.tsx:98-116`（徽章渲染 `:556-581`），而后端 `StrategyStats` **本来就带 `period`**（`src/types/stock-analysis.ts:685`） | 数据在手却主动丢弃；平均还没按 signals 加权 ⇒ 徽章数字既非任一档、也非可信总体 |
| F4 | **缺席显示成 0**：`fmt(pick.holdingDays, 0, fmt(0, 0))` / `fmt(pick.confidence, 0, "0")` | `RecommendationPanel.tsx:829`、`:832` | 「持有 0 天」「置信度 0」是**读数**，不是缺席声明。违反本仓已立的「结构性缺席不得在 UI 造成歧义」 |
| F5 | `degradedReasons` **从未被消费** | 仅类型定义 `src/types/stock-analysis.ts:807`，全 `src/` grep 无读取点；降级提示只渲染 style 名（`RecommendationPanel.tsx:470-475`） | 后端逐风格降级归因文本被丢弃 ⇒ 「为什么这档这个风格降级」用户看不到 |
| F6 | 兜底合成票在缩略视图**无标识** | `CompactRecommendation.tsx:17` 声明了 `synthetic`，渲染 `:87-120` 完全忽略，且排序取 Top3 前未过滤（`:55-61`） | 主面板过滤了合成票（`RecommendationPanel.tsx:542`）却在缩略视图放行 ⇒ 同一份数据两处口径相反 |
| F7 | **档位不随跳转传递**：`jumpToStock({code,name})`、`startAnalysis(code)` | `RecommendationPanel.tsx:870`、`hooks/useStockJump.ts:7-15`、`RecoHistoryModal.tsx:110-118`；分析侧 `recoPeriod`（`types:322`）未被渲染 | 荐股「超短档推荐票」→ 打开分析，看不出分析主档是否也是超短；两链的对照入口形同虚设 |
| F8 | **三套档位显示名并存**：`stockAnalysis.period*` / `stockAnalysis.recommendation.period*` / 定时任务一套 | `src/i18n/locales/zh-CN.json:15244-15247`、`:16447-16450`、`:6508-6511`；另反思侧 4 键仍写「超短线 (1-3天)」`:16851-16854` 与权威 2 交易日冲突 | 改名改不完；文案自证与代码不一致（这条在分析链也是遗留） |
| F9 | 历史**详情条目**丢档位（列表页正常） | 列表有 period 列（`RecoHistoryModal.tsx:223-231`）；详情按 style 分组的 Collapse 条目只渲染 code/name/BUY/synthetic/confidence/时间（`:305-338`），而 `RecoDetailItem.period`（`:22`）已取到数据 | 历史票看不出当初荐它是哪一档 ⇒ 复盘按档判成熟时无从对照 |
| F10 | 测试面缺口：F1~F6 路径无一覆盖 | 已有测试只覆盖 `RecommendationPanel.asOfDate/seedPoolOrigin/syntheticMarking/ScreenerToWorkspace`；夹具 `period` 恒 `ultra_short` | 门只能防住已被测的那一面 |

---

## 三、方法学依据（荐股链特有，其余沿用 `PLAN-multi-horizon-decision-science.md` §一）

1. **预测的粒度必须与持有期一致**：2 天与 90 天是两个不同的一阶矩/自相关结构，取数尺度（bar）与
   因子窗口必须同时随档变 —— 对应 R1/R4。不是把日线多喂几根就叫「长线视角」。
2. **短期反转 / 中期动量 / 长期反转**的期限结构是学术主线：长档必须有一个反转类风格，
   短档不该用估值因子 —— 对应 R7/R8。
3. **风控按波动率与持有期导出**（vol-scaled barriers / 波动率管理组合，Moreira & Muir）—— 对应 R5。
4. **仓位承载风险预算**（`min(凯利%, 100·R/止损%)`），不用经验乘数 —— 对应 R2/R3。
5. **度量必须保持量纲可解释**：分位数（percentile）与概率（win rate）不能同名同字段混用；
   一旦做组内归一化，原值必须同时保留 —— 对应 R9/F4。
6. **换手成本 ∝ 1/持有天数**：超短档不做这层修正，净期望系统性高估 —— 对应 R12（现在用一个拍的 0.85 代替）。

---

## 四、本仓真实能力边界（决定哪些立刻接线、哪些必须新建）

| 需要的东西 | 现状 | 结论 |
|---|---|---|
| 逐尺度取数白名单 + 未知 period 报错 | **已有**（`astock-data/src/scale.rs`、季度/60 分钟通路） | 直接接线（R4），禁止在荐股链再造一份 |
| 波动率止损/止盈基元 | **已有**（`harness::indicators::vol_move_pct` / `realized_vol_pct`，样本标准差口径已收编） | 直接接线（R5） |
| 逐档先验（经验贝叶斯收缩） | **已有**（`analysis-engine/src/horizon_prior.rs`，κ 可调，source ∈ own/shrunk/pooled/neutral_default） | 直接接线给荐股用（R1/R10），**前提是不新建第二份收缩实现**（禁区 12） |
| 逐 (style, period) 信号质量 | **已有**（`backtest_strategy::get_signal_quality`，id 形如 `trend_mid`）；`StrategyStats` 已带 `period` | 观测面不必新建（R10/F3） |
| 逐档 rank IC 基元 | **已有**（`harness::indicators::spearman_rank_ic` + `IC_MIN_SAMPLE=8`） | 荐股侧只需按档喂 (conf, 实际收益) 序列（R9 修好后才有意义） |
| 荐股策略的非日线尺度因子组 | **不存在** | 必须新建（R4 的真实工作量：weekly/monthly/quarterly 的 MA/突破窗口语义） |
| 「该档该不该有该风格」的契约 | **不存在** | 必须新建（R7/R8），是本轮最大的一块新增判据 |
| 四档并置的请求形态 | **不存在**（每次一档） | 待拍板 §六 Q1 |

---

## 五、目标架构

```
候选池（现有：热股榜 + 行业龙头 + 流动性过滤）
   │  ← 新增：流动性窗口按档（2 天博弈 vs 90 天持有的可成交性不同）
   ├─ 每档：尺度化取数（hourly / daily / weekly / monthly→quarter）  ← 复用 astock-data::scale，替换 R4
   ├─ 每档：风格可用性契约（style × period 白名单，缺档必须声明原因） ← 替换 R7/R8
   ├─ 每档：置信 = 该尺度因子分位 × 该档先验 prior_h（复用 horizon_prior） ← 替换 R1/R10/R11
   ├─ 每档：风控 = k1/k2 · σ_daily · √h（复用 vol_move_pct）             ← 替换 R5
   ├─ 每档：仓位 = min(凯利%, 风险预算 R/止损%)，退役 factor()           ← 替换 R2/R3
   └─ 每档：成本修正 ∝ 1/持有天数                                        ← 替换 R12
输出：pick 携带与分析链**同一族**口径标注（scoreSource/priorSource/stopSource/
      positionSource/confidenceBasis），缺席各占一句，同分互标
展示：档位标签取自唯一权威天数表；分位与原概率分列；跨档对照成图
```

**类型复用约束（禁区 12）**：口径标注一律复用分析链已定的字段名与取值词表
（`tier_native`/`daily_fallback`、`vol`/`fallback_pct`、`risk_budget`/`kelly_only`、`table`/`fallback_unity`），
荐股链**不得**新造同义字段；跨链共享的放在 `harness`（两份 DTO 平行定义正是本仓已付过代价的形态）。

---

## 六、分阶段实施

### Phase UI-0 前端纯缺陷（不依赖后端，可先做，约半天）
- 修 F1（三元 → 四档全枚举，未知档归 `null` 并显式「未知周期」，**不猜**）；
- 修 F2（`PERIOD_KEYS` 补 `ultra_short`，改为从 `Period::ALL` 的前端镜像生成）；
- 修 F4（缺席显示「—」+ 单独成句，不再 `0`）；
- 修 F6（缩略视图与主面板同一过滤口径）；
- 消费 F5（`degradedReasons` 渲染进降级条）；F9（历史详情渲染档位）；
- F3 本轮只做「不再平均」：徽章按 `(style, period)` 逐档显示，跨档汇总若需要必须按 signals 加权并写明口径。
- **验收**：新增 Vitest 逐条覆盖 F1/F2/F4/F6（含**反控**：把 F1 的旧三元塞回夹具必须红）。
- ⚠ 只改 `src/`，不动后端 ⇒ 不触发 `TEMPLATE_VERSION`，但要过 `typecheck`/`oxlint`/`dprint`/`check:i18n:key-parity`（新增文案 11 语言同步）。

### Phase R-A 口径统一（低风险，先消灭「两条链打架」）
1. R6：删除 `capital.rs` / `watchlist.rs` 自带天数，一律 `Period::default_holding_days()`；若某风格确需不同持有期，**先在本文件登记理由并经你批准**，再扩展权威源，不许在策略里另开表。
2. R3：`factor()` 与 `position_multiplier()` 收口（方案见 §八 Q2）。
3. R10/R12：把 `mod.rs` 里硬编码的 `"neutral"`、`0.50`、`0.85` 三处换成有出处的输入或删掉。
- **门**（新增 `scripts/check-reco-horizon-parity.mjs`，带 `--selftest`）：
  ① `recommender/` 下**禁止**出现天数字面量表（`holding_days` 只能来自 `default_holding_days`）；
  ② 禁止 `period.factor()` 与 `position_multiplier()` 同时被消费；
  ③ 扫描面非零才绿（沿用 `check-horizon-weight-parity.mjs:29` 的判据）。
- **自证**：门必须先在**当前 HEAD** 上跑出红（R6 的 `7.0` 就是现成靶），修完才转绿。

### Phase R-B 尺度收编（R4 + R7/R8）
- 策略按档取对应 bar 尺度（复用 `scale.rs`，未知 period 直接报错，绝不静默归 daily）；
  窗口语义随尺度重定义（不是「日线 20 根」改「周线 20 根」就完事，阈值需按 `ScoreBands::scaled_for` 同族归一）。
- 新增 `style × period` 可用性契约表（哪个风格在哪档成立 + 不成立时声明原因），
  把 R7/R8 从「注释里的一句话」变成数据结构。
- **验收**：同一票在四档的技术输入**构造上不同源**（行为测试断言窗口与尺度不同）；
  契约表覆盖 6×4=24 格，空格必须带理由 ⇒ 门 `reco_style_horizon_matrix_is_total`。

### Phase R-C 逐档置信（R1/R9/R11）
- `calc_confidence` 增 `period` 与逐档先验（直接消费 `horizon_prior_map`，**不新建收缩实现**）；
  分期化 `ADAPTIVE_WEIGHTS`（`[Period; 4]` 或 `HashMap<Period, ScoringWeights>`，回测按档 nudge）。
- **R9 是本阶段的阻塞项**：`group_by_style_and_trim` 的 min-max 归一化不再覆写 `confidence`，
  改为新增字段 `confidencePercentile`（并写明它是**组内当日分位**）；绝对 `confidence` 原样保留。
  契约变更需同步：`types/stock-analysis.ts`、回测消费方（`backtest_strategy` 读的是哪个 conf？）、11 语言文案。
- **验收**：同一票四档 confidence 不再恒等，且档间差异可归因到「尺度输入 + 该档先验」两项，
  各自有 `scoreSource` / `priorSource` 标注。

### Phase R-D 波动率风控与成本修正（R5/R12）
- 止损/目标 = `k1/k2 · σ_daily · √h`（复用 `vol_move_pct`），σ 不可得退回现有固定百分比 + `stopSource`；
- 仓位 = `min(凯利%, 100·R/止损%)`，`factor()` 退役为降级分支（与分析链 Phase D-2 同一实现，不重写）；
- 成本按 `1/holding_days` 摊薄进超短/短档的净期望，替代拍的 0.85。
- **验收**：同组 k 下，高波标的止损% > 低波标的止损%（荐股链现在完全不具备该性质）。

### Phase R-E 逐档预测力度量（把 R9 修好的数据用起来）
- 按 `(style, period)` 出 rank IC（复用 `spearman_rank_ic` + `IC_MIN_SAMPLE=8`）与半衰期，
  替代 F3 那个「跨档平均胜率」徽章；**只报数不回写权重**。
- **验收**：IC 缺席分句（补字段 / 攒样本 / 换标的），三种缺席在 UI 各占一句。

### Phase R-F 结构性缺席显式化 + 跨档一致性（R13）
- pick/response 增同族口径标注（复用分析链字段名）；同分互标（荐股链的「同分」= 同票四档 confidence 恒等）；
- 跨档对照入口 + F7 档位随跳转传递。
- **门**：延续 `portfolio_mgr_same_source_rhai.rs` 的形态 —— 副本逐字 + 生产包含性 + 同分必互标 + 顺序断言，
  并加**反控**（删标注必须红）。

**每阶段独立可验、不跨阶段混提交**；全量三条（`cargo fmt --all --check` /
`clippy --workspace --all-targets --all-features -- -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace`）
只在**所有阶段完成后**跑一次。阶段内用 `cargo check` + 定点 `cargo clippy -p axagent-analysis-engine --all-targets`。

**新增可调参数一律五点对账**（seed 变量 / `input_mapping` / 面板 `b()` / 面板分组 `resolve([...])` /
rhai `effective_params`），荐股侧对应 `parse_reco_config` + `recommender` 的 `vars` 读取点 + 设置面板
`StockAnalysisConfigPanel.tsx` + 11 语言 i18n；漏一处即「纸面可调」，而编译与类型检查全绿。
改动种子/模板 ⇒ **bump `TEMPLATE_VERSION`**（当前 105）。

---

## 七、风险与不做清单

- **历史可比性**：R-B 改尺度、R-C 改 conf 语义、R-D 改风控 ⇒ 荐股历史票的回测/命中率与新口径不可混算。
  必须与旧口径**并行产出对比后再切**，并在 `RecoPick` 上带口径版本标记（沿用 `TEMPLATE_VERSION`）。
- **不做**：不在 IC 观测面稳定前让 IC 自动改权重；不把「四档同向」当 bug 修 —— 收敛可以是真的，
  不可接受的是**无法分辨收敛与复制**；不在荐股链重写一套 LLM 分析师证据层。
- **风险**：R-C 的 `confidence` 语义变更会同时打到回测、质量校准、i18n 文案与既有 Vitest 夹具
  ⇒ 必须一次收口，不留「字段同名两义」的中间态（那正是 R9 现在的病）。
- **风险**：R-B 若给 ultra_short 开 60 分钟通路，本机可达性尚未实证（分析链同一问题仍待重启实跑）。
  不可达 ⇒ 必须落 `daily_fallback` 并在 UI 单独成句，**不得**静默用日线冒充小时线。
- **不扩范围**：本计划不触碰分析链（Phase A~F 的遗留核对另册），不重开策略包/预设矩阵议题。

---

## 八、待你拍板（按编号逐条点完即整批开工）

1. **请求形态**：荐股是否改成「一次拿四档」？
   (a) 维持单档请求（现状，切档 4 次全量扫描，跨档一致性只能靠前端攒四份）；
   (b) 后端一次跑四档并返回 `picksByHorizon`（与 R13 一致，代价是单次响应 4× 取数压力，需并发与限流复核）。
   **建议 (b)**，但把「逐档缓存 + 增量」写进同一阶段，避免用户等 4 倍时长。
2. **两张仓位数字表怎么收口**：(a) 荐股直接改用分析链的风险预算口径，`factor()` 退役为降级分支（**建议**，与 R2/R3 同解）；
   (b) 保留两张表但新增门锁住「同一票同档两处仓位差不超过 X」；(c) 合并为一张表，两链共用（会改变分析链已验的 D-2 行为，我不建议）。
3. **`confidence` 语义变更（R-C）**：是否接受「绝对置信 + 新增 `confidencePercentile`」双字段？
   还是保留覆写、只把字段改名成 `confidenceRankInGroup`？（前者更诚实但动契约面更广）
4. **风格×档位契约表（R7/R8）**：`value` 要不要退出 ultra_short？`reversion` 要不要补 long（长期反转）？
   我倾向：**value 退出 ultra_short**（估值在 2 天尺度无可兑现路径），**reversion 补 long**，
   但这等于改变现有荐股结果的构成，需要你确认可以接受短期内的结果面变化。
5. **是否重新参数化 trend 的逐档参数**（`trend.rs:3-5` 自述 P1-7 把 ~52 个 per-period 变量硬编码）：
   (a) 保持硬编码 + 门；(b) 关键档位距离（止损/目标/入场带）改由波动率导出后**只留 k1/k2**（建议，与分析链一致）；
   (c) 重新全量参数化进面板（成本最高，且与「参数可被反思优化」需要五点对账同步）。
6. **前端 Phase UI-0 是否立刻先做**：它与后端六个阶段无耦合，能马上消灭 F1（把超短显示成长线）这类
   不可接受的造假。建议：**先单独一批做完并提交**，后端阶段随后独立推进。

---

## 九、拍板登记（2026-09-29，用户一条消息按编号逐条点完 ⇒ 整批开工）

| 问 | 裁定 | 对计划的影响 |
|---|---|---|
| Q1 | **(b) 一次拿四档** | 新增 **Phase R-0（请求形态）**，排在 R-A 之前：后端一次跑四档返回 `picksByHorizon`，逐档缓存 + 增量必须同阶段落地（否则用户等 4 倍时长）。连带解锁 R13 的跨档一致性（前端同时持四档才谈得上互标） |
| Q2 | **(a) 荐股改用分析链风险预算口径，`factor()` 退役为降级分支** | R2/R3 同解；`Period::factor()` 保留但只作 σ 不可得时的兜底分支，且必须带 `positionSource` 标注（同分析链 `risk_budget\|fallback_kelly_x_mult\|kelly_only` 词表，**不新造字段**） |
| Q3 | **接受双字段** | R-C 按原案：绝对 `confidence` 不再被覆写，新增 `confidencePercentile` 且文案写明「本风格组内当日分位」。契约变更同批扫 `backtest_strategy` 的消费点（回测读的是哪个 conf 必须先确认，不能假设） |
| Q4 | **`value` 不退出 ultra_short** | 该档保留，但 2 天尺度上的估值信号**必须显式声明局限**（属「档-因子错配」而非数据缺席，词表要新开一类，不能借 `daily_fallback` 混过去）。⚠ **Q4 的第二半（`reversion` 是否补 long）用户未表态 ⇒ 本轮不动**，留在待裁定区 |
| Q5 | **(b) 逐档价带改由波动率导出，只留 k1/k2** | R-D 按此收口；trend 的 `trend_stop_mult`/`trend_target_mult` 两个乘数并入 k1/k2 语义，新增可调项走五点对账 + `TEMPLATE_VERSION` bump |
| Q6 | **UI-0 先单独一批做完并提交** | 立即开工；提交只 stage 本轮前端文件（工作树混有并行会话改动 + 分析链未提交余下改动，不猜归属） |

**阶段顺序因此调整为**：UI-0（前端，独立提交）→ R-0（一次拿四档）→ R-A（口径统一）→ R-B（尺度收编）
→ R-C（逐档置信 + 双字段）→ R-D（波动率风控与成本）→ R-E（逐档 IC 观测）→ R-F（缺席显式化与跨档一致性）。

**仍未裁定（不擅自扩范围）**：Q4-② `reversion` 补不补 long；R-B 若给 ultra_short 开 60 分钟通路时的
本机可达性（与分析链同一未决项）；`degradedReasons` 之外是否还要把 `asofDegradations` 全量渲染（UI-0 暂按
现有 hot/industry 两条归因过滤，放开与否待看效果）。

---

## 十、Phase UI-0 落地记录（2026-09-29）

**改了什么（5 个文件，全部为前端呈现面）**

| 缺陷 | 实现 | 位置 |
|---|---|---|
| F1 超短显示成「长线」 | 旧三元 `short/mid/else→long` 删除，改判 `horizonSuffix(period)`：认不出**原样显示键名不猜档**；显示名统一到全仓唯一一套键 `stockAnalysis.timeHorizon*` | `dual-view/CompactRecommendation.tsx` |
| F6 兜底票口径两处相反 | 缩略视图与主面板同一过滤（`!p.synthetic`）；全是兜底时渲染「0 条真实候选 · N 条兜底候选已隐藏」，不再用「暂无推荐」把隐藏说成没有 | 同上 |
| F2 矩阵漏一整档 | `PERIOD_KEYS` 补 `ultra_short` ⇒ 四档全枚举；表头同样走 `horizonSuffix`（原第三套键 `recommendation.period*`） | `RecoStrategyMatrix.tsx` |
| F3 跨档算术平均 | 回测统计状态改 `style → period → stats` 两层；徽章只取**当前档**，Tooltip 逐档列出（缺样本的档显示「—」）。旧实现 (30+60+40+80)/4=52.5 这种「哪个档都不是」的数不再出现 | `RecommendationPanel.tsx` |
| F4 缺席冒充读数 | 持仓天数缺失 → 「—」（不再 `0d`）；置信度缺失 → 「—」（不再 `"0"`） | 同上 |
| F5 归因从未消费 | `degradedReasons` 渲染进降级 Alert 的 description，并进每风格 ⛔ 徽标的 Tooltip | 同上 |
| F9 详情丢档位 | 历史详情条目新增 `<PeriodTag period>` ；列表列的档位渲染从手抄键族收敛到同一单点 | `RecoHistoryModal.tsx` |

**呈现层的一处措辞归一（可回退）**：`recommendation.period*` 与 `period*` 两套旧键的 short 值是「短期」，
而全仓唯一一套 `timeHorizon*` 是「短线」⇒ 荐股 Tab / 历史列 / 回测矩阵表头现统一显示「短线」。
理由：同一屏内档位名必须只有一个写法（此前 Tab 写「短期」、徽章 Tooltip 写「短线」）。

**门禁读数**：`npm run typecheck` 绿；`dprint check` 绿；`oxlint src/components/stock-analysis` 0 warning 0 error；
`vitest` 荐股 6 个测试文件 **35/35 绿**（含新增 `RecommendationPanel.horizonPresentation.test.tsx` 7 测）；
`check:i18n:key-parity` 绿且 **15274 键未增**（本批复用既有键，未新增文案 ⇒ 11 语言文件未被触碰）；`check:eol` 绿。

**反控自证（按纪律）**：把上述三个组件文件临时覆写回 `HEAD` 版本（= 修复前真实形态），
跑同一个测试文件 ⇒ **7/7 全红**；从备份还原（md5 逐一比对一致）后 **7/7 全绿**。即每条断言在修复前形态上都会命中。

**本批刻意未做（不是遗漏）**

- `DecisionBanner.tsx:656-663` 与 `:903-911` 两处手抄四元三元（分析链的档位标签）：同文件混有**未提交的分析链改动**，
  与本批切分不开 ⇒ 留给分析链那批一并收编，不在这里逐 hunk 搬运。
- 历史详情每张卡硬编码 `<Tag>BUY</Tag>`（对 `synthetic` 兜底票同样声称买入）——新登记为 **F11**，
  属另一类造假（不是档位问题），未擅自扩范围。
- 旧的两套档位键族（`stockAnalysis.period*`、`stockAnalysis.recommendation.period*`）现在引用为零，
  **键本身未删**（11 个 locale 文件删键的风险/收益不划算），登记为待清理。
- `RecoStrategyMatrix` 的 `reversion_long` 特例注释未动：那是**前端硬编码的可用性契约**，
  正确归属是 Phase R-B 的后端 `style × period` 契约表，届时整段替换。

## 十一、提交归属问题（UI-0 撞上的，需用户裁定）

`horizonSuffix` / `HORIZON_CAMEL_TO_SNAKE`（本批 4 处消费点依赖的单点工具函数）是**分析链 Phase F 的产物，尚未提交**
（`git show HEAD:src/lib/stock-analysis-utils.ts | grep -c horizonSuffix` ⇒ 0；未提交 hunk 为 +18 行纯新增，HEAD 内无人消费）。

**裁定与落地（2026-09-29）**：用户选 **(a) 附带该 hunk 一起提交** ⇒
- 用 `git apply --cached` 只暂存 utils 的**第一个 hunk**（+18 行），`readHorizonActions` 改用单点键表的那个
  hunk（第二个）**留在工作树未提交**，归属分析链那批；
- 提交 = **`e17e67053`**（6 文件：4 组件 + 新测试 + utils 的 18 行）。pre-commit 行尾门随提交跑过（PASS）。
- **该提交可独立编译已实证**：临时 worktree 检出 `e17e67053` + junction 复用 node_modules ⇒
  `tsc --noEmit` **EXIT=0（零输出）**、新测试 **7/7 绿**；随后 worktree 与 junction 已清理（主 `node_modules` 完好）。
  另先静态确认本批复用的符号/键在 HEAD 已存在：`PeriodKey`(types:719)、`synthetic`(:754)、
  `degradedReasons`(:777)、`StrategyStats.period`(:655)、i18n `timeHorizonUltraShort` / `dataQualitySummary` /
  `workflow.aiPanel.noRecommendations` 各 1 处。

## 十二、Phase R-0 落地记录（2026-09-29，一次拿四档）

**引擎（`crates/analysis-engine/src/recommender/`）**

| 改动 | 说明 |
|---|---|
| `struct PreparedScan` + `prepare_scan()` | 把**与档位无关**的准备（健康探测、建候选池、流动性过滤、Serenity 注入、`RecoConfig`、自适应权重、per-code 锁）从逐档扫描里拆出 |
| `scan_period(prepared, period)` | 原「步骤 3~8」，只消费 `PreparedScan`；两源全挂时由 `empty_error_response()` 出**带归因**的空响应（不静默出空表） |
| `recommend_stocks_multi(client, periods, vars, preseed)` | 一次准备 + 逐档并发；**档位级缓存仍按档生效**（TTL 60 s）⇒ 命中档不重扫；返回 `Vec<(Period, Result<RecoResponse, String>)>`，顺序同入参 |
| as-of 降级切片归属 | 只挂在**本轮真正扫描过**的响应上；命中缓存的档保留其原始那轮自己的切片（否则把上一次运行的降级算成本轮，与 `as_of.rs` 记过的 R5 基线陷阱同族） |
| `notify.rs::run_recommendation_scan` | 原来对每个 period `tokio::spawn(recommend_stocks(p))` ⇒ **4× 建池 + 4× 流动性过滤**；现改 `recommend_stocks_multi`（四档共享一份池），下游去重/阈值/top N 逻辑不变 |

**命令层（`src/commands/stock_analysis.rs`）**

- 新增 `recommend_stocks_all_periods(asOfDate)` → `RecoBatchResponse { byHorizon, failedHorizons }`。
  键空间：外层 camelCase，**档位键保持 `Period` 的 snake_case**（与前端 `PeriodKey` 一致）。
- 整批全失败才回 `Err`；**逐档失败由 `failedHorizons` 承载** ⇒ 前端必须与 `byHorizon` 一起渲染，否则「失败」显示成「该档没推荐」。
- 抽出 `load_reco_served_vars()`（模板变量 + 演化权重回流）与 `persist_reco_response_picks()`，
  单档与批量**共用同一份**（此前落库逻辑只长在命令里，两条链各写一遍就是本仓反复出现的形态）。
- 单档命令 `recommend_stocks` 保留（cron 回填 / 回测 / 既有契约不破）。命令注册由 `build.rs` 自动完成
  （已核对 `src/register_commands.rs` 新增 `recommend_stocks_all_periods`）。

**`harness::holding_period::Period`**：补 `PartialOrd, Ord`（= 变体声明序 = 短→长），
使 `BTreeMap<Period, _>` 的 JSON 键序可复现；新增测试 `ord_matches_declaration_order`
（重排变体即红）。同批新增两个 Rust 契约测试：`reco_batch_response_serializes_camel_case_for_ipc`
（含 snake_case 反向锁）与 `reco_batch_by_horizon_key_order_follows_declaration` ——
**检法先于修复存在**：缺 `#[serde(rename_all = "camelCase")]` 时 `check:serde` 不在 CI、编译也不报错，
前端按 camelCase 读只会恒为 undefined（命中率整张卡就是这么失效的）。

**前端（`RecommendationPanel.tsx` + `types/stock-analysis.ts`）**

- 刷新走批量命令；整批存在 `batch` 状态里，**并用 `batchRef` 做镜像**给 `loadCache` 读。
  教训（已写进代码注释）：把 `batch` state 列进 `loadCache` 依赖 ⇒ 每次写入重新触发挂载 effect ⇒
  新 `reqToken` 丢弃在途响应（replay 下自激循环，两个 Vitest 用例正是这样暴露它的）。
- 同一会话内切档直接用已取到的整批切片，不再发任何请求（旧形态每档各扫一遍 / 各读一次缓存）。
- 逐档失败独立成句：新增 `stockAnalysis.recommendation.horizonFailed` × **11 语言**（15274→15275 键，
  `check:i18n:key-parity` 绿）。⚠ 插值必须 `{{param}}` 双括号 —— 首版写单括号时 i18next **原样输出模板串**，
  测试断言直接抓到（这类「看起来翻译了其实没插值」的缺陷靠断言全文而非只断言 key 存在）。
- Vitest：`RecommendationPanel.horizonPresentation.test.tsx` 增至 9 测（新增「刷新走批量命令 + 切档不再请求」
  「某档失败独立成句」），`RecommendationPanel.asOfDate.test.tsx` 的 4 条命令断言随契约同步改写。

**门禁读数（截至本条）**

- 前端：`typecheck` 绿、`dprint check` 绿、`oxlint src/components/stock-analysis` 0/0、
  `check:i18n:key-parity` 绿（11 文件 15275 条一致）、`check:eol` 绿、
  荐股 6 个测试文件 **37/37 绿**。
- Rust：`cargo check -p axagent --lib` **EXIT=0**、`cargo fmt --all --check` **EXIT=0**。
- Rust 测试与定点 clippy：**正在跑**（构建目录被并行会话占用，日志 `/tmp/r0-gates.log`）⇒ 结论以日志正文为准，
  未跑完之前本阶段不叫「完成」。

**事故登记（自己踩的，值得进门）**：第一次格式化时用
`rustfmt --edition=2024 crates/analysis-engine/src/recommender/mod.rs ...`，而 `analysis-engine` 的
**crate edition 是 2021**（`rustfmt.toml` 的 `edition=2024` 只对 workspace edition 生效）⇒ style edition 不同，
import 重排规则不同，且 rustfmt **顺着 `mod` 递归格式化子模块**，连带改了 9 个不属于本次改动的文件
（`pool.rs`/`indicators.rs`/`strategies/*.rs`）。已用 `--edition=2021` 重跑，使它们回到提交形态
（现 `git diff --stat` 在该 crate 只剩 `mod.rs` + `notify.rs`），并以 `cargo fmt --all --check` 绿收口。
**判据**：改 Rust 单文件时不要用 `rustfmt --edition=<猜的>`，要么 `cargo fmt -p <pkg>`，
要么先读该 crate 的 `edition`；跑完必须用 `git diff --stat` 核对影响面是否只含自己碰过的文件。

### 十二-补、R-0 的一条错判（由我自己写的测试当场抓到，2026-09-29）

原文（错）：给 `Period` 补 `Ord` 时我写下「`BTreeMap<Period, _>` 的遍历序**与 JSON 键序**由此固定 ⇒
批量响应键序可复现」，并写了断言 `["ultra_short","short","mid","long"]` 的测试。

实测报红：
`left: ["long", "mid", "short", "ultra_short"]` / `right: ["ultra_short", "short", "mid", "long"]`。

真契约：`serde_json::Value` 的 `Map` = `BTreeMap<String, Value>`（本仓**未启用** `preserve_order`），
序列化后按**字符串字典序**排。`Period: Ord` 只固定 **Rust 侧迭代序**，固定不了 JSON 键序。

⇒ 三条收口：① 测试改名并据实断言 `reco_batch_by_horizon_json_key_order_is_lexicographic_not_tier_order`
（它仍是有效门：若有人启用 `preserve_order` 或改成 `HashMap`，键序会离开字典序 ⇒ 红）；
② `holding_period.rs` 的 `Ord` 文档注释补上「作用范围仅 Rust 侧迭代序」的边界；
③ **新判据**：展示序（短→长）是**消费方契约**，由 `Period::ALL` 决定 ⇒
待 R-A 的门 `check-reco-horizon-parity.mjs` 里加一条：**前端 `PERIOD_ORDER` / `RecoStrategyMatrix.PERIOD_KEYS`
必须与后端 `Period::ALL` 逐项同序**（当前前端手写四处档位序，无门）。

---

## 十三、Phase R-A(a) 落地记录：持有天数回到唯一权威源（2026-09-29）

**门先立，再修**：新门 `scripts/check-reco-horizon-parity.mjs`（四门 a/b/c/d + 扫描面非零 + `--selftest`
正负对照 6/6 PASS）。对**当时未修的树**跑出 18 条红（a 14 / b 1 / c 3）⇒ 检法在缺陷真实形态上命中。

**修了什么（a 段）**

- `strategies/capital.rs`：删掉内置逐档天数表（其中 short=**7**），`holding_days` 一律取
  `Period::default_holding_days()`（权威 short=**5**）⇒ **荐股短档持有期读数由 7 天变为 5 天**（行为变化，非纯重构）。
- `strategies/watchlist.rs`：两处逐档 tuple 去掉 `holding_days` 槽位；删除 8 处
  `read_f64(vars, "wl|syn_*_holding_days")`；两处 `RecoPick` 改取权威源。
- **摘除 8 个用户可调项（用户裁定）**：`seed_variables.rs` 删 8 个 `Variable` 块
  （`wl_{ultra_short,short,mid,long}_holding_days` + `syn_*` 同名 4 个），`StockAnalysisConfigPanel.tsx`
  同步删 8 个条目，`TEMPLATE_VERSION` **105→106**（变量表随模板快照落库，不升版则面板继续列出并写入
  已无人消费的变量 = 纸面可调）。
  ⚠ 这 8 个可调项本身与分析链 v2 裁定「定档 = 公式、持有天数刻意不可调」相矛盾 ——
  同一事实两处可调，是本轮查出的**产品级冲突**，不是代码风格问题。
- 这 8 个变量**没有 i18n 键**（面板标签取自后端 `Variable.description`）⇒ 11 个语言文件未被触碰。

**门与编译读数**

- `node scripts/check-reco-horizon-parity.mjs`：a **OK（扫描面 9）**、d OK（扫描面 2）；
  b/c 仍红（属 R-A 后续段与 R-D，尚未开工）⇒ 整门 exit 1 是**预期**，不为过门放宽判据。
- `cargo check -p axagent-analysis-engine` EXIT=0；`cargo check -p axagent --lib` EXIT=0；
  `cargo fmt --all --check` 无本次引入的 diff；`npm run typecheck` 绿；`npm run contracts` **PASS**（errors=0）。
- 待跑：seed 相关 `cargo test -p axagent --lib seed`（日志 `/tmp/seed-tests.log`）。

**门禁归属（照实说，不当成自己的红）**

- `cargo check -p axagent-analysis-engine --all-targets` 红在 `analysis-engine/src/reflection_stats.rs:870`
  （E0716，测试代码）—— 该文件 `git status` 为 M，属**并行会话**。
- `cargo clippy -p axagent --all-targets -- -D warnings` 唯一红条在 `commands/analyst_feedback.rs:13`
  （`doc_list_item_without_indentation`）—— 同为并行会话未提交文件。
⇒ 这两类「含测试目标 / 全 target」的门此刻无法由我方转绿；我方改动面（recommender、harness/holding_period、
commands/stock_analysis、seed_*、面板）在 lib 口径下全绿。

---

## 十四、Phase R-B / R-C / R-D / R-E / R-F 落地记录（2026-09-30，一批做完不切分提交）

按用户裁定：**只改不提交**，等分析链那批（`crates/analysis-engine/`、`harness/indicators.rs`、
`reflection_stats.rs` 等）落地后再统一按阶段切。

### R-B 尺度收编 + 风格×档位契约表
- 新增 `recommender/scale.rs`：档位→尺度**唯一入口**（超短=60 分钟、短=日线、中=周线、长=季度聚合），
  直接复用分析链 Phase B 的 `astock_data::scale::ScaleProfile`（未知尺度显式报错，不静默归日线）；
  另给 `bars_for_daily_span`（日历日窗口→bar 数）与 `dev_tolerance`（偏离容差按 √d 归一，与 `ScoreBands::scaled_for` 同口径）。
- `trend.rs`：删掉「每档各抄一份 kline_limit/min_kline/容差」的形态，改 `scale::fetch` + 尺度窗口表
  `windows_for`（每小时/日/周/季各自的 fast/slow/anchor/high），容差由**单一**日线基准
  `trend_ma_tolerance_daily=0.99` / `trend_high_tolerance_daily=0.97` 按尺度推导（用户覆盖值同样换算）。
- `capital.rs` / `value.rs` / `reversion.rs`：`get_klines(..., "daily", N)` 全部改走尺度层，
  窗口按日历日换算（`bars_for_daily_span`）；value 三个分支原先各取一次日线，现共用一份按档取数。
- 新增 `recommender/style_matrix.rs`：24 格 `风格×档位` 契约表，不成立的格**必须带理由码**
  （`oversold_rebound_needs_at_least_mid_horizon` 等）；另单列「出票但已知档-因子错配」的第三种状态
  （`value×ultra_short`，用户裁定保留 ⇒ 登记 `valuation_needs_weeks_to_realize_kept_by_user_decision`）。
  `mod.rs::strategies_for_period` 由该表驱动，测试 `matrix_matches_strategy_selection` 锁「表 ⇔ 实现」不漂移。
- ⚠ **新查出的重复实现（R14，本轮未收）**：`analysis-engine/src/backtest_strategy.rs` 里另写了一套
  策略判据（同样读 `trend_short_ma20_tolerance` / `rev_kline_limit` 等，且**仍按日线尺度**）⇒
  回测复算与实盘产出不同尺度、不同阈值。收口方式是让回测调用策略本体，属独立一段，未塞进本轮。

### R-C 逐档置信 + 分位/概率分列
- `scoring::blend_confidence(prior, score, s)`：`logit(conf) = logit(prior_h) + 2s·(score−0.5)`；
  先验表与分析链**共用** `horizon_prior::horizon_prior_map`（κ 同用 `horizon_prior_kappa`），
  由 `commands::stock_analysis::reco_horizon_prior(db, served_vars)` 构造后注入荐股链（单档、批量、cron 三条路都传）。
- `priorSource` / `priorSamples` 进 `RecoPick`；先验不可得 ⇒ `absent` + 退回纯评分，不假装四档各自成立。
- `group_by_style_and_trim` 不再覆写 `confidence`，分位改出新字段 `confidencePercentile`（R9 的量纲混淆收口）。
- 删掉 `ADAPTIVE_WEIGHTS` 的 `Mutex` + `nudge_scoring_weights`（全仓**零调用方** ⇒ 注释宣称的
  「可被回测/反思调优」是纸面能力；留一张没人写的表只会充当假证据）。

### R-D 波动率风控 + 风险预算仓位 + 成本摊薄
- 新增 `recommender/risk.rs`：σ 一律取**日线**（`harness::indicators::realized_vol_pct`，全仓唯一实现），
  止损 = `k1·σ·√h`、目标 = `k2·σ·√h`（带上下限夹逼），仓位 = `min(策略上限, 100·R/止损%)`，
  成本拖累 = `1 − 往返成本/目标位移`（超短档自动多扣，替代原先只对超短 ×0.85 的拍脑袋折扣）。
- 日线收盘按 `(code, as-of 后缀)` 60 s 备忘 ⇒ 四档共享，不重复取数。
- `stopSource: vol|fallback_pct` / `positionSource: risk_budget|fallback_kelly_x_mult` 进 pick；
  每票 reasons 追加一行风控口径（用户能读出这个止损是怎么来的）。
- 新可调 4 + 1 项：`reco_stop_vol_mult`(1.2) / `reco_target_vol_mult`(2.0) / `reco_risk_budget_pct`(1.5) /
  `reco_round_trip_cost_pct`(0.6) / `reco_conf_sensitivity`(1.0)，四点对账（seed + 面板 b() + 面板分组 + i18n），
  `TEMPLATE_VERSION` **106→107→108**。
- 门 b 转绿：`period.factor()` 只剩 `calc_position_fallback_mult()` 一个消费者（Q2 裁定 (a) 的落地形态）。

### R-E 逐档 rank IC
- 新增 `recommender/ic.rs`：从 `decision_validations`（当时置信 + T+N 实际收盘）取成对样本，
  按 `(风格, 档位)` 算 rank IC；基元**全部复用**（`spearman_rank_ic` / `IC_MIN_SAMPLE=8` /
  `signal_half_life_days`），门槛与缺席词表同分析链；cells 顺序按 `Period::ALL`（短→长）。
- 新命令 `reco_ic_stats`（自动登记）。**只报数不回写权重**。
- 新 DTO 全部 `#[serde(rename_all = "camelCase")]` + 逐键锁测试
  `reco_ic_dto_serializes_camel_case_for_ipc`（含 snake_case 反向锁）—— 因为 `check:serde` 不扫 `crates/`。

### R-F 结构性缺席显式化 + 跨档一致性
- 前端 `RecoPick` 补 5 个口径字段并渲染：σ 不可得 ⇒ 橙色标注「固定百分比，非波动率推导」；
  无先验样本 ⇒ 「该档无历史先验样本，置信度仅由该风格评分得出」；分位与绝对置信同时显示。
- 跨档同分对照：`sameScorePeers()` 在**一次拿四档**的批量结果里找同票同分的其它档并点名
  （`rowSameScorePeer`）—— 没有 R-0 的批量响应结构上做不到这件事。
- 新增文案 4 键 × 11 语言（`check:i18n:key-parity` 绿；⚠ 插值必须 `{{param}}` 双括号，
  写单括号时 i18next 会**原样输出模板串**，已被断言抓到两次）。
- Vitest 增至 13 测（含「分位不得覆写绝对置信」「同分必须点名」两条反控式断言）。

### 待办 / 未做（照实登记）
- **R-E 的 UI 已接**（2026-09-30 补做，不留「后端自知而用户看不见」的半件）：`RecoStrategyMatrix`
  每格第二行显示 `IC ρ · n=`，够不到门槛的格用**分析链既有缺席文案**（`horizonIcAbsenceKey`，零新增键），
  完全没有已验证样本的格另加一句 `backtest.icNoValidatedSample`（新键 ×11 语言，15282→15283）；
  每风格行首显示预测半衰期或其拒绝状态。⚠ 取数用 async IIFE + try/catch，不用 `.then`
  —— mock 同步返回 `undefined` 时 `.then` 抛 TypeError 会把整张表带崩（被 Vitest 当场抓到）。
- **未跑**：全量 `cargo clippy --workspace` 与 `cargo test --workspace`（按 AGENTS 只在所有阶段完成后跑一次，
  且现在必然红在并行会话未提交的两处，见 §十四末）；`tauri dev` 实机核对（见 §十五）。
- **R14**（回测链复刻策略判据，仍按日线尺度）需独立一轮。
- Q4-②（`reversion` 是否补 long）用户未表态 ⇒ 矩阵里仍登记为「按设计不做 + 理由码」，未改行为。


## 十五、R-B 引入的**新后果**（必须实机核对，不能只靠门与单测）

尺度收编之后，「拿不到该档尺度的数据」从**伪装**变成了**真的没有产出**：

- 超短档现在按 60 分钟线取数（`scale::fetch` → vendor `klt=60`）。若本机拿不到小时线
  （分析链同一问题至今未实证可达），`trend/capital/value` 在超短档会**整档返回空**，
  面板表现为「超短档无候选」—— 这是正确行为（不再拿日线冒充），但**用户看到的是一整档空**。
- 长档按季度线（由月线聚合，需 ≥60 根季线 ⇒ 约 15 年月线），停牌/新上市票会整档无票。

⇒ 待办（用户侧，一次 `tauri dev` 即可判定）：
① 超短档是否有票；无票时是否被正确解释为「该档尺度数据本机不可达」而不是「今天没有机会」；
② 若确认小时线不可达，需在 `RecoResponse` 上补一条**逐档结构性缺席声明**（R-F 的
   `scoreSource` 家族在荐股链的对应物）并计入 `data_gaps`，让面板在档标题处单独成句，
   而不是留空 —— 与本仓「拿不到可以，伪装成正常结果不行」的既有纪律一致。


## 十六、本轮（R-0 ~ R-F）最终门禁读数与两次自伤

**读数（全部为实跑，非推断）**

| 门 | 结果 |
|---|---|
| `node scripts/check-reco-horizon-parity.mjs` | 六门 a~f 全绿（结论：荐股四档口径一致）；`--selftest` 9/9 PASS |
| `cargo test -p axagent-analysis-engine --lib recommender::` | **55 passed / 0 failed**（risk 4、scale 4、ic 5、style_matrix 4、表⇔实现 1、仓位/降级/分位若干） |
| `cargo clippy -p axagent-analysis-engine -p axagent-harness --all-targets -- -D warnings` | **EXIT=0**（9 条判据逐条真修，未用 `#[allow]`） |
| `cargo check -p axagent --lib` | **EXIT=0**（新命令 `recommend_stocks_all_periods` / `reco_ic_stats` 已自动登记） |
| `cargo fmt --all --check` | **EXIT=0**（连读两次一致） |
| 前端 | `typecheck` 绿、`oxlint` 826 文件 0/0、`dprint check` 绿、`check:i18n:key-parity` 15283 键 ×11 一致、`check:eol` 绿、`check:contracts` PASS、`check:types` 0 errors |
| 荐股 Vitest（7 个文件） | **47 passed / 0 failed** |

**两次自伤，都是被自己的门当场抓到的（记下来免得下次靠运气）**

1. **`rustfmt --edition=2024` 用在了 edition 2021 的 crate 上**，且 rustfmt 顺 `mod` 递归 ⇒ 连带重排 9 个不属于本次改动的文件。
   修法：按 crate 自己的 edition 重跑使其回到提交形态，再用 `git diff --stat` 核对影响面。
2. **`LazyLock` 别名初始化写错**：把 `Mutex<HashMap<..>>` 提成类型别名后写成 `LazyLock::new(DailyClosesCache::new)`，
   而 `parking_lot::Mutex::new` 需要实参 ⇒ 编译当场红（ENG/MAIN/CLIPPY 同时 101）。
   另有一次 `FMT=1` 的读数在复查后**不可复现**（现两次 `EXIT=0`）⇒ 按「工具回显要可复现」的既有纪律，以复现后的读数为准并在此登记差异。

**仍未做（不是忘记）**：Q4-② `reversion` 补 long（等裁定）；R14 回测链复刻判据的收编（独立一轮）；
`tauri dev` 实机核对（§十五）；全量 `clippy/test --workspace`（按 AGENTS 在所有阶段完成后跑，
且当前必然红在并行会话未提交的 `analyst_feedback.rs:13` 与 `reflection_stats.rs:870`）。

---

## 附：R1~R13 逐条对证结果（2026-10-02，按代码核，不按本文件自述）

本文件的诊断表写于 2026-09-29，之后荐股链已推进（Phase R-A~R-F 落地，见 `03e0f2e85`
及并发会话工作树）。**照本文件的表去「待修清单」会领到已修完的活**，故逐条对证：

| # | 现状 | 对证依据 |
|---|---|---|
| R3 | **已收口（主路径）** | `recommender/scoring.rs:195` 主路径改为 `base × c × 一致性惩罚`，仓位由 `risk::risk_budget_position` 承载；`Period::factor()` 仅存在于降级分支 `calc_position_fallback_mult`（`:216`），且调用方必须把 `positionSource` 标为 `fallback_kelly_x_mult`，门 b 锁这一点 |
| R4 | **已落地，但只覆盖策略链** | `recommender/scale.rs`（Phase R-B）是档位→尺度的唯一入口，trend/capital/value/reversion 全改走 `scale::fetch`；门 f 锁「`recommender/strategies/` 内不得出现写死 `"daily"` 的 `get_klines(`」。**⚠ 未覆盖第三条链**：趋势智选 `commands/stock_workflow/serenity.rs:1485` 仍用 `risk::daily_closes`（日线）推 `vol_stop` / `vol_target`，`:1940` 另一处 `get_klines(code, "daily", 120)` ⇒ 用日线幅度覆盖 28/90 天档，正是 `scale::dev_move` 要归一的错。**登记为 R4b，勿再当「R4 未修」** |
| R1 | **已修（并发会话在途、未提交）** | `recommender/scoring.rs` 新增 `candidate_score_to_win_rate`（候选评分→该档上涨胜率：逐档先验→logit 合成→SNR 时间折算），`serenity.rs` 落库前调用 ⇒ 档间不再逐位相同、且与分析链同量纲（工作树未提交） |
| R2 | **已收口** | 同 R3：经验乘数退出主路径 |
| 未登记于本表、本轮实测新发现 | 智选 `pick_data.reasons` 只剩风控一行 | `serenity.rs:1491` 的 `reasons_base` 取自 LLM 候选自身的 `reasons` 字段，实测 688498 / 002812 四档全为空 ⇒ 面板「推荐理由」实际只展示建仓带与仓位上限，**入选判据一个字没有，且缺席未声明**。登记为 R15 |
