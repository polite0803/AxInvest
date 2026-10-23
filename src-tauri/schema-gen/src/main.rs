// SPDX-License-Identifier: AGPL-3.0-only

//! Schema 生成与 Tauri IPC 类型同步工具
//!
//! 功能:
//! 1. 为 workflow 类型生成 JSON Schema 文档
//! 2. 从 Rust DTO 生成 TypeScript 类型定义，确保前后端一致性
//! 3. 从 `axagent_harness::IpcEventName` 生成前端 IPC 事件名联合类型
//! 4. 校验前端类型定义与后端 DTO / 事件名的同步状态
//!
//! 使用方法:
//! ```bash
//! cargo run -p schema-gen                    # 生成所有类型定义
//! cargo run -p schema-gen -- check           # 仅检查类型同步，不生成
//! cargo run -p schema-gen -- ipc-types       # 仅生成 IPC 类型
//! cargo run -p schema-gen -- event-names     # 仅生成 IPC 事件名联合类型
//! ```

use axagent_harness::agent::{
    AgentCapability, AgentExecuteRequest, AgentInfo, AgentPlan, AgentResult, PlanStep,
};
use axagent_harness::conversation_model::{
    ContentBlock, ConversationMessage, SessionInfo, TokenUsage,
};
use axagent_harness::workflow_types::{
    BackoffType, CompensationConfig, CompensationStrategy, NodeKind, Position, RetryConfig,
    Variable, WorkflowNodeBase,
};
use schemars::schema_for;
use std::fs;
use std::path::Path;
use ts_rs::TS;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = if args.len() > 1 { &args[1] } else { "all" };

    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("路径无父目录")
        .parent()
        .expect("路径无祖父目录");

    let out_dir = project_root.join("docs");
    let ts_types_dir = project_root.join("src").join("types").join("generated");

    match mode {
        "check" => run_check(&ts_types_dir),
        "ipc-types" => generate_ipc_types(&ts_types_dir),
        "event-names" => generate_event_names(&ts_types_dir),
        "workflow-schema" => generate_workflow_schema(&out_dir),
        _ => {
            generate_workflow_schema(&out_dir);
            generate_ipc_types(&ts_types_dir);
            generate_event_names(&ts_types_dir);
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
    for name in ["ipc.ts", "events.ts"] {
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
