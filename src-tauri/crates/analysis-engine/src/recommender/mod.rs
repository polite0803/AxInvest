//! 智能荐股 — 顶层模块
//!
//! ## 架构
//! - 18 个子策略（5 风格 × 4 周期，超跌反弹仅短/中线）实现 [`strategy::RecommendStrategy`]
//! - [`recommend_stocks`] 是组合入口：seed pool → 候选过滤 → vendor 降级 → 并行扫描 → 去重 → 按风格分组
//! - 5 min 内存缓存（按 period 维度）
//!
//! ## 公开 API
//! - [`recommend_stocks`] — Tauri command 调用

pub mod gates;
pub mod ic;

/// 荐股算法的**自身版本**（与 `stock_analysis_setup` 的工作流 `TEMPLATE_VERSION` 无关 ——
/// 荐股链不跑工作流模板，它的「算法」= 本 crate 的策略集 + 判据 + 尺度映射）。
///
/// 升版判据（需求⑩原则 5）：**因子增删、判据口径改动、档位×风格矩阵变动** ⇒ +1。
/// 只调参数值（权重/阈值）**不**升版 —— 那些走 `reco_strategy_weights` 的历史表可追溯。
///
/// 为什么由代码常量而不是模板变量承载：荐股链没有「重种」语义，变量可被面板随手改，
/// 而版本必须是样本的**不可变归属**，否则同代筛样本身就是可伪造的。
/// v2（2026-10-18）：`trend × long` 按设计不做（`trend_momentum_not_a_long_thesis`）——
/// 用户裁定「智能荐股的长线和趋势智选是两个不同的概念，不能混为一谈」；trend 的短期
/// 动量证据没有长线兑现路径（002696 案例错配源头，见 `style_matrix::MATRIX` 处长注释）。
/// v4（2026-10-18）：**置信度语义重构——撤伪概率链**。用户裁定「同档所有票同一个
/// 置信度（60/56/53/51）基本上就是造假」——根因是策略「条件满足度」评分被**当概率**
/// 做 logit 合成 + SNR 折算 + quality 校准（对非概率量的伪概率运算，且策略输入几乎
/// 全是模板常数 ⇒ 同档原始置信趋同、输出塌成档位常数）。v4 起 `confidence` =
/// **证据强度**（策略条件满足度 × 风格权重），个股维度、不再自称概率；伪概率链与
/// 其数据依赖（market_regime/style_baseline/conf_sensitivity）整体撤除。
/// 真概率的正路 = 证据分桶 × 闭环 T+N 胜率校准（PLAN-capital-long-evidence §六）。
pub const RECO_ALGORITHM_VERSION: i32 = 4;

/// 荐股链样本的**起算代际** —— 读侧筛样分母的下限（#49 / #12 共用这一条判据）。
///
/// 与 [`RECO_ALGORITHM_VERSION`] 的分工：后者是「这条样本属于哪一代」（写侧盖章），
/// 本常量是「从哪一代起样本彼此可比」（读侧分母）。两者**刻意不是一个值**：
/// 升 `RECO_ALGORITHM_VERSION` 不自动抬这里 —— 抬了等于每次换代把整个窗口清零，
/// 与 `harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR` 同一条规则：
/// **下限，不是等号**（PLAN §五十一-② / §七十一）。
///
/// 现取 1：荐股链自本列引入起就是当前算法形态，第 1 代即第一个可归属的代。
///
/// `reco_version IS NULL` 的存量行 = 归属未知 ⇒ **不进分母**。这与错题本注入语料的
/// 「NULL 保留」处置相反（那条见 `dao::repo::stock_lesson_queries::generation_floor_or_null`，
/// 教训本身跨代仍有价值；命中率/IC 分母则跨代不可比）。
pub const RECO_GENERATION_FLOOR: i32 = 1;

pub mod indicators;
pub mod notify;
pub mod pool;
pub mod reco_loop;
pub mod risk;
pub mod scale;
pub mod scoring;
pub mod strategies;
pub mod strategy;
pub mod style_matrix;
pub mod types;

pub use notify::{build_notification, run_recommendation_scan, RecommendationScan};
pub use strategy::{RecoContext, RecommendStrategy};
pub use types::{
    Period, RecoDegradation, RecoPick, RecoResponse, SeedPoolOrigin, Style, TrimmedPick,
};

/// 回退候选股列表（沪深300核心成分股，覆盖主要行业）
///
/// 当 `get_hot_stocks` / `get_industry_ranking` 都拿不到数据时用作种子池兜底，
/// 保证 `recommend_stocks` 至少能基于一组已知活跃股跑子策略。
/// screener.rs 与 pool.rs 共用此常量，避免重复定义导致不同步。
pub(crate) const FALLBACK_STOCKS: &[(&str, &str)] = &[
    ("600519", "贵州茅台"),
    ("000858", "五粮液"),
    ("300750", "宁德时代"),
    ("600036", "招商银行"),
    ("601318", "中国平安"),
    ("000333", "美的集团"),
    ("002475", "立讯精密"),
    ("600276", "恒瑞医药"),
    ("300059", "东方财富"),
    ("000651", "格力电器"),
    ("002415", "海康威视"),
    ("600900", "长江电力"),
    ("601888", "中国中免"),
    ("300014", "亿纬锂能"),
    ("002594", "比亚迪"),
    ("601012", "隆基绿能"),
    ("000001", "平安银行"),
    ("600030", "中信证券"),
    ("000002", "万科A"),
    ("601166", "兴业银行"),
    ("601899", "紫金矿业"),
    ("300124", "汇川技术"),
    ("600809", "山西汾酒"),
    ("002714", "牧原股份"),
    ("000568", "泸州老窖"),
    ("603259", "药明康德"),
    ("600887", "伊利股份"),
    ("002230", "科大讯飞"),
    ("300274", "阳光电源"),
    ("601088", "中国神华"),
    ("600585", "海螺水泥"),
    ("000725", "京东方A"),
    ("002304", "洋河股份"),
    ("300760", "迈瑞医疗"),
    ("600031", "三一重工"),
    ("601211", "国泰君安"),
    ("002241", "歌尔股份"),
    ("300408", "三环集团"),
    ("603986", "兆易创新"),
    ("600745", "闻泰科技"),
    ("002044", "美年健康"),
    ("300122", "智飞生物"),
    ("000063", "中兴通讯"),
    ("002049", "紫光国微"),
    ("603501", "韦尔股份"),
    ("601398", "工商银行"),
    ("600028", "中国石化"),
    ("601857", "中国石油"),
];

use axagent_astock_data::as_of;
use axagent_astock_data::AStockClient;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
// `HashSet` 在 Rust 2024 edition 已加入 prelude

use crate::recommender::pool::SeedItem;
use crate::recommender::pool::{
    build_seed_pool, clear_cached_vendors, get_cached_vendors,
    liquidity_filter_and_truncate_with_concurrency, load_enabled_vendors_from_template,
    set_cached_vendors,
};
use crate::recommender::scoring::{dedup_and_merge, group_by_style_and_trim_with_audit};
use crate::recommender::strategies::{
    emit_synthetic_picks, CapitalStrategy, ReversionStrategy, SerenityStrategy, TrendStrategy,
    ValueStrategy, WatchlistStrategy,
};
use crate::recommender::strategy::PerCodeLocks;

/// Serenity 工作流产出的候选股，供 SerenityStrategy 读取
/// 自带 TTL（1h），避免 workflow 长时间不运行时使用过期候选。
static SERENITY_SEED: LazyLock<RwLock<Vec<SeedItem>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Serenity 缓存写入时间戳，用于 TTL 检查
static SERENITY_CACHE_TIME: LazyLock<RwLock<Option<Instant>>> = LazyLock::new(|| RwLock::new(None));

/// Serenity 缓存 TTL：1 小时
const SERENITY_CACHE_TTL: Duration = Duration::from_secs(3600);

/// 检查 Serenity 缓存是否过期
fn is_serenity_cache_fresh() -> bool {
    let guard = SERENITY_CACHE_TIME.read();
    if let Some(ts) = *guard {
        return ts.elapsed() < SERENITY_CACHE_TTL;
    }
    false
}

/// 设置 serenity 候选种子（由 run_serenity_screening 命令写入）
pub fn set_serenity_seed(seed: Vec<SeedItem>) {
    tracing::info!("[serenity] 注入 {} 条候选到全局种子", seed.len());
    let mut guard = SERENITY_SEED.write();
    *guard = seed;
    let mut guard = SERENITY_CACHE_TIME.write();
    *guard = Some(Instant::now());
}

/// 读取 serenity 候选种子（缓存过期返回空）
pub fn get_serenity_seed() -> Vec<SeedItem> {
    if !is_serenity_cache_fresh() {
        return Vec::new();
    }
    SERENITY_SEED.read().clone()
}

