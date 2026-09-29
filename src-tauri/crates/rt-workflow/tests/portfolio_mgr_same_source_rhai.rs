// SPDX-License-Identifier: AGPL-3.0-only

//! Phase F 门禁（`PLAN-multi-horizon-decision-science.md` §四 Phase F）：
//! **两档 posterior 恒等 ⇒ 必须同时携带同源标注**。
//!
//! 被锁的缺陷不是「四档同向」（那可以是真的收敛），而是**读者无法分辨收敛与复制**：
//! 长线复用月度线、超短复用日线时，两档后验会一模一样，面板上看起来像「四个独立结论
//! 恰好一致」。本门把这句话变成机器不变量 —— 同 posterior 的档必须互填
//! `sharesPosteriorWith`，缺一个即红。
//!
//! 检法形态与前几轮一致：**逐字副本**（`SAME_SOURCE_SRC`）+ 生产脚本包含性断言。
//! 副本负责「逻辑确实如此」，包含性负责「脚本里没有第二份逻辑」。

use rhai::{Engine, Scope};

/// 生产脚本里的同源环（改这里必须同步 `portfolio-mgr.rhai`，否则包含性断言先红）。
const SAME_SOURCE_SRC: &str = r#"
for a in horizon_tier_keys {
    let peers = [];
    for b in horizon_tier_keys {
        if a != b && decisions_by_horizon[a]["posterior"] == decisions_by_horizon[b]["posterior"] {
            peers.push(b);
        }
    }
    if peers.len() > 0 {
        decisions_by_horizon[a]["sharesPosteriorWith"] = peers;
    }
}
"#;

/// 生产脚本里的逐档结构性缺席声明（posterior 环之前的一段，同为 Phase F 产物）。
const SCORE_GAP_SRC: &str = r#"
let tier_cn = |k| switch k {
    "ultraShort" => "超短线", "short" => "短线", "mid" => "中线", "long" => "长线", _ => k
};
for t in horizon_tier_keys {
    if decisions_by_horizon[t]["scoreSource"] == "daily_fallback" {
        data_gaps.push(`该周期独立评分未产出 ⇒ ${tier_cn.call(t)}档技术腿按日线退化（结构性缺席，不是低分）`);
    }
}
"#;

const KEYS: &str = r#"let horizon_tier_keys = ["ultraShort", "short", "mid", "long"];"#;

/// 构造四档决策表：`posterior` 与 `scoreSource` 按传入逐档给。
fn build_src(posteriors: [f64; 4], sources: [&str; 4]) -> String {
    let mut s = String::new();
    s.push_str(KEYS);
    s.push_str("\nlet data_gaps = [];\nlet decisions_by_horizon = #{\n");
    let names = ["ultraShort", "short", "mid", "long"];
    for (name, (post, src)) in names.iter().zip(posteriors.iter().zip(sources.iter())) {
        s.push_str(&format!(
            "    \"{}\": #{{ \"posterior\": {}, \"scoreSource\": \"{}\", \"sharesPosteriorWith\": [] }},\n",
            *name, *post, *src
        ));
    }
    s.push_str("};\n");
    s.push_str(SCORE_GAP_SRC);
    s.push_str(SAME_SOURCE_SRC);
    // 输出四档各自的标注与缺口条数，供断言。
    s.push_str(
        r#"
#{
    "marks": #{
        "ultraShort": decisions_by_horizon["ultraShort"]["sharesPosteriorWith"],
        "short": decisions_by_horizon["short"]["sharesPosteriorWith"],
        "mid": decisions_by_horizon["mid"]["sharesPosteriorWith"],
        "long": decisions_by_horizon["long"]["sharesPosteriorWith"]
    },
    "gaps": data_gaps
}
"#,
    );
    s
}

fn eval(src: &str) -> rhai::Dynamic {
    let engine = Engine::new();
    let mut scope = Scope::new();
    engine
        .eval_with_scope::<rhai::Dynamic>(&mut scope, src)
        .unwrap_or_else(|e| panic!("Rhai 编译/执行失败: {e}"))
}

