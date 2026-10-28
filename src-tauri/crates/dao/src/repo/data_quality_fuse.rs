// SPDX-License-Identifier: AGPL-3.0-only
//! 数据质量熔断的**读写下沉实现**（#8 P5，2026-10-06）：观测落库 + 连续异常计数 + 阈值判定。
//!
//! ## 为什么在 dao 而不是命令层
//!
//! 写侧的两个调用点在 `commands/stock_workflow/core.rs` 的两处 blackboard 持久化处，
//! 读侧的调用点在 `commands/stock_workflow/hooks.rs` 的运行变量注入处。三处都要碰
//! `data_quality_observations` 实体与查询组合子，而命令层出现 `sea_orm::` /
//! `axagent_entities::` 字面量即命中分层门禁 `scripts/check-layer-discipline.mjs`
//! 规则 1（`commands-no-direct-db`）⇒ 实体、列、`Condition` 一律留在本模块，
//! 命令层只拿 `Option<String>` / `DqiStreak` 这类纯数据（先例：同目录 `stock_lesson_queries.rs`）。
//!
//! ## 「异常」的边界（唯一一处定义，别在 Rhai / 面板里各写一份）
//!
//! `abnormal` = **该轮数据质量确实降档**，含两种情形：
//!   · grade 是 `C` / `D` / `F` ⇒ 异常（`C` 起才是「一般 / 较差」档）；
//!   · grade 为 `NULL` ⇒ **也算异常**。`data-quality` 节点自身 `continue_on_fail = true`，
//!     它整段没跑成时最容易被读成「没有质量问题记录」= 正常；那正是把「拿不到」冒充「没问题」。
//!
//! ⚠ 2026-10-10 语义调整：原判据是 `grade != "A"`（即 **B 也算异常**），按 #8 的
//!   「B 级跨样本熔断」原话实现。实测后果（生产库 4 轮健在观测全为 B）：
//!   `B` 是脚本里明确定义的「良好」档（A≥85 / B≥65），把它并入「异常」等于**把常态档
//!   当作故障**——只要连续 3 轮没拿到近乎满分的 A（而 A 需 gap_count == 0，
//!   见 `data-quality.rhai` 的量程注释），就弹「决策可信度受限 / 本轮置信度上限按最保守档
//!   收紧」，用最保守档压掉本来可信的决策。用户侧据此判定为**误报**（2026-10-10 反馈）。
//!   现收窄为「C/D/F/缺失才异常」：熔断恢复判别力（只在真的降档时触发），A 与 B 都是
//!   可接受档。**这不是放宽证据面**——C/D/F 与「节点没跑成」仍照旧熔断。
//!   ⚠ 判据改后**已回填历史行**（2026-10-10 当日：6 行 grade ∈ {A,B} 且 `abnormal=1` → 0）。
//!   起初的结论是「不回填」——但那理由只覆盖了 `load_recent_streak`（数**从最新一轮往回的前缀**，
//!   下一轮按新判据落库即可打断前缀 ⇒ 读数自愈）。同模块的 `load_fused_intervals` 读的是**全量**
//!   观测并导出熔断时间线（闭环权重历史降权），旧 B 行的 `abnormal=1` 会合成一段**假熔断段**，
//!   把本就合格的历史轮次误降权，且这段假段**不会**随新行自愈 ⇒ 存量观测必须迁移。
//!   回填只改 `abnormal` 列、不动 `grade`，可逆（原值全为 1）：判据仍在写侧判一次，
//!   只是把新判据**回放**到历史行——读侧依旧只数布尔，不重算 grade。
//!
//! 边界只在写侧判一次并落成 `abnormal` 列，读侧只数布尔 —— 反过来（读侧重算 grade 集合）
//! 会让「异常」的定义一改就把历史行的归属跟着改，跨样本计数就变成会漂移的东西。
//!
//! ## 阈值为什么是代码常量而不是面板变量
//!
//! 面板可调参数要走满五处登记（`seed_variables` / 节点 `input_mapping` / 面板控件与分组 /
//! Rhai `effective_params` / 一致性锁），先例 `stop_vol_mult` 在仓内占了 7+3+3+2+4 处；
//! 漏一处就是「纸面可调」而两链编译门全绿（见 PLAN #705 那条）。熔断阈值目前没有调参需求，
//! 故单点常量在此；真需要可调时按那五点对账补齐，**同时**删掉这里的常量而不是留两份。
//! 注入给工作流的是**已判定结果**（`dqi_streak` / `dqi_fused`），Rhai 与 Switch 都不再
//! 自己比较阈值 ⇒ 判据不会漂到第二处。

use axagent_entities::{data_quality_observations, stock_analyses};
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::SelectStatement;
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set};

/// 观测作用域的现阶段唯一值：一轮分析一条，不区分票、不区分档。
pub const DQI_SCOPE_GLOBAL: &str = "global";

/// 连续异常多少轮即熔断（**唯一权威**，含本轮）。
pub const DQI_FUSE_STREAK: usize = 3;

