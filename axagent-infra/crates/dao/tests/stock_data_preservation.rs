// SPDX-License-Identifier: AGPL-3.0-only
//!
//! P6.1 股票域升级收敛数据保留：
//! 既有 `stock_analyses` / `reco_picks` / `portfolio_holdings` 行在重跑
//! `bootstrap_schema` 后必须保留，且代表性字段逐项不变。

use axagent_dao::db::create_test_pool;
use axagent_dao::reconcile::apply::bootstrap_schema;
use axagent_entities::{portfolio_holdings, reco_picks, stock_analyses};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

async fn seed_stock_rows(db: &DatabaseConnection) {
    stock_analyses::ActiveModel {
        id: Set("analysis-preserve-1".into()),
        stock_code: Set("600519".into()),
        stock_name: Set("贵州茅台".into()),
        analysis_date: Set("2026-11-18".into()),
        provider_id: Set("test-provider".into()),
        conversation_id: Set("conv-preserve".into()),
        status: Set("completed".into()),
        decision_action: Set(Some("hold".into())),
        decision_json: Set(Some(r#"{"action":"hold","confidence":0.8}"#.into())),
        blackboard_snapshot: Set(Some(r#"{"report":{"market":"ok"}}"#.into())),
        analysis_kind: Set("live".into()),
        source: Set("live".into()),
        synthetic: Set(0),
        created_at: Set(1_700_000_000_000),
        updated_at: Set(1_700_000_000_000),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("插入股票分析行应成功");

    reco_picks::ActiveModel {
        id: Set("reco-preserve-1".into()),
        generated_at: Set("2026-11-18T10:00:00+08:00".into()),
        period: Set("mid".into()),
        stock_code: Set("600519".into()),
        stock_name: Set("贵州茅台".into()),
        style: Set("value".into()),
        confidence: Set(88),
        synthetic: Set(0),
        seed_pool_json: Set(Some(r#"[["600519","贵州茅台"]]"#.into())),
        strategy_weights_json: Set(Some(r#"{"trend_mid":0.8}"#.into())),
        pick_data: Set(None),
        reco_version: Set(Some(2026)),
        as_of_date: Set(None),
        purpose: Set(Some("live".into())),
        created_at: Set("2026-11-18T10:00:00+08:00".into()),
    }
    .insert(db)
    .await
    .expect("插入荐股行应成功");

    portfolio_holdings::ActiveModel {
        id: Set("holding-preserve-1".into()),
        stock_code: Set("600519".into()),
        stock_name: Set("贵州茅台".into()),
        shares: Set(100.0),
        avg_cost: Set(1480.0),
        notes: Set(Some("保留测试".into())),
        created_at: Set(1_700_000_000_000),
        updated_at: Set(1_700_000_000_000),
    }
    .insert(db)
    .await
    .expect("插入组合持仓应成功");
}

#[tokio::test]
async fn upgrade_reconciliation_preserves_stock_business_rows() {
    let test = create_test_pool().await.expect("创建测试数据库应成功");
    let db = &test.conn;
    seed_stock_rows(db).await;

    assert_eq!(stock_analyses::Entity::find().all(db).await.expect("查分析").len(), 1);
    assert_eq!(reco_picks::Entity::find().all(db).await.expect("查荐股").len(), 1);
    assert_eq!(portfolio_holdings::Entity::find().all(db).await.expect("查持仓").len(), 1);

    let outcome = bootstrap_schema(db).await.expect("股票表升级收敛应成功");
    assert!(outcome.refusal.is_none(), "升级收敛不得被拒");

    assert_eq!(stock_analyses::Entity::find().all(db).await.expect("查分析").len(), 1);
    assert_eq!(reco_picks::Entity::find().all(db).await.expect("查荐股").len(), 1);
    assert_eq!(portfolio_holdings::Entity::find().all(db).await.expect("查持仓").len(), 1);

    let analysis = stock_analyses::Entity::find_by_id("analysis-preserve-1".to_string())
        .one(db)
        .await
        .expect("查分析")
        .expect("分析行必须保留");
    assert_eq!(analysis.stock_code, "600519");
    assert_eq!(analysis.status, "completed");
    assert_eq!(analysis.decision_action.as_deref(), Some("hold"));
    assert_eq!(analysis.blackboard_snapshot.as_deref(), Some(r#"{"report":{"market":"ok"}}"#));

    let pick = reco_picks::Entity::find_by_id("reco-preserve-1".to_string())
        .one(db)
        .await
        .expect("查荐股")
        .expect("荐股行必须保留");
    assert_eq!(pick.confidence, 88);
    assert_eq!(pick.period, "mid");
    assert_eq!(pick.reco_version, Some(2026));

    let holding = portfolio_holdings::Entity::find_by_id("holding-preserve-1".to_string())
        .one(db)
        .await
        .expect("查持仓")
        .expect("持仓行必须保留");
    assert_eq!(holding.shares, 100.0);
    assert_eq!(holding.avg_cost, 1480.0);
    assert_eq!(holding.notes.as_deref(), Some("保留测试"));
}