/// Serenity 候选全量数据缓存（serenity_score / catalysts / exit_signals / attention_metrics），
/// 从 workflow 输出传递到 SerenityStrategy::scan_one，使策略能感知上下文。
static SERENITY_CANDIDATE_CACHE: LazyLock<RwLock<HashMap<String, serde_json::Value>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// 设置全量候选数据缓存（由 run_serenity_screening 写入）
pub fn set_serenity_candidate_cache(cache: HashMap<String, serde_json::Value>) {
    tracing::info!("[serenity] 注入 {} 条候选全量数据到缓存", cache.len());
    let mut guard = SERENITY_CANDIDATE_CACHE.write();
    *guard = cache;
    let mut guard = SERENITY_CACHE_TIME.write();
    *guard = Some(Instant::now());
}

/// 读取单个候选的全量数据（缓存过期返回 None）
pub fn get_serenity_candidate_detail(code: &str) -> Option<serde_json::Value> {
    if !is_serenity_cache_fresh() {
        return None;
    }
    SERENITY_CANDIDATE_CACHE.read().get(code).cloned()
}

/// 清空候选全量数据缓存（同时清时间戳）
pub fn clear_serenity_candidate_cache() {
    let mut guard = SERENITY_CANDIDATE_CACHE.write();
    guard.clear();
    let mut guard = SERENITY_CACHE_TIME.write();
    *guard = None;
    let mut guard = SERENITY_SEED.write();
    guard.clear();
}

/// 从 template_vars 中解析 "reco_strategy_weights" 权重表。
///
/// 这是复盘 → 进化（R1）的注入点：recommend_stocks 会按 (style, period)
/// 对每条 pick 的 confidence 乘上对应的权重，让近期胜率低的策略自然被降权。
///
/// 格式：`reco_strategy_weights` 是一个 JSON 对象：
/// ```json
/// { "trend_short": 1.2, "trend_ultra_short": 0.8, "value_mid": 0.8, ... }
/// ```
/// 缺失的 (style, period) 默认 1.0（不调整）。
pub(crate) fn parse_strategy_weights(
    template_vars: &[(String, serde_json::Value)],
) -> HashMap<(Style, Period), f64> {
    let mut out: HashMap<(Style, Period), f64> = HashMap::new();
    for (name, value) in template_vars {
        if name != "reco_strategy_weights" {
            continue;
        }
        let Some(obj) = value.as_object() else { continue };
        for (key, val) in obj {
            let Some(weight) = val.as_f64() else { continue };
            let Some((style, period)) = parse_style_period_key(key) else { continue };
            out.insert((style, period), weight.clamp(0.0, 2.0));
        }
    }
    out
}

/// 解析 `"{style}_{period}"` 权重键（`load_reco_served_vars_db` 的写入形态）。
///
/// 不能按首个 `_` 切分：**风格名与档位名各自都可能含下划线**（`capital_flow`、
/// `ultra_short`），`splitn(2, '_')` 会把 `capital_flow_mid` 切成
/// (`capital`, `flow_mid`) ⇒ 该键被静默丢弃 —— 闭环写回的权重对
/// `capital_flow` 一族**永远不生效且无任何日志**。这里改为「逐风格试前缀 +
/// 余段必须整体解析成档位」：试 `capital` 时余段 `flow_mid` 不合法而跳过，
/// 试 `capital_flow` 时余段 `mid` 命中 ⇒ 与键的生成侧
/// （`format!("{s}_{p}")`）严格互为往返。
/// `serenity` / `bottleneck` 两种写法都接受（见 `Style::key_names`）。
pub(crate) fn parse_style_period_key(key: &str) -> Option<(Style, Period)> {
    for style in Style::ALL {
        for name in style.key_names() {
            let Some(rest) = key.strip_prefix(name).and_then(|r| r.strip_prefix('_')) else {
                continue;
            };
            if let Ok(period) = rest.parse::<Period>() {
                return Some((style, period));
            }
        }
    }
    None
}

// ── 缓存 ──
// 缓存 key 由 (period, as_of) 二元组组成，确保 live / replay 互相隔离。
// 之前为单条目 RwLock<Option>，同时请求不同 period 会互相驱逐。
// 改为 HashMap<(Period, String), (RecoResponse, Instant)> 后多 period 独立缓存。
// replay 模式后缀为 `asof-YYYYMMDD`，由 `as_of::cache_suffix()` 提供。

#[allow(clippy::type_complexity)]
static RESULT_CACHE: LazyLock<RwLock<HashMap<(Period, String), (RecoResponse, Instant)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
/// 缓存 TTL：从 5 min 缩短到 60 s。
///
/// 荐股的 entry / target / stop_loss 是基于扫描时刻的现价算的，
/// 5 min 内股价可能已经走完整个目标区间（如图 莱伯泰科 5 min 内 38 → 48），
/// 这种"还显示着旧 target / entry 但现价远超"的推荐对用户毫无价值，
/// 还会误导用户按旧价位挂单。60 s 是为价格时效性与扫描性能做的折中。
const CACHE_TTL: Duration = Duration::from_secs(60);

// ── 推荐器配置（从 workflow template variables 读取） ──

/// 用户可配置的推荐器参数
struct RecoConfig {
    trend_enabled: bool,
    reversion_enabled: bool,
    value_enabled: bool,
    capital_enabled: bool,
    watchlist_enabled: bool,
    serenity_enabled: bool,
    min_confidence: u8,
}

impl Default for RecoConfig {
    fn default() -> Self {
        Self {
            trend_enabled: true,
            reversion_enabled: true,
            value_enabled: true,
            capital_enabled: true,
            watchlist_enabled: true,
            serenity_enabled: true,
            min_confidence: 0, // 0 = 不筛选
        }
    }
}

/// 从 template_vars 解析推荐器配置
///
/// `template_vars` 来自 workflow_template 表的 variables 字段，
/// 由前端 StockAnalysisConfigPanel 保存。变量名映射参见前端 getDefaultVariables()。
fn parse_reco_config(template_vars: &[(String, serde_json::Value)]) -> RecoConfig {
    let mut cfg = RecoConfig::default();
    for (name, value) in template_vars {
        let bool_val = || -> Option<bool> {
            value.as_bool().or_else(|| {
                value.as_str().and_then(|s| match s {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                })
            })
        };
        match name.as_str() {
            "reco_trend_enabled" => {
                if let Some(v) = bool_val() {
                    cfg.trend_enabled = v;
                }
            },
            "reco_reversion_enabled" => {
                if let Some(v) = bool_val() {
                    cfg.reversion_enabled = v;
                }
            },
            "reco_value_enabled" => {
                if let Some(v) = bool_val() {
                    cfg.value_enabled = v;
                }
            },
            "reco_capital_enabled" => {
                if let Some(v) = bool_val() {
                    cfg.capital_enabled = v;
                }
            },
            "reco_watchlist_enabled" => {
                if let Some(v) = bool_val() {
                    cfg.watchlist_enabled = v;
                }
            },
            "reco_serenity_enabled" => {
                if let Some(v) = bool_val() {
                    cfg.serenity_enabled = v;
                }
            },
            "reco_min_confidence" => {
                if let Some(n) = value.as_f64() {
                    cfg.min_confidence = n.clamp(0.0, 100.0) as u8;
                }
            },
            _ => {},
        }
    }
    cfg
}

fn cache_get(period: Period) -> Option<RecoResponse> {
    let suffix = as_of::cache_suffix();
    let g = RESULT_CACHE.read();
    g.get(&(period, suffix.clone()))
        .filter(|(_, ts)| ts.elapsed() < CACHE_TTL)
        .map(|(resp, _)| resp.clone())
}

fn cache_put(period: Period, resp: RecoResponse) {
    let suffix = as_of::cache_suffix();
    // 空结果也缓存（同 60s TTL），避免 vendor 故障时每次请求都全量扫描 vendor
    // 旧逻辑跳过了空结果的缓存，导致 vendor 故障期间形成
    // "请求→扫描→空→不缓存→请求"的死循环。
    let mut g = RESULT_CACHE.write();
    g.insert((period, suffix), (resp, Instant::now()));
}

/// 主动失效缓存（设置页保存 vendor 后调用）
///
/// 同时清 vendor 启用集合缓存 + 推荐结果缓存 + Serenity 候选缓存，
/// 保证下次调用立即反映新 vendor 状态和干净的候选池
pub fn invalidate_cache() {
    let mut g = RESULT_CACHE.write();
    g.clear();
    clear_cached_vendors();
    clear_serenity_candidate_cache();
}

// ── 公开入口 ──

