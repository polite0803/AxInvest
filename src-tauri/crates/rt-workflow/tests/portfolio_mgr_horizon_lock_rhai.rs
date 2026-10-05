//! 行为测试（阶段2 换代版，PLAN §四十八 Q1=C）：**主档由四档分支结论选出**。
//!
//! 本文件原本锁的是「主档 = effective_posterior 阈值映射」那条确定性通路（〇-B v2）。
//! 2026-10-04 起那条阈值映射**退居兜底**：四路分支有产出时，定档与主 action 都取自所选档；
//! 四路全缺席时才落回阈值映射，且来源必须标成 `formula_no_branch` 而不是静默冒充正常路径。
//! 因此判据从「阈值表对不对」翻成三件事：
//!   ① 兜底路径仍然确定性、且不受 LLM 自报值影响（原判据保留，换了前提）；
//!   ② 选档判据：可执行优先 → 置信最大 → 全不可执行仍给方向最强档；
//!   ③ 乘数与天数必须跟着**新档**走（否则「档是分支选的、仓位价位是旧档算的」= 第二个同屏矛盾）。
//!
//! 为什么仍然要锁：主档是落库列 `stock_analyses.decision_time_horizon` /
//! `decision_horizon_source` / `decision_expected_holding_days` 的来源，也是**单周期反思**
//! 默认复盘的那一档。只要还留一条「采信模型自报」的旁路，同一份证据就能因模型措辞不同
//! 落到不同档、反思按天判成熟随之错档 —— 换代不变的是这条，变的只是「谁有权定档」。
//!
//! ⚠ 本文件抽取脚本片段的**逐字副本**（同 `portfolio_mgr_veto_rhai.rs` 纪律），
//!   并在测试中断言源文件仍包含该副本 ⇒ 脚本漂移时本测试会红，而不是默默失效。
use rhai::{Engine, Scope};

/// 与 `portfolio-mgr.rhai` **选档段**逐字一致（v129 起该段整体位于风险分类**之前**，
///   顺序即判据 —— 见 `primary_tier_block_matches_source_verbatim` 的反向锁⑤）。
const TIER_PICK_SRC: &str = r##"	// ── 主档改由四档分支结论选出（Q1=C，PLAN §四十八 阶段2；2026-10-04 拍板）──────
	// 语义换代：主档不再是「主链后验阈值落的那一档」，而是「本轮四档里最该看的那一档」——
	//   定档与主 action 都取自该档分支，主链日线口径退为归因/证据列（posterior / evidence
	//   那几项照旧输出）。退役的是「effective_posterior 阈值定档」这一条（〇-B v2 立的）。
	// 判据（写死在此，并被 `rhai_registry` 整脚本门锁住）：
	//   ① 先筛**可执行档**（`positionPct > 0`），在可执行档里取 `confidence` 最大；
	//   ② 四档全部不可执行 ⇒ 仍取 `confidence` 最大档 —— 方向成立但无可执行计划由呈现层
	//      成句声明（§四十八 Q2-B），不在这里把方向悄悄降级成观望；
	//   ③ 一路都没产出 ⇒ 退回旧的阈值定档，来源标成 `formula_no_branch`（缺席本身由下方
//      装配段逐档写进 data_gaps，这里不重复报，也不在此处引用尚未声明的 data_gaps）。
	//      留这条兜底是因为**未重播种的存量库**四路必然全缺席；若直接判「数据缺失」，
	//      全部历史记录会一次性失效 —— 兜底可以，但必须显式声明自己是兜底。
	// ⚠ 「这一行是不是分支的有效输出」只此一份判定（`branch_row_ok`），下方装配段复用
	//   同一闭包 —— 两处各写一遍就会漂移成「选档认为有效、装配认为无效」的同屏矛盾。
	//   自证字段两条：`confidenceMethod`（这条结论用哪种置信算法算的）与 `stopSource`
	//   （这个止损是哪种口径给的）。后者自 2026-10-04 起是**必需**的 —— 主档出场数值改为
	//   直接取所选档行之后，「主档止损的来历」必须由那一行自己说，主链再猜就是冒充。
	let branch_row_ok = |r| type_of(r) == "map" && !r.is_empty() && present(r["confidenceMethod"]) && present(r["stopSource"]);
	// 顺序 = 展示顺序；camel 给前端键，snake 与仲裁/反思/权威周期表对齐。
	let branch_tiers = [
	    #{ camel: "ultraShort", snake: "ultra_short", row: h_ultra_short },
	    #{ camel: "short", snake: "short", row: h_short },
	    #{ camel: "mid", snake: "mid", row: h_mid },
	    #{ camel: "long", snake: "long", row: h_long },
	];
	let picked_row = ();
	let picked_tier = "";
	let best_conf = -1.0;
	let exec_row = ();
	let exec_tier = "";
	let best_exec_conf = -1.0;
	for t in branch_tiers {
	    if branch_row_ok.call(t.row) == false { continue; }
	    let c = if present(t.row["confidence"]) { t.row["confidence"] } else { 0.0 };
	    let p = if present(t.row["positionPct"]) { t.row["positionPct"] } else { 0.0 };
	    if c > best_conf { best_conf = c; picked_row = t.row; picked_tier = t.snake; }
	    if p > 0.0 && c > best_exec_conf {
	        best_exec_conf = c; exec_row = t.row; exec_tier = t.snake;
	    }
	}
	// 可执行优先；全不可执行时保留「方向最强但无可执行计划」那一档。
	if exec_tier != "" { picked_row = exec_row; picked_tier = exec_tier; }
	let branch_picked = picked_tier != "";
