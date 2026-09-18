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
//!   不经过 ToolResolver，计入可解析集合）。

use std::collections::HashSet;

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
