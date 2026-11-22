# PLAN-趋势智选：候选池改「趋势→全关联股票」+ 打分预测力

> 状态：active ｜ 日期：2026-11-01 ｜ 对应版本：v3.0.0+（未定版，待确认）


> 2026-10-31 立。两条用户裁定：① 候选池 = 「从各趋势获取全部关联股票」，不是一上来全市场撒网；
> ② 打分预测力验收 = IC（样本外 > 0）+ 超额命中（跑赢行业/沪深300，> 50%）双口径。

## 一、现网读数（2026-10-31 实测，推翻了代码注释里的旧值）

| 项 | 读数 | 来源 |
|---|---|---|
| 全市场清单 | `market_stock_universe` 6165 行（旧注释写 679，早已过时） | pg-connect 实测 |
| 行业映射 | 4491 行有 `industry`（东财行业名 f100），128 个行业 | pg-connect 实测 |
| 全市场收盘 | `market_daily_close` 6152 只，1996-10-14 起；`close/change_pct/amount` 齐 | pg-connect 实测 |
| 批量行情正门 | 腾讯 `qt.gtimg.cn` 80 码/请求，实测全市场 74 请求零失败 | `market_daily_close.rs:9` |
| serenity 落库 | `reco_picks` 374 行，`pick_data` 含 `serenity_score` 键的 **0 行** | pg-connect 实测 |
| 打分口径 | confidence = 毛利率/增速/ROE/现金流等基本面质地分，非后市预测 | `strategies/serenity.rs:291` |

结论：**数据基础已具备**。缺口在三处——候选池构造（锁死 20 只领涨股 + 每趋势 ≤2 候选）、
因子快照落库（374 行全缺）、打分口径（质地分而非预测分）。

## 二、阶段划分

### 阶段 0：因子快照落库修复（两目标共同的地基）

- 现状：serenity 候选 `pick_data` 缺 `serenity_score` / `bottleneck_composite` / `data_reliability`
  三键（374 行全缺），IC/回测读不到预测时的因子 ⇒ 「预测力」无从验证。
- 改：写侧（serenity 落库路径）把预测因子快照落全。优先塞进 `pick_data` JSON（不新增列，
  避免「加列要重启才生效」；读侧按 JSON 解析）。若 IC 读侧需要 SQL 谓词再补列。
- 验收：新写入的 serenity 行 `pick_data` 含三键 + confidence + 价格/日期（回测配对所需全字段）。

### 阶段 1：全市场因子 IC 普查（预测力目标的方法学必经，不保证结果）

- 用 `market_daily_close`（1996 起，6152 只）算全市场因子，与 T+N 实现收益配对算 Spearman IC。
- 候选因子（全市场批量可取，无需逐票 financials）：动量（1/3/6/12 月）、短期反转（1 月）、
  量价（20 日成交额变化、换手率）、估值分位（PE/PB 相对自身历史分位）。
- 复用基元（禁区 12）：`harness::indicators::spearman_rank_ic`、`reflection_stats::IC_MIN_SAMPLE`、
  `signal_half_life_days`。
- 验收：逐因子给出「样本外 IC / t 值 / 半衰期」表。**诚实结论允许是「无稳定有效单因子」**。

### 阶段 2：候选池改「趋势 → 全关联股票」

- 趋势识别输出改造：`a-trend-scanner`（含原链/快速链）每个趋势给出**行业/概念标签**
  （对齐 `market_stock_universe.industry` 的东财行业名词表），不再只给 2 只手写候选。
- 标签 → 全关联股票：用 `industry` 映射拉该行业全部成分股（现网 4491 只/128 行业）。
  概念板块成分股（东财 board 接口）作为扩展，首批不做。
- 候选池构造：`candidate-pool.rhai` 的「search_stock(keyword) + 领涨股 ≤20」替换为
  「各趋势行业标签 → 全成分股」。
- 验收：一次趋势智选产出的候选池 = 各趋势关联行业全部成分股（几十~上百只），而非 20 只。

### 阶段 3：打分重构（把阶段 1 结论落地到阶段 2 的池）

- 确定性打分 = 阶段 1 选出的 IC>0 预测因子加权，基本面质地分（现 confidence）降为过滤/参考。
- 全关联股票逐票打分排序出票。
- 验收：打分公式的因子权重有阶段 1 读数支撑，无凭空权重。

### 阶段 4：样本外回测验收（IC + 超额命中双口径）

- as-of 回放 + 历史窗口回测：新打分在样本外窗口的 IC > 0、跑赢行业/沪深300 命中率 > 50%。
- 不达标 ⇒ 回阶段 1/3 迭代（换因子/换权重），如实报告。

