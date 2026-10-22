use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 一次荐股扫描的**候选池快照**（每批次一行，0 票批次也写）。
///
/// 为什么不复用 `reco_picks.seed_pool_json`：那条列挂在**出票行**上，0 票批次
/// 一行都不落 ⇒ 漏检归因把「这批扫过、池子是空的」读成「没有任何批次」，
/// 落进 `unexplained`；而 `unexplained_share` 又会让整轮被排除出补分析样本，
/// 形成「留痕不足 ⇒ 不补分析 ⇒ 永远没留痕」的自锁。
///
/// `degraded` 由写侧判定（池快照缺失/为空）；读侧用「本窗所有批次都 degraded」
/// 才判 `pool_degraded`，单个空批次不再把整个事件误判成降级。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "reco_pool_batches")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// `reco` | `serenity`（与 `RecoChain` 值域一致）
    pub chain: String,
    /// 本次扫描的档位
    pub period: String,
    /// 批次时间（与同批 `reco_picks.generated_at` 相同）
    pub generated_at: String,
    /// `None` = 实盘批；`Some(YYYY-MM-DD)` = 该次回放的截止日
    pub as_of_date: Option<String>,
    /// 实际用于扫描的候选池快照（`[[code,name]]` 或智选单对象）；NULL = 取数侧没给
    pub seed_pool_json: Option<String>,
    /// 池内候选数（解析失败按 0，不猜）
    pub stock_count: i32,
    /// 池快照缺失或为空
    pub degraded: bool,
    pub created_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
