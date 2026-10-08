//! 逐档分支的**腿信号推导**（P4′-b 的共用底层）。
//!
//! ## 为什么放在 Rust 而不是写在四份 Rhai 里
//!
//! R-11 要的是「四档走四条不同的算法分支」，而**同一因子的信号口径**在四档里必须是同一个函数
//! —— 否则就是四份手抄的阶梯表，正是 §二十四 那批缺陷（多处一致地错）的成因。
//! 划界因此很清楚：
//!
//! | 层 | 归属 | 内容 |
//! |---|---|---|
//! | 腿信号（本模块） | 单一实现 | 因子原始量 → `[-1, 1]` 有符号强度 |
//! | 腿集合 / 角色 / 配比 / 门 / 出场 | `evidence_weight::horizon_branch_specs` | 逐档不同 ⇒ 这才是「四条分支」 |
//! | 融合与仓位 | 四份 `portfolio-mgr-h-*.rhai` | 各档自己的组合方式 |
//!
//! ## 三条硬约束
//!
//! 1. **拿不到就是拿不到**：任一必需输入缺失 ⇒ 返回 `None`，**绝不返回 0.0**。
//!    0 在融合里是「中性证据」，None 才是「该腿本档不参与」—— 把缺席当 0 会把分母摊薄
//!    （`portfolio-mgr.rhai` 的「退出语义」章节记载的正是这个坑）。
//! 2. **锚必须内禀**：所有映射的中性点都取自量纲本身的定义（分位数的 50、PMI 的荣枯线 50、
//!    封板率的 0.5、Piotroski F-Score 的中点 4.5、催化剂标签的原生分值），
//!    **不引入新调参常量**。需要截断时截断点也写成「超出即满强度」的上界并说明理由，
//!    而不是「经验阈值」。
//! 3. **两处端口径迁移必须逐值等价**：`momentumSignal` 与 `eventCatalyst` 是从
//!    `portfolio-mgr.rhai` 的 f12 / f3 内联阶梯**原样搬来**的（黄金值测试锁住），
//!    搬运动作本身不许顺手「改好一点」。

use serde_json::Value;

/// 取一个数（整数也当浮点读；字符串形态的数**不猜**，直接判缺失）。
fn num(in_: &Value, key: &str) -> Option<f64> {
    in_.get(key).and_then(|v| v.as_f64())
}

fn text<'a>(in_: &'a Value, key: &str) -> Option<&'a str> {
    in_.get(key).and_then(|v| v.as_str())
}

/// 截断到 `[-1, 1]`。
fn cl(v: f64) -> f64 {
    v.clamp(-1.0, 1.0)
}

