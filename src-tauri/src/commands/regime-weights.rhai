// regime-weights.rhai — 基于市场状态的因子权重调节（决策二选 C 轻量实现）
//
// 输入：
//   market_regime_state: "bull"/"bear"/"sideways"/"volatile"/"unknown"
//   market_regime_prior: f64 (0.4-0.65)
//   market_regime_volatility: "high"/"low"
//
// 输出：factor_weights JSON（与 portfolio-mgr 的 input_mapping f_weights 格式兼容）
//   {trend: {weight, regime_mod}, consensus: {weight, regime_mod}, ...}
//
// 原理：
//   牛市 → 趋势+资金面权重↑（趋势跟踪有效）
//   熊市 → 估值+风险权重↑（防御优先），筹码面权重↑（解禁减持在熊市杀伤力更大）
//   高波动 → 所有因子降权，风控优先
//
// P2 修复: 补充 f7(trade_signal)/f9(money_flow)/f10(chip) 的 regime 调节参数
//   trade_signal: 熊市降权（LLM 在熊市易过度悲观），高波动降权（LLM 判断可靠性下降）
//   money_flow: 牛市升权（趋势跟踪有效），熊市降权（资金面萎缩）
//   chip: 熊市升权（解禁减持在熊市杀伤力更大），高波动升权（筹码稳定性更重要）
//
// ── P1-F 契约对齐（2026-09-11；v61 2026-09-20 增列 bottleneck）──
// 实证：portfolio-mgr 实际只对 6 个因子调用 get_weight
//   （trend / consensus / valuation / money_flow / chip / bottleneck；
//    pace/momentum 走 IC 回测权重）。
//   v61 变更：「bottleneck」由硬编码 0.10 改为走 get_weight ⇒ 本表新增该条目并
//   加入 consumed_names。变更前后**中性市况下权重逐值相同**（0.10×1.0），
//   差异只出现在 bull/bear/volatile 三档（系数理由见下方 factors 表注释）。
// 而本脚本原先输出 9 个因子的调节权重，其中 catalyst(0.16) / risk(0.225) /
//   data_quality(0.18) / trade_signal(0.12) 合计 0.685（占全部调整后权重 1.41 的 48.6%）
//   —— 产出后无人消费，属「死计算」：既不生效，又让人误以为权重已随市况自适应。
// 处理：把「被消费」与「未被消费」两类显式分桶输出：
//   factor_weights        → 仅含被 portfolio-mgr 消费的因子（契约真实）
//   unconsumed_suggestions→ 未被消费的调节建议（保留设计意图 + 便于审计，不参与决策）
//   total_weight          → 仅统计被消费因子（与 pm 的 evidence_scale 语义一致）
//   total_weight_all      → 全部因子（审计用）
// 注意：这 4 个因子的权重在 pm 侧为硬编码（catalyst 0.20 / risk 0.15 /
//   data_quality 0.15 / trade_signal 0.10），是 V37/V54 校准过的基线。
//   反事实实证（2026-09-11，9 样本）：若让 f4 改消费本脚本建议的 risk=0.225，
//   8/9 样本 posterior 更低（均值 -1.0pt），且与 f4 成长豁免叠加会造成
//   双重风险放大 → 明确不消费，保持 pm 侧硬编码。

fn present(x) { type_of(x) != "()" }

// 判断因子名是否属于「被 portfolio-mgr 消费」的白名单
fn is_consumed(consumed_names, x) {
    for n in consumed_names {
        if n == x { return true; }
    }
    false
}

// 输入：market_regime_state ("bull"/"bear"/"sideways"/"volatile"/"unknown")
//       market_regime_prior (0-1 f64)
//       market_regime_volatility ("high"/"low")
// stock-analysis crate 的 classify_regime 输出 "sideways" 而非 "neutral"，
// 此处统一映射。高波动状态下 regime_key 走 "volatile" 降权。
let raw_state = if present(market_regime_state) { market_regime_state } else { "neutral" };
let state = if raw_state == "sideways" || raw_state == "unknown" { "neutral" } else { raw_state };
let prior_val = if present(market_regime_prior) { market_regime_prior } else { 0.50 };
let volatility = if present(market_regime_volatility) { market_regime_volatility } else { "low" };

