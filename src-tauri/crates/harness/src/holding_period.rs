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

/// 九个逐档因子（`fields` / `not_applicable` / `qualified` 的划分域）。
/// 新增因子必须同时登记 [`na_key`]，否则 `verdict_spec` 会当场 panic 而不是静默漏判。
const VERDICT_FACTORS: &[&str] = &[
    "momentumSignal",
    "microstructure",
    "flowPersistence",
    "trendStrength",
    "valuationBand",
    "earningsQuality",
    "expectationRevision",
    "macroRegime",
    "supplyShock",
];

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
        "flowPersistence" => "notApplicableFlow",
        "trendStrength" => "notApplicableTrend",
        "valuationBand" => "notApplicableValuation",
        "earningsQuality" => "notApplicableQuality",
        "expectationRevision" => "notApplicableExpectation",
        "macroRegime" => "notApplicableMacro",
        "supplyShock" => "notApplicableSupply",
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
fn qualified_reason(key: &str) -> &'static str {
    match key {
        "microstructurePartialSource" => "连板/封单/炸板有真字段，逐笔与 L2 撮合明细零通路",
        "qualityRealizationBeyondHorizon" => "盈利质量兑现跨季，本窗口只能部分反映",
        "supplyAsRiskNoteOnly" => "供给冲击在窗口内已被趋势吸收，只作风险提示",
        "trendFilterOnly" => "技术腿只作入场时机过滤，不得进方向加权",
        "macroPartialSeriesOnly" => "宏观仅 CPI/PPI/PMI/非制造业 PMI/GDP 五条真序列",
        other => panic!("未登记的条件字段标记键: {other}"),
    }
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
            Period::UltraShort => (
                &["momentumSignal"],
                &[("microstructure", "microstructurePartialSource")],
                "time_stop",
            ),
            Period::Short => (
                &["momentumSignal", "flowPersistence", "trendStrength", "supplyShock"],
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
            Period::Long => (
                &["valuationBand", "earningsQuality", "expectationRevision", "macroRegime"],
                &[("trendStrength", "trendFilterOnly"), ("macroRegime", "macroPartialSeriesOnly")],
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

        HorizonVerdictSpec { fields, not_applicable, qualified: qualified.to_vec(), exit_rule }
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

    /// 逐档契约的结构性不变量：九因子不重不漏、标记键只修饰参与字段、共用字段齐、
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
            vec!["microstructure".to_string(), "momentumSignal".to_string()],
            "超短只算短窗动量与微观结构（含连板/封单，仍缺撮合级）"
        );
        assert_eq!(ultra.exit_rule, "time_stop");
        assert_eq!(ultra.not_applicable.len(), 7);
        assert_eq!(ultra.qualified, vec![("microstructure", "microstructurePartialSource")]);

        let short = Period::Short.verdict_spec();
        assert_eq!(
            golden(&short),
            vec![
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
                "trendStrength".to_string(),
                "valuationBand".to_string(),
            ]
        );
        assert_eq!(long.exit_rule, "target_and_falsified");
        assert_eq!(
            long.qualified,
            vec![("trendStrength", "trendFilterOnly"), ("macroRegime", "macroPartialSeriesOnly"),],
            "长档：趋势只作入场时机过滤、宏观仅五条真序列"
        );
        // 长档不得参与计算的三个短窗因子，必须走显式不适用而不是省略
        let na_long: Vec<&str> = long.not_applicable.iter().map(|(f, _)| *f).collect();
        assert_eq!(
            na_long,
            vec!["momentumSignal", "microstructure", "flowPersistence", "supplyShock"]
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
