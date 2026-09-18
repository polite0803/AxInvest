//! 行为测试：data-verifier.rhai 路径4（report 双重编码 + 围栏）解析验证。
//!
//! 背景（2026-09-11 run d9b7a146 / 7e3801c8 实锤）：a-candidate-mapper content 为
//! {"report":"<json字符串>"} 双重编码形态，report 值可能带 ```tool_json 围栏，
//! data-verifier 路径3 整体 json_parse 得到 {report:"..."} 后 candidates 不可达 →
//! 返回 [] → 最终无候选输出。本测试用真实故障样本验证 extract_json_block + 路径4
//! 能恢复出 candidates。
//!
//! 注意：json_parse 桩与 harness::rhai_engine 行为对齐——serde_json 解析、失败返回
//! Dynamic::UNIT。围栏剥离不在 json_parse 内（harness 未实现），由脚本内
//! extract_json_block 负责，故本测试的桩不含围栏处理。
use rhai::Engine;

fn json_parse_stub(s: &str) -> rhai::Dynamic {
    // 与 harness::rhai_engine::register_common_functions 对齐：serde_json 解析，
    // 失败（含围栏文本）返回 Dynamic::UNIT
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(v) => rhai::serde::to_dynamic(&v).unwrap_or(rhai::Dynamic::UNIT),
        Err(_) => rhai::Dynamic::UNIT,
    }
}

fn register_globals(engine: &mut Engine) {
    engine.register_fn("json_parse", |s: &str| -> rhai::Dynamic { json_parse_stub(s) });
    engine.register_fn("print", |_s: &str| {});
}

/// 直接执行 data-verifier.rhai 中的 extract_json_block + 路径4 逻辑。
/// 抽取脚本里我们关心的片段（fn extract_json_block + candidates_from_raw 路径4 分支），
/// 喂入 candidates_raw 并返回解析出的数组长度。
fn run_parse(candidates_raw: &str) -> Result<i64, Box<rhai::EvalAltResult>> {
    let mut engine = Engine::new();
    engine.set_max_expr_depths(1024, 1024);
    register_globals(&mut engine);
    let script = r#"
fn extract_json_block(s) {
    let start = s.index_of("{");
    if start < 0 { return s; }
    let n = s.len;
    for i in range(n - 1, start - 1, -1) {
        if s.sub_string(i, 1) == "}" {
            return s.sub_string(start, i - start + 1);
        }
    }
    s.sub_string(start, n - start)
}
// 与 data-verifier.rhai 路径3+4 一致的最小复现
let candidates_from_raw = if type_of(candidates_raw) == "string" {
    let p = json_parse(candidates_raw);
    if type_of(p) == "array" { p }
    else if type_of(p) == "map" && p["candidates"] != () && type_of(p["candidates"]) == "array" {
        p["candidates"]
    } else if type_of(p) == "map" && p["report"] != () && type_of(p["report"]) == "string" {
        let q = json_parse(extract_json_block(p["report"]));
        if type_of(q) == "array" { q }
        else if type_of(q) == "map" && q["candidates"] != () && type_of(q["candidates"]) == "array" {
            q["candidates"]
        } else { [] }
    } else { [] }
} else { [] };
candidates_from_raw.len()
"#;
    let mut scope = rhai::Scope::new();
    scope.push("candidates_raw", candidates_raw.to_string());
    engine.eval_with_scope::<i64>(&mut scope, script)
}

#[test]
fn path4_report_double_encoded_plain() {
    // run d9b7a146 形态：content = {"report":"{\"candidates\":[...]}"}
    let raw = r#"{"report":"{\"candidates\":[{\"stock_code\":\"002353\",\"stock_name\":\"杰瑞股份\"}],\"summary\":\"ok\"}"}"#;
    let n = run_parse(raw).expect("脚本执行失败");
    assert_eq!(n, 1, "report 双编码（无围栏）应恢复 1 个候选，实际 {n}");
}

#[test]
fn path4_report_double_encoded_with_fence() {
    // run 7e3801c8 形态：report 值带 ```tool_json 围栏
    let raw = "{\n  \"report\": \"```tool_json\\n{\\\"candidates\\\":[{\\\"stock_code\\\":\\\"002377\\\",\\\"stock_name\\\":\\\"国创高新\\\"},{\\\"stock_code\\\":\\\"002353\\\"}]},\\n```\"\n}";
    let n = run_parse(raw).expect("脚本执行失败");
    assert_eq!(n, 2, "report 双编码（带围栏）应恢复 2 个候选，实际 {n}");
}

#[test]
fn path3_direct_arguments_json_still_works() {
    // 回归：路径3 直接是 {"candidates":[...]} 文本（v44 形态）不受影响
    let raw = r#"{"candidates":[{"stock_code":"300285"}]}"#;
    let n = run_parse(raw).expect("脚本执行失败");
    assert_eq!(n, 1);
}

#[test]
fn path3_bare_array_still_works() {
    // 回归：裸候选数组文本（v44 形态）不受影响
    let raw = r#"[{"stock_code":"300285"}]"#;
    let n = run_parse(raw).expect("脚本执行失败");
    assert_eq!(n, 1);
}

#[test]
fn path4_garbage_report_returns_zero() {
    // report 内层非 JSON → 不 panic、返回 0（json_parse 桩返回 ()，脚本走 [] 分支）
    let raw = r#"{"report":"这不是JSON，是纯文本报告"}"#;
    let n = run_parse(raw).expect("脚本执行失败（不允许 panic）");
    assert_eq!(n, 0);
}
