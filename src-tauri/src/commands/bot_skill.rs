// SPDX-License-Identifier: AGPL-3.0-only

//! P5 每 Bot 技能归属与预设导入 Tauri 命令（docs/bot-integration/P5/IPC-P5 §1.2、ADR-P5-5/6）。

use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_skill_binding as binding_repo;
use axagent_harness::types::{BotSkill, BotSkillPreset, BotSkillPresetResult};
use tauri::State;

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

fn map_err(err: axagent_harness::core_error::AxAgentError) -> String {
    match err {
        axagent_harness::core_error::AxAgentError::NotFound(detail) => {
            ErrorResponse::new(bot_err::NOT_FOUND)
                .with_category(ErrorCategory::Validation)
                .with_detail(detail)
                .to_string()
        }
        other => ErrorResponse::from_error(other, ErrorCategory::Unrecoverable).to_string(),
    }
}

async fn ensure_bot(db: &axagent_dao::DatabaseConnection, bot_id: &str) -> Result<(), String> {
    bot_repo::get_bot(db, bot_id).await.map(|_| ()).map_err(map_err)
}

#[tauri::command]
pub async fn list_bot_skills(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<Vec<BotSkill>, String> {
    let db = state.harness.db();
    ensure_bot(db, &bot_id).await?;
    crate::bot_skill_service::list_bot_skills(db, &bot_id).await
}

#[tauri::command]
pub async fn set_bot_skill_enabled(
    state: State<'_, AppState>,
    bot_id: String,
    skill_name: String,
    enabled: bool,
) -> Result<BotSkill, String> {
    let db = state.harness.db();
    ensure_bot(db, &bot_id).await?;
    let affected = binding_repo::set_enabled(db, &bot_id, &skill_name, enabled)
        .await
        .map_err(map_err)?;
    if !affected {
        return Err(ErrorResponse::new(bot_err::SKILL_NOT_FOUND)
            .with_category(ErrorCategory::Validation)
            .with_param("botId", &bot_id)
            .with_param("skillName", &skill_name)
            .with_detail("未找到该 Bot 的技能归属")
            .to_string());
    }
    let skills = crate::bot_skill_service::list_bot_skills(db, &bot_id).await?;
    skills
        .into_iter()
        .find(|s| s.name == skill_name)
        .ok_or_else(|| {
            ErrorResponse::new(bot_err::SKILL_NOT_FOUND)
                .with_category(ErrorCategory::Validation)
                .with_detail("技能已更新但未能读取视图")
                .to_string()
        })
}

#[tauri::command]
pub async fn import_bot_skill_preset(
    state: State<'_, AppState>,
    bot_id: String,
    preset: BotSkillPreset,
) -> Result<BotSkillPresetResult, String> {
    let db = state.harness.db();
    ensure_bot(db, &bot_id).await?;
    crate::bot_skill_service::import_preset(db, &bot_id, preset).await
}
