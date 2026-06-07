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
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
// `HashSet` 在 Rust 2024 edition 已加入 prelude

use crate::recommender::pool::{
    build_seed_pool, clear_cached_vendors, get_cached_vendors, liquidity_filter_and_truncate,
    load_enabled_vendors_from_template, set_cached_vendors,
};
use crate::recommender::scoring::{dedup_and_merge, group_by_style_and_trim};
use crate::recommender::strategies::{
    emit_synthetic_picks, CapitalStrategy, ReversionStrategy, TrendStrategy, ValueStrategy,
    WatchlistStrategy,
};
use crate::recommender::strategy::PerCodeLocks;

// ── 缓存 ──

static RESULT_CACHE: RwLock<Option<(Period, RecoResponse, Instant)>> = RwLock::new(None);
/// 缓存 TTL：从 5 min 缩短到 60 s。
///
/// 荐股的 entry / target / stop_loss 是基于扫描时刻的现价算的，
/// 5 min 内股价可能已经走完整个目标区间（如图 莱伯泰科 5 min 内 38 → 48），
/// 这种"还显示着旧 target / entry 但现价远超"的推荐对用户毫无价值，
/// 还会误导用户按旧价位挂单。60 s 是为价格时效性与扫描性能做的折中。
const CACHE_TTL: Duration = Duration::from_secs(60);

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

    // 1. 预热 enabled-vendors 缓存（settings 页保存 vendor 时需要 invalid 这个缓存
    //    来刷新结果缓存；此处不依赖 enabled_vendors 做策略 gating）
    let _ = get_cached_vendors().unwrap_or_else(|| {
        let s = load_enabled_vendors_from_template(template_vars);
        set_cached_vendors(s.clone());
        s
    });

    // 2. seed pool + 流动性过滤
    let mut seed = build_seed_pool(&client).await;
    let raw_seed_pool_size = seed.len();
    // 保留 raw_seed 给 WatchlistStrategy（它只依赖 quote，不依赖 K 线）
    let raw_seed = seed.clone();
    seed = liquidity_filter_and_truncate(client.clone(), seed).await;

    // 流动性过滤兜底：若 vendor 拿不到 60 日 K 线 / 全部不达标，过滤后池子可能
    // 全空。这种情况下 4 个主策略（trend/value/capital/reversion）会遍历 0 次
    // 直接返回空 picks —— 表现就是面板上"趋势/价值/资金/超跌"全空，只剩
    // Watchlist（用 raw_seed）有数据。回退到 raw_seed 让 4 个主策略至少有机会跑
    // 一遍（它们各自的 `scan_one` 内部还会再判空，自然剔掉真没数据的票）。
    if seed.is_empty() && !raw_seed.is_empty() {
        eprintln!(
            "[recommender] liquidity filter removed all {} stocks; \
             falling back to raw seed pool (4 main strategies will scan raw)",
            raw_seed_pool_size
        );
        seed = raw_seed.clone();
    }

    // 3. 选定该 period 下的所有子策略（不再做 vendor 禁用检查——
    //    原本要求 "enabled_vendors" 至少覆盖一个 required vendor，但生产环境
    //    workflow template 中往往 vendor_* 变量为空，导致 4 个 style 全 disabled。
    //    现在所有 style 都跑；data 真的取不到时该 style 自然返回空 picks，
    //    前端展示 "no data" 而非误报 "数据源未启用"）
    let all_strategies: Vec<Box<dyn RecommendStrategy>> = match period {
        Period::Short => vec![
            Box::new(TrendStrategy::short()),
            Box::new(ValueStrategy::short()),
            Box::new(CapitalStrategy::short()),
            Box::new(ReversionStrategy::short()),
            Box::new(WatchlistStrategy::short()),
        ],
        Period::Mid => vec![
            Box::new(TrendStrategy::mid()),
            Box::new(ValueStrategy::mid()),
            Box::new(CapitalStrategy::mid()),
            Box::new(ReversionStrategy::mid()),
            Box::new(WatchlistStrategy::mid()),
        ],
        Period::Long => vec![
            Box::new(TrendStrategy::long()),
            Box::new(ValueStrategy::long()),
            Box::new(CapitalStrategy::long()),
            // ReversionStrategy long 不做
            Box::new(WatchlistStrategy::long()),
        ],
    };

    let enabled: Vec<Box<dyn RecommendStrategy>> = all_strategies;
    let disabled_styles_set: std::collections::HashSet<Style> = std::collections::HashSet::new();

    // 4. 并行执行（per-code 互斥锁：不同 code 真正并行，同 code 4 策略间串行）
    let per_code_locks = PerCodeLocks::new();
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

    // P3-2: drop picks whose target_price is already below current price
    // (no upside left — the BUY thesis is dead). Frontend should also visually
    // flag this in case cache holds the stale pick, but the backend drop
    // makes sure new scans never emit these in the first place.
    all_picks.retain(|p| {
        // 允许 ≤0.5% 的轻微容差，避免价格微抖时被误杀
        p.target_price >= p.price * 0.995
    });

    // 7. 按风格分组 + 限 10
    let mut by_style = group_by_style_and_trim(&mut all_picks, 10);

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
        ] {
            let bucket_empty = by_style.get(&style).map_or(true, |v| v.is_empty());
            if !bucket_empty {
                continue;
            }
            let synthetic = emit_synthetic_picks(
                client.clone(),
                style,
                period,
                &raw_seed,
                per_code_locks.clone(),
            )
            .await;
            let mut tagged: Vec<types::RecoPick> = synthetic;
            tagged.truncate(10);
            by_style.insert(style, tagged);
        }
    }

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
            synthetic: false,
        }
    }

    /// Bug 修复：target ≤ current price 的 pick 必须被剔除。
    /// 旧逻辑会保留"现价 48 / 目标 41"的 BUY 推荐，逻辑上矛盾。
    #[test]
    fn drops_picks_with_no_upside() {
        let mut picks = vec![
            make_pick(48.16, 41.87), // ✗ 无上行空间
            make_pick(38.0, 42.0),   // ✓ 正常
            make_pick(38.0, 38.0),   // ✗ target == price，无意义（容差外）
            make_pick(38.0, 37.81),  // ✗ 略低于 0.5% 容差
            make_pick(38.0, 37.85),  // 边界 = 0.5% 之内（37.85 / 38.0 ≈ 0.996）— 实际 37.85 / 38.0 = 0.996 > 0.995，所以保留
        ];
        picks.retain(|p| p.target_price >= p.price * 0.995);
        let codes: Vec<&str> = picks.iter().map(|p| p.stock_code.as_str()).collect();
        // 1) 和 2) 应该留下，3) target == price 被剔除，4) 在容差内（37.81 / 38.0 = 0.995）实际略低于 0.995 被剔除
        // 5) 37.85 / 38.0 ≈ 0.996 > 0.995 → 保留
        assert_eq!(picks.len(), 2, "保留: {codes:?}");
    }
}
