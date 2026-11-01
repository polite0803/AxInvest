# PLAN — 智能荐股系统性重建（2026-11-01）

> 触发：用户确认「要改，而且不是最小顺序，要系统性解决」。
> 本计划不复用 `PLAN-reco-signal-quality.md` 的「最小三步」口径；那份只解决「同一批候选内部排序」，
> 本次要解决的是**产出链本身**：池子 → 评分 → 门控 → 后验 → 组合 → 呈现。

## 0. 现网事实（2026-11-01 本机库实读）

| 事实 | 读数 | 含义 |
|---|---|---|
| 智能荐股真实出票 | 最后一条 2026-10-23 00:24；11-01 四次扫描全部只有 synthetic | 推荐器已结构性停摆 |
| `stock-reco-daily` | 10-26 起每次输出「本次未发现符合条件的新推荐」 | 四个档位全被过滤 |
| 趋势智选真实出票 | 最后一批 2026-10-31 18:35，5 只；10-31/11-01 非交易日跳过 | 面板停在旧批次 |
| 闭环权重 | trend mid/short、capital mid/long、reversion mid/short、value long = 0；其余 0.05–0.24 | 权重 × 置信度 50 地板 = 除 serenity 外全灭 |
| 趋势智选标签 | `outperform/underperform/excess_*` 共 614 条，全部被 `hit_outcome_to_binary_outcome` 判 None | 唯一活着的风格不受闭环约束 |
| 实盘后验（T+5） | capital −1.86%/33.8%、trend −2.86%/30.1%、reversion −2.70%/33.0%、serenity −3.07%/32.8%、value +0.69%/50.8% | 多数风格负期望；唯一正期望的 value 权重被压到 0.06–0.24 |
| 置信度↔后验 | 全样本 −0.115；T+2 −0.021；T+5 −0.161 | 置信度没有排序能力 |

## 1. 设计原则（四条，不可互相替代）

1. **证据与风格健康分离**：`confidence` 是「这只票的证据有多强」，不是「这个风格最近赚不赚钱」。
   风格健康只能作用于**仓位、资格（硬停）与展示**，不得改写证据分后再去撞绝对门槛。
2. **评估与呈现分离，但后验只认一套**：主策略用 `hit/partial/miss/false_hit`，趋势智选用
   `excess_outperform/outperform/underperform/excess_underperform`；两者在**策略权重**层统一为
   「策略是否跑赢自己的基准」，呈现层仍保留各自标签。
3. **池子按策略建，不共用一张小名单**：每个策略的候选集由它自己的廉价预筛产生；
   全市场快照是底座，热榜/行业/兜底只是补充来源，不再无条件混入。
4. **每次扫描必须可观察**：零票也要留批次记录（扫描时间、池大小、各档原因），
   面板必须能区分「扫描过但 0 票」与「没有新批次」。

## 2. 工作流

### W1 评分/门控解耦（基础层，先做）

**改什么**

- `recommender/mod.rs` 的 `scan_period`：
  - `p.confidence` 保持**原始证据分**，不再乘 `style_weight`；
  - `style_weight` 只作用于最终仓位：`position_pct = clamp(pos_before_drag × drag × weight, 0, 95)`；
  - `reco_min_confidence` 继续过滤 `p.confidence`（现在是原始证据分）；
  - 权重 0 仍是**硬停**（跳过该格），但这是「资格」而不是「把分数乘成 0」；
  - 市场状态乘数同样只作用于仓位/资格，不改写证据分。
- `types.rs`：改写 `confidence` / `evidence_score` 字段文档，明确主链是「证据强度」，
  趋势智选链是「经先验合成的胜率」；两者不得再与 `min_confidence` 混用。
- `scoring.rs`：新增纯函数 `apply_style_weight_to_position`，把「权重只缩仓位」这条契约钉在单测里。

**验收**

- 构造 `confidence=60, style_weight=0.11` 的 pick：`min_confidence=50` 必须放行，仓位 = 原仓位 ×0.11。
- 权重 0 的格：仍然跳过，且响应 `degradedReasons` 说明「负期望硬停」。
- 现网同批重扫：不再出现「四档全 synthetic」；至少 value/capital 按证据分重新出现在候选里。
- 不改变趋势智选链自己的 `candidate_score_to_win_rate` 口径。

### W2 统一后验：趋势智选纳入闭环（基础层，先做）

**改什么**

- `reco_loop.rs` 的 `samples_from_validation_rows`：
  - 新增 `strategy_outcome(row) -> Option<(win, realized_return_pct)>`：
    - 通用标签：`hit` ⇒ win，`partial/miss/false_hit` ⇒ loss，收益用原始 T+N 收益；
    - 趋势智选标签：`excess_*`/`outperform` ⇒ win，`underperform`/`excess_*` ⇒ loss，
      收益用**超额收益**（`final_return_pct − factor_snapshot.benchmark_return_pct`）；
    - 趋势智选缺基准 ⇒ 跳过（不拿绝对收益冒充超额）。
  - 去掉「`hit_outcome_to_binary_outcome` 为 None 就 continue」的单域闸门。
- `serenity_hit_rate_backtest.rs` 的文档同步：标签域仍分两套，但闭环现在按域分支消费。
- 趋势智选工作流落库侧（`stock_workflow/serenity.rs`）：
  - live 读取 `(serenity, period)` 的闭环权重；权重 0 ⇒ 本档不落库并在响应里声明原因；
  - 权重 >0 ⇒ 只缩放 `position_pct`，不改写候选评分；
  - as-of 回放**不消费**今天的权重（防前视），保持基线并显式声明。

**验收**

- 构造 serenity `outperform` + benchmark −2%、final −1% 的行：样本入闭环、win=true、
  realized=+1%；`underperform` 同构造 win=false；缺 benchmark 不进分母。
- 同批 generic `hit/partial/miss` 行为不变（既有测试全绿）。
- 闭环面板出现 `serenity` 的权重行；`check-reco-loop-status-coverage` 绿。
- 趋势智选连续两轮负期望后权重下降/硬停，面板能读到原因。

### W3 候选池重建（主体层）

**改什么**

- 新增全市场快照 DTO（`UniverseItem`：code/name/industry/close/change_pct/amount/
  ret_20d/drawdown_20d/amount_ratio_5d）。
- `dao/reco_seed.rs` 新增「最近 N 个交易日收盘历史」查询；命令层组装快照，
  实盘取最新交易日、as-of 取截止日（含前复权口径说明）。
- 引擎新增 `universe.rs` 纯函数：
  - 硬过滤：ST、price ≤0、amount < 1 亿、样本不足；
  - 分桶：`trend`（20 日相对强度 + 流动性）、`reversion`（20 日回撤/超跌 + 缩量）、
    `capital`（成交额放大倍数）、`value`（~~financial_snapshots 的 PE/PB/ROE~~ →
    **W3b 改为 `market_daily_close.pe_ttm/pb`**，取材理由与被退役方案见 §4；
    覆盖不足时显式降级，不伪造）；
  - 每桶取前 K（K 可配置，默认 80–120），union 后仍保留来源计数。
