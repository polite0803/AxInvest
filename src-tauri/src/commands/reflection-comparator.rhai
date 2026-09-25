// reflection-comparator.rhai — 反思工作流：预测 vs 实际定量对比 + untrusted 扫描 (P1-1/P0-2)
//
// 在 sub-analysis 之后、reflection-agent 之前执行。替代原 reflection_untrusted_aggregator.rhai。
// 对比分析师预测与实际走势 + 扫描上游 __untrusted 标记，输出结构化偏差报告。
//
// 设计原则：不依赖 LLM 解析，全确定性计算。
//
// [2026-09-13 实际行情改造] 新增「价格层对比」：
//   —— 反思的对比基准由后端 `compute_market_snapshots()` 用**前复权 K 线**确定性算出
//   （分析日 → 最新交易日），经 `actual_market_json` 变量注入。
//   此前本脚本只消费一个标量 `raw_return_pct`，且批量入口长期传 0.0
//   ⇒ `actual_direction` 恒「横盘」、`direction_match` 恒 false，
//   反思 agent 实际是在对假数据写结论。现在价格、回撤、目标价实现度全部可见。
//
// 输入映射（由 DAG input_mapping 注入）：
//   trader_action       — 交易员最终动作（buy/hold/sell 等）
//   trader_target_price — 目标价（sub-analysis 记忆，可选）
//   trader_confidence   — 交易员置信度（可选）
//   portfolio_action    — 组合投资经理最终 action
//   portfolio_posterior — posterior 概率
//   debate_consensus    — 辩论共识度（0-1）
//   total_score         — 技术评分（0-100）
//   raw_return_pct      — 实际净收益率 %（扣双边成本，来自行情快照）
//   alpha_return_pct    — 相对基准超额 %
//   holding_days        — 实际跨度交易日数
//   original_time_horizon — 原始决策时间维度
//   original_holding_days — 原始期望持有天数
//   latest_price        — 最新收盘价（当前实际行情）
//   entry_price         — 入场基准价（分析日次一交易日开盘）
//   target_price        — 决策目标价（行情快照侧，优先于 trader_target_price）
//   target_progress_pct — 目标价实现度 %
//   max_drawdown_pct    — 期间最大回撤 %
//   period_high/low     — 区间最高/最低价
//   latest_date         — 最新交易日
//   within_expected_horizon — 是否仍未到期望持有期（期中观察标记）
//   u_trader            — trader 节点 __untrusted 标记
//   u_research_mgr      — research-mgr 节点 __untrusted 标记
//   u_catalyst          — a-catalyst 节点 __untrusted 标记
//   u_debate_cnv        — debate-convergence 节点 __untrusted 标记
//   u_data_quality       — data-quality 节点 __untrusted 标记
//   u_risk_cnv          — risk-convergence 节点 __untrusted 标记
//
// 输出结构（deviation_report map）：
//   direction_match      — bool: 预测方向与实际方向是否一致
//   confidence_vs_return — string: 置信度 vs 实际收益的描述
//   return_category      — string: 收益分类
//   horizon_mismatch     — string: 时间维度匹配度
//   deviation_summary    — string: 单句偏差总结（≤200 字符）
//   key_findings         — array: 关键发现列表
//   price_context        — map: 价格层事实（入场/最新/目标价/回撤/实现度）
//   untrusted_count      — i64: 不可信节点数
//   untrusted_sources    — array: 不可信节点名列表
//   sub_analysis_healthy — bool: 子工作流整体健康度

fn is_unit(x) { type_of(x) == "()" }

// ── 四周期 horizon_correct 辅助函数（批次 3）──
fn is_bull(action) {
    action == "买入" || action == "增持" || action == "持有"
    || action == "BUY" || action == "买进" || action == "建仓"
}
fn is_bear(action) {
    action == "卖出" || action == "减持"
    || action == "SELL" || action == "卖空" || action == "清仓"
}

// 判断实际收益类别
fn classify_return(raw_return, horizon) {
    // 短线（≤5天）波动容忍大
    if horizon == "ultra_short" || horizon == "short" {
        if raw_return >= 5.0 { return "大幅盈利"; }
        if raw_return >= 1.5 { return "小幅盈利"; }
        if raw_return > -1.5 { return "基本持平"; }
        if raw_return > -4.0 { return "小幅亏损"; }
        return "大幅亏损";
    }
    // 中线（28天）
    if horizon == "mid" {
        if raw_return >= 10.0 { return "大幅盈利"; }
        if raw_return >= 3.0 { return "小幅盈利"; }
        if raw_return > -3.0 { return "基本持平"; }
        if raw_return > -8.0 { return "小幅亏损"; }
        return "大幅亏损";
    }
    // 长线（90天+）
    if raw_return >= 20.0 { return "大幅盈利"; }
    if raw_return >= 5.0 { return "小幅盈利"; }
    if raw_return > -5.0 { return "基本持平"; }
    if raw_return > -15.0 { return "小幅亏损"; }
    return "大幅亏损";
}

