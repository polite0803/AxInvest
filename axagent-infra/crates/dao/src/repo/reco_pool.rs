// SPDX-License-Identifier: AGPL-3.0-only
//! 候选池两张表（`reco_picks` / `reco_scan_audit`）的写入 —— **持久化边界的唯一实现**。
//!
//! ## 为什么要从命令层搬下来
//!
//! `commands/stock_analysis.rs::persist_reco_picks` 与 `persist_reco_scan_audit` 原本在
//! 命令层直接构造 `reco_picks::ActiveModel` 并 `insert` ⇒ 命令层出现 `sea_orm::` /
//! `axagent_entities::` 字面量，命中分层门禁
//! [`scripts/check-layer-discipline.mjs`] 规则 1（`commands-no-direct-db`）。
//! 而时间旅行批次链 A 也要落同一张表，从命令层调过去又命中规则 2
//! （`commands-no-sibling-call`）。两处都收敛到 dao 之后，三条入口（命令层手动刷新、
//! 00:00 荐股 cron、时间旅行批次）写的是**同一个实现**，字段不会各写各的。
//!
//! 先例见同目录 `stock_analysis_snapshot.rs` 与 `stock_lesson_queries.rs`（同样是
//! 为这道门做的真下沉）。
//!
//! ## 入参为什么不是 `RecoPick`
//!
//! `RecoPick` 定义在 `axagent-analysis-engine`，而该 crate 依赖本 crate —— dao 反向
//! 收它会成环。故这里定义写入用的纯数据结构，由主 crate 侧做一次性映射；
//! `reco_version`（荐股算法代际）同理由调用方传入，dao 不引用那个常量。
//!
//! ## 代际（`as_of_date`）契约
//!
//! `None` = 实盘批次；`Some(YYYY-MM-DD)` = 某次时间旅行回放的截止日。
//! 2026-10-12 裁定：回放的候选同样落库（不落就没有样本可供反思），实盘与回放靠这一列
//! 分代 ⇒ 只读实盘的下游必须显式带 `as_of_date IS NULL`。
//! 读侧漏加谓词由 `scripts/check-reco-generation.mjs` 拦。

use sea_orm::{ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, Set};

use axagent_entities::{reco_picks, reco_pool_batches, reco_scan_audit};
use axagent_harness::DatabaseConnection;

/// 荐股落库的**用途**值域（`reco_picks` / `reco_pool_batches` / `reco_scan_audit` 三表共用）。
///
/// 与代际（`as_of_date`）是两个问题：代际回答「截止哪一天」，用途回答「谁跑的回放」。
/// 三张表各自的列注释都指向这里，值域只此一处 —— 新增用途必须同时
/// 登记进 `scripts/check-reco-generation.mjs`（G7），否则 live 消费者会误读回放行。
pub mod purpose {
    /// 实盘批次（手动刷新 / cron / 面板）。
    pub const LIVE: &str = "live";
    /// 漏检窗口起点的 as-of 回放（`mover_recall_analysis` 批次内）。
    pub const MOVER_REPLAY: &str = "mover_replay";
    /// 时间旅行批次（用户指定日的回放）。
    pub const TIME_TRAVEL_REPLAY: &str = "time_travel_replay";
    /// 面板手动回放（荐股面板带 `asOfDate` 的那一次）。
    pub const MANUAL_REPLAY: &str = "manual_replay";
    /// 本列引入前的 `as_of_date` 非空行（启动期一次性回填，见
    /// [`backfill_legacy_replay_purpose`]）。
    pub const LEGACY_REPLAY: &str = "legacy_replay";
}

/// 一次落库的**批次键**（`generated_at` / `as_of_date` / `purpose` 三列同源同批）。
///
/// 收成一个结构而不是三个平铺参数：三列必须同进同出（跨代拼证据就是「实盘票 + 回放池」），
/// 平铺时调用方漏传一列只会在库里留下不可读的组合，而编译器不会说话。
pub struct BatchKey<'a> {
    /// 本次运行时刻（差异查询按它取，避免同日多次回放串批）
    pub generated_at: &'a str,
    /// `None` = 实盘；`Some(YYYY-MM-DD)` = 回放截止日
    pub as_of_date: Option<&'a str>,
    /// 用途值域见 [`purpose`]
    pub purpose: &'a str,
}

