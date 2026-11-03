// SPDX-License-Identifier: AGPL-3.0-only

//! `trajectory.store` 接缝的**宿主侧实现**（wiring 层，宿主独持 `TrajectoryStorage`）。
//!
//! 契约在 `axagent-core/crates/harness/src/trajectory_store.rs`（已冻结，首批**只读**四个 op：
//! `list` / `get` / `query` / `count`；写面 `append` 被 PLAN §56.1-Q4 判为不给，五条判据见该模块头）。
//! 本文件只做「契约视图 → 宿主原形」的接线，四个落点都是宿主在用的公开方法：
//! [`TrajectoryStorage::get_trajectories`] / [`TrajectoryStorage::get_trajectory`] /
//! [`TrajectoryStorage::query_trajectories`] / [`TrajectoryStorage::count_trajectories`]。
//!
//! ## 映射点唯一（禁区 12，判据 7）
//!
//! [`TrajectoryQueryView`] → 宿主 [`TrajectoryQuery`]（`harness/src/trajectory_types.rs`，**原形不改**，
//! PLAN §56.1-Q6）的转换只住在 [`to_host_query`] 这一个纯函数里，单测直接测它：时间戳单位（epoch
//! **毫秒**）、空字段语义（`None` = 不施加该条件）、camelCase 线上键形态。反向证据：原形的
//! `time_range` 是 `Option<(DateTime<Utc>, DateTime<Utc>)>` 元组且**没有** `rename_all` ⇒ 既不能
//! 直接当线上入参，也不能就地加注解（它被宿主 `storage.rs` 与 proactive / dream / closed-loop 消费）。
//!
//! ## 宿主表达不了的过滤，一律拒收而不是静默返全量
//!
//! 原形只有一个**闭区间元组**，半边开区间（只给 `sinceMs` 或只给 `untilMs`）在它身上没有对应形状。
//! 两种「看起来能用」的做法都坏：丢掉那一头 ⇒ 时间窗整体消失 ⇒ 插件以为自己限了范围、实际拿到
//! 全量最近 N 条；用当前时刻或 1970 补另一头 ⇒ 静默造一个插件没写过的边界。故
//! [`half_open_window_error`] 落成 `Err(Validation)`。同一判据管住 `untilMs < sinceMs`（那种窗口
//! 恒零命中，是「阈值尺度写反」的同型）。契约文档那句「`None` 端点须按原形语义处理（只给
//! `sinceMs` ⇒ 另一端开区间）」在原形上落不了地 ⇒ 已在交回报告 §④ 点名请 leader 裁定（要么改契约
//! 措辞，要么给接缝加一个能表达半边窗口的宿主读口）。
//!
//! 其余过滤字段宿主**全部**支持，逐条落到列上：`session_id` / `user_id` 等值、`topic` 是
//! `LIKE %…%`（子串，不是精确匹配）、`min_quality` 打在 `quality_overall` 列（五项评分里的
//! `overall`，不是逐项阈）、`min_value_score` 等值列 `>=`、`outcome` 按 `{:?}` 变体名匹配。
//! 另有一条宿主自己加的隐含条件：`is_invalidated = 0`（软删的 append-only 证据退出活动查询）——
//! 接缝不开放「读失效证据」，插件拿到的是活动集，与宿主面板同口径。
//!
//! ## 数字的单源与 NaN 纪律（禁区 12 + 裁定 N4）
//!
//! [`TRAJECTORY_LIST_LIMIT_RANGE`] 与 [`TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS`] 是本文件唯一的区间
//! 来源；越界 ⇒ `Err(Validation)`，**不静默夹取**。缺省 limit 不另抄数字：取宿主
//! [`TrajectoryQuery::default`] 的那一份（[`default_list_limit`]），拿不到才退到区间上界。
//! ⚠ **绝不把 `None` 交给 `get_trajectories`** —— 宿主侧 `None` 的语义是「全表扫描 + 每行两次
//! 子查询取 steps/rewards」，那正是本接缝上界存在的理由。
//! `min_quality` / `min_value_score` 的 `NaN` 必须显式单判：宿主把这两个值原样喂给 SQL 的 `gte`，
//! 而任何 `v >= 0.0` / `v <= 1.0` 形式的比较对 NaN 恒 `false`（fail-open）⇒ 先 `is_nan()` 拒掉。
//! `[0, 1]` 是这两个字段的**量纲**（宿主侧它们是评分，`Default` 装的是 0.5），越出量纲的阈值没有
//! 可解释语义 ⇒ 同样拒收，不静默变成空集。
//!
//! ## 视图裁剪（大文本与会话原文不过线）
//!
//! [`TrajectorySummaryView`] 带的是 [`TrajectorySummaryView::summary_snippet`]，不是 `summary` 原文；
//! `steps`（含 `content` / `reasoning` / `tool_calls` / `tool_results` 原文）、`rewards`（含任意
//! `metadata` 载荷）、`patterns`、`last_replay_at` 与 session / message / memory 三族全部不发
//! （契约模块头那张清单是本文件的边界，裁剪先例 `jobs_store.rs`）。裁剪点唯一在
//! [`summary_view`]，按 [`TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS`] 以 **char** 截（`chars().take(n)`，
//! 按字节切中文会 panic）。时间只有一个口径：`created_at` → `createdAtMs`（epoch 毫秒），
//! `DateTime` 原形不过线。
//!
//! ## 查无没有错
//!
//! `list` / `query` 零命中 ⇒ `trajectories` 空数组；`get` 查无 ⇒ `Ok(None)`（帧里 `null`）；
//! `count` 的 `0` 是空库的合法读数。三格都有真库单测钉住（走 `axagent_dao::db::create_test_pool()`，
//! 与生产同一个建表链），不靠 mock 自证。
//!
//! ## 未装配 / 注册时机（裁定 J8）
//!
//! provider 只在 `init/services.rs` 装配成功后就地注册；`trajectory_storage` 是 `AppState` 的
//! 非 `Option` 字段，本批不存在「拿不到依赖」的运行时分支 ⇒ 不写伪装的 `Option` 检查，也不把字段
//! 改成 `Option` 去凑「未装配」那一格（PLAN §58.2-②）。未注册的真实表现在接缝上是分派层取不到
//! provider ⇒ `SEAM_PROVIDER_UNAVAILABLE`。
//!
//! ## 排序确定性
//!
//! `list` 与 `query` 的顺序由宿主决定（两条都 `ORDER BY created_at DESC`），本层不重排 ⇒ 插件侧
//! 可以复现断言（契约 `list` 文档要求的正是这一点）。