- `prepare_scan` 改为持有 `seeds_by_style: HashMap<Style, Vec<SeedItem>>`；
  各策略扫描自己的桶，不再共用一张 190 只名单。
- `FALLBACK_STOCKS` 只在 hot/industry 都取不到时兜底；兜底计入 `seed_pool_origin.fallback`
  并在响应里显式声明「本次候选全部来自内置样本池」。
- 保留用户/漏检/回灌/趋势智选四路前插；as-of 仍显式清空这四路。

**验收**

- 池来源计数：`hot+industry==0 && fallback>0` 时前端必须显示兜底声明。
- 全市场快照覆盖 ≥5,000 只；分桶后每桶 50–150 只；扫描 K 线请求量不高于现网 2 倍。
- 同批 as-of 回放：池不再 100% 来自热榜/行业/兜底；`seed_pool_json` 非空且来源可读。
- 新增门 `check-reco-pool-coverage.mjs`：断「全市场快照被消费」「fallback 条件化」
  「as-of 用截止日」「每桶非空/上限」四条，带负控。

### W4 组合构建与批次留痕（主体层）

**改什么**

- 跨风格排序：用「校准胜率 × 盈亏比」而不是原始 confidence；同票跨档不重复占位。
- 行业分散：同一行业/板块在单批候选中设上限（默认 ≤30%），避免整批半导体。
- 批次留痕：新增 `reco_runs`（或复用既有 `workflow_executions` 的 run 记录）：
  `generated_at, period, pool_size, seeds_by_style, real_picks, synthetic_picks,
  disabled/cell_reasons, as_of_date`。零票也写。
- 面板：显示「最近扫描时间 / 状态 / 池大小 / 各档 0 票原因」，不再把旧批次当新结果。

**验收**

- 零票批次在库里有一行 `reco_runs`；面板不会把 10-31 的批次显示成 11-01。
- 同一批候选的行业集中度可读；超过上限时有显式截断留痕。

### W5 信号研究：新增正交腿（长期层）

**改什么**

- 价值：接 `financial_snapshots` 的 as-of PE/PB/ROE/毛利率/负债率，先测 IC，再决定是否启用；
  覆盖不足时降级为「仅标注」。
- 趋势：加相对强度（个股 20 日 − 行业/指数 20 日）、ATR 归一斜率；不再只用「均线多头+放量」。
- 反弹：把「缩量回踩」做成关注触发（先验已测：前向 5 日 +1.60% vs 基线 +0.36%，右尾驱动）。
- 资金：多日资金流/融资余额/大宗，替代单日快照。
- 每条腿必须过「样本外 IC > 0 或命中率显著高于基线」才启用；否则只做标注。
- 新增 `check-reco-signal-evidence.mjs`：断「启用的腿有后验读数」「无读数的腿不得硬否决」。

**验收**

- 每条新腿有 as-of 回放的 IC/命中率读数与样本数；
- 没有后验支撑的腿不得以「硬门」形态出现。

**可测域（2026-11-01 现网读数，决定哪些腿今天就能测）**

`market_daily_close` 全表 1,480,154 行 / 6,580 个交易日（1996-10-14 ⇒ 2026-10-30），但**截面厚度**
是按年份极不均匀的：

| 年份 | 交易日 | 行数 | 去重代码数 | 日均截面 |
|---|---|---|---|---|
| 2026 | 198 | 1,062,437 | 5,691 | ≈5,360 |
| 2025 | 243 | 336,900 | 5,452 | ≈1,386 |
| 2024 | 242 | 24,089 | 300 | ≈100 |
| 2023 | 242 | 10,160 | 92 | ≈42 |

⇒ 只有 **2026 年内**是「全市场截面」；2025 只有 1/4 覆盖，2024 及以前是个位数百分比的零星回填。
后果按腿判：

- **可今天测（前向 5/10/20 日标签）**：2026-01 ⇒ 2026-10 的截面，样本约 100 万股票日；
- **各前向窗口的可用股票日实测**（`close>0` 且前向价存在，特征日 2026-01-05 ⇒ 2026-10-30）：
  前向 2 日 **1,051,056** / 5 日 **1,033,986** / 28 日 **904,987** / 90 日 **560,882**
  ⇒ 四档的标签都不缺样本，长线档只是比短线档少 47%；「样本不足」在这张表上不成立，
  真正不足的是**成交额**（见上表下方那条）与 2024 及以前的截面。
- **不可今天测**：任何需要多年稳健性的「跨周期不翻号」判据 —— 2024 及以前没有截面，
  做了就是把 300 只的历史当成全市场。要先把日线回填推进到多年前（属里程碑，不属本计划）。
- 术语更正：本节原写「接 `financial_snapshots` 的 as-of PE/PB」—— 该路径已在 W3b 撤除
  （实测覆盖 8.9% 且样本自指），估值改由 `market_daily_close.pe_ttm/pb` 承载。

**W5 首轮离线读数（2026-11-01，纯 SQL 对 `market_daily_close` 做横截面）**

口径：逐交易日算「因子百分位秩 ↔ 前向 5 日收益百分位秩」的 Spearman 型 IC，只保留当日截面
≥500 只的行；按半年分桶当样本内/样本外对照（**同号**才谈启用）。
样本域 2026-01-05 ⇒ 2026-10-23，可用股票日 853,438（H1 417,289 / H2 436,149）。

| 腿 | 因子定义 | H1 IC | H1 正天数 | H2 IC | H2 正天数 | 判定 |
|---|---|---|---|---|---|---|
| 20 日动量 | `close/close(−20) − 1` | −0.0010 | 47/95 | **−0.0861** | 34/98 | 两半期都为负、下半年放大 ⇒ 现「趋势桶=追涨加分」在这段样本上是**反指** |
| 20 日相对强度 | 上式 − 同行业均值 | −0.0045 | 47/95 | −0.0495 | 32/98 | 与动量同源同向 ⇒ 不能作为独立腿 |
| 5 日短线 | `close/close(−5) − 1` | −0.0036 | 49/95 | **+0.0095** | 48/98 | **跨半期翻号** ⇒ 不满足样本外同号，只能做标注 |
| 流动性 | `ln(amount)` | 不可测 | — | −0.0518 | 0/98 | 见下条数据前置：这个读数不可信 |

**成交额类腿今天不可测（数据前置，不是判据问题）**：`market_daily_close.amount` 的采集是
**2026-10-23 才铺满全市场**的——按来源实测 `tencent_kline`（回填，6 月起 525,801 行）里
`amount>0` 只有 **15 行**，而 `tencent_batch`（当日快照）自 10-23 起每天 5,570 行全部 >0；
10-15 ⇒ 10-21 每天只扫到 ~680 只（老的 `market_stock_universe` 681 域）且 amount 多为 0。
⇒ 量能 / 流动性 / 资金流三条腿的共同前置是「全市场快照连续跑够 5 天以上」，
上表的 −0.0518 只反映 10-23 之后那几天，**不得**当成读数（判据写「不可判定」而不是「0」）。