/// 读 streak 时最多回看多少条观测（去重同一 analysis_id 之后）。
///
/// 为什么要有上限：熔断看的是「最近这一段证据面是不是一直差」，不是全史；无上限会把
/// 半年前的行算进来，且行数只增不减。取 12 = `DQI_FUSE_STREAK` 的 4 倍，够容错一次
/// 重跑/回补把中间行改掉。
pub const DQI_STREAK_LOOKBACK: usize = 12;

/// 该轮是否算异常。入参是 `data-quality` 输出的字母等级原值（`None` = 节点没跑成）。
///
/// `A`（优秀）/ `B`（良好）都算正常档；`C` / `D` / `F` 才算降档。
/// 未识别的字母按异常处理（宁可保守，不静默放行）；`NULL` / 空串同「没跑成」。
pub fn grade_is_abnormal(grade: Option<&str>) -> bool {
    match grade.map(str::trim).filter(|g| !g.is_empty()) {
        // NULL / 空串 = 无从判级 ⇒ 按最坏处理（见模块头「异常的边界」第二条）
        None => true,
        Some(g) => !matches!(g, "A" | "B"),
    }
}

/// 连续异常是否已触发熔断。
pub fn is_fused(consecutive_abnormal: usize) -> bool {
    consecutive_abnormal >= DQI_FUSE_STREAK
}

/// 落一行观测。返回 `false` = 该 analysis_id 已有行而**跳过**（重跑不重复计）。
///
/// 幂等按 `analysis_id` 而不是按「本轮 grade」：同一条分析被重跑两次时，第二次通常只是
/// 补跑失败节点，若照单再落一行，读侧的「连续异常轮数」就会被同一条分析虚增。
pub async fn record_observation(
    db: &DatabaseConnection,
    analysis_id: &str,
    grade: Option<&str>,
    observed_at_ms: i64,
) -> Result<bool, DbErr> {
    let existing = data_quality_observations::Entity::find()
        .filter(data_quality_observations::Column::AnalysisId.eq(analysis_id))
        .one(db)
        .await?;
    if existing.is_some() {
        return Ok(false);
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let abnormal = grade_is_abnormal(grade);
    let active = data_quality_observations::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        scope: Set(DQI_SCOPE_GLOBAL.to_string()),
        // 按档 grade 目前不存在（见实体列文档）⇒ 如实写 NULL，不写「四档通用」
        horizon: Set(None),
        grade: Set(grade.map(str::to_string)),
        abnormal: Set(i32::from(abnormal)),
        analysis_id: Set(analysis_id.to_string()),
        observed_at: Set(observed_at_ms),
        created_at: Set(now_ms),
    };
    data_quality_observations::Entity::insert(active).exec(db).await?;
    Ok(true)
}

/// 观测行在聚合里的最小输入（[`streak_from_rows`] 的入参，纯数据 ⇒ 负控能拆在同一处）。
#[derive(Debug, Clone)]
struct DqiObs {
    analysis_id: String,
    observed_at: i64,
    abnormal: bool,
    /// `false` = 所属分析已删除（孤儿行）⇒ 不参与现行判据，只参与披露。
    alive: bool,
}

/// 连续异常计数（读侧聚合的结果，不是写侧状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DqiStreak {
    /// 从最近一轮往回数，连续异常的轮数（含本轮）。`0` = 最近一轮正常，或**一条观测都没有**。
    pub consecutive_abnormal: usize,
    /// 本次参与聚合的观测数（按 analysis_id 去重后）。
    ///
    /// 与 `consecutive_abnormal` 分开是必须的：`0/0` 要说的是「无观测 ⇒ 熔断判据不可用」，
    /// 而不是「证据面很好」——两者在界面上必须是两句话。
    pub observations: usize,
    /// 因同一 analysis_id 重复而被丢掉的行数（重跑/回补的可见痕迹）。
    pub deduped: usize,
    /// 前缀所覆盖的时间跨度内被剔除的**孤儿**观测行数（所属分析已删除）。
    ///
    /// `0` = 本轮计数没有跨任何空档；`> 0` = 「连续 N 轮」这句话里的 N 跳过了 N 个缺口，
    /// 界面必须点名（见 `consecutive_abnormal_incl_orphans`）。
    pub orphans_skipped: usize,
    /// 同一前缀**把孤儿观测也算进来**时的连续异常轮数。
    ///
    /// 与 `consecutive_abnormal` 的差值才是可决策的事实：
    ///   · `incl > cur` ⇒ 被跳过的都是降档轮，现行判据**低估**了证据面持续恶化的长度；
    ///   · `incl < cur` ⇒ 被跳过的里有正常档把前缀断开，现行计数**虚高**（实测 2026-10-28：
    ///     库里 `14:29:51` 一条 B 级孤儿行本可把前缀断在 2，剔除后读出 9）；
    ///   · 相等 ⇒ 跳过与否不影响结论。
    /// 刻意不替判据改口径（剔除仍是判据），只把差值端出来。
    pub consecutive_abnormal_incl_orphans: usize,
}

impl DqiStreak {
    /// 没有任何观测时的取值 —— 刻意不等于「正常」，调用方须按 `observations == 0` 分支。
    pub const EMPTY: DqiStreak = DqiStreak {
        consecutive_abnormal: 0,
        observations: 0,
        deduped: 0,
        orphans_skipped: 0,
        consecutive_abnormal_incl_orphans: 0,
    };