use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::trajectory_store::{
    TRAJECTORY_LIST_LIMIT_RANGE, TRAJECTORY_RATIO_RANGE, TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS,
    TrajectoryCountOutcome, TrajectoryListOutcome, TrajectoryQueryOutcome, TrajectoryQueryView,
    TrajectoryStoreProvider, TrajectorySummaryView,
};
use axagent_harness::trajectory_types::{Trajectory, TrajectoryQuery};
use axagent_trajectory::TrajectoryStorage;

/// `trajectory.store` 的宿主实现。
pub struct HostTrajectoryStore {
    /// 与 `AppState.trajectory_storage` **同一个** `Arc`（两份存储句柄就有两份连接池语义）。
    storage: Arc<TrajectoryStorage>,
}

impl HostTrajectoryStore {
    pub fn new(storage: Arc<TrajectoryStorage>) -> Self {
        Self { storage }
    }
}

// ── 纯函数：闸、映射、裁剪（本文件全部数字与语义的落点）────────────────────

/// 缺省 limit 的唯一来源：宿主 [`TrajectoryQuery::default`] 装的那一份。
///
/// 不抄数字是刻意的 —— 这里曾经是「装配点自己写 100 / 1000」那种形态，两处各写一份就一处会漂。
/// 极端情况（原形 default 被改成 `None`）才退到区间上界，那仍然是一个**有界**读数，不是全表。
fn default_list_limit() -> usize {
    TrajectoryQuery::default().limit.unwrap_or(TRAJECTORY_LIST_LIMIT_RANGE.1)
}

