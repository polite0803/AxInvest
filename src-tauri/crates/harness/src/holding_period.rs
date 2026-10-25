// SPDX-License-Identifier: AGPL-3.0-only
//! 持有周期（四档）—— 全仓唯一权威定义
//!
//! 为什么在 harness：`Period` 同时被 recommendation 打分（analysis-engine）、决策链路
//! （`commands/`）、反思分档（`stock_workflow`）、cron 与前端契约消费，属跨 crate 共享 DTO。
//!
//! ⚠ 禁止在任何 crate 再定义一份周期枚举或「周期 → 持有天数」表：
//!   本文件的 `default_holding_days` 是全仓唯一天数来源
//!   （`stock_workflow/reflection.rs` 已把这条写成注释声明的禁令）。

use serde::{Deserialize, Serialize};

/// 持有周期（4 种）
///
/// 序列化统一为 snake_case（`ultra_short` / `short` / `mid` / `long`），
/// 前端 `PeriodKey`（`src/types/stock-analysis.ts`）与 cron 侧
/// `RecoCronConfig.periods` 均按此契约消费。
///
/// 注意 `UltraShort` 必须显式 `rename`：`rename_all = "lowercase"` 会把它
/// 序列化成 `ultrashort`（无下划线），与前端 `PeriodKey = "ultra_short"` 不符，
/// 导致 `CompactRecommendation` 等按 `response.period` 分支的组件把超短线
/// 误落到 else 分支显示成"长线"。`alias` 保留以兼容历史存档里的旧写法。
/// 序（`PartialOrd`/`Ord`）= **变体声明序** = 由短到长（ultra_short &lt; short &lt; mid &lt; long）。
/// 作用范围要说清：它固定的是 **Rust 侧 `BTreeMap<Period, _>` 的迭代序**（批量任务、逐档聚合可复现）。
/// 它**固定不了 JSON 键序** —— `serde_json::Value` 的 `Map` 按字符串字典序排（本仓未启用 `preserve_order`），
/// 所以「按短→长展示」是消费方契约（按 `Period::ALL` 排），不是序列化层的属性。
/// （2026-09-29 实测：按档位序断言 JSON 键序的测试报红，据此更正，见
/// `commands/stock_analysis.rs` 的 `reco_batch_by_horizon_json_key_order_is_lexicographic_not_tier_order`。）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// 超短线 1-3 天（T+1 隔夜/事件驱动/情绪博弈）
    #[serde(rename = "ultra_short", alias = "ultrashort")]
    UltraShort,
    /// 短线 1-2 周
    Short,
    /// 中线 3-8 周
    Mid,
    /// 长线 3 个月+
    Long,
}

/// 一档的 VERDICT 契约 —— R-11「从分析师起逐档分叉」的**机器可读唯一源**。
///
/// 为什么放 harness（而不是 analysis-engine / 门脚本 / prompt 文件）：这张表同时被三处消费 ——
/// ① 专家 prompt 里的逐档字段清单（人写，由 `seed_consistency_tests.rs` 的门反向对账本表）；
/// ② Rhai 侧解析与数据质量判定（`commands/data-quality.rhai`）；③ CI 门的判据 c
/// 「VERDICT 字段集 == 该档适用因子集」。放任何一侧都会造出第二份权威（禁区 12）。
///
/// ⚠ **三类「没有」的语义不同，必须分开**（R-11 的结论可证伪性就靠这条）：
/// - [`HorizonVerdictSpec::fields`]：该档**确实算了**这件事 ⇒ 字段必须出现；
/// - [`HorizonVerdictSpec::not_applicable`]：该档按构造不适用 ⇒ prompt 必须写**显式不适用文案**
///   （第二元组是文案键），**不许省略** —— 省略就退回 I 轮那种「只有标签没正文」、无法归因的形态；
/// - [`HorizonVerdictSpec::qualified`]：该档**只作过滤 / 只有部分来源** ⇒ 不得进方向加权，
///   且必须带标记键说明它凭什么打折。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HorizonVerdictSpec {
    /// 该档必填字段（camelCase，与 VERDICT JSON 键一致）
    pub fields: Vec<&'static str>,
    /// 不适用字段 → 显式文案键
    pub not_applicable: Vec<(&'static str, &'static str)>,
    /// 条件字段（只作过滤 / 部分来源）→ 标记键
    pub qualified: Vec<(&'static str, &'static str)>,
    /// **进方向加权**的因子（= 契约里既参与、又不带「只作过滤 / 只作风险提示」标记的那一批）。
    ///
    /// 与 `qualified` 的分工：`fields` 是「模型必须输出什么」，
    /// `participating` 是「决策融合拿它做什么」—— 长档的 `trendStrength` 在 `fields` 里（必须输出），
    /// 但不在本清单里（走 `qualified` 的 `trendFilterOnly` ⇒ 只作入场时机过滤）。
    /// P4′ 的逐档分支表按这一划分定 role。
    pub participating: Vec<&'static str>,
    /// 该档出场规则口径（`exitRule` 的取值域）
    pub exit_rule: &'static str,
}

/// 各档共用的 VERDICT 字段。
///
/// 不含 `horizon` —— 它是**代码盖章**的档位标签，不是模型自报字段（模型自报会把「它以为的档」
/// 当成事实，与 `decision.rs` 的 `extract_horizon_source` 退化点名是同一族问题）。
const VERDICT_COMMON_FIELDS: &[&str] = &[
    "direction",
    "confidence",
    "eventCatalyst",
    "exitRule",
    "stopSource",
    "priorSource",
    "positionSource",
];

/// 逐档因子（`fields` / `not_applicable` / `qualified` 的划分域）。
///
/// 新增因子必须同时登记 [`na_key`] **与** [`VERDICT_FACTOR_OWNERS`]，否则 `verdict_spec`
/// 或分析师子集推导会当场 panic 而不是静默漏判。
const VERDICT_FACTORS: &[&str] = &[
    "momentumSignal",
    "microstructure",
    "breadthState",
    "flowPersistence",
    "trendStrength",
    "valuationBand",
    "earningsQuality",
    "expectationRevision",
    "macroRegime",
    "supplyShock",
    "sectorRotation",
];

/// 因子 → **证据属主分析师**的唯一表（节点 id 与 `seed_stock_analysis.rs` 的 `analysts` 数组一致）。
///
/// 存在理由（2026-10-03 P4′ 第一步）：四档子工作流「挂哪些分析师」原先是 PLAN §10-4 里
/// 手抄的四行清单，而**因子集**已由 [`Period::verdict_spec`] 定档 ⇒ 两份清单必然漂移。
/// 实测漂移后果不是文字问题而是死输入：手抄清单的 中档/长档**都没挂 `value-investor`**，
/// 而两档的必填因子都含 `valuationBand` ⇒ 该腿在子图里永远拿不到产出方。
/// ⇒ 挂载集合改为**由本表推导**（`Period::analyst_subset`），§10-4 那张表降为历史注释。
///
/// 两个刻意不在本表里的分析师（有节点、无因子）：
/// - `a-news`：公告方向通道已由 `a-catalyst` 的 `eventCatalyst` 承载，同域双挂=重复计数；
/// - `research-mgr`：裁决/合成层，不是证据方。
pub const VERDICT_FACTOR_OWNERS: &[(&str, &str)] = &[
    ("momentumSignal", "a-market-analyst"),
    ("trendStrength", "a-market-analyst"),
    ("microstructure", "a-hot-money"),
    ("flowPersistence", "a-hot-money"),
    // 涨停家数 / 触板数 / 封板率 / 炸板率 —— 数据侧由 P9-4 的 `LimitUpBreadth` 供上（可按日回溯）。
    ("breadthState", "a-sentiment"),
    ("valuationBand", "value-investor"),
    ("earningsQuality", "a-fundamentals"),
    ("expectationRevision", "a-research"),
    ("macroRegime", "a-policy"),
    ("supplyShock", "a-lockup"),
    ("sectorRotation", "a-sector"),
];

/// 跨档共用因子（在 [`VERDICT_COMMON_FIELDS`] 里，每档都必填）的属主。
pub const COMMON_FACTOR_OWNERS: &[(&str, &str)] = &[("eventCatalyst", "a-catalyst")];

/// **必须由 LLM 以 JSON 数值输出**的逐档因子 → (下界, 上界, 语义说明)。
///
/// 为什么需要本表（2026-10-14 现网实证）：`verdict_contract_prompt` 原先只给字段名、
/// 不给产出形态，模型把 `flowPersistence` 当自由文本槽（"unknown" / "short_pulse" /
/// 中文句子，抽查 0 条数值），而消费端 `analysis-engine::leg_signal` 严格只收
/// `[-1, 1]` 数值 ⇒ **该腿自建成起从未参与过融合**；短档的联合门
/// `trend_and_flow_same_sign` 因此恒「未判定」，买入/增持被封死
/// （v150+ 现网 25/25 `entryGatePassed=false`）。本表让「契约承诺什么形态」与
/// 「消费端接受什么形态」同源：契约渲染读它，消费侧的行为由 `leg_signal` 的测试
/// 反向对齐（改区间不同批 ⇒ 测试红）。
///
/// 当前只有一条：`flowPersistence` 是逐档 VERDICT 里**唯一**由 LLM 直接给出、
/// 且要进方向加权融合的数值因子。`eventCatalyst` 走受控六级标签词表，不在此表；
/// 其余因子（momentumSignal / trendStrength / …）都来自工具或评分节点的数值。
pub const NUMERIC_VERDICT_FACTORS: &[(&str, f64, f64, &str)] = &[(
    "flowPersistence",
    -1.0,
    1.0,
    "正 = 资金持续净流入的强度，负 = 持续净流出的强度，0 = 无明显持续性",
)];

