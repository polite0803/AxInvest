// SPDX-License-Identifier: AGPL-3.0-only
//! 漏检事件表 `mover_recall_events` 的读写收口（补分析批次的数据访问层）。
//!
//! 事件状态机（pending / excluded / running / analyzed / failed / reflected）
//! 的语义归命令层；本模块只提供按状态筛选、认领、回写、批次反查四类原语。

pub use axagent_entities::mover_recall_events::Model as EventModel;
use axagent_entities::mover_recall_events::{self, Column, Entity};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::collections::{HashMap, HashSet};

/// 可进补分析批次的状态（待补 + 上批失败允许跨批重试）。
pub const CANDIDATE_STATUSES: [&str; 2] = ["pending", "failed"];

/// A-2 分链后一条达标事件的行数（荐股链一行、智选链一行）。
pub const CHAIN_ROWS_PER_EVENT: u64 = 2;

/// 允许被终态回写触碰的状态：`running`（本批已认领的两链姊妹行）+ `pending`/`failed`
/// （认领数为 0 即失败的错误分支，行还停在候选态）。
/// 与 `CANDIDATE_STATUSES` 的差别就是 `running` —— 认领之后行已经不是候选态了。
pub const FINISHABLE_STATUSES: [&str; 3] = ["running", "pending", "failed"];

/// 事件的**工作单元键**：票 × 窗口右端日 × 档。
///
/// A-2 起同一键在两条链上各有一行（归因层可以不同），但「把这只票在这一窗补分析一次」
/// 只做一次 ⇒ 选样按三键去重，认领/回写/盖章按三键扩散到姊妹行。
/// 漏做扩散的症状不是报错，而是：一条链补完变 analyzed，姊妹行永远留在 pending 被下一轮再补一遍。
fn event_key(m: &EventModel) -> (String, String, String) {
    (m.stock_code.clone(), m.anchor_date.clone(), m.period.clone())
}

/// 按三键去重，保留**先出现**的那行（调用方已按累计涨幅降序 + 创建时间升序排过）。
/// 纯函数，便于不连库验「两行同键只剩一行、不同键都留着」。
pub fn dedupe_by_event_key(rows: Vec<EventModel>) -> Vec<EventModel> {
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    rows.into_iter().filter(|r| seen.insert(event_key(r))).collect()
}

/// 把一组事件行 id 扩成它们各自的**姊妹行**（同三键、跨链）id 集合。
///
/// 三个 `is_in` 条件的交集是笛卡尔积，会多取到「同票同日但不同档」这类行 ⇒
/// 取出后必须用 `wanted` 精确回筛，不能直接把 SQL 结果当集合用。
async fn expand_to_siblings(db: &DatabaseConnection, ids: &[String]) -> Result<Vec<String>, DbErr> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let anchors = Entity::find().filter(Column::Id.is_in(ids.to_vec())).all(db).await?;
    if anchors.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: HashSet<(String, String, String)> = anchors.iter().map(event_key).collect();
    let rows = Entity::find()
        .filter(
            Column::StockCode
                .is_in(anchors.iter().map(|r| r.stock_code.clone()).collect::<Vec<_>>()),
        )
        .filter(
            Column::AnchorDate
                .is_in(anchors.iter().map(|r| r.anchor_date.clone()).collect::<Vec<_>>()),
        )
        .filter(Column::Period.is_in(anchors.iter().map(|r| r.period.clone()).collect::<Vec<_>>()))
        .all(db)
        .await?;
    Ok(rows.into_iter().filter(|r| wanted.contains(&event_key(r))).map(|r| r.id).collect())
}