/// `limit` 闸：越出 [`TRAJECTORY_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)`，**不夹取**。
///
/// `None` 走 [`default_list_limit`]；`Some(0)` 拒收（契约：`limit = 0` 没有合法语义）。
fn resolve_limit(limit: Option<usize>, op: &str) -> Result<usize> {
    let effective = limit.unwrap_or_else(default_list_limit);
    if (TRAJECTORY_LIST_LIMIT_RANGE.0..=TRAJECTORY_LIST_LIMIT_RANGE.1).contains(&effective) {
        return Ok(effective);
    }
    Err(AxAgentError::Validation(format!(
        "trajectory.store::{op} 的 limit={effective} 越出契约区间 {}..={}（越界拒收，不静默夹取；\
         也不接受 None —— 宿主侧 None 是全表扫描）",
        TRAJECTORY_LIST_LIMIT_RANGE.0, TRAJECTORY_LIST_LIMIT_RANGE.1
    )))
}

/// 评分阈值闸：NaN 单判 + 量纲 [`TRAJECTORY_RATIO_RANGE`]。
fn check_ratio(field: &str, value: Option<f64>) -> Result<()> {
    let Some(ratio) = value else { return Ok(()) };
    if ratio.is_nan() {
        return Err(AxAgentError::Validation(format!(
            "trajectory.store 的 {field} 是 NaN：宿主把它原样交给 SQL 的 gte，而任何 `>=` / `<=` \
             比较对 NaN 恒 false（fail-open），必须先显式拒掉"
        )));
    }
    if !ratio.is_finite() {
        return Err(AxAgentError::Validation(format!(
            "trajectory.store 的 {field}={ratio} 不是有限数（±∞ 会让条件恒真或恒假）"
        )));
    }
    if !(TRAJECTORY_RATIO_RANGE.0..=TRAJECTORY_RATIO_RANGE.1).contains(&ratio) {
        return Err(AxAgentError::Validation(format!(
            "trajectory.store 的 {field}={ratio} 越出宿主量纲 [`TRAJECTORY_RATIO_RANGE`]（它是评分阈值，越界的阈值没有\
             可解释语义 ⇒ 拒收，不静默变成空集）"
        )));
    }
    Ok(())
}

/// epoch 毫秒 → 宿主 `DateTime<Utc>`（单位是**毫秒**，与原形 `time_range` 的转换逐字对齐）。
fn epoch_millis_to_utc(millis: i64, field: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp_millis(millis).ok_or_else(|| {
        AxAgentError::Validation(format!(
            "trajectory.store 的 {field}={millis} 越出可表示时刻范围（epoch 毫秒，不是秒）"
        ))
    })
}

/// 半边时间窗那一格的错误构造（单独成函数让「必须拒收」可被单测钉住）。
fn half_open_window_error(since_ms: Option<i64>, until_ms: Option<i64>) -> AxAgentError {
    AxAgentError::Validation(format!(
        "trajectory.store::query 的时间窗必须两端齐备（sinceMs={since_ms:?}, untilMs={until_ms:?}）：\
         原形 TrajectoryQuery.time_range 是闭区间元组，半边窗口表达不了 ⇒ 拒收，不静默丢掉另一端\
         变成全量，也不替插件伪造一个它没写过的边界"
    ))
}

