//! 金融分析工具 — 29 个：风险模型、信号、数据清洗、技术指标、蒙特卡洛等。
//! 所有计算逻辑内联，不依赖 axagent-stock-analysis（避免循环依赖）。
//!
//! 参数消费：所有可调参数从 `input["_template_vars"]` 读取（tool_executor.rs 自动注入），
//! 缺失时回退到默认行为，保持向后兼容。

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult, global_state};
use async_trait::async_trait;
use axagent_analysis_engine::risk::{
    kelly_criterion_with_thresholds, pe_percentile, peg_ratio as engine_peg_ratio,
    value_at_risk as engine_value_at_risk,
};
use axagent_astock_data::indicators::sma;
use serde::Serialize;
use serde_json::{Value, json};

fn parse_f64s(val: &Value, key: &str) -> Vec<f64> {
    match val.get(key) {
        // P2-7 修复: 同时支持 JSON 字符串数组和原生数组
        Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_f64()).collect(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// 从 tool input 中提取 _template_vars 中指定 key 的 f64 值，取不到则返回默认值。
/// 模板变量在 tool_executor.rs 构建 `resolved_args` 时自动注入。
fn tv_f64(input: &Value, key: &str, default: f64) -> f64 {
    input
        .get("_template_vars")
        .and_then(|tv| tv.get(key))
        .and_then(|v| v.as_f64())
        .unwrap_or(default)
}
fn tv_i64(input: &Value, key: &str, default: i64) -> i64 {
    input
        .get("_template_vars")
        .and_then(|tv| tv.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
}
fn tv_str<'a>(input: &'a Value, key: &str, default: &'a str) -> &'a str {
    input
        .get("_template_vars")
        .and_then(|tv| tv.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or(default)
}

// ═══════════ 内联数学函数 ═══════════

fn max_drawdown(prices: &[f64]) -> f64 {
    if prices.is_empty() {
        return 0.0;
    }
    let mut peak = prices[0];
    if peak <= 0.0 {
        peak = f64::MAX;
    }
    let mut max_dd = 0.0;
    for &p in prices {
        if p > peak {
            peak = p;
        }
        if peak > 0.0 {
            let dd = (peak - p) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

#[derive(Serialize)]
struct SharpeR {
    sharpe: f64,
    annualized: f64,
    mean_return: f64,
    stddev: f64,
}
fn sharpe_ratio(returns: &[f64], rf: f64, annualization: f64) -> SharpeR {
    // P3-C8: 委托 harness 统一实现（样本方差 n-1，避免重复算法分叉）。
    // 保留 SharpeR 名称和 round 行为以稳定历史序列化输出。
    let c = axagent_harness::indicators::sharpe_components(returns, rf, annualization);
    SharpeR {
        sharpe: (c.sharpe * 1000.0).round() / 1000.0,
        annualized: (c.annualized * 1000.0).round() / 1000.0,
        mean_return: (c.mean_return * 10000.0).round() / 100.0,
        stddev: (c.stddev * 10000.0).round() / 100.0,
    }
}

// 2026-09-21 去重：`value_at_risk` **不再有本地实现** —— 统一复用
// `axagent_analysis_engine::risk::value_at_risk`（权威源），照本文件
// `pe_percentile` / `kelly` 先例。
//
// ⚠ 与前两个不同，这次是**先改权威源、再转发** —— 因为两份的算法**不等价**（不是等价副本）：
//   · 本地（生产在跑，`calc_var` 在消费）：`idx = floor((1-conf) * n)`，直接作 0-based 下标
//   · 引擎（当时**全仓零调用**）：`idx = floor((1-conf) * (n+1))`，再 `var_idx = idx - 1`
//   ⇒ 净效果引擎版取**前一位**顺序统计量（更极端的尾部值）。
//   实测 1980 组参数扫描：索引不同 **45.5%**、最终 2 位小数输出不同 **9.44%**、
//   最大 Δ **0.032**（≈6.5 倍舍入半格），样例甚至**符号翻转**
//   （证据：`output/tmp/verify-var-divergence.mjs`）。
//
//   口径裁决：**以本地为准**。`floor((1-c)*n)` 数学上恰好命中 (1-c) 分位；引擎版 `+1` 再
//   `-1` 是偏移叠加后的混合口径（净取前一位 ⇒ 更极端的尾部值），不对应任何标准分位定义。
//   引擎版零调用 ⇒ 改它零行为风险；反过来改本地会改动所有历史产物且无正确性依据
//   ⇒ 改引擎版对齐、此处转发。引擎侧的口径锁在 `risk.rs::tests::test_var`。
//
//   ⚠ 两口径**并非处处可分**：`1.0-c` 的浮点值可能**略小于**数学值
//   （`1.0-0.9 = 0.09999999999999998`）⇒ 「整十置信度 × n=10」下 `floor` 掉一档，
//   恰好抵消旧口径的 `+1`，两口径输出**相同**（实测 c=0.9 时新旧均得 0.05）。
//   选测试点必须避开这类组合 —— 否则拿到的是「区分力为 0 的假绿」。
//   详见 `analysis-engine/src/risk.rs::value_at_risk` doc 的「浮点 floor 边界」段。
//
// 契约等价性：引擎 `VarResult { var_pct, confidence, cvar_pct }` 与本地 `VarR`
// **逐字段同名同型同序** ⇒ 序列化 JSON 零变化（LLM 可见契约不动），故本地结构体一并删除。

// ⚠ 转发类改造的等价性判据 = **逐行比对算法本体**，不是「字段同名同型」——
// 后者只证明 JSON 结构不变，**挡不住阈值/档位数被偷换**（结构不变、值域已变的缺陷
// 编译器与类型检查都不会报，见 `risk.rs` 同名函数的档位表）。
//
// 2026-09-21 去重：`pe_percentile` **不再有本地实现** —— 统一复用
// `axagent_analysis_engine::risk::pe_percentile`（权威源），照本文件 `kelly` 先例。
//
// 逐行等价性已证（证据：本地原实现快照 `output/backup-2026-09-13/dup-fix/finance.rs.before-p0-3`
// 第 114–142 行，与引擎实现逐项比对）：
//   排序口径 `partial_cmp/Ordering::Equal` · `below` 的 `<=` · 空序列 `pct = 50.0` ·
//   五档阈值 `20/40/60/80` → 极低/偏低/合理/极高（**档位数与阈值均一致**）·
//   `median = s[len/2]`（偶数取上中位、不插值）· 舍入 `*10.0/10.0` ·
//   字段 `{ percentile, level, median }` 同名同型。
// ⇒ `calc_pe_percentile` 输出 JSON **结构与值域皆零变化**（LLM 可见契约不动），
// 故本地 `PeR` 结构体一并删除。
//
// 背景：去重前两处副本在 2026-09-21 同日各修过一次（亏损企业 PE<0 曾落进
// `percentile = 0` ⇒ `level = "极低"`，把「亏损」读成「历史估值极低分位」），
// 等价性靠人工比对维持；13 日 `dup-fix` 一轮也只修了单侧。收敛到单一权威源后不再有分叉面。

/// 序列化视图：**只暴露 `peg` / `level`** —— 这是 `calc_peg` 工具的既有输出契约。
///
/// 权威源 `PEGResult` 另带 `pe` / `growth_rate` 两个回显字段；若直接序列化它，
/// 工具输出会凭空多出两个键 ⇒ **改 LLM 可见契约**，故此处保留字段子集适配
/// （刻意不做整删）。判据：**字段同集可整删本地类型，字段超集必须留适配层**。
#[derive(Serialize)]
struct PegR {
    peg: f64,
    level: String,
}

/// 转发到权威源 `axagent_analysis_engine::risk::peg_ratio`，仅取 `peg` / `level`。
///
/// 逐行等价性已证（证据：同上快照第 144–167 行）：`g <= 0 ⇒ {INFINITY, "无意义"}` 守卫、
/// `pe / g` 算式、四档阈值 `0.5/1.0/2.0` → 严重低估/低估/合理/高估、舍入 `*100.0/100.0`
/// 全部一致；两侧唯一差异是权威源多两个回显字段，已由 `PegR` 适配掉。
///
/// 2026-09-21 去重：本地原实现重复了上述四档与 `g <= 0` 守卫，且同日为亏损企业
/// （PE<0 ⇒ 负 peg 落进 `peg < 0.5` ⇒ "严重低估"）单独修过一次 ——
/// 两份副本的等价性靠人工比对维持。收敛到单一权威源后，
/// 该守卫（`pe <= 0.0 ⇒ 无含义`）只在引擎侧维护一处。
/// ⚠ 亏损分支返回 `INFINITY` ⇒ serde_json 序列化为 `null`（JSON 无 Infinity），
///   即工具输出是 `{"peg": null, "level": "无意义"}`，非负值 —— 这是刻意的。
fn peg_ratio(pe: f64, g: f64) -> PegR {
    let r = engine_peg_ratio(pe, g);
    PegR { peg: r.peg, level: r.level }
}

// 凯利公式不再本地实现：统一复用 `axagent_analysis_engine::risk::kelly_criterion_with_thresholds`
// （权威源）。
// 历史两份实现逐行等价（证据：同上快照第 169–203 行）：守卫条件
// `al<=0 || aw<=0 || wr<=0`、`odds = aw/al`、`k = ((wr*(odds+1)-1)/odds).max(0.0)`、
// 三处舍入口径 `1000/1000/10000`、signal 四档 重仓/中等/轻仓/不建议
// ⇒ 本次去重为零行为变更的纯转发。

#[derive(Serialize)]
struct RpR {
    weights: Vec<f64>,
    divers_ratio: f64,
}
fn risk_parity(vols: &[f64], corr_json: &str) -> RpR {
    let n = vols.len();
    if n == 0 {
        return RpR { weights: vec![], divers_ratio: 0.0 };
    }
    let corr_matrix: Option<Vec<Vec<f64>>> = serde_json::from_str(corr_json)
        .ok()
        .filter(|m: &Vec<Vec<f64>>| m.len() == n && m.iter().all(|r| r.len() == n));
    let inv: Vec<f64> = vols.iter().map(|&v| if v > 0.0 { 1.0 / v } else { 0.0 }).collect();
    let total: f64 = inv.iter().sum();
    let w = if let Some(corr) = corr_matrix {
        let mut w: Vec<f64> = inv.clone();
        let w_sum: f64 = w.iter().sum();
        if w_sum > 0.0 {
            for wi in w.iter_mut() {
                *wi /= w_sum;
            }
        }
        for _ in 0..20 {
            let mut risk_contrib = vec![0.0; n];
            for i in 0..n {
                for j in 0..n {
                    risk_contrib[i] += w[i] * w[j] * vols[i] * vols[j] * corr[i][j];
                }
            }
            let total_risk: f64 = risk_contrib.iter().sum();
            if total_risk <= 0.0 {
                break;
            }
            let target = total_risk / n as f64;
            for i in 0..n {
                if risk_contrib[i] > 0.0 {
                    w[i] *= (target / risk_contrib[i]).sqrt().clamp(0.5, 2.0);
                }
            }
            let ws: f64 = w.iter().sum();
            if ws > 0.0 {
                for wi in w.iter_mut() {
                    *wi /= ws;
                }
            }
        }
        w.iter().map(|&x| (x * 10000.0).round() / 10000.0).collect()
    } else if total > 0.0 {
        inv.iter().map(|&x| (x / total * 10000.0).round() / 10000.0).collect()
    } else {
        vec![1.0 / n as f64; n]
    };
    let hhi: f64 = w.iter().map(|x| x * x).sum();
    RpR {
        weights: w,
        divers_ratio: if hhi > 0.0 {
            ((1.0 / (hhi * n as f64)).min(1.0) * 100.0).round() / 100.0
        } else {
            1.0
        },
    }
}

#[derive(Serialize)]
struct CrossR {
    signal: String,
    fast_ma: f64,
    slow_ma: f64,
    latest_price: f64,
    confirmation: String,
}
fn detect_ma_cross(kj: &str, fast: usize, slow: usize) -> CrossR {
    #[derive(serde::Deserialize)]
    struct R {
        close: f64,
    }
    let kl: Vec<R> = serde_json::from_str(kj).unwrap_or_default();
    if kl.len() < slow + 1 {
        return CrossR {
            signal: "none".into(),
            fast_ma: 0.0,
            slow_ma: 0.0,
            latest_price: 0.0,
            confirmation: "n/a".into(),
        };
    }
    let closes: Vec<f64> = kl.iter().map(|k| k.close).collect();
    let n = closes.len();
    let cf = sma(&closes[..n], fast).unwrap_or(0.0);
    let cs = sma(&closes[..n], slow).unwrap_or(0.0);
    let pf = sma(&closes[..n - 1], fast).unwrap_or(cf);
    let ps = sma(&closes[..n - 1], slow).unwrap_or(cs);
    let sig = if pf <= ps && cf > cs {
        "golden_cross"
    } else if pf >= ps && cf < cs {
        "death_cross"
    } else {
        "none"
    };
    let confirmation = if sig != "none" && closes.len() >= slow + 2 {
        let p2f = sma(&closes[..n - 2], fast).unwrap_or(cf);
        let p2s = sma(&closes[..n - 2], slow).unwrap_or(cs);
        if (sig == "golden_cross" && p2f > p2s) || (sig == "death_cross" && p2f < p2s) {
            "confirmed"
        } else {
            "unconfirmed"
        }
    } else if sig != "none" {
        "unconfirmed"
    } else {
        "n/a"
    };
    CrossR {
        signal: sig.into(),
        fast_ma: (cf * 100.0).round() / 100.0,
        slow_ma: (cs * 100.0).round() / 100.0,
        latest_price: kl.last().map(|k| k.close).unwrap_or(0.0),
        confirmation: confirmation.into(),
    }
}

#[derive(Serialize)]
struct BrkR {
    breakout_type: String,
    current_price: f64,
    confidence: String,
    volume_confirmation: bool,
}
fn detect_breakout(kj: &str, sup: f64, res: f64, vol_confirm_th: f64) -> BrkR {
    #[derive(serde::Deserialize)]
    struct R {
        close: f64,
        volume: f64,
    }
    let kl: Vec<R> = serde_json::from_str(kj).unwrap_or_default();
    if kl.is_empty() {
        return BrkR {
            breakout_type: "none".into(),
            current_price: 0.0,
            confidence: "low".into(),
            volume_confirmation: false,
        };
    }
    let last = kl.last().unwrap();
    let price = last.close;
    let avg_v = if kl.len() >= 5 {
        kl[kl.len() - 6..kl.len() - 1].iter().map(|k| k.volume).sum::<f64>() / 5.0
    } else {
        kl.iter().map(|k| k.volume).sum::<f64>() / kl.len() as f64
    };
    let vr = if avg_v > 0.0 {
        Some(last.volume / avg_v)
    } else {
        None
    };
    let (bt, conf) = if price > res {
        let c = if vr.unwrap_or(1.0) > vol_confirm_th {
            "high"
        } else {
            "medium"
        };
        ("resistance_break", c)
    } else if price < sup {
        let c = if vr.unwrap_or(1.0) > vol_confirm_th {
            "high"
        } else {
            "medium"
        };
        ("support_break", c)
    } else {
        ("none", "low")
    };
    BrkR {
        breakout_type: bt.into(),
        current_price: price,
        confidence: conf.into(),
        volume_confirmation: vr.unwrap_or(1.0) > vol_confirm_th,
    }
}

#[derive(Serialize)]
struct OutR {
    cleaned: Vec<f64>,
    removed_count: usize,
}
fn remove_outliers(pj: &str, method: &str, th: f64) -> OutR {
    let prices: Vec<f64> = serde_json::from_str(pj).unwrap_or_default();
    if prices.len() < 4 {
        return OutR { cleaned: prices, removed_count: 0 };
    }
    if method == "iqr" {
        let mut s = prices.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q1 = s[(s.len() as f64 * 0.25).floor() as usize];
        let q3 = s[(s.len() as f64 * 0.75).floor() as usize];
        let iqr = q3 - q1;
        if iqr < 1e-10 {
            return OutR { cleaned: prices, removed_count: 0 };
        }
        let (lo, hi) = (q1 - th * iqr, q3 + th * iqr);
        let mut cleaned = Vec::with_capacity(prices.len());
        let mut rm = 0usize;
        for &p in &prices {
            if p < lo {
                cleaned.push((lo * 100.0).round() / 100.0);
                rm += 1;
            } else if p > hi {
                cleaned.push((hi * 100.0).round() / 100.0);
                rm += 1;
            } else {
                cleaned.push(p);
            }
        }
        OutR { cleaned, removed_count: rm }
    } else {
        let n = prices.len();
        let m = prices.iter().sum::<f64>() / n as f64;
        let std = (prices.iter().map(|p| (p - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt();
        if std < 1e-10 {
            return OutR { cleaned: prices, removed_count: 0 };
        }
        let mut cleaned = Vec::with_capacity(prices.len());
        let mut rm = 0usize;
        for &p in &prices {
            let z = (p - m).abs() / std;
            if z > th {
                let clamped = if p > m { m + th * std } else { m - th * std };
                cleaned.push((clamped * 100.0).round() / 100.0);
                rm += 1;
            } else {
                cleaned.push(p);
            }
        }
        OutR { cleaned, removed_count: rm }
    }
}

#[derive(Serialize)]
struct FillR {
    filled: Vec<Option<f64>>,
    filled_count: usize,
}
fn fill_missing(pj: &str, method: &str) -> FillR {
    let prices: Vec<Option<f64>> = serde_json::from_str(pj).unwrap_or_default();
    if prices.is_empty() {
        return FillR { filled: vec![], filled_count: 0 };
    }
    if method == "linear" {
        let mut r = prices.clone();
        let mut cnt = 0usize;
        let n = r.len();
        let first = r.iter().position(|v| v.is_some());
        if first.is_none() {
            return FillR { filled: r, filled_count: 0 };
        }
        let f = first.unwrap();
        let hv = r[f].unwrap();
        for v in r.iter_mut().take(f) {
            *v = Some(hv);
            cnt += 1;
        }
        let mut i = f;
        while i < n {
            if r[i].is_some() {
                i += 1;
                continue;
            }
            let gs = i;
            while i < n && r[i].is_none() {
                i += 1;
            }
            let ge = i;
            if ge < n {
                let (l, ri) = (r[gs - 1].unwrap(), r[ge].unwrap());
                let steps = (ge - gs + 1) as f64;
                for (j, v) in r.iter_mut().enumerate().take(ge).skip(gs) {
                    *v = Some(l + (ri - l) * (j - gs + 1) as f64 / steps);
                    cnt += 1;
                }
            } else {
                let tv = r[gs - 1].unwrap();
                for v in r.iter_mut().skip(gs) {
                    *v = Some(tv);
                    cnt += 1;
                }
            }
        }
        FillR { filled: r, filled_count: cnt }
    } else {
        let mut r = prices.clone();
        let mut last: Option<f64> = None;
        let mut cnt = 0usize;
        for v in r.iter_mut() {
            if let Some(val) = v {
                last = Some(*val);
            } else if let Some(fill) = last {
                *v = Some(fill);
                cnt += 1;
            }
        }
        FillR { filled: r, filled_count: cnt }
    }
}

#[derive(Serialize)]
struct AdjKLine {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}
#[derive(Serialize)]
struct AdjR {
    adjusted_klines: Vec<AdjKLine>,
    adjustment_factor: f64,
}
fn adjust_prices(kj: &str, dj: &str) -> AdjR {
    #[derive(serde::Deserialize)]
    struct K {
        date: String,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        #[serde(default)]
        volume: f64,
    }
    #[derive(serde::Deserialize)]
    struct D {
        date: String,
        cash_dividend: f64,
        share_dividend: f64,
    }
    let mut kl: Vec<K> = serde_json::from_str(kj).unwrap_or_default();
    let div: Vec<D> = serde_json::from_str(dj).unwrap_or_default();
    if kl.is_empty() {
        return AdjR { adjusted_klines: vec![], adjustment_factor: 1.0 };
    }
    kl.sort_by(|a, b| b.date.cmp(&a.date));
    let mut factor = 1.0;
    for k in kl.iter_mut() {
        for d in &div {
            if d.date == k.date {
                let tr = d.cash_dividend / k.close + d.share_dividend;
                if tr > 0.0 {
                    factor /= 1.0 + tr;
                }
            }
        }
        k.open = (k.open * factor * 100.0).round() / 100.0;
        k.close = (k.close * factor * 100.0).round() / 100.0;
        k.high = (k.high * factor * 100.0).round() / 100.0;
        k.low = (k.low * factor * 100.0).round() / 100.0;
        k.volume = (k.volume / factor * 100.0).round() / 100.0;
    }
    AdjR {
        adjusted_klines: kl
            .into_iter()
            .map(|k| AdjKLine {
                date: k.date,
                open: k.open,
                high: k.high,
                low: k.low,
                close: k.close,
                volume: k.volume,
            })
            .collect(),
        adjustment_factor: (factor * 10000.0).round() / 10000.0,
    }
}

// ── 技术指标 ──

fn compute_atr(args: &Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    struct R {
        high: f64,
        low: f64,
        close: f64,
    }
    let kl: Vec<R> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let period = args.get("period").and_then(|v| v.as_u64()).unwrap_or(tv_i64(
        args,
        "atr_period",
        14,
    ) as u64) as usize;
    let n = kl.len();
    if n < 2 || period == 0 {
        return Ok(json!({"atr": 0.0, "period": period}));
    }
    let mut trs = vec![0.0; n - 1];
    for i in 1..n {
        let (p, c) = (&kl[i - 1], &kl[i]);
        trs[i - 1] = (c.high - c.low).max((c.high - p.close).abs()).max((c.low - p.close).abs());
    }
    let atr = if trs.len() <= period {
        trs.iter().sum::<f64>() / trs.len() as f64
    } else {
        let mut a = trs[..period].iter().sum::<f64>() / period as f64;
        for &t in &trs[period..] {
            a = (a * (period - 1) as f64 + t) / period as f64;
        }
        a
    };
    Ok(json!({"atr": (atr * 100.0).round() / 100.0, "period": period}))
}

fn compute_kdj(args: &Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    struct R {
        high: f64,
        low: f64,
        close: f64,
    }
    let kl: Vec<R> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let n =
        args.get("n").and_then(|v| v.as_u64()).unwrap_or(tv_i64(args, "kdj_n", 9) as u64) as usize;
    if kl.len() < n {
        return Ok(json!({"k": 50.0, "d": 50.0, "j": 50.0, "signal": "中性"}));
    }
    let (mut k, mut d) = (50.0, 50.0);
    for i in (n - 1)..kl.len() {
        let w = &kl[i + 1 - n..=i];
        let lo = w.iter().map(|x| x.low).fold(f64::MAX, f64::min);
        let hi = w.iter().map(|x| x.high).fold(f64::MIN, f64::max);
        let rsv = if (hi - lo).abs() > 1e-10 {
            (w.last().unwrap().close - lo) / (hi - lo) * 100.0
        } else {
            50.0
        };
        k = 2.0 / 3.0 * k + 1.0 / 3.0 * rsv;
        d = 2.0 / 3.0 * d + 1.0 / 3.0 * k;
    }
    let j = 3.0 * k - 2.0 * d;
    let sig = if j > 100.0 {
        "严重超买"
    } else if j > 80.0 {
        "超买"
    } else if j < 0.0 {
        "严重超卖"
    } else if j < 20.0 {
        "超卖"
    } else if k > d {
        "多头"
    } else {
        "空头"
    };
    Ok(
        json!({"k": (k * 100.0).round() / 100.0, "d": (d * 100.0).round() / 100.0, "j": (j * 100.0).round() / 100.0, "signal": sig}),
    )
}

fn compute_obv(args: &Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    struct R {
        close: f64,
        volume: f64,
    }
    let kl: Vec<R> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if kl.is_empty() {
        return Ok(json!({"obv": 0.0}));
    }
    let mut obv = 0.0;
    for i in 1..kl.len() {
        if kl[i].close > kl[i - 1].close {
            obv += kl[i].volume;
        } else if kl[i].close < kl[i - 1].close {
            obv -= kl[i].volume;
        }
    }
    Ok(json!({"obv": (obv / 1e8 * 100.0).round() / 100.0, "unit": "亿"}))
}

fn calc_beta(args: &Value) -> Result<Value, String> {
    let s = parse_f64s(args, "stock_returns_json");
    let m = parse_f64s(args, "market_returns_json");
    let n = s.len().min(m.len());
    if n < 2 {
        return Ok(json!({"beta": 1.0}));
    }
    let (ms, mm) = (s[..n].iter().sum::<f64>() / n as f64, m[..n].iter().sum::<f64>() / n as f64);
    let cov = s[..n].iter().zip(m[..n].iter()).map(|(&a, &b)| (a - ms) * (b - mm)).sum::<f64>()
        / (n - 1) as f64;
    let vm = m[..n].iter().map(|&x| (x - mm).powi(2)).sum::<f64>() / (n - 1) as f64;
    Ok(json!({"beta": if vm > 1e-10 { (cov / vm * 1000.0).round() / 1000.0 } else { 1.0 }}))
}

// ── P2/P3 ──

fn detect_earnings(args: &Value) -> Result<Value, String> {
    let a = args.get("actual_eps").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let c = args.get("consensus_eps").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if c.abs() < 1e-10 {
        return Ok(json!({"surprise_pct": 0.0, "level": "无预期"}));
    }
    // 2026-09-19（A2）：估算值**不得**用作「超预期」判定的基准。
    //   背景：`astock-data` 的 C-fallback 在 vendor 全失败时按挂牌板块给一个常数
    //   EPS（科创/创业 0.40、北交所 0.25、其余 0.55），并标记 `is_estimated = true`。
    //   那个数在原数据里与真实一致预期**数值上不可区分**，但拿它算
    //   `(actual - consensus) / consensus` 得到的是一个**假基准的相对量** ——
    //   比如常数 0.55 遇上真实 EPS 0.30，会报出「大幅低于预期」，而实际预期可能远低于 0.30。
    //   这里返回显式不可用（`level` 说明原因），而不是给一个看起来正常的档位。
    let eps_estimated =
        args.get("consensus_eps_is_estimated").and_then(|v| v.as_bool()).unwrap_or(false);
    if eps_estimated {
        return Ok(json!({
            "surprise_pct": null,
            "level": "预期基准不可靠",
            "actual_eps": a,
            "consensus_eps": c,
            "reason": "consensus_eps 为板块常数估算值（is_estimated=true），不得据此判定超预期/不及预期",
        }));
    }
    let s = (a - c) / c.abs() * 100.0;
    // 业绩超预期分级阈值（用户可在设置面板中调整）
    let th_huge = tv_f64(args, "earnings_th_huge_pos", 50.0);
    let th_strong = tv_f64(args, "earnings_th_strong_pos", 20.0);
    let th_mild = tv_f64(args, "earnings_th_mild_pos", 5.0);
    let th_mild_neg = tv_f64(args, "earnings_th_mild_neg", -5.0);
    let th_strong_neg = tv_f64(args, "earnings_th_strong_neg", -20.0);
    let th_huge_neg = tv_f64(args, "earnings_th_huge_neg", -50.0);
    let l = if s > th_huge {
        "大幅超预期"
    } else if s > th_strong {
        "超预期"
    } else if s > th_mild {
        "略超预期"
    } else if s > th_mild_neg {
        "符合预期"
    } else if s > th_strong_neg {
        "略低于预期"
    } else if s > th_huge_neg {
        "低于预期"
    } else {
        "大幅低于预期"
    };
    Ok(
        json!({"surprise_pct": (s * 100.0).round() / 100.0, "level": l, "actual_eps": a, "consensus_eps": c}),
    )
}

fn detect_pledge(args: &Value) -> Result<Value, String> {
    let p = args.get("pledge_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // 质押风险阈值（用户可在设置面板中调整）
    let w = args.get("warning_line").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        args,
        "pledge_warning_line",
        50.0,
    ));
    let lq = args.get("liquidation_line").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        args,
        "pledge_liquidation_line",
        70.0,
    ));
    let med = tv_f64(args, "pledge_medium_line", 30.0);
    let low = tv_f64(args, "pledge_low_line", 10.0);
    let (r, wa) = if p >= lq {
        ("极高风险", "大股东质押濒临平仓线")
    } else if p >= w {
        ("高风险", "质押比例超过预警线")
    } else if p >= med {
        ("中风险", "质押比例偏高")
    } else if p > low {
        ("低风险", "质押比例正常")
    } else {
        ("安全", "质押比例低")
    };
    Ok(
        json!({"pledge_pct": p, "risk_level": r, "warning": wa, "distance_to_warning": ((p / w - 1.0) * 10000.0).round() / 100.0}),
    )
}

fn calc_corr_matrix(args: &Value) -> Result<Value, String> {
    let m: Vec<Vec<f64>> = args
        .get("returns_matrix_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let n = m.len();
    if n < 2 {
        return Ok(json!({"avg_correlation": 1.0, "asset_count": n}));
    }
    let k = m[0].len();
    if k < 2 {
        return Ok(json!({"avg_correlation": 0.0, "asset_count": n}));
    }
    let means: Vec<f64> = m.iter().map(|r| r.iter().sum::<f64>() / k as f64).collect();
    let stds: Vec<f64> = m
        .iter()
        .map(|r| {
            let mean = r.iter().sum::<f64>() / k as f64;
            (r.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (k - 1) as f64).sqrt()
        })
        .collect();
    let (mut total, mut cnt) = (0.0, 0u32);
    for i in 0..n {
        for j in i + 1..n {
            let cov = m[i]
                .iter()
                .zip(m[j].iter())
                .map(|(&a, &b)| (a - means[i]) * (b - means[j]))
                .sum::<f64>()
                / (k - 1) as f64;
            let r = if stds[i] > 1e-10 && stds[j] > 1e-10 {
                cov / (stds[i] * stds[j])
            } else {
                0.0
            };
            total += r;
            cnt += 1;
        }
    }
    let avg = if cnt > 0 {
        (total / cnt as f64 * 1000.0).round() / 1000.0
    } else {
        0.0
    };
    Ok(json!({"avg_correlation": avg, "asset_count": n}))
}

fn xorshift128plus(s0: &mut u64, s1: &mut u64) -> u64 {
    let result = s0.wrapping_add(*s1);
    *s1 ^= *s0;
    *s0 = s0.rotate_left(24) ^ *s1 ^ (*s1 << 16);
    *s1 = s1.rotate_left(37);
    result
}

fn normal_approx(s0: &mut u64, s1: &mut u64) -> f64 {
    let u1 = (xorshift128plus(s0, s1) >> 11) as f64 / (1u64 << 53) as f64;
    let u2 = (xorshift128plus(s0, s1) >> 11) as f64 / (1u64 << 53) as f64;
    let u1 = u1.max(1e-15);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn monte_carlo(args: &Value) -> Result<Value, String> {
    // 蒙特卡洛参数（用户可在设置面板中调整默认值）
    let price = args.get("current_price").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        args,
        "mc_default_price",
        10.0,
    ));
    let ret = args.get("annual_return").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        args,
        "mc_default_return",
        0.08,
    ));
    let vol = args.get("annual_volatility").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        args,
        "mc_default_volatility",
        0.3,
    ));
    let days =
        args.get("days")
            .and_then(|v| v.as_u64())
            .unwrap_or(tv_i64(args, "mc_default_days", 30) as u64) as usize;
    let sims = args.get("simulations").and_then(|v| v.as_u64()).unwrap_or(tv_i64(
        args,
        "mc_default_simulations",
        1000,
    ) as u64) as usize;
    let (dr, dv) = (ret / 252.0, vol / (252.0f64).sqrt());
    let mut outs = Vec::with_capacity(sims);
    let mut s0 = 1234567890123456789u64;
    let mut s1 = 9876543210987654321u64;
    for _ in 0..sims {
        let mut p = price;
        for _ in 0..days {
            let nrm = normal_approx(&mut s0, &mut s1);
            p *= 1.0 + dr + dv * nrm;
        }
        outs.push((p * 100.0).round() / 100.0);
    }
    outs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = outs.len();
    let pct = |p: f64| outs[((p * n as f64) as usize).min(n - 1)];
    Ok(
        json!({"p50": pct(0.5), "p10": pct(0.1), "p90": pct(0.9), "mean_price": (outs.iter().sum::<f64>() / n as f64 * 100.0).round() / 100.0, "simulations": sims}),
    )
}

fn industry_pos(args: &Value) -> Result<Value, String> {
    let sp = args.get("stock_pe").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let sg = args.get("stock_growth").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let ip = args.get("industry_avg_pe").and_then(|v| v.as_f64()).unwrap_or(sp);
    let ig = args.get("industry_avg_growth").and_then(|v| v.as_f64()).unwrap_or(sg);
    if ip <= 0.0 || ig <= 0.0 {
        return Ok(json!({"position": "数据无效"}));
    }
    let (pr, gr) = (sp / ip, sg / ig);
    // 行业内估值/增长对比的判定阈值（用户可调）
    let pe_cheap = tv_f64(args, "industry_pe_cheap", 1.0);
    let pe_expensive = tv_f64(args, "industry_pe_expensive", 1.5);
    let gr_high = tv_f64(args, "industry_growth_high", 1.2);
    let score = if pr < pe_cheap && gr > 1.0 {
        "质优价廉"
    } else if pr < pe_cheap {
        "低估值低增长"
    } else if pr > pe_expensive && gr > gr_high {
        "高估值高增长"
    } else if pr > pe_expensive {
        "相对高估"
    } else {
        "相对合理"
    };
    Ok(
        json!({"pe_ratio": (pr * 100.0).round() / 100.0, "growth_ratio": (gr * 100.0).round() / 100.0, "overall": score}),
    )
}

fn limit_up(args: &Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    struct R {
        close: f64,
        high: f64,
        volume: f64,
    }
    let kl: Vec<R> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let mt = args.get("market_type").and_then(|v| v.as_str()).unwrap_or("main");
    let lp = match mt {
        "star" | "chinext" => tv_f64(args, "limit_pct_star", 20.0),
        "bj" => tv_f64(args, "limit_pct_bj", 30.0),
        _ => tv_f64(args, "limit_pct_main", 10.0),
    };
    if kl.len() < 10 {
        return Ok(json!({"potential": "数据不足", "confidence": 0.0}));
    }
    let n = kl.len();
    let hits = kl[n - 10..]
        .iter()
        .filter(|k| {
            (k.high - k.close * (1.0 + lp / 100.0)).abs() < k.close * (1.0 + lp / 100.0) * 0.005
        })
        .count();
    let avg_v = kl[n - 10..].iter().map(|k| k.volume).sum::<f64>() / 10.0;
    let vr = if avg_v > 0.0 {
        kl[n - 1].volume / avg_v
    } else {
        1.0
    };
    let up_d = kl[n - 10..].iter().filter(|k| k.close > k.high * 0.99).count();
    let trend = (up_d as f64 / 10.0 - 0.5) * 2.0;
    // 涨停潜力评分的权重（用户可调）
    let w_trend = tv_f64(args, "limit_up_w_trend", 40.0);
    let w_volume = tv_f64(args, "limit_up_w_volume", 20.0);
    let w_hits = tv_f64(args, "limit_up_w_hits", 15.0);
    let score = trend * w_trend + (vr.min(3.0) - 1.0) * w_volume + (hits as f64) * w_hits;
    let th_high = tv_f64(args, "limit_up_th_high", 60.0);
    let th_med = tv_f64(args, "limit_up_th_med", 30.0);
    let th_low = tv_f64(args, "limit_up_th_low", 10.0);
    let pot = if score > th_high {
        "高"
    } else if score > th_med {
        "中"
    } else if score > th_low {
        "低"
    } else {
        "极低"
    };
    Ok(
        json!({"potential": pot, "confidence": (score / 100.0).min(0.95), "recent_hits": hits, "volume_ratio": (vr * 100.0).round() / 100.0, "limit_pct": lp}),
    )
}

// ═══════════ 宏 ═══════════

macro_rules! calc_tool {
    ($name:ident, $fn:ident, $display:literal, $desc:literal) => {
        pub struct $name;
        #[async_trait]
        impl Tool for $name {
            fn name(&self) -> &str {
                $display
            }
            fn description(&self) -> &str {
                $desc
            }
            fn input_schema(&self) -> Value {
                json!({})
            }
            fn category(&self) -> ToolCategory {
                ToolCategory::Finance
            }
            fn is_concurrency_safe(&self) -> bool {
                true
            }
            async fn call(
                &self,
                input: Value,
                _ctx: &ToolContext,
            ) -> Result<ToolResult, ToolError> {
                $fn(&input)
                    .map(|v| ToolResult::success(v.to_string()))
                    .map_err(|e| ToolError::execution_failed(e))
            }
        }
    };
}

macro_rules! calc_tool_r {
    ($name:ident, $display:literal, $desc:literal, |$input:ident| $prep:expr) => {
        pub struct $name;
        #[async_trait]
        impl Tool for $name {
            fn name(&self) -> &str {
                $display
            }
            fn description(&self) -> &str {
                $desc
            }
            fn input_schema(&self) -> Value {
                json!({})
            }
            fn category(&self) -> ToolCategory {
                ToolCategory::Finance
            }
            fn is_concurrency_safe(&self) -> bool {
                true
            }
            async fn call(
                &self,
                $input: Value,
                _ctx: &ToolContext,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success({ $prep }.to_string()))
            }
        }
    };
}

calc_tool_r!(CalcMaxDrawdownTool, "calc_max_drawdown", "计算最大回撤比例", |input| {
    let prices = parse_f64s(&input, "prices_json");
    let dd = max_drawdown(&prices);
    json!({"max_drawdown_pct": (dd * 10000.0).round() / 100.0})
});
calc_tool_r!(CalcSharpeRatioTool, "calc_sharpe_ratio", "计算夏普比率", |input| {
    let returns = parse_f64s(&input, "returns_json");
    let rf = input.get("risk_free").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        &input,
        "risk_free_rate",
        0.03,
    ));
    // P3-C8: 默认年化因子统一为 A 股 244 天
    let ann = tv_f64(
        &input,
        "risk_sharpe_annualization",
        axagent_harness::indicators::A_SHARE_TRADING_DAYS_PER_YEAR,
    );
    serde_json::to_value(sharpe_ratio(&returns, rf, ann)).unwrap_or_default()
});
calc_tool_r!(CalcVarTool, "calc_var", "历史模拟法 VaR 计算", |input| {
    let returns = parse_f64s(&input, "returns_json");
    let conf = input.get("confidence").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        &input,
        "var_confidence",
        0.95,
    ));
    serde_json::to_value(engine_value_at_risk(&returns, conf)).unwrap_or_default()
});
calc_tool_r!(CalcPEPercentileTool, "calc_pe_percentile", "PE 历史分位数", |input| {
    let cur = input.get("current_pe").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let hist = parse_f64s(&input, "historical_pes_json");
    serde_json::to_value(pe_percentile(cur, &hist)).unwrap_or_default()
});
calc_tool_r!(CalcPEGTool, "calc_peg", "PEG 估值指标", |input| {
    let pe = input.get("pe").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let g = input.get("growth_rate").and_then(|v| v.as_f64()).unwrap_or(0.0);
    serde_json::to_value(peg_ratio(pe, g)).unwrap_or_default()
});
calc_tool_r!(CalcKellyTool, "calc_kelly", "凯利公式仓位计算", |input| {
    let wr = input.get("win_rate").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        &input,
        "kelly_default_win_rate",
        0.5,
    ));
    let aw = input.get("avg_win").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        &input,
        "kelly_default_avg_win",
        0.05,
    ));
    let al = input.get("avg_loss").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        &input,
        "kelly_default_avg_loss",
        0.05,
    ));
    let heavy = tv_f64(&input, "risk_kelly_heavy_threshold", 0.25);
    let med = tv_f64(&input, "risk_kelly_medium_threshold", 0.1);
    serde_json::to_value(kelly_criterion_with_thresholds(wr, aw, al, heavy, med))
        .unwrap_or_default()
});
calc_tool_r!(CalcRiskParityTool, "calc_risk_parity", "风险平价权重计算", |input| {
    let vols = parse_f64s(&input, "volatilities_json");
    let corr_json = input.get("correlations_json").and_then(|v| v.as_str()).unwrap_or("[]");
    serde_json::to_value(risk_parity(&vols, corr_json)).unwrap_or_default()
});

