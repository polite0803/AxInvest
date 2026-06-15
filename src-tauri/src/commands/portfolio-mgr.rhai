// portfolio-mgr.rhai — 确定性决策公式（含多时间维度）
// Input via input_mapping:
//   totalScore, dqi_score, overall_risk, catalyst_level,
//   institutional_trace, consensusScore,
//   trader_time_horizon, trader_holding_days (optional overrides)
// All inputs have safe defaults when missing
//
// 时间维度定义（与 Period::default_holding_days 一致）：
//   ultra_short → T+1 隔夜 持有天数 2
//   short       → 1-2 周      持有天数 5
//   mid         → 3-8 周      持有天数 28
//   long        → 3 个月+     持有天数 90

// ── 空值保护 ──
let safe_num = |x, default| {
    if type_of(x) == "()" { default } else { x }
};
let safe_str = |x, default| {
    if type_of(x) == "()" { default } else { x }
};

let total_score = safe_num(totalScore, 50.0);
let dqi_score_num = safe_num(dqi_score, 50.0);
let overall_risk_str = safe_str(overall_risk, "中");
let catalyst_level_str = safe_str(catalyst_level, "");
let institutional_trace_str = safe_str(institutional_trace, "");
let consensus_score = safe_num(consensusScore, 50.0);
// trader 结构化参数（可选覆盖）
let trader_time_horizon_raw = safe_str(trader_time_horizon, "");
let trader_holding_days_raw = safe_num(trader_holding_days, 0.0);
let has_trader_input = trader_time_horizon_raw != "" && trader_holding_days_raw > 0.0;

// ── 辅助函数 ──
let consensus_adj = (consensus_score - 50.0) / 100.0 * 10.0;
let dqi_adj = (dqi_score_num - 50.0) / 100.0 * 5.0;

// 风险调整（概率校准版本）：
//   基于 CAPM 框架：高风险=高波动=信号噪声大=置信度应适当下调
//   但风险也意味着潜在回报，所以调整幅度适度对称：
//     低风险(β<0.8) → +2  (防御性，信号可靠度提升)
//     中风险(β~1.0) →  0  (市场平均)
//     高风险(β>1.2) → -4  (信号噪声增大，但回报潜力仍在)
//     极高(β>2.0)   → -8  (极高波动，置信度显著下降)
//   注意：这是"对置信度的修正"而非"对收益的预测"，所以负向偏大是合理的
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

// ── 整体决策（保留原公式） ──
let adjustment = consensus_adj + dqi_adj + risk_adj + cat_bonus + inst_bonus;
let confidence = clamp(total_score + adjustment, 0.0, 100.0);
let base_pos = if risk_adj >= 0.0 { confidence * 0.8 } else { confidence * 0.5 };
let position_pct = clamp(base_pos, 0.0, 100.0);

let overall_action = if confidence >= 80.0 && position_pct >= 30.0 { "增持" }
    else if confidence >= 60.0 { "买入" }
    else if confidence >= 40.0 { "持有" }
    else if confidence < 30.0 { "卖出" }
    else if position_pct < 20.0 { "减持" }
    else { "持有" };

// ── 时间维度推导（加权打分法） ──
// 替代原 if/else 链，改为 4 阶段独立打分 → 选最高分
// 原理：每个时间维度从不同因子获取权重，反映该维度的决策适用性
//
// 长线打分：高置信度 + 高质量数据 + 低风险 → 基本面持有
//   权重: confidence×0.4 + dqi×0.3 + risk_score×0.3
//   最大: 100×0.4 + 100×0.3 + 90×0.3 = 40+30+27 = 97
// 中线打分：高共识 + 中等置信度 → 均衡持仓
//   权重: consensus×0.5 + confidence×0.3 + dqi×0.2
//   最大: 100×0.5 + 100×0.3 + 100×0.2 = 50+30+20 = 100
// 短线打分：催化剂 + 机构痕迹 + 动量 → 5日交易
//   权重: totalScore×0.15 + cat_bonus×3.0 + inst_bonus×3.0 + risk_bonus
//   最大: 100×0.15 + 12×3.0 + 5×3.0 + 15 = 15+36+15+15 = 81
// 超短线打分：强催化剂 + 强机构 + 高风险 → T+1 博弈
//   权重: cat_bonus×5.0 + inst_bonus×5.0 + high_risk_bonus
//   最大: 12×5.0 + 5×5.0 + 20 = 60+25+20 = 105
//   仅在 L2/L3 催化剂 + 机构痕迹 + 高风险时显著，正常市况为 0

let risk_score = switch overall_risk_str {
    "低风险" | "低" => 90.0,
    "高风险" | "高" => 30.0,
    "极高" => 10.0,
    _ => 50.0       // "中" 或未知
};

