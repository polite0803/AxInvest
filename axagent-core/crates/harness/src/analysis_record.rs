// SPDX-License-Identifier: AGPL-3.0-only

//! 单股分析记录只读接缝（`analysis.record`）— DTO + Trait。
//!
//! 宿主把 `stock_analyses` 表的**列表读面**经本接缝暴露给插件侧；插件经
//! `call_seam("analysis.record", "list", { limit?, offset? })` 按页读取。首批**只有 `list` 一个
//! op，且全是读面**（无写 op、无权限闸）。
//!
//! ## 为什么需要它（与 `workflow.run` 的分工）
//!
//! `workflow.run` 的 `list_recent` 只看得见**本插件自己发起的**执行（按 owner 隔离，
//! `workflow_run.rs:11-16`）—— 宿主侧工作台 / 定时任务发起的单股分析，插件一条都看不见。
//! 而任务中心的 `stock-analysis` 源读的是**记录表**（`list_stock_analyses` →
//! `stock_analyses`），不是执行表：一条记录有 `stockCode` / 决策结论 / 完成时刻这些执行视图
//! 里没有的列，且宿主发起的记录**没有本插件的 execution id**。两者既不同表也不同语义
//! （一个是「跑过什么」，一个是「得出过什么结论」），故不共用 `list_recent`。
//!
//! ## 行形状：宿主命令投影 ∪ 任务中心适配器，取并集
//!
//! 字段集合逐条对照两处**现存消费面**，不外扩：
//! - 宿主 `query_stock_analysis_items`（`src-tauri/src/commands/stock_analysis.rs`）的
//!   `StockAnalysisListItem`：它决定「列表能查到哪些列」；
//! - 前端 `mapStockAnalysisRun`（`src/pages/TasksCenterPage/adapters/stockAnalysis.adapter.ts`）
//!   与 `AnalysisSummary`（`src/types/stock-analysis.ts`）：它决定「列表真的会用到哪些列」。
//!
//! ## 视图裁剪：哪些字段永不过线
//!
//! - `decision_json`（宿主实体列）：决策 JSON 原文，含模型推理与逐档明细，单条可达数 KB；
//!   列表渲染需要的三格已各有专列（`decisionAction` / `decisionPositionState` /
//!   `decisionPositionPct`）⇒ **整列不过线**（裁剪先例 `jobs_store.rs:13-16` 的
//!   「`last_result.output` 不过帧」）。要读明细应走宿主详情命令，不在列表接缝上搬原文。
//!   ⚠ 如实的代价：宿主复用的列表查询（`query_stock_analysis_items`）当前**仍 SELECT 该列**
//!   （`StockAnalysisListItem` 的既有形状），本接缝只在映射时丢弃 ⇒ 它不会过帧，但仍会进宿主
//!   内存。要连读都省掉得给那条查询加列裁剪开关，那会改动既有 IPC 命令，不属本接缝的改动面。
//! - `blackboard_snapshot` / `decision_reasoning` / `llm_decision_json`：宿主列表投影本来就
//!   不 SELECT（见 `stock_analysis.rs` 的「排除大字段」注释），本接缝同样不取。
//! - 表里其余写面列（`config_id` / `source` / `synthetic` 等）：任务中心不用，且 `config_id`
//!   现网 0 行有值（`stockAnalysis.adapter.ts:55` 的注释实证「触发来源不可区分」）⇒ 不进视图。
//!
//! ## `total` 的口径
//!
//! `total` = 与 `records` **同一个集合**在 `limit` 之前的匹配总数（当前无过滤条件 ⇒
//! 即整表行数），照 `workflow_run.rs` 的 [`crate::WorkflowRunActivePage`] 口径。
//! ⚠ 宿主任务中心只展示**根记录**（`parentAnalysisId == null`，replay 子行由 time-travel
//! 源代表），但那层过滤在消费侧完成 —— 本接缝**不替调用方筛**，`parentAnalysisId` 原样下发，
//! 于是 `total` 与 `records.len()` 的差值天然包含子行。插件要算「根记录总数」必须自己按
//! `parentAnalysisId` 收窄（与宿主适配器的过滤条件同一条判据）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

