// SPDX-License-Identifier: AGPL-3.0-only

//! Schema 生成与 Tauri IPC 类型同步工具
//!
//! 功能:
//! 1. 为 workflow 类型生成 JSON Schema 文档
//! 2. 从 Rust DTO 生成 TypeScript 类型定义，确保前后端一致性
//! 3. 从 `axagent_harness::IpcEventName` 生成前端 IPC 事件名联合类型
//! 4. 从 Rust DTO 生成**前端真正消费**的类型（`src/types/generated/dto.ts`）
//! 5. 校验前端类型定义与后端 DTO / 事件名的同步状态
//!
//! 使用方法:
//! ```bash
//! cargo run -p schema-gen                    # 生成所有类型定义
//! cargo run -p schema-gen -- check           # 仅检查类型同步，不生成
//! cargo run -p schema-gen -- ipc-types       # 仅生成 IPC 同步校验类型
//! cargo run -p schema-gen -- event-names     # 仅生成 IPC 事件名联合类型
//! cargo run -p schema-gen -- frontend-dto    # 仅生成前端可消费 DTO
//! ```

use axagent_harness::agent::{
    AgentCapability, AgentExecuteRequest, AgentInfo, AgentPlan, AgentResult, PlanStep,
};
use axagent_harness::conversation_model::{
    ContentBlock, ConversationMessage, SessionInfo, TokenUsage,
};
use axagent_harness::types::provider_model::{
    ModelCapability, ModelType, ProviderKey, ProviderProxyConfig, ProviderType,
};
use axagent_harness::workflow_types::{
    BackoffType, CompensationConfig, CompensationStrategy, NodeKind, Position, RetryConfig,
    SubWorkflowOrigin, Variable, WorkflowNodeBase,
};
use schemars::schema_for;
use std::fs;
use std::path::Path;
use ts_rs::TS;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = if args.len() > 1 { &args[1] } else { "all" };

    // 本 crate 位于 axagent-plugins/crates/schema-gen ⇒ 上溯三级才是仓库根
    // （L2 拆分前在 src-tauri/schema-gen，只需两级）。
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("路径无父目录")
        .parent()
        .expect("路径无祖父目录")
        .parent()
        .expect("路径无曾祖父目录");

    let out_dir = project_root.join("docs");
    let ts_types_dir = project_root.join("src").join("types").join("generated");

    match mode {
        "check" => run_check(&ts_types_dir),
        "ipc-types" => generate_ipc_types(&ts_types_dir),
        "event-names" => generate_event_names(&ts_types_dir),
        "frontend-dto" => generate_frontend_dto(&ts_types_dir),
        "workflow-schema" => generate_workflow_schema(&out_dir),
        _ => {
            generate_workflow_schema(&out_dir);
            generate_ipc_types(&ts_types_dir);
            generate_event_names(&ts_types_dir);
            generate_frontend_dto(&ts_types_dir);
            run_check(&ts_types_dir);
        },
    }
}

// ============================================================================
// Workflow JSON Schema 生成
// ============================================================================