"##;

/// 与 `portfolio-mgr.rhai` 定档段逐字一致（天数闭包、阈值兜底、乘数与期望持有期）。
const PRIMARY_TIER_SRC: &str = r##"	let days_for = |h| horizon_const.call(h, "days");


	// ⚠ 这里**不做**四档 switch —— 四档各自的决策由下方 decisionsByHorizon 逐档产出，
	//   主档只是「本轮最值得看的那一档」的指针（该指针现在由四档结论选出，见上）。
	let time_horizon = if branch_picked {
	    picked_tier
	} else if effective_posterior >= 0.65 { "long" } else if effective_posterior >= 0.50 { "mid" } else if effective_posterior >= 0.35 { "short" } else { "ultra_short" };
	// 「谁定的档」出口：`branch_pick` = 四档分支选的；`formula_no_branch` = 四路全缺席时
	// 退回后验阈值定档（**不是**静默兜底，值域权威在 harness 的 HorizonSource）。
	let horizon_source = if branch_picked { "branch_pick" } else { "formula_no_branch" };
	// ⚠ v131：按档风险的**取值与收紧**已整体上移到风险分类段之后（`risk_tier` / `depth_upgrade` /
	//   `risk_tier_applied`，见 :542 前那段），因为 f4 与 risk_bias 都排在选档段之前 —— 留在原地就
	//   只能管否决一处。本段现在**只**负责把已按档的 `overall_risk` 交给下方 veto/cap。

	// 周期仓位乘数：与 evidence_weight.rs 的 `compute_recommended_position` 消费**同一个**
	//   `Period::position_multiplier`（超短 0.6 / 短 0.8 / 中 1.0 / 长 1.2）。
	let horizon_position_mult = horizon_const.call(time_horizon, "mult");
	// 主档期望持有天数：取**所选档分支自报**的那个数（它本来就是分支表 `days`，见各脚本输出），
	//   四路全缺席时退回权威表。两条都**不采信 LLM 的自由数字** —— 否则同一决策里
	//   「哪个周期」是公式定的、「该周期多久」却是模型随口报的，反思按天判成熟即错档。
	let picked_days = if branch_picked { num_of(picked_row["expectedHoldingDays"]) } else { () };
	//   两条来源归一到 **i64**：权威表 `days` 是嵌层整数（注入即 i64），分支行可能是 `28` 或 `28.0`
	//   ⇒ 不桥则同一列在两代样本里落两种 JSON 型别，下游按整数读的那条会读空。
	let expected_holding_days = if present(picked_days) { picked_days.to_int() } else { days_for.call(time_horizon) };
"##;

/// 与生产**同段骨架**逐字副本（整行注释已剔）：v129 的按档风险收紧。
/// 只锁代码不锁注释：这里要锁的是「谁与谁比、单向朝哪边、缺席算不算」这三件事。
const RISK_TIER_SRC: &str = r##"let risk_tier = if !branch_picked {
    ()
} else if picked_tier == "ultra_short" {
    overall_risk_ultra_short
} else if picked_tier == "short" {
    overall_risk_short
} else if picked_tier == "mid" {
    overall_risk_mid
} else {
    overall_risk_long
};
let tier_llm_usable = type_of(risk_tier) == "string" && present(risk_tier);
let depth_upgrade = tier_llm_usable
    && present(overall_risk_llm) && type_of(overall_risk_llm) == "string"
    && risk_rank(risk_tier) > risk_rank(overall_risk_llm);
let risk_tier_applied = depth_upgrade && risk_rank(risk_tier) > risk_rank(overall_risk);
let overall_risk_algo = overall_risk;
if risk_tier_applied {
    overall_risk = risk_tier;
}
let risk_llm_eff = if depth_upgrade { risk_tier } else { overall_risk_llm };
"##;
/// 与生产 :50 的 `risk_rank` 逐字副本（本门的判据依赖它，不能自带一份简化版 ——
///   两套 rank 序就会测不出「本档更松」这种情形）。
const RISK_RANK_FN: &str = r##"fn risk_rank(label) {
    switch label {
        "低风险"|"低" => 0,
        "中风险"|"中" => 1,
        "高风险"|"高" => 2,
        "极高风险"|"极高" => 3,
        _ => 1
    }
}
"##;

/// 与生产同一份 `present`（选档段用它判字段是否注入）。`fn` 体读不到注入变量，只操作自身参数。
const PRESENT_FN: &str = r#"fn present(x) { type_of(x) != "()" }"#;

/// 周期常量闭包（逐字副本，见 `portfolio_mgr_days_for_rhai.rs` 的同名副本）。
const HORIZON_CONST_CLOSURE: &str = r#"let horizon_const = |h, field| {
	    if type_of(horizon_consts_json) != "map" || horizon_consts_json.is_empty() {
	        throw "决策中止：缺少工作流变量 horizon_consts_json（周期常量表，权威源 crates/harness/src/holding_period.rs 的 Period::decision_consts_map，由 stock_workflow/hooks.rs 注入）";
	    }
	    let found = false;
	    for tier_key in horizon_consts_json.keys() {
	        if tier_key == h { found = true; }
	    }
	    if !found {
	        throw `决策中止：周期 '${h}' 不在周期常量表中（应为 ultra_short/short/mid/long 之一）`;
	    }
	    let row = horizon_consts_json[h];
	    if type_of(row) != "map" || type_of(row[field]) == "()" {
	        throw `决策中止：周期常量表缺字段 '${field}'（周期 ${h}）`;
	    }
	    row[field]
	};"#;

