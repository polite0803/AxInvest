// SPDX-License-Identifier: AGPL-3.0-only

//! `opc_org_employees` / `opc_work_items` 的**批量读口** —— 舰队同步（`sync_fleet` op）的存储半边。
//!
//! ## 为什么在 dao
//!
//! 迁移前这两段直连 sea_orm 的查询长在宿主命令体里（`src-tauri/src/commands/opc.rs` 的
//! `opc_sync_fleet` 与它的私有辅助 `opc_batch_role_status`）。`opc.store` 接缝的宿主 provider
//! （`src-tauri/src/init/opc_store_service.rs`）分层纪律是「只调 dao、不直连库」
//! （判据见同目录 `opc_blog_post.rs` 模块头）⇒「取哪几列 / 按什么排 / 谁先谁后」下沉到这里，
//! provider 里再写一份 `Entity::find()` 就会出现第二份列清单与第二套排序口径。
//!
//! ## 两条函数的语义与迁移前逐字一致
//!
//! - 员工扫描：`status == "active"`，只取 `role_id` / `employee_id` 两列 —— 舰队成员的
//!   `agent_slug` / `agent_id` 正是这两个值；
//! - 工作项聚合：`owner_role_id IN (…)` 按 `updated_at DESC`，**每个角色取第一条**
//!   （= 最新一条），`owner_role_id` 为 `NULL` / 空串的行跳过；`role_ids` 为空时**直接回空 map**
//!   （迁移前同一早退，不发 `IN ()` 那条恒假查询）。
//!
//! 刻意**不**在这里补「无工作项角色的缺省状态」：迁移前那句 `or_insert(Idle)` 的落点是
//! `FleetMemberStatus`（harness 的 fleet 枚举），而 dao 是存储层、不认识这个枚举。
//! 「phase 串 → 成员状态」的收敛只留一处（宿主 provider 的 `_ => Idle` 分支），
//! 在 dao 里塞一个空串哨兵等于替宿主决定「没有工作项」这条业务语义。

use std::collections::HashMap;

use axagent_entities::{opc_org_employees, opc_work_items};
use axagent_harness::core_error::Result;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

/// 在册员工的 `(role_id, employee_id)` 列表（`status == "active"`）。
///
/// 不设 `ORDER BY`：迁移前的 `Entity::find()` 也没有，且调用方按 `agent_slug` 逐条判重，
/// 新增 / 更新成员与遍历次序无关 —— 排一次序只是多一次排序开销，还会给出「顺序有语义」的假象。
///
/// 列裁剪做在 SQL 侧（`select_only` 两列）：本函数只服务舰队同步，取整行会把 `expert_id` /
/// `experience_ref` 这类舰队用不到的列也拉进内存。
pub async fn list_active_opc_employees(db: &DatabaseConnection) -> Result<Vec<(String, String)>> {
    let rows = opc_org_employees::Entity::find()
        .select_only()
        .column(opc_org_employees::Column::RoleId)
        .column(opc_org_employees::Column::EmployeeId)
        .filter(opc_org_employees::Column::Status.eq("active"))
        .into_tuple::<(String, String)>()
        .all(db)
        .await?;
    Ok(rows)
}

/// 每个角色**最新一条**工作项的 phase（`owner_role_id` → `phase`）。
///
/// 排序键只有 `updated_at DESC`（与迁移前的 `order_by_desc(UpdatedAt)` 逐字同值）；
/// 同一毫秒撞键时 SQLite 的返回顺序不保证稳定，但「每个角色只取第一条」这条在这里
/// **只影响同刻并列的那几行里谁胜出**（业务上无差别：同一角色的两条同刻工作项是批灌产物），
/// 而迁移前的实现同样不做第二排序键 —— 收口口径一致，不在此单方面收紧。
///
/// `role_ids` 空 ⇒ 空 map（判据同模块头）。
pub async fn latest_work_item_phase_by_role(
    db: &DatabaseConnection,
    role_ids: &[String],
) -> Result<HashMap<String, String>> {
    let mut map: HashMap<String, String> = HashMap::new();
    if role_ids.is_empty() {
        return Ok(map);
    }

    let rows = opc_work_items::Entity::find()
        .select_only()
        .column(opc_work_items::Column::OwnerRoleId)
        .column(opc_work_items::Column::Phase)
        .filter(opc_work_items::Column::OwnerRoleId.is_in(role_ids.iter().map(|s| s.as_str())))
        .order_by_desc(opc_work_items::Column::UpdatedAt)
        .into_tuple::<(Option<String>, String)>()
        .all(db)
        .await?;

    for (owner_role_id, phase) in rows {
        let Some(role_id) = owner_role_id else { continue };
        if role_id.is_empty() || map.contains_key(&role_id) {
            continue;
        }
        map.insert(role_id, phase);
    }
    Ok(map)
}
