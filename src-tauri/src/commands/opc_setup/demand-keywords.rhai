// demand-keywords.rhai — 能力驱动需求发现的检索词规划（确定性后处理）
//
// 上游 a-keywords（Agent）产出检索词；本脚本做确定性清洗与兜底，
// 保证下游 Loop 节点拿到的一定是干净的关键词数组（哪怕上游全链失败）。
//
// keyword_plan 实际形态（2026-09-09 执行实证）：
//   A. 字符串数组 —— input_mapping 点路径解析对 JSON 字符串做 auto JSON.parse（主路径）
//   B. JSON 字符串 —— json_parse 解析
//   C. 纯文本 —— 按行拆分兜底（跳过围栏行与 "-" 前缀）
//
// Rhai 1.25/1.26 兼容（全部实证踩坑，测试见 rt-workflow/tests/demand_keywords_rhai.rs）：
//   - 禁 `let mut`（关键字解析 bug）
//   - 禁 `as int`（解析器报 Expecting ';'）→ 全程 f64，int/float 混合比较原生支持
//   - trim() 是 &mut 原地修改方法，返回 () —— 必须当语句用：
//       `let t = kw; t.trim();` ✓    `let t = kw.trim();` ✗（t 恒为 ()）
//   - 去前缀用 sub_string()（不是 JS 的 substring，不存在）
//   - 函数体表达式深度限制：debug 构建仅 16 层（release 32，rhai limits.rs）——
//     嵌套控制流必须拍平（早 return / continue 代替嵌套 if），否则 debug 下
//     "Expression exceeds maximum complexity"、release 下又能跑，极易漏测
//   - 函数内修改数组参数不传播（Dynamic 按值传参），push_unique 返回新数组
//
// 返回字符串数组 → Loop 节点（iter_input_var = "c-keywords.result"）
// 逐词调用 OpcDiscoverLeads 工具扫描入库。

fn present(x) { type_of(x) != "()" }
fn is_str(x) { type_of(x) == "string" }

// 去重追加：存在同值时原样返回，否则 push 后返回
fn push_unique(arr, kw) {
    for existing in arr {
        if existing == kw {
            return arr;
        }
    }
    arr.push(kw);
    arr
}

// 从字符串数组收集关键词（去空 + 去重；continue 拍平嵌套）
fn collect_from_array(arr) {
    let out = [];
    if type_of(arr) != "array" {
        return out;
    }
    for kw in arr {
        if !is_str(kw) {
            continue;
        }
        let t = kw;
        t.trim();
        if t == "" {
            continue;
        }
        out = push_unique(out, t);
    }
    out
}

// 从纯文本按行收集（跳过空行 / markdown 围栏 / 列表序号前缀）
fn collect_from_text(text) {
    let out = [];
    for line in text.split("\n") {
        let t = line;
        t.trim();
        if t == "" || t.starts_with("`") {
            continue;
        }
        if t.starts_with("-") {
            t = t.sub_string(1, t.len());
            t.trim();
        }
        if t != "" {
            out = push_unique(out, t);
        }
    }
    out
}

// JSON 字符串 → 数组收集；解析失败回退按行拆分
fn collect_from_plan(plan) {
    let parsed = json_parse(plan);
    if type_of(parsed) == "array" {
        return collect_from_array(parsed);
    }
    collect_from_text(plan)
}

// 数量上限：max_keywords 来自模板变量（code_executor 把 JSON 数字统一转 f64 注入）
// 保持 f64 全程不整数化，int/float 混合比较 Rhai 原生支持 —— 彻底绕开 `as int` 解析 bug
let cap = 5.0;
if type_of(max_keywords) == "f64" {
    cap = max_keywords;
} else if type_of(max_keywords) == "i64" {
    cap = max_keywords * 1.0;
}
if cap < 1.0 { cap = 1.0; }
if cap > 10.0 { cap = 10.0; }

let kws = [];

if type_of(keyword_plan) == "array" {
    // 主路径：input_mapping 已 auto JSON.parse
    kws = collect_from_array(keyword_plan);
} else if is_str(keyword_plan) {
    let plan = keyword_plan;
    plan.trim();
    if plan != "" {
        kws = collect_from_plan(plan);
    }
}

// 截断到上限（超出部分从尾部丢弃；len() 是 int，与 f64 混合比较合法）
while kws.len() > cap {
    kws.pop();
}

// 全空兜底：与旧 extract_domain_queries 的默认词表保持同源
if kws.len() == 0 {
    kws = push_unique(kws, "AI");
    kws = push_unique(kws, "软件");
    kws = push_unique(kws, "设计");
    kws = push_unique(kws, "营销");
    kws = push_unique(kws, "写作");
    kws = push_unique(kws, "翻译");
}

kws
