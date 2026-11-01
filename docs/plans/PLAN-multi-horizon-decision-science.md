# PLAN：四周期决策的科学化重做（替换「共用主链 + 只换技术分」）

日期：2026-09-29 · 状态：**已批准（四问全按建议点单）；Phase A 已实施，B–F 待做**· 前置：`PLAN-horizon-four-cycle-closure.md` §7.3/§7.4 的自查结论

## 〇、为什么现有算法是错的（不是「不够好」，是构造性错误）

| # | 缺陷 | 证据 | 后果 |
|---|---|---|---|
| E1 | 四档**共用同一个后验体系**：`prior` / `evidence_scale` / `risk_bias` 与非技术因子 f2~f13 全部继承主链，档间只替换技术分 `f1s` | `portfolio-mgr.rhai:2687-2688`（逐档复用同一 prior/avg 基座）、`:2668-2671`（注释自述非技术因子共用） | 四档不是四个预测，而是**一个预测加三个阈值标签**；档间差异被压缩到 f1 一项，方向几乎必然同向 |
| E2 | 长线**复用月线评分**（与中线同一个数） | `seed_stock_analysis.rs:4202` 映射同源、`:3581` 注释自述「不新增季度节点」 | mid 与 long 的 action/置信/仓位**恒等**，四档实际只有 3 个输入、2 个增量 |
| E3（**2026-09-29 实施时更正**） | 真缺陷不是「回看窗口写死」—— 指标窗口以 **bar** 为单位（`get_klines(code, period, 120)`），周线 MA20 = 20 周 ≈ 100 交易日，回看本身已随尺度变。真缺陷是三条：① `mcp_tools.rs:1237` 未知 period **静默归 daily**（调用方以为拿到月线，实得日线）；② 评分分段阈值 `ScoreBands`（偏离度 1%/2%/3%/5%/8%、支撑容差 3%）是**按日线波动幅度标定的固定百分比**，套到周/月线上会让分数长期落在同一档 ⇒ 尺度间不可比；③ 缺 quarterly 与 sub-daily 两个尺度（长线复用月线、超短复用日线） | 见 §七-B 的实施记录；本行原文（「只是把日线指标换根 K 线重算」）是**错的**，保留以自证 |
| E4 | 分析师证据的**周期权重表已存在但只按主档调一次** | `evidence_weight.rs:249`（四档表齐全）→ 唯一调用链 `stock_analysis.rs:5928` ← `stockAnalysisStore.ts:2742`，`time_horizon` 取 `get().decision?.timeHorizon` | 科学上最站得住、成本最低的那一层（同一批证据、不同尺度加权）**根本没跑**；且前端另有一份已漂移的表（`stock-analysis-utils.ts:1118`，缺 `a-technical`） |
| E5 | 风控档位是**拍脑袋百分比**，与标的/周期波动率无关 | `portfolio-mgr.rhai:2613-2637` + `seed_variables.rs:2696/2745`（3/5/8/12、5/10/18/30） | 同一组数字对低波蓝筹和高波题材股、对 2 天和 90 天同样适用 ⇒ 止损既不是波动率刻度也不是持有期函数 |
| E6 | 仓位乘数 0.6/0.8/1.0/1.2 是**经验数**，且只作用于主档 | `holding_period.rs:96`（只注入 days+mult）、`recommender/scoring.rs:147`（factor 只乘仓位） | 四档的仓位不含各自的风险预算；「周期越长仓位越大」没有推导 |
| E7 | 没有逐档的**预测力度量**（IC / 半衰期），也就无法标定档间权重 | 全仓 grep 无 IC；`reflection_stats.rs:206/298` 只有方向命中率/平均净收益/alpha，且消费面仅展示（`ReflectionPanel.tsx:675`） | 「哪个因子在哪档有信息」只能靠人写死，无法被数据纠正 |

一句话：**现在是「一套算法 + 四个阈值 + 一组固定百分比」，不是「四套按尺度成立的算法」。**

## 一、方法学依据（每条对应要替换的缺陷）

