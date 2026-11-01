// candidate-pool.rhai — 快速趋势智选 Phase D：候选代码池（确定性清洗）
//
// 归属：`serenity-screening-fast` 模板新增资产（原链无对应脚本）。
//
// 输入：
//   constituents   — t-constituents.result（get_industry_constituents 信封 JSON 文本：
//                    {status,data:[{industry,matchedIndustry,matchMode,stocks:[{stockCode,stockName}]}]})
//   search_results — t-candidate-search.result（search_stock，JSON 文本 [{code,name,market}]）
//   scanner_brief  — c-scanner-brief.result（Phase B 的 map 输出）
// 输出：
//   {stocks, labels, pool_text, count, sources, industry_groups, unmatched_industries,
//    label_truncated}
//
// 口径（2026-10-31 阶段 2「候选池改趋势→全关联股票」）：
//   来源优先级 ① 各热门行业**全部成分股**（t-constituents，几十~上百只，这是本改造的主体）
//             ② brief.hot_industries 的领涨股
//             ③ search_stock 命中（降为兜底——LLM 自由检索此前正是「候选老是那几只」的根源）
//   stocks = 全量池（1000 安全上限）；labels 供 j-candidate-pick 分类器，
//   按**行业桶轮转交错**截断到 LABELS_CAP（保持行业多样性，否则截断头全是第一大行业），
//   label_truncated=true 时 labels 只是池的抽样视图（top_pick 本就是第三优先来源，见合并 prompt）。
//   ⚠️ labels 不带序号：LlmClassifierExecutor 会对每个类别前缀 "N. "（llm_classifier_executor.rs:76-81），
//      脚本再带序号会变成 "1. 1. 300285 国瓷材料" 双重编号。序号只出现在人读的 pool_text 里。
//   宇宙表 27% 个股行业为空 ⇒ 不在成分组里 —— 属数据事实，由 unmatched_industries 与
//   pool_text 的分组计数如实呈现，不做静默补齐。

