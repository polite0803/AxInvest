/// 风险档位（与 portfolio_monitor::compute_risk_level 对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskTier {
    /// 低风险
    Low,
    /// 中风险
    Medium,
    /// 中高风险
    MediumHigh,
    /// 高风险
    High,
    /// 极高风险
    Extreme,
}

impl RiskTier {
    /// 该风险档位下的单股仓位上限（%）
    ///
    /// ⚠️ 语义约束：本表**只能收紧**，不能放宽。
    /// 唯一消费方式是 `min(PositionLimits::max_single_stock_pct, tier_cap)`（见
    /// `check_new_position_with_risk` 与 `portfolio_formula::portfolio_risk_gate`），
    /// 因此任何 > `PositionLimits::default().max_single_stock_pct`（20%）的档位值都
    /// 是**恒等死分支**——永远不会生效。
    ///
    /// 2026-09-13 修复（两处）：
    ///   ① `Extreme` 由 0.0 改为 10.0，与另两处「极高风险仓位上限」口径对齐
    ///      （模板参数 `pos_cap_extreme` 默认 10、`portfolio_formula::apply_risk_cap`
    ///      的 10）。旧值 0.0 会把「减持 8.85%」直接清零成「清仓」——
    ///      极高风险的真实语义是「禁开新仓」（见 `forbid_new_position`）＋限仓，
    ///      而不是强制清仓；同一个语义三套值（0 / 10 / 10）属铁律 5「同量被两套判定
    ///      用不同口径消费」的变体。
    ///   ② `High` 由 35.0 改为 20.0：旧值 35 > 20 恒被 `min` 覆盖，是永不生效的死分支
    ///      （原注释「高风险 35% 上限」在机制上无法成立）。
    pub fn max_single_stock_pct(self) -> f64 {
        match self {
            RiskTier::Low => 20.0,
            RiskTier::Medium => 20.0,
            RiskTier::MediumHigh => 20.0,
            // 2026-09-13: 旧值 35.0 被 min(20) 恒等覆盖（死分支），回归 20.0（= 全局默认）
            RiskTier::High => 20.0,
            // 2026-09-13: 旧值 0.0 会把「减持」清零成「清仓」；对齐 pos_cap_extreme=10
            RiskTier::Extreme => 10.0,
        }
    }

    /// 是否禁止开新仓
    pub fn forbid_new_position(self) -> bool {
        matches!(self, RiskTier::Extreme)
    }

    /// 修复 H7: 统一风险分级归一化函数
    /// 将三套口径的字符串统一映射到 RiskTier:
    /// - decision.rs: 低/中/高（3 级）
    /// - portfolio_monitor.rs: 低/中/中高/高/无持仓（5 级）
    /// - backtest_strategy.rs: 极高/高/低（3 级混用）
    pub fn from_risk_str(s: &str) -> Self {
        let s = s.trim();
        // 2026-09-21: 补 `extreme` / `critical` 英文别名 —— 同级 high/medium/low 都已配
        //   ASCII 别名（`eq_ignore_ascii_case`），唯独最高档只认中文「极高」，是漏配。
        //   后果：英文值域 `EXTREME` 会静默落到 `Medium` 兜底 ⇒ **极高风险的
        //   「禁止开新仓」失效**（`forbid_new_position()` 只对 Extreme 为真），
        //   而同一档在中文值域下是生效的 —— 同一语义因书写形态不同而结论不同（fail-open）。
        //   消费端：`portfolio_formula::portfolio_risk_gate` 的 R-200 否决、
        //   `portfolio_formula::apply_risk_veto` 的「极高风险禁止持仓」。
        if s.contains("极高")
            || s.eq_ignore_ascii_case("extreme")
            || s.eq_ignore_ascii_case("critical")
        {
            RiskTier::Extreme
        } else if s.contains("中高") {
            RiskTier::MediumHigh
        } else if s.contains("高") || s.eq_ignore_ascii_case("high") {
            RiskTier::High
        } else if s.contains("中") || s.eq_ignore_ascii_case("medium") {
            RiskTier::Medium
        } else if s.contains("低") || s.eq_ignore_ascii_case("low") {
            RiskTier::Low
        } else {
            // "无持仓" 或未知 → 默认中风险（保守不激进）
            RiskTier::Medium
        }
    }