/// 与生产同段的**代码骨架**逐字副本（注释已剔）：主档仓位 = 分支结论被风险预算封顶。
/// 只锁代码不锁注释：注释会随解释文字改写，而这里要锁的恰恰是「谁进 min、谁被 clamp、来源标成什么」。
const POSITION_SOURCE_SRC: &str = r##"	let main_risk_budget_pct = risk_budget_position.call(time_horizon);
	let branch_position_pct = if branch_picked { num_of(picked_row["positionPct"]) } else { () };
	let position_pct = if type_of(branch_position_pct) != "()" {
		if type_of(main_risk_budget_pct) == "()" {
			clamp(branch_position_pct, 0.0, 95.0)
		} else {
			clamp(min(branch_position_pct, main_risk_budget_pct), 0.0, 95.0)
		}
	} else if type_of(main_risk_budget_pct) == "()" {
		clamp(position_pct_raw * horizon_position_mult, 0.0, 95.0)
	} else {
		clamp(min(position_pct_raw, main_risk_budget_pct), 0.0, 95.0)
	};
	let position_source = if type_of(branch_position_pct) != "()" {
		if type_of(main_risk_budget_pct) == "()" { "branch_position" } else { "branch_position_capped" }
	} else if type_of(main_risk_budget_pct) == "()" { "fallback_kelly_x_mult" } else { "risk_budget" };"##;

/// 输出：定档 + 来源 + 该档乘数/天数 + 覆盖后的 action。
const OUT: &str = r#"
#{ "h": time_horizon, "s": horizon_source, "mult": horizon_position_mult, "days": expected_holding_days, "act": base_action }
"#;

/// 主 action 取自所选档（逐字副本）—— 它与 `pm_risk_veto` 的**先后**是本文件的一条锁。
const ACTION_OVERRIDE_SRC: &str = r#"	if branch_picked {
		base_action = picked_row["action"];
	}
"#;

/// 剥掉**整行注释**后的脚本文本：骨架逐字锁的对象。
/// 注释会随解释文字改写，而这里要锁的恰恰是「谁进 min、谁被 clamp、来源标成什么」，
/// 所以判据两侧都必须先做同一层剥离 —— 只剥一边就是拿原文去比注释版，必然假红。
fn without_comment_lines(src: &str) -> String {
    src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join(
        "
",
    )
}

/// 自证：剥离器自己不能是恒真的（注释一行都没被去掉 ⇒ 它在测的不是这件事）。
#[test]
fn comment_stripper_actually_strips() {
    // 探针用**行数组**拼，不用转义字面量：上一版经生成脚本写入时，字符串里的 `\n` 被折行吞掉，
    // 期望值与真实输入差了一个空行 ⇒ 自证反过来成了假红源（2026-10-04 实测）。
    let probe = ["// 顶格注释", "let a = 1;", "", "\t// 缩进注释", "let b = 2;"].join("\n");
    assert_eq!(without_comment_lines(&probe), "let a = 1;\n\nlet b = 2;");
}

fn consts_map() -> rhai::Map {
    let mut m = rhai::Map::new();
    for (k, d, mult) in
        [("ultra_short", 2_i64, 0.6_f64), ("short", 5, 0.8), ("mid", 28, 1.0), ("long", 90, 1.2)]
    {
        let mut row = rhai::Map::new();
        row.insert("days".into(), rhai::Dynamic::from(d));
        row.insert("mult".into(), rhai::Dynamic::from(mult));
        m.insert(k.into(), rhai::Dynamic::from(row));
    }
    m
}

/// 一档分支输出：`(置信度, 仓位%)`；任一为 `None` = 该路未产出（注入 unit，与生产同形）。
fn row(conf: Option<f64>, pos: Option<f64>, action: &str) -> rhai::Dynamic {
    let (Some(c), Some(p)) = (conf, pos) else { return rhai::Dynamic::UNIT };
    let mut m = rhai::Map::new();
    m.insert("confidence".into(), rhai::Dynamic::from(c));
    m.insert("positionPct".into(), rhai::Dynamic::from(p));
    m.insert("action".into(), rhai::Dynamic::from(action.to_string()));
    // 自证字段：缺它就不是分支输出（`branch_row_ok` 会拒），故正常夹具一律带上两条 ——
    // `confidenceMethod`（置信怎么算的）与 `stopSource`（止损哪来的），后者自阶段3′ 起
    // 是主档出场标签的唯一来源，缺它这行就不能被当主档用。
    m.insert("confidenceMethod".into(), rhai::Dynamic::from("fixture_method".to_string()));
    m.insert("stopSource".into(), rhai::Dynamic::from("vol_band".to_string()));
    rhai::Dynamic::from(m)
}

