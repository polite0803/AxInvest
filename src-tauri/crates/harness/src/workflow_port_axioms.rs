// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流 DAG 的**端口公理**（C1，2026-09-14 落地）。
//!
//! # 问题
//!
//! `WorkflowEdge.source_handle` / `target_handle` 是 `Option<String>` —— **无类型、无约束**。
//! 但出边靠它与分支对齐：
//!
//! | 源节点 | 约定 |
//! |---|---|
//! | `Switch` | `source_handle` == `SwitchCase.label`（`None` = 默认分支） |
//! | `Condition` | `source_handle` == `"true"` / `"false"`（`None` 时由 `edge_type` 兜底） |
//!
//! 这些约定**只写在注释与文档里** —— 例：`src/commands/stock_analysis_setup/seed_stock_analysis.rs:3655`
//! 用注释解释「case 命中 ⇒ 具名边；default ⇒ 无 handle 的边」。引擎侧没有任何校验
//! （`commands/workflow_template.rs::validate_workflow_template` 只查了边的源/目标节点是否存在、
//! 以及 `ParallelBranch` 是否源自 `Parallel` 节点）。
//!
//! # 权威来源（本模块的公理**照抄引擎**，不自行发明）
//!
//! `rt-workflow/src/work_engine/engine/dag_store.rs`：
//!
//! - `:100-121` `switch_edge_should_follow` —— Switch 出边激活判据（4 处调度路径共用）
//! - `:188` / `:268` / `:446` / `:502` / `:690` —— 五处相同的兜底映射：
//!   `source_handle` 缺省时 `ConditionTrue → "true"`、`ConditionFalse → "false"`、其余 `→ "true"`
//!
//! 由此得出两条**可静态判定**的不变量：
//!
//! 1. **同一条边不得有两个答案**：`source_handle` 与由 `edge_type` 推出的兜底分支必须一致；
//! 2. **边必须可激活 / 分支必须可达**：`handle` 打错 ⇒ 边永不激活（死边）；
//!    `case` 没有对应出边 ⇒ 该分支不可达。
//!
//! # 本轮只产出「违规」，不决定「是否阻断」
//!
//! [`validate_port_axioms`] 是纯函数，只吐违规清单。把它当 `error`（`is_valid=false`）
//! 还是 `warning` 是**产品决策**：存量模板里可能存在违规，升为 error 会让用户在保存时被挡下。
//! 调用方（`commands/workflow_template.rs`）当前选择 `warning`。

use crate::workflow_types::{EdgeType, SwitchNode, WorkflowEdge, WorkflowNode};

// ── 句柄词汇表（只登记观察到的取值；`HandleKind` 标明它属于哪一类）──

/// 真分支句柄（`Condition` / `ConditionTrue`）。
pub const HANDLE_TRUE: &str = "true";
/// 假分支句柄（`Condition` / `ConditionFalse`）。
pub const HANDLE_FALSE: &str = "false";
/// 回边句柄（`Loop` 的 `edge_type = loopBack`）。
pub const HANDLE_BACK: &str = "back";
/// 通用出口句柄（`harness/src/assembly_builder.rs:300`）。
pub const HANDLE_OUT: &str = "out";
/// 通用入口句柄（`harness/src/assembly_builder.rs:318`）。
pub const HANDLE_IN: &str = "in";

/// 句柄的类别 —— 决定它是否受公理约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleKind {
    /// 控制流分支：取值被引擎硬解析（`true` / `false`）
    ControlBranch,
    /// 循环回边：取值被引擎硬解析（`back`）
    LoopBack,
    /// 通用数据端口：名称自由（`out` / `in` / `input_{i}`）
    DataPort,
    /// **语义分支标签**：由模板自定义（`go` / `no-go` / `acceptable` …），
    /// 必须与 `Switch` 的某个 `case.label` 一致才有意义
    SemanticLabel,
}

/// 一个已观察到的句柄。
#[derive(Debug, Clone, Copy)]
pub struct HandleDecl {
    pub handle: &'static str,
    pub kind: HandleKind,
    pub meaning: &'static str,
    pub evidence: &'static str,
}

/// 已观察到的句柄登记表（**不是**闭合词表 —— `SemanticLabel` 天然开放）。
pub const HANDLE_DECLS: &[HandleDecl] = &[
    HandleDecl {
        handle: HANDLE_TRUE,
        kind: HandleKind::ControlBranch,
        meaning: "条件为真分支",
        evidence: "rt-workflow/.../dag_store.rs:188 ｜ kit/src/preset_templates.rs:545",
    },
    HandleDecl {
        handle: HANDLE_FALSE,
        kind: HandleKind::ControlBranch,
        meaning: "条件为假分支",
        evidence: "rt-workflow/.../dag_store.rs:189 ｜ kit/src/preset_templates.rs:555",
    },
    HandleDecl {
        handle: HANDLE_BACK,
        kind: HandleKind::LoopBack,
        meaning: "循环回边",
        evidence: "src/commands/*/*.json 的 \"sourceHandle\": \"back\"",
    },
    HandleDecl {
        handle: HANDLE_OUT,
        kind: HandleKind::DataPort,
        meaning: "通用出口",
        evidence: "harness/src/assembly_builder.rs:300,318",
    },
    HandleDecl {
        handle: HANDLE_IN,
        kind: HandleKind::DataPort,
        meaning: "通用入口",
        evidence: "harness/src/assembly_builder.rs:318",
    },
    HandleDecl {
        handle: "go",
        kind: HandleKind::SemanticLabel,
        meaning: "语义分支：GO（OPC 生产流程的 go/no-go 门）",
        evidence: "src/commands/opc_workflows/seed_production.rs:404（对应 case label :366）",
    },
    HandleDecl {
        handle: "no-go",
        kind: HandleKind::SemanticLabel,
        meaning: "语义分支：NO-GO",
        evidence: "src/commands/opc_workflows/seed_production.rs:413（对应 case label :367）",
    },
    HandleDecl {
        handle: "acceptable",
        kind: HandleKind::SemanticLabel,
        meaning: "语义分支：数据质量可接受（stock-analysis 的 quality-gate）",
        evidence: "src/commands/stock_analysis_setup/seed_stock_analysis.rs:3554（对应 case label :3500）",
    },
];