/// 「按构造不适用」的显式文案键。
///
/// ⚠ PLAN §十二 的「缺席时的显式文案键」一列每行只给了一个键，而**同一因子在不同档的缺席
/// 理由并不相同**（`trendStrength` 超短是「按构造不适用」、长档是「只作入场过滤」；
/// `macroRegime` 短/中是「不适用」、长档是「只有五条序列」）。本函数把「不适用」类的键
/// 按因子固定，「有条件」那类走 [`qualified`] 的标记键 —— 两列从此不再混用。
fn na_key(factor: &'static str) -> &'static str {
    match factor {
        "momentumSignal" => "notApplicableMomentum",
        "microstructure" => "notApplicableMicrostructure",
        "breadthState" => "notApplicableBreadth",
        "flowPersistence" => "notApplicableFlow",
        "trendStrength" => "notApplicableTrend",
        "valuationBand" => "notApplicableValuation",
        "earningsQuality" => "notApplicableQuality",
        "expectationRevision" => "notApplicableExpectation",
        "macroRegime" => "notApplicableMacro",
        "supplyShock" => "notApplicableSupply",
        "sectorRotation" => "notApplicableSector",
        other => panic!("未登记的逐档因子名（新增因子要同时补 na_key）: {other}"),
    }
}

/// 条件字段标记键的**一句**理由说明（注入 prompt 用）。
///
/// 每条都能指到本仓的实测缺口，不是理念措辞：
/// - `microstructurePartialSource`：连板/封单/炸板有真字段（P9-4 同花顺涨停池，可按日回溯），
///   但逐笔/L2 撮合明细零通路（P9-6 未证实）⇒ 只能写「部分来源」。
/// - `breadthGateOnly`：情绪广度是**市场级量**，只作背景过滤 / 入场必要条件
///   （超短的 `breadth_required` 门；短档亦只过滤），不得进方向加权 —— 一旦计权，
///   同一天的全部个股会共享同一个方向信号。
/// - `qualityRealizationBeyondHorizon`：盈利质量的兑现周期跨季，28 日窗口内只能部分反映。
/// - `supplyAsRiskNoteOnly`：解禁/减持在中档已被趋势吸收，降为风险提示、不参与方向。
/// - `trendFilterOnly`：长档技术腿只作入场时机过滤（R-11 明文不得进方向加权）。
/// - `valuationFilterOnly`：90 日档估值腿只作**高估否决**（便宜不是买入理由，严重高估才拦下），
///   不进方向加权（P0-3，`PLAN-stock-decision-reliability.md`：估值在 90 日窗内无方向预测力，
///   实证 10-09 紫金 DCF 放行长档买入后 90 日内跌破决策价）。
/// - `macroPartialSeriesOnly`：宏观只有五条真序列（P9-1），货币/社融类无供应。
/// - `sectorRotationAsOfGap`：行业景气/轮动**live 有真源**（`get_industry_ranking`），
///   但回放不可得 —— BK 板块历史源已普查穷尽（本机对 push2* 族按累积量/速率触发 RST），
///   属结构性缺口 ⇒ 长档可引用它作入场背景，**不得进方向加权**，回放时必须带标记。
fn qualified_reason(key: &str) -> &'static str {
    match key {
        "microstructurePartialSource" => "连板/封单/炸板有真字段，逐笔与 L2 撮合明细零通路",
        "breadthGateOnly" => "情绪广度是市场级量，只作入场必要条件 / 背景过滤，不进方向加权",
        "qualityRealizationBeyondHorizon" => "盈利质量兑现跨季，本窗口只能部分反映",
        "supplyAsRiskNoteOnly" => "供给冲击在窗口内已被趋势吸收，只作风险提示",
        "trendFilterOnly" => "技术腿只作入场时机过滤，不得进方向加权",
        "valuationFilterOnly" => "90 日窗内估值无方向预测力：便宜不是买入理由，只作严重高估否决",
        "macroPartialSeriesOnly" => "宏观仅 CPI/PPI/PMI/非制造业 PMI/GDP 五条真序列",
        "sectorRotationAsOfGap" => "行业轮动 live 有源、回放结构性不可得，只作背景不进方向",
        other => panic!("未登记的条件字段标记键: {other}"),
    }
}

/// 渲染「数值型字段」说明节：只列出本档 `fields` 里、且落在 `table` 登记中的因子。
///
/// `table` 做成参数而不是在函数内硬读 [`NUMERIC_VERDICT_FACTORS`]，是为了让负控能走
/// **同一个谓词**（传空表 ⇒ 该节必须消失；传真表 ⇒ 必须出现），而不必去魔改常量。
/// 两个守卫（表读取、字段筛选）各配一条负控，见
/// `numeric_section_has_two_guards_each_negatively_controlled`。
fn numeric_fields_section(fields: &[&str], table: &[(&str, f64, f64, &str)]) -> String {
    let rows: Vec<String> = table
        .iter()
        .filter(|(f, _, _, _)| fields.contains(f))
        .map(|(f, lo, hi, meaning)| {
            format!(
                "- {f}：必须是 [{lo}, {hi}] 之间的 **JSON 数值**（{meaning}）；\
                 确实无法判断时输出 null（不要写 \"unknown\"、\"无法判断\" 等文字）\n"
            )
        })
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    format!("数值型字段（必须是 JSON 数值或 null，不得输出文字描述或标签）：\n{}", rows.concat())
}

/// `stock_analyses.decision_horizon_source` 的**唯一值域** —— 主档「是谁定的档」。
///
/// 为什么必须在 harness 立这一份：同一个值有三处载体 —— 产出方 `portfolio-mgr.rhai`
/// （脚本字面量）、落库方 `stock_workflow::decision::extract_horizon_source`（白名单归一）、
/// 展示方前端 `horizonSourceLabelKey`（值 → i18n 键）。三处各抄一遍字面量时，任一处漏一个值
/// 的后果不是报错而是**静默归一成 `model`（= 采信模型自报）** —— 那是把「四档分支选档」说成
/// 「模型说了算」，与「缺席不得伪装成别的来源」同族。值域在此单点，白名单与门禁都从这里读。
pub const HORIZON_SOURCES: &[&str] = &[
    "branch_pick",       // 现网正常路径：主档由四档分支结论选出（Q1=C，PLAN §四十八）
    "formula_no_branch", // 兜底：四路分支全部未产出 ⇒ 退回后验阈值定档（必须与上一值可区分）
    "formula",           // 历史值：〇-B v2 ~ R-11 之前的本地公式阈值定档
    "model",             // 历史值：v2 之前采信 trader 自报
    "user",              // 历史值：v1 的用户入口锁档（通路已撤除，仅存量行可能带此值）
];

/// 判定一个 `horizonSource` 字面量是否在值域内（落库白名单与门禁共用）。
pub fn is_horizon_source(v: &str) -> bool {
    HORIZON_SOURCES.contains(&v)
}

impl Period {
    /// 全部档位（用于「按周期分档」的批量任务枚举，如 batch-reflection 的 4 周期筛选）。
    pub const ALL: [Period; 4] = [Period::UltraShort, Period::Short, Period::Mid, Period::Long];

