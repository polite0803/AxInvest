// SPDX-License-Identifier: AGPL-3.0-only

//! 审批规则的纯函数匹配器。
//!
//! 与 `command_validator` 同层：只做「规则集合 × 已切好的 argv」的判定，
//! **不碰命令字符串、不碰存储、不碰 IO**。命令切段在
//! `axagent-tools::approval_rules`（复用 `bash::parser`），本模块不做第二套分词。
//!
//! ## 危险包装器黑名单
//!
//! `sudo` / `env` / `nohup` / `timeout` / `bash -c` 这类前缀把**真实命令藏在参数里**：
//! 对它们沉淀规则（例如一条 `sudo` 规则）会连带放行 `sudo rm -rf /`。
//! 故这些程序**既不参与匹配、也不允许沉淀** —— 必须看第二个 token 才算真实程序，
//! 而「看第二个 token」意味着规则键要落在被包装的命令上，这已超出「首 token 精确索引」
//! 的模型，因此本轮一律拒绝（保守优先于便利）。

use axagent_harness::{ApprovalRule, RuleDecision};

/// 改写型包装器：真实程序在参数里，规则键不可信。
///
/// 覆盖三类：
/// - 权限/环境包装：`sudo` `doas` `su` `env` `nohup` `timeout` `nice` `ionice` `setsid` `stdbuf`
/// - 间接执行：`command` `builtin` `exec` `xargs`
/// - 壳/解释器入口：`bash` `sh` `zsh` `dash` `fish` `ksh` `csh` `tcsh` `cmd` `powershell` `pwsh`
const REWRITE_WRAPPERS: &[&str] = &[
    "sudo",
    "doas",
    "su",
    "env",
    "nohup",
    "timeout",
    "nice",
    "ionice",
    "setsid",
    "stdbuf",
    "command",
    "builtin",
    "exec",
    "xargs",
    "bash",
    "sh",
    "zsh",
    "dash",
    "fish",
    "ksh",
    "csh",
    "tcsh",
    "cmd",
    "cmd.exe",
    "powershell",
    "powershell.exe",
    "pwsh",
];

/// 子命令前缀最多取几个 token。
///
/// 取 2 是为了覆盖 `npm run build` / `cargo test xxx` 这类「两级子命令」；
/// 再多会让规则退化成「记住一条完整命令」，既不适配参数变化，也让复算失去意义。
const MAX_ARGS_PREFIX: usize = 2;

/// 该程序是否是改写型包装器（不可匹配、不可沉淀）。
#[must_use]
pub fn is_rewrite_wrapper(program: &str) -> bool {
    REWRITE_WRAPPERS.contains(&program.to_ascii_lowercase().as_str())
}

/// 求一条命令的沉淀键 `(program, args_prefix)`；不可沉淀时返回 `None`。
///
/// `args_prefix` = argv 前段的非 flag token（遇到首个以 `-` 开头者即停止），
/// 最多 [`MAX_ARGS_PREFIX`] 个。`ls -la` ⇒ `("ls", [])`（程序级规则），
/// `git commit -m x` ⇒ `("git", ["commit"])`。
#[must_use]
pub fn sediment_key(program: &str, args: &[String]) -> Option<(String, Vec<String>)> {
    let program = program.trim().to_ascii_lowercase();
    if program.is_empty() || is_rewrite_wrapper(&program) {
        return None;
    }
    let mut prefix = Vec::new();
    for arg in args {
        if prefix.len() >= MAX_ARGS_PREFIX || arg.starts_with('-') || arg.is_empty() {
            break;
        }
        prefix.push(arg.to_ascii_lowercase());
    }
    Some((program, prefix))
}

/// 在规则集合中选出对 `(program, args)` 生效的裁决。
///
/// 命中条件：`rule.program == program` 且 `rule.args_prefix` 是 `args` 的前缀。
/// 多条命中取最严（[`RuleDecision`] 的 `Ord`，最严者胜）。
///
/// 包装器程序直接返回 `None`（即使库里存在对应规则也不生效）—— 见模块文档。
#[must_use]
pub fn select_decision(
    rules: &[ApprovalRule],
    program: &str,
    args: &[String],
) -> Option<RuleDecision> {
    let program = program.trim().to_ascii_lowercase();
    if program.is_empty() || is_rewrite_wrapper(&program) {
        return None;
    }
    let args_lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    rules
        .iter()
        .filter(|rule| rule.program.eq_ignore_ascii_case(&program))
        .filter(|rule| args_lower.starts_with(&rule.args_prefix))
        .map(|rule| rule.decision)
        .max()
}