fn marks_of(result: &rhai::Dynamic) -> Vec<Vec<String>> {
    let map = result.clone().try_cast::<rhai::Map>().expect("结果应为 map");
    let marks = map
        .iter()
        .find(|(k, _)| k.as_str() == "marks")
        .expect("缺 marks")
        .1
        .clone()
        .try_cast::<rhai::Map>()
        .expect("marks 应为 map");
    let mut out = Vec::new();
    for name in ["ultraShort", "short", "mid", "long"] {
        let v = marks
            .iter()
            .find(|(k, _)| k.as_str() == name)
            .unwrap_or_else(|| panic!("marks 缺档 {name}"))
            .1
            .clone()
            .try_cast::<rhai::Array>()
            .unwrap_or_else(|| panic!("{name} 的标注应为数组"));
        out.push(v.iter().map(|d| d.clone().into_string().unwrap_or_default()).collect());
    }
    out
}

/// 缺口清单（Rhai 侧不注册 `join`，故原样取回数组在 Rust 断言）。
fn gaps_of(result: &rhai::Dynamic) -> Vec<String> {
    let map = result.clone().try_cast::<rhai::Map>().expect("结果应为 map");
    let arr = map
        .iter()
        .find(|(k, _)| k.as_str() == "gaps")
        .expect("缺 gaps")
        .1
        .clone()
        .try_cast::<rhai::Array>()
        .expect("gaps 应为数组");
    arr.iter().map(|d| d.clone().into_string().unwrap_or_default()).collect()
}

/// 正控：两档后验相同 ⇒ 必须互填对方。
#[test]
fn equal_posterior_tiers_mark_each_other() {
    let src = build_src(
        [62.0, 62.0, 55.0, 48.0],
        ["tier_native", "tier_native", "tier_native", "tier_native"],
    );
    let r = eval(&src);
    let marks = marks_of(&r);
    assert_eq!(marks[0], vec!["short".to_string()], "超短应标注与 short 同源");
    assert_eq!(marks[1], vec!["ultraShort".to_string()], "short 应反向标注");
    assert!(marks[2].is_empty(), "独立后验不得被标注");
    assert!(marks[3].is_empty(), "独立后验不得被标注");
    // 全档 tier_native ⇒ 不得凭空写缺口
    assert!(gaps_of(&r).is_empty(), "无退化档却写了 data_gaps: {:?}", gaps_of(&r));
}

/// 三档同分 ⇒ 每档都要点名其余两档（不是只标一对）。
#[test]
fn three_way_tie_marks_every_peer() {
    let src = build_src(
        [60.0, 60.0, 60.0, 41.0],
        ["daily_fallback", "daily_fallback", "daily_fallback", "tier_native"],
    );
    let r = eval(&src);
    let marks = marks_of(&r);
    assert_eq!(marks[0].len(), 2);
    assert_eq!(marks[1].len(), 2);
    assert_eq!(marks[2].len(), 2);
    assert!(marks[3].is_empty());
    // 逐档缺席声明：三个 daily_fallback ⇒ 三条缺口，且逐条点名档位
    let gaps = gaps_of(&r);
    assert_eq!(gaps.len(), 3, "退化档数与缺口条数不符: {gaps:?}");
    let joined = gaps.join("|");
    // 中文档名（文案给人读；camelCase 键只留给机器字段）。
    // ⚠ 判「短线」必须带前缀 `⇒ `：`超短线` 里含 `短线`，裸 contains 会把超短也算成短线。
    for name in ["⇒ 超短线档", "⇒ 短线档", "⇒ 中线档"] {
        assert!(joined.contains(name), "缺口未点名「{name}」: {joined}");
    }
    assert!(!joined.contains("⇒ 长线档"), "未退化的长线不得进缺口: {joined}");
}

