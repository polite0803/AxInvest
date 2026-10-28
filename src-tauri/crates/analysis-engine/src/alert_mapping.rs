// SPDX-License-Identifier: AGPL-3.0-only

//! price_alerts 表 ↔ RealtimeMonitor 双向转换（v203 数据模型对齐）
//!
//! ## 背景
//!
//! v203 迁移为 `price_alerts` 表新增 `alert_type` / `condition_type` / `threshold`
//! 三列，与 `RealtimeMonitor` 的 6 类 `alert_type` 对齐。本模块集中提供双向转换，
//! 替换 `monitor_emitter.rs` / `stock_analysis.rs` / `stock_workflow/core.rs`
//! 三处散落的硬编码 match（AGENTS.md Rule 12 禁止重复定义）。
//!
//! ## 权威模型
//!
//! 以 `RealtimeMonitor` 的 6 类 `alert_type` 为权威：
//! - `stop_loss` / `take_profit` / `resistance` / `support` → `condition_type=price`
//! - `change` → `condition_type=change_pct`
//! - `volume` → `condition_type=turnover_rate`

use crate::monitor::MonitorConfig;

/// 6 类告警类型常量（与 RealtimeMonitor alert_type 字段对齐）
pub mod alert_types {
    pub const STOP_LOSS: &str = "stop_loss";
    pub const TAKE_PROFIT: &str = "take_profit";
    pub const RESISTANCE: &str = "resistance";
    pub const SUPPORT: &str = "support";
    pub const CHANGE: &str = "change";
    pub const VOLUME: &str = "volume";
}

/// 阈值语义类型
pub mod condition_types {
    pub const PRICE: &str = "price";
    pub const CHANGE_PCT: &str = "change_pct";
    pub const TURNOVER_RATE: &str = "turnover_rate";
}

/// 从 `MonitorAlert.alert_type` 推导 `condition_type`
///
/// - `stop_loss` / `take_profit` / `resistance` / `support` → `price`
/// - `change` → `change_pct`
/// - `volume` → `turnover_rate`
/// - 未知类型 → `price`（保守回退）
pub fn condition_type_for(alert_type: &str) -> &'static str {
    match alert_type {
        alert_types::CHANGE => condition_types::CHANGE_PCT,
        alert_types::VOLUME => condition_types::TURNOVER_RATE,
        _ => condition_types::PRICE,
    }
}

/// 把任意来源的「告警类型」写法归一到 6 类之一。
///
/// 三个入口各收一种拼写：面板与 agent 工具收老 `condition` 值（`above`/`below`/
/// `change_up`/`change_down`/`volume_spike`），工作流与新代码收 6 类标准值。
/// 不归一就会让「同一条规则」在按 `(股票代码, 告警类型)` 去重时被算成两类。
/// 未知值保守回退 `take_profit` —— 这是改造前面板那条路就有的行为，此处原样保留。
pub fn normalize_alert_type(input: &str) -> &'static str {
    // 逐个返回常量而非 `input`：匹配到常量并不缩短 `input` 的生命周期，
    // 直接 `=> input` 会报 "lifetime may not live long enough"。
    match input {
        alert_types::STOP_LOSS => alert_types::STOP_LOSS,
        alert_types::TAKE_PROFIT => alert_types::TAKE_PROFIT,
        alert_types::RESISTANCE => alert_types::RESISTANCE,
        alert_types::SUPPORT => alert_types::SUPPORT,
        alert_types::CHANGE => alert_types::CHANGE,
        alert_types::VOLUME => alert_types::VOLUME,
        other => legacy_condition_to_alert_type(other).unwrap_or(alert_types::TAKE_PROFIT),
    }
}

/// 新 alert_type → 老 `condition` 列的取值（写侧兼容用）。
///
/// 为什么要有这一份：此前每个写入点各自手写 `"above"` / `"below"` 字面量，
/// 与读侧的映射表分两处维护 —— 一处改了就静默不一致（禁区 12）。
///
/// ⚠ **这一份是 6→5 有损的**：老列只有 `above`/`below`/`change_up`/`change_down`/
/// `volume_spike` 五个取值，装不下六类 —— `resistance` 与 `take_profit` 同写 `above`、
/// `support` 与 `stop_loss` 同写 `below`。所以**不许**拿 `condition` 当去重键或分支依据，
/// 去重与求值一律读 `alert_type`（[`normalize_alert_type`]）。写兼容列只是为了仍在读老列的
/// 消费者（面板那一列显示 ≥/≤）不至于空白。
pub fn legacy_condition_for(alert_type: &str) -> Option<&'static str> {
    match alert_type {
        alert_types::TAKE_PROFIT | alert_types::RESISTANCE => Some("above"),
        alert_types::STOP_LOSS | alert_types::SUPPORT => Some("below"),
        alert_types::CHANGE => Some("change_up"),
        alert_types::VOLUME => Some("volume_spike"),
        _ => None,
    }
}