// ── 各因子基权重 + 市场状态调节系数 ──
// P2 修复新增: money_flow (f9), chip (f10), trade_signal (f7)
//
// P1-D 重估（2026-09-11）：money_flow base 0.08 → 0.12
//   实证：f9 的最终有效权重被三重削减后仅 0.042 ——
//     base 0.08 → bear regime_mod 0.7 → 0.056 → f1↔f9 共振衰减 ×0.75 → 0.042，
//   占全因子总权重（1.438）的 2.9%，成为最低权重因子，近乎无效。
//   而资金流（主力净流入/龙虎榜）在 A 股是核心量价证据源，不应低于 chip。
//   重估后 bear 市况 = 0.12×0.7×0.75 = 0.063（+50%），仍低于 trend(0.105)，
//   保持「资金面是趋势的伴随证据」这一朴素贝叶斯独立性的保守假设。
let factors = #{
    "trend": #{ "base": 0.15, "bull": 1.4, "bear": 0.7, "neutral": 1.0, "volatile": 0.6 },
    "consensus": #{ "base": 0.25, "bull": 1.0, "bear": 1.0, "neutral": 1.0, "volatile": 0.8 },
    "catalyst": #{ "base": 0.20, "bull": 1.1, "bear": 0.8, "neutral": 1.2, "volatile": 0.7 },
    "risk": #{ "base": 0.15, "bull": 0.8, "bear": 1.5, "neutral": 1.0, "volatile": 1.8 },
    "valuation": #{ "base": 0.15, "bull": 0.7, "bear": 1.4, "neutral": 1.1, "volatile": 1.2 },
    "data_quality": #{ "base": 0.15, "bull": 1.0, "bear": 1.2, "neutral": 1.0, "volatile": 1.3 },
    // P2 新增的三个因子
    "trade_signal": #{ "base": 0.15, "bull": 1.1, "bear": 0.8, "neutral": 1.0, "volatile": 0.7 },
    "money_flow": #{ "base": 0.12, "bull": 1.3, "bear": 0.7, "neutral": 1.0, "volatile": 0.5 },
    "chip": #{ "base": 0.08, "bull": 0.8, "bear": 1.3, "neutral": 1.0, "volatile": 1.5 },
    // v61(2026-09-20): f13 瓶颈因子 —— portfolio-mgr 由「硬编码 0.10」改为
    //   `get_weight(f_weights, "bottleneck", 0.10)`，本表必须提供该条目，
    //   否则 get_weight 永远回落 fallback，白名单等于空接。
    //
    //   系数取值理由：瓶颈 = 供给刚性 / 需求弹性 / 不可替代性，是**结构属性**。
    //   曾有政策以「结构属性 ⇒ 市况不敏感」为由把 f13 权重硬编码，但那是把
    //   「**信号源取值**不随市况变」误当成「**该因子的判别价值**不随市况变」：
    //     牛市 —— β 普涨淹没个股结构性差异 ⇒ 边际信息量下降（0.9）
    //     熊市/高波动 —— 结构是价格之外先存活下来的维度，噪声越大越该看结构（1.2/1.3）
    //   方向与 valuation / chip 一致（压力市况升权），但幅度刻意小于它们：
    //     f13 绝对权重仅 0.10（全因子最低档），摆幅过大会让单一结构因子主导后验；
    //     且其信号源 `serenity_score` 自身已含市况敏感度，叠加过强即重复计数
    //     （同 pm 侧 P1-1 协方差衰减要解决的问题）。
    //   ⚠️ base 0.10 与 neutral 系数 1.0 ⇒ **中性市况下权重与变更前逐值相同**
    //     （0.10×1.0），非中性市况才产生位移。
    //   ⚠️ 待真正标定：应按「参数四道门」把 base 与四档系数上提为面板可调参数。
    "bottleneck": #{ "base": 0.10, "bull": 0.9, "bear": 1.2, "neutral": 1.0, "volatile": 1.3 },
};

// 解析状态：高波动（volatility="high"）时统一走 "volatile" 降权路径
let regime_key = if volatility == "high" { "volatile" }
    else if state == "bull" || state == "bear" || state == "neutral" { state }
    else { "neutral" };

let factor_names = ["trend", "consensus", "catalyst", "risk", "valuation", "data_quality",
                    "trade_signal", "money_flow", "chip", "bottleneck"];

// portfolio-mgr 实际消费的因子白名单（其余为死计算，见文件头 P1-F 说明）
// v61(2026-09-20): 加入 "bottleneck" —— pm 侧 f13 已由硬编码改走 get_weight，
//   两处必须**同时**改：只改 pm 则 get_weight 恒取 fallback（白名单外 ⇒ 本表把
//   bottleneck 丢进 unconsumed_suggestions，factor_weights 里没有它）；
//   只改本表则 pm 仍读硬编码（白名单条目产出后无人消费 ⇒ 又一处死计算）。
let consumed_names = ["trend", "consensus", "valuation", "money_flow", "chip", "bottleneck"];

// 计算调节后的因子权重（按消费状态分桶）
let result = #{};
let unconsumed = #{};
let total_weight = 0.0;
let total_weight_all = 0.0;
for name in factor_names {
    let f = factors[name];
    let base = f["base"];
    let mod_val = f[regime_key];
    let adjusted = base * mod_val;
    let entry = #{ "weight": (adjusted * 1000.0).round() / 1000.0, "regime_mod": mod_val };
    total_weight_all = total_weight_all + adjusted;
    if is_consumed(consumed_names, name) {
        result[name] = entry;
        total_weight = total_weight + adjusted;
    } else {
        unconsumed[name] = entry;
    }
}

#{ "factor_weights": result,
   "unconsumed_suggestions": unconsumed,
   // v40(2026-09-13) 字段更名 prior → regime_confidence：
   //   本值实为 market_regime 分类器的**置信度**（模板映射
   //   market_regime_prior ← market_regime.confidence），而非决策使用的方向先验。
   //   方向先验由 portfolio-mgr 按市况**方向**自行派生（REGIME_PRIOR_BULL 0.55 /
   //   SIDEWAYS 0.50 / BEAR 0.45，见 portfolio-mgr.rhai:182-193）；`market_regime_prior`
   //   在那里只剩一条向后兼容分支（market_regime_state 缺失时启用，且被 clamp 到
   //   [0.35, 0.65]，正是为了防「把分类置信度当 95% 看多先验」）。
   //   旧名 `prior` 与 pm 的 prior **同名不同义** —— 同一次 run 里会同时出现本字段 0.5
   //   与 pm 实际使用的 0.45 两个「先验」，极易被当成矛盾或误读成"决策用了 0.5"
   //   （铁律 41：同名字段跨链语义分裂）。
   //   前端仅消费 factor_weights（decisionInputDiagnosis.ts），改名无破坏；且全仓无
   //   `regime-weights.result.prior` 的消费点（已 grep 确认）。
   "regime_confidence": prior_val,
   "regime": state,
   "total_weight": (total_weight * 1000.0).round() / 1000.0,
   "total_weight_all": (total_weight_all * 1000.0).round() / 1000.0 }
