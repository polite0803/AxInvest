# PLAN-capital-long-evidence — capital×long 证据腿换血（真正实现）

> 状态：active ｜ 日期：2026-10-18 ｜ 对应版本：v2.9.9
> 已落地的前置：`style_matrix::MISFIT_DECLARATIONS` 对 capital×long 做了错配声明
> （`capital_evidence_is_daily_snapshot_declared_misfit`）——本计划是声明的兑现路径。

## 一、背景与裁定链

1. **002696 案例**：四连板题材股以 trend-long 身份进了长线候选，分析端长线档判卖出。复盘发现 trend×long 证据（价格动量）与 90 交易日持有期之间零兑现路径 ⇒ trend 退役长线档（`trend_momentum_not_a_long_thesis`，`RECO_ALGORITHM_VERSION` v2）。
2. **用户质疑 capital×long**：「为什么要退役而不是真正实现？」——成立。资金面**存在**真长线证据形态（筹码集中 / 机构建仓），问题在当前证据本体（日级快照）不在概念。
3. **裁定**：不退役。三态处置 = B（MISFIT 声明，已落地）+ C（本计划：扩数据源换证据腿）。

## 二、现状缺陷（为什么必须换血而不是调参）

| 证据源 | 现状（`strategies/capital.rs`） | 缺陷 |
|---|---|---|
| 主力净流入 `main_net_inflow` | **仅当日**（文件头注释自认「v1 资金流向数据仅当日……真正多日窗口需后续扩 AStockClient 接口」）；long 档门槛 ≥100 万，比 mid（≥500 万）还松 | 1~3 天半衰期的信号，与 90 交易日持有期零兑现路径 |
| 北向持仓占比 `nb_ratio` | **已全源失效**（代码注释：冷启动重试实测 ≈8.5s/只） | long 档第二条腿实际是死的 |
| 龙虎榜净买入 | 事件驱动短线资金（002696 实证「游资一日游」） | 短线事件，不是持仓趋势 |
| K 线回退分支 | 量比 + 5 日动量（`synthetic: true`） | 就是趋势跟踪证据换名 |

结论：调参救不了证据本体——这是「证据与持有期脱节」，与 trend×long 同构。

## 三、目标证据形态（长线资金证据，兑现尺度以月计）

| # | 证据 | 数据语义 | 频次 | 长线论点 | 判据草案 |
|---|---|---|---|---|---|
| E1 | 股东户数 | 期未股东户数序列 | 季/半年 | 筹码集中 = 知情资金沉淀 | 最近 ≥2 期连降，或 N 期降幅 ≥ 阈值（`cap_long_holder_drop_min`） |
| E2 | 十大流通股东 / 基金持仓 | 持有人名单与股数变化 | 季 | 机构建仓/增持 | 新进机构家数 ↑ 或核心机构持股比例环比 ↑ |
| E3 | 融资余额 | 余额日序列 | 日（可聚合周/月趋势） | 杠杆资金中期布局 | N 周余额趋势斜率 > 0 且非短期脉冲（区分「趋势」与「事件」） |
| E4 | 大宗交易 | 成交价/量/折溢价率 | 事件聚合 | 折价吸筹 / 溢价接盘 | 近 N 月连续小幅折价（≤ 阈值）且买方营业部集中 |

**共同纪律**：判据阈值全部走模板变量（`cap_long_*`，五点对账口径），不得写死。

## 四、AStockClient 接口草案

> **修正（2026-10-18，P2/P3 实施时二次修正）**：原判断对 **E1/E3/E4 全部不成立**——
> `get_margin_data`（E3 单点）、`get_block_trades`（E4 明细，含 `discount_pct` 折溢价 +
> `truncate_block_trades_by_asof` as-of 截断）、`get_holder_count`（E1 单点，`HolderCount`
> 自带环比 `holder_num_ratio` 与公告日 `notice_date`）**三个接口全链已存在**。
> 三条影子腿的实际工作都是**接入既有 client 方法 + 影子判据**，不是新数据域。
> 真正的缺口只有两处：① E3 的**多日序列**（P1 已补：`get_margin_balance_trend`）；
> ② E1 的**多期序列**（`RPT_F10_EH_HOLDERNUM`，既有实现已声明属下一轮）。
> 教训沉淀：**「接口不存在」的结论必须按方法名逐个 grep 验证**，按领域关键词
> （margin/rzrq）grep 会漏（block/holder 是另一族命名）。

