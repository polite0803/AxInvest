// SPDX-License-Identifier: AGPL-3.0-only

//! `analysis.record` 接缝的**宿主侧实现**（wiring 层，宿主独占 `DatabaseConnection`）。
//!
//! 契约在 `axagent-core/crates/harness/src/analysis_record.rs`（首批**只读**，唯一 op = `list`）。
//! 本文件只做「契约入参 → 宿主查询」的接线与「宿主行 → 契约视图」的裁剪。
//!
//! ## 为什么落在 `init/` 而不是 `commands/`
//!
//! 分层门禁规则 1（`scripts/check-layer-discipline.mjs` 的 `commands-no-direct-db`）禁止
//! `src/commands/**` 直连 `sea_orm` / `axagent_entities`，而 `total` 必须是一次 `COUNT(*)`
//! （不能拿「本页不满」推断总数）。本文件是 **wiring**（AGENTS.md「wiring：全栈胶水」把
//! `src/commands/` 与 `src/init/` 同列），落在 `init/` 就不必为一次计数去改 dao（本批文件面
//! 不含 `axagent-infra`）。同目录已有 `init/plugin_schema.rs` / `init/stock_domain_tools.rs`
//! 这类「装配期直接用 ORM」的先例。
//!
//! ## 投影唯一（禁区 12）
//!
//! 行数据**不另写一份 SELECT**：直接复用宿主既有的
//! [`crate::commands::stock_analysis::query_stock_analysis_items`]（同一个「排除大字段」列清单、
//! 同一套 `decision_action` 空值哨兵、同一次执行层质量分批量取）。那份查询是任务中心历史列表的
//! 唯一投影来源；接缝侧再抄一份列清单，两份就会各漂各的（并且新列只会被加进其中一份）。
//! 本文件只负责把它映射成契约视图（[`record_view`]，映射点唯一）。
//!
//! 复用带来的一个如实口径：那个函数返回的是 `Result<_, String>`（Tauri 命令形态），
//! 本层把它包成 [`AxAgentError::Provider`]。所以 DB 报错的**结构化错误码**
//! （`WF_INTERNAL`）在这一层丢失，分派层会把它落成 `PLUGIN_CALL_FAILED` + message。
//! 这不影响可读性（详情里带原始错误串），但确实不是错误码级的中继 —— 若将来要错误码中继，
//! 应在 `query_stock_analysis_items` 侧改成返回 `AxAgentError`（那会牵动既有 IPC 命令）。
//!
//! ## 视图裁剪
//!
//! [`record_view`] 是唯一映射点：`decision_json`（决策 JSON 原文，单条可达数 KB）与
//! 表里的写面列一律不取 —— 裁剪清单与理由见契约模块头。`decisionAction` 的「空值哨兵」
//! 已由复用的查询做过（`running` 行除外），本层不再补第二次。
//!
//! ## 闸（越界拒收，不静默夹取）
//!
//! - `limit`：`None` ⇒ [`ANALYSIS_RECORD_LIST_DEFAULT_LIMIT`]；显式值越出
//!   [`ANALYSIS_RECORD_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)`。为什么不学宿主 IPC 命令那样
//!   夹到 1000：那条命令面向同进程前端，越界截断只丢一点数据；插件侧的 `limit` 会参与
//!   分页算术，夹取后「我给 500 却只回 200」会让它把下一页的 offset 算错（同一判据见
//!   `trajectory_store_service.rs` 的 `resolve_limit`）。
//! - `offset`：`None` ⇒ `0`；> `u32::MAX` ⇒ `Err(Validation)`（宿主查询函数收 `u32`，
//!   `as u32` 会静默截断成一个完全不同的位置）。
//!
//! 闸在 IO **之前**执行：越界参数不该换来一次全表扫描。

use async_trait::async_trait;
use axagent_entities::stock_analyses;
use axagent_harness::DatabaseConnection;
use axagent_harness::analysis_record::{
    ANALYSIS_RECORD_LIST_DEFAULT_LIMIT, ANALYSIS_RECORD_LIST_LIMIT_RANGE,
    AnalysisRecordListOutcome, AnalysisRecordProvider, AnalysisRecordView,
};
use axagent_harness::core_error::{AxAgentError, Result};
use sea_orm::{EntityTrait, PaginatorTrait};

use crate::commands::stock_analysis::{StockAnalysisListItem, query_stock_analysis_items};

