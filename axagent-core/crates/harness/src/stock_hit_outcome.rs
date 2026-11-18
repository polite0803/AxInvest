// SPDX-License-Identifier: AGPL-3.0-only
//! 决策命中结果判定（纯函数，无 IO、无时钟）。
//!
//! 判定规则被插件侧闭环与宿主回填链共用，权威定义只在此处；`analysis-engine`
//! 的 `hit_rate_backtest` 与 `axagent-stock-core` 均保留路径级 re-export。

/// 决策命中结果判定（纯函数，无 IO、无时钟）。
///
/// 为什么在这里而不是 `analysis-engine`：判定规则被**插件侧闭环**与**宿主回填链**共用，
/// 而 `analysis-engine` 是 implementor（拖 `dao` / `entities`）⇒ 放进 foundation 才能让
/// 插件进程只链接纯逻辑。权威定义只此一份，`analysis-engine` 的 `hit_rate_backtest`
/// 以 `pub use` 回指（消费路径
/// `axagent_analysis_engine::hit_rate_backtest::compute_hit_outcome` 保持不变）。
///
/// ⚠ 用内联模块而不新增文件：`scripts/capability-register.json` 的 `k1RsFileCount`
/// 是 hard 锚点（当前 2047），本批只做「两进两出」的文件数净 0 搬家。
/// 计算单条决策的 hit_outcome（基于 T+N 实际价格）
///
/// ## ⚠ 入参必须是**净**收益（已扣交易成本）
///
/// 2026-10-25（G4）：本函数原文档写 `final_return_pct > 0% → "hit"`，而调用方传的是
/// **毛**收益 ⇒ **+0.01% 也算命中**，实盘却是亏成本。现约定入参为净收益，
/// 调用方用 `PriceMetrics::net_return_pct_default` / `PickValidation::net_return_pct`。
/// 判定阈值本身不变（`> 0` 就是「实盘赚钱」），变的是**零点的基准**。
///
/// ## 判定规则（**顺序即优先级**，与函数体逐行一致）
/// - **buy**:
///   - 触及 stop_loss → "miss"（止损优先于任何盈利判定）
///   - 触及 target 或 净收益 > 0% → "hit"
///   - 净收益 <= -5% → "false_hit"（"看多却跌" 是最严重的误判）
///   - 其余（-5% ~ 0%）→ "partial"
/// - **sell**:
///   - 净收益 < 0% → "hit"（看空看对了）
///   - 净收益 > 5% → "false_hit"
///   - 其他 → "partial"
/// - **hold**: 窄幅震荡判断
///   - |净收益| <= 1% → "hit"（震荡判断正确，价格确实未明显变动）
///   - |净收益| > 5% → "false_hit"（判断震荡但市场走出趋势）
///   - 其他 → "partial"
/// - **数据不足**: 任意 None → "insufficient"
pub fn compute_hit_outcome(
    action: &str,
    net_return_pct: Option<f64>,
    hit_stop_loss: Option<i32>,
    hit_target: Option<i32>,
) -> Option<String> {
    let Some(ret) = net_return_pct else {
        return Some("insufficient".to_string());
    };
    match action {
        "buy" => {
            if hit_stop_loss == Some(1) {
                Some("miss".to_string())
            } else if hit_target == Some(1) || ret > 0.0 {
                Some("hit".to_string())
            } else if ret < -5.0 {
                Some("false_hit".to_string())
            } else {
                Some("partial".to_string())
            }
        },
        "sell" => {
            if ret < 0.0 {
                Some("hit".to_string())
            } else if ret > 5.0 {
                Some("false_hit".to_string())
            } else {
                Some("partial".to_string())
            }
        },
        // 修复 P1: hold 原一律 "partial"，未反映震荡判断的正确性。
        // 窄幅震荡（|ret| ≤ 1%）→ hit；明显趋势（|ret| > 5%）→ false_hit。
        "hold" => {
            let abs_ret = ret.abs();
            if abs_ret <= 1.0 {
                Some("hit".to_string())
            } else if abs_ret > 5.0 {
                Some("false_hit".to_string())
            } else {
                Some("partial".to_string())
            }
        },
        _ => Some("insufficient".to_string()),
    }
}

/// 把 hit_outcome 归一化为决策级二进制 outcome（"win" / "loss"），供决策记录回填。
///
/// ## 口径（这是**方向判定**，不是盈利判定）
/// - `hit` / `partial` → `win`
/// - `miss` / `false_hit` → `loss`
/// - `insufficient` / `None` → 数据不足不判定 → `None`（跳过）
///
/// B3「决策-结果闭环」要求回填口径与离线回测口径一致：回填决策记录（stock_analyses.outcome）
/// 与计算命中率必须复用同一套判定，避免两处各自实现导致口径漂移（如一侧把 partial 算 hit、
/// 另一侧不算）。本函数是二者共用的唯一判定源。
///
/// ## ⚠ 与 `ActionStats::strict_win_rate` 的关系（2026-10-25 G4 澄清）
///
/// 下面三个出口**刻意不同**，不是口径漂移 —— 它们回答的不是同一个问题：
///
/// | 出口 | 回答的问题 | `partial`（-5%~0 微亏）归属 |
/// |---|---|---|
/// | **本函数** → `stock_analyses.outcome` | 方向判对了吗 | **win**（微亏但仍看对方向） |
/// | `ActionStats::strict_win_rate` | 按这个判断会不会亏 | **输**（partial 计输） |
/// | `ActionStats::net_win_rate` | 扣成本后赚没赚 | 由净收益符号决定 |
///
/// 把本函数改成 `partial → loss` 会让「方向对了、只是成本吃掉利润」被记成决策失败，
/// 污染的是**决策质量**归因；而 strict / net 只关心**钱**。两件事不可合并。
pub fn hit_outcome_to_binary_outcome(hit_outcome: Option<&str>) -> Option<&'static str> {
    match hit_outcome {
        Some("hit") | Some("partial") => Some("win"),
        Some("miss") | Some("false_hit") => Some("loss"),
        _ => None,
    }
}
