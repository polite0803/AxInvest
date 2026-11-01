// factor-score.rhai — 趋势智选阶段 3「打分重构」：横截面反转打分（确定性，零 token）
//
// 归属：两链共用（快速链 v5 / 原链 v65 各挂一个 Code 节点）。
//
// # 为什么是「1 月反转」且权重无凭空
//
// 阶段 1 全市场因子 IC 普查（PLAN-serenity-full-coverage-predictive.md §阶段1）结论：
// 可测因子族中唯一稳定有效的是 rev_1m（1 月反转，mean IC +0.060~+0.066，NW t 在
// 5/10/20 日视野显著）；动量方向整体为负或不显著。故本打分**只给这一个因子定价**
// （权重 100%），不掺任何动量项、不掺 LLM 自报分——这就是「无凭空权重」的落实。
// 质地/瓶颈复合分（strategy-scorer 产物）不再进排序键，降为 data-verifier 输出里的
// 参考字段（quality_score）。
//
// # 打分公式（因子权重有阶段 1 读数支撑）
//
//   输入  chg1m = 截至 as-of 的 1 月涨跌幅（21 个交易日间隔，market_daily_close）
//   方向  rev_1m = -chg1m（跌得多 ⇒ 分高；阶段 1 实测 A 股横截面反转成立）
//   打分  池内横截面百分位：chg1m 升序排秩 r（1 = 跌最多），
//         score = 100 × (1 − (r−1)/(n−1))；n=1 时中性 50。
//
// # 缺席语义（结构性缺口不得伪装成正常结果）
//
// 因子取不到的票（新股不足 22 行 / 停牌 / 宇宙外）**不参与横截面、不给中性分**：
// score_map 无其键、labels_ranked 里缺席票殿后（保原序），stats.absent 与
// summary 显式计数。下游 data-verifier 对缺席票把最终分封顶 54（「没算过的票
// 不得冒充高分」，与组装端对 LLM 自报分的既有封顶同口径）。
//
// # 输入
//   stocks  — 池清单：快速链 c-candidate-pool.result.stocks（[{code,name,source,industry}]）
//             原链 t-constituents.result（get_industry_constituents 信封，分组嵌套 stocks）
//   factors — t-factors.result（get_stock_factor 信封：{status,data:[{stockCode,chg1m,…}],missing}）
// # 输出
//   {score_map, ranked_stocks, labels_ranked, ranked_text, stats, summary}

fn to_array(v) {
    if type_of(v) == "array" { v }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "array" { p }
        else if type_of(p) == "map" && p["data"] != () && type_of(p["data"]) == "array" { p["data"] }
        else { [] }
    } else if type_of(v) == "map" {
        if v["content"] != () && type_of(v["content"]) == "string" {
            let p = json_parse(v["content"].to_string());
            if type_of(p) == "array" { p }
            else if type_of(p) == "map" && p["data"] != () && type_of(p["data"]) == "array" { p["data"] }
            else { [] }
        } else if v["data"] != () && type_of(v["data"]) == "array" { v["data"] }
        else { [] }
    } else { [] }
}

fn field_str(m, key) {
    if type_of(m) != "map" { return ""; }
    let v = m[key];
    if v == () { return ""; }
    if type_of(v) == "string" { return v; }
    v.to_string()
}

fn to_f64(v, fallback) {
    if type_of(v) == "f64" { v }
    else if type_of(v) == "i64" { v.to_float() }
    else { fallback }
}

// ── 归一因子行：code → chg1m ──
let factor_map = #{};
let factors_missing_count = 0;
if type_of(factors) == "map" && factors["missing"] != () && type_of(factors["missing"]) == "array" {
    factors_missing_count = factors["missing"].len();
};
for row in to_array(factors) {
    let code = field_str(row, "stockCode");
    if code == "" { continue; }
    let c = row["chg1m"];
    if c == () || (type_of(c) != "f64" && type_of(c) != "i64") { continue; }
    factor_map[code] = to_f64(c, 0.0);
}

// ── 归一池清单（嵌套分组信封自动下钻 stocks）──
let pool = [];
let pool_codes = [];
for g in to_array(stocks) {
    let members = if g["stocks"] != () && type_of(g["stocks"]) == "array" { g["stocks"] } else { [g] };
    for s in members {
        let code = field_str(s, "stockCode");
        if code == "" { code = field_str(s, "code"); }
        if code == "" { continue; }
        pool.push(s);
        pool_codes.push(code);
    };
}

