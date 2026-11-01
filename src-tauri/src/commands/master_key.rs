// SPDX-License-Identifier: AGPL-3.0-only

//! 主密钥（master.key）导出 / 导入命令。
//!
//! 背景：master.key 是每机本地的根加密钥。多机共享同一数据库（尤其 Postgres）时，
//! 落库密文由「写入方」的钥匙加密，消费方读不出来 —— v2 密文格式把加密方指纹写进
//! 密文头并报 `KEY_FROM_OTHER_DEVICE`（见 `axagent_crypto::CIPHERTEXT_V2_PREFIX`）。
//! 该错误的用户侧出路就是本模块：从加密方设备导出 master.key（hex），到消费方设备导入。
//!
//! 三条安全契约：
//!
//! 1. **绝不静默生成** —— 导入 / 导出只读已存在的 master.key（缺失报 `MASTER_KEY_NOT_FOUND`）。
//!    与 `init::database::load_or_create_master_key` 的关键差异：文件缺失时生成新钥是启动
//!    场景的正确行为，对导入却是「先落一把陌生钥匙、再覆盖掉正要导入的钥匙」的坑。
//! 2. **先验证后落盘** —— `db_config.json` 的 `pg_password_enc` 必须能用旧钥解开才允许换钥
//!    （解不开 ⇒ 无法重加密，覆盖钥匙 = 把这份密码永久丢死）；预检失败时**不修改任何文件**。
//! 3. **旧物必备份** —— 换钥前先备份 `master.key.bak-<ts>`；重加密 `db_config.json` 前先备份
//!    `db_config.json.bak-<ts>`；两者共享同一时间戳后缀，可成对找回。
//!
//! ⚠ 运行中的应用把钥匙载在内存里（`RuntimeHarness.master_key`，启动时载入一次）：
//! 导入成功后必须**重启应用**才生效；重启前不要保存新的密钥类数据 —— 那些密文会以
//! 内存中的旧钥匙加密落库，重启后反而读不出来（v2 指纹归因会如实报 `KEY_FROM_OTHER_DEVICE`）。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::command;

use axagent_agent_macro::agent_command;
use axagent_crypto::{decrypt_key, encrypt_key, master_key_fingerprint, master_key_from_hex};
use axagent_dao::config::DbConfig;

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::{common, master_key as master_key_err};

/// 当前主密钥的概要（只有指纹，不含钥匙本体）。前端用它展示「本机当前钥匙是谁」，
/// 与 v2 密文头携带的加密方指纹对得上才能解。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterKeyInfo {
    /// 本机主密钥指纹（sha256 前 4 字节 hex，8 字符，机器无关）
    pub fingerprint: String,
}

/// 主密钥导出结果 —— `key_hex` 就是钥匙本体（64 位 hex）， Dangerous 级别的返回值。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterKeyExport {
    pub fingerprint: String,
    pub key_hex: String,
}

/// 主密钥导入结果报告。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterKeyImportReport {
    /// 导入前本机旧钥匙的指纹；本机原本没有 master.key 时为 `None`
    pub old_fingerprint: Option<String>,
    /// 导入后的新钥匙指纹
    pub new_fingerprint: String,
    /// 本次导入产生的备份文件路径（优先指 master.key 的备份；本机原本无钥匙时
    /// 指 `db_config.json` 的备份）。同指纹 no-op 导入为 `None`
    pub backup_path: Option<String>,
    /// 是否重加密了 `db_config.json` 的 `pg_password_enc`
    pub db_config_reencrypted: bool,
}

/// 读取**已存在**的 master.key —— 绝不静默生成（模块注释契约 1）。
fn load_existing_master_key() -> Result<[u8; 32], ErrorResponse> {
    let path = crate::paths::axagent_home().join("master.key");
    if !path.exists() {
        return Err(ErrorResponse::new(master_key_err::NOT_FOUND)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(
                "master.key 文件不存在；导出需要先在加密方设备上导出，导入则需要提供对方导出的 hex",
            ));
    }
    let bytes = fs::read(&path).map_err(|e| {
        let msg = format!("master.key 读取失败: {e}");
        ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
    })?;
    let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        ErrorResponse::new(common::INVALID_INPUT)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(format!("master.key 损坏：应为 32 字节，实际 {} 字节", bytes.len()))
    })?;
    Ok(key)
}

