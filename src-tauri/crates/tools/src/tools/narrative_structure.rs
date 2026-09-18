// SPDX-License-Identifier: AGPL-3.0-only

//! 叙事结构确定性工具 —— 让文学创作工作流直连 harness 的叙事引擎。
//!
//! ## 背景
//!
//! `axagent_harness::narrative` 是一个 774 行的叙事结构引擎，提供
//! 「按章节号筛选弧线阶段 / 伏笔埋设回收 / 交汇点触发」的**确定性**计算
//! （`NarrativeStructure::get_chapter_instructions` + `to_prompt_constraints`）。
//! 但该模块此前全仓零调用：工作流改用 LLM 节点去「生成」本该确定性算出来的
//! 逐章结构指令，既不确定、产出键名又与消费端对不上。
//!
//! 本模块把引擎暴露为两个工作流 ToolNode 可调用的工具，替换掉那次 LLM 调用：
//!
//! 1. `narrative_chapter_instructions` —— 给定完整叙事结构 JSON + 章节号，
//!    返回该章的确定性结构约束（引擎产出的中文约束文本 + 结构化指令）。
//! 2. `narrative_structure_persist` —— 把校验通过的叙事结构落库到
//!    `narrative_structures` 表。
//!
//! ## 为什么必须放在 tools crate
//!
//! 工作流 ToolNode 经 `ToolResolver` → `state.local_tool_registry` 按工具名解析，
//! 而 `local_tool_registry` 由 `UnifiedToolRegistry::new()` 自动 `register_all()`
//! 填充（`crates/tools/src/registry.rs:704-711`）。本模块的全部构件
//! （harness 引擎 / dao 落库 / 全局 DB 句柄）在本层均可达，且 tools crate 的
//! `Cargo.toml` 已依赖 `axagent-harness` 与 `axagent-dao`，零新增依赖。
//!
//! ## 输出契约（硬依赖，改种子 input_mapping 前必读）
//!
//! 生产环境的 ToolNode 走 ToolResolver 回调路径，工具返回的 `ToolResult.content`
//! 字符串会被两层包裹：
//!
//! ```text
//! variables[<tool_node_id>] = {
//!   "tool_name": "<工具名>",
//!   "result": { "content": "<ToolResult.content 原文>" },
//!   "node_id": "<node id>"
//! }
//! ```
//!
//! 包裹由 `rt-workflow` 的 `tool_executor.rs:177-188`（外层 `result`）
//! 与 `src/init/services.rs:1972-1976`（内层 `content`）共同产生。
//! 因此下游 input_mapping 必须写 `<tool_node_id>.result.content.<字段名>`；
//! `resolve_var_path` 在导航进入字符串字段时会自动 JSON 解析
//! （`executors/mod.rs:119-147`），所以多层字段可直接点号穿透。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult};
use axagent_dao::axagent_entities::narrative_structures;
use axagent_dao::repo::narrative as narrative_repo;
use axagent_harness::narrative::{ChapterStructureInstruction, NarrativeStructure};
use sea_orm::Set;
use serde_json::Value;

/// 工具名 1：按章节号取确定性结构指令。
pub const NARRATIVE_CHAPTER_INSTRUCTIONS_TOOL: &str = "narrative_chapter_instructions";

/// 工具名 2：叙事结构落库。
pub const NARRATIVE_STRUCTURE_PERSIST_TOOL: &str = "narrative_structure_persist";

/// `chapter` 参数为对象时的取值键优先级。
///
/// Loop 迭代变量在不同模板里可能叫 num / index / chapter / chapterNumber / no / number，
/// 这里统一按固定优先级取值，避免下游模板改动导致工具静默取不到章节号。
///
/// `num` 置于首位：文学创作模板的大纲契约（`seed_content_media.rs` 的 `lc-outline`
/// 要求顶层数组、元素为 `{num, title, summary, key_events, conflict_idx, focal}`）
/// 与 `lc-draft-loop` 的 `iteratee_var = "chapter"` 都以后者为准，`num` 是权威键。
const CHAPTER_KEY_PRIORITY: [&str; 6] =
    ["num", "index", "chapter", "chapterNumber", "no", "number"];