let long_score = confidence * 0.4 + dqi_score_num * 0.3 + risk_score * 0.3;
let mid_score = consensus_score * 0.5 + confidence * 0.3 + dqi_score_num * 0.2;
let short_score = total_score * 0.15 + cat_bonus * 3.0 + inst_bonus * 3.0 +
    (if is_high_risk { 15.0 } else { 0.0 });
let ultra_short_score = cat_bonus * 5.0 + inst_bonus * 5.0 +
    (if is_high_risk { 20.0 } else { 0.0 });

// trader 结构化参数优先覆盖
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
        "ultra_short" => 2,
        "short" => 5,
        "mid" => 28,
        "long" => 90,
        _ => 5
    }
};

let target_timeframe = switch time_horizon {
    "ultra_short" => "1d-2d",
    "short" => "1w",
    "mid" => "1m",
    "long" => "3m",
    _ => "1m"
};

// ── 分时间维度信号 ──
// 超短线：催化剂 + 资金痕迹权重最高，加共识低权重，容忍高风险
// 修复：total_score 权重从 0.3 提到 0.25，加 consensus_score×0.15，cat/inst 奖金提升
let ultra_short_adj = cat_bonus * 1.5 + inst_bonus * 2.0;
let ultra_short_confidence = clamp(total_score * 0.25 + consensus_score * 0.15 + ultra_short_adj, 0.0, 100.0);
// 最大值: 100*0.25 + 100*0.15 + 12*1.5 + 5*2.0 = 25+15+18+10 = 68，可突破 55 买入阈值
let ultra_short_action = if ultra_short_confidence >= 55.0 { "买入" }
    else if ultra_short_confidence >= 35.0 { "持有" }
    else { "观望" };

// 短期：动量 + 催化剂 + 轻度数据质量
let short_adj = cat_bonus + inst_bonus + dqi_adj * 0.3 + (if is_high_risk { -2.0 } else { 2.0 });
let short_confidence = clamp(total_score * 0.4 + consensus_score * 0.2 + short_adj, 0.0, 100.0);
let short_action = if short_confidence >= 65.0 { "买入" }
    else if short_confidence >= 45.0 { "持有" }
    else { "观望" };

// 中期：共识 + 总分权重最高
let mid_adj = consensus_adj + dqi_adj;
let mid_confidence = clamp(total_score * 0.5 + consensus_score * 0.3 + mid_adj, 0.0, 100.0);
let mid_action = if mid_confidence >= 70.0 { "买入" }
    else if mid_confidence >= 50.0 { "持有" }
    else { "观望" };

// 长期：数据质量 + 低风险 + 总分
let long_adj = dqi_adj + risk_adj.max(0.0);
let long_confidence = clamp(total_score * 0.6 + dqi_score_num * 0.3 + long_adj, 0.0, 100.0);
let long_action = if long_confidence >= 65.0 { "买入" }
    else if long_confidence >= 45.0 { "持有" }
    else { "观望" };

// ── 按时间维度差异化止损/止盈/仓位系数 ──
let stop_loss_pct = switch time_horizon {
    "ultra_short" => 3.0,
    "short" => 5.0,
    "mid" => 8.0,
    "long" => 12.0,
    _ => 5.0
};
let take_profit_pct = switch time_horizon {
    "ultra_short" => 5.0,
    "short" => 10.0,
    "mid" => 18.0,
    "long" => 30.0,
    _ => 10.0
};

// ── 输出 ──
#{
    // 整体决策（向下兼容）
    "action": overall_action,
    "positionPct": position_pct,
    "confidence": confidence,
    "riskLevel": overall_risk_str,
    "stopLossPct": if position_pct > 0.0 { stop_loss_pct } else { 0.0 },
    "takeProfitPct": if position_pct > 0.0 { take_profit_pct } else { 0.0 },
    "reasoning": `确定性决策：总分=${total_score}, 决策=${overall_action}, 置信度=${confidence}, 仓位=${position_pct}%, 风险=${overall_risk_str}, 共识=${consensus_score}`,
    "key_conditions_to_track": [],
    "decisive_bull_acks": [],
    "decisive_bear_acks": [],

    // 多时间维度字段
    "timeHorizon": time_horizon,
    "expectedHoldingDays": expected_holding_days,
    "targetTimeframe": target_timeframe,

    // 各时间段独立信号
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

    "computation_logs": `整体: conf=${total_score}+(con=${consensus_score} dqi=${dqi_adj} risk=${risk_adj} cat=${cat_bonus} inst=${inst_bonus})=${confidence} | 时间=${time_horizon}, 持有=${expected_holding_days}天 | 超短=${ultra_short_confidence} 短=${short_confidence} 中=${mid_confidence} 长=${long_confidence}`
}
