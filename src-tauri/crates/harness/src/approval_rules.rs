// SPDX-License-Identifier: AGPL-3.0-only

//! 审批规则（批准沉淀）—— 权威 DTO 与存储契约。
//!
//! 对标 codex `execpolicy` 的 amendment 语义：用户批准一条命令后，把它沉淀成
//! 一条规则，下次同命令不再询问。
//!
//! ## 分层（为什么 DTO 与 trait 在 harness）
//!
//! | 关注点 | 落点 | 理由 |
//! |---|---|---|
//! | DTO + 存储契约 | **本模块**（harness） | `tools` 是 hybrid，按 AGENTS.md 铁律禁止依赖 entities / dao；读写只能经 trait |
//! | 持久化实现 | wiring（`src-tauri/src/init/approval_rule_store.rs`） | 只有 wiring 能同时看到 DB 与工具层 |
//! | 规则匹配 | `axagent-kit::approval_rules`（纯函数） | 与 `command_validator` 同层，可单测 |
//! | 命令切段 + 复算 | `axagent-tools::approval_rules` | 复用既有的 `bash::parser`，不引入第二个分词器 |
//!
//! ## 匹配语义
//!
//! 规则键是 `(program, args_prefix)`：`program` 为首 token（小写），`args_prefix`
//! 为 argv 前段的子命令前缀。命中要求「程序名精确相等 + 前缀逐 token 相等」。
//! 多条同时命中时取 [`RuleDecision::max`]（派生 `Ord` 的顺序即严重度，
//! `Allow < Prompt < Forbidden`）—— **最严者胜**。

use serde::{Deserialize, Serialize};

/// 规则裁决档。
///
/// `Ord` 的派生顺序即严重度：`Allow < Prompt < Forbidden`。多条规则命中时取
/// `max()`，与 codex `execpolicy/src/decision.rs` 的 `Ord` 派生顺序一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RuleDecision {
    /// 已批准：不再询问，直接按沙箱形态执行。
    #[default]
    Allow,
    /// 仍需询问（比 `Allow` 严，用于「只在特定条件下才问」的规则）。
    Prompt,
    /// 一律拒绝（硬拒，优先于任何 `Allow`）。
    Forbidden,
}

impl RuleDecision {
    /// 落库用的稳定字符串（与 serde 的 camelCase 输出一致）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Prompt => "prompt",
            Self::Forbidden => "forbidden",
        }
    }

    /// 解析落库字符串；未识别值回退 [`RuleDecision::Forbidden`]。
    ///
    /// 回退到**最严**档而不是 `Allow`：库里出现脏值时宁可多问一次，也不放行。
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "allow" => Self::Allow,
            "prompt" => Self::Prompt,
            "forbidden" => Self::Forbidden,
            _ => Self::Forbidden,
        }
    }
}

/// 一条沉淀规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRule {
    /// 首 token（程序名，小写，如 `git` / `npm`）。
    pub program: String,
    /// 子命令前缀（argv 前段非 flag token，最多 2 个；空 = 程序级规则）。
    pub args_prefix: Vec<String>,
    /// 裁决档。
    pub decision: RuleDecision,
    /// 来源：沉淀它的那次批准（当前记 conversation_id；手工创建记 `manual`）。
    pub source: String,
}

/// 规则存储契约。
///
/// `list` 返回 `Vec` 而非 `Result`：规则表很小，读失败时「无规则」与「没有规则表」
/// 的后果一致（退化为照常询问），因此在实现侧吞掉错误并记日志，避免把存储故障
/// 升级成调用方的错误分支。
#[async_trait::async_trait]
pub trait ApprovalRuleStore: Send + Sync + std::fmt::Debug {
    /// 全量规则。调用方自行按 `program` 建索引。
    async fn list(&self) -> Vec<ApprovalRule>;

    /// 写入（或覆盖）一条规则；`(program, args_prefix)` 幂等。
    async fn upsert(&self, rule: &ApprovalRule) -> Result<(), String>;

    /// 撤销一条规则。
    async fn revoke(&self, program: &str, args_prefix: &[String]) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::RuleDecision;

    #[test]
    fn decision_order_is_severity_ascending() {
        assert!(RuleDecision::Allow < RuleDecision::Prompt);
        assert!(RuleDecision::Prompt < RuleDecision::Forbidden);
        assert_eq!(
            [RuleDecision::Allow, RuleDecision::Forbidden, RuleDecision::Prompt].into_iter().max(),
            Some(RuleDecision::Forbidden)
        );
    }

    #[test]
    fn decision_str_roundtrip() {
        for d in [RuleDecision::Allow, RuleDecision::Prompt, RuleDecision::Forbidden] {
            assert_eq!(RuleDecision::from_str_lossy(d.as_str()), d);
        }
    }

    #[test]
    fn unknown_decision_falls_back_to_forbidden() {
        assert_eq!(RuleDecision::from_str_lossy(""), RuleDecision::Forbidden);
        assert_eq!(RuleDecision::from_str_lossy("ALLOW"), RuleDecision::Forbidden);
        assert_eq!(RuleDecision::from_str_lossy("garbage"), RuleDecision::Forbidden);
    }
}