/// 「有密文时能不能换钥」的预检决策（模块注释契约 2）。
///
/// 输入只有旧钥匙与盘上旧密文，加密 / 解密均为纯函数 ⇒ 本函数不碰文件系统，可单测。
/// 返回 `Ok(Some(明文))` = 密文可用旧钥解开，明文供写回阶段重加密复用（免二次解密）；
/// `Ok(None)` = 盘上没有密文（SQLite 模式 / 从未存过 PG 密码），无需重加密；
/// `Err` = 必须中止（不修改任何文件）。
fn plan_reencryption(
    old_key: Option<&[u8; 32]>,
    prev_enc: Option<&str>,
) -> Result<Option<String>, ErrorResponse> {
    match (prev_enc, old_key) {
        // 无密文 ⇒ 无需重加密（旧钥有无都无关紧要）
        (None, _) => Ok(None),
        // 有密文但没有旧钥：无法验证这份密码属于谁，更无法重加密 ⇒ 中止
        (Some(_), None) => Err(ErrorResponse::new(master_key_err::NOT_FOUND)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(
                "导入中止：db_config.json 存在已加密的数据库密码，但本机没有 master.key，无法验证与重加密；未修改任何文件",
            )),
        // 有密文有旧钥：必须先解开 —— 这一步同时是「旧钥可用性」的终审；
        // 解不开（含密文由其他设备钥匙写入的跨设备情形）⇒ 中止，避免把数据库密码换死
        (Some(enc), Some(old)) => decrypt_key(enc, old).map(Some).map_err(|e| {
            ErrorResponse::from_axagent_error(e, ErrorCategory::Unrecoverable).with_detail(
                "导入中止：db_config.json 中的数据库密码无法用当前主密钥解开（含密文由其他设备钥匙写入的情形），无法重加密；未修改任何文件",
            )
        }),
    }
}

/// 查询当前主密钥指纹（不含钥匙本体）。master.key 缺失时报 `MASTER_KEY_NOT_FOUND`，
/// **不会**顺手生成一把 —— 设置页该看到的是「本机还没有钥匙」，不是一把刚造出来的假钥匙。
#[agent_command(domain = "devops", safety = Safe, call_mode = StateOnly, description = "获取主密钥指纹")]
#[command]
pub fn get_master_key_info() -> Result<MasterKeyInfo, String> {
    let key = load_existing_master_key().map_err(String::from)?;
    Ok(MasterKeyInfo { fingerprint: master_key_fingerprint(&key) })
}

/// 导出主密钥（64 位 hex）。
///
/// ⚠ Dangerous：返回值经 IPC **明文**送达前端，只应由用户显式点击「导出」触发
/// （用于拷贝到其他设备导入）。每次导出都打 warn 级审计日志（指纹不是秘钥，可进日志）。
#[agent_command(domain = "devops", safety = Dangerous, call_mode = StateOnly, description = "导出主密钥（hex）")]
#[command]
pub fn export_master_key() -> Result<MasterKeyExport, String> {
    let key = load_existing_master_key().map_err(String::from)?;
    let fingerprint = master_key_fingerprint(&key);
    tracing::warn!(target: "axagent.master_key", %fingerprint, "主密钥已被导出（hex 明文经 IPC 送达前端）");
    Ok(MasterKeyExport { fingerprint, key_hex: hex::encode(key) })
}

/// 导入主密钥（64 位 hex）。流程与三条安全契约见模块注释；成功后**必须重启应用**。
#[agent_command(
    domain = "devops",
    safety = Dangerous,
    call_mode = StateInput,
    description = "导入主密钥（备份旧钥并重加密本机数据库密码）"
)]
#[command]
pub fn import_master_key(key_hex: String) -> Result<MasterKeyImportReport, String> {
    import_master_key_impl(&key_hex).map_err(String::from)
}

