// SPDX-License-Identifier: AGPL-3.0-only

//! Rhai 脚本引擎适配器契约 + 通用函数注册。
//!
//! 提供 Rhai 脚本的编译和执行能力，用于工作流中的动态脚本节点。
//! 同时提供 `register_common_functions` 等纯函数，供所有执行 Rhai 脚本的 Engine 实例复用，
//! 避免分散注册导致遗漏（历史上 rt-workflow / quant / market-sim 各自维护一份导致 bug）。

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rhai::Engine;
use serde_json::Value as JsonValue;

/// Rhai 脚本引擎适配器契约
///
/// 封装 Rhai 脚本的批量编译和按名执行能力。
/// 实现方（`axagent-tools::rhai_engine`）管理内部脚本缓存。
pub trait RhaiEngineAdapter: fmt::Debug + Send + Sync {
    /// 批量注册并编译脚本（在工作流初始化时调用）
    ///
    /// `scripts`：脚本定义数组，每个元素为 `{ "tool_name": "...", "code": "..." }`
    fn register_scripts(&self, scripts: &[JsonValue]);

    /// 执行已注册的指定脚本
    ///
    /// - `script_name`：要执行的脚本名称（与注册时的 `tool_name` 对应）
    /// - `args`：输入参数
    /// - `tool_fns`：可被脚本调用的工具函数映射，key=工具名，value= `(name, args) -> Result`
    fn execute_script(
        &self,
        script_name: &str,
        args: JsonValue,
        tool_fns: &HashMap<String, RhaiToolFn>,
    ) -> Result<JsonValue, String>;
}

/// Rhai 可调用工具函数
///
/// 签名：`(工具名, JSON参数) -> Result<JSON结果, 错误信息>`
pub type RhaiToolFn = Arc<dyn Fn(String, JsonValue) -> Result<JsonValue, String> + Send + Sync>;

// ────────────────────────────────────────────────────────────────────────────
// 通用 Rhai 函数注册（下沉自 rt-workflow::code_executor，消除 quant 同义重复定义）
// ────────────────────────────────────────────────────────────────────────────

/// JSON 数字 → f64；非数值（unit / string / bool / 地图…）→ `None`。
///
/// 整数走 `as_int`（嵌层注入的整数是 i64，见 [`register_common_functions`] 的口径说明），
/// 浮点走 `as_float`。**不做字符串解析** —— 上游契约是 JSON Number，把 `"22.7"` 当数字
/// 参与比较会静默产出错误档位（`risk-level.rhai` / `trader-proxy.rhai` 的同口径注释）。
fn num_to_f64(x: &rhai::Dynamic) -> Option<f64> {
    if let Ok(i) = x.as_int() {
        // `f64: From<i64>` / `TryFrom<i64>` 都不存在（可能损精度，std 只给 i32 及更窄的），
        // 故用 `as`：价带/天数/置信度这些量纲远小于 2^53，损精度不是本域的实际风险；
        // 真要保精度应当整链改成定点小数，那是另一件事。
        return Some(i as f64);
    }
    x.as_float().ok()
}

