//! 候选池构造 + vendor 启用集合检测

use axagent_astock_data::AStockClient;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 候选池条目：(code, name, sector)
pub type SeedItem = (String, String, Option<String>);

/// 回退候选股列表（沪深 300 核心成分股 + 行业龙头，覆盖主要行业）
///
/// 当 `get_hot_stocks` / `get_industry_ranking` 都拿不到数据时用作种子池兜底，
/// 保证 `recommend_stocks` 至少能基于一组已知活跃股跑子策略。
const FALLBACK_STOCKS: &[(&str, &str)] = &[
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

/// 构造 seed pool
///
/// 顺序：get_hot_stocks(30) ∪ get_industry_ranking 龙头(10) ∪ FALLBACK_STOCKS 兜底
///
/// `get_hot_stocks` / `get_industry_ranking` 在 vendor 未启用 / 网络失败时返回 `Ok(vec![])`，
/// 因此函数末尾兜底加入 [`FALLBACK_STOCKS`]，保证 seed pool 永远非空。
pub async fn build_seed_pool(client: &AStockClient) -> Vec<SeedItem> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<SeedItem> = Vec::new();

    // 1. 热门个股
    if let Ok(hot) = client.get_hot_stocks().await {
        for h in hot.iter().take(30) {
            if seen.insert(h.stock_code.clone()) {
                out.push((h.stock_code.clone(), h.stock_name.clone(), h.sector.clone()));
            }
        }
    }

    // 2. 行业排名龙头
    if let Ok(industries) = client.get_industry_ranking().await {
        for ind in industries.iter().take(10) {
            if let (Some(code), Some(name)) = (&ind.leader_code, &ind.leader_name) {
                if seen.insert(code.clone()) {
                    out.push((code.clone(), name.clone(), Some(ind.industry_name.clone())));
                }
            }
        }
    }

    // 3. 兜底：若前两个源都没拿到（vendor 缺失 / 网络失败 / 非交易时段），
    // 退到一组已知活跃股，至少能跑 K 线 / 估值等基础策略
    if out.is_empty() {
        for (code, name) in FALLBACK_STOCKS {
            if seen.insert((*code).to_string()) {
                out.push(((*code).to_string(), (*name).to_string(), None));
            }
        }
    }

    out
}

/// 流动性过滤：日均成交额 ≥ 1 亿；排除 ST / 上市 < 60 日
/// 截断到 200
///
/// 用 `FuturesUnordered` 并发（最多 8 并发），避免串行 200 只导致整体超时
pub async fn liquidity_filter_and_truncate(
    client: Arc<AStockClient>,
    seed: Vec<SeedItem>,
) -> Vec<SeedItem> {
    use futures::stream::{FuturesUnordered, StreamExt};
    use tokio::sync::Semaphore;

    let sem = Arc::new(Semaphore::new(8));
    let mut tasks: FuturesUnordered<_> = FuturesUnordered::new();

    for item in seed {
        let sem = sem.clone();
        let client = client.clone();
        tasks.push(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            filter_one(&client, item).await
        });
    }

    let mut kept: Vec<SeedItem> = Vec::new();
    while let Some(res) = tasks.next().await {
        if let Some(item) = res {
            kept.push(item);
            if kept.len() >= 200 {
                break;
            }
        }
    }
    kept
}

async fn filter_one(client: &AStockClient, item: SeedItem) -> Option<SeedItem> {
    let (code, name, sector) = item;
    let quote = client.get_quote(&code).await.ok()?;
    if quote.is_st {
        return None;
    }
    let klines = client.get_klines(&code, "daily", 60).await.ok()?;
    if klines.len() < 55 {
        return None;
    }
    let avg: f64 = klines.iter().map(|k| k.amount).sum::<f64>() / klines.len() as f64;
    if avg < 100_000_000.0 {
        return None;
    }
    Some((code, name, sector))
}

/// 缓存 vendor 启用集合（5 min）
use std::sync::RwLock;
use std::time::{Duration, Instant};

static VENDOR_CACHE: RwLock<Option<(HashSet<String>, Instant)>> = RwLock::new(None);
const VENDOR_TTL: Duration = Duration::from_secs(300);

