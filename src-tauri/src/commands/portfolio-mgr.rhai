// portfolio-mgr.rhai - deterministic decision formula
// Input via input_mapping: totalScore, dqi_score, overall_risk, catalyst_level,
//   institutional_trace, consensusScore
// All inputs have safe defaults when missing

let total_score = totalScore;
let consensus_adj = (consensusScore - 50.0) / 100.0 * 10.0;
let dqi_adj = (dqi_score - 50.0) / 100.0 * 5.0;

let risk_adj = switch overall_risk {
    "低" => 5.0,
    "高" => -5.0,
    "极高" => -10.0,
    _ => 0.0
};

let cat_bonus = switch catalyst_level {
    "L3估值体系级" => 12.0,
    "L2业绩拐点级" => 6.0,
    "L1普通消息" => 2.0,
    _ => 0.0
};

let inst_bonus = if institutional_trace == "有建仓痕迹" || institutional_trace == "疑似建仓" { 5.0 } else { 0.0 };

let adjustment = consensus_adj + dqi_adj + risk_adj + cat_bonus + inst_bonus;
let confidence = clamp(total_score + adjustment, 0.0, 100.0);

let base_pos = if risk_adj >= 0.0 { confidence * 0.8 } else { confidence * 0.5 };
let position_pct = clamp(base_pos, 0.0, 100.0);

let decision = if confidence >= 80.0 && position_pct >= 30.0 { "增持" }
    else if confidence >= 60.0 { "买入" }
    else if confidence >= 40.0 { "持有" }
    else if position_pct < 10.0 { "减持" }
    else { "持有" };

#{
    "decision": decision,
    "positionPct": position_pct,
    "confidence": confidence,
    "riskLevel": overall_risk,
    "stopLossPct": if position_pct > 0.0 { 8.0 } else { 0.0 },
    "takeProfitPct": if position_pct > 0.0 { 15.0 } else { 0.0 },
    "reasoning": `deterministic: totalScore=${total_score}, dqi=${dqi_score}, risk=${overall_risk}, catalyst=${catalyst_level}, consensus=${consensusScore}, adj=${adjustment}, conf=${confidence}, pos=${position_pct}`,
    "key_conditions_to_track": [],
    "decisive_bull_acks": [],
    "decisive_bear_acks": [],
    "computation_logs": `conf=${total_score}+(con=${consensusScore} dqi=${dqi_adj} risk=${risk_adj} cat=${cat_bonus} inst=${inst_bonus})=${confidence}`
}