fn generate_workflow_schema(out_dir: &Path) {
    fs::create_dir_all(out_dir).expect("Schema 生成：创建输出目录失败");

    let schema = schema_for!(axagent_harness::workflow_types::WorkflowTemplateInput);
    let schema_str =
        serde_json::to_string_pretty(&schema).expect("Schema 生成：序列化 schema 失败");
    fs::write(out_dir.join("workflow-schema.json"), &schema_str)
        .expect("Schema 生成：写入 JSON 失败");
    eprintln!("Generated docs/workflow-schema.json");

    let mut md = String::new();
    md.push_str("# 工作流 Schema 文档\n\n");
    md.push_str("> 自动生成自 `axagent-harness::workflow_types`。\n\n");

    md.push_str("## 核心类型\n\n");
    md.push_str("| 类型 | 说明 |\n");
    md.push_str("|------|------|\n");
    md.push_str("| `WorkflowNode` | 工作流节点（28 种变体的标签联合）|\n");
    md.push_str("| `WorkflowEdge` | 工作流边定义 |\n");
    md.push_str("| `WorkflowTemplateInput` | 工作流模板创建/更新入参 |\n");
    md.push_str("| `WorkflowTemplateResponse` | 工作流模板查询响应 |\n");
    md.push_str("| `TriggerConfig` | 触发配置（manual/schedule/webhook/event）|\n");
    md.push_str("| `RetryConfig` | 节点重试策略 |\n");
    md.push_str("| `ErrorConfig` | 节点错误处理配置 |\n\n");

    let variants = [
        ("trigger", "TriggerNode", "TriggerConfig"),
        ("agent", "AgentNode", "AgentNodeConfig"),
        ("llm", "LLMNode", "LLMNodeConfig"),
        ("condition", "ConditionNode", "ConditionNodeConfig"),
        ("parallel", "ParallelNode", "ParallelNodeConfig"),
        ("loop", "LoopNode", "LoopNodeConfig"),
        ("merge", "MergeNode", "MergeNodeConfig"),
        ("delay", "DelayNode", "DelayNodeConfig"),
        ("tool", "ToolNode", "ToolNodeConfig"),
        ("code", "CodeNode", "CodeNodeConfig"),
        ("subWorkflow", "SubWorkflowNode", "SubWorkflowNodeConfig"),
        ("workflowRef", "WorkflowRefNode", "WorkflowRefNodeConfig"),
        ("end", "EndNode", "EndNodeConfig"),
        ("switch", "SwitchNode", "SwitchNodeConfig"),
        ("httpRequest", "HttpRequestNode", "HttpRequestNodeConfig"),
        ("databaseQuery", "DatabaseQueryNode", "DatabaseQueryNodeConfig"),
        ("notification", "NotificationNode", "NotificationNodeConfig"),
        ("approval", "ApprovalNode", "ApprovalNodeConfig"),
        ("fileOperation", "FileOperationNode", "FileOperationNodeConfig"),
        ("dataTransformer", "DataTransformerNode", "DataTransformerNodeConfig"),
        ("webhookSend", "WebhookSendNode", "WebhookSendNodeConfig"),
        ("logging", "LoggingNode", "LoggingNodeConfig"),
        ("llmClassifier", "LlmClassifierNode", "LlmClassifierNodeConfig"),
        ("aggregator", "AggregatorNode", "AggregatorNodeConfig"),
        ("email", "EmailNode", "EmailNodeConfig"),
        ("debate", "DebateNode", "DebateNodeConfig"),
        ("validation", "ValidationNode", "ValidationNodeConfig"),
        ("documentParser", "DocumentParserNode", "DocumentParserNodeConfig"),
        ("vectorRetrieve", "VectorRetrieveNode", "VectorRetrieveNodeConfig"),
    ];
    md.push_str("## WorkflowNode 变体\n\n");
    md.push_str("| `type` 标签 | 节点类型 | 配置结构体 |\n");
    md.push_str("|------------|----------|------------|\n");
    for (tag, node, cfg) in &variants {
        md.push_str(&format!("| `{}` | `{}` | `{}` |\n", tag, node, cfg));
    }

    md.push_str("\n## 关键字段说明\n\n");
    let schema_val: serde_json::Value =
        serde_json::from_str(&schema_str).expect("Schema 生成：反序列化 schema JSON 失败");
    // `definitions` 在「没有共享子 schema」时整个键缺席（schemars 的行为），旧写法直接
    // `.expect("definitions 应为 object")` ⇒ mode "all" 每次当场 panic、后面的 ipc-types
    // 永远跑不到（本工具接不进 CI 的直接原因）。缺席就如实写一行，不 panic。
    let empty_defs = serde_json::Map::new();
    let defs = schema_val.get("definitions").and_then(|v| v.as_object()).unwrap_or(&empty_defs);
    if defs.is_empty() {
        md.push_str("（本 schema 无共享 definitions）\n\n");
    }
    for (name, def) in defs {
        let title = def["title"].as_str().unwrap_or(name);
        let desc = def["description"].as_str().unwrap_or("");
        if !desc.is_empty() || title.contains("Config") || title.contains("Node") {
            md.push_str(&format!("### `{}`\n\n", title));
            if !desc.is_empty() {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(props) = def["properties"].as_object() {
                md.push_str("| 字段 | 类型 | 必填 | 说明 |\n");
                md.push_str("|------|------|------|------|\n");
                let required = def["required"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                    .unwrap_or_default();
                for (fname, fdef) in props {
                    let ftype = fdef["type"].as_str().unwrap_or("object");
                    let fdesc = fdef["description"].as_str().unwrap_or("");
                    let is_req = required.contains(&fname.as_str());
                    md.push_str(&format!(
                        "| `{}` | `{}` | {} | {} |\n",
                        fname,
                        ftype,
                        if is_req { "✅" } else { "❌" },
                        fdesc
                    ));
                }
            }
            md.push('\n');
        }
    }

    md.push_str("## EdgeType 枚举\n\n");
    md.push_str("| 值 | 说明 |\n");
    md.push_str("|----|------|\n");
    md.push_str("| `direct` | 直接连线（默认） |\n");
    md.push_str("| `conditionTrue` | 条件节点 true 分支 |\n");
    md.push_str("| `conditionFalse` | 条件节点 false 分支 |\n");
    md.push_str("| `loopBack` | 循环回边 |\n");
    md.push_str("| `parallelBranch` | 并行分支边 |\n");
    md.push_str("| `merge` | 合并边 |\n");
    md.push_str("| `debateRound` | 辩论轮次边 |\n");
    md.push_str("| `error` | 错误处理边 |\n");
    md.push_str("| `grouping` | 装饰分组边（不参与校验和布局） |\n\n");
    md.push_str("## 嵌套限制\n\n");
    md.push_str("- WorkflowRef 嵌套深度：≤ 3 层\n");
    md.push_str("- 循环引用检测：执行时回溯调用栈\n\n");
    md.push_str("## 向后兼容\n\n");
    md.push_str("- 所有 Optional 字段均有 `#[serde(default)]`，向前兼容\n");
    md.push_str("- `kind` 字段默认 `\"executable\"`，旧数据自动兼容\n");
    md.push_str("- `edge_type` 新增 `\"grouping\"`，旧解析器忽略未知值\n\n");
    md.push_str("---\n");
    md.push_str("*文档自动生成，更新类型后请重新运行 `cargo run -p schema-gen`*\n");

    fs::write(out_dir.join("workflow-schema.md"), &md).expect("Schema 生成：写入 Markdown 失败");
    eprintln!("Generated docs/workflow-schema.md");
}

// ============================================================================
// Tauri IPC 类型生成与同步校验
// ============================================================================

/// 需要同步到前端的 Tauri IPC DTO 类型列表
///
/// 这些类型是跨前后端传输的数据结构，必须保持同步。
/// 新增 IPC DTO 时，需要在此处添加。
const IPC_DTO_TYPES: &[(&str, &str)] = &[
    // Agent 相关
    ("AgentExecuteRequest", "agent"),
    ("AgentResult", "agent"),
    ("AgentCapability", "agent"),
    ("AgentInfo", "agent"),
    ("AgentPlan", "agent"),
    ("PlanStep", "agent"),
    // Conversation 相关
    ("TokenUsage", "conversation"),
    ("ContentBlock", "conversation"),
    ("ConversationMessage", "conversation"),
    ("SessionInfo", "conversation"),
    // Workflow 相关
    ("Position", "workflow"),
    ("RetryConfig", "workflow"),
    ("BackoffType", "workflow"),
    ("CompensationConfig", "workflow"),
    ("CompensationStrategy", "workflow"),
    ("NodeKind", "workflow"),
    ("Variable", "workflow"),
    ("WorkflowNodeBase", "workflow"),
];

/// 生成 Tauri IPC 类型的 TypeScript 定义文件
fn generate_ipc_types(out_dir: &Path) {
    fs::create_dir_all(out_dir).expect("IPC 类型生成：创建输出目录失败");

    eprintln!("Generating Tauri IPC TypeScript type definitions...");

    let mut output = String::new();
    output.push_str("// 此文件由 schema-gen 自动生成，请勿手动编辑\n");
    output.push_str("// 修改 Rust DTO 后请重新运行: cargo run -p schema-gen -- ipc-types\n");
    output.push_str("//\n");
    output.push_str("// 本文件包含所有 Tauri IPC 跨进程传输的数据结构定义，\n");
    output.push_str("// 用于确保前后端类型一致性。\n\n");
    output.push_str("// 生成文件中的类型别名仅供后端 DTO 同步校验，\n");
    output.push_str("// 不在前端运行时直接引用，故禁用 no-unused-vars 规则。\n");
    output.push_str("// oxlint-disable no-unused-vars\n\n");

    // 基础类型必须**由生成器输出**：下面的 DTO 引用 `MessageRole` / `JsonValue`，而这两条
    // 此前只存在于已提交的 ipc.ts 里（= 手改生成物）。一旦谁重新生成，产物就编译不过
    // —— 本次接字节级漂移门时正是这样现形的（`tsc` 报 Cannot find name 'MessageRole'）。
    output.push_str(
        "// ============================================================================\n",
    );
    output.push_str("// 基础类型（补充定义）\n");
    output.push_str(
        "// ============================================================================\n\n",
    );
    output.push_str("type MessageRole = \"system\" | \"user\" | \"assistant\" | \"tool\";\n\n");
    output.push_str("type JsonValue =\n  | string\n  | number\n  | boolean\n  | null\n  | JsonValue[]\n  | { [key: string]: JsonValue };\n\n");

    output.push_str(
        "// ============================================================================\n",
    );
    output.push_str("// Agent 相关 DTO\n");
    output.push_str(
        "// ============================================================================\n\n",
    );
    output.push_str(&gen_type::<AgentExecuteRequest>());
    output.push_str(&gen_type::<AgentResult>());
    output.push_str(&gen_type::<AgentCapability>());
    output.push_str(&gen_type::<AgentInfo>());
    output.push_str(&gen_type::<AgentPlan>());
    output.push_str(&gen_type::<PlanStep>());

    output.push_str(
        "\n// ============================================================================\n",
    );
    output.push_str("// Conversation 相关 DTO\n");
    output.push_str(
        "// ============================================================================\n\n",
    );
    output.push_str(&gen_type::<TokenUsage>());
    output.push_str(&gen_type::<ContentBlock>());
    output.push_str(&gen_type::<ConversationMessage>());
    output.push_str(&gen_type::<SessionInfo>());

    output.push_str(
        "\n// ============================================================================\n",
    );
    output.push_str("// Workflow 相关 DTO\n");
    output.push_str(
        "// ============================================================================\n\n",
    );
    output.push_str(&gen_type::<Position>());
    output.push_str(&gen_type::<RetryConfig>());
    output.push_str(&gen_type::<BackoffType>());
    output.push_str(&gen_type::<CompensationConfig>());
    output.push_str(&gen_type::<CompensationStrategy>());
    output.push_str(&gen_type::<NodeKind>());
    output.push_str(&gen_type::<Variable>());
    output.push_str(&gen_type::<WorkflowNodeBase>());

    output.push_str(
        "\n// ============================================================================\n",
    );
    output.push_str("// 同步检查元数据\n");
    output.push_str(
        "// ============================================================================\n\n",
    );
    // ⚠ 这里**不放生成时间戳**（旧实现写过 `// Generated at: SystemTime{…}`）：
    //   一行每次都变的内容会让「在库生成物 == 重新生成结果」这类字节比对永远红，
    //   于是 CI 接不进生成通路（每次跑都脏一个文件）—— 这正是它长期不被检查的成因。
    output.push_str(&format!("// Total DTO types: {}\n", IPC_DTO_TYPES.len()));

    let output_path = out_dir.join("ipc.ts");
    fs::write(&output_path, &output)
        .unwrap_or_else(|e| panic!("IPC 类型生成：写入 {} 失败: {}", output_path.display(), e));

    eprintln!("✅ Generated Tauri IPC types: {}", output_path.display());
}

fn gen_type<T: TS>() -> String {
    let cfg = ts_rs::Config::default();
    let decl = T::decl(&cfg);
    format!("{}\n\n", decl)
}

// ============================================================================
// IPC 事件名联合类型生成与同步校验
// ============================================================================

/// 生成 IPC 事件名联合类型（前端 `listen` 的形参类型）
///
/// 权威来源是 `axagent_harness::IpcEventName`；本函数只做投影，不含任何逻辑，
/// 因此**刻意不写时间戳** —— 生成物必须可复现，否则「重新生成后内容不变」
/// 无法作为同步判据（`ipc.ts` 带时间戳是历史遗留，不要照抄到这里）。
fn generate_event_names(out_dir: &Path) {
    fs::create_dir_all(out_dir).expect("事件名生成：创建输出目录失败");

    eprintln!("Generating IPC event name union type...");

    let mut output = String::new();
    output.push_str("// 此文件由 schema-gen 自动生成，请勿手动编辑\n");
    output.push_str(
        "// 修改 Rust `axagent_harness::IpcEventName` 后请重新运行: \
         cargo run -p schema-gen -- event-names\n",
    );
    output.push_str("//\n");
    output.push_str("// 本文件把后端事件名契约投影到前端：`@/lib/invoke` 的 `listen` 形参类型即\n");
    output
        .push_str("// `IpcEventName`，因此监听到「后端不存在的事件名」会在 `npm run typecheck`\n");
    output.push_str("// 阶段报错，而不是运行期静默收不到消息。\n\n");
    output.push_str(
        "/** 全部 IPC 事件名（顺序与后端 `axagent_harness::IpcEventName::ALL` 一致）。 */\n",
    );
    output.push_str("export const IPC_EVENT_NAMES = [\n");
    for evt in axagent_harness::IpcEventName::ALL {
        output.push_str(&format!("  \"{}\",\n", evt.as_str()));
    }
    output.push_str("] as const;\n\n");
    output.push_str("/** IPC 事件名联合类型（`listen` 只接受这些名字）。 */\n");
    output.push_str("export type IpcEventName = (typeof IPC_EVENT_NAMES)[number];\n");

    let output_path = out_dir.join("events.ts");
    fs::write(&output_path, &output)
        .unwrap_or_else(|e| panic!("事件名生成：写入 {} 失败: {}", output_path.display(), e));

    eprintln!("✅ Generated IPC event names: {}", output_path.display());
}

// ============================================================================
// 前端可消费 DTO 生成（`src/types/generated/dto.ts`）
// ============================================================================

/// 前端 DTO 生成用的 ts-rs 配置。
///
/// 与 `gen_type`（`ipc.ts`，只做同步校验）**刻意不同**：ts-rs 默认把 `i64/u64` 映射成
/// `bigint`，而 Tauri IPC 走 JSON 文本 ⇒ 前端拿到的就是 `number`。`ipc.ts` 不加 `export`
/// 没人消费，`bigint` 只是纸面误差；本文件的产物要被 `src/types/index.ts` 直接 re-export
/// 给消费方，`createdAt: bigint` 传到 `Date.now()` 比较处就是当场编译不过。
fn frontend_dto_cfg() -> ts_rs::Config {
    ts_rs::Config::new().with_large_int("number")
}

/// 前端 DTO 名单（**依赖在前**，生成顺序即声明顺序）。
///
/// 三条准入判据（缺一不可；第三条由 [`foreign_type_refs`] 机械校验）：
/// 1. **权威定义在 `axagent-harness`** —— schema-gen 只依赖 harness，`src-tauri` 里的宿主
///    DTO（`ScheduledTaskDto` / `AnalysisSummary` / `PipelineRun` 一族）本工具**拿不到**，
///    必须先上收到 harness 才能进这张名单；
/// 2. **已派生 `ts_rs::TS`** 且 serde 输出 camelCase（枚举按各自 `rename_all` 逐字；
///    注意 `ProviderType` 是 `lowercase` 而不是 `camelCase`）；
/// 3. **依赖闭包全部在名单内** —— 生成物不得引用未定义的名字。
///
/// 首刀（B8-3）只收「依赖全为原始类型」的 6 个：`ProviderConfig` / `Model` /
/// `ModelParamOverrides` 的闭包里有**可选性不兼容**的成员（见 §91.3 交付说明），
/// 待前端消费点改完再加。
fn frontend_dto_decls(cfg: &ts_rs::Config) -> Vec<DtoDecl> {
    vec![
        // Provider 域（`axagent-harness::types::provider_model`）
        DtoDecl::new(PROVIDER, "ProviderType", gen_export::<ProviderType>(cfg)),
        DtoDecl::new(PROVIDER, "ModelType", gen_export::<ModelType>(cfg)),
        DtoDecl::new(PROVIDER, "ModelCapability", gen_export::<ModelCapability>(cfg)),
        DtoDecl::new(PROVIDER, "ProviderKey", gen_export::<ProviderKey>(cfg)),
        DtoDecl::new(PROVIDER, "ProviderProxyConfig", gen_export::<ProviderProxyConfig>(cfg)),
        // Workflow 域（`axagent-harness::workflow_types`）
        DtoDecl::new(WORKFLOW, "SubWorkflowOrigin", gen_export::<SubWorkflowOrigin>(cfg)),
    ]
}

/// 一条前端 DTO 声明：所属分节 + 类型名 + ts-rs 投影出的 TS 文本。
struct DtoDecl {
    section: &'static str,
    name: &'static str,
    decl: String,
}

impl DtoDecl {
    fn new(section: &'static str, name: &'static str, decl: String) -> Self {
        Self { section, name, decl }
    }
}

const PROVIDER: &str = "Provider 域（`axagent-harness::types::provider_model`）";
const WORKFLOW: &str = "Workflow 域（`axagent-harness::workflow_types`）";

/// `ts_rs::TS::decl` 的输出是 `type X = …`（不加 `export`，因为原用途是「同文件内校验」）。
/// 本函数只补一个 `export ` 前缀，其余逐字保留（含 Rust doc 注释投影出的 JSDoc）。
fn gen_export<T: TS>(cfg: &ts_rs::Config) -> String {
    let decl = T::decl(cfg);
    let mut out = String::new();
    let mut exported = false;
    for line in decl.lines() {
        if !exported && line.starts_with("type ") {
            out.push_str("export ");
            exported = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    // 假设失效就当场炸，不要静默产出一个「没有 export 的生成物」——那会让
    // `src/types/index.ts` 的 re-export 变成 no-op，而症状只是别的文件少几个类型。
    assert!(exported, "ts-rs 未输出 `type ` 声明行（生成器假设失效）");
    out
}

/// 扫描生成物里引用的**名单外大写标识符**（= 未定义的 TS 类型名）。
///
/// 先剥掉注释与字符串字面量：Rust doc 注释里有 `OpenAI` / `TypeSafe` 这类词，字符串
/// 字面量里有 `"TextChat"` 这类枚举成员 —— 它们不是类型引用，误判会让守卫变成噪音。
fn foreign_type_refs(body: &str, known: &[&str]) -> Vec<String> {
    // TS 内置泛型：ts-rs 对 `Vec<T>` 输出 `Array<T>`，不属于外部依赖。
    const BUILTIN: &[&str] = &["Array", "Record"];

    let mut code = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            },
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            },
            '"' | '\'' | '`' => {
                let quote = ch;
                let mut escaped = false;
                for c in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == quote {
                        break;
                    }
                }
            },
            _ => code.push(ch),
        }
    }

    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut Vec<String>| {
        let starts_upper = word.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        if starts_upper && !known.contains(&word.as_str()) && !BUILTIN.contains(&word.as_str()) {
            out.push(word.clone());
        }
        word.clear();
    };
    for ch in code.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            word.push(ch);
        } else {
            flush(&mut word, &mut out);
        }
    }
    flush(&mut word, &mut out);
    out.sort();
    out.dedup();
    out
}