calc_tool_r!(DetectMACrossTool, "detect_ma_cross", "MA 金叉死叉检测", |input| {
    let kj = input.get("klines_json").and_then(|v| v.as_str()).unwrap_or("[]");
    let fast = input.get("fast_period").and_then(|v| v.as_u64()).unwrap_or(tv_i64(
        &input,
        "signal_ma_fast",
        5,
    ) as u64) as usize;
    let slow = input.get("slow_period").and_then(|v| v.as_u64()).unwrap_or(tv_i64(
        &input,
        "signal_ma_slow",
        20,
    ) as u64) as usize;
    serde_json::to_value(detect_ma_cross(kj, fast, slow)).unwrap_or_default()
});
calc_tool_r!(DetectBreakoutTool, "detect_breakout", "支撑阻力突破检测", |input| {
    let kj = input.get("klines_json").and_then(|v| v.as_str()).unwrap_or("[]");
    let sup = input.get("support").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let res = input.get("resistance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let vol_th = tv_f64(&input, "breakout_volume_threshold", 1.5);
    serde_json::to_value(detect_breakout(kj, sup, res, vol_th)).unwrap_or_default()
});

calc_tool_r!(CleanOutliersTool, "clean_outliers", "异常值剔除 (zscore/iqr)", |input| {
    let pj = input.get("prices_json").and_then(|v| v.as_str()).unwrap_or("[]");
    let method = input.get("method").and_then(|v| v.as_str()).unwrap_or(tv_str(
        &input,
        "outlier_method",
        "zscore",
    ));
    let th = input.get("threshold").and_then(|v| v.as_f64()).unwrap_or(tv_f64(
        &input,
        "outlier_threshold",
        2.0,
    ));
    serde_json::to_value(remove_outliers(pj, method, th)).unwrap_or_default()
});
calc_tool_r!(
    CleanFillMissingTool,
    "clean_fill_missing",
    "缺失值填充 (forward/linear)",
    |input| {
        let pj = input.get("prices_json").and_then(|v| v.as_str()).unwrap_or("[]");
        let method = input.get("method").and_then(|v| v.as_str()).unwrap_or(tv_str(
            &input,
            "fill_missing_method",
            "forward",
        ));
        serde_json::to_value(fill_missing(pj, method)).unwrap_or_default()
    }
);
calc_tool_r!(AdjustPricesTool, "adjust_prices", "前复权价格调整", |input| {
    let kj = input.get("klines_json").and_then(|v| v.as_str()).unwrap_or("[]");
    let dj = input.get("dividends_json").and_then(|v| v.as_str()).unwrap_or("[]");
    serde_json::to_value(adjust_prices(kj, dj)).unwrap_or_default()
});

calc_tool!(ComputeATRTool, compute_atr, "compute_atr", "计算 ATR 平均真实波幅");
calc_tool!(ComputeKDJTool, compute_kdj, "compute_kdj", "计算 KDJ 随机指标");
calc_tool!(ComputeOBVTool, compute_obv, "compute_obv", "计算 OBV 能量潮");
calc_tool!(CalcBetaTool, calc_beta, "calc_beta", "计算 Beta 系数");

calc_tool!(
    DetectEarningsTool,
    detect_earnings,
    "detect_earnings_surprise",
    "检测业绩超预期/低于预期"
);
calc_tool!(DetectPledgeRiskTool, detect_pledge, "detect_pledge_risk", "检测大股东质押风险");
calc_tool!(
    CalcCorrMatrixTool,
    calc_corr_matrix,
    "calc_correlation_matrix",
    "计算收益率相关系数矩阵"
);
calc_tool!(RunMonteCarloTool, monte_carlo, "run_monte_carlo", "蒙特卡洛模拟价格路径");
calc_tool!(
    AnalyzeIndustryTool,
    industry_pos,
    "analyze_industry_position",
    "行业内估值/增长对比分析"
);
calc_tool!(DetectLimitUpTool, limit_up, "detect_limit_up_potential", "涨停潜力评估");

// ── K线形态与背离检测（基于 astock-data 模块）──

/// K 线形态检测包装函数
fn detect_candlestick_patterns(input: &Value) -> Result<String, String> {
    let kj = input.get("klines_json").and_then(|v| v.as_str()).unwrap_or("[]");
    let klines: Vec<axagent_astock_data::KLine> =
        serde_json::from_str(kj).map_err(|e| format!("解析 K 线数据失败: {e}"))?;
    let patterns = axagent_astock_data::candlestick_pattern::detect_all_patterns(&klines);
    serde_json::to_string(&patterns).map_err(|e| format!("序列化形态结果失败: {e}"))
}

/// 价量背离检测包装函数
fn detect_divergence(input: &Value) -> Result<String, String> {
    let kj = input.get("klines_json").and_then(|v| v.as_str()).unwrap_or("[]");
    let rsi_period = input.get("rsi_period").and_then(|v| v.as_f64()).unwrap_or(14.0) as usize;
    let lookback = input.get("lookback").and_then(|v| v.as_f64()).unwrap_or(14.0) as usize;
    let klines: Vec<axagent_astock_data::KLine> =
        serde_json::from_str(kj).map_err(|e| format!("解析 K 线数据失败: {e}"))?;
    let results =
        axagent_astock_data::divergence::detect_all_divergences(&klines, rsi_period, lookback);
    serde_json::to_string(&results).map_err(|e| format!("序列化背离结果失败: {e}"))
}

calc_tool!(
    DetectCandlestickPatternsTool,
    detect_candlestick_patterns,
    "detect_candlestick_patterns",
    "检测 K 线形态（吞没/锤子/晨星等 12 种）"
);
calc_tool!(
    DetectDivergenceTool,
    detect_divergence,
    "detect_divergence",
    "检测价量背离（RSI 顶底背离 + OBV 背离）"
);

// ═══════════ 数据 API 工具（需要 AStockClient）═══════════

macro_rules! api_tool {
    ($name:ident, $display:literal, $desc:literal, |$input:ident, $c:ident| $body:expr) => {
        pub struct $name;
        #[async_trait]
        impl Tool for $name {
            fn name(&self) -> &str {
                $display
            }
            fn description(&self) -> &str {
                $desc
            }
            fn input_schema(&self) -> Value {
                json!({})
            }
            fn category(&self) -> ToolCategory {
                ToolCategory::Finance
            }
            fn is_concurrency_safe(&self) -> bool {
                true
            }
            async fn call(
                &self,
                $input: Value,
                _ctx: &ToolContext,
            ) -> Result<ToolResult, ToolError> {
                let $c = global_state::get_astock_client().ok_or_else(|| {
                    ToolError::execution_failed("AStockClient 未初始化".to_string())
                })?;
                $body
            }
        }
    };
}

api_tool!(ResearchReportsTool, "get_research_reports", "获取券商研报", |input, c| {
    let code = input.get("stock_code").and_then(|v| v.as_str()).unwrap_or("000001");
    c.get_research_reports(code)
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});
api_tool!(ConceptBlocksTool, "get_concept_blocks", "获取概念板块归属", |input, c| {
    let code = input.get("stock_code").and_then(|v| v.as_str()).unwrap_or("000001");
    c.get_concept_blocks(code)
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});
api_tool!(NorthBoundFlowTool, "get_north_bound_flow", "获取北向资金流向", |_input, c| {
    c.get_north_bound_flow()
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});
api_tool!(DragonTigerTool, "get_market_dragon_tiger", "获取龙虎榜数据", |_input, c| {
    c.get_market_dragon_tiger()
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});
api_tool!(ClsFlashTool, "get_cls_flash", "获取财联社实时快讯", |_input, c| {
    c.get_cls_flash()
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});

// ═══════════ 单元测试 ═══════════
//
// 2026-09-19（A2）：`detect_earnings` 的「估算基准拒判」是阶段一 A2 的消费端落地，
// 此前**零测试覆盖** —— 而它正是「不编数」这条纪律在本文件里的唯一执行点。
// 内联 test mod 必须追加到文件末尾：插在中间会触发 `clippy::items_after_test_module`，
// 而该 lint **只在 clippy 下暴露**（`cargo check` / `cargo test` 全绿也照样违规）。
#[cfg(test)]
mod tests {
    use super::*;
    // 显式导入以免依赖 glob 是否带入父模块的私有 `use`（显式 use 遮蔽 glob，不冲突）
    use serde_json::json;

    /// 估算基准必须**拒判**，而不是拿假基准算出一个像样的档位。
    ///
    /// 数值取自真实兜底规则：主板常数 `0.55`（`astock-data` C-fallback）遇上实际 EPS `0.30`。
    /// 若不拒判，这里会算出约 −45.45% ⇒ 报「低于预期」，而真实一致预期可能远低于 0.30
    /// （即实际上是「超预期」）—— 结论方向都是反的。
    #[test]
    fn detect_earnings_refuses_estimated_consensus() {
        let r = detect_earnings(&json!({
            "actual_eps": 0.30,
            "consensus_eps": 0.55,
            "consensus_eps_is_estimated": true,
        }))
        .expect("拒判是 Ok 分支，不能变成 Err");
        assert_eq!(
            r["surprise_pct"],
            Value::Null,
            "拒判时不得给出数值 —— 编一个数就等于放行假信号"
        );
        assert_eq!(r["level"], "预期基准不可靠");
        // 留痕：两个入参原样带回，决策 JSON 里才能归因「为什么没有超预期结论」
        assert_eq!(r["actual_eps"].as_f64(), Some(0.30));
        assert_eq!(r["consensus_eps"].as_f64(), Some(0.55));
    }

    /// **同数值、只翻转标记** ⇒ 输出必须不同。
    ///
    /// 这是「拒判由 provenance 驱动，而非按数值猜来源」的对照锁：若日后有人把守卫改成
    /// 「`consensus_eps` 等于 0.25/0.40/0.55 之一就拒判」的启发式，本用例会红 ——
    /// 那属于按值域猜来源，真实预期恰好是 0.55 的股票会被误杀。
    #[test]
    fn detect_earnings_treats_explicit_false_as_real() {
        let r = detect_earnings(&json!({
            "actual_eps": 0.30,
            "consensus_eps": 0.55,
            "consensus_eps_is_estimated": false,
        }))
        .expect("正常路径应返回 Ok");
        assert_ne!(r["level"], "预期基准不可靠");
        let pct = r["surprise_pct"].as_f64().expect("正常路径必须给出数值");
        assert!((pct + 45.45).abs() < 0.01, "期望约 −45.45，实得 {pct}");
    }

    /// ⚠ 本用例**故意锁住当前偏弱的行为**：标记缺失时按「真实值」处理。
    ///
    /// `detect_earnings_surprise` 是 `calc_tool!` 注册的通用计算工具，全仓**无** rhai /
    /// 节点硬接线调用它 ⇒ 入参由调用方（LLM）填写，缺省即放行。也就是说 A2 在消费端
    /// 只是一道**软防线**；产出端不再造数才是硬解。订正后的验收口径见
    /// `AUDIT-codebase-review-roadmap-2026-09-19.md` 的 A2 段。
    ///
    /// 2026-09-19（待裁决 ① 已落地）：产出端已收敛 —— `astock-data` 的 C-fallback 不再
    /// 产出板块常数估算（改 `record_degradation` + `return Ok(None)`）。⇒ 本软肋的**触发
    /// 前提**（存在 `is_estimated = true` 的生产者）**当前已消失**：全链只剩 `false`。
    /// 但**契约与守卫都保留** —— 未来任何新的估算来源都必须继续走
    /// `consensus_eps_is_estimated` 自报 provenance，届时本守卫才会重新发挥作用。
    #[test]
    fn detect_earnings_defaults_to_trusting_when_flag_absent() {
        let r = detect_earnings(&json!({ "actual_eps": 0.30, "consensus_eps": 0.55 }))
            .expect("缺省标记应照常计算");
        assert_ne!(
            r["level"], "预期基准不可靠",
            "缺省按真实处理 —— 这是已登记的软肋，本用例只负责把它钉住、不负责修"
        );
    }

    /// 基准为 0 时走「无预期」的既有分支（该检查排在拒判**之前**）—— 防回归。
    #[test]
    fn detect_earnings_reports_no_expectation_on_zero_consensus() {
        let r = detect_earnings(&json!({ "actual_eps": 1.0, "consensus_eps": 0.0 }))
            .expect("应返回 Ok");
        assert_eq!(r["level"], "无预期");
        assert_eq!(r["surprise_pct"].as_f64(), Some(0.0));
    }

    /// 真实基准下的分级不因 A2 改动而漂移（挑一个远离阈值的点，避开 `>` 的边界歧义）。
    #[test]
    fn detect_earnings_keeps_grading_for_real_consensus() {
        let r = detect_earnings(&json!({ "actual_eps": 1.6, "consensus_eps": 1.0 }))
            .expect("正常路径应返回 Ok");
        assert_eq!(r["level"], "大幅超预期");
        let pct = r["surprise_pct"].as_f64().expect("应有数值");
        assert!((pct - 60.0).abs() < 0.01, "期望约 60.0，实得 {pct}");
    }

    /// 2026-09-21 新增（A 修复，**活路径**）：`calc_pe_percentile` 收到亏损企业 PE
    /// （负值，来自 t-risk 的 `peTTM`）时不得报「极低」分位 —— 负 cur 在历史正 PE
    /// 序列里命中 0 条 ⇒ 旧实现 `percentile = 0` ⇒ `level = "极低"`。
    #[test]
    fn test_pe_percentile_negative_pe_is_meaningless() {
        let hist = vec![10.0, 12.0, 15.0, 18.0, 20.0, 22.0, 25.0, 30.0];
        let r = pe_percentile(-144.08, &hist);
        assert_eq!(r.level, "无意义", "亏损 PE 不得被读成极低分位");
        // 正控：同序列下正 PE 仍走原路径（守卫不得吃掉正常输入）
        let ok = pe_percentile(16.0, &hist);
        assert!(ok.percentile > 30.0 && ok.percentile < 60.0);
        assert_ne!(ok.level, "无意义");
    }

    /// 2026-09-21 新增（A 修复，**活路径**）：`calc_peg` 收到亏损企业 PE 时不得报
    /// 「严重低估」（旧实现 −144.08 / 25 = −5.76 落进 `peg < 0.5`）。
    #[test]
    fn test_peg_ratio_negative_pe_is_meaningless() {
        let r = peg_ratio(-144.08, 25.0);
        assert_eq!(r.level, "无意义", "亏损 PE 不得被读成严重低估");
        assert!(r.peg.is_infinite());
        // 正控
        let ok = peg_ratio(20.0, 25.0);
        assert_eq!(ok.level, "低估");
    }

    // ── 去重契约锁（2026-09-21）──────────────────────────────────────────────
    // `pe_percentile` / `peg_ratio` 由本地副本收敛到 `axagent_analysis_engine::risk`
    // 权威源，等价性已逐行证过（见文件上半部注释 + 快照证据）。但**将来**权威源
    // 加字段 / 改档位时会**静默**改到 `calc_pe_percentile` / `calc_peg` 的 LLM 可见
    // 输出 —— 类型检查抓不到。下面三个测试把「工具输出的精确 JSON 形态」钉死。

    /// 键集合必须恰为 `{peg, level}`：权威源 `PEGResult` 多出的 `pe` / `growth_rate`
    /// **不得**泄露到工具输出 —— 这正是 `PegR` 适配层存在的全部理由。
    #[test]
    fn test_calc_peg_output_contract_keys() {
        let v = serde_json::to_value(peg_ratio(20.0, 25.0)).expect("序列化不应失败");
        let obj = v.as_object().expect("应为 JSON 对象");
        let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["level", "peg"], "calc_peg 输出键集合被改动 ⇒ LLM 可见契约变化");
    }

    /// 键集合必须恰为 `{percentile, level, median}`。该函数**无适配层**（权威源结构体
    /// 直接序列化）⇒ 权威源一旦加字段就会直接漏进工具输出，此处是唯一拦点。
    #[test]
    fn test_calc_pe_percentile_output_contract_keys() {
        let hist = [10.0, 12.0, 15.0, 18.0, 20.0, 22.0, 25.0, 30.0];
        let v = serde_json::to_value(pe_percentile(16.0, &hist)).expect("序列化不应失败");
        let obj = v.as_object().expect("应为 JSON 对象");
        let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["level", "median", "percentile"],
            "calc_pe_percentile 输出键集合被改动"
        );
    }

    /// 亏损分支的 `f64::INFINITY` 经 serde_json 落为 `null`（JSON 无 Infinity）
    /// ⇒ 工具输出是 `{"peg": null, "level": "无意义"}`。钉死形态：防将来有人把它
    /// 改成有效数值（负 peg 会被读成「严重低估」，正是 A 缺陷的原形）。
    #[test]
    fn test_calc_peg_negative_pe_serializes_infinity_as_null() {
        let v = serde_json::to_value(peg_ratio(-144.08, 25.0)).expect("序列化不应失败");
        assert!(v["peg"].is_null(), "Infinity 必须以 null 落地，实得 {:?}", v["peg"]);
        assert_eq!(v["level"], "无意义");
    }

    /// 键集合必须恰为 `{var_pct, confidence, cvar_pct}`。该函数**无适配层**
    /// （权威源 `VarResult` 直接序列化）⇒ 权威源一旦加字段就会直接漏进 `calc_var`
    /// 的 LLM 可见输出，此处是唯一拦点。
    #[test]
    fn test_calc_var_output_contract_keys() {
        let rets = [-0.05, -0.03, -0.02, -0.01, -0.01, 0.01, 0.01, 0.02, 0.02, 0.03];
        let v = serde_json::to_value(engine_value_at_risk(&rets, 0.9)).expect("序列化不应失败");
        let obj = v.as_object().expect("应为 JSON 对象");
        let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["confidence", "cvar_pct", "var_pct"],
            "calc_var 输出键集合被改动 ⇒ LLM 可见契约变化"
        );
    }

    /// 口径锁（工具侧视角）：转发之后 `calc_var` 输出的必须是**生产原有口径**
    /// （`idx = floor((1-c) * n)` 直接取值），而不是引擎旧口径
    /// （`floor((1-c) * (n+1))` 再 `-1`，净取前一位）。
    ///
    /// 参数刻意取 `c = 0.875`（`1-c = 0.125` 是**二进制精确值**）：整十置信度 × n=10
    /// 那类组合下两口径输出相同、没有区分力（见 `risk.rs` doc 的「浮点 floor 边界」）。
    /// 本条与 `risk.rs::tests::test_var` 构成双保险 —— 谁改回旧口径，两侧同时红。
    #[test]
    fn test_calc_var_keeps_production_percentile_convention() {
        let rets = [-0.50, -0.40, -0.30, -0.20, -0.10, 0.00, 0.10, 0.20, 0.30, 0.40];
        let r = engine_value_at_risk(&rets, 0.875);
        assert!(
            (r.var_pct - 0.40).abs() < 1e-9,
            "c=0.875 ⇒ idx=1 ⇒ 0.40；旧混合口径为 0.50（取最小值）"
        );
        assert!((r.cvar_pct - 0.45).abs() < 1e-9, "尾均值应与 var 用同一个 idx");
    }
}
