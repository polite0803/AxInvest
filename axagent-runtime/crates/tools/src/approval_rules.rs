// SPDX-License-Identifier: AGPL-3.0-only

//! 审批规则的命令切段、整体评估与「沉淀前复算」。
//!
//! 本模块是规则与**真实命令字符串**之间的唯一粘合层：
//! - 切段复用 [`crate::bash::parser::parse_command`]（已有 `|` / `&&` / `||` / `;` /
//!   `Redirect` 词法，不引入第二个分词器）；
//! - 逐段匹配复用 [`axagent_kit::approval_rules::select_decision`]（纯函数）；
//! - 一切「是否放行」的最终裁决仍归 `crate::approval`（沙箱形态 + 审批策略），
//!   本模块只产出「规则能否覆盖整条命令」这一结论。
//!
//! ## 为什么必须逐段评估
//!
//! `a && rm -rf /` 里若只按首 token 匹配，`a` 命中允许规则后整条命令就变宽松了。
//! 故判定 [`RuleVerdict::AllowAll`] 要求**每一个子命令段**都被允许规则覆盖；
//! 任一段未被覆盖即返回 [`RuleVerdict::Incomplete`]，交回审批策略照常裁决。

use crate::bash::parser::{ParsedCommand, parse_command};
use axagent_harness::{ApprovalRule, RuleDecision};

/// 一条命令相对规则集合的整体裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleVerdict {
    /// 全部子命令段都被 `Allow` 覆盖 ⇒ 可免询问。
    AllowAll,
    /// 任一子命令段命中 `Forbidden` ⇒ 一律拒绝（不被其他段的 `Allow` 抵消）。
    Deny,
    /// 未命中 / 仅部分命中 / 命中 `Prompt` ⇒ 交回沙箱与审批策略裁决。
    Incomplete,
}

/// 摊平 `ParsedCommand` 链为 `(program, args)` 序列。
///
/// 覆盖三种连接：管道（`next_pipe`）、逻辑连接（`&&` / `||`，[`ConditionalOp`]）、
/// 以及 `argv` 里携带的环境变量前缀（`FOO=1 cmd` —— parser 已把 env 拆进
/// `env_vars`，`argv[0]` 就是真实程序名，无需再剥）。
fn flatten(parsed: &ParsedCommand, out: &mut Vec<(String, Vec<String>)>) {
    if let Some((program, rest)) = parsed.argv.split_first() {
        out.push((program.clone(), rest.to_vec()));
    }
    if let Some(next) = parsed.next_pipe.as_deref() {
        flatten(next, out);
    }
    if let Some((_, next)) = parsed.next_conditional.as_ref() {
        flatten(next, out);
    }
}

/// 预扫描：拒绝 `bash::parser` **会静默丢弃/吞并**的结构。
///
/// 这是本模块最重要的保守判据。`parse_command` 的词法有两处已知损失：
/// - `Token::Semicolon` 直接 `break` ⇒ `a ; rm -rf /` 只会看到 `a`；
/// - `Token::Background` 不中断 ⇒ `a & rm -rf /` 会把 `rm` 并进 `a` 的 argv。
///
/// 两者都会让「单段命令」的判定失真（隐藏了真实存在的第二个命令），据此沉淀规则
/// 等于给一条被隐藏的命令背书。故凡出现这类分隔符（以及命令替换 `` ` `` / `$(`），
/// 一律视为**不可用规则判定**。
fn scan_rule_unsafe(input: &str) -> Result<(), String> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < chars.len() {
        let c = chars[i];
        if in_single {
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => in_single = true,
            '"' => in_double = true,
            ';' | '\n' | '\r' | '`' => {
                return Err(format!("命令含规则不可判定的分隔符 '{c}'"));
            },
            '&' => {
                // `&&` 由 parser 正确表达，单个 `&`（后台/分隔）不行
                if chars.get(i + 1) != Some(&'&') {
                    return Err("命令含后台/分隔符 '&'".to_string());
                }
                i += 1;
            },
            '$' if chars.get(i + 1) == Some(&'(') => {
                return Err("命令含命令替换 '$('".to_string());
            },
            _ => {},
        }
        i += 1;
    }
    if in_single || in_double {
        return Err("命令含未闭合引号".to_string());
    }
    Ok(())
}