/// 生成 `src/types/generated/dto.ts` —— 前端**直接 re-export 消费**的那一份。
///
/// 与 `ipc.ts` 的分工（两份都不能省）：
/// - `ipc.ts`：不加 `export`，只回答「后端 DTO 有没有变」（字节级漂移门）；
/// - 本文件：加 `export`，是前端类型的**唯一来源**，手写定义在 `src/types/index.ts` 里删除。
fn generate_frontend_dto(out_dir: &Path) {
    fs::create_dir_all(out_dir).expect("前端 DTO 生成：创建输出目录失败");
    eprintln!("Generating frontend-consumable DTO type definitions...");
    let output_path = out_dir.join("dto.ts");
    fs::write(&output_path, render_frontend_dto())
        .unwrap_or_else(|e| panic!("前端 DTO 生成：写入 {} 失败: {}", output_path.display(), e));
    eprintln!("✅ Generated frontend DTO types: {}", output_path.display());
}

/// 渲染 `dto.ts` 全文（不落盘，便于单测逐字断言）。
fn render_frontend_dto() -> String {
    let cfg = frontend_dto_cfg();
    let decls = frontend_dto_decls(&cfg);
    let names: Vec<&str> = decls.iter().map(|d| d.name).collect();
    let body: String = decls.iter().map(|d| d.decl.as_str()).collect();
    let foreign = foreign_type_refs(&body, &names);
    assert!(
        foreign.is_empty(),
        "前端 DTO 生成：生成物引用了名单外的类型 {:?} —— 把它们加进 frontend_dto_decls()（依赖在前），\
         否则生成物缺定义、`npm run typecheck` 会在消费方报 Cannot find name",
        foreign
    );

    let mut output = String::new();
    output.push_str("// 此文件由 schema-gen 自动生成，请勿手动编辑\n");
    output.push_str("// 修改 Rust DTO 后请重新运行: cargo run -p schema-gen -- frontend-dto\n");
    output.push_str("//\n");
    output.push_str(
        "// 与同目录 `ipc.ts` 的分工：那一份只做「后端 DTO 有没有变」的同步校验（类型不加\n",
    );
    output.push_str(
        "// export，无人消费）；本文件是**前端真正消费**的类型 —— 权威定义在后端结构体，\n",
    );
    output.push_str("// `src/types/index.ts` 直接 re-export，不再维护第二份手写定义。\n");
    output.push_str("//\n");
    output.push_str("// 映射口径（由 ts-rs 按 serde 属性推导，禁止在本文件手改）：\n");
    output.push_str("// - `#[serde(rename_all = \"camelCase\")]` ⇒ 字段名 camelCase；\n");
    output
        .push_str("// - `Option<T>` ⇒ `T | null`；带 `skip_serializing_if` 的 ⇒ 可选字段 `?:`；\n");
    output.push_str(
        "// - `i64/u64` ⇒ `number`（ts-rs 默认 `bigint`，与 JSON 线上类型不符，生成器已改）。\n\n",
    );

    let mut section: Option<&str> = None;
    for d in &decls {
        if section != Some(d.section) {
            if section.is_some() {
                output.push('\n');
            }
            output.push_str(
                "// ============================================================================\n",
            );
            output.push_str(&format!("// {}\n", d.section));
            output.push_str(
                "// ============================================================================\n\n",
            );
            section = Some(d.section);
        }
        output.push_str(&d.decl);
        output.push('\n');
    }

    output.push_str(&format!("// Total DTO types: {}\n", decls.len()));

    output
}