/// 老 condition 值 → 新 alert_type 的兼容映射
///
/// 用于读取老数据（condition 列有值、alert_type 列为 NULL 的迁移过渡期数据）。
/// v203 迁移已批量回填，但新代码仍应支持读老值兜底。
pub fn legacy_condition_to_alert_type(condition: &str) -> Option<&'static str> {
    match condition {
        "above" => Some(alert_types::TAKE_PROFIT),
        "below" => Some(alert_types::STOP_LOSS),
        "change_up" | "change_down" => Some(alert_types::CHANGE),
        "volume_spike" => Some(alert_types::VOLUME),
        _ => None,
    }
}

/// 六类告警的中文标签 —— **写进中文散文**用（每日复盘 / 通知正文）。
///
/// H-8（2026-10-15）：这些文案此前直接拼 `price_alerts.condition`（老兼容列）⇒ ① 拼出来是
/// 英文 token（`above触发: …`）；② 那一列 6→5 有损，`resistance` 与 `take_profit` 同写
/// `above` ⇒ 「压力位突破」会被写成止盈那条的词。前端那份走 i18n 键
/// （`stockAnalysis.alert.type*`），本函数只服务 Rust 侧的中文散文。
/// 未知写法先过 [`normalize_alert_type`]（老值 `above` 之类也认）。
pub fn alert_type_label_zh(alert_type: &str) -> &'static str {
    match normalize_alert_type(alert_type) {
        alert_types::STOP_LOSS => "止损",
        alert_types::TAKE_PROFIT => "止盈",
        alert_types::RESISTANCE => "压力位突破",
        alert_types::SUPPORT => "支撑位跌破",
        alert_types::CHANGE => "涨跌幅",
        alert_types::VOLUME => "放量",
        // normalize 只会返上面六个常量之一，这一格不可达（match 需要兜底臂）
        _ => "价格",
    }
}

/// 该类型是「跌破/低于」方向吗（`≤`）—— 否则是「突破/高于」方向（`≥`）。
///
/// 止损与支撑位跌破看下沿，止盈与压力位突破看上沿；涨跌幅 / 放量两种方向由阈值符号表达，
/// 归到「高于」（`≥`）那侧。原先这个方向由老 `condition == "above"` 判，同样带上 6→5 的有损。
pub fn is_below_alert(alert_type: &str) -> bool {
    matches!(normalize_alert_type(alert_type), alert_types::STOP_LOSS | alert_types::SUPPORT)
}

/// 从 price_alerts Model 反向构造 MonitorConfig 字段
///
/// 用于启动时从 DB 加载告警配置到 RealtimeMonitor。
/// 返回 `(field_name, value)`，调用方据此填充 MonitorConfig 的 6 个 Option 字段。
///
/// 优先读 `alert_type` + `threshold`；若 `alert_type` 为 NULL（老数据未回填），
/// 回退到 `legacy_condition_to_alert_type(condition)` + `target_price`。
pub fn db_model_to_config_field(
    alert_type: Option<&str>,
    condition_type: Option<&str>,
    threshold: Option<f64>,
    legacy_condition: &str,
    legacy_target_price: f64,
) -> Option<(&'static str, f64)> {
    // 优先走新字段
    if let Some(at) = alert_type {
        let static_at: &'static str = match at {
            alert_types::STOP_LOSS => alert_types::STOP_LOSS,
            alert_types::TAKE_PROFIT => alert_types::TAKE_PROFIT,
            alert_types::RESISTANCE => alert_types::RESISTANCE,
            alert_types::SUPPORT => alert_types::SUPPORT,
            alert_types::CHANGE => alert_types::CHANGE,
            alert_types::VOLUME => alert_types::VOLUME,
            _ => return None,
        };
        let value = threshold.unwrap_or(legacy_target_price);
        return Some((static_at, value));
    }
    // 回退到老 condition
    let static_at = legacy_condition_to_alert_type(legacy_condition)?;
    // 老数据只有 price 类阈值
    let _ = condition_type; // 老数据的 condition_type 为 NULL，忽略
    Some((static_at, legacy_target_price))
}

