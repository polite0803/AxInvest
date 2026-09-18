// SPDX-License-Identifier: AGPL-3.0-only

//! 通用工具函数
//!
//! 零业务逻辑的纯工具函数，供各 crate 共享使用。

/// 生成 UUID v4 字符串 ID
pub fn gen_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 获取当前 Unix 时间戳（秒）
pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 获取当前 Unix 时间戳（毫秒）
///
/// 毫秒是运行时侧（TTL 过期、能力统计、会话状态）的统一时间单位；
/// 秒级 `now_ts` 只用于 DB TEXT 时间列与跨天计算，两者不要混用。
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 返回当前 UTC 时间的 `"YYYY-MM-DD HH:MM:SS"` 格式字符串。
///
/// 这是数据库 TEXT 类型时间列的统一写入格式，与 SQLite `datetime('now')`
/// 以及 PG `to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS')`
/// 默认值格式完全一致，保证字符串排序与时序一致。
pub fn now_datetime_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 返回"本地时区今日 00:00:00"对应的 Unix 时间戳（秒）。
///
/// 用于 dashboard / gateway 的"今日"统计：避免 `chrono::Utc::now().date_naive()`
/// 带来的 UTC 日切换偏移（对中国用户，UTC 日等于本地 08:00 才切换，凌晨 0–8 点
/// 看到的"今日"会变成 UTC 的"昨日"）。
///
/// 若本地时区检测失败（极少见），回退到 UTC 今日 0 点。
pub fn today_start_local_ts() -> i64 {
    use chrono::{Datelike, TimeZone};
    let now_local = chrono::Local::now();
    chrono::Local
        .with_ymd_and_hms(now_local.year(), now_local.month(), now_local.day(), 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| {
            // 回退：UTC 今日 0 点
            chrono::Utc::now()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap_or_else(|| {
                    chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
                        .expect("1970-01-01 is a valid NaiveDate")
                        .and_hms_opt(0, 0, 0)
                        .expect("00:00:00 is a valid NaiveTime")
                })
                .and_utc()
                .timestamp()
        })
}

/// 获取当前时间的 RFC3339 格式字符串
pub fn current_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Estimate the number of tokens in a text string.
///
/// Heuristic:
/// - ASCII / Latin characters: ~4 characters per token
/// - CJK / fullwidth characters: ~1.5 characters per token
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    let mut ascii_chars: usize = 0;
    let mut cjk_chars: usize = 0;

    for ch in text.chars() {
        if is_cjk(ch) {
            cjk_chars += 1;
        } else {
            ascii_chars += 1;
        }
    }

    let ascii_tokens = ascii_chars.div_ceil(4);
    let cjk_tokens = (cjk_chars * 2).div_ceil(3);

    ascii_tokens + cjk_tokens
}

/// Estimate tokens for an entire chat message (content + role overhead).
pub fn estimate_message_tokens(role: &str, content: &str) -> usize {
    const PER_MESSAGE_OVERHEAD: usize = 4;
    estimate_tokens(role) + estimate_tokens(content) + PER_MESSAGE_OVERHEAD
}

/// Check if a character is in a CJK Unicode block.
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{3000}'..='\u{303F}'
        | '\u{FF00}'..='\u{FFEF}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{3040}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
    )
}

/// 生成 `n` 个 SQL 参数占位符（逗号分隔），按后端的方言产出。
///
/// # 为什么必须按 `backend` 产出，不能用 `bool is_pg`
///
/// 占位符方言是 **driver 协议层面** 的差异：PostgreSQL 用 `$1, $2, …`，
/// SQLite / MySQL 用 `?, ?, …`。`Statement::from_sql_and_values` **不做转换** ——
/// 它只把 `sql` 文本、`values`、`backend` 三者一起交给 driver。
///
/// 因此凡是拼接含参 SQL 的地方，**SQL 文本里的占位符与后端必须一致**：
/// - 在 PG 上写 `?` ⇒ 语法错误
/// - 在 SQLite 上写 `$1` ⇒ 语法错误（`$` 不是 SQLite 的参数前缀）
///
/// 反面教材（本仓库真实存在过的三类缺陷，均已修）：
/// ① 把值直接 `format!` 拼进 SQL 文本（`VALUES (..., {"k":"v"}, ...)` ⇒ 语法错误，
///    且引号/反斜杠注入）；
/// ② 后端动态取 `get_database_backend()`，SQL 却硬编码 `$1` ⇒ SQLite 路径必崩；
/// ③ 后端硬编码 `DbBackend::Sqlite`，SQL 却是 `$1` ⇒ PG 路径语义错。
///
/// `n == 0` 返回空串（两侧一致），可用于「无条件子句」的拼接。
pub fn bind_placeholders(n: usize, backend: sea_orm::DbBackend) -> String {
    match backend {
        sea_orm::DbBackend::Postgres => {
            (1..=n).map(|i| format!("${i}")).collect::<Vec<_>>().join(", ")
        },
        _ => vec!["?"; n].join(", "),
    }
}

