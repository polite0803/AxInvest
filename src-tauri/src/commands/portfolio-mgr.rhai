// portfolio-mgr.rhai — 贝叶斯收缩 + 反身性 + 预测集成 决策公式
// Input via input_mapping:
//   totalScore, dqi_score, overall_risk, catalyst_level,
//   institutional_trace, consensusScore,
//   trader_time_horizon, trader_holding_days (optional overrides)
//   aggregate_direction, aggregate_confidence (from debate-convergence)
//   signal_quality_win_rate, signal_quality_sample_count (from backtest)
// All inputs have safe defaults when missing
//
// 三层实战框架：
//   筛选层: market_regime 选择对应的 Bayesian 先验
//   概率层: Bayesian 后验 = (先验权重×先验 + n×样本胜率)/(先验权重 + n)
//   应对层: 反身性系数 × 预测聚合 → 最终置信度

// ── 空值保护 ──
fn safe_num(x, dflt) {
    if type_of(x) == "()" { dflt } else { x }
}
fn safe_str(x, dflt) {
    if type_of(x) == "()" { dflt } else { x }
}

let total_score = safe_num(totalScore, 50.0);
let dqi_score_num = safe_num(dqi_score, 50.0);
let overall_risk_str = safe_str(overall_risk, "中");
let catalyst_level_str = safe_str(catalyst_level, "");
let institutional_trace_str = safe_str(institutional_trace, "");
let consensus_score = safe_num(consensusScore, 50.0);

// trader + aggregate_prediction
let trader_time_horizon_raw = safe_str(trader_time_horizon, "");
let trader_holding_days_raw = safe_num(trader_holding_days, 0.0);
let has_trader_input = trader_time_horizon_raw != "" && trader_holding_days_raw > 0.0;
let aggregate_direction_str = safe_str(aggregate_direction, "");
let aggregate_confidence_num = safe_num(aggregate_confidence, 0.0);
let has_aggregate_prediction = aggregate_direction_str != "" && aggregate_confidence_num > 0.0;

// ── 第一层：因果筛选 → 选择贝叶斯先验 ──
// 市场环境决定"无条件胜率"（不做策略时随机持有多头的胜率）
//   "bull"  → 0.55（牛市上升趋势，搭车效应）
//   "bear"  → 0.45（熊市下降趋势，逆风）
//   _       → 0.50（中性/未知，拉普拉斯无差别先验）
let market_regime_str = safe_str(market_regime, "neutral");
let bayesian_prior = switch market_regime_str {
    "bull" => 0.55,
    "bear" => 0.45,
    _ => 0.50
};

// ── 第二层：贝叶斯收缩 → 后验胜率 ──
// 公式: posterior = (prior_weight × prior + n × sample_win_rate) / (prior_weight + n)
// prior_weight = 20（虚拟样本量：我们对先验的信赖程度）
//   n < 20  → 后验显著向先验收缩（对抗小样本幻觉）
//   n > 200 → 后验接近原始样本胜率（数据充分时相信数据）
let signal_quality_win_rate = safe_num(signal_quality_win_rate, bayesian_prior);
let signal_quality_sample_count = safe_num(signal_quality_sample_count, 0.0);
let has_backtest_data = signal_quality_sample_count >= 5.0;
let prior_weight = 20.0;

let bayesian_posterior = if has_backtest_data {
    (prior_weight * bayesian_prior + signal_quality_sample_count * signal_quality_win_rate)
        / (prior_weight + signal_quality_sample_count)
} else {
    bayesian_prior  // 无回测数据 → 只用先验
};

// 贝叶斯收缩后，映射到信号质量乘数 [0.7, 1.3]
//   posterior >= 0.55 → 乘数 ≥ 1.0
//   posterior < 0.45  → 乘数 ≤ 1.0
let signal_quality_factor = clamp((bayesian_posterior / 0.50) - 0.20, 0.7, 1.3);

// ── 第三层：反身性系数 ──
// 高波动/高拥挤环境下，技术信号的反身性效应显著：
//   信号本身改变了参与者的行为 → 可预测性下降
//   低风险(β<0.8) → ×1.10（防御性，信号可靠度提升）
//   高风险(β>1.2) → ×0.85（高波动噪声大）
//   极高(β>2.0)   → ×0.60（极高博弈性，潜在反转）
let reflexivity_factor = switch overall_risk_str {
    "低风险" | "低" => 1.10,
    "高风险" | "高" => 0.85,
    "极高" => 0.60,
    _ => 1.0
};

// ── 基础评分 ──
let consensus_adj = (consensus_score - 50.0) / 100.0 * 10.0;
let dqi_adj = (dqi_score_num - 50.0) / 100.0 * 5.0;

