//! 行为测试（A3 补门）：portfolio-mgr.rhai **V47 空头否决**的「输入可信」前置。
//!
//! 背景（2026-09-19 审计 A3 → v78 修复）：`trader_bearish` 由检查 1.5（源码 L1242-1254）
//! 判定，而**随后**的检查 2（L1256-1263）才把「|偏离近价| > 70%」判为数据异常并置
//! `trader_data_valid = false`，但 `trader_bearish` **不回滚**。修复在 V47 否决块（L1852）
//! 联立 `trader_data_valid`：**异常输入不得行使否决权** —— 否则一个被系统自己判为异常
//! 的目标价仍能把决策强制翻成「卖出」（最强动作），与「异常数据不可信」自相矛盾。
//!
//! 本测试抽取检查 1.5 + 检查 2 + V47 否决块的最小复现（与
//! `verifier_report_unwrap.rs` 的「抽取关心片段」模式一致），喂正负对照输入断言：
//! - **正控（真否决生效）**：偏离 -50%（<70%，数据可信）→ R-201，`持有→卖出`
//! - **负控（异常输入被抑制）**：偏离 -80%（>70%，数据异常）→ R-201-SUPPRESSED，action 不变
//!
//! ⚠ 若 portfolio-mgr.rhai 的这三段逻辑漂移，本最小复现需同步（与 verifier_report_unwrap.rs
//! 同一纪律）；漂移时以 `include_str!` 处的源码为准，非以本文件为准。
use rhai::Engine;

/// 抽取检查1.5 + 检查2 + V47 否决块的最小复现脚本。
const SCRIPT: &str = r#"
// ── 检查 1.5（A3 语义：先判方向，判据对齐 portfolio-mgr.rhai L1242-1254）──
let trader_bearish = false;
let bearish_magnitude = 0.0;
if trader_data_valid && current_price > 0.0 {
    let upside = (trader_target_price - current_price) / current_price;
    if upside < -0.15 {
        trader_bearish = true;
        bearish_magnitude = upside;
    }
}
// ── 检查 2（L1256-1263）：偏离 >70% 判数据异常 → 置 trader_data_valid = false ──
if trader_data_valid && current_price > 0.0 {
    let deviation = (trader_target_price - current_price) / current_price;
    if deviation.abs() > 0.70 {
        trader_data_valid = false;
    }
}
// ── V47 否决块（L1852-1872）：联立 trader_data_valid（v78/A3 修复）──
let trail = [];
if trader_bearish && trader_data_valid {
    if final_action == "买入" || final_action == "增持" || final_action == "持有" {
        final_action = "卖出";
        trail.push("R-201");
    }
} else if trader_bearish && !trader_data_valid {
    trail.push("R-201-SUPPRESSED");
}
#{
    "action": final_action,
    "trail": trail,
}
"#;

struct Outcome {
    action: String,
    ruled_201: bool,
    suppressed: bool,
}

fn run(current_price: f64, target_price: f64, init_action: &str) -> Outcome {
    let engine = Engine::new();
    let mut scope = rhai::Scope::new();
    scope.push("trader_data_valid", true);
    scope.push("current_price", current_price);
    scope.push("trader_target_price", target_price);
    scope.push("final_action", init_action.to_string());
    let result: rhai::Map = engine.eval_with_scope(&mut scope, SCRIPT).expect("脚本执行失败");

    let action = result
        .get("action")
        .and_then(|d| d.clone().try_cast::<String>())
        .unwrap_or_else(|| panic!("action 应为字符串"));
    let trail: Vec<String> = match result.get("trail") {
        Some(d) => match d.clone().try_cast::<rhai::Array>() {
            Some(arr) => arr.iter().map(|d| d.clone().try_cast::<String>().unwrap()).collect(),
            None => panic!("trail 应为数组"),
        },
        None => panic!("trail 缺失"),
    };
    Outcome {
        action,
        ruled_201: trail.iter().any(|t| t == "R-201"),
        suppressed: trail.iter().any(|t| t == "R-201-SUPPRESSED"),
    }
}

/// 场景 B：`current=10, target=5`。upside=-50%（< -15% → 看空），
/// deviation=-50%（|−0.5|<0.7 → 数据**可信**）⇒ 否决**生效**：R-201，持有→卖出。
#[test]
fn veto_fires_on_trusted_bearish() {
    let o = run(10.0, 5.0, "持有");
    assert!(o.ruled_201, "可信看空应产生 R-201 否决");
    assert!(!o.suppressed, "可信看空不应走 R-201-SUPPRESSED");
    assert_eq!(o.action, "卖出", "否决生效应把持有强翻为卖出");
}

/// 场景 A（A3 核心）：`current=10, target=2` —— LLM 幻觉出**远低于现价**的目标价。
/// upside=-80%（看空）+ deviation=-80%（>70% → 数据**异常**）⇒ 修复联立后
/// 否决被**抑制**：R-201-SUPPRESSED，action **不得**被强翻成卖出。
#[test]
fn veto_suppressed_when_input_untrusted_hallucination() {
    let o = run(10.0, 2.0, "持有");
    assert!(o.suppressed, "异常输入应走 R-201-SUPPRESSED");
    assert!(!o.ruled_201, "异常输入不得行使 R-201 否决");
    assert_eq!(o.action, "持有", "异常输入不得把决策强翻成卖出");
}
