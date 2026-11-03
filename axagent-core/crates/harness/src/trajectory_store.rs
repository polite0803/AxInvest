// SPDX-License-Identifier: AGPL-3.0-only

//! 轨迹库只读接缝（`trajectory.store`）— DTO + Trait。
//!
//! 宿主把 `axagent-trajectory` 的 `TrajectoryStorage` 的**读面**经本接缝暴露给插件侧；
//! 插件经 `call_seam("trajectory.store", op, args)` 列表 / 单条 / 条件查询 / 计数。
//! 权威类型 `Trajectory` / `TrajectoryQuery` 住在同目录 [`crate::trajectory_types`]，
//! 本模块只定义**跨边界视图**，不改那两个原形。
//!
//! ## 首批无写 op（PLAN §56.1-Q4）
//!
//! `append`（写轨迹）裁定为**不给**。五条判据摘要：
//!
//! 1. **多表事务不可复刻**：宿主 `save_trajectory` 一次联动主表 + steps + rewards + FTS 索引，
//!    协议层只有一次 `block_on` 调用的原子性，插件侧无法表达「写失败回滚多表」。
//! 2. **幂等靠域内派生键**：`save_pattern` 的幂等完全依赖自然键派生 id + `ON CONFLICT`；
//!    外部写入绕过派生即导致表无界增长。
//! 3. **写入口 = 唯一样本源被污染**：RL 训练（`commands/rl_training.rs`）、进化门槛
//!    （`stock_workflow/reflection.rs:4515` 取 30 条、<10 条拒进化）、协同进化按 `outcome`
//!    占比决策（`tasks/coevolution_task.rs`）都读这张表；而 `outcome` / `quality` /
//!    `value_score` 是**可写字段**（`trajectory_types.rs:105-108`）⇒ 插件伪造一条 `Success`
//!    就能抬进化的分母。
//! 4. **权限面单薄**：写 op 只有 `write` 一档闸，没有「只能写自己的轨迹」这种粒度。
//! 5. **收益不成立**：插件写轨迹的唯一动机是「让宿主反思/进化看见我」，而反思链的输入本来就是
//!    宿主自己的会话，外部写只增噪不增信。
//!
//! 若将来必开，走「最小写子集」：单 op + 强制插件前缀 `agentName` + 入参禁带 `steps` /
//! `rewards` / `quality` / `valueScore`（由宿主补默认）+ 速率闸，且**先补** `TRAJECTORY_*`
//! 错误码族与 11 语言译键。
//!
//! ## 视图裁剪：哪些字段永不过线（res105 B8）
//!
//! 泄敏面是**大文本 + 会话内容**，不是凭据 —— 全表没有 `api_key` / `token` 类字段，别把它夸大成
//! 「密钥不过线」。逐条：
//!
//! - `Trajectory.steps`（`trajectory_types.rs:110`）里的 `TrajectoryStep.content`（:36）、
//!   `reasoning`（:37）、`tool_calls`（:38，元素含 `ToolCall.arguments` 原文）、
//!   `tool_results`（:39，元素含 `TrajectoryToolResult.output` 原文）⇒ 全量工具调用明细 +
//!   模型思考原文，单条轨迹可达数百 KB。
//! - `Trajectory.rewards`（:111）里的 `RewardSignal.metadata: serde_json::Value`（:205）⇒
//!   任意载荷，可能夹带 provider 配置。
//! - `Trajectory.patterns`（:109）与 `TrajectoryPattern.trajectory_ids`（:215）⇒ 模式反向索引
//!   大数组；模式的读面属 skill/pattern 域，不在本接缝。
//! - `Trajectory.summary`（:104）⇒ 摘要正文只给截断预览
//!   [`TrajectorySummaryView::summary_snippet`]。
//! - `Trajectory.last_replay_at`（:114，`Option<DateTime<Utc>>`）⇒ 首批不发；时间只以
//!   `createdAtMs` 这一个 epoch 毫秒口径出线。
//! - session / message / memory 三族（`TrajectorySession.system_prompt`、
//!   `Message.tool_calls` 等）⇒ 用户会话原文与系统提示，**整族**不过线，本接缝也不开它们的 op。
//!
//! 裁剪先例：`jobs_store.rs:15-16`（`last_result.output` 不过帧）+ :29 的 `CronJobRunView`。
//!
//! ## 为什么不改 `TrajectoryQuery` 原形（PLAN §56.1-Q6）
//!
//! [`crate::trajectory_types::TrajectoryQuery`]（:288）**没有** `#[serde(rename_all)]`（:287 只
//! derive），且 `time_range` 是 `Option<(DateTime<Utc>, DateTime<Utc>)>` 元组（:295）⇒ 既不能
//! 直接当线上入参，也不能就地加注解：它被宿主 `storage.rs:409` 与 proactive / dream /
//! closed-loop 等消费方使用，改注解会同时改掉落库与既有 IPC 的键形态。
//! ⇒ 新建 [`TrajectoryQueryView`]（camelCase + `sinceMs` / `untilMs` 两个 `i64` epoch 毫秒），
//! 原形一字不动。
//!
//! ## 写面权限
//!
//! 本接缝首批**全部是读 op** ⇒ 无写权限闸；宿主 provider 内部按 [`TRAJECTORY_LIST_LIMIT_RANGE`]
//! 钳位即可。
//!
//! ## 错误语义
//!
//! 形状 / 准入 / 无 provider 类错误走协议码；provider 内部任何 `Err` 由分派层兜底成
//! `PLUGIN_CALL_FAILED` + message。harness **没有** `TRAJECTORY_*` 码族（首批只读 ⇒ 零新增码，
//! 也就零 11 语言译键负担；PLAN §56.1-Q5）。
//! **「查无此轨迹」= `null`，不是错误**（协议层没有通用 NOT_FOUND，先例 `jobs.store` 的 `get`）。
//!
//! ## `as_of`
//!
//! 四个 op 都是 DB 读，不取行情 ⇒ **一律不收 `as_of`**。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};
use crate::trajectory_types::{TrajectoryOutcome, TrajectoryQuality};

