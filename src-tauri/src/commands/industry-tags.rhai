// industry-tags.rhai — 原链趋势智选：聚合各趋势槽位的 industry_tags
//
// 归属：`serenity` 原链模板新增资产（阶段 2「候选池改趋势→全关联股票」）。
//
// 输入（input_mapping 注入，取自 c-trend-split 的具名槽位）：
//   trend1..trend5 — 各趋势对象（a-trend-scanner 输出经 trend-split.rhai 分槽；
//                    scanner prompt 自 v64 起要求每趋势给 industry_tags 1-3 个，
//                    逐字取自 industry_ranking 行业名）
// 输出：
//   {tags: [...去重后行业名], tags_text: "、"-连接串, tag_count: n}
//
// 设计口径：纯确定性聚合，零 token。tags 供 t-constituents
// （get_industry_constituents）按行业拉全成分股，注入 a-candidate-mapper 作
// 确定性候选池 —— 断掉「模型凭训练知识自由选票」这一「候选老是那几只」的根源。
// scanner 没给 industry_tags（旧版本模型行为）时 tags 为空数组 ⇒ 下游工具
// industries=[] 显式报错、mapper 拿到空池说明 —— 缺席如实呈现，不伪装。

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
        else { [] }
    } else { [] }
}

let slots = #{};
slots["trend1"] = trend1;
slots["trend2"] = trend2;
slots["trend3"] = trend3;
slots["trend4"] = trend4;
slots["trend5"] = trend5;

let tags = [];
let slot_states = [];
for name in ["trend1", "trend2", "trend3", "trend4", "trend5"] {
    let t = slots[name];
    let slot_tags = [];
    if type_of(t) == "map" {
        for tag in to_array(t["industry_tags"]) {
            let s = "";
            if type_of(tag) == "string" { s = tag; }
            if s != "" && !tags.contains(s) {
                tags.push(s);
                slot_tags.push(s);
            }
        }
    };
    slot_states.push(#{ "slot": name, "tags": slot_tags });
};

let tags_text = "";
for t in tags {
    if tags_text != "" { tags_text += "、"; }
    tags_text += t;
};

#{
    "tags": tags,
    "tags_text": tags_text,
    "tag_count": tags.len(),
    "slot_states": slot_states
}