/// 视图 → 宿主原形的**唯一**映射点（禁区 12）。
fn to_host_query(filter: &TrajectoryQueryView) -> Result<TrajectoryQuery> {
    check_ratio("minQuality", filter.min_quality)?;
    check_ratio("minValueScore", filter.min_value_score)?;
    let limit = resolve_limit(filter.limit, "query")?;

    let time_range = match (filter.since_ms, filter.until_ms) {
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => {
            return Err(half_open_window_error(filter.since_ms, filter.until_ms));
        },
        (Some(since_ms), Some(until_ms)) => {
            let since = epoch_millis_to_utc(since_ms, "sinceMs")?;
            let until = epoch_millis_to_utc(until_ms, "untilMs")?;
            if until < since {
                return Err(AxAgentError::Validation(format!(
                    "trajectory.store::query 的时间窗反了：untilMs={until_ms} < sinceMs={since_ms}\
                     （这种窗口恒零命中，与「阈值尺度写反」同型 ⇒ 拒收而不是返回空集）"
                )));
            }
            Some((since, until))
        },
    };

    Ok(TrajectoryQuery {
        session_id: filter.session_id.clone(),
        user_id: filter.user_id.clone(),
        topic: filter.topic.clone(),
        // 复用 harness 既有枚举，原样递给宿主（宿主按 `{:?}` 变体名匹配列）。
        outcome: filter.outcome,
        min_quality: filter.min_quality,
        min_value_score: filter.min_value_score,
        time_range,
        // 落回原形时**必须**是 `Some`：`None` 在原形里是「不限量」。
        limit: Some(limit),
    })
}

/// 摘要正文预览裁剪：**按 char**，上限 [`TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS`]。
fn snippet(text: &str) -> String {
    text.chars().take(TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS).collect()
}

/// 宿主 [`Trajectory`] → 契约视图（**唯一**映射点：大文本与会话原文在这里被甩掉）。
///
/// 时间口径只有一个：`created_at` → `created_at_ms`（epoch 毫秒）。`last_replay_at`、`patterns`、
/// `steps`、`rewards` 一个都不取，连读都不读（读了再丢也算把原文搬进了接缝进程的内存）。
fn summary_view(trajectory: Trajectory) -> TrajectorySummaryView {
    TrajectorySummaryView {
        id: trajectory.id,
        session_id: trajectory.session_id,
        user_id: trajectory.user_id,
        agent_name: trajectory.agent_name,
        topic: trajectory.topic,
        summary_snippet: snippet(&trajectory.summary),
        outcome: trajectory.outcome,
        duration_ms: trajectory.duration_ms,
        quality: trajectory.quality,
        value_score: trajectory.value_score,
        created_at_ms: trajectory.created_at.timestamp_millis(),
        replay_count: trajectory.replay_count,
    }
}

// ── Trait 实现 ────────────────────────────────────────────────────────────

#[async_trait]
impl TrajectoryStoreProvider for HostTrajectoryStore {
    /// 列表（最近 N 条，宿主按 `created_at` 倒序）。
    async fn list(&self, limit: Option<usize>) -> Result<TrajectoryListOutcome> {
        // 闸在 IO 之前：`Some(0)` / `Some(201)` 不该换来一次全表扫描。
        let effective = resolve_limit(limit, "list")?;
        let rows = self.storage.get_trajectories(Some(effective)).await.map_err(|e| {
            AxAgentError::Provider(format!("trajectory.store::list 取轨迹失败: {e}"))
        })?;
        Ok(TrajectoryListOutcome { trajectories: rows.into_iter().map(summary_view).collect() })
    }

    /// 单条：查无 ⇒ `Ok(None)`（帧里 `null`），**不是错误**、不借错误码。
    async fn get(&self, id: &str) -> Result<Option<TrajectorySummaryView>> {
        let row = self.storage.get_trajectory(id).await.map_err(|e| {
            AxAgentError::Provider(format!("trajectory.store::get 取轨迹失败: {e}"))
        })?;
        Ok(row.map(summary_view))
    }

    /// 条件查询：视图先经 [`to_host_query`] 落成原形（映射点唯一）。
    async fn query(&self, filter: TrajectoryQueryView) -> Result<TrajectoryQueryOutcome> {
        let host_filter = to_host_query(&filter)?;
        let rows = self.storage.query_trajectories(&host_filter).await.map_err(|e| {
            AxAgentError::Provider(format!("trajectory.store::query 查轨迹失败: {e}"))
        })?;
        Ok(TrajectoryQueryOutcome { trajectories: rows.into_iter().map(summary_view).collect() })
    }