// ── 参数解析助手 ──

/// 取必填参数；缺失或显式 null 时报错（不静默兜底）。
fn require_input<'a>(input: &'a Value, key: &str) -> Result<&'a Value, ToolError> {
    input
        .get(key)
        .filter(|v| !v.is_null())
        .ok_or_else(|| ToolError::invalid_input(format!("缺少必填参数 {key}")))
}

/// 取可选的字符串参数：缺失 / 非字符串 / 全空白 时返回 None（不报错）。
fn optional_non_empty_str(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 取必填的**非空字符串**参数：缺失 / 非字符串 / 全空白 一律报错。
///
/// 与 [`optional_non_empty_str`] 的区别是**不兜底**。用于 `genre` 这类「缺了就是上游断链」
/// 的字段：`input_mapping` 解析失败会注入 `Value::Null` 而不是跳过节点，若这里静默补一个
/// 默认体裁，就会把「上游断链」伪装成「上游明确给了这个体裁」——这是往库里写伪造标签的
/// fail-open，零容忍。
fn require_non_empty_str(input: &Value, key: &str) -> Result<String, ToolError> {
    optional_non_empty_str(input, key)
        .ok_or_else(|| ToolError::invalid_input(format!("缺少必填参数 {key}（或不是非空字符串）")))
}

/// `name` 缺省自理派生：`<genre>-<yyyyMMddHHmmss>`。
///
/// 之所以自派生：本工具的主调用方是 `lc-conceive` 之后的 ToolNode，而 `lc-conceive`
/// 的契约里没有可直接当「作品名」用的字段 —— 强制 `name` 必填只会逼调用方现编一个，
/// 或干脆调不通。时间戳走真实时钟（`chrono::Utc::now()`），非确定性，测试只断言形态。
///
/// `genre` 已由 [`require_non_empty_str`] 保证非空，故此处直接收 `&str`（无退化分支）。
fn derive_structure_name(genre: &str) -> String {
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    format!("{genre}-{stamp}")
}

/// `NarrativeStructure` 的顶层字段名（`harness/src/narrative.rs:19-23`，
/// `#[serde(rename_all = "camelCase")]`）。用于区分「真结构本体」与「节点输出信封」。
const NARRATIVE_STRUCTURE_TOP_LEVEL_KEYS: [&str; 3] = ["arcs", "confluences", "foreshadows"];

/// 信封解包计数上限，按「消费一次 `content` 载荷层」计：
/// 1 层覆盖 AgentNode 形 `{role, model, content: "<JSON>", ...}`；
/// 2 层覆盖 ToolNode 回调形 `{tool_name, result: {content: "<JSON>"}, node_id}`（`result` 是承载层，不计数）。
const MAX_ENVELOPE_UNWRAP_DEPTH: usize = 2;

/// 信封解包的一步。
enum EnvelopeStep<'a> {
    /// 承载层（`result`）：只是外壳，不消耗层数。
    Carrier(&'a Value),
    /// 载荷层（`content` 字符串）：消费一层。
    Payload(&'a str),
}

/// 探测信封：**仅在能确认是信封时**返回 Some，避免把合法结构误判成信封。
///
/// 判定条件（全部满足才算信封）：
/// 1. 是 JSON 对象；
/// 2. **不含**任一 `NarrativeStructure` 顶层键（`arcs` / `confluences` / `foreshadows`）；
/// 3. 含非空 `result`（承载层），或含字符串类型的 `content`（载荷层）。
fn envelope_step(v: &Value) -> Option<EnvelopeStep<'_>> {
    let Value::Object(map) = v else { return None };
    if NARRATIVE_STRUCTURE_TOP_LEVEL_KEYS.iter().any(|k| map.contains_key(*k)) {
        return None;
    }
    if let Some(inner) = map.get("result").filter(|v| !v.is_null()) {
        return Some(EnvelopeStep::Carrier(inner));
    }
    match map.get("content") {
        Some(Value::String(s)) => Some(EnvelopeStep::Payload(s.as_str())),
        _ => None,
    }
}

/// 把 `structure` 参数解析为 `NarrativeStructure`。
///
/// 兼容三种形态：
/// 1. `NarrativeStructure` JSON 对象；
/// 2. 承载该 JSON 的字符串（Loop/Agent 上游常把结构压成字符串）；
/// 3. 节点输出**信封**（最多 [`MAX_ENVELOPE_UNWRAP_DEPTH`] 层）—— 调用方若直接把
///    整个节点输出当 `structure` 传，裸报 `missing field arcs` 会完全指不到「多包了一层」。
///
/// 任何解析失败都返回 Err —— 绝不退化成空结构，否则会伪造「本章无约束」的假结果。
fn parse_narrative_structure(raw: &Value) -> Result<NarrativeStructure, ToolError> {
    parse_narrative_structure_inner(raw, 0)
}

/// [`parse_narrative_structure`] 的递归体。`unwrapped` = 已消费的 `content` 载荷层数。
fn parse_narrative_structure_inner(
    raw: &Value,
    unwrapped: usize,
) -> Result<NarrativeStructure, ToolError> {
    // ① 字符串：解析成 JSON 后继续；字符串本身不计信封层数（它是信封的产物，不是信封）。
    if let Value::String(s) = raw {
        let parsed = serde_json::from_str::<Value>(s)
            .map_err(|e| ToolError::invalid_input(format!("structure 字符串不是合法 JSON: {e}")))?;
        return parse_narrative_structure_inner(&parsed, unwrapped);
    }

    // ② 信封：解包后重试。
    if let Some(step) = envelope_step(raw) {
        match step {
            EnvelopeStep::Carrier(inner) => {
                return parse_narrative_structure_inner(inner, unwrapped);
            },
            EnvelopeStep::Payload(s) => {
                if unwrapped >= MAX_ENVELOPE_UNWRAP_DEPTH {
                    let keys = if let Value::Object(map) = raw {
                        map.keys().cloned().collect::<Vec<_>>().join(", ")
                    } else {
                        "<非对象>".to_string()
                    };
                    return Err(ToolError::invalid_input(format!(
                        "structure 信封嵌套超过 {MAX_ENVELOPE_UNWRAP_DEPTH} 层：已解包信封 {unwrapped} 层后仍未出现叙事结构字段（当前信封键: {keys}）"
                    )));
                }
                let parsed = serde_json::from_str::<Value>(s).map_err(|e| {
                    ToolError::invalid_input(format!(
                        "structure 已解包信封第 {} 层，但其 content 不是合法 JSON: {e}",
                        unwrapped + 1
                    ))
                })?;
                return parse_narrative_structure_inner(&parsed, unwrapped + 1);
            },
        }
    }

    // ③ 直接按结构本体反序列化（结构本体 / 无法识别的对象 / 其它类型都收敛到这里）。
    serde_json::from_value::<NarrativeStructure>(raw.clone()).map_err(|e| {
        let hint = if unwrapped > 0 {
            format!("（已尝试解包信封 {unwrapped} 层）")
        } else {
            String::new()
        };
        ToolError::invalid_input(format!("structure 无法解析为 NarrativeStructure{hint}: {e}"))
    })
}

/// 从 `chapter` 参数提取章节号。
///
/// 宽容接受三种形态：数字（含 2.0 这类整值浮点）、数字字符串、含章节号的对象
/// （按 `CHAPTER_KEY_PRIORITY` 取第一个命中的键）。
/// 取不到时返回 Err 并带上原始值 —— 不允许默认成 1 或 0 伪造数据。
fn extract_chapter(raw: &Value) -> Result<u32, ToolError> {
    chapter_from_value(raw).ok_or_else(|| {
        ToolError::invalid_input(format!(
            "无法从 chapter 参数提取章节号: {}",
            serde_json::to_string(raw).unwrap_or_else(|_| "<unserializable>".to_string())
        ))
    })
}

/// 递归从任意 JSON 值里提取章节号；命中不了返回 None。
fn chapter_from_value(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => number_to_chapter(n),
        Value::String(s) => s.trim().parse::<u32>().ok(),
        Value::Object(map) => {
            CHAPTER_KEY_PRIORITY.iter().find_map(|k| map.get(*k).and_then(chapter_from_value))
        },
        _ => None,
    }
}

