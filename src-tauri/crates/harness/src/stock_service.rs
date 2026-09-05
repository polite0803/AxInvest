// SPDX-License-Identifier: AGPL-3.0-only

//! 股票分析/自选股存储契约层 — 消除 gateway → axagent-entities 违规依赖。
//!
//! gateway 的 `stock_handlers` 只消费本 trait；返回 `serde_json::Value`
//! 而非镜像 DTO（stock_analyses 有 ~30 字段，镜像维护成本高且易漂移）。
//!
//! - 实现方：主 crate `gateway_stock_store::DaoStockStore`（entities 后端，
//!   与 `gateway_memory_store::DaoMemoryStore` 同款接缝注入模式）
//! - 消费者：gateway（`/api/stock/*` 路由）

use async_trait::async_trait;
use serde_json::Value;

use crate::core_error::Result;

/// 股票分析与自选股存储接口
#[async_trait]
pub trait StockStore: Send + Sync {
    /// 按创建时间倒序分页列出分析记录
    async fn list_analyses(&self, limit: u64, offset: u64) -> Result<Vec<Value>>;

    /// 读取单条分析记录；不存在返回 `None`
    async fn get_analysis(&self, analysis_id: &str) -> Result<Option<Value>>;

    /// 自选股列表（按创建时间倒序）
    async fn list_watchlist(&self) -> Result<Vec<Value>>;

    /// 添加自选股，返回落库后的完整记录
    async fn add_watchlist(&self, stock_code: &str, stock_name: &str) -> Result<Value>;

    /// 删除自选股；返回是否实际删除（false = 记录不存在）
    async fn delete_watchlist(&self, id: &str) -> Result<bool>;
}