    /// 总数：`0` 是空库的合法读数（宿主走 `COUNT(*)`，不加载实体）。
    async fn count(&self) -> Result<TrajectoryCountOutcome> {
        let count = self.storage.count_trajectories().await.map_err(|e| {
            AxAgentError::Provider(format!("trajectory.store::count 计数失败: {e}"))
        })?;
        Ok(TrajectoryCountOutcome { count })
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 「查无没有错」那三格走真库（`create_test_pool()` 与生产同一个建表链，空表）而不是 mock：
// mock 只会证明「我按我想的写了」，证不了宿主在那张表为空时确实返 `Ok`。
// 未覆盖项如实登记：非空轨迹的字段映射由契约侧形态锁 + 本文件 `summary_view` 的裁剪测试覆盖，
// 「库里真有一条轨迹」的端到端读要宿主 migrations 之外的数据面，属跨批项。

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::trajectory_store::TRAJECTORY_LIST_LIMIT_RANGE;
    use axagent_harness::trajectory_types::TrajectoryOutcome;
    use serde_json::{Value, json};

    fn view_with(since_ms: Option<i64>, until_ms: Option<i64>) -> TrajectoryQueryView {
        TrajectoryQueryView {
            session_id: None,
            user_id: None,
            topic: None,
            outcome: None,
            min_quality: None,
            min_value_score: None,
            since_ms,
            until_ms,
            limit: None,
        }
    }

    fn sample_trajectory(summary: &str) -> Trajectory {
        let mut t = Trajectory::new(
            "sess-1".into(),
            "user-1".into(),
            "贵州茅台建仓".into(),
            summary.into(),
            TrajectoryOutcome::Partial,
            4200,
            Vec::new(),
        );
        t.agent_name = Some("股票分析师".into());
        // 这三族在视图里没有对应字段 —— 填上它们就是为了证明「填了也不得过线」。
        t.replay_count = 3;
        t
    }

    #[test]
    fn limit_gate_pins_the_contract_range() {
        let (low, high) = TRAJECTORY_LIST_LIMIT_RANGE;
        assert_eq!(resolve_limit(Some(low), "list").ok(), Some(low));
        assert_eq!(resolve_limit(Some(high), "list").ok(), Some(high));
        for bad in [0, high + 1, usize::MAX] {
            let err = resolve_limit(Some(bad), "list").expect_err("{bad} 应拒收，不该夹取");
            assert!(matches!(err, AxAgentError::Validation(_)), "越界应落成 Validation：{err:?}");
        }
        // 缺省走宿主自己那份 default，且**有界**（绝不是 None ⇒ 全表）。
        let defaulted = resolve_limit(None, "list").expect("缺省 limit 必须合法");
        assert!((low..=high).contains(&defaulted), "缺省值应落在契约区间内：{defaulted}");
        assert_eq!(
            Some(defaulted),
            TrajectoryQuery::default().limit,
            "缺省该等于宿主原形 default，不是本文件另抄的数字"
        );
    }

    #[test]
    fn ratio_gates_reject_nan_and_out_of_scale() {
        // 量纲数字住在契约层，provider 只引用它；本条把它钉在测试里 —— 契约改数即红
        // （先例 `monitor_seam_service.rs` 的 `aggregator_ranges_are_the_harness_numbers`）。
        assert_eq!(TRAJECTORY_RATIO_RANGE, (0.0f64, 1.0f64));
        for bad in [f64::INFINITY, f64::NEG_INFINITY, -0.5, 1.5] {
            assert!(
                matches!(check_ratio("minQuality", Some(bad)), Err(AxAgentError::Validation(_))),
                "{bad} 应被拒掉（越出宿主量纲 [0, 1]）"
            );
        }
        assert!(check_ratio("minQuality", None).is_ok());
        assert!(check_ratio("minQuality", Some(0.0)).is_ok());
        assert!(check_ratio("minValueScore", Some(1.0)).is_ok());
        // NaN 必须由 `is_nan()` 那一支**自己**咬住：范围判据的 `contains` 对 NaN 也返 false，
        // 所以只断言「返回 Err」的话删掉那一支测试照样绿 —— 文案指得到 NaN 才算钉住。
        let err = check_ratio("minQuality", Some(f64::NAN)).expect_err("NaN 必须被显式拒掉");
        match err {
            AxAgentError::Validation(message) => assert!(
                message.contains("NaN"),
                "应走 is_nan() 那一支，报错文案要能指到 NaN：{message}"
            ),
            other => panic!("应为 Validation，实际 {other:?}"),
        }
    }

    #[test]
    fn half_open_time_window_is_refused_not_silently_unbounded() {
        for filter in
            [view_with(Some(1_600_000_000_000), None), view_with(None, Some(1_700_000_000_000))]
        {
            let err = to_host_query(&filter).expect_err("半边窗口应拒收");
            assert!(matches!(err, AxAgentError::Validation(_)), "应为 Validation：{err:?}");
        }
        // 两端齐备才落成闭区间；两端皆缺 = 不施加时间条件（不是全量错误，而是契约的 None 语义）。
        assert!(
            to_host_query(&view_with(None, None)).expect("无时间窗应合法").time_range.is_none()
        );
        let both = to_host_query(&view_with(Some(1_600_000_000_000), Some(1_700_000_000_000)))
            .expect("闭区间应合法");
        let (since, until) = both.time_range.expect("应有时间窗");
        // 单位逐字对：epoch **毫秒**，别把毫秒当秒（那会把窗口挪到 1799 年）。
        assert_eq!(since.timestamp_millis(), 1_600_000_000_000);
        assert_eq!(until.timestamp_millis(), 1_700_000_000_000);
        assert_ne!(since.timestamp_millis(), 1_600_000_000, "按秒解释会读到这个错值");
    }

    #[test]
    fn reversed_time_window_is_refused() {
        let err = to_host_query(&view_with(Some(1_700_000_000_000), Some(1_600_000_000_000)))
            .expect_err("until < since 恒零命中，应拒收");
        assert!(matches!(err, AxAgentError::Validation(_)));
    }

    #[test]
    fn host_query_passes_through_every_supported_field() {
        let filter = TrajectoryQueryView {
            session_id: Some("sess-1".into()),
            user_id: Some("user-1".into()),
            topic: Some("建仓".into()),
            outcome: Some(TrajectoryOutcome::Abandoned),
            min_quality: Some(0.7),
            min_value_score: Some(0.5),
            since_ms: Some(1_600_000_000_000),
            until_ms: Some(1_700_000_000_000),
            limit: Some(50),
        };
        let host = to_host_query(&filter).expect("九字段都应能映射");
        assert_eq!(host.session_id.as_deref(), Some("sess-1"));
        assert_eq!(host.user_id.as_deref(), Some("user-1"));
        assert_eq!(host.topic.as_deref(), Some("建仓"));
        assert_eq!(host.outcome, Some(TrajectoryOutcome::Abandoned));
        assert_eq!(host.min_quality, Some(0.7));
        assert_eq!(host.min_value_score, Some(0.5));
        assert_eq!(host.limit, Some(50));
        // 原形没有 `time_range` 之外的时间字段 ⇒ 不造第二份时间口径。
        assert!(host.time_range.is_some());
    }

    #[test]
    fn query_view_wire_form_is_camel_case_epoch() {
        let filter = TrajectoryQueryView {
            session_id: Some("sess-1".into()),
            user_id: None,
            topic: None,
            outcome: Some(TrajectoryOutcome::Success),
            min_quality: None,
            min_value_score: None,
            since_ms: Some(1_700_000_000_000),
            until_ms: None,
            limit: Some(20),
        };
        let val = serde_json::to_value(&filter).expect("序列化应成功");
        let mut keys: Vec<String> = val.as_object().expect("应为对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "limit",
                "minQuality",
                "minValueScore",
                "outcome",
                "sessionId",
                "sinceMs",
                "topic",
                "untilMs",
                "userId"
            ]
        );
        assert_eq!(val["sinceMs"].as_i64(), Some(1_700_000_000_000), "epoch 毫秒是 i64，不是浮点");
        // 空对象 = 全 None（钳位与缺省都由本层决定，不由插件猜）。
        let empty: TrajectoryQueryView = serde_json::from_value(json!({})).expect("空对象应合法");
        assert!(empty.limit.is_none() && empty.since_ms.is_none() && empty.outcome.is_none());
    }

