// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 部门 Tauri 命令（docs/bot-integration/P2/IPC-P2B §1/§2.1、ADR-P2B-6）。
//!
//! 本模块只做校验 + 编排：数据访问全部走 `axagent_dao::repo::{bot, bot_department}`，
//! 不直接触碰 SeaORM / entities（check:layers）。部门是常设组织，Bot 是可删个体：
//! 删 Bot 时清归属、Chief 置空，部门保留（ADR-P2B-6）。

use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_harness::core_error::AxAgentError;
use axagent_harness::types::{BotDepartment, CreateBotDepartment, UpdateBotDepartmentPatch};
use tauri::State;

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;

fn validation_error(code: &str, field: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(code)
        .with_category(ErrorCategory::Validation)
        .with_param("field", field)
        .with_detail(detail)
        .to_string()
}

fn map_dept_error(err: AxAgentError) -> String {
    match err {
        AxAgentError::NotFound(detail) => ErrorResponse::new(bot_err::DEPARTMENT_NOT_FOUND)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
            .to_string(),
        AxAgentError::Validation(detail) if detail.contains("belongs to a department") => {
            ErrorResponse::new(bot_err::DEPARTMENT_MEMBER_EXISTS)
                .with_category(ErrorCategory::Validation)
                .with_detail(detail)
                .to_string()
        },
        other => ErrorResponse::from_error(other, ErrorCategory::Unrecoverable).to_string(),
    }
}

fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    let len = trimmed.chars().count();
    if len == 0 || len > MAX_NAME_CHARS {
        return Err(validation_error(
            bot_err::NAME_INVALID,
            "name",
            format!("department name must be 1..{MAX_NAME_CHARS} characters"),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_description(description: Option<&str>) -> Result<String, String> {
    let trimmed = description.unwrap_or("").trim().to_string();
    if trimmed.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(validation_error(
            bot_err::FIELD_TOO_LONG,
            "description",
            format!("description must be at most {MAX_DESCRIPTION_CHARS} characters"),
        ));
    }
    Ok(trimmed)
}

/// 校验 Chief：存在且 enabled。None = 摘除 Chief（合法）。
async fn validate_chief(
    db: &axagent_dao::DatabaseConnection,
    chief: Option<&str>,
) -> Result<(), String> {
    if let Some(id) = chief {
        let bot = bot_repo::get_bot(db, id)
            .await
            .map_err(|_| validation_error(bot_err::NOT_FOUND, "chiefBotId", "Chief Bot 不存在"))?;
        if !bot.is_enabled {
            return Err(validation_error(bot_err::NOT_FOUND, "chiefBotId", "Chief Bot 已禁用"));
        }
    }
    Ok(())
}

/// 校验成员 Bot：存在且 enabled。
async fn validate_member(db: &axagent_dao::DatabaseConnection, bot_id: &str) -> Result<(), String> {
    let bot = bot_repo::get_bot(db, bot_id)
        .await
        .map_err(|_| validation_error(bot_err::NOT_FOUND, "botId", "Bot 不存在"))?;
    if !bot.is_enabled {
        return Err(validation_error(bot_err::NOT_FOUND, "botId", "Bot 已禁用"));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_bot_departments(
    state: State<'_, AppState>,
) -> Result<Vec<BotDepartment>, String> {
    dept_repo::list_departments(state.harness.db()).await.map_err(map_dept_error)
}

#[tauri::command]
pub async fn create_bot_department(
    state: State<'_, AppState>,
    input: CreateBotDepartment,
) -> Result<BotDepartment, String> {
    let db = state.harness.db();
    let name = validate_name(&input.name)?;
    let description = validate_description(input.description.as_deref())?;
    validate_chief(db, input.chief_bot_id.as_deref()).await?;
    dept_repo::create_department(db, &name, &description, input.chief_bot_id.as_deref())
        .await
        .map_err(map_dept_error)
}

#[tauri::command]
pub async fn update_bot_department(
    state: State<'_, AppState>,
    department_id: String,
    input: UpdateBotDepartmentPatch,
) -> Result<BotDepartment, String> {
    let db = state.harness.db();
    let current = dept_repo::get_department(db, &department_id).await.map_err(map_dept_error)?;

    let mut patch = input;
    if let Some(name) = &patch.name {
        patch.name = Some(validate_name(name)?);
    }
    if let Some(description) = &patch.description {
        patch.description = Some(Some(validate_description(description.as_deref())?));
    }
    // Chief 置空合法；给出 id 则校验存在且 enabled。
    let chief_id = patch.chief_bot_id.as_ref().and_then(|v| v.as_deref());
    validate_chief(db, chief_id).await?;
    let _ = &current; // 存在性已由 get_department 校验

    dept_repo::update_department(db, &department_id, &patch).await.map_err(map_dept_error)
}

/// 删除部门：清成员行（Bot 本体不动）+ 删配额行 + 删部门记录（ADR-P2B-6 Q4、ADR-P4-1）。
#[tauri::command]
pub async fn delete_bot_department(
    state: State<'_, AppState>,
    department_id: String,
) -> Result<u64, String> {
    let db = state.harness.db();
    // 先确认部门存在，避免「删了个空」返回成功。
    dept_repo::get_department(db, &department_id).await.map_err(map_dept_error)?;
    // ADR-P3-3 + ADR-P4-1：编排转实例归属、清成员行、删配额行、删部门行
    // 全部在同一个事务内完成（任一步失败整事务回滚）。
    dept_repo::delete_department_cascade(db, &department_id).await.map_err(map_dept_error)
}

#[tauri::command]
pub async fn add_bot_to_department(
    state: State<'_, AppState>,
    department_id: String,
    bot_id: String,
) -> Result<BotDepartment, String> {
    let db = state.harness.db();
    // Q2：一个 Bot 只属一个部门 —— 复合 UNIQUE 只挡同部门重复行，
    // 跨部门约束在命令层显式查（IPC-P2B §1 / MODEL-P2B §2.2）。
    if let Some(existing) =
        dept_repo::department_of_bot(db, &bot_id).await.map_err(map_dept_error)?
    {
        if existing != department_id {
            return Err(validation_error(
                bot_err::DEPARTMENT_MEMBER_EXISTS,
                "botId",
                format!("Bot {bot_id} 已归属部门 {existing}"),
            ));
        }
        return Err(validation_error(
            bot_err::DEPARTMENT_MEMBER_EXISTS,
            "botId",
            format!("Bot {bot_id} 已在部门 {department_id}"),
        ));
    }
    validate_member(db, &bot_id).await?;
    // 部门存在性由 add 时的 FK 语义兜底：先查一次给前端明确错误码。
    dept_repo::get_department(db, &department_id).await.map_err(map_dept_error)?;
    dept_repo::add_bot_to_department(db, &department_id, &bot_id).await.map_err(map_dept_error)?;
    dept_repo::get_department(db, &department_id).await.map_err(map_dept_error)
}

#[tauri::command]
pub async fn remove_bot_from_department(
    state: State<'_, AppState>,
    department_id: String,
    bot_id: String,
) -> Result<BotDepartment, String> {
    let db = state.harness.db();
    dept_repo::remove_bot_from_department(db, &department_id, &bot_id)
        .await
        .map_err(map_dept_error)?;
    dept_repo::get_department(db, &department_id).await.map_err(map_dept_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_description_are_bounded() {
        assert_eq!(validate_name("  投研一部  ").expect("合法"), "投研一部");
        assert!(validate_name("  ").is_err());
        assert!(validate_name(&"名".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert_eq!(validate_description(Some("  研究  ")).expect("合法"), "研究");
        assert!(validate_description(Some(&"字".repeat(MAX_DESCRIPTION_CHARS + 1))).is_err());
    }

    /// 部门 → 清成员行 → 删部门 → Bot 本体仍在（ADR-P2B-6 Q4）。
    #[tokio::test]
    async fn department_delete_keeps_member_bots() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库");
        let db = &h.conn;
        let bot = bot_repo::create_bot(
            db,
            &axagent_harness::types::CreateBot { name: "成员 Bot".into(), ..Default::default() },
        )
        .await
        .expect("建 Bot");
        let chief = bot_repo::create_bot(
            db,
            &axagent_harness::types::CreateBot { name: "Chief Bot".into(), ..Default::default() },
        )
        .await
        .expect("建 Chief");
        let dept =
            dept_repo::create_department(db, "投研部", "", Some(&chief.id)).await.expect("建部门");
        dept_repo::add_bot_to_department(db, &dept.id, &bot.id).await.expect("加入部门");

        let members = dept_repo::delete_department_members(db, &dept.id).await.expect("清成员");
        assert_eq!(members, 1);
        dept_repo::delete_department(db, &dept.id).await.expect("删部门");

        assert!(dept_repo::get_department(db, &dept.id).await.is_err(), "部门应已删");
        assert_eq!(dept_repo::department_of_bot(db, &bot.id).await.expect("查归属"), None);
        assert!(bot_repo::get_bot(db, &bot.id).await.is_ok(), "成员 Bot 本体应保留");
    }

    /// 删 Chief Bot：部门保留、chief 置空；成员行清掉。
    #[tokio::test]
    async fn chief_removal_keeps_department() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库");
        let db = &h.conn;
        let chief = bot_repo::create_bot(
            db,
            &axagent_harness::types::CreateBot { name: "Chief".into(), ..Default::default() },
        )
        .await
        .expect("建 Bot");
        let dept =
            dept_repo::create_department(db, "部门", "", Some(&chief.id)).await.expect("建部门");
        dept_repo::add_bot_to_department(db, &dept.id, &chief.id).await.expect("加入");

        let cleared = dept_repo::clear_chief_for_bot(db, &chief.id).await.expect("清 Chief");
        assert_eq!(cleared, 1);
        let removed = dept_repo::delete_memberships_of_bot(db, &chief.id).await.expect("清归属");
        assert_eq!(removed, 1);
        bot_repo::delete_bot(db, &chief.id).await.expect("删 Bot");

        let kept = dept_repo::get_department(db, &dept.id).await.expect("部门仍在");
        assert_eq!(kept.chief_bot_id, None, "Chief 置空");
        assert!(kept.bot_ids.is_empty(), "成员归属已清");
    }

    /// 重复 add 的错误码映射：repo 冲突 → BOT_DEPARTMENT_MEMBER_EXISTS。
    #[test]
    fn duplicate_error_maps_to_member_exists() {
        let err =
            map_dept_error(AxAgentError::Validation("bot already belongs to a department".into()));
        assert!(err.contains(bot_err::DEPARTMENT_MEMBER_EXISTS), "{err}");
    }
}