/// 注册通用 Rhai 函数（clamp / join / json_parse / num_of）。
///
/// 所有执行 Rhai 脚本的 Engine 实例都应调用此函数，确保脚本可用的
/// 自定义函数集一致，避免分散注册导致遗漏。
///
/// 历史背景：原 `register_common_functions` 定义在 `rt-workflow::code_executor`，
/// 但 `quant` / `market-sim` 等 consumer crate 按依赖铁律不能依赖 rt-workflow（hybrid 层），
/// 导致各自维护一份 `json_value_to_rhai` 同义实现（违反铁律 4「禁止重复定义」）。
/// 下沉到 harness（foundation 层）后，所有 crate 通过 `pub use` 复用同一份实现。
///
/// 参考：portfolio-mgr.rhai / consistency-check.rhai / bottleneck-calc.rhai 等脚本
/// 均依赖 `json_parse`；`clamp` 用于信号夹紧；`join` 用于数组拼接。
pub fn register_common_functions(engine: &mut Engine) {
    engine.register_fn("clamp", |value: f64, min: f64, max: f64| -> f64 {
        if value < min {
            min
        } else if value > max {
            max
        } else {
            value
        }
    });
    // ── 数值桥 `num_of`：脚本侧读 JSON 数字的**唯一**正确姿势 ──
    //
    // 为什么必须是引擎级共享函数，而不是各脚本各写一份本地 `fn num_of`：
    // JSON 数字落进 Rhai 有**两条形态不同的注入通道** ——
    //   · 顶层标量参数：`code_executor` 的 `Value::Number` 分支先 `as_f64` ⇒ 一律 f64；
    //   · 嵌在 map/array 里的（含**顶层 map/array 参数的每一层**、`json_parse()` 的产物）：
    //     走本 crate 的 [`json_value_to_dynamic`]，`as_i64` 优先 ⇒ **整数是 i64**。
    // 于是 `type_of(x) == "f64"` 只在第一种形态下等价于「x 是个数」，拿它当数值判据
    // 会把整数**静默判成缺席**（2026-10-04 实证：分支表 `days`/`volLookbackDays` 是整数 ⇒
    // 四档价带恒 0）。此前 `portfolio-mgr.rhai` / `risk-level.rhai` / `trader-proxy.rhai`
    // 各有一份同语义实现（铁律 4 的重复定义），现统一到这里；`"int"` 那个分支在 64-bit
    // 构建下恒不成立（`type_of` 给 `"i64"`），故不再保留。
    //
    // 两个元数语义不同，都注册：
    //   · `num_of(x)` → f64，非数值 → unit（调用点自己决定缺席语义，**不许**用哨兵值伪装读数）；
    //   · `num_of(x, dflt)` → f64 或 `dflt`（既有两个脚本用的就是这版，逐字保留语义）。
    engine.register_fn("num_of", |x: rhai::Dynamic| -> rhai::Dynamic {
        match num_to_f64(&x) {
            Some(f) => rhai::Dynamic::from(f),
            None => rhai::Dynamic::UNIT,
        }
    });
    engine.register_fn("num_of", |x: rhai::Dynamic, dflt: f64| -> f64 {
        num_to_f64(&x).unwrap_or(dflt)
    });
    engine.register_fn("join", |arr: rhai::Array, sep: &str| -> String {
        arr.iter().map(|item| item.to_string()).collect::<Vec<_>>().join(sep)
    });
    engine.register_fn("json_parse", |s: &str| -> rhai::Dynamic {
        match serde_json::from_str::<serde_json::Value>(s) {
            Ok(v) => json_value_to_dynamic(&v),
            Err(first_err) => {
                // 2026-07-31 完整修复：LLM 输出的 JSON 常带尾逗号（如 "key": "value",\n}），
                // serde_json 严格模式直接失败 → Rhai 脚本 json_parse 返回 () → 上游 no_data
                // （21:47 轮 c-scorer-trend2 实锤：trailing comma at line 73）。
                // 清理"逗号后紧跟 } 或 ]（含中间空白）"的尾逗号后再试一次，全部脚本受益。
                let cleaned = strip_trailing_commas(s);
                if cleaned != s {
                    match serde_json::from_str::<serde_json::Value>(&cleaned) {
                        Ok(v) => json_value_to_dynamic(&v),
                        Err(second_err) => {
                            tracing::warn!(
                                "[harness::rhai_engine] json_parse 失败（清理尾逗号后仍失败）: {second_err}"
                            );
                            rhai::Dynamic::UNIT
                        },
                    }
                } else {
                    tracing::warn!("[harness::rhai_engine] json_parse 失败: {first_err}");
                    rhai::Dynamic::UNIT
                }
            },
        }
    });
}

/// 注册领域本体查询函数（当前仅 `band_for_score`）。
///
/// 为什么不并入 `register_common_functions`：本体查询属于**域知识**，不是「Rhai 语言
/// 能力补充」；不同引擎按需取舍（动态工具引擎要、纯表达式求值引擎不需要）。
///
/// 历史缺陷（2026-09-22）：`band_for_score` 此前**只**在
/// `crates/tools/src/rhai_engine.rs` 内联注册，共享 Engine（DAG 主路径）与 AxInvest
/// 本地自建 Engine（What-If / rerun）都没有 ⇒ `strategy-scorer.rhai:68` 与
/// `bottleneck-calc.rhai:148` 一旦真的执行就报 `Function not found`；
/// 与 `bottleneck_node_score` 完全同型（同批修复）。
/// 权威实现是 [`crate::domain_ontology::band_for_score`]，此处只做薄包装。
pub fn register_ontology_functions(engine: &mut Engine) {
    engine.register_fn("band_for_score", |score: f64| -> String {
        crate::domain_ontology::band_for_score(score).to_string()
    });
}