fn classify_confidence_level(c) {
    if is_unit(c) { return "未知"; }
    let v = c;
    if v >= 0.8 { return "非常高"; }
    if v >= 0.65 { return "高"; }
    if v >= 0.5 { return "中等"; }
    if v >= 0.35 { return "低"; }
    return "非常低";
}

fn main() {
    // ── 基本量提取（TypeScript → Rhai: Value::Null → ()）──
    let raw_return = if !is_unit(raw_return_pct) { raw_return_pct } else { 0.0 };
    let alpha_return = if !is_unit(alpha_return_pct) { alpha_return_pct } else { 0.0 };
    let actual_days = if !is_unit(holding_days) && type_of(holding_days) == "i64" {
        holding_days
    } else if !is_unit(holding_days) && type_of(holding_days) == "f64" {
        holding_days
    } else { 0 };

    let horizon = if !is_unit(original_time_horizon) && type_of(original_time_horizon) == "string" {
        original_time_horizon.to_string()
    } else { "unknown" };

    let expected_days = if !is_unit(original_holding_days) { original_holding_days } else { 0 };

    // ── [实际行情] 价格层事实提取 ──
    // 由后端 compute_market_snapshots() 用前复权 K 线确定性算出，经 actual_market_json 注入。
    // 全部做 is_unit 守卫：行情不可用时后端注入 {} ⇒ 这些键取到 ()，不能让脚本崩。
    let cur_price = if !is_unit(latest_price) { latest_price } else { 0.0 };
    let base_price = if !is_unit(entry_price) { entry_price } else { 0.0 };
    let dd_pct = if !is_unit(max_drawdown_pct) { max_drawdown_pct } else { 0.0 };
    let prog_pct = if !is_unit(target_progress_pct) { target_progress_pct } else { 0.0 };
    let hi_price = if !is_unit(period_high) { period_high } else { 0.0 };
    let lo_price = if !is_unit(period_low) { period_low } else { 0.0 };
    let latest_d = if !is_unit(latest_date) && type_of(latest_date) == "string" {
        latest_date.to_string()
    } else { "" };
    let within_horizon = if !is_unit(within_expected_horizon)
        && type_of(within_expected_horizon) == "bool" {
        within_expected_horizon
    } else { false };

    // 目标价：优先取行情快照侧（三个入口都能供给），回退 sub-analysis 记忆里的 trader 输出。
    // 注：此前 DAG 声明了 trader_target_price 映射但本脚本从未消费它（死映射），
    // 目标价维度实际从未参与过反思对比 —— 本次一并接线。
    let tgt_price = if !is_unit(target_price) && target_price > 0.0 {
        target_price
    } else if !is_unit(trader_target_price) && trader_target_price > 0.0 {
        trader_target_price
    } else { 0.0 };

    let has_price_facts = base_price > 0.0 && cur_price > 0.0;

    // ── 方向对比：交易员动作 vs 实际收益 ──
    let trader_act_str = if !is_unit(trader_action) && type_of(trader_action) == "string" {
        trader_action.to_string()
    } else { "" };

    // 判断 trader 方向（看多/看空/中性）
    let predicted_direction = if trader_act_str == "买入" || trader_act_str == "增持" || trader_act_str == "持有" {
        "看多"
    } else if trader_act_str == "卖出" || trader_act_str == "减持" {
        "看空"
    } else {
        "观望"
    };

    // 判断实际方向（正收益=看多正确，负收益=看空正确）
    let actual_direction = if raw_return >= 1.0 {
        "上涨"
    } else if raw_return <= -1.0 {
        "下跌"
    } else {
        "横盘"
    };

    // 方向是否一致
    let direction_match = (predicted_direction == "看多" && actual_direction == "上涨")
        || (predicted_direction == "看空" && actual_direction == "下跌");

    // ── 置信度 vs 收益评估 ──
    let trader_conf = if !is_unit(trader_confidence) { trader_confidence } else { 0.0 };
    let post = if !is_unit(portfolio_posterior) { portfolio_posterior } else { 0.0 };
    let conf_level = classify_confidence_level(post);

    let confidence_vs_return = if direction_match {
        if raw_return.abs() < 1.0 {
            `方向正确但收益有限(conf=${conf_level}, return=${raw_return})`
        } else {
            `方向正确(conf=${conf_level}, return=${raw_return}%)`
        }
    } else if predicted_direction == "观望" {
        `保持观望,实际${actual_direction}(${raw_return}%)`
    } else {
        `方向错误(预测${predicted_direction}→实际${actual_direction}, conf=${conf_level})`
    };

    // ── 时间维度匹配检查 ──
    let horizon_mismatch = if actual_days > 0 && expected_days > 0 {
        let ratio = actual_days.to_float() / expected_days.to_float();
        if ratio > 2.0 {
            `超期持有：实际${actual_days}天/预期${expected_days}天(${ratio}倍)`
        } else if ratio < 0.3 && actual_days < expected_days {
            `提前了结：实际${actual_days}天/预期${expected_days}天`
        } else {
            `时间匹配：实际${actual_days}天≈预期${expected_days}天`
        }
    } else {
        "时间维度数据不足"
    };

    // ── 收益率分类 ──
    let return_cat = classify_return(raw_return, horizon);

    // ── 聚合分数 ──
    let sc = if !is_unit(total_score) { total_score } else { 0 };
    let consensus = if !is_unit(debate_consensus) { debate_consensus } else { 0.0 };

    // ── untrusted 节点扫描（替代原 reflection_untrusted_aggregator.rhai）──
    let untrusted_sources = [];
    let sink = |flag, name| {
        if !is_unit(flag) && type_of(flag) == "bool" && flag {
            untrusted_sources.push(name);
        }
    };
    // ⚠ sink 是闭包（`let f = |...|` ⇒ FnPtr）⇒ 必须 `sink.call(...)`；按名 `sink(...)`
    //   会抛 Function not found，整段扫描静默失效（untrusted_count 恒 0）。
    sink.call(u_trader, "trader");
    sink.call(u_research_mgr, "research-mgr");
    sink.call(u_catalyst, "catalyst");
    sink.call(u_debate_cnv, "debate-convergence");
    sink.call(u_data_quality, "data-quality");
    sink.call(u_risk_cnv, "risk-convergence");
    let untrusted_count = untrusted_sources.len();
    let sub_analysis_healthy = untrusted_count == 0;

    // ── 关键发现列表 ──
    let findings = [];

    // 1. 方向发现
    findings.push(
        if direction_match { `预测方向正确：${predicted_direction}→实际${actual_direction}` }
        else { `预测方向偏差：${predicted_direction}→实际${actual_direction}(收益${raw_return}%)` }
    );

    // 2. 超额发现
    if alpha_return != 0.0 {
        let alpha_sign = if alpha_return >= 0.0 { "+" } else { "" };
        findings.push(`相对基准超额：${alpha_sign}${alpha_return}%`);
    }

    // 3. 置信度发现
    if post > 0.0 {
        findings.push(`决策置信度${conf_level}(posterior=${post})`);
    }

    // 4. 评分发现
    if sc > 0 {
        findings.push(`技术评分${sc}/100`);
    }

    // 5. 辩论共识发现
    if consensus > 0.0 {
        findings.push(`辩论共识度${consensus}`);
    }

    // 6. untrusted 节点发现
    if untrusted_count > 0 {
        findings.push(`上游${untrusted_count}个节点不可信：${untrusted_sources}`);
    }

    // 7. [实际行情] 价格层发现 —— 让反思 agent 能引用具体价格而非只有百分比
    if has_price_facts {
        let chg_pct = ((cur_price - base_price) / base_price * 10000.0).round() / 100.0;
        findings.push(`价格事实：入场基准 ${base_price} → 最新 ${cur_price}（${latest_d}），涨跌 ${chg_pct}%`);
        findings.push(`区间最高 ${hi_price} / 最低 ${lo_price}，期间最大回撤 ${dd_pct}%`);
    } else {
        findings.push("实际行情数据不可用：无法给出价格层对比");
    }

    // 8. [实际行情] 目标价兑现度（此前 trader_target_price 是死映射，从未参与对比）
    if tgt_price > 0.0 {
        let reached = if has_price_facts && cur_price >= tgt_price { "已达到/超过" } else { "未达到" };
        findings.push(`目标价 ${tgt_price}：最新价${reached}目标价，目标实现度 ${prog_pct}%`);
    }

    // 9. [实际行情] 期中观察标记 —— 未到期就反思时，agent 必须降低结论权重
    if within_horizon {
        findings.push(`⚠ 尚未到期望持有期（实际 ${actual_days} 天 / 期望 ${expected_days} 天），本结论属期中观察而非事后终评`);
    }

    // ── 偏差总结（≤200 字符）──
    let deviation_summary = if !has_price_facts {
        // 行情缺失时必须显式告知，而不是拿 raw_return=0.0 冒充"横盘"
        `实际行情数据不可用，无法做定量方向对比，请仅基于定性记忆反思。评分=${sc}`
    } else if direction_match {
        if raw_return.abs() < 2.0 {
            `方向判断正确但收益有限(${raw_return}%)。后验=${post}，评分=${sc}`
        } else {
            `预测方向正确，实际收益${raw_return}%。后验=${post}，评分=${sc}`
        }
    } else if predicted_direction == "观望" {
        `实际${actual_direction}(${raw_return}%)但预测观望。评分=${sc}，共识=${consensus}`
    } else {
        `方向误判：预测${predicted_direction}但实际${actual_direction}(${raw_return}%)。后验=${post}，评分=${sc}`
    };

    // ── 四周期 horizon_correct 确定性判定（批次 3）──
    // 对每个周期独立判定：决策方向是否被实际行情验证。
    // 判定逻辑与 deterministic_was_correct 同源（Rhai 移植），
    // 结果写入 horizon_correct map，
    // key=周期名，value=true/false/"neutral"/"immature"/"unavailable"。
    let horizon_correct = #{};
    let have_decisions =
        !is_unit(horizon_decisions_json) && type_of(horizon_decisions_json) == "string";
    let have_facts = !is_unit(horizon_market_facts);

    if have_decisions && have_facts {
        let decisions_map = if horizon_decisions_json.to_string() != "" {
            parse_json(horizon_decisions_json.to_string())
        } else {
            #{}
        };

        let horizons = ["ultra_short", "short", "mid", "long"];

        for h in horizons {
            let snap = horizon_market_facts.get(h);
            let decision_obj = decisions_map.get(h);

            let have_snap = !is_unit(snap) && type_of(snap) == "map";
            let have_dec = !is_unit(decision_obj) && type_of(decision_obj) == "map";

            if !have_snap || !have_dec {
                horizon_correct[h] = "unavailable";
                continue;
            }

            let action_v = decision_obj.get("action");
            let action_str = if !is_unit(action_v) && type_of(action_v) == "string" {
                action_v.to_string()
            } else {
                horizon_correct[h] = "unavailable";
                continue;
            };

            let within = snap.get("withinExpectedHorizon");
            let is_within = !is_unit(within) && type_of(within) == "bool" && within;

            if is_within {
                horizon_correct[h] = "immature";
                continue;
            }

            let ret = snap.get("netReturnPct");
            let ret_val = if !is_unit(ret) {
                if type_of(ret) == "f64" { ret }
                else if type_of(ret) == "i64" { ret.to_float() }
                else { 0.0 }
            } else { 0.0 };

            if is_bull(action_str) {
                horizon_correct[h] = ret_val > 0.0;
            } else if is_bear(action_str) {
                horizon_correct[h] = ret_val < 0.0;
            } else {
                horizon_correct[h] = "neutral";
            }
        }
    }

    #{
        "status": "ok",
        "direction_match": direction_match,
        "predicted_direction": predicted_direction,
        "actual_direction": actual_direction,
        "raw_return_pct": raw_return,
        "alpha_return_pct": alpha_return,
        "confidence_vs_return": confidence_vs_return,
        "return_category": return_cat,
        "confidence_level": conf_level,
        "horizon_mismatch": horizon_mismatch,
        "deviation_summary": deviation_summary,
        "key_findings": findings,
        "actual_holding_days": actual_days,
        "expected_holding_days": expected_days,
        "has_price_facts": has_price_facts,
        "horizon_correct": horizon_correct,
        "price_context": #{
            "available": has_price_facts,
            "entry_price": base_price,
            "latest_price": cur_price,
            "latest_date": latest_d,
            "period_high": hi_price,
            "period_low": lo_price,
            "max_drawdown_pct": dd_pct,
            "target_price": tgt_price,
            "target_progress_pct": prog_pct,
            "within_expected_horizon": within_horizon
        },
        "untrusted_count": untrusted_count,
        "untrusted_sources": untrusted_sources,
        "sub_analysis_healthy": sub_analysis_healthy
    }
}