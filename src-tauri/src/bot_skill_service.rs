// SPDX-License-Identifier: AGPL-3.0-only

//! P5 每 Bot 技能归属服务（docs/bot-integration/P5/ADR-P5-5/6、MODEL-P5 §4）。
//!
//! 目录是真相源：技能正文在 `~/.axagent/bots/<botId>/skills/<name>/SKILL.md`；
//! 全局技能在 `~/.axagent/skills/<name>/SKILL.md`。`bot_skill_bindings` 表只作
//! **归属声明与审计**（谁声明了这个技能、是否启用、来自哪个 preset）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot_skill_binding as binding_repo;
use axagent_harness::types::{
    BotSkill, BotSkillPreset, BotSkillPresetResult, BotSkillPresetSkipped, BotSkillSource,
};

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

/// 技能名安全校验：非空、长度 ≤ 64、不含路径分隔符与 `..`、仅 ASCII 字母数字/`-`/`_`。
fn is_safe_skill_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.len() <= 64
        && !n.contains('/')
        && !n.contains('\\')
        && !n.contains("..")
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn preset_invalid(detail: impl Into<String>) -> String {
    ErrorResponse::new(bot_err::SKILL_PRESET_INVALID)
        .with_category(ErrorCategory::Validation)
        .with_detail(detail)
        .to_string()
}

fn skills_root() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".axagent").join("skills")
}

pub fn bot_skills_dir(bot_id: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".axagent").join("bots").join(bot_id).join("skills")
}

/// 读 `SKILL.md`：返回 (技能名, 描述)。目录无 SKILL.md ⇒ None。
fn read_skill_md(dir: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let description = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("---"))
        .unwrap_or("")
        .chars()
        .take(160)
        .collect::<String>();
    let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Some((name, description))
}

/// 扫描技能根目录，返回 (名, 描述, 目录路径)。
fn scan_dir_skills(dir: &Path) -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some((name, description)) = read_skill_md(&path) {
            out.push((name, description, path));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 列出 Bot 可见技能：全局 + Bot 自身目录；同名时 Bot 目录优先（`shadowed_global=true`）。
pub async fn list_bot_skills(
    db: &DatabaseConnection,
    bot_id: &str,
) -> Result<Vec<BotSkill>, String> {
    let bindings = binding_repo::list_by_bot(db, bot_id).await.map_err(|e| e.to_string())?;
    let binding_map: HashMap<String, _> =
        bindings.into_iter().map(|b| (b.skill_name.clone(), b)).collect();

    let mut by_name: HashMap<String, BotSkill> = HashMap::new();

    // 先放全局（后放的同名 Bot 项会覆盖它）。
    for (name, description, _path) in scan_dir_skills(&skills_root()) {
        let b = binding_map.get(&name);
        by_name.insert(
            name.clone(),
            BotSkill {
                name,
                description,
                source: BotSkillSource::Global,
                enabled: b.map(|x| x.enabled).unwrap_or(true),
                shadowed_global: false,
                missing: false,
                preset_id: b.and_then(|x| x.preset_id.clone()),
                updated_at: b.map(|x| x.updated_at),
            },
        );
    }

    // 再放 Bot 自身目录：同名覆盖全局，并标记 shadowed_global。
    for (name, description, _path) in scan_dir_skills(&bot_skills_dir(bot_id)) {
        let b = binding_map.get(&name);
        let shadowed = by_name.contains_key(&name);
        by_name.insert(
            name.clone(),
            BotSkill {
                name,
                description,
                source: BotSkillSource::Bot,
                enabled: b.map(|x| x.enabled).unwrap_or(true),
                shadowed_global: shadowed,
                missing: false,
                preset_id: b.and_then(|x| x.preset_id.clone()),
                updated_at: b.map(|x| x.updated_at),
            },
        );
    }

    // 归属行存在但目录缺失 ⇒ missing=true（ADR-P5-5 的一致性口径）。
    for (name, b) in &binding_map {
        if by_name.contains_key(name) {
            continue;
        }
        by_name.insert(
            name.clone(),
            BotSkill {
                name: name.clone(),
                description: String::new(),
                source: if b.source == "global" {
                    BotSkillSource::Global
                } else {
                    BotSkillSource::Bot
                },
                enabled: b.enabled,
                shadowed_global: false,
                missing: true,
                preset_id: b.preset_id.clone(),
                updated_at: Some(b.updated_at),
            },
        );
    }

    let mut items: Vec<BotSkill> = by_name.into_values().collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

/// 生成 `<bot-skills>` 提示词段（P5 C1-2）。
///
/// 只列当前 Bot 可见且启用的技能名与一行描述，不注入 SKILL.md 全文；
/// 无可见技能时返回 `None`，调用方不得追加空段（保持非 Bot/无技能行为不变）。
pub async fn bot_skills_prompt_segment(
    db: &DatabaseConnection,
    bot_id: &str,
) -> Option<String> {
    let skills = match list_bot_skills(db, bot_id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(bot_id, error = %e, "Bot 技能提示词段解析失败，跳过 <bot-skills>");
            return None;
        },
    };
    let mut lines: Vec<String> = Vec::new();
    for s in skills {
        if !s.enabled || s.missing {
            continue;
        }
        let desc = s.description.trim();
        if desc.is_empty() {
            lines.push(format!("- {}", s.name));
        } else {
            let one_line = desc.lines().next().unwrap_or(desc).trim();
            lines.push(format!("- {}: {}", s.name, one_line));
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!(
            "<bot-skills>\n以下技能对当前 Bot 可用（仅名字与摘要，正文按需用 SkillView 加载）：\n{}\n</bot-skills>",
            lines.join("\n")
        ))
    }
}

/// preset 结构校验（整包校验；任一不合法 ⇒ 整包失败，ADR-P5-6）。
fn validate_preset(preset: &BotSkillPreset) -> Result<(), String> {
    if preset.version != "bot-skill-preset.v1" {
        return Err(preset_invalid(format!(
            "不支持的 preset version: {}",
            preset.version
        )));
    }
    if preset.name.trim().is_empty() {
        return Err(preset_invalid("preset.name 不能为空"));
    }
    if preset.skills.is_empty() {
        return Err(preset_invalid("preset.skills 不能为空"));
    }
    for item in &preset.skills {
        if !is_safe_skill_name(&item.name) {
            return Err(preset_invalid(format!("非法技能名: {}", item.name)));
        }
        if item.content.trim().is_empty() {
            return Err(preset_invalid(format!(
                "技能 {} 的 content 不能为空",
                item.name
            )));
        }
        if item.content.len() > 256 * 1024 {
            return Err(preset_invalid(format!("技能 {} 的 content 过大", item.name)));
        }
    }
    Ok(())
}

/// 原子写入技能目录：先写 tmp，再 rename；已有目录先备份后替换（失败回滚）。
fn write_skill_dir_atomically(target_dir: &Path, content: &str) -> Result<(), String> {
    let parent = target_dir
        .parent()
        .ok_or_else(|| preset_invalid("无效的技能目录"))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| preset_invalid(format!("创建技能目录失败: {e}")))?;
    let stem = target_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".to_string());
    let tmp = parent.join(format!(".tmp-{stem}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).map_err(|e| preset_invalid(format!("创建临时目录失败: {e}")))?;
    if let Err(e) = std::fs::write(tmp.join("SKILL.md"), content) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(preset_invalid(format!("写入 SKILL.md 失败: {e}")));
    }
    if target_dir.exists() {
        let backup =
            parent.join(format!(".bak-{stem}-{}", uuid::Uuid::new_v4()));
        std::fs::rename(target_dir, &backup)
            .map_err(|e| preset_invalid(format!("备份旧技能目录失败: {e}")))?;
        if let Err(e) = std::fs::rename(&tmp, target_dir) {
            let _ = std::fs::rename(&backup, target_dir);
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(preset_invalid(format!("替换技能目录失败: {e}")));
        }
        let _ = std::fs::remove_dir_all(&backup);
    } else if let Err(e) = std::fs::rename(&tmp, target_dir) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(preset_invalid(format!("落盘技能目录失败: {e}")));
    }
    Ok(())
}