/// 取补分析候选事件：pending/failed，按累计涨幅降序（先补最妖的），
/// 同分按创建时间升序（先老后新）。`run_id` 为 None 时跨全部轮次选样。
pub async fn list_candidate_events(
    db: &DatabaseConnection,
    run_id: Option<&str>,
    limit: u64,
) -> Result<Vec<mover_recall_events::Model>, DbErr> {
    let mut query = Entity::find()
        .filter(Column::Status.is_in(CANDIDATE_STATUSES))
        .order_by_desc(Column::CumGainPct)
        .order_by_asc(Column::CreatedAtMs);
    if let Some(run_id) = run_id.filter(|s| !s.is_empty()) {
        query = query.filter(Column::RunId.eq(run_id.to_owned()));
    }
    // 每事件 `CHAIN_ROWS_PER_EVENT` 行 ⇒ 先按行数放大取，再按三键去重、截回配额，
    // 否则配额会在两链之间对半砍（10 只变 5 只）而面板仍显示 10。
    let rows = query.limit(limit.saturating_mul(CHAIN_ROWS_PER_EVENT)).all(db).await?;
    let mut picked = dedupe_by_event_key(rows);
    // 行数上限已被 `limit × 链数` 约束，这里的收窄不会截到 usize 边界以外
    picked.truncate(limit as usize);
    Ok(picked)
}

/// 从各轮核查的 `rates_json` 提取 unexplained 占比：run_id → 占比。
///
/// 占比为 null（NaN，如无漏检）或缺字段时**缺键**；调用方据此判「无信息不拦截」。
/// runs 表为周频小表，直接全量读（避免在 dao 层依赖 analysis-engine 的 DTO，
/// JSON 字段名 `unexplainedShare` 与 `RecallRates` 的 camelCase 序列化对齐）。
pub async fn load_run_unexplained_shares(
    db: &DatabaseConnection,
) -> Result<HashMap<String, f64>, DbErr> {
    let rows = axagent_entities::mover_recall_runs::Entity::find().all(db).await?;
    let mut out = HashMap::new();
    for r in rows {
        // rates_json 为非空 String 列（空率时存 null 字面量的 JSON）
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.rates_json)
            && let Some(share) = v.get("unexplainedShare").and_then(|x| x.as_f64())
        {
            out.insert(r.id, share);
        }
    }
    Ok(out)
}

/// 认领事件：pending/failed → running 并打批次标。仅认领仍处于候选态的行
/// （防并发/重入下重复认领），返回受影响行数。
pub async fn claim_events(
    db: &DatabaseConnection,
    event_ids: &[String],
    batch_id: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    if event_ids.is_empty() {
        return Ok(0);
    }
    // 认领按三键扩散到姊妹行：两链的两行一起进 running，补分析才不会被下一轮重复认领
    let event_ids = expand_to_siblings(db, event_ids).await?;
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("running"))
        .col_expr(Column::BatchId, Expr::value(batch_id.to_owned()))
        .col_expr(Column::ErrorDetail, Expr::value::<Option<String>>(None))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.is_in(event_ids.to_vec()))
        .filter(Column::Status.is_in(CANDIDATE_STATUSES))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 反思判定「本应入池」⇒ 把那条事件盖成 `reflow_pending`（D-4 清单半段）。
///
/// 只在 `analyzed` / `reflected` 上盖章：`pending`/`running` 还没被分析，拿它当回灌依据
/// 等于用「还没看过的票」喂池子。返回受影响行数（0 = 状态不符，调用方据此留日志）。
pub async fn mark_reflow_pending(
    db: &DatabaseConnection,
    event_id: &str,
    note: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("reflow_pending"))
        .col_expr(Column::ReflowNote, Expr::value(note.to_owned()))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.eq(event_id.to_owned()))
        .filter(Column::Status.is_in(["analyzed", "reflected"]))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 最近一轮**荐股链**核查里「池子没收」的票（`not_in_pool` / `unexplained`）。