/// 一条待落库的候选出票（`reco_picks` 的一行）。
pub struct NewRecoPick {
    pub period: String,
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub confidence: i32,
    pub synthetic: bool,
    /// 完整 pick 的 JSON —— `get_cached_recommendation` 靠它还原缓存，
    /// 必须与实时拉取结果 schema 等价，故由调用方序列化后整体传入。
    pub pick_data: Option<String>,
}

/// 落 `reco_picks`，返回写入行数。
///
/// 写失败**不静默**（逐行 warn 带代码/档位/风格）：此前 `let _ = insert(..)` 失败无感知，
/// 前端表现为"无缓存"、定时链路表现为"候选池为空"，两头都查不到根因。
///
/// ## 幂等（R-3g，2026-10-26）
///
/// **同一业务键重复跑批不再累积重复行**：业务键 =
/// `(as_of_date, period, stock_code, style, purpose)`。
///
/// 背景：`reco_picks` 只有主键 `id`（随机 uuid），**无业务唯一约束**
/// ⇒ 同一 `as_of` 重跑一次就多一份行。实测（2026-10-26）：
/// `2026-09-28 / short` 累计 **74 行 / 21 只票（53 行重复）**。
/// 且每日 cron 选的是「最近可用快照日」⇒ **快照未更新的连续多天会反复选到同一个 as_of**
/// ⇒ 重复不是异常路径，是常态。
///
/// 为什么**不**改成「跑批前 DELETE 旧行」：批次中途失败会把旧的可���数据删掉又补不回来。
/// 为什么**不**加 DB 唯一约束：要改表结构（与「声明式 schema 同步」那条线冲突），
/// 且现存重复行会让建索引直接失败。
/// ⇒ 选**应用层同键覆盖**：不动表结构、不删任何行、重复跑批只更新不新增。
///
/// 覆盖时**保留原 `id`**（`id` 被 `get_cached_recommendation` 等按 id 引用）。
pub async fn insert_picks(
    db: &DatabaseConnection,
    picks: &[NewRecoPick],
    seed_pool_json: Option<String>,
    strategy_weights_json: Option<String>,
    batch: &BatchKey<'_>,
    reco_version: Option<i32>,
) -> usize {
    let mut written = 0usize;
    for pick in picks {
        match upsert_pick(
            db,
            pick,
            seed_pool_json.as_deref(),
            strategy_weights_json.as_deref(),
            batch,
            reco_version,
        )
        .await
        {
            Ok(()) => written += 1,
            Err(e) => tracing::warn!(
                "[reco_pool::insert_picks] 写入 reco_picks 失败 ({} {} {}): {e}",
                pick.period,
                pick.stock_code,
                pick.style
            ),
        }
    }
    written
}