// ── 硬上限（契约级数字，单源）────────────────────────────────────────────
//
// ⚠ 宿主 provider 与 plugins 分派层**只准引用这里**，不得另建一套（先例
// `stock_monitor.rs:342-348` 的 `AGGREGATOR_*` 一族）。

/// `list` 的 `limit` 与 `query` 的 `limit` 的合法闭区间。
///
/// 上界 200 的理由：每条只发摘要视图（[`TrajectorySummaryView`]），但其体积由两个数相乘决定
/// —— `limit × snippet 上限`。取 200 × 200 字符（中文 UTF-8 ≈ 600 B/条）⇒ 一帧最坏 ≈ 120 KB，
/// 远低于帧上限，同时给插件足够一次拉一屏的样本量。下界 1：`limit = 0` 没有合法语义，
/// 「不想要数据」应当不调用。
pub const TRAJECTORY_LIST_LIMIT_RANGE: (usize, usize) = (1, 200);

/// [`TrajectorySummaryView::summary_snippet`] 的字符上限（按 `char` 计，不是字节）。
///
/// 定 200 的理由：宿主侧 `Trajectory.summary` 是模型生成的一到两句摘要，200 个汉字/字符足以
/// 覆盖其主干，同时保证**它不能被当作原文的替代品** —— 想让插件拿到接近全文的正文，得把这个数
/// 抬到与整条摘要同量级，而那正是本视图刻意裁掉的东西（见模块头「视图裁剪」）。
/// 截断由宿主 provider 单点执行，接缝不承诺截断处不切词。
pub const TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS: usize = 200;

// 区间自洽 + 摘要上限非零（0 会让 snippet 恒为空串，等于悄悄关掉读面）。
const _: () = assert!(TRAJECTORY_LIST_LIMIT_RANGE.0 <= TRAJECTORY_LIST_LIMIT_RANGE.1);
const _: () = assert!(TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS > 0);

/// [`TrajectoryQueryView::min_quality`] 与 [`TrajectoryQueryView::min_value_score`] 的量纲闭区间
/// —— 定义域，不是可调预算。
///
/// **为什么是量纲而不是可调区间**：[`TRAJECTORY_LIST_LIMIT_RANGE`] 是**预算**（一帧拉多少条，值可谈，
/// 受帧体积约束）；而宿主的 `quality` 五项评分与 `value_score` 本来就是归一到 `[0, 1]` 的**比例**，
/// 越界的阈值没有可解释语义（`> 1` 恒零命中、`< 0` 恒全通过 —— 后者等于把过滤悄悄关掉），且宿主把
/// 它原样交给 SQL 的 `gte`：越界值不报错，只给出一张错集。这里没有「取多少合适」可调，只有定义域。
/// 单源纪律同前：宿主 provider 的闸只准引用本常量，不得在落点再抄一遍 `0.0..=1.0`
/// （先例 `stock_monitor.rs:348` 的 `AGGREGATOR_MIN_STRENGTH`，同为 `[0, 1]` 量纲）。
pub const TRAJECTORY_RATIO_RANGE: (f64, f64) = (0.0, 1.0);