///
/// 这两层才是「已被市场验证该涨、而候选池没看见」的反证样本；`l3_scored_out` 是评分算过并主动淘汰的，
/// 再回灌只会与评分口径打架；`other_period` / `by_design` / `pool_degraded` 都不属于「池子漏了它」。
/// 按涨幅降序、按票去重，供荐股扫描前并入候选池。
pub async fn latest_reco_miss_codes(
    db: &DatabaseConnection,
    limit: usize,
) -> Result<Vec<(String, String)>, DbErr> {
    use axagent_entities::mover_recall_runs;
    let Some(run) = mover_recall_runs::Entity::find()
        .filter(mover_recall_runs::Column::Chain.eq("reco"))
        .order_by_desc(mover_recall_runs::Column::CreatedAtMs)
        .one(db)
        .await?
    else {
        return Ok(Vec::new());
    };
    let rows = Entity::find()
        .filter(Column::RunId.eq(run.id))
        .filter(Column::MissLayer.is_in(["not_in_pool", "unexplained"]))
        .order_by_desc(Column::CumGainPct)
        .all(db)
        .await?;
    let mut seen = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|r| seen.insert(r.stock_code.clone()))
        .take(limit)
        .map(|r| (r.stock_code, r.stock_name))
        .collect())
}

/// 回灌候选里**已有结算样本**的只数（`decision_validations` 里出现过该票）。
///
/// 这是转正判据的「样本量」输入：没有结算就没有后验，任何转正都只是偏好。
/// ⚠ 只数**实盘代**（`as_of_date IS NULL`）：回放批次的结算行带截止日，混进来会让
/// 「样本够了」被一次回放的量凑出来，而转正后跑的是实盘 —— 与权重闭环、
/// `list_decision_validations` 同一代际口径。
pub async fn count_settled_reflow(
    db: &DatabaseConnection,
    codes: &[String],
) -> Result<usize, DbErr> {
    if codes.is_empty() {
        return Ok(0);
    }
    let rows = axagent_entities::decision_validations::Entity::find()
        .filter(axagent_entities::decision_validations::Column::AsOfDate.is_null())
        .filter(axagent_entities::decision_validations::Column::StockCode.is_in(codes.to_vec()))
        .all(db)
        .await?;
    let settled: std::collections::HashSet<&str> =
        rows.iter().map(|r| r.stock_code.as_str()).collect();
    Ok(settled.len())
}

/// 回灌清单（跨轮次，最新在前）：尚未被消费的 `reflow_pending` 事件。
/// 只读、不改状态 —— 消费在 `refresh_mover_seed`，且必须过 fail-closed 判据。
pub async fn list_reflow_candidates(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<EventModel>, DbErr> {
    Entity::find()
        .filter(Column::Status.eq("reflow_pending"))
        .order_by_desc(Column::UpdatedAtMs)
        .limit(limit)
        .all(db)
        .await
}
/// 单只事件终态回写（analyzed 带 analysis_id；failed 带 error_detail）。
///
/// ⚠ 终态行不得被回写：A-2 的姊妹扩散按**三键**跨轮找行，同三键在旧轮里可能已经是
/// `reflected`/`analyzed`/`excluded`。少了这道状态筛，新一轮批次会把旧轮的 `reflected`
/// 降回 `analyzed`（`reflection_id` 却留在行上）——账本变成「有反思 id 但状态说没反思过」，
/// 补分析面板按 status 的计数也会无因移动。
#[allow(clippy::too_many_arguments)]
pub async fn finish_event(
    db: &DatabaseConnection,
    event_id: &str,
    status: &str,
    analysis_id: Option<&str>,
    error_detail: Option<&str>,
    now_ms: i64,
) -> Result<(), DbErr> {
    let siblings = expand_to_siblings(db, &[event_id.to_owned()]).await?;
    Entity::update_many()
        .col_expr(Column::Status, Expr::value(status.to_owned()))
        .col_expr(Column::AnalysisId, Expr::value(analysis_id.map(str::to_owned)))
        .col_expr(Column::ErrorDetail, Expr::value(error_detail.map(str::to_owned)))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.is_in(siblings))
        .filter(Column::Status.is_in(FINISHABLE_STATUSES))
        .exec(db)
        .await?;
    Ok(())
}