    pub fn as_str(&self) -> &'static str {
        match self {
            Period::UltraShort => "ultra_short",
            Period::Short => "short",
            Period::Mid => "mid",
            Period::Long => "long",
        }
    }

    /// 该档在 `decisionsByHorizon` / `horizonPriceMap` 里的 **camelCase 键**。
    ///
    /// 与 [`Self::as_str`]（snake，用于落库列 / 仲裁 / 反思）是**同一个档的两种拼写**：
    /// 展示层（`HORIZON_KEYS`）与装配段（`decisions_by_horizon[b.camel]`）都按 camel 读，
    /// 而 Rust 侧此前没有权威 ⇒ `decision::compute_decision_agreement` 想按 LLM 自报档
    /// 取对齐行时只能自己再编一份键映射（禁区 12 的第二份真相）。此处单点。
    pub fn camel_key(&self) -> &'static str {
        match self {
            Period::UltraShort => "ultraShort",
            Period::Short => "short",
            Period::Mid => "mid",
            Period::Long => "long",
        }
    }

    /// 周期因子（用于动态仓位）
    pub fn factor(&self) -> f64 {
        match self {
            Period::UltraShort => 0.4,
            Period::Short => 0.6,
            Period::Mid => 0.8,
            Period::Long => 1.0,
        }
    }

    /// 建议持有天数
    pub fn default_holding_days(&self) -> u32 {
        match self {
            Period::UltraShort => 2,
            Period::Short => 5,
            Period::Mid => 28,
            Period::Long => 90,
        }
    }

    /// 四档的**验证窗口集**（荐股线 T+N 默认值与逐档表现窗口的唯一来源）。
    ///
    /// 为什么收在这里而不是各读点手抄：`[5, 20, 60]` 那组数此前同时抄在
    /// `backtest_validation.rs` 的默认值、两处文档注释与三个统计读点上 ⇒ 换窗口集必然漏一处，
    /// 而漏掉的那处会把「旧窗口的命中」和「新窗口的命中」并成同一个分母
    /// （`compute_hit_rate_report` 的 by_action / by_style 就是**跨窗口池化**的）。
    /// 单位 = 交易日，与 [`Period::default_holding_days`] 同口径（K 线 bar 位移天然只数交易日）。
    pub fn validation_windows() -> Vec<i32> {
        Self::ALL.iter().map(|p| p.default_holding_days() as i32).collect()
    }

    /// 某个 T+N 是否属于**当前**窗口集。统计读点用它把「上线前窗口」的样本挡在分母外
    /// （拍板 6：不回填、如实标成上线前样本，而不是拿旧窗口的命中跟新窗口比大小）。
    pub fn is_validation_window(t_plus_n: i32) -> bool {
        Self::ALL.iter().any(|p| p.default_holding_days() as i32 == t_plus_n)
    }

    /// 本档的**技术面尺度**（取值 = `astock-data::scale::ScaleProfile::resolve` 认的那些键）。
    ///
    /// ## 定阶规则（2026-10-18 用户裁定，`PLAN-horizon-scale-alignment.md` §一）
    ///
    /// **`d ≤ h / 3`**：每根 bar 覆盖的交易日 `d` 不得超过本档持有期 `h` 的三分之一 ——
    /// 否则「这一档的走势」只由 1~2 根 bar 决定，按构造分辨不出持有期内的结构。
    ///
    /// | 档 | `h`（交易日） | 尺度 | `d` | `h/d` |
    /// |---|---|---|---|---|
    /// | 超短 | 2 | hourly | 0.25 | 8.0 |
    /// | 短 | 5 | daily | 1 | 5.0 |
    /// | 中 | 28 | weekly | 5 | 5.6 |
    /// | 长 | **90** | monthly | 20 | 4.5 |
    ///
    /// ⚠ **第 4 档是「90 交易日」，不是「季线」**：`quarterly` 的 `d = 60`，`h/d = 1.5`
    /// ⇒ 违反本条规则（且季线自身的 60 期地板要 **15 年**历史，见
    /// `recommender/scale.rs` 与 `PLAN-horizon-scale-alignment.md` D2）。2026-10-18 之前
    /// 短/中/长三档分别是 weekly/monthly/quarterly（`h/d` = 1.0 / 1.4 / 1.5），三档**同时**违规。
    ///
    /// 存在理由（历史）：「超短=小时线 / 短=周线 / 中=月线 / 长=季线」曾是阶段 2 的拍板，但此前
    /// **只写在**种子注释和「哪档读哪个 `t-scoring-*`」的字面量里，代码里没有权威 ⇒ 于是
    /// PLAN §九十二 量到的那条 `period` 死参数（四档实际取的都是同一份日线）**没有任何门能发现**。
    ///
    /// 分工（分层依赖方向）：本函数只定「哪档配哪个尺度」；尺度自身的日历跨度
    /// （`trading_days_per_bar`）与窗口根数换算住在 `astock-data::scale`，那里才是有 `d_s` 的地方
    /// —— 把换算放 harness 会让 `0.25/1/5/20/60` 这张表出现第二份（禁区 12）。
    /// 荐股链（`analysis-engine/recommender/scale.rs`）**不得**再写自己的四臂表，
    /// 一律转调本函数（门 `check-reco-horizon-parity.mjs` 的门 n 锁这条）。
    pub fn scale_key(&self) -> &'static str {
        match self {
            Period::UltraShort => "hourly",
            Period::Short => "daily",
            Period::Mid => "weekly",
            Period::Long => "monthly",
        }
    }

    /// 本档的评分节点 id —— 与 [`Self::scale_key`] 是**同一个决定的两种拼写**，必须同臂改。
    ///
    /// 放在 harness 而不是让种子自己拼字符串：`scripts/check-tier-purity.mjs` 的 R5 要拿权威
    /// 反查种子（「这一档的 `tier_score` 来源节点 ≡ 本档尺度对应的节点」），
    /// 拼接的话门就只能猜后缀（`hour` vs `hourly` 的差别正是曾经出错的地方）。
    ///
    /// ⚠ 命名后缀 = 尺度名的短形，**不是**持有期名：`t-scoring-day` 属短档（5 交易日）、
    /// `t-scoring-month` 属长档（**90 交易日**）。「月」在这里指 bar 粒度（20 交易日/根），
    /// 不是「持有一个月」—— 长期档的口径永远是 90 交易日（见 [`Self::scale_key`] 的定阶表）。
    pub fn scoring_node_id(&self) -> &'static str {
        match self {
            Period::UltraShort => "t-scoring-hour",
            Period::Short => "t-scoring-day",
            Period::Mid => "t-scoring-week",
            Period::Long => "t-scoring-month",
        }
    }

    /// [`Self::scale_key`] 的**反查**：尺度名 → 该尺度唯一对应的那一档。
    ///
    /// 存在理由（PLAN §一○二(4) 片 A 的待决形状，已按「反查」拍定）：
    /// `compute_scoring` 拿得到的只有 `period` 串（尺度名），而窗口计划
    /// `ScaleWindowPlan::for_period` 要的是 `Period`。另一条路是给每张档子模板再加一个
    /// `const-scoring-horizon` 常量节点、多传一个工具参数 —— 那会让图再变、再换代，
    /// 而「哪档配哪尺度」这张表的权威本来就在本文件，反查不需要第二处事实。
    ///
    /// 实现刻意**不写第二份映射**：遍历 [`Self::ALL`] 比 `scale_key()` ⇒ 加第五档时
    /// 只有 `as_str` / `scale_key` / `scoring_node_id` 三处必须一起长（编译器 + R5 门），
    /// 本函数自动跟着变。
    ///
    /// ⚠ **2026-10-18 起 `daily` 与 `quarterly` 的角色对调了**（`PLAN-horizon-scale-alignment.md`
    /// §一）：短档的尺度由 weekly 下移到 `daily`，长档由 `quarterly` 上移到 `monthly`。于是
    /// ①`from_scale_key("daily") == Some(Short)`，日线**不再是**「无主的公共尺度」；
    /// ②`from_scale_key("quarterly") == None`，季线**才是**新晋的无主尺度（它的 60 期地板要
    /// 15 年历史，见 `recommender/scale.rs` 的 D2）。
    ///
    /// ⇒ **调用方不得再用「尺度串能否反查到档」来区分「主链日线」与「按档日线」** ——
    /// 两者都是 `daily`，只能靠显式 `horizon` 参数分辨（`astock-data::mcp_tools::compute_scoring`
    /// 就是这么做的：`horizon` 优先，`period` 仅作冲突校验）。这正是本函数注释必须写清
    /// 正反两侧现状的原因：留一句「daily 返回 None」的旧话，下一个读它的人会照着写出
    /// 「主链走 None 分支」的错代码，而编译器不会反对。
    pub fn from_scale_key(scale: &str) -> Option<Period> {
        Period::ALL.into_iter().find(|p| p.scale_key() == scale)
    }

    /// 把任意持有天数归到最近的档位。
    ///
    /// 用途：`stock_analyses.decision_expected_holding_days` 是 LLM 给的自由数字
    /// （或缺失时兜底 28），要按 4 周期分档筛选就必须先做最近邻归一。
    pub fn nearest_for_holding_days(days: i64) -> Period {
        Period::ALL
            .iter()
            .copied()
            .min_by_key(|p| (p.default_holding_days() as i64 - days).abs())
            .unwrap_or(Period::Mid)
    }

    /// 周期仓位乘数 —— **决策链口径**（证据权重层 `evidence_weight` 与
    /// `portfolio-mgr.rhai` 共用本函数；不是荐股链 `Period::factor` 的那个口径，
    /// 两者语义不同：`factor` 是荐股候选的相对仓位系数，本乘数是「同一决策在
    /// 不同周期上应下注多重」的修正，历史上曾在两处各写一份数字）。
    pub fn position_multiplier(&self) -> f64 {
        match self {
            Period::UltraShort => 0.6,
            Period::Short => 0.8,
            Period::Mid => 1.0,
            Period::Long => 1.2,
        }
    }

    /// 注入 Rhai 决策脚本的「周期常量表」：
    /// `{ultra_short: {days: 2, mult: 0.6}, …}`。
    ///
    /// 供 `stock_workflow/hooks.rs` 注入为变量 `horizon_consts_json`；
    /// 脚本侧**不得**再手抄天数或乘数（见 `portfolio-mgr.rhai` 的 `days_for` /
    /// `position_pct` 两处消费点）。
    pub fn decision_consts_map() -> serde_json::Value {
        serde_json::Value::Object(
            Period::ALL
                .iter()
                .map(|p| {
                    (
                        p.as_str().to_string(),
                        serde_json::json!({
                            "days": p.default_holding_days(),
                            "mult": p.position_multiplier(),
                        }),
                    )
                })
                .collect(),
        )
    }
    /// 渲染**本档**的 VERDICT 字段契约，供 Agent 节点执行期注入 system prompt。
    ///
    /// 为什么要注入而不是把清单手抄进各专家 md：手抄会与 [`Period::verdict_spec`] 漂移 ——
    /// 而这张表是 prompt、Rhai 校验、CI 门三方的唯一源（禁区 12）。注入后模型看到的清单
    /// 与门校验的是同一份数据，改表即改 prompt，不存在「md 说七项、门按八项判」的中间态。
    ///
    /// 触发方（`agent_executor.rs`）只在节点上下文带 `horizon` 变量时注入 ——
    /// 即该节点属于 P4′ 的某个持有期分支。现状（分析师无档运行）下本函数不参与执行，
    /// 所以落这一步不会改变任何现网输出。
    pub fn verdict_contract_prompt(&self) -> String {
        let spec = self.verdict_spec();
        let mut out = String::new();
        out.push_str(&format!(
            "\n\n--- 本档 VERDICT 字段契约（{}，建议持有 {} 日）---\n",
            self.label_zh(),
            self.default_holding_days()
        ));
        out.push_str(&format!(
            "本节点属于按持有期分档的分支，档位由代码盖章为 `{}`，不得自行改写档位归属。\n",
            self.as_str()
        ));

        out.push_str("本档**必须**输出的字段（每一项都是本档确实计算的事）：");
        out.push_str(&spec.fields.join("、"));
        out.push('\n');

        // v155：数值型字段的产出形态必须进契约 —— 原先只给字段名不给值域，模型把
        // `flowPersistence` 当自由文本槽（现网实证 "unknown"/"short_pulse"/中文句子），
        // 而消费端（`analysis-engine::leg_signal`）只收数值 ⇒ 该腿从未参与融合
        // （短档联合门因此恒「未判定」、买入/增持被封死）。值域单源在
        // [`NUMERIC_VERDICT_FACTORS`]，消费侧由 `leg_signal` 的测试反向对齐。
        // 本档 fields 里没有数值型因子 ⇒ 该节为空串（超短/长档的契约逐字不变）。
        out.push_str(&numeric_fields_section(&spec.fields, NUMERIC_VERDICT_FACTORS));

        out.push_str(&format!("`exitRule` 在本档固定为 `{}`。\n", spec.exit_rule));

        if !spec.qualified.is_empty() {
            out.push_str("条件字段（可以输出，但必须同时给出标记，且**不得进方向加权**）：\n");
            for (factor, key) in &spec.qualified {
                out.push_str(&format!(
                    "- {factor} → 标记 `{key}`（{why}）\n",
                    why = qualified_reason(key)
                ));
            }
        }

        if !spec.not_applicable.is_empty() {
            out.push_str(
                "本档**按构造不适用**的因子：不得省略、不得悄悄给分，必须写进 `notApplicable` 数组并点名下键：\n",
            );
            for (factor, key) in &spec.not_applicable {
                out.push_str(&format!("- {factor} → `{key}`\n"));
            }
        }

        out.push_str("VERDICT JSON 必须含这三类键：必填字段各自、`notApplicable`: [因子名…]、`qualified`: {因子名: 标记键}。\n");
        out.push_str("契约里没列的因子不得出现在必填位置；列了却留空的，按未计算处理。\n");
        out
    }

    /// 档位的中文标签 —— **全仓唯一档名**。
    ///
    /// 三处必须逐字与本题字面一致，任一处漂移都会被门抓红
    /// （`scripts/check-horizon-label-parity.mjs`）：
    /// ① 前端 i18n 的**唯一**档名键族 `stockAnalysis.timeHorizon*`
    ///    （取值一律经 `horizonLabelKey()`，`src/lib/stock-analysis-utils.ts`）；
    /// ② Rhai 装配段的 `cn_tier`（`portfolio-mgr.rhai`，写进 `dataGaps`）；
    /// ③ 导出模块的 `labelMap`（`src/lib/stock-analysis-export.ts`）。
    /// 另有第 ④ 处**非 i18n** 副本：`e2e/stock-reflection-four-horizon.spec.ts` 的
    /// `horizonTab(..., "🟢 长线")` 硬编码锚点（判据 D 盯它）。
    ///
    /// ⚠ 2026-10-16 收敛：此前前端有 **8 套**档名键族（`period.*` / `period*` /
    /// `recommendation.period*` / `reflection.horizon*` / `scheduledAnalysis.period.*`、
    /// `replayWorkbench.batch.period.*`、`settings.scheduled-recommendation.period-*`
    /// 与 `timeHorizon*`），值虽统一但**定义是 8 份** ⇒ 任一族漏改即前后端同屏两名。
    /// 当日把另 7 套整体删除（28 键 × 11 语言 + 2 个被删空的壳对象），门禁改为
    /// 「唯一族 + 禁回流」（判据 E）。列在此处是为了说明**为何只剩一族**，不是现存清单。
    ///
    /// ⚠ 第 4 档是「长线」而**不是**「季度」：`quarterly` 是**评分尺度名**
    /// （`Scale::Quarterly` = 60 交易日/bar，见 `astock-data/src/scale.rs`），
    /// 而本档的持有期是 **90 交易日**（≈ 4.2 自然月）—— 二者相差 1.5 倍。
    /// 2026-10-16 曾按「90 天 = 一个季度」把对外名误改为「季度」，
    /// 造成「季度 90 天」这类自相矛盾文案与前后端同屏两名；当日已回退。
    /// 事故与本轮更正的完整取证见 `AUDIT-horizon-naming-2026-10-16.md`。
    pub fn label_zh(&self) -> &'static str {
        match self {
            Period::UltraShort => "超短线",
            Period::Short => "短线",
            Period::Mid => "中线",
            Period::Long => "长线",
        }
    }

    /// 逐档因子全集 —— 契约表三类划分（必填 / 不适用 / 条件）的取值域。
    ///
    /// 门（`seed_consistency_tests.rs`）读这个而不是另抄一份名单：否则「表有九项、门按十项判」
    /// 会成为新的漂移源。
    pub fn verdict_factors() -> &'static [&'static str] {
        VERDICT_FACTORS
    }

    /// 该档的 VERDICT 契约（见 [`HorizonVerdictSpec`]）。
    ///
    /// 单元格判定依据是本仓**数据现实**（P0/P9 实测），不是理念偏好。例：超短的
    /// `microstructure` 从「无逐笔也无连板字段」升级为「连板/封单/炸板有真字段、仍无逐笔」，
    /// 是 P9-4 接入同花顺涨停池的结果；它仍留在 `qualified` 而非算作无条件必填，
    /// 因为撮合级明细至今零通路（P9-6 待探）。
    pub fn verdict_spec(&self) -> HorizonVerdictSpec {
        // (该档直接用到的因子, 只作过滤/部分来源的因子 → 标记键, 出场口径)
        let (plain, qualified, exit_rule): (&[&str], &[(&str, &str)], &str) = match self {
            // 超短（2 日）：方向腿 = 短窗动量 **+ 事件催化剂**（v156，#22 另一半）。
            // 催化剂是个股级事件证据、2 日窗恰是事件驱动窗口（PLAN §十一 原设计即三腿：
            // 动量 / 广度 / 催化剂）；它是与 MACD 独立的信息源，接入后 posterior 不再
            // 只由小时线 MACD 四态决定（此前单腿 × 离散阶梯 ⇒ 输出坍缩到 8 个离散值，
            // 动作在买入/卖出间二值化，见 `PLAN-horizon-decision-repair.md` P1）。
            // ⚠ 催化剂**不进** `not_applicable`/`fields` 的构建：它在 `VERDICT_COMMON_FIELDS`
            //   里本就逐档必填，进 `plain` 只影响 `participating`（= 方向腿集合），
            //   `analyst_subset()` 因 `COMMON_FACTOR_OWNERS` 早已含 `a-catalyst` ⇒ 子集不变。
            // 涨停板情绪广度（家数/封板率/炸板，P9-4 真字段）是**市场级量** —— 按本档
            // `entryGate`（`breadth_required`）只作入场必要条件，**不进方向加权**
            // （见 `portfolio-mgr-h-ultra-short.rhai` 头部第 1 条专属判据）。
            // 把它当加权腿会连出两种错：① 允许「广度很差但个股很强」合成出买入；
            // ② 同一天不同个股拿到**同一个**方向信号（封板率与个股无关）。
            // `microstructure`（封单/连板结构）只到板级、无撮合级 ⇒ 条件字段。
            Period::UltraShort => (
                &["momentumSignal", "eventCatalyst"],
                &[
                    ("microstructure", "microstructurePartialSource"),
                    ("breadthState", "breadthGateOnly"),
                ],
                "time_stop",
            ),
            Period::Short => (
                // 短线（5 日）：方向腿 = 动量 / 资金持续性 / 趋势 / 供给冲击。
                // 涨停板情绪广度（`sealRate` → `breadthState`）与超短同源，是**市场级量**
                // ⇒ 2026-10-10 起只作背景过滤（`qualified`），**不进方向加权**：
                // 一旦计权，同一天全部个股会共享同一个方向信号（实证 002241/2026-10-10：
                // 广度 = 封板率 0.683，与个股无关，却把短档后验抬高）。
                // ⚠ 本档的门仍是 `trend_and_flow_same_sign`（趋势 × 资金同号），**不是**广度门
                //   —— `breadthGateOnly` 在这里只借「市场级量、只作过滤、权重恒 0」这层语义。
                // ⚠ 短线（5 日）：本档窗内基准用**中证全指**同窗累计收益（口径与注入口
                //   同 mid 档那条注释；实现在 `harness::indicators::excess_return_pct`，
                //   单一实现，Rhai 侧不重算）。
                &[
                    "momentumSignal",
                    "flowPersistence",
                    "trendStrength",
                    "supplyShock",
                    "relativeStrength",
                ],
                &[("breadthState", "breadthGateOnly")],
                "time_stop+fixed_stop",
            ),
            Period::Mid => (
                // `relativeStrength`（阶段 1，2026-10-19）：同窗「个股累计收益 − 基准累计收益」。
                // 与 `breadthState`（封板率）**相反**，它是**个股自身**的量 —— 同一天每只票拿到
                // 不同的数，所以可以进方向加权而不犯「市场级量被逐票共享 ⇒ 全市场同向抬后验」
                // 那条错（002241/2026-10-10 的实证教训）。
                // 立项实证：002696 as-of 2026-09-28 判「观望 3.3%」，其后 12 交易日 +65.3%，
                // 而同期创业板 −11.7%；锚点当天的 5 日窗实测超额 **+2.25 个百分点**
                // （个股 −5.39% / 中证全指 −7.64%，见 `indicators::excess_return_pct` 的同名真数据测试）
                // —— 这份证据在框架里**从来没有入口**。
                &[
                    "flowPersistence",
                    "trendStrength",
                    "valuationBand",
                    "expectationRevision",
                    "relativeStrength",
                ],
                &[
                    ("earningsQuality", "qualityRealizationBeyondHorizon"),
                    ("supplyShock", "supplyAsRiskNoteOnly"),
                ],
                "k_sigma_band",
            ),
            // 长档（90 日）：技术腿只作入场过滤；行业轮动 live 有源但回放不可得 ⇒ 条件字段。
            //
            // P0-3（`PLAN-stock-decision-reliability.md`，2026-10-16）：估值腿**移出方向加权、
            // 降为高估否决过滤腿**（`valuationFilterOnly`）。实证：90 日窗口内估值分位/DCF
            // 安全边际没有方向预测力（10-09 紫金矿业：DCF 上行 25.7% 放行长档买入，90 日内
            // 即跌破决策价），而「低估 ⇒ 放行买入」的门会把周期顶部的乐观 DCF 当成开仓依据。
            // 新口径（单向否决，与入场门 `portfolio-mgr-h-long.rhai` 同源改）：
            //   估值**便宜不再是买入理由**，只有**严重高估才否决**；腿仍必产出（子图照挂
            //   value-investor），信号照算，只是权重恒 0、不进 posterior。
            Period::Long => (
                &["earningsQuality", "expectationRevision", "macroRegime"],
                &[
                    ("valuationBand", "valuationFilterOnly"),
                    ("trendStrength", "trendFilterOnly"),
                    ("macroRegime", "macroPartialSeriesOnly"),
                    ("sectorRotation", "sectorRotationAsOfGap"),
                ],
                "target_and_falsified",
            ),
        };

        // used = 直接用到 ∪ 有条件用到；not_applicable = 九因子 − used
        let mut used: Vec<&'static str> = Vec::new();
        for f in plain.iter().copied().chain(qualified.iter().map(|(f, _)| *f)) {
            if !used.contains(&f) {
                used.push(f);
            }
        }
        let fields: Vec<&'static str> =
            VERDICT_COMMON_FIELDS.iter().copied().chain(used.iter().copied()).collect();
        let not_applicable: Vec<(&'static str, &'static str)> = VERDICT_FACTORS
            .iter()
            .copied()
            .filter(|f| !used.contains(f))
            .map(|f| (f, na_key(f)))
            .collect();

        HorizonVerdictSpec {
            fields,
            not_applicable,
            qualified: qualified.to_vec(),
            participating: plain.to_vec(),
            exit_rule,
        }
    }

    /// 因子 → 证据属主分析师（[`VERDICT_FACTOR_OWNERS`] ∪ [`COMMON_FACTOR_OWNERS`]）。
    pub fn factor_owner(factor: &str) -> Option<&'static str> {
        VERDICT_FACTOR_OWNERS
            .iter()
            .chain(COMMON_FACTOR_OWNERS.iter())
            .find(|(f, _)| *f == factor)
            .map(|(_, a)| *a)
    }

    /// 该档**子工作流应挂载的分析师** = 该档全部参与因子（必填 ∪ 条件）的属主 ∪ 共用因子属主，
    /// 去重后按名排序（排序是为了让门与快照可比，不代表优先级）。
    ///
    /// 为什么由推导而不是清单：清单里漏挂一个属主 ⇒ 该腿在子图里拿不到产出方（实测
    /// PLAN §10-4 的中档/长档都漏了 `value-investor`，而两档的 `valuationBand` 都是必填）；
    /// 多挂一个无因子的分析师 ⇒ 白烧一次 LLM 调用并把「它在场」误读成「它的证据进了融合」。
    pub fn analyst_subset(&self) -> Vec<&'static str> {
        let spec = self.verdict_spec();
        let mut out: Vec<&'static str> = spec
            .fields
            .iter()
            .filter_map(|f| Self::factor_owner(f))
            .chain(spec.qualified.iter().filter_map(|(f, _)| Self::factor_owner(f)))
            .chain(COMMON_FACTOR_OWNERS.iter().map(|(_, a)| *a))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl std::str::FromStr for Period {
    type Err = String;

    /// 兼容 DB 里存的 `ultra_short` / `short` / `mid` / `long`
    /// 与历史存档里的 `ultrashort`。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ultra_short" | "ultrashort" => Ok(Self::UltraShort),
            "short" => Ok(Self::Short),
            "mid" => Ok(Self::Mid),
            "long" => Ok(Self::Long),
            other => Err(format!("未知持有周期: {other}")),
        }
    }
}