/// 把一条「单一阈值的告警」落成 `MonitorConfig`
///
/// 三个写入点（面板手建、agent 工具、工作流每次分析自动建单）原先各自手抄一遍
/// 11 字段结构体字面量，面板那条还额外手抄了一个 6 臂 match（禁区 12）。
/// 收敛到这一处后，新增一类告警只需改 [`condition_type_for`] 与本函数。
///
/// 未知 `alert_type` 返回 `None`：调用方**不得**把一条六个阈值全空的配置塞进
/// 引擎 —— 那会在监控列表里占一个位置却什么都不盯，比不接更坏。
pub fn config_for_alert(
    alert_id: String,
    stock_code: String,
    stock_name: String,
    alert_type: &str,
    threshold: f64,
) -> Option<MonitorConfig> {
    let (
        stop_loss,
        take_profit,
        resistance_break,
        support_break,
        change_pct_alert,
        turnover_rate_alert,
    ) = match alert_type {
        alert_types::STOP_LOSS => (Some(threshold), None, None, None, None, None),
        alert_types::TAKE_PROFIT => (None, Some(threshold), None, None, None, None),
        alert_types::RESISTANCE => (None, None, Some(threshold), None, None, None),
        alert_types::SUPPORT => (None, None, None, Some(threshold), None, None),
        alert_types::CHANGE => (None, None, None, None, Some(threshold), None),
        alert_types::VOLUME => (None, None, None, None, None, Some(threshold)),
        _ => return None,
    };
    Some(MonitorConfig {
        alert_id,
        stock_code,
        stock_name,
        stop_loss,
        take_profit,
        resistance_break,
        support_break,
        change_pct_alert,
        turnover_rate_alert,
        enabled: true,
    })
}