/// 由 `edge_type` 推出的**兜底分支名** —— 与引擎五处实现逐字一致。
///
/// 返回 `None` 表示该边类型**不参与**分支解析（引擎里落到 `_ => "true"`，
/// 但对非条件边该值不会被使用）。
///
/// 入参取引用：`EdgeType` 未派生 `Copy`（见 `workflow_types.rs` 的 derive 列表），
/// 传值会从共享引用里 move 出来（E0507）。
pub fn branch_of_edge_type(edge_type: &EdgeType) -> Option<&'static str> {
    match edge_type {
        EdgeType::ConditionTrue => Some(HANDLE_TRUE),
        EdgeType::ConditionFalse => Some(HANDLE_FALSE),
        _ => None,
    }
}

// ── 违规类型 ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortAxiomViolation {
    /// `Condition` 出边的 `handle` 与 `edge_type` 推出的兜底分支**不一致**
    /// ⇒ 「按 handle 解析」与「按 edge_type 解析」得到两个不同分支（同一边两个答案）
    HandleContradictsEdgeType {
        node_id: String,
        edge_id: String,
        handle: String,
        branch_from_edge_type: &'static str,
    },
    /// `Condition` 出边的 `handle` 既不是 `true` 也不是 `false` ⇒ 两个分支都不激活（死边）
    ConditionHandleNotABranch { node_id: String, edge_id: String, handle: String },
    /// `Switch` 具名出边的 `handle` **既不是任何 `case.label`、也不等于 `default_case`**
    /// ⇒ `matched_label` 永远取不到该值，该边永不激活（死边）
    ///
    /// ⚠ 判据的**可达集**是 `{case.label} ∪ {default_case}`，不是只有 label：
    /// `default_case` 是引擎在「`actual` 缺失」或「无一 case 命中」时**赋给 `matched_label` 的值**
    /// （`switch_executor.rs` 的 `found.or_else(|| c.default_case.clone())`），
    /// 因此 handle == default_case 是**合法的兜底绑定**，不是死边（漏掉这一点会误报）。
    SwitchEdgeHandleNotACase {
        node_id: String,
        edge_id: String,
        handle: String,
        labels: Vec<String>,
    },
    /// `Switch` 的某个 `case.label` 没有出边（且未被默认分支覆盖）⇒ 该分支不可达
    SwitchCaseUnreachable { node_id: String, label: String },
    /// **双重激活**：`default_case` 同时是某条具名边的 handle，且还存在默认（无 handle）出边
    /// ⇒ 匹配到该值时两条边同时激活（2026-09-13 修的是「未声明 default_case」那一半，
    /// 这一半（声明了 default_case 且它又是具名分支）仍无校验）
    SwitchDefaultBranchDoubleActivation {
        node_id: String,
        label: String,
        named_edge_id: String,
        default_edge_id: String,
    },
    /// 声明了 `default_case` 但既没有默认（无 handle）出边、也没有 handle == default_case 的具名边
    /// ⇒ 兜底分支不可达
    SwitchDefaultBranchUnreachable { node_id: String, default_case: String },
}

/// 违规严重度。
///
/// 分级依据是**引擎的实际行为**（`dag_store.rs:100-121` 的 `switch_edge_should_follow`
/// 与五处兜底映射），不是主观轻重：
///
/// - [`Error`](Self::Error) —— 该边 / 该分支**永不激活**：handle 落不到任何分支、
///   或某个 case 根本没有出边。调度器不会走它，模板属于「跑不通」而不是「跑得怪」
///   ⇒ 保存前必须挡住（2026-09-14 起 C1 升为 error）。
/// - [`Warning`](Self::Warning) —— 边**会激活**，但激活语义有两个答案
///   （同一条边按 handle 与按 edge_type 解出不同分支；或匹配到某 case 时两条边同时激活）。
///   这可能是有意的并行，也可能是配置疏忽 ⇒ 交给人判断。
///
/// 分级本身是**可测公理**（`test_severity_split_is_locked_by_engine_semantics`），
/// 不允许在调用方各写一份判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortAxiomSeverity {
    /// 结构性死链 ⇒ 阻断保存。
    Error,
    /// 语义冲突 ⇒ 仅提示。
    Warning,
}

impl PortAxiomViolation {
    /// 违规所属节点 id（供校验结果回填 `node_id`）。
    pub fn node_id(&self) -> Option<&str> {
        match self {
            Self::HandleContradictsEdgeType { node_id, .. }
            | Self::ConditionHandleNotABranch { node_id, .. }
            | Self::SwitchEdgeHandleNotACase { node_id, .. }
            | Self::SwitchCaseUnreachable { node_id, .. }
            | Self::SwitchDefaultBranchDoubleActivation { node_id, .. }
            | Self::SwitchDefaultBranchUnreachable { node_id, .. } => Some(node_id),
        }
    }

