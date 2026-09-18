// SPDX-License-Identifier: AGPL-3.0-only

//! 数据库连接配置命令（DB 外持久化）。
//!
//! DbConfig 定义在 `axagent_dao::config`（init 与 commands 共享，避免重复定义）。
//! 密码字段使用 master.key（Aes256Gcm）加密后落盘，明文不写入文件。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use sea_orm::{ConnectOptions, ConnectionTrait, Database};
use tauri::State;
use tauri::command;

use axagent_crypto::{decrypt_key, encrypt_key};

use axagent_dao::config::DbConfig;
use axagent_dao::migrations::{SchemaRepairReport, SchemaStatus};

use axagent_agent_macro::agent_command;

use crate::AppState;

fn db_config_path() -> PathBuf {
    crate::paths::axagent_home().join("db_config.json")
}

/// 读取 master.key（用于密码加解密），复用 init::database 的加载逻辑。
fn load_master_key() -> Result<[u8; 32], String> {
    let app_dir = crate::paths::axagent_home();
    let key_path = app_dir.join("master.key");
    crate::init::database::load_or_create_master_key(&key_path, &app_dir)
}

#[agent_command(domain = "devops", safety = Safe, call_mode = StateOnly, description = "获取数据库配置")]
#[command]
pub fn get_db_config() -> Result<DbConfig, String> {
    let path = db_config_path();
    if !path.exists() {
        return Ok(DbConfig::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    let mut cfg: DbConfig = serde_json::from_str(&content).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    // 解密密码供前端填充表单（pg_password_enc -> pg_password）
    if let Some(enc) = cfg.pg_password_enc.take() {
        if let Ok(key) = load_master_key() {
            if let Ok(plain) = decrypt_key(&enc, &key) {
                cfg.pg_password = Some(plain);
            }
        }
    }
    Ok(cfg)
}

/// 「该拿 `pg_password_enc` 怎么办」的纯决策（**不碰文件系统、不做加密**）。
///
/// 抽成纯函数的原因：`save_db_config` 自身依赖 `{axagent_home()}/db_config.json` 与
/// `master.key`，而 `axagent_home()` 读的是**进程级全局**环境变量（`USERPROFILE` / `HOME`），
/// 在单测里改它会与并行用例互相污染 —— 那条路径没法可靠隔离。于是把本轮唯一的判定逻辑
/// 挤进这个无副作用函数，由下方 `#[cfg(test)] mod tests` 穷举（见其 4 个用例）。
///
/// 判定**只看 `incoming`**（前端传来的 `pg_password`）；`prev_enc`（磁盘上的旧密文）
/// 不参与判定，只在 `Keep` 分支被带出去 —— 这样 `(incoming, prev_enc)` 在**签名**上
/// 就绑成一对，调用方没法「把 `prev_enc` 丢在一边、拿 `Keep` 当清空用」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordAction<'a> {
    /// 保留给定的已落盘密文（`None` = 盘上本来就没有密文 ⇒ 落盘仍为 `null`）
    Keep(Option<&'a str>),
    /// 清除已落盘密文（前端显式传空串）
    Clear,
    /// 加密这份明文后覆盖（前端填写 / 修改了密码）
    Replace(&'a str),
}

fn password_action<'a>(incoming: Option<&'a str>, prev_enc: Option<&'a str>) -> PasswordAction<'a> {
    match incoming {
        // 字段缺席 ⇒ 保留磁盘值（含「盘上也没有」的情形，结果同样是 None）
        None => PasswordAction::Keep(prev_enc),
        // 空串 ⇒ 清除
        Some("") => PasswordAction::Clear,
        // 非空 ⇒ 加密覆盖
        Some(pw) => PasswordAction::Replace(pw),
    }
}

/// 保存数据库配置到 `{axagent_home()}/db_config.json`（整体覆盖，不做字段级合并）。
///
/// ## `pg_password` 的三条缺席 / 空值语义（**契约，勿改**）
///
/// 前端 `DbConfigForm` **不认识** `pg_password_enc`（那是后端内部的密文槽位），
/// 它只会回传 `pg_password` 的明文。因此「字段是否出现」必须承担明确语义，
/// 否则一次普通保存就会把用户已存的 PG 密码静默删掉：
///
/// | `pg_password` 入参 | `pg_password_enc` 落盘结果 | 场景 |
/// |---|---|---|
/// | `None`（字段缺席） | **保留**磁盘上原值 | 前端不掌管密码（如 `get_db_config` 解密失败、返回的 `pg_password` 为 `None`） |
/// | `Some("")`（空串） | 清除（`None`） | 用户显式清空密码 |
/// | `Some(非空)` | 加密后覆盖 | 用户填写 / 修改密码 |
///
/// ⚠ 第一行是**修复点**：旧实现只在 `Some` 分支里改 `pg_password_enc`，
/// `None` 时保持的却是**前端传来的值** —— 而前端从不设置该字段 ⇒ 落盘即 `None`
/// ⇒ 已存密码被静默删除。现在 `None` 走「读旧文件兜底」，文件不存在 / 解析失败
/// 按 `None` 处理（首次保存场景）。
///
/// 明文密码永远不落盘（写出前 `pg_password` 已被 `take()` 置空）。
#[agent_command(domain = "devops", safety = Caution, call_mode = StateInput, description = "保存数据库配置")]
#[command]
pub fn save_db_config(config: DbConfig) -> Result<(), String> {
    let path = db_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    }
    let mut to_save = config;
    // 旧文件的密文作为「前端未传密码」时的兜底。读不到 / 解析不了都按 `None`
    // 处理 —— 前者是首次保存，后者是文件损坏，此时也确实没有可信的密文可保留。
    let prev_password_enc = fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<DbConfig>(&c).ok())
        .and_then(|c| c.pg_password_enc);
    // 先把明文取到局部变量，判定逻辑交给纯函数（`PasswordAction` 的借用期因此绑定在
    // 函数体内的两个局部量上，不依赖 match 临时量存活期这种易错的细节）。
    let incoming_password = to_save.pg_password.take();
    match password_action(incoming_password.as_deref(), prev_password_enc.as_deref()) {
        // 字段缺席 ⇒ 保留已落盘密文（防「未传 = 清空」的静默删除，见上方契约表）
        PasswordAction::Keep(kept) => to_save.pg_password_enc = kept.map(String::from),
        // 空密码表示清除
        PasswordAction::Clear => to_save.pg_password_enc = None,
        // 非空 ⇒ 加密覆盖
        PasswordAction::Replace(pw) => {
            let key = load_master_key()?;
            let enc = encrypt_key(pw, &key).map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
            to_save.pg_password_enc = Some(enc);
        },
    }
    // 明文密码不落盘
    let content = serde_json::to_string_pretty(&to_save).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    fs::write(&path, content).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 测试数据库连接是否可用（不持久化）。
///
/// 直接用传入的 DbConfig 构建连接 URL 并打开一个最小连接，执行 `SELECT 1`
/// 验证连通性与凭据正确性。PostgreSQL 走 `build_db_url` 的密码解密逻辑。
///
/// ## 返回类型与错误契约
///
/// 返回 `Result<(), String>` —— **成功路径不携带任何文案**。旧实现返回
/// `Ok("连接成功".to_string())` 这种写死简体中文的自由文本，它跨 IPC 边界直达
/// 前端 `message.success(result || t(...))`，后果有两个：10 个非中文语言的用户
/// 看到中文；且前端的 `settings.database.testSuccess` 键在桌面模式恒被它遮蔽
/// （`result` 恒为真值），成为一个永不生效的死 key。成功文案只应存在于 i18n。
///
/// 失败一律返回结构化的 `ErrorResponse`（`{code, category, detail}`，见
/// `crate::commands::error`）：`code` 供前端本地化，`detail` 放底层 sqlx 原文，
/// 供前端作为「技术详情」一并展示 —— 本地化不得以丢原因为代价。
/// 连接失败与验证查询失败是两个码（理由见 `commands::error_code::db` 的模块注释）。
#[agent_command(domain = "devops", safety = Safe, call_mode = StateInput, description = "测试数据库连接")]
#[command]
pub async fn test_db_connection(config: DbConfig) -> Result<(), String> {
    let app_dir = crate::paths::axagent_home();
    let master_key = load_master_key()?;
    let (url, _is_sqlite) = crate::init::database::build_db_url(&config, &app_dir, &master_key)?;

    let mut opt = ConnectOptions::new(&url);
    opt.max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .sqlx_logging(false);

    let conn = Database::connect(opt).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error_with_code(
            crate::commands::error_code::db::CONNECT_FAILED,
            e,
            crate::commands::error::ErrorCategory::Retryable,
        ))
    })?;
    conn.execute_unprepared("SELECT 1").await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error_with_code(
            crate::commands::error_code::db::QUERY_VERIFY_FAILED,
            e,
            crate::commands::error::ErrorCategory::Retryable,
        ))
    })?;
    Ok(())
}