/// 拉取指定周期的荐股结果
///
/// 调用方应保证传入的 `template_vars` 是当前 workflow template 的变量列表（vendor_* 启用状态）
///
/// `client` 用 `Arc` 包一层，因为后续并行任务（liquidity filter + 4 子策略）需要共享所有权
///
/// `preseed` 为可选的预置种子池：
/// - `Some(seed)`：直接使用该种子，跳过 `build_seed_pool`（热门股/行业龙头/兜底股），
///   但仍会执行流动性过滤和 Serenity 注入。用于自动化管道从历史推荐中构建候选池。
/// - `None`：走默认的 `build_seed_pool` 路径（热门股 + 行业龙头 + FALLBACK_STOCKS）。
pub async fn recommend_stocks(
    client: Arc<AStockClient>,
    period: Period,
    template_vars: &[(String, serde_json::Value)],
    preseed: Option<Vec<SeedItem>>,
    horizon_prior: Option<serde_json::Value>,
) -> Result<RecoResponse, String> {
    // H3(2026-09-27)：把降级留痕绑定到**本次运行**。
    //
    // 两层作用：
    //   ① `with_degradation_log` 提供 task-local 容器 —— 本函数全程在同一个 task 里
    //      （策略与流动性过滤用 FuturesUnordered，不 spawn），父任务的记录只有进了
    //      容器才不靠全局缓冲兜底；缺容器时 `take_global_degradations_since` 会打
    //      一条「访问失败」warn（工作流入口 `stock_workflow/core.rs` 就是这么包的）。
    //   ② 进出各取一次水位 ⇒ 切片只含本次运行新增/复现的降级。直接读全局面板会把
    //      同截止日上一次运行的残留算进来（R5 基线陷阱同族）。
    as_of::with_degradation_log(async {
        let deg_watermark = as_of::global_degradation_seq_watermark();
        let mut resp =
            recommend_stocks_inner(client, period, template_vars, preseed, horizon_prior).await?;
        if as_of::current_as_of().is_some() {
            resp.asof_degradations = as_of::take_global_degradations_since(deg_watermark)
                .into_iter()
                .map(Into::into)
                .collect();
        }
        Ok(resp)
    })
    .await
}

/// 行情 + K 线两源全挂时的空响应（带归因，不静默出空表）。
fn empty_error_response(period: Period, detail: String) -> RecoResponse {
    RecoResponse {
        period,
        picks: std::collections::BTreeMap::new(),
        disabled_styles: vec![],
        degraded_styles: vec![],
        degraded_reasons: std::collections::HashMap::new(),
        generated_at: chrono::Utc::now().timestamp_millis(),
        raw_seed_pool_size: 0,
        seed_pool_origin: SeedPoolOrigin::default(),
        asof_degradations: vec![],
        as_of_date: None,
        mode: "live".to_string(),
        error_detail: Some(detail),
        seed_pool_snapshot: None,
        scan_audit: vec![],
        gate_blocked: vec![],
    }
}

/// 一次拿多档（面板默认四档全取）：候选池**只建一次**，逐档并发扫描。
///
/// 与逐次调用 `recommend_stocks` 的区别就是这次共享：建池 + 流动性过滤（每股一次
/// 60 日 K 线）此前按档各做一遍，四档即 4× 全量取数。
///
/// 档位级缓存仍在**每档**粒度生效（TTL 60 s）：命中的档不重扫 ⇒ 刷新某一档不会
/// 让另外三档过期。as-of 模式下的降级留痕按「本次运行」整体切片，只挂在**本轮真正扫描过**
/// 的响应上；命中缓存的档保留其原始那轮自己的切片（否则把上一次运行的降级算成本轮）。
///
/// 返回顺序与 `periods` 一致；单档失败只影响该档（逐档 `Result`），不让整批变成一次错误。
pub async fn recommend_stocks_multi(
    client: Arc<AStockClient>,
    periods: &[Period],
    template_vars: &[(String, serde_json::Value)],
    preseed: Option<Vec<SeedItem>>,
    horizon_prior: Option<serde_json::Value>,
) -> Vec<(Period, Result<RecoResponse, String>)> {
    if periods.is_empty() {
        return Vec::new();
    }
    let use_cache = preseed.is_none();
    as_of::with_degradation_log(async {
        let deg_watermark = as_of::global_degradation_seq_watermark();
        let mut hits: std::collections::HashMap<Period, Result<RecoResponse, String>> =
            std::collections::HashMap::new();
        let mut misses: Vec<Period> = Vec::new();
        for &p in periods.iter() {
            match if use_cache { cache_get(p) } else { None } {
                Some(cached) => {
                    hits.insert(p, Ok(cached));
                },
                None => misses.push(p),
            }
        }
        if !misses.is_empty() {
            let prepared = prepare_scan(client, template_vars, preseed, horizon_prior).await;
            let as_of_active = as_of::current_as_of().is_some();
            let slice: Vec<RecoDegradation> = if as_of_active {
                as_of::take_global_degradations_since(deg_watermark)
                    .into_iter()
                    .map(Into::into)
                    .collect()
            } else {
                Vec::new()
            };
            let scanned =
                futures::future::join_all(misses.iter().map(|p| scan_period(&prepared, *p))).await;
            for (p, resp) in misses.iter().zip(scanned) {
                hits.insert(
                    *p,
                    resp.map(|mut r| {
                        if as_of_active {
                            r.asof_degradations = slice.clone();
                        }
                        r
                    }),
                );
            }
        }
        periods
            .iter()
            .map(|p| (*p, hits.remove(p).unwrap_or_else(|| Err("内部：该档未被扫描".to_string()))))
            .collect()
    })
    .await
}

/// 一次扫描中**与持有档位无关**的准备结果：健康探测、候选池、流动性过滤、Serenity 注入、
/// 推荐器配置、自适应权重、per-code 互斥锁。
///
/// 为什么要单独成结构：此前「建池 + 流动性过滤」与「按档跑策略」焊在同一个函数里，
/// 于是「一次拿四档」只能调四次 ⇒ 同一个候选池被建 4 次、流动性过滤（每股一次 60 日
/// K 线）做 4 次。定时任务 `run_recommendation_scan` 并行 spawn 四个 period 时正是
/// 这个形态（4× 全量取数，纯属重复）。拆出来之后四档共享一份池与一把 per-code 锁。
struct PreparedScan {
    client: Arc<AStockClient>,
    /// 流动性过滤 + Serenity 注入后的实际扫描池（非 Watchlist 策略消费它）
    seed: Vec<SeedItem>,
    /// 过滤前的原始池（Watchlist 与合成兜底消费它）
    raw_seed: Vec<SeedItem>,
    seed_pool_origin: SeedPoolOrigin,
    raw_seed_pool_size: usize,
    vars_map: HashMap<String, serde_json::Value>,
    reco_cfg: RecoConfig,
    strategy_weights: HashMap<(Style, Period), f64>,
    /// 跨档共享：同一只票在四档之间也串行，避免并发打爆 vendor
    per_code_locks: Arc<PerCodeLocks>,
    /// `preseed` 路径不读写结果缓存
    use_cache: bool,
    /// 行情 + K 线两源全挂时的归因文本；非 `None` 时逐档直接出降级响应（不静默出空表）
    all_sources_down: Option<String>,
    /// 逐档先验表（`horizon_prior::horizon_prior_map` 的输出，与分析链**同一份实现**）：
    /// `{档位: {prior, samples, source}}`。
    /// v4（2026-10-18）起**不再进入个股置信度合成**（伪概率链撤除，见 scan_period），
    /// 保留为档位级观测（debug 留痕）——它的正确展示位是档位级「历史胜率」，
    /// 个股级校准等证据分桶样本积累（`PLAN-capital-long-evidence.md` §六）。
    horizon_prior: Option<serde_json::Value>,
}

/// ⚠ v4（2026-10-18）：`compute_style_baselines` 与 `detect_market_regime` 已随伪概率链
/// 撤除（它们只服务 quality 校准——校正一个假起点只会让假数字显得更可信）。
/// 个股级校准（证据分桶 × 真实 T+N 胜率）是恢复这条链的**唯一正当理由**，
/// 设计要点留在 `PLAN-capital-long-evidence.md` §六与本项目记忆 2026-10-18。
async fn recommend_stocks_inner(
    client: Arc<AStockClient>,
    period: Period,
    template_vars: &[(String, serde_json::Value)],
    preseed: Option<Vec<SeedItem>>,
    horizon_prior: Option<serde_json::Value>,
) -> Result<RecoResponse, String> {
    // preseed 模式跳过缓存（种子来自 DB 历史推荐，内容可能每次不同，不应命中缓存）
    let use_cache = preseed.is_none();
    if use_cache {
        if let Some(cached) = cache_get(period) {
            return Ok(cached);
        }
    }
    let prepared = prepare_scan(client, template_vars, preseed, horizon_prior).await;
    scan_period(&prepared, period).await
}