    /// 稳定标识（供前端/日志分组）。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::HandleContradictsEdgeType { .. } => "handle_contradicts_edge_type",
            Self::ConditionHandleNotABranch { .. } => "condition_handle_not_a_branch",
            Self::SwitchEdgeHandleNotACase { .. } => "switch_edge_handle_not_a_case",
            Self::SwitchCaseUnreachable { .. } => "switch_case_unreachable",
            Self::SwitchDefaultBranchDoubleActivation { .. } => "switch_default_double_activation",
            Self::SwitchDefaultBranchUnreachable { .. } => "switch_default_branch_unreachable",
        }
    }

    /// 严重度（见 [`PortAxiomSeverity`] 的分级依据）。
    pub fn severity(&self) -> PortAxiomSeverity {
        match self {
            // 结构性死链：handle 落不到任何分支 / case 无出边 ⇒ 该边永不激活
            Self::ConditionHandleNotABranch { .. }
            | Self::SwitchEdgeHandleNotACase { .. }
            | Self::SwitchCaseUnreachable { .. }
            | Self::SwitchDefaultBranchUnreachable { .. } => PortAxiomSeverity::Error,
            // 语义冲突：边会激活，但「激活哪条」有两个答案 / 同时激活两条
            Self::HandleContradictsEdgeType { .. }
            | Self::SwitchDefaultBranchDoubleActivation { .. } => PortAxiomSeverity::Warning,
        }
    }

    /// 人可读消息。**消息文本与公理放在一起**，避免调用方各写一份、日久漂移。
    ///
    /// **语言约定：英文。** 本函数产出的是 `ValidationResult.warnings[].message`，与
    /// `commands/workflow_template.rs` 里既有的全部校验消息（`:645`/`:655`/`:750`/`:834`… 约 20 条）
    /// 保持同一种语言 —— 该字段由 `DebugPanel.tsx:1134` **原样渲染**，混用语言会让同一个
    /// 警告列表里两种语言并存。文件内其余中文串只出现在 `tracing` 日志与 `command` 属性里，
    /// 不进入校验结果。新增违规类型时**照此写英文**。
    pub fn describe(&self) -> String {
        match self {
            Self::HandleContradictsEdgeType { node_id, edge_id, handle, branch_from_edge_type } => {
                format!(
                    "Condition node '{node_id}' edge '{edge_id}' resolves to two different branches: \
                     sourceHandle=\"{handle}\", while edgeType implies the fallback branch \
                     \"{branch_from_edge_type}\". The engine resolves by handle whereas other paths fall \
                     back to edgeType, so the same edge behaves differently per scheduling path"
                )
            },
            Self::ConditionHandleNotABranch { node_id, edge_id, handle } => format!(
                "Condition node '{node_id}' edge '{edge_id}' has sourceHandle=\"{handle}\", which is \
                 neither \"true\" nor \"false\"; neither branch activates and the edge never fires"
            ),
            Self::SwitchEdgeHandleNotACase { node_id, edge_id, handle, labels } => format!(
                "Switch node '{node_id}' edge '{edge_id}' has sourceHandle=\"{handle}\", which matches \
                 no case label (current: {}); the edge never activates (dead edge)",
                labels.join(" / ")
            ),
            Self::SwitchCaseUnreachable { node_id, label } => format!(
                "Switch node '{node_id}' case \"{label}\" has no outgoing edge; the flow terminates \
                 whenever this case is matched"
            ),
            Self::SwitchDefaultBranchDoubleActivation {
                node_id,
                label,
                named_edge_id,
                default_edge_id,
            } => format!(
                "Switch node '{node_id}' default_case=\"{label}\" is also the handle of named edge \
                 '{named_edge_id}', and a default (handle-less) edge '{default_edge_id}' exists as well; \
                 when \"{label}\" is matched both edges activate and the two downstream chains run in \
                 parallel (same defect class as the \"undeclared default degrades to parallel\" fix on \
                 2026-09-13)"
            ),
            Self::SwitchDefaultBranchUnreachable { node_id, default_case } => format!(
                "Switch node '{node_id}' declares default_case=\"{default_case}\" but has neither a \
                 default (handle-less) outgoing edge nor a named edge whose handle equals that value; \
                 the fallback branch is unreachable"
            ),
        }
    }
}

// ── 校验入口 ──

/// 对整张 DAG 校验端口公理。纯函数，只吐违规。
pub fn validate_port_axioms(
    nodes: &[WorkflowNode],
    edges: &[WorkflowEdge],
) -> Vec<PortAxiomViolation> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            WorkflowNode::Condition(c) => validate_condition(&c.base.id, edges, &mut out),
            WorkflowNode::Switch(s) => validate_switch(s, edges, &mut out),
            _ => {},
        }
    }
    out
}

// ── 写路径门禁（P0 下沉，2026-09-14）──
//
// 背景：`validate_workflow_template`（Tauri 命令）是**客户端自检**，全仓唯一消费方是前端
// 手动保存动作；而自动保存（`useWorkflowAutoSave`，5 秒定时器）、命令层 create/update、
// 种子写入、导入、LLM 生成等**七条写路径都不经过它**（端口公理审计 2026-09-14）。
// 于是「谁在写工作流」与「谁被校验」两个集合几乎不相交 —— 曾被打穿的 `s-gonogo`
// 死链就是从种子进来的。
//
// 修法是把**同一份判据**搬到写路径的落库前，而不是在写路径各写一份 `matches!`。