**顺带证实的一条（与 W3 直接相关）**：`universe::build_universe` 要求某票的
**最新一行**满足 `amount ≥ MIN_AMOUNT`，否则按流动性不达标剔除 ⇒ 全市场桶的可用性
**逐票**取决于「当天的全市场快照有没有把它扫进去」。实测对照：10-23 起每天 5,570 只带成交额，
而 10-21 只有 **684** 只（老 `market_stock_universe` 681 域）⇒ 同样的代码在两日会产出
5,570 只与 ~700 只两种池规模。这就是「面板说扫了 212 只」与「其实能扫 5,570 只」之间的开关，
也说明 W6 的「交易日必须留批次记录」不是运维洁癖，而是这条链的可用性判据。



### W6 运行与门禁（贯穿）

- `stock-reco-daily` 在交易日必须留下批次记录；非交易日跳过要有明确状态。
- 定时链部分完成（Partial）不得被当成成功；面板要显示 partial。
- 门禁接线：新门按 AGENTS.md 的两处同源注册（`ci-check.mjs` + `ci.yml`），
  自检先行；`ci.workflow.test.ts` 的对称差集护栏必须绿。
- 全量门禁只在全部阶段收尾跑一次（clippy --workspace --all-targets --all-features、
  test --workspace、fmt --all --check）；阶段内只跑定点 crate/文件。

## 3. 执行顺序

| 阶段 | 内容 | 交付判据 |
|---|---|---|
| A（本次） | W1 + W2 | 不再全 synthetic；趋势智选进闭环；定点测试绿 |
| B | W3 + W6 的批次留痕 | 全市场分桶候选；零票批次可读 |
| C | W4 + W5 | 组合分散；新腿有后验读数 |

> 顺序不是「最小修复」，而是依赖顺序：W1/W2 不改，W3 的票仍会被门控吃掉；
> W3 不改，W1/W2 只是让同一张小名单重新出票；W4/W5 才解决「质量」。

## 4. 执行状态（2026-11-01 首轮）

- **W1 已落地**：`confidence` 还原为原始证据分；`style_weight` 只缩仓位；
  `reco_min_confidence` 判据收口到 `passes_min_confidence`。新增 2 条单测。
- **W2 已落地**：`reco_loop::strategy_outcome` 双标签域；趋势智选 `excess_*` 以**超额收益**
  进入样本，缺基准剔除；新增 1 条单测。`check-reco-generation` 与
  `check-reco-loop-status-coverage` 均绿。W2b：趋势智选工作流也消费 `(serenity, period)`
  的闭环权重（live 权重 0 ⇒ 本档不落库/不呈现；>0 ⇒ 只缩 positionPct；as-of 不消费，
  防前视）。
- **W3 首片已落地**：新增 `recommender/universe.rs`（全市场快照 → trend/reversion/capital
  三桶，用户/涨幅榜前插，80 只/桶上限）；dao 新增
  `load_close_history_on_or_before` / `load_universe_industries`；命令层
  `refresh_universe_seed_db` 接入**手动四档 / 定时 / 单档**三个入口；as-of 通过
  `SeedOverrides::universe` 显式带截止日快照（不读进程级全局，防并发前视）。
  `analysis-engine` 荐股单测 **173 passed / 0 failed**。
  **现网规模（2026-10-30 快照，SQL 近似）**：5,545 只 ≥21 根；成交额 ≥1 亿 2,983 只；
  其中 20 日为正 549、为负 2,425、量能放大 >1.2 倍 938 ⇒ 每桶取前 80，
  不再是旧的「全池约 190 只」。
- **并行会话已存在的工作**：`analysis-engine/src/industry_constituents.rs` 与 `stock_factor.rs`
  正在做趋势智选的「行业成分股 + 1 月反转因子」；本计划**不重复实现**，推荐器侧的分桶与
  趋势智选侧的全成分股是两条链，不合并。
- **W3b 已落地（价值桶 + 兜底名单条件化），并修正 W3 的一处取材决策**：
  - **退役**：W3 原文写的是「`value` 用 `financial_snapshots` 的 PE/PB/ROE」。现网读数是
    该表最新一天 **1 行**、最好的日子 290 行，且这些行只在单票分析时写入 ⇒ 用它建桶等于
    「在我们已经看过的票里挑便宜的」，是自指样本，不是全市场截面。改为
    **给 `market_daily_close` 补 `pe_ttm`/`pb` 两列**：采集侧走的腾讯批量行情同一行本来就带
    这两个字段（`StockQuote.pe/pb`），此前被丢弃。代价是历史行是 NULL（回放无价值面），
    换来的是 live 与 as-of 同源、**零额外网络请求**（原方案每次扫描要多发 ~38 个批量请求）。
    量化依据（2026-11-01 现网）：最新交易日成交额 ≥1 亿的票 **3,001** 只，其中在
    `financial_snapshots` 里**有任何一行**的只有 **268** 只（8.9%）—— 用旧方案建桶，
    截面覆盖率连 `VALUE_MIN_COVERAGE=0.5` 的三分之一都到不了。
  - 桶内判据：盈利收益率（1/PE，仅 PE>0）与账面收益率（1/PB，仅 PB>0）各排一次**截面百分位**
    再取均值；PE≤0 的亏损票只按 PB 参与，不得因「1/负数」被判成最便宜。
  - 可用性判据：`VALUE_MIN_COVERAGE = 0.5`（带估值的票至少占快照一半）。不达标 ⇒
    `ValueBucket::Degraded`，**不建桶**，理由经 `degradedReasons[value]` 进面板
    （现网在跑过新采集之前恒为此态，如实显示）。
  - `FALLBACK_STOCKS` 由**无条件混入**改为 `should_merge_fallback(hot, industry)`：只有两个
    真实榜源都没给票时才并入（旧形态是每天带同一批 48 只进扫描）。
  - 新增单测：universe 6 条（含覆盖率两侧负控、NaN 估值不算覆盖、亏损票不判最便宜）、
    pool 1 条、dao `repo_crud` 1 条（全 NULL 行不进覆盖率分子、不跨日）、
    close 采集 2 条（PE/PB 随行落库、NaN 归 NULL 而负 PE 保留）。