/// 反控（本门的存在理由）：同分却无标注 ⇒ 必须被检出。
///
/// 这里跑的是**删掉同源环**的同一份数据 —— 修复前生产脚本就是这个形态（四档各自出数、
/// 无人互指），新检法必须在那个真实形态上命中，否则它只是个恒真断言。
#[test]
fn pre_fix_shape_loses_the_marks() {
    let mut src = String::new();
    src.push_str(KEYS);
    src.push_str("\nlet data_gaps = [];\nlet decisions_by_horizon = #{\n");
    src.push_str("    \"ultraShort\": #{ \"posterior\": 62.0, \"scoreSource\": \"tier_native\", \"sharesPosteriorWith\": [] },\n");
    src.push_str("    \"short\": #{ \"posterior\": 62.0, \"scoreSource\": \"tier_native\", \"sharesPosteriorWith\": [] },\n");
    src.push_str("    \"mid\": #{ \"posterior\": 55.0, \"scoreSource\": \"tier_native\", \"sharesPosteriorWith\": [] },\n");
    src.push_str("    \"long\": #{ \"posterior\": 48.0, \"scoreSource\": \"tier_native\", \"sharesPosteriorWith\": [] },\n");
    src.push_str("};\n#{ \"marks\": #{ \"ultraShort\": decisions_by_horizon[\"ultraShort\"][\"sharesPosteriorWith\"] } }\n");
    let r = eval(&src);
    let map = r.clone().try_cast::<rhai::Map>().unwrap();
    let marks = map
        .iter()
        .find(|(k, _)| k.as_str() == "marks")
        .unwrap()
        .1
        .clone()
        .try_cast::<rhai::Map>()
        .unwrap();
    let ultra = marks
        .iter()
        .find(|(k, _)| k.as_str() == "ultraShort")
        .unwrap()
        .1
        .clone()
        .try_cast::<rhai::Array>()
        .unwrap();
    assert!(ultra.is_empty(), "无同源环时标注必为空 —— 这正是修复前的形态");
}

/// 防漂移：两段副本必须逐字在生产脚本里，且档位键表也在。
#[test]
fn production_script_contains_the_blocks_verbatim() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(pm.contains(SAME_SOURCE_SRC), "portfolio-mgr.rhai 的同源环与副本已漂移，请同步");
    assert!(pm.contains(SCORE_GAP_SRC), "portfolio-mgr.rhai 的逐档缺席声明与副本已漂移，请同步");
    assert!(pm.contains(KEYS), "档位键表缺失或改名未同步");
    // 反向锁：posterior 恒等判定不得改用融合前的 hpost（那会与面板显示口径错位）
    assert!(
        pm.contains("decisions_by_horizon[a][\"posterior\"] =="),
        "同源判定必须比较**输出值** posterior"
    );
    // 每档必须真的产出 scoreSource 字段（否则声明环恒不命中）
    assert!(
        pm.contains(
            "\"scoreSource\": if present(ts) { \"tier_native\" } else { \"daily_fallback\" }"
        ),
        "逐档 scoreSource 缺席 ⇒ 结构性无处可声明"
    );
    // 计数与清单必须同刻：gap_note 要在逐档缺口 push **之后**求值。
    // 原位置（推理段之前）会让文案写「缺口(3)」而 data_gaps 实有 5 条 ⇒ 顺序本身是契约。
    let gap_at = pm.find("let gap_note = if data_gaps.len() > 0").expect("gap_note 应存在");
    // 互斥声明锁：超短档「取 60 分钟粒度评分」不得**无条件**写进 verdict。
    // 闭包在评分缺失时已往同一串里追加「该周期评分缺失，按日线退化」⇒ 无条件那句
    // 会让一条结论同时声称「吃了 60 分钟」和「退化成日线」，把退化伪装成正常来源。
    assert!(
        !pm.contains("+ \"[超短取 60 分钟粒度评分，置信下限35%]\""),
        "verdict 里又出现无条件的 60 分钟来源声明（与缺失注记互斥，必须按 scoreSource 分支）"
    );
    assert!(
        pm.contains("if ultra_short_dec[\"scoreSource\"] == \"tier_native\""),
        "超短档的来源声明必须按 scoreSource 分支"
    );

    let push_at = pm
        .find("\u{8be5}\u{5468}\u{671f}\u{72ec}\u{7acb}\u{8bc4}\u{5206}\u{672a}\u{4ea7}\u{51fa}")
        .expect("逐档退化声明应存在");
    assert!(
        push_at < gap_at,
        "gap_note 必须在逐档缺口 push 之后求值：当前 push={push_at} note={gap_at}"
    );
}