/// 检查前端类型定义与后端 DTO 的同步状态
/// 生成物漂移检查 —— **失败必须让进程非零退出**。
///
/// 旧实现只 `eprintln!` 警告、永不返回非零，且判据仅是「类型名有没有出现在生成文件里」
/// ⇒ 接进 CI 只会得到一道恒绿门（少一个字段、类型改名、生成器改行为，它全看不见）。
/// 现在除了名字在不在，还做**字节级比对**：把两个生成物重跑到临时目录，与在库版本逐字节比。
///
/// 字节比对成立的前提是**生成过程确定性**（头部注释是常量、无时间戳，ts-rs 按声明序输出）。
/// 将来谁给生成器加了时间戳，这条门会当场红，而不是静默失去覆盖面。
fn run_check(ts_types_dir: &Path) {
    eprintln!("Checking Tauri IPC type synchronization...");
    let mut failures: Vec<String> = Vec::new();

    // 1) 注册表里的每个 DTO 都必须出现在生成的 ipc.ts 中（漏登记 ⇒ 该类型前端拿不到）
    let generated_path = ts_types_dir.join("ipc.ts");
    match fs::read_to_string(&generated_path) {
        Ok(content) => {
            for (dto_name, _module) in IPC_DTO_TYPES {
                if !content.contains(&dto_name.to_string()) {
                    failures.push(format!(
                        "ipc.ts 里没有类型 `{dto_name}`（注册表 IPC_DTO_TYPES 有它）"
                    ));
                }
            }
        },
        Err(e) => failures.push(format!("读不到在库生成物 {}: {e}", generated_path.display())),
    }

    // 2) 事件名契约
    check_event_names_sync(ts_types_dir, &mut failures);

    // 3) 字节级：在库版本 == 现在重跑一遍的版本
    check_generated_drift(ts_types_dir, &mut failures);

    if failures.is_empty() {
        eprintln!(
            "✅ 生成物与权威一致（注册表 {} 个 DTO + 事件名 + 字节级比对）",
            IPC_DTO_TYPES.len()
        );
        return;
    }
    eprintln!(
        "❌ 生成物漂移 {} 项 —— 跑 `cargo run -p schema-gen -- all` 重新生成：",
        failures.len()
    );
    for f in &failures {
        eprintln!("   - {f}");
    }
    std::process::exit(1);
}