// 定义域自洽：下界若大于上界，`RangeInclusive::contains` 对任何入参恒 false ⇒ 两个阈值全被拒。
const _: () = assert!(TRAJECTORY_RATIO_RANGE.0 <= TRAJECTORY_RATIO_RANGE.1);

// ── DTO（跨边界视图；宿主内部类型与原文不过线）──────────────────────────

/// 轨迹摘要视图 —— `Trajectory` 的**跨边界形态**，字段裁剪见模块头。
///
/// 时间只有一个口径：`created_at`（`DateTime<Utc>`）→ [`Self::created_at_ms`]（epoch 毫秒），
/// 避免把 `chrono` 的字符串序列化推给插件侧再解回来（与 §56.1-Q6 同一理由）。
/// 转换点**唯一**在宿主 provider（照 `trading_engine_service.rs` 把 `trade_view` 当唯一映射点
/// 的做法），其他消费方不得各写一份字段映射。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectorySummaryView {
    pub id: String,
    pub session_id: String,
    pub user_id: String,
    /// 结构化 agent 标识（`AgentProfile` 名称），与宿主同形：可能是 `None`。
    pub agent_name: Option<String>,
    pub topic: String,
    /// 摘要正文的截断预览，上限 [`TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS`] 字符。
    /// **展示用**，不得当作原文参与再计算。
    pub summary_snippet: String,
    /// 复用 harness 既有枚举（`trajectory_types.rs:44`，`#[serde(rename_all = "lowercase")]`）
    /// ⇒ 线上字面量是 `"success"` / `"failure"` / `"partial"` / `"abandoned"`，与宿主落库一致。
    pub outcome: TrajectoryOutcome,
    pub duration_ms: u64,
    /// 复用 harness 既有结构（`trajectory_types.rs:74`，camelCase 五项评分）。
    /// 只读投影：插件拿得到分，但**改不了**（首批无写 op）。
    pub quality: TrajectoryQuality,
    pub value_score: f64,
    /// 创建时刻（epoch 毫秒）。
    pub created_at_ms: i64,
    pub replay_count: u32,
}

/// `query` op 的入参 —— `TrajectoryQuery` 的视图，**不改原形**（PLAN §56.1-Q6）。
///
/// 与原形的两处差别：① 键为 camelCase（原形无 `rename_all`）；② `time_range` 元组换成
/// `sinceMs` / `untilMs` 两个 epoch 毫秒。宿主 provider 把它映射回 `TrajectoryQuery` 时，
/// **只给一端 ⇒ `Err(Validation)`**：原形 `time_range` 是闭区间元组，开区间表达不了 ⇒ 拒收，不静默当无界（映射点同样**唯一**）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryQueryView {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub outcome: Option<TrajectoryOutcome>,
    #[serde(default)]
    pub min_quality: Option<f64>,
    #[serde(default)]
    pub min_value_score: Option<f64>,
    /// 时间窗起点（epoch 毫秒，含）；`None` = 不设下界。
    #[serde(default)]
    pub since_ms: Option<i64>,
    /// 时间窗终点（epoch 毫秒，含）；`None` = 不设上界。
    #[serde(default)]
    pub until_ms: Option<i64>,
    /// 返回条数上界，合法区间 [`TRAJECTORY_LIST_LIMIT_RANGE`]；`None` 由宿主按原形缺省
    /// （`TrajectoryQuery::default()` 的 100）取值并钳位到上界。
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `list` op 的返回：具名对象，不返裸数组。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryListOutcome {
    pub trajectories: Vec<TrajectorySummaryView>,
}

/// `query` op 的返回。
///
/// 与 [`TrajectoryListOutcome`] 今天同形，但**刻意不共用一个类型**：`list` 是「最近 N 条」，
/// `query` 是条件过滤 ⇒ 将来只会给后者加分页 / 总数字段，绑在一起会让两个 op 的形态互相牵制。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryQueryOutcome {
    pub trajectories: Vec<TrajectorySummaryView>,
}

/// `count` op 的返回：具名对象 `{"count": n}`，不返裸值
/// （与 `jobs.store` 的 `count` 载荷同风格；裸值会让插件侧要猜帧结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrajectoryCountOutcome {
    pub count: u64,
}