/// 清理 JSON 文本中的尾逗号（LLM 输出常见病：`"key": "value",}` / `[1,2,]`）。
///
/// 逐字符扫描：遇到逗号时，若其后方（跳过空白）紧跟 `}` 或 `]`，则该逗号是尾逗号，丢弃。
/// 不修改任何其他内容（保留原字符串格式与空白）。
fn strip_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ',' {
            // 跳过逗号后的空白，看第一个非空白字符
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], ' ' | '\t' | '\n' | '\r') {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                // 尾逗号：丢弃，跳到空白后的 } / ]
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 将 `serde_json::Value` 转换为 Rhai `Dynamic`。
///
/// 整数语义：JSON Number 优先尝试 `as_i64()` 保持整数类型，避免整数精度丢失
/// （原 rt-workflow 版本优先 `as_f64` 会把 JSON 整数 5 静默变成 Rhai float 5.0）。
/// 采纳自 `quant::script::json_value_to_rhai` 的更正确实现。
pub fn json_value_to_dynamic(v: &serde_json::Value) -> rhai::Dynamic {
    match v {
        serde_json::Value::Null => rhai::Dynamic::UNIT,
        serde_json::Value::Bool(b) => rhai::Dynamic::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                rhai::Dynamic::from(i)
            } else if let Some(f) = n.as_f64() {
                rhai::Dynamic::from(f)
            } else {
                rhai::Dynamic::UNIT
            }
        },
        serde_json::Value::String(s) => rhai::Dynamic::from(s.clone()),
        serde_json::Value::Array(arr) => {
            let items: rhai::Array = arr.iter().map(json_value_to_dynamic).collect();
            rhai::Dynamic::from_array(items)
        },
        serde_json::Value::Object(obj) => {
            let mut map = rhai::Map::new();
            for (k, v) in obj {
                map.insert(k.clone().into(), json_value_to_dynamic(v));
            }
            map.into()
        },
    }
}

/// 将 Rhai `Dynamic` 转换回 `serde_json::Value`。
///
/// 与 [`json_value_to_dynamic`] 互为反函数。注意 Rhai 的整数和浮点数
/// 会分别映射到 JSON Number 的 i64 / f64 表示。
pub fn dynamic_to_json_value(v: &rhai::Dynamic) -> serde_json::Value {
    if v.is_unit() {
        return serde_json::Value::Null;
    }
    if v.is_bool() {
        return serde_json::Value::Bool(v.as_bool().unwrap_or(false));
    }
    if let Ok(s) = v.clone().into_string() {
        return serde_json::Value::String(s);
    }
    if let Ok(i) = v.as_int() {
        return serde_json::Value::Number(serde_json::Number::from(i));
    }
    if let Ok(f) = v.as_float() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return serde_json::Value::Number(n);
        }
        return serde_json::Value::Number(serde_json::Number::from(0));
    }
    // Array
    if let Some(arr) = v.clone().try_cast::<rhai::Array>() {
        return serde_json::Value::Array(
            arr.into_iter().map(|item| dynamic_to_json_value(&item)).collect(),
        );
    }
    // Map
    if let Some(map) = v.clone().try_cast::<rhai::Map>() {
        let mut obj = serde_json::Map::new();
        for (k, val) in &map {
            obj.insert(format!("{k}"), dynamic_to_json_value(val));
        }
        return serde_json::Value::Object(obj);
    }
    serde_json::Value::String(format!("{v}"))
}

