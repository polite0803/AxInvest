// SPDX-License-Identifier: AGPL-3.0-only

//! 审批规则存储的 sea_orm 实现（PLAN-codex-parity-adoption R2-1，wiring 层）。
//!
//! `tools` 是 hybrid crate，不得依赖 entities / dao，故落库实现只能放在 wiring：
//! 本模块实现 [`axagent_harness::ApprovalRuleStore`]，由 `init/state.rs` 经
//! [`axagent_tools::registry::set_global_approval_rule_store`] 注入全局。

use async_trait::async_trait;
use axagent_entities::approval_rules as entity;
use axagent_harness::{ApprovalRule, ApprovalRuleStore, RuleDecision};
use sea_orm::{DatabaseConnection, EntityTrait, QueryOrder, Set};

/// `args_prefix` 的存储分隔符（Unit Separator）—— 程序名与参数不会含该控制字符。
const SEP: char = '\u{1f}';

/// 派生主键：`{program}\u{1f}{args_prefix 连接}`。
///
/// 与 [`axagent_kit::approval_rules::sediment_key`] 的键口径一致，使同一规则的
/// upsert 幂等（同 program + 同前缀只会有一行）。
fn rule_id(program: &str, args_prefix: &[String]) -> String {
    let mut id = String::from(program);
    for a in args_prefix {
        id.push(SEP);
        id.push_str(a);
    }
    id
}

/// `\u{1f}` 连接的存储值 → 参数前缀向量（空串 ⇒ 空向量）。
fn parse_prefix(stored: &str) -> Vec<String> {
    if stored.is_empty() {
        return Vec::new();
    }
    stored.split(SEP).map(str::to_string).collect()
}

/// 基于主库连接的审批规则存储。
pub struct SeaApprovalRuleStore {
    conn: DatabaseConnection,
}

impl SeaApprovalRuleStore {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

// 手写 Debug：`DatabaseConnection` 的 Debug 会打印连接串（可能含口令），不透明化更安全。
impl std::fmt::Debug for SeaApprovalRuleStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeaApprovalRuleStore").finish_non_exhaustive()
    }
}

#[async_trait]
impl ApprovalRuleStore for SeaApprovalRuleStore {
    async fn list(&self) -> Vec<ApprovalRule> {
        let rows = match entity::Entity::find()
            .order_by_asc(entity::Column::CreatedAt)
            .all(&self.conn)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                // 读规则失败等价「无规则」：只告警，绝不把存储故障升级成调用方错误分支
                // （否则一次 DB 抖动会让 Bash 工具整体不可用）。
                tracing::warn!(error = %e, "读取审批规则失败（按无规则处理）");
                return Vec::new();
            },
        };
        rows.into_iter()
            .map(|m| ApprovalRule {
                program: m.program,
                args_prefix: parse_prefix(&m.args_prefix),
                decision: RuleDecision::from_str_lossy(&m.decision),
                source: m.source,
            })
            .collect()
    }

    async fn upsert(&self, rule: &ApprovalRule) -> Result<(), String> {
        let id = rule_id(&rule.program, &rule.args_prefix);
        let model = entity::ActiveModel {
            id: Set(id.clone()),
            program: Set(rule.program.clone()),
            args_prefix: Set(rule.args_prefix.join(&SEP.to_string())),
            decision: Set(rule.decision.as_str().to_string()),
            source: Set(rule.source.clone()),
            created_at: Set(now_secs()),
        };
        // 先删后插实现幂等 upsert：SQLite / PostgreSQL 双方言一致，无需分支。
        entity::Entity::delete_by_id(id)
            .exec(&self.conn)
            .await
            .map_err(|e| format!("清理旧审批规则失败: {e}"))?;
        entity::Entity::insert(model)
            .exec(&self.conn)
            .await
            .map_err(|e| format!("写入审批规则失败: {e}"))?;
        Ok(())
    }

    async fn revoke(&self, program: &str, args_prefix: &[String]) -> Result<(), String> {
        entity::Entity::delete_by_id(rule_id(program, args_prefix))
            .exec(&self.conn)
            .await
            .map_err(|e| format!("删除审批规则失败: {e}"))?;
        Ok(())
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
