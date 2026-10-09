//! seed 工作流模板工具声明 ↔ 运行时解析空间 一致性校验。
//!
//! 检测面：seed_stock_analysis.rs / seed_serenity.rs 里 ToolDef.name 声明的每个工具，
//! 必须能命中 ToolResolver 的解析空间：
//!
//! ```text
//! 全局 registry（tools::tools::register_all） ∪ stock_mcp_tools schema 清单
//!   ∪ industry_chain（stock-analysis） ∪ RhaiToolDef（workflow 内部 rhai 工具）
//! ```
//!
//! 若命中不了 → 运行时 ToolResolver 返回 None → 工具调用被 core.rs Failed 分支
//! `emit degraded: true` **静默吞掉**（节点标记 completed 但结果为空，无报错）。
//! 这是最隐蔽的故障形态，本测试用源码级提取把失联工具暴露出来。
//!
//! 提取规则说明（源码级，经实锤）：
//! - `name: "..."` 且下方 3 行内出现 `var_type:` → 是 Variable.name（模板变量），非工具，跳过；
//! - `name: "..."` 其余 → ToolDef.name（工具声明）；
//! - `tool_name: "..."` 字面量 → RhaiToolDef（workflow 内部 rhai 工具，走 code_executor，
//!   不经过 ToolResolver，计入可解析集合）；
//! - `tool_node(id, title, "tool", …)` 的**第 3 个位置实参**（仅字面量）→ ToolNode 声明的工具名。
//!   ⚠ 2026-09-21 才补上这条：缺它时，用位置参数声明的节点会**完全躲过**本门禁
//!   （见 `tool_node_positional_tool_names` 的说明与报告 §6.14）。

// 生产 Rhai 引擎工厂（与 `code_executor` 同一份配置）。本文件四道「按生产同配置真执行」的门
// 共用它 ⇒ 单点在模块级 import：横向引用从 8 处全路径收到 1 处（分层棘轮 `commands-no-sibling-call`
// 按行计数，逐处写全路径会把同一个依赖记成 8 个坑）。
use crate::commands::stock_workflow::rhai_registry::{RhaiSandboxLimits, build_stock_rhai_engine};
use regex::Regex;
use std::collections::HashSet;

/// 抽 `tool_node(...)` **位置参数**形态声明的工具名（第 3 个顶层实参，**仅字面量**）。
///
/// 形态：`tool_node(id, title, "tool_name", output_key, &[], None, x, y)`。
///
/// ## 为什么必须补这一条（2026-09-21，判据 #655 的实证）
///
/// 原抽取面只认 `name: "..."` / `tool_name: "..."` 两种**具名**形态，
/// 于是用位置参数声明的节点名**从未进入 `declared`** —— 门禁连它的存在都不知道，
/// 断言再对也是假绿。V79 的 `t-valuation-band` 就是这样躲过了这条专为它而写的门禁
/// （该节点调的工具名当时并未注册 ⇒ 运行时 `ToolResolver` 返 `None`
/// ⇒ `core.rs` Failed 分支 `emit degraded: true` 静默吞掉 ⇒ 输出空、无报错）。
///
/// ⚠ **第 3 项不是字面量时必须跳过**：通用包装函数里写的是变量
/// （`for … { nodes.push(tool_node(id, title, tool_name, …)) }`），
/// 把变量名当工具名会制造**假红**。
///
/// ⚠ **仍未覆盖的形态（已知盲区，本批未修，见报告 §6.14.6）**：两张元组表
/// （`tool_assignments: &[(&str, &str, &str, &str)]`、`algo_tools: &[AlgoToolRow]`）
/// 以**变量**把工具名传进 `tool_node`，文本级抽取覆盖不到它们；
/// 实测「元组第 3 元素」这种启发式会**溢出到无关元组**（`research-manager` / `trader` /
/// `tracing` 宏的参数都会被误当工具名）⇒ 用它扩面会制造大量假红，故不做。
fn tool_node_positional_tool_names(src: &str) -> Vec<String> {
    let needle = "tool_node(";
    let mut out: Vec<String> = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = src[search..].find(needle) {
        let call_start = search + rel + needle.len();
        // ① 括号平衡扫出整段实参（跳过字符串字面量内的括号与转义）
        let mut depth = 1i32;
        let mut in_str = false;
        let mut escaped = false;
        let mut call_end: Option<usize> = None;
        for (off, ch) in src[call_start..].char_indices() {
            if in_str {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_str = false;
                }
                continue;
            }
            match ch {
                '"' => in_str = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        call_end = Some(call_start + off);
                        break;
                    }
                },
                _ => {},
            }
        }
        let Some(call_end) = call_end else { break };
        // ② 按顶层逗号切实参
        let body = &src[call_start..call_end];
        let mut args: Vec<&str> = Vec::new();
        let mut depth = 0i32;
        let mut in_str = false;
        let mut escaped = false;
        let mut start = 0usize;
        for (off, ch) in body.char_indices() {
            if in_str {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_str = false;
                }
                continue;
            }
            match ch {
                '"' => in_str = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(&body[start..off]);
                    start = off + 1;
                },
                _ => {},
            }
        }
        args.push(&body[start..]);
        // ③ 第 3 个实参且为字符串字面量
        if let Some(third) = args.get(2) {
            if let Some(rest) = third.trim().strip_prefix('"') {
                if let Some(name) = rest.split('"').next() {
                    if !name.is_empty() && !out.iter().any(|n| n == name) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        search = call_end;
    }
    out
}

/// 抽取面自证：正控 + 两个负控 + 对真 seed 的端到端断言。
///
/// ## 为什么这条测试必须存在（判据 #643 / #655）
///
/// `tool_node_positional_tool_names` 是**判据工具**，而判据工具会**静默撒谎**：
/// 它少抽一个名字，下游 `assert_all_resolvable` 就**照样绿**（少检查一个而已）。
/// 本文件里另外两个同类抽取器（`round_literal_scanner_detects_known_bad_forms`、
/// `profile_tools_resolvable_scanner_discriminates`）都自带这种对照，本抽取面
/// 2026-09-21 新增时**没有** —— 补上，否则它就是下一个「断言对、输入面错」的假绿源。
///
/// ⚠ 最后一组断言是**这一条测试的真正价值**：它直接断言真 seed 源里的
/// `compute_valuation_band` 被这条规则**看见**。少了它，本规则可以「一个都没抽到」
/// 而让 `seed_stock_analysis_tools_all_resolvable` 保持绿色 —— 这正是 V79 的形态。
#[test]
fn tool_node_positional_scanner_discriminates() {
    // 正控①：位置参数字面量必须抽到（含嵌套括号/逗号/字符串内逗号括号的干扰）
    let positive = r#"
        nodes.push(tool_node("n1", "标题", "tool_alpha", "k", &[], None, 0.0, 0.0));
        nodes.push(tool_node("n2", "标题", "tool_beta", "k", &[("a", "b")], Some("x,y(z)"), f(1, 2), 0.0));
    "#;
    assert_eq!(
        tool_node_positional_tool_names(positive),
        vec!["tool_alpha".to_string(), "tool_beta".to_string()],
        "位置参数字面量必须被抽到"
    );

    // 正控②：同一文件里**两种形态混排**时互不干扰
    let mixed = r#"
        let n = tool_node(id, title, tool_name, out, &[], None, x, y);
        nodes.push(tool_node("n3", "标题", "tool_gamma", "k", &[], None, 1.0, 1.0));
    "#;
    assert_eq!(
        tool_node_positional_tool_names(mixed),
        vec!["tool_gamma".to_string()],
        "变量形态必须被跳过、字面量形态必须抽到"
    );

    // 负控①：第 3 项是**变量**（通用包装函数）⇒ 必须跳过。
    // 把变量名当工具名会制造大量假红，这条是防假红的守门人。
    let var_form = r#"nodes.push(tool_node(id, title, tool_name, output_key, &[], None, x, y));"#;
    assert!(
        tool_node_positional_tool_names(var_form).is_empty(),
        "变量形态必须跳过（否则把 `tool_name` 这种变量名当工具名 ⇒ 假红）"
    );

    // 负控②：实参不足 3 个 ⇒ 不产出、不 panic
    assert!(
        tool_node_positional_tool_names(r#"tool_node("n9", "标题")"#).is_empty(),
        "缺少第 3 实参时不得产出"
    );

    // 端到端：真 seed 源里本批新增的节点必须被这条规则**看见**
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/stock_analysis_setup/seed_stock_analysis.rs");
    let real = std::fs::read_to_string(&path).expect("读取 seed_stock_analysis.rs 失败");
    let names = tool_node_positional_tool_names(&real);
    assert!(
        names.iter().any(|n| n == "compute_valuation_band"),
        "`t-valuation-band` 的工具名未被抽取面看到 ⇒ 下游门禁会假绿（V79 成因）: {names:?}"
    );
}

/// 从 seed 源文件提取 (工具声明集, rhai 工具名集)。
fn seed_tool_def_names(seed_rs: &str) -> (Vec<String>, Vec<String>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/stock_analysis_setup")
        .join(seed_rs);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {seed_rs} 失败: {e}"));
    let lines: Vec<&str> = src.lines().collect();
    let mut declared: Vec<String> = Vec::new();
    let mut rhai: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(start) = line.find("name: \"") {
            let rest = &line[start + 7..];
            if let Some(end) = rest.find('"') {
                let name = rest[..end].to_string();
                // 下方 3 行内出现 var_type: → 模板变量，跳过
                let is_var = lines.iter().skip(i + 1).take(3).any(|l| l.contains("var_type:"));
                if !is_var {
                    declared.push(name);
                }
            }
        }
        // RhaiToolDef：`tool_name: "..."` 字面量（"tool_name: \"" 共 12 字符）
        if let Some(start) = line.find("tool_name: \"") {
            let rest = &line[start + 12..];
            if let Some(end) = rest.find('"') {
                let name = rest[..end].to_string();
                if !rhai.contains(&name) {
                    rhai.push(name);
                }
            }
        }
    }
    // ToolNode 的**位置参数**形态（具名形态已由上面的 `name:` 规则覆盖）。
    // 缺这一段时，`tool_node(id, title, "tool", …)` 声明的工具名完全躲过本门禁。
    for name in tool_node_positional_tool_names(&src) {
        if !declared.iter().any(|n| n == &name) {
            declared.push(name);
        }
    }
    (declared, rhai)
}

/// ToolResolver 解析空间 = 全局 registry ∪ stock schema 清单 ∪ G3 产业链 ∪ rhai 工具。
fn resolvable_tool_names(rhai: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    // 全局 registry（tools crate 全部内置工具，与 init/services.rs ToolResolver 一致）
    let mut registry = axagent_tools::registry::ToolRegistry::new();
    axagent_tools::tools::register_all(&mut registry);
    set.extend(registry.list_all().into_iter().map(|t| t.name));
    // astock-data 股票工具 schema 清单（STOCK_TOOL_NAMES 来源之一）
    set.extend(
        axagent_astock_data::mcp_tools::stock_mcp_tools()
            .into_iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from)),
    );
    // G3 产业链工具（STOCK_TOOL_NAMES 来源之二）
    set.extend(
        axagent_analysis_engine::mcp_tools::industry_chain_mcp_tools()
            .into_iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from)),
    );
    // workflow 内部 rhai 工具（code_executor 执行，不经 ToolResolver）
    set.extend(rhai.iter().cloned());
    set
}

fn assert_all_resolvable(seed_rs: &str) {
    let (declared, rhai) = seed_tool_def_names(seed_rs);
    let resolvable = resolvable_tool_names(&rhai);
    let missing: Vec<String> =
        declared.iter().filter(|n| !resolvable.contains(*n)).cloned().collect();
    assert!(
        missing.is_empty(),
        "[{seed_rs}] 声明 {} 个工具，其中 {} 个不在 ToolResolver 解析空间:\n  {:?}\n\
         这些工具运行时调用会解析为 None，被 degraded 机制静默吞掉（节点 completed 但结果为空）。\n\
         rhai 工具（可解析）: {:?}",
        declared.len(),
        missing.len(),
        missing,
        rhai
    );
}

/// P2 写侧结构门：每个 `reco_picks::ActiveModel` 字面量都必须给 `reco_version`。
///
/// 为什么锁写入而不是读取：算法归属是**不可逆**的 —— 今天不盖章的样本，将来无论
/// 怎么补筛都拿不回来（只能整批当「未知代」排除）。而荐股闭环是按 rank IC 降权的，
/// 没有归属就是把「旧算法的预测」和「新算法的预测」配进同一格。
#[test]
fn every_reco_picks_insert_stamps_algorithm_version() {
    let files = ["src/commands/stock_analysis.rs", "src/commands/stock_workflow/serenity.rs"];
    let mut checked = 0usize;
    for rel in files {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"));
        for (i, line) in src.lines().enumerate() {
            if !line.contains("reco_picks::ActiveModel {") {
                continue;
            }
            checked += 1;
            // 字面量向后扫到闭合（同层 `};`），其间必须出现 reco_version
            let rest = &src[src.find(line).unwrap()..];
            let body = rest
                .split(
                    "
        };",
                )
                .next()
                .unwrap_or(rest);
            let body = body
                .split(
                    "
            };",
                )
                .next()
                .unwrap_or(body);
            assert!(
                body.contains("reco_version:"),
                "[{rel}:{}] `reco_picks` 建行没盖章 `reco_version` ⇒ 该批样本算法归属永久未知，                 闭环 IC 只能整批排除它（或更糟：混进当前代）。",
                i + 1
            );
        }
    }
    assert_eq!(checked, 2, "预期扫到 2 处 reco_picks 建行点，实际 {checked} —— 有新增写入点未入册");
}

/// P2 依赖锁：`reco_ic_gate` 要能翻到 `on`，前提是 IC 聚合已按算法版本筛同代样本。
///
/// 现状（实证）：库里**根本没有** `reco_ic_gate` 这个变量 ⇒ 走代码缺省 `shadow`
/// （`reco_loop.rs:530-537`），所以跨代混池**当前未激活**，但它离激活只差一个变量。
/// 本锁把这条依赖固定下来：一旦 `IC 行结构` 里出现 `algorithm_version`（即筛样已实现），
/// 才允许 seed 里出现 `on`；否则 seed 出现 `on` 即红。
#[test]
fn ic_gate_can_only_turn_on_after_version_screening_exists() {
    let ic_src = {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/analysis-engine/src/recommender/ic.rs");
        std::fs::read_to_string(&path).expect("读取 recommender/ic.rs 失败")
    };
    let screening_ready = ic_src.contains("algorithm_version");
    let seed_turns_on = {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/commands/stock_analysis_setup/seed_variables.rs");
        std::fs::read_to_string(&path).expect("读取 seed_variables.rs 失败")
    };
    let gate_on_seeded =
        seed_turns_on.lines().any(|l| l.contains("reco_ic_gate") && l.contains("\"on\""));
    assert!(
        !gate_on_seeded || screening_ready,
        "seed 把 reco_ic_gate 设成 on，但 `RecoIcRow` 还不带 algorithm_version ⇒          IC 会把跨代样本混成一格后自动降权。先做同代筛样，再翻闸。"
    );
}

/// 负控：筛样判据本身要能红 —— 造一段「seed 已 on 且 ic 无版本谓词」的文本必须被识别。
#[test]
fn ic_gate_dependency_lock_detects_the_bad_pairing() {
    let bad_seed = "    (\"reco_ic_gate\".into(), \"on\".into()),";
    let hits = bad_seed.lines().any(|l| l.contains("reco_ic_gate") && l.contains("\"on\""));
    assert!(hits, "负控失效：判据认不出「seed=on」");
    let good_seed = "    (\"reco_ic_gate\".into(), \"shadow\".into()),";
    assert!(!good_seed.lines().any(|l| l.contains("reco_ic_gate") && l.contains("\"on\"")));
}

/// P1-11（2026-10-03，需求③「时间旅行模式下不得让模型联网取数」）。
///
/// 现状是**结构性**满足：股票模板的工具面是白名单，里面没有联网类工具，
/// 且 `crates/providers/` 里根本不存在搜索开关（普查实证零命中）。
/// 但这条性质此前**无人看守** —— 谁给某个分析师挂上 WebFetch/WebSearch，
/// 回放就会让模型读到截止日之后的内容，而库里读不出任何异常。
/// 于是把它做成门：模板声明的工具 ∩ registry 里 `ToolCategory::Network` = 空集。
fn assert_no_network_tools(seed_rs: &str) {
    let (declared, _) = seed_tool_def_names(seed_rs);
    let mut registry = axagent_tools::registry::ToolRegistry::new();
    axagent_tools::tools::register_all(&mut registry);
    let network: HashSet<String> = registry
        .list_all()
        .into_iter()
        .filter(|t| t.category == axagent_harness::tool::ToolCategory::Network)
        .map(|t| t.name)
        .collect();
    let hits: Vec<String> = declared.iter().filter(|n| network.contains(*n)).cloned().collect();
    assert!(
        hits.is_empty(),
        "[{seed_rs}] 声明了联网类工具 {hits:?}。时间旅行回放里这等于让模型看截止日之后的内容，         且 provider 层没有可用的搜索开关 —— 要放行请先改判据并显式登记豁免理由。"
    );
}

/// 正负对照：判据本身要能红（把联网工具名塞进声明文本，必须被点名）。
#[test]
fn network_tool_gate_fires_on_webfetch_and_stays_clean_on_real_seeds() {
    let mut registry = axagent_tools::registry::ToolRegistry::new();
    axagent_tools::tools::register_all(&mut registry);
    let network: Vec<String> = registry
        .list_all()
        .into_iter()
        .filter(|t| t.category == axagent_harness::tool::ToolCategory::Network)
        .map(|t| t.name)
        .collect();
    assert!(!network.is_empty(), "registry 里应当存在 Network 类工具，否则本门恒真是空判据");
    // 负控文本：真实存在的一个联网工具名写进 `name: "..."` 形态，必须被抽到并命中
    let fake = format!("    let t = ToolDef {{ name: \"{}\".into(), ..}};", network[0]);
    let declared: Vec<String> = {
        let mut v = Vec::new();
        for line in fake.lines() {
            if let Some(start) = line.find("name: \"") {
                let rest = &line[start + 7..];
                if let Some(end) = rest.find('"') {
                    v.push(rest[..end].to_string());
                }
            }
        }
        v
    };
    assert!(declared.contains(&network[0]), "负控应抽到联网工具名：{declared:?}");
    assert_no_network_tools("seed_stock_analysis.rs");
    assert_no_network_tools("seed_serenity.rs");
}

#[test]
fn seed_stock_analysis_tools_all_resolvable() {
    assert_all_resolvable("seed_stock_analysis.rs");
}

#[test]
fn seed_serenity_tools_all_resolvable() {
    assert_all_resolvable("seed_serenity.rs");
}

/// 2026-09-20 补：`seed_daily_market_events.rs` 此前**从未**被本门禁覆盖
/// —— 上面两条只查 `seed_stock_analysis.rs` / `seed_serenity.rs`。
///
/// 覆盖盲区的代价已实证：该模板声明的 `market_mainline_batch_upsert` 长期不在
/// 解析空间里（工作流工具解析走 ToolResolver，不含 `#[agent_command]` 元数据），
/// 而模板提示词却要求模型调用它 ⇒ 每次运行都走 degraded 静默吞掉，
/// **无任何测试报警**。本断言让同型问题下次直接红在 CI。
#[test]
fn seed_daily_market_events_tools_all_resolvable() {
    assert_all_resolvable("seed_daily_market_events.rs");
}

// ─────────────────────────────────────────────────────────────────────────────
// 专家注册三表一致性
//
// 背景（2026-09-14，真实故障）：模板节点 `decision-explainer` 的
// `agent_profile_id = "stock-explainer"`，但 `.md` / `EMBEDDED_PROMPTS` /
// `EXPERT_ROLE_MAP` 三处**全无** explainer ⇒ `seed_agent_profiles` 不会为它建
// profile 行 ⇒ `agent_executor` 解析 profile 得 None ⇒ expert 提示词整段被
// 跳过（只打一条 WARN，节点仍 completed）——**静默降级**，无任何测试报警。
//
// 三个数组各只管一件事，缺任何一个都会让专家"半残"：
//   - `.md` + EMBEDDED_PROMPTS → agency_experts 行（提示词正文来源）
//   - EXPERT_ROLE_MAP          → agent_profiles 行的**存在性**（遍历它才建 profile）
//   - PROFILE_TOOLS            → agent_profiles.recommended_tools
//
// 覆盖范围声明：本测试只校验**三表 key 集合**（静态、编译期事实）。
// 模板侧 `agent_profile_id` 是否落在三表内，由 `seed_stock_analysis` 种子化时
// 的越界告警兜底（见 `warn_unknown_agent_profiles`），不在本测试覆盖内。
// ─────────────────────────────────────────────────────────────────────────────

/// 精确提取单个 `.md` 的 `name:`（frontmatter）—— 用于反向校验"文件是否被引用"。
fn embedded_prompt_ids() -> Vec<String> {
    use super::EMBEDDED_PROMPTS;
    EMBEDDED_PROMPTS.iter().map(|(id, _)| (*id).to_string()).collect()
}

fn expert_role_map_ids() -> Vec<String> {
    use super::EXPERT_ROLE_MAP;
    EXPERT_ROLE_MAP.iter().map(|(id, _)| (*id).to_string()).collect()
}

fn profile_tools_ids() -> Vec<String> {
    use super::PROFILE_TOOLS;
    PROFILE_TOOLS.iter().map(|(id, _)| (*id).to_string()).collect()
}

/// 断言一个 id 清单内无重复（同一 id 定义两次 ⇒ 后者静默覆盖前者）。
fn assert_no_dup(label: &str, ids: &[String]) {
    let mut seen = std::collections::HashSet::new();
    let dups: Vec<&String> = ids.iter().filter(|i| !seen.insert((*i).clone())).collect();
    assert!(dups.is_empty(), "[{label}] 存在重复 id（后者会静默覆盖前者）: {dups:?}");
}

#[test]
fn expert_registration_tables_are_consistent() {
    let embedded = embedded_prompt_ids();
    let roles = expert_role_map_ids();
    let tools = profile_tools_ids();

    assert_no_dup("EMBEDDED_PROMPTS", &embedded);
    assert_no_dup("EXPERT_ROLE_MAP", &roles);
    assert_no_dup("PROFILE_TOOLS", &tools);

    let e: HashSet<&String> = embedded.iter().collect();
    let r: HashSet<&String> = roles.iter().collect();
    let t: HashSet<&String> = tools.iter().collect();

    let only_embedded: Vec<&&String> = e.difference(&r).collect();
    let only_role: Vec<&&String> = r.difference(&e).collect();
    let no_tools_row: Vec<&&String> = e.difference(&t).collect();

    assert!(
        only_embedded.is_empty() && only_role.is_empty(),
        "EMBEDDED_PROMPTS 与 EXPERT_ROLE_MAP 的专家集合不一致 ——\n\
         EXPERT_ROLE_MAP 是 `seed_agent_profiles` 的**遍历源**，不在其中就不会建 profile，\n\
         模板引用该 profile 时 agent_executor 解析为 None → expert 提示词静默跳过。\n\
         仅在 EMBEDDED_PROMPTS（有提示词但无 profile）: {only_embedded:?}\n\
         仅在 EXPERT_ROLE_MAP（有 profile 但无提示词）: {only_role:?}"
    );
    assert!(
        no_tools_row.is_empty(),
        "以下专家在 EMBEDDED_PROMPTS 里但不在 PROFILE_TOOLS 中 ⇒ \
         agent_profiles.recommended_tools 落 NULL（「未配置」而非「确认为空」）。\n\
         若该专家确实不需要工具，请显式登记为 `(\"<id>\", &[])`：{no_tools_row:?}"
    );
}