/// 批次异常中断后释放其仍在 running 的事件：回 pending，清批次标，
/// 使其可被后续批次重新认领。
pub async fn release_running_events(
    db: &DatabaseConnection,
    batch_id: &str,
    error_detail: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("pending"))
        .col_expr(Column::BatchId, Expr::value::<Option<String>>(None))
        .col_expr(Column::ErrorDetail, Expr::value(error_detail.to_owned()))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::BatchId.eq(batch_id.to_owned()))
        .filter(Column::Status.eq("running"))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 按主键取单个事件（反思工作流按补分析行盖章的 `anchor_event_id` 反查用）。
pub async fn fetch_by_id(
    db: &DatabaseConnection,
    event_id: &str,
) -> Result<Option<mover_recall_events::Model>, DbErr> {
    Entity::find_by_id(event_id.to_owned()).one(db).await
}

/// 三键（stock_code + anchor_date + period）定位事件，同三键跨多轮核查时
/// 取最新一轮（created_at_ms DESC）；查无返回 `Ok(None)`。
///
/// `chain = Some(c)` 优先取该链那一行（A-2 起同一三键两链各一行，归因层可以不同 ⇒
/// 面板展开行显示的层必须与它上面那一行同链）；该链没有行时**回退到任意链的最近一行**
/// —— 回退只用于分析/批次状态这些两链同值的字段，调用方不得把回退行的 `miss_layer`
/// 当作指定链的结论。
pub async fn find_latest_by_key(
    db: &DatabaseConnection,
    stock_code: &str,
    anchor_date: &str,
    period: &str,
    chain: Option<&str>,
) -> Result<Option<mover_recall_events::Model>, DbErr> {
    if let Some(c) = chain {
        let exact = Entity::find()
            .filter(Column::StockCode.eq(stock_code.to_owned()))
            .filter(Column::AnchorDate.eq(anchor_date.to_owned()))
            .filter(Column::Period.eq(period.to_owned()))
            .filter(Column::Chain.eq(c.to_owned()))
            .order_by_desc(Column::CreatedAtMs)
            .one(db)
            .await?;
        if exact.is_some() {
            return Ok(exact);
        }
    }
    Entity::find()
        .filter(Column::StockCode.eq(stock_code.to_owned()))
        .filter(Column::AnchorDate.eq(anchor_date.to_owned()))
        .filter(Column::Period.eq(period.to_owned()))
        .order_by_desc(Column::CreatedAtMs)
        .one(db)
        .await
}

/// 漏检事件闭环盖章：置 reflected 并记 reflection_id。带 `status != 'reflected'`
/// 行级守卫 —— 一个事件多档反思时只让首个 completed 的档生效，返回 0 表示
/// 已被更早的档盖章（幂等，不是冲突）。
pub async fn stamp_reflected(
    db: &DatabaseConnection,
    event_id: &str,
    reflection_id: Option<&str>,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let siblings = expand_to_siblings(db, &[event_id.to_owned()]).await?;
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("reflected"))
        .col_expr(Column::ReflectionId, Expr::value(reflection_id.map(str::to_owned)))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.is_in(siblings))
        .filter(Column::Status.ne("reflected"))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 反查某批次的逐票明细（明细抽屉）：累计涨幅降序。
pub async fn list_by_batch(
    db: &DatabaseConnection,
    batch_id: &str,
) -> Result<Vec<mover_recall_events::Model>, DbErr> {
    Entity::find()
        .filter(Column::BatchId.eq(batch_id.to_owned()))
        .order_by_desc(Column::CumGainPct)
        .all(db)
        .await
}

/// 统计某批次各状态事件数（批次表计数的校验口径，测试用）。
pub async fn count_by_batch_status(
    db: &DatabaseConnection,
    batch_id: &str,
    status: &str,
) -> Result<u64, DbErr> {
    Entity::find()
        .filter(Column::BatchId.eq(batch_id.to_owned()))
        .filter(Column::Status.eq(status.to_owned()))
        .count(db)
        .await
}

#[cfg(test)]
mod event_key_tests {
    use super::*;