/// 通过 `get_workflow_template` 读 `vendor_*` 变量，返回启用的 vendor 集合
///
/// 此函数依赖 AppState 提供的 `harness.db()`；调用方传入 db handle
pub fn load_enabled_vendors_from_template(
    template_vars: &[(String, serde_json::Value)],
) -> HashSet<String> {
    let mut set = HashSet::new();
    for (name, value) in template_vars {
        if name.starts_with("vendor_") && name != "vendor_iwencai_key" {
            if let Some(v) = value.as_str() {
                if !v.is_empty() {
                    set.insert(name.trim_start_matches("vendor_").to_string());
                }
            } else if value.as_bool().unwrap_or(false) {
                set.insert(name.trim_start_matches("vendor_").to_string());
            }
        }
    }
    set
}

/// 读取缓存（命中且未过期直接返回）
pub fn get_cached_vendors() -> Option<HashSet<String>> {
    let guard = VENDOR_CACHE.read().ok()?;
    if let Some((set, ts)) = guard.as_ref() {
        if ts.elapsed() < VENDOR_TTL {
            return Some(set.clone());
        }
    }
    None
}

pub fn set_cached_vendors(set: HashSet<String>) {
    if let Ok(mut g) = VENDOR_CACHE.write() {
        *g = Some((set, Instant::now()));
    }
}

pub fn clear_cached_vendors() {
    if let Ok(mut g) = VENDOR_CACHE.write() {
        *g = None;
    }
}

/// 评估一组必需 vendor 中是否至少有一个启用
pub fn vendors_satisfied(required: &[&str], enabled: &HashSet<String>) -> bool {
    required.iter().any(|v| enabled.contains(*v))
}

/// 把 vendor 列表 (a股 vendor 名) 映射到 PANEL_VENDORS 里的 key
/// 与前端 [PANEL_VENDORS](file:///d:/OneManager/AxInvest/src/components/stock-analysis/vendorCheck.ts) 保持一致
pub fn required_vendors_for_style(
    style: crate::recommender::types::Style,
) -> &'static [&'static str] {
    use crate::recommender::types::Style::*;
    match style {
        // 趋势跟踪：依赖 K 线（任意一家支持 K 线的 vendor）
        Trend => &["eastmoney", "tencent", "ths", "akshare"],
        // 价值低估：需要估值 / 财务
        Value => &["eastmoney", "ths", "akshare"],
        // 资金驱动：需要资金流向 + 龙虎（ths / baidu_stock 任一即可）
        Capital => &["ths", "baidu_stock"],
        // 超跌反弹：依赖 K 线
        Reversion => &["eastmoney", "tencent", "ths", "akshare"],
        // 候选池兜底：只依赖实时 quote
        Watchlist => &["tencent", "eastmoney", "mootdx"],
    }
}

/// 给后端 logs 用的：从 HashSet 序列化为 vendor → bool 表，方便调试
pub fn vendors_to_map(enabled: &HashSet<String>) -> HashMap<String, bool> {
    let all = [
        "eastmoney",
        "tencent",
        "ths",
        "baidu_stock",
        "akshare",
        "iwencai",
        "cninfo",
        "sina",
    ];
    all.iter()
        .map(|v| (v.to_string(), enabled.contains(*v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommender::types::Style;

    #[test]
    fn vendors_satisfied_any_match() {
        let mut set = HashSet::new();
        set.insert("ths".to_string());
        assert!(vendors_satisfied(&["eastmoney", "ths", "akshare"], &set));
    }

    #[test]
    fn vendors_satisfied_none_match() {
        let set: HashSet<String> = HashSet::new();
        assert!(!vendors_satisfied(&["eastmoney", "ths"], &set));
    }

    #[test]
    fn vendor_template_extraction() {
        let vars = vec![
            ("vendor_eastmoney".to_string(), serde_json::json!("enabled")),
            ("vendor_tencent".to_string(), serde_json::json!("")),
            ("vendor_akshare".to_string(), serde_json::json!("enabled")),
            ("vendor_iwencai_key".to_string(), serde_json::json!("abc")),
        ];
        let s = load_enabled_vendors_from_template(&vars);
        assert!(s.contains("eastmoney"));
        assert!(s.contains("akshare"));
        assert!(!s.contains("tencent")); // 空字符串
        assert!(!s.contains("iwencai_key")); // iwencai_key 排除
    }

    #[test]
    fn each_style_has_required_vendors() {
        for s in [
            Style::Trend,
            Style::Value,
            Style::Capital,
            Style::Reversion,
            Style::Watchlist,
        ] {
            let v = required_vendors_for_style(s);
            assert!(!v.is_empty(), "{:?} missing vendors", s);
        }
    }
}