- **W4-1 / W4-2 已落地（行业字段接通 + 单行业配额）**：
  - 现网读数（2026-11-01）：`reco_picks.pick_data` 1,507 行里**带 `sector` 键的 0 行**
    （`market_stock_universe` 有行业的 4,491/6,165 = 73%，但候选来源建的是
    `(code, name, None)`）⇒ 行业分散判据直接建在这个字段上就是空转。先接通数据面：
    `pool::attach_industries`（纯函数，只填缺席槽、不覆写来源自带行业、映射没有的保持 `None`）
    接进**用户标的 / 涨幅榜 / as-of 涨幅榜**三条来源，与 W3 快照桶共用同一张行业表。
  - 集中度实证（把历史 picks 按行业回join，按 `日 × 档 × 风格` 分组）：
    单行业占比最高的一批是 **serenity mid/long 80%**、**value long 58%**、
    **value mid 50%**、**watchlist long 44%**（样本 ≥5 只的批次里排序）。
    ⚠ 边界：serenity 走工作流链，**不经过**本次的截断点 ⇒ 配额对它无效，那两批 80% 的
    集中度不是这条改动能修的（要修得在智选链另接，属 W4 剩余面）。
  - 配额实现：`MAX_INDUSTRY_SHARE_PER_STYLE = 0.30` ⇒ `industry_quota(10) = 3`，
    与 top-N 是**同一个截断点**（`trim_with_industry_quota`）。做过一版「只把超额票排到组尾」
    的软实现，实测语义下第 10 个名额仍会被第 4 只同行业票占掉 ⇒ 撤掉，改为直接不占名额，
    被挤出的票照旧进 `reco_scan_audit`（漏检归因读得出「算出但被配额挤出」）。
  - 单测 3 条（配额=3、超额不占名额、无行业不受限且不计为配额挤出）+ 1 条 attach 测试；
    `recommender::` 全量 **184 passed / 0 failed**。
    读数：`cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` **0**、
    `cargo check -p axagent --lib` **EXIT=0**、`check-reco-generation` /
    `check-reco-loop-status-coverage` / `check-reco-horizon-parity` / `check-sql-dialect` 绿；
    `check-layer-discipline` 仍只红在并行会话的 `master_key.rs`（新增的跨命令调用都走
    `_shared_reco_pool`，在豁免表内 ⇒ 计数 95 未变，本批没贡献新越界）。
  - **待办（明确列出，不静默）**：① 配额现为代码内常量，做成面板变量要按五点对账；
    ② 「跨风格排序用校准胜率」未做——`isotonic` 校准在本代样本 0 行的前提下物理不可执行，
    现在能接的是闭环后验（`(style,period)` 格的胜率 × 盈亏比），需要先有样本；
    ③ 批次留痕（`reco_pool_batches` 已有写入、**零前端读路径**）与面板的「批次日期/年龄」
    （现在只显示 `16:03` 这种时分，旧批次看起来像新结果）→ 拆成 W4-3，本轮开工。
- **W4-3 开工前的现网读数与口径修正（2026-11-01）**：
  - 留痕表本身有数据（`reco_pool_batches` 307 行 = `reco` 238 + `serenity` 69）：`reco/live` 14 行、`reco/mover_replay` 148 行、
    `reco/time_travel_replay` 62 行、`serenity/live` 9 行；`reco/live` 里 1 行 `seed_pool_json IS NULL`、
    1 行 `stock_count=0` ⇒ 面板侧必须把「没批次 / 有批次但池未留痕 / 池真的空 / 池 N 只」分成四句，
    与 `stock_analysis.rs:4705` 的 0-C 裁定同一条纪律（`None` 与 `"[]"` 含义相反，不得折空）。
  - **先修口径再接读路径**：引擎的 `seed_pool_snapshot` 只序列化 `seed`（基础池），而 W3 之后四个主
    策略实际扫的是 `strategy_seeds` 各桶 ⇒ 直接接上面板会把「在桶里」的票留痕成「不在池」。
    改造前基线（`reco_picks`，`synthetic=0 AND as_of_date IS NULL`，近 30 天 461 行）：
    `seed_pool_json IS NULL` **0 行**、票码不在自己快照内 **0 行** —— 今天这条留痕是真的，
    破坏它的是 W3，所以修它是 W4-3 的第一件事而不是新特性。
  - 池规模对照：现网基础池 212 只（`reco/live` 最新批），而 `market_daily_close` 最近交易日
    **5,570 只**、全表去重 6,152 只 ⇒ 快照若不含分桶，面板会说「本批扫了 212 只」而真实可扫面大 26 倍。
