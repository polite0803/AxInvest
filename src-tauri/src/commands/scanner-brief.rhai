// scanner-brief.rhai — 快速趋势智选 Phase B：确定性市场简报
//
// 归属：`serenity-screening-fast` 模板新增资产（原链无对应脚本 ⇒ 不是同源双版本分支）。
//
// 输入（input_mapping 注入，均取自 ToolNode 的 `.result`）：
//   industry_ranking — get_industry_ranking（JSON 文本，Vec<IndustryRank>）
//   cls_flash        — get_cls_flash（JSON 文本，Vec<ClsFlashItem>）
//   policy_news      — search_news（JSON 文本，Vec<NewsItem>）
//   user_themes      — 模板变量（字符串或字符串数组）
// 输出：
//   {brief, keyword, hot_industries[], excluded[], news_highlights[], data_available}
//
// 设计口径：全部阈值 / 算术留在脚本（确定性），不调用 LLM —— 这正是「快速链」省时的来源。
//   - 严格档：涨幅 2%~15% 且主力净流入 > 0（对齐原 trend-scanner prompt 的「萌芽→加速」）
//   - 严格档为空 → 退化为「主力净流入 > 0」
//   - 过热档（涨幅 > 15%）记入 excluded
//   - keyword 取最佳行业的 leaderName（供 t-candidate-search 的 search_stock 用）
//   ⚠️ IndustryRank 没有「1 月涨幅」字段，原 prompt 的「1 月涨幅 > 30% 排除」不在此实现。
//
// Rhai v1.25 约束：不支持 let mut（一律裸 let）；fn 是纯函数看不到注入变量，
// 故需要读注入变量的逻辑全部写在顶层。

// ── 通用防御（形态与 strategy-scorer.rhai 一致） ──
fn to_array(v) {
    if type_of(v) == "array" { v }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "array" { p } else { [] }
    } else if type_of(v) == "map" {
        if v["content"] != () && type_of(v["content"]) == "string" {
            let p = json_parse(v["content"].to_string());
            if type_of(p) == "array" { p }
            else if type_of(p) == "map" && p["data"] != () && type_of(p["data"]) == "array" { p["data"] }
            else { [] }
        } else if v["data"] != () && type_of(v["data"]) == "array" { v["data"] }
        else if v["items"] != () && type_of(v["items"]) == "array" { v["items"] }
        else { [] }
    } else { [] }
}

fn to_f64(v, fallback) {
    if type_of(v) == "f64" { v }
    else if type_of(v) == "i64" { v.to_float() }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "f64" { p } else if type_of(p) == "i64" { p.to_float() } else { fallback }
    } else { fallback }
}

/// map 字段 → 字符串（缺失 / 非字符串一律空串）
fn field_str(m, key) {
    if type_of(m) != "map" { return ""; }
    let v = m[key];
    if v == () { return ""; }
    if type_of(v) == "string" { return v; }
    v.to_string()
}

/// map 字段 → 浮点（缺失 / 非数值走 fallback）
fn field_f64(m, key, fallback) {
    if type_of(m) != "map" { return fallback; }
    let v = m[key];
    if v == () { return fallback; }
    to_f64(v, fallback)
}

/// 截断（Rhai 无 chars 迭代惯例，用 sub_string）
fn clip(s, max_len) {
    if type_of(s) != "string" { return ""; }
    if s.len() <= max_len { return s; }
    s.sub_string(0, max_len) + "…"
}

/// 6 位纯数字代码判据（逐字符查表；Rhai 字符串 contains(string) 为可靠子集）
fn is_code6(s) {
    if type_of(s) != "string" { return false; }
    if s.len() != 6 { return false; }
    let digits = "0123456789";
    for i in range(0, 6) {
        if !digits.contains(s.sub_string(i, 1)) { return false; }
    }
    true
}

/// 在 arr 中挑「changePct 最大且未被 picked」的下标；无线索返回 -1
fn pick_max_index(arr, picked) {
    let best = -1;
    let best_chg = -9999.0;
    for i in range(0, arr.len()) {
        let nm = field_str(arr[i], "name");
        if picked.contains(nm) { continue; }
        let chg = field_f64(arr[i], "changePct", -9999.0);
        if chg > best_chg {
            best_chg = chg;
            best = i;
        }
    }
    best
}

// ═══════════════════════════════════════════════
// 主流程（顶层：可直接读注入变量）
// ═══════════════════════════════════════════════

let rows = to_array(industry_ranking);
let data_available = rows.len() > 0;

// 三个档位：strict（2~15% 且流入为正）/ inflow（仅流入为正）/ excluded（过热）
let strict = [];
let inflow = [];
let excluded = [];