/// 一次「多档」荐股的共享准备（候选池只建一次）。
async fn prepare_scan(
    client: Arc<AStockClient>,
    template_vars: &[(String, serde_json::Value)],
    preseed: Option<Vec<SeedItem>>,
    horizon_prior: Option<serde_json::Value>,
) -> PreparedScan {
    let use_cache = preseed.is_none();

    // ── 前置健康探测 ──
    // 同时探测 quote 和 K 线，两者都失败时短路，避免 200 次无效 API 调用
    let quote_probe = client.get_quote("000001").await;
    let kline_probe = client.get_klines("000001", "daily", 5).await;

    let quote_ok = quote_probe.is_ok();
    let kline_ok = kline_probe.as_ref().is_ok_and(|k| k.len() >= 2);

    let all_sources_down = if !quote_ok && !kline_ok {
        let quote_err = quote_probe.err().map(|e| e.to_string());
        let kline_err = kline_probe.err().map(|e| e.to_string());
        let detail = format!(
            "行情与K线数据源均不可用 (quote: {}, klines: {})，跳过荐股扫描",
            quote_err.as_deref().unwrap_or("返回异常"),
            kline_err.as_deref().unwrap_or("返回不足(<2条)"),
        );
        tracing::warn!("[recommender] {detail}");
        Some(detail)
    } else {
        None
    };

    if !kline_ok {
        tracing::warn!("[recommender] K 线数据源当前不可用, 趋势/资金/超跌策略将跳过");
    }
    if !quote_ok {
        tracing::warn!("[recommender] 行情数据源当前不可用, Watchlist/Synthetic 兜底将跳过");
    }

    // 1. 预热 enabled-vendors 缓存（settings 页保存 vendor 时需要 invalid 这个缓存
    //    来刷新结果缓存；此处不依赖 enabled_vendors 做策略 gating）
    let _ = get_cached_vendors().unwrap_or_else(|| {
        let s = load_enabled_vendors_from_template(template_vars);
        set_cached_vendors(s.clone());
        s
    });

    // 2. seed pool + 流动性过滤
    //    preseed 为 Some 时使用调用方提供的种子（如自动化管道从历史推荐构建），
    //    为 None 时走默认 build_seed_pool（热门股+行业龙头+兜底）。
    //    来源构成随池一起返回：preseed 的来历由调用方决定，本层无法归因 ⇒ 全 0（= 未知）。
    let (mut seed, seed_pool_origin) = match preseed {
        Some(custom) => {
            tracing::info!("[recommender] using custom preseed size={}", custom.len());
            // 去重（同一 code+name 只保留一条）
            let mut seen = std::collections::HashSet::new();
            let deduped: Vec<SeedItem> = custom
                .into_iter()
                .filter(|item| seen.insert((item.0.clone(), item.1.clone())))
                .collect();
            (deduped, SeedPoolOrigin::default())
        },
        None => build_seed_pool(&client).await,
    };
    let raw_seed_pool_size = seed.len();
    tracing::info!("[recommender] raw_seed_pool_size={}", raw_seed_pool_size);
    // 保留 raw_seed 给 WatchlistStrategy（它只依赖 quote，不依赖 K 线）
    let raw_seed = seed.clone();
    // P3-D12: 根据 vendor 健康度动态调节流动性过滤并发数
    // - healthy_ratio >= 0.6 → 8（默认全速）
    // - 0.3 <= healthy_ratio < 0.6 → 4（部分降级，降速避免雪崩）
    // - healthy_ratio < 0.3 → 2（保命模式）
    let health_snapshot = client.health_tracker.get_all_health().await;
    let liquidity_concurrency = {
        use axagent_astock_data::vendor_health::VendorStatus;
        let active: Vec<_> =
            health_snapshot.iter().filter(|h| h.status != VendorStatus::Disabled).collect();
        if active.is_empty() {
            8 // 无数据默认全速
        } else {
            let healthy = active.iter().filter(|h| h.status == VendorStatus::Healthy).count();
            let ratio = healthy as f64 / active.len() as f64;
            if ratio < 0.3 {
                2
            } else if ratio < 0.6 {
                4
            } else {
                8
            }
        }
    };
    tracing::info!("[recommender] P3-D12 liquidity_filter concurrency={}", liquidity_concurrency,);
    seed =
        liquidity_filter_and_truncate_with_concurrency(client.clone(), seed, liquidity_concurrency)
            .await;
    tracing::info!(
        "[recommender] after liquidity filter, seed={}, raw_seed={}",
        seed.len(),
        raw_seed.len()
    );

    // 流动性过滤兜底：若 vendor 拿不到 60 日 K 线 / 全部不达标，过滤后池子可能
    // 全空。这种情况下 4 个主策略（trend/value/capital/reversion）会直接跳过。
    // Watchlist 用 raw_seed（不过流动性过滤），保证面板至少显示内容。
    // 注意：非 Watchlist 策略不再 fallback 到 raw_seed——raw_seed 未过滤流动性，
    // 可能导致大量无效 scan_one 调用（scan_one 内部判空后 return None）。
    if seed.is_empty() && !raw_seed.is_empty() {
        eprintln!(
            "[recommender] liquidity filter removed all {} stocks; \
             Watchlist will use raw seed, main strategies skip",
            raw_seed_pool_size
        );
    }

    // 注入 Serenity 候选（由 serenity-screening workflow 发现，写入 reco_picks，
    // 并通过 set_serenity_seed 同步到全局静态）
    let serenity_seed = get_serenity_seed();
    if !serenity_seed.is_empty() {
        for item in &serenity_seed {
            if !seed.contains(item) {
                seed.push(item.clone());
            }
        }
    }

    // 3. 选定该 period 下的所有子策略（不再做 vendor 禁用检查——
    //    原本要求 "enabled_vendors" 至少覆盖一个 required vendor，但生产环境
    //    workflow template 中往往 vendor_* 变量为空，导致 4 个 style 全 disabled。
    //    现在所有 style 都跑；data 真的取不到时该 style 自然返回空 picks，
    //    前端展示 "no data" 而非误报 "数据源未启用"）

    PreparedScan {
        client,
        seed,
        raw_seed,
        seed_pool_origin,
        raw_seed_pool_size,
        vars_map: template_vars.iter().cloned().collect(),
        reco_cfg: parse_reco_config(template_vars),
        // 复盘→进化：按 (style, period) 注入自适应权重
        strategy_weights: parse_strategy_weights(template_vars),
        // 跨档共享同一把 per-code 锁：四档并发扫同一只票时串行，避免并发打爆 vendor
        per_code_locks: PerCodeLocks::new(),
        use_cache,
        all_sources_down,
        horizon_prior,
    }
}