/// JSON 数字 → 章节号；只接受非负整值（2 与 2.0 可以，-1 与 2.5 不行）。
fn number_to_chapter(n: &serde_json::Number) -> Option<u32> {
    if let Some(u) = n.as_u64() {
        return u32::try_from(u).ok();
    }
    let f = n.as_f64()?;
    if f < 0.0 || f.fract() != 0.0 {
        return None;
    }
    u32::try_from(f as u64).ok()
}

/// 引擎指令 → 工具返回载荷（字段名即下游 input_mapping 的契约）。
fn instruction_to_payload(instruction: &ChapterStructureInstruction) -> Value {
    serde_json::json!({
        "chapter": instruction.chapter,
        "constraints": instruction.to_prompt_constraints(),
        "instruction": instruction,
        "is_empty": instruction.is_empty(),
    })
}

/// 纯计算：解析参数 → 调引擎 → 组装载荷。抽成自由函数以便单测直连（无需 ToolContext / DB）。
fn compute_chapter_instructions(
    raw_structure: &Value,
    raw_chapter: &Value,
) -> Result<Value, ToolError> {
    let structure = parse_narrative_structure(raw_structure)?;
    let chapter = extract_chapter(raw_chapter)?;
    let instruction = structure.get_chapter_instructions(chapter);
    Ok(instruction_to_payload(&instruction))
}