## 三、次级裁定（我拍板的默认，随阶段推进可改）

1. **预测因子候选集**：动量/反转/量价/估值分位四族，全部可由 `market_daily_close` 批量算出，
   不引入逐票 financials（那会毁掉全市场覆盖的可行性）。
2. **因子快照落库位置**：优先 `pick_data` JSON 内联（不新增列），读侧按 JSON 解析；
   仅在 IC 读侧确需 SQL 谓词时再补列 + migration。
3. **「全关联股票」的数据边界**：= 该东财行业（`industry` 字段）的全部成分股，非产业链精确映射
   （LLM 给不出精确的产业链↔个股映射，且无对应数据源）。
4. **概念板块成分**：首批不做（缺现成取数接口），列为阶段 2 的扩展项。

## 四、代价与风险

- **阶段 1 可能得出「无稳定有效因子」**：这是诚实交付，不是失败；届时打分改造止步于
  「把质地分如实标注为过滤分」，并如实报告「选不出稳定有效的预测因子」。
- **阶段 2 动 LLM prompt**：趋势识别输出 schema 变化会影响两链（原链 + 快速链）与前端
  呈现，需同步 11 语言、测试、门禁（组件中立性/i18n 三道门）。
- **全市场覆盖 ≠ 全市场出票**：最终出票仍是各趋势关联行业成分股里打分靠前的，覆盖广度
  取决于「识别出多少个趋势」，不是「全市场 6165 只都出」。

---

## 五、阶段0+1 执行读数（2026-10-31 落，脚本在 output/tmp/factor_ic_census.sql + factor_ic_summary.mjs）

### 阶段0：因子快照落库（已完成，实测绿）

- 写侧三处补齐：`candidate-assembler.rhai`（新增 `bottleneck_composite` 输出，与 `serenity_score` 分列——
  折价是「数据不确定性代价」，混进复合分会把「证据弱」读成「得分低」）、`data-verifier.rhai`（两分支透传）、
  `stock_workflow/serenity.rs::pick_data_val`（pick_data 落 `serenityScore`/`bottleneckComposite`/`dataReliability` 三键）。
- 读侧零改动：`backtest_validation.rs::serenity_metrics` 的双拼写兼容现成可用。
- 回归测试两条（`rhai_registry.rs` tests）：`candidate_assembler_persists_bottleneck_composite_alongside_serenity_score`
  （DET 票复合分透传 + LLM 票缺席不折 0）、`data_verifier_passes_bottleneck_composite_through`（两分支透传）。
  实测：`__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::stock_workflow::rhai_registry`
  ⇒ **26 passed / 0 failed**（首次跑漏设该环境变量，EXIT=127 = 测试 exe 启动失败，非代码问题）。

### 阶段1：全市场因子 IC 普查（全样本口径；「样本外」留给阶段4回放窗）

数据边界实测（推翻两条旧读数）：
- `market_daily_close` 无 volume/PE/PB ⇒ **换手率、估值分位两族不可算**。
- amount 实测仅 2026-10 有（37368 行；2025 年仅 3 行）⇒ **amt_chg_20 不可测**（此前「1996 起 amount 齐」读数作废）。
- 全市场截面（~5100 只/日）实测自 **2025-10** 起，约一年；63 日纵深 2026-01 中旬才越过 ⇒ mom_12m 判死。

普查结果（横截面 Spearman，平均秩与 `harness::indicators::spearman_rank_ic` 同口径，float8；NW t 取 lag=n）：

| 因子 | T+N | n_dates | 中位截面 | mean IC | 简单 t | NW t | IC>0 |
|---|---|---|---|---|---|---|---|
| **rev_1m（1月反转）** | 5 | 415 | 5156 | **+0.061** | 7.1 | **3.4** | 66.7% |
| | 10 | 410 | 5154 | **+0.062** | 6.9 | **2.3** | 66.3% |
| | 20 | 400 | 5151 | **+0.066** | 7.9 | **2.3** | 64.0% |
| | 60 | 360 | 244 | +0.060 | 6.1 | 1.7 | 69.2% |
| mom_1m | 5~60 | 360-415 | — | -0.060~-0.066 | — | -1.7~-3.4 | 31-36% |
| mom_3m | 5~60 | 318-373 | ~240-375 | -0.006~-0.053 | — | 全不显著 | 42-45% |
| mom_6m | 5/10/20 | 259-304 | ~224 | +0.03~+0.05 | — | 全不显著 | 49-54% |
| mom_6m | 60 | 144 | 89 | -0.143 | — | -1.4 | 27.1% |
| amt_chg_20 | — | 0 | — | 不可测 | — | — | — |