/// 档位后缀的分隔符 —— **唯一权威值**。
///
/// 为什么不是单横线：base id 自身就含单横线（`a-market-analyst`、`a-hot-money`），
/// 用单横线拼接后「该剥到哪一段」没有唯一解；`--` 不在 base 域里出现 ⇒ `strip` 唯一可逆。
/// 配套的门反向锁「base 域里禁止出现 `--`」，否则这条前提会在未来某个新分析师身上静默失效。
pub const TIER_ID_SEP: &str = "--";

/// 分隔符的历史名（v133 起就在用，当时只有分析师实例带档）。
///
/// ⚠ **不是第二份定义**：值由 [`TIER_ID_SEP`] 引过来，全仓只有那一个字面量。
/// v161（逐档辩论）把带档后缀扩到摘要 / 辩论容器 / 辩手 / 收敛四类节点，
/// 这些节点族不该借用「ANALYST」这个名字 ⇒ 新代码一律用 [`TIER_ID_SEP`] + [`tier_node_id`]；
/// 保留本别名只是为了让既有引用面（`analyst_base_of` 与前端镜像）不必同批改名。
/// （`AGENTS.md` 禁区 12：扩展而非重定义。）
pub const ANALYST_TIER_SEP: &str = TIER_ID_SEP;

/// 给**任意** base 节点 id 挂上档位后缀 —— 逐档节点的通用正门。
///
/// 存在理由（v161 逐档辩论）：后缀形态 `base--<tier>` 自 v133 起就不只属于分析师
/// （`value-verify--<档>`、`analyst-brief--<档>`、`debate-convergence--<档>` 同形）。
/// 拼法若各处再写一遍 `format!("{stem}--{tier}")`，就会出现「一处用 `--`、一处用 `-`」
/// 这种剥不回来的分叉 —— 与 `analyst_node_id` 当初要解决的是同一个病。
/// 故本函数是唯一拼口，[`analyst_node_id`] 只是它的分析师别名。
///
/// 用前自检两条（都是判据级，不是风格）：
/// ① base 里**不得**已含 `--`（否则 [`base_of_tiered_id`] 剥不唯一）；
/// ② 后缀必须是 [`Period::as_str`] 的四臂之一（调用侧用 `p.as_str()`，不手抄档名）。
///
/// ```
/// use axagent_harness::holding_period::tier_node_id;
/// assert_eq!(tier_node_id("debate-convergence", "mid"), "debate-convergence--mid");
/// ```
pub fn tier_node_id(base: &str, tier_snake: &str) -> String {
    debug_assert!(
        !base.contains(TIER_ID_SEP),
        "base 域禁止包含分隔符 {TIER_ID_SEP}（会让按档剥离不唯一）"
    );
    format!("{base}{TIER_ID_SEP}{tier_snake}")
}