    /// 转中文显示
    pub fn to_cn(self) -> &'static str {
        match self {
            RiskTier::Low => "低风险",
            RiskTier::Medium => "中风险",
            RiskTier::MediumHigh => "中高风险",
            RiskTier::High => "高风险",
            RiskTier::Extreme => "极高风险",
        }
    }
}

/// 全局仓位限制配置
///
/// `PartialEq` 是给 `panel_landing_tests` 用的：面板三条的「回落」与「拒绝覆盖」都要和
/// `Default` 整体比对（逐字段比太容易漏掉一条，漏掉的那条正好是「接线没落地」的形态）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionLimits {
    pub max_single_stock_pct: f64,
    pub max_total_positions: u32,
    pub max_sector_exposure_pct: f64,
}

/// 仓位三条的**权威默认值**（20% / 10 只 / 40%）。
///
/// ⚠ 这里同时是「面板取不到值时的回落值」：`from_panel_vars` 从本 `Default` 出发再覆盖，
///   所以「接线」不改现网任何一个数 —— 面板 `pos_max_*` 三条的默认值与这里逐字相等，
///   对账由 `scripts/check-panel-var-landing.mjs` 的 P1 负责（两侧任一处改数即红）。
impl Default for PositionLimits {
    fn default() -> Self {
        Self { max_single_stock_pct: 20.0, max_total_positions: 10, max_sector_exposure_pct: 40.0 }
    }
}

/// 空头预测阈值：targetPrice < current × 0.85 视为强烈看空，应强制卖出
pub const BEARISH_TARGET_PRICE_RATIO: f64 = 0.85;