- **唯一稳定有效因子 = 1 月短期反转 rev_1m**：四个视野 mean IC +0.060~+0.066，NW 校正后 5/10/20 日视野仍显著
  （60 日视野 NW t=1.68 边缘）；无衰减（IC 平坦）⇒ 半衰期按 `signal_half_life_days` 判「无法拟合」（非衰减形态）。
- 方向含义：过去一月**跌得多**的票之后跑赢涨得多的。现网 serenity 候选池锁「领涨股」的方向在 A 股全市场
  横截面上**可能是反的**——这是阶段 2/3 打分重构的核心输入（行业中性化后的行业内结构是阶段 2 要重测的面）。
- mom_6m 读数大半来自 ~225 只老票的小截面时代（2026-04 前全市场纵深不够 126 日），不可采信。

普查方法学自证（对拍）：单日 (2026-06-01, mom_1m, n=5) JS 独立重算 Spearman = 0.08195433664，
SQL 逐位一致。过程中抓出并修掉三个实现错：① 并列秩公式（`row_number+(k-1)/2` 会抬高并列组后位元素，
正解 `(min+max)/2`）；② NUMERIC 精确十进制 vs float64 口径差（对齐为 float8，与将来 Rust 移植一致）；
③ **秩必须算在配对样本上**——两侧过滤不同（次新股有 fc5 无 pc21），先排名后 JOIN 会把 y 秩按 5221 全集排
（实测 y max 5221 > 5208），系统性抬高所有靠后秩。

局限（如实声明）：in-sample 普查；档案仅 ~1 年（跨一个牛熊片段）；幸存者偏差（宇宙=今日存续票）；
mom_3m/mom_6m 受小截面时代拖累。诚实结论：**在可测因子族中只有一个稳定有效（短期反转），
动量方向整体为负或不稳定——「无稳定正向动量因子」本身就是阶段 3 打分公式不能凭空放动量权重的依据。**

---

## 六、阶段4 执行读数（2026-11-01 落，脚本 output/tmp/serenity_pool_ic.sql + serenity_hit_rate.sql + serenity_quintile.sql + 汇总 serenity_backtest_summary.mjs）

**先记两条环境事实**（影响后续所有跑 cargo 的人）：
- 并行会话把 `src-tauri/crates/**` 拆成 8 个独立 workspace 根（analysis-engine→`axagent-quant/`、tools→`axagent-runtime/` 等），
  每根自带 `rust-toolchain.toml`（1.97.0）与 target 目录 ⇒ **cargo 必须 cd 进对应 workspace 根**。阶段3 门禁在此布局下复跑全绿（见下）。
- 库内**无指数日线历史**（已核三处：`market_daily_close` 无 000300/399300 行、`astock_daily_snapshot` 仅个股 scope、
  无 index/kline 专表；`portfolio_metrics_daily` 只存 beta 不存基准序列；每日归档白名单不含指数）
  ⇒ 验收第二条基准改用「**全市场等权**」显式替代，不静默顶替 沪深300 之名。

### 阶段3 终局门禁读数（拆仓后最终盘态）

`cargo fmt --check` 三 workspace = 0；主 crate `clippy --all-targets --all-features -- -D warnings` **EXIT=0**（17m45s）；
`rhai_registry::tests` **34 passed / 0 failed**（含 4 条阶段3新增）；`seed_consistency` 定点 5 passed；
`axagent-analysis-engine` stock_factor 3 passed；`axagent-tools` stock_factor 3 passed。

### 回测口径

- 池 = 东财行业全部成分股（`market_stock_universe.industry`，与 `get_industry_constituents` 同源，128 行业，池内配对 ≥20 才计入）
- 分 = rev_1m（score = -(close/lag21-1)；生产的百分位变换单调 ⇒ Spearman 与直接用 score 同值）
- 窗：全窗 2025-10-01..2026-10-30（228 个交易日）；在样窗 <2026-07（151 日）；伪样本外窗 ≥2026-07（77 日）
  ⚠ **伪样本外**：因子族与方向的选择用了全样本（阶段1），切窗只是稳定性检验，不是无泄漏 OOS
- 变体：`all_industry`（全部行业池）/ `top10_hot`（当日行业 1 月动量前 10 = 伪趋势近似，真实趋势由 LLM 给出、历史不可复现）
- 出票：每池 Top-5（`serenity_hit_rate.sql`）；另按五分位复算（`serenity_quintile.sql`）判「k 太窄」还是「因子不稳」