fn import_master_key_impl(key_hex: &str) -> Result<MasterKeyImportReport, ErrorResponse> {
    // ① 解析新钥（纯校验，失败不动任何文件）
    let new_key = master_key_from_hex(key_hex).map_err(|e| {
        ErrorResponse::new(common::INVALID_INPUT)
            .with_category(ErrorCategory::Validation)
            .with_detail(e.to_string())
    })?;
    let new_fp = master_key_fingerprint(&new_key);

    // ② 读旧钥（可缺失 —— master.key 不存在是合法的导入起点）
    let old_key: Option<[u8; 32]> = load_existing_master_key().ok();
    let old_fp = old_key.as_ref().map(master_key_fingerprint);

    // ③ 同一把钥匙 ⇒ no-op（幂等：重复导入同一份导出不应产生备份文件）
    if old_fp.as_deref() == Some(new_fp.as_str()) {
        return Ok(MasterKeyImportReport {
            old_fingerprint: old_fp,
            new_fingerprint: new_fp,
            backup_path: None,
            db_config_reencrypted: false,
        });
    }

    // ④ 预检：db_config.json 的 pg_password_enc 必须能用旧钥解开（见 plan_reencryption）
    let cfg_path = crate::paths::db_config_path();
    let prev_enc: Option<String> = fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|c| serde_json::from_str::<DbConfig>(&c).ok())
        .and_then(|c| c.pg_password_enc);
    let pg_plain = plan_reencryption(old_key.as_ref(), prev_enc.as_deref())?;

    // ⑤ 换钥：备份 → 写新钥 →（有密码密文时）重加密 db_config
    let app_dir = crate::paths::axagent_home();
    let key_path = app_dir.join("master.key");
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let key_bak = app_dir.join(format!("master.key.bak-{ts}"));

    let mut key_backup: Option<PathBuf> = None;
    if key_path.exists() {
        fs::copy(&key_path, &key_bak).map_err(|e| {
            let msg = format!("备份 master.key 失败，导入中止（未修改任何文件）: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        key_backup = Some(key_bak);
    }

    // 落盘原始 32 字节（与 load_or_create_master_key 的读取格式一致），并收紧文件权限
    fs::write(&key_path, new_key).map_err(|e| {
        let msg = format!("写入 master.key 失败（旧钥匙已备份，后缀 {ts}）: {e}");
        ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
    })?;
    crate::init::database::restrict_file_permissions(&key_path).map_err(|e| {
        let msg = format!("收紧 master.key 文件权限失败: {e}");
        ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
    })?;

    let mut cfg_backup: Option<PathBuf> = None;
    let mut reencrypted = false;
    if let Some(plain) = &pg_plain {
        let new_enc = encrypt_key(plain, &new_key).map_err(|e| {
            let msg = format!("用新钥匙重加密数据库密码失败: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        let content = fs::read_to_string(&cfg_path).map_err(|e| {
            let msg = format!("重读 db_config.json 失败: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        let mut cfg: DbConfig = serde_json::from_str(&content).map_err(|e| {
            let msg = format!("解析 db_config.json 失败: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        cfg.pg_password_enc = Some(new_enc);
        let cfg_bak = app_dir.join(format!("db_config.json.bak-{ts}"));
        fs::copy(&cfg_path, &cfg_bak).map_err(|e| {
            let msg = format!("备份 db_config.json 失败: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        let out = serde_json::to_string_pretty(&cfg).map_err(|e| {
            let msg = format!("序列化 db_config.json 失败: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        fs::write(&cfg_path, out).map_err(|e| {
            let msg = format!("写回 db_config.json 失败: {e}");
            ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).with_detail(msg)
        })?;
        cfg_backup = Some(cfg_bak);
        reencrypted = true;
    }

    // ⑥ 审计日志（指纹不是秘钥 —— crypto.rs 明确指纹可进日志 / 界面）
    tracing::info!(
        target: "axagent.master_key",
        old_fingerprint = ?old_fp,
        new_fingerprint = %new_fp,
        reencrypted,
        "主密钥已导入：内存中的钥匙在重启应用前仍是旧值，重启前不要保存新的密钥类数据"
    );

    // ⑦ 报告。backup_path：优先指 master.key 的备份；本机原本无钥匙时指 db_config.json 的备份
    let backup_path = key_backup.or(cfg_backup).map(|p| p.to_string_lossy().into_owned());
    Ok(MasterKeyImportReport {
        old_fingerprint: old_fp,
        new_fingerprint: new_fp,
        backup_path,
        db_config_reencrypted: reencrypted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_crypto::generate_master_key;

    fn code_of(r: Result<Option<String>, ErrorResponse>) -> String {
        r.expect_err("应中止").code
    }

    /// 盘上没有 db_config 密文 ⇒ 无需重加密（SQLite 模式 / 未存过 PG 密码时导入照常进行）
    #[test]
    fn no_ciphertext_means_no_reencryption() {
        let old = generate_master_key();
        assert_eq!(plan_reencryption(Some(&old), None).expect("应放行"), None);
        assert_eq!(plan_reencryption(None, None).expect("应放行"), None);
    }

    /// 有密文但本机没有旧钥匙 ⇒ MASTER_KEY_NOT_FOUND 中止（无法验证与重加密）
    #[test]
    fn ciphertext_without_old_key_aborts_with_not_found() {
        let enc = encrypt_key("pg-pass", &generate_master_key()).expect("加密应成功");
        assert_eq!(code_of(plan_reencryption(None, Some(&enc))), master_key_err::NOT_FOUND);
    }

    /// 正常路径：旧钥能解开密文 ⇒ 放行并携带明文（供写回阶段重加密复用，免二次解密）
    #[test]
    fn ciphertext_decryptable_with_old_key_yields_plaintext() {
        let old = generate_master_key();
        let enc = encrypt_key("pg-pass", &old).expect("加密应成功");
        assert_eq!(
            plan_reencryption(Some(&old), Some(&enc)).expect("应放行"),
            Some("pg-pass".to_string())
        );
    }

    /// 密文由另一把钥匙写入（跨设备 / 钥匙已被换过）⇒ 中止且错误码必须是
    /// KEY_FROM_OTHER_DEVICE（而非一锅端的 COMMON_INTERNAL）——这正是本模块要修的形态。
    #[test]
    fn ciphertext_from_other_key_aborts_with_foreign_code() {
        let enc = encrypt_key("pg-pass", &generate_master_key()).expect("加密应成功");
        let wrong = generate_master_key();
        let e = plan_reencryption(Some(&wrong), Some(&enc)).expect_err("应中止");
        assert_eq!(e.code, master_key_err::FROM_OTHER_DEVICE);
        let params = e.params.expect("跨设备错误必须携带指纹参数");
        assert!(
            params.contains_key("cipherFingerprint") && params.contains_key("localFingerprint")
        );
    }
}
