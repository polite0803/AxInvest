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
}
