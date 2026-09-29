//! 智能荐股 — 公共类型

use serde::{Deserialize, Serialize};

/// 风格
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// 趋势跟踪
    Trend,
    /// 价值低估
    Value,
    /// 资金驱动
    Capital,
    /// 超跌反弹
    Reversion,
    /// 候选池兜底
    Watchlist,
    /// Serenity 趋势智选 - 供给瓶颈分析
    #[serde(alias = "serenity")]
    Bottleneck,
    /// 趋势智选 - 政策驱动分析
    Policy,
    /// 趋势智选 - 业绩驱动分析
    Earnings,
    /// 趋势智选 - 资金驱动分析
    #[serde(alias = "capital_flow")]
    CapitalFlow,
    /// 趋势智选 - 事件驱动分析
    Event,
    /// 趋势智选 - 技术面驱动分析
    Technical,
}

impl Style {
    pub fn as_str(&self) -> &'static str {
        match self {
            Style::Trend => "trend",
            Style::Value => "value",
            Style::Capital => "capital",
            Style::Reversion => "reversion",
            Style::Watchlist => "watchlist",
            Style::Bottleneck => "bottleneck",
            Style::Policy => "policy",
            Style::Earnings => "earnings",
            Style::CapitalFlow => "capital_flow",
            Style::Event => "event",
            Style::Technical => "technical",
        }
    }
}

// 「持有周期四档」的唯一权威定义在 `axagent_harness::holding_period`（Period）。
// 本行只做 re-export：下游仍按 `crate::recommender::types::Period` /
// `axagent_analysis_engine::recommender::Period` 路径使用，序列化契约与 `as_str` /
// `factor` / `default_holding_days` / `nearest_for_holding_days` 语义逐字节不变。
pub use axagent_harness::holding_period::Period;

/// 单条推荐
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoPick {
    pub stock_code: String,
    pub stock_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
    pub style: Style,
    pub period: Period,
    /// 当前价
    pub price: f64,
    /// 入场下沿
    pub entry_low: f64,
    /// 入场上沿
    pub entry_high: f64,
    /// 止损
    pub stop_loss: f64,
    /// 目标位
    pub target_price: f64,
    /// 建议仓位（%）
    pub position_pct: f64,
    /// 持有天数
    pub holding_days: u32,
    /// 置信度 0-100
    pub confidence: u8,
    /// 命中理由
    pub reasons: Vec<String>,
    /// 风险提示
    pub risk_notes: Vec<String>,
    /// 次选风格（同票被多策略命中时记录）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary_styles: Vec<Style>,
    /// 是否为兜底合成 pick（true = 系统初筛 / 数据稀疏兜底，无技术信号支撑；
    /// false = 主策略真实命中）。前端用此字段显示"真实/兜底"标识。
    #[serde(default)]
    pub synthetic: bool,
}

/// 候选池来源构成 —— 每个来源**实际入池**的标的数（去重后）
///
/// 为什么必须有这个结构：`FALLBACK_STOCKS` 是无条件混入的 ⇒ 候选池在任何数据源状态
/// 下都非空，"有没有候选"完全推不出"候选是不是真实的"。as-of 回放里两个真实榜源
/// （热股榜 / 行业龙头）按设计返回空，池子 100% 是内置样本，而荐股结果看起来和
/// live 一模一样 —— 用户据此以为"那天真的没有热门股"。
///
/// `hot + industry == 0 && fallback > 0` 是「候选全部来自内置样本池」的机械判据；
/// 三者全 0 = 走了调用方自备的 preseed（本结构不适用），前端不得据此报警。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedPoolOrigin {
    /// 来自热股/涨停榜
    pub hot: usize,
    /// 来自行业排名的领涨龙头
    pub industry: usize,
    /// 来自内置 `FALLBACK_STOCKS` 样本池
    pub fallback: usize,
}

/// 本次荐股运行期间的 as-of 降级留痕（H3）
///
/// 与 `DegradationEntry`（权威定义在 harness，降级面板 `get_asof_degradation_log` 用的
/// 就是它）逐字同形，**只去掉 `as_of`** —— 响应里已有同值的 `asOfDate`，重复一个字段
/// 只会给前端两个可能不一致的真相。TS 侧因此可直接
/// `Omit<AsOfDegradationEntry, "as_of">` 复用既有类型，不另建一套。
///
/// 之所以不直接塞 `DegradationEntry`：它是 `Serialize`-only（无 `Deserialize`），
/// 而 `RecoResponse` 两个 derive 都要。`kind` 用 harness 的 `DegradationKind::label()`，
/// 与降级面板同一套词表（同一批 i18n 键）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoDegradation {
    pub vendor: String,
    /// 降级的取数方法名，如 `get_hot_stocks`
    pub method: String,
    /// 归因文本（面向用户的中文说明）
    pub reason: String,
    /// `failure` | `noData` | `structuralGap`
    pub kind: String,
}

