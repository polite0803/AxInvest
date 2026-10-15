// SPDX-License-Identifier: AGPL-3.0-only

//! 价格告警写入的唯一入口 —— 同股同类型只留一条未触发行（PLAN §11 裁定 10 取 ①）
//!
//! 原先有四条写入路各自 `insert`：面板手建、agent 工具 `stock_create_price_alert`、
//! 工作流每次分析追加的止盈行与止损行。后果是现网 56 行只分布在 16 只股票上，
//! `(stock_code, alert_type)` 重复组合 16 个（`601899` 单只 3 行止损）。触发即终态
//! （阶段 B）生效后更糟：同一股会留下 N-1 条**仍然活跃**的重复行继续盯同一语义，
//! 每轮各算一次、各发一条告警。
//!
//! 收口后的形状：未触发行按 `(股票代码, 告警类型)` **复用并更新阈值**；
//! 已触发的行是终态，不吸收新单（新一轮监控从一条新行开始）。
//! 复用不改变 `id`，因此内存引擎侧的 `add_config`（按 `alert_id` 覆盖）天然对齐。
//!
//! ## 本模块只做归一化与编排，SQL 在 dao
//!
//! `price_alert` 表的查 / 复用更新 / 新建全部落在 `axagent_dao::repo::price_alert`
//! （同 `_shared_reco_pool` 对 `reco_pool` 的拆法）：命令层出现 `sea_orm::` /
//! `axagent_entities::` 字面量会命中分层门禁
//! [`scripts/check-layer-discipline.mjs`] 规则 1，而本模块是**新文件**（基线为 0）。
//! alert_type 归一化在 `axagent_analysis_engine::alert_mapping`，dao 反向依赖该 crate
//! 会成环 ⇒ 归一化后的三个标量由本模块算好再传下去。

use axagent_analysis_engine::alert_mapping::{
    condition_type_for, config_for_alert, legacy_condition_for, normalize_alert_type,
};
use axagent_analysis_engine::monitor::RealtimeMonitor;
use axagent_dao::repo::price_alert::{self as alert_dao, AlertRow};
use std::sync::Arc;

/// 写一条价格告警。返回 `(行, 是否复用了既有未触发行)`。
pub async fn upsert_price_alert(
    db: &axagent_dao::DatabaseConnection,
    stock_code: &str,
    stock_name: &str,
    alert_type_input: &str,
    threshold: f64,
) -> Result<(AlertRow, bool), String> {
    let alert_type = normalize_alert_type(alert_type_input);
    let condition_type = condition_type_for(alert_type);
    let legacy_condition = legacy_condition_for(alert_type).ok_or_else(|| {
        format!("告警类型 {alert_type} 没有对应的兼容列取值，拒绝落库（不要静默写一个错值）")
    })?;
    let now = chrono::Utc::now().timestamp_millis();

    let existing = alert_dao::find_open_untriggered(db, stock_code, alert_type)
        .await
        .map_err(|e| format!("查询既有价格告警失败: {e}"))?;

    if let Some(row) = existing {
        let id = row.id.clone();
        let saved = alert_dao::reuse_open_alert(
            db,
            row,
            stock_name,
            legacy_condition,
            condition_type,
            threshold,
            now,
        )
        .await
        .map_err(|e| format!("更新价格告警失败: {e}"))?;
        tracing::info!(
            "[price_alert] 复用未触发行 id={id}：{stock_code} {alert_type} 阈值改为 {threshold}"
        );
        return Ok((saved, true));
    }

    let saved = alert_dao::insert_open_alert(
        db,
        stock_code,
        stock_name,
        alert_type,
        legacy_condition,
        condition_type,
        threshold,
        now,
    )
    .await
    .map_err(|e| format!("创建价格告警失败: {e}"))?;
    Ok((saved, false))
}