    fn row(id: &str, code: &str, anchor: &str, period: &str, status: &str) -> EventModel {
        EventModel {
            id: id.to_string(),
            run_id: "mrr-1".to_string(),
            stock_code: code.to_string(),
            stock_name: format!("股{}", code),
            period: period.to_string(),
            anchor_date: anchor.to_string(),
            window_start: "2026-09-30".to_string(),
            window_end: anchor.to_string(),
            window_days: 2,
            cum_gain_pct: 20.0,
            max_daily_pct: 10.0,
            threshold_pct: 10.0,
            market_type: "main_sz".to_string(),
            chain: Some("reco".to_string()),
            miss_layer: "not_in_pool".to_string(),
            picked_any_period: 0,
            scored_out_styles_json: None,
            batch_id: None,
            analysis_id: None,
            reflection_id: None,
            status: status.to_string(),
            error_detail: None,
            reflow_note: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    /// A-2：一条达标事件两链各一行 ⇒ 选样只剩一行（否则同一只票被补分析两遍）。
    #[test]
    fn dedupe_keeps_one_row_per_event_key() {
        let rows = vec![
            row("e1", "002696", "2026-10-09", "ultra_short", "pending"),
            row("e2", "002696", "2026-10-09", "ultra_short", "pending"),
            row("e3", "002696", "2026-10-09", "short", "pending"),
        ];
        let kept: Vec<String> = dedupe_by_event_key(rows).into_iter().map(|r| r.id).collect();
        assert_eq!(kept, vec!["e1".to_string(), "e3".to_string()], "同三键只留先出现的一行");
    }

    /// 负控：换档/换右端日都必须各自保留（把去重写成「按票去重」会吞掉同票的多窗事件）。
    #[test]
    fn dedupe_does_not_collapse_different_windows() {
        let rows = vec![
            row("e1", "002696", "2026-10-09", "ultra_short", "pending"),
            row("e2", "002696", "2026-10-16", "ultra_short", "pending"),
        ];
        assert_eq!(dedupe_by_event_key(rows).len(), 2);
    }

    /// 状态值域的权威在实体文档（`crates/entities/src/mover_recall_events.rs` 的「状态机」清单），
    /// 这里**现场取词**而不是抄一份：抄来的清单不会在声明加状态时变红。
    fn declared_statuses(src: &str) -> Vec<String> {
        src.lines()
            .filter_map(|l| l.strip_prefix("/// - `"))
            .map(|rest| rest[..rest.find('`').unwrap_or(rest.len())].to_string())
            .collect()
    }

    /// A-2 姊妹扩散跨轮 ⇒ `finish_event` 的状态筛是唯一防线。
    /// 本门钉「可回写侧 ⊆ 声明值域」且「终态/旁路三态一律不可回写」，值域漂移即红。
    #[test]
    fn finishable_guard_partitions_the_declared_status_domain() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../entities/src/mover_recall_events.rs");
        let entity_src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
        let domain = declared_statuses(&entity_src);
        assert_eq!(domain.len(), 6, "状态值域应取到 6 条，实得 {domain:?} ⇒ 取词姿势失效（恒绿）");
        let declared = |s: &str| domain.iter().any(|d| d.as_str() == s);
        for s in ["excluded", "pending", "running", "analyzed", "failed", "reflected"] {
            assert!(declared(s), "声明里少了状态 {s}");
        }
        for s in FINISHABLE_STATUSES {
            assert!(declared(s), "可回写侧写了声明外的状态：{s}");
        }
        for s in ["excluded", "analyzed", "reflected"] {
            assert!(
                !FINISHABLE_STATUSES.contains(&s),
                "{s} 是终态或旁路，跨轮批次不得把它回写（会把 reflected 降成 analyzed）"
            );
        }

        // 负控拆在取词那一格：删掉声明的一行 ⇒ 必须少一个取值
        let shrunk: String = entity_src
            .lines()
            .filter(|l| !l.starts_with("/// - `reflected`"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(declared_statuses(&shrunk).len(), 5, "负控失效：少一行声明必须少一个状态");
        assert!(!declared_statuses(&shrunk).iter().any(|d| d.as_str() == "reflected"));
    }
}
