// SPDX-License-Identifier: AGPL-3.0-only

//! 宿主任务卡片投影（#216 批2 ③-B「审批动作宿主化」的 Rust 半）。
//!
//! 与插件卡（`PluginManifest.task_cards`）同走前端一张注册表（`pluginCardRegistry`），
//! 但真相源不同：
//! - 插件卡：声明在插件清单，写权限闸在装载期（`TaskCardActionWritePermissionMissing`）；
//! - 宿主卡：声明在这里（宿主常量表 `TASK_CARD_HOST_ACTIONS` + 首张审批卡），**不经**
//!   插件 `permissions[]` 闸 —— 宿主命令不是插件命令，不存在「装上了但没有 write」的档位；
//!   写语义的执行闸由命令自身的 `#[agent_command]` safety 档位 + 前端确认 UI 承担。
//!
//! 投影形状与 `PluginTaskCardsInfo` 逐字段同构（复用 `PluginTaskCard` / `PluginTaskCardAction`），
//! 前端 `registerCardContributions` 因此零改动吃到宿主卡（宿主先注册 ⇒ 先到先得下插件抢不了键，
//! 与 §130.3「宿主优先」裁定一致）。⚠ 唯一语义差：宿主卡的 `title` / `actions[].title` 是
//! **宿主 i18n key**（前端 `t()` 渲染），与插件卡「纯文本、不走宿主 i18n」的口径刻意相反；
//! 区分锚是 `id == "host"`。

use axagent_plugins::types::{PluginCardSurface, PluginTaskCard, PluginTaskCardAction};
use serde::Serialize;

/// 宿主侧「虚拟插件」id：宿主卡的注册表键前缀（如 `host:evolution-approval`）。
pub const HOST_PLUGIN_ID: &str = "host";

/// 待审批进化计划卡 id（#178④ 的任务中心第二入口）。
pub const EVOLUTION_APPROVAL_CARD_ID: &str = "evolution-approval";

/// 宿主动作声明表（§130.3 裁定 ③-B）：宿主自己声明的审批动作，不依赖插件 `task_cards` 声明。
///
/// `title` 是宿主 i18n key（11 语言已有，前端半零新增翻译）；`command` 必须与
/// `stock_evolution_approval.rs` 的 `#[tauri::command]` 函数名逐字同步（tauri 命令名就是
/// 函数名，改函数名必须同步这里 —— 单测钉桩会红提醒）。**宿主动作恒可执行**（见模块文档），
/// `mutating: true` 只供前端决定是否二次确认，不再有权限过滤。
///
/// 用函数而非 `const`：`PluginTaskCardAction` 的字段是 `String`，const 上下文构造不了。
fn task_card_host_actions() -> [PluginTaskCardAction; 2] {
    [
        PluginTaskCardAction {
            id: "approve".to_string(),
            title: "stockAnalysis.evolutionApproval.approve".to_string(),
            // #344：接受命令已迁插件命名命令（走插件解析路由）。
            command: "stock_approve_evolution_plan".to_string(),
            mutating: true,
        },
        PluginTaskCardAction {
            id: "reject".to_string(),
            title: "stockAnalysis.evolutionApproval.reject".to_string(),
            // #343：拒绝命令已迁插件命名命令，前端执行器按插件注册表解析路由。
            command: "stock_reject_evolution_plan".to_string(),
            mutating: true,
        },
    ]
}

/// `task_card_host_cards` 的返回条目：与 `PluginTaskCardsInfo` 逐字段同构（含 serde camelCase），
/// 前端注册管线零分支吃到宿主卡。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTaskCardsInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    /// 恒 `true`：宿主卡不进插件启停管理（没有对应开关，也不该有）。
    pub enabled: bool,
    pub task_cards: Vec<PluginTaskCard>,
}