/// 落**一行** `reco_picks`：同业务键覆盖（保留既有 `id`），返回成功 / 首个错误。
///
/// 与 [`insert_picks`] 同一份幂等实现，区别只在错误处置：批量入口把失败降级为 warn
/// 并继续写下一行；调用方需要**逐行的结构化失败信息**（趋势智选要把「哪只 / 为什么」
/// 回传前端）时用本函数。
///
/// ⚠ 2026-10-29：趋势智选链此前**绕过**本模块直接 `pick.insert(db)`，每行现造
/// `serenity-{ts_ms}-…` 主键 ⇒ 同业务键第二次落库必撞库级唯一索引
/// `uq_reco_picks_business_key`，表现为「筛选已完成，但结果未能保存到历史记录」。
pub async fn upsert_pick(
    db: &DatabaseConnection,
    pick: &NewRecoPick,
    seed_pool_json: Option<&str>,
    strategy_weights_json: Option<&str>,
    batch: &BatchKey<'_>,
    reco_version: Option<i32>,
) -> Result<(), DbErr> {
    // ── 幂等：先查同业务键的既有行（一次 SELECT，不在循环里 N+1）──
    let existing_id = find_existing_pick_id(
        db,
        batch.as_of_date,
        batch.purpose,
        &pick.period,
        &pick.stock_code,
        &pick.style,
    )
    .await;

    // ⚠ `existing_id` 只能消费一次 ⇒ 先取判据再移动（E0382）
    let is_update = existing_id.is_some();
    let pick_id = existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // 用**闭包**构造 ActiveModel，而不是先造一个值 ——
    // `update`/`insert` 都会**消费** ActiveModel（E0382），
    // 而「撞唯一约束后重试覆盖」需要**再造一次** ⇒ 值形态做不到，闭包可以。
    let build_model = || reco_picks::ActiveModel {
        id: Set(pick_id.clone()),
        generated_at: Set(batch.generated_at.to_string()),
        period: Set(pick.period.clone()),
        stock_code: Set(pick.stock_code.clone()),
        stock_name: Set(pick.stock_name.clone()),
        style: Set(pick.style.clone()),
        confidence: Set(pick.confidence),
        synthetic: Set(i32::from(pick.synthetic)),
        seed_pool_json: Set(seed_pool_json.map(str::to_string)),
        strategy_weights_json: Set(strategy_weights_json.map(str::to_string)),
        pick_data: Set(pick.pick_data.clone()),
        reco_version: Set(reco_version),
        as_of_date: Set(batch.as_of_date.map(|s| s.to_string())),
        purpose: Set(Some(batch.purpose.to_string())),
        created_at: Set(batch.generated_at.to_string()),
    };
    let outcome = match is_update {
        // 同键已存在 ⇒ 覆盖（保留 id），不新增行
        true => build_model().update(db).await.map(|_| ()),
        false => build_model().insert(db).await.map(|_| ()),
    };
    match outcome {
        Ok(_) => Ok(()),
        Err(e) if is_unique_violation(&e) => {
            // ⚠ 唯一约束冲突（2026-10-26 加 `uq_reco_picks_business_key` 之后）：
            //   「先查已有行 → 无 ⇒ INSERT」这条路在**并发**下会撞（两个进程同时查到
            //   无已有行、都去 INSERT）⇒ 第二个撞唯一约束。
            //   撞了**不能只 warn**：那等于静默丢一行荐股票（= 少一个候选）。
            //   正确处置 = 当成「同键已被别人写进」⇒ 重查该键的 id 并改成覆盖（保留 id）。
            //   ⚠ live 荐股 cron **没有全局互斥锁**（只有时间旅行批次有）⇒ 撞约束是现实的。
            match find_existing_pick_id(
                db,
                batch.as_of_date,
                batch.purpose,
                &pick.period,
                &pick.stock_code,
                &pick.style,
            )
            .await
            {
                // 用**已存在行的 id** 覆盖（保留 id ⇒ 下游引用不悬空）。
                // ⚠ 不能用 `..build_model()`（那会把 `id` 字段重复指定 ⇒ 编译错）
                //   ⇒ 先造再改 `id`。
                Some(existing_id) => {
                    let mut retry = build_model();
                    retry.id = Set(existing_id);
                    retry.update(db).await.map(|_| ())
                },
                // 重查不到 ⇒ 原样上抛（不是可自愈的并发冲突）
                None => Err(e),
            }
        },
        Err(e) => Err(e),
    }
}

/// 是不是「唯一约束冲突」（并发写同一业务键）。
///
/// 判据：**按错误消息文本**匹配。⚠ 这比「按 SQLSTATE 23505」弱（依赖 driver 的消息格式），
/// 但本仓的 `sea_orm::DbErr` 不携带结构化的 SQLSTATE 可供匹配，而消息里的
/// `duplicate key value violates unique constraint` 是 PostgreSQL 的**稳定英文措辞**。
/// ⇒ 匹配失败时的兜底是**重新按业务键查一次并覆盖**（不是丢行），所以误判成本低。
fn is_unique_violation(e: &sea_orm::DbErr) -> bool {
    let msg = e.to_string();
    msg.contains("duplicate key value violates unique constraint")
        || msg.contains("duplicate key")
        || msg.contains("UNIQUE constraint failed")
}