```rust
// E3 已落地（P1）：get_margin_balance_trend(code, days) —— 复用 RPTA_WEB_RZRQ_GGMX
// E4 已接入（P3）：既有 get_block_trades(code) -> Vec<BlockTrade>（RPT_DATA_BLOCKTRADE）
// E1 已接入（P2）：既有 get_holder_count(code) -> Option<HolderCount>（RPT_HOLDERNUMLATEST）
// E1 序列化（待办）：RPT_F10_EH_HOLDERNUM 多期 —— 既有实现已声明属下一轮
// E2 待实现：RPT_F10_EH_FREEHOLDERS（十大流通股东，UPDATE_DATE=披露日可严格截断）
```

- 每个接口自带 vendor 路由 + 健康追踪 + 缓存（与 `get_financials` 同款骨架）。
- **as-of 纪律（本项目踩过的坑，前置约束）**：季频数据（E1/E2）必须按**披露日**截断——
  与 `FinancialReport::effective_disclosure_date` 同款模式。禁止「用今天知道的季报
  回放过去」；回放语义 = 披露日 ≤ as_of_date 才可见。
- vendor 可得性风险：东财反爬（资金流向已实测被拦）、部分接口可能要换源。P1 先做
  可得性 probe，全部不可得 ⇒ 该证据腿不实现（fail-open 声明缺口），不伪造。

## 五、capital 策略长线腿换血设计

`scan_one` 的 `Period::Long` 分支**整体替换**：

- 旧判据（当日净流入 / 北向占比 / 龙虎榜）退役出 long 档；
- 新判据 = E1~E4 证据腿的加权合成（权重进 `horizon_branch_specs`，权威源单一）；
- 「当日资金快照」证据**保留给 ultra_short/short/mid**（那是它的兑现尺度）——换血只动 long 腿，不做全局改动；
- 数据不可得 ⇒ fail-open：缺哪条腿记哪条（`data_gaps`/`reasons` 留痕），全缺 ⇒ 不出票（与出票门同一纪律：宁可放过不可错杀，但缺口必须可归因）。

## 六、分期与验收

> **⚠ v4 语义变更（2026-10-18，用户裁定「同档常数置信度基本上就是造假」后的重构）**：
> `RECO_ALGORITHM_VERSION` 已至 **v4**——荐股主链的伪概率链整体撤除（logit 先验合成 +
> SNR 时间折算 + quality 校准 + 其数据依赖 market_regime/style_baseline/conf_sensitivity）。
> `confidence` 新语义 = **证据强度**（策略条件满足度 × IC 风格权重），个股维度、不再自称
> 「上涨概率」；面板标签已从「置信度」改为「证据强度」（i18n ×11）。
> **本节「个股级校准」= 真概率的唯一正路**：按 (style, period, 证据分桶) 用
> `decision_validations` 真实 T+N 胜率校准——样本足够才输出「校准置信度」，不足显示
> 「样本不足」。`weighted_signal_calibration`（真数据后验）保留为 pub，等证据分落库后
> 以分桶形态恢复。回归测试断言：`recommender` 120 项全绿。
>
> **✅ 数据层已实施（2026-10-18 当日）**：
> ① `RecoPick.evidence_score`（策略原始证据分，风格权重调制前）——随 pick_data JSON
> 落库，**零 DB 迁移**；scan_period 写入，旧行缺字段由校准筛样排除。校准输入必须用
> 权重调制前的值：权重是闭环降权因子，混入会把「风格被降权」误读成「该票证据弱」。
> ② `reco_loop::bucket_calibrated_win_rate`（纯函数 + 单测×3）：按
> (style, period, 证据桶 20 分宽×5) 统计 `decision_validations` 真实 T+N 胜率；
> 桶内样本 < `EVIDENCE_BUCKET_MIN_SAMPLES`(20) ⇒ `None`（显式「样本不足」，不得回落
> 任何合成值）；代际闸门 `min_reco_version` 隔离旧代伪概率常数（60/56/53/51）。
> 口径事实：同格 (style, period) 内风格权重是同一常数，confidence 对桶只是整体平移，
> 桶纯度不受影响，无需反推原值。
> ③ **待接线**：出票/展示侧实时调用校准（样本积累后才有效果）；届时置信度显示
> 「校准置信度（N 样本）」并换代 v5。当前数据积累中——期望 2~4 周后长线档逐步达标。