// ── 硬上限（契约级数字，单源）────────────────────────────────────────────
//
// ⚠ 宿主 provider 与 plugins 分派层**只准引用这里**，不得另建一套（先例
// `trajectory_store.rs:95` 的 `TRAJECTORY_LIST_LIMIT_RANGE`）。

/// `list` 的 `limit` 合法闭区间。
///
/// 上界 200 的理由（与 [`crate::trajectory_store::TRAJECTORY_LIST_LIMIT_RANGE`] 同一推导）：
/// 每条只发裁剪后的行视图（无 `decisionJson`，字符串列都是代码 / 名称 / 日期量级），
/// 实测形态约 250–400 B/条 ⇒ 200 条一帧 ≈ 50–80 KB，远低于协议帧上限；下界 1：
/// `limit = 0` 没有合法语义，「不想要数据」应当不调用。
///
/// 注意它**比宿主 IPC 命令的 1000 更严**（`list_stock_analyses` 的 `MAX_LIMIT`）：那条命令面向
/// 同进程前端、不过帧；本接缝要过插件帧，帧预算是它自己的约束。两处数字不同不是漂移。
pub const ANALYSIS_RECORD_LIST_LIMIT_RANGE: (usize, usize) = (1, 200);

/// `list` 不传 `limit` 时的缺省页大小。
///
/// 定 50 的理由：宿主任务中心这条源自取的正是 50（`stockAnalysis.adapter.ts` 的
/// `{ limit: 50, offset: 0 }`），插件侧的缺省档也与它同值（`plugin-stock` 的
/// `DEFAULT_TASK_SOURCE_LIMIT`）⇒ 取现网消费方的既有读数，不另拍一个数。
pub const ANALYSIS_RECORD_LIST_DEFAULT_LIMIT: usize = 50;

// 区间自洽 + 缺省值落在区间内（任一写错，`list` 会在宿主侧恒拒或恒夹取）。
const _: () = assert!(ANALYSIS_RECORD_LIST_LIMIT_RANGE.0 <= ANALYSIS_RECORD_LIST_LIMIT_RANGE.1);
const _: () = assert!(
    ANALYSIS_RECORD_LIST_DEFAULT_LIMIT >= ANALYSIS_RECORD_LIST_LIMIT_RANGE.0
        && ANALYSIS_RECORD_LIST_DEFAULT_LIMIT <= ANALYSIS_RECORD_LIST_LIMIT_RANGE.1
);

// ── DTO（跨边界视图；宿主内部实体不直接过线）────────────────────────────