/// 查同业务键的既有行 id（`reco_picks` 的幂等键）。
///
/// 键 = `(as_of_date, period, stock_code, style, purpose)`。
/// `as_of_date` / `purpose` 可空 ⇒ 用 `IS NOT DISTINCT FROM` 精确匹配 NULL
/// （`= NULL` 在 SQL 里恒为 NULL，**不能**用来比可空列）。
async fn find_existing_pick_id(
    db: &DatabaseConnection,
    as_of_date: Option<&str>,
    purpose: &str,
    period: &str,
    stock_code: &str,
    style: &str,
) -> Option<String> {
    use sea_orm::ColumnTrait;

    // ⚠ `as_of_date` 可空 ⇒ 必须**二选一**（`is_null()` 或 `eq(X)`），
    // 绝不能写成 `is_null().or(eq(X))` —— 那会让 as_of 为 NULL 的**实盘行**
    // 也被回放行匹配上 ⇒ 回放覆盖实盘数据。
    // （SQL 里 `= NULL` 恒为 NULL，不能用 `eq` 比可空列的空值。）
    let as_of_filter = match as_of_date {
        Some(d) => reco_picks::Column::AsOfDate.eq(d),
        None => reco_picks::Column::AsOfDate.is_null(),
    };
    let purpose_filter = match purpose {
        "" => reco_picks::Column::Purpose.is_null(),
        p => reco_picks::Column::Purpose.eq(p),
    };

    reco_picks::Entity::find()
        .filter(as_of_filter)
        .filter(purpose_filter)
        .filter(reco_picks::Column::Period.eq(period))
        .filter(reco_picks::Column::StockCode.eq(stock_code))
        .filter(reco_picks::Column::Style.eq(style))
        .order_by_desc(reco_picks::Column::CreatedAt)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|m| m.id)
}

/// 一条 L3 截断留痕（算出该票、但在组内 top-N 被截掉）。
pub struct NewScanAuditTrim {
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub confidence: i32,
    pub rank: i32,
}

/// 落 `reco_scan_audit`，返回写入行数。
///
/// 语义边界见 `axagent_entities::reco_scan_audit` 的文档：**无行**不等于该风格把票选出去了。
/// 与同批 picks 共用同一个 `generated_at`（批次键）与同一个 `as_of_date`（代际），
/// 否则漏检归因会出现「实盘的票 + 回放的留痕」这种跨代拼出来的证据。
pub async fn insert_scan_audit(
    db: &DatabaseConnection,
    trimmed: &[NewScanAuditTrim],
    period: &str,
    batch: &BatchKey<'_>,
) -> usize {
    let mut written = 0usize;
    for t in trimmed {
        let am = reco_scan_audit::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            generated_at: Set(batch.generated_at.to_string()),
            period: Set(period.to_string()),
            stock_code: Set(t.stock_code.clone()),
            stock_name: Set(t.stock_name.clone()),
            style: Set(t.style.clone()),
            confidence: Set(t.confidence),
            rank: Set(t.rank),
            as_of_date: Set(batch.as_of_date.map(|s| s.to_string())),
            purpose: Set(Some(batch.purpose.to_string())),
            created_at: Set(batch.generated_at.to_string()),
        };
        match am.insert(db).await {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!(
                "[reco_pool::insert_scan_audit] 写入失败 ({} {} {}): {e}",
                period,
                t.stock_code,
                t.style
            ),
        }
    }
    written
}

