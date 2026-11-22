// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 归属层 Tauri 命令（docs/bot-integration/IPC.md §2 / §5）。
//!
//! 本模块只做 wiring 与输入校验：数据访问走 `axagent_dao::repo::bot`，文件访问走
//! `axagent_agent::bot_workspace`，不直接触碰 SeaORM / entities（`check:layers`）。
//!
//! SOUL 保存顺序固定为「镜像先落盘 → 再写 DB」：磁盘失败时不留下「DB 说存了、
//! 文件没变」的分叉；MEMORY 只读（P1 不提供写入命令）。

use std::path::Path;

use axagent_agent::bot_workspace::BotWorkspace;
use axagent_dao::db::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_harness::constants::memory as mem_const;
use axagent_harness::core_error::AxAgentError;
use axagent_harness::types::Conversation;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use unicode_segmentation::UnicodeSegmentation;

use crate::AppState;
use crate::commands::error::{CommandError, ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

/// Bot 输出/输入 DTO 的唯一权威定义在 harness，此处仅做命令层别名（禁区 12）。
pub use axagent_harness::types::{
    Bot, CreateBot as CreateBotInput, UpdateBotPatch as UpdateBotInput,
};

const MAX_NAME_CHARS: usize = 64;
const MAX_TITLE_CHARS: usize = 120;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_SOUL_BYTES: usize = 64 * 1024;
const MAX_ICON_BYTES: usize = 8;
const APPROVAL_MODES: &[&str] = &["ask", "edits", "auto", "full", "custom"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotSoul {
    pub content: String,
    pub hash: String,
    pub drift: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBudget {
    pub max_lines: i64,
    pub max_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryTopic {
    pub relative_path: String,
    pub category: String,
    pub title: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotMemory {
    pub index_content: String,
    pub line_count: i64,
    pub byte_count: i64,
    pub budget: MemoryBudget,
    pub topics: Vec<MemoryTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBotResult {
    pub detached_threads: i64,
    pub memory_path: String,
}

// ───────────────────────────── 校验 ─────────────────────────────

fn validation_error(code: &str, field: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(code)
        .with_category(ErrorCategory::Validation)
        .with_param("field", field)
        .with_detail(detail)
        .to_string()
}

fn normalized_opt(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn normalized_patch(value: Option<Option<String>>) -> Option<Option<String>> {
    value.map(normalized_opt)
}

fn effective(patch: &Option<Option<String>>, current: &Option<String>) -> Option<String> {
    match patch {
        Some(v) => v.clone(),
        None => current.clone(),
    }
}

fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    let len = trimmed.chars().count();
    if len == 0 {
        return Err(validation_error(bot_err::NAME_INVALID, "name", "name must not be blank"));
    }
    if len > MAX_NAME_CHARS {
        return Err(validation_error(
            bot_err::NAME_INVALID,
            "name",
            format!("name must be at most {MAX_NAME_CHARS} characters (got {len})"),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_title(title: Option<&str>) -> Result<(), String> {
    if let Some(t) = title {
        let len = t.chars().count();
        if len > MAX_TITLE_CHARS {
            return Err(validation_error(
                bot_err::FIELD_TOO_LONG,
                "title",
                format!("title must be at most {MAX_TITLE_CHARS} characters (got {len})"),
            ));
        }
    }
    Ok(())
}

fn validate_description(description: Option<&str>) -> Result<(), String> {
    if let Some(d) = description {
        let len = d.chars().count();
        if len > MAX_DESCRIPTION_CHARS {
            return Err(validation_error(
                bot_err::FIELD_TOO_LONG,
                "description",
                format!(
                    "description must be at most {MAX_DESCRIPTION_CHARS} characters (got {len})"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_soul(soul: &str) -> Result<(), String> {
    if soul.len() > MAX_SOUL_BYTES {
        return Err(validation_error(
            bot_err::SOUL_TOO_LARGE,
            "soul",
            format!("soul must be at most {MAX_SOUL_BYTES} bytes (got {})", soul.len()),
        ));
    }
    Ok(())
}

fn validate_approval_mode(mode: &str) -> Result<(), String> {
    if APPROVAL_MODES.contains(&mode) {
        return Ok(());
    }
    Err(validation_error(
        bot_err::APPROVAL_MODE_INVALID,
        "approvalMode",
        format!("approvalMode must be one of {APPROVAL_MODES:?} (got {mode:?})"),
    ))
}

fn validate_model_pair(model_id: Option<&str>, provider_id: Option<&str>) -> Result<(), String> {
    if model_id.is_some() == provider_id.is_some() {
        return Ok(());
    }
    Err(validation_error(
        bot_err::MODEL_PAIR_INVALID,
        "modelId",
        "modelId and providerId must be provided together or omitted together",
    ))
}

fn validate_workdir(workdir: Option<&str>) -> Result<(), String> {
    let Some(dir) = workdir else { return Ok(()) };
    let path = Path::new(dir);
    if !path.is_absolute() || !path.is_dir() {
        return Err(validation_error(
            bot_err::WORKDIR_INVALID,
            "workdir",
            format!("workdir must be an existing absolute directory (got {dir:?})"),
        ));
    }
    Ok(())
}

fn validate_icon(icon: &str) -> Result<(), String> {
    let graphemes = icon.graphemes(true).count();
    if graphemes != 1 || icon.len() > MAX_ICON_BYTES {
        return Err(validation_error(
            bot_err::ICON_INVALID,
            "icon",
            format!(
                "icon must be exactly one grapheme of at most {MAX_ICON_BYTES} bytes \
                 (got {graphemes} grapheme(s), {} bytes)",
                icon.len()
            ),
        ));
    }
    Ok(())
}

fn validate_create(mut input: CreateBotInput) -> Result<CreateBotInput, String> {
    input.name = validate_name(&input.name)?;
    input.title = normalized_opt(input.title);
    input.description = normalized_opt(input.description);
    input.color = normalized_opt(input.color);
    input.section = normalized_opt(input.section);
    input.model_id = normalized_opt(input.model_id);
    input.provider_id = normalized_opt(input.provider_id);
    input.effort = normalized_opt(input.effort);
    input.workdir = normalized_opt(input.workdir);
    input.agent_profile_id = normalized_opt(input.agent_profile_id);
    input.icon = normalized_opt(input.icon);

    validate_title(input.title.as_deref())?;
    validate_description(input.description.as_deref())?;
    validate_soul(input.soul.as_deref().unwrap_or(""))?;
    validate_approval_mode(input.approval_mode.as_deref().unwrap_or("full"))?;
    validate_model_pair(input.model_id.as_deref(), input.provider_id.as_deref())?;
    validate_workdir(input.workdir.as_deref())?;
    if let Some(icon) = input.icon.as_deref() {
        validate_icon(icon)?;
    }
    Ok(input)
}

/// 校验合并后的状态，并把 `soul` 从补丁中取出（由调用方按「镜像先写」的顺序落盘）。
fn validate_update(
    existing: &Bot,
    mut patch: UpdateBotInput,
) -> Result<(UpdateBotInput, Option<String>), String> {
    patch.title = normalized_patch(patch.title);
    patch.description = normalized_patch(patch.description);
    patch.color = normalized_patch(patch.color);
    patch.model_id = normalized_patch(patch.model_id);
    patch.provider_id = normalized_patch(patch.provider_id);
    patch.effort = normalized_patch(patch.effort);
    patch.workdir = normalized_patch(patch.workdir);
    patch.agent_profile_id = normalized_patch(patch.agent_profile_id);
    patch.section = normalized_opt(patch.section);
    patch.icon = normalized_opt(patch.icon);

    let name = match patch.name.as_deref() {
        Some(name) => validate_name(name)?,
        None => existing.name.clone(),
    };
    patch.name = Some(name);

    let title = effective(&patch.title, &existing.title);
    let description = effective(&patch.description, &existing.description);
    let model_id = effective(&patch.model_id, &existing.model_id);
    let provider_id = effective(&patch.provider_id, &existing.provider_id);
    let workdir = effective(&patch.workdir, &existing.workdir);
    let soul = patch.soul.as_deref().unwrap_or(&existing.soul);

    validate_title(title.as_deref())?;
    validate_description(description.as_deref())?;
    validate_soul(soul)?;
    validate_approval_mode(patch.approval_mode.as_deref().unwrap_or(&existing.approval_mode))?;
    validate_model_pair(model_id.as_deref(), provider_id.as_deref())?;
    validate_workdir(workdir.as_deref())?;
    if let Some(icon) = patch.icon.as_deref() {
        validate_icon(icon)?;
    }

    let soul_update = patch.soul.take();
    Ok((patch, soul_update))
}

// ───────────────────────────── 错误映射 ─────────────────────────────

fn map_bot_error(err: AxAgentError) -> String {
    match err {
        AxAgentError::NotFound(detail) => ErrorResponse::new(bot_err::NOT_FOUND)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
            .to_string(),
        other => CommandError::from_error(other, ErrorCategory::Unrecoverable).to_string(),
    }
}

fn soul_write_failed(detail: impl Into<String>) -> String {
    ErrorResponse::new(bot_err::SOUL_WRITE_FAILED)
        .with_category(ErrorCategory::Unrecoverable)
        .with_detail(detail.into())
        .to_string()
}

fn memory_read_failed(detail: impl Into<String>) -> String {
    ErrorResponse::new(bot_err::MEMORY_READ_FAILED)
        .with_category(ErrorCategory::Unrecoverable)
        .with_detail(detail.into())
        .to_string()
}

// ───────────────────────────── 内部实现 ─────────────────────────────

/// 镜像原子写盘 → 回写 DB body/hash。失败一律带 `BOT_SOUL_WRITE_FAILED`。
async fn persist_soul(
    db: &DatabaseConnection,
    workspace: &BotWorkspace,
    bot_id: &str,
    content: &str,
) -> Result<BotSoul, String> {
    workspace.write_soul_atomic(content).map_err(soul_write_failed)?;
    let hash = BotWorkspace::soul_sha256(content);
    bot_repo::set_soul(db, bot_id, content, &hash).await.map_err(map_bot_error)?;
    Ok(BotSoul { content: content.to_string(), hash, drift: false })
}

fn soul_snapshot(bot: &Bot, content: String) -> BotSoul {
    let hash = BotWorkspace::soul_sha256(&content);
    // DB hash 为空只可能来自手改数据：退回「DB 正文的 hash」，避免把空 hash 当成漂移。
    let expected = if bot.soul_hash.is_empty() {
        BotWorkspace::soul_sha256(&bot.soul)
    } else {
        bot.soul_hash.clone()
    };
    BotSoul { content, hash: hash.clone(), drift: hash != expected }
}

fn memory_payload(workspace: &BotWorkspace) -> Result<BotMemory, String> {
    let content = workspace.read_memory_index().map_err(memory_read_failed)?;
    let topics = workspace
        .scan_topics()
        .into_iter()
        .map(|t| MemoryTopic {
            relative_path: t.relative_path,
            category: t.category,
            title: t.title,
            updated_at: t.updated_at,
        })
        .collect();
    Ok(BotMemory {
        line_count: content.lines().count() as i64,
        byte_count: content.len() as i64,
        index_content: content,
        budget: MemoryBudget {
            max_lines: mem_const::MEMORY_INDEX_MAX_LINES as i64,
            max_bytes: mem_const::MEMORY_INDEX_MAX_BYTES as i64,
        },
        topics,
    })
}

// ───────────────────────────── 命令 ─────────────────────────────

#[tauri::command]
pub async fn list_bots(
    state: State<'_, AppState>,
    include_disabled: Option<bool>,
) -> Result<Vec<Bot>, String> {
    bot_repo::list_bots(state.harness.db(), include_disabled.unwrap_or(false))
        .await
        .map_err(map_bot_error)
}

#[tauri::command]
pub async fn get_bot(state: State<'_, AppState>, bot_id: String) -> Result<Bot, String> {
    bot_repo::get_bot(state.harness.db(), &bot_id).await.map_err(map_bot_error)
}

#[tauri::command]
pub async fn create_bot(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CreateBotInput,
) -> Result<Bot, String> {
    // P1 不发事件；保留 app 参数供 P2 的 `bot-updated` 使用（IPC.md §6）。
    let _ = &app;
    let db = state.harness.db();
    let input = validate_create(input)?;
    let bot = bot_repo::create_bot(db, &input).await.map_err(map_bot_error)?;

    let workspace = BotWorkspace::for_bot(&bot.id);
    if let Err(err) = persist_soul(db, &workspace, &bot.id, &bot.soul).await {
        // 镜像失败 ⇒ 回滚刚建的记录（顺序相反地删掉，避免留下无 SOUL 的僵尸 Bot）。
        if let Err(cleanup) = bot_repo::delete_bot(db, &bot.id).await {
            tracing::error!(bot_id = %bot.id, error = %cleanup, "Bot 创建失败后回滚记录也失败");
        }
        return Err(err);
    }

    bot_repo::get_bot(db, &bot.id).await.map_err(map_bot_error)
}

#[tauri::command]
pub async fn update_bot(
    app: AppHandle,
    state: State<'_, AppState>,
    bot_id: String,
    input: UpdateBotInput,
) -> Result<Bot, String> {
    let _ = &app;
    let db = state.harness.db();
    let existing = bot_repo::get_bot(db, &bot_id).await.map_err(map_bot_error)?;
    let (patch, soul_update) = validate_update(&existing, input)?;

    let workspace = BotWorkspace::for_bot(&bot_id);
    if let Some(soul) = soul_update.as_deref() {
        // 先落盘：磁盘失败时 DB 保持旧值，不会出现「保存成功但文件没换」。
        workspace.write_soul_atomic(soul).map_err(soul_write_failed)?;
    }

    bot_repo::update_bot(db, &bot_id, &patch).await.map_err(map_bot_error)?;
    if let Some(soul) = soul_update {
        bot_repo::set_soul(db, &bot_id, &soul, &BotWorkspace::soul_sha256(&soul))
            .await
            .map_err(map_bot_error)?;
    }
    bot_repo::get_bot(db, &bot_id).await.map_err(map_bot_error)
}

#[tauri::command]
pub async fn delete_bot(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<DeleteBotResult, String> {
    let db = state.harness.db();
    let workspace = BotWorkspace::for_bot(&bot_id);
    let memory_path = workspace.root().display().to_string();
    let detached = bot_repo::delete_bot(db, &bot_id).await.map_err(|err| match err {
        AxAgentError::NotFound(detail) => ErrorResponse::new(bot_err::NOT_FOUND)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
            .to_string(),
        other => ErrorResponse::new(bot_err::DELETE_FAILED)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(other.to_string())
            .to_string(),
    })?;
    Ok(DeleteBotResult { detached_threads: detached as i64, memory_path })
}

#[tauri::command]
pub async fn list_bot_threads(
    state: State<'_, AppState>,
    bot_id: String,
    include_archived: Option<bool>,
) -> Result<Vec<Conversation>, String> {
    let db = state.harness.db();
    // 先确认 Bot 存在：否则「删掉的 Bot」和「没有线程的 Bot」返回同一个空数组。
    bot_repo::get_bot(db, &bot_id).await.map_err(map_bot_error)?;
    bot_repo::list_bot_threads(db, &bot_id, include_archived.unwrap_or(false))
        .await
        .map_err(map_bot_error)
}

#[tauri::command]
pub async fn read_bot_soul(state: State<'_, AppState>, bot_id: String) -> Result<BotSoul, String> {
    let db = state.harness.db();
    let bot = bot_repo::get_bot(db, &bot_id).await.map_err(map_bot_error)?;
    let workspace = BotWorkspace::for_bot(&bot_id);
    let content = workspace
        .read_soul()
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).to_string())?;
    let snapshot = soul_snapshot(&bot, content);
    if snapshot.drift != bot.soul_drift {
        // 读路径顺手修正标记；失败不影响本次返回（下次读取会重算）。
        if let Err(err) = bot_repo::set_soul_drift(db, &bot_id, snapshot.drift).await {
            tracing::warn!(bot_id = %bot_id, error = %err, "修正 SOUL 漂移标记失败，本次仍按实时比对结果返回");
        }
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn write_bot_soul(
    app: AppHandle,
    state: State<'_, AppState>,
    bot_id: String,
    content: String,
) -> Result<BotSoul, String> {
    let _ = &app;
    let db = state.harness.db();
    // 先确认存在：不存在的 id 不该在 home 下建出一个孤儿目录。
    bot_repo::get_bot(db, &bot_id).await.map_err(map_bot_error)?;
    validate_soul(&content)?;
    let workspace = BotWorkspace::for_bot(&bot_id);
    persist_soul(db, &workspace, &bot_id, &content).await
}

#[tauri::command]
pub async fn read_bot_memory(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<BotMemory, String> {
    let db = state.harness.db();
    bot_repo::get_bot(db, &bot_id).await.map_err(map_bot_error)?;
    memory_payload(&BotWorkspace::for_bot(&bot_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::types::CreateBot;

    fn bot_with(model: Option<&str>, provider: Option<&str>) -> Bot {
        Bot {
            id: "bot-1".to_string(),
            name: "测试 Bot".to_string(),
            title: None,
            description: None,
            soul: String::new(),
            soul_hash: BotWorkspace::soul_sha256(""),
            soul_drift: false,
            icon: "🤖".to_string(),
            color: None,
            section: "general".to_string(),
            model_id: model.map(str::to_string),
            provider_id: provider.map(str::to_string),
            effort: None,
            workdir: None,
            approval_mode: "full".to_string(),
            agent_profile_id: None,
            is_enabled: true,
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn name_is_trimmed_and_bounded() {
        assert_eq!(validate_name("  投研助手  ").expect("合法名称"), "投研助手");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"名".repeat(MAX_NAME_CHARS)).is_ok());
        assert!(validate_name(&"名".repeat(MAX_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn approval_mode_is_enum_checked() {
        for mode in APPROVAL_MODES {
            assert!(validate_approval_mode(mode).is_ok(), "{mode} 应合法");
        }
        let err = validate_approval_mode("yolo").expect_err("未知档位应报错");
        assert!(err.contains(bot_err::APPROVAL_MODE_INVALID), "{err}");
    }

    #[test]
    fn model_and_provider_must_pair() {
        assert!(validate_model_pair(None, None).is_ok());
        assert!(validate_model_pair(Some("m"), Some("p")).is_ok());
        assert!(validate_model_pair(Some("m"), None).is_err());
        assert!(validate_model_pair(None, Some("p")).is_err());
    }

    #[test]
    fn icon_requires_single_grapheme_within_byte_budget() {
        assert!(validate_icon("🤖").is_ok());
        assert!(validate_icon("🇨🇳").is_ok());
        assert!(validate_icon("ab").is_err(), "两个字符不是单个 emoji");
        assert!(validate_icon("🤖🤖").is_err());
        assert!(validate_icon("").is_err());
    }

    #[test]
    fn workdir_must_be_existing_absolute_dir() {
        let dir = tempfile::tempdir().expect("临时目录");
        assert!(validate_workdir(dir.path().to_str()).is_ok());
        assert!(validate_workdir(Some("relative/path")).is_err());
        assert!(validate_workdir(Some("/definitely/not/here/axagent")).is_err());
        assert!(validate_workdir(None).is_ok());
    }

    #[test]
    fn update_clears_with_some_none_and_keeps_untouched_fields() {
        let existing = bot_with(None, None);
        let patch =
            UpdateBotInput { title: Some(Some("新标题".to_string())), ..Default::default() };
        let (patch, soul) = validate_update(&existing, patch).expect("应通过校验");
        assert_eq!(patch.title, Some(Some("新标题".to_string())));
        assert_eq!(patch.name.as_deref(), Some("测试 Bot"), "未给 name 时沿用旧值");
        assert_eq!(soul, None);

        let (patch, _) =
            validate_update(&existing, UpdateBotInput { title: Some(None), ..Default::default() })
                .expect("置空应通过校验");
        assert_eq!(patch.title, Some(None));
    }

    #[test]
    fn update_rejects_half_model_pair_even_if_only_one_side_changes() {
        let existing = bot_with(Some("m"), Some("p"));
        let err = validate_update(
            &existing,
            UpdateBotInput { model_id: Some(None), ..Default::default() },
        )
        .expect_err("只清 model 会拆开配对");
        assert!(err.contains(bot_err::MODEL_PAIR_INVALID), "{err}");

        assert!(
            validate_update(
                &existing,
                UpdateBotInput {
                    model_id: Some(None),
                    provider_id: Some(None),
                    ..Default::default()
                },
            )
            .is_ok(),
            "两边一起清空应合法"
        );
    }

    #[test]
    fn soul_budget_is_utf8_bytes() {
        // 中文按 UTF-8 3 字节计：字符数未超限、字节数超限也必须拒绝。
        assert!(validate_soul(&"记".repeat(MAX_SOUL_BYTES / 3 + 1)).is_err());
        assert!(validate_soul(&"a".repeat(MAX_SOUL_BYTES)).is_ok());
        assert!(validate_soul(&"a".repeat(MAX_SOUL_BYTES + 1)).is_err());
    }

    #[tokio::test]
    async fn persist_soul_writes_mirror_then_db_hash() {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;
        let bot = bot_repo::create_bot(
            db,
            &CreateBot { name: "持久化 Bot".to_string(), ..Default::default() },
        )
        .await
        .expect("创建 Bot 应成功");

        let dir = tempfile::tempdir().expect("临时目录");
        let workspace = BotWorkspace::new(dir.path().join("bot"));
        let snapshot =
            persist_soul(db, &workspace, &bot.id, "# 投研助手").await.expect("写 SOUL 应成功");

        assert_eq!(snapshot.hash, BotWorkspace::soul_sha256("# 投研助手"));
        assert!(!snapshot.drift);
        assert_eq!(workspace.read_soul().expect("读镜像"), "# 投研助手");

        let stored = bot_repo::get_bot(db, &bot.id).await.expect("读回 Bot");
        assert_eq!(stored.soul, "# 投研助手");
        assert_eq!(stored.soul_hash, snapshot.hash);
        assert!(!stored.soul_drift);
    }

    #[test]
    fn soul_snapshot_detects_external_edit() {
        let mut bot = bot_with(None, None);
        bot.soul = "A".to_string();
        bot.soul_hash = BotWorkspace::soul_sha256("A");
        assert!(!soul_snapshot(&bot, "A".to_string()).drift);
        assert!(soul_snapshot(&bot, "B".to_string()).drift);
    }

    #[test]
    fn memory_payload_reports_counts_and_budget() {
        let dir = tempfile::tempdir().expect("临时目录");
        let workspace = BotWorkspace::new(dir.path());
        workspace.ensure().expect("建目录");
        std::fs::write(workspace.memory_path(), "# 索引\n- 行二\n").expect("写 MEMORY.md");

        let payload = memory_payload(&workspace).expect("读取记忆");
        assert_eq!(payload.line_count, 2);
        assert_eq!(payload.byte_count, "# 索引\n- 行二\n".len() as i64);
        assert_eq!(payload.budget.max_lines, mem_const::MEMORY_INDEX_MAX_LINES as i64);
        assert_eq!(payload.budget.max_bytes, mem_const::MEMORY_INDEX_MAX_BYTES as i64);
        assert!(payload.topics.is_empty());
    }
}