// ── Trait ───────────────────────────────────────────────────────────────

/// 轨迹库提供者（宿主独占 `TrajectoryStorage`，首批**只读**四个 op）。
///
/// 默认实现统一返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op。
///
/// 首批无写 op ⇒ 本 trait 没有需要 `write` 权限的方法（裁定与判据见模块头）。
#[async_trait]
pub trait TrajectoryStoreProvider: Send + Sync {
    /// 列表（`list` op）。落点是宿主 `get_trajectories(Option<usize>)`。
    ///
    /// `limit = None` ⇒ 由实现方取缺省并钳位到 [`TRAJECTORY_LIST_LIMIT_RANGE`]。
    /// 排序须**确定**（宿主按 `created_at` 倒序），否则插件侧断言不可复现。
    async fn list(&self, _limit: Option<usize>) -> Result<TrajectoryListOutcome> {
        Err(AxAgentError::Provider(
            "TrajectoryStoreProvider::list 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 单条（`get` op）。落点是宿主 `get_trajectory(&str)`。
    ///
    /// 查无 ⇒ `Ok(None)`（帧里是 `null`），**不是错误**、不借错误码（协议层无通用 NOT_FOUND）。
    async fn get(&self, _id: &str) -> Result<Option<TrajectorySummaryView>> {
        Err(AxAgentError::Provider(
            "TrajectoryStoreProvider::get 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 条件查询（`query` op）。落点是宿主 `query_trajectories(&TrajectoryQuery)`。
    ///
    /// 实现方把 [`TrajectoryQueryView`] 映射回原形再调用；映射点唯一，不得在多处各写一份
    /// （禁区 12）。零命中 ⇒ `trajectories` 为空数组，**不是错误**。
    async fn query(&self, _filter: TrajectoryQueryView) -> Result<TrajectoryQueryOutcome> {
        Err(AxAgentError::Provider(
            "TrajectoryStoreProvider::query 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 总数（`count` op）。落点是宿主 `count_trajectories()`。
    ///
    /// 返回具名对象 [`TrajectoryCountOutcome`]；`0` 是合法值（空库），**不是错误**。
    async fn count(&self) -> Result<TrajectoryCountOutcome> {
        Err(AxAgentError::Provider(
            "TrajectoryStoreProvider::count 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// 把序列化出的 JSON 键集合钉死：本两条件接缝**没有**门禁覆盖，测试就是唯一的形态锁。
    fn keys_of(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> =
            value.as_object().expect("DTO 必须序列化成对象").keys().cloned().collect();
        keys.sort();
        keys
    }

    fn expect_keys(value: &Value, expected: &[&str]) {
        let mut want: Vec<String> = expected.iter().map(|s| (*s).to_string()).collect();
        want.sort();
        assert_eq!(keys_of(value), want, "键集合漂移：{value:?}");
    }

    fn sample_view() -> TrajectorySummaryView {
        TrajectorySummaryView {
            id: "traj-1".into(),
            session_id: "sess-1".into(),
            user_id: "user-1".into(),
            agent_name: Some("股票分析师".into()),
            topic: "贵州茅台建仓".into(),
            summary_snippet: "先观察再小仓试买……（截断预览）".into(),
            outcome: TrajectoryOutcome::Success,
            duration_ms: 4200,
            quality: TrajectoryQuality::default(),
            value_score: 0.62,
            created_at_ms: 1_700_000_000_000,
            replay_count: 3,
        }
    }

    #[test]
    fn summary_view_key_set_is_exactly_twelve() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        expect_keys(
            &val,
            &[
                "id",
                "sessionId",
                "userId",
                "agentName",
                "topic",
                "summarySnippet",
                "outcome",
                "durationMs",
                "quality",
                "valueScore",
                "createdAtMs",
                "replayCount",
            ],
        );
        // snake_case 残留一律不许
        for bad in ["session_id", "agent_name", "summary_snippet", "duration_ms", "value_score"] {
            assert!(val.get(bad).is_none(), "不得残留 snake_case 键 {bad}");
        }
    }

    #[test]
    fn summary_view_carries_no_bulk_or_conversation_fields() {
        // 模块头「视图裁剪」清单的形态锁：这些字段哪天被加回来，本测试即红。
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        for forbidden in [
            "steps",
            "rewards",
            "patterns",
            "toolCalls",
            "tool_calls",
            "content",
            "reasoning",
            "toolResults",
            "summary",
            "systemPrompt",
            "lastReplayAt",
        ] {
            assert!(val.get(forbidden).is_none(), "{forbidden} 属大文本/会话原文，不过线");
        }
        // 嵌套的 quality 视图也必须是 camelCase（键集合逐字钉住）
        expect_keys(
            &val["quality"],
            &[
                "overall",
                "taskCompletion",
                "toolEfficiency",
                "reasoningQuality",
                "userSatisfaction",
            ],
        );
        // 时间只有一个口径
        assert_eq!(val["createdAtMs"], json!(1_700_000_000_000_i64));
        assert!(val.get("createdAt").is_none(), "DateTime 原形不过线，只发 epoch 毫秒");
    }

    #[test]
    fn summary_view_outcome_lowercase_literals_match_host() {
        let mut view = sample_view();
        view.agent_name = None;
        let val = serde_json::to_value(&view).expect("序列化应成功");
        assert_eq!(val["outcome"], json!("success"), "字面量须与宿主落库口径逐字相同");
        assert_eq!(val["agentName"], json!(null));

        for (outcome, literal) in [
            (TrajectoryOutcome::Success, "success"),
            (TrajectoryOutcome::Failure, "failure"),
            (TrajectoryOutcome::Partial, "partial"),
            (TrajectoryOutcome::Abandoned, "abandoned"),
        ] {
            view.outcome = outcome;
            let val = serde_json::to_value(&view).expect("序列化应成功");
            assert_eq!(val["outcome"], json!(literal));
        }
    }

    #[test]
    fn summary_view_roundtrip() {
        let back: TrajectorySummaryView =
            serde_json::from_value(serde_json::to_value(sample_view()).expect("序列化应成功"))
                .expect("往返应成立");
        assert_eq!(back.id, "traj-1");
        assert_eq!(back.session_id, "sess-1");
        assert_eq!(back.agent_name.as_deref(), Some("股票分析师"));
        assert_eq!(back.outcome, TrajectoryOutcome::Success);
        assert_eq!(back.duration_ms, 4200);
        assert_eq!(back.value_score, 0.62);
        assert_eq!(back.created_at_ms, 1_700_000_000_000);
        assert_eq!(back.replay_count, 3);
    }

    #[test]
    fn query_view_camel_case_epoch_and_all_omitted() {
        let filter = TrajectoryQueryView {
            session_id: Some("sess-1".into()),
            user_id: Some("user-1".into()),
            topic: Some("建仓".into()),
            outcome: Some(TrajectoryOutcome::Partial),
            min_quality: Some(0.7),
            min_value_score: Some(0.5),
            since_ms: Some(1_600_000_000_000),
            until_ms: Some(1_700_000_000_000),
            limit: Some(50),
        };
        let val = serde_json::to_value(&filter).expect("序列化应成功");
        expect_keys(
            &val,
            &[
                "sessionId",
                "userId",
                "topic",
                "outcome",
                "minQuality",
                "minValueScore",
                "sinceMs",
                "untilMs",
                "limit",
            ],
        );
        // 原形的 `time_range` 元组不得出现在接缝侧
        assert!(val.get("timeRange").is_none());
        assert!(val.get("time_range").is_none());
        assert!(val.get("min_quality").is_none());
        assert_eq!(val["outcome"], json!("partial"));
        assert_eq!(val["sinceMs"], json!(1_600_000_000_000_i64));

        // 全部字段可省略（九缺省 = 「无条件、由宿主取缺省 limit」）
        let empty: TrajectoryQueryView =
            serde_json::from_value(json!({})).expect("空对象应能反序列化成全 None 视图");
        assert!(empty.session_id.is_none());
        assert!(empty.since_ms.is_none());
        assert!(empty.until_ms.is_none());
        assert!(empty.outcome.is_none());
        assert!(empty.limit.is_none(), "limit 缺省是 None，钳位与默认值由宿主 provider 决定");
    }

    #[test]
    fn query_view_roundtrip() {
        let filter: TrajectoryQueryView = serde_json::from_value(json!({
            "outcome": "abandoned",
            "sinceMs": 1_600_000_000_000_i64,
            "limit": 20,
        }))
        .expect("部分字段应能反序列化");
        assert_eq!(filter.outcome, Some(TrajectoryOutcome::Abandoned));
        assert_eq!(filter.since_ms, Some(1_600_000_000_000));
        assert_eq!(filter.limit, Some(20));

        let back: TrajectoryQueryView =
            serde_json::from_value(serde_json::to_value(&filter).expect("序列化应成功"))
                .expect("往返应成立");
        assert_eq!(back.limit, Some(20));
        assert!(back.session_id.is_none());
    }

    #[test]
    fn list_and_query_outcomes_are_named_objects() {
        // 具名对象而非裸数组：插件侧按 `trajectories` 取，帧结构不随宿主实现漂移
        let list = TrajectoryListOutcome { trajectories: vec![sample_view()] };
        let val = serde_json::to_value(&list).expect("序列化应成功");
        expect_keys(&val, &["trajectories"]);
        assert_eq!(val["trajectories"].as_array().expect("应为数组").len(), 1);
        let back: TrajectoryListOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.trajectories[0].id, "traj-1");

        let query = TrajectoryQueryOutcome { trajectories: Vec::new() };
        let val = serde_json::to_value(&query).expect("序列化应成功");
        expect_keys(&val, &["trajectories"]);
        assert_eq!(val["trajectories"], json!([]), "零命中 = 空数组，不是错误");
        let back: TrajectoryQueryOutcome = serde_json::from_value(val).expect("往返应成立");
        assert!(back.trajectories.is_empty());
    }

    #[test]
    fn count_outcome_is_named_object() {
        let zero = TrajectoryCountOutcome { count: 0 };
        let val = serde_json::to_value(&zero).expect("序列化应成功");
        expect_keys(&val, &["count"]);
        assert_eq!(val["count"], json!(0), "空库是合法值，不返裸值也不报错");

        let back: TrajectoryCountOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.count, 0);
    }

    #[test]
    fn get_absent_is_null_not_error_shape() {
        // 「查无」的帧形态：Option 序列化成 null（协议层无通用 NOT_FOUND，沿 jobs.store 口径）
        let none: Option<TrajectorySummaryView> = None;
        assert_eq!(serde_json::to_value(&none).expect("序列化应成功"), json!(null));
        let some: Option<TrajectorySummaryView> = Some(sample_view());
        let val = serde_json::to_value(&some).expect("序列化应成功");
        assert_eq!(val["id"], json!("traj-1"));
        let back: Option<TrajectorySummaryView> = serde_json::from_value(val).expect("往返应成立");
        assert!(back.is_some());
    }

    #[test]
    fn limits_live_in_the_contract_layer() {
        // 数字住在契约层：宿主 provider 与分派层只准引用这两个常量
        assert_eq!(TRAJECTORY_LIST_LIMIT_RANGE, (1, 200));
        assert_eq!(TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS, 200);
        // 区间的自洽性由模块里的 `const _: () = assert!(…)` 在编译期钉住，
        // 故此处不写常量对常量的 assert!（会撞 clippy::assertions_on_constants）。
    }

    /// 未覆盖任何方法的实现：四个 op 全部吃默认实现 ⇒ 一律 `Err(Provider)`。
    struct BareStore;
    impl TrajectoryStoreProvider for BareStore {}

    #[test]
    fn default_methods_unimplemented() {
        let provider = BareStore;
        let checks: Vec<(&str, Result<Value>)> = vec![
            ("list", futures::executor::block_on(provider.list(Some(10))).map(|v| json!(v))),
            ("get", futures::executor::block_on(provider.get("traj-1")).map(|v| json!(v))),
            (
                "query",
                futures::executor::block_on(provider.query(TrajectoryQueryView {
                    session_id: Some("sess-1".into()),
                    user_id: None,
                    topic: None,
                    outcome: None,
                    min_quality: None,
                    min_value_score: None,
                    since_ms: None,
                    until_ms: None,
                    limit: None,
                }))
                .map(|v| json!(v)),
            ),
            ("count", futures::executor::block_on(provider.count()).map(|v| json!(v))),
        ];
        assert_eq!(checks.len(), 4);
        for (method, outcome) in checks {
            let err = outcome.expect_err("默认实现必须返回 Err");
            match err {
                AxAgentError::Provider(msg) => {
                    assert!(msg.contains("TrajectoryStoreProvider::"), "文案须点明 trait：{msg}");
                    assert!(msg.contains(method), "文案须点明方法 {method}：{msg}");
                },
                other => panic!("{method} 的默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn trait_is_object_safe() {
        // 注册进 capability_registry 靠 `Arc<dyn TrajectoryStoreProvider>`，对象安全是前提
        let provider: std::sync::Arc<dyn TrajectoryStoreProvider> = std::sync::Arc::new(BareStore);
        assert!(futures::executor::block_on(provider.count()).is_err());
    }
}