    pub fn fused(&self) -> bool {
        self.observations > 0 && is_fused(self.consecutive_abnormal)
    }
}

/// 观测行必须仍对应一条现存分析 —— 不参与**判据**。
///
/// ## 为什么读侧过滤而不是删除时级联
///
/// ① **存量即刻失效**：加过滤当轮就排除历史孤儿行，免迁移；删除命令级联只对**将来**的删除生效。
/// ② **覆盖所有删除路径**（含未来新增的删除入口），级联只能钉住已知两条命令
///    （`commands/stock_analysis.rs` 的 `delete_stock_analysis` / `batch_delete_stock_analyses`）。
/// ③ **只改「行是否参与」**，不重算 `abnormal` 归属 —— 与模块头「边界只在写侧判一次」的纪律一致。
///
/// ⚠ 2026-10-28 订正：此前本节声称「剔除孤儿行不会把某条真实轮次算少」，**不成立**。
/// 孤儿行对应的轮**确实跑过**，只是结论被删了；把它从前缀里摘掉会让「连续 N 轮」跨过
/// 一个空档继续数下去 ⇒ N 可高可低（实测被剔除的一条 B 级正常档本应打断前缀）。
/// 判据口径**保持剔除**（③ 的理由仍然成立，且删除的轮不该继续代表任何现存结论），
/// 但被跳过的条数与「计入时的连续轮数」必须由 [`streak_from_rows`] 一并算出并注入，
/// 由熔断文案点名 ⇒ 见 `orphans_skipped` / `consecutive_abnormal_incl_orphans`。
fn alive_analysis_ids() -> SelectStatement {
    stock_analyses::Entity::find().select_only().column(stock_analyses::Column::Id).into_query()
}

/// 由观测行算连续异常前缀（纯函数，不碰库；入参顺序无关，内部自行排序）。
///
/// 抽成纯函数是为了让负控能拆在同一守卫上：判据与披露两条口径各断一次。
fn streak_from_rows(rows: &[DqiObs]) -> DqiStreak {
    let mut ordered = rows.to_vec();
    ordered.sort_by_key(|row| std::cmp::Reverse(row.observed_at));

    // ① 判据口径：只在**存活行**上去重、数前缀（与 `load_recent_streak` 的历史行为逐位一致）
    let mut alive_distinct: Vec<&DqiObs> = Vec::with_capacity(ordered.len());
    let mut deduped = 0usize;
    for row in &ordered {
        if !row.alive {
            continue;
        }
        if alive_distinct.iter().any(|kept| kept.analysis_id == row.analysis_id) {
            deduped += 1;
            continue;
        }
        alive_distinct.push(row);
    }
    let window = alive_distinct.len().min(DQI_STREAK_LOOKBACK);
    // 前缀连续异常：从最近一轮往回数，遇到第一个正常轮就停
    let mut consecutive_abnormal = 0usize;
    for row in alive_distinct.iter().take(window) {
        if !row.abnormal {
            break;
        }
        consecutive_abnormal += 1;
    }

    // ② 披露口径：判据前缀所覆盖的跨度内，有多少行因「所属分析已删除」被摘掉，
    //    以及**把它们也算进来**时前缀会数到几。无前缀（streak=0）时两者恒 0 ——
    //    没有「连续」可谈，也就没有跨空档一说。
    if consecutive_abnormal == 0 {
        return DqiStreak {
            consecutive_abnormal,
            observations: window,
            deduped,
            orphans_skipped: 0,
            consecutive_abnormal_incl_orphans: 0,
        };
    }
    // 跨度下界 = 这条「连续」声明真正结束的地方：**第一个存活正常档**（判据在此 break），
    // 而不是最后一条被数的异常轮。取后者会漏报「正常档孤儿行嵌在两条存活异常轮之间」
    // 那种形态（实测该形态下计入与否会把 1 轮改成 2 轮，却因跨度太窄而一个字都不披露）。
    // 窗口内一条正常档都没有 ⇒ 下界退化成窗口里最旧那条参与行。
    let span_from = alive_distinct
        .iter()
        .take(window)
        .copied()
        .find(|row| !row.abnormal)
        .or_else(|| alive_distinct.get(window - 1).copied())
        .expect("window >= 1 时必有元素")
        .observed_at;
    let mut seen_orphan_ids: Vec<&str> = Vec::new();
    let mut orphans_skipped = 0usize;
    let mut orphans: Vec<&DqiObs> = Vec::new();
    for row in &ordered {
        if row.alive || row.observed_at < span_from {
            continue;
        }
        if seen_orphan_ids.contains(&row.analysis_id.as_str()) {
            continue;
        }
        seen_orphan_ids.push(row.analysis_id.as_str());
        orphans_skipped += 1;
        orphans.push(row);
    }
    // 计入孤儿行的同一条前缀走法（孤儿参与、也参与断开）
    let mut merged: Vec<&DqiObs> = Vec::with_capacity(alive_distinct.len() + orphans.len());
    merged.extend(orphans);
    merged.extend(alive_distinct.iter().copied());
    merged.sort_by_key(|row| std::cmp::Reverse(row.observed_at));
    let merged_window = merged.len().min(DQI_STREAK_LOOKBACK);
    let mut consecutive_abnormal_incl_orphans = 0usize;
    for row in merged.iter().take(merged_window) {
        if !row.abnormal {
            break;
        }
        consecutive_abnormal_incl_orphans += 1;
    }

    DqiStreak {
        consecutive_abnormal,
        observations: window,
        deduped,
        orphans_skipped,
        consecutive_abnormal_incl_orphans,
    }
}