/// `analysis.record` 的宿主实现。
pub struct HostAnalysisRecordStore {
    /// 与 `AppState.harness.db()` **同一个**连接池句柄（`DatabaseConnection` 是 Clone 的句柄，
    /// 不是第二套连接语义）。
    db: DatabaseConnection,
}

impl HostAnalysisRecordStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

// ── 纯函数：闸与映射（本文件全部语义的落点）─────────────────────────────────

/// `limit` 闸：越出 [`ANALYSIS_RECORD_LIST_LIMIT_RANGE`] ⇒ `Err(Validation)`，**不夹取**。
fn resolve_limit(limit: Option<usize>) -> Result<usize> {
    let effective = limit.unwrap_or(ANALYSIS_RECORD_LIST_DEFAULT_LIMIT);
    if (ANALYSIS_RECORD_LIST_LIMIT_RANGE.0..=ANALYSIS_RECORD_LIST_LIMIT_RANGE.1)
        .contains(&effective)
    {
        return Ok(effective);
    }
    Err(AxAgentError::Validation(format!(
        "analysis.record::list 的 limit={effective} 越出契约区间 {}..={}（越界拒收，不静默夹取：\
         插件按 limit 算分页 offset，夹取会让下一页落在错误位置）",
        ANALYSIS_RECORD_LIST_LIMIT_RANGE.0, ANALYSIS_RECORD_LIST_LIMIT_RANGE.1
    )))
}

/// `offset` 闸：宿主查询函数收 `u32`，超出即拒（`as u32` 会静默截断到另一个位置）。
fn resolve_offset(offset: Option<usize>) -> Result<u32> {
    let effective = offset.unwrap_or(0);
    u32::try_from(effective).map_err(|_| {
        AxAgentError::Validation(format!(
            "analysis.record::list 的 offset={effective} 超出宿主查询可接受的 u32 范围\
             （静默截断会把本页挪到完全不同的位置）"
        ))
    })
}

/// 宿主列表行 → 契约视图（**唯一**映射点：裁剪与字段改名都在这里）。
///
/// 逐字段一一对应，没有派生 / 换算：宿主列与契约字段同名同单位（时间是 epoch 毫秒）。
/// `decision_json` 与表里的写面列在这里被甩掉 ⇒ **不过帧**。⚠ 如实：复用的宿主查询
/// （`query_stock_analysis_items` 的列清单）**仍会 SELECT `decision_json`** —— 那是
/// `StockAnalysisListItem` 的既有形状，本层不另写 SELECT 是为投影唯一（禁区 12）；代价是该列
/// 会进宿主内存（只是不出现在契约视图里）。要连读都省掉需改那条既有 IPC 查询。
fn record_view(row: StockAnalysisListItem) -> AnalysisRecordView {
    AnalysisRecordView {
        id: row.id,
        stock_code: row.stock_code,
        stock_name: row.stock_name,
        analysis_date: row.analysis_date,
        status: row.status,
        decision_action: row.decision_action,
        decision_position_state: row.decision_position_state,
        decision_position_pct: row.decision_position_pct,
        analysis_kind: row.analysis_kind,
        as_of_date: row.as_of_date,
        parent_analysis_id: row.parent_analysis_id,
        template_id: row.template_id,
        template_version: row.template_version,
        decision_time_horizon: row.decision_time_horizon,
        decision_horizon_source: row.decision_horizon_source,
        created_at: row.created_at,
        updated_at: row.updated_at,
        completed_at: row.completed_at,
        execution_quality: row.execution_quality,
    }
}

// ── Trait 实现 ────────────────────────────────────────────────────────────