/// 写库 + **同步内存引擎** —— 两条路（面板命令 / agent 工具）共用这一份。
///
/// 裁定 15（2026-10-15）：agent 工具那条路原先只拿得到 `db`，写库后不调 `add_config`，
/// 于是同一个「建告警」有两种生效时刻（面板即时、agent 要等下次启动装载）。把「落库 ⇒
/// 归一化 ⇒ 造配置 ⇒ 进引擎」这条链收到一处，两条路就不可能再分叉。
///
/// `monitor` 传 `None`（引擎未装配）时**如实返回 `false`**：调用方据此在返回体里写明
/// 「本轮未进引擎」，而不是静默假装已经生效。
pub async fn upsert_price_alert_and_wire(
    db: &axagent_dao::DatabaseConnection,
    monitor: Option<&Arc<RealtimeMonitor>>,
    stock_code: &str,
    stock_name: &str,
    alert_type_input: &str,
    threshold: f64,
) -> Result<(AlertRow, bool, bool), String> {
    let (saved, reused) =
        upsert_price_alert(db, stock_code, stock_name, alert_type_input, threshold).await?;

    let Some(monitor) = monitor else {
        tracing::warn!(
            "[price_alert] RealtimeMonitor 未初始化：{stock_code} 的告警仅写入 DB，本轮不会进监控引擎"
        );
        return Ok((saved, reused, false));
    };

    let alert_type = normalize_alert_type(alert_type_input);
    let Some(config) = config_for_alert(
        saved.id.clone(),
        stock_code.to_string(),
        stock_name.to_string(),
        alert_type,
        threshold,
    ) else {
        tracing::warn!(
            "[price_alert] alert_type={alert_type} 落不成监控配置，该行只写入 DB，不会进监控引擎"
        );
        return Ok((saved, reused, false));
    };
    monitor.add_config(config).await;
    tracing::info!(
        "[price_alert] 已加入实时监控（{}）: {stock_code} {stock_name} {alert_type_input} → alert_type={alert_type} threshold={threshold:.2}",
        if reused {
            "复用未触发行"
        } else {
            "新建行"
        }
    );
    Ok((saved, reused, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    // 六类常量只有测试在用（生产侧一律经 `normalize_alert_type` 归一，不列清单）
    use axagent_analysis_engine::alert_mapping::alert_types;

    async fn two_rows_same_code_and_type(
        db: &axagent_dao::DatabaseConnection,
        second: f64,
    ) -> Vec<AlertRow> {
        upsert_price_alert(db, "601899", "紫金矿业", "stop_loss", 12.0)
            .await
            .expect("第一次建单应成功");
        upsert_price_alert(db, "601899", "紫金矿业", "stop_loss", second)
            .await
            .expect("第二次建单应成功");
        alert_dao::find_by_stock(db, "601899").await.expect("按股票读回告警行应成功")
    }

    /// 裁定 10 的正主：同股同类型建两次 ⇒ 表里仍只有一行，且阈值是第二次给的那个。
    #[tokio::test]
    async fn same_stock_same_type_second_create_reuses_the_untriggered_row() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;

        let rows = two_rows_same_code_and_type(db, 11.4).await;
        assert_eq!(rows.len(), 1, "同一 (股票, 告警类型) 的未触发行必须被复用");
        assert_eq!(rows[0].threshold, Some(11.4), "复用时要改成新的阈值");
        assert_eq!(rows[0].alert_type.as_deref(), Some(alert_types::STOP_LOSS));
        assert_eq!(rows[0].condition, "below", "兼容列要与新列同向");
    }

    /// 已触发的行是终态：不得被新单「复活」，否则「已触发」Tab 的历史会凭空消失。
    #[tokio::test]
    async fn triggered_row_is_not_absorbed_by_a_new_order() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;

        let (first, _) =
            upsert_price_alert(db, "600519", "贵州茅台", "take_profit", 30.0).await.unwrap();
        // 置为终态走监控侧既有的回写实现（同 `monitor_emitter` 生产路径），不另写一份 UPDATE
        crate::init::monitor_emitter::mark_triggered_by_id(db, &first.id, 1_760_000_000_000).await;

        let (second, reused) =
            upsert_price_alert(db, "600519", "贵州茅台", "take_profit", 33.0).await.unwrap();
        assert!(!reused, "终态行不该被复用");
        assert_ne!(second.id, first.id, "新监控周期从一条新行开始");
        let rows = alert_dao::find_by_stock(db, "600519").await.expect("按股票读回告警行应成功");
        let kept = rows.iter().find(|r| r.id == first.id).expect("历史行应仍在表里");
        assert_eq!(kept.is_triggered, 1, "历史行保持已触发");
        assert_eq!(kept.threshold, Some(30.0), "历史行的阈值不能被改写");
    }

    /// 同股不同类型仍是两行（去重口径不能顺手把止损和止盈合成一条）。
    #[tokio::test]
    async fn different_types_on_the_same_stock_stay_separate() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;

        for kind in ["stop_loss", "take_profit"] {
            upsert_price_alert(db, "300750", "宁德时代", kind, 12.0).await.unwrap();
        }
        let rows = alert_dao::find_by_stock(db, "300750").await.expect("按股票读回告警行应成功");
        assert_eq!(rows.len(), 2);

        // 再来一轮：仍各自复用，不追加
        for kind in ["stop_loss", "take_profit"] {
            let (_, reused) =
                upsert_price_alert(db, "300750", "宁德时代", kind, 13.0).await.unwrap();
            assert!(reused, "{kind} 这一轮应命中既有未触发行");
        }
        let after = alert_dao::find_by_stock(db, "300750").await.expect("按股票读回告警行应成功");
        assert_eq!(after.len(), 2, "重复跑分析不能再涨行数");
    }

    /// 老面板传的是 `above`/`below` 这类兼容值，必须归一到 6 类再谈去重。
    #[tokio::test]
    async fn legacy_spellings_normalise_onto_the_same_row() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;

        upsert_price_alert(db, "000001", "平安银行", "below", 9.0).await.unwrap();
        let (second, reused) =
            upsert_price_alert(db, "000001", "平安银行", "stop_loss", 8.5).await.unwrap();
        assert!(reused, "「below」与「stop_loss」是同一类，不能各留一行");
        assert_eq!(second.condition_type.as_deref(), Some("price"));
    }
}