- **W4-3 已落地（并集留痕 + 面板批次日期/年龄 + 留痕读路径）**：
  - **口径修正（先行，属 W3 的配套）**：留痕改为**本轮各策略输入的并集** ——
    `recommender/mod.rs` 新增 `pool_snapshot_json`（唯一构造点：升序 + 去重，因为桶来自
    HashMap，不排序会让同两份同池产出两份字节不同的留痕）与 `pool_snapshot_of(raw_seed, seed, buckets)`；
    `PreparedScan` 多带一个 `pool_snapshot: String`（在**准备阶段**算一次，四档共用，不再逐档重拼），
    `scan_period` 的 `seed_pool_snapshot` 由「只序列化 `seed`」改为读该字段。
  - **消费侧断言**（不是只断字符串含该码）：`pool_snapshot_includes_bucket_only_candidates` 里
    把产出的 JSON 直接喂给真正读它的 `mover_recall::code_in_pool_snapshot_opt`，断桶内票判
    `Some(true)`、既不在池也不在桶的票判 `Some(false)` —— 形态与解析侧错开时归因会整批读成
    「不在池」，而只看字符串包含的断言照样绿。
  - **读路径**：dao `latest_live_pool_batch(db, chain)`（`select_only` 六列、**排除** `seed_pool_json`
    原文但保留其存在性、写死 `as_of_date IS NULL`、按 `generated_at DESC` 取一行）→
    命令 `get_reco_pool_batch`（无入参，链由 `RecoChain::Reco` 给定，值域不手抄字符串）→
    面板 `data-testid="reco-pool-batch"` 一格，**四种缺席各占一句**：
    「池 {{size}} 只」/「池为空」/「未留候选池快照」/「该链还没有实盘批次」，
    第五种（读失败或形状不符）**什么都不显示** —— 面板不替后端编造结论。
  - **批次时刻**：`03:49` ⇒ 日期 + 时分 + 满一天才给「N 天前」（跨年也读得清）；
    `generatedAtText` 这个已格式化的 state 撤掉，改存原始毫秒，格式化只发生在输出侧。
  - **DTO 对齐补记**：TS 侧 `SeedPoolOrigin` 此前**缺** `reflow` / `moverMiss`
    （Rust 七个计数域全部必送，无 `skip_serializing_if`）⇒ 补齐并把该测试夹具的默认值同步，
    否则「池子由什么推动」这个维度在类型层面就是少两列。
  - 现网读数（新命令今天会显示什么）：`generated_at=2026-11-01T03:49:11.540`、`stock_count=212`、
    `degraded=f`、`has_snapshot=t`、`purpose=live` —— 即旧代码那一批的真实留痕；重启并跑过
    全市场快照后这一格应变到数千只量级（面板上第一次可以直接回答「是不是只有那几个票」）。
  - 测试：Rust 引擎 2 条新单测（`recommender::` **179 passed / 0 failed**，含消费侧断言）+
    dao 3 条（跳过更晚的回放批 / 未留痕 ≠ 空池 / 按链切片 —— 第三条把智选批故意造得**更晚**，
    漏掉 chain 谓词才会红）+ 前端 7 条（四态、契约漂移不下声明、年龄两条控制）。
    **变异自证**：把 `null ⇒ none` 改成 `null ⇒ unknown` 后**只有 D 那一条红**（1 failed / 6 passed），
    复原后 7/7。
  - 门禁读数：`cargo clippy -p axagent-dao --all-targets -- -D warnings` **0**、
    `cargo test -p axagent-dao --lib` **370 passed / 0 failed**、
    `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` **0**、
    `cargo test -p axagent-analysis-engine --lib -- recommender::` **179 passed / 0 failed**、
    `check-reco-generation` / `check-reco-horizon-parity`(a–q) / `check-i18n-key-parity`（16,294 键 × 11）/
    `check-i18n-placeholder-usage` / `check-jsx-text-i18n`（基线外 0）/ `check-i18n-orphan-diff` /
    `check-horizon-weight-parity` / `check-reco-loop-status-coverage` 各 **EXIT=0**。
  - **两处由本批暴露、当场修掉的门红**：① `check-single-source-facts` 报 1 条硬错 ——
    `domain_semantics.rs:377` 的 `evidence` 指向 `recommender/mod.rs:1333`，而我插行后那行成了空行
    ⇒ 按引用处行号重定位到 1384（同一 `position_pct` 的 NaN 守卫，真源是代码不是引用）；
    ② `check-contracts` 报 `[A] 命令已定义但未注册: get_reco_pool_batch` —— 注册表是构建产物
    （`src-tauri/build.rs` 生成 `register_commands.rs`），跑过一次 src-tauri 构建即闭合，
    手改那份文件是禁止的。
  - 复跑读数（前端与契约面，全部按 CI 口径）：`npm run typecheck` **0**、`npx dprint check` **0**、
    `oxlint` **0**（1,424 files）、荐股面板 5 份测试文件 **51/51**、`check-contracts` **EXIT=0**
    （`get_reco_pool_batch` 随 src-tauri 构建自动进 `register_commands.rs` —— 那份是构建产物，
    手改会被下一次构建静默覆盖）、`check-single-source-facts --ci` **EXIT=0**（硬判据 0，LOCATED 598）。
    ⚠ 同一条命令**不带 `--ci`** 时退 **3**：那是软判据（34 处「指向收尾括号 / 引号值找不到」需人判）
    的既有形态，按设计只打印不失败 ⇒ 抄命令时连同 `--ci` 一起抄，否则会把既有软判据当成本批新增。
  - **主 crate 终局读数**：`cd src-tauri && cargo check -p axagent --lib -v` **EXIT=0、0 warning**。
    取这一条是因为前一轮的 `MAIN_CHECK_EXIT=0` **不构成覆盖**：那一次的输出里没有
    `Checking axagent v3.0.0` 这一行（cargo 判它 fresh），而撤耦合批那 4 条 warning 的清除、
    以及新命令 `get_reco_pool_batch` 的编译都落在这一单元上 —— 退 0 只说明「没重编所以没新错」。
    带 `-v` 复跑才拿到 `Dirty axagent … the dependency was rebuilt` + `Checking axagent` ⇒ 这一次
    才是对现源的读数（10m55s，撤耦合批那 4 条 warning 不再出现）。
    ⚠ 教训入例：**`cargo check` 退 0 但日志里没有该单元的 `Checking` 行 ⇒ 该单元没被检过**，
      判覆盖要认这一行，不能认退出码（同「门禁命令的覆盖面本身就是门禁的一部分」）。
  - **本地 CI 镜像（`node scripts/ci-check.mjs --quick`）第一次是红的，抓的是我**：
    `组件中立性（跨 feature 耦合棘轮）` 报 1 条基线外新违规 ——
    新测试文件 `RecommendationPanel.poolBatchLedger.test.tsx` 为把面板摆成回放态而
    `import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore"`
    （owner=timeAnchor ≠ stock-analysis）。处置走门自己写的第一选项「当场修（注入）」而不是登记：
    把「那一格说什么」从 JSX 内联条件提成导出的纯函数 `poolBatchLineFor(state, isReplay)`，
    测试直接打这个判据 ⇒ 渲染层用例改用 store 的**默认态**（`mode:"live"`），
    本文件不再 import 任何别的 feature store；顺带把负控做得更准 ——
    四种缺席、契约漂移、回放抑制现在打在**同一个守卫**上。
    - 基线与白名单**一条都没动**（既有 212 条基线里本就有 22 条是测试文件的跨域 import，
      那是存量待修，不是可以再新增的名额）。
    - 变异自证：删掉 `if (isReplay) { return null; }` 这一行 ⇒ **只有 G1 红**（1 failed / 9 passed），
      复原后 10/10；面板族 5 份文件复跑 **54/54**。
  - 本地 CI 镜像复跑：`node scripts/ci-check.mjs --quick` **EXIT=0**，
    67 步全 ✓、0 ✗（耗时 147.5s；含 dprint / oxlint / tsc / 全部 workspace 的 cargo fmt /
    组件中立性棘轮与其余静态门各带自检）。
  - **W4 验收没有全部闭合的两条（登记，不静默）**：
    ① 「同一批候选的行业集中度可读」——**不做成前端现算**。配额判据的计数域是
    `style × period × 组内`（`trim_with_industry_quota`），而面板一格看到的是
    **单档跨风格**的切片；把后者算出的「4/10 = 40%」与 30% 上限并排显示，就是拿两个
    不同分母装作可比（本项目反复出过这一类）。正解是一条**按批聚合**的读路径
    （`style × period × industry` 计数 + 同组的挤出数），属 W4-4。
    ② 「超过上限时有显式截断留痕」——落库侧已有（被挤出的行照旧进 `reco_scan_audit`，
      漏检归因读得出「算出但被配额挤出」），但 `scan_audit` 在**前端零消费者**
      （`grep -rn scanAudit src` = 0）⇒ 界面读不到。与 ① 同一个读路径一起补。
  - 另加一条呈现纪律（本批内已实现）：这一格**只在实盘态显示**。它读的是
    `as_of_date IS NULL` 的最近一批，而回放态整屏讲的是某一天那一代的结果，
    不加限定并排就是把今天的池说成那天的池（测试 G 钉住）。
- **价值桶出数据的前置条件**：需在重启后跑一次采集（面板的收盘快照按钮 = `sweep_market_close`，
  或等 16:00 定时）——列在该次采集之后的日期才有 PE/PB。