/// 某档分析师子集里一个分析师的**实例节点 id**（分析师族别名，拼法见 [`tier_node_id`]）。
///
/// 存在理由（B2-1）：四档各跑 [`Period::analyst_subset`] 列出的分析师 ⇒ 同一个 base 需要多份
/// 节点 id。若把档位后缀写死进字符串字面量，全仓十几处「按完整 id 精确匹配」的读端
/// （种子里的 `match *id`、data-quality 的映射、`decision.rs` 的 expert_mapping、
/// `astock-data/quality.rs` 的必采清单等）会**静默失效**：不报错，只是那一格永远是空的。
/// 读写两端都经由本函数与 [`analyst_base_of`]，消费侧的键名域（短名、i18n key、面板行 key）
/// 就完全不需要知道「档位后缀」这件事存在。
///
/// ```
/// use axagent_harness::holding_period::analyst_node_id;
/// assert_eq!(analyst_node_id("a-market-analyst", "mid"), "a-market-analyst--mid");
/// ```
pub fn analyst_node_id(base: &str, tier_snake: &str) -> String {
    tier_node_id(base, tier_snake)
}

/// 从节点 id 剥回 base；不是「base--已知档」形状时返回 `None`。
///
/// `None` 是有意义的两种情形，调用方**不得猜**：① 历史行与快速链里的裸 base id（改造前的
/// 节点、以及刻意不分档的那些链），② 后缀不是四档之一（接线接错对象，该报不该兜）。
///
/// ```
/// use axagent_harness::holding_period::analyst_base_of;
/// assert_eq!(analyst_base_of("a-hot-money--ultra_short"), Some("a-hot-money"));
/// assert_eq!(analyst_base_of("a-hot-money"), None, "裸 base id 不当成某档实例");
/// assert_eq!(analyst_base_of("a-hot-money--deca"), None, "未知档位后缀不猜 base");
/// ```
pub fn analyst_base_of(node_id: &str) -> Option<&str> {
    let (base, tier) = node_id.split_once(ANALYST_TIER_SEP)?;
    if base.is_empty() || !Period::ALL.iter().any(|p| p.as_str() == tier) {
        return None;
    }
    Some(base)
}

/// data-quality 诊断的**槽位缩写 ↔ 分析师 base 节点 id** 对照（10 个槽位，Rust 侧唯一登记处）。
///
/// 语义与 `commands/data-quality.rhai` 头部那张对照表一致。为什么必须是 const 而不是各调用点
/// 各抄一遍字面量：这张表同时被三处消费 —— 种子里 data-quality 的 40 键 `input_mapping`、
/// `analyst_feedbacks` 的写侧（诊断缩写 → 反馈表 node_id）、以及排障时的反查。此前它只长在
/// 种子的闭包里（一份内联数组），写侧要用的时候只能再抄一份 ⇒ 正是本仓反复登记的
/// 「读写两端键域不一致 ⇒ 查表恒落空、被兜底掩盖成看起来正常」的形态。
///
/// ⚠ 前端另有一份镜像（`src/lib/dataQualityDiagnosis.ts` 的 `EXPERT_ID_TO_DQ_ABBR`），
///   两侧由 `scripts/check-dq-abbr-parity.mjs` 逐槽位对拍：新增/摘除分析师时漏改任一侧即红。
pub const DQ_ABBR_TO_BASE: &[(&str, &str)] = &[
    ("mk", "a-market-analyst"),
    ("sent", "a-sentiment"),
    ("fund", "a-fundamentals"),
    ("pol", "a-policy"),
    ("hm", "a-hot-money"),
    ("lk", "a-lockup"),
    ("res", "a-research"),
    ("sec", "a-sector"),
    ("cat", "a-catalyst"),
    ("val", "value-investor"),
];