/// 生成第 `index`（从 1 开始）个占位符，用于只参数化部分条件的场景。
///
/// 与 [`bind_placeholders`] 同源，避免调用点各自手写 `"$1"` 字面量。
pub fn placeholder_at(index: usize, backend: sea_orm::DbBackend) -> String {
    match backend {
        sea_orm::DbBackend::Postgres => format!("${index}"),
        _ => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_placeholders_postgres_uses_dollar_numbering() {
        assert_eq!(bind_placeholders(1, sea_orm::DbBackend::Postgres), "$1");
        assert_eq!(bind_placeholders(3, sea_orm::DbBackend::Postgres), "$1, $2, $3");
    }

    #[test]
    fn bind_placeholders_sqlite_uses_question_marks() {
        assert_eq!(bind_placeholders(1, sea_orm::DbBackend::Sqlite), "?");
        assert_eq!(bind_placeholders(3, sea_orm::DbBackend::Sqlite), "?, ?, ?");
    }

    /// `n == 0` 两侧必须都返回空串 —— 否则「无条件子句」的拼接会出现
    /// PG 空、SQLite 吐出 `" "` 之类的方言不对称。
    #[test]
    fn bind_placeholders_zero_is_empty_on_both_backends() {
        for backend in [sea_orm::DbBackend::Postgres, sea_orm::DbBackend::Sqlite] {
            assert_eq!(bind_placeholders(0, backend), "", "backend = {backend:?}");
        }
    }

    #[test]
    fn placeholder_at_matches_backend_dialect() {
        assert_eq!(placeholder_at(1, sea_orm::DbBackend::Postgres), "$1");
        assert_eq!(placeholder_at(7, sea_orm::DbBackend::Postgres), "$7");
        assert_eq!(placeholder_at(1, sea_orm::DbBackend::Sqlite), "?");
        assert_eq!(placeholder_at(7, sea_orm::DbBackend::Sqlite), "?");
    }

    /// 反向断言：`$n` 编号必须从 1 开始且连续 ——
    /// 起点写错（如 `0..n`）在 PG 上会报「there is no parameter $0」，
    /// 而 SQLite 侧毫无察觉（`?` 无编号），只有这个测试能拦住。
    #[test]
    fn postgres_placeholders_start_at_one_and_are_contiguous() {
        let out = bind_placeholders(4, sea_orm::DbBackend::Postgres);
        assert_eq!(out, "$1, $2, $3, $4");
        assert!(!out.contains("$0"), "PG 参数编号从 1 开始，不存在 $0");
    }

    #[test]
    fn truncate_ascii_unchanged() {
        assert_eq!(truncate_to_char_boundary("hello world", 5), "hello");
    }

    #[test]
    fn truncate_chinese_safe_at_boundary() {
        // 5 个汉字 = 15 字节；14 字节落在字符中间，必须回退到 12（4 字符）
        let s = "一二三四五"; // 5 chars, 15 bytes
        assert_eq!(s.len(), 15);
        assert_eq!(truncate_to_char_boundary(s, 14), "一二三四");
        assert_eq!(truncate_to_char_boundary(s, 16), "一二三四五");
    }

    #[test]
    fn truncate_short_input_unchanged() {
        let s = "abc";
        assert_eq!(truncate_to_char_boundary(s, 100), "abc");
    }

    #[test]
    fn truncate_emoji_4byte() {
        let s = "🎉🎉";
        assert_eq!(s.len(), 8);
        assert_eq!(truncate_to_char_boundary(s, 3), "");
        assert_eq!(truncate_to_char_boundary(s, 4), "🎉");
    }

    #[test]
    fn truncate_empty() {
        assert_eq!(truncate_to_char_boundary("", 0), "");
        assert_eq!(truncate_to_char_boundary("", 100), "");
    }
}
