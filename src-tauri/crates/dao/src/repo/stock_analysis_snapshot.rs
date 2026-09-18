// SPDX-License-Identifier: AGPL-3.0-only
//! `stock_analyses.blackboard_snapshot` 的读写 —— **持久化边界的唯一实现**。
//!
//! ## 为什么要单独成一个 repo 函数
//!
//! 决策落库后的仿真挂钩（`commands/stock_workflow/sim_hook.rs`）原本在**命令层**
//! 直接 `axagent_entities::stock_analyses::Entity::…` 读写快照 ⇒ 命中分层门禁
//! `scripts/check-layer-discipline.mjs` 规则 1（`commands-no-direct-db`：
//! 命令层不得直连 `sea_orm` / `axagent_entities`，须经 dao / service）。
//!
//! 把读写搬到这里是**真下沉**而非换 import：实体与 `sea_query` 表达式不再出现在
//! 命令层，命令层只拿到 `serde_json::Value`。
//!
//! ## ⚠ 三态语义必须保留，勿合并
//!
//! `blackboard_snapshot` 是 `TEXT` 列，存的是整份黑板 JSON 字符串。历史数据里
//! 存在「记录在但快照为空/非 JSON」的行，调用方（sim_hook）据此区分三件事：
//!
//! | 情形 | 返回 | 调用方动作 |
//! |---|---|---|
//! | 记录在 + 可解析 | `Ok(Some(对象))` | 合并字段后写回 |
//! | 记录在 + 空/损坏 | `Ok(Some({}))` | **照样写回**（不丢该行其他字段之外的语义） |
//! | 记录不存在 | `Ok(None)` | 放弃写回（否则会造出一行只有快照的残缺记录） |
//! | 查询失败 | `Err` | 打日志、放弃写回 |
//!
//! 第二态刻意**不**返回 `None`、也不报错：空快照是合法现状，不是故障。

use axagent_entities::stock_analyses;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

/// 读 `blackboard_snapshot` 并解析为 JSON 对象。语义见模块文档的三态表。
pub async fn read_blackboard_snapshot(
    db: &DatabaseConnection,
    analysis_id: &str,
) -> Result<Option<serde_json::Value>, sea_orm::DbErr> {
    match stock_analyses::Entity::find_by_id(analysis_id.to_string()).one(db).await? {
        Some(rec) => Ok(Some(
            rec.blackboard_snapshot
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| serde_json::json!({})),
        )),
        None => Ok(None),
    }
}

/// **覆盖**写回 `blackboard_snapshot`（整列替换，不是合并）。
///
/// 返回受影响行数：调用方需要区分「写成功」与「写了个寂寞」——
/// 目标行不存在时 `execute_many` 不报错，只返回 0 行（静默无效）。
pub async fn write_blackboard_snapshot(
    db: &DatabaseConnection,
    analysis_id: &str,
    snapshot: &serde_json::Value,
) -> Result<u64, sea_orm::DbErr> {
    let res = stock_analyses::Entity::update_many()
        .col_expr(stock_analyses::Column::BlackboardSnapshot, Expr::value(snapshot.to_string()))
        .filter(stock_analyses::Column::Id.eq(analysis_id.to_string()))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}