/// 四路输入按 ultra_short / short / mid / long 顺序给；返回 `(档, 来源, 乘数, 天数, action)`。
fn pick(effective_posterior: f64, rows: [rhai::Dynamic; 4]) -> (String, String, f64, i64, String) {
    // ⚠ 与生产同一函数集（v126 起本段的分支行读取要调 `num_of`）：`clamp` 是 Rhai 自带、
    //   `num_of` **不是** —— 用裸 `Engine::new()` 会得到运行期 `Function not found`，
    //   而那不是被锁缺陷、是本门自己的夹具缺覆盖面（判据同「运行门 scope 要与生产同源」）。
    let mut engine = Engine::new();
    axagent_harness::register_common_functions(&mut engine);
    // 顺序与生产一致：先建 const scope，再编译，再 eval_ast_with_scope。
    let mut scope = Scope::new();
    scope.push_constant("effective_posterior", rhai::Dynamic::from(effective_posterior));
    // ⚠ 模型自报值**故意也在 scope 里** —— 它必须影响不到结果（被撤除的旁路）。
    scope.push_constant("trader_time_horizon", rhai::Dynamic::from("short".to_string()));
    scope.push_constant("trader_holding_days", rhai::Dynamic::from(5.0_f64));
    scope.push_constant("horizon_consts_json", rhai::Dynamic::from(consts_map()));
    for (name, r) in ["h_ultra_short", "h_short", "h_mid", "h_long"].iter().zip(rows) {
        scope.push_constant(*name, r);
    }
    // `base_action` 给一套「后验落在增持区」的正常产物，用于验分支结论是否真的覆盖它。
    let script = format!(
        "{PRESENT_FN}\n{HORIZON_CONST_CLOSURE}\n{TIER_PICK_SRC}{PRIMARY_TIER_SRC}\nlet base_action = \"增持\";\n{ACTION_OVERRIDE_SRC}{OUT}"
    );
    let ast = engine.compile_with_scope(&scope, &script).expect("定档段应可编译");
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).expect("定档段应可求值");
    let g = |k: &str| {
        r.get(k)
            .and_then(|x| x.clone().try_cast::<String>())
            .unwrap_or_else(|| panic!("输出缺字段 {k}"))
    };
    let mult = r.get("mult").and_then(|x| x.clone().try_cast::<f64>()).expect("mult 应为浮点");
    let days = r.get("days").and_then(|x| x.clone().try_cast::<i64>()).expect("days 应为整数");
    (g("h"), g("s"), mult, days, g("act"))
}

/// 四路全缺席 ⇒ 兜底定档（旧阈值表，前提已换成「无分支可用」）。
fn fallback(effective_posterior: f64) -> (String, String, f64, i64, String) {
    pick(effective_posterior, [rhai::Dynamic::UNIT; 4])
}