let risk_adj = switch overall_risk_str {
    "低风险" | "低" => 2.0,
    "高风险" | "高" => -4.0,
    "极高" => -8.0,
    _ => 0.0
};

let cat_bonus = switch catalyst_level_str {
    "L3估值体系级" => 12.0,
    "L2业绩拐点级" => 6.0,
    "L1普通消息" => 2.0,
    _ => 0.0
};

let inst_bonus = if institutional_trace_str == "有建仓痕迹" || institutional_trace_str == "疑似建仓" { 5.0 } else { 0.0 };

// ── 整体决策：基础置信度 ──
let adjustment = consensus_adj + dqi_adj + risk_adj + cat_bonus + inst_bonus;
let confidence = clamp(total_score + adjustment, 0.0, 100.0);

// ── 预测聚合：LLM 分析师一致方向修正 ──
let prediction_boost = if has_aggregate_prediction && aggregate_direction_str == "bullish" {
    aggregate_confidence_num * 15.0
} else if has_aggregate_prediction && aggregate_direction_str == "bearish" {
    -(aggregate_confidence_num * 20.0)
} else if has_aggregate_prediction && aggregate_direction_str == "divided" {
    -5.0
} else { 0.0 };

let boosted_confidence = clamp(confidence + prediction_boost, 0.0, 100.0);

// ── 贝叶斯收缩 + 反身性 → 最终置信度 ──
//   统计学 = 开仓的勇气 (boosted_confidence)
//   反身性 = 持仓的理性 (reflexivity_factor)
//   贝叶斯 = 空仓的定力 (signal_quality_factor)
let final_confidence = clamp(
    boosted_confidence * signal_quality_factor * reflexivity_factor,
    0.0, 100.0
);

let base_pos = if risk_adj >= 0.0 { final_confidence * 0.8 } else { final_confidence * 0.5 };
let position_pct = clamp(base_pos, 0.0, 100.0);

let overall_action = if final_confidence >= 80.0 && position_pct >= 30.0 { "增持" }
    else if final_confidence >= 60.0 { "买入" }
    else if final_confidence >= 40.0 { "持有" }
    else if final_confidence < 30.0 { "卖出" }
    else if position_pct < 20.0 { "减持" }
    else { "持有" };

// ── 时间维度推导（加权打分法） ──
let is_high_risk = overall_risk_str == "高风险" || overall_risk_str == "高" || overall_risk_str == "极高";

let risk_score = switch overall_risk_str {
    "低风险" | "低" => 90.0,
    "高风险" | "高" => 30.0,
    "极高" => 10.0,
    _ => 50.0
};

let long_score = final_confidence * 0.4 + dqi_score_num * 0.3 + risk_score * 0.3;
let mid_score = consensus_score * 0.5 + final_confidence * 0.3 + dqi_score_num * 0.2;
let short_score = total_score * 0.15 + cat_bonus * 3.0 + inst_bonus * 3.0 +
    (if is_high_risk { 15.0 } else { 0.0 });
let ultra_short_score = cat_bonus * 5.0 + inst_bonus * 5.0 +
    (if is_high_risk { 20.0 } else { 0.0 });

let time_horizon = if has_trader_input { trader_time_horizon_raw }
    else {
        let max_score = long_score.max(mid_score.max(short_score.max(ultra_short_score)));
        if ultra_short_score == max_score { "ultra_short" }
        else if short_score == max_score { "short" }
        else if mid_score == max_score { "mid" }
        else { "long" }
    };

let expected_holding_days = if has_trader_input {
    trader_holding_days_raw.to_int()
} else {
    switch time_horizon {
        "ultra_short" => 2, "short" => 5, "mid" => 28, "long" => 90, _ => 5
    }
};

let target_timeframe = switch time_horizon {
    "ultra_short" => "1d-2d", "short" => "1w", "mid" => "1m", "long" => "3m", _ => "1m"
};

// ── 分时间维度信号 ──
let ultra_short_adj = cat_bonus * 1.5 + inst_bonus * 2.0;
let ultra_short_confidence = clamp(total_score * 0.25 + consensus_score * 0.15 + ultra_short_adj, 0.0, 100.0);
let ultra_short_action = if ultra_short_confidence >= 55.0 { "买入" }
    else if ultra_short_confidence >= 35.0 { "持有" } else { "观望" };

let short_adj = cat_bonus + inst_bonus + dqi_adj * 0.3 + (if is_high_risk { -2.0 } else { 2.0 });
let short_confidence = clamp(total_score * 0.4 + consensus_score * 0.2 + short_adj, 0.0, 100.0);
let short_action = if short_confidence >= 65.0 { "买入" }
    else if short_confidence >= 45.0 { "持有" } else { "观望" };