/// 缩写 → base 节点 id。未知缩写返回 `None`（**不猜**：新分析师没登记进来时，
/// 症状应当是「那一格没有反馈」，而不是被顶到某个既有分析师上）。
///
/// ```
/// use axagent_harness::holding_period::dq_base_of_abbr;
/// assert_eq!(dq_base_of_abbr("mk"), Some("a-market-analyst"));
/// assert_eq!(dq_base_of_abbr("zz"), None);
/// ```
pub fn dq_base_of_abbr(abbr: &str) -> Option<&'static str> {
    DQ_ABBR_TO_BASE.iter().find(|(a, _)| *a == abbr).map(|(_, b)| *b)
}

/// base 节点 id → 缩写（携带逐档后缀的实例 id 会先剥档，再查）。
///
/// ```
/// use axagent_harness::holding_period::dq_abbr_of_base;
/// assert_eq!(dq_abbr_of_base("a-market-analyst--mid"), Some("mk"));
/// assert_eq!(dq_abbr_of_base("value-investor"), Some("val"));
/// ```
pub fn dq_abbr_of_base(node_id: &str) -> Option<&'static str> {
    let base = analyst_base_of(node_id).unwrap_or(node_id);
    DQ_ABBR_TO_BASE.iter().find(|(_, b)| *b == base).map(|(a, _)| *a)
}

/// 反馈表里某分析师的 `node_id` 取值形态：**去掉 `a-` 前缀的 base**（`market-analyst`），
/// 且**不带档位后缀**。
///
/// 这不是随手选的写法，而是与唯一读侧对齐的结果：`evolve_node_command` / `get_node_feedbacks`
/// 按 `NodeId.eq(...)` 精确过滤，而调用方传的就是去前缀的 base（分析师卡的 `expertId`）。
/// 写侧若加 `a-` 或加 `--mid`，新行一条也查不到（症状与前端的缩写表一模一样）。
/// 档位信息走 `quality_metrics_json.tier`；要按 (分析师, 档) 分桶得先改读侧聚合，
/// 不能靠给写侧再发明一种键形态 —— 那会是第四种。
///
/// ```
/// use axagent_harness::holding_period::dq_feedback_node_id;
/// assert_eq!(dq_feedback_node_id("a-market-analyst"), "market-analyst");
/// assert_eq!(dq_feedback_node_id("value-investor"), "value-investor");
/// ```
pub fn dq_feedback_node_id(base: &str) -> &str {
    base.strip_prefix("a-").unwrap_or(base)
}

/// 统计 / 先验 / 错题本**按代筛样的起算代际**（PLAN four-horizon §五十一-②，2026-10-04 批准）。
///
/// 语义是**下限**而不是「等于当前代」：125 起（四档分支机制落地那一代）的样本算法口径可比，
/// 早于它的样本进统计就是「两代判据的加权平均」。用等号有个致命副作用 —— 每次换代
/// （本仓 v125→v132 只用了两周）分母都会被清零，指标长期停在「样本不足」，
/// 而真因是筛法本身，不是数据不够。
///
/// 值域与代际同处（`workflow_templates.version` 整数）。读侧只做 `>= floor` 判定，
/// **不读库**：floor 是判据参数（量纲常量），不是运行时状态。
pub const HORIZON_BRANCH_GENERATION_FLOOR: i32 = 125;

#[cfg(test)]
mod tests {
    use super::*;

    /// 天数表是全仓唯一权威源：Rhai 注入、反思分档、荐股持有期都从这里取。
    /// 决策链仓位乘数必须逐档显式（历史上它在 evidence_weight 里以 `_ => 1.0` 兜住 mid）。
    #[test]
    fn position_multiplier_is_per_tier_and_mid_explicit() {
        let m = Period::decision_consts_map();
        assert_eq!(m["ultra_short"]["mult"], serde_json::json!(0.6));
        assert_eq!(m["short"]["mult"], serde_json::json!(0.8));
        assert_eq!(m["mid"]["mult"], serde_json::json!(1.0));
        assert_eq!(m["long"]["mult"], serde_json::json!(1.2));
        assert_eq!(m["mid"]["days"], serde_json::json!(28));
    }

    /// `rename_all = "lowercase"` 会把 UltraShort 压成 `ultrashort` —— 前端契约依赖显式 rename。
    #[test]
    fn ultra_short_serializes_with_underscore_and_reads_both() {
        assert_eq!(serde_json::to_string(&Period::UltraShort).unwrap(), "\"ultra_short\"");
        assert_eq!(serde_json::from_str::<Period>("\"ultrashort\"").unwrap(), Period::UltraShort);
    }

    /// `Ord` = 变体声明序 = 由短到长。批量响应靠 `BTreeMap<Period, _>` 固定 JSON 键序，
    /// 若变体被重排，这里必须红（而不是让键序静默变化）。
    #[test]
    fn ord_matches_declaration_order() {
        let mut map = std::collections::BTreeMap::new();
        for p in Period::ALL.iter().rev() {
            map.insert(*p, ());
        }
        let ordered: Vec<Period> = map.keys().copied().collect();
        assert_eq!(ordered, Period::ALL.to_vec());
    }

    /// 逐档契约的结构性不变量：逐档因子不重不漏、标记键只修饰参与字段、共用字段齐、
    /// **四档的因子集必须真的不同**（R-11 的「分叉是真的」机械证明；若有人把一套表抄四遍，
    /// 最后那条立刻红）。
    #[test]
    fn verdict_spec_partitions_factors_and_differs_across_tiers() {
        let mut shapes: Vec<Vec<String>> = Vec::new();
        let mut exits: Vec<&str> = Vec::new();

        for p in Period::ALL {
            let spec = p.verdict_spec();
            let in_fields: Vec<&str> =
                spec.fields.iter().copied().filter(|f| VERDICT_FACTORS.contains(f)).collect();

            for f in &spec.not_applicable {
                assert!(!in_fields.contains(&f.0), "{p:?} 因子 {} 同时算参与与不适用", f.0);
                assert!(f.1.starts_with("notApplicable"), "{p:?} 缺席键形态错: {}", f.1);
            }
            assert_eq!(
                in_fields.len() + spec.not_applicable.len(),
                VERDICT_FACTORS.len(),
                "{p:?} 九因子不重不漏判据破了"
            );
            for (f, key) in &spec.qualified {
                assert!(in_fields.contains(f), "{p:?} qualified 引用了本档不参与计算的 {f}");
                assert!(!key.starts_with("notApplicable"), "{p:?} 条件键与缺席键串了: {key}");
            }
            for f in VERDICT_COMMON_FIELDS {
                assert!(spec.fields.contains(f), "{p:?} 少了共用字段 {f}");
            }
            shapes.push(in_fields.iter().map(|s| s.to_string()).collect());
            exits.push(spec.exit_rule);
        }

        assert!(
            shapes.windows(2).any(|w| w[0] != w[1]),
            "四档逐档因子集完全相同 ⇒ 分叉是假的，违反 R-11：{shapes:?}"
        );
        for pair in exits.windows(2) {
            assert_ne!(pair[0], pair[1], "出场口径逐档必须不同（{exits:?}）");
        }
    }

    /// 黄金值锁表：这张表是 prompt、Rhai 与 CI 门三方的唯一源，改动必须是**有意的**，
    /// 所以逐档把「参与因子」与「缺席文案键」钉死成字面量。
    /// 实测口径来源：超短含连板/封单（P9-4 同花顺涨停池，可按日回溯）但无逐笔；
    /// 长档含宏观五条真序列（P9-1，按发布日裁）而货币社融类缺席。
    #[test]
    fn verdict_spec_golden_values() {
        let ultra = Period::UltraShort.verdict_spec();
        assert_eq!(
            golden(&ultra),
            vec![
                "breadthState".to_string(),
                "microstructure".to_string(),
                "momentumSignal".to_string(),
            ],
            "超短算短窗动量 + 涨停板情绪广度 + 封单/连板结构（仍缺撮合级）"
        );
        assert_eq!(ultra.exit_rule, "time_stop");
        assert_eq!(ultra.not_applicable.len(), 8);
        assert_eq!(
            ultra.qualified,
            vec![
                ("microstructure", "microstructurePartialSource"),
                ("breadthState", "breadthGateOnly"),
            ],
            "超短：情绪广度是市场级量 ⇒ 只作入场必要条件（门），不得进方向加权"
        );

        let short = Period::Short.verdict_spec();
        assert_eq!(
            golden(&short),
            vec![
                "breadthState".to_string(),
                "flowPersistence".to_string(),
                "momentumSignal".to_string(),
                "supplyShock".to_string(),
                "trendStrength".to_string(),
            ]
        );
        assert_eq!(short.exit_rule, "time_stop+fixed_stop");
        assert_eq!(
            short.qualified,
            vec![("breadthState", "breadthGateOnly")],
            "短档：情绪广度与超短同源（市场级量）⇒ 只作背景过滤，不得进方向加权"
        );

        let mid = Period::Mid.verdict_spec();
        assert_eq!(
            golden(&mid),
            vec![
                "earningsQuality".to_string(),
                "expectationRevision".to_string(),
                "flowPersistence".to_string(),
                "supplyShock".to_string(),
                "trendStrength".to_string(),
                "valuationBand".to_string(),
            ]
        );
        assert_eq!(mid.exit_rule, "k_sigma_band");
        assert_eq!(
            mid.qualified,
            vec![
                ("earningsQuality", "qualityRealizationBeyondHorizon"),
                ("supplyShock", "supplyAsRiskNoteOnly"),
            ],
            "中档：盈利质量兑现跨季、解禁降为风险提示"
        );

        let long = Period::Long.verdict_spec();
        assert_eq!(
            golden(&long),
            vec![
                "earningsQuality".to_string(),
                "expectationRevision".to_string(),
                "macroRegime".to_string(),
                "sectorRotation".to_string(),
                "trendStrength".to_string(),
                "valuationBand".to_string(),
            ]
        );
        assert_eq!(long.exit_rule, "target_and_falsified");
        assert_eq!(
            long.qualified,
            vec![
                ("valuationBand", "valuationFilterOnly"),
                ("trendStrength", "trendFilterOnly"),
                ("macroRegime", "macroPartialSeriesOnly"),
                ("sectorRotation", "sectorRotationAsOfGap"),
            ],
            "长档：估值只作高估否决（P0-3）、趋势只作入场过滤、宏观仅五条真序列、行业轮动回放不可得"
        );
        // P0-3：估值腿降背景后仍必须**输出**（门要消费它做高估否决），只是不进方向加权
        assert!(
            long.fields.contains(&"valuationBand")
                && !long.participating.contains(&"valuationBand"),
            "长档估值腿必须是「必填但只作高估否决过滤」"
        );
        // 长档不得参与计算的短窗因子，必须走显式不适用而不是省略
        let na_long: Vec<&str> = long.not_applicable.iter().map(|(f, _)| *f).collect();
        assert_eq!(
            na_long,
            vec![
                "momentumSignal",
                "microstructure",
                "breadthState",
                "flowPersistence",
                "supplyShock",
            ]
        );
    }