#[cfg(test)]
mod tests {
    use super::{is_rewrite_wrapper, sediment_key, select_decision};
    use axagent_harness::{ApprovalRule, RuleDecision};

    fn rule(program: &str, prefix: &[&str], decision: RuleDecision) -> ApprovalRule {
        ApprovalRule {
            program: program.to_string(),
            args_prefix: prefix.iter().map(|s| s.to_string()).collect(),
            decision,
            source: "test".to_string(),
        }
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn wrappers_are_never_matchable() {
        for w in ["sudo", "env", "nohup", "timeout", "bash", "cmd", "PowerShell", "xargs"] {
            assert!(is_rewrite_wrapper(w), "{w} 应被视为改写型包装器");
            assert_eq!(sediment_key(w, &args(&["rm", "-rf", "/"])), None, "{w} 不应可沉淀");
            let rules = vec![rule(w, &[], RuleDecision::Allow)];
            assert_eq!(
                select_decision(&rules, w, &args(&["rm"])),
                None,
                "{w} 的既有规则也不应生效"
            );
        }
    }

    #[test]
    fn sediment_key_takes_leading_non_flag_tokens() {
        assert_eq!(sediment_key("ls", &args(&["-la"])), Some(("ls".to_string(), vec![])));
        assert_eq!(
            sediment_key("git", &args(&["commit", "-m", "x"])),
            Some(("git".to_string(), vec!["commit".to_string()]))
        );
        assert_eq!(
            sediment_key("npm", &args(&["run", "build", "extra"])),
            Some(("npm".to_string(), vec!["run".to_string(), "build".to_string()]))
        );
        // 大小写归一
        assert_eq!(
            sediment_key("GIT", &args(&["COMMIT"])),
            Some(("git".to_string(), vec!["commit".to_string()]))
        );
    }

    #[test]
    fn sediment_key_rejects_empty_program() {
        assert_eq!(sediment_key("", &args(&["ls"])), None);
        assert_eq!(sediment_key("   ", &args(&["ls"])), None);
    }

    #[test]
    fn program_level_rule_matches_any_args() {
        let rules = vec![rule("ls", &[], RuleDecision::Allow)];
        assert_eq!(select_decision(&rules, "ls", &args(&["-la"])), Some(RuleDecision::Allow));
        assert_eq!(select_decision(&rules, "ls", &[]), Some(RuleDecision::Allow));
        // 程序名不同不命中
        assert_eq!(select_decision(&rules, "cat", &args(&["-la"])), None);
    }

    #[test]
    fn prefix_rule_requires_token_prefix() {
        let rules = vec![rule("git", &["commit"], RuleDecision::Allow)];
        assert_eq!(
            select_decision(&rules, "git", &args(&["commit", "-m", "x"])),
            Some(RuleDecision::Allow)
        );
        // 前缀必须从第一个 token 起严格相等
        assert_eq!(select_decision(&rules, "git", &args(&["push"])), None);
        assert_eq!(select_decision(&rules, "git", &args(&["commit-extra"])), None);
        assert_eq!(select_decision(&rules, "git", &[]), None);
    }

    #[test]
    fn multiple_hits_take_most_restrictive() {
        let rules = vec![
            rule("git", &[], RuleDecision::Allow),
            rule("git", &["commit"], RuleDecision::Forbidden),
            rule("git", &["commit"], RuleDecision::Prompt),
        ];
        assert_eq!(
            select_decision(&rules, "git", &args(&["commit"])),
            Some(RuleDecision::Forbidden),
            "Allow + Prompt + Forbidden 同命中必须取 Forbidden"
        );
        // 更精确的前缀不改变严重度语义：仍取最严
        assert_eq!(select_decision(&rules, "git", &args(&["status"])), Some(RuleDecision::Allow));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let rules = vec![rule("git", &["commit"], RuleDecision::Allow)];
        assert_eq!(select_decision(&rules, "GIT", &args(&["COMMIT"])), Some(RuleDecision::Allow));
    }
}
