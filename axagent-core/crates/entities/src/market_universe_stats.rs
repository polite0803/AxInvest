// SPDX-License-Identifier: AGPL-3.0-only

//! 全市场清单的**覆盖率读数**（0-D，2026-10-18，PLAN-mover-coverage-and-reflection-exit）。
//!
//! ## 为什么要单独一张表
//! `axagent_analysis_engine::market_close_store::refresh_universe_pages` 每次 tick 都从
//! clist 接口同时拿到两个量：本地已落库的 `universe_size` 与**接口自报的全市场票数
//! `reported_total`**。后者原先只进 `tracing::info!`（`init/services.rs` 的清单扩页日志），
//! 于是「清单到底缺了多少」这个问题在库里**无法回答** —— 只能人肉去读一次日志，
//! 而日志既不累积也不可断言。
//!
//! 这条缺口是实测出来的：现网 `market_stock_universe` 679 行，其中深市主板 618 / 沪主板 37 /
//! 创业板 16 / 科创板 8；接口分页上限是 60 页 × `pz=100` = 6000 槽位，本该覆盖全市场，
//! 实际只有约 6 页成功落库（push2 族在本机的累积/速率触发封禁，失败页在
//! `refresh_universe_pages` 里只 `warn` 后跳过）。**荐股候选宇宙 = 这张清单**，
//! 所以覆盖率就是「智能荐股能看见多少只票」。
//!
//! ## 为什么不并进 `data_quality_observations`
//! 那张表的 `scope`/`grade`/`abnormal` 是 DQI 熔断的分母（单标的证据质量），
//! 把「全市场枚举完整性」这种市场级量塞进去，等于把两套计数域并成一个分母 ——
//! 正是本仓反复修的那类混池。宁可新开一张窄表。
//!
//! ## 读侧
//! 本表由 `scripts/check-universe-coverage.mjs` 断言（比值与分市场配比）。
//! 那条门**不注册进 CI**：CI 里没有本机 PG，比值只能从实库取（理由写在该脚本头部）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "market_universe_stats")]
pub struct Model {
    /// 主键 = 观测时刻（毫秒）。同一次 tick 只落一行，重复观测按毫秒天然分开。
    #[sea_orm(primary_key, auto_increment = false)]
    pub observed_at: i64,
    /// 本地清单已落库票数（`market_stock_universe` 行数）
    pub universe_size: i32,
    /// **接口自报**的全市场票数 —— 分母的唯一来源，不写死外部常数（那是第五套参数）。
    /// 本表只在它 > 0 时落行，所以这里不是 Option：落行即有分母。
    pub reported_total: i64,
    /// `universe_size / reported_total × 100`，两位小数。存下来是为了让历史可比
    /// （读侧现算也行，但会把「当初怎么算的」留给下一个改公式的人）。
    pub coverage_pct: f64,
    /// 本次 tick 请求的页数（`init/services.rs` 里是 6 页一批）
    pub pages_requested: i32,
    /// 本次 tick 取数失败的页数 —— 覆盖率上不去的直接原因就是这个数
    pub pages_failed: i32,
    /// 本次 tick 新落库的票数
    pub rows_upserted: i32,
    #[sea_orm(default_value = 0)]
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