    /// 因子属主表 + 由它推导的逐档分析师挂载集合（P4′ 第一步的唯一权威）。
    ///
    /// 钉死的是**推导结果**，不是某份手抄清单 —— 实测手抄清单 PLAN §10-4 的中档/长档都漏挂
    /// `value-investor`，而两档的 `valuationBand` 都是必填因子 ⇒ 那条腿会在子图里永远
    /// 拿不到产出方。这类「清单比表少一行」的缺陷由本测试而不是由注释拦住。
    #[test]
    fn analyst_subset_is_derived_from_factor_owners() {
        // 每个逐档因子都得有属主（新增因子忘了登记 ⇒ 这里红，而不是挂载集合静默少一人）
        for f in VERDICT_FACTORS {
            assert!(Period::factor_owner(f).is_some(), "因子 {f} 没有登记证据属主分析师");
        }
        for (f, a) in VERDICT_FACTOR_OWNERS {
            assert!(
                VERDICT_FACTORS.contains(f),
                "属主表里的 {f} 不在逐档因子集里（因子改名要同步两处）"
            );
            assert!(!a.is_empty(), "属主表里 {f} 的分析师为空");
        }

        assert_eq!(
            Period::UltraShort.analyst_subset(),
            vec!["a-catalyst", "a-hot-money", "a-market-analyst", "a-sentiment"],
            "超短挂载 = 动量/情绪广度/封单结构 + 共用催化剂"
        );
        assert_eq!(
            Period::Short.analyst_subset(),
            vec!["a-catalyst", "a-hot-money", "a-lockup", "a-market-analyst", "a-sentiment"],
            "短档挂载含筹码面（supplyShock）与趋势（trendStrength）"
        );
        assert_eq!(
            Period::Mid.analyst_subset(),
            vec![
                "a-catalyst",
                "a-fundamentals",
                "a-hot-money",
                "a-lockup",
                "a-market-analyst",
                "a-research",
                "value-investor"
            ],
            "中档必须挂 value-investor（valuationBand 是必填）与 a-fundamentals（earningsQuality 条件参与）"
        );
        assert_eq!(
            Period::Long.analyst_subset(),
            vec![
                "a-catalyst",
                "a-fundamentals",
                "a-market-analyst",
                "a-policy",
                "a-research",
                "a-sector",
                "value-investor"
            ],
            "长档含 a-sector（sectorRotation 条件参与）与 a-policy（macroRegime）"
        );

        // 两个「有节点、无因子」的分析师绝不该出现在任何挂载集合里 ——
        // a-news 与 a-catalyst 同读公告域（双挂=重复计数），research-mgr 是裁决/合成层。
        for p in Period::ALL {
            for banned in ["a-news", "research-mgr"] {
                assert!(
                    !p.analyst_subset().contains(&banned),
                    "{p:?} 挂了 {banned}，但它没有任何因子可产出 ⇒ 白烧一次调用并伪装成证据在场"
                );
            }
            // 每一档里出现的属主，都必须在该档真的参与某个因子（反向也成立 ⇒ 无死腿）
            for a in p.analyst_subset() {
                let owns_something = p
                    .verdict_spec()
                    .fields
                    .iter()
                    .chain(p.verdict_spec().qualified.iter().map(|(f, _)| f))
                    .any(|f| Period::factor_owner(f) == Some(a));
                assert!(owns_something, "{p:?} 挂载的 {a} 在本档没有任何参与因子");
            }
        }
    }

    /// `participating`（进方向加权）与 `fields`（必须输出）**不是一回事**，本测试锁这条划界：
    /// 长档 `trendStrength` 要输出但只作入场过滤；中档 `supplyShock` 要输出但只作风险提示。
    /// 划界破了 ⇒ P4′ 分支表会把「只作过滤」的腿当真凭据加权回去（正是 R-11 要退役的形态）。
    #[test]
    fn participating_excludes_filter_and_risk_note_factors() {
        for p in Period::ALL {
            let spec = p.verdict_spec();
            for f in &spec.participating {
                assert!(spec.fields.contains(f), "{p:?} participating 里的 {f} 不在必填里");
            }
            for (f, key) in &spec.qualified {
                if *key == "trendFilterOnly"
                    || *key == "supplyAsRiskNoteOnly"
                    || *key == "breadthGateOnly"
                    || *key == "valuationFilterOnly"
                {
                    assert!(
                        !spec.participating.contains(f),
                        "{p:?} 的 {f} 标记为 {key}，却仍在进方向加权的清单里"
                    );
                }
            }
        }
        let ultra = Period::UltraShort.verdict_spec();
        assert!(
            !ultra.participating.contains(&"breadthState")
                && ultra.fields.contains(&"breadthState"),
            "超短情绪广度是市场级量：必须输出（门要用）但不得进方向加权"
        );
        // v156（#22 另一半）：超短方向腿 = 短窗动量 + 事件催化剂。
        // 退回单腿会让 posterior 只由小时线 MACD 四态决定（输出坍缩为离散格、
        // 动作二值化，见 `PLAN-horizon-decision-repair.md` P1 的审计证据）。
        assert!(
            ultra.participating.contains(&"eventCatalyst")
                && ultra.fields.contains(&"eventCatalyst"),
            "超短方向腿必须含事件催化剂（个股级证据，2 日窗的事件驱动性正来自它）"
        );
        let short = Period::Short.verdict_spec();
        assert!(
            !short.participating.contains(&"breadthState")
                && short.fields.contains(&"breadthState"),
            "短档情绪广度与超短同源（市场级量）：必须输出但不得进方向加权"
        );
        let long = Period::Long.verdict_spec();
        assert!(
            long.fields.contains(&"trendStrength")
                && !long.participating.contains(&"trendStrength"),
            "长档技术腿必须是「必填但只作过滤」"
        );
        assert!(
            long.fields.contains(&"valuationBand")
                && !long.participating.contains(&"valuationBand"),
            "P0-3：长档估值腿必须是「必填但只作高估否决」，便宜不能当买入理由"
        );
        assert!(
            long.participating.contains(&"macroRegime"),
            "长档宏观在 §十二 是必填参与项，`macroPartialSeriesOnly` 只限定其来源面 ⇒ 应进方向"
        );
        let mid = Period::Mid.verdict_spec();
        assert!(
            !mid.participating.contains(&"supplyShock") && mid.fields.contains(&"supplyShock"),
            "中档解禁应只作风险提示但仍必须输出"
        );
    }

    /// 取「参与因子」并按名排序，便于与字面量对账（不依赖表内书写顺序）。
    fn golden(spec: &HorizonVerdictSpec) -> Vec<String> {
        let mut v: Vec<String> = spec
            .fields
            .iter()
            .copied()
            .filter(|f| VERDICT_FACTORS.contains(f))
            .map(|s| s.to_string())
            .collect();
        v.sort();
        v
    }

