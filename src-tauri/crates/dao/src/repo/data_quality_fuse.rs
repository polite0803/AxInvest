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
//! `abnormal` = **该轮没拿到 A 级**，含两种情形：
//!   · grade 是 `B` / `C` / `D` / `F` ⇒ 异常（#8 的原话就是「B 级跨样本熔断」，B 含在内）；
//!   · grade 为 `NULL` ⇒ **也算异常**。`data-quality` 节点自身 `continue_on_fail = true`，
//!     它整段没跑成时最容易被读成「没有质量问题记录」= 正常；那正是把「拿不到」冒充「没问题」。
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

use axagent_entities::data_quality_observations;
use sea_orm::entity::prelude::*;
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder, Set};

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
pub fn grade_is_abnormal(grade: Option<&str>) -> bool {
    match grade.map(str::trim).filter(|g| !g.is_empty()) {
        // NULL / 空串 = 无从判级 ⇒ 按最坏处理（见模块头「异常的边界」第二条）
        None => true,
        Some(g) => g != "A",
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
}

impl DqiStreak {
    /// 没有任何观测时的取值 —— 刻意不等于「正常」，调用方须按 `observations == 0` 分支。
    pub const EMPTY: DqiStreak = DqiStreak { consecutive_abnormal: 0, observations: 0, deduped: 0 };

    pub fn fused(&self) -> bool {
        self.observations > 0 && is_fused(self.consecutive_abnormal)
    }
}

/// 取最近 `DQI_STREAK_LOOKBACK` 条观测（同一条分析只算一次）并数前缀连续异常。
pub async fn load_recent_streak(db: &DatabaseConnection) -> Result<DqiStreak, DbErr> {
    let rows = data_quality_observations::Entity::find()
        .filter(data_quality_observations::Column::Scope.eq(DQI_SCOPE_GLOBAL))
        .order_by_desc(data_quality_observations::Column::ObservedAt)
        .all(db)
        .await?;

    // `rows` 已按 `observed_at` 倒序 ⇒ 同一个 analysis_id 的**第一**次出现就是它最近的一行；
    // 后面的重复行只记数不参与（否则一条分析重跑两次会被算成两轮异常）。
    let mut distinct: Vec<&data_quality_observations::Model> = Vec::with_capacity(rows.len());
    let mut deduped = 0usize;
    for row in &rows {
        if distinct.iter().any(|kept| kept.analysis_id == row.analysis_id) {
            deduped += 1;
            continue;
        }
        distinct.push(row);
    }
    let window = distinct.len().min(DQI_STREAK_LOOKBACK);
    // 前缀连续异常：从最近一轮往回数，遇到第一个正常轮就停
    let mut consecutive_abnormal = 0usize;
    for row in distinct.iter().take(window) {
        if row.abnormal == 0 {
            break;
        }
        consecutive_abnormal += 1;
    }

    Ok(DqiStreak { consecutive_abnormal, observations: window, deduped })
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

/// 读全量观测并导出熔断时间线（同一 analysis_id 只认最近一行，与 [`load_recent_streak`] 同口径）。
pub async fn load_fused_intervals(db: &DatabaseConnection) -> Result<Vec<FusedInterval>, DbErr> {
    let rows = data_quality_observations::Entity::find()
        .filter(data_quality_observations::Column::Scope.eq(DQI_SCOPE_GLOBAL))
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

    #[test]
    fn abnormal_predicate_treats_missing_grade_as_worst_case() {
        assert!(!grade_is_abnormal(Some("A")), "A 级是唯一正常档");
        for g in ["B", "C", "D", "F"] {
            assert!(grade_is_abnormal(Some(g)), "{g} 级必须算异常（#8 的『B 级及以下』含 B）");
        }
        // 「拿不到」不得冒充「没问题」
        assert!(grade_is_abnormal(None));
        assert!(grade_is_abnormal(Some("")), "空串等价于没跑成");
        assert!(grade_is_abnormal(Some("   ")), "空白不参与 A 比较");
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
        assert!(!DqiStreak { consecutive_abnormal: 0, observations: 0, deduped: 0 }.fused());
    }

    #[tokio::test]
    async fn streak_counts_leading_run_and_breaks_on_first_good_grade() {
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        // 期望的**最近→最旧**顺序：F, B, A, B ⇒ 前缀连续异常 = 2（遇 A 就断，后面的 B 不参与）。
        // ⚠ 时间戳必须与这个顺序同向（数组首元素 = 最新一轮）：第一版照数组顺序递增秒数，
        //   于是库里最新一轮是 `B`、前缀只有 1 —— 代码对、夹具反了，红得有价值但报错了对象。
        let grades = ["F", "B", "A", "B"];
        for (i, g) in grades.iter().enumerate() {
            let at = 1_700_000_000_000 + (grades.len() - i) as i64;
            let inserted = record_observation(db, &format!("an-{i}"), Some(g), at)
                .await
                .expect("落观测行应成功");
            assert!(inserted, "首次落行不该被去重: {g}");
        }

        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.consecutive_abnormal, 2, "前缀连续异常 = 2，遇 A 断链");
        assert_eq!(s.observations, 4);
        assert_eq!(s.deduped, 0);
        assert!(!s.fused(), "2 < {DQI_FUSE_STREAK} ⇒ 不该熔断");

        // 再来一轮异常 ⇒ 3 轮，跨阈 ⇒ 熔断
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

        record_observation(db, "same-analysis", Some("B"), 1_700_000_000_000)
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
        for (at, grade) in [(1_700_000_000_000_i64, "A"), (1_800_000_000_000_i64, "B")] {
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
        assert_eq!(s.consecutive_abnormal, 1, "认最近那行（B ⇒ 异常 1），而不是 A+B 两行");
    }

    #[tokio::test]
    async fn missing_grade_is_recorded_as_abnormal_and_streaks_like_the_rest() {
        let handle = create_test_pool().await.expect("测试库可创建");
        let db = &handle.conn;
        for i in 0..DQI_FUSE_STREAK {
            record_observation(db, &format!("null-grade-{i}"), None, 1_700_000_000_000 + i as i64)
                .await
                .expect("NULL grade 也应落行");
        }
        let s = load_recent_streak(db).await.expect("读 streak 应成功");
        assert_eq!(s.consecutive_abnormal, DQI_FUSE_STREAK);
        assert!(s.fused(), "节点反复没跑成 ⇒ 与「一直 B 级」同等对待，不静默放行");
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
}