/// 取最近的观测并数前缀连续异常（判据只认存活行；孤儿行只参与披露，见 [`streak_from_rows`]）。
pub async fn load_recent_streak(db: &DatabaseConnection) -> Result<DqiStreak, DbErr> {
    let alive_rows = data_quality_observations::Entity::find()
        .filter(data_quality_observations::Column::Scope.eq(DQI_SCOPE_GLOBAL))
        .filter(data_quality_observations::Column::AnalysisId.in_subquery(alive_analysis_ids()))
        .all(db)
        .await?;
    let orphan_rows = data_quality_observations::Entity::find()
        .filter(data_quality_observations::Column::Scope.eq(DQI_SCOPE_GLOBAL))
        .filter(data_quality_observations::Column::AnalysisId.not_in_subquery(alive_analysis_ids()))
        .all(db)
        .await?;

    let mut rows: Vec<DqiObs> = Vec::with_capacity(alive_rows.len() + orphan_rows.len());
    for row in &alive_rows {
        rows.push(DqiObs {
            analysis_id: row.analysis_id.clone(),
            observed_at: row.observed_at,
            abnormal: row.abnormal != 0,
            alive: true,
        });
    }
    for row in &orphan_rows {
        rows.push(DqiObs {
            analysis_id: row.analysis_id.clone(),
            observed_at: row.observed_at,
            abnormal: row.abnormal != 0,
            alive: false,
        });
    }
    Ok(streak_from_rows(&rows))
}

// ── 熔断时间线（#8 P5 第三生效面：闭环权重历史降权用）──

/// 一段「数据质量处于熔断态」的时间区间。`to_ms = None` = 到读取时刻仍未解除。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FusedInterval {
    pub from_ms: i64,
    pub to_ms: Option<i64>,
}

impl FusedInterval {
    /// 时刻 `t` 是否落在本段内。区间**左闭右开**：解除那一刻起的分析不再算熔断。
    pub fn covers(&self, t: i64) -> bool {
        t >= self.from_ms && self.to_ms.is_none_or(|end| t < end)
    }
}

/// 由观测序列导出熔断时间线（纯函数，不碰库）。
///
/// 入参 `&[(observed_at_ms, abnormal)]` **必须按时间升序**；乱序会让前缀计数失去意义，
/// 因此调用方（[`load_fused_intervals`]）负责排序，本函数只信任输入 —— 与
/// [`fused_at`] 的三态契约配套：早于**首个熔断段**的时刻一律返回 `None`（不知道，不是正常）；
/// 从未熔断过 ⇒ 时间线为空 ⇒ 所有时刻都是 `None`（本线不介入，而不是给全历史发清白证书）。
pub fn fused_intervals(observations: &[(i64, bool)]) -> Vec<FusedInterval> {
    let mut out = Vec::new();
    let mut streak = 0usize;
    let mut open: Option<FusedInterval> = None;
    for (at, abnormal) in observations {
        if *abnormal {
            streak += 1;
            // 只在"刚跨阈"时开段；后续异常轮延长同一段（不重复开）
            if open.is_none() && is_fused(streak) {
                open = Some(FusedInterval { from_ms: *at, to_ms: None });
            }
        } else {
            streak = 0;
            if let Some(mut iv) = open.take() {
                iv.to_ms = Some(*at);
                out.push(iv);
            }
        }
    }
    if let Some(iv) = open.take() {
        out.push(iv);
    }
    out
}

/// 某时刻是否处于熔断态；`None` = 观测面**还没给出过任何熔断证据**（表为空，或该时刻早于首个熔断段）。
///
/// 三态而不是二态是刻意的：降权线拿 `None` 当「本条样本不参与判定」，
/// 若把「没有观测」压成 `false`，就等于让「设施没跑过」给策略发一张清白证明。
/// 反过来也不给：本函数只**正向认定**「确实出生于已知的熔断段」，
/// 熔断前的历史时段一律算「无从判定」而不是「干净」——那段时间没有任何熔断判据可言。
/// 于是本线在首次熔断之前完全不介入，之后才逐段生效；这是保守方向（宁可不动权重）。
pub fn fused_at(intervals: &[FusedInterval], t_ms: i64) -> Option<bool> {
    // 空时间线 ⇒ 观测面完全没有（`first()?` 同时承担"表为空"与下面"早于首个熔断段"两种无从判定）
    let earliest = intervals.first()?.from_ms;
    if t_ms < earliest {
        return None;
    }
    Some(intervals.iter().any(|iv| iv.covers(t_ms)))
}

