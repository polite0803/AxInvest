use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 条件单定义 —— **本表是条件单定义的唯一权威**。
///
/// 为什么要新建这张表：此前条件单的「定义」有两个互相不通的来源 —— `settings` 表的
/// `conditional_orders` JSON 键（现网 0 行）与「从已触发的 `stock_analyses` 行的推理文本里
/// 用中文关键词嗅探重建」（阈值硬编码成 3%/10%、上限硬编码 3 次/冷却 30 分）。后者会让
/// 一次触发写下的意图行在下一趟热加载时**变成新的定义**，用户在创建时给的阈值全部丢失，
/// 语义倒置成「结果产生规则」。两者都已退役。
///
/// 建表走本仓正门：实体列属性 + 启动期声明式对账（`crates/dao/src/reconcile/`），
/// **不写 migration**；平表不需要进 `reconcile/extras.rs`（`price_alerts` 同样不在那里）。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "conditional_orders")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub stock_code: String,
    pub stock_name: String,
    /// 条件类别，取值与 `analysis_engine::conditional_order::ConditionType` 的字符串形态逐字一致：
    /// `stop_loss` / `take_profit` / `resistance_break` / `support_break` / `change_pct` / `turnover_rate`
    #[sea_orm(indexed)]
    pub condition_type: String,
    /// 阈值，**语义随 `condition_type` 而变**（与 `price_alerts.threshold` 同一个口径设计）：
    /// - `stop_loss` / `take_profit` / `resistance_break` / `support_break` → 绝对价格（元）
    /// - `change_pct` / `turnover_rate` → 百分比数值（5.0 表示 5%）
    ///
    /// 四类价格条件都必须给值：旧实现不收用户阈值，而是拿前收盘 × 固定 7%/15%/5% 冒充，
    /// 于是「跌破我设的止损价」实际算的是「较前收盘跌 7%」。
    pub condition_value: Option<f64>,
    /// 动作类别：`buy` / `sell` / `reduce` / `notify`
    pub action_type: String,
    /// 动作参数，语义随 `action_type` 而变：`buy` → 股数；`reduce` → 比例（0.0–1.0）；
    /// `sell` / `notify` → `None`（`sell` 的 `None` 是「卖出全部持仓」，不是缺值）
    pub action_value: Option<f64>,
    pub enabled: bool,
    /// 生效时间窗（北京时间 `"HH:MM"`）。**两侧同时有值才生效**；任一侧缺失即全天。
    /// 旧实现要求两侧同时有值却所有构造点都传 `None/None`，且比较用 `Local`（机器时区）
    /// 的字符串序 —— 现在是北京分钟数的数值比较。
    pub active_start: Option<String>,
    pub active_end: Option<String>,
    pub cool_down_minutes: i32,
    /// 单日触发上限，`0` = 不限（与引擎里的判据同一条分母：按北京日计）
    pub max_triggers_per_day: i32,
    /// 本北京日已触发次数。计数与 `trigger_date` **成对读**：日期不是今天就从 0 起算，
    /// 旧形态把计数标在 `#[serde(skip)]` 上、每次加载归零且永不跨日滚动。
    pub triggers_today: i32,
    /// `triggers_today` 归属的北京日（`YYYY-MM-DD`）；`None` = 从未触发过
    pub trigger_date: Option<String>,
    /// 最后一次触发时刻（UTC ms）
    pub last_triggered_at: Option<i64>,
    /// 最后一次触发的判定说明（人话，供列表与复盘直读；不再把 Rust `Debug` 形态端给前端）
    pub last_trigger_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