/// 防漂移：脚本里的定档段与本文件副本必须逐字相同。
#[test]
fn primary_tier_block_matches_source_verbatim() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains(PRIMARY_TIER_SRC),
        "portfolio-mgr.rhai 的主周期定档段与本测试副本已漂移，请同步"
    );
    assert!(pm.contains(TIER_PICK_SRC), "选档段与副本已漂移（v129 起它是独立一段）");
    // v129（#45）：按档收紧这段是**主链风险口径的唯一出处**，锁骨架（剥整行注释）而非锁文案。
    assert!(
        without_comment_lines(pm).contains(RISK_TIER_SRC),
        "按档风险收紧的骨架已漂移（比谁、单向朝哪、缺席算不算 —— 三件事任一改动都要显式同步本副本）"
    );
    // 反向锁⑤（v129 的**顺序即判据**）：选档段必须早于风险分类段 —— 上移就是本项的全部内容，
    // 回到下边就又变成 v128「只有否决按档」。
    let pick_at = pm.find(TIER_PICK_SRC).expect("选档段应存在");
    let risk_at = pm.find("let overall_risk = \"中风险\"").expect("风险分类段应存在");
    assert!(
        pick_at < risk_at,
        "选档段必须在风险分类之前（否则 f4/risk_bias 又读不到按档值）：选档={pick_at} 风险={risk_at}"
    );
    // 负控：把选档段搬到风险分类**之后**，同一套 find 判据必须判出「选档晚于风险」，
    // 否则上面那条顺序锁是恒真的（它读的是字符偏移，不改文本就永远成立）。
    let moved_pick = pm.replace(TIER_PICK_SRC, "").replace(
        "let risk_rank_val = risk_rank(overall_risk);",
        &format!("let risk_rank_val = risk_rank(overall_risk);\n{TIER_PICK_SRC}"),
    );
    let p2 = moved_pick.find(TIER_PICK_SRC).expect("变异后选档段应存在");
    let r2 = moved_pick.find("let overall_risk = \"中风险\"").expect("变异后风险分类段应存在");
    assert!(p2 > r2, "负控失效：选档段搬到风险分类之后，顺序判据却抓不到 ⇒ 它没在测这件事");
    assert!(pm.contains(ACTION_OVERRIDE_SRC), "主 action 取自所选档那段与副本已漂移");
    // 反向锁①：v1 的两条旁路不得回来
    assert!(
        !pm.contains("user_time_horizon") && !pm.contains("horizon_user_locked"),
        "v1 的用户锁档通路应已撤除，脚本里不得再出现 user_time_horizon / horizon_user_locked"
    );
    assert!(
        !pm.contains("} else if horizon_model_reported {"),
        "v1 的「采信 trader 自报定档」分支应已撤除"
    );
    // 反向锁②：定档必须在凯利仓位之前（乘数否则进不了主决策，Phase 3 就还是旁路）
    let tier_at = pm.find("let time_horizon = if branch_picked").expect("定档段应存在");
    let kelly_at = pm.find("let base_position_pct = pm_kelly_position").expect("凯利段应存在");
    assert!(
        tier_at < kelly_at,
        "定档段必须在凯利仓位之前：周期乘数当前在 char {tier_at}，仓位在 {kelly_at}"
    );
    // 反向锁③（阶段2 新增，**顺序即判据**）：分支结论覆盖 base_action 必须早于
    // `pm_risk_veto` ⇒ 风控只可能把分支结论往保守方向改，不得反向。
    let override_at = pm.find("base_action = picked_row[\"action\"];").expect("覆盖段应存在");
    let veto_at = pm.find("final_action = pm_risk_veto").expect("风控否决段应存在");
    assert!(
        override_at < veto_at,
        "主 action 的分支覆盖必须早于 pm_risk_veto（否则分支结论会盖掉风控）：覆盖={override_at} 否决={veto_at}"
    );
    // 负控（证明上面那条顺序判据不是恒真）：把覆盖段搬到 veto **之后**，
    // 同一套 find 判据必须得到「覆盖晚于否决」的结论。
    let moved = pm.replace(ACTION_OVERRIDE_SRC, "").replace(
        "final_action = pm_risk_veto(final_action, overall_risk);",
        &format!("final_action = pm_risk_veto(final_action, overall_risk);\n{ACTION_OVERRIDE_SRC}"),
    );
    let o2 = moved.find("base_action = picked_row[\"action\"];").expect("变异后覆盖段应存在");
    let v2 = moved.find("final_action = pm_risk_veto").expect("变异后否决段应存在");
    assert!(o2 > v2, "负控失效：覆盖段搬到 veto 之后，顺序判据却抓不到 ⇒ 它没在测这件事");
    // 反向锁④（v125 批准 ① 后翻向）：主档仓位 = **所选档分支的结论**被主链风险预算封顶；
    //   四路全缺席时才回到「凯利 ∧ 风险预算」或「凯利 × 经验乘数」那两条旧分支。
    //   同时锁 `position_source` 的**四值** —— 「谁定的数」必须在输出里可反解，
    //   少一个值就等于把降级重新伪装成主口径（Phase D-2 立这条的理由一字没变，只是值多了两个）。
    // ⚠ 锁的是**剥掉整行注释之后**的骨架：副本本身是按「去掉注释」构造的，
    //   拿原文去 contains 必然假红（首轮实测就是 False）。判据要锁的是代码形态，
    //   注释本来就该允许改写。
    assert!(
        without_comment_lines(pm).contains(POSITION_SOURCE_SRC),
        "主档仓位的来源与上限关系已漂移（阶段3′ 批准 ①：分支结论 ∧ 风险预算取小；四路全缺席才走主链凯利）"
    );
    assert!(
        pm.contains("\"branch_position\"")
            && pm.contains("\"branch_position_capped\"")
            && pm.contains("\"risk_budget\"")
            && pm.contains("\"fallback_kelly_x_mult\""),
        "仓位来源标注必须四值齐备 —— 少一个值就等于把「谁被用上」重新变回不可反解"
    );
    // R-11 换心脏的两条反向锁（禁词带调用标点 ⇒ 免疫退役说明注释）：
    // 主链不得再逐档算赔率/√h 折算；装配段不得失踪（缺席档见 rhai_registry 整脚本门 ④）。
    assert!(
        !pm.contains("let hodds =") && !pm.contains("pm_snr_confidence("),
        "主链又出现逐档赔率/√h 折算 ⇒ R-11 退役的形态回归（逐档口径应在 portfolio-mgr-h-*.rhai 里）"
    );
    assert!(
        pm.contains("let decisions_by_horizon = #{};"),
        "装配段失踪 ⇒ 四档可能又由主链代算（生产 scope 同源的整脚本门见 rhai_registry.rs）"
    );
}

/// 兜底路径：四路全缺席时阈值映射必须仍然确定性，且来源必须自报兜底身份。
/// （原 `four_tiers_map_deterministically`，换了前提：它现在是**兜底**通路。）
#[test]
fn fallback_ladder_is_deterministic_and_declares_itself() {
    for (p, want_h, want_mult, want_days) in [
        (0.70, "long", 1.2_f64, 90_i64),
        (0.65, "long", 1.2, 90),
        (0.55, "mid", 1.0, 28),
        (0.50, "mid", 1.0, 28),
        (0.40, "short", 0.8, 5),
        (0.35, "short", 0.8, 5),
        (0.20, "ultra_short", 0.6, 2),
    ] {
        let (h, s, mult, days, _) = fallback(p);
        assert_eq!(
            (h.as_str(), mult, days),
            (want_h, want_mult, want_days),
            "无分支时 effective_posterior={p} 的兜底定档/乘数/天数错档"
        );
        assert_eq!(s, "formula_no_branch", "兜底路径必须自报 formula_no_branch，实得 {s}");
    }
}