/// 把命令切成 `(program, args)` 序列；不可用于规则判定时返回 `Err`。
///
/// 不可用 ⇒ 调用方必须保守处理（不做任何规则免询问、不沉淀）。
pub fn segments(command: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    scan_rule_unsafe(command)?;
    let parsed = parse_command(command)?;
    let mut out = Vec::new();
    flatten(&parsed, &mut out);
    Ok(out)
}

/// 评估一条命令相对规则集合的整体裁决。
///
/// `Err`（不可解析）与空段序列都归入 [`RuleVerdict::Incomplete`] —— 即「规则管不了」，
/// 由审批策略照常裁决，绝不因为「看不懂」而放行。
pub fn evaluate(command: &str, rules: &[ApprovalRule]) -> RuleVerdict {
    let Ok(segs) = segments(command) else {
        return RuleVerdict::Incomplete;
    };
    if segs.is_empty() {
        return RuleVerdict::Incomplete;
    }

    let mut all_allow = true;
    for (program, args) in &segs {
        match axagent_kit::approval_rules::select_decision(rules, program, args) {
            Some(RuleDecision::Forbidden) => return RuleVerdict::Deny,
            Some(RuleDecision::Allow) => {},
            // Prompt 或未命中：整条命令不满足「全部允许」
            Some(RuleDecision::Prompt) | None => all_allow = false,
        }
    }
    if all_allow {
        RuleVerdict::AllowAll
    } else {
        RuleVerdict::Incomplete
    }
}

/// 求本次批准的沉淀规则；不满足沉淀条件时返回 `None`。
///
/// ## 沉淀前的复算（对标 codex amendment 里最易漏的一环）
///
/// 三条门槛，缺一不可：
///
/// 1. **只允许单段命令** —— 多段命令（`a && rm -rf /`）里任一段被单独放行，都可能让
///    整条命令的语义变宽松；与其逐段复算「是否变宽松」，不如直接拒绝沉淀。这一条
///    同时覆盖了验收用例「`a && rm -rf /` 不得沉淀出 `a` 的规则」。
/// 2. **不是改写型包装器** —— 见 [`axagent_kit::approval_rules::is_rewrite_wrapper`]。
/// 3. **复算自证** —— 把候选规则加入现有规则集后重新评估本命令，必须得到
///    [`RuleVerdict::AllowAll`]。若不然，说明候选键与实际切段口径不一致（规则写了也
///    不生效），此时**写入一条无效规则比不写更糟**（用户以为已记住，实际仍会再问）。
pub fn safe_to_sediment(
    command: &str,
    rules: &[ApprovalRule],
    source: &str,
) -> Option<ApprovalRule> {
    let segs = segments(command).ok()?;
    if segs.len() != 1 {
        return None;
    }
    let (program, args) = &segs[0];
    let (program, args_prefix) = axagent_kit::approval_rules::sediment_key(program, args)?;
    let candidate = ApprovalRule {
        program,
        args_prefix,
        decision: RuleDecision::Allow,
        source: source.to_string(),
    };

    let mut with_candidate = rules.to_vec();
    with_candidate.push(candidate.clone());
    if evaluate(command, &with_candidate) != RuleVerdict::AllowAll {
        return None;
    }
    Some(candidate)
}

#[cfg(test)]
mod tests {
    use super::{RuleVerdict, evaluate, safe_to_sediment, segments};
    use axagent_harness::{ApprovalRule, RuleDecision};

    fn rule(program: &str, prefix: &[&str], decision: RuleDecision) -> ApprovalRule {
        ApprovalRule {
            program: program.to_string(),
            args_prefix: prefix.iter().map(|s| s.to_string()).collect(),
            decision,
            source: "test".to_string(),
        }
    }

    #[test]
    fn segments_splits_pipes_and_conditionals() {
        let segs = segments("ls -la | grep foo && cat bar.txt").expect("可解析");
        let programs: Vec<&str> = segs.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(programs, vec!["ls", "grep", "cat"]);
        assert_eq!(segs[0].1, vec!["-la".to_string()]);
        assert_eq!(segs[2].1, vec!["bar.txt".to_string()]);
    }

    #[test]
    fn segments_strips_env_prefix() {
        let segs = segments("FOO=bar ls").expect("可解析");
        assert_eq!(segs[0].0, "ls");
        assert!(segs[0].1.is_empty());
    }