/// 对**单个档位**执行策略扫描（消费已准备好的 [`PreparedScan`]）。
///
/// 档位之间可并发调用本函数（共享同一个 `PreparedScan`），也可单独调用 ——
/// `recommend_stocks` 就是「准备一次 + 扫一档」，`recommend_stocks_multi` 是「准备一次 + 扫四档」。
async fn scan_period(prepared: &PreparedScan, period: Period) -> Result<RecoResponse, String> {
    // 两源全挂：该档直接出带归因的空响应（不静默出空表，也不建池不扫描）
    if let Some(detail) = &prepared.all_sources_down {
        let resp = empty_error_response(period, detail.clone());
        if prepared.use_cache {
            cache_put(period, resp.clone());
        }
        return Ok(resp);
    }

    let client = prepared.client.clone();
    let seed = prepared.seed.clone();
    let raw_seed = prepared.raw_seed.clone();
    let seed_pool_origin = prepared.seed_pool_origin;
    let raw_seed_pool_size = prepared.raw_seed_pool_size;
    let vars_map = prepared.vars_map.clone();
    let reco_cfg = &prepared.reco_cfg;
    let strategy_weights = &prepared.strategy_weights;
    let per_code_locks = prepared.per_code_locks.clone();
    let use_cache = prepared.use_cache;
    let horizon_prior = prepared.horizon_prior.clone();

    // 3. 选定该 period 下的所有子策略（不再做 vendor 禁用检查——
    //    原本要求 "enabled_vendors" 至少覆盖一个 required vendor，但生产环境
    //    workflow template 中往往 vendor_* 变量为空，导致 4 个 style 全 disabled。
    //    现在所有 style 都跑；data 真的取不到时该 style 自然返回空 picks，
    //    前端展示 "no data" 而非误报 "数据源未启用"）
    // 策略选择由 `style_matrix` 驱动（24 格契约表），不再在本函数里各档手抄一份清单；
    // 每格「做 / 按设计不做 + 理由码」的唯一来源是那张表（门 e / 单测双向锁）。
    let all_strategies: Vec<Box<dyn RecommendStrategy>> = strategies_for_period(period);

    let mut disabled_styles_set: std::collections::HashSet<Style> =
        std::collections::HashSet::new();
    let enabled: Vec<Box<dyn RecommendStrategy>> = all_strategies
        .into_iter()
        .filter(|s| {
            let ok = match s.style() {
                Style::Trend => reco_cfg.trend_enabled,
                Style::Value => reco_cfg.value_enabled,
                Style::Capital => reco_cfg.capital_enabled,
                Style::Reversion => reco_cfg.reversion_enabled,
                Style::Watchlist => reco_cfg.watchlist_enabled,
                Style::Bottleneck => reco_cfg.serenity_enabled,
                Style::Policy => reco_cfg.serenity_enabled,
                Style::Earnings => reco_cfg.serenity_enabled,
                Style::CapitalFlow => reco_cfg.serenity_enabled,
                Style::Event => reco_cfg.serenity_enabled,
                Style::Technical => reco_cfg.serenity_enabled,
            };
            if !ok {
                disabled_styles_set.insert(s.style());
            }
            ok
        })
        .collect();

    // 4. 并行执行（per-code 互斥锁：不同 code 真正并行，同 code 4 策略间串行；
    //    锁与池来自 `PreparedScan`，四档并发时同一只票跨档也串行）
    // 波动率风控参数（Phase R-D）。默认值即 PLAN §九 Q5 拍板值：k1=1.2、k2=2.0、R=1.5%、
    // 往返成本 0.6%。这四个键同时登记在 seed 变量表与股票分析设置面板（五点对账）。
    let read_par = |name: &str, default: f64| -> f64 {
        vars_map.get(name).and_then(|v| v.as_f64()).unwrap_or(default)
    };
    let stop_k1 = read_par("reco_stop_vol_mult", 1.2);
    let target_k2 = read_par("reco_target_vol_mult", 2.0);
    let risk_budget_pct = read_par("reco_risk_budget_pct", 1.5);
    let round_trip_cost_pct = read_par("reco_round_trip_cost_pct", 0.6);

    let ctx_pool = seed.clone();
    let raw_ctx_pool = raw_seed.clone();
    let mut futures = Vec::new();
    for s in enabled.iter() {
        // WatchlistStrategy 用 raw_seed（不过流动性过滤），
        // 这样 K 线全部拿不到时它仍能基于 raw pool 出 picks
        let use_raw = matches!(s.style(), Style::Watchlist);
        let s_ref: &dyn RecommendStrategy = s.as_ref();
        let client_ref: Arc<AStockClient> = client.clone();
        let lock_ref = per_code_locks.clone();
        let seed_ref = if use_raw {
            raw_ctx_pool.clone()
        } else {
            ctx_pool.clone()
        };
        let period_val = period;
        let vars_for_future = vars_map.clone();
        let horizon_prior = horizon_prior.clone();
        // 复盘→进化：该 (style, period) 当前的权重
        let style_weight = strategy_weights.get(&(s.style(), period)).copied().unwrap_or(1.0);
        let fut = async move {
            let ctx = RecoContext {
                client: &client_ref,
                seed: &seed_ref,
                per_code_locks: lock_ref,
                period: period_val,
                vars: &vars_for_future,
            };
            let mut raw = s_ref.scan(&ctx).await?;
            // 应用自适应权重：confidence 与 position_pct 同步缩放
            for p in raw.iter_mut() {
                let old_conf = p.confidence;
                // ── 置信度语义重构（2026-10-18，v4）：撤伪概率链 ──
                // 旧链（v1~v3）把策略「条件满足度」评分**当概率**喂给 logit 合成（对
                // 非概率量做贝叶斯运算，数学上从第一步就错）→ SNR 时间折算（第二次伪
                // 概率操作）→ quality 校准。而策略层输入几乎全是模板常数（value 五参
                // 全常数 / trend·capital 仅量比二值）⇒ 同档所有票塌成「档位常数」
                // （60/56/53/51）—— 用户裁定「这基本上就是造假」。
                // 新语义：`confidence` = **证据强度**（策略条件满足度 × 风格权重），
                // 个股维度（v3 已把连续证据强度引入策略层），明确不是「上涨概率」。
                // 风格权重保留：IC 闭环的真数据（该风格该档该不该降权），对同档所有
                // 票统一缩放，不伪造个股区分。
                // 档位历史胜率（真数据）的正确展示位是**档位级**——个股级校准等证据
                // 分桶样本积累（`PLAN-capital-long-evidence.md` §六）。
                let new_conf = ((old_conf as f64) * style_weight).round().clamp(0.0, 100.0) as u8;
                p.prior_source = Some("evidence".to_string());
                p.prior_samples = None;
                // 原始证据分（风格权重调制前）随 pick_data JSON 落库——分桶校准的输入
                // （校准必须用权重调制前的值：权重是闭环降权因子，混入会把「风格被
                // 降权」误读成「该票证据弱」，见 types.rs 字段注释）。
                p.evidence_score = Some(old_conf);
                // 档位先验降级为观测留痕（debug 档——它是档位级真数据，但不再进入
                // 个股数字；观察其长期形态供 §六 校准设计参考）。
                if let Some(row) = horizon_prior.as_ref().and_then(|j| j.get(period_val.as_str())) {
                    tracing::debug!("[reco] period={} tier_prior={}", period_val.as_str(), row);
                }
                // 还原策略给出的 base：主路径已**不乘**经验周期乘数（见 scoring.rs 注释），
                // 故这里只除掉置信度一项。
                let base = if old_conf > 0 {
                    p.position_pct / (old_conf as f64 / 100.0)
                } else {
                    p.position_pct
                };
                p.confidence = new_conf;

                // ── 波动率风控（Phase R-D）──
                // σ 一律取**日线**口径（`harness::indicators::realized_vol_pct`，全仓唯一实现），
                // 因为跨档可比的是「日波动 × √持有天数」，用各档自己的 bar 算 σ 会让四档不可比。
                // 取不到 σ ⇒ 显式退回策略自带的固定百分比，并把来源标出来，不伪装成波动率口径。
                let h = period_val.default_holding_days() as usize;
                let closes =
                    crate::recommender::risk::daily_closes(&client_ref, &p.stock_code).await;
                let vol_stop =
                    closes.as_ref().and_then(|c| crate::recommender::risk::stop_pct(c, h, stop_k1));
                let vol_target = closes
                    .as_ref()
                    .and_then(|c| crate::recommender::risk::target_pct(c, h, target_k2));
                let price_ok = p.price.is_finite() && p.price > 0.0;

                let band_stop_pct = if price_ok && p.stop_loss > 0.0 {
                    ((p.price - p.stop_loss) / p.price * 100.0).max(0.0)
                } else {
                    0.0
                };
                let band_target_pct = if price_ok && p.target_price > 0.0 {
                    ((p.target_price - p.price) / p.price * 100.0).max(0.0)
                } else {
                    0.0
                };

                let (used_stop_pct, stop_src) = match vol_stop {
                    Some(v) => (v, "vol"),
                    None => (band_stop_pct, "fallback_pct"),
                };
                let used_target_pct = vol_target.unwrap_or(band_target_pct);
                if price_ok && used_stop_pct > 0.0 {
                    p.stop_loss = (p.price * (1.0 - used_stop_pct / 100.0) * 100.0).round() / 100.0;
                }
                if price_ok && used_target_pct > 0.0 {
                    p.target_price =
                        (p.price * (1.0 + used_target_pct / 100.0) * 100.0).round() / 100.0;
                }
                p.stop_source = Some(stop_src.to_string());

                // ── 仓位 = min(策略上限, 该档风险预算) × 成本拖累 ──
                // 风险预算仓位是「止损被打掉时组合恰好损失 R%」的定义式；σ 不可得 ⇒ 止损宽度
                // 来自固定百分比，风险预算仍能算，但来源标成 fallback；连止损都没有 ⇒ 退乘数分支。
                let strategy_cap = base * (new_conf as f64 / 100.0);
                let budget =
                    crate::recommender::risk::risk_budget_position(used_stop_pct, risk_budget_pct);
                let (pos_before_drag, pos_src) = match budget {
                    Some(b) => (strategy_cap.min(b), "risk_budget"),
                    None => (
                        crate::recommender::scoring::calc_position_fallback_mult(
                            base, new_conf, period_val,
                        ),
                        "fallback_kelly_x_mult",
                    ),
                };
                let drag = vol_target
                    .or(if band_target_pct > 0.0 {
                        Some(band_target_pct)
                    } else {
                        None
                    })
                    .map(|t| crate::recommender::risk::cost_drag_factor(t, round_trip_cost_pct))
                    .unwrap_or(1.0);
                p.position_pct = (pos_before_drag * drag).clamp(0.0, 95.0);
                p.position_source = Some(pos_src.to_string());
                p.reasons.push(format!(
                    "风控: 止损 {:.2}% ({}) · 目标 {:.2}% · 仓位上限 {:.1}% × 风险预算 {:.1}% × 成本拖累 {:.2}",
                    used_stop_pct,
                    stop_src,
                    used_target_pct,
                    strategy_cap,
                    budget.unwrap_or(0.0),
                    drag
                ));
                // ── 信号质量校准已随 v4 撤下（2026-10-18）──
                // 它校正的对象是伪概率（旧链的「先验合成 + SNR 折算」输出）——校准一个
                // 假起点只会让假数字显得更可信。`weighted_signal_calibration`（真数据：
                // 按 (style, period) 的后验胜率）保留为 pub，个股级校准恢复时以
                // 「证据分桶 × 真实 T+N 胜率」的形态回来（PLAN-capital-long-evidence §六）。
                // 旧「反身性」折扣（只对超短 ×0.85）此前已删：拍的数、无推导。
            }
            Ok::<_, String>(raw)
        };
        futures.push(fut);
    }
    let results: Vec<Result<Vec<types::RecoPick>, String>> =
        futures::future::join_all(futures).await;

    // 5. 合并
    let mut all_picks: Vec<types::RecoPick> = Vec::new();
    for mut picks in results.into_iter().flatten() {
        all_picks.append(&mut picks);
    }
    // 诊断日志 V52：扫描完成后输出各策略的出票数
    {
        let mut counts: std::collections::BTreeMap<types::Style, usize> =
            std::collections::BTreeMap::new();
        for p in &all_picks {
            *counts.entry(p.style).or_insert(0) += 1;
        }
        tracing::info!(
            "[recommender] period={:?}, raw_picks={}, by_style={:?}",
            period,
            all_picks.len(),
            counts
        );
    }

    // 6. 去重
    dedup_and_merge(&mut all_picks);
    tracing::info!("[recommender] after dedup, picks={}", all_picks.len());

    // P3-1: drop picks whose numeric fields are NaN/inf — these would render as "NaN" in JSON.
    all_picks.retain(|p| {
        !p.price.is_nan()
            && !p.entry_low.is_nan()
            && !p.entry_high.is_nan()
            && !p.stop_loss.is_nan()
            && !p.target_price.is_nan()
            && !p.position_pct.is_nan()
    });
    tracing::info!("[recommender] after NaN filter, picks={}", all_picks.len());

    // P3-2: drop picks whose target_price is already below current price
    // (no upside left — the BUY thesis is dead). Frontend should also visually
    // flag this in case cache holds the stale pick, but the backend drop
    // makes sure new scans never emit these in the first place.
    all_picks.retain(|p| {
        // 允许 ≤0.5% 的轻微容差，避免价格微抖时被误杀
        p.target_price >= p.price * 0.995
    });
    tracing::info!("[recommender] after target<price filter, picks={}", all_picks.len());

    // P3-3: drop picks below user-configured min_confidence
    // (reco_min_confidence from StockAnalysisConfigPanel, 0 = no filter)
    //
    // Watchlist 豁免：它是候选池兜底策略，置信度按设计就低
    // （watchlist.rs 注释「信心度低（0.55），因为没有技术信号支撑」，
    // calc_confidence(0.55, 0.5, 0.5, 0.0, 1.0) ≈ 50，即上限约 50），
    // 且自带专属门槛 wl_min_confidence（默认 30）。全局绝对阈值
    // reco_min_confidence 一旦 ≥ 50 就会把 watchlist 结构性全灭，
    // 其空桶随即被 emit_synthetic_picks 的「信号缺失，按现价合成」占位填满，
    // 前端看到的是 10 条假候选而非「该风格无真实信号」。
    if reco_cfg.min_confidence > 0 {
        tracing::info!(
            "[recommender] before confidence filter (min={}), picks={}",
            reco_cfg.min_confidence,
            all_picks.len()
        );
        all_picks.retain(|p| {
            p.style == types::Style::Watchlist || p.confidence >= reco_cfg.min_confidence
        });
        tracing::info!("[recommender] after confidence filter, picks={}", all_picks.len());
    }

    // ── P0+P1 出票门（2026-10-18，002696 案例复盘）：mid/long 档财报预检 + 分析回环否决 ──
    // 入池判据是「热门 ∪ 行业领涨」（零基本面门，pool.rs），题材脉冲股必然入池；分析端
    // 又会对同一批票判「长线卖出」⇒「入池按趋势、分析按基本面」结构性对立。本门把两条
    // 硬否决前移到出票口：最新一期净利 ≤0 / 连续两期同比下滑 ⇒ 拦（P0 财报门）；最近
    // 30 天 live 分析对本档判 卖出/减持 ⇒ 拦（P1 回环门，否决表由 `reco_loop` 经
    // `reco_analysis_vetoes` 模板变量注入，as-of 回放不注入）。只拦非 synthetic picks；
    // 财报取数失败 fail-open（gates.rs 内 warn 留痕）；拦截逐条进 resp.gate_blocked
    // （结构化载体，不靠日志反查）。ultra_short/short 不设此门 —— 与
    // `portfolio-mgr-h-ultra-short.rhai`「基本面按构造不适用」同一档位裁定。
    let mut gate_blocked: Vec<types::GateBlockedPick> = Vec::new();
    if matches!(period, Period::Mid | Period::Long) {
        // ⚠ 回放不消费否决表（回测语义 =「当时的判定」，不带今天的回环）。
        //   守卫必须在这里而不是加载侧：served_vars 在命令层 as_of scope 之外加载，
        //   加载时 current_as_of() 恒为 None ⇒ 加载侧守卫是假守卫（防线漂移经典形态）。
        let veto_var = if as_of::current_as_of().is_none() {
            vars_map.get("reco_analysis_vetoes")
        } else {
            None
        };
        gate_blocked = gates::apply_exit_gates(&client, period, &mut all_picks, veto_var).await;
        if !gate_blocked.is_empty() {
            tracing::info!(
                "[reco_gate] period={} 出票门拦截 {} 只（逐条理由见 [reco_gate] 拦截行）",
                period.as_str(),
                gate_blocked.len()
            );
        }
    }

    // 7. 按风格分组 + 限 10（截断尾部留痕，供 L3 归因与 Phase 4 降权）
    let (mut by_style, scan_audit) = group_by_style_and_trim_with_audit(&mut all_picks, 10);
    for (style, picks) in &by_style {
        tracing::info!("[recommender] final bucket: style={:?}, picks={}", style, picks.len());
    }

    // 8. 数据稀疏兜底：5 个 style 桶里若有空，且 raw_seed 非空，用 get_quote 拉
    //    基础行情 emit 合成 picks 填入对应 style 桶。解决"只有 Watchlist 有 10 条，
    //    其他 4 个主风格全空"的问题（K 线 / 财务 / 资金数据全不可用时主策略都
    //    短路返回 None，但 watchlist 仅依赖 quote 仍能出 picks）。
    //
    //    合成 pick 的 reason 明确标注"信号缺失，按现价合成"，UI 可区分。
    //    跳过 dedup：兜底数据加在 by_style 之后，不与真实 pick 合并。
    if !raw_seed.is_empty() {
        for style in [
            Style::Trend,
            Style::Value,
            Style::Capital,
            Style::Reversion,
            Style::Watchlist,
            Style::Bottleneck,
            Style::Policy,
            Style::Earnings,
            Style::CapitalFlow,
            Style::Event,
            Style::Technical,
        ] {
            // 用户已禁用的风格不补充合成 picks
            if disabled_styles_set.contains(&style) {
                continue;
            }
            let bucket_empty = by_style.get(&style).is_none_or(|v| v.is_empty());
            if !bucket_empty {
                continue;
            }
            let synthetic = emit_synthetic_picks(
                client.clone(),
                style,
                period,
                &raw_seed,
                per_code_locks.clone(),
                &vars_map,
            )
            .await;
            let mut tagged: Vec<types::RecoPick> = synthetic;
            tagged.truncate(10);
            tracing::info!(
                "[recommender] style={:?} bucket was empty, filled with {} synthetic picks",
                style,
                tagged.len()
            );
            by_style.insert(style, tagged);
        }
    }

    let as_of_ctx = as_of::current_as_of();
    // spec §8: as-of 模式下因数据截断被降级(≠ 缺失)的风格,前端用橙色显示
    // 简化: 若 as-of 激活 + 某风格被 disabled,等同降级;后续 B11 可细化原因
    let disabled_vec: Vec<Style> = disabled_styles_set.into_iter().collect();
    let (degraded_styles, degraded_reasons) = if as_of_ctx.is_some() {
        let reasons: std::collections::HashMap<Style, String> = disabled_vec
            .iter()
            .map(|s| (*s, "as-of 截断后该风格依赖的历史数据不可用".to_string()))
            .collect();
        (disabled_vec.clone(), reasons)
    } else {
        (Vec::new(), std::collections::HashMap::new())
    };
    let resp = RecoResponse {
        period,
        picks: by_style,
        disabled_styles: disabled_vec,
        degraded_styles,
        degraded_reasons,
        generated_at: chrono::Utc::now().timestamp_millis(),
        raw_seed_pool_size,
        seed_pool_origin,
        // 由外层 `recommend_stocks` 按运行水位回填；inner 本身不感知切片。
        asof_degradations: vec![],
        as_of_date: as_of_ctx.as_ref().map(|c| c.as_string()),
        mode: as_of_ctx
            .as_ref()
            .map(|c| c.source.to_string())
            .unwrap_or_else(|| "live".to_string()),
        error_detail: None,
        // P3 修复(2026-08-01): 落库快照用扫描实际使用的池（流动性过滤后 seed），
        // command 层不再二次 build_seed_pool（避免快照与真实扫描不一致 + 浪费请求）。
        seed_pool_snapshot: Some(
            serde_json::to_string(
                &seed.iter().map(|(c, n, _)| vec![c.as_str(), n.as_str()]).collect::<Vec<_>>(),
            )
            .unwrap_or_default(),
        ),
        scan_audit,
        gate_blocked,
    };
    if use_cache {
        cache_put(period, resp.clone());
    }
    Ok(resp)
}