/// 选档判据①：**可执行优先**。超短 87.8 分但仓位 0、短线 77.8 分且仓位 5 ⇒ 选短线。
/// 同时锁「乘数与天数跟着新档」—— 阶段3「用新档重算」在定档段的体现。
#[test]
fn pick_prefers_executable_tier_and_moves_days_and_mult_with_it() {
    let (h, s, mult, days, act) = pick(
        0.20,
        [
            row(Some(87.8), Some(0.0), "买入"),
            row(Some(77.8), Some(5.0), "持有"),
            row(Some(56.8), Some(0.0), "持有"),
            row(Some(33.1), Some(0.0), "减持"),
        ],
    );
    assert_eq!((h.as_str(), s.as_str()), ("short", "branch_pick"), "可执行档优先没生效：{h}/{s}");
    assert_eq!((mult, days), (0.8, 5), "乘数/天数没跟着新档重算（还是旧档的值 ⇒ 同屏两个口径）");
    assert_eq!(act, "持有", "主 action 必须取所选档的结论");
}

/// 选档判据②：四档全部不可执行 ⇒ 仍取置信最大那一档。方向不得被悄悄降级成观望
/// ——「无可执行计划」由呈现层成句声明（PLAN §四十八 Q2-B）。
#[test]
fn all_inexecutable_still_picks_strongest_direction() {
    let (h, s, mult, days, act) = pick(
        0.20,
        [
            row(Some(87.8), Some(0.0), "买入"),
            row(Some(77.8), Some(0.0), "持有"),
            row(Some(56.8), Some(0.0), "持有"),
            row(Some(33.1), Some(0.0), "减持"),
        ],
    );
    assert_eq!(
        (h.as_str(), s.as_str()),
        ("ultra_short", "branch_pick"),
        "全不可执行时应取置信最大档"
    );
    assert_eq!((mult, days), (0.6, 2), "乘数/天数应跟到 ultra_short");
    assert_eq!(act, "买入", "方向不得被悄悄降级成观望");
}

/// 选档判据③：缺自证字段的一路**不得**被选中（与装配段共用 `branch_row_ok`）。
/// 这是「两处判定漂移成不同结论」的复发方向：装配段按缺席处理、选档段却当它有效 ⇒
/// 主档会来自一行根本不会出现在面板里的输出。
#[test]
fn row_without_self_proof_is_not_picked() {
    let mut bogus = rhai::Map::new();
    bogus.insert("confidence".into(), rhai::Dynamic::from(99.0_f64));
    bogus.insert("positionPct".into(), rhai::Dynamic::from(50.0_f64));
    bogus.insert("action".into(), rhai::Dynamic::from("买入".to_string()));
    let (h, s, _, _, _) = pick(
        0.20,
        [
            rhai::Dynamic::from(bogus),
            row(Some(70.0), Some(5.0), "持有"),
            row(Some(40.0), Some(0.0), "观望"),
            row(Some(30.0), Some(0.0), "减持"),
        ],
    );
    assert_eq!(
        (h.as_str(), s.as_str()),
        ("short", "branch_pick"),
        "无自证字段的行被选中了：{h}/{s}"
    );
}

/// 模型自报值在 scope 里也**不得**改变结果（v2「LLM 不参与定档」的正控，两条通路各验一遍）。
#[test]
fn llm_reported_horizon_cannot_move_the_tier() {
    // 兜底路径：0.70 应落 long / 90 天，若被 LLM 值污染则会变成 short / 5 天。
    let (h, _, mult, days, _) = fallback(0.70);
    assert_eq!((h.as_str(), mult, days), ("long", 1.2, 90), "模型自报值泄漏进了兜底定档");
    // 分支路径：选档只看分支行，后验与 LLM 值都不参与（0.70 的兜底档是 long，这里必须落 short）。
    // ⚠ `pick` 的返回序是 (档, 来源, 乘数, 天数, action) —— 第三个是乘数不是天数，
    //   首版按名字顺序取参数位，把乘数当成天数断言（编译期即 E0308 报出）。
    let (bh, _, bmult, bdays, _) = pick(
        0.70,
        [
            row(Some(60.0), Some(3.0), "持有"),
            row(Some(90.0), Some(8.0), "买入"),
            row(Some(50.0), Some(2.0), "持有"),
            row(Some(45.0), Some(2.0), "观望"),
        ],
    );
    assert_eq!(
        (bh.as_str(), bmult, bdays),
        ("short", 0.8, 5_i64),
        "分支在场时后验/LLM 值不该改变定档"
    );
}

/// 同一后验两次求值必须同档（确定性；防止重新掺入 LLM 值或随机项）。
#[test]
fn same_posterior_same_tier() {
    assert_eq!(fallback(0.52).0, fallback(0.52).0);
    let rows = [
        row(Some(60.0), Some(3.0), "持有"),
        row(Some(61.0), Some(3.0), "持有"),
        row(Some(62.0), Some(3.0), "持有"),
        row(Some(63.0), Some(3.0), "持有"),
    ];
    assert_eq!(pick(0.52, rows.clone()).0, pick(0.52, rows).0);
}