// 空实现 — 总是失败（Rhai 引擎未配置）
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::NoopRhaiEngineAdapter;

    #[test]
    fn noop_never_succeeds_on_execute() {
        let adapter = NoopRhaiEngineAdapter;
        adapter.register_scripts(&[]);
        let result = adapter.execute_script("test", JsonValue::Null, &HashMap::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not configured"));
    }

    #[test]
    fn register_common_functions_provides_clamp_join_json_parse() {
        let mut engine = Engine::new();
        register_common_functions(&mut engine);
        // clamp
        let r: f64 = engine.eval_expression("clamp(15.0, 0.0, 10.0)").unwrap();
        assert_eq!(r, 10.0);
        // json_parse: eval_expression 只能解析表达式，不能含 let 语句，
        // 改用 eval 解析整段脚本。json_parse 返回 Map，通过 ["x"] 索引访问。
        let r: i64 = engine.eval::<i64>("let obj = json_parse(`{\"x\": 5}`); obj[\"x\"]").unwrap();
        assert_eq!(r, 5);
        // join
        let r: String = engine.eval_expression::<String>("join([1, 2, 3], \", \")").unwrap();
        assert_eq!(r, "1, 2, 3");
    }

    /// 数值桥 `num_of` 的三件事：**嵌层整数算数**、**缺席保持显式**、**默认值形态**。
    ///
    /// 为什么单独立一条门而不是只靠脚本侧注释：`type_of(x) == "f64"` 对嵌层整数恒假
    /// （注入落 i64），这条缺陷的形态是「数字进得来、结果恒 0」，编译门与 clippy 都查不出，
    /// 只有拿**真整数**跑一次生产注入路径才会红。
    #[test]
    fn num_of_bridges_nested_integers_and_keeps_absence_explicit() {
        let mut engine = Engine::new();
        register_common_functions(&mut engine);

        // ① 嵌层整数（json_parse ⇒ i64）必须被读成数，而不是判成缺席。
        let days: f64 = engine
            .eval::<f64>(r#"let m = json_parse(`{"days": 28}`); num_of(m["days"])"#)
            .expect("嵌层整数应被 num_of 桥成 f64");
        assert_eq!(days, 28.0);

        // ② 缺席/非数值 → unit：不许用哨兵值把「拿不到」伪装成一个读数。
        let absent_unit: bool = engine
            .eval::<bool>(r#"let m = json_parse(`{"x": "22.7"}`); type_of(num_of(m["x"])) == "()""#)
            .expect("非数值应可读回类型");
        assert!(absent_unit, "字符串不得被 num_of 当数值（上游契约是 JSON Number）");

        // ③ 带默认值形态（原 `risk-level.rhai` / `trader-proxy.rhai` 的两参语义）。
        let defaulted: f64 = engine
            .eval::<f64>(r#"let m = json_parse(`{"x": "22.7"}`); num_of(m["x"], -1.0)"#)
            .expect("非数值应落默认值");
        assert_eq!(defaulted, -1.0);

        // ④ 桥后与 f64 形参函数（clamp）无型别分歧 —— 这是「整数注入会不会打断下游调用」的实证。
        let clamped: f64 = engine
            .eval::<f64>(r#"let m = json_parse(`{"p": 7}`); clamp(num_of(m["p"]), 0.0, 5.0)"#)
            .expect("num_of 的输出应可直接进 clamp");
        assert_eq!(clamped, 5.0);

        // ⑤ **探针**：不经桥、把嵌层整数直接喂给 f64 形参的 `clamp`。
        //   这条事实决定「桥是不是必需」——v125 主链把分支行的 `positionPct` 直接喂 `clamp`，
        //   若 Rhai 不自动升格，那条路一旦遇到整数分支就是运行期 `Function not found`（整脚本降级），
        //   而编译门看不出来。读数由本条测试给，不靠推断。
        let raw_int_to_clamp =
            engine.eval::<f64>(r#"let m = json_parse(`{"p": 7}`); clamp(m["p"], 0.0, 5.0)"#);
        assert!(
            raw_int_to_clamp.is_err(),
            "Rhai 竟把 i64 自动升格进了 f64 形参（返回 {raw_int_to_clamp:?}）⇒ \
             桥的必要性论证要改写（不代表可以不解这条缺陷：type_of==\"f64\" 仍恒假）"
        );
    }

    #[test]
    fn json_value_to_dynamic_preserves_integer() {
        let v = serde_json::json!({"count": 42, "price": 3.15});
        let d = json_value_to_dynamic(&v);
        let map = d.try_cast::<rhai::Map>().unwrap();
        // 整数保持 i64，不被静默转 f64
        let count = map.get("count").unwrap();
        assert_eq!(count.as_int().unwrap(), 42);
        // 浮点数保持 f64
        let price = map.get("price").unwrap();
        assert!((price.as_float().unwrap() - 3.15).abs() < 1e-9);
    }

    #[test]
    fn dynamic_to_json_value_roundtrip() {
        let original = serde_json::json!({
            "name": "test",
            "count": 42,
            "active": true,
            "items": [1, 2, 3],
            "nested": {"key": "value"}
        });
        let dynamic = json_value_to_dynamic(&original);
        let back = dynamic_to_json_value(&dynamic);
        assert_eq!(back, original);
    }
}
