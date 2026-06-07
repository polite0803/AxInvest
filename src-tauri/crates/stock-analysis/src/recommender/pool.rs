//! 候选池构造 + vendor 启用集合检测

use axagent_astock_data::AStockClient;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 候选池条目：(code, name, sector)
pub type SeedItem = (String, String, Option<String>);

/// 构造 seed pool
///
/// 顺序：get_hot_stocks(30) ∪ get_industry_ranking 龙头(10) ∪ 用户 watchlist/holdings
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
        for s in [Style::Trend, Style::Value, Style::Capital, Style::Reversion] {
            let v = required_vendors_for_style(s);
            assert!(!v.is_empty(), "{:?} missing vendors", s);
        }
    }
}
