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
/// - `qualityRealizationBeyondHorizon`：盈利质量的兑现周期跨季，28 日窗口内只能部分反映。
/// - `supplyAsRiskNoteOnly`：解禁/减持在中档已被趋势吸收，降为风险提示、不参与方向。
/// - `trendFilterOnly`：长档技术腿只作入场时机过滤（R-11 明文不得进方向加权）。
/// - `macroPartialSeriesOnly`：宏观只有五条真序列（P9-1），货币/社融类无供应。
/// - `sectorRotationAsOfGap`：行业景气/轮动**live 有真源**（`get_industry_ranking`），
///   但回放不可得 —— BK 板块历史源已普查穷尽（本机对 push2* 族按累积量/速率触发 RST），
///   属结构性缺口 ⇒ 长档可引用它作入场背景，**不得进方向加权**，回放时必须带标记。
fn qualified_reason(key: &str) -> &'static str {
    match key {
        "microstructurePartialSource" => "连板/封单/炸板有真字段，逐笔与 L2 撮合明细零通路",
        "qualityRealizationBeyondHorizon" => "盈利质量兑现跨季，本窗口只能部分反映",
        "supplyAsRiskNoteOnly" => "供给冲击在窗口内已被趋势吸收，只作风险提示",
        "trendFilterOnly" => "技术腿只作入场时机过滤，不得进方向加权",
        "macroPartialSeriesOnly" => "宏观仅 CPI/PPI/PMI/非制造业 PMI/GDP 五条真序列",
        "sectorRotationAsOfGap" => "行业轮动 live 有源、回放结构性不可得，只作背景不进方向",
        other => panic!("未登记的条件字段标记键: {other}"),
    }
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

    /// 档位的中文标签（注入文案与荐股面板用同一串，不再各处手抄）。
    pub fn label_zh(&self) -> &'static str {
        match self {
            Period::UltraShort => "超短",
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
            // 超短（2 日）：短窗动量 + 涨停板情绪广度（家数/封板率/炸板，P9-4 真字段）。
            // `microstructure`（封单/连板结构）只到板级、无撮合级 ⇒ 条件字段。
            Period::UltraShort => (
                &["momentumSignal", "breadthState"],
                &[("microstructure", "microstructurePartialSource")],
                "time_stop",
            ),
            Period::Short => (
                &[
                    "momentumSignal",
                    "breadthState",
                    "flowPersistence",
                    "trendStrength",
                    "supplyShock",
                ],
                &[],
                "time_stop+fixed_stop",
            ),
            Period::Mid => (
                &["flowPersistence", "trendStrength", "valuationBand", "expectationRevision"],
                &[
                    ("earningsQuality", "qualityRealizationBeyondHorizon"),
                    ("supplyShock", "supplyAsRiskNoteOnly"),
                ],
                "k_sigma_band",
            ),
            // 长档（90 日）：技术腿只作入场过滤；行业轮动 live 有源但回放不可得 ⇒ 条件字段。
            Period::Long => (
                &["valuationBand", "earningsQuality", "expectationRevision", "macroRegime"],
                &[
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

/// 档位后缀的分隔符。
///
/// 为什么不是单横线：base id 自身就含单横线（`a-market-analyst`、`a-hot-money`），
/// 用单横线拼接后「该剥到哪一段」没有唯一解；`--` 不在 base 域里出现 ⇒ `strip` 唯一可逆。
/// 配套的门反向锁「base 域里禁止出现 `--`」，否则这条前提会在未来某个新分析师身上静默失效。
pub const ANALYST_TIER_SEP: &str = "--";

/// 某档分析师子集里一个分析师的**实例节点 id**。
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
    debug_assert!(
        !base.contains(ANALYST_TIER_SEP),
        "base 域禁止包含分隔符 {ANALYST_TIER_SEP}（会让 analyst_base_of 的剥离不唯一）"
    );
    format!("{base}{ANALYST_TIER_SEP}{tier_snake}")
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
        assert_eq!(ultra.qualified, vec![("microstructure", "microstructurePartialSource")]);

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
        assert!(short.qualified.is_empty(), "短档没有「只作过滤」的因子");

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
                ("trendStrength", "trendFilterOnly"),
                ("macroRegime", "macroPartialSeriesOnly"),
                ("sectorRotation", "sectorRotationAsOfGap"),
            ],
            "长档：趋势只作入场时机过滤、宏观仅五条真序列、行业轮动回放结构性不可得"
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
                if *key == "trendFilterOnly" || *key == "supplyAsRiskNoteOnly" {
                    assert!(
                        !spec.participating.contains(f),
                        "{p:?} 的 {f} 标记为 {key}，却仍在进方向加权的清单里"
                    );
                }
            }
        }
        let long = Period::Long.verdict_spec();
        assert!(
            long.fields.contains(&"trendStrength")
                && !long.participating.contains(&"trendStrength"),
            "长档技术腿必须是「必填但只作过滤」"
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
        assert!(long.contains("notApplicableMomentum"), "长档要显式声明短窗动量不适用");
        assert!(!long.contains("microstructurePartialSource"), "串档：超短的条件键跑到长档文案里");

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
}