/// 一条单股分析记录的**列表视图**（`stock_analyses` 的裁剪投影）。
///
/// 时间口径只有一个单位：epoch **毫秒**（`i64`），与宿主列 `created_at` / `updated_at` /
/// `completed_at` 同单位，不做二次换算，也不发 `chrono` 字符串。
///
/// 字段 `None` 的语义逐条写死（`None` ≠ 0，也 ≠ 某档默认）：
/// - `decisionAction`：宿主已按「缺失即权威哨兵」补齐非 `running` 行的空值
///   （`stock_analysis.rs` 的 2026-10-22 裁定）⇒ `None` 只在 `status == "running"` 时出现；
/// - `completedAt`：`None` = 完成时刻未知（本列引入前的历史行 / `interrupted`）。
///   **不得**回退到 `updatedAt`（后者会被交易意图审核 / 结果回写顶掉）；
/// - `executionQuality`：`None` = 该次分析尚无执行层反思（闭环未跑/失败），**不是 0 分**；
/// - `templateVersion` / `templateId` / `decisionTimeHorizon` / `decisionHorizonSource`：
///   `None` = 该列引入前的记录 ⇒ 按「未知」渲染，不得回退成 0 / 当前代 / 某档。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRecordView {
    /// 记录 id（任务中心 deepLink 与详情查询的唯一句柄）。
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 分析日期 `YYYY-MM-DD`（宿主列 `analysis_date`，不是时间戳）。
    pub analysis_date: String,
    /// 记录状态，字面量与宿主落库一致（`running` / `completed` / `failed` / `error` /
    /// `interrupted` / `paused` / `queued` / `pending` …）⇒ 状态词表翻译归消费方
    /// （宿主适配器 `mapAnalysisState` / 插件的同一张表），本接缝不改口径。
    pub status: String,
    pub decision_action: Option<String>,
    /// 决策持仓状态轴（`EMPTY` / `OPENING` / `HOLDING` / `TRIMMING`）；`None` = 该列引入前的记录。
    pub decision_position_state: Option<String>,
    /// 决策仓位百分比（0–100）。
    pub decision_position_pct: Option<f64>,
    /// `"live"` | `"replay"`（宿主 `analysis_kind`）。
    pub analysis_kind: String,
    /// 回放基准日 `YYYY-MM-DD`（仅 replay 非空）。
    pub as_of_date: Option<String>,
    /// 父记录 id：`None` = 根记录。replay 分析是时间旅行批次的子行，任务中心只展示根记录
    /// ⇒ 消费方按本字段过滤（判据见模块头「`total` 的口径」）。
    pub parent_analysis_id: Option<String>,
    /// 生成该记录的工作流模板 id（`stock-analysis` / `stock-analysis-fast`）；
    /// `None` = 该列引入前的记录或非模板产出，**不得**据此推断链路。
    pub template_id: Option<String>,
    /// 生成该记录时实际使用的图代（`workflow_templates.version` 快照）。
    pub template_version: Option<i32>,
    /// 主结论的周期档（`ultra_short` / `short` / `mid` / `long`）；`None` = 档位未知。
    pub decision_time_horizon: Option<String>,
    /// 主档来源，值域权威 = [`crate::holding_period::HORIZON_SOURCES`]。
    pub decision_horizon_source: Option<String>,
    /// 创建时刻（epoch 毫秒）。
    pub created_at: i64,
    /// 最近一次写库时刻（epoch 毫秒）——**不是**结束时刻，别拿它算耗时。
    pub updated_at: i64,
    /// 完成时刻（epoch 毫秒）；`None` = 未知（见类型级注释）。
    pub completed_at: Option<i64>,
    /// 执行层反思总体质量分（1–10，工作流口径）；`None` = 尚无反思，**不是 0 分**。
    pub execution_quality: Option<i32>,
}

/// `list` 的返回：具名对象（不返裸数组），行 + `limit` 之前的匹配总数。
///
/// 具名对象而非裸数组的理由同 `jobs.store` 的 `count`（`{"count": n}`）：裸数组没地方挂
/// `total`，插件侧就只能拿「本页不满」猜「没有下一页」，而整页恰好填满时那个推断是错的。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRecordListOutcome {
    /// 本页记录（宿主按 `created_at` **降序**；同刻并列的先后不在契约内）。
    pub records: Vec<AnalysisRecordView>,
    /// `limit` **之前**的匹配总数（当前无过滤条件 ⇒ 整表行数）。
    pub total: u64,
}

// ── Trait ───────────────────────────────────────────────────────────────

