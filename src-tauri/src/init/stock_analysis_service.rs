// SPDX-License-Identifier: AGPL-3.0-only

//! `stock.analysis` 接缝的**宿主侧实现**（wiring 层）。
//!
//! 契约在 `axagent-core/crates/harness/src/stock_analysis.rs`（三个 op：两个读 + 一个写）。
//! 三个 op 的实现体在 `analysis-engine`（implementor，持 `entities`）；本文件是宿主与接缝
//! 之间**唯一**的 `Value` ↔ 域 DTO 转换点。
//!
//! ## 为什么转换点必须唯一
//!
//! harness 是 foundation，按依赖铁律不得依赖 `analysis-engine`（implementor）⇒ 接缝上
//! 只能传 `serde_json::Value`，DTO 权威仍在域包。于是「JSON → DTO」这一步必须只出现在
//! 一处：本文件。插件侧、worker 分派层都只透传，任何一层多做一次字段改写都是第二套形状
//! （禁区 12），而第二套形状的失败形态是**静默**的（字段名漂一个字母不会报错，
//! `serde` 的 `#[serde(default)]` 会把它当成「调用方没给」）。
//!
//! ## 为什么不在这层写 SQL
//!
//! 三个权威函数（`fetch_industry_constituents` / `batch_upsert_mainlines` /
//! `fetch_stock_chg_1m`）已经是 `analysis-engine` 的**唯一实现**，宿主 IPC 命令
//! （`src/commands/` 侧）与工作流工具此前都指向它们。本层再拼一次 `Entity::find()` 就会
//! 出现两份列清单 / 两套归一逻辑（同 `opc_store_service.rs` 模块头的判据）。
//!
//! ## 与工具出参信封的关系
//!
//! ⚠ **接缝出参不是工具出参信封**：`{status, data, detail}` / `{inserted, updated,
//! archived}` 这层包装归**工具**（`axagent-runtime/crates/tools/src/tools/`），本层只回域
//! DTO 的 JSON（`{groups, universeTotal, …}` / `{rows, requested, …}`）。判据是「信封是
//! 工具面契约，换调用通道（工作流工具 / 智能体命令 / 前端 invoke）时形状不变的是信封之外的
//! 数据面」。把信封也搬进接缝，会让另一个消费方（插件命令的直接调用者）拿到一个它不认识的
//! `data` 层套娃。
//!
//! ## 闸
//!
//! - `industry_constituents`：空 `industries` ⇒ `Err(Validation)`。判据沿用工具侧原文
//!   （「至少给一个行业名」）—— 空清单换来的是「全宇宙空结果」，那看起来像「这些行业没有
//!   成分股」，是另一种谎。
//! - `stock_factor`：入参是**宽容形态**的代码清单（字符串数组 / 代码对象数组 /
//!   `{stocks: [...]}` 信封），与工具入参逐字相同 —— `parse_stock_refs` 的权威在
//!   `analysis-engine`（implementor），工具侧（consumer）不得依赖它，故归一只能在此处。
//!   归一后为空 ⇒ `Err(Validation)`。同判据。
//! - `batch_upsert_mainlines`：**不做字段级闸**，一律交给 `BatchUpsertInput` 的 `serde`
//!   （它的 `#[serde(default)]` + `alias` 是唯一权威，且缺 `mainline_date` 必须报错 ——
//!   见 `market_mainline.rs` 的 `missing_required_field_is_an_error`）。本层预判一次
//!   「字段在不在」就会与该 DTO 的演进脱钩。

use async_trait::async_trait;
use axagent_analysis_engine::industry_constituents::{
    IndustryConstituentsInput, fetch_industry_constituents,
};
use axagent_analysis_engine::market_mainline::{BatchUpsertInput, batch_upsert_mainlines};
use axagent_analysis_engine::stock_factor::{fetch_stock_chg_1m, parse_stock_refs};
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::stock_analysis::StockAnalysisProvider;
use serde::Deserialize;
use serde_json::Value;