impl From<axagent_astock_data::as_of::DegradationEntry> for RecoDegradation {
    fn from(e: axagent_astock_data::as_of::DegradationEntry) -> Self {
        Self {
            vendor: e.vendor,
            method: e.method,
            reason: e.reason,
            kind: e.kind.label().to_string(),
        }
    }
}

/// 荐股响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoResponse {
    pub period: Period,
    /// 按风格分组的 picks，每组 ≤ 10
    /// 使用 BTreeMap 确保序列化到 JSON 时 key 保持 Style 判别式顺序
    pub picks: std::collections::BTreeMap<Style, Vec<RecoPick>>,
    /// 被 vendor 缺失禁用的风格（live 模式下由 vendor 状态决定）
    pub disabled_styles: Vec<Style>,
    /// 被时间锚定 / as-of 截断降级的风格（spec §8）
    /// 与 `disabled_styles` 区别：disabled 是 vendor 完全不可用；
    /// degraded 是该风格对当前 as_of_date 没有历史语义（如 PE-TTM 仅有快照、
    /// 资金流无 N 日前对比等）。前端展示时用不同颜色(灰 / 橙)。
    /// 仅在 as-of 模式下非空；live 模式恒为 `vec![]`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_styles: Vec<Style>,
    /// `degraded_styles` 中各风格的降级原因（key=style, value=降级原因文本）
    /// 用于前端"⛔ 已降级：{reason}"提示。serde 序列化为 camelCase。
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub degraded_reasons: std::collections::HashMap<Style, String>,
    /// 生成时间戳（毫秒）
    pub generated_at: i64,
    /// **过滤前**的 seed pool 大小（hot + industry 龙头去重后）
    /// 实际参与扫描的池大小更小（流动性过滤会进一步剔除）
    pub raw_seed_pool_size: usize,
    /// 候选池的**来源构成**（真实榜 vs 内置样本池）。
    /// 前端据此声明「本次候选全部来自内置样本池」，消除「池子非空 ⇒ 候选是真实的」歧义。
    /// preseed 路径（调用方自备种子）下恒为全 0，表示"来源未知"。
    pub seed_pool_origin: SeedPoolOrigin,
    /// **本次运行**期间记录的 as-of 降级切片（按运行边界水位取，不是进程全局累计）。
    ///
    /// 为什么必须绑定到响应而不是只看降级面板：面板轮询的是进程级全局环形缓冲
    /// （`get_asof_degradation_log`），同截止日的上一次运行残留会被当成本轮降级 ——
    /// 与 `as_of.rs` 记过的 R5 基线陷阱同族。用户据此无法判断「热股榜无历史通道」
    /// 这条到底是这次荐股还是上周那次回放的。
    /// live 模式恒为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asof_degradations: Vec<RecoDegradation>,
    /// 时间旅行模式截止日 (YYYY-MM-DD)；live 模式为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of_date: Option<String>,
    /// 模式标签：live / replay / backtest_sweep
    pub mode: String,
    /// 数据获取错误详情。当 recommed_stocks 前置健康探测失败或全部 vendor 不可用时
    /// 填充此字段，前端据此显示具体错误文本而非泛化的"连接失败"。
    /// None = 正常执行（即使 picks 为空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
    /// 本次扫描**实际使用**的候选池快照（流动性过滤后，`[[code,name],...]` JSON）。
    ///
    /// 仅用于落库 reco_picks.seed_pool_json（回测负向样本 = 候选池 − 正向样本）。
    /// 由 `recommend_stocks` 内部填充（与扫描用的同一个 seed，避免 command 层二次
    /// `build_seed_pool` 造成快照与真实扫描池不一致 + 浪费 API 调用）。
    /// `#[serde(skip)]`：不参与前端 JSON 序列化，前端契约不变。
    #[serde(skip)]
    pub seed_pool_snapshot: Option<String>,
}