// ── 横截面排秩：chg1m 升序（跌最多 = 秩 1 = 分最高）──
// Rhai 不依赖内置 sort：选择法逐位取当前最小，n ≤ 池上限 1000，O(n²) 可接受。
let scored_codes = [];
let scored_chg = [];
for code in pool_codes {
    if factor_map[code] != () {
        scored_codes.push(code);
        scored_chg.push(factor_map[code]);
    };
};
let n = scored_codes.len();
let used = [];
for i in range(0, n) { used.push(false); };
let ranked = [];   // [{code, chg1m}] 按分降序（chg1m 升序）
for k in range(0, n) {
    let best = -1;
    for i in range(0, n) {
        if used[i] { continue; }
        if best < 0 || scored_chg[i] < scored_chg[best] { best = i; }
    };
    used[best] = true;
    ranked.push(#{ "code": scored_codes[best], "chg1m": scored_chg[best] });
};

// ── 百分位打分 ──
let score_map = #{};
let denom = if n > 1 { (n - 1).to_float() } else { 1.0 };
for k in range(0, n) {
    let score = if n == 1 { 50.0 } else { 100.0 * (1.0 - k.to_float() / denom) };
    let code = ranked[k]["code"];
    score_map[code] = #{ "score": score, "chg1m": ranked[k]["chg1m"], "rank": k + 1 };
};

// ── ranked_stocks：打过分的按分降序在前，缺席票殿后（保池内原序）──
let ranked_stocks = [];
for k in range(0, n) {
    for s in pool {
        if field_str(s, "stockCode") == ranked[k]["code"] || field_str(s, "code") == ranked[k]["code"] {
            ranked_stocks.push(s);
            break;
        }
    };
};
let absent_codes = [];
for s in pool {
    let code = field_str(s, "stockCode");
    if code == "" { code = field_str(s, "code"); }
    if score_map[code] == () {
        ranked_stocks.push(s);
        absent_codes.push(code);
    };
};

// ── labels_ranked：label 形如 "002049 紫光国微"，取前 6 位查分；缺席殿后保原序 ──
let scored_labels = [];
let absent_labels = [];
for lb in to_array(labels) {
    let s = lb.to_string();
    let code = if s.len() >= 6 { s.sub_string(0, 6) } else { "" };
    if code != "" && score_map[code] != () {
        scored_labels.push(#{ "label": s, "code": code });
    } else {
        absent_labels.push(s);
    };
};
// 选择法按 score 降序
let m = scored_labels.len();
let lused = [];
for i in range(0, m) { lused.push(false); };
let labels_ranked = [];
for k in range(0, m) {
    let best = -1;
    for i in range(0, m) {
        if lused[i] { continue; }
        if best < 0 || score_map[scored_labels[i]["code"]]["score"] > score_map[scored_labels[best]["code"]]["score"] { best = i; }
    };
    lused[best] = true;
    labels_ranked.push(scored_labels[best]["label"]);
};
for lb in absent_labels { labels_ranked.push(lb); };

// ── ranked_text：给原链 mapper 的人读清单（前 50，带分与基期）──
let ranked_text = "因子分（1 月反转，跌多分高）Top " + (if n > 50 { "50" } else { n.to_string() }) + " / 共 " + n.to_string() + " 只可打分：";
let top_n = if n > 50 { 50 } else { n };
for k in range(0, top_n) {
    let code = ranked[k]["code"];
    let pct = (ranked[k]["chg1m"] * 100.0);
    ranked_text += "\n  " + (k + 1).to_string() + ". " + code + " 因子分=" + score_map[code]["score"].to_string() + " 1月涨跌=" + pct.to_string() + "%";
};

let absent = n_missing_absent(absent_codes, factors_missing_count);
let summary = if n == 0 {
    "因子打分 0 只：get_stock_factor 未返回任何行（取数失败或池为空），缺席 " + absent.to_string() + " 只——打分将按缺席口径封顶"
} else {
    "因子打分 " + n.to_string() + " / 池 " + pool.len().to_string() + "（缺席 " + absent.to_string()
        + " 只，不足 22 行收盘或停牌），因子=1月反转（阶段1 IC 普查唯一稳定有效因子）"
};

fn n_missing_absent(absent_codes, factors_missing_count) {
    if absent_codes.len() > factors_missing_count { absent_codes.len() } else { factors_missing_count }
}

#{
    "score_map": score_map,
    "ranked_stocks": ranked_stocks,
    "labels_ranked": labels_ranked,
    "ranked_text": ranked_text,
    "stats": #{ "requested": pool.len(), "scored": n, "absent": absent, "factor": "rev_1m" },
    "summary": summary
}