/// 从 [`config_for_alert`] 产出的配置里读回它设的那个阈值
///
/// 用于「一条配置 ↔ 一类告警」这个约定本身可被验证：六个字段里有且只有一个非空，
/// 且返回值等于该值。生产代码不读它，判据读它（防 `config_for_alert` 退化成
/// 全空配置却仍被 `add_config` 接受）。
pub fn single_threshold_of(config: &MonitorConfig) -> Option<(&'static str, f64)> {
    let hits = [
        (alert_types::STOP_LOSS, config.stop_loss),
        (alert_types::TAKE_PROFIT, config.take_profit),
        (alert_types::RESISTANCE, config.resistance_break),
        (alert_types::SUPPORT, config.support_break),
        (alert_types::CHANGE, config.change_pct_alert),
        (alert_types::VOLUME, config.turnover_rate_alert),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|x| (k, x)))
    .collect::<Vec<_>>();
    if hits.len() == 1 { Some(hits[0]) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_type_for_returns_price_for_price_alerts() {
        assert_eq!(condition_type_for(alert_types::STOP_LOSS), condition_types::PRICE);
        assert_eq!(condition_type_for(alert_types::TAKE_PROFIT), condition_types::PRICE);
        assert_eq!(condition_type_for(alert_types::RESISTANCE), condition_types::PRICE);
        assert_eq!(condition_type_for(alert_types::SUPPORT), condition_types::PRICE);
    }

    #[test]
    fn condition_type_for_returns_change_pct_for_change() {
        assert_eq!(condition_type_for(alert_types::CHANGE), condition_types::CHANGE_PCT);
    }

    #[test]
    fn condition_type_for_returns_turnover_for_volume() {
        assert_eq!(condition_type_for(alert_types::VOLUME), condition_types::TURNOVER_RATE);
    }

    /// 写侧兼容列与读侧映射的对账。分两组断 —— 老列只有五个取值，装不下六类，
    /// 「严格互逆」这个前提本身就是错的（本条测试第一版就是按错前提写的，实测把
    /// `resistance → above → take_profit` 判成失败）。
    #[test]
    fn legacy_condition_mapping_is_exclusive_where_it_can_be() {
        // 独占组：老值只对应这一类 ⇒ 必须原样读回
        for at in [
            alert_types::STOP_LOSS,
            alert_types::TAKE_PROFIT,
            alert_types::CHANGE,
            alert_types::VOLUME,
        ] {
            let legacy = legacy_condition_for(at).unwrap_or_else(|| panic!("{at} 应有兼容列值"));
            assert_eq!(
                legacy_condition_to_alert_type(legacy),
                Some(at),
                "{at} → {legacy} 必须能原样读回"
            );
        }
        // 共享组：resistance/support 与 take_profit/stop_loss 挤在同一个老值里
        assert_eq!(legacy_condition_for(alert_types::RESISTANCE), Some("above"));
        assert_eq!(legacy_condition_for(alert_types::SUPPORT), Some("below"));
        // ⇒ 兼容列**不得**当去重键或分支依据：它读回来的是兄弟类，只有 alert_type 分得开
        assert_ne!(
            legacy_condition_to_alert_type("above"),
            Some(alert_types::RESISTANCE),
            "把兼容列当唯一键会把阻力位告警并进取盈告警"
        );
        assert_eq!(legacy_condition_for("margin_call"), None, "未知类别不得兜成某个老值");
    }

    #[test]
    fn config_for_alert_sets_exactly_one_field_per_type() {
        for at in [
            alert_types::STOP_LOSS,
            alert_types::TAKE_PROFIT,
            alert_types::RESISTANCE,
            alert_types::SUPPORT,
            alert_types::CHANGE,
            alert_types::VOLUME,
        ] {
            let cfg = config_for_alert("id-1".into(), "601899".into(), "紫金矿业".into(), at, 12.5)
                .unwrap_or_else(|| panic!("{at} 应能落成配置"));
            assert_eq!(
                single_threshold_of(&cfg),
                Some((at, 12.5)),
                "{at} 应只设自己那一个阈值字段"
            );
            assert!(cfg.enabled, "写入点建的都是启用态");
        }
    }

    /// 未知类型必须落成 `None`，不能落成「六个全空、占位不盯」的配置。
    #[test]
    fn config_for_alert_refuses_unknown_type() {
        let r =
            config_for_alert("id-2".into(), "000001".into(), "平安银行".into(), "margin_call", 9.0);
        assert!(r.is_none(), "未知类型不得落成全空配置");
    }

    /// 归一化：三种入口拼写都落到同一类，否则按 (股票, 类型) 去重会把同一条规则算成两类。
    #[test]
    fn normalize_alert_type_merges_legacy_and_standard_spellings() {
        assert_eq!(normalize_alert_type("stop_loss"), alert_types::STOP_LOSS);
        assert_eq!(normalize_alert_type("below"), alert_types::STOP_LOSS);
        assert_eq!(normalize_alert_type("change_down"), alert_types::CHANGE);
        assert_eq!(normalize_alert_type("volume_spike"), alert_types::VOLUME);
        assert_eq!(normalize_alert_type("margin_call"), alert_types::TAKE_PROFIT);
    }

    #[test]
    fn legacy_condition_to_alert_type_maps_above_below() {
        assert_eq!(legacy_condition_to_alert_type("above"), Some(alert_types::TAKE_PROFIT));
        assert_eq!(legacy_condition_to_alert_type("below"), Some(alert_types::STOP_LOSS));
        assert_eq!(legacy_condition_to_alert_type("change_up"), Some(alert_types::CHANGE));
        assert_eq!(legacy_condition_to_alert_type("volume_spike"), Some(alert_types::VOLUME));
        assert_eq!(legacy_condition_to_alert_type("unknown"), None);
    }

    #[test]
    fn db_model_to_config_field_prefers_new_fields() {
        // 新字段优先
        let r = db_model_to_config_field(
            Some(alert_types::RESISTANCE),
            Some(condition_types::PRICE),
            Some(15.5),
            "above",
            12.0,
        );
        assert_eq!(r, Some((alert_types::RESISTANCE, 15.5)));

        // 老字段回退
        let r = db_model_to_config_field(None, None, None, "below", 9.2);
        assert_eq!(r, Some((alert_types::STOP_LOSS, 9.2)));

        // 老字段未知值
        let r = db_model_to_config_field(None, None, None, "unknown", 1.0);
        assert_eq!(r, None);
    }

    /// H-8：中文标签与方向必须按 `alert_type` 判 —— 老兼容列分不开 resistance/take_profit
    /// （同写 `above`）、support/stop_loss（同写 `below`），拿它拼文案会把类型写错。
    #[test]
    fn chinese_label_and_direction_follow_alert_type_not_legacy_condition() {
        assert_eq!(alert_type_label_zh(alert_types::RESISTANCE), "压力位突破");
        assert_eq!(alert_type_label_zh(alert_types::TAKE_PROFIT), "止盈");
        assert_ne!(
            alert_type_label_zh(alert_types::RESISTANCE),
            alert_type_label_zh(alert_types::TAKE_PROFIT),
            "两类同写 `above` 的告警，中文标签必须分得开"
        );
        // 老写法也认（normalize 兜住）
        assert_eq!(alert_type_label_zh("above"), "止盈");
        assert_eq!(alert_type_label_zh("volume_spike"), "放量");

        assert!(is_below_alert(alert_types::STOP_LOSS));
        assert!(is_below_alert(alert_types::SUPPORT));
        assert!(!is_below_alert(alert_types::TAKE_PROFIT));
        assert!(!is_below_alert(alert_types::RESISTANCE));
        assert!(!is_below_alert(alert_types::CHANGE), "涨跌幅方向由阈值符号表达，归 `≥` 那侧");
    }
}