/// 因子腿信号：原始量 → `[-1, 1]`（正=看多该因子）。`None` = 输入不足 ⇒ 该腿不参与。
///
/// 因子名取值域 = `axagent_harness::Period::verdict_factors()` ∪ `{eventCatalyst}`
/// （由 `evidence_weight::VERDICT_FACTOR_SOURCES` 的通道决定哪些因子读哪些键）。
pub fn leg_signal(factor: &str, in_: &Value) -> Option<f64> {
    match factor {
        // ── 从 portfolio-mgr.rhai 的 f12 原样搬运（v19 趋势状态因子）──
        // 四态：零上多头 / 多头回调 / 零下金叉修复 / 零下空头，RSI 只做健康度调制。
        "momentumSignal" => {
            let dif = num(in_, "macdDif")?;
            let dea = num(in_, "macdDea")?;
            // 原阶梯有四态（零上多头 / 多头回调 / 零下金叉修复 / 零下空头），其中「多头回调」
            // 与「零下金叉修复」同为 +0.3 ⇒ 这里合成一个 `||` 分支（clippy::if_same_then_else）。
            // ⚠ NaN 不改变语义：NaN 时两个不等号都不命中 ⇒ 落到 -0.7，与原链逐情形一致
            //   （本仓踩过「为躲 lint 改成区间判断反而改语义」的坑，这次是纯 `||`，不动区间）。
            let base = if dif > 0.0 && dif > dea {
                0.7
            } else if dif > 0.0 || dif > dea {
                0.3
            } else {
                -0.7
            };
            // RSI 键名 #41（片 B）从 `rsi14` 改成中立的 `rsi`：v136 起按档尺度的评分节点
            // 喂进来的不再是「14 周期」那个命名槽，而是 `scaleMomentum.value`
            // （周期 = 该档窗口计划算出来的那一个）。旧键名在月线/季节点上是**假读数**：
            // 命名槽按数值认领，档位周期不落进 {6,12,14,24} 时它停在初值 50.0，
            // 而 50.0 恰好命中下面的「健康 +0.2」区间 ⇒ 每一档都被无谓地抬高 0.2。
            // 调制区间本身**不随尺度缩**（RSI 是 0-100 有界统计量，§九十六(2) 同一裁定）。
            let modulated = match num(in_, "rsi") {
                Some(rsi) if (50.0..=75.0).contains(&rsi) => base + 0.2,
                Some(rsi) if rsi > 75.0 => base - 0.2,
                _ => base,
            };
            Some(cl(modulated))
        },
        // 该档评分节点（t-scoring-hour|week|month|quarter）的 totalScore ∈ [0,100]，
        // 50 = 评分体系的中性中枢 ⇒ 线性映射，无新常量。
        "trendStrength" => num(in_, "totalScore").map(|ts| cl((ts - 50.0) / 50.0)),
        // ── 从 portfolio-mgr.rhai 的 f3 催化剂阶梯原样搬运 ──
        // 判定次序与源脚本一致：先负后正（`L-1` 含 `-1`，反序会串档）。
        "eventCatalyst" => {
            let level = text(in_, "catalystLevel")?;
            if level.is_empty() || level == "无" {
                return None;
            }
            let hits = |keys: &[&str]| keys.iter().any(|k| level.contains(k));
            if hits(&["L-3", "退市", "造假"]) {
                Some(-0.8)
            } else if hits(&["L-2", "业绩暴雷"]) {
                Some(-0.5)
            } else if hits(&["L-1", "普通利空"]) {
                Some(-0.2)
            } else if hits(&["L3", "估值体系级"]) {
                Some(0.8)
            } else if hits(&["L2", "业绩拐点级"]) {
                Some(0.5)
            } else if hits(&["L1", "普通消息"]) {
                Some(0.2)
            } else {
                // 标签不在六级体系内 ⇒ 判缺失，不按 0 参与（模型漏吐字段时的形态）
                None
            }
        },
        // PE 历史分位：0 = 近 5 年最便宜 / 100 = 最贵 ⇒ 便宜为正、50 分位为中性，
        // 锚点来自分位数定义本身（`t-valuation-band` 已把样本不足判成 null）。
        "valuationBand" => num(in_, "pePercentile").map(|p| cl((50.0 - p) / 50.0)),
        // Piotroski F-Score ∈ [0,9]：中点 4.5 = 无信息，线性到两端。
        "earningsQuality" => num(in_, "fScore").map(|f| cl((f - 4.5) / 4.5)),
        // 一致预期相对最近实际值的修正幅度。实际值 ≤ 0 时比值无经济含义（亏损股
        // 「预期改善 2 倍」可能是亏损扩大）⇒ 判缺失，不做绝对值技巧。
        "expectationRevision" => {
            let cons = num(in_, "consensusEps")?;
            let act = num(in_, "latestEps")?;
            if act <= 0.0 {
                return None;
            }
            Some(cl((cons - act) / act))
        },
        // 宏观只取 **PMI**：它是本仓五条真序列里唯一自带荣枯线（50）的量。
        // CPI/PPI/GDP 不参与方向 —— 同比口径不同，混进同一个带符号强度就是量纲错误。
        // 幅度用 ±10 个点截断（PMI 历史波动带），这是**上界**而不是阈值。
        "macroRegime" => num(in_, "pmi").map(|p| cl((p - 50.0) / 10.0)),
        // 未来解禁规模占流通市值比 r：r = 0 无供给冲击；r ≥ 10% 已是「一次放出十分之一盘」
        // ⇒ 10% 是满强度截断点（不是选参），其间线性。
        "supplyShock" => num(in_, "lockupFloatRatio").map(|r| -cl(r / 0.10)),
        // 行业相对强度分位：50 = 行业中位 ⇒ 强于中位为正。长档只作背景（role=partial）。
        "sectorRotation" => num(in_, "industryRankPercentile").map(|p| cl((p - 50.0) / 50.0)),
        // 情绪广度：封板率本身就是概率（封住 ÷ 触板），0.5 = 触板一半封住 = 中性。
        // 涨停家数**不进信号**（量级随市场扩容漂移），只做 `breadth_required` 门的输入。
        "breadthState" => num(in_, "sealRate").map(|r| cl((r - 0.5) / 0.5)),
        // 个股封单结构：封住且零炸板 = 满强度正；每次炸板扣 1/3（当日反复开板 3 次即打满负），
        // 3 次之后不再累加 —— 截断上界，不是阈值。未进涨停池由调用方判「缺席」而不是 0。
        "microstructure" => {
            if in_.get("inLimitUpPool").and_then(|v| v.as_bool()) != Some(true) {
                return None;
            }
            let breaks = num(in_, "breakCount").unwrap_or(0.0);
            Some(cl(1.0 - breaks / 3.0))
        },
        // 资金持续性只有 LLM 分析师能给（本仓无 ≥5 日资金流序列节点）⇒
        // 契约要求 `a-hot-money` 输出 `[-1,1]` 的有符号强度；越界即判不守约（None），
        // 不再套一层缩放把它拉回值域 —— 那等于替模型的自相矛盾背书。
        "flowPersistence" => match num(in_, "flowPersistence") {
            Some(v) if (-1.0..=1.0).contains(&v) => Some(v),
            _ => None,
        },
        other => {
            // 未登记的名字 = 分支表与信号实现漂移（例如因子改名只改了一处）。
            // 必须点名：静默 None 会让那条腿「永远缺席」，而面板上只看到「证据不足」。
            tracing::warn!("[leg_signal] 未登记的因子名 '{other}' ⇒ 该腿按缺席处理");
            None
        },
    }
}