fn to_array(v) {
    if type_of(v) == "array" { v }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        // search_news/search_stock 自 2026-10-28 起返回 {status,data,detail} 信封
        // ⇒ 注入串带信封，必须下钻 data（否则候选池恒空 = 「今天搜不到票」的假象）。
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
        else if v["items"] != () && type_of(v["items"]) == "array" { v["items"] }
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

/// 6 位纯数字代码判据
fn is_code6(s) {
    if type_of(s) != "string" { return false; }
    if s.len() != 6 { return false; }
    let digits = "0123456789";
    for i in range(0, 6) {
        if !digits.contains(s.sub_string(i, 1)) { return false; }
    }
    true
}

/// 可入池判据：6 位代码 + 非 ST / 退市 / 空名
fn acceptable(code, name) {
    if !is_code6(code) { return false; }
    if name == "" { return false; }
    let upper = name.to_upper();
    if upper.contains("ST") { return false; }
    if name.contains("退") { return false; }
    true
}

let pool_limit = 1000;
let labels_cap = 120;

let stocks = [];
let codes = [];
let sources = [];
let industry_groups = [];
let unmatched_industries = [];

// ── 来源 1：各热门行业全部成分股 ──
let groups = to_array(constituents);
for g in groups {
    let ind = field_str(g, "industry");
    let matched = field_str(g, "matchedIndustry");
    let mode = field_str(g, "matchMode");
    let members = to_array(g["stocks"]);
    let group_count = 0;
    for m in members {
        let code = field_str(m, "stockCode");
        let name = field_str(m, "stockName");
        if !acceptable(code, name) { continue; }
        if codes.contains(code) { continue; }
        codes.push(code);
        stocks.push(#{ "code": code, "name": name, "source": "industry_constituents", "industry": matched });
        group_count += 1;
    };
    if mode == "missing" {
        unmatched_industries.push(ind);
    } else if group_count > 0 {
        industry_groups.push(#{ "industry": ind, "matched": matched, "match_mode": mode, "count": group_count });
        if !sources.contains("industry_constituents") { sources.push("industry_constituents"); }
    };
};

// ── 来源 2：brief 里的行业领涨股 ──
if type_of(scanner_brief) == "map" {
    let hot = to_array(scanner_brief["hot_industries"]);
    for hr in hot {
        let code = field_str(hr, "leader_code");
        let name = field_str(hr, "leader_name");
        if !acceptable(code, name) { continue; }
        if codes.contains(code) { continue; }
        codes.push(code);
        stocks.push(#{ "code": code, "name": name, "source": "industry_leader", "industry": "" });
    };
    if hot.len() > 0 && !sources.contains("industry_leader") { sources.push("industry_leader"); }
};

// ── 来源 3：search_stock 命中（兜底） ──
let hits = to_array(search_results);
for h in hits {
    let code = field_str(h, "code");
    let name = field_str(h, "name");
    if !acceptable(code, name) { continue; }
    if codes.contains(code) { continue; }
    codes.push(code);
    stocks.push(#{ "code": code, "name": name, "source": "search_stock", "industry": "" });
};
if hits.len() > 0 && !sources.contains("search_stock") { sources.push("search_stock"); }

// ── 池安全截断 ──
let total_after_clean = stocks.len();
let pool_limit = if total_after_clean > pool_limit { pool_limit } else { total_after_clean };
let pool = [];
for i in range(0, pool_limit) { pool.push(stocks[i]); };

// ── labels：行业桶轮转交错截断（保持行业多样性）──
// 桶序 = 池内出现序（成分组按涨幅降序在前，领涨/检索归「其它」桶殿后）。
let buckets = [];
let bucket_names = [];
for s in pool {
    let ind = field_str(s, "industry");
    if ind == "" { ind = "其它"; }
    let bi = -1;
    for k in range(0, bucket_names.len()) {
        if bucket_names[k] == ind { bi = k; break; }
    };
    if bi < 0 {
        bucket_names.push(ind);
        buckets.push([field_str(s, "code") + " " + field_str(s, "name")]);
    } else {
        buckets[bi].push(field_str(s, "code") + " " + field_str(s, "name"));
    };
};
let labels = [];
let cursor = [];
for b in buckets { cursor.push(0); };
let progressed = true;
while labels.len() < labels_cap && progressed {
    progressed = false;
    for bi in range(0, buckets.len()) {
        if labels.len() >= labels_cap { break; }
        let i = cursor[bi];
        if i < buckets[bi].len() {
            labels.push(buckets[bi][i]);
            cursor[bi] = i + 1;
            progressed = true;
        }
    };
};
let label_truncated = total_after_clean > labels.len();

// ── pool_text：按行业分组的完整清单（scanner LLM 消费）──
let pool_text = "候选池总数 " + total_after_clean.to_string() + "（行业成分 " + pool.len().to_string() + " 只，上限 1000）";
for g in industry_groups {
    pool_text += "\n【" + g["industry"] + "】成分股 " + g["count"].to_string() + " 只（匹配=" + g["match_mode"] + "）";
};
let seq = 0;
let cur_ind = "?";
for s in pool {
    let ind = field_str(s, "industry");
    if ind != cur_ind {
        cur_ind = ind;
        let head = if ind == "" { "【领涨股/检索命中】" } else { "【" + ind + "】" };
        pool_text += "\n" + head;
    };
    seq += 1;
    pool_text += "\n  " + seq.to_string() + ". " + field_str(s, "code") + " " + field_str(s, "name");
};
if unmatched_industries.len() > 0 {
    pool_text += "\n⚠ 未匹配到成分股的行业: ";
    for u in unmatched_industries { pool_text += u + " "; };
};
if total_after_clean == 0 {
    pool_text = "（候选池为空：行业成分股/领涨股/search_stock 三路均无命中）";
};

#{
    "stocks": pool,
    "labels": labels,
    "pool_text": pool_text,
    "count": pool.len(),
    "total_after_clean": total_after_clean,
    "sources": sources,
    "industry_groups": industry_groups,
    "unmatched_industries": unmatched_industries,
    "label_truncated": label_truncated
}