| 期 | 内容 | 验收门 |
|---|---|---|
| P1 ✅ | **已实施（2026-10-18）**：E3 影子腿——`StockVendor::get_margin_balance_trend`（默认空实现）+ eastmoney 序列覆写（RPTA_WEB_RZRQ_GGMX pageSize=N）+ `AStockClient::get_margin_balance_trend`（live 路由 + 300s 缓存）+ capital×long 影子腿（reasons 追加「融资余额20日趋势 ±x%」，不 gate 不加权，转正在 P4；趋势纯函数 fail-open 单测×3） | reasons 携带新证据 ✓；判据 gate 未变 ⇒ 不换代 ✓ |
| P2 ✅ | **已实施（2026-10-18）**：① E3 回放语义——eastmoney 序列覆写按 `current_as_of` 追加 `(DATE<=截止日)` 范围过滤（判据与 `get_margin_data_with_asof` 同源：必须 `<=` 而非等值，非交易日等值必空）；client 缓存改 **live-only**（cache_key 无代际后缀，回放共用会互踩）。② E1 影子腿——接入**既有** `client.get_holder_count`（单点快照，`HolderCount.holder_num_ratio` 环比现成；as-of 走降级账本 StructuralGap），capital×long 追加「股东户数环比 x%(筹码集中)」（ratio<0 才报）。多期连降 streak 需 RPT_F10_EH_HOLDERNUM 序列——既有实现已声明属下一轮。③ **E2 调研结论**：报表 `RPT_F10_EH_FREEHOLDERS`（十大流通股东，SECURITY_CODE+END_DATE 过滤；字段含 **UPDATE_DATE=披露日** ⇒ 严格不穿越语义有真实字段可用；IS_HOLDORG 区分机构、HOLDER_NEWTYPE 分类、HOLDNUM_CHANGE_NAME 新进/增加/减少）——E2 实现留下一轮（结构复杂值得单独做对：多期快照 + 机构分类判据 + UPDATE_DATE 严格截断） | E3 回放不穿越（vendor 层 DATE<=）✓；E1/E4 影子 reasons ✓；判据 gate 未变 ⇒ 不换代 ✓ |
| P3 ✅ | **已实施（2026-10-18）**：E4 影子腿——接入**既有** `client.get_block_trades`（`Vec<BlockTrade>` 含 `discount_pct` 折溢价与 `truncate_block_trades_by_asof` as-of 截断，回放语义免费；routing.block_trades 独立桶现成）+ capital×long 折价画像（窗口锚点=序列内最新成交日，不依赖时钟 live/回放自洽；中位数<0 才报「近60日大宗N笔 折价中位x%(买方y家 金额z亿)」；`discount_pct` 缺失的笔不进中位数；≥3 笔且 ≥3 个折价数据点才产画像，fail-open 单测×3）。买方集中度判据（去重家数阈值）P4 转正时定。⚠ 报表勘误：个股明细是 `RPT_DATA_BLOCKTRADE`（`RPT_BLOCKTRADE` 是活跃股统计） | 画像纯函数单测×3 ✓；判据 gate 未变 ⇒ 不换代 ✓ |
| P4 | 证据腿换血合入（旧判据退役；E1 核披露日字段）+ `RECO_ALGORITHM_VERSION` +1（判据口径改动 ⇒ 换代） | `decision_validations` / reco_loop 闭环：capital_long 样本 ≥ `IC_MIN_SAMPLE` 后 IC ≥ 0；胜率不低于换血前 |

## 七、不做的事

- 不在换血前调 capital×long 的旧阈值——旧证据上调参是给错配镀金。
- 不动 ultra_short/short/mid 的资金判据（那是它们的兑现尺度）。
- 不新建表：证据序列若需落库走 `astock_daily_snapshot` 既有模式（method 标注），不另起炉灶。