let mid_adj = consensus_adj + dqi_adj;
let mid_confidence = clamp(total_score * 0.5 + consensus_score * 0.3 + mid_adj, 0.0, 100.0);
let mid_action = if mid_confidence >= 70.0 { "买入" }
    else if mid_confidence >= 50.0 { "持有" } else { "观望" };

let long_adj = dqi_adj + risk_adj.max(0.0);
let long_confidence = clamp(total_score * 0.6 + dqi_score_num * 0.3 + long_adj, 0.0, 100.0);
let long_action = if long_confidence >= 65.0 { "买入" }
    else if long_confidence >= 45.0 { "持有" } else { "观望" };

// ── 止损/止盈 ──
let stop_loss_pct = switch time_horizon {
    "ultra_short" => 3.0, "short" => 5.0, "mid" => 8.0, "long" => 12.0, _ => 5.0
};
let take_profit_pct = switch time_horizon {
    "ultra_short" => 5.0, "short" => 10.0, "mid" => 18.0, "long" => 30.0, _ => 10.0
};

// ── reasoning ──
let bayes_note = if has_backtest_data {
    ` | 贝叶斯: prior=${bayesian_prior} n=${signal_quality_sample_count} sample=${signal_quality_win_rate} posterior=${bayesian_posterior} factor=${signal_quality_factor}`
} else { "" };
let refl_note = ` refl=${reflexivity_factor}`;
let pred_note = if has_aggregate_prediction {
    ` | 预测: ${aggregate_direction_str}(${aggregate_confidence_num})`
} else { "" };

// ── 输出 ──
#{
    "action": overall_action,
    "positionPct": position_pct,
    "confidence": final_confidence,
    "riskLevel": overall_risk_str,
    "stopLossPct": if position_pct > 0.0 { stop_loss_pct } else { 0.0 },
    "takeProfitPct": if position_pct > 0.0 { take_profit_pct } else { 0.0 },
    "reasoning": `决策=${overall_action} 置信度=${final_confidence} 仓位=${position_pct}% 风险=${overall_risk_str} 共识=${consensus_score}${bayes_note}${refl_note}${pred_note}`,
    "key_conditions_to_track": [],
    "decisive_bull_acks": [],
    "decisive_bear_acks": [],

    "timeHorizon": time_horizon,
    "expectedHoldingDays": expected_holding_days,
    "targetTimeframe": target_timeframe,

    // Bayesian internals (for debugging)
    "bayesianPrior": bayesian_prior,
    "bayesianPosterior": bayesian_posterior,
    "bayesianSampleCount": signal_quality_sample_count,
    "reflexivityFactor": reflexivity_factor,
    "signalQualityFactor": signal_quality_factor,

    "ultraShortTermSignal": #{
        "timeframe": "ultra_short",
        "action": ultra_short_action,
        "confidence": ultra_short_confidence,
        "positionPct": clamp(ultra_short_confidence * 0.3, 0.0, 100.0),
        "riskLevel": overall_risk_str,
        "reasoning": `超短线：催化剂=${cat_bonus}, 资金=${inst_bonus}, 置信度=${ultra_short_confidence}`
    },
    "shortTermSignal": #{
        "timeframe": "short",
        "action": short_action,
        "confidence": short_confidence,
        "positionPct": clamp(short_confidence * 0.5, 0.0, 100.0),
        "riskLevel": overall_risk_str,
        "reasoning": `短线：催化剂=${cat_bonus}, 资金=${inst_bonus}, 置信度=${short_confidence}`
    },
    "midTermSignal": #{
        "timeframe": "mid",
        "action": mid_action,
        "confidence": mid_confidence,
        "positionPct": clamp(mid_confidence * 0.6, 0.0, 100.0),
        "riskLevel": if is_high_risk { "中" } else { overall_risk_str },
        "reasoning": `中线：共识=${consensus_score}, 总分=${total_score}, 置信度=${mid_confidence}`
    },
    "longTermSignal": #{
        "timeframe": "long",
        "action": long_action,
        "confidence": long_confidence,
        "positionPct": clamp(long_confidence * 0.7, 0.0, 100.0),
        "riskLevel": if is_high_risk { "低" } else { overall_risk_str },
        "reasoning": `长线：数据质量=${dqi_score_num}, 风险=${overall_risk_str}, 置信度=${long_confidence}`
    },

    "computation_logs": `base_conf=${confidence} boost=${prediction_boost}=${boosted_confidence} bayes=${signal_quality_factor} refl=${reflexivity_factor}=${final_confidence} | ${time_horizon}/${expected_holding_days}d | U=${ultra_short_confidence} S=${short_confidence} M=${mid_confidence} L=${long_confidence}`
}
