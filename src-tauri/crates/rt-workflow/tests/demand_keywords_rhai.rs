// demand-keywords.rhai 脚本健康测试：用真 Rhai 引擎编译并执行四种输入形态。
// 背景：v5 脚本 `as int` 触发 Rhai 1.25/1.26 解析 bug（Expecting ';'）导致
// c-keywords 节点 VALIDATION_FAILED、Loop 空转、线索 0（2026-09-09 实证）。
// 本测试防止同类回归再次静默上线。

use rhai::{Dynamic, Engine, Scope};

const SCRIPT: &str = include_str!("../../../src/commands/opc_setup/demand-keywords.rhai");

fn make_engine() -> Engine {
    let mut engine = Engine::new();
    // json_parse 桩：与 harness register_common_functions 行为对齐（失败返回 ()）
    engine.register_fn("json_parse", |s: &str| {
        let v: serde_json::Value = serde_json::from_str(s).unwrap_or(serde_json::Value::Null);
        match serde_json::from_value::<Dynamic>(v) {
            Ok(d) => d,
            Err(_) => Dynamic::UNIT,
        }
    });
    engine
}

fn run(plan: Dynamic, max_keywords: f64) -> Vec<String> {
    let engine = make_engine();
    let mut scope = Scope::new();
    scope.push("keyword_plan", plan);
    scope.push("max_keywords", max_keywords);
    let result: Dynamic = engine.eval_with_scope(&mut scope, SCRIPT).expect("脚本执行失败");
    let arr = result.into_array().expect("返回值不是数组");
    arr.into_iter().map(|d| d.to_string()).collect::<Vec<_>>()
}

/// 构造真 rhai 数组（注意：Dynamic::from(Vec<String>) 会保留 Rust 类型名，
/// type_of 不是 "array"；生产路径从 JSON 反序列化而来是真 array，此处对齐）
fn str_array(items: &[&str]) -> Dynamic {
    Dynamic::from(items.iter().map(|s| Dynamic::from(s.to_string())).collect::<rhai::Array>())
}

#[test]
fn script_compiles() {
    let engine = make_engine();
    engine.compile(SCRIPT).expect("脚本编译失败");
}

/// 主路径（生产实证形态）：input_mapping 点路径已把 LLM 的 JSON 字符串解析成数组
#[test]
fn array_input_dedup_and_cap() {
    let kws = run(
        str_array(&[
            "AI视频制作",
            "AI视频制作", // 重复 → 去重
            "  ",         // 空白 → 剔除
            "RPA开发",
            "爬虫代做",
            "网站开发",
            "PPT代做",
            "数据分析代做",
        ]),
        5.0,
    );
    assert_eq!(kws, vec!["AI视频制作", "RPA开发", "爬虫代做", "网站开发", "PPT代做"]);
}

/// 上限钳制：cap=99 → 钳到 10
#[test]
fn cap_clamped_to_ten() {
    let words: Vec<String> = (0..19).map(|i| format!("词{i}")).collect();
    let refs: Vec<&str> = words.iter().map(|s| s.as_str()).collect();
    let kws = run(str_array(&refs), 99.0);
    assert_eq!(kws.len(), 10);
}

/// 下限钳制：cap=0 → 钳到 1
#[test]
fn cap_clamped_to_one() {
    let kws = run(str_array(&["词0", "词1", "词2", "词3", "词4"]), 0.0);
    assert_eq!(kws.len(), 1);
}

/// JSON 字符串形态（LLM 直接回纯 JSON 数组文本）
#[test]
fn json_string_input() {
    let kws = run(Dynamic::from("[\"A\",\"B\",\"C\"]".to_string()), 5.0);
    assert_eq!(kws, vec!["A", "B", "C"]);
}

/// 纯文本兜底：围栏行跳过、"- " 前缀剥离
#[test]
fn plain_text_fallback() {
    let text = "```text\n- 关键词一\n关键词二\n```\n关键词三".to_string();
    let kws = run(Dynamic::from(text), 5.0);
    assert_eq!(kws, vec!["关键词一", "关键词二", "关键词三"]);
}

/// 全空兜底：返回默认词表
#[test]
fn empty_plan_falls_back_to_defaults() {
    let kws = run(Dynamic::UNIT, 5.0);
    assert_eq!(kws, vec!["AI", "软件", "设计", "营销", "写作", "翻译"]);
}
