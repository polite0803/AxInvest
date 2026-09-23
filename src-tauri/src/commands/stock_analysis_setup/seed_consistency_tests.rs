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
// `debate_max_rounds` 派生（`seed_stock_analysis.rs:1872` 的 `for round in
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
    let vi_start = seed_src.find("let vi_id = \"value-investor\"").expect(
        "在 seed_stock_analysis.rs 里找不到 `let vi_id = \"value-investor\"` —— \
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
    let mcp = read_ws_file("crates/astock-data/src/mcp_tools.rs");
    let producer_slice =
        slice_between("mcp_tools.rs", &mcp, "let level = if score >= 70", "(score, level)");
    let producer = quoted_strings(producer_slice);
    assert_eq!(
        producer,
        MOAT_LEVEL_VOCABULARY.to_vec(),
        "生产端 `compute_moat_score` 的档位与本地声明的权威取值域不一致。\n\
         若这是有意的改档 ⇒ 请**同时**更新三处：本常量、Rhai 比对值、生产端源码；\n\
         只改生产端会让 Rhai 静默走 else 分支（乘子退化为 1.0，无任何报错）。"
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
    let engine = crate::commands::stock_workflow::rhai_registry::build_stock_rhai_engine(
        crate::commands::stock_workflow::rhai_registry::RhaiSandboxLimits::PORTFOLIO,
    );
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
        let last = args.last().cloned().unwrap_or_default();
        sites.push((key, args.len(), last));
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
    let map_end = seed[map_at..].find("\n                ]").map_or(seed.len(), |p| map_at + p);
    let map_block = &seed[map_at..map_end];
    // 不做「键/路径」配对解析（映射表有多行写法），只取块内**全部字符串字面量**作
    // 「声明过的名字」超集 —— 足以抓出拼写错误这类目标缺陷。
    let declared: std::collections::HashSet<String> = map_block
        .lines()
        .flat_map(|l| {
            l.split('"').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, v)| v.to_string())
        })
        .collect();
    // 正控②：抽取面自证 —— 必须看到 P2-1 新增的键
    assert!(
        declared.contains("lk_report") && declared.contains("lk_tool_calls"),
        "input_mapping 抽取面失效（没看到 lk_report / lk_tool_calls），共 {} 项",
        declared.len()
    );

    let mut bad_vars: Vec<String> = Vec::new();
    let mut bad_prefix: Vec<String> = Vec::new();
    for (key, _, last) in &sites {
        let Some(inner) = last.strip_prefix("attribution_note(").and_then(|r| r.strip_suffix(')'))
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
    }
    assert!(bad_vars.is_empty(), "`attribution_note` 的实参有问题：\n  {}", bad_vars.join("\n  "));
    assert!(
        bad_prefix.is_empty(),
        "`attribution_note` 把不同分析师的报告与调用记录配到了一起：\n  {}",
        bad_prefix.join("\n  ")
    );
}