    #[test]
    fn evaluate_allows_only_when_every_segment_covered() {
        let rules = vec![rule("ls", &[], RuleDecision::Allow)];
        assert_eq!(evaluate("ls -la", &rules), RuleVerdict::AllowAll);
        // 第二个段未被覆盖 ⇒ 整条不满足
        assert_eq!(evaluate("ls -la && cat x", &rules), RuleVerdict::Incomplete);
    }

    #[test]
    fn evaluate_forbidden_wins_over_allow() {
        let rules =
            vec![rule("ls", &[], RuleDecision::Allow), rule("cat", &[], RuleDecision::Forbidden)];
        assert_eq!(evaluate("ls -la | cat x", &rules), RuleVerdict::Deny);
    }

    #[test]
    fn evaluate_prompt_is_not_allow_all() {
        let rules = vec![rule("ls", &[], RuleDecision::Prompt)];
        assert_eq!(evaluate("ls -la", &rules), RuleVerdict::Incomplete);
    }

    #[test]
    fn unsafe_separators_are_incomplete() {
        // 故意传入能覆盖 `ls` 的规则：若切段失真（`;` 之后被 parser 丢弃、
        // `&` 之后被并进 argv），这里会误判为 AllowAll —— 本测试即锁住该风险。
        let rules = vec![rule("ls", &[], RuleDecision::Allow)];
        assert_eq!(evaluate("ls ; rm -rf /", &rules), RuleVerdict::Incomplete);
        assert_eq!(evaluate("ls & rm -rf /", &rules), RuleVerdict::Incomplete);
        assert_eq!(evaluate("ls `rm -rf /`", &rules), RuleVerdict::Incomplete);
        assert_eq!(evaluate("ls $(rm -rf /)", &rules), RuleVerdict::Incomplete);
        // 未闭合引号同样不可判定
        assert_eq!(evaluate("ls 'unclosed", &rules), RuleVerdict::Incomplete);
    }

    #[test]
    fn sediment_rejects_multi_segment_commands() {
        // 验收用例：`a && rm -rf /` 不得沉淀出 `a` 的规则
        assert_eq!(safe_to_sediment("a && rm -rf /", &[], "conv-1"), None);
        assert_eq!(safe_to_sediment("git status | tee log", &[], "conv-1"), None);
    }

    #[test]
    fn sediment_rejects_wrappers() {
        assert_eq!(safe_to_sediment("sudo rm -rf build", &[], "conv-1"), None);
        assert_eq!(safe_to_sediment("bash -c 'rm -rf /'", &[], "conv-1"), None);
        assert_eq!(safe_to_sediment("env FOO=1 cmd", &[], "conv-1"), None);
    }

    #[test]
    fn sediment_produces_program_level_and_prefix_rules() {
        let r = safe_to_sediment("ls -la", &[], "conv-1").expect("可沉淀");
        assert_eq!(r.program, "ls");
        assert!(r.args_prefix.is_empty());
        assert_eq!(r.decision, RuleDecision::Allow);
        assert_eq!(r.source, "conv-1");

        let r = safe_to_sediment("git commit -m msg", &[], "conv-1").expect("可沉淀");
        assert_eq!(r.program, "git");
        assert_eq!(r.args_prefix, vec!["commit".to_string()]);
    }

    #[test]
    fn sediment_recompute_self_checks_the_candidate() {
        // 沉淀后的规则集必须能覆盖本命令（否则写入的是无效规则）
        let existing = vec![rule("git", &["push"], RuleDecision::Forbidden)];
        let r = safe_to_sediment("git commit -m msg", &existing, "conv-1").expect("可沉淀");
        let mut with_new = existing.clone();
        with_new.push(r);
        assert_eq!(evaluate("git commit -m msg", &with_new), RuleVerdict::AllowAll);
        // 既有 Forbidden 仍生效于其自身命令
        assert_eq!(evaluate("git push origin main", &with_new), RuleVerdict::Deny);
    }

    #[test]
    fn sediment_rejects_unsafe_separators() {
        // 不得为被 parser 隐藏的第二条命令背书（`;` 丢弃 / `&` 吞并）
        assert_eq!(safe_to_sediment("ls ; rm -rf /", &[], "conv-1"), None);
        assert_eq!(safe_to_sediment("ls & rm -rf /", &[], "conv-1"), None);
        assert_eq!(safe_to_sediment("ls 'unclosed", &[], "conv-1"), None);
    }
}