- **重启后的运行核对清单（W3/W3b，只有用户能跑）**：
  1. 建表对账：`select column_name from information_schema.columns where table_name='market_daily_close'`
     应多出 `pe_ttm`、`pb` 两列。2026-11-01 现网读数是**没有**这两列（声明式对账在启动期跑，
     加列必须重启才生效）。
  2. 采集：荐股面板的「全市场收盘快照」按钮（命令 `sweep_market_close`）或等 16:00 定时。
     跑完后 `select count(*) filter (where pe_ttm is not null) from market_daily_close where trade_date='<当日>'`
     应接近当日行数（腾讯批量对亏损票给 `-` ⇒ 那几只是 NULL，属正常）。
  3. 建桶：日志出现 `[reco] 全市场快照 N 只；分桶 trend=… / reversion=… / capital=… / value=…（估值覆盖 x/y）`，
     四桶都非空。value 若写成「未建(估值覆盖 … 低于门限)」说明第 2 步没跑或该日没采集。
  4. 面板：`degradedReasons` 里 value 那条缺席声明应随第 3 步消失；若仍显示，是覆盖率没达标而不是桶坏了。
  5. 多样性：同一票连续出现在四档 picks 的比例、以及「最近 N 天 picks 的去重只数」应显著上升
     （旧值参考：一个月内 31% 的真实 pick 落在 48 只静态兜底名单上）。
  6. W4-3 的候选池那一格（`reco-pool-batch`）：**不必**等采集就该出现，现网最近一批读数是
     「本批候选池 212 只」；跑过第 2 步的全市场快照并刷新荐股后应跳到数千量级。
     若它显示「未留候选池快照」或「还没有实盘批次」，说明落库侧没写而不是面板坏了 —— 两者是不同的话。
  7. 批次时刻应带日期（`2026-11-01 03:49`），跨天的那一批还要带「N 天前」；
     当天刷新的不应出现「0 天前」。
- **W3 验收里的 `check-reco-pool-coverage.mjs` 不建**（决策，不是遗漏）：四条判据中
  「fallback 条件化」「每桶上限」「NaN/负 PE 的处置」已被 Rust 单测钉住（语义强于 grep 静态门）；
  「全市场快照被消费」这一条的载体是 `prepare_scan → scan_period` 的 `strategy_seeds.get(&style)`，
  纯静态门只能断字符串存在、断不了它真的进了扫描 ⇒ 归**运行核对**（重启后看日志
  `[reco] 全市场快照 N 只；分桶 trend=… / reversion=… / capital=… / value=…` 的四桶计数
  与四档 picks 的代码是否落在桶内），不伪装成「有门管着」。
- **门禁现状**：`check-reco-generation` / `check-reco-loop-status-coverage` /
  `check-reco-horizon-parity` / `check-orphan-modules` 绿；`check-layer-discipline`
  仍红，但唯一违规是并行会话的 `src-tauri/src/commands/master_key.rs`
  （`commands-no-sibling-call` 1 > 基线 0），本轮已把新加的跨命令调用收口到
  `_shared_reco_pool`，不再贡献违规。
- **顺带修好一道崩溃的门**：`check-replay-live-consistency.mjs` 此前每次运行都以
  `division by zero` 崩溃（SQL 里直接除以 `l.px`，而现网回放当日的 804 行实盘 picks 里有
  **8 行 `pick_data.price = 0`**）——崩溃既不是 PASS 也不是「样本不足」，是「门看着在、
  一行没跑」。改成「可比 / 不可比」分列后浮出的真实读数（2026-11-01）：配对 12 票、
  style 一致 75%（2026-10-09 只有 3/6）、入场价一致 94.7%（可比 19 对）、
  **2026-10-09 一条回放 picks 的 price = -187,161,529**（垃圾值，把最大偏差顶到 6.1e6%）。
  ⚠ 该门的 SQL 注释必须写 ASCII：脚本用 execFileSync 把 SQL 当 argv 传，Windows 上中文
  会被本地代码页转成非法 UTF-8 字节（实测 0xc5 0xcc ⇒ PG 报错）。
- **由这条读数冒出的待裁项（未动代码）**：`reco_picks.pick_data.price` 的写侧没有任何有效性
  判据（0 与负数都能落库）。闭环侧已受保护（`reco_loop::strategy_outcome` 对
  `entry_price <= 0` 返回 `None`），暴露的是 R-3b 对照与一切朴素消费者。修法要么在落库口
  加「有限且 >0」判据（牵动 RecoPick 的构造面），要么先只把这批行按代际筛掉 —— 需拍板。
- **W3b 后端读数**：`cargo check -p axagent --lib` **EXIT=0（12m22s）**；
  `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` **0**、
  `-p axagent-dao` **0**、`-p axagent-entities` **0**；
  `cargo test -p axagent-analysis-engine --lib -- recommender::` **180 passed / 0 failed**
  （W3 时 173，本轮 +7）、`-- market_close_store` **9 passed / 0 failed**、
  `cargo test -p axagent-dao --test repo_crud` **6 passed / 0 failed**。
  `check-reco-generation` / `check-reco-loop-status-coverage` / `check-reco-horizon-parity` /
  `check-sql-dialect` / `check-line-endings` 绿。
- **本轮仍红的 5 道门没有一条指向本批文件**：`check-layer-discipline`（并行会话的
  `master_key.rs` 新越界 1 > 基线 0）、`check-rhai-numeric-typing`（`data-verifier.rhai:473`，
  该文件由并行会话改动）、`check-universe-coverage`（C6 数据读数 94.0% < 95%，是采集覆盖
  不是代码）、`check-replay-live-consistency`（上面那批既存数据缺陷）、
  `check-decl-evidence` 与 `check-single-source-facts` 的 rc=3 是**软判据**（`--ci` 按设计容忍），
  引用面在 `market_mainline.rs` / `GraphView.tsx`，非本批。
  （主 crate 全量 clippy 与 workspace 全量测试仍未跑 —— 按 AGENTS.md 口径留到全部阶段收尾
  那一次；`node scripts/ci-check.mjs --quick` 走查在第 4 步（分层护栏）就中断，
  故上面用的是逐门直调的读数。）

## 4b. 架构裁定 R-DEC-1（2026-11-01）：荐股与分析链彻底解耦

**裁定原文（用户）**：「智能选股也要按照四周期来做啊，四周期不能给删了」＋「三处必须完全消除分析链的
影响，荐股必须完全来自市场数据进行，与分析链没有任何关系」。

**口径**：荐股（`recommender/`）的候选来源、留弃与分数只能来自市场数据；任何其他链（含 LLM 分析
链与趋势智选工作流）的结论都不得作为荐股的入池理由、加分项或否决项。**档位轴（四周期）一格不减**，
被撤的是风格轴上的耦合。

**撤掉的四条接线**：

