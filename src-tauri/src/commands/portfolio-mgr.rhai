// portfolio-mgr.rhai — 贝叶斯因子融合 + 凯利仓位
//
// 理论依据：
//   贝叶斯更新: P(上涨|证据) ∝ P(证据|上涨) × P(上涨)
//   凯利公式:   f* = (p × odds - q) / odds
//     p = P(上涨|证据)   q = 1-p
//     odds = 预期收益% / 预期亏损%
//
// 数据驱动（P1/P2）：
//   - 先验 P(上涨) = t-scoring 回测的市场状态（牛市0.55/熊市0.45/中性0.50）
//   - 因子权重 = t-scoring 回测的信息系数(IC)归一化
//
// 缺失因子不参与融合（不编造数据）。

fn present(x) { type_of(x) != "()" }

// ── 先验：数据驱动的市场状态 ──
let prior = if present(market_regime_prior) {
    market_regime_prior
} else { 0.50 };

// ── 因子权重：数据驱动的 IC 归一化 ──
// factor_weights 是 JSON 对象 {trend:{weight,win_rate,ic}, macd:{...}, ...}
// 从 Rhai 的 Map 中读取各因子权重
let f_weights = if present(factor_weights) && type_of(factor_weights) == "map" {
    factor_weights
} else { #{} };  // 空 map 触发等权 fallback

// 尝试从 factor_weights 读取权重；找不到则用等权 fallback
fn get_weight(weights, factor, fallback) {
    if type_of(weights) == "map" {
        let f = weights[factor];
        if type_of(f) != "()" && type_of(f) == "map" {
            let w = f["weight"];
            if type_of(w) == "f64" { w } else { fallback }
        } else { fallback }
    } else { fallback }
}

// ── 因子 1: 技术面 ──
let f1_default = 0.15;  // V49 修复: 从 0.25 下调 — 技术面(趋势跟踪)必然滞后,
                          // 股价涨完才触发"多头排列", 不应占最高权重
let f1_weight = if present(totalScore) { get_weight(f_weights, "trend", f1_default) } else { 0.0 };
let f1_signal = if present(totalScore) { (totalScore - 50.0) / 50.0 } else { 0.0 };

// ── 因子 2: 共识 ──
let f2_default = 0.25;
// V29 修复: 共识因子应使用独立的回测权重键 "consensus"，而非复用 "trend"
// factor_weights 结构为 {trend:{...}, macd:{...}, rsi:{...}, consensus:{...}, ...}
// 若回测数据无 "consensus" 键，get_weight 会 fallback 到 f2_default
let f2_weight = if present(consensusScore) { get_weight(f_weights, "consensus", f2_default) } else { 0.0 };
let f2_signal = if present(consensusScore) { (consensusScore - 50.0) / 50.0 } else { 0.0 };

// ── 因子 3: 催化剂 ──
// V37 修复: 催化剂信号来自 LLM（a-catalyst.content.catalyst_level），与技术面回测因子
// （volume=成交量）语义无关，不调用 get_weight 避免权重被无关回测数据绑架。
// 仅在 catalyst_level 非空且非"无"时激活默认权重；"无"表示 LLM 明确判断无催化剂。
// V38 修复: 扩展催化剂信号为双向（利好+利空），利空催化剂（退市预警/商誉爆雷等）
// 输出负信号拉低 posterior，避免对"有催化剂但实质利空"的股票给出偏多建议。
let f3_default = 0.20;
let f3_has_catalyst = present(catalyst_level) && catalyst_level != "" && catalyst_level != "无";
let f3_weight = if f3_has_catalyst { f3_default } else { 0.0 };
let f3_signal = if f3_weight > 0.0 {
    // V40 修复: 用 contains 而非精确 switch 匹配，保证与 prompt 字段值对齐
    // prompt 输出格式: "L3估值体系级" / "L2业绩拐点级" / "L1普通消息"
    //                   "L-3退市/造假级" / "L-2业绩暴雷级" / "L-1普通利空"
    if catalyst_level.contains("L-3") || catalyst_level.contains("退市") || catalyst_level.contains("造假") { -0.8 }
    else if catalyst_level.contains("L-2") || catalyst_level.contains("业绩暴雷") { -0.5 }
    else if catalyst_level.contains("L-1") || catalyst_level.contains("普通利空") { -0.2 }
    else if catalyst_level.contains("L3") || catalyst_level.contains("估值体系级") { 0.8 }
    else if catalyst_level.contains("L2") || catalyst_level.contains("业绩拐点级") { 0.5 }
    else if catalyst_level.contains("L1") || catalyst_level.contains("普通消息") { 0.2 }
    else { 0.0 }
} else { 0.0 };

// ── 因子 4: 风险 ──
// V37 修复: 风险信号来自 LlmClassifier（risk-level.category），与技术面回测因子
// （rsi=RSI超买超卖）语义无关，直接用默认权重，避免被 RSI 回测数据绑架。
//
// V50 修复: 优化 f4_signal 映射
//   旧: 低→+0.5 中→0.0 高→-0.4 极高→-0.7
//   问题①: 中→0.0 意味着"中风险=无风险信号"，逻辑矛盾
//   问题②: 低→中跳变 0.5 但高→极高只跳 0.3，不对称
//   新: 基于风险等级在 0-100 上的位置线性映射再反转
//      低(20)→+0.40 中(50)→-0.10 高(80)→-0.40 极高(95)→-0.65
//   原则: 中风险≠无风险，应有轻微负信号；跳变更均匀
let f4_default = 0.15;
let f4_weight = if present(overall_risk) { f4_default } else { 0.0 };
let f4_signal = if f4_weight > 0.0 {
    switch overall_risk {
        "低风险"|"低" => 0.30,
        "中风险"|"中" => -0.05,   // V51: 中风险轻微负信号（原-0.10）
        "高风险"|"高" => -0.25,   // V51: 降低过度惩罚（原-0.40→-0.25）
        "极高风险"|"极高" => -0.50, // V51: 降低极端惩罚（原-0.65→-0.50）
        _ => 0.0
    }
} else { 0.0 };

// ── 因子 5: 估值风险 ──
// V51 新增: 估值风险信号来自 t-valuation（DCF/格雷厄姆上行空间）
//   估值过高 = 风险（负信号），估值低估 = 机会（正信号）
//   与 f4_signal（技术面/基本面风险）正交，独立衡量估值维度。
let f5_default = 0.15;
// 激活条件: t-valuation 已执行且输出有效（dcf_upside 或 graham_upside 存在）
let f5_has_valuation = present(valuation_dcf_upside) || present(valuation_graham_upside);
let f5_weight = if f5_has_valuation { f5_default } else { 0.0 };
let f5_signal = if f5_weight > 0.0 {
    // 优先用 DCF 上行空间（理论内在价值 - 当前价）/ 当前价
    // dcf_upside: 百分数（如 20.5 表示上行空间 20.5%），需要除以 100 转为十进制
    // 用 clamp 把上行空间映射到 [-1, 1]：低估→正，过高→负
    let dcf_sig = if present(valuation_dcf_upside) {
        clamp(valuation_dcf_upside / 50.0, -1.0, 1.0)
    } else { 0.0 };
    let graham_sig = if present(valuation_graham_upside) {
        clamp(valuation_graham_upside / 50.0, -1.0, 1.0)
    } else { 0.0 };
    // F-Score 加成: score >= 7（财务强劲）→ 估值信号可信度更高，信号放大 1.3x
    let fscore_mult = if present(valuation_fscore) && valuation_fscore >= 7 { 1.3 } else { 1.0 };
    let raw_sig = if dcf_sig.abs() > graham_sig.abs() { dcf_sig } else { graham_sig };
    clamp(raw_sig * fscore_mult, -1.0, 1.0)
} else { 0.0 };

// ── 因子 6: 数据质量 ──
// V37 修复: 数据质量信号来自 AgentNode（data-quality.content.score），与技术面回测因子
// （bias=乖离率）语义无关，直接用默认权重，避免被乖离率回测数据绑架。
let f6_default = 0.15;
let f6_weight = if present(dqi_score) { f6_default } else { 0.0 };
let f6_signal = if f6_weight > 0.0 { (dqi_score - 50.0) / 50.0 } else { 0.0 };

// ── 因子 7: 交易方案方向与质量 ──
// V47 修复: 旧逻辑只看 targetPrice/stopLoss 比值，完全忽略 targetPrice vs currentPrice 的方向。
//   例: currentPrice=20 targetPrice=8 stopLoss=7 → ratio=1.14 → f7_signal=+0.1(正向!)
//   但 targetPrice 远低于 currentPrice 是明确的空头信号，应产生负信号。
//
// 新逻辑（两级判断）:
//   第一级: targetPrice vs currentPrice 方向
//     - targetPrice > currentPrice*1.05 → 多头方向，信号强度=涨幅×缩放
//     - targetPrice < currentPrice*0.95 → 空头方向，信号强度=跌幅×缩放(强制负)
//     - 中间 → 中性 0.0
//   第二级: targetPrice vs stopLoss 质量（仅多头方向生效）
//     - ratio > 1.15 → 加成；ratio < 1.0 → 衰减
//   无 current_price 时回退到旧逻辑（仅看比值）
let f7_weight = if present(trader_target_price) && present(trader_stop_loss)
    && trader_stop_loss > 0.0 { 0.30 } else { 0.0 };
	let f7_signal = if f7_weight > 0.0 {
	    if present(current_price) && current_price > 0.0 {
	        let upside = (trader_target_price - current_price) / current_price;
	        if upside > 0.05 {
	            // 多头方向: 目标价高于现价 → 正信号，信号强度与涨幅成正比
	            let ratio = trader_target_price / trader_stop_loss;
	            // base: 涨幅的1.5倍缩放(上限0.8), ratio加成: >1.15额外+0.1
	            let base = clamp(upside * 1.5, 0.05, 0.8);
	            if ratio > 1.15 { clamp(base + 0.1, 0.1, 1.0) } else { base }
	        } else if upside < -0.05 {
	            // 空头方向: 目标价低于现价 → 强制负信号，幅度与跌幅成正比
	            // 例: currentPrice=20 targetPrice=8 → upside=-0.60 → signal=-0.80
	            clamp(upside * 1.3, -1.0, -0.15)
	        } else {
	            // 目标价≈现价（±5%），中性
	            0.0
	        }
	    } else {
	        // 无现价时回退: 仅看 targetPrice/stopLoss 比值
	        let ratio = trader_target_price / trader_stop_loss;
	        if ratio > 1.15 { (ratio - 1.0) / 1.0 } else if ratio < 0.85 { -0.5 } else if ratio > 1.0 { 0.1 } else { 0.0 }
	    }
	} else { 0.0 };
// ── 数据 sanity 预检（必须在 position_pct/final_action 之前定义）──
let trader_data_valid = true;
let sanity_reason = "";

// 检查 1: targetPrice <= stopLoss → 无意义的交易方案
if present(trader_target_price) && present(trader_stop_loss) && trader_stop_loss > 0.0 {
    if trader_target_price <= trader_stop_loss {
        trader_data_valid = false;
        sanity_reason = `targetPrice(${trader_target_price})<=stopLoss(${trader_stop_loss})`;
    }
}

// V47 新增检查 1.5: targetPrice 显著低于 currentPrice → 空头预测（做多无意义）
// 与检查2不同: 检查2只标记"偏离>70%"为数据异常（可能是LLM幻觉），
// 而此处标记"明确看空"为逻辑不一致——trader预测下跌但系统仍给出持仓建议。
// 阈值15%: 低于此值可能是短期波动，高于此值应触发空头处理。
let trader_bearish = false;
let bearish_magnitude = 0.0;
if trader_data_valid && present(current_price) && present(trader_target_price) && current_price > 0.0 {
    let upside = (trader_target_price - current_price) / current_price;
    if upside < -0.15 {
        trader_bearish = true;
        bearish_magnitude = upside;
    }
}

// 检查 2: 有 current_price 时，targetPrice 偏离超过 70% → 数据异常
if trader_data_valid && present(current_price) && present(trader_target_price) && current_price > 0.0 {
    let deviation = (trader_target_price - current_price) / current_price;
    if deviation.abs() > 0.70 {
        trader_data_valid = false;
        sanity_reason = `targetPrice(${trader_target_price})偏离currentPrice(${current_price})超过70%`;
    }
}

// 检查 3: stopLoss < 1 元或 targetPrice < 1 元 → 垃圾数据
if trader_data_valid {
    if present(trader_target_price) && trader_target_price < 1.0 {
        trader_data_valid = false;
        sanity_reason = `targetPrice(${trader_target_price})<1元`;
    }
    if present(trader_stop_loss) && trader_stop_loss < 1.0 {
        trader_data_valid = false;
        sanity_reason = `stopLoss(${trader_stop_loss})<1元`;
    }
}

// ── 贝叶斯融合 ──
// V51: 因子数 7（f1~f7），新增 f5=估值风险 f6=数据质量 f7=交易方案
let total_weight = f1_weight + f2_weight + f3_weight + f4_weight + f5_weight + f6_weight + f7_weight;
let weighted_signal = f1_weight*f1_signal + f2_weight*f2_signal
                    + f3_weight*f3_signal + f4_weight*f4_signal
                    + f5_weight*f5_signal + f6_weight*f6_signal
                    + f7_weight*f7_signal;
let avg_signal = if total_weight > 0.0 { weighted_signal / total_weight } else { 0.0 };
// V38 修复: 证据数量自适应缩放 — 证据越多（total_weight 越大），对先验的修正力度越强。
// 单因子（total_weight≈0.20）时缩放 0.20（保守），全因子（total_weight≈1.20）时缩放 0.50。
// 固定 0.35 使单因子和多因子修正力度相同，违反贝叶斯"更多证据→更大更新"的核心思想。
// V40 修复: 单因子（total_weight<0.3）时 evidence_scale 降到 0.15，
// 避免单个强信号因子（如 catalyst=L3）完全主导贝叶斯更新。
// 全因子（total_weight≈1.20）时 0.50，正常多因子信号充分。
let evidence_scale = if total_weight < 0.3 {
    0.15
} else {
    clamp(0.20 + total_weight * 0.25, 0.20, 0.50)
};
	// V49 修复: trader 否决权 — 技术面（滞后指标）不应覆盖 LLM trader 的实时判断
	// f1(技术评分) 是趋势跟踪指标，必然滞后。当 trader 根据最新信息判断看空时，
	// posterior 不应被技术面推高到"增持/买入"区。
	// 否决规则（仅在 trader 数据有效且 f7 信号强烈时生效）:
	//   f7_signal < -0.5（强烈看空）→ posterior 封顶 0.48（最高"持有"）
	//   f7_signal < -0.2（温和看空）→ posterior 封顶 0.53（最高"增持"，会被风险否决降级）
	let posterior_cap = if trader_data_valid && f7_weight > 0.0 && present(f7_signal) {
	    if f7_signal < -0.5 { 0.48 }
	    else if f7_signal < -0.2 { 0.53 }
	    else { 1.0 }
	} else { 1.0 };
	let raw_val = prior + avg_signal * evidence_scale;
	let posterior = clamp(if raw_val > posterior_cap { posterior_cap } else { raw_val }, 0.0, 1.0);

	// ── V42+V44: 风险的多维度处理 ──
	// 风险有两个独立维度, 分别处理:
	//   维度1 (方向): 风险作为看空因子 → f4_signal 在贝叶斯里拉低 posterior (已做)
	//   维度2 (操作): risk_bias 左移 action 阈值 + V43 硬性否决 (见下方)
	//
	// 注意: confidence 不受风险 cap (V44 曾错误 cap, 已回滚)。
	//   权威定义 (Nature Neurosci 2016): confidence = P(决策正确 | 证据), 是后验概率。
	//   posterior 已通过 f4_signal 综合风险因子, confidence = posterior × 100 即可。
	//   "高风险 + 高置信度看多 + action=持有" 是自洽的风险管理, 不是矛盾。
	let risk_bias = if present(overall_risk) {
	    switch overall_risk {
	        "极高" => -0.15,      // 极高风险 → 卖出门槛下降
	        "高风险"|"高" => -0.08, // 高风险 → 减持门槛下降
	        "中风险"|"中" => 0.0,
	        "低风险"|"低" => 0.05,
	        _ => 0.0
	    }
	} else { 0.0 };
	let effective_posterior = clamp(posterior + risk_bias, 0.0, 1.0);

	// ── V47 修复: 凯利赔率使用标准盈亏比 ──
	// 旧: odds = targetPrice / stopLoss（仅看两个绝对价格比值，忽略 currentPrice）
	//   例: currentPrice=20 targetPrice=8 stopLoss=7 → odds=1.14 → 算出正仓位
	//   但 targetPrice < currentPrice 意味着做多无盈利空间，赔率应为 0。
	// 新: odds = (targetPrice - currentPrice) / (currentPrice - stopLoss)
	//   即 潜在收益 / 潜在亏损，仅在 targetPrice > currentPrice 时 > 0。
	//   currentPrice=20 targetPrice=25 stopLoss=18 → odds=(25-20)/(20-18)=2.5 ✓
	//   currentPrice=20 targetPrice=8  stopLoss=7  → targetPrice<currentPrice → odds=0 ✓
	//   赔率上限 10（避免极端值主导 Kelly）
	let odds = if present(trader_target_price) && present(trader_stop_loss) && present(current_price)
	    && trader_stop_loss > 0.0 && current_price > trader_stop_loss
	    && trader_target_price > current_price {
	    let profit = trader_target_price - current_price;
	    let loss = current_price - trader_stop_loss;
	    if loss > 0.0 { clamp(profit / loss, 0.0, 10.0) } else { 0.0 }
	} else { 0.0 };
	let q = 1.0 - posterior;
	let kelly_raw = if posterior >= 0.50 && odds > 0.001 {
	    (posterior * odds - q) / odds
	} else { 0.0 };
	let kelly_negative = kelly_raw <= 0.0001;
	// 凯利仓位（风险限制上限）
	let base_position_pct = if posterior < 0.50 || kelly_negative {
	    0.0
	} else {
	    let kelly_pos = clamp(kelly_raw / 2.0 * 100.0, 0.0, 95.0);
	    if present(overall_risk) && overall_risk == "极高" {
	        clamp(kelly_pos, 0.0, 10.0)   // 极高风险: 仓位≤10%
	    } else if present(overall_risk) && (overall_risk == "高风险" || overall_risk == "高") {
	        clamp(kelly_pos, 0.0, 35.0)   // 高风险: 仓位≤35%
	    } else if present(overall_risk) && (overall_risk == "中风险" || overall_risk == "中") {
	        clamp(kelly_pos, 0.0, 50.0)   // 中风险: 仓位≤50%
	    } else {
	        kelly_pos
	    }
	};
	let position_pct = if !trader_data_valid { 0.0 } else { base_position_pct };
	// ── 置信度 = P(决策正确 | 证据) = posterior × 100 ──
	// 权威定义 (Nature Neurosci 2016, "Confidence and certainty: distinct probabilistic quantities"):
	//   confidence = P(decision correct | evidence), 是后验概率, 与特定决策绑定。
	//   certainty  = 对变量估计的概率分布宽度 (另一个概念, 这里不用)。
	//
	// 关键: 风险已经通过 f4_signal 进入贝叶斯融合拉低 posterior (方向维度)。
	//   posterior = P(上涨 | 证据) 已综合风险因子, confidence = posterior × 100
	//   在数学和语义上都正确, 不需要再按风险 cap。
	//
	// V44 曾错误地按风险 cap confidence (基于"高风险→低置信"的错误推理),
	//   混淆了 risk (输入证据/收益离散度) 和 confidence (对决策正确的后验概率)。
	//   "高风险 + 高置信度看多 + action=持有" 是自洽的风险管理:
	//   我70%确定会涨, 但风险大所以不加仓。V43 的 action 否决已足够。
	//
	// V50 修复: 接入 risk-convergence 的三方分歧度。
	//   当 risk_disagreement 存在且高时(>50), 三个风险评估师分歧大,
	//   说明风险评估本身不确定性高, 应降低最终置信度。
	//   这是对"风险评估的可靠性"的独立修正，与 f4_signal(风险方向)正交。
	//   分歧度 0~100, 修正因子 = (100 - disagreement) / 100, 范围 [0.3, 1.0]
	let disagreement_mod = if present(risk_disagreement) {
	    clamp((100.0 - risk_disagreement) / 100.0, 0.3, 1.0)
	} else { 1.0 };
	let final_confidence = clamp(posterior * 100.0 * disagreement_mod, 0.0, 100.0);

	// ── M2 修复：LLM 失败级联兜底 ──
	let weights_collapsed = total_weight < 0.3;
	if weights_collapsed {
	    position_pct = 0.0;
	    final_confidence = clamp(final_confidence / 2.0, 0.0, 100.0);
	}

	// ── V42 重构: action 阈值使用 effective_posterior(=posterior+risk_bias) ──
	// 买入 ← 0.63+仓位15%+
	// 增持 ← 0.53+仓位10%+
	// 持有 ← 0.48-0.53（模糊下界）
	// 观望 ← 0.38-0.50（模糊上界+下界）
	// 减持 ← 0.30-0.40（模糊上界）
	//   卖出 ← <0.30
	let base_action = "卖出";
	// 买入区（effective_posterior≥0.63 且仓位充足）
	if effective_posterior >= 0.63 && position_pct >= 15.0 {
	    base_action = "买入";
	// 增持区（effective_posterior≥0.53 且仓位达标）
	} else if effective_posterior >= 0.53 && position_pct >= 10.0 {
	    base_action = "增持";
	// 持有区（effective_posterior≥0.48，与"观望"的模糊边界）
	} else if effective_posterior >= 0.48 {
	    base_action = "持有";
	// 观望区（posterior 0.38-0.50，与"减持"的模糊边界）
	} else if effective_posterior >= 0.38 {
	    base_action = "观望";
	// 减持区（posterior 0.30-0.40，与"卖出"的模糊边界）
	} else if effective_posterior >= 0.30 {
	    base_action = "减持";
	}

	// ── 最终 action：sanity 不通过或权重坍缩则强制降级 ──
	let final_action = if !trader_data_valid {
	    // 数据异常时：根据 posterior 决定是"观望"还是"卖出"
	    if posterior < 0.45 { "卖出" } else { "观望" }
	} else if weights_collapsed {
	    // 权重坍缩：大部分因子缺失，强制"观望"（不给买入/增持/减持方向信号）
	    "观望"
	} else if position_pct <= 0.0 && (base_action == "买入" || base_action == "增持" || base_action == "持有") {
	    // V42: 仓位为 0 时不应给出持仓性建议。若 base_action 暗示持仓但实际空仓，
	    // 降级为"观望"（后验仍可保留，但行动上有零仓位匹配）
	    "观望"
	} else { base_action };

	// ── V43 修复: 风险等级硬性 action 约束（风控否决权）──
	// 根因: risk_bias=-0.08 只是软性偏移, posterior=0.7 时 effective_posterior=0.62
	//   仍落入增持区(≥0.53), 导致"高风险+增持"的逻辑矛盾。
	//   风险等级必须拥有 action 否决权, 不能仅靠阈值偏移。
	// 规则:
	//   高风险 → 禁止"买入/增持"(加仓=放大高风险敞口, 违背风控), 降级为"持有"
	//   极高风险 → 禁止"买入/增持/持有"(极高风险不应继续持仓), 降级为"观望"
	// 注意: 必须在仓位对齐之前执行, 使降级后的 action 正确触发仓位调整
	let risk_downgraded = false;
	let risk_downgrade_note = "";
	if present(overall_risk) {
	    if overall_risk == "极高" && (final_action == "买入" || final_action == "增持" || final_action == "持有") {
	        risk_downgraded = true;
	        risk_downgrade_note = ` | ⚠️极高风险风控否决:${base_action}→观望(极高风险不应继续持仓)`;
	        final_action = "观望";
	    } else if (overall_risk == "高风险" || overall_risk == "高") && (final_action == "买入" || final_action == "增持") {
	        risk_downgraded = true;
	        risk_downgrade_note = ` | ⚠️高风险风控否决:${base_action}→持有(高风险禁止加仓)`;
	        final_action = "持有";
	    }
	}

	// ── V47 新增: 空头 trader 预测否决 ──
	// 当 trader 预测 targetPrice 显著低于 currentPrice（≥15%）时，意味着系统自身
	// 的交易员认为股价将大幅下跌。此时给出"持有"或更积极的建议是逻辑矛盾。
	// 规则: 空头预测 + 持仓建议 → 强制卖出
	// 排除: 已经"卖出"或"减持"的不再降级；"观望"降为"卖出"（明确看空不应观望）
	let bearish_downgraded = false;
	let bearish_downgrade_note = "";
	if trader_bearish {
	    if final_action == "买入" || final_action == "增持" || final_action == "持有" {
	        bearish_downgraded = true;
	        bearish_downgrade_note = ` | ⚠️空头预测否决:targetPrice(${trader_target_price})<现价(${current_price})${(bearish_magnitude*100.0).to_int()}%,${final_action}→卖出`;
	        final_action = "卖出";
	    } else if final_action == "观望" {
	        bearish_downgraded = true;
	        bearish_downgrade_note = ` | ⚠️空头预测否决:targetPrice(${trader_target_price})<现价(${current_price})${(bearish_magnitude*100.0).to_int()}%,${final_action}→卖出`;
	        final_action = "卖出";
	    }
	    // "减持"/"卖出"保持不动（已经是看空方向）
	}

	// ── 仓位与 action 对齐（V42）: action 决定了仓位性质 ──
	// 如果 action 是减持/卖出，仓位应趋于零或极低
	if final_action == "卖出" {
	    position_pct = 0.0;
	} else if final_action == "减持" && position_pct > 15.0 {
	    position_pct = 15.0;   // 减持通道，仓位不超15%
	} else if final_action == "观望" && position_pct > 0.0 {
	    position_pct = 0.0;     // 观望=不持仓，清零目标仓位
	}

	let contradiction_note = if !trader_data_valid {
	    ` | ⚠️trader数据异常:${sanity_reason},action已从${base_action}修正为${final_action}`
	} else if position_pct <= 0.0 && base_action != final_action {
	    ` | ⚠️零仓位修正:${base_action}→${final_action}(仓位0%,不可持股)`
	} else if final_action == "观望" || final_action == "减持" || final_action == "卖出" {
	    ` | ⚠️仓位修正:action=${final_action}仓位调至${position_pct}%`
	} else { "" };

	// M2 兜底：权重坍缩时在 reasoning 中标记
	let collapse_note = if weights_collapsed {
	    ` | ⚠️因子权重坍缩(${(total_weight*100.0).to_int()}%),强制观望+空仓`
	} else { "" };

	// V43 修复: position_pct 用 clamp(只压不抬), 避免 risk_note 把凯利低仓位
	//   (如 21%)强行抬高到风险上限(35%), 违背"高风险限制仓位上限"的语义。
	//   风控是上限不是强制值, 低仓位不应被抬高。
	// V43: 去掉"action阈值左移"描述, 因风险 action 约束已改为硬性否决(见上方 risk_downgrade)
	let risk_note = if present(overall_risk) && overall_risk == "极高" {
	    if position_pct > 10.0 { position_pct = 10.0; }
	    ` | ⚠️极高风险:仓位≤10%`
	} else if present(overall_risk) && (overall_risk == "高风险" || overall_risk == "高") {
	    if position_pct > 35.0 { position_pct = 35.0; }
	    ` | ⚠️高风险:仓位≤35%`
	} else if present(overall_risk) && (overall_risk == "中风险" || overall_risk == "中") && position_pct > 50.0 {
	    position_pct = 50.0;
	    ` | ⚠️中风险:仓位上限50%`
	} else { "" };

// ── 缺失数据 ──
let data_gaps = [];
if !present(totalScore) { data_gaps.push("技术服务评分(t-scoring)"); }
if !present(consensusScore) { data_gaps.push("辩论共识分数(debate-convergence)"); }
if !present(catalyst_level) || catalyst_level == "" { data_gaps.push("催化剂评估(a-catalyst)"); }
if !present(overall_risk) { data_gaps.push("风险等级(risk-level)"); }
// V51: evidence_pct 分母用 7 因子的理论最大权重和(1.50)
// 7 因子默认权重: f1=0.25 + f2=0.25 + f3=0.20 + f4=0.15 + f5=0.15 + f6=0.15 + f7=0.30 = 1.50
let max_weight = 0.25 + 0.25 + 0.20 + 0.15 + 0.15 + 0.15 + 0.30;
let evidence_pct_raw = (total_weight / max_weight * 100.0).to_int();
let evidence_pct = if evidence_pct_raw > 100 { 100 } else { evidence_pct_raw };

// ── 时间 ──
let time_horizon = if present(trader_time_horizon) && present(trader_holding_days)
    && trader_holding_days > 0.0 { trader_time_horizon } else if posterior >= 0.65 { "long" } else if posterior >= 0.50 { "mid" } else if posterior >= 0.35 { "short" } else { "ultra_short" };
let expected_holding_days = if present(trader_holding_days) && trader_holding_days > 0.0 {
    trader_holding_days.to_int()
} else { switch time_horizon { "ultra_short"=>2, "short"=>5, "mid"=>28, "long"=>90, _=>5 } };
let target_timeframe = switch time_horizon {
    "ultra_short"=>"1d-2d", "short"=>"1w", "mid"=>"1m", "long"=>"3m", _=>"1m"
};

// ── 推理 ──
let prior_src = if present(market_regime_state) { `(${market_regime_state})` } else { "" };
let gap_note = if data_gaps.len() > 0 {
    ` | 缺口(${data_gaps.len()}) 证据=${evidence_pct}%`
} else { "" };
let bayes_note = ` | 先验=${prior}${prior_src} 后验=${(posterior*100.0).round()/100.0}`;
let kelly_note = ` | 赔率=${(odds*10.0).round()/10.0} 凯利=${(kelly_raw*1000.0).round()/1000.0}`;
// V50: 分歧度修正标注（仅在 disagreement 显著时标记）
let disagreement_note = if present(risk_disagreement) && risk_disagreement > 50 {
    ` | ⚠️风险分歧(${risk_disagreement})置信修正×${(disagreement_mod*100.0).round().to_int()}%`
} else { "" };

// ── 输出 ──
	#{
    "action": final_action,
    "positionPct": (position_pct * 10.0).round() / 10.0,
    "confidence": (final_confidence * 10.0).round() / 10.0,
    "riskLevel": if present(overall_risk) { overall_risk } else { "未知" },
    "stopLossPct": if position_pct > 0.0 { switch time_horizon {
        "ultra_short"=>3.0,"short"=>5.0,"mid"=>8.0,"long"=>12.0,_=>5.0
    }} else { 0.0 },
    "takeProfitPct": if position_pct > 0.0 { switch time_horizon {
        "ultra_short"=>5.0,"short"=>10.0,"mid"=>18.0,"long"=>30.0,_=>10.0
    }} else { 0.0 },
    "reasoning": `决策=${final_action} 置信=${final_confidence} 仓位=${position_pct}%${gap_note}${bayes_note}${kelly_note}${disagreement_note}${contradiction_note}${risk_downgrade_note}${bearish_downgrade_note}${risk_note}${collapse_note}`,
    "data_gaps": data_gaps,
    "timeHorizon": time_horizon,
    "expectedHoldingDays": expected_holding_days,
    "isContradictory": present(trader_target_price) && present(trader_stop_loss) && trader_stop_loss > 0.0 && trader_target_price <= trader_stop_loss,
    "targetTimeframe": target_timeframe,
    "evidence": #{
        "prior": prior, "posterior": (posterior*1000.0).round()/1000.0,
        "avgSignal": (avg_signal*100.0).round()/100.0,
        "activeFactors": total_weight, "totalWeight": total_weight,
    },
    "computation_logs": `prior=${prior}${prior_src} post=${(posterior*100.0).round()/100.0} sig=${(avg_signal*100.0).round()/10.0} w=${total_weight} odds=${(odds*10.0).round()/10.0} k=${(kelly_raw*1000.0).round()/1000.0} pos=${position_pct}`
}