/// preset 导入：整包校验通过才落盘；白名单外的条目跳过并回报（ADR-P5-6）。
pub async fn import_preset(
    db: &DatabaseConnection,
    bot_id: &str,
    preset: BotSkillPreset,
) -> Result<BotSkillPresetResult, String> {
    validate_preset(&preset)?;

    let preset_id = format!("preset-{}", uuid::Uuid::new_v4());
    let bot_dir = bot_skills_dir(bot_id);
    std::fs::create_dir_all(&bot_dir)
        .map_err(|e| preset_invalid(format!("创建 Bot 技能目录失败: {e}")))?;

    let mut imported = Vec::new();
    let mut skipped = Vec::new();

    for item in &preset.skills {
        // 白名单外跳过并回报（不整包失败）。当前白名单 = 名称安全校验。
        if !is_safe_skill_name(&item.name) {
            skipped.push(BotSkillPresetSkipped {
                name: item.name.clone(),
                reason: "not_whitelisted".to_string(),
                detail: "技能名不在白名单内（仅允许字母数字、- 与 _，长度 ≤ 64）".to_string(),
            });
            continue;
        }
        write_skill_dir_atomically(&bot_dir.join(&item.name), &item.content)?;
        binding_repo::upsert(
            db,
            bot_id,
            &item.name,
            "bot",
            Some(&preset_id),
            true,
        )
        .await
        .map_err(|e| e.to_string())?;
        imported.push(item.name.clone());
    }

    Ok(BotSkillPresetResult { preset_id, imported, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_are_accepted_and_paths_rejected() {
        assert!(is_safe_skill_name("pdf-report"));
        assert!(is_safe_skill_name("skill_1"));
        assert!(!is_safe_skill_name(""));
        assert!(!is_safe_skill_name("../escape"));
        assert!(!is_safe_skill_name("a/b"));
        assert!(!is_safe_skill_name("a\\b"));
        assert!(!is_safe_skill_name(&"x".repeat(65)));
    }

    #[test]
    fn preset_validation_rejects_bad_version() {
        let preset = BotSkillPreset {
            version: "bot-skill-preset.v2".into(),
            name: "p".into(),
            description: String::new(),
            skills: vec![],
        };
        let err = validate_preset(&preset).expect_err("版本不支持");
        assert!(err.contains("BOT_SKILL_PRESET_INVALID"), "{err}");
    }

    #[test]
    fn preset_validation_rejects_empty_skill_list() {
        let preset = BotSkillPreset {
            version: "bot-skill-preset.v1".into(),
            name: "p".into(),
            description: String::new(),
            skills: vec![],
        };
        let err = validate_preset(&preset).expect_err("空技能列表");
        assert!(err.contains("BOT_SKILL_PRESET_INVALID"), "{err}");
    }
}