### 口径一：IC —— 达标（正），但幅度小且时间不稳

| 变体 | n | 全窗 mean IC | NW t | IC>0 | 在样窗 | 伪样本外窗 |
|---|---|---|---|---|---|---|
| all_industry | 5 | **+0.047** | 3.0 | 64.9% | +0.025（NW t 1.8） | +0.090（NW t 2.7） |
| all_industry | 10 | **+0.058** | 2.7 | 70.9% | +0.026（1.6） | +0.124（2.7） |
| all_industry | 20 | **+0.071** | 2.2 | 69.0% | +0.036（1.9） | +0.156（2.1） |
| top10_hot | 5/10/20 | **+0.082/+0.087/+0.091** | 4.0/2.9/2.0 | 64.5~66.7% | +0.044~+0.054 | +0.137~+0.189 |

⇒ **IC>0 通过**；行业内池的中位池数 74、配对 ~3800 只/日。但两窗差 3~4 倍 ⇒ 方向强度依赖时期。

### 口径二：超额命中 —— 未达标（全窗 48%，门槛 >50%）

Top-5 池均值口径（`池-日` 为单位）：

| 变体 | 方向 | n | 全窗跑赢行业 | 全窗跑赢等权 | 在样(行业) | 伪样本外(行业) |
|---|---|---|---|---|---|---|
| all_industry | **new_score** | 5/10/20 | **48.2 / 47.7 / 48.6%** | 46.4 / 45.9 / 44.8% | 48.1 / 47.0 / 46.1% | 48.5 / 49.2 / **54.5%** |
| all_industry | old_leader（对照） | 5/10/20 | 43.6 / 41.2 / 40.4% | 44.5 / 42.2 / 40.4% | 48.9 / 47.9 / 46.4% | 33.1 / 27.3 / 25.7% |
| top10_hot | **new_score** | 5/10/20 | **50.0 / 50.7 / 49.7%** | 51.4 / 52.4 / 49.1% | 50.1 / 51.6 / 46.7% | 49.7 / 48.8 / 56.9% |
| top10_hot | old_leader | 5/10/20 | 43.4 / 41.2 / 37.6% | 45.7 / 43.6 / 38.5% | 47.5 / 46.0 / 43.0% | 35.3 / 31.1 / 24.2% |

五分位口径（q5=跌最多，q1=涨最多）：全窗 spread **+0.36/+0.72/+1.10%**（all_industry n=5/10/20），
top10_hot **+0.57/+1.56/+2.34%** ⇒ 方向为正；q5 跑赢池均：全窗 49.4~50.2%，top10_hot n=10 53.0%。

**稳健性（决定性一击）**：holdout 剔除 2026-07 单月后 all_industry n=20 的 q5 命中率 **57.3% → 42.9%**（n=10 53.0%→47.4%）；
月度 spread 摆动剧烈（2026-04 −3.15%、2026-05 −3.50%、2026-07 **+10.25%**、2026-08 +0.67%）。
在样窗剔除最差两月也只有 50.2%/49.8% ⇒ **「过 50%」依赖单月，不具稳定性**。

### 阶段4 判定

- 口径一 IC>0：**通过**。
- 口径二 超额命中 >50%：**未通过**（全窗 48.2~48.6%；五分位 49.4~50.2%；唯一过线读数依赖 2026-07 单月）。
- 但**新方向确实优于旧方向**（全窗 48.2% vs 40.4%，且 old_leader 在 holdout 崩到 25~33%）
  ⇒ 阶段2/3 的「撤掉领涨股优先」是对的，只是「单因子打分足以跑赢基准」未被证实。
- 按 PLAN 第四节预置处置：**不达标 ⇒ 回阶段 1/3 迭代 + 如实报告**。当前库内可算因子族已被阶段1 穷尽
  （无 volume/PE/PB ⇒ 换手率/估值族不可算；amount 仅 2026-10 有 ⇒ amt_chg_20 不可测）
  ⇒ 「换因子」在现数据面上**没有第二个显著候选可换**，迭代必须先补数据面或改出票粒度，两者代价不同，待裁。

---

## 七、阶段5 裁定与执行读数（2026-11-01，用户点选「开新一轮：补数据面再换因子」）

### 先普查既有实现（禁区 12 / 「扩字段 > 换申报 > 新建」）