| # | 原接线 | 位置（撤除前） | 撤除后 |
|---|---|---|---|
| ① | 分析逐档结论把 watchlist 占位票升级成真实票（并可按结论隐藏） | `strategies/watchlist.rs` 的 `apply_analysis_tier` / `order_scan_codes` | 删除；该桶一律 `synthetic: true`；扫描队列回归池序（上限 30 保留） |
| ② | mid/long 出票门的「分析回环否决」（30 天内判卖出/减持 ⇒ 拦） | `gates.rs` 的 `analysis_veto_reason`、`reco_loop::load_analysis_veto_var`、`reco_analysis_vetoes` 模板变量 | 删除；P0 财报**标注**保留（取数是 vendor 财务数据，不是另一条链的结论） |
| ②b | 响应里的拦截载体 `gate_blocked` | `types.rs` 的 `GateBlockedPick`、面板 `reco-gate-blocked` 横幅、`gateReasonText`、11 语言 `bannerGateBlocked`/`gateReason.*` | 整条载体删除（无生产者即删，不留恒空横幅） |
| ③ | 趋势智选候选注入荐股池 + 荐股内置 `SerenityStrategy`（风格 `bottleneck`/名目 `serenity`） | `SERENITY_SEED`/`SERENITY_CANDIDATE_CACHE` 四个全局、`SeedOverrides.serenity`、`strategies/serenity.rs`、`reco_serenity_enabled` 开关、面板 `STYLE_KEYS` 的 serenity 桶 | 删除；`Style::Bottleneck` 与矩阵里 `serenity × mid/long` 两格**保留**（那是智选工作流自己的落库词表与闭环样本域，撤了会饿掉 W2b 的权重格） |

**排除项的唯一权威**：`style_matrix::RECOMMENDER_EXCLUDED_STYLE_NAMES = ["serenity"]` +
`recommender_styles_for()`；`strategies_for_period` 与对账单测都读它。新增排除项会被
`recommender_exclusion_list_is_small_and_real` 拦下（名单内容 + 每项必须在矩阵里真实占格）。

**现网证据（撤除的代价是有读数的）**：`reco_picks` 里 watchlist 风格 `synthetic=0` 的行
（10-18 起 4 天：36/16/7/2 共 **61 条**）**全部**带着【分析定档】理由 ⇒ ① 撤除后该桶的
「真实推荐」直接归零，这是真实水平而不是回归。`reco_analysis_tiers` 自 2026-10-23 起物理上
一条都读不到（`as_of_date IS NULL AND kind='live'` 命中 0 行），②③ 的撤除因此不改变今天的出票。

**已知遗留（登记，不算完成）**：
1. ~~门 h 报 `BROKEN h … 扫描面为 0`~~ **已闭环（2026-11-01）**：判据重心从「两份文件互抄」改为
   「工作流链单源 + 荐股侧不得再出现策略文件」——`checkSerenityRiskParity(workflowSrc, riskSrc,
   strategyFileExists)`，第三参为真即红（荐股侧 `strategies/serenity.rs` 复活 ⇒ 违反本裁定），
   注册面只以 `SERENITY_WORKFLOW` 存在为前提。自检四条：负控 got=20 / 1 / 19，正控 got=0；
   判定面名目改为「h 趋势智选风控单源（荐股侧不再复制一份）」，扫描面 23，OK。
2. `serenity_base_position` / `serenity_target_pe` 的**面板控件与种子声明已撤**
   （`check-variable-consumer-registry` 因此回到 ✅），但存量库里那两行变量还在
   （声明式对账 `additive_only` ⇒ 删声明不等于删行）。
   - 现网定位（2026-11-01 实读 `workflow_templates`）：残留在 **`id='stock-analysis'` v194 的
     `variables` JSON** 里（`has_base=t` / `has_pe=t`）；四个 `stock-horizon-*` 与
     `serenity-screening` 都已不含；同一查询里 `agent_temperature` 在各行均为 `f`
     ⇒ 证明 v189 的 `drop_variables` 机制确实能把这个键从存量删掉。
   - **本批不 bump `TEMPLATE_VERSION`**（决策，含代价）：升版会重播整张图（v141 那轮
     「首播种炸图」的教训），而这两个键现在的状态是**面板不渲染、代码零读取**，
     只剩库里两行 ⇒ 不为 DB 洁癖付重播的代价。
   - **但不是「以后自然好」**：`merge_variable_values` 无条件保留 DB 旧值 ⇒ 不加删除门，
     这两键会在每一次升版时从存量被带回。正解已备好，挂在**下一次因任何理由升 v** 的同批：
     ① `const SERENITY_VARS_RETIRED_VERSION: i32 = <新版号>` + `assert!(TEMPLATE_VERSION >= …)` 防倒挂；
     ② `drop_variables(&variables_val, &["serenity_base_position", "serenity_target_pe"])`
        接在该水位线门内（照 `SAMPLING_VARS_RETIRED_VERSION` 的写法，常量写在函数体内，
        不并入模块级 `_MIGRATION_VERSION` 那组 —— 下方有按行号写的注释引用）；
     ③ 退役测试照 `sampling_template_variables_are_retired_across_all_three_layers` 三层各一条
        （种子侧定义 / 纯函数逐键删 / 播种路径真调用）。
3. `list_reco_history` 与推荐历史面板仍会读到历史上 `style='serenity'` 的旧行 —— 那是智选链
   自己写的，语义正确，不清理。

### 4b-1 撤耦合批的验证读数（2026-11-01）

- **引擎定点**：`cd axagent-quant && cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings`
  **EXIT=0**（3m22s，含等构建锁）；`cargo test -p axagent-analysis-engine --lib -- recommender::`
  **177 passed / 0 failed**（752 filtered out）。
- **格式**：`cargo fmt --all --check` 全仓唯一 diff 在 `recommender/mod.rs:1898`（一处 assert 折行），
  `cargo fmt -p axagent-analysis-engine` 收敛后 0。
- **门 h（本裁定自己的判据）**：自检 4 条按预期 —— 负控 got=20 / 1 / 19，正控 got=0；
  判定面名目「h 趋势智选风控单源（荐股侧不再复制一份）」扫描面 23 ⇒ OK；整门 a–q **17 条全 OK**。
- **相邻门各 EXIT=0**：`check-reco-generation`（代际一致）、`check-layer-discipline`（ratchet 通过）、
  `check-ontology-consistency`、`check-domain-single-source`、`check-i18n-key-parity`（16,289 键 × 11 语言）、
  `check-errorcode-alignment`（450 码对齐）、`audit-inject-coverage --strict`（注入覆盖差集空）。
- **退役名目**：`serenity_base_position` / `serenity_target_pe`（含两个 camelCase 形态）全仓 grep
  **0 命中** ⇒ 撤的是整条线，不是只撤面板控件。
- **前端**：`npm run typecheck` 0、`npx dprint check` 0、`oxlint` 0 warnings（1,423 files）。
- **前端测试**：首轮全量 2,908 passed / **3 failed**（`DemandDiscoveryPage` ×2、`McpServerSettings` ×1），
  复跑 2,907 passed / **4 failed**（集合变成 `DemandDiscoveryPage` ×3、`kgFusedScale` ×1）⇒ 两次失败集
  不同；单跑前两份 **12/12**、单跑 `kgFusedScale` **22/22** ⇒ 判为并发负载下的既有 flaky 类，
  不是本批回归（本批未触碰这三个文件，也不与它们的消费面重叠）。
