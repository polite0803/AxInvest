//! 子工作流**扇出键齐备性**审计（仅测试构建编译）。
//!
//! 存在理由（PLAN `PLAN-four-horizon-workflow-alignment.md` §九十 / §九十一(3)）：
//! SubWorkflow 节点启动的子执行只拿到父扇出 `input_mapping` **target 键**那一份变量快照，
//! 而 `subworkflow_executor::resolve_var_path`（`crates/rt-workflow/src/work_engine/executors/subworkflow_executor.rs:390`）
//! 是**严格私有副本** —— 不做 JSON 字符串穿透 ⇒ 子节点引用一个没被传进来的名字，
//! 报错点在**运行期**（`Variable 'x' not found` ⇒ 整节点硬错）而不是 CI。
//! 所以「哪些键必须显式传」必须由代码算出来，不能靠人抄清单（§八十 的 A1 漏项就是抄漏的形态）。
//!
//! 为什么是 Rust 而不是 node 门：判据要的是「每个节点读/写了哪些变量名」，
//! 种子是用 `data_transformer_node(id, title, pos, input_var, expr, output_var)` 这类
//! **位置参数**辅助函数造的 ⇒ 文本抽取要逐 maker 对齐参数下标（`check-input-mapping.mjs`
//! 历史上就为 `strings[6]` 错过一次，注释里记着那条史）。这里拿到的是
//! `Vec<WorkflowNode>` **类型化**值，`match` 到 config 字段直接取，覆盖面由编译器给。
//!
//! 两个使用方共用本模块（禁区 12：不许各写一份比较公式）：
//! - `init::cognitive_router_init`（存量 3 张认知子模板，§九十）
//! - `commands::stock_analysis_setup`（B-2b 之后的四张档子模板，§九十一）

use axagent_harness::workflow_types::WorkflowNode;
use axagent_rt_workflow::work_engine::executors::{
    USER_INPUT_VAR, USER_MESSAGE_VAR, WORKFLOW_MODEL_VAR, WORKFLOW_PROVIDER_ID_VAR,
};
use axagent_rt_workflow::work_engine::node_type_of;
use std::collections::{BTreeMap, BTreeSet};

/// 取声明式变量路径的根段（`input_mapping` 的值、`context_sources`、
/// `Condition::var_path`、`End`/`DataTransformer` 的 `output_var`/`input_var`）。
///
/// ⚠ 这里**不能**复用 `root_of` 的标识符判定：本仓节点 id 大量带连字符
/// （`t-scoring-hour`、`cls-risk-level-ultra-short`、`a-hot-money--short`），连字符是
/// 标识符的**一部分**而不是分隔符。首版用 `root_of` 处理这些路径时，`t-risk` 被判定为
/// 「含非法字符 ⇒ 不是变量名」，函数于是继续往后找第一个合法 token —— 每条
/// `<节点>.result.content.…` 都退化成 `result`，needs 里因此既少了真上游、又多了
/// 三个不存在的键（2026-10-06 拿四张档模板首次实测时才浮出来）。
///
/// 空段返回空串，由 `external_reads` 过滤（空路径不是「读了某个变量」）。
pub(crate) fn path_root(path: &str) -> String {
    path.split('.').next().unwrap_or("").trim().to_string()
}