/// 最近一批荐股的候选池代码（`reco_picks.seed_pool_json` 的 `[["code","name"], …]`）。
///
/// 用途（2026-10-20，P-n 的覆盖前提）：每日估值归档要补的正是**荐股扫描的那批票**，
/// 而不是全市场 681 只逐个拉估值（收益与请求量不成比例）。此处直接复用已经落库的池快照，
/// 零额外网络。解析失败按空处理（归档跳过，不影响收盘快照）。
///
/// ⚠ 只读实盘代（`as_of_date IS NULL`）：唯一调用方是收盘钩子 `start_market_close_sweep`，
/// 它归档的是**今天这套池子**的历史估值。回放候选自 2026-10-12 裁定起同表落库，漏掉这一
/// 谓词时「刚跑完一次回放」就会让实盘归档去按历史池选码 —— 读起来像成功，实际补错了票。
pub async fn latest_seed_pool_codes(
    db: &DatabaseConnection,
) -> Result<Vec<String>, sea_orm::DbErr> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let row = reco_picks::Entity::find()
        .filter(reco_picks::Column::SeedPoolJson.ne(""))
        .filter(reco_picks::Column::AsOfDate.is_null())
        .order_by_desc(reco_picks::Column::GeneratedAt)
        .one(db)
        .await?;
    let Some(r) = row else {
        return Ok(Vec::new());
    };
    let Some(raw) = r.seed_pool_json.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let pairs: Vec<(String, String)> = serde_json::from_str(raw).unwrap_or_default();
    Ok(pairs.into_iter().map(|(code, _)| code).collect())
}

/// 存量回填：把 `purpose` 列引入前、`as_of_date` 非空的行标成 `legacy_replay`。
///
/// 为什么必须回填而不能按「缺省 live」读：这些行是 2026-10-12 裁定之前/之后用手动回放
/// 与时间旅行写下的**回放**候选，当成实盘会让历史面板与 live 消费者把历史池当今天的票。
/// 反过来，`purpose IS NULL AND as_of_date IS NULL` 的行才是真正的旧实盘行 —— 它们
/// 不回填（live 是多数行，写一遍没有信息增量，`NULL` 在读取侧等价 `live`）。
///
/// 幂等：`WHERE purpose IS NULL AND as_of_date IS NOT NULL`，二次执行为 0 行。
/// 返回三表各自受影响行数 `(picks, pool_batches, scan_audit)`。
pub async fn backfill_legacy_replay_purpose(
    db: &DatabaseConnection,
) -> Result<(u64, u64, u64), DbErr> {
    use sea_orm::sea_query::Expr;
    let picks = reco_picks::Entity::update_many()
        .col_expr(reco_picks::Column::Purpose, Expr::value(purpose::LEGACY_REPLAY))
        .filter(reco_picks::Column::Purpose.is_null())
        .filter(reco_picks::Column::AsOfDate.is_not_null())
        .exec(db)
        .await?
        .rows_affected;
    let pool = reco_pool_batches::Entity::update_many()
        .col_expr(reco_pool_batches::Column::Purpose, Expr::value(purpose::LEGACY_REPLAY))
        .filter(reco_pool_batches::Column::Purpose.is_null())
        .filter(reco_pool_batches::Column::AsOfDate.is_not_null())
        .exec(db)
        .await?
        .rows_affected;
    let audit = reco_scan_audit::Entity::update_many()
        .col_expr(reco_scan_audit::Column::Purpose, Expr::value(purpose::LEGACY_REPLAY))
        .filter(reco_scan_audit::Column::Purpose.is_null())
        .filter(reco_scan_audit::Column::AsOfDate.is_not_null())
        .exec(db)
        .await?
        .rows_affected;
    Ok((picks, pool, audit))
}

/// 一个候选池批次快照（0 票批次也写）。
pub struct NewPoolBatch {
    pub id: String,
    pub chain: String,
    pub period: String,
    pub generated_at: String,
    pub as_of_date: Option<String>,
    /// 用途值域见 [`purpose`]（`live` / `mover_replay` / `time_travel_replay`）。
    pub purpose: String,
    pub seed_pool_json: Option<String>,
    pub stock_count: i32,
    pub degraded: bool,
    pub created_at_ms: i64,
}

/// 落一行候选池批次快照。
pub async fn insert_pool_batch(db: &DatabaseConnection, batch: &NewPoolBatch) -> Result<(), DbErr> {
    let am = reco_pool_batches::ActiveModel {
        id: Set(batch.id.clone()),
        chain: Set(batch.chain.clone()),
        period: Set(batch.period.clone()),
        generated_at: Set(batch.generated_at.clone()),
        as_of_date: Set(batch.as_of_date.clone()),
        purpose: Set(Some(batch.purpose.clone())),
        seed_pool_json: Set(batch.seed_pool_json.clone()),
        stock_count: Set(batch.stock_count),
        degraded: Set(batch.degraded),
        created_at_ms: Set(batch.created_at_ms),
    };
    reco_pool_batches::Entity::insert(am).exec(db).await?;
    Ok(())
}