#[async_trait]
impl AnalysisRecordProvider for HostAnalysisRecordStore {
    /// 列表：一页记录 + `limit` 之前的匹配总数。
    async fn list(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<AnalysisRecordListOutcome> {
        // 闸在 IO 之前：越界参数不该换来一次全表扫描。
        let limit = resolve_limit(limit)?;
        let offset = resolve_offset(offset)?;

        let rows =
            query_stock_analysis_items(&self.db, limit as u32, offset).await.map_err(|e| {
                AxAgentError::Provider(format!("analysis.record::list 取分析记录失败: {e}"))
            })?;

        // `total` 是 `limit` 之前的匹配总数；本接缝没有过滤条件 ⇒ 整表行数。
        // 不复用 `rows.len()` 兜：那会把「最后一页不满」读成「总共就这么多」。
        let total = stock_analyses::Entity::find()
            .count(&self.db)
            .await
            .map_err(|e| AxAgentError::Provider(format!("analysis.record::list 计数失败: {e}")))?;

        Ok(AnalysisRecordListOutcome {
            records: rows.into_iter().map(record_view).collect(),
            total,
        })
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露）。
//
// 这里只测「闸 + 映射」这两件本文件自己的事：投影查询（列清单 / 空值哨兵 / 质量分）归
// `commands/stock_analysis.rs` 既有测试，真库端到端读属跨批项（需要预置 `stock_analyses`
// 行的迁移之外数据面）。

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn sample_row() -> StockAnalysisListItem {
        StockAnalysisListItem {
            id: "analysis-1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            analysis_date: "2026-11-05".into(),
            status: "completed".into(),
            decision_action: Some("BUY".into()),
            decision_position_state: Some("HOLDING".into()),
            decision_position_pct: Some(12.5),
            // 大字段：映射函数必须把它丢掉（下面按键集合钉住）。
            decision_json: Some("{\"reasoning\":\"很长的原文……\"}".into()),
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

    #[test]
    fn limit_gate_pins_the_contract_range() {
        let (low, high) = ANALYSIS_RECORD_LIST_LIMIT_RANGE;
        assert_eq!(resolve_limit(Some(low)).unwrap(), low);
        assert_eq!(resolve_limit(Some(high)).unwrap(), high);
        for bad in [0, high + 1, usize::MAX] {
            let err = resolve_limit(Some(bad)).expect_err("越界应拒收，不该夹取");
            assert!(matches!(err, AxAgentError::Validation(_)), "越界应落成 Validation：{err:?}");
        }
        // 缺省取契约层那一份（不是本文件另抄的数字）。
        assert_eq!(resolve_limit(None).unwrap(), ANALYSIS_RECORD_LIST_DEFAULT_LIMIT);
    }

    #[test]
    fn offset_gate_rejects_beyond_u32() {
        assert_eq!(resolve_offset(None).unwrap(), 0, "缺省 offset = 0");
        assert_eq!(resolve_offset(Some(100)).unwrap(), 100);
        assert_eq!(
            resolve_offset(Some(u32::MAX as usize)).unwrap(),
            u32::MAX,
            "边界值本身合法（不静默截断）"
        );
        // u32::MAX + 1 若被 `as u32` 截断会变成 0 ⇒ 必须拒收而不是回到第一页。
        let err = resolve_offset(Some(u32::MAX as usize + 1)).expect_err("超出 u32 应拒收");
        assert!(matches!(err, AxAgentError::Validation(_)), "应为 Validation：{err:?}");
    }

    #[test]
    fn record_view_maps_every_field_and_drops_bulk_column() {
        let view = record_view(sample_row());
        assert_eq!(view.id, "analysis-1");
        assert_eq!(view.stock_code, "600519");
        assert_eq!(view.status, "completed");
        assert_eq!(view.decision_action.as_deref(), Some("BUY"));
        assert_eq!(view.decision_position_pct, Some(12.5));
        assert_eq!(view.template_version, Some(143));
        assert_eq!(view.created_at, 1_700_000_000_000);
        assert_eq!(view.completed_at, Some(1_700_000_050_000));
        assert_eq!(view.execution_quality, Some(7));

        let val: Value = serde_json::to_value(&view).expect("序列化应成功");
        // 键集合逐字钉住：多一个键 = 有大字段漏过线
        let mut keys: Vec<String> = val.as_object().expect("应为对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "analysisDate",
                "analysisKind",
                "asOfDate",
                "completedAt",
                "createdAt",
                "decisionAction",
                "decisionHorizonSource",
                "decisionPositionPct",
                "decisionPositionState",
                "decisionTimeHorizon",
                "executionQuality",
                "id",
                "parentAnalysisId",
                "status",
                "stockCode",
                "stockName",
                "templateId",
                "templateVersion",
                "updatedAt",
            ]
        );
        assert!(val.get("decisionJson").is_none(), "决策 JSON 原文不过线");
    }

    #[test]
    fn record_view_keeps_unknown_as_null() {
        let mut row = sample_row();
        row.completed_at = None;
        row.execution_quality = None;
        row.decision_position_state = None;
        let view = record_view(row);
        let val = serde_json::to_value(&view).expect("序列化应成功");
        assert_eq!(val["completedAt"], json!(null), "未知完成时刻不得回退成 updatedAt");
        assert_eq!(val["executionQuality"], json!(null), "无反思不是 0 分");
        assert_eq!(val["decisionPositionState"], json!(null), "该列引入前不得读成 EMPTY");
    }
}
