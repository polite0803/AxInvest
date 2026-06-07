//! 智能荐股 — 顶层模块
//!
//! ## 架构
//! - 12 个子策略（4 风格 × 3 周期）实现 [`strategy::RecommendStrategy`]
//! - [`recommend_stocks`] 是组合入口：seed pool → 候选过滤 → vendor 降级 → 并行扫描 → 去重 → 按风格分组
//! - 5 min 内存缓存（按 period 维度）
//!
//! ## 公开 API
//! - [`recommend_stocks`] — Tauri command 调用

pub mod indicators;
pub mod pool;
pub mod scoring;
pub mod strategies;
pub mod strategy;
pub mod types;

pub use strategy::{RecoContext, RecommendStrategy};
pub use types::{Period, RecoResponse, Style};

use axagent_astock_data::AStockClient;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::recommender::pool::{
    build_seed_pool, clear_cached_vendors, get_cached_vendors, liquidity_filter_and_truncate,
    load_enabled_vendors_from_template, set_cached_vendors, vendors_satisfied,
};
use crate::recommender::scoring::{dedup_and_merge, group_by_style_and_trim};
use crate::recommender::strategies::{
    CapitalStrategy, ReversionStrategy, TrendStrategy, ValueStrategy,
};
use crate::recommender::strategy::PerCodeLocks;

// ── 缓存 ──

static RESULT_CACHE: RwLock<Option<(Period, RecoResponse, Instant)>> = RwLock::new(None);
const CACHE_TTL: Duration = Duration::from_secs(300);

fn cache_get(period: Period) -> Option<RecoResponse> {
    let g = RESULT_CACHE.read().ok()?;
    if let Some((p, resp, ts)) = g.as_ref() {
        if *p == period && ts.elapsed() < CACHE_TTL {
            return Some(resp.clone());
        }
    }
    None
}

fn cache_put(period: Period, resp: RecoResponse) {
    if let Ok(mut g) = RESULT_CACHE.write() {
        *g = Some((period, resp, Instant::now()));
    }
}

/// 主动失效缓存（设置页保存 vendor 后调用）
///
/// 同时清 vendor 启用集合缓存 + 推荐结果缓存，保证下次调用立即反映新 vendor 状态
pub fn invalidate_cache() {
    if let Ok(mut g) = RESULT_CACHE.write() {
        *g = None;
    }
    clear_cached_vendors();
}

// ── 公开入口 ──

/// 拉取指定周期的荐股结果
///
/// 调用方应保证传入的 `template_vars` 是当前 workflow template 的变量列表（vendor_* 启用状态）
///
/// `client` 用 `Arc` 包一层，因为后续并行任务（liquidity filter + 4 子策略）需要共享所有权
pub async fn recommend_stocks(
    client: Arc<AStockClient>,
    period: Period,
    template_vars: &[(String, serde_json::Value)],
) -> Result<RecoResponse, String> {
    if let Some(cached) = cache_get(period) {
        return Ok(cached);
    }

    // 1. 加载 enabled vendor 集合
    let enabled_vendors: HashSet<String> = match get_cached_vendors() {
        Some(s) => s,
        None => {
            let s = load_enabled_vendors_from_template(template_vars);
            set_cached_vendors(s.clone());
            s
        },
    };

    // 2. seed pool + 流动性过滤
    let mut seed = build_seed_pool(&client).await;
    let raw_seed_pool_size = seed.len();
    seed = liquidity_filter_and_truncate(client.clone(), seed).await;

    // 3. 选定该 period 下的所有子策略 + vendor 降级
    let all_strategies: Vec<Box<dyn RecommendStrategy>> = match period {
        Period::Short => vec![
            Box::new(TrendStrategy::short()),
            Box::new(ValueStrategy::short()),
            Box::new(CapitalStrategy::short()),
            Box::new(ReversionStrategy::short()),
        ],
        Period::Mid => vec![
            Box::new(TrendStrategy::mid()),
            Box::new(ValueStrategy::mid()),
            Box::new(CapitalStrategy::mid()),
            Box::new(ReversionStrategy::mid()),
        ],
        Period::Long => vec![
            Box::new(TrendStrategy::long()),
            Box::new(ValueStrategy::long()),
            Box::new(CapitalStrategy::long()),
            // ReversionStrategy long 不做
        ],
    };

    let mut enabled: Vec<Box<dyn RecommendStrategy>> = Vec::new();
    let mut disabled_styles_set: std::collections::HashSet<Style> =
        std::collections::HashSet::new();
    for s in all_strategies {
        let reqs = s.required_vendors();
        if vendors_satisfied(reqs, &enabled_vendors) {
            enabled.push(s);
        } else {
            disabled_styles_set.insert(s.style());
        }
    }

    // 4. 并行执行（per-code 互斥锁：不同 code 真正并行，同 code 4 策略间串行）
    let per_code_locks = PerCodeLocks::new();
    let ctx_pool = seed.clone();
    let mut futures = Vec::new();
    for s in enabled.iter() {
        let s_ref: &dyn RecommendStrategy = s.as_ref();
        let client_ref: Arc<AStockClient> = client.clone();
        let lock_ref = per_code_locks.clone();
        let seed_ref = ctx_pool.clone();
        let period_val = period;
        let fut = async move {
            let ctx = RecoContext {
                client: &client_ref,
                seed: &seed_ref,
                per_code_locks: lock_ref,
                period: period_val,
            };
            s_ref.scan(&ctx).await
        };
        futures.push(fut);
    }
    let results: Vec<Result<Vec<types::RecoPick>, String>> =
        futures::future::join_all(futures).await;

    // 5. 合并
    let mut all_picks: Vec<types::RecoPick> = Vec::new();
    for r in results {
        if let Ok(mut picks) = r {
            all_picks.append(&mut picks);
        }
    }

    // 6. 去重
    dedup_and_merge(&mut all_picks);

    // P3-1: drop picks whose numeric fields are NaN/inf — these would render as "NaN" in JSON.
    all_picks.retain(|p| {
        !p.price.is_nan()
            && !p.entry_low.is_nan()
            && !p.entry_high.is_nan()
            && !p.stop_loss.is_nan()
            && !p.target_price.is_nan()
            && !p.position_pct.is_nan()
    });

    // 7. 按风格分组 + 限 10
    let by_style = group_by_style_and_trim(&mut all_picks, 10);

    let resp = RecoResponse {
        period,
        picks: by_style,
        disabled_styles: disabled_styles_set.into_iter().collect(),
        generated_at: chrono::Utc::now().timestamp_millis(),
        raw_seed_pool_size,
    };
    cache_put(period, resp.clone());
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommender::types::{Period, Style};

    #[test]
    fn cache_invalidate_works() {
        invalidate_cache();
        // 重建缓存
        let resp = RecoResponse {
            period: Period::Short,
            picks: std::collections::HashMap::new(),
            disabled_styles: vec![Style::Capital],
            generated_at: 0,
            raw_seed_pool_size: 0,
        };
        cache_put(Period::Short, resp);
        assert!(cache_get(Period::Short).is_some());
        invalidate_cache();
        assert!(cache_get(Period::Short).is_none());
    }
}