/// 把两个前端消费的生成物重跑到临时目录，与在库版本逐字节比对。
fn check_generated_drift(ts_types_dir: &Path, failures: &mut Vec<String>) {
    let tmp = std::env::temp_dir().join(format!("schema-gen-check-{}", std::process::id()));
    if let Err(e) = fs::create_dir_all(&tmp) {
        failures.push(format!("无法创建临时比对目录 {}: {e}", tmp.display()));
        return;
    }
    generate_ipc_types(&tmp);
    generate_event_names(&tmp);
    generate_frontend_dto(&tmp);
    for name in ["ipc.ts", "events.ts", "dto.ts"] {
        let committed = ts_types_dir.join(name);
        let fresh = tmp.join(name);
        let (Ok(old), Ok(new)) = (fs::read_to_string(&committed), fs::read_to_string(&fresh))
        else {
            failures.push(format!("{name}: 在库版本或重生成版本读不到"));
            continue;
        };
        if old != new {
            let line = old
                .lines()
                .zip(new.lines())
                .position(|(a, b)| a != b)
                .map(|i| i + 1)
                .unwrap_or_else(|| old.lines().count().max(new.lines().count()));
            failures.push(format!(
                "{name} 与生成器输出不一致（首个差异在第 {line} 行；在库 {} 字节 vs 重跑 {} 字节）",
                old.len(),
                new.len()
            ));
        }
    }
    if let Err(e) = fs::remove_dir_all(&tmp) {
        eprintln!("⚠️  临时比对目录未清理（不影响判定）: {} — {e}", tmp.display());
    }
}

