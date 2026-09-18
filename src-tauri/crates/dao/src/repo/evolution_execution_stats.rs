// SPDX-License-Identifier: AGPL-3.0-only

//! 进化产物执行统计 repository —— 持久化真实执行反馈（阶段四后置闭环 · D3）。
//!
//! `EvolutionFeedbackSinkImpl::record` 在更新内存统计后异步调用
//! [`upsert_execution_feedback`] 落库；应用启动时 [`load_all_execution_stats`]
//! 一次性读回内存，保证重启后真实执行证据不丢失。
//!
//! 表 `evolution_execution_stats` 的 DDL 由 v122 建表，复合主键
//! `(conversation_id, tool_id)`；实体声明见 [`axagent_entities::evolution_execution_stats`]。
//!
//! ## 改造记录（2026-09-16）
//!
//! 原实现用 `Statement::from_sql_and_values` 手写 SQLite(`?N`) / PostgreSQL(`$N`)
//! 双分支。改走 SeaORM 实体后双分支消失。
//!
//! 两处方言差异的处置：
//! 1. **自增列的 SET 表达式**：原 PG 分支写
//!    `usage_count = evolution_execution_stats.usage_count + 1`（**表限定**），
//!    SQLite 分支写裸列名。2026-09-16 统一为裸列名时**判断错了** ——
//!    ⚠ PG 的 `ON CONFLICT ... DO UPDATE SET` 命名空间里**同时**有目标表与
//!    `excluded` 伪关系 ⇒ 未限定的列名命中两处，解析期报
//!    `42702 column reference "..." is ambiguous`。**与表达式里是否出现
//!    `excluded.` 无关**：`usage_count + 1` 单独一项也照样报错
//!    （真库实测 PG 18，2026-09-17）。SQLite 宽容，故只在 PG 暴露。
//!    2026-09-17 起改用 `Expr::col((表名, 列))` ⇒ 渲染成 `"表"."列"`，
//!    两方言都接受；表名取自实体（[`EntityName`]）而非手抄字符串。
//! 2. **递增项**：不再引用 `excluded.<col>`，直接用 Rust 侧已知的增量字面量
//!    （与 `ActiveModel` 写入的是同一个变量，天然同源），少一处方言相关写法。
//!
//! ⚠ 解码容错性变化：原实现逐列 `unwrap_or(0)`，单行脏数据不会中断整批读取；
//! 改实体后一行解码失败即整查询报错。DDL 对三列均为
//! `INTEGER NOT NULL DEFAULT 0`，理论上不存在 NULL，故未保留逐列兜底
//! （保留兜底反而会掩盖列被改成可空这类 schema 漂移）。

use axagent_entities::evolution_execution_stats::{
    ActiveModel, Column, Entity as EvolutionExecutionStats,
};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::workflow_evolution::ToolExecutionStats;
use sea_orm::sea_query::{Alias, Expr, OnConflict};
use sea_orm::{DatabaseConnection, EntityName, EntityTrait, Set};
use std::collections::HashMap;

/// UPSERT 一次执行反馈：`usage_count + 1`，并按成败累计 successes / failures。
///
/// 用增量更新的 `ON CONFLICT ... DO UPDATE`（而非 `INSERT OR REPLACE`，
/// 后者会整行覆盖、丢失既有计数）。
pub async fn upsert_execution_feedback(
    db: &DatabaseConnection,
    conversation_id: &str,
    tool_id: &str,
    success: bool,
) -> Result<()> {
    let (success_inc, failure_inc) = if success {
        (1_i32, 0_i32)
    } else {
        (0_i32, 1_i32)
    };

    let am = ActiveModel {
        conversation_id: Set(conversation_id.to_string()),
        tool_id: Set(tool_id.to_string()),
        usage_count: Set(1),
        successes: Set(success_inc),
        failures: Set(failure_inc),
    };

    // ⚠ `ExprTrait` 必须**在本函数内**导入，不能提到模块级：
    // 它是对「所有类型」的 blanket impl（`impl<T> ExprTrait for T`），模块级导入会让
    // 同模块 `load_all_execution_stats` 里的 `row.usage_count.max(0)` 变成
    // E0034 `multiple applicable items in scope`（`ExprTrait::max` vs `Ord::max`）。
    // 这是编译器实测出来的，不是推测。
    use sea_orm::ExprTrait;

    // ⚠ SET 表达式里引用**现有行**的列必须限定（见文件头「改造记录 1」）。
    // 表名取自实体声明 —— 不手抄字符串，实体改名时此处自动跟随。
    let table = Alias::new(EvolutionExecutionStats.table_name());

    EvolutionExecutionStats::insert(am)
        .on_conflict(
            OnConflict::columns([Column::ConversationId, Column::ToolId])
                .values([
                    (Column::UsageCount, Expr::col((table.clone(), Column::UsageCount)).add(1)),
                    (
                        Column::Successes,
                        Expr::col((table.clone(), Column::Successes)).add(success_inc),
                    ),
                    (
                        Column::Failures,
                        Expr::col((table.clone(), Column::Failures)).add(failure_inc),
                    ),
                ])
                .to_owned(),
        )
        .exec(db)
        .await
        .map(|_| ())
        .map_err(AxAgentError::Database)
}

/// 一次性读取全部持久化的执行统计，按 `conversation_id → tool_id → stats` 组装。
///
/// 启动时调用，把上次会话的真实执行证据加载回 `AppState.evolution_execution_stats`。
/// 空表返回空 HashMap（正常，非错误）。
pub async fn load_all_execution_stats(
    db: &DatabaseConnection,
) -> Result<HashMap<String, HashMap<String, ToolExecutionStats>>> {
    let rows = EvolutionExecutionStats::find().all(db).await.map_err(AxAgentError::Database)?;

    let mut result: HashMap<String, HashMap<String, ToolExecutionStats>> = HashMap::new();
    for row in rows {
        result.entry(row.conversation_id).or_default().insert(
            row.tool_id,
            ToolExecutionStats {
                // 负值钳制到 0：与改造前的 `.max(0) as u32` 语义一致
                usage_count: row.usage_count.max(0) as u32,
                successes: row.successes.max(0) as u32,
                failures: row.failures.max(0) as u32,
            },
        );
    }
    Ok(result)
}