/// 宿主审批卡（首张宿主卡）：纯计数卡，行集由前端 invoke `summary_command` 取
/// （{count} 契约 = 计数与行集同源，后端不出第二个 count 读数）。
fn evolution_approval_card() -> PluginTaskCard {
    PluginTaskCard {
        id: EVOLUTION_APPROVAL_CARD_ID.to_string(),
        title: "tasksCenter.health.pendingEvolution".to_string(),
        surface: PluginCardSurface::TaskCard,
        // #341：宿主命令已迁插件命名命令；前端 `OverviewTab` 按插件解析路由执行。
        summary_command: "stock_list_pending_evolution_plans".to_string(),
        detail_schema: None,
        tone: Some("gold".to_string()),
        order: 0,
        actions: task_card_host_actions().to_vec(),
    }
}

/// 宿主卡投影：与 `plugin_list_task_cards` 同族的中性读数（纯静态组装，无 IO、无状态）。
///
/// 行集校验不在投影层：`approve_evolution_plan` / `reject_evolution_plan` 的状态机 +
/// 并发条件 UPDATE 已在命令层闭环（非 pending 行拒绝、accepted 幂等、并发 affected==0 报错）。
#[tauri::command]
pub fn task_card_host_cards() -> Result<Vec<HostTaskCardsInfo>, String> {
    Ok(vec![HostTaskCardsInfo {
        id: HOST_PLUGIN_ID.to_string(),
        name: "host".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "host-owned task cards".to_string(),
        author: None,
        enabled: true,
        task_cards: vec![evolution_approval_card()],
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 正控钉桩：投影形状与命令名字面值。股票审批命令改名时这里红，提醒同步常量表
    /// （tauri 命令名 = 函数名，无法编译期对账，只能靠这组断言）。
    #[test]
    fn host_cards_projection_shape_is_pinned() {
        let infos = task_card_host_cards().expect("静态组装不应失败");
        assert_eq!(infos.len(), 1);
        let info = &infos[0];
        assert_eq!(info.id, "host");
        assert!(info.enabled);
        assert_eq!(info.task_cards.len(), 1);

        let card = &info.task_cards[0];
        assert_eq!(card.id, "evolution-approval");
        // 键可无损拆回两段：卡 id 不允许出现分隔符（前端 `cardKey` 拼键契约）。
        assert!(!card.id.contains(':'));
        assert_eq!(card.surface, PluginCardSurface::TaskCard);
        assert_eq!(card.summary_command, "stock_list_pending_evolution_plans");
        assert_eq!(card.detail_schema, None);
        assert_eq!(card.order, 0);

        assert_eq!(card.actions.len(), 2);
        let mut ids: Vec<&str> = card.actions.iter().map(|a| a.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["approve", "reject"]);
        for action in &card.actions {
            assert!(action.mutating, "审批动作是写语义（mutating 供前端确认用）");
            assert!(!action.command.contains(':'));
        }
        let commands: Vec<&str> = card.actions.iter().map(|a| a.command.as_str()).collect();
        assert_eq!(commands, vec!["approve_evolution_plan", "reject_evolution_plan"]);
    }

    /// 禁区 13 钉桩：投影序列化必须是 camelCase（前端 `PluginTaskCardsInfoDto` 消费面）。
    #[test]
    fn host_cards_serialize_camel_case() {
        let infos = task_card_host_cards().expect("静态组装不应失败");
        let json = serde_json::to_value(&infos).expect("序列化不应失败");
        let entry = json.as_array().expect("数组").first().expect("首条");
        assert!(entry.get("taskCards").is_some(), "taskCards 必须是 camelCase");
        assert!(entry.get("enabled").is_some());
        let card = &entry["taskCards"][0];
        assert!(card.get("summaryCommand").is_some(), "summaryCommand 必须 camelCase");
        assert!(
            card.get("detailSchema").is_some(),
            "detailSchema 必须 camelCase（None 序列化为 null）"
        );
        assert!(card.get("actions").is_some());
        let action = &card["actions"][0];
        assert!(action.get("mutating").is_some());
    }
}