/// 读全量观测并导出熔断时间线（同一 analysis_id 只认最近一行，与 [`load_recent_streak`] 同口径；
/// 孤儿观测行同样由 [`alive_analysis_ids`] 剔除）。
pub async fn load_fused_intervals(db: &DatabaseConnection) -> Result<Vec<FusedInterval>, DbErr> {
    let rows = data_quality_observations::Entity::find()
        .filter(data_quality_observations::Column::Scope.eq(DQI_SCOPE_GLOBAL))
        .filter(data_quality_observations::Column::AnalysisId.in_subquery(alive_analysis_ids()))
        .order_by_asc(data_quality_observations::Column::ObservedAt)
        .all(db)
        .await?;
    // 升序 ⇒ 同一 analysis_id 的**最后**一次出现才是最近一轮；先前出现的都算重复。
    let mut newest_by_analysis: std::collections::HashMap<&str, (i64, bool)> =
        std::collections::HashMap::new();
    for row in &rows {
        newest_by_analysis.insert(row.analysis_id.as_str(), (row.observed_at, row.abnormal != 0));
    }
    let mut points: Vec<(i64, bool)> = newest_by_analysis.into_values().collect();
    points.sort_by_key(|(at, _)| *at);
    Ok(fused_intervals(&points))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_pool;

    /// 直插一条最简 `stock_analyses` 行（只为让观测行不成为孤儿）。
    ///
    /// 读侧自 [`alive_analysis_ids`] 起对观测行做存在性过滤 ⇒ 夹具必须先造出对应分析，
    /// 否则观测行会被整批剔除、断言恒为 0（这正是「孤儿行不参与」的判据本身）。
    async fn insert_analysis(db: &DatabaseConnection, id: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        let active = stock_analyses::ActiveModel {
            id: Set(id.to_string()),
            stock_code: Set("000001".to_string()),
            stock_name: Set("测试票".to_string()),
            analysis_date: Set("2026-10-09".to_string()),
            provider_id: Set("test".to_string()),
            conversation_id: Set("test-conv".to_string()),
            status: Set("completed".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        stock_analyses::Entity::insert(active).exec(db).await.expect("插入分析行应成功");
    }

    #[test]
    fn abnormal_predicate_treats_missing_grade_as_worst_case() {
        for g in ["A", "B"] {
            assert!(!grade_is_abnormal(Some(g)), "{g} 级是可接受档（2026-10-10 起 B 不再算异常）");
        }
        for g in ["C", "D", "F"] {
            assert!(grade_is_abnormal(Some(g)), "{g} 级必须算异常（C 起才是降档）");
        }
        // 「拿不到」不得冒充「没问题」
        assert!(grade_is_abnormal(None));
        assert!(grade_is_abnormal(Some("")), "空串等价于没跑成");
        assert!(grade_is_abnormal(Some("   ")), "空白不参与 A/B 比较");
        assert!(
            grade_is_abnormal(Some(" b ")),
            "未 trim 的大小写/空白不静默放行（原判据同款严格）"
        );
        // 未知字母按异常处理（宁可保守，不静默放行）
        assert!(grade_is_abnormal(Some("Z")));
    }

    #[test]
    fn fuse_threshold_is_the_single_source_and_inclusive() {
        assert!(!is_fused(DQI_FUSE_STREAK - 1));
        assert!(is_fused(DQI_FUSE_STREAK), "含本轮：第 N 轮即熔断，不是第 N+1 轮");
        assert!(is_fused(DQI_FUSE_STREAK + 5));
        // 空 streak 不等于「已熔断」，也不等于「正常」——由 observations 区分
        assert!(!DqiStreak::EMPTY.fused());
        assert!(
            !DqiStreak {
                consecutive_abnormal: 0,
                observations: 0,
                deduped: 0,
                orphans_skipped: 0,
                consecutive_abnormal_incl_orphans: 0,
            }
            .fused()
        );
        // 披露字段不得顶替判据：只有 consecutive_abnormal 参与 fused
        assert!(
            DqiStreak {
                consecutive_abnormal: DQI_FUSE_STREAK,
                observations: 5,
                deduped: 0,
                orphans_skipped: 3,
                consecutive_abnormal_incl_orphans: 1,
            }
            .fused(),
            "孤儿披露条数不得反过来解除熔断（判据口径不变）"
        );
    }

    #[tokio::test]
    async fn streak_counts_leading_run_and_breaks_on_first_good_grade() {
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        // 期望的**最近→最旧**顺序：F, D, B, C ⇒ 前缀连续异常 = 2（F/D 异常，遇 B 就断）。
        // ⚠ 时间戳必须与这个顺序同向（数组首元素 = 最新一轮）：第一版照数组顺序递增秒数，
        //   于是库里最新一轮是 `B`、前缀只有 1 —— 代码对、夹具反了，红得有价值但报错了对象。
        // ⚠ 2026-10-10 起 `B` 是**正常档** ⇒ 断链那一轮改用 `B`（原夹具用 `A`；用 B 才能同时
        //   锁住「B 不再算异常」这条新语义，否则本用例在旧定义下也会绿）。
        let grades = ["F", "D", "B", "C"];
        for (i, g) in grades.iter().enumerate() {
            let at = 1_700_000_000_000 + (grades.len() - i) as i64;
            insert_analysis(db, &format!("an-{i}")).await;
            let inserted = record_observation(db, &format!("an-{i}"), Some(g), at)
                .await
                .expect("落观测行应成功");
            assert!(inserted, "首次落行不该被去重: {g}");
        }

        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.consecutive_abnormal, 2, "前缀连续异常 = 2，遇正常档 B 断链");
        assert_eq!(s.observations, 4);
        assert_eq!(s.deduped, 0);
        assert!(!s.fused(), "2 < {DQI_FUSE_STREAK} ⇒ 不该熔断");

        // 再来一轮异常 ⇒ 3 轮，跨阈 ⇒ 熔断
        insert_analysis(db, "an-newest").await;
        record_observation(db, "an-newest", Some("D"), 1_800_000_000_000)
            .await
            .expect("追加观测应成功");
        let s2 = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s2.consecutive_abnormal, 3, "新轮在最前 ⇒ 前缀 3");
        assert!(s2.fused());
    }

    #[tokio::test]
    async fn rerun_of_same_analysis_does_not_inflate_streak() {
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        insert_analysis(db, "same-analysis").await;
        record_observation(db, "same-analysis", Some("C"), 1_700_000_000_000)
            .await
            .expect("首条应落库");
        let again = record_observation(db, "same-analysis", Some("F"), 1_800_000_000_000)
            .await
            .expect("重跑记录应成功");
        assert!(!again, "同一条分析重跑不得再落一行（否则连续计数被同一条虚增）");

        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.consecutive_abnormal, 1, "只算一轮");
        assert_eq!(s.observations, 1);
    }

    #[tokio::test]
    async fn legacy_duplicate_rows_are_deduped_on_read_not_dropped() {
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        // 写侧有 analysis_id 幂等守卫 ⇒ 同 id 两行只可能来自守卫之前的存量/回补行。
        // 读侧必须「每个 analysis_id 只认最近一行」，否则一条分析被算成两轮 ⇒ 熔断提前。
        // 这里绕过写入口直插，就是在测那条读侧分支（不测它，这行代码永远没被走过）。
        insert_analysis(db, "legacy-dup").await;
        for (at, grade) in [(1_700_000_000_000_i64, "A"), (1_800_000_000_000_i64, "D")] {
            let active = data_quality_observations::ActiveModel {
                id: Set(uuid::Uuid::new_v4().to_string()),
                scope: Set(DQI_SCOPE_GLOBAL.to_string()),
                horizon: Set(None),
                grade: Set(Some(grade.to_string())),
                abnormal: Set(i32::from(grade_is_abnormal(Some(grade)))),
                analysis_id: Set("legacy-dup".to_string()),
                observed_at: Set(at),
                created_at: Set(at),
            };
            data_quality_observations::Entity::insert(active)
                .exec(db)
                .await
                .expect("直插历史重复行应成功");
        }

        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.deduped, 1, "同一条分析的旧行必须被识别为重复");
        assert_eq!(s.observations, 1, "一条分析只算一轮");
        assert_eq!(s.consecutive_abnormal, 1, "认最近那行（D ⇒ 异常 1），而不是 A+D 两行");
    }

    #[tokio::test]
    async fn missing_grade_is_recorded_as_abnormal_and_streaks_like_the_rest() {
        let handle = create_test_pool().await.expect("测试库可创建");
        let db = &handle.conn;
        for i in 0..DQI_FUSE_STREAK {
            insert_analysis(db, &format!("null-grade-{i}")).await;
            record_observation(db, &format!("null-grade-{i}"), None, 1_700_000_000_000 + i as i64)
                .await
                .expect("NULL grade 也应落行");
        }
        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.consecutive_abnormal, DQI_FUSE_STREAK);
        assert!(s.fused(), "节点反复没跑成 ⇒ 照旧并入异常（不因 B 已归正常档而放行）");
    }

    /// 夹具相对权威常量构造（不写死 3）：前 N-1 轮异常不该开段，第 N 轮才开。
    #[test]
    fn fused_intervals_open_at_threshold_and_close_on_first_good_grade() {
        let base = 1_700_000_000_000_i64;
        let step = 1_000_i64;
        // 前缀 = (STREAK-1) 轮异常 ⇒ 未跨阈；再加一轮 ⇒ 开段；随后一轮 A 级 ⇒ 闭段
        let mut obs: Vec<(i64, bool)> =
            (0..DQI_FUSE_STREAK - 1).map(|i| (base + i as i64 * step, true)).collect();
        let open_at = base + (DQI_FUSE_STREAK - 1) as i64 * step;
        obs.push((open_at, true));
        let close_at = open_at + step;
        obs.push((close_at, false));

        let ivs = fused_intervals(&obs);
        assert_eq!(ivs.len(), 1, "只跨阈一次 ⇒ 一段");
        assert_eq!(ivs[0].from_ms, open_at, "段起点 = 刚跨阈那一轮的观测时刻，不是第一轮异常");
        assert_eq!(ivs[0].to_ms, Some(close_at), "遇到正常轮即解除");

        // 负控：全部低于阈值 ⇒ 一段时间线为空，不能让「没熔断」写成「有熔断记录」
        let low: Vec<(i64, bool)> =
            (0..DQI_FUSE_STREAK - 1).map(|i| (base + i as i64 * step, true)).collect();
        assert!(fused_intervals(&low).is_empty(), "未跨阈 ⇒ 不应产出任何熔断段");
    }

    #[test]
    fn fused_interval_stays_open_until_cleared_and_covers_later_reads() {
        let base = 1_700_000_000_000_i64;
        let obs: Vec<(i64, bool)> = (0..DQI_FUSE_STREAK).map(|i| (base + i as i64, true)).collect();
        let ivs = fused_intervals(&obs);
        assert_eq!(ivs.len(), 1);
        assert_eq!(ivs[0].to_ms, None, "没有正常轮来解除 ⇒ 段必须开着，不能自己收尾");
        // 左闭：跨阈那一刻就算熔断（本轮分析就是用这条观测判的）
        assert!(ivs[0].covers(ivs[0].from_ms));
        // 更晚的时刻仍在段内（读侧未来取数时同样归到这段）
        assert!(ivs[0].covers(ivs[0].from_ms + 10));
    }

    /// 三态契约：`None`（无从判定）既不是「熔断」也不是「正常」——降权线靠它把样本剔出分母。
    #[test]
    fn fused_at_keeps_unobserved_distinct_from_clear() {
        let base = 1_700_000_000_000_i64;
        let obs: Vec<(i64, bool)> = (0..DQI_FUSE_STREAK)
            .map(|i| (base + i as i64 * 10, true))
            .chain([(base + DQI_FUSE_STREAK as i64 * 10, false)])
            .collect();
        let ivs = fused_intervals(&obs);

        assert_eq!(fused_at(&[], base), None, "观测表为空 ⇒ 无从判定，不是「没熔断」");
        assert_eq!(fused_at(&ivs, base - 1), None, "早于首个熔断段 ⇒ 无从判定（不是「已知正常」）");
        assert_eq!(fused_at(&ivs, ivs[0].from_ms), Some(true), "段内 = 熔断");
        assert_eq!(
            fused_at(&ivs, base + DQI_FUSE_STREAK as i64 * 10),
            Some(false),
            "解除后 = 已知正常（这一刻在时间线覆盖之内）"
        );
        assert_eq!(fused_at(&ivs, base + DQI_FUSE_STREAK as i64 * 10 + 1), Some(false));
    }

    /// 孤儿观测行（`analysis_id` 不在 `stock_analyses`）必须在**两个读口**都被剔除。
    ///
    /// 实证来源：生产库 `data_quality_observations` 全表 4 行中有 2 行对应分析已删除，
    /// 却仍计入连续轮数 —— 界面显示「连续 3 轮」而实际健在的只有 2 行。
    #[tokio::test]
    async fn orphan_observation_is_ignored_by_streak_and_timeline() {
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        // 健在轮次：1 正常 + 3 异常（跨阈）。异常档用 `D`（2026-10-10 起 `B` 已是正常档，
        // 用它造异常轮会让本用例的 `consecutive_abnormal` 断言恒为 0）。
        insert_analysis(db, "alive-ok").await;
        record_observation(db, "alive-ok", Some("A"), 1_000).await.expect("落行应成功");
        for (i, id) in ["alive-b1", "alive-b2", "alive-b3"].iter().enumerate() {
            insert_analysis(db, id).await;
            record_observation(db, id, Some("D"), 3_000 + i as i64 * 1_000)
                .await
                .expect("落行应成功");
        }
        // 孤儿行：直插（绕过写入口，模拟「分析已删除但观测行残留」）——刻意插在**更早**，
        // 若不剔除，它会把熔断段起点从 alive-b3 提前到 alive-b1/b2。
        for (at, id) in [(2_000_i64, "orphan-b1"), (2_500_i64, "orphan-b2")] {
            let active = data_quality_observations::ActiveModel {
                id: Set(uuid::Uuid::new_v4().to_string()),
                scope: Set(DQI_SCOPE_GLOBAL.to_string()),
                horizon: Set(None),
                grade: Set(Some("B".to_string())),
                abnormal: Set(1),
                analysis_id: Set(id.to_string()),
                observed_at: Set(at),
                created_at: Set(at),
            };
            data_quality_observations::Entity::insert(active)
                .exec(db)
                .await
                .expect("直插孤儿行应成功");
        }

        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.observations, 4, "孤儿行不得计入观测数（实际健在 4 轮）");
        assert_eq!(s.consecutive_abnormal, 3, "孤儿异常行不得虚增连续轮数");
        assert!(s.fused());
        assert_eq!(s.deduped, 0, "孤儿行不算「同一条分析重复」");

        let ivs = load_fused_intervals(db).await.expect("读时间线应成功");
        assert_eq!(ivs.len(), 1, "只跨阈一次 ⇒ 一段");
        assert_eq!(
            ivs[0].from_ms, 5_000,
            "熔断段起点必须是健在轮 alive-b3；孤儿行若被算进会提前到 3_000"
        );
    }

    fn obs(analysis_id: &str, at: i64, abnormal: bool, alive: bool) -> DqiObs {
        DqiObs { analysis_id: analysis_id.to_string(), observed_at: at, abnormal, alive }
    }

    /// 披露口径①：孤儿行是**正常档**且嵌在前缀中间 ⇒ 现行计数虚高，差值必须报出来。
    ///
    /// 复现的是 2026-10-28 的现网形态：界面读「连续 9 轮」，而库里 `10-27 14:29:51` 那条
    /// B 级（正常档）观测因其所属分析已删除被剔除，本可把前缀断在 2。
    #[test]
    fn orphan_normal_grade_discloses_an_inflated_streak() {
        // 乱序传入，顺带锁住「入参顺序无关」（纯函数自己排序）
        let rows = vec![
            obs("a3", 700, true, true),
            obs("a1", 1_000, true, true),
            obs("o1", 800, false, false),
            obs("a4", 600, false, true),
            obs("a2", 900, true, true),
        ];
        let s = streak_from_rows(&rows);
        assert_eq!(s.consecutive_abnormal, 3, "判据口径不变：只数存活行");
        assert_eq!(s.orphans_skipped, 1, "跨度内的正常档孤儿行必须被点名");
        assert_eq!(s.consecutive_abnormal_incl_orphans, 2, "计入孤儿行则前缀断在 2 ⇒ 现行虚高");
        assert_eq!(s.observations, 4, "存活行参与数不得被孤儿行顶替");
        assert_eq!(s.deduped, 0, "孤儿行不得算成「同一条分析重复」");
    }

    /// 披露口径②：孤儿行是**异常档** ⇒ 现行计数低估。方向与上一条相反，
    /// 两条各钉一格 —— 只写一条就会放过「只在一个方向上算差值」的实现。
    #[test]
    fn orphan_abnormal_grade_discloses_a_deflated_streak() {
        let rows = vec![
            obs("b1", 1_000, true, true),
            obs("o1", 900, true, false),
            obs("b2", 800, false, true),
        ];
        let s = streak_from_rows(&rows);
        assert_eq!(s.consecutive_abnormal, 1);
        assert_eq!(s.orphans_skipped, 1);
        assert_eq!(s.consecutive_abnormal_incl_orphans, 2, "被跳过的都是降档轮 ⇒ 低估一轮");
    }

    /// 披露口径③：孤儿行落在跨度之外（比真正的断开点更旧）⇒ 一个字都不该披露。
    #[test]
    fn orphan_outside_the_claimed_span_is_not_disclosed() {
        let rows = vec![
            obs("c1", 1_000, true, true),
            obs("c2", 800, false, true),
            obs("o1", 500, true, false),
        ];
        let s = streak_from_rows(&rows);
        assert_eq!(s.consecutive_abnormal, 1);
        assert_eq!(s.orphans_skipped, 0, "前缀在 800 就断了，500 那行不在这句「连续」的跨度内");
        assert_eq!(
            s.consecutive_abnormal_incl_orphans, s.consecutive_abnormal,
            "无跨度空档 ⇒ 差值为 0"
        );
    }

    /// 披露口径④：最近一轮就是正常档（无前缀）⇒ 连「跳过多少轮」都不适用。
    #[test]
    fn no_prefix_means_no_orphan_disclosure() {
        let rows = vec![obs("d1", 1_000, false, true), obs("o1", 900, false, false)];
        let s = streak_from_rows(&rows);
        assert_eq!(s.consecutive_abnormal, 0);
        assert_eq!(s.orphans_skipped, 0, "没有「连续 N 轮」这句话，就没有跨空档一说");
    }

    /// 端到端：同一形态经**真落库**读出（纯函数对上生产查询口径，防「只测纯函数、库照旧」）。
    #[tokio::test]
    async fn load_recent_streak_discloses_orphans_from_the_database() {
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        for (id, at) in [("e1", 4_000_i64), ("e2", 3_000_i64), ("e3", 1_000_i64)] {
            insert_analysis(db, id).await;
            let grade = if id == "e3" { "A" } else { "D" };
            record_observation(db, id, Some(grade), at).await.expect("落行应成功");
        }
        // 孤儿行：正常档（B），时刻插在 **两条存活异常轮之间**（4000 与 3000 之间）。
        // 第一版把它插在 3000 之后（2000），于是计入与不计入都得 2 —— 端到端那条用例
        // 只是复述了纯函数已经钉过的差值为 0，白跑一趟库。插在中间才真走到「虚高」那一格。
        let active = data_quality_observations::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            scope: Set(DQI_SCOPE_GLOBAL.to_string()),
            horizon: Set(None),
            grade: Set(Some("B".to_string())),
            abnormal: Set(0),
            analysis_id: Set("orphan-normal".to_string()),
            observed_at: Set(3_500),
            created_at: Set(3_500),
        };
        data_quality_observations::Entity::insert(active).exec(db).await.expect("直插孤儿行应成功");

        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.consecutive_abnormal, 2, "判据仍只数存活行");
        assert_eq!(s.orphans_skipped, 1);
        assert_eq!(s.consecutive_abnormal_incl_orphans, 1, "计入那条正常档 ⇒ 前缀断在 1");
    }
}