/// 读某链在 `[from, to]` 内的候选池批次快照（代际显式）。
///
/// `None` 只读实盘代；`Some(YYYY-MM-DD)` 只读该次回放的批次 —— 与
/// `mover_recall::load_picks_in_range` 同一条代际谓词。
pub async fn load_pool_batches(
    db: &DatabaseConnection,
    chain: &str,
    from: &str,
    to: &str,
    as_of_date: Option<&str>,
) -> Result<Vec<reco_pool_batches::Model>, DbErr> {
    reco_pool_batches::Entity::find()
        .filter(reco_pool_batches::Column::Chain.eq(chain.to_string()))
        .filter(reco_pool_batches::Column::GeneratedAt.gte(from.to_string()))
        .filter(reco_pool_batches::Column::GeneratedAt.lte(format!("{to}T23:59:59.999")))
        .filter(match as_of_date {
            Some(d) => reco_pool_batches::Column::AsOfDate.eq(d.to_string()),
            None => reco_pool_batches::Column::AsOfDate.is_null(),
        })
        .order_by_asc(reco_pool_batches::Column::GeneratedAt)
        .all(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建库走**与生产同一条**建表链（理由见 `db::create_test_pool` 处的改判说明）。
    async fn setup() -> DatabaseConnection {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        handle.conn.clone()
    }

    /// 用生产写入口落一行，池快照按 `[["code","name"], …]` 的生产形态给值。
    async fn insert_pool_row(
        db: &DatabaseConnection,
        generated_at: &str,
        pool_codes: &[&str],
        as_of_date: Option<&str>,
    ) {
        let pairs: Vec<[String; 2]> =
            pool_codes.iter().map(|c| [(*c).to_string(), format!("{c} 名")]).collect();
        let seed = serde_json::to_string(&pairs).expect("池快照应可序列化");
        let pick = NewRecoPick {
            period: "mid".to_string(),
            stock_code: pool_codes.first().copied().unwrap_or("000000").to_string(),
            stock_name: "测试票".to_string(),
            style: "trend".to_string(),
            confidence: 80,
            synthetic: false,
            pick_data: None,
        };
        let batch = BatchKey { generated_at, as_of_date, purpose: purpose::LIVE };
        let written = insert_picks(db, &[pick], Some(seed), None, &batch, Some(1)).await;
        assert_eq!(written, 1, "候选行应写入成功（写失败只 warn，不断言就会让夹具静默空表）");
    }

    /// 只认实盘代：回放批次即使**更晚入库**，也不得被当成「最近一批」送去归档估值。
    ///
    /// 两侧都有电 —— 撤掉 `AsOfDate.is_null()` 则选中回放池（`000001`），
    /// 谓词写成 `is_not_null()` 则实盘行读不到、返回空。
    #[tokio::test]
    async fn latest_seed_pool_codes_skips_replay_generation() {
        let db = setup().await;
        insert_pool_row(&db, "2026-09-01T15:00:00Z", &["600000"], None).await;
        insert_pool_row(&db, "2026-10-05T15:00:00Z", &["000001"], Some("2026-07-21")).await;

        let codes = latest_seed_pool_codes(&db).await.expect("读取应成功");
        assert_eq!(codes, vec!["600000".to_string()]);
    }

    /// 库里只剩回放代 ⇒ 返回空（收盘钩子据此走「池快照为空，跳过」，不冒充归档成功）。
    #[tokio::test]
    async fn latest_seed_pool_codes_returns_empty_when_only_replay_generation_exists() {
        let db = setup().await;
        insert_pool_row(&db, "2026-10-05T15:00:00Z", &["000001"], Some("2026-07-21")).await;

        assert!(latest_seed_pool_codes(&db).await.expect("读取应成功").is_empty());
    }

    /// 0 票批次也要留痕；读侧按代际隔离（实盘查询不得读到回放批次）。
    #[tokio::test]
    async fn pool_batches_keep_empty_batches_and_respect_generation() {
        let db = setup().await;
        let live_empty = NewPoolBatch {
            id: "rpb-live-empty".to_string(),
            chain: "reco".to_string(),
            period: "mid".to_string(),
            generated_at: "2026-10-05T15:00:00Z".to_string(),
            as_of_date: None,
            purpose: purpose::LIVE.to_string(),
            seed_pool_json: Some("[]".to_string()),
            stock_count: 0,
            degraded: true,
            created_at_ms: 1,
        };
        insert_pool_batch(&db, &live_empty).await.expect("0 票批次应可写入");
        let replay = NewPoolBatch {
            id: "rpb-replay".to_string(),
            chain: "reco".to_string(),
            period: "mid".to_string(),
            generated_at: "2026-10-05T15:00:01Z".to_string(),
            as_of_date: Some("2026-07-21".to_string()),
            purpose: purpose::MOVER_REPLAY.to_string(),
            seed_pool_json: Some(r#"[["600000","n"]]"#.to_string()),
            stock_count: 1,
            degraded: false,
            created_at_ms: 2,
        };
        insert_pool_batch(&db, &replay).await.expect("回放批次应可写入");

        let live = load_pool_batches(&db, "reco", "2026-10-01", "2026-10-31", None)
            .await
            .expect("读实盘批次应成功");
        assert_eq!(live.len(), 1, "实盘查询不得读到回放批次");
        assert_eq!(live[0].id, "rpb-live-empty");
        assert!(live[0].degraded, "空池批次必须标降级");
        assert_eq!(live[0].stock_count, 0);

        let replay_rows =
            load_pool_batches(&db, "reco", "2026-10-01", "2026-10-31", Some("2026-07-21"))
                .await
                .expect("读回放批次应成功");
        assert_eq!(replay_rows.len(), 1);
        assert_eq!(replay_rows[0].id, "rpb-replay");
        assert!(!replay_rows[0].degraded);
    }

    /// 回归（2026-10-29）：同业务键重跑必须**覆盖**，不得再插一行。
    ///
    /// 背景：趋势智选链曾绕过本模块直接 `insert` 现造主键 ⇒ 撞库级唯一索引
    /// `uq_reco_picks_business_key`（键不含 `generated_at`），表现为
    /// 「筛选已完成，但结果未能保存到历史记录」。这里锁住 dao 的正确行为：
    /// 第二次同键写入成功、行数仍为 1、`id` 保留、`generated_at` 更新到新批次。
    #[tokio::test]
    async fn insert_picks_overwrites_same_business_key_and_keeps_id() {
        let db = setup().await;
        insert_pool_row(&db, "2026-10-29T09:00:00Z", &["688498"], None).await;
        let before = reco_picks::Entity::find().all(&db).await.expect("读应成功");
        assert_eq!(before.len(), 1);
        let kept_id = before[0].id.clone();

        let pick = NewRecoPick {
            period: "mid".to_string(),
            stock_code: "688498".to_string(),
            stock_name: "测试票".to_string(),
            style: "trend".to_string(),
            confidence: 90,
            synthetic: false,
            pick_data: None,
        };
        let batch = BatchKey {
            generated_at: "2026-10-29T14:00:00Z",
            as_of_date: None,
            purpose: purpose::LIVE,
        };
        let written =
            insert_picks(&db, &[pick], Some("[]".to_string()), None, &batch, Some(1)).await;
        assert_eq!(written, 1, "同键覆盖应算作成功写入");

        let after = reco_picks::Entity::find().all(&db).await.expect("读应成功");
        assert_eq!(after.len(), 1, "同业务键不得累积重复行");
        assert_eq!(after[0].id, kept_id, "覆盖必须保留原 id（decision_validations 按 id 引用）");
        assert_eq!(after[0].generated_at, "2026-10-29T14:00:00Z");
    }
}