for r in rows {
    let nm = field_str(r, "industryName");
    if nm == "" { continue; }
    let chg = field_f64(r, "changePct", 0.0);
    let inf = field_f64(r, "mainInflow", 0.0);
    let ld = field_str(r, "leaderName");
    let lc = field_str(r, "leaderCode");
    let item = #{
        "name": nm,
        "changePct": chg,
        "mainInflow": inf,
        "leader_name": ld,
        "leader_code": lc,
    };
    if chg > 15.0 {
        excluded.push(#{ "name": nm, "changePct": chg, "reason": "涨幅过高(>15%)，已非萌芽期" });
    }
    if inf > 0.0 { inflow.push(item); }
    if chg >= 2.0 && chg <= 15.0 && inf > 0.0 { strict.push(item); }
};

// 档位选择：严格档优先，空则退化为「流入为正」
let tier = if strict.len() > 0 { strict } else { inflow };
let tier_name = if strict.len() > 0 { "strict" } else if inflow.len() > 0 { "inflow_only" } else { "none" };

// 取前 5 个（按 changePct 降序手选；Rhai 无非数值数组 sort 的可靠形态，故手选）
let hot_industries = [];
let picked = [];
let hot_limit = if tier.len() > 5 { 5 } else { tier.len() };
for k in range(0, hot_limit) {
    let idx = pick_max_index(tier, picked);
    if idx < 0 { break; }
    picked.push(field_str(tier[idx], "name"));
    hot_industries.push(tier[idx]);
};

// keyword：最佳行业的领涨股名（供 search_stock）；无领涨股则退化为行业名
let keyword = "";
let best_name = "";
let best_chg = 0.0;
let best_inflow = 0.0;
if hot_industries.len() > 0 {
    let b = hot_industries[0];
    best_name = field_str(b, "name");
    best_chg = field_f64(b, "changePct", 0.0);
    best_inflow = field_f64(b, "mainInflow", 0.0);
    let ln = field_str(b, "leader_name");
    keyword = if ln != "" { ln } else { best_name };
};

// 新闻亮点：财联社快讯 + 政策新闻标题（各取前 3，总上限 6）
let news_highlights = [];
let flashes = to_array(cls_flash);
let flash_limit = if flashes.len() > 3 { 3 } else { flashes.len() };
for i in range(0, flash_limit) {
    let t = clip(field_str(flashes[i], "title"), 60);
    if t != "" { news_highlights.push("【财联社】" + t); }
};
let policies = to_array(policy_news);
let policy_limit = if policies.len() > 3 { 3 } else { policies.len() };
for i in range(0, policy_limit) {
    let t = clip(field_str(policies[i], "title"), 60);
    if t != "" { news_highlights.push("【政策新闻】" + t); }
};

// 用户指定主题（字符串或数组）
let themes_text = "";
if user_themes != () {
    if type_of(user_themes) == "array" {
        for t in user_themes {
            if themes_text != "" { themes_text += "、"; }
            themes_text += t.to_string();
        }
    } else {
        themes_text = user_themes.to_string();
    }
};

// ── 组装 brief（纯文本，供下游 Agent 直接消费） ──
let brief = "【市场简报 · 确定性初筛】\n";
brief += "行业排名样本: " + rows.len().to_string() + " 个行业\n";
brief += "初筛档位: " + tier_name + "（严格档=涨幅2~15%且主力净流入>0）\n";
if hot_industries.len() > 0 {
    brief += "候选行业(按涨幅降序):\n";
    for h in hot_industries {
        brief += "  - " + field_str(h, "name")
            + " 涨幅 " + field_f64(h, "changePct", 0.0).to_string() + "%"
            + " 主力净流入 " + field_f64(h, "mainInflow", 0.0).to_string()
            + " 领涨股 " + field_str(h, "leader_name")
            + "(" + field_str(h, "leader_code") + ")\n";
    }
} else {
    brief += "候选行业: 无（行业排名数据缺失或全部不满足初筛条件）\n";
};
if excluded.len() > 0 {
    brief += "已排除(过热): ";
    for e in excluded {
        brief += field_str(e, "name") + " " + field_f64(e, "changePct", 0.0).to_string() + "% ";
    }
    brief += "\n";
};
if news_highlights.len() > 0 {
    brief += "消息面亮点:\n";
    for n in news_highlights { brief += "  - " + n + "\n"; }
};
if themes_text != "" {
    brief += "用户指定主题: " + themes_text + "（主题优先于上述市场初筛）\n";
};
brief += "检索关键词: " + keyword;

#{
    "brief": brief,
    "keyword": keyword,
    "hot_industries": hot_industries,
    "excluded": excluded,
    "news_highlights": news_highlights,
    "data_available": data_available
}