/// 查询当前数据库结构状态（声明式引擎 reconcile 视角）。
///
/// 返回方言、期望/实况表数，以及引擎下次启动会补的变更数（`pending_apply`）、
/// 本方言无原生 DDL 的已知限制数（`pending_unsupported`）、需人工处理的条数
/// （`pending_manual`）、声明漂移数（`advisories`）与异常备注（`notes`）。
/// 供前端诊断「结构未收敛」类问题（如启动后补列未跑完导致表缺失）。
/// 注意 `applied_version` / `latest_version` 仅为版本化迁移时代的遗留记账。
#[agent_command(domain = "devops", safety = Safe, call_mode = StateOnly, description = "获取数据库结构状态")]
#[command]
pub async fn get_schema_status(state: State<'_, AppState>) -> Result<SchemaStatus, String> {
    let db = state.harness.db();
    axagent_dao::migrations::get_schema_status(db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 修复数据库结构：补全缺失列 + 修复已存在列的**类型错配**。
///
/// 底层是 `schema_diff::heal_all`，也是启动期唯一的列类型放宽通道
/// （`AlterColumnType` 不在引擎的纯新增白名单里，引擎永远不做类型修复）。
/// 只补列/改类型，不删列、不改语义，不影响运行中的会话数据。
#[agent_command(domain = "devops", safety = Caution, call_mode = StateOnly, description = "修复数据库结构：补缺失列 + 修列类型错配")]
#[command]
pub async fn repair_schema(state: State<'_, AppState>) -> Result<SchemaRepairReport, String> {
    let db = state.harness.db();
    axagent_dao::migrations::repair_schema(db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::{PasswordAction, password_action};

    /// **本轮修复的核心**：前端未传密码（`None`）+ 盘上已有密文 ⇒ **保留**，
    /// 而不是把 `pg_password_enc` 落成 `None`（旧实现即如此 ⇒ 静默删除已存密码）。
    ///
    /// 这个组合是**真实可达**的，不是构造出来的：`get_db_config` 在 `master.key`
    /// 缺失 / 损坏导致 `decrypt_key` 失败时，回给前端的 `pg_password` 就是 `None`
    /// （`db_config.rs` 的 `get_db_config`，`if let Ok(plain) = decrypt_key(...)` 无 else 分支），
    /// 用户此时点一次「保存」就会走这一支 —— 修前即在此丢掉密码。
    #[test]
    fn none_incoming_keeps_previous_ciphertext() {
        assert_eq!(password_action(None, Some("OLD")), PasswordAction::Keep(Some("OLD")));
    }

    /// 字段缺席但盘上也没有密文（首次保存）⇒ 仍是 `Keep(None)`：落盘结果同样是 `null`，
    /// 与旧实现**零回归**。这一支与上一支共用同一条判定（只看 `incoming`），
    /// 故「盘上有没有旧密文」不会让分支走偏。
    #[test]
    fn none_incoming_without_previous_keeps_nothing() {
        assert_eq!(password_action(None, None), PasswordAction::Keep(None));
    }

    /// 显式空串 ⇒ 清除（用户的「删除已保存密码」这一动作必须仍然有效）。
    #[test]
    fn empty_incoming_clears_previous_ciphertext() {
        assert_eq!(password_action(Some(""), Some("OLD")), PasswordAction::Clear);
        // 盘上本来就没有密文时，Clear 与 Keep(None) 的落盘结果相同（都是 null），
        // 但语义不同，所以仍应返回 Clear —— 保持判定只依赖 incoming。
        assert_eq!(password_action(Some(""), None), PasswordAction::Clear);
    }

    /// 非空 ⇒ 加密覆盖；`Replace` 携带**明文**，加密由调用方（需要 master.key）完成，
    /// 故本函数仍是纯函数。
    #[test]
    fn non_empty_incoming_replaces_previous_ciphertext() {
        assert_eq!(password_action(Some("new-pw"), Some("OLD")), PasswordAction::Replace("new-pw"));
        assert_eq!(password_action(Some("new-pw"), None), PasswordAction::Replace("new-pw"));
    }
}