- **主 crate**：`cd src-tauri && cargo check -p axagent --lib` **EXIT=0**，但报 4 条 warning，
  全部落在 `src-tauri/src/commands/stock_workflow/serenity.rs:1633/1635` —— 是撤 ③ 时留下的两个死局部
  （`detail_cache`、`serenity_seed`）。CI 口径是 `-D warnings` ⇒ 这四条会让 CI 红，已当场删除，
  终局读数见 §4b 的「主 crate 终局读数」（EXIT=0、0 warning，且日志确认该单元真被重编）。


## 4c. W5 逐档前向窗口读数（2026-11-01 第二轮，同表同法，但要求四个前向标签都存在）

上一条表只要求前向 5 日标签；这一条把样本收严到「2/5/28/90 四个标签齐」（`f90` 未满的日期
因此被剔除 ⇒ H2 只剩 13 个交易日）。日均值 IC：

| 因子 | H1 f2 | H1 f5 | H1 f28 | H1 f90 | H2 f2 | H2 f5 | H2 f28 | H2 f90 |
|---|---|---|---|---|---|---|---|---|
| 相对强度（20 日 − 行业） | −0.0092 | −0.0046 | −0.0023 | **−0.0241** | +0.0014 | −0.0010 | −0.0164 | **−0.1161** |
| 20 日动量 | −0.0078 | −0.0011 | +0.0065 | **−0.0369** | +0.0260 | +0.0279 | −0.0566 | **−0.1854** |
| 5 日短线 | −0.0051 | −0.0038 | +0.0103 | **−0.0201** | −0.0153 | −0.0082 | −0.1049 | **−0.1000** |

（H1 95 天 / 417,136 股票日；H2 13 天 / 56,827 股票日 ⇒ **H2 那四列只能算方向**。）

三条判读，以及一条方法论上的自我警告：

1. **90 日窗口上三个因子两半期都为负**（−0.02 ～ −0.19）⇒ 「动量/相对强度外推到长线」
   在 2026 年这段样本上是**反向**的。这从数据侧支持了矩阵里那条早就存在的裁定
   —— `("trend","long", Some("trend_momentum_not_a_long_thesis"))`，也说明 `value`/`serenity`
   才该是长线候选的来源。
2. **2/5/28 日窗口上没有任何一条因子两半期同号**：|IC| 全部 <0.03 且符号随窗口/样本漂移
   ⇒ 「样本外 IC>0 才启用」这道门今天没有一条腿过得去 ⇒ 新腿一律只做标注，不做硬门。
3. **成交额/资金类腿仍在数据前置之后**（amount 自 2026-10-23 起才铺满全市场），
   本轮根本没进这张表 —— 不是「测出来是 0」。
4. ⚠ **自我警告（同一份数据、两种样本筛法给出不同读数）**：20 日动量对**前向 5 日**的 IC，
   在「只要求 f5 存在」的样本里是 −0.0861（98 天），在「四个标签齐」的样本里是 +0.0279（13 天）
   ⇒ 这些数的**绝对值不可当效应量引用**，能引用的只有「同号性」与「量级都远小于可用门槛」。
   下一轮如果要把某条腿做成开关，必须先固定样本筛法（哪一天起算、哪个标签必须满），
   否则同一份库会给出相反结论 —— 这是本项目反复出现的「度量口径本身造差」那一类。

## 4d. 闭环侧的对照读数（与 §4c 的横截面 IC **不是同一计数域**，不得并排比大小）

`strategy_weight_history` 按 `(strategy_id, period)` 汇总（`trigger='reco-loop'`，2026-11-01 实读）：

| 格 | 变更次数 | 历史最大样本 n | 该格见过的最高胜率 |
|---|---|---|---|
| capital long | 16 | 68 | 0.722 |
| capital mid | 20 | 43 | 0.786 |
| capital short | 17 | 40 | 0.595 |
| capital ultra_short | 16 | 30 | 0.375 |
| reversion mid | 16 | 49 | 0.632 |
| reversion short | 11 | 53 | 0.952 |
| trend mid | 13 | 35 | 0.378 |
| trend short | 14 | 24 | 0.778 |
| trend ultra_short | 12 | 23 | 0.273 |
| value long | 12 | 29 | 0.419 |
| value mid | 12 | 17 | 0.556 |
| value short | 11 | 29 | 0.839 |
| value ultra_short | 4 | 30 | 0.469 |
| watchlist 四档 | 8–12 | **0** | 0.000（= 无样本，不是「0% 胜率」） |

三条判读：

1. **每格样本 17–68 条** ⇒ 那些 0.27 / 0.95 的差值在这个 n 下只是方向，不能当效应量用
   （与本项目「n<30 只报方向」的既有纪律同源）；`worst_wr` 每格都是 0.000 也是小样本的必然。
2. **上表 17 条格与矩阵声明的 17 个荐股出票格逐一对应**（trend 3 + value 4 + capital 4 +
   reversion 2 + watchlist 4），0 缺 0 多。我第一版把「`reversion long` / `reversion ultra_short` /
   `trend long` 没有历史行」读成可疑的判定缺口 —— **读错了**：这三格在
   `style_matrix::MATRIX` 里就是 `Some(理由码)` 的**按设计不做**（
   `oversold_rebound_not_applied_to_long_horizon` / `oversold_rebound_needs_at_least_mid_horizon` /
   `trend_momentum_not_a_long_thesis`），没有权重史是预期结果而不是漏格。
   真正该盯的是反向缺口：「矩阵说出票、闭环却无行」—— 现网 0 条。
3. **`watchlist` 四档 `sample_size=0`** 是撤耦合 ① 之后的预期形态：该桶的真实票来源
   （分析定档升级）已删，全桶 `synthetic=true` ⇒ 按构造被闭环排除在分母外。

分工：闭环量的是「**这批票**在该档持有期内的命中」，§W5 量的是「因子与个股**前向收益的
截面秩相关**」。前者样本受限于实际出票，后者一格就有几十万观测 —— 两侧同向才叫证据，
只有一侧时另一侧的说法要收窄。

## 5. 风险与回退

- **共享工作树**：当前工作树有约 1,955 项未提交的 crate 重组；本计划只改目标文件、不提交、
  不碰 workspace 结构。若与并行会话重叠，按「只改不提交」处理。
- **W1 语义变更**：`confidence` 从「乘权重后的分」改为「原始证据分」。下游 IC/命中率按新值重算，
  历史行仍是旧口径 ⇒ 读取侧必须按 `pick_data.evidenceScore` 存在与否分代，不得混算。
- **W2 权重变严**：趋势智选进入闭环后可能被硬停。必须先让面板显示原因，再上线；
  回放不消费 live 权重，避免前视。
- **W3 取数量**：全市场预筛只读 DB 快照，不增加网络请求；详细策略只对分桶后的
  50–150 只取 K 线，目标是不高于现网 2 倍。
- **回退**：每阶段独立开关（模板变量 `reco_*`）或代码路径可回退；W1/W2 不依赖新表，
  回退成本最低。