1. **把每个持有期当作独立预测任务，用分层收缩估计其先验**（empirical Bayes / James–Stein partial pooling）：
   `prior_h = (n_h·p_h + κ·p_pool) / (n_h + κ)`，`p_h` 取该档历史方向命中率（已有：`reflection_stats` 的 `by_horizon`），
   `p_pool` 取全档合并值，`κ` 是收缩强度。⇒ 既不共用一个先验（E1），也不让短线小样本把自己饿死。
   参考：[The James-Stein Estimator](https://stat210a.berkeley.edu/fall-2024/reader/jamesstein.html)、
   [Hierarchical Models / partial pooling](https://www.y1zhou.com/series/bayesian-stat/bayesian-stat-hierarchical-models/)、
   [Empirical Bayes shrinkage](https://metricgate.com/docs/empirical-bayes-shrinkage/)
2. **因子暴露必须按时间尺度分别定义**（收益的可预测结构随尺度换向）：超短以流动性/反转与资金流为主，
   短—中期是动量与趋势，长期以估值/质量为主并计入长期反转 —— 即「短期反转 / 中期动量 / 长期反转」的期限结构一脉。
   参考：[Short-term reversals, short-term momentum, and news](https://www.sciencedirect.com/science/article/abs/pii/S0378426621000261)、
   [Is there momentum or reversal in weekly returns](https://xueshu.baidu.com/usercenter/paper/show?paperid=e8ebb872837212c509e44c629402cf4b)、
   [A trend factor: information horizon](https://www.sciencedirect.com/science/article/abs/pii/S0304405X16301271)
3. **同一批证据、不同尺度重新加权**（不是重跑 LLM）：`get_horizon_base_weights` 那张表就是这个思路，
   缺的只是「按四档各算一次」。这是**成本不变、收益最大**的一刀。
4. **风控与仓位由波动率导出，不用固定百分比**：ATR / 已实现波动率缩放止损止盈（三阶障碍法的上下轨就是按波动率定的），
   仓位按波动率目标归一。
   参考：[Volatility-Managed Portfolios（Moreira & Muir）](https://economy.alljournals.cn/view_abstract.aspx?aid=C0AA8CB0927F7510A298847FF5ED4F80)、
   [波动率目标与趋势](https://view.inews.qq.com/a/20250919A04Z9100)、
   [Triple-Barrier 方法及其扩展](https://github.com/nkonts/barrier-method)、
   [AFML 第 3 章标签（含 vol-scaled barriers）](https://zhuanlan.zhihu.com/p/634366718)
5. **因子预测力随期衰减，要用 IC 衰减曲线定「该档用哪些因子、权重多少」**，而不是人写死：
   参考：[因子 IC 衰减曲线](https://blog.csdn.net/gitblog_00055/article/details/151774547)、
   [Information coefficient](https://grokipedia.com/page/information_coefficient)、
   [ML enhanced multi-factor modeling](https://arxiv.org/html/2507.07107)
6. **成本随持有期摊薄**：换手成本 ∝ 1/holding_days，超短档若不做这层修正，净期望必然被高估。

## 二、本仓的真实能力边界（决定哪些能立刻做、哪些必须先补数据）

| 需要的东西 | 现状 | 结论 |
|---|---|---|
| 四档分析师权重表 | 后端已有且逐档显式（`evidence_weight.rs:249`） | **可直接接线**（E4） |
| 逐档历史胜率/收益样本 | 已有（`reflection_stats.rs:206/298`、`period_breakdown` `backtest.rs:286-318`） | 可直接喂分层收缩（E1） |
| 逐档回测轨迹 | 支持按记录 `expected_holding_days` 分别回测（`backtest.rs:219`） | 可扩成「一次分析四条退出轨迹」（E1 的样本来源） |
| ATR | 已实现但**没接线**（`tools/finance.rs:551`，trader 节点 tools 已清空 `seed_stock_analysis.rs:4093`） | 需接线，非新建（E5） |
| 已实现波动率 | 只有 regime 内部私有 20 日年化（`astock-data/src/regime.rs:142`），无对外逐档 API | 需提为逐档可调用 |
| 按尺度的因子参数组 | **不存在**（回看窗口写死 `scoring.rs:121`） | 必须新建（E3），是本次最大的一块真实工作量 |
| 季度评分（长线独立输入） | **不存在**（period 白名单只有日/周/月，`mcp_tools.rs:1237`） | 要么补，要么显式声明「长线复用月线」为已知降级（E2） |
| 分钟级数据（超短独立输入） | vendor 有 5/15/30/60 分钟 klt（`vendors/eastmoney.rs:1470`），但工具 schema 只暴露三档（`mcp_tools.rs:116`）、无分钟评分 | 要么打通，要么显式声明超短复用日线（E2 同类） |
| IC / 半衰期统计 | **不存在** | 需新建（E7），是「让权重可被数据纠正」的前提 |

## 三、目标架构（四档各自成立的一条链）

```
同一批分析师报告（LLM 文本证据，档无关）
        │
        ├─ 每档：证据权重表 get_horizon_base_weights(h) → 该档加权共识 + 该档置信   ← 成本不变，替换 E4
        ├─ 每档：尺度因子组 factors(h)（超短:资金/流动性/1-3日反转；短:5-20日动量+技术；
        │        中:20-120日趋势+基本面兑现；长:估值/质量+长期反转）              ← 替换 E3
        ├─ 每档：先验 prior_h = 分层收缩(该档历史命中率, 全档合并, n_h, κ)          ← 替换 E1
        ├─ 每档：后验 posterior_h = 贝叶斯融合(prior_h, 该档证据, 该档成本)
        └─ 每档：风控 σ_h = ATR/已实现波动(该档尺度) → 止损=k1·σ_h、止盈=k2·σ_h、
                 仓位 = min(风险预算/σ_h·√(holding_h/252), 凯利上限, 成本修正)      ← 替换 E5/E6
输出：四档各自的 action/置信/仓位/止损止盈（价位）+ 每档「本档证据来源与降级声明」
标定：k1/k2/κ 与因子权重由逐档 IC / 命中率经反思回写（替换人写死的 3/5/8/12）
```

**关键取舍（必须你拍板的一项）**：分析师 LLM 报告**不重跑四遍**。学术上站得住 —— 证据相同、时间尺度不同 ⇒ 变的是权重与因子，不是证据本身。
重跑四遍成本 ×4 且引入措辞噪声；只重算权重层成本不变。若你要「连证据也逐档独立」，那是另一量级的工程，需要单列。

## 四、分阶段实施（每阶段独立可验，不跨阶段混提交）

- **Phase A 接线已有能力（低风险，先做）**：`compute_evidence_weights` 按四档各调一次（后端一次返回四套，避免四次 IPC），
  结果进 `decisionsByHorizon`；删前端重复的 `ANALYST_TIME_HORIZON_WEIGHT`（`stock-analysis-utils.ts:1118`）改为读后端；
  ATR 接入 trader/决策链（`tools/finance.rs:551` 已在，补节点 tools 与注入）。
  验收：四档的 `confidence` 不再恒等；权重表只有一份权威源（门：`check-single-source-facts` + 新增逐档断言）。
- **Phase B 尺度因子组**：`ScoringEngine::score` 增 `period` 参数与逐档参数组（回看窗口/指标集合），
  `compute_scoring` 白名单扩到真实支持的取值，**未知值不得静默归 daily**（现在是 `mcp_tools.rs:1237` 静默降级，属伪装成正常结果）。
  验收：同一标的的 daily/weekly/monthly 评分在构造上不同源（新增行为测试断言窗口不同）。
- **Phase C 逐档回测与先验收缩**：一次分析产四条退出轨迹；`reflection_stats` 增 per-horizon 方向命中率→先验映射 +
  James–Stein 收缩（κ 可调、进设置面板）；样本不足时**显式回退全档合并并标注来源**。
  验收：短线档的先验随该档样本量变化，长线档不受短线样本污染（真库行为测试）。
- **Phase D 波动率风控**：止损/止盈由 `k·σ_h` 导出，`k1/k2` 进可调参数并被反思建议覆盖；
  固定百分比档降级为「σ 不可得时的兜底」，并在输出里带 `stopSource: "vol" | "fallback_pct"`。
  验收：同一组 k 下，高波标的止损% > 低波标的止损%（这是现在完全不具备的性质）。
- **Phase E 预测力度量**：新增 per-horizon rank IC / 半衰期统计（面板数据已有），作为因子权重的标定输入与
  反思优化目标；先把 IC 算出来并展示，**不立刻接回权重**（避免观测面未稳就自调）。
- **Phase F 结构性缺席的显式声明**：长线（无季度分）、超短（无分钟通路）在产出与 UI 上各自成句声明，
  并给出门禁：`decisionsByHorizon` 中若两档的 `posterior` 恒等，必须同时携带同源标注，否则测试红。

## 五、风险与不做清单

- 不做：不重跑四遍 LLM 分析师（除非你另批）；不在 Phase E 之前让 IC 自动改权重；不把「四档同向」当 bug 修 —— 收敛可以是真的，**不可接受的是无法分辨收敛与复制**。
- 风险 1：Phase B 改评分内核会牵动回测与荐股的历史可比性 ⇒ 必须与旧口径并行产出、对比后再切。
- 风险 2：Phase C 的收缩在小样本上会把所有档拉回同一个先验 ⇒ 要报出 `n_h` 与 κ，让「收缩到几乎没用该档信息」这件事可见。
- 风险 3：Phase D 换成波动率档位后，反思建议的调参面从「8 个百分比」变成「k1/k2 + σ 来源」，设置面板与 i18n 需同步重做。

## 六、待你拍板

1. 是否按 A→F 顺序做，还是先只做 A（成本最低、立刻让四档真的不同）再评估？
2. 三处「补数据」要不要开：季度评分节点（长线独立）、60 分钟评分通路（超短独立）、分钟数据工具 schema 扩档？
3. 分析师证据是否接受「同一批报告、逐档重加权」这一取舍（§三）？
4. κ（收缩强度）与 k1/k2（波动率倍数）是进设置面板可被反思优化，还是先固定为文献常见值（如 κ=20、k1=1.5、k2=3.0）？

---

## 七、Phase A 落地记录（2026-09-29）

**做了什么**

1. **逐档重加权落到 Rust 权威源 + 桥表**：新增 `evidence_weight::DECISION_LEG_ANALYST`
   （决策腿 f1~f13 → 分析师 id 的唯一桥；f4 风险分类 / f6 数据质量 / f7 trader 观点显式标 `None`
   ⇒ 乘数恒 1.0，不为「看起来有差异」硬塞分析师）与 `horizon_leg_multipliers()`
   （四档 × 十二腿，数值**全部派生**自 `get_horizon_base_weights`，函数本身不新增任何数字）。
2. **注入链**：`stock_workflow/hooks.rs` 恒注入 `horizon_leg_weights_json`；
   seed 的 portfolio-mgr `input_mapping` 加同名映射；`TEMPLATE_VERSION` **98→99**。
3. **脚本侧融合重做**（`portfolio-mgr.rhai`）：删掉 `non_tech_total_weight` /
   `non_tech_weighted_signal` 这两个「非技术腿四档共用」的捷径变量；改为腿表 `decision_legs`
   + 取值器 `leg_mult` + 融合循环 `w = leg.weight * leg_mult.call(h, leg.key)`。
   超短档从「直接取主链 `effective_posterior`」（旧方案B）改为走**同一逐档融合**，
   日线粒度限制仍真实存在 ⇒ 保留 `confLowerBound: 0.35` 并在 verdict 标注。
   乘数表缺失 ⇒ 全腿 1.0 退化 + `data_gaps` 记一条 + 每档输出 `weightsSource: "table" | "fallback_unity"`
   （降级可检，不伪装成逐档加权）。
4. **A2 前端兜底表对齐 + 新门**：`ANALYST_TIME_HORIZON_WEIGHT` 实测比后端少 6 项
   （ultra_short 缺 `a-technical`/`macro`/`a-sector`，short 缺 `macro`/`a-sector`/`research-mgr`），
   而缺键会被 `getAnalystWeight` 的 `default` **静默吸收成 1.0** ⇒ 半张表长期失效且无报警。
   已补齐，并新增门禁 `scripts/check-horizon-weight-parity.mjs`（`npm run check:horizon-parity`）：
   ① 前后端逐字段一致；② 桥表引用的分析师在四档都存在；③ 扫描面非零。
   自检 `--selftest` 5 条全绿，含三个用真实缺陷形态复刻的正控（删键 / 改值 / 桥表写错分析师）。
5. **A3（ATR 接线）挪到 Phase D**：ATR 的唯一消费者就是波动率档位，提前注入会变成悬空映射
   （`audit-inject-coverage` ② 会报）。故 A 只做「已有能力接线」里真正闭环的两件。

**顺带修掉的审计门假阳性（原「待批第 ① 项」）**

`audit-inject-coverage.mjs` 现新增判据 ③：被 `present()` 守卫的**形参**不再被当成
「无注入来源的工作流变量」（`ts` 那条既存红消失），但排除是有条件的 ——
必须证明「该定义至少有一个调用点，且对应位置的实参根标识符是已注入变量或脚本内自给变量」，
否则报 `定义.形参`。⇒ 不是拿假阳性换假阴性。自检 7 条（含 `present` 自身形参不报、
实参是字面量要报、实参是未注入名字要报）。

**过程中自己造成并当场修掉的三处**（都属「新写的门自身要过编译/跑通」这一族）

1. python 用 `lines[j:j]=[...]` 替换单行时写成了**插入**而非覆盖 ⇒ `const pass` 重复声明，
   审计脚本 `SyntaxError` 直接起不来。是 `node --check` + 立刻跑 `--selftest` 抓到的。
2. 同一批编辑里 `
` 被 heredoc 折叠成 `
`（老坑），导致第二个锚点不匹配、
   整批回滚未写盘 —— 断言先行救了一次；随后改用**行手术 + 原文 raw 字符串**。
3. 判据 ③ 首版把 `bar["amount"]` 这类**成员访问实参**判成「无来源」⇒ 新假阳性。
   修法是取实参的根标识符再比对（`rootOf`），而不是放宽到「不检查」。

**A 阶段验收**：`audit-inject-coverage --strict` 首次三段全绿（① 空 / ② 无悬空 / ③ 无悬空形参，
其中 `horizon_decision.ts ← totalScore_short/mid/long` 被显式证明有来源）；
`check:horizon-parity` 绿（56 键 / 12 腿）；`evidence_weight` 14/14（含 4 条新测试）；
`portfolio_mgr_tier_params_rhai` 7/7（新增「乘数真的进融合」的行为测试 + 「捷径变量不得复活」的锁）；
`cargo fmt --all --check` 绿；`typecheck` / `dprint` / `oxlint` 绿。

---

## 七-B、Phase B 落地记录（2026-09-29，进行中）

**先更正我自己写错的前提**：E3 原文「因子回看窗口不随周期变 ⇒ 周线只是日线换根 K 线重算」**是错的**。
`compute_scoring` 走 `get_klines(code, period, 120)`，MA/RSI/MACD 窗口以 **bar** 为单位，
周线 MA20 = 20 周 ≈ 100 交易日，回看本来就随尺度变。已把 E3 改写成三条**真**缺陷（保留原文以自证）。

**已落地（B0 / B2 / B4）**

1. 新模块 `src-tauri/crates/astock-data/src/scale.rs`：`ScaleProfile`（五尺度
   hourly/daily/weekly/monthly/quarterly，各带 vendor period、取数根数、每 bar 覆盖交易日数、
   **出分最少 bar 数**）+ `resolve()` —— **未知尺度一律 Err 并列出允许值**，
   取代旧 `if weekly||monthly else "daily"` 的静默归 daily；
   + `aggregate_monthly_to_quarterly()` —— 季线由月线按自然季度本地聚合
   （开=季首月开、收=季末月收、高低取极值、量额求和、日期取季末；坏日期跳过不猜），
   因为 vendor 没有可靠的季度 klt，**不猜参数**。5 条单测覆盖别名解析、尺度序、聚合口径、跨季断开、坏日期。
2. `compute_scoring` 改吃 `ScaleProfile`；样本不足 ⇒ **拒绝出分**（不拿 3 根 bar 算 MA60 给一个看起来正常的分数）。
3. 长档接季线：新节点 `t-scoring-quarter`（+ 边 month→quarter、quarter→portfolio-mgr），
   `totalScore_long` 映射改指季线 ⇒ 终结「mid/long 共用月线、两档方向永远一样」。
   `FAST_REQUIRED_NODE_IDS` 与 `FAST_BRIEF_INPUTS`（20→21）同步，快速链不缺档。
   `TEMPLATE_VERSION` **99→100**。

**B1 阈值尺度归一（已落地）**

`ScoreBands::scaled_for(&ScaleProfile)`：偏离度五档边界与支撑容差按 **√(每 bar 交易日数)** 缩放
（随机游走的位移标准差 ∝ √时间 ⇒ 周线典型偏离 ≈ 日线 ×√5、月 ×√20、季 ×√60、小时 ×0.5）。
`compute_scoring` 改走 `score_with_bands`；**日线 f=1.0 ⇒ 与历史逐分一致（零回归）**。
刻意不缩放并写明理由的三类：RSI 阈值（有界且按相对涨跌归一，尺度无关）、BOLL 半带宽因子
（用该尺度自身 bar 的标准差，已自适配）、`deviation_score_*`（序数量纲）。
测试 `scoring::scale_band_tests` 4 条：日线不动 / √ 比例与单调序 /
**同一相对偏离跨尺度得同一分**（不缩放时该条必红，故非空转）/ 尺度无关字段未被顺手改动。

**B3 超短接 60 分钟（已落地，实跑待验）**

新节点 `t-scoring-hour`（`compute_scoring(period="hourly")`，vendor klt=60）+ 边 + 映射
`totalScore_ultra_short`；脚本超短档技术腿改指该值（原为日线 `totalScore`）。
取不到小时线 ⇒ 该参数为 unit ⇒ 闭包退回主链日线 f1 并在 verdict 标「该周期评分缺失，按日线退化」
（**不静默**）。`confLowerBound: 0.35` 保留，语义改为「60 分钟是本链可达最细粒度、2 交易日窗口信息量有限」的**声明下限**。
两个工具 schema 同步：`get_stock_kline` 增 `60`；`compute_scoring` 列出五尺度并写明
「未知值直接报错、阈值按 √ 缩放」。快速链 `FAST_REQUIRED_NODE_IDS` / `FAST_BRIEF_INPUTS`(21→22) 同步。
`TEMPLATE_VERSION` **100→101**。

⚠ **B3 的实跑验证仍欠**：本机 push2* 历史端点曾被 RST（见 `PLAN` 与记忆），小时线是否真取得到
要在 `tauri dev` 侧看 verdict 是否出现「按日线退化」标注才能定 —— 取不到就按 Phase F 显式声明，
**不得**让「超短吃小时线」成为一句写在代码里但不成立的自述。

---

## 七-C、Phase C 落地进度（2026-09-29，进行中）

**已落地：收缩先验的纯函数核**（`crates/analysis-engine/src/horizon_prior.rs`，6 条单测全绿、clippy/fmt 绿）

- `shrink_prior(own, n, pooled, κ)`：`prior_h = (n·p_h + κ·p_pool)/(n+κ)`，
  并连同**来源**一起返回（`own` / `shrunk` / `pooled` / `neutral_default`）——
  「这个先验有几分来自本档」必须可审计，不能只给一个数。
- 端点自证：κ=0 ⇒ 完全采信本档；κ→∞ ⇒ 收敛到合并基准（= 旧的共用先验行为，**必须可表达**，
  这样 κ 才是一个能被反思推着走的旋钮，而不是新的写死数）。
- 防饱和：先验夹在 [0.05, 0.95]（取 0/1 会把后验钉死、让证据失效；与拉普拉斯平滑同族）。
- `horizon_prior_map(stats, κ)`：从 `reflection_stats::HitrateStats.by_horizon` 出四档表，
  该档无统计也要出现并标 `pooled`，不得整档消失。

**C1 事实核查结果（推翻我上一轮的怀疑）**：四周期退出轨迹**早已存在** ——
`reflection.rs:2826 build_horizon_results_json` 逐档构造、按**各档自己的期望持有期**判
`mature/immature/unavailable/legacy`，`reflection_stats.rs:159-165` 再把成熟档展开成
「每周期一条样本」。上一轮我只在 `src/commands/stock_workflow` 目录内 grep 就断言「找不到产出点」，
是**检索面不足**导致的错判（该函数在同目录 `reflection.rs:625` 被调用）。⇒ C1 无需新建。

**C2 已落地（`TEMPLATE_VERSION` 101→102）**

1. `reflection_stats` 命令体抽成 `build_hitrate_stats(db)`（命令与 hooks 共用同一取数，
   否则就是违禁区 12 的重复实现）。
2. hooks 注入 `horizon_prior_json`（四档 `{prior, samples, source}`）；κ 从模板变量
   `horizon_prior_kappa` 读，读不到用 `DEFAULT_KAPPA`（同一常量，三处不各写一份）。
   统计取数失败 ⇒ 注入 null ⇒ 脚本退回共用 prior 并标注（不静默）。
3. 脚本 `prior_for(h)` 闭包取代融合里的裸 `prior`：`hpost = clamp(hp["value"] + avg·evidence_scale)`，
   每档输出 `priorSource` / `priorSamples` ⇒ 「这档先验有几分来自本档」在产出里可见。
4. κ 五点对账全走通：`seed_variables` 定义（默认 = `DEFAULT_KAPPA`）→ `PORTFOLIO_MGR_TUNABLE_PARAMS`
   36→37 → 面板 `portfolio_mgr_horizon` 分组 → 派生 `input_mapping` → `effective_params` 输出。
   前端 `StockAnalysisConfigPanel.test.tsx` 13/13 绿（它正是抓「登记了但面板没有入口 /
   生效快照看不到」的那道门）。
5. 新测试：`prior_for_uses_the_tier_estimate_and_labels_the_fallback`（命中档取本档值、
   缺档退回并标 `shared_regime_prior`）+ `per_tier_prior_is_wired_into_the_fusion`
   （逐字锁 `prior_for` + 锁「后验用 hp[value] 而非裸 prior」+ 锁 `priorSource` 透出）。
6. `audit-inject-coverage --strict` 三段全绿（新变量无悬空、无形参悬空）。

**C 过程中被门抓到的两处（都是我的实现错，不是门错）**

- `prior_for` 用了三元 `cond ? a : b` ⇒ 生产引擎报 `Unknown operator: '?'`。
  **纯语法门放过**，是主 crate 的 `portfolio_mgr_rhai_compiles`（按 `RhaiSandboxLimits::PORTFOLIO`
  生产配置编译）报出来的 —— 又一次证明「门的配置面必须与生产一致」这条判据（#697 同族）。已登记为 #701 ⑤。
- `horizon_prior_kappa` 只写进 `effective_params` 输出、**没有 `present()` 守卫** ⇒
  `tunable_params_have_variable_and_rhai_guard` 报「有变量有清单但脚本不读」。
  这条门的语义是对的：旧模板快照没这个变量时，裸读会抛 `Variable not found` 让整条决策失败。
  已改为 `if present(κ) { κ } else { () }`（缺 ⇒ 输出 null，而不是让脚本崩）。

---

## 七-D、Phase D 落地记录（2026-09-29，TEMPLATE_VERSION 102→103）

止损/止盈改为波动率推导：`sl = clamp(k1 · σ_daily · √h, 0.5, 30)`、`tp = clamp(k2 · σ_daily · √h, 1, 100)`。

- σ 口径只有一份：`harness::indicators::realized_vol_pct`（simple return 的**样本**标准差 ×100，
  与 `sharpe_components` 同口径；样本不足 / 含非正价格 / 非有限 ⇒ None，**不当 0 波动**）
  + `vol_move_pct(closes, lookback, h) = σ·√h`。4 条新单测（手算对照、样本不足拒绝、
  样本 vs 总体方差可区分、√4=2 倍 / √9=3 倍 / h=0 拒绝）。
- 为什么在 Rust 算 √：本仓共享 Rhai 引擎未注册 sqrt/Math（脚本内 0 处使用），为开方新增注册面
  不如把这一行放进已带单测的口径函数 ⇒ 只注册一个 `pm_vol_move_pct(bars, lookback, days)`。
- 默认 k1=1.2 / k2=2.0：盈亏比 1:1.67 与旧固定档（5/3）同量级 ⇒ 只把「不看波动的绝对数」
  换成「按波动的相对数」，没有改变风险回报偏好。
- 降级路径：σ 不可得 ⇒ 退回八项可调百分比（`sl_pct_fallback_for` / `tp_pct_fallback_for`），
  data_gaps 记一条 + 每档与主档都输出 `stopSource: vol | fallback_pct`；主档另出 `realizedVolDailyPct`。
  截断值（0.5/30、1/100）在代码里显式声明为语义上限，不是隐藏常数。
- k1/k2 五点对账：seed_variables 定义 → PORTFOLIO_MGR_TUNABLE_PARAMS 37→39 → 面板
  portfolio_mgr_horizon 分组 → 派生 input_mapping → effective_params 输出。
  σ 与 stopSource 刻意**不放进** effective_params —— 那张表由门要求「字段集合 ≡ 可调参数」，
  观测量放主结果，否则门会报「登记为可调但脚本不读」或反向的多余项。

**D 未做的一项（已裁决 → Phase D-2 实施，见 §七-D2）**：仓位侧曾仍用经验乘数 0.6/0.8/1.0/1.2（只作用主档）。
换成波动率目标法会**反转方向**：按 `w ∝ 1/(σ√h)` 归一后 σ 被约掉，乘数变成纯 `√(28/h)`
⇒ 超短 3.74×、长线 0.56×，与现行「周期越长仓位越大」正好相反。这不是实现细节而是策略主张
（现行阶梯主张「时间换空间」，逆波动主张「同风险预算」），需先定方向，且改时必须连带
风险封顶与成本摊薄一起重推。

**顺带发现的口径不一致（已裁决「一起切」，已实施）**：`astock-data/src/regime.rs::volatility` 此前用
**总体方差（÷n）**，而本仓 `sharpe_components` 已修过同类 bug（测试名即为证）。两者差
`√(n/(n−1))`（n=20 ⇒ +2.6%）。用户裁定「一起切」⇒ regime.rs 现委托 `realized_vol_pct`，全仓只剩一份 σ 实现；`regime.rs` 内另加一条**量化影响可复算**的单测（新值 = 旧值 × √(n/(n−1))）。「高波动」阈值是序数口径（比大小），整体平移不改档 ⇒ 分类不漂移。

## 七-D2、Phase D-2 落地记录（2026-09-29，TEMPLATE_VERSION 103→104）

**用户裁定**：「时间换空间 vs 同风险预算」→ **仓位只承载风险预算**，「长线更值得」放到**收益/判定侧**
（SNR ∝ √h），经验乘数 0.6/0.8/1.0/1.2 不再进主口径。理由：仓位这一维只应承载可推导的量
（σ、h、R），把偏好伪装成乘数正是本轮要修的构造性错误。

**做了什么**

- 主决策仓位：`position_pct = clamp(min(凯利%, 100·R/止损%), 0, 95)`，R = 新可调参数
  `risk_budget_pct`（缺省 1.5%）。止损来自 D-1 的波动率口径 ⇒ 风险预算与 σ 天然闭环。
- 降级分支保留旧乘数但**必须自报身份**：`positionSource: "risk_budget" | "fallback_kelly_x_mult"`。
- 判定侧 SNR：`pm_snr_confidence(p, h, anchor)`（锚 = 中线 28 交易日），逐档 `hconf` 驱动
  该档的 action / odds / 凯利 ⇒ 长线边缘被 √(90/28)=1.79 放大、超短被 √(2/28)=0.27 压回 0.5。
  **多空对称**：负边缘同样被时间放大，这是 SNR 的性质不是态度。
- `hpos = min(该档凯利仓位, 该档风险预算)`，逐档出 `positionSource`。
- 五点对账补齐 `risk_budget_pct`（seed_variables → tunables 39→40 → 面板分组 → input_mapping
  → effective_params），TEMPLATE_VERSION 103→104。

**门禁**：`portfolio_mgr_horizon_lock_rhai.rs` 的反向锁③原先锁 `position_pct_raw * horizon_position_mult`
（乘数在主链）⇒ D-2 后**必然**要改锁，改为逐字锁「risk_budget 优先 + 乘数只在降级分支」两体 +
`positionSource` 两值齐备 + `hconf` 那行存在（4/4 绿）。**改锁不是放宽**：删断言会让「乘数偷偷回主链」
重新可用，新锁反而把两种口径的身份标注一起锁死。

**同时收掉的两处文档腐烂**（§七-D 原文就地更正，保留裁定过程）：regime.rs 的 σ 口径已从「待裁决」
变「已切」；仓位口径从「未做的需裁决项」变「已裁定并实施」。

## 七-E、Phase E 落地记录（2026-09-29）

**范围**：只做观测面（逐档 rank IC + 预测半衰期），**不接回任何权重**（§五「不在 Phase E 之前
让 IC 自动改权重」的延伸：IC 面板未稳，先让人看见数）。

**做了什么**

- `harness::indicators` 新增三个基元：`average_ranks`（1-indexed mid-rank，并列判据沿用
  本仓既有的 `|a−b|<1e-9`）、`pearson`（无定义 ⇒ None，不返回 0）、`spearman_rank_ic`。
- **禁区 12 收编**：本仓此前有**三份**同族实现 —— `hit_rate_backtest`（私有 rank_average +
  pearson_correlation，9 因子 IC）、`portfolio_monitor::pearson_correlation`、以及我写 Phase E
  时新起的第四份。现三处全部委托 harness：hit_rate_backtest 删掉两个私有函数
  （`.unwrap_or(0.0)` 保住它原来的兜底语义），portfolio_monitor 只保留自己的 ≥5 点门槛。
  **我新写的那份是错的**，发现后删掉改为 re-export。
- `reflection_stats`：样本新增 `confidence`（IC 的预测侧，只从四周期 JSON 的
  `decision.confidence` 取；legacy 回退路径拿不到 ⇒ None ⇒ 不进 IC 分母，不拿 action 反推冒充）。
  逐档聚合出 `rankIc / icSamples / icStatus / holdingDays`，顶层出 `signalHalfLifeDays / usableIcTiers`。
- IC 门槛 `IC_MIN_SAMPLE = 8` **高于**命中率门槛 5，是刻意的：相关系数是四阶矩量，n=5 的抽样
  噪声就能把 ρ 推到 ±0.8。
- 半衰期 = 对 ln(IC) 关于持有天数做最小二乘 ⇒ `t½ = ln2/λ`，三条拒绝条件（有效档 <3 /
  斜率 ≥0 / R²<0.5）任一命中即 None。反例单测锁住「两点连线」「IC 随持有期上升」「非正 IC」。
- 缺席必须分句：`icStatus ∈ {ok, insufficient_ic_samples, no_confidence, degenerate_variance}`
  ⇒ 前端三种缺席各占一句（补字段 / 攒样本 / 换标的，要做的事完全不同）。

**我在这条上写错过一次论证（原文保留在下面，供审计），实况与更正见 §七-E2。**

<details>
<summary>我当时的错误论证（原文，已作废）</summary>

**跨模板版本混合样本对 IC 无害（已验，别再来「修」一次）**：逐档 `confidence` 的口径在 v104
（Phase D-2）变过 —— 从 `生效后验` 改成 `0.5+(生效后验−0.5)·√(h/28)`。但 IC 是**按档**算秩相关，
同一档的 h 固定 ⇒ 这个换算是该档内 `(p−0.5)` 的**正系数线性**变换 ⇒ 严格单调 ⇒ 秩不变 ⇒ Spearman 不变。
量纲也一样（两版都是 0–100）。所以 v104 前后的样本混在同一个档里不会污染 IC；
跨档比较 IC 才需要谨慎（各档的系数不同），而半衰期拟合比较的正是跨档 IC —— 那里唯一的真实约束是
「拟合只用 v104 之后产出的记录」，本轮不自动筛（面板是观测面），已在 `IC_MIN_SAMPLE` 注释外在此登记。

</details>

**顺带查出的既有 IPC 缺陷（非本轮引入）**：`HitrateStats` / `HitrateGroup` 定义在
`analysis-engine`，而 `scripts/check-serde-annotations.ts` 的 SCAN_DIRS 只扫 `harness/` 与
`commands/` ⇒ 这两个 DTO **从来没有** `#[serde(rename_all = "camelCase")]`，出边界是 snake_case，
而 `src/types/stock-analysis.ts` 与 `ReflectionPanel.tsx` 按 camelCase 读 ⇒ 命中率整张卡
（样本数 / 方向命中率 / 目标价命中率 / 平均收益 / alpha）**恒为 undefined**，面板表现成
「样本数 0 / 暂无数据」。这不是「四档显示」的问题，是**整块观测面从未生效**。
修法：两个 DTO 补注解 + 新增 `hitrate_dto_serializes_camel_case_for_ipc` 逐键锁 camelCase，
并反向锁 snake_case 不得回来。

**仍未收的门禁覆盖面缺口（需裁决，未动）**：`check:serde` 的 SCAN_DIRS 不含 `crates/`（36 个
成员，本仓大部分逻辑在此）。实测把 `analysis-engine/src` 加进去会浮出 **20 个文件 / 61 处**
既有报告，其中 `commands/analyst_feedback.rs` 的 6 处在当前 SCAN_DIRS 内**就已经报红** ⇒
`npm run check:serde` 在本仓是**长期红且不在 CI**（CI 只跑 `npm run contracts -- --only=a,c,d,f,g,h`）。
逐条裁决（补注解 vs 登记例外）是独立一轮，不塞进本轮。

## 七-F、Phase F 落地记录（2026-09-29，TEMPLATE_VERSION 104→105）

**验收判据（PLAN §四 Phase F 原文）**：`decisionsByHorizon` 中若两档 `posterior` 恒等，必须同时携带
同源标注，否则测试红。加上线上原话「**不可接受的是无法分辨收敛与复制**」⇒ 本阶段的产物不是
「让四档更不同」，而是**让「相同」这件事可被机器检出、且在 UI 上单独成句**。

**做了什么**

- 逐档 `scoreSource: tier_native | daily_fallback` —— 后者 = 该档专属粒度评分节点没出数
  （超短没 60 分钟 / 长线没季度），融合里的 f1 腿退回主链日线信号。这是**结构性缺席**，
  与「算出低分」是两件事；此前只有 verdict 末尾一个方括号注，读不出是哪种缺席。
- 凡 `daily_fallback` 的档**各写一条** `data_gaps`（不是合并成一条「部分周期退化」）。
- `posterior` **恒等**的档互填 `sharesPosteriorWith`。比较的是**四舍五入后的输出值**（与面板同口径）
  —— 拿融合前的 `hpost` 比会得到「没同源」而面板显示同分，正是本段要消灭的错位（已在代码注释锁住）。
- 前端：`HorizonDecision` 补 9 个口径字段（`scoreSource`/`sharesPosteriorWith`/`stopSource`/
  `positionSource`/`weightsSource`/`priorSource`/`priorSamples`/`posteriorEffective`/`snrAnchorDays`）；
  `DecisionBanner` 的周期 Tab 详情里**两种缺席各占一句**；`horizonSuffix` 做 camel↔snake 单点互转
  （`decisionsByHorizon` 用 camel 键、i18n 后缀表按 snake 建模，两族键名并存是既成事实）。
  i18n 新增 2 键 × 11 语言（`check-i18n-key-parity` 绿）。
- Vitest 契约面（`stockAnalysisHorizon.test.ts`）：新增 `horizonSuffix` 双形态 + 未知档名归 null；
  新增「`normalizeDecision` 必须把四个口径标注原样透传到展示层」—— 该函数按白名单构造返回体，
  **漏加一个键就等于把「该档按日线退化」在 IPC 之后静默丢弃**，这条测的就是那个白名单。

**门（新文件 `portfolio_mgr_same_source_rhai.rs`，4/4 绿）**

| 测 | 锁什么 |
|---|---|
| `equal_posterior_tiers_mark_each_other` | 两档同分 ⇒ **互填**（单向标注等于没标）；全档 native ⇒ 不得凭空写缺口 |
| `three_way_tie_marks_every_peer` | 三档同分 ⇒ 每档点名其余两档；三条缺口逐条点名档位，未退化档不得进缺口 |
| `pre_fix_shape_loses_the_marks` | **反控**：删掉同源环的同一份数据 ⇒ 标注必为空 ⇒ 检法在修复前真实形态上命中（同 #644） |
| `production_script_contains_the_blocks_verbatim` | 两段副本逐字 + 档位键表 + 「必须比较输出值 posterior」+ `scoreSource` 字段存在 + **顺序断言** |

**顺带修掉的一处口径分裂（由上面的顺序断言锁住）**：`gap_note`（推理文案「缺口(N)」）定义在逐档缺口
push **之前** ⇒ 文案恒少算这几条，而 `data_gaps` 数组自己是全的 —— 两边各自都「没 bug」，
`all_rhai_scripts_compile` 与 `typecheck` 全绿。修法是把定义挪到最后一次写入之后（写入依赖四档结果，
挪不动写入，就只能挪定义）。登记为判据 **#708**。

**Phase A~F 全部完成。** 剩余都在用户侧：重启应用 + `axagent-batch-rerun` 四档实跑核对
（重点两条：60 分钟数据本机是否可达 ⇒ 决定超短档是 `tier_native` 还是恒 `daily_fallback`；
命中率卡在 camelCase 修复后是否第一次真的出数），以及**提交拆分决定**（本轮全部未提交）。

## 七-F2、Phase F 收尾补记（同日，全量门禁前）

四处「实现完了但契约面没收口」的事，都是跑门时才浮出来的：

- **`gap_note` 与逐档 push 的先后**：见上节末，已做成**顺序断言**（比字节偏移），不只测值。
- **Vitest 契约面新增两测**（`stockAnalysisHorizon.test.ts`，62/62 绿）：
  ① `horizonSuffix` 对 camel / snake 两族键都出同一后缀，认不出的归 `null`（不猜档位）；
  ② `normalizeDecision` 必须把 `scoreSource` / `sharesPosteriorWith` / `stopSource` / `positionSource`
  **原样透传** —— 该函数按白名单构造返回体，漏一个键就等于在 IPC 之后静默丢弃这条缺席声明。
  这正是 #705「五点对账」在前端这一侧的形态：**类型写了 ≠ 展示层拿得到**。
- **i18n 从 8 键增到 10 键**（新增 `horizonScoreFallbackHint` / `horizonSharesPosteriorHint`），
  11 语言逐条补齐，`check:i18n:key-parity` 绿（15272→15274 条/文件）。
- **clippy 抓到自己写的 Rust 测试**：`needless_range_loop`（`for i in 0..8 { perm[i] }`）两处 ——
  一处在引擎的半衰期用例，一处在 `build_src` 夹具。改为 `enumerate()` / `zip`，**没有**用
  `#[allow]` 绕过：这是纯机械改写，不涉及浮点语义（区别于 #AGENTS 里 `manual_range_contains`
  那种「建议会改 NaN 语义」的情形，那边才需要逐情形验证）。

- **又抓到一处互斥声明（同族 #709）**：超短档 verdict 尾部**无条件**拼着「[超短取 60 分钟粒度评分，置信下限35%]」，
  而闭包在该档评分缺失时已经往同一串里写了「[该周期评分缺失，按日线退化]」⇒ 60 分钟没取到时，一条结论
  同时声称「吃了 60 分钟」和「退化成日线」。这是**文本语义**缺陷：编译门、类型门、Vitest 全绿。
  修法 = 该句改判 `ultra_short_dec["scoreSource"] == "tier_native"`；门**双向锁**
  （正向锁条件分支存在、反向锁旧的字面量不得回来 —— 只锁正向，有人拆掉条件保留键名时门仍绿）。

**门禁状态（截至本条）**：定点门全绿 —— harness 35 / analysis-engine 520 / rt-workflow 全套（含新增
`portfolio_mgr_same_source_rhai` 4 测）/ 主 crate `seed_consistency_tests` 24 测（含 `portfolio_mgr_rhai_compiles`）
/ Vitest 182 文件 2059 测（单独跑，排除并行 CPU 抢占的 flaky）/ typecheck / oxlint / dprint / eol /
i18n parity / contracts(a,c,d,f,g,h) / horizon-parity / paired / types / valuation-parity / layers / audit-inject --strict。
`check:facts` rc=3 = 软判据独立退出码（14 处「疑似腐烂」全部**不含**本轮新增引用，逐条 grep 为 0）。
**全量三条（CI 口径，已跑完，正文为证）**：

| 门 | 结果 | 正文读数 |
|---|---|---|
| `cargo fmt --all --check` | 0 | 无 diff |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | `error` 行数 0 |
| `__TAURI_WORKSPACE__=true cargo test --workspace` | 0 | **7948 通过 / 0 失败 / 41 忽略**，145 个测试套件全 `ok` |

## 八、并发会话造成的「半提交」——HEAD 自身不自洽（提交拆分前必读）

核对 `git show HEAD` 得出的实证（不是推测）：并发会话在 `TEMPLATE_VERSION=102` 时把我这轮的
**脚本内容**提交了，但**没提交与之配套的参数面与测试副本**：

| 面 | HEAD 实况 | 应为 |
|---|---|---|
| `portfolio-mgr.rhai` | 已含 D/D-2（`pm_vol_move_pct`×7、`risk_budget_position`/`pm_snr_confidence`×5） | —— |
| `seed_variables.rs` | `stop_vol_mult` / `take_profit_vol_mult` / `risk_budget_pct` **0 命中** | 三项必须有 |
| `PORTFOLIO_MGR_TUNABLE_PARAMS` | 同上 **0 命中** ⇒ 不生成映射 ⇒ 脚本只能吃硬编码缺省 | 37→40 |
| 设置面板分组 | **0 命中** ⇒ 用户改不到、反思建议无处落 | 必须列进 `portfolio_mgr_horizon` |
| `TEMPLATE_VERSION` | **102**（脚本已是 v103/104 形态） | ≥104；本轮已到 105 |
| `portfolio_mgr_tier_params_rhai.rs` | 副本仍是旧 `let sl_pct_for = |h| switch h {` | 与脚本逐字同步 |

两条硬后果：

1. **HEAD 上 `tier_params_block_matches_source_verbatim` 必红** —— 该断言就是
   `pm.contains(TIER_PARAMS_SRC.trim())`，用 `git show` 取两份 HEAD 正文做逐字包含检查，结果为
   `False`（副本首行在脚本里存在，差异在其后的止损/止盈闭包体）。即并发会话**没跑这条门就提交了**。
2. **版本门停在 102 ⇒ 已播种的 DB 永不重播**：`existing.version >= TEMPLATE_VERSION ⇒ skip`，
   于是「仓库已修、DB 仍是坏的」（同 AGENTS.md 禁区 10 与判据 #166）。用户重启后看到的仍是旧脚本形态。

⇒ **提交纪律**：本轮余下改动必须**整组同批**提交（脚本 + seed_variables + tunables + 面板 +
effective_params + TEMPLATE_VERSION + 三个锁定测试）。只挑其中一半提交，就是在复制上面这个状态。
本轮仍**未提交**（等用户裁定拆分方式）。

## 七-E2、我上一条论证错在哪，以及为此改了什么（同日，审计驱动）

**错在哪**：我写「同一档的 SNR 换算是正系数线性 ⇒ 严格单调 ⇒ 秩不变 ⇒ IC 与版本无关」。
单调性只在**同一代刻度内部**成立；把 v103（conf = 生效后验）与 v104（conf = 0.5+(生效后验−0.5)·√(h/28)）
两代记录混进同一档做**池化**排名时，跨版本那一对样本的先后会被各自的系数打乱 —— 例：
旧记录 heff=0.60 ⇒ conf 60.0，新记录 heff=0.56 ⇒ conf 60.7，真实强弱是前者高，池化秩却把后者排前。
所以「按秩稳健」救不了「两把尺混量」。**结论反了：必须按版本筛，不是不必筛。**

**为此改了什么（第 4 条）**

1. **口径水印**：`HorizonDecision` 新增 `snr_anchor_days`，反思落库时写进
   `horizon_results_json.decision.snrAnchorDays`（该字段本就在逐档决策输出里，此前被
   反思的白名单构造**丢在 IPC 之后** ⇒ 光有水印字段也没法筛，这一步是前置条件）。
2. **IC 只吃同代样本**：`expand_horizon_samples` 原样读水印（camel/snake 两形都认，**缺就是缺，
   不兜底成 28**）；引擎里 `confidence` 有值但水印缺失 ⇒ 不进 IC 分母，并计入新字段
   `icRegimeExcluded`（换代进度可见，归零即说明该档样本已全部是当前口径）。
3. **第四种缺席单独成句**：`ic_status` 增加 `pre_snr_regime`，与 `no_confidence`
   （根本没预测值）、`insufficient_ic_samples`（当代样本不够）、`degenerate_variance`
   （取值无差异）四句分列；映射收在 `horizonIcAbsenceKey`（可测函数，不放在组件里），
   Vitest 锁「四键互不相同 + 未知状态返回 null 不猜翻译」。i18n 新增 1 键 × 11 语言。
4. **测试**：引擎 27/27（含新增四态混代用例：全旧 ⇒ pre_snr_regime+排除 8 条；混合 ⇒
   只数当代 3 条并报 insufficient；攒够 8 条当代 ⇒ 同一形态出数，证明筛的是水印不是数量）；
   命令层 6/6（水印两形态可读、缺失不伪造）。

**顺带（第 3 条）**：四个档名 i18n 标签此前各自手抄天数（`reflection.horizonUltraShort` 写
「(1-3天)」而权威是 **2 交易日** —— 抄本已腐烂且没有门会响）。修法不是把数字改对，而是
**标签里不再出现数字**：44 处（11 语言 × 4 键）全部剥掉区间，命中率表新增「期望持有」列，
渲染 `byHorizon[].holdingDays`（值来自 `harness::Period::default_holding_days`，唯一权威）。
防腐门：`check:horizon-parity` 新增判据 ④「档名标签出现数字即红」+ 权威表必须可解析出 2/5/28/90，
selftest 加 4 例（含「把 (1-3天) 抄回一个标签必须报 1 处」「11 语言全抄必须报 44 处」）。

## 九、审计修复第一轮（2026-09-30，用户逐条批复「同意」后的第 1–5 条）

用户把我列的 6 条处理顺序整批批准。已完成 1–5，第 6 条留作下一轮（真下一轮工作量）。

| # | 做了什么 | 门 |
|---|---|---|
| 1 | 分析链 A~F 整组提交 `859a1fcf2`（31 文件，零混入并发会话文件） | 提交前 fmt/clippy/test 全量三条正文为证（7948 通过 / 0 失败 / 145 套件） |
| 2 | `save_node_feedback` 三道断点：命令参数名 `req`→`request`（前端一直传 `request`）、6 个 DTO 补 camelCase 注解、前端失败不再只 console.warn（新增 `feedbackUploadFailed` × 11 语言 + 弹窗独立 Tag） | 新增 4 条 serde 往返测试（含「纯 snake 载荷必须 Err」反向锁）；`npm run check:serde` 由长期红转 **0** |
| 3 | 档名标签不再手抄天数（44 处 = 11 语言 × 4 键，全部剥掉区间；`f7` 的裸值 0.10 抽成具名）；命中率表新增「期望持有」列，值取 `byHorizon[].holdingDays`（权威 `default_holding_days`） | `check:horizon-parity` 新增判据 ④「标签含数字即红」+ 权威表可读性护栏；selftest 10/10（含「抄回 (1-3天) 必报 1 处」「11 语言全抄必报 44 处」） |
| 4 | 我上轮「跨版本混池对 IC 无害」的论证是**错的**（已在 §七-E 留原文并作废）：单调性只在同代刻度内成立，跨代混池会改池化秩。落地口径水印 `snrAnchorDays` → `horizon_results_json.decision`，IC 只吃带水印样本，新状态 `pre_snr_regime` + 计数 `icRegimeExcluded`（换代进度可见） | 引擎 27/27（全旧 / 混合 / 攒够三态）；命令层 6/6（水印 camel+snake 都读、缺失不兜底）；缺席四分句映射收进可测函数 `horizonIcAbsenceKey`，Vitest 锁「四键互不相同 + 未知状态不猜翻译」 |
| 5(a) | 逐档 `evidence_scale`：分母由 `leg_bases` 基线表按该档乘数缩放（`evidence_max_for`），不再借用主链；`max_weight` 的 11 项手抄 literal 改为同表派生（累加顺序不变） | `evidence_max_reduces_to_max_weight`：unity 表逐位等于主链、f13 激活 1.59、skew 表四档互不相同 + 反向锁（literal 与 `avg * evidence_scale` 都不得回来）。该门首跑即红，红因两条都在我这边：① 我把注释的四舍五入读数 1.49 当精确值（真实逐项累加 = 1.4900000000000004）；② 我第一版给 long 配的 `f5×1.8 + f3×0.4` 净增减正好抵消 ⇒ 与主链同值（分母是加权和，夹具须让净增减非零） |
| 5(b) | 逐档赔率 = 该档价带之比 `止盈% ÷ 止损%`（= k2/k1；σ 与 √h 同时约掉），替代「置信阶梯 → 保守赔率」的拍表；输出新增 `odds` / `oddsSource` / `evidenceScale` | 锁在 `portfolio_mgr_horizon_lock_rhai.rs`：正向锁 ladder 式、**反向锁旧表 `if hconf >= 0.70 { 2.5 }` 不得回来** |

**口径结论（与直觉相反，值得留档）**：5(b) 批准后我原以为会做出「赔率随 h 变」，推导结果是**赔率与 h 无关**
（TP/SL 同为 `k·σ·√h`，比值退化成 k2/k1）。真正错的是旧表把「置信高」当成「赔率好」——同一个信念既定下注概率
又定回报倍数，是自指。改后盈亏比只有一个入口：`k1`/`k2`（面板可调、可被反思建议覆盖）。

**版本**：TEMPLATE_VERSION 105 → **108**（106/107 被并发会话的荐股链改动占用）。5(a)/5(b) 都改变四档
posterior 与仓位数值 ⇒ 不重播种则存量 DB 仍按旧口径出决策。

**提交状态（诚实标注）**：第 2–5 条改动**已完成但未提交**。原因是文件级真重叠，不是我漏跑门：
`src/i18n/locales/*.json` 与 `seed_stock_analysis.rs` 里现在都含并发会话未提交的荐股链改动
（`reflection/horizonFailed` 一行 × 11、v106/v107 两段）。按用户裁定「等他们先提 locale」，
我这批保持已验证、待落库。另有两处瞬态由并发会话造成（`recommender/ic.rs` 类型错误、
`build.rs` 写 register_commands.rs 撞 Windows 用户映射锁 `Os 1224`），均不属本轮改动。