| 通道 | 现场读数 | 结论 |
|---|---|---|
| `harness::market_data::StockQuote`（实时批量） | 已带 `volume` / `amount` / **`turnover_rate`** / `pe` / `pb` / `total_mv` / `circulating_mv` | 采集侧**已在手上但丢弃** ⇒ 走「扩字段」，不新建表、不换 vendor |
| `harness::market_data::KLine`（日K线回填） | 带 `volume`（`vendors/tencent.rs:381`，手→股）；**`turnover_rate` 恒 `None`**（同文件 :370）；2026 年 API 日线仅 6 字段、`amount` 常缺 | **量能族可回填历史**；换手率/市值**不可回填**，只能从加列当天往后攒 |
| `market_daily_close` 实体 | `pe_ttm` / `pb` 已由并行会话于 2026-11-01 加列并接通写侧（服务荐股价值桶） | ⚠ **不重复实现**；本批只补 volume / turnover_rate / total_mv / circulating_mv |
| 库内表清单 | 无 index/kline 专表；`astock_daily_snapshot` 仅个股 scope；`portfolio_metrics_daily` 只存 beta 不存基准序列；归档白名单不含指数 | 阶段4 的「沪深300」缺口属**数据面缺失**，非读侧漏接 |

### 本批落地（只改不提交）

1. **实体加四列**（`axagent-core/crates/entities/src/market_daily_close.rs`，紧随 `amount` 之后）：
   `volume`（`default_value = 0`，**0 = 未采集**，与 amount 同缺席语义）、`turnover_rate` / `total_mv` /
   `circulating_mv`（`Option<f64>`，NULL = 未采集；市值列的用途写明是**规模分层/中性化判据** ——
   反转因子若只在小票成立，单因子全池打分就是拿子层效应冒充全池效应，必须查得出来）。
   加列正门 = 实体列属性（`schema_diff.rs:236` 按 `E::Column::iter()` 自动 additive reconcile，启动期生效需重启）。
2. **写侧**（`market_close_store.rs::close_row`，:93 起）：新参三个置于形参末尾；
   `volume` 即便只用于推定成交额也**必须单独落库**（量能族原料）；换手率 ≤0/非有限 → NULL
   （**停牌日换手率就是 0**，落成 0 等于伪造一片有效低值观测）；市值 ≤0/非有限 → NULL；
   既有判据不回归：负 PE 仍原样保留。
   实时批量调用点（:678）透传 `q.turnover_rate / q.total_mv / q.circulating_mv`；
   `on_conflict` 的 `update_columns` 同步加四列（:693 起），否则重采当日只覆盖旧列。
3. **回填侧**（同文件 :240 起）：KLine 的 `volume` 落库（同样的 >0/有限判据），
   `turnover_rate / total_mv / circulating_mv` **显式 `Set(None)`** —— 回填通道给不出，不得伪造。
4. **测试面同批跟**：`market_close_store.rs` 内 10 处 `close_row` 调用补三参；
   `axagent-infra/crates/dao/tests/repo_crud.rs:288` 的 `ActiveModel` 字面量补四列；
   新增单测 `volume_turnover_and_mv_follow_absence_semantics`（钉住①②③与「负 PE 保留」不回归）。

### 门禁读数（三段）

- **已取**：`cargo fmt -p axagent-analysis-engine --check` = 0；`cargo check -p axagent-analysis-engine --lib` **EXIT=0**；
  `cargo test -p axagent-dao --test repo_crud` **6 passed / 0 failed**（实体加列后并行会话的估值测试照样编译通过）。
- **被挡**：`cargo test -p axagent-analysis-engine --lib` **EXIT=101**，两条 `E0609: no field 'serenity' on SeedOverrides`
  报在 `recommender/mod.rs:2075 / :2097` —— 该文件状态 `AM`（并行会话正在逐字写荐股桶的 serenity 种子腿），
  `pool.rs:28` 的 `SeedOverrides` 只有 mover/miss/reflow/user/universe 五字段。
  **不属本批、不代改**（改别人半落地的符号会造出假修复）。
- **待取**：等那批落地后复跑 `--lib market_close_store`（含新单测）+ 定点 clippy + 主 crate 全量。

### 阶段5-2（下一步，需用户动作）

重启应用使四列物理生效 → 跑一次日K线回填铺 `volume` 历史（换手率/市值不可回填）→
重跑阶段1 普查，新增 `vol_chg_20`（20 日量能变化）与往后逐日攒的 `turnover_rate` 两族 ⇒
若出新显著因子则回阶段3 重估权重；若仍无第二个显著因子，**如实报告「数据面已补齐但预测力仍只有 1 月反转」**，
不为凑达标凭空加权重。