/// 校验 `src/types/generated/events.ts` 与 `IpcEventName` 逐项一致
///
/// 判据用「枚举成员的 `"name",` 行是否都出现在生成物中」而不是整文件字节比对：
/// 前者对生成器的注释措辞改动容忍，后者一旦改注释就报「脏」，
/// 会让维护者习惯性重跑而不看差异（门禁的出口不严比没有更糟）。
fn check_event_names_sync(ts_types_dir: &Path, failures: &mut Vec<String>) {
    let events_path = ts_types_dir.join("events.ts");
    let actual = match fs::read_to_string(&events_path) {
        Ok(s) => s,
        Err(e) => {
            failures.push(format!("读不到在库生成物 {}: {e}", events_path.display()));
            return;
        },
    };
    let missing: Vec<&str> = axagent_harness::IpcEventName::ALL
        .iter()
        .map(|e| e.as_str())
        .filter(|name| !actual.contains(&format!("\"{name}\",")))
        .collect();

    if missing.is_empty() {
        eprintln!(
            "✅ IPC 事件名契约同步检查通过（{} 个成员）",
            axagent_harness::IpcEventName::ALL.len()
        );
    } else {
        for name in &missing {
            failures.push(format!("events.ts 落后于 IpcEventName，缺成员 `{name}`"));
        }
    }
}

