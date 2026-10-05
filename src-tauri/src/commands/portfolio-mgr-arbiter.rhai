// portfolio-mgr-arbiter.rhai — **P5 跨档仲裁**（PLAN §十一 11-1 的 arbiter CodeNode）
//
// 职责边界是用户裁定 Q3 写死的：**只回答「资金投向哪档」**，输出新字段 `arbitration`；
// 四档并列结论 `decisionsByHorizon` 与按档复盘**一律不动**，也不产「综合报告」。
// 之所以要单独一个节点而不并进 portfolio-mgr：主链脚本里那段注释（`:1932-1934`）明文
// 「这里不做四档 switch」—— 仲裁若塞进主链就是推翻既有裁定，独立成节点才是「新增」而非「改写」。
//
// 排名口径 = **每日对数资本增长率**（`pm_kelly_growth`，Kelly 1956 的增长率除以持有天数）。
// 不用「哪档仓位大 / 哪档置信高」：8% 仓锁 2 天与 30% 仓锁 90 天不可直接比，
// 可比的是单位时间的资本增长期望 —— 这也让四档真的可比（同一量纲），而不是各报各的。
//
// 三条 P5′ 容错纪律（PLAN §十一 11-5）：
// 1. 缺席档**退出比较**，不记 0 分 —— 把「这一档没跑出来」当成「这一档看空」是最贵的错；
// 2. 无仓位 / 赔率非正 / 持有 0 天 ⇒ 判 `ineligible`（该档算过但不可比），与缺席分列；
// 3. 一档都不入选 ⇒ `primaryTier = ""` + 点名原因，**不硬选一个**。

let TIERS = ["ultra_short", "short", "mid", "long"];

// 与 `portfolio-mgr.rhai` 同规：`present` 由脚本自带（宿主引擎没注册它）。
fn present(x) { type_of(x) != "()" }

// 四路结论由节点 input_mapping 逐个注入（节点缺席时是 unit，不是空对象 ⇒ 必须按 unit 判）。
let results = #{
    ultra_short: if present(r_ultra_short) { r_ultra_short } else { () },
    short: if present(r_short) { r_short } else { () },
    mid: if present(r_mid) { r_mid } else { () },
    long: if present(r_long) { r_long } else { () },
};

fn numof(row, key) {
    if type_of(row) != "map" { return (); }
    let v = row[key];
    if type_of(v) == "f64" { return v; }
    if type_of(v) == "i64" { return v.to_float(); }
    return ();
}

let ranked = [];
let candidates = [];
let absent = [];
let ineligible = [];

for tier in TIERS {
    let o = results[tier];
    if type_of(o) != "map" {
        absent.push(tier);
        ranked.push(#{ tier: tier, status: "absent", growth: (), action: "", positionPct: () });
        continue;
    }
    let conf = numof(o, "confidence");
    let odds = numof(o, "odds");
    let pos = numof(o, "positionPct");
    let days = numof(o, "expectedHoldingDays");
    let action = if type_of(o["action"]) == "string" { o["action"] } else { "" };
    if type_of(conf) == "()" || type_of(odds) == "()" || type_of(pos) == "()" || type_of(days) == "()" {
        // 结论在，但缺仲裁要的字段 ⇒ 不可比（不是不看好）
        ineligible.push(tier);
        ranked.push(#{ tier: tier, status: "incomplete", growth: (), action: action, positionPct: pos });
        continue;
    }
    let g = pm_kelly_growth(conf / 100.0, odds, pos, days.to_int());
    if type_of(g) == "()" {
        ineligible.push(tier);
        ranked.push(#{
            tier: tier, status: "ineligible", growth: (), action: action, positionPct: pos,
        });
        continue;
    }
    candidates.push(#{ tier: tier, status: "ranked", growth: g, action: action, positionPct: pos });
    ranked.push(#{ tier: tier, status: "ranked", growth: g, action: action, positionPct: pos });
}

let coverage = `${candidates.len()}/${TIERS.len()}`;
let primary = "";
let primary_growth = ();
let reason = "";

if candidates.len() == 0 {
    reason = `四档无一可参与仲裁（缺席 ${absent.len()} 档、不可比 ${ineligible.len()} 档）⇒ 本字段不指定投向，不做硬选`;
} else {
    // 取最大增长率；并列时按「持有期更长」优先（锁仓更久意味着该结论更不需要择时兑现）。
    let best = candidates[0];
    for row in candidates {
        if row["growth"] > best["growth"] {
            best = row;
        } else if row["growth"] == best["growth"] {
            let cur_days = numof(results[best["tier"]], "expectedHoldingDays");
            let cand_days = numof(results[row["tier"]], "expectedHoldingDays");
            if cand_days > cur_days { best = row; }
        }
    }
    primary = best["tier"];
    primary_growth = best["growth"];
    reason = `按每日对数资本增长率取档；参与比较 ${coverage}，缺席 ${absent.len()}，不可比 ${ineligible.len()}`;
}

#{
    // 只答「资金投向哪档」；四档并列结论在 decisionsByHorizon，本字段不改写它们（裁定 Q3）。
    primaryTier: primary,
    primaryGrowthPerDay: if type_of(primary_growth) == "()" { () } else { (primary_growth * 1e6).round() / 1e6 },
    coverage: coverage,
    absentTiers: absent,
    ineligibleTiers: ineligible,
    ranked: ranked,
    reason: reason,
    basis: "kelly_growth_rate_per_holding_day",
    scope: "capital_allocation_only",
}