    #[test]
    fn summary_view_truncates_and_drops_bulk_fields() {
        let long_summary: String = "先观察再小仓试买".repeat(60); // 420 字 > 上限
        let view = summary_view(sample_trajectory(&long_summary));
        assert_eq!(
            view.summary_snippet.chars().count(),
            TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS,
            "摘要正文只发截断预览"
        );
        assert!(long_summary.chars().count() > TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS);

        let val = serde_json::to_value(&view).expect("序列化应成功");
        let mut keys: Vec<String> = val.as_object().expect("应为对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "agentName",
                "createdAtMs",
                "durationMs",
                "id",
                "outcome",
                "quality",
                "replayCount",
                "sessionId",
                "summarySnippet",
                "topic",
                "userId",
                "valueScore"
            ]
        );
        for forbidden in ["steps", "rewards", "patterns", "content", "summary", "lastReplayAt"] {
            assert!(val.get(forbidden).is_none(), "{forbidden} 属大文本/会话原文，不过线");
        }
        assert_eq!(val["outcome"], json!("partial"), "字面量与宿主落库口径逐字同");
        assert_eq!(val["agentName"], json!("股票分析师"));
        assert_eq!(view.replay_count, 3);
        assert_eq!(view.duration_ms, 4200);
    }

    #[test]
    fn summary_view_time_has_a_single_epoch_unit() {
        let mut trajectory = sample_trajectory("摘要");
        trajectory.created_at =
            chrono::DateTime::from_timestamp_millis(1_788_684_590_755).expect("测试时刻应可表示");
        let view = summary_view(trajectory);
        assert_eq!(view.created_at_ms, 1_788_684_590_755, "epoch 毫秒原样，不做二次换算");
        let val = serde_json::to_value(&view).expect("序列化应成功");
        assert!(val.get("createdAt").is_none(), "DateTime 原形不过线");
    }

    /// 空库句柄：与生产**同一个**建表链（`create_test_pool` 内部走 `initialize_schema`）。
    async fn provider_on_empty_db() -> HostTrajectoryStore {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试：建连接池应成功");
        let storage = Arc::new(TrajectoryStorage::new(Arc::new(handle.conn)));
        HostTrajectoryStore::new(storage)
    }

    #[tokio::test]
    async fn empty_table_reads_are_ok_values_not_errors() {
        let provider = provider_on_empty_db().await;
        // 三格「查无没有错」：list 空数组 / get Ok(None) / count 0。
        let list = provider.list(None).await.expect("空表 list 不该 Err");
        assert!(list.trajectories.is_empty());
        let one = provider.get("no-such-trajectory").await.expect("空表 get 不该 Err");
        assert!(one.is_none(), "查无 = Ok(None)，不借错误码");
        let count = provider.count().await.expect("空库 count 不该 Err");
        assert_eq!(count.count, 0, "0 是合法读数");

        let val: Value = serde_json::to_value(&one).expect("序列化应成功");
        assert_eq!(val, json!(null), "帧里是 null");
        let query = provider.query(view_with(None, None)).await.expect("空表 query 不该 Err");
        assert!(query.trajectories.is_empty(), "零命中 = 空数组");
    }

    #[tokio::test]
    async fn out_of_range_limit_is_rejected_before_the_scan() {
        let provider = provider_on_empty_db().await;
        let err = provider.list(Some(TRAJECTORY_LIST_LIMIT_RANGE.1 + 1)).await.expect_err("应拒收");
        assert!(matches!(err, AxAgentError::Validation(_)), "应为 Validation：{err:?}");
        let err = provider.list(Some(0)).await.expect_err("limit=0 没有合法语义");
        assert!(matches!(err, AxAgentError::Validation(_)));
    }
}