/// 只取 **Error 级**违规（结构性死链：该边 / 该分支永不激活）。
///
/// 分级判据**不在此处重写**，一律走 [`PortAxiomViolation::severity`]。
/// 写一份 `matches!(v, ...)` 就等于开了第二份分级，`severity` 的登记表测试会形同虚设。
pub fn port_axiom_errors(
    nodes: &[WorkflowNode],
    edges: &[WorkflowEdge],
) -> Vec<PortAxiomViolation> {
    validate_port_axioms(nodes, edges)
        .into_iter()
        .filter(|v| v.severity() == PortAxiomSeverity::Error)
        .collect()
}

/// **写入路径门禁**：存在 Error 级违规 ⇒ `Err(全部违规的人可读清单)`。
///
/// 调用点（写库前一步，2026-09-14 P0 接线）：
/// - `dao::repo::workflow_template::{insert,upsert,update}_workflow_template`
///
/// Warning 级**不阻断**（语义有两个答案 ≠ 跑不通），仍由 `validate_workflow_template`
/// 作为即时反馈呈现。
///
/// 消息语言与 [`PortAxiomViolation::describe`] 一致（英文）：该串经命令层
/// `ErrorResponse::from_error` 进 `detail`，与校验结果同一种语言，避免同一错误
/// 在 UI 上一半中文一半英文。
pub fn enforce_port_axioms(nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> Result<(), String> {
    let errs = port_axiom_errors(nodes, edges);
    if errs.is_empty() {
        return Ok(());
    }
    let mut msg = format!(
        "workflow rejected by port axioms: {} structural violation(s) would leave an edge or a \
         branch permanently unreachable",
        errs.len()
    );
    for v in &errs {
        // 带 kind：`describe()` 是人可读句子，不含类型标识；调用方（命令层 / 日志 /
        // 前端）要按类型分组或去重时只靠句子只能做字符串匹配 ⇒ 这里显式给出稳定 id。
        msg.push_str("\n  - [");
        msg.push_str(v.kind());
        msg.push_str("] ");
        msg.push_str(&v.describe());
    }
    Err(msg)
}

/// **软门禁**：只记 `warn` 日志、不阻断 —— 返回「是否干净」。
///
/// # 为什么需要软的一档（而不是到处硬拦）
///
/// 写入方分两类，处置策略不同，**判据仍是同一份**（[`port_axiom_errors`]）：
///
/// | 写入方 | 例 | 处置 | 理由 |
/// |---|---|---|---|
/// | 用户 / LLM / 导入 | `create`/`update_workflow_template`、技能转工作流、AI 编译、导入 | **硬拦**（[`enforce_port_axioms`]） | 产物跑不通，不该落库；立刻报错才挡得住 |
/// | 我们写死的种子 | `seed_*`（14 域包 / 17 领域 / production / stock 侧 6 个…） | **软**（本函数） | 种子非法是**开发期 bug**，应当在启动日志里可见并被修；但让应用因种子拦不进去而起不来，代价过大 |
///
/// 种子的正确验证点是**开发期**：`cargo run -p axagent-harness --example port_axiom_audit`
/// 扫真库（用例见 example 头注释）。实测 2026-09-14：111 个模板里 1 个非法。
pub fn warn_port_axioms(context: &str, nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> bool {
    let errs = port_axiom_errors(nodes, edges);
    if errs.is_empty() {
        return true;
    }
    tracing::warn!(
        context = context,
        violation_count = errs.len(),
        violations = %errs
            .iter()
            .map(|v| format!("[{}] {}", v.kind(), v.describe()))
            .collect::<Vec<_>>()
            .join(" | "),
        "种子模板违反端口公理（结构性死链，已被软门禁记录，未阻断写入）"
    );
    false
}

/// [`warn_port_axioms`] 的 JSON 文本版 —— 种子函数手里通常只有
/// `serde_json::to_string(&data.nodes)` 得到的字符串（`ActiveModel` 的列值）。
///
/// 解析失败按「无法判定」处理：记 `warn` 并返回 `false`（不阻断）。
/// 这里**刻意不报 Err** —— 软门禁是给「我们写死的种子」用的观测点，不是数据校验器；
/// 硬拦的解析失败判定在 DAO 的 `enforce_port_axioms_json` 里。
pub fn warn_port_axioms_json(context: &str, nodes_json: &str, edges_json: &str) -> bool {
    let nodes: Vec<WorkflowNode> = match serde_json::from_str(nodes_json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(context = context, error = %e, "端口公理软门禁跳过：nodes 不是合法 JSON 数组");
            return false;
        },
    };
    let edges: Vec<WorkflowEdge> = match serde_json::from_str(edges_json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(context = context, error = %e, "端口公理软门禁跳过：edges 不是合法 JSON 数组");
            return false;
        },
    };
    warn_port_axioms(context, &nodes, &edges)
}

fn validate_condition(node_id: &str, edges: &[WorkflowEdge], out: &mut Vec<PortAxiomViolation>) {
    for edge in edges.iter().filter(|e| e.source == node_id) {
        let Some(handle) = edge.source_handle.as_deref() else {
            continue;
        };
        match branch_of_edge_type(&edge.edge_type) {
            // 条件边：handle 必须与 edge_type 推出的分支一致
            Some(branch) if handle != branch => {
                out.push(PortAxiomViolation::HandleContradictsEdgeType {
                    node_id: node_id.to_string(),
                    edge_id: edge.id.clone(),
                    handle: handle.to_string(),
                    branch_from_edge_type: branch,
                });
            },
            Some(_) => {},
            // 非条件边（例如 direct）：handle 仍必须是合法的布尔分支，否则该边永不激活
            None if handle != HANDLE_TRUE && handle != HANDLE_FALSE => {
                out.push(PortAxiomViolation::ConditionHandleNotABranch {
                    node_id: node_id.to_string(),
                    edge_id: edge.id.clone(),
                    handle: handle.to_string(),
                });
            },
            None => {},
        }
    }
}