/// `stock_factor` op 的入参：**宽容形态原样**（与工具入参逐字相同）。
///
/// 归一必须在这里做、也只在这里做：`parse_stock_refs` 的权威实现在
/// `analysis-engine`（implementor），工具侧（`axagent-runtime/crates/tools`，consumer）
/// 不得依赖它 —— 那正是本批要消掉的跨包口。工具只做「明显缺料」闸，宽容扁平化
/// （字符串数组 / 对象数组 / `{stocks: [...]}` 信封）归本层。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StockFactorRequest {
    /// 宽容形态的股票代码清单，交给 `parse_stock_refs` 归一。
    ///
    /// 主名 camelCase（禁区 13），但工具与工作流模板传的是 snake_case 的 `stock_codes`
    /// （插件侧原样透传、不改名）⇒ 必须收下 alias，否则整个读 op 会因「缺字段」失败。
    #[serde(alias = "stock_codes")]
    stock_codes: Value,
    /// 可选截止日（`YYYY-MM-DD`）；缺省 = 各票序列最新。
    #[serde(default, alias = "as_of_date")]
    as_of_date: Option<String>,
}

/// `stock.analysis` 的宿主实现。
pub struct HostStockAnalysis {
    /// 与 `AppState.harness.db()` **同一个**连接池句柄（Clone 是句柄语义，不是第二套连接）。
    db: DatabaseConnection,
}

impl HostStockAnalysis {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

// ── Trait 实现 ────────────────────────────────────────────────────────────

#[async_trait]
impl StockAnalysisProvider for HostStockAnalysis {
    /// 读 op：按行业名取全 A 股成分股分组。
    async fn industry_constituents(&self, input: Value) -> Result<Value> {
        let parsed: IndustryConstituentsInput = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::industry_constituents 参数解析失败: {e}。期望 \
                 {{industries: [...]}}（元素为行业名字符串或 {{name, leaderCode}}）"
            ))
        })?;
        // 闸在 IO 之前：空清单换来的「全宇宙空结果」会被读成「这些行业没有成分股」。
        if parsed.industries.is_empty() {
            return Err(AxAgentError::Validation(
                "stock.analysis::industry_constituents industries 为空：至少给一个行业名".into(),
            ));
        }
        let report = fetch_industry_constituents(&self.db, &parsed).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::industry_constituents 查询失败: {e}"))
        })?;
        serde_json::to_value(&report).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::industry_constituents 序列化失败: {e}"))
        })
    }

    /// 写 op：按日批量 upsert 市场主线（`archive_missing = true` 会归档当日未提及的主线）。
    ///
    /// 权限闸**不在本层**（`worker.rs` 的 `required_permission_for` 已登记 `Write`，
    /// 本 trait 的文档也写明不重复校验）。本层只做形状转换与错误归类。
    async fn batch_upsert_mainlines(&self, input: Value) -> Result<Value> {
        // **不做字段级预判**：`BatchUpsertInput` 的 serde 是形状唯一权威（含 snake/camel
        // 双写 alias 与「缺 mainline_date 必须失败」）。预判会与该 DTO 的演进脱钩。
        let params: BatchUpsertInput = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::batch_upsert_mainlines 参数解析失败: {e}。期望字段: \
                 mainline_date(YYYY-MM-DD) / mainlines(数组) / archive_missing(可选 bool)"
            ))
        })?;
        let result = batch_upsert_mainlines(&self.db, params).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::batch_upsert_mainlines 写入失败: {e}"))
        })?;
        serde_json::to_value(&result).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::batch_upsert_mainlines 序列化失败: {e}"
            ))
        })
    }

    /// 读 op：逐票 1 月涨跌幅（反转因子原料）。
    async fn stock_factor(&self, input: Value) -> Result<Value> {
        let request: StockFactorRequest = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::stock_factor 参数解析失败: {e}。期望 {{stockCodes: ..., \
                 asOfDate?: \"YYYY-MM-DD\"}}（stockCodes 为宽容形态：字符串数组 / \
                 代码对象数组 / 含 stocks 分组的信封）"
            ))
        })?;
        // 归一一律在本层做（`parse_stock_refs` 属 implementor，工具侧够不着）。
        let codes = parse_stock_refs(&request.stock_codes);
        // 闸在 IO 之前：空清单换来的「0 只票、全部缺席」会被读成「这些票都没数据」。
        if codes.is_empty() {
            return Err(AxAgentError::Validation(
                "stock.analysis::stock_factor codes 为空：无法打分（缺席要显式报错，不得伪装成空结果）"
                    .into(),
            ));
        }
        let report =
            fetch_stock_chg_1m(&self.db, &codes, request.as_of_date.as_deref()).await.map_err(
                |e| AxAgentError::Provider(format!("stock.analysis::stock_factor 查询失败: {e}")),
            )?;
        serde_json::to_value(&report).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::stock_factor 序列化失败: {e}"))
        })
    }
}