/// 阶段3′：主档**期望持有天数**取自所选档分支的自报值，行里没这个键时才退回周期常量表。
/// 两条通路都要有测试 —— 只留一条的话，「谁给的天数」又变成不可反解，而反思按天判成熟
/// 认的就是这个数（错档 = 错判成熟，比错档位更难发现）。
#[test]
fn days_come_from_the_branch_row_and_fall_back_to_the_authority_table() {
    let mut with_days = rhai::Map::new();
    with_days.insert("confidence".into(), rhai::Dynamic::from(70.0_f64));
    with_days.insert("positionPct".into(), rhai::Dynamic::from(6.0_f64));
    with_days.insert("action".into(), rhai::Dynamic::from("增持".to_string()));
    with_days
        .insert("confidenceMethod".into(), rhai::Dynamic::from("sigma_band_position".to_string()));
    with_days.insert("stopSource".into(), rhai::Dynamic::from("sigma_band".to_string()));
    // 与权威表**故意不同**的数：short 档表里是 5，这里给 7 ⇒ 输出必须是 7（分支自报优先）。
    with_days.insert("expectedHoldingDays".into(), rhai::Dynamic::from(7_i64));
    let (_, _, _, days, _) = pick(
        0.20,
        [
            rhai::Dynamic::UNIT,
            rhai::Dynamic::from(with_days),
            rhai::Dynamic::UNIT,
            rhai::Dynamic::UNIT,
        ],
    );
    assert_eq!(days, 7, "分支自报的持有天数没被采用（仍是权威表的值 ⇒ 出场口径与档位脱钩）");
    // 对照半：同一档、行里不带该键 ⇒ 退回权威表 5（既不是 0，也不是别的档的天数）。
    let (_, _, _, days_fb, _) = pick(
        0.20,
        [
            rhai::Dynamic::UNIT,
            row(Some(70.0), Some(6.0), "增持"),
            rhai::Dynamic::UNIT,
            rhai::Dynamic::UNIT,
        ],
    );
    assert_eq!(days_fb, 5, "行内无天数时必须退回权威表（short = 5 个交易日）");
}

/// v129（#45）：主链风险口径按档收紧，**两条轴都必须只升不降**。
///
/// 与 `pick()` 同理，这里跑的是生产的**逐字骨架**（`RISK_TIER_SRC`），不是重写版：
/// 锁「谁与谁比」的同时，必须真求值 —— 编译门查不出「单向」写反（把 `>` 写成 `>=` 或
/// 把 `depth_upgrade` 换成 `tier_llm_usable` 都能编译）。
fn tighten(
    branch_picked: bool,
    picked_tier: &str,
    tier: rhai::Dynamic,
    llm_global: &str,
    algo: &str,
) -> (String, String, bool) {
    let mut engine = Engine::new();
    axagent_harness::register_common_functions(&mut engine);
    let mut scope = Scope::new();
    scope.push_constant("branch_picked", rhai::Dynamic::from(branch_picked));
    scope.push_constant("picked_tier", rhai::Dynamic::from(picked_tier.to_string()));
    scope.push_constant("overall_risk_llm", rhai::Dynamic::from(llm_global.to_string()));
    // 只有**所选档**那一格有值，其余三格留 unit —— 顺带证明四路 switch 取的是对的那一格。
    let slots = ["ultra_short", "short", "mid", "long"];
    for k in slots {
        let v = if branch_picked && k == picked_tier {
            tier.clone()
        } else {
            rhai::Dynamic::UNIT
        };
        scope.push_constant(format!("overall_risk_{k}"), v);
    }
    let script = format!(
        "{RISK_RANK_FN}
{PRESENT_FN}
let overall_risk = \"{algo}\";
{RISK_TIER_SRC}
let llm_out = if type_of(risk_llm_eff) == \"string\" {{ risk_llm_eff }} else {{ \"<absent>\" }};
         #{{ \"risk\": overall_risk, \"llm\": llm_out, \"applied\": risk_tier_applied }}"
    );
    let ast = engine.compile_with_scope(&scope, &script).expect("按档收紧段应可编译");
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).expect("按档收紧段应可求值");
    let g = |k: &str| {
        r.get(k)
            .and_then(|x| x.clone().try_cast::<String>())
            .unwrap_or_else(|| panic!("输出缺字段 {k}"))
    };
    let applied =
        r.get("applied").and_then(|x| x.clone().try_cast::<bool>()).expect("applied 应为布尔");
    (g("risk"), g("llm"), applied)
}

#[test]
fn v129_risk_tier_tightens_both_axes_one_way_only() {
    let high = rhai::Dynamic::from("高风险".to_string());
    let low = rhai::Dynamic::from("低风险".to_string());
    // A 本档比全局节点更严 ⇒ 风险档与 f4 口径**同时**换上本档
    assert_eq!(
        tighten(true, "mid", high.clone(), "中风险", "中风险"),
        ("高风险".to_string(), "高风险".to_string(), true),
        "本档更严却没收紧 ⇒ 按档形同虚设"
    );
    // B 本档比全局节点更松 ⇒ **两条轴都不换**（f4 不得因按档拿到更漂亮的证据）
    assert_eq!(
        tighten(true, "mid", low.clone(), "中风险", "中风险"),
        ("中风险".to_string(), "中风险".to_string(), false),
        "本档更松却被采用 ⇒ 按档成了放松风险的通道（只升不降失守）"
    );
    // C 四路全缺席 ⇒ 与 v128 逐位相同（存量库未重播种的现网形态）
    assert_eq!(
        tighten(false, "", rhai::Dynamic::UNIT, "中风险", "中风险"),
        ("中风险".to_string(), "中风险".to_string(), false),
        "无分支时本段应整体不生效"
    );
    // D 所选档那一格缺席（unit）⇒ 不得把「没接到」当成某一档
    assert_eq!(
        tighten(true, "long", rhai::Dynamic::UNIT, "中风险", "中风险"),
        ("中风险".to_string(), "中风险".to_string(), false),
        "缺席被 risk_rank(unit) 的默认档伪装成了结论"
    );
    // E 算法档本就更严 ⇒ `overall_risk` 不动，但 f4 口径仍可采用比全局严的本档
    let (risk, llm, applied) = tighten(true, "short", high.clone(), "中风险", "极高风险");
    assert_eq!((risk.as_str(), llm.as_str(), applied), ("极高风险", "高风险", false));
}