fn validate_switch(node: &SwitchNode, edges: &[WorkflowEdge], out: &mut Vec<PortAxiomViolation>) {
    let node_id = node.base.id.as_str();
    let labels: Vec<String> = node.config.cases.iter().map(|c| c.label.clone()).collect();
    let default_case = node.config.default_case.clone();
    let outgoing: Vec<&WorkflowEdge> = edges.iter().filter(|e| e.source == node_id).collect();

    // ① 具名出边的 handle 必须**可达**：`matched_label` 的取值域是
    //    `{case.label} ∪ {default_case}`（`switch_executor.rs:180-377` 三个分支都取
    //    `case.label`；`actual` 缺失或全不命中时取 `default_case`）
    //    ⇒ handle 落在这个集合之外才是真正的死边。
    for edge in &outgoing {
        if let Some(handle) = edge.source_handle.as_deref()
            && !labels.iter().any(|l| l == handle)
            && default_case.as_deref() != Some(handle)
        {
            out.push(PortAxiomViolation::SwitchEdgeHandleNotACase {
                node_id: node_id.to_string(),
                edge_id: edge.id.clone(),
                handle: handle.to_string(),
                labels: labels.clone(),
            });
        }
    }

    // ② 每个 case label 必须有出边；`default_case` 允许由默认（无 handle）出边兜住
    let has_plain_edge = outgoing.iter().any(|e| e.source_handle.is_none());
    for label in &labels {
        let named = outgoing.iter().any(|e| e.source_handle.as_deref() == Some(label.as_str()));
        let covered_by_default = default_case.as_deref() == Some(label.as_str()) && has_plain_edge;
        if !named && !covered_by_default {
            out.push(PortAxiomViolation::SwitchCaseUnreachable {
                node_id: node_id.to_string(),
                label: label.clone(),
            });
        }
    }

    // ③④ 默认分支
    if let Some(dc) = default_case.as_deref() {
        let named_default =
            outgoing.iter().find(|e| e.source_handle.as_deref() == Some(dc)).map(|e| e.id.clone());
        let plain_default =
            outgoing.iter().find(|e| e.source_handle.is_none()).map(|e| e.id.clone());

        match (named_default, plain_default) {
            (Some(named_edge_id), Some(default_edge_id)) => {
                out.push(PortAxiomViolation::SwitchDefaultBranchDoubleActivation {
                    node_id: node_id.to_string(),
                    label: dc.to_string(),
                    named_edge_id,
                    default_edge_id,
                });
            },
            (None, None) => {
                out.push(PortAxiomViolation::SwitchDefaultBranchUnreachable {
                    node_id: node_id.to_string(),
                    default_case: dc.to_string(),
                });
            },
            _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_types::*;

    fn make_base(id: &str) -> WorkflowNodeBase {
        WorkflowNodeBase {
            id: id.to_string(),
            title: format!("node_{id}"),
            description: None,
            position: Position::default(),
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            continue_on_fail: false,
            compensation: None,
        }
    }

    fn condition_node(id: &str) -> WorkflowNode {
        WorkflowNode::Condition(ConditionNode {
            base: make_base(id),
            config: ConditionNodeConfig {
                conditions: Vec::new(),
                logical_op: LogicalOperator::And,
                judge_by_llm: None,
                routing_prompt: None,
                routing_model: None,
                confidence_threshold: None,
            },
        })
    }

    fn switch_node(id: &str, labels: &[&str], default: Option<&str>) -> WorkflowNode {
        WorkflowNode::Switch(SwitchNode {
            base: make_base(id),
            config: SwitchNodeConfig {
                input_var: "x".to_string(),
                cases: labels
                    .iter()
                    .map(|l| SwitchCase { value: l.to_string(), label: l.to_string() })
                    .collect(),
                default_case: default.map(str::to_string),
                match_mode: "exact".to_string(),
                use_llm: None,
                llm_prompt: None,
                llm_model: None,
                output_var: String::new(),
            },
        })
    }

    fn edge(id: &str, source: &str, handle: Option<&str>, edge_type: EdgeType) -> WorkflowEdge {
        WorkflowEdge {
            id: id.to_string(),
            source: source.to_string(),
            source_handle: handle.map(str::to_string),
            target: "t".to_string(),
            target_handle: None,
            edge_type,
            label: None,
        }
    }

    // ── Switch：正向与负向 ──

    #[test]
    fn test_switch_wellformed_passes() {
        // 现状模板的形态：具名分支 + default_case 也是具名分支（seed_production 的 s-gonogo）
        let nodes = vec![switch_node("s", &["go", "no-go"], Some("no-go"))];
        let edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-nogo", "s", Some("no-go"), EdgeType::Direct),
        ];
        assert!(validate_port_axioms(&nodes, &edges).is_empty());
    }

    #[test]
    fn test_switch_plain_default_branch_passes() {
        // quality-gate 的形态：具名 acceptable + default_case=low-quality（非 case label）+ 无 handle 边
        let nodes = vec![switch_node("q", &["acceptable"], Some("low-quality"))];
        let edges = vec![
            edge("e-named", "q", Some("acceptable"), EdgeType::Direct),
            edge("e-default", "q", None, EdgeType::Direct),
        ];
        assert!(validate_port_axioms(&nodes, &edges).is_empty());
    }

    #[test]
    fn test_switch_dead_edge_is_reported() {
        let nodes = vec![switch_node("s", &["go"], None)];
        let edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-typo", "s", Some("g0"), EdgeType::Direct), // 打错
        ];
        let v = validate_port_axioms(&nodes, &edges);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind(), "switch_edge_handle_not_a_case");
        assert_eq!(v[0].node_id(), Some("s"));
    }

    /// C1 精度修正（2026-09-14，由真实数据驱动）：`handle == default_case` 是**合法的兜底绑定**。
    ///
    /// 引擎在「`input_var` 缺失」或「无一 case 命中」时把 `matched_label` 置为 `default_case`
    /// （`switch_executor.rs` 的 `found.or_else(|| c.default_case.clone())`）⇒ 该 handle **可达**。
    /// 缺这条豁免会把一个能工作的模板误判为 Error 级死链 —— 而 C1-升级 后 Error 会**阻断保存**，
    /// 误报的代价从「误导」升级为「挡人」。
    #[test]
    fn test_switch_named_default_handle_is_not_a_dead_edge() {
        let nodes = vec![switch_node("s", &["go"], Some("no-go"))];
        let edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            // handle == default_case（不是任何 case label）
            edge("e-nogo", "s", Some("no-go"), EdgeType::Direct),
        ];
        let v = validate_port_axioms(&nodes, &edges);
        assert!(
            !v.iter().any(|x| matches!(x, PortAxiomViolation::SwitchEdgeHandleNotACase { .. })),
            "handle 等于 default_case 时不得报死边（引擎会把 default_case 赋给 matched_label）：{v:?}"
        );

        // 负向对照：既不是 label 也不等于 default_case ⇒ 必须报死边
        let edges2 = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-bogus", "s", Some("go-live"), EdgeType::Direct),
        ];
        let v2 = validate_port_axioms(&nodes, &edges2);
        assert!(
            v2.iter().any(|x| matches!(
                x,
                PortAxiomViolation::SwitchEdgeHandleNotACase { handle, .. } if handle == "go-live"
            )),
            "既不是 case label 也不等于 default_case 的 handle 必须报死边：{v2:?}"
        );
    }

    #[test]
    fn test_switch_unreachable_case_is_reported() {
        let nodes = vec![switch_node("s", &["go", "no-go"], None)];
        let edges = vec![edge("e-go", "s", Some("go"), EdgeType::Direct)];
        let v = validate_port_axioms(&nodes, &edges);
        assert!(v.iter().any(|x| matches!(
            x,
            PortAxiomViolation::SwitchCaseUnreachable { label, .. } if label == "no-go"
        )));
    }

    #[test]
    fn test_switch_default_covered_by_plain_edge_is_not_reported() {
        let nodes = vec![switch_node("s", &["go", "no-go"], Some("no-go"))];
        let edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-plain", "s", None, EdgeType::Direct), // 兜住 default
        ];
        let v = validate_port_axioms(&nodes, &edges);
        assert!(
            !v.iter().any(|x| matches!(x, PortAxiomViolation::SwitchCaseUnreachable { .. })),
            "default_case 由无 handle 边兜住时不得报不可达：{v:?}"
        );
    }

    /// 2026-09-13 修掉的是「未声明 `default_case` ⇒ 默认退化成并行」；
    /// **声明了 `default_case` 且它同时是具名分支** 那一半今天仍无校验 —— 本用例锁住它。
    #[test]
    fn test_switch_default_double_activation_is_reported() {
        let nodes = vec![switch_node("s", &["go", "no-go"], Some("no-go"))];
        let edges = vec![
            edge("e-nogo", "s", Some("no-go"), EdgeType::Direct),
            edge("e-default", "s", None, EdgeType::Direct),
        ];
        let v = validate_port_axioms(&nodes, &edges);
        assert!(
            v.iter().any(|x| matches!(
                x,
                PortAxiomViolation::SwitchDefaultBranchDoubleActivation { .. }
            )),
            "default_case 与具名 handle 重合 + 存在无 handle 边 ⇒ 必须报双重激活：{v:?}"
        );

        // 负向对照：default_case 换成既不是 case label 也不是具名 handle 的值 ⇒ 不报
        let nodes2 = vec![switch_node("s", &["go", "no-go"], Some("other"))];
        let v2 = validate_port_axioms(&nodes2, &edges);
        assert!(
            !v2.iter().any(|x| matches!(
                x,
                PortAxiomViolation::SwitchDefaultBranchDoubleActivation { .. }
            )),
            "非重合形态不得误报：{v2:?}"
        );
    }

    #[test]
    fn test_switch_default_unreachable_is_reported() {
        let nodes = vec![switch_node("s", &["go"], Some("low"))];
        let edges = vec![edge("e-go", "s", Some("go"), EdgeType::Direct)];
        let v = validate_port_axioms(&nodes, &edges);
        assert!(
            v.iter()
                .any(|x| matches!(x, PortAxiomViolation::SwitchDefaultBranchUnreachable { .. }))
        );
    }

    // ── Condition ──

    #[test]
    fn test_condition_consistent_handle_passes() {
        let nodes = vec![condition_node("c")];
        let edges = vec![
            edge("e-t", "c", Some("true"), EdgeType::ConditionTrue),
            edge("e-f", "c", Some("false"), EdgeType::ConditionFalse),
        ];
        assert!(validate_port_axioms(&nodes, &edges).is_empty());
    }

    #[test]
    fn test_condition_handle_contradicts_edge_type_is_reported() {
        // 真实风险形态：edgeType = conditionTrue 但 handle 写成 "false"
        let nodes = vec![condition_node("c")];
        let edges = vec![edge("e-bad", "c", Some("false"), EdgeType::ConditionTrue)];
        let v = validate_port_axioms(&nodes, &edges);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind(), "handle_contradicts_edge_type");
        let msg = v[0].describe();
        assert!(msg.contains("false") && msg.contains("true"), "消息须同时点出两侧取值：{msg}");
    }

    #[test]
    fn test_condition_semantic_handle_on_condition_node_is_reported() {
        // 在 Condition 上写语义标签（如 "acceptable"）⇒ 两个分支都不激活
        let nodes = vec![condition_node("c")];
        let edges = vec![edge("e", "c", Some("acceptable"), EdgeType::Direct)];
        let v = validate_port_axioms(&nodes, &edges);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].kind(), "condition_handle_not_a_branch");
    }

    #[test]
    fn test_no_handle_edges_are_never_reported() {
        // `None` 句柄 = 默认分支，引擎有明确语义 ⇒ 不得报违规
        let nodes = vec![switch_node("s", &["go"], None), condition_node("c")];
        let edges = vec![edge("e-plain", "s", None, EdgeType::Direct)];
        let v = validate_port_axioms(&nodes, &edges);
        assert!(
            !v.iter().any(|x| matches!(
                x,
                PortAxiomViolation::HandleContradictsEdgeType { .. }
                    | PortAxiomViolation::ConditionHandleNotABranch { .. }
            )),
            "无 handle 的边不得被判为分支异常：{v:?}"
        );
    }

    #[test]
    fn test_unrelated_nodes_are_ignored() {
        // 非 Switch / 非 Condition 节点的出边不受端口公理约束（避免误报）
        let nodes = vec![WorkflowNode::End(EndNode {
            base: make_base("end"),
            config: EndNodeConfig { output_var: None },
        })];
        let edges = vec![edge("e", "end", Some("whatever"), EdgeType::Direct)];
        assert!(validate_port_axioms(&nodes, &edges).is_empty());
    }

    #[test]
    fn test_branch_of_edge_type_mirrors_engine() {
        assert_eq!(branch_of_edge_type(&EdgeType::ConditionTrue), Some("true"));
        assert_eq!(branch_of_edge_type(&EdgeType::ConditionFalse), Some("false"));
        // 引擎里其余类型落到 `_ => "true"`，但那不是「分支语义」，本函数返回 None 以区分
        assert_eq!(branch_of_edge_type(&EdgeType::Direct), None);
        assert_eq!(branch_of_edge_type(&EdgeType::LoopBack), None);
        assert_eq!(branch_of_edge_type(&EdgeType::Merge), None);
    }

    #[test]
    fn test_handle_registry_is_consistent() {
        for d in HANDLE_DECLS {
            assert!(!d.handle.is_empty() && !d.evidence.is_empty() && !d.meaning.is_empty());
        }
        let mut seen: Vec<&str> = Vec::new();
        for d in HANDLE_DECLS {
            assert!(!seen.contains(&d.handle), "句柄重复登记：{}", d.handle);
            seen.push(d.handle);
        }
        // 控制流句柄必须是引擎硬解析的那两个
        let mut control: Vec<&str> = HANDLE_DECLS
            .iter()
            .filter(|d| d.kind == HandleKind::ControlBranch)
            .map(|d| d.handle)
            .collect();
        control.sort();
        assert_eq!(control, vec!["false", "true"]);
    }

    /// 一个包含**全部**违规类型的样本数组。
    ///
    /// 新增违规类型时：`severity()` 的穷举 `match` 会先编译失败，随后本数组长度
    /// 与 `test_severity_split_is_locked_by_engine_semantics` 的登记表会再拦一次
    /// —— 强制「新增类型 = 同时决定级别」。
    fn all_variant_samples() -> [PortAxiomViolation; 6] {
        [
            PortAxiomViolation::HandleContradictsEdgeType {
                node_id: "n".into(),
                edge_id: "e".into(),
                handle: "false".into(),
                branch_from_edge_type: "true",
            },
            PortAxiomViolation::ConditionHandleNotABranch {
                node_id: "n".into(),
                edge_id: "e".into(),
                handle: "x".into(),
            },
            PortAxiomViolation::SwitchEdgeHandleNotACase {
                node_id: "n".into(),
                edge_id: "e".into(),
                handle: "x".into(),
                labels: vec!["a".into()],
            },
            PortAxiomViolation::SwitchCaseUnreachable { node_id: "n".into(), label: "a".into() },
            PortAxiomViolation::SwitchDefaultBranchDoubleActivation {
                node_id: "n".into(),
                label: "a".into(),
                named_edge_id: "e1".into(),
                default_edge_id: "e2".into(),
            },
            PortAxiomViolation::SwitchDefaultBranchUnreachable {
                node_id: "n".into(),
                default_case: "a".into(),
            },
        ]
    }

    #[test]
    fn test_describe_is_non_empty_for_every_variant() {
        for s in &all_variant_samples() {
            assert!(!s.describe().is_empty(), "{s:?} 缺消息文本");
            assert!(s.node_id().is_some());
            assert!(!s.kind().is_empty());
        }
    }

    /// 严重度分级锁（C1-升级）。
    ///
    /// 分级判据是**引擎行为**：`Error` = 该边/该分支永不激活（结构性死链）；
    /// `Warning` = 边会激活但「激活哪条」有两解。这里把 6 类逐条登记，
    /// 未登记的新类型直接 panic（不允许「默认 Warning」溜过去）。
    #[test]
    fn test_severity_split_is_locked_by_engine_semantics() {
        let expected = [
            ("handle_contradicts_edge_type", PortAxiomSeverity::Warning),
            ("condition_handle_not_a_branch", PortAxiomSeverity::Error),
            ("switch_edge_handle_not_a_case", PortAxiomSeverity::Error),
            ("switch_case_unreachable", PortAxiomSeverity::Error),
            ("switch_default_double_activation", PortAxiomSeverity::Warning),
            ("switch_default_branch_unreachable", PortAxiomSeverity::Error),
        ];
        let samples = all_variant_samples();
        assert_eq!(
            samples.len(),
            expected.len(),
            "违规类型数已变（{} vs 登记 {}",
            samples.len(),
            expected.len()
        );
        for s in &samples {
            let want =
                expected.iter().find(|(k, _)| *k == s.kind()).map(|(_, v)| *v).unwrap_or_else(
                    || panic!("`{}` 未决定严重度 —— 新增类型必须同时决定级别", s.kind()),
                );
            assert_eq!(s.severity(), want, "{} 的严重度与登记不符", s.kind());
        }
        // 反向对照：两侧都必须非空，否则分级退化成「全拦」或「全放」而没人发现
        assert!(
            samples.iter().any(|s| s.severity() == PortAxiomSeverity::Error),
            "没有任何 Error 级 ⇒ 分级退化（结构性死链不再阻断保存）"
        );
        assert!(
            samples.iter().any(|s| s.severity() == PortAxiomSeverity::Warning),
            "没有任何 Warning 级 ⇒ 分级退化（所有违规都阻断保存）"
        );
    }

    // ── 写路径门禁（P0 下沉）──

    /// 门禁只拦 Error 级：`Warning` 会被 `enforce` 放行（它只是「语义有两解」，边仍会激活）。
    ///
    /// 反向对照与正向对照都必须在同一用例里出现 —— 只测一边时，「全拦」和「全放」
    /// 两种退化都能通过。
    #[test]
    fn test_enforce_blocks_error_level_and_passes_warning_level() {
        // 正向（Error 级死边）⇒ Err
        let nodes = vec![switch_node("s", &["go"], None)];
        let edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-typo", "s", Some("g0"), EdgeType::Direct),
        ];
        let err = enforce_port_axioms(&nodes, &edges).expect_err("死边必须被门禁拦下");
        assert!(err.contains("switch_edge_handle_not_a_case"), "错误应点名违规类型：{err}");
        assert!(err.contains("g0"), "错误应点名出错的 handle：{err}");

        // 反向（Warning 级：同一条边两个答案，但边会激活）⇒ Ok
        let c_nodes = vec![condition_node("c")];
        let c_edges = vec![edge("e-bad", "c", Some("false"), EdgeType::ConditionTrue)];
        assert!(!validate_port_axioms(&c_nodes, &c_edges).is_empty(), "该形态应产出 Warning");
        assert!(
            enforce_port_axioms(&c_nodes, &c_edges).is_ok(),
            "Warning 级不得阻断写入（边会激活，只是解出两个分支）"
        );

        // 干净模板 ⇒ Ok
        let ok_nodes = vec![switch_node("s", &["go"], Some("no-go"))];
        let ok_edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-nogo", "s", Some("no-go"), EdgeType::Direct),
        ];
        assert!(enforce_port_axioms(&ok_nodes, &ok_edges).is_ok());
    }

    /// `port_axiom_errors` 必须是 `validate_port_axioms` 的**纯过滤**，不得引入新判据。
    #[test]
    fn test_port_axiom_errors_is_pure_filter_of_all_violations() {
        let nodes = vec![switch_node("s", &["go", "no-go"], None), condition_node("c")];
        let edges = vec![
            edge("e-go", "s", Some("go"), EdgeType::Direct),
            edge("e-typo", "s", Some("g0"), EdgeType::Direct),
            edge("e-cbad", "c", Some("false"), EdgeType::ConditionTrue),
        ];
        let all = validate_port_axioms(&nodes, &edges);
        let errs = port_axiom_errors(&nodes, &edges);
        assert!(!all.is_empty() && !errs.is_empty(), "构造的样本应同时产出两类：{all:?}");
        assert!(errs.len() < all.len(), "本样本里 Warning 应被过滤掉（{errs:?} vs {all:?}）");
        for v in &errs {
            assert_eq!(v.severity(), PortAxiomSeverity::Error, "过滤后不得残留 Warning：{v:?}");
            assert!(all.contains(v), "过滤只能从全集里挑，不得新增/改写违规：{v:?}");
        }
    }

    /// 门禁消息必须**逐条点名 kind**：`describe()` 是人可读句子，调用方要按类型分组/
    /// 去重时只能做字符串匹配 ⇒ 消息里没有稳定 id 就没法区分「哪一类死了几条」。
    #[test]
    fn test_enforce_message_names_every_violation_kind() {
        let nodes = vec![
            switch_node("s1", &["go"], None), // 死边：handle "g0" 落不到任何 case
            switch_node("s2", &["a", "b"], None), // case "b" 无可达出边
            switch_node("s3", &["x"], Some("low")), // default_case 既无默认边也无具名边
        ];
        let edges = vec![
            edge("e1a", "s1", Some("go"), EdgeType::Direct),
            edge("e1b", "s1", Some("g0"), EdgeType::Direct),
            edge("e2a", "s2", Some("a"), EdgeType::Direct),
            edge("e3a", "s3", Some("x"), EdgeType::Direct),
        ];
        let err = enforce_port_axioms(&nodes, &edges).expect_err("三类 Error 都应被拦下");
        for kind in [
            "switch_edge_handle_not_a_case",
            "switch_case_unreachable",
            "switch_default_branch_unreachable",
        ] {
            assert!(err.contains(kind), "门禁消息缺少类型标识 `{kind}`：\n{err}");
        }
    }
}