/// 本模块认识的因子名（供门与分支表交叉核对；与 `VERDICT_FACTOR_SOURCES` 同域）。
pub const SUPPORTED_FACTORS: &[&str] = &[
    "momentumSignal",
    "trendStrength",
    "eventCatalyst",
    "valuationBand",
    "earningsQuality",
    "expectationRevision",
    "macroRegime",
    "supplyShock",
    "sectorRotation",
    "breadthState",
    "microstructure",
    "flowPersistence",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 数值断言一律带容差：本模块的信号是「加减 + 除法 + 截断」，直接 `assert_eq!`
    /// 会把 `0.7 + 0.2` 判成 ≠ 0.9（f64 表示误差），那是**假红**；反过来用 `approx` 也不会
    /// 放过真错（下面的期望值都是逐情形从原阶梯算出来的，误差量级 1e-1 才能蒙混）。
    fn assert_sig(name: &str, got: Option<f64>, want: f64) {
        assert!(got.is_some_and(|v| (v - want).abs() < 1e-9), "{name}：实得 {got:?}，期望 {want}");
    }

    #[test]
    fn momentum_ladder_matches_the_rhai_it_was_ported_from() {
        // 黄金值取自 portfolio-mgr.rhai 的 f12 分支（搬运前逐情形实测），改动即红
        let cases: [(&str, f64, f64, f64, f64); 5] = [
            ("零上多头且健康", 1.2, 0.6, 60.0, 0.9),
            ("零上多头且过热", 1.2, 0.6, 80.0, 0.5),
            ("多头回调", 1.2, 1.6, 60.0, 0.5),
            ("零下金叉修复", -1.2, -1.6, 55.0, 0.5),
            ("零下空头", -1.2, -0.6, 30.0, -0.7),
        ];
        for (name, dif, dea, rsi, want) in cases {
            assert_sig(
                name,
                leg_signal(
                    "momentumSignal",
                    &json!({ "macdDif": dif, "macdDea": dea, "rsi": rsi }),
                ),
                want,
            );
        }
        // 缺 RSI 只做 base，不判缺失（RSI 是调制项不是必需项）
        assert_sig(
            "缺 RSI",
            leg_signal("momentumSignal", &json!({ "macdDif": 1.2, "macdDea": 0.6 })),
            0.7,
        );
        // 缺 MACD ⇒ 缺席而不是 0
        assert_eq!(leg_signal("momentumSignal", &json!({ "rsi": 60.0 })), None);
    }

    #[test]
    fn catalyst_ladder_is_order_sensitive_and_rejects_unknown_labels() {
        let cases = [
            ("L-3退市风险", -0.8),
            ("L-2业绩暴雷", -0.5),
            ("L-1普通利空", -0.2),
            ("L3估值体系级", 0.8),
            ("L2业绩拐点级", 0.5),
            ("L1普通消息", 0.2),
        ];
        for (label, want) in cases {
            assert_sig(
                label,
                leg_signal("eventCatalyst", &json!({ "catalystLevel": label })),
                want,
            );
        }
        // 「无」与空串 = 模型明确说没有催化剂 ⇒ 该腿不参与，不是中性 0
        assert_eq!(leg_signal("eventCatalyst", &json!({ "catalystLevel": "无" })), None);
        assert_eq!(leg_signal("eventCatalyst", &json!({ "catalystLevel": "" })), None);
        // 越界标签（模型自造等级）⇒ 判缺失，不套回值域
        assert_eq!(leg_signal("eventCatalyst", &json!({ "catalystLevel": "L9重大利好" })), None);
    }

    #[test]
    fn anchors_are_intrinsic_and_out_of_range_inputs_are_absence() {
        // 分位数 / F-Score / PMI / 封板率的中性点来自量纲定义本身
        assert_sig("评分中枢", leg_signal("trendStrength", &json!({ "totalScore": 50.0 })), 0.0);
        assert_sig("PE 中位", leg_signal("valuationBand", &json!({ "pePercentile": 50.0 })), 0.0);
        assert_sig("F-Score 中点", leg_signal("earningsQuality", &json!({ "fScore": 4.5 })), 0.0);
        assert_sig("PMI 荣枯线", leg_signal("macroRegime", &json!({ "pmi": 50.0 })), 0.0);
        assert_sig("封板率一半", leg_signal("breadthState", &json!({ "sealRate": 0.5 })), 0.0);
        // 便宜（低分位）为正、解禁越大越负
        assert_sig("低分位", leg_signal("valuationBand", &json!({ "pePercentile": 10.0 })), 0.8);
        assert_sig(
            "半成解禁",
            leg_signal("supplyShock", &json!({ "lockupFloatRatio": 0.05 })),
            -0.5,
        );
        // 10% 是满强度截断点，20% 不继续累加
        assert_sig(
            "超界截断",
            leg_signal("supplyShock", &json!({ "lockupFloatRatio": 0.20 })),
            -1.0,
        );
        // 亏损股的一致预期修正无经济含义 ⇒ 判缺失（不做 |actual| 技巧）
        assert_eq!(
            leg_signal("expectationRevision", &json!({ "consensusEps": 2.0, "latestEps": -1.0 })),
            None
        );
        assert_sig(
            "预期上修五成",
            leg_signal("expectationRevision", &json!({ "consensusEps": 3.0, "latestEps": 2.0 })),
            0.5,
        );
        // 分析师自报越界 ⇒ 不替它缩回值域
        assert_eq!(leg_signal("flowPersistence", &json!({ "flowPersistence": 1.8 })), None);
        assert_sig(
            "资金持续性",
            leg_signal("flowPersistence", &json!({ "flowPersistence": 0.4 })),
            0.4,
        );
        // 未进涨停池 ⇒ 缺席；进池后按炸板次数扣分，3 次打满、之后不累加
        assert_eq!(leg_signal("microstructure", &json!({ "breakCount": 0 })), None);
        assert_sig(
            "一次没炸",
            leg_signal("microstructure", &json!({ "inLimitUpPool": true, "breakCount": 0 })),
            1.0,
        );
        assert_sig(
            "炸满三次",
            leg_signal("microstructure", &json!({ "inLimitUpPool": true, "breakCount": 3 })),
            0.0,
        );
        assert_sig(
            "越界仍截断",
            leg_signal("microstructure", &json!({ "inLimitUpPool": true, "breakCount": 9 })),
            -1.0,
        );
    }

    /// 扫描面自证：本模块认识的因子必须与分支表的来源表**逐名相同**。
    /// 两边漂移的后果不对称（表里有、这里没有 ⇒ 腿静默拿不到数），所以由测试钉死。
    #[test]
    fn supported_factors_equal_the_branch_source_table() {
        let crate_factors = crate::evidence_weight::VERDICT_FACTOR_SOURCES;
        assert_eq!(
            SUPPORTED_FACTORS.len(),
            crate_factors.len(),
            "因子来源表与本模块的信号实现条数不符（新增因子要两处同做）"
        );
        for (f, _, _) in crate_factors {
            assert!(SUPPORTED_FACTORS.contains(f), "来源表登记了 {f}，本模块没有信号实现");
        }
        for f in SUPPORTED_FACTORS {
            assert!(
                crate_factors.iter().any(|(cf, _, _)| cf == f),
                "本模块实现了 {f} 但来源表没有"
            );
            let inputs = json!({
                "macdDif": 1.0, "macdDea": 0.5, "rsi": 60.0, "totalScore": 60.0,
                "catalystLevel": "L2业绩拐点级", "pePercentile": 20.0, "fScore": 7.0,
                "consensusEps": 3.0, "latestEps": 2.0, "pmi": 51.0,
                "lockupFloatRatio": 0.02, "industryRankPercentile": 70.0, "sealRate": 0.7,
                "inLimitUpPool": true, "breakCount": 1.0, "flowPersistence": 0.3
            });
            assert!(
                leg_signal(f, &inputs).is_some(),
                "{f} 在「全部输入齐备」下仍拿不到信号 ⇒ 键名或分支写错"
            );
        }
    }
}