/// 反向：PROFILE_TOOLS 里出现、但 EMBEDDED_PROMPTS 没有的 id
/// ⇒ 工具白名单挂在了一个不存在的专家上（死映射，恒不生效）。
#[test]
fn profile_tools_have_no_orphans() {
    let embedded: HashSet<String> = embedded_prompt_ids().into_iter().collect();
    let orphans: Vec<String> =
        profile_tools_ids().into_iter().filter(|id| !embedded.contains(id)).collect();
    assert!(
        orphans.is_empty(),
        "PROFILE_TOOLS 中存在无对应专家的孤儿条目（工具白名单永远不会被读取）: {orphans:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 辩论轮次引用必须参数化 —— 悬空入边回归守护
//
// 背景（2026-09-14，真实故障，整条链连建模都过不去）：
// `TEMPLATE_VERSION` 47 → 48 把 `debate_max_rounds` 由 3 改为 1 后，
// `bull-r2 / bear-r2 / bull-r3 / bear-r3` 不再生成。但 seed 里有一处写死了
// `edge("e-bear-r3-t-scoring", "bear-r3", "t-scoring")` ⇒ 悬空入边 ⇒
// `create_workflow` **启动期硬失败**：
//
// ```text
// 创建工作流失败: Node 't-scoring' depends on non-existent 'bear-r3'
// ```
//
// 注意它**不是**运行期降级（那种会静默跳过下游），是「工作流根本建不起来」，
// 用户看到的是「启动失败」。
//
// 判据：辩手节点 id 的正确形态是 `bull-r{round}` / `bear-r{round}`，轮数由
// `debate_max_rounds` 派生（`seed_stock_analysis.rs:3313` 的 `for round in
// 0..debate_max_rounds`）。因此任何**以字符串字面量形式**出现在「边的 source /
// target」位置的 `bull-rN` / `bear-rN` 都是定时炸弹：轮数一变即悬空。
//
// ⚠️ 必须覆盖**两种**写法 —— 实测只查 `edge(...)` 会漏 6 处：
//   ① `edge(id, source, target)` 闭包调用（同文件 `:1405` 定义）；
//   ② 直接 `edges.push(WorkflowEdge { id: "...", source: "...", target: "..." })`。
//
// 本测试对**源码**做字符串级提取（与 `seed_tool_def_names` 同款风格），
// 因为 seed 函数需要 `&DatabaseConnection`，无法在单测里直接调用来取 nodes/edges。
// ─────────────────────────────────────────────────────────────────────────────

/// 去注释（字符串感知），换行原样保留以维持行号对应。
fn strip_rust_comments(src: &str) -> String {
    let cs: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let (mut in_str, mut in_block) = (false, false);
    while i < cs.len() {
        let c = cs[i];
        let n = if i + 1 < cs.len() { cs[i + 1] } else { '\0' };
        if in_block {
            if c == '*' && n == '/' {
                in_block = false;
                out.push_str("  ");
                i += 2;
            } else {
                out.push(if c == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if in_str {
            if c == '\\' {
                out.push(c);
                if i + 1 < cs.len() {
                    out.push(cs[i + 1]);
                }
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && n == '/' {
            while i < cs.len() && cs[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if c == '/' && n == '*' {
            in_block = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `bull-rN` / `bear-rN`（整串精确匹配，N 为纯数字且非空）。
fn is_round_id(s: &str) -> bool {
    for pat in ["bull-r", "bear-r"] {
        if let Some(d) = s.strip_prefix(pat) {
            return !d.is_empty() && d.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

/// 扫描结果：(WorkflowEdge 字面量块数, edge() 调用数, 命中的 source/target/id 字面量行数, 违规清单)。
///
/// 前三个计数是**自证**：`0 命中` 与「扫描器根本没扫到」必须可区分
/// （否则就是「审计脚本自己撒谎」——本项目已固化判据 #7）。
fn scan_round_literal_edge_endpoints(src: &str) -> (usize, usize, usize, Vec<String>) {
    let stripped = strip_rust_comments(src);
    let lines: Vec<&str> = stripped.lines().collect();
    let mut bad: Vec<String> = Vec::new();
    let mut blocks = 0usize;
    let mut calls = 0usize;
    let mut key_hits = 0usize;

    // ① 直接 edges.push(WorkflowEdge { source: "...".into(), target: "...".into() })
    //
    // ⚠️ 端点写法带 `.into()`（`source: "trigger".into(),`）。模式必须容忍它，
    //    否则整块扫不到 —— 实测漏写 `.into()` 时 6 个块全部静默跳过，
    //    扫描器会报「0 违规」而其实一个字都没看。
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("edges.push(WorkflowEdge") {
            continue;
        }
        blocks += 1;
        for (j, bl) in lines.iter().enumerate().skip(i).take(40) {
            let t = bl.trim();
            if t == "});" {
                break;
            }
            for key in ["source:", "target:", "id:"] {
                let Some(rest) = t.strip_prefix(key) else { continue };
                let rest = rest.trim();
                let Some(rest) = rest.strip_prefix('"') else { continue };
                let Some(close) = rest.find('"') else { continue };
                let inner = &rest[..close];
                let tail = rest[close + 1..].trim();
                // 允许 `,` 或 `.into(),`
                if !(tail == "," || tail == ".into(),") {
                    continue;
                }
                key_hits += 1;
                if is_round_id(inner) {
                    bad.push(format!(
                        "L{}  WorkflowEdge {{ {key} \"{inner}\".into() }}  ← 字面量轮次端点",
                        j + 1
                    ));
                }
            }
        }
    }

    // ② edge(closure) 调用：取前 3 个实参，检查第 2/3 个（source/target）
    let cs: Vec<char> = stripped.chars().collect();
    let mut i = 0usize;
    while i + 5 <= cs.len() {
        if !(cs[i] == 'e'
            && cs[i + 1] == 'd'
            && cs[i + 2] == 'g'
            && cs[i + 3] == 'e'
            && cs[i + 4] == '(')
        {
            i += 1;
            continue;
        }
        // 跳过闭包定义自身：`let edge = |id: &str, ...|`
        let before: String = cs[i.saturating_sub(24)..i].iter().collect();
        if before.trim_end().ends_with("let") {
            i += 5;
            continue;
        }
        calls += 1;
        let line_no = stripped[..stripped.char_indices().nth(i).map(|(b, _)| b).unwrap_or(0)]
            .matches('\n')
            .count()
            + 1;

        let mut depth = 0i32;
        let mut k = i + 5;
        let mut args: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut in_str = false;
        while k < cs.len() {
            let c = cs[k];
            if in_str {
                if c == '\\' && k + 1 < cs.len() {
                    cur.push(c);
                    cur.push(cs[k + 1]);
                    k += 2;
                    continue;
                }
                if c == '"' {
                    in_str = false;
                }
                cur.push(c);
                k += 1;
                continue;
            }
            if c == '"' {
                in_str = true;
                cur.push(c);
                k += 1;
                continue;
            }
            if c == '(' || c == '[' || c == '{' {
                depth += 1;
                cur.push(c);
                k += 1;
                continue;
            }
            if c == ')' || c == ']' || c == '}' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                cur.push(c);
                k += 1;
                continue;
            }
            if c == ',' && depth == 0 {
                args.push(cur.trim().to_string());
                cur.clear();
                k += 1;
                continue;
            }
            cur.push(c);
            k += 1;
        }
        args.push(cur.trim().to_string());

        for (ai, key) in [(1usize, "source"), (2usize, "target")] {
            let Some(raw) = args.get(ai) else { continue };
            if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
                let inner = &raw[1..raw.len() - 1];
                if is_round_id(inner) {
                    bad.push(format!(
                        "L{line_no}  edge(.., \"{inner}\", ..)  ← 字面量轮次端点（应为 `&format!(\"{key}\"... )`）"
                    ));
                }
            }
        }
        i = k;
    }

    (blocks, calls, key_hits, bad)
}

#[test]
fn debater_round_refs_are_parameterized() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/stock_analysis_setup")
        .join("seed_stock_analysis.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 seed_stock_analysis.rs 失败: {e}"));

    let (blocks, calls, key_hits, bad) = scan_round_literal_edge_endpoints(&src);

    // 自证：扫描器必须真的扫到了边，否则「0 违规」毫无意义。
    // 若这里是扫描器坏了（例如 WorkflowEdge 端点改写法导致模式失配），
    // 必须**先修扫描器**，而不是放宽本断言。
    assert!(
        blocks >= 1 && calls >= 1 && key_hits >= 1,
        "扫描器未覆盖到任何边端点（WorkflowEdge 块={blocks}, edge() 调用={calls}, \
         命中端点字面量行={key_hits}）——\n\
         0 命中 ≠ 没问题。先让扫描器证明它看过了边（已知 seed 里有 6 处 WorkflowEdge \
         字面量块 / 15 行端点字面量 / 数十次 edge() 调用）。"
    );

    assert!(
        bad.is_empty(),
        "辩手节点的 id 被**硬编码**在边的端点上 —— 轮数一变即成悬空入边，\n\
         `create_workflow` 会在启动期直接拒绝（不是运行期降级）：\n  {}\n\n\
         修复：源/目标改用 `&format!(\"bear-r{{debate_max_rounds}}\")` 派生，\n\
         不要写死 `bear-r3`（2026-09-14 真实故障：t-scoring depends on non-existent 'bear-r3'）。",
        bad.join("\n  ")
    );
}

/// 扫描器的**负控**：喂进已知坏形态，必须抓到。
///
/// 没有这个测试，`debater_round_refs_are_parameterized` 只能证明「当前没违规」，
/// 无法证明「扫描器不是瞎的」—— 而 2026-09-14 的实战教训恰恰是：扫描器连
/// `source: "x".into()` 的 `.into()` 都没容忍，6 个块全部静默跳过却报「0 违规」。
#[test]
fn round_literal_scanner_detects_known_bad_forms() {
    // 形态 ①：edge() 闭包调用，source 写死
    let bad_closure = r#"
fn f() {
    edges.push(edge("e-bear-r3-t-scoring", "bear-r3", "t-scoring"));
}
"#;
    let (blocks, calls, key_hits, bad) = scan_round_literal_edge_endpoints(bad_closure);
    assert_eq!(bad.len(), 1, "应抓到 edge() 的 source 硬编码，实际: {bad:?}");
    assert!(bad[0].contains("bear-r3"), "定位信息应含节点 id: {bad:?}");
    assert_eq!((blocks, calls, key_hits), (0, 1, 0), "计数自证不符");

    // 形态 ②：WorkflowEdge 字面量块，source 带 .into()（实战漏掉的那种写法）
    let bad_literal = r#"
fn f() {
    edges.push(WorkflowEdge {
        id: "e-bull-r2-x".into(),
        source: "bull-r2".into(),
        source_handle: None,
        target: "t-x".into(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    });
}
"#;
    let (blocks, calls, _kh, bad) = scan_round_literal_edge_endpoints(bad_literal);
    assert_eq!(bad.len(), 1, "应抓到 WorkflowEdge 的 source 硬编码，实际: {bad:?}");
    assert!(bad[0].contains("bull-r2"), "定位信息应含节点 id: {bad:?}");
    assert_eq!((blocks, calls), (1, 0), "计数自证不符");

    // 形态 ③（合法）：参数化写法必须**零误报**
    let good = r#"
fn f() {
    edges.push(edge(&format!("e-bear-r{n}-t"), &format!("bear-r{n}"), "t-scoring"));
    edges.push(WorkflowEdge {
        id: "e-a-b".into(),
        source: "a".into(),
        target: "b".into(),
    });
}
"#;
    let (blocks, calls, key_hits, bad) = scan_round_literal_edge_endpoints(good);
    assert!(bad.is_empty(), "参数化写法被误报: {bad:?}");
    assert_eq!((blocks, calls), (1, 1), "计数自证不符");
    assert!(key_hits >= 2, "WorkflowEdge 端点字面量行应被计入覆盖自证: {key_hits}");

    // 形态 ④（合法）：**注释里**的坏写法必须被忽略（去注释器生效）
    let commented = r#"
fn f() {
    // 历史 bug: edges.push(edge("e-bear-r3-t-scoring", "bear-r3", "t-scoring"));
    edges.push(edge("e-x-y", "x", "y"));
}
"#;
    let (_, calls, _, bad) = scan_round_literal_edge_endpoints(commented);
    assert!(bad.is_empty(), "注释里的坏写法被误报（去注释器失效）: {bad:?}");
    assert_eq!(calls, 1, "注释里的 edge() 未去注释（calls 应为 1）: {calls}");

    // 形态 ⑤（合法）：bull-r1/bear-r1 是**字面量但不在边端点位置**（专家 id map）
    let expert_map = r#"
fn f() {
    let e = match r { 2 => "bull-r2", 3 => "bull-r3", _ => "bull-researcher" };
    edges.push(edge("e-a-b", "a", "b"));
}
"#;
    let (_, _, _, bad) = scan_round_literal_edge_endpoints(expert_map);
    assert!(bad.is_empty(), "非端点位置的轮次字面量被误报: {bad:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// PROFILE_TOOLS 推荐工具名 ↔ 运行时解析空间
//
// 背景（2026-09-19，真实缺陷）：`PROFILE_TOOLS` 里曾有 **7 个在任何解析空间都
// 不存在的工具名**，且**两条消费链都是静默丢弃**：
//
//   ① 工作流侧：`seed_stock_analysis.rs` 的 `tool_def_map`
//      （`filter_map(|tn| tool_def_map.get(tn))` —— 未命中直接跳过，无日志）；
//   ② chat 侧：`agent_profiles.recommended_tools`（`mod.rs` 的 `PROFILE_TOOLS.iter().cloned()`
//      → `recommended_tools: Set(tools_json)` 两处 UPSERT 落库）
//      → `local_tool.rs` 的 `get_chat_tools_by_names`（按名 `filter`，未命中跳过）。
//
// 唯一「可见」的后果是前端 `ExpertSelector.tsx` 照显这些工具的**数量**
// （`t("expertSelector.tools", { count: role.recommendedTools.length })` —— 只渲染
//  计数、不渲染名字）⇒ 数字虚高 ⇒ **声明 ≠ 实际**。
//
// 与既有 `seed_*_tools_all_resolvable` 的分工（务必区分，否则会以为已覆盖）：
//   - 那两条断言的是 **seed 文件里声明的 ToolDef.name**；
//   - 本条断言的是 **PROFILE_TOOLS 的专家推荐工具名**。
// 两者是不同集合 —— 实测 7 个幽灵名只在 `PROFILE_TOOLS` 出现、seed 侧一个都没有，
// 所以旧测试**结构上不可能**发现它们。
//
// 判据空间直接复用 `resolvable_tool_names()`（全局 registry ∪ stock_mcp_tools
// ∪ industry_chain_mcp_tools ∪ rhai 工具），不另建一份并集（避免第 N 份手抄副本）。
// ─────────────────────────────────────────────────────────────────────────────

/// 已知的历史幽灵名（2026-09-19 清理）。用途：**防回流**负控 + 记录「它们不存在」。
///
/// ⚠ 若将来其中某个真的被实现为工具，本条会失败 —— 那是**预期信号**：应从本清单移除，
/// 并在 `PROFILE_TOOLS` 里按需启用，而不是放宽断言。
const HISTORIC_GHOST_TOOL_NAMES: &[&str] = &[
    "get_dragon_tiger_list",
    "get_north_flow",
    "trace_industry_chain",
    "get_account_info",
    "get_stock_risk_metrics",
    "compute_volatility",
];

// 2026-09-20 从上方清单**移除** `market_mainline_batch_upsert` —— 它已被实现为**真实工具**
// （`crates/tools/src/tools/market_mainline.rs`，注册于 `tools/mod.rs` 的
// `register_all`）。这正是本清单注释预设的「预期信号」分支：实现后必须移除，
// 否则负控断言 `profile_tools_resolvable_scanner_discriminates` 会红。
//
// 为什么必须补上这个工具（而不是只复活 Tauri 命令层）：工作流的工具解析走
// `init/services.rs` 的 `ToolResolver`，判据是 `reg.list_all_tool_names()` ∪
// `reg.mcp.mcp_tools`，**不含** `#[agent_command]` 元数据。只修命令层时，
// daily-market-events 模板声明的这个名字仍解析为 None，调用被静默降级
// （节点 completed、结果为空）。同型先例见 `tools/mod.rs` 的 OPC 段注释。

/// 2026-09-19 用它们**替换**了幽灵名的等价真实工具（改名未同步的两个）。
const REPLACEMENT_TOOL_NAMES: &[&str] = &["get_market_dragon_tiger", "get_north_bound_flow"];

/// 构造完整判据空间（含两个 seed 文件里的 rhai 工具名）。
fn full_resolvable_tool_names() -> HashSet<String> {
    let (_, mut rhai) = seed_tool_def_names("seed_stock_analysis.rs");
    let (_, rhai_serenity) = seed_tool_def_names("seed_serenity.rs");
    rhai.extend(rhai_serenity);
    resolvable_tool_names(&rhai)
}

#[test]
fn profile_tools_all_resolvable() {
    use super::PROFILE_TOOLS;

    let resolvable = full_resolvable_tool_names();

    // 自证①：解析空间必须真的装到了东西（防「空间为空 ⇒ 全部 missing ⇒ 假红」
    // 与反过来的「空间巨大 ⇒ 恒绿」两种自欺）。
    assert!(
        resolvable.len() >= 50,
        "解析空间异常：仅 {} 个工具名（预期 ≥ 50 —— registry + 57 个 MCP 工具 + rhai）。\
         先修扫描器/空间构造，不要放宽本断言。",
        resolvable.len()
    );

    let mut total = 0usize;
    let mut missing: Vec<String> = Vec::new();
    for (expert, tools) in PROFILE_TOOLS.iter() {
        for t in tools.iter() {
            total += 1;
            if !resolvable.contains(*t) {
                missing.push(format!("{expert} → {t}"));
            }
        }
    }

    // 自证②：扫描面必须非零。`0 命中 ≠ 没问题`（判据 #7）。
    // 阈值取保守下界（本表实际名次远多于它），只用来抓「扫描器失配 ⇒ total=0」这一类。
    assert!(
        total >= 100,
        "扫描面异常：PROFILE_TOOLS 只解析出 {total} 个工具名次 —— 若 PROFILE_TOOLS 的\
         写法变了、或扫描器正则失配，先修扫描器/本测试，不要放宽本断言。"
    );

    assert!(
        missing.is_empty(),
        "PROFILE_TOOLS 声明了 {} 个解析空间里不存在的工具名（共扫描 {total} 个名次）：\n  {}\n\n\
         后果：这些名字在**两条消费链**上都被静默丢弃（`filter_map` / 按名 filter，均无告警），\n\
         但前端 `ExpertSelector` 会照显 ⇒ 用户以为专家有这些工具。\n\
         处理：① 改成真实工具名（先确认它在 `resolvable_tool_names()` 里）；\n\
               ② 或删除 —— 删除属**零行为变更**（它们本就解析不到）。\n\
         ⚠ 不要用 allowlist 放行：加一个就该先问「为什么留着一个解析不到的名字」。",
        missing.len(),
        missing.join("\n  ")
    );
}

/// 负控 + 正向对照：证明幽灵名确实不可解析、替换目标确实可解析。
///
/// 没有这条，`profile_tools_all_resolvable` 只能证明「当前声明都能解析」，
/// 无法证明「它真能分辨得出不能解析的名字」—— 而解析口径若有变（例如
/// `resolvable_tool_names` 被改成包含 agent_command 全集），本测试会静默失效。
#[test]
fn profile_tools_resolvable_scanner_discriminates() {
    let resolvable = full_resolvable_tool_names();

    for ghost in HISTORIC_GHOST_TOOL_NAMES {
        assert!(
            !resolvable.contains(*ghost),
            "负控失败：`{ghost}` 现在**竟然**在解析空间里了。\n\
             要么解析口径变了（需重新审视本清单与 `PROFILE_TOOLS`），\
             要么它被实现成了真实工具（那是好事 —— 请从 HISTORIC_GHOST_TOOL_NAMES 移除，\
             并按需在 PROFILE_TOOLS 启用）。"
        );
    }

    for good in REPLACEMENT_TOOL_NAMES {
        assert!(
            resolvable.contains(*good),
            "正向对照失败：`{good}` 不在解析空间 ⇒ 2026-09-19 用它替换幽灵名的改动无效。"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// md 提示词里的工具名引用 ↔ 运行时解析空间（第三条消费链）
//
// 背景（2026-09-19，真实缺陷）：`stock-analysis/market-synthesizer.md` 正文写的是
//   `get_dragon_tiger_list` / `get_north_flow` —— 这两个名字**全仓不存在**；
//   而同一仓库里另外 5 处（`skills/stock-pick/SKILL.md`、`skills/risk-management/SKILL.md`、
//   `skills/market-mainline/SKILL.md`、`stock-analysis/trend-scanner.md`、
//   `stock-analysis/fundamentals-analyst.md`）早已用真实名
//   `get_market_dragon_tiger` / `get_north_bound_flow` ⇒ market-synthesizer 是唯一落伍者。
//
// 危害面：md 是 LLM **实际读到的 prompt 文本** —— `EMBEDDED_PROMPTS` 经
//   `seed_agency_experts`（无条件 UPSERT，见同文件 `mod.rs`）写进
//   `agency_experts.system_prompt`，随后作为专家提示词注入。幽灵名会让模型去调用
//   一个不存在的工具，而解析失败路径**不抛错**（与 PROFILE_TOOLS 族同形），
//   表现为「模型声称查过了、实际没数据」。
//
// 三条消费链的判据空间**共用** `full_resolvable_tool_names()`，不另建副本：
//   链① seed 文件里的 `ToolDef.name`          → `seed_*_tools_all_resolvable`
//   链② `PROFILE_TOOLS` → recommended_tools   → `profile_tools_all_resolvable`
//   链③ **md 自身**（正文反引号 + frontmatter data_sources）→ 本测试
//
// ⚠ 扫描面**必须按目录分域**：只扫 `stock-analysis/` 与 `skills/`。
//   `opc/**` 的 md 走**另一套工具空间**（`OpcGetXxx` / `Bash` / `WebSearch` 等，
//   由 OPC 自己的注册表提供）。实测把 `agency_experts/**` 全部 173 个 md 混扫会
//   union 出 190 个名字、其中 184 个属 OPC 域，全是假阳性。
//   口径 `^[a-z0-9_]+$` 天然排除 PascalCase，与分域构成双保险。
//
// ⚠ 覆盖边界（勿误读为「全覆盖」）：口径是**工具动词前缀**
//   （get_/compute_/check_/trace_/search_/list_/fetch_/query_/scan_/upsert_）+
//   全小写 snake_case。因此不带这些前缀的工具名（如 `market_mainline_batch_upsert`）
//   若变成幽灵，**本测试抓不到**。放宽前缀会立刻引入大量假阳性（字段名 / 变量名），
//   故此处保持窄口径，边界显式登记于此。
// ─────────────────────────────────────────────────────────────────────────────

/// md 中**刻意点名但不可调用**的名字 —— 全部经源码级实测确认为「非工具」。
///
/// 准入判据：该处提及的语义必须是「警告模型不要调用」或「描述系统内部机制」，
/// **不能**是「让它去调用」。若某条日后被真的实现成工具，本测试会因
/// 「豁免项竟在解析空间」而失败 —— 那是预期信号：改用真实名，别放宽断言。
const PROMPT_NON_TOOL_MENTIONS: &[(&str, &str)] = &[
    ("get_market_regime", "fundamentals-analyst.md —— 显式声明「未实现，不要尝试调用」"),
    (
        "get_announcement_content",
        "catalyst-analyst.md / reflection.md —— 显式声明「未实现，不要尝试调用」",
    ),
    (
        "compute_decision_agreement",
        "trader.md —— Rust 内部函数（stock_workflow/decision.rs），描述系统做双视角对比，非可调用工具",
    ),
];

/// md 工具名口径：全小写 snake_case 且带工具动词前缀。
fn md_tool_prefix(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "get_", "compute_", "check_", "trace_", "search_", "list_", "fetch_", "query_", "scan_",
        "upsert_",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
        && name.len() >= 5
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 提取一行里反引号包裹的工具名（容忍 `` `xxx()` `` 形态）。
fn extract_backticked_tool_names(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else { break };
        let raw = &after[..end];
        let name = raw.strip_suffix("()").unwrap_or(raw);
        if md_tool_prefix(name) {
            out.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

/// 提取 frontmatter `data_sources: [a, b, c]` 里的工具名。
///
/// 形态限定为**单行 inline 列表**（本仓 173 个 md 实测皆如此）；
/// 多行 YAML 序列形态（`data_sources:\n  - a`）不在覆盖内。
fn extract_data_sources_tool_names(line: &str) -> Vec<String> {
    let Some(rest) = line.trim().strip_prefix("data_sources:") else { return Vec::new() };
    let inner = rest.trim().trim_start_matches('[').trim_end_matches(']');
    inner.split(',').map(str::trim).filter(|s| md_tool_prefix(s)).map(String::from).collect()
}

/// 递归收集目录下的 `.md`。
fn collect_md_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_md_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

#[test]
fn md_prompt_tool_refs_are_resolvable() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("agency_experts");

    // 分域扫描：stock 域只有这两个子目录（opc/** 属另一套工具空间，见文件头说明）
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for sub in ["stock-analysis", "skills"] {
        collect_md_files(&root.join(sub), &mut files);
    }
    // 自证①：扫描面必须真的装到东西（目录改名 / 迁移会让本测试静默失效）
    assert!(
        files.len() >= 40,
        "md 扫描面异常：只在 stock-analysis/ + skills/ 找到 {} 个 .md —— \
         若目录结构变了，先修本测试的路径，不要放宽断言。",
        files.len()
    );

    let resolvable = full_resolvable_tool_names();
    assert!(
        resolvable.len() >= 50,
        "解析空间异常：仅 {} 个工具名，先修扫描器/空间构造。",
        resolvable.len()
    );

    let exempt: HashSet<&str> = PROMPT_NON_TOOL_MENTIONS.iter().map(|(n, _)| *n).collect();

    let mut total = 0usize;
    let mut unique: HashSet<String> = HashSet::new();
    let mut missing: Vec<String> = Vec::new();
    let mut seen_exempt: HashSet<String> = HashSet::new();

    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        let rel = f
            .strip_prefix(&root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| f.display().to_string());
        for (i, line) in src.lines().enumerate() {
            let names: Vec<String> = extract_backticked_tool_names(line)
                .into_iter()
                .chain(extract_data_sources_tool_names(line))
                .collect();
            for n in names {
                total += 1;
                unique.insert(n.clone());
                if exempt.contains(n.as_str()) {
                    seen_exempt.insert(n);
                    continue;
                }
                if !resolvable.contains(&n) {
                    missing.push(format!("{rel}:{} → {n}", i + 1));
                }
            }
        }
    }

    // 自证②：扫描面非零 —— `0 命中 ≠ 没问题`（判据 #7）。
    assert!(
        total >= 30 && unique.len() >= 20,
        "扫描面异常：只提取到 {total} 个名次 / {} 个唯一名（预期远超此数）—— \
         先怀疑提取口径失配（例如反引号写法变了），不要放宽本断言。",
        unique.len()
    );

    assert!(
        missing.is_empty(),
        "md 提示词里出现了 {} 个运行时解析不到的工具名（共扫 {total} 个名次 / {} 个唯一名）：\n  {}\n\n\
         这些名字会写进 `agency_experts.system_prompt` 并被 LLM 读到 ⇒ 模型会去调用不存在的工具，\n\
         而解析失败是**静默**的（节点仍 completed、结果为空）。\n\
         修复：改用真实工具名（先确认它在 `full_resolvable_tool_names()` 里）；\n\
         确属「警告不要调用」的负向提及，才登记进 `PROMPT_NON_TOOL_MENTIONS` 并写明理由。",
        missing.len(),
        unique.len(),
        missing.join("\n  ")
    );

    // 反向自证：豁免项必须**真的在 md 里出现**，否则是腐烂豁免
    //（名字改了/段落删了却留着表项 ⇒ 下一轮有人以为「这条已处理」）。
    for (n, why) in PROMPT_NON_TOOL_MENTIONS {
        assert!(
            seen_exempt.contains(*n),
            "豁免项 `{n}`（{why}）已不在任何 stock 域 md 中出现 ⇒ 应从 PROMPT_NON_TOOL_MENTIONS 删除。"
        );
        assert!(
            !resolvable.contains(*n),
            "豁免项 `{n}` 竟已存在于解析空间 ⇒ 它已经是真工具，请改用真实名并删除本豁免。"
        );
    }
}

/// 提取器的**负控**：证明它能分辨「工具名 / 非工具内容 / 另一套工具空间」。
///
/// 没有这条，`md_prompt_tool_refs_are_resolvable` 只能证明「当前没违规」，
/// 无法证明「提取器不是瞎的」—— 而本项目的实战教训恰恰是扫描器静默失明
/// （`source: "x".into()` 漏写 `.into()` 时 6 个块全部跳过却报「0 违规」）。
#[test]
fn md_tool_ref_extractor_discriminates() {
    // ① 正文反引号：历史幽灵名必须被提出（这正是 2026-09-19 漏掉的那个）
    assert_eq!(
        extract_backticked_tool_names("- `get_dragon_tiger_list` — 龙虎榜数据"),
        vec!["get_dragon_tiger_list"]
    );
    // ② `xxx()` 形态（trend-scanner.md 的写法）
    assert_eq!(
        extract_backticked_tool_names("- `get_north_bound_flow()` 返回北向资金流向"),
        vec!["get_north_bound_flow"]
    );
    // ③ 非工具内容不得被提出：模板变量、无前缀字段名
    assert!(
        extract_backticked_tool_names("- `{{market_regime}}` 与 `kline_json` 变量").is_empty(),
        "非工具内容被误提取"
    );
    // ④ OPC 域的 PascalCase 不得被提出（跨域双保险）
    assert!(
        extract_backticked_tool_names("- `OpcGetCustomerProfile` 与 `WebSearch`").is_empty(),
        "OPC 域工具名被误提取（分域失效）"
    );
    // ⑤ frontmatter inline 列表
    assert_eq!(
        extract_data_sources_tool_names("data_sources: [get_stock_kline, get_north_bound_flow]"),
        vec!["get_stock_kline", "get_north_bound_flow"]
    );
    // ⑥ frontmatter 里的跨域名同样不得被提出
    assert!(
        extract_data_sources_tool_names("data_sources: [OpcGetCustomer, Bash, FileRead]")
            .is_empty(),
        "frontmatter 跨域名被误提取"
    );
    // ⑦ 端到端方向确认：幽灵名不可解析、替换名可解析
    let resolvable = full_resolvable_tool_names();
    for ghost in ["get_dragon_tiger_list", "get_north_flow"] {
        assert!(!resolvable.contains(ghost), "负控失败：`{ghost}` 竟在解析空间");
    }
    for ok in ["get_market_dragon_tiger", "get_north_bound_flow"] {
        assert!(resolvable.contains(ok), "正向对照失败：`{ok}` 不在解析空间");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 辩论轮数必须参数化且同源 —— 防「建图轮数与变量值脱钩」回归
//
// 背景（2026-09-21，本轮改动）：此前 `debate_max_rounds` 被 hardcode 为 `1`，
// 导致多空辩论永远只跑第一轮（假多轮）。本次改为从 DB 变量 `debate_rounds`
// 经 `resolve_debate_rounds` 求值，DAG 展开几对 bull/bear、下游锚点
// `bear-r{debate_max_rounds}`、落库变量三处同源。
//
// 本组测试用两条腿锁死该约定：
//   ① 纯函数行为：`resolve_debate_rounds` 的取值 / 夹紧 / 回退（可直接单测，无需 DB）；
//   ② 源码级不变式：`seed_stock_analysis.rs` 里展开逻辑必须与它同源，
//      谁再打破就红在 CI。
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn resolve_debate_rounds_clamps_and_defaults() {
    use super::resolve_debate_rounds;
    use super::seed_variables::DEFAULT_DEBATE_ROUNDS;

    // 缺省：无旧变量 / 空串 / 变量表里没有 debate_rounds ⇒ 回退默认
    assert_eq!(resolve_debate_rounds(None), DEFAULT_DEBATE_ROUNDS as usize);
    assert_eq!(resolve_debate_rounds(Some("")), DEFAULT_DEBATE_ROUNDS as usize);
    assert_eq!(
        resolve_debate_rounds(Some(r#"[{"name":"other","value":5}]"#)),
        DEFAULT_DEBATE_ROUNDS as usize
    );

    // 命中合法值 ⇒ 直接采用（不再被强制成 1）
    assert_eq!(resolve_debate_rounds(Some(r#"[{"name":"debate_rounds","value":3}]"#)), 3);

    // 防御性夹紧：0 → 1、39 → 10（上层把 usize 当节点循环上界，防极端值撑爆 DAG）
    assert_eq!(resolve_debate_rounds(Some(r#"[{"name":"debate_rounds","value":0}]"#)), 1);
    assert_eq!(resolve_debate_rounds(Some(r#"[{"name":"debate_rounds","value":39}]"#)), 10);

    // 坏 JSON / 值非数字 ⇒ 回退默认（不 panic）
    assert_eq!(resolve_debate_rounds(Some("not-json")), DEFAULT_DEBATE_ROUNDS as usize);
    assert_eq!(
        resolve_debate_rounds(Some(r#"[{"name":"debate_rounds","value":"x"}]"#)),
        DEFAULT_DEBATE_ROUNDS as usize
    );
}

/// 源码级不变式：seed 展开逻辑必须与 `resolve_debate_rounds` 同源。
///
/// 逐条约束（与 `scan_round_literal_edge_endpoints` 同款「先证明扫描器看过了」风格）：
///   ① 展开轮数 `debate_max_rounds` 必须由 `resolve_debate_rounds(...)` 派生，
///      不得为字面量（否则辩论永远只跑固定轮数 / 与面板变量脱钩）；
///   ② `debater_steps` 必须按 `(0..debate_max_rounds).flat_map(...)` 生成 2N 个
///      独立辩手节点（写死数组 ⇒ 轮数一变即与展开的 bull/bear 对不上）；
///   ③ 容器的 `max_rounds` 必须保持固定 `1` —— 轮次由独立节点边承担；
///      若被改成 `debate_max_rounds`，引擎会对整批 debater_steps 做 N 遍轮播，
///      第 2+ 遍被「Completed 复用」短路 → 假多轮浪费（正是本次修复的病根）；
///   ④ 落库变量 `debate_rounds` 必须复用 `debate_max_rounds`（同源），
///      不得独立写成字面量（防悬空 / 防面板显示与实际展开不一致）。
#[test]
fn debate_expansion_is_parameterized_to_max_rounds() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/stock_analysis_setup")
        .join("seed_stock_analysis.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 seed_stock_analysis.rs 失败: {e}"));

    assert!(
        src.lines().any(|l| l.contains("let debate_max_rounds: usize = resolve_debate_rounds(")),
        "展开轮数 `debate_max_rounds` 必须由 `resolve_debate_rounds(...)` 从变量求值，\n\
         不得写死字面量（否则辩论永远只跑固定轮数 / 与面板变量脱钩）。"
    );

    assert!(
        src.lines().any(|l| l.contains("debater_steps: (0..debate_max_rounds)")),
        "debater_steps 必须由 `(0..debate_max_rounds).flat_map(...)` 生成 2N 个辩手；\n\
         若改为写死数组，轮数一变即与展开的 bull-rN/bear-rN 节点对不上。"
    );

    assert!(
        src.lines().any(|l| l.trim() == "max_rounds: 1,"),
        "辩论容器的 `max_rounds` 必须保持 `1`。\n\
         轮次由独立节点边（bull-rN→bear-rN→bull-r(N+1)）承担；\n\
         若被改成 debate_max_rounds，引擎会对整批 debater_steps 反复轮播，\n\
         第 2+ 遍被 Completed 复用短路 → 假多轮（正是本次修复的病根）。"
    );

    assert!(
        src.contains("serde_json::json!(debate_max_rounds)"),
        "落库的 `debate_rounds` 变量必须复用 `debate_max_rounds`（与展开同源），\n\
         不得独立写成某个字面量（防悬空边 / 防面板显示与实际展开不一致）。"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 两层 prompt 的「同一信号 ⇒ 同一处置」契约
//
// 背景（2026-09-21，真实缺陷，本轮由我引入后自查发现）：
//   DCF 可用性判据被写在**两层** prompt 里，运行时**拼接后一起注入**，LLM 同时读到：
//     层① 专家 prompt：`agency_experts/stock-analysis/custom/value-investor.md`
//         → `mod.rs` 的 `EMBEDDED_PROMPTS` 经 `include_str!` → `seed_agency_experts`
//         （**无条件 UPSERT**，每次跑工作流前由 `ensure_stock_analysis_experts_seeded` 调用）
//     层② 节点任务指令：`seed_stock_analysis.rs` 里 value-investor 的 inline
//         `system_prompt`（经 `TEMPLATE_VERSION` 门写进 `workflow_templates.nodes`）
//
//   本轮我给层① 加了「`is_fallback_anchor != true` 才算可用（否则填 null）」，
//   给层② 加了「锚定口径披露：数值**可以引用**」⇒ **同一信号被两层给出相反处置**。
//
//   ⚠ 为什么「集合级一致性」抓不到：两层**都提到**了 `is_fallback_anchor`，
//     所以任何「两边提到的标识符集合相等」的断言都会全绿。必须断言的是
//     **信号与其处置分档词的共现位置** —— 该信号只能落在「软衰减」段，不得落在
//     「硬不可用」段。
//
//   ⚠ 权威口径在代码里：`mcp_tools.rs` 的 `applicable` 计算处明文写着
//     「当期 FCF 数据缺失：**不**判不适用（缺数据 ≠ 模型不成立），由 fallback 锚定
//      + `is_fallback_anchor` 承担置信度衰减」⇒ 代理锚 = **软衰减**，
//     `applicable == false` = **硬不可用**。本测试即把这条口径钉进门禁。
// ─────────────────────────────────────────────────────────────────────────────

/// 硬不可用段的起始标记（命中 ⇒ 字段填 `null` / 写「无算法估值锚」）。
///
/// ⚠ **必须是带 `【】` 的唯一标记**，不能用裸词 `硬不可用` —— 两层 prompt 的**字段说明行**
/// 也会提到这两个词（如「DCF **硬不可用**时填 null；**软衰减**时照常填」），
/// 用裸词 `find()` 会把分段点切到那一行，导致硬段退化成一行、软段吞掉整篇。
/// 实测（2026-09-21）：正是这个分档点错位让断言在**真文件上假红**（硬段里找不到 `applicable`）。
const DCF_HARD_MARKER: &str = "【硬不可用】";
/// 软衰减段的起始标记（命中 ⇒ 照常填，但须披露口径 + 下调 confidence）。
const DCF_SOFT_MARKER: &str = "【软衰减】";

/// 从一段文本里切出【硬不可用】段与【软衰减】段。
///
/// 契约：两段标记**各自恰好出现一次**，且**硬在前、软在后**（两层 prompt 的既有段序）。
/// 段序 / 出现次数变了就要同步改本函数并重跑 —— 届时两侧文案都已改，不是假红。
fn split_dcf_availability_blocks(label: &str, src: &str) -> (String, String) {
    for (marker, what) in [(DCF_HARD_MARKER, "硬不可用"), (DCF_SOFT_MARKER, "软衰减")] {
        let n = src.matches(marker).count();
        assert_eq!(
            n, 1,
            "{}: 分段标记「{}」出现 {} 次（要求恰好 1 次）。\n\
             多于 1 次 ⇒ 分段点不确定（`find` 取首个），本测试会静默切成错误的段；\n\
             0 次 ⇒ 该层漏了「{}」分档。\n\
             两层的分档标记是**公共契约**：改名 / 新增提及时须两层同改。",
            label, marker, n, what
        );
    }
    let hard_at = src.find(DCF_HARD_MARKER).expect("已断言存在");
    let soft_at = src.find(DCF_SOFT_MARKER).expect("已断言存在");
    assert!(
        hard_at < soft_at,
        "{}: 段序异常（{}@{} 应在 {}@{} 之前）。\n\
         本测试用「hard 段 = hard_at..soft_at」切分，段序反过来会把两段内容互相错位。",
        label,
        DCF_HARD_MARKER,
        hard_at,
        DCF_SOFT_MARKER,
        soft_at
    );
    (src[hard_at..soft_at].to_string(), src[soft_at..].to_string())
}

#[test]
fn dcf_availability_signal_layers_agree() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // 层① 专家 prompt
    let md_path = manifest.join("agency_experts/stock-analysis/custom/value-investor.md");
    let md = std::fs::read_to_string(&md_path)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", md_path.display()));

    // 层② 节点任务指令：从 seed 源码里切出 value-investor 这一块。
    // 用**结构锚点**（`let vi_id = ...` → `nodes.push(vi);`）而非行号 —— 行号会漂移。
    let seed_path = manifest.join("src/commands/stock_analysis_setup/seed_stock_analysis.rs");
    let seed_src = std::fs::read_to_string(&seed_path)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", seed_path.display()));
    // v133（B2-2）：value-investor 改为逐档循环生成 ⇒ 起点锚改为**循环头**
    // （原 `let vi_id = …` 已进入循环体、不再唯一；循环头与切片语义一致：
    //   层② = 该节点的任务指令正文）。
    let vi_start = seed_src
        .find("for (base, p, title, _expert) in tiered.iter().filter(|(b, ..)| *b == VALUE_INVESTOR_ID)")
        .expect(
            "在 seed_stock_analysis.rs 里找不到 value-investor 的逐档生成循环头 —— \
             该锚点是本测试的切片起点，节点构造写法变了须同步改",
        );
    let vi_end = seed_src[vi_start..]
        .find("nodes.push(vi);")
        .map(|i| vi_start + i)
        .expect("value-investor 块尾锚点 `nodes.push(vi);` 找不到");
    let seed_vi = &seed_src[vi_start..vi_end];

    // 自证①：两个切片的体量必须合理（切片失败会得到空串而断言"看不见东西"全绿）
    assert!(
        md.len() >= 3000,
        "层① 专家 prompt 只有 {} 字节 —— 明显不是完整 md，先修读取路径",
        md.len()
    );
    assert!(
        seed_vi.len() >= 1500,
        "层② 节点任务指令切片只有 {} 字节 —— 切片锚点可能失配",
        seed_vi.len()
    );

    let (md_hard, md_soft) = split_dcf_availability_blocks("value-investor.md", &md);
    let (seed_hard, seed_soft) =
        split_dcf_availability_blocks("seed_stock_analysis.rs(vi)", seed_vi);

    // 自证②：分档词必须真的**出现在正文里**（不是只在注释里被提到一次）
    for (label, hard, soft) in [
        ("value-investor.md", &md_hard, &md_soft),
        ("seed_stock_analysis.rs(vi)", &seed_hard, &seed_soft),
    ] {
        assert!(
            hard.contains("null") && soft.contains("confidence"),
            "{label}: 分档段内容异常 —— 硬段应含 `null` 处置、软段应含 `confidence` 处置。\n\
             实际硬段 {} 字节 / 软段 {} 字节。先确认分档文案没有被换成别的措辞。",
            hard.len(),
            soft.len()
        );
    }

    // ── 正例 + 反例（同时存在，证明本测试有区分力）────────────────────────
    //
    // 软衰减信号：代理锚。基于「缺数据/代理锚 ≠ 模型不成立」的权威口径，
    // 它**只能**落在软段。若有人在某一层把它挪进硬段（两边处置相反），本断言报红。
    for (label, hard, soft) in [
        ("value-investor.md", &md_hard, &md_soft),
        ("seed_stock_analysis.rs(vi)", &seed_hard, &seed_soft),
    ] {
        assert!(
            !hard.contains("is_fallback_anchor"),
            "{}: `is_fallback_anchor` 出现在{}段内 ⇒ \
             该层要求「代理锚 ⇒ 填 null / 写无算法估值锚」。\n\
             但 `mcp_tools.rs` 的 `applicable` 计算处明确规定「缺数据/代理锚 ≠ 模型不成立」，\n\
             由 fallback 锚定 + `is_fallback_anchor` 承担**置信度衰减**（软处置）。\n\
             两层给出相反处置时，LLM 读到哪层先就照哪层办 —— 这正是 2026-09-21 的实际缺陷。",
            label,
            DCF_HARD_MARKER
        );
        assert!(
            soft.contains("is_fallback_anchor"),
            "{}: `is_fallback_anchor` 未出现在{}段内 ⇒ 该层漏了代理锚的处置。\n\
             `0 命中 ≠ 没问题`（判据 #7）：先看是不是措辞改了（改用别的标识符命名就同步改本断言），\n\
             不要直接放宽。",
            label,
            DCF_SOFT_MARKER
        );
        // 反向对照：`applicable` 是**硬**信号，必须落在硬段。
        // 这条同时证明上面的「禁止」不是「硬段里什么都看不见」的假绿。
        assert!(
            hard.contains("applicable"),
            "{}: `applicable` 未出现在{}段内 ⇒ 该层漏了硬处置。",
            label,
            DCF_HARD_MARKER
        );
        assert!(
            !soft.contains("applicable == false") && !soft.contains("applicable=false"),
            "{}: `applicable == false` 被写进{}段 ⇒ 硬信号被降级成软处置。",
            label,
            DCF_SOFT_MARKER
        );
    }
}

// ── 2026-09-21: 护城河档位的跨语言等式门禁 ──────────────────────────────────
//
// 背景：`portfolio-mgr.rhai` 的 `moat_mult` 曾比对 `"宽护城河" / "wide" /
// "窄护城河" / "narrow"`，而生产端 `compute_moat_score` 输出的是
// `"宽阔" / "狭窄" / "无"` —— **三个值全部 miss**，乘子恒 1.0，
// 是一条从未生效过的死分支，且**零测试发现**（38 条 DB 样本按旧比对值命中 0/21）。
//
// 根因不是笔误，而是**数据源认错**：`input_mapping` 把 `valuation_moat` 接到
// 算法字段 `moat.label`，而作者以为接的是 LLM 的 `verdict.moat_rating`
// （那确实写「宽护城河」—— LLM 把算法的「宽阔」转写成了行业惯用语）。
// 两套词汇表共存 ⇒ 只对一侧改名不会有任何报错。

/// 生产端档位（`compute_moat_score`）在此声明为**唯一权威**。
///
/// 消费端在 Rhai，而 Rhai 无法引用 Rust 常量 ⇒ 物理上必然手抄第二份。
/// 本门禁不追求「消除重复」，而是**钉住两侧相等**：任何一侧改名而漏改另一侧，
/// 或有人把已废弃的 LLM 词汇表写回 Rhai ⇒ 立即变红。
const MOAT_LEVEL_VOCABULARY: [&str; 3] = ["宽阔", "狭窄", "无"];

/// 切出 `from` 之后、`to` 之前的片段（不含 `to`）。
fn slice_between<'a>(label: &str, src: &'a str, from: &str, to: &str) -> &'a str {
    let a = src.find(from).unwrap_or_else(|| {
        panic!("{label}: 找不到起点标记 `{from}` —— 生产/消费端的结构变了，须同步本测试")
    });
    let b = src[a + from.len()..]
        .find(to)
        .map(|i| a + from.len() + i)
        .unwrap_or_else(|| panic!("{label}: 找不到终点标记 `{to}` —— 同上"));
    &src[a..b]
}

/// 提取片段里所有 `"…"` 字面量的内容。
///
/// 依赖「字符串内无转义引号」—— 本项目取值域是中文字/短词，不涉及转义。
/// 若将来取值域出现转义，本函数会静默切错 ⇒ 用 `assert!` 拦住可疑形态。
fn quoted_strings(s: &str) -> Vec<String> {
    assert!(
        !s.contains("\\\""),
        "片段含转义引号，quoted_strings 的假设不成立，须改用真正的词法扫描"
    );
    s.split('"').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, v)| v.to_string()).collect()
}

/// 提取片段里的**非零**浮点字面量（形如 `0.56`），按出现顺序返回。
///
/// `0.0`（`present()` 兜底用的 `else { 0.0 }` 占位）被过滤 —— 它不是权重本身。
///
/// ⚠️ 本函数只认 `数字.数字` / `数字.` 形态，**不认**科学计数法（`5.6e-1`）与整数。
/// 若将来权重改成这两类写法，本函数会**静默漏抓** ⇒ 调用处必须用数量断言
/// （`assert_eq!(w.len(), 3)`）把「漏抓」变成「报红」，而不是让它退化成少比几项。
fn f64_literals_in(s: &str) -> Vec<f64> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'_') {
            j += 1;
        }
        if j < b.len() && b[j] == b'.' {
            let mut k = j + 1;
            while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'_') {
                k += 1;
            }
            if k > j + 1 {
                let text = s[start..k].replace('_', "");
                if let Ok(v) = text.parse::<f64>() {
                    if v != 0.0 {
                        out.push(v);
                    }
                }
                i = k;
                continue;
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// 抽取「**档位比较**」用到的字符串字面量：仅保留 `<标识符> == "..."` 形态。
///
/// ## 为什么不能直接复用 `quoted_strings`
///
/// `moat_mult` 段里除了三档比较，还有一句守卫：
/// `type_of(valuation_moat) == "string"` —— 那个 `"string"` 是**类型名**，
/// 不是档位词，且 `==` 左侧是**函数调用**（以 `)` 结尾）。
/// 若一并抓取，断言会拿 `["string", "宽阔", "狭窄"]` 去比对档位表而报假红
/// （2026-09-21 实测）。此处按**语法形态**把类型判定排除在外：
/// `==` 左侧以 `)` / `]` 结尾者一律不视为档位比较（函数返回值、索引结果）。
///
/// `!=` / `>=` / `<=` 天然不会被 `strip_suffix("==")` 命中，无需额外分支。
fn comparison_strings(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < b.len() && b[j] != b'"' {
            j += 1;
        }
        assert!(j < b.len(), "字面量未闭合，comparison_strings 的假设不成立：{s:.120}");
        if let Some(head) = s[..i].trim_end().strip_suffix("==") {
            let left = head.trim_end();
            let is_cmp = !left.ends_with(')')
                && !left.ends_with(']')
                && !left.ends_with('=')
                && !left.ends_with('!');
            if is_cmp {
                out.push(s[start..j].to_string());
            }
        }
        i = j + 1;
    }
    out
}

/// 断言某条生产编译路径**消费**共享沙箱档位，而不是自己内联 `set_max_*`。
///
/// ## 为什么不再读 `set_max_expr_depths(a, b)` 的字面量（2026-09-22 改）
///
/// 原实现要求 `stock_analysis.rs` 与 `decision.rs` **各自内联**一份
/// `set_max_expr_depths(256, 256)`，再由本测试断言两份相等 —— 那是
/// 「两处手抄，测试当第三只眼」。
///
/// 本轮已把它收敛为**单一来源**（`rhai_registry::RhaiSandboxLimits::PORTFOLIO`
/// 与 `rhai_registry::build_stock_rhai_engine`）：两条路径都只是**消费方**，
/// 内联的那两行已不存在。继续从源码里 parse 数字，等于在给一个已消除的形态招魂
/// —— 症状是找不到 marker 而 panic，看起来像「生产缺配置」，实为**守护过时**。
///
/// 现在的守护对象是**形态**：两条路径都必须经由那个唯一入口，且不得重新内联
/// `set_max_expr_depths`（一处内联即一处收紧、另一处不动 ⇒ 同型缺陷复发）。
/// **数字本身不再断言** —— `portfolio_mgr_rhai_compiles` 直接调那个生产工厂建引擎，
/// 「测试与生产同配置」由**是同一个函数**保证，强于读源码比对。
fn assert_consumes_shared_sandbox(rel: &str, label: &str) {
    let src = read_ws_file(rel);
    assert!(
        src.contains("build_stock_rhai_engine(") && src.contains("RhaiSandboxLimits::PORTFOLIO"),
        "{label}（{rel}）未走 `rhai_registry::build_stock_rhai_engine` +\n\
         `RhaiSandboxLimits::PORTFOLIO`。本仓把「脚本需要多少深度上限」收敛为唯一来源；\n\
         此处若自建 Engine，两条编译路径的档位就重新各说各话，收紧的那条会在用户点按钮时\n\
         抛 `Expression exceeds maximum complexity`（编译期错误，脚本内 try/catch 捕获不到）。"
    );
    assert!(
        !src.contains("set_max_expr_depths("),
        "{label}（{rel}）重新内联了 `set_max_expr_depths` ⇒ 档位又变成手抄多副本。\n\
         请改回消费 `RhaiSandboxLimits::PORTFOLIO`；要新档位就在 `RhaiSandboxLimits` 里\n\
         加具名常量，不要在调用点写死数字。"
    );
}

fn read_ws_file(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()))
}

#[test]
fn moat_level_vocabulary_matches_rhai_consumer() {
    // ── ① 生产端档位 == 本地权威声明 ──
    // 2026-10-09 B 批：门限从写死的 `if score >= 70` 改成 [`MoatTiers`]（面板
    // `value_moat_threshold` 派生），**词表三个值一字未动** ⇒ 本切片改锚在纯函数
    // `moat_level_of` 上（它是词表的唯一产地），不再依赖某个具体门限的数字形态。
    let mcp = read_ws_file("crates/astock-data/src/mcp_tools.rs");
    let producer_slice = slice_between(
        "mcp_tools.rs",
        &mcp,
        "fn moat_level_of(score: u32, tiers: &MoatTiers)",
        "/// 护城河量化评分",
    );
    let producer = quoted_strings(producer_slice);
    assert_eq!(
        producer,
        MOAT_LEVEL_VOCABULARY.to_vec(),
        "生产端 `compute_moat_score` 的档位与本地声明的权威取值域不一致。\n\
         若这是有意的改档 ⇒ 请**同时**更新三处：本常量、Rhai 比对值、生产端源码；\n\
         只改生产端会让 Rhai 静默走 else 分支（乘子退化为 1.0，无任何报错）。"
    );
    // 词表与门限**分离**的结构锁：档位判据必须读 `MoatTiers`，不许把数字抄回 `if` 里
    // （抄回去就又变成「面板一道、判据一道」两处权威，接线静默失效）。
    assert!(
        producer_slice.contains("tiers.wide") && producer_slice.contains("tiers.narrow"),
        "mcp_tools.rs: `moat_level_of` 不再读 `MoatTiers` 的两道门 ⇒ 门限被写回了字面量，\n\
         面板 `value_moat_threshold` 又变成空接线（定向门 check-panel-var-landing.mjs 同批会红）。"
    );

    // ── ② 消费端比对值 == 生产端档位的前缀 ──
    // 抽取器选择：① 生产端是 `if score >= 70 { "宽阔" }` 的**分支字面量**形态 ⇒ quoted_strings；
    //             ② 消费端是 `x == "宽阔"` 的**比较**形态 ⇒ comparison_strings（须排除类型名）。
    let rhai = read_ws_file("src/commands/portfolio-mgr.rhai");
    let consumer_slice = slice_between(
        "portfolio-mgr.rhai",
        &rhai,
        "let moat_mult = if present(valuation_moat)",
        "} else { 1.0 };",
    );
    let consumer = comparison_strings(consumer_slice);
    assert!(
        consumer.len() >= 2,
        "Rhai 的 `moat_mult` 至少应处理「宽阔」「狭窄」两档，实际只提到 {consumer:?}"
    );
    assert_eq!(
        consumer,
        producer[..consumer.len()].to_vec(),
        "Rhai 的 moat 比对值必须与生产端逐档一致（顺序敏感）。\n\
         期望前缀 {:?}，实际 {:?}。\n\
         注意：末尾档位（「无」）可由 `else` 兜底，故只要求前缀 —— 但它**不能**被换成别的词。",
        &producer[..consumer.len()],
        consumer
    );

    // ── ③ 防回流：已废弃的 LLM 侧词汇表不得写回消费端 ──
    for stale in ["宽护城河", "窄护城河", "wide", "narrow"] {
        assert!(
            !consumer_slice.contains(stale),
            "Rhai 的 `moat_mult` 段出现了 `{stale}` —— 那是 **LLM 侧**\n\
             `value-investor.verdict.moat_rating` 的词汇表，不是本变量接入的算法字段。\n\
             写回它会让乘子再次恒为 1.0（2026-09-21 修复的正是这个死分支）。"
        );
    }

    // ── ④ 数据源钉住：必须接算法字段，不得接 LLM 自由文本 ──
    let seed = read_ws_file("src/commands/stock_analysis_setup/seed_stock_analysis.rs");
    assert!(
        seed.contains(r#"("valuation_moat", "t-valuation.result.content.moat.label")"#),
        "`valuation_moat` 的注入源必须是算法字段 `t-valuation.result.content.moat.label`。\n\
         若改成 LLM 的 `verdict.moat_rating`，则取值域换成自由文本 ⇒ 本门禁 ①② 全部失效，\n\
         必须同时重写它们（自由文本不可枚举）。"
    );
}

/// f5 估值融合的**权重结构不变量**（V79 三腿改造的守护）。
///
/// ## 为什么需要它
///
/// V79 把 f5 权重从硬编码的 `0.7 / 0.3` 改成 `0.56 / 0.24 / 0.20`（新增 PE 分位腿），
/// 其**向后兼容性完全依赖**一条恒等式：
/// **band 腿缺位时归一化结果必须逐位回到 `0.7 / 0.3`** —— 归一化除的是
/// 「**可用腿**权重之和」，故 `0.56 / 0.8 = 0.7`、`0.24 / 0.8 = 0.3`。
///
/// 这条恒等式原本只写在注释里 ⇒ 谁把它改成 `0.7 / 0.3 / 0.2`（三者和 1.2）都能编译、
/// 能跑、注释也不会报错，但 band 腿份额会从 0.20 悄悄变成 0.167，
/// 且「band 缺位 ⇒ 逐位等于旧值」这条**可自证性**随之消失。
///
/// | # | 断言 | 为什么 |
/// |---|---|---|
/// | ① | 恰有 3 个非零权重字面量 | 漏抓/多抓都要报红，不允许静默少比 |
/// | ② | 三者之和 == 1.0 | ⇒ band 腿份额恒为 0.20（不随缺位状态浮动） |
/// | ③ | `w_dcf / (w_dcf + w_graham) == 0.7` | ⇒ band 缺位时 dcf 份额回到 0.7 |
/// | ④ | 最终 `clamp(...)` 里**不含** `dcf_anchor_decay` | 防「分腿衰减」被二次施加 |
///
/// ⚠️ 本测试只钉**结构**，不验语义。数值效果由 `output/tmp-v79-replay.mjs` 的
/// A1/A2/A3 段负责（38 条生产样本逐位对账）—— 两者缺一不可：
/// 本测试能拦住「权重写歪」，但拦不住「衰减位置放错」。
#[test]
fn valuation_leg_weights_degrade_to_legacy() {
    let rhai = read_ws_file("src/commands/portfolio-mgr.rhai");

    // 切片终点取 `let dcf_anchor_decay` —— 它紧跟 fusion_wsum 之后。
    // ⚠️ 若终点写成 `let fused_sig =`，会把 `dcf_anchor_decay` 的 0.5 / 1.0 一起圈进来
    // ⇒ 5 个非零字面量 ⇒ 数量断言假红（首版实测）。
    let slice =
        slice_between("portfolio-mgr.rhai", &rhai, "let fusion_wsum =", "let dcf_anchor_decay");
    let w = f64_literals_in(slice);
    assert_eq!(
        w.len(),
        3,
        "融合权重应恰有 3 个非零字面量（顺序 = 源码顺序 = dcf / graham / band），实得 {w:?}。\n\
         少于 3 ⇒ 权重被合并、或写成了整数/科学计数法（`f64_literals_in` 会漏抓）；\n\
         多于 3 ⇒ 切片范围被改宽，把别的常量圈进来了。"
    );
    let (w_dcf, w_graham, w_band) = (w[0], w[1], w[2]);

    let sum = w_dcf + w_graham + w_band;
    assert!(
        (sum - 1.0).abs() < 1e-12,
        "三腿权重之和应为 1.0（即 band 腿份额恒为 {w_band}），实得 {sum:.6}。\n\
         若改成 0.7 / 0.3 / 0.2（和 1.2）⇒ band 份额变成 0.167 而不再是 0.20 —— 那是有意的语义变更，\n\
         须**同时**更新本断言与 `output/tmp-v79-replay.mjs` 的 A1 段，不能只改数值。"
    );

    let dcf_share_without_band = w_dcf / (w_dcf + w_graham);
    assert!(
        (dcf_share_without_band - 0.7).abs() < 1e-12,
        "band 缺位时 DCF 腿的归一化份额必须恰为 0.7（这是「旧样本行为不变」的**唯一**保证），\n\
         实得 {dcf_share_without_band:.6}（现状 {w_dcf} / ({w_dcf} + {w_graham})）。\n\
         若有意改前两腿比例，须同步改本断言的 0.7。"
    );
    assert!(
        (w_graham / (w_dcf + w_graham) - 0.3).abs() < 1e-12,
        "band 缺位时 graham 腿份额必须恰为 0.3，实得 {:.6}",
        w_graham / (w_dcf + w_graham)
    );

    // ④ 分腿衰减必须在**融合之内**；最终 clamp 不得再乘一次。
    assert!(
        rhai.contains("clamp(fused_sig * fscore_mult * moat_mult, -1.0, 1.0)"),
        "最终表达式应为 `clamp(fused_sig * fscore_mult * moat_mult, ...)`，**不含** dcf_anchor_decay。\n\
         V79 把 fallback 衰减改为**分腿**（进融合表达式、只乘 DCF 腿）⇒ 最终 clamp 若仍乘它\n\
         就是**二次衰减**（σ 被腰斩两遍）。改动前的\n\
         `clamp(fused_sig * fscore_mult * moat_mult * dcf_anchor_decay, ...)` 必须不再存在。"
    );
    assert!(
        !rhai.contains("* moat_mult * dcf_anchor_decay"),
        "检测到 `* moat_mult * dcf_anchor_decay` ⇒ 分腿衰减与总值衰减**同时存在**，σ 被衰减两次。"
    );
}

/// `portfolio-mgr.rhai` 必须能编译（语法门）。
///
/// ## 为什么需要它
///
/// 该脚本在**生产侧只有两条编译路径**：What-If 回测与决策重跑 —— 都是用户点按钮
/// 才会走到。也就是说语法错误**不会**在 `cargo check` / `cargo test` 阶段暴露，
/// 而是等用户点了按钮才报「Rhai AST 编译失败」。改脚本的人（含 AI）极易在
/// 长注释/多行 `if` 上写坏却毫无反馈。
///
/// 本测试把这一步提前：**直接调用生产那个引擎工厂**
/// （`rhai_registry::build_stock_rhai_engine` + `RhaiSandboxLimits::PORTFOLIO`，
/// 见 `rhai_registry.rs`——What-If 与重跑两条路径共用它），
/// 通过 `include_str!` 编译 DAG 实际使用的那份文件。
///
/// ⚠️ 2026-09-22 改：原先测试自己 `Engine::new()`，并手抄 `register_common_functions`
/// 与 `set_max_expr_depths`，再断言「我的副本 == 生产源码里的两处副本」。
/// 档位收敛为具名单点后，那份手抄既无必要、又会让漂移重新变成静默假绿 ——
/// 现在函数集与档位**都是生产的那一份**。
///
/// ⚠️ 它只证明「能编译」，不证明「跑出来对」。语义仍由 `moat_level_vocabulary_matches_rhai_consumer`
/// 与估值链的 DB 取证负责。
#[test]
fn portfolio_mgr_rhai_compiles() {
    // 与 `src/commands/stock_analysis.rs` 的 `include_str!("portfolio-mgr.rhai")` 同一份文件。
    // ⚠ 路径基准是本文件所在目录（`commands/stock_analysis_setup/`）⇒ 上跳**一级**即 `commands/`。
    // 写成 `../../` 会解析到 `src/portfolio-mgr.rhai`（不存在）⇒ 直接编译失败。
    let code = include_str!("../portfolio-mgr.rhai");

    // ── ① 两条生产编译路径都必须消费唯一档位来源（形态守护，不再断言数字） ──
    // 本脚本因子多、表达式嵌套深 —— 超过 Rhai **默认**上限（实测 `(32, 16)`）⇒ 必须放宽。
    // 实测需求：函数体外 38 / `fn` 体内 19（零锁探针：`rustc --extern rhai=<deps rlib>` 直编单文件）。
    // 档位由 `rhai_registry::RhaiSandboxLimits::PORTFOLIO`（256）单点声明。
    assert_consumes_shared_sandbox("src/commands/stock_analysis.rs", "What-If 回测");
    assert_consumes_shared_sandbox("src/commands/stock_workflow/decision.rs", "决策重跑");

    // ── ② 直接用**生产那个引擎工厂**：「同配置」由是同一个函数保证，而非靠比对 ──
    // ⚠ 不要退回「本地 `Engine::new()` + 手抄 register + 手抄 set_max_expr_depths」：
    //   那样函数集与档位又变成两份副本，漂移后本测试照样全绿（**假绿**）。
    let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);
    axagent_harness::get_or_compile_ast("portfolio-mgr-syntax-gate", code, &engine).unwrap_or_else(
        |e| {
            panic!(
                "portfolio-mgr.rhai 在生产同配置（RhaiSandboxLimits::PORTFOLIO）下编译失败：{e}\n\
                 生产侧只在 What-If / 重跑时才编译本脚本 ⇒ 语法错误会留到用户点按钮才暴露。\n\
                 常见原因：多行 `if` 条件写坏、注释里出现未闭合的 `/*`、`#{{ }}` 对象字面量括号不配对。\n\
                 若报 `Expression exceeds maximum complexity` ⇒ 本轮新增的表达式已比 PORTFOLIO 更深，\n\
                 应展开嵌套或提取中间变量，**不要**只把上限调大（那会悄悄吃掉两条生产路径的余量）。"
            )
        },
    );

    // ── ③ 反向对照：默认上限下该脚本**必须**编不过 ──
    // 证明上面那次放宽不是抄来的冗余配置（否则本条变红 ⇒ 可复核是否收窄生产配置）。
    //
    // ⚠ 用 `Engine::compile` 直接编译，不走 `get_or_compile_ast`：
    //   本测试要判的是「**这个引擎**能不能编译这段 code」，属纯粹的编译能力问题，
    //   走缓存入口会把「缓存键怎么分桶」这个正交问题混进来（曾踩过：当时缓存键只含
    //   code 哈希 ⇒ 上面那次成功编译已把 AST 写进全局桶，第二次调用**直接命中返回 Ok**，
    //   反向对照被静默吞掉、看起来永远"没区分力"）。
    //   该分桶缺陷已于 2026-09-21 修复（缓存键 = code 哈希 **+ 解析期配置指纹**，
    //   见 `axagent_harness::rhai_ast_cache::parse_config_fingerprint`）——
    //   但**本测试继续用 `Engine::compile`**：判编译能力就不该借道缓存，
    //   否则将来缓存实现再变一次，这条反向对照会以同样的方式静默失效。
    let strict = rhai::Engine::new();
    assert!(
        strict.compile(code).is_err(),
        "在**默认**深度上限的引擎下 portfolio-mgr.rhai 竟然能编译 ⇒ 生产那份\n\
         `RhaiSandboxLimits::PORTFOLIO`（max_expr_depths=256）已成冗余配置（或本探测失效，\n\
         例如改回了走缓存的编译入口）。请复核后再决定是否收窄生产配置与其注释。"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 白名单授权 ↔ ToolDef 登记 —— 「授权了但送不到 LLM」的静默丢弃守护
//
// 背景（2026-09-21，v72 实测量到的**假修复**）：
//   `seed_stock_analysis.rs` 里节点 `config.tools` 由
//     `tool_names.iter().filter_map(|tn| tool_def_map.get(tn).cloned())`
//   生成 —— **`filter_map` 对查不到的名字静默丢弃**（不报错、不留痕）。
//   ⇒ 只往 `PROFILE_TOOLS` 加名字、漏了 `tool_def_map`，结果是
//     「配置上已授权，LLM 手里根本没有该工具」：该维度仍恒缺，
//     行为与修复前**完全一致**，且没有任何日志能提示这件事。
//   本轮补「股权质押」数据时正是先只改了白名单，靠人工比对才发现这条断链
//   （详见 `AUDIT-pledge-attribution-2026-09-21.md` §六）。
//
// 断言范围为什么只取 `analysts` 数组里的专家：
//   `PROFILE_TOOLS` 是**双消费**表 ——
//     ① chat 侧：`seed_agent_profiles` 写 `agent_profiles.recommended_tools`，
//        再由 `local_tool.rs::get_chat_tools_by_names` 按名过滤（**同样静默丢弃**）；
//     ② 工作流侧：`seed_stock_analysis.rs` 里**只有分析师循环这一处**真实读取它
//        （全文件 grep 仅 1 处非注释命中）。
//   其余专家（如 `stop-loss-reviewer`）只走 ① —— 其模板节点是显式 `tools: vec![]`
//   （见 `mod.rs` 的「生效面（勿误读）」段）⇒ 要求它们也登记 `tool_def_map`
//   属**过度约束**，会制造假红。故本测试的输入面刻意收在分析师数组上。
// ─────────────────────────────────────────────────────────────────────────────

/// 读取 `stock_analysis_setup/` 下的 seed 源文件（抽取面测试专用）。
fn read_seed_source_file(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/stock_analysis_setup")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {file} 失败: {e}"))
}

/// 抽 `analysts` 数组第 3 个元素（专家 id）。
///
/// 形态：`("a-lockup", "解禁减持与质押风险排查", "lockup-watcher"),`
///
/// 之所以能安全抽取（而 `tool_assignments` 的元组抽取已被本文件判定为会溢出、
/// 刻意不做，见该处说明）：本数组每个元素恰好 3 个字符串字面量、不跨行、
/// 不含嵌套调用，且取值域内无转义引号。
fn analyst_expert_ids(src: &str) -> Vec<String> {
    let start = src.find("let analysts = [").expect("找不到 analysts 数组起点");
    let end = start + src[start..].find("\n    ];").expect("找不到 analysts 数组终点");
    let mut out = Vec::new();
    for line in src[start..end].lines() {
        let t = line.trim();
        if !t.starts_with("(\"") {
            continue;
        }
        let parts = quoted_strings(t);
        // 恰好 3 项才算分析师行；形态不合就跳过（宁可少抽 —— 由下方正控把「少抽」变成报红）
        if parts.len() == 3 {
            out.push(parts[2].clone());
        }
    }
    out
}

/// 抽 `tool_def_map` 的 key 集合（即「登记进 ToolDef 表」的工具名）。
///
/// 覆盖两种声明形态（实测 Map 里并存）：
///   A. 名字与值同行：`("get_stock_quote", td_quote.clone()),`
///      以及内联：     `("search_stock", ToolDef { … })`
///   B. `(` 独占一行，名字在下一行（长内联 ToolDef 的换行写法）
fn tool_def_map_keys(src: &str) -> Vec<String> {
    let start = src.find("let tool_def_map").expect("找不到 tool_def_map 起点");
    let end = start + src[start..].find(".collect();").expect("找不到 tool_def_map 终点");
    let lines: Vec<&str> = src[start..end].lines().collect();
    // 工具名形态：字母开头 + [a-z0-9_]（把 JSON schema 里的 `"object"` 也放进来了，
    // 属可接受的过抽 —— 主断言是**单向包含**，过抽只会让断言更松，
    // 而「少抽」由下面的正控拦住）。
    let looks_like_tool_name = |s: &str| {
        let mut chars = s.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("(\"") {
            if let Some(j) = rest.find('"') {
                let name = &rest[..j];
                if looks_like_tool_name(name) {
                    out.push(name.to_string());
                    continue;
                }
            }
        }
        if t == "(" {
            let next = lines.get(i + 1).map(|l| l.trim());
            if let Some(rest) = next.and_then(|l| l.strip_prefix('"')) {
                if let Some(j) = rest.find('"') {
                    let name = &rest[..j];
                    if looks_like_tool_name(name) {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out
}

#[test]
fn analyst_profile_tools_are_all_declared_in_tool_def_map() {
    let src = read_seed_source_file("seed_stock_analysis.rs");
    let experts = analyst_expert_ids(&src);
    let declared: std::collections::HashSet<String> = tool_def_map_keys(&src).into_iter().collect();

    // ── 正控①：输入面必须看见真实内容，否则主断言空转 ──
    // （判据 #643 的形态：判据工具会静默撒谎，少抽一个名字，下游断言照样绿）
    assert!(
        experts.iter().any(|e| e == "lockup-watcher"),
        "分析师数组抽取面没看到 `lockup-watcher` ⇒ 抽取器失效，主断言会假绿: {experts:?}"
    );
    // ── 正控②：ToolDef 抽取面自证 —— 必须同时看到「长期存在项」与「v72 新增项」──
    // 只断言新增项是不够的：抽取器可能恰好只认某一种形态。
    assert!(
        declared.contains("get_stock_margin_data") && declared.contains("get_stock_pledge_data"),
        "tool_def_map 抽取面失效（既有工具名或 v72 新增名未被看到）: {declared:?}"
    );

    // ── 主断言：分析师用到的白名单工具必须全部登记 ToolDef ──
    let mut missing: Vec<String> = Vec::new();
    for e in &experts {
        let Some((_, tools)) = super::PROFILE_TOOLS.iter().find(|(k, _)| k == e) else {
            missing.push(format!("{e} → <该专家不在 PROFILE_TOOLS 中>"));
            continue;
        };
        for t in *tools {
            if !declared.contains(*t) {
                missing.push(format!("{e} → {t}"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "以下白名单工具**未登记** `tool_def_map` ⇒ 节点 `config.tools` 的 `filter_map` \
         会静默丢弃它们：\n  {}\n\
         ⇒ 后果是「配置上已授权，LLM 手里没有该工具」：相关维度仍恒缺，且无任何报错、无日志。\n\
         修法：在 `seed_stock_analysis.rs` 里补一个 `ToolDef`，并把名字登记进 `tool_def_map`。",
        missing.join("\n  ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P2-1(2026-09-21)：`data-quality.rhai` 的 `diag_for` **实参个数**守护
//
// 为什么必须单独守：Rhai 的函数调用**实参个数不匹配是运行期错误** ——
//   `cargo check` / `cargo clippy` 完全看不见（它们不解析 `.rhai`），
//   现有 `portfolio_mgr_rhai_compiles` 只覆盖 portfolio-mgr，且只做语法/常量级校验。
//   ⇒ 给 `diag_for` 加一个形参、却漏改 10 个调用点中的任意一个，
//   **data-quality 节点整体失败**，进而拖垮 quality-gate / portfolio-mgr 一条链。
//
// 这与本文件末尾 `analyst_profile_tools_are_all_declared_in_tool_def_map` 是同一类
// 「改了定义端、漏改消费端」，区别只在：那次是**静默**失效，这次是**响亮**失败。
// 两者都不该靠人眼去数实参。
// ─────────────────────────────────────────────────────────────────────────────

/// 从 `src[open]`（必须是 `(`）起做括号匹配，返回括号内文本（引号 / 行注释感知）。
///
/// 反斜杠写成常量 `92`，避免本文件里出现成串转义。
fn paren_body(src: &str, open: usize) -> Option<&str> {
    const BACKSLASH: u8 = 92;
    const SLASH: u8 = b'/';
    const QUOTE: u8 = b'"';
    const TICK: u8 = b'`';
    const LF: u8 = 10;

    let bytes = src.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str: Option<u8> = None;
    let mut i = open;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if c == BACKSLASH {
                i += 2;
                continue;
            }
            if c == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == QUOTE || c == TICK {
            in_str = Some(c);
            i += 1;
            continue;
        }
        // 行注释：跳到行尾（避免把注释里的括号算进深度）
        if c == SLASH && bytes.get(i + 1) == Some(&SLASH) {
            while i < bytes.len() && bytes[i] != LF {
                i += 1;
            }
            continue;
        }
        if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(&src[open + 1..i]);
            }
        }
        i += 1;
    }
    None
}

/// 按**顶层**逗号切分实参（括号 / 引号感知，故 `attr(1, 2)` 这类嵌套只算一个）。
fn split_top_level_commas(s: &str) -> Vec<String> {
    const BACKSLASH: u8 = 92;
    let bytes = s.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if c == BACKSLASH {
                i += 2;
                continue;
            }
            if c == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'`' {
            in_str = Some(c);
            i += 1;
            continue;
        }
        if c == b'(' || c == b'[' || c == b'{' {
            depth += 1;
        } else if c == b')' || c == b']' || c == b'}' {
            depth -= 1;
        } else if c == b',' && depth == 0 {
            out.push(s[start..i].trim().to_string());
            start = i + 1;
        }
        i += 1;
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

#[test]
fn data_quality_rhai_diag_for_arity_matches_all_call_sites() {
    let rhai_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/data-quality.rhai");
    let src = std::fs::read_to_string(&rhai_path)
        .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", rhai_path.display()));

    // ── ① 形参个数 ──
    let def_at = src.find("fn diag_for(").expect("找不到 `fn diag_for(`");
    let def_open = def_at + "fn diag_for".len();
    let params = split_top_level_commas(paren_body(&src, def_open).expect("形参括号不匹配"));
    assert!(
        params.len() >= 8,
        "`diag_for` 形参只有 {} 个（预期 ≥ 8，含 P2-1 的 `attr_note`）。实际：{params:?}",
        params.len()
    );

    // ── ② 每个调用点的顶层实参个数必须与形参一致 ──
    let needle = "diag_for(";
    let mut from = 0usize;
    let mut sites: Vec<(String, usize, String)> = Vec::new();
    while let Some(rel) = src[from..].find(needle) {
        let at = from + rel;
        from = at + needle.len();
        // 跳过定义本身（其所在行含 `fn `）
        let line_start = src[..at].rfind('\n').map_or(0, |p| p + 1);
        if src[line_start..at].contains("fn ") {
            continue;
        }
        let open = at + needle.len() - 1;
        let Some(body) = paren_body(&src, open) else {
            panic!("第 {} 个调用点括号不匹配（无法解析实参）", sites.len() + 1);
        };
        let args = split_top_level_commas(body);
        let key = args.first().cloned().unwrap_or_default();
        // S6/R9b 在 attr_note 之后追加了第 9 实参 `asof_ex`、第 10 实参 `raw_ph_n`
        // ⇒ attribution_note 不再是末位实参，改按**第 8 位固定**取（形参 `attr_note` 即第 8 个）。
        let attr = args.get(7).cloned().unwrap_or_else(|| "<第 8 实参缺失>".to_string());
        sites.push((key, args.len(), attr));
    }

    // 正控①：调用点个数。10 = `diagnostics` map 的 10 个分析师。
    //   少于 10 ⇒ 抽取器失配（本断言红）；多于 10 ⇒ 新增了分析师，须同步此数。
    assert_eq!(
        sites.len(),
        10,
        "`diag_for` 调用点应为 10 个（= 10 个分析师），实际 {} 个。\
         若新增/删除分析师请同步本数字；否则先查抽取器。",
        sites.len()
    );

    // 主断言：实参个数一致
    let mismatched: Vec<String> = sites
        .iter()
        .filter(|(_, n, _)| *n != params.len())
        .map(|(k, n, _)| format!("调用点 {k} → 实参 {n} 个（形参 {} 个）", params.len()))
        .collect();
    assert!(
        mismatched.is_empty(),
        "以下 `diag_for` 调用点的实参个数与形参不符：\n  {}\n\
         ⇒ Rhai 的实参个数不匹配是**运行期**错误（`cargo check`/`clippy` 看不见）\
         ⇒ data-quality 节点整体失败，并拖垮 quality-gate / portfolio-mgr 一条链。",
        mismatched.join("\n  ")
    );

    // ── ③ 第 8 实参必须是 `attribution_note(<报告>, <调用记录>)`，且两变量已在
    //        `input_mapping` 声明（Rhai 引用未声明变量不报错，只得到 unit ⇒ 静默失效）──
    let seed = read_seed_source_file("seed_stock_analysis.rs");
    let dq_at = seed.find("let dq_id = \"data-quality\";").expect("定位 data-quality 的锚点失效");
    let map_at = seed[dq_at..]
        .find("input_mapping")
        .map(|p| dq_at + p)
        .expect("找不到 data-quality 节点的 `input_mapping`");
    // v133（B2-2）：DQ 的 input_mapping 改为块表达式（生成式 + extend）⇒ 原 `\n                ]`
    // 收尾锚失效（会兜到文件尾、把 7000 行都算进声明集）。新收尾 = 生成块的最后一句。
    let map_end =
        seed[map_at..].find("dq_input.into_iter().collect()").map_or(seed.len(), |p| map_at + p);
    let map_block = &seed[map_at..map_end];
    // 不做「键/路径」配对解析（映射表有多行写法），只取块内**全部字符串字面量**作
    // 「声明过的名字」超集 —— 足以抓出拼写错误这类目标缺陷。
    let mut declared: std::collections::HashSet<String> = map_block
        .lines()
        .flat_map(|l| {
            l.split('"').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, v)| v.to_string())
        })
        .collect();
    // v133（B2-2）：分析师侧 40 键改为生成式（`{abbr}_{kind}`）——字面量只剩 abbr 表
    // （`("mk", "a-market-analyst")` 行的第一段）与四类后缀，补全笛卡尔积。
    // abbr 表从块内抽取（不另抄第二份）；下限自证防抽取面失效。
    let abbrs: std::collections::HashSet<String> = map_block
        .lines()
        .filter_map(|l| {
            let strs: Vec<&str> =
                l.split('"').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, v)| v).collect();
            // v133：abbr 表含 `("val", "value-investor")`（无 `a-` 前缀）⇒ 两种形态都收。
            if strs.len() == 2 && (strs[1].starts_with("a-") || strs[1] == "value-investor") {
                Some(strs[0].to_string())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        abbrs.len(),
        10,
        "abbr 表抽取面失效（应为 10 槽：9 a-* + value-investor）: {abbrs:?}"
    );
    for kind in ["verdict", "report", "untrusted", "tool_calls"] {
        for abbr in &abbrs {
            declared.insert(format!("{abbr}_{kind}"));
        }
    }
    // 正控②：抽取面自证 —— 必须看到 P2-1 新增的键
    assert!(
        declared.contains("lk_report") && declared.contains("lk_tool_calls"),
        "input_mapping 抽取面失效（没看到 lk_report / lk_tool_calls），共 {} 项",
        declared.len()
    );

    let mut bad_vars: Vec<String> = Vec::new();
    let mut bad_prefix: Vec<String> = Vec::new();
    for (key, _, last) in &sites {
        // R2(2026-10-02)：第 8 实参允许被 `merge_two_notes(...)` 包一层 —— 措辞性缺席的说明串
        //   与归因核对结论**共用既有的 `attr_note` 通道**（不为此新增输出字段 + 前端行 + 11 语言 key）。
        //   解包后本段原有三项检查**一条不减**：内层仍是 `attribution_note(x, y)`、
        //   两变量仍须在 `input_mapping` 声明过、仍须同属一个分析师；
        //   并**再加一项**：外层第二实参须是 `<分析师前缀>_word_note` 且该变量在脚本里真的 `let` 过
        //   —— Rhai 引用未定义变量不报错、只静默给 unit，与本段开头 ③ 的原始动机同一条。
        let (attr_expr, word_note) =
            match last.strip_prefix("merge_two_notes(").and_then(|r| r.strip_suffix(')')) {
                Some(inner) => {
                    let mut parts = split_top_level_commas(inner);
                    if parts.len() != 2 {
                        bad_vars.push(format!(
                            "调用点 {key} 的 merge_two_notes 有 {} 个实参（应为 2）：{inner}",
                            parts.len()
                        ));
                        continue;
                    }
                    let note = parts.pop().expect("已判 len==2");
                    (parts.pop().expect("已判 len==2"), Some(note.trim().to_string()))
                },
                None => (last.clone(), None),
            };
        let Some(inner) =
            attr_expr.strip_prefix("attribution_note(").and_then(|r| r.strip_suffix(')'))
        else {
            bad_vars.push(format!("调用点 {key} 的第 8 实参形态非 attribution_note(x, y)：{last}"));
            continue;
        };
        let Some((a, b)) = inner.split_once(',') else {
            bad_vars.push(format!("调用点 {key} 的 attribution_note 只有 1 个实参：{inner}"));
            continue;
        };
        let (a, b) = (a.trim(), b.trim());
        for v in [a, b] {
            if !declared.contains(v) {
                bad_vars.push(format!(
                    "调用点 {key} 引用了**未声明**变量 `{v}` ⇒ Rhai 里静默得到 unit、不报错"
                ));
            }
        }
        // 结构核对：报告与调用记录必须属于**同一个**分析师（前缀一致）
        if a.split('_').next() != b.split('_').next() {
            bad_prefix.push(format!("调用点 {key}：`{a}` 与 `{b}` 不是同一个分析师"));
        }
        if let Some(n) = &word_note {
            let prefix = a.split('_').next().unwrap_or_default();
            let want = format!("{prefix}_word_note");
            if *n != want {
                bad_vars.push(format!(
                    "调用点 {key} 的 merge_two_notes 第二实参应为 `{want}`（与内层同一分析师），实得 `{n}`"
                ));
            } else if !src.contains(&format!("let {want} =")) {
                bad_vars.push(format!(
                    "`{want}` 在脚本里没有 `let` 定义 ⇒ Rhai 静默得到 unit、措辞性缺席的说明串永远为空"
                ));
            }
        }
    }
    assert!(bad_vars.is_empty(), "`attribution_note` 的实参有问题：\n  {}", bad_vars.join("\n  "));
    assert!(
        bad_prefix.is_empty(),
        "`attribution_note` 把不同分析师的报告与调用记录配到了一起：\n  {}",
        bad_prefix.join("\n  ")
    );
}

/// 一份 VERDICT 专家提示词的结构性缺陷体检，返回命中的缺陷类别。
///
/// 四条判据各自对应「模型学会只回标签」的一条成因，缺一漏一类：
/// - `fence`：代码围栏未闭合 ⇒ 后续小节整段被当成代码，指令语义走形
/// - `leaked-tag`：示例的 `<!-- VERDICT -->` 标签泄漏到围栏之外 ⇒ 模型把它当正文读
/// - `dup-example`：`## 参考示例` 段内同一段示例贴了两遍（实证缺陷的形态）
/// - `soft-body`：有「关键规则」清单却没把正文写成硬指标
fn verdict_prompt_defects(src: &str) -> Vec<&'static str> {
    let mut defects: Vec<&'static str> = Vec::new();
    let blocks: Vec<&str> = src.split("```").collect();
    if blocks.len().is_multiple_of(2) {
        defects.push("fence");
    }
    // 只看**独占一行**的标签 —— 行内 `` `<!-- VERDICT: {...} -->` `` 是格式说明，合法。
    for (i, seg) in blocks.iter().enumerate() {
        if i % 2 == 0 && seg.lines().any(|l| l.trim_start().starts_with("<!-- VERDICT")) {
            defects.push("leaked-tag");
            break;
        }
    }
    // 查重范围必须收在 `## 参考示例` 之内：全文级查重会误伤「强制降级报告」这类
    // 在别处声明、又在示例里复现的合法重复（hot-money-tracker 实测）。
    if let Some(start) = src.find("## 参考示例") {
        let rest = &src[start + "## 参考示例".len()..];
        let section = rest.find("\n## ").map_or(rest, |k| &rest[..k]);
        let mut seen: HashSet<String> = HashSet::new();
        for (i, seg) in section.split("```").enumerate() {
            let trimmed = seg.trim();
            if i % 2 == 1 && trimmed.len() > 40 && !seen.insert(trimmed.to_string()) {
                defects.push("dup-example");
                break;
            }
        }
    }
    if src.contains("报告正文是自由自然语言") && !src.contains("没有分析正文的输出视为无效")
    {
        defects.push("soft-body");
    }
    defects
}

/// 「只有 VERDICT 标签、没有正文」的示范形态不得留在专家提示词里（2026-09-28 I 轮）。
///
/// 背景：`a-sector` 实证一次「模型只回标签、零正文」的输出（225 token、非截断）。查明提示词
/// 本身在示范这个形态 —— 参考示例的正文只有两行，且同一段示例被**贴了两遍**、第三份正文
/// 连同 `<!-- VERDICT -->` 标签**泄漏到代码围栏之外**（模型把它当指令正文读），
/// 连带把 `## 自检` 整段困进未闭合的代码块里。
#[test]
fn verdict_expert_prompts_do_not_model_bodyless_output() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("agency_experts")
        .join("stock-analysis");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_md_files(&root, &mut files);
    let targets: Vec<std::path::PathBuf> = files
        .into_iter()
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains("VERDICT")))
        .collect();

    // 自证①：扫描面必须真的装到东西（目录改名会让本测试静默失效）
    assert!(
        targets.len() >= 10,
        "VERDICT 专家 md 扫描面异常：只在 {} 找到 {} 个 —— 若目录结构变了，先修路径，不要放宽断言。",
        root.display(),
        targets.len()
    );

    let mut report: Vec<String> = Vec::new();
    for f in &targets {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        for d in verdict_prompt_defects(&src) {
            report.push(format!("{} ⇒ {}", f.display(), d));
        }
    }
    assert!(
        report.is_empty(),
        "VERDICT 专家提示词存在「示范只回标签」形态的缺陷：\n  {}",
        report.join("\n  ")
    );

    // 自证②：判据对**修复前**的真实形态必须命中，否则本门禁等于没装电池。
    // 样本按 2026-09-28 修复前的 sector-analyst.md 复刻：示例贴两遍 → 第三份正文连标签
    // 泄漏到围栏外 → 反例块只开不闭（`## 自检` 整段被困进代码块，故围栏总数为奇数）。
    let corrupted = concat!(
        "## 输出格式\n\n1. 报告正文是自由自然语言，任意格式都可以\n\n## 参考示例\n\n",
        "```\n两行正文\n\n<!-- VERDICT: {\"verdict\": \"中性\"} -->\n```\n\n",
        "```\n两行正文\n\n<!-- VERDICT: {\"verdict\": \"中性\"} -->\n```\n\n",
        "## 量价分析\n\n一段泄漏的正文。\n\n<!-- VERDICT: {\"verdict\": \"中性\"} -->\n\n",
        "```\n（反例说明）\n\n## 自检\n\n- [ ] 检查项\n",
    );
    let hit = verdict_prompt_defects(corrupted);
    for want in ["fence", "leaked-tag", "dup-example", "soft-body"] {
        assert!(hit.contains(&want), "判据 `{want}` 对修复前的真实形态未命中 ⇒ 检法失效");
    }
}

/// Phase 2（PLAN-horizon-four-cycle-closure.md）：专家提示词里的 `{{占位符}}`
/// 必须**有注入来源** —— 要么注册为种子模板变量，要么是所属 AgentNode `input_mapping` 的 target。
///
/// 为什么值得单独一道门：渲染由 `prompt_template.rs::render_prompt` 完成，
/// **两条路都没有的占位符在运行期**才报 `VARIABLE_NOT_FOUND`
/// （`agent_executor.rs` 映射到 `error_code::VARIABLE_NOT_FOUND`）。
/// 也就是说 prompt 里多写一个变量名，`cargo check` / `cargo test` / clippy **全绿**，
/// 要到真实跑分析时才炸 —— 本门把这类缺陷前移到构建期。
///
/// ⚠ 判据必须是「并集」而不是「变量表」：`{{stock_code}}` / `{{market_regime}}` 这类
///   占位符**不在** `build_template_variables()` 里，而由所属节点的 `input_mapping`
///   注入（先例见 `seed_stock_analysis.rs` 的 reflection-agent 映射表）。
///   首版本门只查变量表，对全项目 60+ 个合法占位符**全部误报** —— 一道满屏假阳性的门
///   等于逼人把它关掉，故补节点侧清单 + 负控自证。
#[test]
fn expert_prompt_placeholders_all_have_an_injection_source() {
    use regex::Regex;

    let re = Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)").expect("占位符正则应合法");
    let root_of = |ph: &str| ph.split('.').next().unwrap_or(ph).to_string();

    // 来源 1：种子模板变量表
    let var_names: std::collections::HashSet<String> =
        super::seed_variables::build_template_variables()
            .into_iter()
            .map(|v| root_of(&v.name))
            .collect();

    // 来源 2：各 AgentNode 的 input_mapping target（key 侧）。
    //   节点太多且映射表由 `agent()` 辅助函数分散构造，此处以「整份 seed 源文本里
    //   出现的 ("target", "source") 元组第一项」为清单 —— 宁可宽（漏判）也不误报，
    //   真正的窄门由 `audit-inject-coverage.mjs` 在 Rhai 侧承担。
    let seed_src = include_str!("seed_stock_analysis.rs");
    let tuple_re = Regex::new(r##"\(\s*"([A-Za-z_][A-Za-z0-9_]*)"\s*,"##).expect("元组正则应合法");
    let mapping_targets: std::collections::HashSet<String> =
        tuple_re.captures_iter(seed_src).map(|c| c[1].to_string()).collect();

    // 来源 3：**运行期注入**的工作流变量（enhance 钩子 / 反思播种现场 push 进 variables）。
    //   `stock_name` / `market_regime` / `bull_lessons` / `original_time_horizon` 等
    //   既不在种子变量表、也不是 input_mapping 手写项，而是 `Variable { name: "x" … }`
    //   结构体字面量现场注入 —— 首版门只查前两路，把这批全打成假阳性。
    let name_re = Regex::new(r##"name:\s*"([A-Za-z_][A-Za-z0-9_]*)""##).expect("变量名正则应合法");
    let runtime_names: std::collections::HashSet<String> = [
        include_str!("../../commands/stock_workflow/hooks.rs"),
        include_str!("../../commands/stock_workflow/reflection.rs"),
        include_str!("../mod.rs"),
    ]
    .into_iter()
    .flat_map(|src| name_re.captures_iter(src).map(|c| c[1].to_string()))
    .collect();

    let mut unsourced: Vec<String> = Vec::new();
    for (id, md) in super::EMBEDDED_PROMPTS {
        for cap in re.captures_iter(md) {
            let ph = root_of(&cap[1]);
            if !var_names.contains(&ph)
                && !mapping_targets.contains(&ph)
                && !runtime_names.contains(&ph)
            {
                unsourced.push(format!("{id}: {{{{{ph}}}}}"));
            }
        }
    }
    assert!(
        unsourced.is_empty(),
        "以下占位符既非注册变量也非 input_mapping target ⇒ 运行期必报 VARIABLE_NOT_FOUND: {unsourced:?}"
    );

    // ── 负控（新检法必须自证命中）──
    // ① 判据能认出「注册过的」：stock_lessons 是 reflection-agent 的真变量，不得被误判为缺失
    assert!(
        var_names.contains("stock_lessons"),
        "变量表应含 stock_lessons（判据的输入面本身失效）"
    );
    // ①' 运行期注入面也要被认出来：regime_prompt_bias 由 hooks.rs 现场注入，
    //     若第三路来源失效（正则/文件路径漂了），它会重新被误报 ⇒ 判据当场自证失效。
    assert!(
        runtime_names.contains("regime_prompt_bias"),
        "运行期注入面扫描失效：hooks.rs 的 regime_prompt_bias 没被抓到"
    );
    // ② 判据能认出「未注册的」：一个绝不存在的名字必须落进 unsourced 桶
    let probe = root_of("__definitely_not_a_placeholder__");
    assert!(
        !var_names.contains(&probe)
            && !mapping_targets.contains(&probe)
            && !runtime_names.contains(&probe),
        "负控失效：判据抓不到无来源占位符"
    );
    // ③ 正则真的能从文本里抓出占位符（否则上面两条都是空转）
    let hit = re
        .captures("正文 {{__definitely_not_a_placeholder__}} 结尾")
        .expect("正则应命中占位符形态");
    assert_eq!(&hit[1], "__definitely_not_a_placeholder__");
}

/// Phase 3（〇-B v2 第 3 条）：可调参数的「三处齐备」必须有门。
///
/// 判据（`seed_stock_analysis.rs` 头部注释写明的规矩）：登记进
/// `PORTFOLIO_MGR_TUNABLE_PARAMS` 的每一个参数名，
///   ① 在 `seed_variables::build_template_variables()` 里有同名变量定义；
///   ② 在 `portfolio-mgr.rhai` 里被 `present(<名>)` 真读。
///
/// 为什么这两条都要机械检查（V71 实证形态）：只加清单不加变量 ⇒ `present()` 恒假
/// ⇒ 参数**静默走脚本硬编码**，面板显示「可配置」但改了没有任何效果；
/// 只加变量不读 ⇒ 反思建议按名回写了一个没人消费的变量。两种都不报错、不打日志，
/// `cargo check` / clippy / 编译期 Rhai parse 门全绿。
#[test]
fn tunable_params_have_variable_and_rhai_guard() {
    use super::seed_stock_analysis::PORTFOLIO_MGR_TUNABLE_PARAMS;
    use super::seed_variables::build_template_variables;

    let var_names: std::collections::HashSet<String> =
        build_template_variables().into_iter().map(|v| v.name).collect();
    let rhai = include_str!("../../commands/portfolio-mgr.rhai");

    let mut missing_var: Vec<&str> = Vec::new();
    let mut unread_in_rhai: Vec<&str> = Vec::new();
    for name in PORTFOLIO_MGR_TUNABLE_PARAMS.iter() {
        if !var_names.contains(*name) {
            missing_var.push(name);
        }
        if !rhai.contains(&format!("present({name})")) {
            unread_in_rhai.push(name);
        }
    }
    assert!(
        missing_var.is_empty(),
        "以下可调参数登记了清单但没有变量定义 ⇒ 静默走硬编码默认值: {missing_var:?}"
    );
    assert!(
        unread_in_rhai.is_empty(),
        "以下可调参数有变量/清单但 portfolio-mgr.rhai 不读 ⇒ 配置与反思建议均无消费方: {unread_in_rhai:?}"
    );

    // 负控（新检法必须自证命中）：本仓既有的两个逐周期档位参数必须真的在三门里齐备，
    // 且判据能识别「一个不存在的名字」—— 否则上面两条断言是空转。
    for probe in ["sl_pct_ultra_short", "tp_pct_long"] {
        assert!(var_names.contains(probe), "档位参数应已注册为变量: {probe}");
        assert!(rhai.contains(&format!("present({probe})")), "档位参数应被脚本读取: {probe}");
    }
    let ghost = "__definitely_not_a_tunable__";
    assert!(!var_names.contains(ghost) && !rhai.contains(&format!("present({ghost})")));
}

/// Phase 7 结构性门：**同一份节点构造块不得在 seed 里出现两次**（窗口式编辑错锚的检法）。
///
/// 实证（2026-09-28，本轮我自己造成）：用 `str.index()` 定位「删我自己那 2 行注释」时，
/// 起点与终点各自解析到**不同份**的同形文本 ⇒ 窗口跨越 500 余行，把整个 portfolio-mgr
/// 节点构造块**复制成两份**。它照样能编译（只是 `let pm` 被遮蔽），所以
/// `cargo check` 全绿；`fast_seed_skips_when_identical_*` 也抓不到（重复块文本自等）；
/// 只有 clippy 的 unused-variable 会响，而 **clippy 遇首个 error 即短路**，不保证覆盖到。
///
/// 判据取「同一 `id` 字面量在整文件出现两次」—— 节点 id 必须唯一，重复即后者静默覆盖前者。
#[test]
fn seed_node_constructors_are_not_duplicated() {
    let src = include_str!("seed_stock_analysis.rs");
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for cap in
        Regex::new(r#"(?m)^\s{8,}id: "([a-zA-Z0-9_\-]+)"\.into\(\),"#).unwrap().captures_iter(src)
    {
        *counts.entry(cap[1].to_string()).or_insert(0) += 1;
    }
    let dups: Vec<(&String, &usize)> = counts.iter().filter(|(_, n)| **n > 1).collect();
    assert!(
        dups.is_empty(),
        "以下节点 id 在 seed 里被构造了多次 ⇒ 极可能是窗口式编辑复制出的整块重复: {dups:?}"
    );

    // 负控（自证判别力）：把事故形态在内存里复刻一遍 —— 复制 pm 构造块，判据必须报出 portfolio-mgr。
    let start =
        src.find("    let pm = WorkflowNode::Code(CodeNode {").expect("负控起点：pm 构造块");
    let end =
        src.find("    nodes.push(pm);").expect("负控终点：pm 入表") + "    nodes.push(pm);".len();
    let mutated = format!("{}\n{}{}", &src[..end], &src[start..end], &src[end..]);
    let mut mc: std::collections::BTreeMap<String, usize> = Default::default();
    for cap in Regex::new(r#"(?m)^\s{8,}id: "([a-zA-Z0-9_\-]+)"\.into\(\),"#)
        .unwrap()
        .captures_iter(&mutated)
    {
        *mc.entry(cap[1].to_string()).or_insert(0) += 1;
    }
    assert_eq!(
        mc.get("portfolio-mgr"),
        Some(&2),
        "负控失效：复刻整块重复后判据抓不到（则该门是空转的）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// v115(2026-10-01)：`kline_limit` 的**跨侧接线**守护（「声明了但没人消费」型缺陷）
//
// 被判缺陷（600406 运行 `a7e590a4` 实证）：`market-analyst.md` 方法论第 1 条要求
// 「30/60/120/250 日均线状态」，而 `kline_limit` 变量自建立以来**全仓零消费方** ——
// 只出现在 `seed_variables.rs`（声明）、设置面板（可调）、单测里，
// `seed_stock_analysis.rs` 一次都没引用 ⇒ `get_stock_kline` 恒走缺省 120 根
// （`mcp_tools.rs`：`unwrap_or(120).min(500)`）⇒ **MA250 物理上算不出来**
// ⇒ 报告写「250日均线数据缺失」⇒ 该节点被判「⚠️ 低置信」，归因还写成「上游工具数据不完整」。
//
// 为什么必须**跨两侧**断言：这类缺陷的形状是「两处各自自洽、合起来不成立」——
//   · 声明端：变量表里 `kline_limit` 一应俱全（默认值、描述、面板入口）；
//   · 消费端：tool 节点**合法地**不传 `limit`（工具侧有缺省值，不传不报错）。
// 任一单侧测试都会绿。只有把「变量名出现在 tool 节点的 input_mapping 里」钉住，
// 下一次「重构 tool 节点」才不会静默摘掉接线。
// ─────────────────────────────────────────────────────────────────────────────

/// 折叠空白，便于对**跨行**书写做不敏感匹配（rustfmt 会按行宽重排参数列表）。
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn kline_limit_is_wired_to_market_data_node() {
    let src = read_seed_source_file("seed_stock_analysis.rs");

    // ① 接线本体：`t-market-data` 必须把 `kline_limit` 映射到工具的 `limit` 参数。
    //    只在 tool 节点构造循环内找（切到首次 `nodes.push(tool_node(` 为止），
    //    避免命中别处的同形字面量而**假绿**。
    let start = src
        .find(
            "for (i, (tool_id, tool_title, tool_name, arg_key)) in tool_assignments.iter().enumerate()",
        )
        .expect("找不到 tool 节点构造循环（seed 结构已变，请同步本判据）");
    let region = &src[start..];
    let end = region.find("nodes.push(tool_node(").expect("循环体内找不到 tool_node 调用");
    let norm = normalize_ws(&region[..end]);
    assert!(
        norm.contains(r#""t-market-data""#),
        "判据的定位前提失效：该循环体内未出现 t-market-data ⇒ 本测试会退化为恒绿，请检查 seed 结构"
    );
    assert!(
        norm.contains(r#"("limit", "kline_limit")"#),
        "`t-market-data` 必须把 `kline_limit` 接到工具的 `limit` 参数上。\
         缺失 ⇒ 又回到 `get_stock_kline` 的缺省 120 根 ⇒ MA250 恒不可算\
         （实证：技术面报告写「250日均线数据缺失（数据仅覆盖4月至今）」→ 该节点被判低置信）"
    );

    // ② 默认值必须够算 250 日均线（MA250 = 最近 250 根收盘 ⇒ 至少 250 根）。
    //    ⚠ 这一条**刻意不在本测试里**断言：`assert!(CONST >= 250)` 是**运行期**断言，
    //    clippy `assertions_on_constants` 在 `-D warnings` 下当场报红（本测试首版即踩）。
    //    已改为 `seed_variables.rs` 里紧随该常量的**编译期**断言
    //    （`const _: () = assert!(DEFAULT_ANALYST_KLINE_LIMIT >= 250, …)`）——
    //    形态更强（不依赖有人记得跑测试），且与 DCF/K 线两个迁移门的断言同型。
    //    「落库后的变量值也够 250」由对象面测试
    //    `seed_stock_analysis::fast_workflow_derivation_tests::market_data_node_passes_kline_limit_and_migrates_stale_value` 覆盖。

    // ③ 存量覆写与一次性门必须仍在。
    //    `merge_variable_values` 对同名变量**无条件保留旧值**（用户 DB 里是旧种子缺省 120）
    //    ⇒ 只改默认值是「仓库里改了、用户库里没改」的假修复（v74 的 DCF 同型实证）。
    //    也不得去掉门：无门则每次无关升版都把它打回默认，抹掉用户面板里的调整。
    let norm_all = normalize_ws(&src);
    assert!(
        norm_all.contains(r#"force_variable_value( &variables_val, "kline_limit","#),
        "`kline_limit` 的存量覆写（force_variable_value）缺失或被改写：\
         merge_variable_values 会保留 DB 旧值 120 ⇒ 本修复对存量安装不生效（假修复）"
    );
    assert!(
        norm_all.contains("KLINE_LIMIT_MIGRATION_VERSION"),
        "存量覆写必须由一次性门 `KLINE_LIMIT_MIGRATION_VERSION` 守护"
    );
}

/// 逐档因子清单**只能有一份**：权威表 `Period::verdict_spec()`。
///
/// 专家 md 里出现因子名 = 手抄了第二份，必然与表漂移（「md 写七项、门按八项判」）。
/// P3′ 已把契约改成执行期按表注入（`agent_executor.rs` 的 4j 段 + `verdict_contract_prompt()`），
/// 本门锁住这个不变量。
fn hand_copied_horizon_factors(src: &str) -> Vec<&'static str> {
    axagent_harness::Period::verdict_factors().iter().copied().filter(|f| src.contains(f)).collect()
}

#[test]
fn expert_prompts_do_not_hand_copy_per_horizon_factor_lists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("agency_experts");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_md_files(&root, &mut files);

    // 自证①：扫描面必须真的装到东西 —— 目录改名会让本门静默失效
    assert!(
        files.len() >= 20,
        "专家 md 扫描面异常：{} 只找到 {} 个 —— 若目录结构变了，先修路径，不要放宽断言。",
        root.display(),
        files.len()
    );

    let report: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let src = std::fs::read_to_string(f).unwrap_or_default();
            let hits = hand_copied_horizon_factors(&src);
            (!hits.is_empty()).then(|| format!("{} ⇒ 手抄了 {:?}", f.display(), hits))
        })
        .collect();
    assert!(
        report.is_empty(),
        "专家提示词里出现逐档因子名。契约应由 verdict_spec() 注入，不得复制进 md：\n  {}",
        report.join("\n  ")
    );

    // 自证②：负控 —— 合成一份手抄文本，判据必须命中，否则本门等于没装电池。
    // （不靠改生产码来证明门会红：坏样本喂给判据函数本身。）
    let synth = "本档必须输出：momentumSignal、flowPersistence、valuationBand、notApplicableFlow";
    let hit = hand_copied_horizon_factors(synth);
    assert_eq!(hit.len(), 3, "负控未命中 ⇒ 检法失效，实得 {hit:?}");
    // 通用 6 键（confidence/bull_score…）**允许**留在 md —— 它们是跨档共用核心，
    // 不在因子全集里，所以下面这条必须为空命中：证明本门只拦逐档因子、不拦通用键。
    assert!(
        hand_copied_horizon_factors("confidence 0-100 整数，bull_score/bear_score 之和接近 100")
            .is_empty(),
        "误拦通用键：本门只该管逐档因子"
    );
}

/// P4′-b：四份**逐档决策脚本**的三道结构门（R-11 的机械证明，不靠人工 review）。
///
/// ① **禁词**：退役形态不得回归 —— `leg_mult` / `horizon_leg_multipliers` /
///    `pm_snr_confidence` / `horizon_decision` / `snrAnchorDays`。
///    存在理由：这三样是「一个算法 + 四套参数」在代码里的藏身处；分支脚本里再出现任何一个，
///    就说明档间差异又退回了「同一个数乘不同标量」。
/// ② **正向断言**：每份都必须真的读 `branch_json` 并调 `pm_leg_signal` ——
///    只查禁词的话，把整段融合删掉也能过门（本仓为「恒真/恒假断言」付过代价）。
/// ③ **四份必须两两不同**：复制粘贴四份再改档位名，是最省事也最危险的假分支形态。
/// ④ **编译**：用生产同一个引擎工厂与同一个沙箱档位（`RhaiSandboxLimits::PORTFOLIO`），
///    「同配置」由是同一个函数保证，而不是靠比对参数表。
#[test]
fn horizon_branch_rhai_scripts_compile_and_are_genuinely_forked() {
    const BANNED: &[&str] = &[
        "leg_mult",
        "horizon_leg_multipliers",
        "pm_snr_confidence",
        "horizon_decision",
        "snrAnchorDays",
    ];
    let scripts: [(&str, &str); 4] = [
        ("ultra_short", include_str!("../portfolio-mgr-h-ultra-short.rhai")),
        ("short", include_str!("../portfolio-mgr-h-short.rhai")),
        ("mid", include_str!("../portfolio-mgr-h-mid.rhai")),
        ("long", include_str!("../portfolio-mgr-h-long.rhai")),
    ];
    let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);

    for (tier, code) in &scripts {
        let body = code_only(code);
        assert!(
            body.lines().count() > 40,
            "档 {tier} 剥注释后只剩 {} 行 ⇒ 注释剥离把代码也吃了，本门的结论不可信",
            body.lines().count()
        );
        for b in BANNED {
            assert!(
                !body.contains(b),
                "档 {tier} 的分支脚本**代码里**出现退役形态 `{b}` ⇒ 又走回共享算法"
            );
        }
        assert!(
            body.contains("branch_json"),
            "档 {tier} 未读注入的分支表 ⇒ 腿集合与配比又变成脚本内手抄"
        );
        assert!(
            body.contains("pm_leg_signal"),
            "档 {tier} 未调 pm_leg_signal ⇒ 因子信号口径又各自实现一份（迟早漂移）"
        );
        assert!(
            body.contains(&format!("\"{tier}\"")),
            "档 {tier} 未声明自己的档位标签 ⇒ 分支表错配检测（tier != h）会静默失效"
        );
        axagent_harness::get_or_compile_ast(&format!("horizon-branch-{tier}"), code, &engine)
            .unwrap_or_else(|e| panic!("档 {tier} 的分支脚本在生产同配置下编译失败：{e}"));
    }

    // 两两不同：四份的**融合段**（从 `let tw` 到动作阶梯）必须互不相同
    for i in 0..scripts.len() {
        for j in (i + 1)..scripts.len() {
            assert_ne!(
                scripts[i].1.replace(char::is_whitespace, ""),
                scripts[j].1.replace(char::is_whitespace, ""),
                "分支脚本 {} 与 {} 逐字相同 ⇒ 四路是复制出来的假分支",
                scripts[i].0,
                scripts[j].0
            );
        }
    }
}

/// v129（#45）的**代码域**禁词与结构锁：主链风险档必须整条按档，不许留「只有否决按档」的旁路。
///
/// 为什么住在本文件而不是 `rhai_registry.rs`：这里才有 `code_only()`（四份分支脚本的禁词门
/// 共用同一份剥离实现）。不剥注释就查 `risk_for_veto` 必然假红 —— 退役说明与本轮的
/// 归因注释都要提到那个已退役的变量名，禁词门若把注释也算进去，改的是注释而不是判据。
#[test]
fn main_chain_risk_grade_is_tier_scoped_in_code_domain() {
    let raw = include_str!("../portfolio-mgr.rhai");
    let code = code_only(raw);
    // ① 退役变量不得在**代码域**复活（prose 里提它是合法的，故必须先剥）。
    assert!(
        !code.contains("risk_for_veto"),
        "退役变量 `risk_for_veto` 又出现在代码里 ⇒ v129 的「主链风险档整体按档」被旁路化"
    );
    // 自证剥离没吃掉代码：把同一 token 放进真代码行，判据必须抓到。
    let probe = code_only(&format!("{code}\nlet risk_for_veto = overall_risk;"));
    assert!(probe.contains("risk_for_veto"), "自证失效：注入代码行后 code_only 读不到该 token");
    // ② 否决回到 `overall_risk`，且全脚本恰一处（两处意味着另有一条并行口径）。
    assert_eq!(
        code.matches("pm_risk_veto(final_action, overall_risk)").count(),
        1,
        "应恰有一处 `pm_risk_veto(final_action, overall_risk)`"
    );
    // ③ 按档结果必须覆盖 `overall_risk` 本身 —— f4 强度、f4_signal 口径、risk_bias、仓位 cap
    //    全都读它；只挂给否决就是回到 v128 的半按档形态。
    assert!(
        code.contains("overall_risk = risk_tier;"),
        "按档值没落进 `overall_risk` 本体 ⇒ 下游三处（f4 / risk_bias / cap）仍在读全局档"
    );
    // ④ 顺序即判据（代码域版）：覆盖发生在 `risk_rank_val` 之前，否则 f4 惩罚强度用不上本档。
    let cover = code.find("overall_risk = risk_tier;").expect("③ 已锁存在");
    let rank = code.find("let risk_rank_val = risk_rank(overall_risk);").expect("f4 强度段应存在");
    assert!(cover < rank, "按档覆盖必须早于 f4 惩罚强度：覆盖={cover} 强度={rank}");
}

/// 「谁定的档」这条值的**三处载体**必须同源：脚本产出的字面量 ⊆ harness 值域，
/// 且值域里每个现役值都真的有人产出。
///
/// 存在理由（2026-10-04 实测的失败形态）：`decision_horizon_source` 的落库白名单原本是
/// `"formula" => "formula", _ => "model"` 的手写清单。脚本换成产出 `branch_pick` 之后，
/// 这个真值会在落库前被静默改写成 `model`（= 采信模型自报）—— **一个真值被换成一句假话，三处都不报错**。本门把「值域 ↔ 产出」的双向覆盖钉死：
///   ① 脚本里出现的每个 `horizonSource` 字面量都必须在 harness 值域内（防脚本自造值）；
///   ② 值域里的**现役**值（非历史）必须能在脚本里找到（防值域与产出一边倒：加了枚举没人产出，
///      面板就会永远不出现该标签，与本仓「永不亮起的分支」同族）。
#[test]
fn horizon_source_literals_are_two_way_covered_by_the_harness_domain() {
    use axagent_harness::holding_period::HORIZON_SOURCES;
    let code = code_only(include_str!("../portfolio-mgr.rhai"));
    // ① 脚本产出的值 ∈ 值域。
    let mut emitted: Vec<&str> = Vec::new();
    for v in HORIZON_SOURCES {
        if code.contains(&format!("\"{v}\"")) {
            emitted.push(v);
        }
    }
    // 兜底与正常路径两条都必须在场（缺一条就是通路断了）。
    for must in ["branch_pick", "formula_no_branch"] {
        assert!(
            HORIZON_SOURCES.contains(&must),
            "harness 值域缺现役值 {must} ⇒ 落库白名单会把它归一成 model（假话）"
        );
        assert!(emitted.contains(&must), "脚本不再产出 {must} ⇒ 该值成了只登记不产出的死枚举");
    }
    // 脚本里不得出现值域外的来源字面量（扫 `let horizon_source = ...` 那一行的引号串）。
    let line =
        code.lines().find(|l| l.contains("let horizon_source =")).expect("定档来源赋值应存在");
    for cap in ["\"formula\"", "\"model\"", "\"user\"", "\"branch_pick\"", "\"formula_no_branch\""]
    {
        if line.contains(cap) {
            let bare = cap.trim_matches('"');
            assert!(
                HORIZON_SOURCES.contains(&bare),
                "脚本产出来源值 {bare} 不在 harness 值域内 ⇒ 会被落库白名单吞掉"
            );
        }
    }
    // 自证（判据①有牙）：合成一个值域外的产出，判据必须能抓到。
    let bad_line = "let horizon_source = if branch_picked { \"gut_feeling\" } else { \"x\" };";
    let leaked =
        ["\"formula\"", "\"model\"", "\"user\"", "\"branch_pick\"", "\"formula_no_branch\""]
            .iter()
            .any(|cap| bad_line.contains(cap));
    assert!(!leaked, "自证失效：合成坏样本里混进了合法值，判据①的样本不纯");
    assert!(
        !HORIZON_SOURCES.contains(&"gut_feeling"),
        "自证失效：gut_feeling 竟在值域内 ⇒ 上面的反向锁没有对象"
    );
}

/// 波动带窗口的**两份载体**必须逐字相等：分支表注入的
/// `axagent_analysis_engine::evidence_weight::VOL_LOOKBACK_DAYS` 与主链脚本里的
/// `let VOL_LOOKBACK_DAYS = …;`。
///
/// 存在理由：四份分支脚本改读注入值之后，主链仍保留自己的脚本内常量（退役动作与图改动同批做）。
/// 两处同名的数字若各自漂移，得到的是「短档用 20 日波动、超短用 30 日波动」这种**档间不一致**，
/// 而两侧都self-consistent ⇒ 没有任何一侧会报错。
#[test]
fn main_chain_vol_lookback_matches_injected_const() {
    let code = include_str!("../portfolio-mgr.rhai");
    let needle = format!(
        "let VOL_LOOKBACK_DAYS = {};",
        axagent_analysis_engine::evidence_weight::VOL_LOOKBACK_DAYS
    );
    assert!(
        code.contains(&needle),
        "主链脚本的波动带窗口不再是 {} 日（或写法变了）⇒ 与分支表注入值分叉，两处会给出不同口径的价带。当前应能在 portfolio-mgr.rhai 找到：{needle}",
        axagent_analysis_engine::evidence_weight::VOL_LOOKBACK_DAYS
    );
}

/// R-11 换心脏的**收口门**：主链脚本 `portfolio-mgr.rhai` 的**代码域**里不得再出现逐档算法
/// 与三个退役输出字段。
///
/// 为什么这条必须在主 crate、且必须剥注释（2026-10-04 实测）：首版把它写在
/// `crates/rt-workflow/tests/portfolio_mgr_tier_params_rhai.rs` 里、按**裸文本**判定，
/// 当场假红 —— 主链留有五段退役说明注释（`:861`、`:2054`、`:2816`、`:2829`、`:2870`、`:2875`），
/// 逐字提到 `leg_mult` / `horizon_decision` / `pm_snr_confidence` / `sharesPosteriorWith`。
/// 那些注释是「这里为什么不再有乘数」的因果留痕，删不得；而禁词门查的应该是**代码**。
/// 本 crate 有模块级 `code_only()`（四份分支脚本的禁词门共用同一实现），rt-workflow 拿不到它，
/// 也不该复制第二份剥离器（铁律 12）。
#[test]
fn main_script_keeps_retired_per_tier_algebra_out_of_code() {
    // 只列**代码域**判据能表达的形态：三个退役输出字段 + 五个退役算法名 + 注入键。
    const BANNED: &[&str] = &[
        "leg_mult",
        "horizon_decision",
        "pm_snr_confidence",
        "prior_for",
        "evidence_max_for",
        "horizon_leg_weights_json",
        "weightsSource",
        "sharesPosteriorWith",
        "snrAnchorDays",
    ];
    let code = include_str!("../portfolio-mgr.rhai");
    let body = code_only(code);

    // 自证①（剥离器没吃代码）：主链是 3200+ 行的脚本，剥注释后必须仍留下可判定的代码体。
    assert!(
        body.lines().count() > 2_000,
        "剥注释后只剩 {} 行 ⇒ 剥离器把代码也吃了，下面的禁词结论不可信",
        body.lines().count()
    );
    for needle in [
        "let decisions_by_horizon = #{};",
        "decisions_by_horizon[b.camel] = row",
        "let risk_source = \"算法\"",
        "sl_pct_for.call(",
    ] {
        assert!(
            body.contains(needle),
            "剥注释后连 `{needle}` 都没了 ⇒ 扫描面已不是代码，本门的禁词断言无意义"
        );
    }

    for b in BANNED {
        assert!(
            !body.contains(b),
            "主链**代码里**出现退役形态 `{b}` ⇒ 「一个算法 + 四套参数」或退役字段回归（逐档算法应在 portfolio-mgr-h-*.rhai）"
        );
    }

    // 正向：装配段确实读四路分支输出（只查禁词的话，把整段装配删掉也能过门）。
    for src in ["h_ultra_short", "h_short", "h_mid", "h_long"] {
        assert!(body.contains(src), "主链代码不再读 {src} ⇒ 该路分支输出没接进来");
    }

    // 自证②（门的扫描面是代码而不是全文）：同一个禁词写成注释必须**不**命中。
    let comment_only =
        code_only("let keep = 1;\n// leg_mult(\"mid\", \"f5\")\n/* horizon_decision */");
    assert!(
        !comment_only.contains("leg_mult") && !comment_only.contains("horizon_decision"),
        "剥离器没起作用 ⇒ 上面的禁词门会退化成「注释里不许提到退役形态」"
    );
    // 自证③（坏样本必须红）：把退役调用写进代码，判据必须抓得住，否则禁词是恒假断言。
    let bad = code_only(&format!("{code}\nlet __probe = leg_mult(\"mid\", \"f5\");"));
    assert!(bad.contains("leg_mult"), "负控失效：退役形态进了代码也查不出来 ⇒ 本门恒真");
}

/// P5 仲裁节点 `portfolio-mgr-arbiter.rhai` 的**运行门**（存在理由同 §28：Rhai 编译期
/// 既不解析函数名也不查变量，`pm_kelly_growth` 的签名对不上要等真跑才发现）。
///
/// 四条裁定各钉一遍：
/// ① 可比档按「每日对数增长率」取最大 ⇒ **短档凭时间效率胜出**，证明排名不是「看谁仓位大」；
/// ② 缺席档进 `absentTiers` 且**不参与比较**（不得被当成 0 增长 = 看空）；
/// ③ 有结论但零仓位 ⇒ `ineligibleTiers`，与缺席**分列**；
/// ④ 一档都不入选 ⇒ `primaryTier` 为空串 + 说明原因，**不硬选**。
#[test]
fn arbiter_rhai_executes_and_never_scores_absence_as_bearish() {
    use axagent_rt_workflow::expression_engine::rhai_eval::value_to_dynamic;
    use rhai::{Dynamic, Map, Scope};
    use serde_json::json;

    let code = include_str!("../portfolio-mgr-arbiter.rhai");
    let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);
    let ast = engine.compile(code).expect("arbiter 脚本应在生产同配置下编译通过");

    let branch = |tier: &str, conf: f64, pos: f64, days: i64| {
        json!({
            "horizon": tier,
            "action": if pos > 0.0 { "买入" } else { "观望" },
            "confidence": conf,
            "odds": 1.0,
            "positionPct": pos,
            "expectedHoldingDays": days,
        })
    };
    let eval = |injected: &[(&str, &serde_json::Value)]| -> Map {
        let mut scope = Scope::new();
        for name in ["r_ultra_short", "r_short", "r_mid", "r_long"] {
            scope.push_constant(name, Dynamic::UNIT);
        }
        for (k, v) in injected {
            scope.push_constant(*k, value_to_dynamic(v));
        }
        engine
            .eval_ast_with_scope::<Map>(&mut scope, &ast)
            .unwrap_or_else(|e| panic!("arbiter 运行失败：{e}"))
    };
    let text = |m: &Map, k: &str| -> String {
        m.get(k).and_then(|v| v.clone().try_cast::<String>()).unwrap_or_default()
    };
    let list_len = |m: &Map, k: &str| -> usize {
        // 缺键必须炸，不能静默算 0 —— 否则「契约名拼错」与「确实没有缺席档」在断言里长得一样
        // （本门第一版就是这么漏掉 absent/ineligible 两个键名的）。
        let v = m
            .get(k)
            .unwrap_or_else(|| panic!("arbiter 输出里没有键 {k} ⇒ 契约名拼错或脚本改了输出形状"));
        v.clone()
            .try_cast::<rhai::Array>()
            .unwrap_or_else(|| panic!("arbiter 的键 {k} 不是数组"))
            .len()
    };

    // ① 四档齐备：同胜率同赔率下，锁 2 天的 8% 仓胜过锁 90 天的 30% 仓（时间归一生效）
    let all_four = eval(&[
        ("r_ultra_short", &branch("ultra_short", 66.0, 8.0, 2)),
        ("r_short", &branch("short", 66.0, 10.0, 5)),
        ("r_mid", &branch("mid", 66.0, 20.0, 28)),
        ("r_long", &branch("long", 66.0, 30.0, 90)),
    ]);
    assert_eq!(
        text(&all_four, "primaryTier"),
        "ultra_short",
        "① 排名应看单位时间增长：{all_four:?}"
    );
    assert_eq!(text(&all_four, "coverage"), "4/4");
    assert_eq!(text(&all_four, "basis"), "kelly_growth_rate_per_holding_day");
    assert_eq!(text(&all_four, "scope"), "capital_allocation_only", "仲裁只做资金投向（裁定 Q3）");

    // ② 超短缺席 ⇒ 记 absent，并在其余三档里选（缺席不得变成最低分）
    let miss_one = eval(&[
        ("r_short", &branch("short", 66.0, 10.0, 5)),
        ("r_mid", &branch("mid", 66.0, 20.0, 28)),
        ("r_long", &branch("long", 66.0, 30.0, 90)),
    ]);
    assert_eq!(list_len(&miss_one, "absentTiers"), 1, "② 应恰有一档缺席：{miss_one:?}");
    assert_eq!(text(&miss_one, "primaryTier"), "short", "② 缺席档不得参与胜出");
    assert_eq!(text(&miss_one, "coverage"), "3/4");

    // ③ 四档全在、但长线零仓位 ⇒ ineligible 恰 1、absent 必须 0（两类语义分列），
    //    胜出仍是超短 —— 若把「零仓位」错算成缺席，absent 会变 1，本断言立刻抓到。
    let with_zero = eval(&[
        ("r_ultra_short", &branch("ultra_short", 66.0, 8.0, 2)),
        ("r_short", &branch("short", 66.0, 10.0, 5)),
        ("r_mid", &branch("mid", 66.0, 20.0, 28)),
        ("r_long", &branch("long", 66.0, 0.0, 90)),
    ]);
    assert_eq!(list_len(&with_zero, "ineligibleTiers"), 1, "③ 零仓位应判不可比：{with_zero:?}");
    assert_eq!(list_len(&with_zero, "absentTiers"), 0, "③ 四路都注入了 ⇒ 不该有任何档算缺席");
    assert_eq!(text(&with_zero, "primaryTier"), "ultra_short");
    assert_eq!(text(&with_zero, "coverage"), "3/4");

    // ④ 全缺席 ⇒ 不硬选
    let nothing = eval(&[]);
    assert_eq!(text(&nothing, "primaryTier"), "", "④ 无可仲裁时不得凭空指定：{nothing:?}");
    assert!(text(&nothing, "reason").contains("不指定投向"), "④ 原因要点名：{nothing:?}");
    assert_eq!(text(&nothing, "coverage"), "0/4");
}

/// 剥掉注释后再判定 —— 从 `horizon_branch_rhai_scripts_compile_and_are_genuinely_forked`
/// 内部提到模块级，本文件多个门共用**同一份**剥离实现（铁律 12：同一算法两处实现迟早漂移）。
/// 局限：不处理字符串字面量里的 `//`（这些脚本与映射区块里都没有 URL，扩用前先加词法）。
fn code_only(src: &str) -> String {
    let no_line: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::with_capacity(no_line.len());
    let mut depth = 0usize;
    let chars: Vec<char> = no_line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            depth += 1;
            i += 2;
            continue;
        }
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' && depth > 0 {
            depth -= 1;
            i += 2;
            continue;
        }
        if depth == 0 {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

/// **分支节点的注入面必须恰好等于脚本运行时要的自由变量面**（2026-10-04 实测缺陷的锁）。
///
/// 为什么前一条运行门拦不住（同日实锤）：`horizon_branch_rhai_scripts_execute_end_to_end`
/// 的 scope 来自**测试自己手写的 24 项清单**，而生产里 `pm-h-ultra-short` 只注入 11 项 ⇒
/// 脚本读 `catalyst_level` / `flow_persistence` / `pool_in_pool` 这三个**节点没映射**的名字时，
/// 测试里它们是 unit（顺利通过），生产里是 `Variable not found` ⇒ **整节点失败**
/// （`code_executor.rs:184` 只把 `input_mapping` 的 key 推进 scope；「解析不到才是 unit」
/// 说的是**映射过的键**，没映射的键连名字都不存在）。
/// 也就是说「未接线 = 诚实缺席」这个前提，在宽 scope 的测试里成立、在生产里不成立 ——
/// 而它正是 R-11 分支最容易犯的形态（四份脚本各写一遍，越界读取各不相同）。
///
/// 本门把 scope 的**唯一来源换成种子**：解析该节点 `input_mapping` 区块的键集合，
/// 只用这些键构造 scope，跑生产同一个引擎工厂 + 同一个沙箱档位 ⇒ 越界读取当场红。
/// 内置负控：少注入一个脚本真读的名字 ⇒ 必须**运行失败**，证明「红」来自 scope 而不是别的。
///
/// ⚠ 覆盖面限定（不自夸）：Rhai 的变量查找发生在**运行时**，只有执行到的路径上的越界会报；
/// 未被走到的分支里的越界读取仍会漏。静态穷查需要词法器 —— 那条门已因自证失败被删
/// （PLAN §三十 30-3），这里不重犯，只掐掉「测试 scope ⊋ 生产 scope」这个假绿来源。
#[test]
fn branch_node_scope_is_exactly_the_seed_mapping() {
    use axagent_rt_workflow::expression_engine::rhai_eval::value_to_dynamic;
    use rhai::{Dynamic, Map, Scope};

    let seed = include_str!("seed_stock_analysis.rs");
    // v135（B-2b #36）：四个 `pm-h-<档>` 分支节点已从主图搬进档子模板 builder ⇒ 本门抽映射的
    // **载体**换成那个文件（判据本体没变：Rhai scope 必须恰好等于节点 `input_mapping` 的键集）。
    // 搬的是节点声明，不是这条判据 —— 若还按主图找，`include_str!` 锚定位会直接 panic，
    // 也就是「门找不到对象」而不是「门放行」（§九十一(3) 步骤 3 要求门与本批同批改）。
    let tier_builder = include_str!("horizon_tier_template.rs");

    /// 从种子里抽出某节点 `input_mapping: [ … ]` 区块的 **target 键名**。
    /// 做法：定位该节点 `include_str!` 行 ⇒ **先剥注释** ⇒ 取到 `input_mapping: [` ⇒
    /// 找闭合 `]` ⇒ 抽字符串字面量 ⇒ 奇偶交替即 (target, source)。
    /// `required_key` 是**该形态节点必接的键**（分支节点 = `branch_json`，仲裁节点 = 四路之一的
    /// `r_ultra_short`）：抽不到就说明区块被提前截断，让解析失败在**解析期**暴露，
    /// 而不是变成一条指向生产代码的假缺陷。
    /// 两个次序都是实测换来的：
    /// ① 不用正则 —— rustfmt 会把长名元组拆成跨行（本文件另一处门为此踩过三次），
    ///    而字面量序列不受换行影响；
    /// ② **必须在找 `]` 之前剥注释** —— 映射区块上方的说明里写过 `#[serde(rename_all="…")]`
    ///    这种**自带 `]` 的 Rust 属性示例**，拿原文找闭合会在区块中途截断。
    ///    本门首跑正是这样少收了 `seal_rate`/`pool_break_count`，反过来把**门自己的解析缺陷**
    ///    报成「节点没注入」⇒ 见下面的三条结构自证。
    fn mapping_keys(
        seed: &str,
        basename: &str,
        required_key: &str,
        min_keys: usize,
    ) -> Vec<String> {
        let anchor = format!("include_str!(\"../{basename}\")");
        let start = seed
            .find(&anchor)
            .unwrap_or_else(|| panic!("种子里找不到 {basename} 的节点区块（接线被删？）"));
        let rest = code_only(&seed[start..]);
        let open_mark = "input_mapping: [";
        let open = rest
            .find(open_mark)
            .unwrap_or_else(|| panic!("{basename} 的节点不是 `{open_mark}` 形态"));
        let after = &rest[open + open_mark.len()..];
        let close = after.find(']').unwrap_or_else(|| panic!("{basename} 的 input_mapping 未闭合"));
        let block = &after[..close];
        let mut lits: Vec<String> = Vec::new();
        let chars: Vec<char> = block.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] == '"' {
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                lits.push(s);
            }
            i += 1;
        }
        assert!(
            lits.len().is_multiple_of(2),
            "{basename} 的映射区块里字符串数是奇数 ⇒ 解析姿势不对（区块里混进了非配对字面量），实得 {lits:?}"
        );
        let keys: Vec<String> = lits.into_iter().step_by(2).collect();
        // 结构自证三条（缺一条本门就可能是「扫到半个区块」却报成功）：
        // ① 该形态节点必接的键必须抽得到 ⇒ 少它就是区块被提前截断；
        // ② 键必须像 Rust 标识符 ⇒ 抽到注释残渣/属性片段会当场红；
        // ③ 键数下限（分支节点 ≥ 8；仲裁节点由调用方自己给下限）。
        assert!(
            keys.iter().any(|k| k == required_key),
            "{basename} 的映射里抽不到 {required_key:?} ⇒ 区块被提前截断或形态变了，实得 {keys:?}"
        );
        for k in &keys {
            assert!(
                k.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                    && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{basename} 的映射键 {k:?} 不是小写标识符 ⇒ 解析器抓到了非键内容（注释/属性/路径）"
            );
        }
        assert!(
            keys.len() >= min_keys,
            "{basename} 的映射键只有 {} 个（下限 {min_keys}）⇒ 区块解析跑偏：{keys:?}",
            keys.len()
        );
        keys
    }

    let branch_all = axagent_analysis_engine::evidence_weight::horizon_branch_specs();
    // 退化态用的**空先验表**（必须是具名绑定，不能内联 `&json!({})` —— 那会在借用仍存活时被释放）
    let empty_prior_table = serde_json::json!({});
    let prior_table = serde_json::json!({
        "ultra_short": { "prior": 0.5, "source": "gate_hitrate", "samples": 12.0 },
        "short": { "prior": 0.5, "source": "gate_hitrate", "samples": 12.0 },
        "mid": { "prior": 0.5, "source": "gate_hitrate", "samples": 12.0 },
        "long": { "prior": 0.5, "source": "gate_hitrate", "samples": 12.0 },
    });
    // 夹具必须是**有离散度**的价格序列：单调等差数列的日收益率标准差只有 1e-5 量级，
    // `k·σ·√h` 四舍五入到两位小数后恒为 0 ⇒ 「价带算出来了」这条断言会失去区分力。
    let bars: serde_json::Value = (0..25)
        .map(|i| {
            let cyc = [10.0_f64, 10.42, 10.08, 10.63, 10.21][i as usize % 5];
            serde_json::json!({ "close": cyc + (i as f64) * 0.01 })
        })
        .collect();
    let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);
    const ACTIONS: &[&str] = &["买入", "增持", "持有", "观望", "减持", "卖出"];

    // 已接线的档：(档位, 脚本文件, 负控要剥的那个名字)。新增接线时在这里加一行 —— 漏加会在下面的
    // match 分支**点名**是哪个文件没登记源，而不是静默少测一档。
    // `peel` 必须是该档脚本**无条件执行到的读取**（都在 `raw`/门判定里），否则剥了也不报错，
    // 负控会退化成空跑。
    let wired: [(&str, &str, &str); 4] = [
        ("ultra_short", "portfolio-mgr-h-ultra-short.rhai", "seal_rate"),
        ("short", "portfolio-mgr-h-short.rhai", "seal_rate"),
        ("mid", "portfolio-mgr-h-mid.rhai", "pe_percentile"),
        ("long", "portfolio-mgr-h-long.rhai", "valuation_dcf_upside"),
    ];

    for (tier, basename, peel) in wired {
        let code = match basename {
            "portfolio-mgr-h-ultra-short.rhai" => {
                include_str!("../portfolio-mgr-h-ultra-short.rhai")
            },
            "portfolio-mgr-h-short.rhai" => include_str!("../portfolio-mgr-h-short.rhai"),
            "portfolio-mgr-h-mid.rhai" => include_str!("../portfolio-mgr-h-mid.rhai"),
            "portfolio-mgr-h-long.rhai" => include_str!("../portfolio-mgr-h-long.rhai"),
            other => panic!("分支脚本 {other} 未在本门登记源，无法按节点 scope 运行"),
        };
        let keys = mapping_keys(tier_builder, basename, "branch_json", 8);

        // 「齐备态」的值表：每个键都要有真值样本。新接一个键却没在这里登记 ⇒ 红，
        // 否则「齐备态」会悄悄变成「那个键其实是 unit」，测的已经不是齐备输入。
        fn value_for(k: &str) -> Option<serde_json::Value> {
            Some(match k {
                "tier_score" => serde_json::json!(62.0),
                "macd_dif" => serde_json::json!(0.12),
                "macd_dea" => serde_json::json!(0.04),
                "rsi_value" => serde_json::json!(58.0),
                "seal_rate" => serde_json::json!(0.72),
                "pool_break_count" => serde_json::json!(0.0),
                "stop_vol_mult" => serde_json::json!(1.2),
                "take_profit_vol_mult" => serde_json::json!(2.0),
                "pe_percentile" => serde_json::json!(18.0),
                "f_score" => serde_json::json!(7.0),
                "consensus_eps" => serde_json::json!(3.0),
                // 齐备态必须是**非估算**的一致预期，否则中/长档的脚本会正确地按缺席处理
                "consensus_estimated" => serde_json::json!(false),
                // #24（v131）：预期修正腿的**分母**（年报口径 EPS）与它的缺席原因码。
                // 齐备态必须给「有值 + basis=annual_report」形态，否则本门测的就不是齐备输入。
                "latest_eps" => serde_json::json!(1.2),
                "latest_eps_basis" => serde_json::json!("annual_report"),
                // #23（v132）：解禁供给占比（小数）。齐备态必须给**有值**形态；
                // `lockup_supply_reason` 只在缺席时才有意义，这里给一个真值是为了
                // 「齐备态 = 每个映射键都有料」这条不缩水（它在齐备态里不参与任何分支，
                // 正因如此才更要登记 —— 否则脚本那条条件式从没被求值过）。
                "lockup_float_ratio" => serde_json::json!(0.03),
                "lockup_supply_reason" => serde_json::json!("no_float_market_cap"),
                "flow_persistence" => serde_json::json!(0.4),
                "pmi" => serde_json::json!(51.0),
                "valuation_dcf_upside" => serde_json::json!(38.0),
                "valuation_dcf_applicable" => serde_json::json!(true),
                // v138（裁定 3「让用户看出各档实际几根」）：本档评分节点的**窗口回显**与尺度。
                // 齐备态必须给真实形态（字段名 = `indicators::IndicatorWindows` 的 serde camelCase 输出），
                // 否则脚本里那条 `present(scoring_windows)` 分支从未被求值过 —— 与本门登记
                // `lockup_supply_reason` 同一条理由：正因为它在齐备态里不参与任何分支，才更要登记。
                "scoring_windows" => serde_json::json!({
                    "maPeriods": [2, 6],
                    "macdFast": 2,
                    "macdSlow": 3,
                    "macdSignal": 2,
                    "rsiPeriods": [2],
                    "bollPeriod": 2,
                    "volumeLookback": 2,
                }),
                // `scoring_scale` **不在这里登记**：它由 `run_with` 按档给本权威尺度（见那里）。
                // 留一个固定串在这里就是一份「看着像数据源其实永远不会被读到」的第二副本。
                _ => return None,
            })
        }

        // v139：逐档**可区分**的夹具索引 —— 两带/动量的数值随档变，脚本若硬编码或串档
        // （读成别档那一格），另外三档的断言当场就红。声明在闭包**外**：
        // 齐备态断言也要用它，写在 `run_with` 里面就只是夹具的局部变量。
        let tier_idx = axagent_harness::holding_period::Period::ALL
            .into_iter()
            .position(|p| p.as_str() == tier)
            .expect("档名必须落进 Period::ALL");
        let run_with = |names: &[String], full: bool| -> Result<Map, String> {
            let mut scope = Scope::new();
            let mut unhandled: Vec<&str> = Vec::new();
            for name in names {
                let dyn_value = match name.as_str() {
                    "branch_json" => value_to_dynamic(&branch_all[tier]),
                    // 退化态故意给**空表**：脚本必须把「表在但没有本档那行」也判成先验不可得
                    "horizon_prior_json" => value_to_dynamic(if full {
                        &prior_table
                    } else {
                        &empty_prior_table
                    }),
                    "overall_risk" => Dynamic::from("中风险"),
                    // v138 裁定 3：尺度给**本档权威的那一个**（`Period::scale_key`），不是固定串。
                    // 脚本若把尺度硬编码成某档的值，另外三档的断言当场就红 —— 这条是逐档 passthrough 的锁。
                    "scoring_scale" => {
                        if full {
                            Dynamic::from(
                                axagent_harness::holding_period::Period::ALL
                                    .into_iter()
                                    .find(|p| p.as_str() == tier)
                                    .map(|p| p.scale_key())
                                    .unwrap_or_else(|| panic!("档 {tier} 不在 Period 权威表里")),
                            )
                        } else {
                            // ⚠ 退化态必须给 UNIT：齐备与退化共用一份料 = 本门的「退化留痕」覆盖面
                            //   被这条夹具悄悄放宽，什么都没测到。
                            Dynamic::UNIT
                        }
                    },
                    "kline_bars" => value_to_dynamic(&bars),
                    // v139 呈现层补齐：两带与动量对象按档给**不同数值**（退化态同样给 UNIT）。
                    "scale_trend" => {
                        if !full {
                            Dynamic::UNIT
                        } else {
                            value_to_dynamic(&serde_json::json!({
                                "fastBars": 2 + tier_idx,
                                "slowBars": 6 + 3 * tier_idx,
                                "fast": 11.5 + tier_idx as f64,
                                "slow": 10.25 + tier_idx as f64,
                                "diffPct": 12.5 + tier_idx as f64,
                                "fastSlope": 0.5 + tier_idx as f64,
                            }))
                        }
                    },
                    "scale_momentum" => {
                        if !full {
                            Dynamic::UNIT
                        } else {
                            value_to_dynamic(&serde_json::json!({
                                "period": 2 + tier_idx,
                                "value": 44.0 + tier_idx as f64,
                            }))
                        }
                    },
                    other => match (full, value_for(other)) {
                        (true, Some(v)) => value_to_dynamic(&v),
                        (false, _) => Dynamic::UNIT,
                        (true, None) => {
                            unhandled.push(other);
                            Dynamic::UNIT
                        },
                    },
                };
                scope.push_constant(name.as_str(), dyn_value);
            }
            assert!(
                unhandled.is_empty(),
                "档 {tier} 有映射键在本门的值表里登记不到：{unhandled:?} ⇒ 「齐备态」其实缺料，先补值表再改门"
            );
            let ast = engine.compile(code).expect("分支脚本应可编译");
            engine.eval_ast_with_scope::<Map>(&mut scope, &ast).map_err(|e| format!("{e}"))
        };

        // ① 齐备态：按**节点真实 scope** 必须跑通，且形状与上一门一致
        let out = run_with(&keys, true).unwrap_or_else(|e| {
            panic!(
                "档 {tier} 按节点真实 scope（{} 个键）运行失败：{e}\n键集合 {keys:?}",
                keys.len()
            )
        });
        let action =
            out.get("action").and_then(|v| v.clone().try_cast::<String>()).unwrap_or_default();
        assert!(ACTIONS.contains(&action.as_str()), "档 {tier} 动作不在六档词表：{action:?}");
        assert_eq!(
            out.get("horizon").and_then(|v| v.clone().try_cast::<String>()).as_deref(),
            Some(tier),
            "档 {tier} 自报档位不符"
        );
        let legs =
            out.get("legs").and_then(|v| v.clone().try_cast::<rhai::Array>()).unwrap_or_default();
        assert!(!legs.is_empty(), "档 {tier} 齐备态一条腿都没有 ⇒ 腿名与分支表漂移");
        // ①′ #24（v131）：中/长档齐备态必须**真的有** `expectationRevision` 这条腿。
        //     v130 及以前它在两档恒缺席（分母没有数据面），所以这条在改前是红的 ——
        //     它锁的是「分母接到了」这件事本身，而不是这条腿最后贡献多少。
        if matches!(tier, "mid" | "long") {
            let has_er = legs.iter().any(|l| {
                l.clone()
                    .try_cast::<Map>()
                    .and_then(|m| m.get("factor").cloned().and_then(|f| f.try_cast::<String>()))
                    .as_deref()
                    == Some("expectationRevision")
            });
            assert!(
                has_er,
                "档 {tier} 齐备态没有 expectationRevision 腿 ⇒ latestEps 没进 raw，\
                 或 latest_eps 的映射路径断了"
            );
        }

        // ①″ #23（v132）：short 与 mid 齐备态必须**有** `supplyShock` 这条腿（超短/长档腿集里没有 ⇒ 不查）。
        //     short 是方向腿、mid 是 riskNote，两者都要求「取到数」—— 旧形态下这条腿要么恒缺席，
        //     要么（主链那一条）恒满负，两种都是常数而不是证据。
        if matches!(tier, "short" | "mid") {
            let has_ss = legs.iter().any(|l| {
                l.clone()
                    .try_cast::<Map>()
                    .and_then(|m| m.get("factor").cloned().and_then(|f| f.try_cast::<String>()))
                    .as_deref()
                    == Some("supplyShock")
            });
            assert!(has_ss, "档 {tier} 齐备态没有 supplyShock 腿 ⇒ supply_shock 块没产出入，");
        }
        // ①‴ v138 裁定 3「让用户看出各档实际几根」：齐备态必须把**本档自己的**窗口回显与尺度
        //     原样带进决策行。锁两件事：
        //     ① 尺度逐档不同 —— 夹具给的是本档权威 `Period::scale_key()`，脚本若硬编码某档的值，
        //        另外三档当场红（这一条同时证明「注入 → 回写」这条链真的通）；
        //     ② 窗口对象真的穿过脚本 —— Rhai 不能枚举 scope/map 的键，键名写错就是**静默缺席**
        //        （§四十九 那条限制），所以必须按形状断言而不是只看它跑不跑。
        let scale_out = out
            .get("scoringScale")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();
        let want_scale = axagent_harness::holding_period::Period::ALL
            .into_iter()
            .find(|p| p.as_str() == tier)
            .map(|p| p.scale_key())
            .unwrap_or_default();
        assert_eq!(
            scale_out.as_str(),
            want_scale,
            "档 {tier} 回写的尺度不是本档权威尺度（产端应是回显，不是常量）"
        );
        let win =
            out.get("scoringWindows").and_then(|v| v.clone().try_cast::<Map>()).unwrap_or_else(
                || panic!("档 {tier} 齐备态没带出 scoringWindows 对象 ⇒ 键名或透传断了"),
            );
        let ma = win.get("maPeriods").and_then(|v| v.clone().try_cast::<rhai::Array>());
        assert!(
            ma.map(|a| !a.is_empty()).unwrap_or(false),
            "档 {tier} 的 scoringWindows.maPeriods 为空 ⇒ 窗口根数读不出来，面板那一行无从成句"
        );
        // v139 呈现层补齐：两带与动量的**数值**必须逐档原样带出（夹具随档变 ⇒ 硬编码/串档都红）。
        // 取值器两个数值型都收（产端 `usize` ⇒ JSON 整数 ⇒ Rhai i64；`f64` ⇒ f64），
        // 本条锁的是「逐档透传」而不是数值类型 —— 类型口径由 `check-rhai-numeric-typing.mjs`
        // 与上面那条 windows 断言各自覆盖，这里放宽不会掩盖它们。
        let dyn_num = |v: &rhai::Dynamic| -> Option<f64> {
            v.clone().try_cast::<f64>().or_else(|| v.clone().try_cast::<i64>().map(|i| i as f64))
        };
        let row_map = |k: &str| out.get(k).and_then(|v| v.clone().try_cast::<Map>());
        let want_slow_bars = 6.0 + 3.0 * tier_idx as f64;
        let want_diff_pct = 12.5 + tier_idx as f64;
        let want_mom_value = 44.0 + tier_idx as f64;
        let trend = row_map("scoringTrend")
            .unwrap_or_else(|| panic!("档 {tier} 齐备态没带出 scoringTrend 对象 ⇒ 键名或透传断了"));
        assert_eq!(
            trend.get("slowBars").and_then(dyn_num),
            Some(want_slow_bars),
            "档 {tier} 的慢带根数不是本档那份（夹具随档变 ⇒ 红即串档/硬编码）"
        );
        assert_eq!(
            trend.get("diffPct").and_then(dyn_num),
            Some(want_diff_pct),
            "档 {tier} 的两带差不等于本档夹具值 ⇒ 数值被改写或漏传"
        );
        let mom = row_map("scoringMomentum")
            .unwrap_or_else(|| panic!("档 {tier} 齐备态没带出 scoringMomentum 对象"));
        assert_eq!(
            mom.get("value").and_then(dyn_num),
            Some(want_mom_value),
            "档 {tier} 的动量值不是本档夹具值 ⇒ 与 rsi_value 那条不再是同一个数"
        );
        // 以下四条从被删除的「手抄 scope 运行门」迁移过来（那条门的 scope 比生产宽，是假绿来源；
        // 独有覆盖不能跟着删 ⇒ 见 4f 的「删 + 交代」规矩）：置信值域 / 缺席点名 / 退化留痕 / 点名先验。
        let conf = out.get("confidence").and_then(|v| v.clone().try_cast::<f64>()).unwrap_or(-1.0);
        assert!((0.0..=100.0).contains(&conf), "档 {tier} 置信越出 0-100：{conf}");

        // ①′ 齐备态必须**真的算出波动率价带**（2026-10-04 实证：000710 四档的
        //     stopLossPct / takeProfitPct / odds / positionPct 全为 0，而 `stopSource` 谎报
        //     `fallback_pct` —— 固定百分比兜底其实从未参与）。根因：分支表的 `days` /
        //     `volLookbackDays` 是 JSON **整数**，经 `json_value_to_dynamic` 落成 Rhai i64，
        //     而脚本只认 "f64" ⇒ band_* 恒 0 ⇒ 宿主调用被整段跳过。这类「按类型漏判」
        //     编译门与旧断言（只看形状）都查不出 ⇒ 必须按**数值**断。
        let band_of =
            |k: &str| out.get(k).and_then(|v| v.clone().try_cast::<f64>()).unwrap_or(-1.0);
        let (g_stop, g_take, g_odds) =
            (band_of("stopLossPct"), band_of("takeProfitPct"), band_of("odds"));
        assert!(
            g_stop > 0.0 && g_take > 0.0 && g_odds > 0.0,
            "档 {tier} 齐备态价带没算出来（止损 {g_stop} / 止盈 {g_take} / 赔率 {g_odds}）\
             ⇒ 波动率口径整段落空，仓位必然恒 0"
        );
        let g_src =
            out.get("stopSource").and_then(|v| v.clone().try_cast::<String>()).unwrap_or_default();
        assert_ne!(
            g_src, "fallback_pct",
            "档 {tier} 齐备态却走了固定百分比兜底 ⇒ 上方数值断言与 stopSource 说的是两件事"
        );
        // 前提自证：夹具给的是**生产形态**的分支表（整数天数）。哪天产出端把 days 改成浮点，
        // 这条会红 —— 那意味着本门不再覆盖 i64 分支，而不是判据可以删。
        assert!(
            value_to_dynamic(&branch_all[tier]["days"]).is_int()
                && value_to_dynamic(&branch_all[tier]["volLookbackDays"]).is_int(),
            "档 {tier} 分支表的 days/volLookbackDays 不再是 JSON 整数 ⇒ 本门的 i64 判据覆盖失效，\
             须换样本而不是删断言"
        );

        // ② 退化态（全 unit，只给分支表 + 空先验表）：拒绝出结论、不产仓位
        let empty =
            run_with(&keys, false).unwrap_or_else(|e| panic!("档 {tier} 退化态运行失败：{e}"));
        assert_eq!(
            empty.get("action").and_then(|v| v.clone().try_cast::<String>()).unwrap_or_default(),
            "观望",
            "档 {tier} 无腿无先验却给出非观望动作：{empty:?}"
        );
        let pos =
            empty.get("positionPct").and_then(|v| v.clone().try_cast::<f64>()).unwrap_or(99.0);
        assert!(pos <= 0.0, "档 {tier} 无输入仍给出仓位 {pos} ⇒ 伪装有结论");
        let absent = empty
            .get("absentLegs")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .unwrap_or_default();
        assert!(!absent.is_empty(), "档 {tier} 全无输入却不报任何缺席腿 ⇒ 缺席被当成了中性");
        let gaps = empty
            .get("dataGaps")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .unwrap_or_default();
        assert!(!gaps.is_empty(), "档 {tier} 退化时必须在 dataGaps 留痕");
        assert!(
            gaps.iter().any(|g| g.clone().try_cast::<String>().is_some_and(|s| s.contains("先验"))),
            "档 {tier} 退化原因必须点名「先验不可得」，实得 {gaps:?}"
        );

        // ③ 负控（本门自证）：剥掉一个脚本真读的名字 ⇒ 必须**运行失败**且点名它。
        //    若这里也跑通，说明门测的不是节点 scope（或脚本根本不读它），本门对该档就是空的。
        let narrowed: Vec<String> = keys.iter().filter(|&k| k != peel).cloned().collect();
        assert_eq!(
            narrowed.len(),
            keys.len() - 1,
            "负控失效：节点映射里没有 `{peel}` 可剥，键集合 {keys:?}"
        );
        let err = run_with(&narrowed, true).err();
        assert!(err.is_some(), "剥掉 `{peel}` 后仍跑通 ⇒ 越界读取查不出来，本门对档 {tier} 无效");
        let msg = err.unwrap_or_default();
        assert!(msg.contains(peel), "负控报的不是被剥掉的名字，可能被别的错误掩盖：{msg}");
    }

    // ── 仲裁节点（P5）：同一个判据，换一份 scope 来源 ──
    // 它只读四路分支的 `result` ⇒ 「节点映射 = 脚本要的自由变量面」对它同样成立。
    // 产出形状与分支不同（没有 legs/action 词表那一套），所以这里只验两件事：
    //   ① 四路全不注入值时**不硬选**（`primaryTier` 必须是空串）——缺席不是看空；
    //   ② 剥掉其中一路 ⇒ 必须**运行失败且点名那一路**（负控，证明门有电池）。
    let arbiter_keys = mapping_keys(seed, "portfolio-mgr-arbiter.rhai", "r_ultra_short", 4);
    let arbiter_code = include_str!("../portfolio-mgr-arbiter.rhai");
    let mut arbiter_scope_names: Vec<&str> = arbiter_keys.iter().map(String::as_str).collect();
    arbiter_scope_names.sort();
    assert_eq!(
        arbiter_scope_names,
        vec!["r_long", "r_mid", "r_short", "r_ultra_short"],
        "仲裁节点的注入面应当恰好是四路分支输出，实得 {arbiter_keys:?}"
    );
    let run_arbiter = |names: &[String]| -> Result<Map, String> {
        let mut scope = Scope::new();
        for name in names {
            // 四路全部按「没接到」注入 unit ⇒ 脚本必须判缺席，而不是拿 0 参与比较
            scope.push_constant(name.as_str(), Dynamic::UNIT);
        }
        let ast = engine.compile(arbiter_code).expect("仲裁脚本应可编译");
        engine.eval_ast_with_scope::<Map>(&mut scope, &ast).map_err(|e| format!("{e}"))
    };
    let all_absent = run_arbiter(&arbiter_keys).expect("四路缺席时仲裁必须能跑完，而不是崩");
    assert_eq!(
        all_absent
            .get("primaryTier")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default(),
        "",
        "四路都缺席却指定了投向 ⇒ 拿「没数据」当成了结论：{all_absent:?}"
    );
    let peeled: Vec<String> = arbiter_keys.iter().filter(|&k| k != "r_mid").cloned().collect();
    let arbiter_err = run_arbiter(&peeled).err();
    assert!(arbiter_err.is_some(), "剥掉 `r_mid` 后仲裁仍跑通 ⇒ 本门对仲裁节点是空的");
    assert!(arbiter_err.unwrap_or_default().contains("r_mid"), "仲裁负控没点名被剥的那一路");
}

// ─────────────────────────────────────────────────────────────────────────────
// v133（B2-2）：analyst-brief 的键集锁 —— 脚本 21 条显式读取 == seed 生成的映射键集。
// v145：21 = 23 − 2（`value-investor--mid` / `--long` 不参与，理由见下方 want 的 filter）。
// ─────────────────────────────────────────────────────────────────────────────

/// `analyst-brief.rhai` 的 21 条 `present(<var>)` 读取（变量名 = input_mapping 的键）
/// 必须与 seed 侧 `ab_input` 生成的键集**同集**。
///
/// 为什么需要这条锁：Rhai 不能枚举 scope 变量 ⇒ 脚本侧「显式列 21 条」与 Rust 侧
/// 「由 `tiered` 生成」是**两份**必须同步的清单；任一侧增删实例而另一侧没跟，
/// 后果是静默少一段摘要（脚本读注入不存在的变量走 present=false 跳过；Rust 多生成的键
/// 则白注入）——正是本仓「清单比表少一行」的老形态。该锁让不同步当场红。
#[test]
fn analyst_brief_keys_match_tiered_instances() {
    use axagent_harness::holding_period::Period;

    // ① 脚本侧：抽 `present(<ident>)` 的实参（跳过函数形参 `x`）。
    let script = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/analyst-brief.rhai"),
    )
    .expect("读取 analyst-brief.rhai 失败");
    let mut script_vars: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut rest = script.as_str();
    while let Some(i) = rest.find("present(") {
        let after = &rest[i + "present(".len()..];
        let Some(j) = after.find(')') else { break };
        let name = after[..j].trim();
        if !name.is_empty()
            && name != "x"
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            script_vars.insert(name.to_string());
        }
        rest = &after[j..];
    }
    // 正控：抽取面必须看见足量变量（避免解析失效后与空集比绿）。
    assert!(script_vars.len() >= 21, "脚本 present() 抽取面失效: {script_vars:?}");

    // ② Rust 侧：重算期望键集（与 seed 的 ab_input **同一公式、同一排除**：
    //    子集 × analyst_short_key，再剔掉辩论下游的 `value-investor`）。
    //    排除必须走 `VALUE_INVESTOR_ID` 这个模块级权威 —— 在本测试里另抄一遍
    //    "value-investor" 字面量，就等于允许「seed 改了、锁没改」继续绿。
    let mut want: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in Period::ALL {
        for base in p.analyst_subset() {
            if base == super::seed_stock_analysis::VALUE_INVESTOR_ID {
                continue;
            }
            want.insert(format!(
                "{}__{}",
                super::seed_stock_analysis::analyst_short_key(base),
                p.as_str()
            ));
        }
    }
    assert_eq!(want.len(), 21, "期望键集应为 21（四档全跑 23 减 value-investor 两档）: {want:?}");

    assert_eq!(
        script_vars, want,
        "analyst-brief.rhai 的 present() 变量集与 seed 生成的 input_mapping 键集不同步"
    );
}

// ── 面板变量默认值的**种子侧 ↔ 落点侧**对账（2026-10-08 A 批接线）──────────────────
//
// 这一条锁的是「两侧各自自洽、合起来不成立」那族（同 `kline_limit_is_wired_to_market_data_node`
// 的思路，但方向相反）：落点（`astock-data` 评分 / `analysis-engine` 仓位）的回落默认与
// 设置面板 `b()` 的默认可以各自都写对，只要**种子表里那条变量的默认值**是另一个数，
// 那么升版播种后落点读到的就是那个数 —— 于是「接线不改现网」变成空话。
//
// 判据：把 `build_template_variables()` 的默认值当成变量表喂给三个落点的**纯构造函数**，
// 结果必须与该落点的 `Default` 逐字段相等 ⇒ 这句话在这里被真的算一遍，而不是写在注释里。
// 用纯函数而不是进程内快照：后者要动全局态，会和同二进制里的其它测试互相踩。
#[test]
fn seed_defaults_land_on_unchanged_rust_defaults() {
    use axagent_analysis_engine::position_limits::PositionLimits;
    use axagent_astock_data::scoring::{PeBands, ScoreBands};
    use std::collections::HashMap;

    let vars: HashMap<String, serde_json::Value> =
        super::seed_variables::build_template_variables()
            .into_iter()
            .map(|v| (v.name, v.value))
            .collect();

    // ① RSI 内带（面板/种子 30 与 70 == `ScoreBands::default()` 的 `rsi_oversold` / `_overbought`）
    let bands = ScoreBands::default().with_panel_overlay(&vars);
    assert_eq!(
        (bands.rsi_oversold, bands.rsi_overbought),
        (30.0, 70.0),
        "signal_rsi_oversold/overbought 的种子默认必须把带落回 30/70，否则接线本身在改评分"
    );
    assert_eq!(bands.rsi_oversold, ScoreBands::default().rsi_oversold);
    assert_eq!(bands.rsi_overbought, ScoreBands::default().rsi_overbought);

    // ② 仓位三条（20 / 10 / 40）
    assert_eq!(
        PositionLimits::from_panel_vars(&vars),
        PositionLimits::default(),
        "pos_max_single_pct / pos_max_total / pos_max_sector_pct 的种子默认必须逐条落回今天的 20/10/40"
    );

    // ③ PE 两档（15 / 50）
    let pe = PeBands::default().with_panel_overlay(&vars);
    assert_eq!(pe, PeBands::default(), "val_pe_low / val_pe_high 的种子默认必须落回 15/50");
    assert_eq!(pe.low, 15.0);
    assert_eq!(pe.high, 50.0);
}