    /// 注入文案必须由表渲染：逐档的必填/条件/不适用三类各自出现在自己的档里，
    /// 且**不得串档**（超短的 prompt 里出现「技术腿只作入场过滤」就说明渲染串了）。
    #[test]
    fn verdict_contract_prompt_is_rendered_from_the_table() {
        let ultra = Period::UltraShort.verdict_contract_prompt();
        assert!(ultra.contains("档位由代码盖章为 `ultra_short`"), "{ultra}");
        assert!(ultra.contains("momentumSignal"), "超短必填漏了短窗动量");
        assert!(ultra.contains("microstructurePartialSource"), "超短的条件标记没渲染出来");
        assert!(ultra.contains("notApplicableFlow"), "超短的不适用因子必须显式点名，不能靠省略");
        assert!(!ultra.contains("trendFilterOnly"), "串档：长档的条件键出现在超短文案里");
        let ultra_required = ultra
            .lines()
            .find(|l| l.starts_with("本档**必须**输出的字段"))
            .expect("必填清单行必须存在");
        assert!(
            !ultra_required.contains("valuationBand"),
            "串档：估值带出现在超短必填里：{ultra_required}"
        );
        assert!(
            ultra_required.contains("momentumSignal"),
            "超短必填应含短窗动量：{ultra_required}"
        );

        let long = Period::Long.verdict_contract_prompt();
        let long_required = long
            .lines()
            .find(|l| l.starts_with("本档**必须**输出的字段"))
            .expect("必填清单行必须存在");
        assert!(
            !long_required.contains("momentumSignal"),
            "串档：短窗动量出现在长档必填里：{long_required}"
        );
        assert!(long.contains("`target_and_falsified`"), "长档出场口径没渲染");
        assert!(long.contains("macroPartialSeriesOnly"), "长档宏观只有五条，必须标部分来源");
        assert!(long.contains("trendFilterOnly"), "长档技术腿只作入场过滤");
        assert!(
            long.contains("valuationFilterOnly"),
            "P0-3：长档估值腿必须渲染为「只作高估否决」的条件字段"
        );
        assert!(long.contains("notApplicableMomentum"), "长档要显式声明短窗动量不适用");
        assert!(!long.contains("microstructurePartialSource"), "串档：超短的条件键跑到长档文案里");

        // v155：数值型字段的产出形态（PLAN-horizon-decision-repair.md P0）——
        // short/mid 的必填含 flowPersistence ⇒ 契约必须给值域与「无法判断输出 null」；
        // 超短/长档的必填里没有数值型因子 ⇒ 该节整段不出现（不产出空壳标题）。
        let short = Period::Short.verdict_contract_prompt();
        assert!(short.contains("flowPersistence"), "短档必填含 flowPersistence");
        assert!(short.contains("数值型字段"), "短档契约缺「数值型字段」说明节");
        assert!(short.contains("[-1, 1]"), "短档契约缺 flowPersistence 的值域");
        assert!(short.contains("null"), "短档契约未说明「无法判断输出 null」");
        let mid = Period::Mid.verdict_contract_prompt();
        assert!(mid.contains("数值型字段") && mid.contains("[-1, 1]"), "中档契约缺数值节");
        assert!(!ultra.contains("数值型字段"), "超短必填里没有数值型因子 ⇒ 不应出现该节");
        assert!(!long.contains("数值型字段"), "长档必填里没有数值型因子 ⇒ 不应出现该节");

        // 四段契约互不相同（同一段抄四遍 = 分叉是假的，这里必须红）
        let all: Vec<String> = Period::ALL.iter().map(|p| p.verdict_contract_prompt()).collect();
        for pair in all.windows(2) {
            assert_ne!(pair[0], pair[1], "逐档契约文案完全相同 ⇒ 不是按表渲染");
        }
    }

    /// 把四档契约原文打出来（人工审阅用，`#[ignore]`）：
    /// `cargo test -p axagent-harness verdict_contract_prompt_dump -- --ignored --nocapture`
    #[test]
    #[ignore = "仅用于人工审阅注入文案"]
    fn verdict_contract_prompt_dump() {
        for p in Period::ALL {
            println!("{}", "-".repeat(30));
            print!("{}", p.verdict_contract_prompt());
        }
    }

    /// 数值节的两个守卫各自可拆（负控拆在**真正决定该断言的那道守卫**上）：
    /// ① 表读取守卫 —— 空表 ⇒ 整节消失；② 字段筛选守卫 —— 字段不在表里 ⇒ 整节消失。
    /// 只测「真表出文案」会漏掉「表被清空后照渲染空壳标题」与「字段不命中仍出节」两种形态。
    #[test]
    fn numeric_section_has_two_guards_each_negatively_controlled() {
        // 正控：真表 + 命中字段 ⇒ 节在、且带区间与 null 约定
        let s = numeric_fields_section(&["flowPersistence"], NUMERIC_VERDICT_FACTORS);
        assert!(s.contains("[-1, 1]") && s.contains("null"), "{s}");
        // 负控①（表读取守卫）：空表 ⇒ 整节消失
        assert!(
            numeric_fields_section(&["flowPersistence"], &[]).is_empty(),
            "表为空时仍在渲染数值节 ⇒ 表读取守卫没电"
        );
        // 负控②（字段筛选守卫）：字段不在表里 ⇒ 整节消失
        assert!(
            numeric_fields_section(&["momentumSignal"], NUMERIC_VERDICT_FACTORS).is_empty(),
            "字段不命中仍渲染数值节 ⇒ 字段筛选守卫没电"
        );
    }

    /// 档位 ↔ 尺度 ↔ 评分节点 这三张臂必须**同向**，且由短到长严格变粗。
    ///
    /// 期望表写死在这里是有意的：§九十二 的缺陷正是「四档尺度实际相同（都是日线）」而各处
    /// 字面量互相印证、没有权威可对。本条把那件事变成改一臂就红。
    ///
    /// 2026-10-18 按「`d ≤ h/3`」定阶重排（`PLAN-horizon-scale-alignment.md` §一）：
    /// 短/中/长三档由 weekly/monthly/quarterly 改为 daily/weekly/monthly ——
    /// 旧三档的 `h/d` 是 1.0 / 1.4 / 1.5，全部违反规则；长期档的口径是 **90 交易日**，不是季线。
    /// ⚠ 这条表**不含** `d`：`trading_days_per_bar` 属 `astock-data::scale`（分层方向见 `scale_key` 的文档），
    /// 规则 R 的数值断言在那边（`tier_scales_resolve_the_horizon_with_at_least_three_bars`）。
    #[test]
    fn scale_key_and_scoring_node_are_one_table_per_tier() {
        let expected = [
            (Period::UltraShort, "hourly", "t-scoring-hour", 2),
            (Period::Short, "daily", "t-scoring-day", 5),
            (Period::Mid, "weekly", "t-scoring-week", 28),
            (Period::Long, "monthly", "t-scoring-month", 90),
        ];
        let mut scale_keys: Vec<&str> = Vec::new();
        let mut node_ids: Vec<&str> = Vec::new();
        for (period, want_scale, want_node, want_days) in expected {
            assert_eq!(
                period.scale_key(),
                want_scale,
                "{period:?} 的尺度被改 —— 要与种子的评分节点、窗口推导同批改"
            );
            assert_eq!(period.scoring_node_id(), want_node, "{period:?} 的评分节点被改");
            assert_eq!(
                period.default_holding_days(),
                want_days,
                "{period:?} 的持有期与尺度表不再同臂"
            );
            scale_keys.push(period.scale_key());
            node_ids.push(period.scoring_node_id());
        }
        // 四档尺度/节点互不相同：两档共用一个尺度 ⇒ 两档评分输入恒等（就是历史缺陷的形状）
        assert_eq!(scale_keys.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        assert_eq!(node_ids.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        // 节点 id 的命名约定（种子与 R5 门都按这个前缀定位）
        for id in node_ids {
            assert!(id.starts_with("t-scoring-"), "评分节点 id {id} 不符合 `t-scoring-*` 约定");
        }
        // 反查（`from_scale_key`）必须与正查同臂，且**一一**而非就近：
        // `compute_scoring` 只拿得到尺度串，按档窗口计划靠这张反表拿到档 ⇒ 反查错一档
        // 就是「四档里有一档悄悄换成别的尺度的窗口」，而图与脚本都不会报错。
        for (period, want_scale, _, _) in expected {
            assert_eq!(
                Period::from_scale_key(want_scale),
                Some(period),
                "尺度 {want_scale:?} 反查不到 {period:?} ⇒ 正反两张臂分了家"
            );
        }
        // 无主尺度必须反查成 `None`，且**不得**就近归到某档（负控向：`weekl` 若被归成
        // short，就等于把 §九十二 那条「静默降级」在反方向上重做一遍）。
        //
        // ⚠ 负控值随 §一定阶一起换过：2026-10-18 之前无主的是 `daily`（短档当时配周线），
        // 现在无主的是 **`quarterly`** —— 长档从季线上移到月线后，没有任何档再落在季线。
        // 断言这里仍写 `daily` 会红（它已是短档的尺度），而「红」本身就是这条负控仍有效的证明：
        // 它锁住的不是「日线」这个名字，而是「四档之外的尺度一律 None」这条规则。
        assert_eq!(
            Period::from_scale_key("daily"),
            Some(Period::Short),
            "日线已是短档（5 交易日）的尺度 —— 若回到 None，说明短档又挪走了"
        );
        assert_eq!(
            Period::from_scale_key("quarterly"),
            None,
            "季线不该被算成某一档（60 期地板 = 15 年历史，无档可用）"
        );
        for bad in ["", "weekl", "Monthly", "quarter"] {
            assert_eq!(
                Period::from_scale_key(bad),
                None,
                "未登记的尺度名 {bad:?} 被反查吞掉了 ⇒ 必须返回 None 让调用方显式处理"
            );
        }
    }

    /// B3（拍板 7）：验证窗口集必须是**四档天数现场推导**的那一份，不是手抄的第二套参数。
    ///
    /// 判据写法：正断言只锁「与 `default_holding_days` 同序列」和条数，**不锁具体数字**——
    /// 把 2/5/28/90 再抄一遍进测试，等于允许权威与测试各自漂移还互相看不见。
    /// 负控锁旧窗口集的 20/60：它们不属于当前四档 ⇒ 三个统计读点必须把它们挡在分母外。
    #[test]
    fn validation_windows_derive_from_tier_days_and_reject_the_old_set() {
        let want: Vec<i32> = Period::ALL.iter().map(|p| p.default_holding_days() as i32).collect();
        assert_eq!(Period::validation_windows(), want, "窗口集与四档天数分了家");
        assert_eq!(Period::validation_windows().len(), 4, "窗口集条数必须等于档数");
        for n in &want {
            assert!(Period::is_validation_window(*n), "{n} 是自己产出的窗口却不被认");
        }
        // 旧默认 [5,20,60]：5 两代同义（留在分母），20/60 属上线前窗口
        assert!(Period::is_validation_window(5), "N=5 新旧同义，筛掉它等于丢真样本");
        for stale in [20, 60, 0, -3, 91] {
            assert!(!Period::is_validation_window(stale), "{stale} 不该被当成当前窗口");
        }
    }
}