// ============================================================================
// 单元测试（⚠ 必须留在文件末尾：插在函数之间会触发 clippy::items_after_test_module）
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// 生成物金标：与 `tests/golden/frontend-dto.ts` **逐字**一致。
    ///
    /// 这是「**Rust 结构体改了，生成物就会变**」的判据（本批验收第 4 条）：
    /// 字段增删 / 改名 / `Option` ⇄ 非 Option / serde 属性变更 / 枚举变体改名 / ts-rs 版本
    /// 升级导致输出格式变化，都会让 `render_frontend_dto()` 与金标出现差异 ⇒ 本条当场红。
    /// 它同时是「`src/types/generated/dto.ts` 确实由本函数产出」的独立证词 —— 金标写在 crate
    /// 内（不读仓库根那份产物），所以「两边一起被人手改」这种共谋无从发生。
    const GOLDEN: &str = include_str!("../tests/golden/frontend-dto.ts");

    /// camelCase 命名：字段名由 serde 的 `rename_all = "camelCase"` 投影，不是 Rust 的
    /// snake_case。判据点两个（一个单词字段、一个多词字段），并反查 snake_case 不得漏出。
    #[test]
    fn camel_case_field_names_come_from_serde() {
        let key = gen_export::<ProviderKey>(&frontend_dto_cfg());
        assert!(key.contains("providerId: string"), "缺 camelCase 字段 providerId：{key}");
        assert!(key.contains("keyEncrypted: string"), "缺 camelCase 字段 keyEncrypted：{key}");
        assert!(key.contains("rotationIndex: number"), "缺 camelCase 字段 rotationIndex：{key}");
        assert!(!key.contains("provider_id"), "snake_case 漏进生成物：{key}");
        assert!(!key.contains("key_encrypted"), "snake_case 漏进生成物：{key}");
    }

    /// `Option<T>` → `T | null`；`i64` → `number`（不是 ts-rs 默认的 `bigint`）。
    #[test]
    fn option_maps_to_nullable_number() {
        let key = gen_export::<ProviderKey>(&frontend_dto_cfg());
        assert!(
            key.contains("lastValidatedAt: number | null"),
            "Option<i64> 应映射成 `number | null`（含 large_int=number）：{key}"
        );
        assert!(
            key.contains("lastError: string | null"),
            "Option<String> 应映射成 `string | null`：{key}"
        );
        assert!(!key.contains("bigint"), "i64 不得落成 bigint（前端 IPC 是 JSON number）：{key}");
    }

    /// `Option<T>` + `#[serde(skip_serializing_if)]` → **可选**字段 `?:`（可缺席，不只是可空）。
    ///
    /// 判据取 `Model`（不在首刀名单里，但它的 `input_price_per_mtok` 正是这种形态）：
    /// 生成器对「可缺席」与「必发 null」两种 Option 必须给出不同拼写，否则前端无法用
    /// 对象字面量省略该字段。
    #[test]
    fn option_with_skip_serializing_if_maps_to_optional_field() {
        let model =
            gen_export::<axagent_harness::types::provider_model::Model>(&frontend_dto_cfg());
        assert!(
            model.contains("inputPricePerMtok?: number | null"),
            "skip_serializing_if 的 Option 应映射成可选字段 `?:`：{model}"
        );
        // 反手抄：不带 skip_serializing_if 的 Option 仍是必填可空（同一张结构体里两种形态并存）。
        assert!(
            model.contains("groupName: string | null"),
            "普通 Option 不应被写成可选字段：{model}"
        );
    }

    /// 枚举联合类型：`rename_all` / 逐变体 `rename` 都要投影到字符串字面量。
    #[test]
    fn enum_variants_follow_serde_spelling() {
        let pt = gen_export::<ProviderType>(&frontend_dto_cfg());
        assert!(pt.contains("\"openai_responses\""), "逐变体 rename 丢失：{pt}");
        assert!(pt.contains("\"llama_cpp\""), "逐变体 rename 丢失：{pt}");
        assert!(pt.contains("\"typesafe\""), "lowercase 投影丢失：{pt}");
        let mc = gen_export::<ModelCapability>(&frontend_dto_cfg());
        assert!(mc.contains("\"TextChat\""), "无 rename_all 时应按变体名逐字：{mc}");
    }

    /// 名单自洽：生成物**不得**引用名单外的类型（否则前端拿到的是 Cannot find name）。
    /// 反向负控见下一条 —— 证明这条守卫本身有区分力，而不是恒空。
    #[test]
    fn registry_closure_is_self_contained() {
        let cfg = frontend_dto_cfg();
        let decls = frontend_dto_decls(&cfg);
        let names: Vec<&str> = decls.iter().map(|d| d.name).collect();
        let body: String = decls.iter().map(|d| d.decl.as_str()).collect();
        assert_eq!(foreign_type_refs(&body, &names), Vec::<String>::new());
    }

    /// 负控：引用未登记类型必须被抓出；注释与字符串里的词不得误判。
    #[test]
    fn foreign_ref_guard_has_teeth_and_no_false_positives() {
        let body = "export type A = { b: Ghost, c: Array<B>, };\n\
                    /** 文档里的 OpenAI / TypeSafe 不是类型引用 */\n\
                    export type B = \"TextChat\" | \"Vision\";";
        let hits = foreign_type_refs(body, &["A", "B"]);
        assert_eq!(hits, vec!["Ghost".to_string()]);
    }

    /// 金标比对（本批验收第 4 条的判据本体）。
    #[test]
    fn generated_text_matches_golden_snapshot() {
        assert_eq!(
            render_frontend_dto(),
            GOLDEN,
            "生成物与 tests/golden/frontend-dto.ts 不一致 —— Rust 结构体变了？\
             跑 `cargo run -p schema-gen -- frontend-dto` 重新生成，\
             再把差异同步到 src/types/generated/dto.ts 与金标"
        );
    }
}