// ═══════════════════════════════════════════════════════════════════
// 工具 1：narrative_chapter_instructions
// ═══════════════════════════════════════════════════════════════════

/// 按章节号从叙事结构中提取确定性结构指令。
///
/// 对应删除掉的 LLM 节点 `lc-structure-injector`：该节点原本让模型「读结构、
/// 逐章生成指令」，而这一步本就是 `get_chapter_instructions` 的等值筛选。
pub struct NarrativeChapterInstructionsTool;

#[async_trait::async_trait]
impl Tool for NarrativeChapterInstructionsTool {
    fn name(&self) -> &str {
        NARRATIVE_CHAPTER_INSTRUCTIONS_TOOL
    }

    fn description(&self) -> &str {
        "确定性叙事结构引擎：给定完整叙事结构（arcs/confluences/foreshadows）与章节号，\
         返回该章的弧线推进/伏笔埋设回收/交汇点触发约束。纯计算，不调用 LLM。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "structure": {
                    "type": ["object", "string"],
                    "description": "叙事结构：JSON 对象，或承载同一 JSON 的字符串。需含 arcs/confluences/foreshadows 三段。"
                },
                "chapter": {
                    "type": ["number", "string", "object"],
                    "description": "当前章节号。接受数字、数字字符串，或含章节号的对象（键优先级 num > index > chapter > chapterNumber > no > number）。取不到即报错，不做默认值。"
                }
            },
            "required": ["structure", "chapter"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Automation
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::ContentCreation
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn is_idempotent(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let raw_structure = require_input(&input, "structure")?;
        let raw_chapter = require_input(&input, "chapter")?;
        let payload = compute_chapter_instructions(raw_structure, raw_chapter)?;
        let content = serde_json::to_string(&payload)
            .map_err(|e| ToolError::execution_failed(format!("序列化章节结构指令失败: {e}")))?;
        Ok(ToolResult::success(content))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 工具 2：narrative_structure_persist
// ═══════════════════════════════════════════════════════════════════

/// 把叙事结构落库到 `narrative_structures` 表。
///
/// 走 `axagent_dao::repo::narrative::insert_narrative_structure`（新增，version=1）。
/// `structure` 列是**单列 JSON 文本**（原 `v126` 迁移；
/// 该迁移文件已于 2026-09-16 随 74 个迁移文件一起删除，现在该列由声明式引擎按
/// `entities/src/narrative_structures.rs` 的声明建出），
/// 写入前会重新序列化校验过的 `NarrativeStructure`，保证列内只存规范形态。
pub struct NarrativeStructurePersistTool;

#[async_trait::async_trait]
impl Tool for NarrativeStructurePersistTool {
    fn name(&self) -> &str {
        NARRATIVE_STRUCTURE_PERSIST_TOOL
    }

    fn description(&self) -> &str {
        "把叙事结构（arcs/confluences/foreshadows）校验后落库到 narrative_structures 表，\
         返回新建记录的 id / name / version。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "叙事结构名称。可省略：缺省或空白时自派生为 `<genre>-<yyyyMMddHHmmss>`。"
                },
                "genre": { "type": "string", "description": "体裁标签，如 小说/剧本/散文（不能为空）" },
                "structure": {
                    "type": ["object", "string"],
                    "description": "叙事结构：JSON 对象，或承载同一 JSON 的字符串；也可直接传节点输出信封（最多解包 2 层）。"
                }
            },
            "required": ["genre", "structure"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Automation
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::ContentCreation
    }

    fn is_idempotent(&self) -> bool {
        false
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // genre 必填且非空：上游断链时宁可在此报错，也不能落一个伪造的体裁标签。
        // 该校验必须排在 DB 检查之前，否则「参数错」会被误报成「数据库未初始化」。
        let genre = require_non_empty_str(&input, "genre")?;
        let raw_structure = require_input(&input, "structure")?;
        // 先校验可解析，再回序列化：写入列内的永远是规范 JSON，杜绝脏文本入库。
        let structure = parse_narrative_structure(raw_structure)?;
        let structure_json = serde_json::to_string(&structure)
            .map_err(|e| ToolError::execution_failed(format!("序列化叙事结构失败: {e}")))?;

        // name 可省略：缺省时自派生，避免逼调用方现编一个「作品名」。
        let name =
            optional_non_empty_str(&input, "name").unwrap_or_else(|| derive_structure_name(&genre));

        let db = crate::global_state::get_sea_db().ok_or_else(|| {
            ToolError::execution_failed("数据库未初始化，无法落库叙事结构".to_string())
        })?;

        let now = chrono::Utc::now().timestamp_millis();
        let id = uuid::Uuid::new_v4().to_string();
        let active = narrative_structures::ActiveModel {
            id: Set(id),
            name: Set(name),
            description: Set(None),
            genre: Set(genre),
            structure: Set(structure_json),
            is_template: Set(false),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        };

        let row = narrative_repo::insert_narrative_structure(db.as_ref(), active)
            .await
            .map_err(|e| ToolError::execution_failed(format!("叙事结构落库失败: {e}")))?;

        let payload = serde_json::json!({
            "id": row.id,
            "name": row.name,
            "version": row.version,
        });
        let content = serde_json::to_string(&payload)
            .map_err(|e| ToolError::execution_failed(format!("序列化落库结果失败: {e}")))?;
        Ok(ToolResult::success(content))
    }
}

// ═══════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════
// 注意：本模块必须位于文件最末尾，否则触发 clippy::items_after_test_module。

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 合法样本：1 条弧线 + stage@chapter=2 + 1 个伏笔 setup@chapter=3。
    fn sample_structure() -> Value {
        json!({
            "arcs": [{
                "id": "arc-1",
                "arcType": "transformative",
                "subject": "主角",
                "want": "找到真相",
                "need": "面对恐惧",
                "stages": [{
                    "name": "觉醒之章",
                    "chapter": 2,
                    "description": "角色意识到自己被欺骗"
                }],
                "currentProgress": 10.0
            }],
            "confluences": [],
            "foreshadows": [{
                "id": "fs-1",
                "setupChapter": 3,
                "payoffChapter": null,
                "status": "setup",
                "description": "神秘信件出现",
                "payoffDescription": null,
                "relatedArcs": []
            }]
        })
    }

    #[test]
    fn constraints_contain_stage_name_for_matching_chapter() {
        let payload = compute_chapter_instructions(&sample_structure(), &json!(2))
            .expect("合法结构 + 章节号 2 应计算成功");
        assert_eq!(payload["chapter"], json!(2));
        assert_eq!(payload["is_empty"], json!(false));
        let constraints = payload["constraints"].as_str().unwrap_or_default();
        assert!(
            constraints.contains("觉醒之章"),
            "constraints 应包含该章阶段名，实际: {constraints}"
        );
    }

    #[test]
    fn structure_accepted_as_json_string() {
        let raw = serde_json::to_string(&sample_structure()).expect("序列化样本");
        let payload =
            compute_chapter_instructions(&json!(raw), &json!(2)).expect("字符串形态应被接受");
        assert_eq!(payload["is_empty"], json!(false));
    }

    #[test]
    fn chapter_accepted_from_object_and_numeric_string() {
        for chapter in
            [json!({"num": 2}), json!({"index": 2}), json!({"chapter": 2}), json!("2"), json!(2.0)]
        {
            let payload = compute_chapter_instructions(&sample_structure(), &chapter)
                .unwrap_or_else(|e| panic!("chapter={chapter} 应被接受，实际错误: {e}"));
            assert_eq!(payload["chapter"], json!(2));
        }
    }

    #[test]
    fn chapter_object_prefers_num() {
        // num 是文学创作模板的权威键（lc-outline 大纲元素契约），优先于其余别名。
        let payload = compute_chapter_instructions(
            &sample_structure(),
            &json!({"number": 3, "chapter": 3, "index": 3, "num": 2}),
        )
        .expect("对象章节号应可提取");
        assert_eq!(payload["chapter"], json!(2));
    }

    #[test]
    fn chapter_object_falls_back_to_index_without_num() {
        let payload = compute_chapter_instructions(
            &sample_structure(),
            &json!({"number": 3, "chapter": 3, "index": 2}),
        )
        .expect("对象章节号应可提取");
        assert_eq!(payload["chapter"], json!(2));
    }

    #[test]
    fn chapter_full_outline_item_shape_is_accepted() {
        // lc-outline 契约的元素形状：{num, title, summary, key_events, conflict_idx, focal}
        let payload = compute_chapter_instructions(
            &sample_structure(),
            &json!({
                "num": 2,
                "title": "觉醒",
                "summary": "主角意识到真相",
                "key_events": ["神秘信件"],
                "conflict_idx": 1,
                "focal": "主角"
            }),
        )
        .expect("大纲章节元素应可提取章节号");
        assert_eq!(payload["chapter"], json!(2));
        assert_eq!(payload["is_empty"], json!(false));
    }

    #[test]
    fn missing_chapter_returns_err_not_default() {
        let err = compute_chapter_instructions(&sample_structure(), &json!({}))
            .expect_err("取不到章节号必须报错，不得默认成 1 或 0");
        assert!(
            err.to_string().contains("无法从 chapter 参数提取章节号"),
            "错误信息应指明无法提取章节号，实际: {err}"
        );
    }

    #[test]
    fn invalid_chapter_type_returns_err() {
        assert!(compute_chapter_instructions(&sample_structure(), &json!(true)).is_err());
        assert!(compute_chapter_instructions(&sample_structure(), &json!("第二章")).is_err());
        assert!(compute_chapter_instructions(&sample_structure(), &json!(-1)).is_err());
    }

    #[test]
    fn malformed_structure_returns_err_not_empty_structure() {
        assert!(compute_chapter_instructions(&json!("这不是 JSON"), &json!(2)).is_err());
        // 缺 fields 的对象不是合法 NarrativeStructure
        assert!(compute_chapter_instructions(&json!({"arcs": []}), &json!(2)).is_err());
    }

    #[test]
    fn chapter_without_elements_is_not_error() {
        let payload =
            compute_chapter_instructions(&sample_structure(), &json!(99)).expect("空章节是合法的");
        assert_eq!(payload["is_empty"], json!(true));
        let constraints = payload["constraints"].as_str().unwrap_or_default();
        assert!(
            constraints.contains("本章无特殊叙事结构约束"),
            "空章节应返回引擎兜底原文，实际: {constraints}"
        );
    }

    #[test]
    fn foreshadow_setup_chapter_is_reported() {
        let payload =
            compute_chapter_instructions(&sample_structure(), &json!(3)).expect("章节 3 应可计算");
        let instruction = &payload["instruction"];
        assert_eq!(instruction["foreshadowInstructions"][0]["foreshadowId"], json!("fs-1"));
    }

    #[test]
    fn persist_rejects_unparseable_structure() {
        assert!(parse_narrative_structure(&json!({"arcs": "not-an-array"})).is_err());
        assert!(parse_narrative_structure(&json!(12345)).is_err());
    }

    // ── 改动 1：name 可选 + 自派生；genre 硬必填 ──

    #[test]
    fn persist_name_derived_from_genre_when_missing() {
        let name = derive_structure_name("科幻");
        let re = regex::Regex::new(r"^科幻-\d{14}$").expect("正则应编译");
        assert!(re.is_match(&name), "name 应形如 <genre>-<14 位时间戳>，实际: {name}");
    }

    /// genre 缺失 ⇒ `Err`。**没有**兜底前缀，绝不能落默认体裁。
    #[tokio::test]
    async fn persist_rejects_missing_genre() {
        let input = json!({ "structure": sample_structure() });
        let err = NarrativeStructurePersistTool
            .call(input, &ToolContext::new("."))
            .await
            .expect_err("genre 缺失时应在参数校验阶段报错");
        let msg = err.to_string();
        assert!(msg.contains("genre"), "错误信息应指向 genre 参数，实际: {msg}");
        assert!(!msg.contains("数据库未初始化"), "genre 校验必须排在 DB 检查之前，实际: {msg}");
    }

    /// genre 为空串 / 全空白 ⇒ `Err`（同样不许兜底）。
    #[tokio::test]
    async fn persist_rejects_blank_genre() {
        for blank in ["", "   "] {
            let input = json!({ "genre": blank, "structure": sample_structure() });
            let err = NarrativeStructurePersistTool
                .call(input, &ToolContext::new("."))
                .await
                .expect_err("genre 为空/全空白时应在参数校验阶段报错");
            let msg = err.to_string();
            assert!(msg.contains("genre"), "错误信息应指向 genre 参数，实际: {msg}");
        }
    }

    #[test]
    fn persist_schema_requires_genre_and_structure_only() {
        let schema = NarrativeStructurePersistTool.input_schema();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required 应是数组")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(required, vec!["genre", "structure"]);
        assert!(!required.contains(&"name"), "name 不应再是必填项");
    }

    #[tokio::test]
    async fn persist_accepts_missing_name_and_fails_only_on_db() {
        // 回归守卫：name 缺省时不得再因「参数 name 不能为空」被拦下。
        // 单测进程内从未调用过 `global_state::set_sea_db`，故必然停在 DB 未初始化这一步。
        let input = json!({ "genre": "科幻", "structure": sample_structure() });
        let err = NarrativeStructurePersistTool
            .call(input, &ToolContext::new("."))
            .await
            .expect_err("无 DB 时应失败");
        let msg = err.to_string();
        assert!(
            msg.contains("数据库未初始化"),
            "name 缺省不应阻塞流程，应只因 DB 未初始化失败，实际: {msg}"
        );
        assert!(!msg.contains("name"), "错误信息不应再指向 name 参数，实际: {msg}");
    }

    // ── 改动 2：structure 信封兼容 ──

    /// 构造 `layers` 层 ToolNode 形信封 `{result: {content: "<下一层 JSON>"}}`，
    /// 最内层为 `structure` 本体。`layers = 1` 即 `{result:{content:"<结构 JSON>"}}`。
    fn envelope_with_layers(structure: &Value, layers: usize) -> Value {
        let mut current = structure.clone();
        for _ in 0..layers {
            let content = serde_json::to_string(&current).expect("序列化信封载荷");
            current = json!({ "result": { "content": content } });
        }
        current
    }

    #[test]
    fn structure_parsed_from_bare_object() {
        let parsed = parse_narrative_structure(&sample_structure()).expect("裸结构对象应直接解析");
        assert_eq!(parsed.arcs.len(), 1);
        assert_eq!(parsed.foreshadows.len(), 1);
    }

    #[test]
    fn structure_unwrapped_from_single_layer_envelope() {
        // ② 包一层 `{result:{content:"<JSON>"}}`
        let raw = envelope_with_layers(&sample_structure(), 1);
        let parsed = parse_narrative_structure(&raw).expect("单层信封应被解包");
        assert_eq!(parsed.arcs.len(), 1);
    }

    #[test]
    fn structure_unwrapped_from_double_nested_envelope() {
        // ③ 包两层
        let raw = envelope_with_layers(&sample_structure(), 2);
        let parsed = parse_narrative_structure(&raw).expect("两层信封应被解包");
        assert_eq!(parsed.arcs.len(), 1);
    }

    #[test]
    fn structure_rejects_envelope_nesting_beyond_limit() {
        // ④ 包三层 ⇒ 超限报错，且错误信息必须能指出是信封问题
        let raw = envelope_with_layers(&sample_structure(), 3);
        let err = parse_narrative_structure(&raw).expect_err("三层信封应超过解包上限");
        let msg = err.to_string();
        assert!(msg.contains("信封"), "错误信息应指明信封问题，实际: {msg}");
        assert!(
            msg.contains(&MAX_ENVELOPE_UNWRAP_DEPTH.to_string()),
            "错误信息应带上限值，实际: {msg}"
        );
    }

    #[test]
    fn structure_unwrapped_from_agent_node_envelope() {
        // ⑤ AgentNode 形 `{role, model, content:"<JSON>", ...}`
        let content = serde_json::to_string(&sample_structure()).expect("序列化结构");
        let raw = json!({
            "role": "assistant",
            "model": "some-model",
            "content": content,
            "thinking": null,
            "usage": { "input_tokens": 1 },
            "node_id": "lc-conceive"
        });
        let parsed = parse_narrative_structure(&raw).expect("Agent 信封应被解包");
        assert_eq!(parsed.arcs.len(), 1);
    }

    #[test]
    fn structure_with_content_key_and_top_level_arcs_is_not_treated_as_envelope() {
        // ⑥ 顶层已有 arcs ⇒ 是结构本体，不能被误判成信封。
        // 把 content 设成非 JSON 文本：若被误判成信封，解析必然失败。
        let mut raw = sample_structure();
        raw["content"] = json!("<<< 不是 JSON，若被当信封解包则必然报错 >>>");
        let parsed = parse_narrative_structure(&raw).expect("含顶层 arcs 的对象应走直接解析分支");
        assert_eq!(parsed.arcs.len(), 1);
        assert_eq!(parsed.confluences.len(), 0);
    }
}