/// 面板仓位三条：`(变量名, 可用判据, 落点)` **同表**。
///
/// 为什么键、判据、落点必须在一张表里（照 `astock-data::mcp_tools::INDICATOR_BAR_ARGS` 的形）：
/// 加第四条却忘了写落点 ⇒ 那个参数被静默忽略 ⇒ 又是一次「面板改了没反应」。`fn` 指针
/// 让「表里有」与「落得了地」成为同一件事，生产路径不需要 `unreachable!`。
///
/// 判据不是装饰：`max_total_positions` 会被 `as u32` 截断，非整数（`10.5`）会把 10 只
/// 变成 10 只却**报出 10.5**；`*_pct` 越过 (0, 100] 就不是「占比」。这类值一律**拒绝覆盖 +
/// warn**（同 v137 那批的「显式失败而不是静默按默认继续」；这里没有 `Result` 通道，
/// 于是失败表达成「不覆盖 + 留痕」）。
type PositionLimitVar = (&'static str, fn(f64) -> bool, fn(&mut PositionLimits, f64));

const POSITION_LIMIT_VARS: [PositionLimitVar; 3] = [
    (
        "pos_max_single_pct",
        |v| v.is_finite() && v > 0.0 && v <= 100.0,
        |l, v| l.max_single_stock_pct = v,
    ),
    (
        "pos_max_total",
        // 闭区间用 `contains`（`clippy::manual_range_contains` 在 CI 的 `-D warnings` 下必红）。
        // 浮点套用这条建议**不是纯等价改写** —— `RangeInclusive::contains` 对 NaN 恒 `false`，
        // 所以必须单独验：本式对 NaN / ±INF 先被 `is_finite()` 短路成 `false`（拒绝覆盖 ⇒ 回落默认），
        // 与改写前的 `v >= 1.0 && v <= 1_000.0`（NaN 两个不等号都不命中 ⇒ 同样 `false`）逐情形一致。
        // 锁住它的测试是 `invalid_panel_position_values_are_refused`（10.5 / 0 两例）。
        |v| v.is_finite() && v.fract() == 0.0 && (1.0..=1_000.0).contains(&v),
        |l, v| l.max_total_positions = u32::try_from(v as i64).unwrap_or(10),
    ),
    (
        "pos_max_sector_pct",
        |v| v.is_finite() && v > 0.0 && v <= 100.0,
        |l, v| l.max_sector_exposure_pct = v,
    ),
];

impl PositionLimits {
    /// 按**变量表**构造（纯函数版，落地面）。
    ///
    /// 起点是 `Self::default()`，只有「面板给了可用值」才覆盖 ⇒ 缺失/非法一律回落到与今天
    /// 逐字相同的那组数。**刻意不做**「取不到就返回 0 / 报错」：仓位上限变 0 等于禁止一切
    /// 建仓，报错则让风控门整段 fail-safe 退回原始决策 —— 两者都是改现网数值。
    pub fn from_panel_vars(vars: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        let mut limits = Self::default();
        for (key, valid, set) in POSITION_LIMIT_VARS {
            let Some(value) = axagent_harness::panel_variables::numeric_in(vars, key) else {
                continue;
            };
            if !valid(value) {
                tracing::warn!(
                    "[position_limits] 面板 {key} = {value} 不是可用的仓位限制（占比需落在 (0,100]、只数需为正整数）⇒ 拒绝覆盖，按默认值继续"
                );
                continue;
            }
            set(&mut limits, value);
        }
        limits
    }

    /// 生产路径的仓位限制 = `Default` + 面板 `pos_max_*` 覆盖（读进程内变量表快照）。
    ///
    /// ⚠ 与 `RiskTier::max_single_stock_pct` 的次序：**档位表只能收紧**。实际单股上限是
    ///   `min(面板 pos_max_single_pct, tier_cap)`（见 `portfolio_risk_gate`），而 tier_cap 表
    ///   最高就是 20 ⇒ 把面板值调到 20 以上**不会放宽**仓位（那 20 是档位表的天花板，
    ///   不是本类型的手误）。往小调（如 15）则真实生效。
    pub fn panel_effective() -> Self {
        Self::from_panel_vars(&axagent_harness::panel_variables::panel_variables())
    }

    /// 检查新增仓位是否合规
    ///
    /// 修复 P2-9: 当 `total_portfolio_value == 0` 时原代码把 new_pct 置为 0，
    /// 静默绕过单股仓位与行业暴露上限检查。这在"空仓首次建仓"场景下形成风控漏洞
    /// —— 任意金额的买单都会"合规"。改为明确拒绝，迫使 caller 传入含现金的
    /// 组合总价值（持仓市值 + 可用现金），让仓位上限检查真实生效。
    pub fn check_new_position(
        &self,
        new_position_value: f64,
        total_portfolio_value: f64,
        current_positions: usize,
        new_sector: Option<&str>,
        current_sector_exposures: &[(String, f64)],
    ) -> Result<(), String> {
        if total_portfolio_value <= 0.0 {
            return Err(format!(
                "组合总价值为 {}，无法计算仓位比例（请传入 持仓市值+可用现金 作为分母）",
                total_portfolio_value
            ));
        }

        if let Some(sector) = new_sector {
            let current_sector_pct = current_sector_exposures
                .iter()
                .filter(|(s, _)| s == sector)
                .map(|(_, pct)| *pct)
                .next()
                .unwrap_or(0.0);
            let new_pct = (new_position_value / total_portfolio_value) * 100.0;
            if current_sector_pct + new_pct > self.max_sector_exposure_pct {
                return Err(format!(
                    "行业{}暴露{:.1}%将超过上限{:.0}%",
                    sector,
                    current_sector_pct + new_pct,
                    self.max_sector_exposure_pct
                ));
            }
        }

        if current_positions >= self.max_total_positions as usize {
            return Err(format!(
                "持仓数量已达上限 ({}只)，请先减仓再新增",
                self.max_total_positions
            ));
        }

        let new_pct = (new_position_value / total_portfolio_value) * 100.0;

        if new_pct > self.max_single_stock_pct {
            return Err(format!(
                "单股仓位 {:.1}% 超过上限 {:.0}%，请减少买入数量",
                new_pct, self.max_single_stock_pct
            ));
        }

        Ok(())
    }

    /// 修复 H5: 风险档位感知的仓位检查
    /// - 极高风险档位直接拒绝开新仓（强制观望）
    /// - 其余档位取 `min(max_single_stock_pct, tier_cap)` —— 注意这里只能**收紧**：
    ///   档位上限大于全局上限时不会放宽（2026-09-13 更正旧注释「高风险 35% 覆盖
    ///   max_single_stock_pct」，`min` 语义下不可能向上覆盖）
    pub fn check_new_position_with_risk(
        &self,
        new_position_value: f64,
        total_portfolio_value: f64,
        current_positions: usize,
        new_sector: Option<&str>,
        current_sector_exposures: &[(String, f64)],
        risk_tier: RiskTier,
    ) -> Result<(), String> {
        if risk_tier.forbid_new_position() {
            return Err("风险档位为极高，强制观望，禁止开新仓".to_string());
        }
        // 临时覆盖单股上限为风险档位上限（取较小者）
        let original = self.max_single_stock_pct;
        let tier_cap = risk_tier.max_single_stock_pct();
        let effective_cap = original.min(tier_cap);
        let capped = PositionLimits {
            max_single_stock_pct: effective_cap,
            max_total_positions: self.max_total_positions,
            max_sector_exposure_pct: self.max_sector_exposure_pct,
        };
        capped.check_new_position(
            new_position_value,
            total_portfolio_value,
            current_positions,
            new_sector,
            current_sector_exposures,
        )
    }

    /// 修复 H5: 空头预测强制卖出检查
    /// 当 target_price < current_price × 0.85 时，应强制卖出该持仓
    pub fn check_bearish_force_sell(
        current_price: f64,
        target_price: Option<f64>,
    ) -> Result<bool, String> {
        match target_price {
            Some(tp) if tp > 0.0 && current_price > 0.0 => {
                let ratio = tp / current_price;
                if ratio < BEARISH_TARGET_PRICE_RATIO {
                    // 强制卖出信号
                    Ok(true)
                } else {
                    Ok(false)
                }
            },
            _ => Ok(false),
        }
    }
}

/// 面板 `pos_max_*` 三条落地的行为锁（2026-10-08 A 批）。
///
/// 三条各锁一种复发形态：① 默认值对账（接线本身零数值变化）② 变量缺失 ⇒ 回落同一组默认
/// （**不许**变成 0 或报错，那等于禁止建仓 / 让风控门整段 fail-safe）③ 变量被改 ⇒ 新值真进到
/// `check_new_position` 的三条判据里。数值断言按被测判据现算，不手敲结论。
#[cfg(test)]
mod panel_landing_tests {
    use super::*;
    use axagent_harness::panel_variables::variables_map;
    use std::collections::HashMap;

    /// 与 wiring 装入快照时逐字同形（`[{name, value}]`），避免测试自己造出第二种表形态。
    fn vars(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        let entries: Vec<serde_json::Value> = pairs
            .iter()
            .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
            .collect();
        variables_map(&serde_json::Value::Array(entries))
    }

    /// 面板三条的默认值 == `Default` 的那组数 ⇒ 「接线」不改现网任何一条风控判据。
    ///
    /// 左边三个数字手抄自设置面板 `b("pos_max_single_pct", 20, …)` 等与
    /// `seed_variables.rs` 的 `json!(20.0)` / `json!(10)` / `json!(40.0)`；
    /// 门 `check-panel-var-landing.mjs` 的 P1 也按同样的两侧字面量对账。
    #[test]
    fn panel_default_position_limits_equal_todays_constants() {
        let d = PositionLimits::default();
        assert_eq!(d.max_single_stock_pct, 20.0, "面板 pos_max_single_pct 默认 20");
        assert_eq!(d.max_total_positions, 10, "面板 pos_max_total 默认 10");
        assert_eq!(d.max_sector_exposure_pct, 40.0, "面板 pos_max_sector_pct 默认 40");
        let overlaid = PositionLimits::from_panel_vars(&vars(&[
            ("pos_max_single_pct", serde_json::json!(20.0)),
            ("pos_max_total", serde_json::json!(10)),
            ("pos_max_sector_pct", serde_json::json!(40.0)),
        ]));
        assert_eq!(overlaid.max_single_stock_pct, d.max_single_stock_pct);
        assert_eq!(overlaid.max_total_positions, d.max_total_positions);
        assert_eq!(overlaid.max_sector_exposure_pct, d.max_sector_exposure_pct);
    }

    /// 变量缺失 ⇒ 回落今天的默认，并且风控判据仍按那组数走（不是 0、不是报错）。
    #[test]
    fn missing_panel_vars_fall_back_to_todays_limits() {
        let limits = PositionLimits::from_panel_vars(&vars(&[("unrelated", serde_json::json!(7))]));
        assert_eq!(limits, PositionLimits::default(), "缺失不得把上限改成 0 或留空");
        // 按判据现算：25% 的单股建仓 > 20 ⇒ 拒绝；15% ⇒ 通过。
        let total = 1000.0;
        assert!(
            limits.check_new_position(250.0, total, 0, None, &[]).is_err(),
            "默认 20% 上限下 25% 建仓必须被拒（这是今天的现网行为）"
        );
        assert!(limits.check_new_position(150.0, total, 0, None, &[]).is_ok());
    }

    /// 面板改值 ⇒ 三条判据各自真的跟着动（逐条点名，防「表里有键、落点没接」）。
    #[test]
    fn panel_values_reach_each_position_check() {
        let total = 1000.0;
        // ① 单股上限 20 ⇒ 10：15% 建仓原本通过，收紧后必须拒绝。
        let tightened = PositionLimits::from_panel_vars(&vars(&[(
            "pos_max_single_pct",
            serde_json::json!(10.0),
        )]));
        assert!(
            tightened.check_new_position(150.0, total, 0, None, &[]).is_err(),
            "面板 10% 下 15% 应拒"
        );
        assert!(PositionLimits::default().check_new_position(150.0, total, 0, None, &[]).is_ok());

        // ② 只数上限 10 ⇒ 3：`current_positions >= max` ⇒ 第 4 只（index 3）被拒。
        let few =
            PositionLimits::from_panel_vars(&vars(&[("pos_max_total", serde_json::json!(3))]));
        assert_eq!(few.max_total_positions, 3);
        assert!(
            few.check_new_position(10.0, total, 3, None, &[]).is_err(),
            "3 只上限下第 4 只应拒"
        );
        assert!(few.check_new_position(10.0, total, 2, None, &[]).is_ok());

        // ③ 行业上限 40 ⇒ 20：现有暴露 25% + 新 5% = 30%，默认通过、收紧后拒绝。
        let exposures = vec![("白酒".to_string(), 25.0)];
        let sector = PositionLimits::from_panel_vars(&vars(&[(
            "pos_max_sector_pct",
            serde_json::json!(20.0),
        )]));
        assert!(
            sector.check_new_position(50.0, total, 0, Some("白酒"), &exposures).is_err(),
            "面板 20% 行业上限下 25%+5% 应拒"
        );
        assert!(
            PositionLimits::default()
                .check_new_position(50.0, total, 0, Some("白酒"), &exposures)
                .is_ok(),
            "默认 40% 下同一笔应通过"
        );
    }

    /// 非法面板值 ⇒ 拒绝覆盖而不是把风控上限变成 0 / 把只数截断。
    #[test]
    fn invalid_panel_position_values_are_refused() {
        for (key, bad) in [
            ("pos_max_single_pct", serde_json::json!(0.0)),
            ("pos_max_single_pct", serde_json::json!(-5.0)),
            ("pos_max_single_pct", serde_json::json!(120.0)),
            ("pos_max_single_pct", serde_json::json!("两成")),
            ("pos_max_total", serde_json::json!(10.5)),
            ("pos_max_total", serde_json::json!(0)),
            ("pos_max_sector_pct", serde_json::json!(null)),
        ] {
            let got = PositionLimits::from_panel_vars(&vars(&[(key, bad.clone())]));
            assert_eq!(
                got,
                PositionLimits::default(),
                "{key}={bad} 不是可用限制 ⇒ 必须整条回落默认"
            );
        }
    }
}