/// 按档位构造策略集合 —— **选择来自 [`style_matrix`]**，构造仍是逐 `(style, tier)` 显式。
///
/// 一致性由单测 `matrix_matches_strategy_selection` 锁：表里出票的格必须真能构造出策略，
/// 反之亦然（表改了代码没改 = 红）。
pub(crate) fn strategies_for_period(period: Period) -> Vec<Box<dyn RecommendStrategy>> {
    let mut out: Vec<Box<dyn RecommendStrategy>> = Vec::new();
    for style in style_matrix::styles_for(period) {
        let s: Option<Box<dyn RecommendStrategy>> = match (style, period) {
            (Style::Trend, Period::UltraShort) => Some(Box::new(TrendStrategy::ultra_short())),
            (Style::Trend, Period::Short) => Some(Box::new(TrendStrategy::short())),
            (Style::Trend, Period::Mid) => Some(Box::new(TrendStrategy::mid())),
            (Style::Trend, Period::Long) => Some(Box::new(TrendStrategy::long())),
            (Style::Value, Period::UltraShort) => Some(Box::new(ValueStrategy::ultra_short())),
            (Style::Value, Period::Short) => Some(Box::new(ValueStrategy::short())),
            (Style::Value, Period::Mid) => Some(Box::new(ValueStrategy::mid())),
            (Style::Value, Period::Long) => Some(Box::new(ValueStrategy::long())),
            (Style::Capital, Period::UltraShort) => Some(Box::new(CapitalStrategy::ultra_short())),
            (Style::Capital, Period::Short) => Some(Box::new(CapitalStrategy::short())),
            (Style::Capital, Period::Mid) => Some(Box::new(CapitalStrategy::mid())),
            (Style::Capital, Period::Long) => Some(Box::new(CapitalStrategy::long())),
            (Style::Reversion, Period::Short) => Some(Box::new(ReversionStrategy::short())),
            (Style::Reversion, Period::Mid) => Some(Box::new(ReversionStrategy::mid())),
            (Style::Watchlist, Period::UltraShort) => {
                Some(Box::new(WatchlistStrategy::ultra_short()))
            },
            (Style::Watchlist, Period::Short) => Some(Box::new(WatchlistStrategy::short())),
            (Style::Watchlist, Period::Mid) => Some(Box::new(WatchlistStrategy::mid())),
            (Style::Watchlist, Period::Long) => Some(Box::new(WatchlistStrategy::long())),
            (Style::Bottleneck, Period::Mid) => Some(Box::new(SerenityStrategy::mid())),
            (Style::Bottleneck, Period::Long) => Some(Box::new(SerenityStrategy::long())),
            _ => None,
        };
        match s {
            Some(boxed) => out.push(boxed),
            // 表说该格出票、构造却落 `_ => None` ⇒ 显式 panic（静默少一个风格就是「表与实现漂移」）
            None => panic!("style_matrix 声明 {style:?}×{period:?} 出票，但没有对应构造函数"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommender::types::{Period, RecoPick, Style};

    /// 构造一个带 dummy pick 的 RecoResponse（避免 cache_put 跳过空结果）
    fn dummy_resp(period: Period, generated_at: i64, mode: &str) -> RecoResponse {
        let pick = RecoPick {
            stock_code: "000001".into(),
            stock_name: "平安银行".into(),
            sector: None,
            style: Style::Trend,
            period,
            price: 10.0,
            entry_low: 9.5,
            entry_high: 10.5,
            stop_loss: 9.0,
            target_price: 12.0,
            position_pct: 10.0,
            holding_days: 5,
            confidence: 70,
            reasons: vec!["测试".into()],
            risk_notes: vec![],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,

            synthetic: false,
        };
        let mut picks = std::collections::BTreeMap::new();
        picks.insert(Style::Trend, vec![pick]);
        RecoResponse {
            period,
            picks,
            disabled_styles: vec![],
            degraded_styles: vec![],
            degraded_reasons: std::collections::HashMap::new(),
            generated_at,
            raw_seed_pool_size: 1,
            seed_pool_origin: SeedPoolOrigin::default(),
            asof_degradations: vec![],
            as_of_date: None,
            mode: mode.to_string(),
            error_detail: None,
            seed_pool_snapshot: None,
            scan_audit: vec![],
            gate_blocked: vec![],
        }
    }

    /// 进程级缓存是 `static RESULT_CACHE`，而 `invalidate_cache()` 清**整张表**
    /// ⇒ 任何两条碰缓存的测试并发跑都会互删条目（实测：本文件的
    /// `cache_invalidate_works` 把 `cache_live_and_replay_are_isolated` step-1 写入的
    /// live 条目清掉，使它在「切回 live 仍应命中」那一步假红）。
    /// 修在测试侧而不是给每条测试挑一个"没人用的 Period"——那只是把下一次踩雷
    /// 留给第三个加缓存测试的人。⚠ 用 `tokio::sync::Mutex`：`std::sync::Mutex` 及其
    /// Guard 在 `clippy.toml` 的 `disallowed-types` 里，且这里持锁要跨 `await`，
    /// 不属于「不跨 await 的同步临界区」那条例外。
    static CACHE_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[tokio::test]
    async fn cache_invalidate_works() {
        let _g = CACHE_TEST_LOCK.lock().await;
        invalidate_cache();
        let resp = dummy_resp(Period::Short, 0, "live");
        cache_put(Period::Short, resp);
        assert!(cache_get(Period::Short).is_some());
        invalidate_cache();
        assert!(cache_get(Period::Short).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn cache_live_and_replay_are_isolated() {
        use axagent_astock_data::as_of::{AsOfContext, AsOfSource};
        use chrono::NaiveDate;
        // 与 `cache_invalidate_works` 共用一把锁（见上方 `CACHE_TEST_LOCK` 的说明）
        let _g = CACHE_TEST_LOCK.lock().await;
        invalidate_cache();

        // 1) live scope 写入
        let live_resp = dummy_resp(Period::Short, 1, "live");
        cache_put(Period::Short, live_resp);

        // 2) live 读命中 (generated_at == 1)
        assert_eq!(cache_get(Period::Short).unwrap().generated_at, 1);

        // 3) replay scope 内读应当 miss
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let got_in_replay = as_of::AS_OF.scope(Some(ctx), async { cache_get(Period::Short) }).await;
        assert!(
            got_in_replay.is_none(),
            "live cache must not leak into replay scope (got generated_at={:?})",
            got_in_replay.as_ref().map(|r| r.generated_at)
        );

        // 4) replay scope 内写入 → 退出 scope 后 live 仍然 miss，但 replay 自己命中
        let mut replay_resp = dummy_resp(Period::Short, 2, "user_replay");
        replay_resp.as_of_date = Some("2026-06-01".into());
        let replay_cached = as_of::AS_OF
            .scope(Some(ctx), async {
                cache_put(Period::Short, replay_resp);
                cache_get(Period::Short)
            })
            .await
            .expect("replay cache miss after put");
        assert_eq!(replay_cached.generated_at, 2);

        // 5) 切回 live 后再读：不会读到 replay 写入的脏数据，应返回 step 1 的 live entry
        let live_hit = cache_get(Period::Short);
        assert!(live_hit.is_some(), "live cache entry from step 1 should still exist");
        assert_eq!(
            live_hit.unwrap().generated_at,
            1,
            "replay entry (generated_at=2) must not leak into live cache"
        );

        // 6) live scope 再次写入 → 覆盖
        let live_resp2 = dummy_resp(Period::Short, 11, "live");
        cache_put(Period::Short, live_resp2);
        assert_eq!(cache_get(Period::Short).unwrap().generated_at, 11);

        invalidate_cache();
    }

    #[test]
    fn reco_response_serializes_asof_fields() {
        let resp = RecoResponse {
            period: Period::Mid,
            picks: std::collections::BTreeMap::new(),
            disabled_styles: vec![],
            degraded_styles: vec![],
            degraded_reasons: std::collections::HashMap::new(),
            generated_at: 100,
            raw_seed_pool_size: 50,
            seed_pool_origin: SeedPoolOrigin::default(),
            asof_degradations: vec![],
            as_of_date: Some("2026-06-01".into()),
            mode: "user_replay".into(),
            error_detail: None,
            seed_pool_snapshot: None,
            scan_audit: vec![],
            gate_blocked: vec![],
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"asOfDate\":\"2026-06-01\""));
        assert!(s.contains("\"mode\":\"user_replay\""));
    }

    #[test]
    fn reco_response_live_omits_asof_field() {
        let resp = RecoResponse {
            period: Period::Mid,
            picks: std::collections::BTreeMap::new(),
            disabled_styles: vec![],
            degraded_styles: vec![],
            degraded_reasons: std::collections::HashMap::new(),
            generated_at: 100,
            raw_seed_pool_size: 50,
            seed_pool_origin: SeedPoolOrigin::default(),
            asof_degradations: vec![],
            as_of_date: None,
            mode: "live".into(),
            error_detail: None,
            seed_pool_snapshot: None,
            scan_audit: vec![],
            gate_blocked: vec![],
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(!s.contains("asOfDate"), "as_of_date should be skipped when None");
        assert!(s.contains("\"mode\":\"live\""));
    }

    fn make_pick(price: f64, target: f64) -> types::RecoPick {
        types::RecoPick {
            stock_code: "688056".into(),
            stock_name: "莱伯泰科".into(),
            sector: None,
            style: Style::Trend,
            period: Period::Short,
            price,
            entry_low: price * 0.98,
            entry_high: price * 1.02,
            stop_loss: price * 0.95,
            target_price: target,
            position_pct: 5.0,
            holding_days: 5,
            confidence: 60,
            reasons: vec!["test".into()],
            risk_notes: vec![],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,

            synthetic: false,
        }
    }

    /// Bug 修复：target ≤ current price 的 pick 必须被剔除。
    /// 旧逻辑会保留"现价 48 / 目标 41"的 BUY 推荐，逻辑上矛盾。
    ///
    /// 阈值 0.995 的实际行为：
    /// - 41.87 / 48.16 = 0.869 → DROP
    /// - 42.00 / 38.00 = 1.105 → KEEP
    /// - 38.00 / 38.00 = 1.000 → KEEP（target == price 在容差内）
    /// - 37.81 / 38.00 ≈ 0.995 → KEEP（恰好 >= 0.995 浮点边界）
    /// - 37.85 / 38.00 ≈ 0.996 → KEEP
    ///   期望保留 4 个（pick 2/3/4/5）。
    #[test]
    fn drops_picks_with_no_upside() {
        let mut picks = vec![
            make_pick(48.16, 41.87), // ✗ 无上行空间
            make_pick(38.0, 42.0),   // ✓ 正常
            make_pick(38.0, 38.0),   // ✓ target == price 在 0.995 容差内
            make_pick(38.0, 37.81),  // ✓ 浮点边界，>= 0.995
            make_pick(38.0, 37.85),  // ✓ 37.85/38 ≈ 0.996 > 0.995
        ];
        picks.retain(|p| p.target_price >= p.price * 0.995);
        let codes: Vec<&str> = picks.iter().map(|p| p.stock_code.as_str()).collect();
        assert_eq!(picks.len(), 4, "保留: {codes:?}");
    }

    #[test]
    fn parse_strategy_weights_basic() {
        let vars = vec![(
            "reco_strategy_weights".to_string(),
            serde_json::json!({
                "trend_short": 1.2,
                "value_mid": 0.5,
                "watchlist_long": 0.0
            }),
        )];
        let m = parse_strategy_weights(&vars);
        assert_eq!(m.get(&(Style::Trend, Period::Short)).copied(), Some(1.2));
        assert_eq!(m.get(&(Style::Value, Period::Mid)).copied(), Some(0.5));
        assert_eq!(m.get(&(Style::Watchlist, Period::Long)).copied(), Some(0.0));
        assert!(!m.contains_key(&(Style::Capital, Period::Short)), "缺失 key 不应有值");
    }

    #[test]
    fn parse_strategy_weights_clamps_to_2x() {
        let vars =
            vec![("reco_strategy_weights".to_string(), serde_json::json!({ "trend_short": 10.0 }))];
        let m = parse_strategy_weights(&vars);
        assert_eq!(m.get(&(Style::Trend, Period::Short)).copied(), Some(2.0), "应 clamp 到 2.0");
    }

    #[test]
    fn parse_strategy_weights_ignores_malformed_keys() {
        let vars = vec![(
            "reco_strategy_weights".to_string(),
            serde_json::json!({
                "trend_extra_short": 1.5,
                "unknown_short": 1.0,
                "trend_week": 1.0
            }),
        )];
        let m = parse_strategy_weights(&vars);
        assert!(m.is_empty(), "所有 key 都应被忽略");
    }

    #[test]
    fn parse_strategy_weights_absent_var_returns_empty() {
        let vars = vec![("reco_trend_enabled".to_string(), serde_json::json!(true))];
        let m = parse_strategy_weights(&vars);
        assert!(m.is_empty());
    }
}

#[cfg(test)]
mod style_matrix_wiring_tests {
    use super::*;

    /// 表 ⇔ 实现：每档构造出的策略风格集合必须与矩阵登记的出票格**逐项相等**。
    #[test]
    fn matrix_matches_strategy_selection() {
        for period in Period::ALL {
            let from_matrix: Vec<String> = {
                let mut v: Vec<String> = style_matrix::styles_for(period)
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect();
                v.sort();
                v
            };
            // 两侧同名目比较：矩阵里 `serenity` 映射到 `Style::Bottleneck`（SerenityStrategy::style() 同值）
            let mut built: Vec<String> = strategies_for_period(period)
                .iter()
                .map(|s| s.style().as_str().to_string())
                .collect();
            built.sort();
            built.dedup();
            assert_eq!(from_matrix, built, "档位 {period:?} 的矩阵与实现漂移");
            assert!(
                !built.is_empty(),
                "档位 {period:?} 一个策略都没构造出来（矩阵或构造表被清空）"
            );
        }
    }
}

#[cfg(test)]
mod strategy_weight_key_tests {
    use super::*;

    /// 键空间**往返恒等**：写侧 `format!("{style}_{period}")` 产出的每个键，
    /// 读侧都必须解析回同一个 (风格, 档位)。
    ///
    /// 这条是 `capital_flow_*` 静默丢弃（`splitn(2,'_')` 时代）的定点锁：
    /// 含下划线的风格名一旦新增/改名，此处先红，而不是等到「闭环权重不生效」才被怀疑。
    #[test]
    fn style_period_keys_round_trip() {
        for style in Style::ALL {
            for period in Period::ALL {
                for name in style.key_names() {
                    let key = format!("{name}_{}", period.as_str());
                    assert_eq!(
                        parse_style_period_key(&key),
                        Some((style, period)),
                        "键 `{key}` 未解析回 ({style:?}, {period:?})"
                    );
                }
            }
        }
    }

    /// 多词风格名不得被更短的风格前缀截胡：`capital_flow_mid` 属 CapitalFlow，
    /// 不能落进 Capital（余段 `flow_mid` 不是档位）。
    #[test]
    fn multiword_style_is_not_stolen_by_shorter_prefix() {
        assert_eq!(
            parse_style_period_key("capital_flow_ultra_short"),
            Some((Style::CapitalFlow, Period::UltraShort))
        );
        assert_eq!(parse_style_period_key("capital_mid"), Some((Style::Capital, Period::Mid)));
        // serenity 一名两写：两种写法都必须落到 Bottleneck
        assert_eq!(
            parse_style_period_key("serenity_long"),
            Some((Style::Bottleneck, Period::Long))
        );
        assert_eq!(
            parse_style_period_key("bottleneck_long"),
            Some((Style::Bottleneck, Period::Long))
        );
    }

    /// 端到端：闭环写回的 `capital_flow` 键必须被 `parse_strategy_weights` 消费到
    /// （旧实现下该断言恒假 —— 键被静默丢弃，权重表里根本没有这一格）。
    #[test]
    fn capital_flow_weight_reaches_the_table() {
        let vars = vec![(
            "reco_strategy_weights".to_string(),
            serde_json::json!({ "capital_flow_mid": 0.7, "serenity_short": 0.9 }),
        )];
        let m = parse_strategy_weights(&vars);
        assert_eq!(m.get(&(Style::CapitalFlow, Period::Mid)).copied(), Some(0.7));
        assert_eq!(m.get(&(Style::Bottleneck, Period::Short)).copied(), Some(0.9));
    }
}