/// 单股分析记录提供者（宿主独占 `stock_analyses` 读面，首批只有 `list`）。
///
/// 实现方：宿主 wiring 层（`src-tauri/src/init/analysis_record_service.rs`，持 `DatabaseConnection`）。
/// 默认实现返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op
/// （与 `JobStoreProvider` / `TrajectoryStoreProvider` 同款缺省）。
///
/// 首批无写 op ⇒ 本 trait 没有需要 `write` 权限的方法，`worker.rs` 的
/// `required_permission_for` 也**一条都不许加**（多登记一条会让只读插件被无端拒掉）。
#[async_trait]
pub trait AnalysisRecordProvider: Send + Sync {
    /// 列表（`list` op）。
    ///
    /// - `limit = None` ⇒ 取 [`ANALYSIS_RECORD_LIST_DEFAULT_LIMIT`]；显式值越出
    ///   [`ANALYSIS_RECORD_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)` 拒收（**不静默夹取**：
    ///   夹取会让插件拿到的条数与它以为的不同，分页据此算错）；
    /// - `offset = None` ⇒ `0`；越出宿主 `u32` 参数带（> `u32::MAX`）⇒ `Err(Validation)`；
    /// - 排序由宿主决定（`ORDER BY created_at DESC`）；**同一时刻并列的先后不在契约内**
    ///   （宿主投影没有第二排序键）⇒ 插件不得依赖两同刻行的相对顺序；
    /// - 空表 / 越界 offset ⇒ `records` 空数组、`total` 照常返回，**不是错误**。
    async fn list(
        &self,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> Result<AnalysisRecordListOutcome> {
        Err(AxAgentError::Provider(
            "AnalysisRecordProvider::list 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── 测试 ────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// 把序列化出的 JSON 键集合钉死：本接缝**没有**登记表门禁覆盖，测试就是唯一的形态锁。
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

    fn sample_view() -> AnalysisRecordView {
        AnalysisRecordView {
            id: "analysis-1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            analysis_date: "2026-11-05".into(),
            status: "completed".into(),
            decision_action: Some("BUY".into()),
            decision_position_state: Some("HOLDING".into()),
            decision_position_pct: Some(12.5),
            analysis_kind: "live".into(),
            as_of_date: None,
            parent_analysis_id: None,
            template_id: Some("stock-analysis".into()),
            template_version: Some(143),
            decision_time_horizon: Some("short".into()),
            decision_horizon_source: Some("branch_pick".into()),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_060_000,
            completed_at: Some(1_700_000_050_000),
            execution_quality: Some(7),
        }
    }

    /// 行键集合逐字钉住：19 个键，全部 camelCase。
    #[test]
    fn record_view_key_set_is_exactly_nineteen_camel_case() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        expect_keys(
            &val,
            &[
                "id",
                "stockCode",
                "stockName",
                "analysisDate",
                "status",
                "decisionAction",
                "decisionPositionState",
                "decisionPositionPct",
                "analysisKind",
                "asOfDate",
                "parentAnalysisId",
                "templateId",
                "templateVersion",
                "decisionTimeHorizon",
                "decisionHorizonSource",
                "createdAt",
                "updatedAt",
                "completedAt",
                "executionQuality",
            ],
        );
        // snake_case 残留一律不许（全站 camelCase 约定，禁区 13）
        for bad in [
            "stock_code",
            "stock_name",
            "analysis_date",
            "decision_action",
            "decision_position_state",
            "decision_position_pct",
            "analysis_kind",
            "as_of_date",
            "parent_analysis_id",
            "template_id",
            "template_version",
            "decision_time_horizon",
            "decision_horizon_source",
            "created_at",
            "updated_at",
            "completed_at",
            "execution_quality",
        ] {
            assert!(val.get(bad).is_none(), "不得残留 snake_case 键 {bad}");
        }
    }

    /// 模块头「视图裁剪」清单的形态锁：这些量哪天被加回来，本测试即红。
    #[test]
    fn record_view_carries_no_bulk_columns() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        for forbidden in [
            "decisionJson",
            "decision_json",
            "blackboardSnapshot",
            "decisionReasoning",
            "llmDecisionJson",
            "configId",
            "source",
            "synthetic",
        ] {
            assert!(val.get(forbidden).is_none(), "{forbidden} 属大字段/写面列，不过线");
        }
    }

    /// 时间只有 epoch 毫秒一个单位：不要出 `Date` 字符串、也不要把毫秒当秒。
    #[test]
    fn record_view_time_is_epoch_millis_only() {
        let val = serde_json::to_value(sample_view()).expect("序列化应成功");
        assert_eq!(val["createdAt"], json!(1_700_000_000_000_i64));
        assert_eq!(val["completedAt"], json!(1_700_000_050_000_i64));
        for bad in ["createdAtMs", "completedAtMs", "createdAtIso"] {
            assert!(val.get(bad).is_none(), "不得造第二套时间口径 {bad}");
        }
    }

    /// `None` 与 `0` / 空串必须可区分：未知的完成时刻与未知的质量分不能伪装成读数。
    #[test]
    fn none_fields_stay_null_not_zero() {
        let mut view = sample_view();
        view.completed_at = None;
        view.execution_quality = None;
        view.decision_action = None;
        let val = serde_json::to_value(&view).expect("序列化应成功");
        assert_eq!(val["completedAt"], json!(null));
        assert_eq!(val["executionQuality"], json!(null));
        assert_eq!(val["decisionAction"], json!(null));
    }

    #[test]
    fn record_view_roundtrip() {
        let back: AnalysisRecordView =
            serde_json::from_value(serde_json::to_value(sample_view()).expect("序列化应成功"))
                .expect("往返应成立");
        assert_eq!(back.id, "analysis-1");
        assert_eq!(back.stock_code, "600519");
        assert_eq!(back.decision_position_pct, Some(12.5));
        assert_eq!(back.template_version, Some(143));
        assert_eq!(back.completed_at, Some(1_700_000_050_000));
        assert_eq!(back.execution_quality, Some(7));
    }

    /// 出参是具名对象：`{records, total}`，不是裸数组（裸数组没地方挂 `total`）。
    #[test]
    fn list_outcome_is_named_object() {
        let empty = AnalysisRecordListOutcome { records: Vec::new(), total: 0 };
        let val = serde_json::to_value(&empty).expect("序列化应成功");
        expect_keys(&val, &["records", "total"]);
        assert_eq!(val["records"], json!([]), "空表 = 空数组，不是错误");
        assert_eq!(val["total"], json!(0), "空库的 0 是合法读数");

        let one = AnalysisRecordListOutcome { records: vec![sample_view()], total: 338 };
        let val = serde_json::to_value(&one).expect("序列化应成功");
        assert_eq!(val["total"], json!(338));
        assert_eq!(val["records"].as_array().expect("应为数组").len(), 1);
        let back: AnalysisRecordListOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.total, 338);
        assert_eq!(back.records[0].id, "analysis-1");
    }

    /// 数字住在契约层：宿主 provider 与分派层只准引用这两个常量。
    #[test]
    fn limits_live_in_the_contract_layer() {
        assert_eq!(ANALYSIS_RECORD_LIST_LIMIT_RANGE, (1, 200));
        assert_eq!(ANALYSIS_RECORD_LIST_DEFAULT_LIMIT, 50);
        // 区间自洽性由模块里的 `const _: () = assert!(…)` 在编译期钉住，
        // 故此处不写常量对常量的 assert!（会撞 clippy::assertions_on_constants）。
    }

    /// 未覆盖任何方法的实现：`list` 吃默认实现 ⇒ `Err(Provider)`。
    struct BareStore;
    impl AnalysisRecordProvider for BareStore {}

    #[test]
    fn default_method_is_unimplemented() {
        let provider = BareStore;
        let err = futures::executor::block_on(provider.list(Some(10), None))
            .expect_err("默认实现必须是错误");
        match err {
            AxAgentError::Provider(message) => {
                assert!(
                    message.contains("AnalysisRecordProvider::list"),
                    "文案须点明 trait 与方法：{message}"
                );
            },
            other => panic!("默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
        }
    }

    #[test]
    fn trait_is_object_safe() {
        // 注册进 capability_registry 靠 `Arc<dyn AnalysisRecordProvider>`，对象安全是前提
        let provider: std::sync::Arc<dyn AnalysisRecordProvider> = std::sync::Arc::new(BareStore);
        assert!(futures::executor::block_on(provider.list(None, None)).is_err());
    }
}
