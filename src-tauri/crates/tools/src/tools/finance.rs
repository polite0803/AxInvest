//! 金融分析工具 — 31 个：风险模型、信号、数据清洗、技术指标、蒙特卡洛等。
//! 所有计算逻辑内联，不依赖 axagent-stock-analysis（避免循环依赖）。

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult, global_state};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};

fn parse_f64s(val: &Value, key: &str) -> Vec<f64> {
    val.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

// ═══════════ 内联数学函数 ═══════════

fn max_drawdown(prices: &[f64]) -> f64 {
    if prices.is_empty() {
        return 0.0;
    }
    let (mut peak, mut max_dd) = (prices[0], 0.0);
    for &p in prices {
        if p > peak {
            peak = p;
        }
        let dd = (peak - p) / peak;
        if dd > max_dd {
            max_dd = dd;
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
fn sharpe_ratio(returns: &[f64], rf: f64) -> SharpeR {
    let n = returns.len();
    if n < 2 {
        return SharpeR {
            sharpe: 0.0,
            annualized: 0.0,
            mean_return: 0.0,
            stddev: 0.0,
        };
    }
    let m = returns.iter().sum::<f64>() / n as f64;
    let v = returns.iter().map(|r| (r - m).powi(2)).sum::<f64>() / (n - 1) as f64;
    let std = v.sqrt();
    let sh = if std > 0.0 { (m - rf) / std } else { 0.0 };
    SharpeR {
        sharpe: (sh * 1000.0).round() / 1000.0,
        annualized: (sh * (252.0f64).sqrt() * 1000.0).round() / 1000.0,
        mean_return: (m * 10000.0).round() / 100.0,
        stddev: (std * 10000.0).round() / 100.0,
    }
}

#[derive(Serialize)]
struct VarR {
    var_pct: f64,
    confidence: f64,
    cvar_pct: f64,
}
fn value_at_risk(returns: &[f64], conf: f64) -> VarR {
    let n = returns.len();
    if n < 5 {
        return VarR {
            var_pct: 0.0,
            confidence: conf,
            cvar_pct: 0.0,
        };
    }
    let mut s = returns.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((1.0 - conf) * n as f64).floor() as usize;
    let var = if idx < n { -s[idx] * 100.0 } else { 0.0 };
    let tail: f64 = s[..=idx.min(n - 1)].iter().map(|r| -r).sum();
    let cvar = tail / (idx + 1) as f64 * 100.0;
    VarR {
        var_pct: (var * 100.0).round() / 100.0,
        confidence: conf,
        cvar_pct: (cvar * 100.0).round() / 100.0,
    }
}

#[derive(Serialize)]
struct PeR {
    percentile: f64,
    level: String,
    median: f64,
}
fn pe_percentile(cur: f64, hist: &[f64]) -> PeR {
    let mut s = hist.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let below = s.iter().filter(|&&p| p <= cur).count();
    let pct = if s.is_empty() {
        50.0
    } else {
        below as f64 / s.len() as f64 * 100.0
    };
    let level = if pct < 20.0 {
        "极低"
    } else if pct < 40.0 {
        "偏低"
    } else if pct < 60.0 {
        "合理"
    } else if pct < 80.0 {
        "偏高"
    } else {
        "极高"
    };
    let med = if !s.is_empty() { s[s.len() / 2] } else { cur };
    PeR {
        percentile: (pct * 10.0).round() / 10.0,
        level: level.into(),
        median: med,
    }
}

#[derive(Serialize)]
struct PegR {
    peg: f64,
    level: String,
}
fn peg_ratio(pe: f64, g: f64) -> PegR {
    if g <= 0.0 {
        return PegR {
            peg: f64::INFINITY,
            level: "无意义".into(),
        };
    }
    let peg = pe / g;
    PegR {
        peg: (peg * 100.0).round() / 100.0,
        level: if peg < 0.5 {
            "严重低估"
        } else if peg < 1.0 {
            "低估"
        } else if peg < 2.0 {
            "合理"
        } else {
            "高估"
        }
        .into(),
    }
}

#[derive(Serialize)]
struct KellyR {
    kelly_fraction: f64,
    half_kelly: f64,
    position_pct: f64,
    signal: String,
}
fn kelly(wr: f64, aw: f64, al: f64) -> KellyR {
    if al <= 0.0 || aw <= 0.0 || wr <= 0.0 {
        return KellyR {
            kelly_fraction: 0.0,
            half_kelly: 0.0,
            position_pct: 0.0,
            signal: "不适用".into(),
        };
    }
    let odds = aw / al;
    let k = ((wr * (odds + 1.0) - 1.0) / odds).max(0.0);
    let h = k / 2.0;
    KellyR {
        kelly_fraction: (k * 1000.0).round() / 1000.0,
        half_kelly: (h * 1000.0).round() / 1000.0,
        position_pct: (h * 10000.0).round() / 100.0,
        signal: if k > 0.25 {
            "重仓"
        } else if k > 0.1 {
            "中等"
        } else if k > 0.0 {
            "轻仓"
        } else {
            "不建议"
        }
        .into(),
    }
}

#[derive(Serialize)]
struct RpR {
    weights: Vec<f64>,
    divers_ratio: f64,
}
fn risk_parity(vols: &[f64]) -> RpR {
    let n = vols.len();
    if n == 0 {
        return RpR {
            weights: vec![],
            divers_ratio: 0.0,
        };
    }
    let inv: Vec<f64> = vols
        .iter()
        .map(|&v| if v > 0.0 { 1.0 / v } else { 0.0 })
        .collect();
    let total: f64 = inv.iter().sum();
    let w: Vec<f64> = if total > 0.0 {
        inv.iter()
            .map(|&x| (x / total * 10000.0).round() / 10000.0)
            .collect()
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

fn sma(prices: &[f64], period: usize) -> Option<f64> {
    if period == 0 || prices.len() < period {
        return None;
    }
    Some(prices[prices.len() - period..].iter().sum::<f64>() / period as f64)
}

#[derive(Serialize)]
struct CrossR {
    signal: String,
    fast_ma: f64,
    slow_ma: f64,
    latest_price: f64,
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
    CrossR {
        signal: sig.into(),
        fast_ma: (cf * 100.0).round() / 100.0,
        slow_ma: (cs * 100.0).round() / 100.0,
        latest_price: kl.last().map(|k| k.close).unwrap_or(0.0),
    }
}

#[derive(Serialize)]
struct BrkR {
    breakout_type: String,
    current_price: f64,
    confidence: String,
}
fn detect_breakout(kj: &str, sup: f64, res: f64) -> BrkR {
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
        };
    }
    let last = kl.last().unwrap();
    let price = last.close;
    let avg_v = if kl.len() >= 5 {
        kl[kl.len() - 6..kl.len() - 1]
            .iter()
            .map(|k| k.volume)
            .sum::<f64>()
            / 5.0
    } else {
        kl.iter().map(|k| k.volume).sum::<f64>() / kl.len() as f64
    };
    let vr = if avg_v > 0.0 {
        Some(last.volume / avg_v)
    } else {
        None
    };
    let (bt, conf) = if price > res {
        let c = if vr.unwrap_or(1.0) > 1.5 {
            "high"
        } else {
            "medium"
        };
        ("resistance_break", c)
    } else if price < sup {
        let c = if vr.unwrap_or(1.0) > 1.5 {
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
        return OutR {
            cleaned: prices,
            removed_count: 0,
        };
    }
    if method == "iqr" {
        let mut s = prices.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q1 = s[(s.len() as f64 * 0.25).floor() as usize];
        let q3 = s[(s.len() as f64 * 0.75).floor() as usize];
        let iqr = q3 - q1;
        if iqr < 1e-10 {
            return OutR {
                cleaned: prices,
                removed_count: 0,
            };
        }
        let (lo, hi) = (q1 - th * iqr, q3 + th * iqr);
        let cleaned: Vec<f64> = prices
            .iter()
            .filter(|&&p| p >= lo && p <= hi)
            .copied()
            .collect();
        let rm = prices.len() - cleaned.len();
        OutR {
            cleaned,
            removed_count: rm,
        }
    } else {
        let n = prices.len();
        let m = prices.iter().sum::<f64>() / n as f64;
        let std = (prices.iter().map(|p| (p - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt();
        if std < 1e-10 {
            return OutR {
                cleaned: prices,
                removed_count: 0,
            };
        }
        let cleaned: Vec<f64> = prices
            .iter()
            .filter(|&&p| (p - m).abs() / std <= th)
            .copied()
            .collect();
        let rm = prices.len() - cleaned.len();
        OutR {
            cleaned,
            removed_count: rm,
        }
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
        return FillR {
            filled: vec![],
            filled_count: 0,
        };
    }
    if method == "linear" {
        let mut r = prices.clone();
        let mut cnt = 0usize;
        let n = r.len();
        let first = r.iter().position(|v| v.is_some());
        if first.is_none() {
            return FillR {
                filled: r,
                filled_count: 0,
            };
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
        FillR {
            filled: r,
            filled_count: cnt,
        }
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
        FillR {
            filled: r,
            filled_count: cnt,
        }
    }
}

#[derive(Serialize)]
struct AdjR {
    adjustment_factor: f64,
}
fn adjust_prices(kj: &str, dj: &str) -> AdjR {
    #[derive(serde::Deserialize)]
    struct K {
        date: String,
        open: f64,
        close: f64,
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
        return AdjR {
            adjustment_factor: 1.0,
        };
    }
    kl.sort_by(|a, b| b.date.cmp(&a.date));
    let mut factor = 1.0;
    for k in kl.iter_mut() {
        for d in &div {
            if d.date == k.date {
                let tr = d.cash_dividend / k.close + d.share_dividend;
                if tr > 0.0 {
                    factor *= 1.0 + tr;
                }
            }
        }
        k.open = (k.open * factor * 100.0).round() / 100.0;
        k.close = (k.close * factor * 100.0).round() / 100.0;
    }
    AdjR {
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
    let period = args.get("period").and_then(|v| v.as_u64()).unwrap_or(14) as usize;
    let n = kl.len();
    if n < 2 || period == 0 {
        return Ok(json!({"atr": 0.0, "period": period}));
    }
    let mut trs = vec![0.0; n - 1];
    for i in 1..n {
        let (p, c) = (&kl[i - 1], &kl[i]);
        trs[i - 1] = (c.high - c.low)
            .max((c.high - p.close).abs())
            .max((c.low - p.close).abs());
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
    let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(9) as usize;
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
    let cov = s[..n]
        .iter()
        .zip(m[..n].iter())
        .map(|(&a, &b)| (a - ms) * (b - mm))
        .sum::<f64>()
        / (n - 1) as f64;
    let vm = m[..n].iter().map(|&x| (x - mm).powi(2)).sum::<f64>() / (n - 1) as f64;
    Ok(json!({"beta": if vm > 1e-10 { (cov / vm * 1000.0).round() / 1000.0 } else { 1.0 }}))
}

// ── P2/P3 ──

fn detect_earnings(args: &Value) -> Result<Value, String> {
    let a = args
        .get("actual_eps")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let c = args
        .get("consensus_eps")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if c.abs() < 1e-10 {
        return Ok(json!({"surprise_pct": 0.0, "level": "无预期"}));
    }
    let s = (a - c) / c.abs() * 100.0;
    let l = if s > 50.0 {
        "大幅超预期"
    } else if s > 20.0 {
        "超预期"
    } else if s > 5.0 {
        "略超预期"
    } else if s > -5.0 {
        "符合预期"
    } else if s > -20.0 {
        "略低于预期"
    } else if s > -50.0 {
        "低于预期"
    } else {
        "大幅低于预期"
    };
    Ok(
        json!({"surprise_pct": (s * 100.0).round() / 100.0, "level": l, "actual_eps": a, "consensus_eps": c}),
    )
}

fn detect_pledge(args: &Value) -> Result<Value, String> {
    let p = args
        .get("pledge_pct")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let w = args
        .get("warning_line")
        .and_then(|v| v.as_f64())
        .unwrap_or(50.0);
    let lq = args
        .get("liquidation_line")
        .and_then(|v| v.as_f64())
        .unwrap_or(70.0);
    let (r, wa) = if p >= lq {
        ("极高风险", "大股东质押濒临平仓线")
    } else if p >= w {
        ("高风险", "质押比例超过预警线")
    } else if p >= 30.0 {
        ("中风险", "质押比例偏高")
    } else if p > 10.0 {
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

fn monte_carlo(args: &Value) -> Result<Value, String> {
    let price = args
        .get("current_price")
        .and_then(|v| v.as_f64())
        .unwrap_or(10.0);
    let ret = args
        .get("annual_return")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.08);
    let vol = args
        .get("annual_volatility")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.3);
    let days = args.get("days").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
    let sims = args
        .get("simulations")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000) as usize;
    let (dr, dv) = (ret / 252.0, vol / (252.0f64).sqrt());
    let mut outs = Vec::with_capacity(sims);
    let mut state = 42u64;
    for _ in 0..sims {
        let mut p = price;
        for _ in 0..days {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let z = (state as f64 % 1e9) / 1e9 * 2.0 - 1.0;
            let z2 = (state.wrapping_mul(2) as f64 % 1e9) / 1e9 * 2.0 - 1.0;
            let nrm = (-2.0 * (1.0 - z2.abs()).ln()).sqrt() * z.signum();
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
    let sg = args
        .get("stock_growth")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let ip = args
        .get("industry_avg_pe")
        .and_then(|v| v.as_f64())
        .unwrap_or(sp);
    let ig = args
        .get("industry_avg_growth")
        .and_then(|v| v.as_f64())
        .unwrap_or(sg);
    if ip <= 0.0 || ig <= 0.0 {
        return Ok(json!({"position": "数据无效"}));
    }
    let (pr, gr) = (sp / ip, sg / ig);
    let score = if pr < 1.0 && gr > 1.0 {
        "质优价廉"
    } else if pr < 1.0 {
        "低估值低增长"
    } else if pr > 1.5 && gr > 1.2 {
        "高估值高增长"
    } else if pr > 1.5 {
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
    let mt = args
        .get("market_type")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    let lp = match mt {
        "star" | "chinext" => 20.0,
        "bj" => 30.0,
        _ => 10.0,
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
    let up_d = kl[n - 10..]
        .iter()
        .filter(|k| k.close > k.high * 0.99)
        .count();
    let trend = (up_d as f64 / 10.0 - 0.5) * 2.0;
    let score = trend * 40.0 + (vr.min(3.0) - 1.0) * 20.0 + (hits as f64) * 15.0;
    let pot = if score > 60.0 {
        "高"
    } else if score > 30.0 {
        "中"
    } else if score > 10.0 {
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
    let rf = input
        .get("risk_free")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.03);
    serde_json::to_value(sharpe_ratio(&returns, rf)).unwrap_or_default()
});
calc_tool_r!(CalcVarTool, "calc_var", "历史模拟法 VaR 计算", |input| {
    let returns = parse_f64s(&input, "returns_json");
    let conf = input
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.95);
    serde_json::to_value(value_at_risk(&returns, conf)).unwrap_or_default()
});
calc_tool_r!(CalcPEPercentileTool, "calc_pe_percentile", "PE 历史分位数", |input| {
    let cur = input
        .get("current_pe")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let hist = parse_f64s(&input, "historical_pes_json");
    serde_json::to_value(pe_percentile(cur, &hist)).unwrap_or_default()
});
calc_tool_r!(CalcPEGTool, "calc_peg", "PEG 估值指标", |input| {
    let pe = input.get("pe").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let g = input
        .get("growth_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    serde_json::to_value(peg_ratio(pe, g)).unwrap_or_default()
});
calc_tool_r!(CalcKellyTool, "calc_kelly", "凯利公式仓位计算", |input| {
    let wr = input
        .get("win_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let aw = input
        .get("avg_win")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.05);
    let al = input
        .get("avg_loss")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.05);
    serde_json::to_value(kelly(wr, aw, al)).unwrap_or_default()
});
calc_tool_r!(CalcRiskParityTool, "calc_risk_parity", "风险平价权重计算", |input| {
    let vols = parse_f64s(&input, "volatilities_json");
    serde_json::to_value(risk_parity(&vols)).unwrap_or_default()
});

calc_tool_r!(DetectMACrossTool, "detect_ma_cross", "MA 金叉死叉检测", |input| {
    let kj = input
        .get("klines_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let fast = input
        .get("fast_period")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as usize;
    let slow = input
        .get("slow_period")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    serde_json::to_value(detect_ma_cross(kj, fast, slow)).unwrap_or_default()
});
calc_tool_r!(DetectBreakoutTool, "detect_breakout", "支撑阻力突破检测", |input| {
    let kj = input
        .get("klines_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let sup = input.get("support").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let res = input
        .get("resistance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    serde_json::to_value(detect_breakout(kj, sup, res)).unwrap_or_default()
});

calc_tool_r!(CleanOutliersTool, "clean_outliers", "异常值剔除 (zscore/iqr)", |input| {
    let pj = input
        .get("prices_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let method = input
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("zscore");
    let th = input
        .get("threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(2.0);
    serde_json::to_value(remove_outliers(pj, method, th)).unwrap_or_default()
});
calc_tool_r!(
    CleanFillMissingTool,
    "clean_fill_missing",
    "缺失值填充 (forward/linear)",
    |input| {
        let pj = input
            .get("prices_json")
            .and_then(|v| v.as_str())
            .unwrap_or("[]");
        let method = input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("forward");
        serde_json::to_value(fill_missing(pj, method)).unwrap_or_default()
    }
);
calc_tool_r!(AdjustPricesTool, "adjust_prices", "前复权价格调整", |input| {
    let kj = input
        .get("klines_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let dj = input
        .get("dividends_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
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
    let code = input
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    c.get_research_reports(code)
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});
api_tool!(
    ConceptBlocksTool,
    "get_concept_blocks",
    "获取概念板块归属",
    |input, c| {
        let code = input
            .get("stock_code")
            .and_then(|v| v.as_str())
            .unwrap_or("000001");
        c.get_concept_blocks(code)
            .await
            .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
            .map_err(|e| ToolError::execution_failed(e.to_string()))
    }
);
api_tool!(
    NorthBoundFlowTool,
    "get_north_bound_flow",
    "获取北向资金流向",
    |_input, c| {
        c.get_north_bound_flow()
            .await
            .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
            .map_err(|e| ToolError::execution_failed(e.to_string()))
    }
);
api_tool!(
    DragonTigerTool,
    "get_market_dragon_tiger",
    "获取龙虎榜数据",
    |_input, c| {
        c.get_market_dragon_tiger()
            .await
            .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
            .map_err(|e| ToolError::execution_failed(e.to_string()))
    }
);
api_tool!(ClsFlashTool, "get_cls_flash", "获取财联社实时快讯", |_input, c| {
    c.get_cls_flash()
        .await
        .map(|v| ToolResult::success(serde_json::to_value(v).unwrap_or_default().to_string()))
        .map_err(|e| ToolError::execution_failed(e.to_string()))
});