/// 负控：把**第二道**单向比较（本档 vs 算法档）换成「只要不同就采用」，B 情形必须翻车
/// 这条存在理由是 `tighten` 的 A/B 两条若由同一个恒真判据得出，就锁不住方向。
#[test]
fn v129_one_way_guard_is_what_stops_the_looser_tier() {
    // 两道单向比较**一起**换成「只要不同就采用」——只拆第二道会被第一道（depth_upgrade）挡住，
    // 那样 mutant 与原判据同结果，负控就成了假红（本轮实测踩过一次，形态不同但同一类错）。
    let mutated = RISK_TIER_SRC
        .replace(
            "    && risk_rank(risk_tier) > risk_rank(overall_risk_llm);",
            "    && risk_tier != overall_risk_llm;",
        )
        .replace(
            "let risk_tier_applied = depth_upgrade && risk_rank(risk_tier) > risk_rank(overall_risk);",
            "let risk_tier_applied = depth_upgrade && risk_tier != overall_risk;",
        );
    assert_ne!(mutated, RISK_TIER_SRC, "变异未生效：这段副本里没有那两条单向比较");
    let mut engine = Engine::new();
    axagent_harness::register_common_functions(&mut engine);
    let mut scope = Scope::new();
    scope.push_constant("branch_picked", rhai::Dynamic::from(true));
    scope.push_constant("picked_tier", rhai::Dynamic::from("mid".to_string()));
    scope.push_constant("overall_risk_llm", rhai::Dynamic::from("中风险".to_string()));
    scope.push_constant("overall_risk_mid", rhai::Dynamic::from("低风险".to_string()));
    let script = format!(
        "{RISK_RANK_FN}
{PRESENT_FN}
let overall_risk = \"中风险\";
{mutated}
         #{{ \"risk\": overall_risk, \"llm\": risk_llm_eff }}"
    );
    let ast = engine.compile_with_scope(&scope, &script).expect("变异段应可编译");
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).expect("变异段应可求值");
    let got = r.get("risk").and_then(|x| x.clone().try_cast::<String>()).expect("risk");
    assert_eq!(got, "低风险", "负控失效：去掉单向守卫后仍没放松 ⇒ B 那条断言没在测方向");
}

/// v132（#23）：主链解禁臂必须消费取数层的真占比；旧「全数组求和再比 5.0」那条形状不得回归。
/// 旧实现把逐条 `unlockRatio`（= 股东 ÷ 当日合计 ×10000）当占比用，现网 152/154 次执行
/// 因此恒取 `clamp` 边界 —— 那是常数而不是证据，所以这里既锁新形状，也锁旧形状不许回来。
#[test]
fn v132_main_chain_lockup_arm_reads_supply_shock_not_the_summed_ratio() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    let code = without_comment_lines(pm);
    assert!(code.contains(r#"lb["supply_shock"]"#), "主链解禁臂没在消费 supply_shock 块");
    assert!(
        code.contains("clamp(r / 0.10, 0.0, 1.0)"),
        "该臂必须与 leg_signal 同一条线性口径（0.10 满强度）；换成 pm_saturate 就是同一个量两套读法"
    );
    assert!(!code.contains("upcoming_ratio"), "旧的求和变量回来了 ⇒ 这条臂又是常数偏置");
    assert!(
        !code.contains(r#"l["unlockRatio"]"#),
        "不得再按占比读日内份额字段（它已由 intraday_share_pct 取代）"
    );
    // 档位回落必须是 mid（本段标题原写的「未来 30 天」正是 mid 的 28 交易日），不是随手挑一档
    assert!(
        code.contains(r#"if branch_picked { picked_tier } else { "mid" }"#),
        "选档回落口径变了要显式同步本锁"
    );
    // 负控：把旧形状塞回去，上面两条禁词必须各抓到一个 —— 否则它们是恒真的
    let revived = code.replace(
        r#"let ss = lb["supply_shock"];"#,
        r#"let mut upcoming_ratio = 0.0;
            let _ = l["unlockRatio"];"#,
    );
    assert_ne!(revived, code, "负控未生效：锚定的那行不在主链脚本里");
    assert!(
        revived.contains("upcoming_ratio") && revived.contains(r#"l["unlockRatio"]"#),
        "负控失效：旧形状塞回去了却没被禁词抓到 ⇒ 那两条断言没在测这件事"
    );
    // 深度上界不许顺带改（本批只改量纲与窗口）
    assert!(
        code.contains("pressure * 0.35 * 0.25"),
        "该臂的深度上界不再是旧 0.35×0.25 ⇒ 与量纲修正在同一批里，归因会混"
    );
}