/// 取**表达式**里的根名（只用于 `ValidationAssertion::expression` 这类条件/断言串）：
/// 表达式里 `-` 是减号，所以不能沿用 `path_root` 的「整段即名字」。
/// ⚠ 不解析 Rhai 自由文本，也不用于任何带点的路径 —— 那是 `path_root` 的活。
pub(crate) fn root_of(s: &str) -> String {
    let ident = |t: &str| {
        !t.is_empty()
            && t.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && t.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    for token in s.split(|c: char| {
        c == '.'
            || c.is_whitespace()
            || matches!(c, '(' | ')' | '[' | ']' | '=' | '!' | '<' | '>' | '|' | '&' | ',' | ';')
    }) {
        if ident(token) {
            return token.to_string();
        }
    }
    String::new()
}

/// 引擎在每个 run 里自动注入的工作流级变量 ⇒ 它们不需要父扇出传，也不该被报成缺键。
/// 名字**直接取引擎侧常量**（`work_engine/executors/mod.rs:91-94`）而不是在这里抄一遍 ——
/// 抄的话常量一改，本门就会开始误报。
pub(crate) fn engine_injected_names() -> [&'static str; 4] {
    [USER_INPUT_VAR, USER_MESSAGE_VAR, WORKFLOW_MODEL_VAR, WORKFLOW_PROVIDER_ID_VAR]
}

/// 一个节点从变量池**读**的键 / **写**的键；第三个返回元素 = 未覆盖的节点类型（盲区计数）。
///
/// ⚠ 只覆盖**声明式**字段（`input_mapping` 两侧、`context_sources`、`input_var`、
/// `categories_var`、`Condition::var_path`、`ValidationAssertion::expression` 的根名）。
/// **不**覆盖 LLM 提示词与 Rhai 表达式体里的自由变量名 —— 那一族运行期不报错、只拿到 `unit`
/// （`seed_consistency_tests.rs` 同一结论），本模块不假装能查它。
pub(crate) fn node_var_io(node: &WorkflowNode) -> (Vec<String>, Vec<String>, Option<&'static str>) {
    let roots = |vals: &[String]| -> Vec<String> { vals.iter().map(|v| path_root(v)).collect() };
    match node {
        WorkflowNode::Trigger(_) => (vec![], vec![], None),
        // End 的 `output_var` 是「选哪个变量作终值」⇒ 记成**读**（宁可多要求一个键，
        // 也不要漏掉「子图结束时取不到值」这种运行期才浮出来的形态）。
        WorkflowNode::End(e) => (
            e.config.output_var.as_deref().map(|v| vec![path_root(v)]).unwrap_or_default(),
            vec![],
            None,
        ),
        WorkflowNode::Condition(c) => {
            (c.config.conditions.iter().map(|x| path_root(&x.var_path)).collect(), vec![], None)
        },
        WorkflowNode::Validation(v) => (
            v.config
                .assertions
                .iter()
                .filter_map(|a| a.expression.as_deref())
                .map(root_of)
                .collect(),
            vec![],
            None,
        ),
        WorkflowNode::LlmClassifier(c) => {
            let mut reads = vec![path_root(&c.config.input_var)];
            if let Some(cv) = &c.config.categories_var {
                reads.push(path_root(cv));
            }
            (reads, vec![path_root(&c.config.output_var)], None)
        },
        WorkflowNode::DataTransformer(d) => {
            (vec![path_root(&d.config.input_var)], vec![path_root(&d.config.output_var)], None)
        },
        WorkflowNode::Agent(a) => {
            let mut reads = roots(&a.config.context_sources);
            reads.extend(a.config.input_mapping.values().map(|v| path_root(v)));
            (reads, vec![path_root(&a.config.output_var)], None)
        },
        WorkflowNode::Tool(t) => (
            t.config.input_mapping.values().map(|v| path_root(v)).collect(),
            vec![path_root(&t.config.output_var)],
            None,
        ),
        WorkflowNode::Code(c) => (
            c.config.input_mapping.values().map(|v| path_root(v)).collect(),
            vec![path_root(&c.config.output_var)],
            None,
        ),
        WorkflowNode::SubWorkflow(s) => (
            s.config.input_mapping.values().map(|v| path_root(v)).collect(),
            vec![path_root(&s.config.output_var)],
            None,
        ),
        other => (vec![], vec![], Some(node_type_of(other))),
    }
}

/// 一个子图的「自产名集合」：节点 id（引擎双键写回）+ 各节点声明式写出的变量名。
///
/// 抽成函数而不是内联：`external_reads` 与 `external_reads_for_kind` 必须共用**同一份**
/// 自产集公式，否则两条判据会对「谁算已到账」各说一套（禁区 12）。
fn produced_names(
    children: &[WorkflowNode],
    blind_kinds: &mut BTreeSet<&'static str>,
) -> BTreeSet<String> {
    let mut produced: BTreeSet<String> = BTreeSet::new();
    for c in children {
        // 引擎把节点结果**同时**写在 `node_id` 与 `output_var` 两个键上
        // （`work_engine/engine/mod.rs:1772-1775`）⇒ 下游用 node_id 形态引用上游是合法的，
        // 自产集必须含节点 id，否则会把合法引用误报成缺键。
        produced.insert(c.base_id().to_string());
        let (_, writes, blind) = node_var_io(c);
        produced.extend(writes);
        if let Some(k) = blind {
            blind_kinds.insert(k);
        }
    }
    produced
}

/// 只取**指定种类**节点（`node_type_of` 的返回值，如 `"tool"`）的外部读取集。
///
/// 存在理由（PLAN §九十二(4) 末那条待补的门）：`tool_executor.rs:75-80` 对
/// `input_mapping` 的**值**一律 `resolve_var_path` 后 `unwrap_or(Null)` ⇒ 一个指向
/// 「不存在的工作流变量」的参数**不报错**，只会让工具走自己的缺省值。
/// 实锤形态：四档评分节点的 `("period","hourly")` 全仓没有名为 `hourly` 的变量
/// ⇒ `compute_scoring` 恒按日线出分，四档评分恒等（§九十二(4)），而 `cargo check` /
/// `clippy` / 全套既有门**全绿**。整图的 needs 判据（下面 `external_reads`）管的是
/// 「子模板要读什么父得传」，管不到这一族 —— ToolNode 的参数**不会**出现在子模板 needs 里
/// （它是工具参数名，不是工作流变量），所以必须单列一条按节点种类切开的判据。
pub(crate) fn external_reads_for_kind(
    children: &[WorkflowNode],
    kind: &str,
    blind_kinds: &mut BTreeSet<&'static str>,
) -> BTreeSet<String> {
    let produced = produced_names(children, blind_kinds);
    let mut needs: BTreeSet<String> = BTreeSet::new();
    for c in children {
        if node_type_of(c) != kind {
            continue;
        }
        let (reads, _, blind) = node_var_io(c);
        needs.extend(reads.into_iter().filter(|r| !r.is_empty() && !produced.contains(r)));
        if let Some(k) = blind {
            blind_kinds.insert(k);
        }
    }
    needs
}

/// 一个子图的「**外部读取集**」：子图节点声明式读取、且不被子图自己写出的变量根名。
///
/// 这是 `audit_fanouts` 里那一步 needs 计算的**唯一出口** —— B-2b 要在改图之前先拿到
/// 「父侧每档必须显式传哪些键」的机器答案（PLAN §九十一(3) 步骤 1），那时还没有扇出节点，
/// 只能靠这个函数算。另写一份公式＝两套真相（判据层同族教训）。
///
/// `blind_kinds` 回填调用方收集到的未覆盖节点类型（`None` 表示本函数没往里塞东西的余地，
/// 故用 `&mut` 出参而不是返回值元组）。
pub(crate) fn external_reads(
    children: &[WorkflowNode],
    blind_kinds: &mut BTreeSet<&'static str>,
) -> BTreeSet<String> {
    let produced = produced_names(children, blind_kinds);
    let mut needs: BTreeSet<String> = BTreeSet::new();
    for c in children {
        let (reads, _, blind) = node_var_io(c);
        needs.extend(reads.into_iter().filter(|r| !r.is_empty() && !produced.contains(r)));
        if let Some(k) = blind {
            blind_kinds.insert(k);
        }
    }
    needs
}

/// 一次扇出审计的**事实**集合。判红全在调用侧断言 ——
/// 函数本身不 panic，负控才能拿同一个函数、同一套公式验「抽掉键会不会红」。
pub(crate) struct FanoutAudit {
    /// `(所在图, 子模板 id, 父扇出节点 id, 父没传进来的键)`
    pub gaps: Vec<(String, String, String, BTreeSet<String>)>,
    /// **所有**被审计图里真正的图内扇出（`system_*` 已排除）—— 含子模板内部再套的扇出
    pub fanouts: BTreeSet<String>,
    /// 本门登记了数据源的子模板集合（要与 `fanouts` 双向一致）
    pub registered: BTreeSet<String>,
    pub excluded_system: BTreeSet<String>,
    /// 指向未登记子模板的扇出 ⇒ 判据对它没有数据源（不能当成"没有缺口"）
    pub unregistered: Vec<String>,
    /// `node_var_io` 还没覆盖的节点类型 ⇒ 对它们是盲区，必须补 match 臂
    pub blind_kinds: BTreeSet<&'static str>,
}

/// 对每个图内扇出算「子图声明式读取 − 子图自产 − 引擎注入 − 父已传」的缺口。
///
/// `graphs` 要包含**每一层**图（主图 + 每个已登记子模板）：首版只走主图，实测就把
/// 「子模板内部再扇出」整层漏掉了（L3 子模板里那 2 个 `system_*` 扇出连被排除的计数都没进）。
pub(crate) fn audit_fanouts(
    graphs: &[(&str, &[WorkflowNode])],
    templates: &BTreeMap<&str, Vec<WorkflowNode>>,
) -> FanoutAudit {
    let mut audit = FanoutAudit {
        gaps: Vec::new(),
        fanouts: BTreeSet::new(),
        registered: templates.keys().copied().map(str::to_string).collect(),
        excluded_system: BTreeSet::new(),
        unregistered: Vec::new(),
        blind_kinds: BTreeSet::new(),
    };
    let sites: Vec<(&str, &WorkflowNode)> =
        graphs.iter().flat_map(|(host, nodes)| nodes.iter().map(move |n| (*host, n))).collect();
    for (host, node) in sites {
        let WorkflowNode::SubWorkflow(s) = node else { continue };
        if s.config.sub_workflow_id.starts_with("system_") {
            // 系统能力回调不产生子执行、也不查 workflow_templates 表 ⇒ 不是图内扇出
            audit.excluded_system.insert(s.config.sub_workflow_id.clone());
            continue;
        }
        audit.fanouts.insert(s.config.sub_workflow_id.clone());
        let Some(children) = templates.get(s.config.sub_workflow_id.as_str()) else {
            audit
                .unregistered
                .push(format!("{}: {}→{}", host, s.base.id, s.config.sub_workflow_id));
            continue;
        };

        let needs = external_reads(children, &mut audit.blind_kinds);
        // 键是「子快照里的变量名」，不是路径 ⇒ 用 path_root（含连字符的名字要原样保留），
        // 不能用 root_of 的标识符判定（`h-ultra-short` 这类键会被判成「不是名字」而丢段）。
        let provided: BTreeSet<String> =
            s.config.input_mapping.keys().map(|k| path_root(k)).collect();
        let missing: BTreeSet<String> = needs
            .difference(&provided)
            .filter(|k| !engine_injected_names().contains(&k.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            audit.gaps.push((
                host.to_string(),
                s.config.sub_workflow_id.clone(),
                s.base.id.clone(),
                missing,
            ));
        }
    }
    audit
}

/// 待审计的**全部**图：主图（带它自己的标签，缺口报告要能指回所在图）+ 每个已登记子模板。
/// 少列一层就等于漏检那一层里的扇出。
pub(crate) fn graphs_to_audit<'a>(
    main_label: &'a str,
    main: &'a [WorkflowNode],
    templates: &'a BTreeMap<&str, Vec<WorkflowNode>>,
) -> Vec<(&'a str, &'a [WorkflowNode])> {
    let mut v = vec![(main_label, main)];
    for (id, nodes) in templates {
        v.push((*id, nodes.as_slice()));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `root_of` 的形态自证：点分路径、表达式、以及"不是标识符"三种输入。
    #[test]
    fn root_of_shapes() {
        assert_eq!(root_of("l1_rule_result.hit"), "l1_rule_result");
        assert_eq!(root_of("l1_result.confidence >= 0.5"), "l1_result");
        assert_eq!(root_of("__l1_categories"), "__l1_categories");
        assert_eq!(root_of("0.5"), "", "数字不是变量名 ⇒ 不得当成引用");
    }

    /// `path_root` 必须原样保留**带连字符的节点 id**。
    ///
    /// 这条是本轮实测出来的缺陷的反面锁：首版用 `root_of` 抽声明式路径的根名，
    /// `t-risk` 被判定为「含非法字符」而丢段，四条 `<节点>.result.content.…` 全部退化成
    /// `result` ⇒ needs 既漏真上游又凭空多出假键，而门**照样绿**。
    #[test]
    fn path_root_keeps_hyphenated_node_ids() {
        assert_eq!(path_root("t-risk.result.content.stockRiskProfile"), "t-risk");
        assert_eq!(
            path_root("a-hot-money--short.content.verdict.flowPersistence"),
            "a-hot-money--short"
        );
        assert_eq!(
            path_root("cls-risk-level-ultra-short.result.category"),
            "cls-risk-level-ultra-short"
        );
        assert_eq!(path_root("horizon_branch_json.ultra_short"), "horizon_branch_json");
        assert_eq!(path_root("stock_code"), "stock_code", "单段平键");
        assert_eq!(path_root(""), "", "空路径不是「读了某个变量」，由调用侧过滤");
        // 反向锁：`root_of`（表达式口径）在这批路径上**必然**给出错误答案 ——
        // 两条函数各自适用范围不重叠，这条断言就是「谁都不能被拿去顶替谁」。
        assert_eq!(
            root_of("t-risk.result.content"),
            "result",
            "表达式口径会丢段 ⇒ 声明式路径禁止用它"
        );
    }
}
