// SPDX-License-Identifier: AGPL-3.0-only

//! 设置面板变量表的**进程内快照**：wiring 层装入，下层按名读。
//!
//! ## 为什么必须有它（不是偏好，是分层逼出来的唯一出口）
//!
//! 设置面板（`src/components/settings/StockAnalysisConfigPanel.tsx`）改的是 DB 里
//! `workflow_template.variables`（`stock-analysis` 那一行），走的是通用命令
//! `update_workflow_template`。而一部分参数的**落点在下面的 crate 里**：
//! `astock-data` 的评分分段（RSI 内带、PE 修正阈值）、新闻取数条数，
//! `analysis-engine` 的仓位限制。那些函数既拿不到 DB 连接，也接不到节点
//! `input_mapping`（接图就得 bump `TEMPLATE_VERSION`，而「不发版」是这批的前提 ——
//! 见 `PLAN-four-horizon-workflow-alignment.md` §一一七）。
//!
//! 于是：wiring 层（`src/init/`、种子完成处、模板保存处）把变量表**整表**装进本模块，
//! 下层按名取值。**依赖方向仍是 `组件 → harness ← 实现`** —— 本模块零 `axagent-*` 依赖，
//! 与 `feedback_data_lake::register_feedback_lake` / `capability_registry` 同一形
//! （wiring 注入、下游只认 harness 契约）。
//!
//! ## 一条不可让的判据：本模块**不持有任何默认值**
//!
//! 默认值住在各落点自己的 `Default` / 常量里（那是今天的权威）。本模块只回答
//! 「面板有没有给这个键一个**可用**数值」：缺失 / 非数值 / null / NaN / ±INF 一律 `None`，
//! 由调用方回落到它自己的默认值。若在这里写一份默认值，就造出**第二权威** ——
//! 本仓「同一个量两处、值不同」那族缺陷的成因正是这样一份手抄默认值。
//!
//! ⚠ 读侧的「可用」判据刻意做成**纯函数**（`number_in` / `integer_in` 收 `&HashMap`），
//!   快照版（`number` / `integer`）只是三行包装。这样「缺失 ⇒ 回落」「改值 ⇒ 生效」
//!   两类断言都不依赖全局态、可在并行测试里逐情形测。

use std::collections::HashMap;
use std::sync::LazyLock;

use parking_lot::RwLock;

/// 快照本体：变量名 → 变量值（即 DB 里每条 `Variable.value` 的原样 JSON）。
///
/// 用 `parking_lot::RwLock` 而非 `tokio::sync::RwLock`：读侧大量落在**同步**纯函数里
/// （`ScoreBands` 构造、`PositionLimits` 构造），异步锁会把整条同步链改成 async。
/// 临界区只做一次 `HashMap` 读 / 整表写，不跨 await，符合 `clippy.toml` 的合法例外口径。
static SNAPSHOT: LazyLock<RwLock<HashMap<String, serde_json::Value>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// 变量表 JSON（`[{ "name": …, "value": … }, …]`）→ 名值映射。
///
/// 纯函数（不碰全局态）⇒ 逐情形可测：非数组、条目缺 `name`、条目缺 `value`、
/// 同名重复（**后者覆盖前者**，与 `merge_variable_values` 的按名一对一语义一致）。
/// 入参是 `Value::Null`（DB 列 `variables IS NULL`，即模板从未播种过变量）时返回空表
/// ⇒ 读侧全部回落默认值，而不是 panic 或报错。
pub fn variables_map(table: &serde_json::Value) -> HashMap<String, serde_json::Value> {
    let Some(entries) = table.as_array() else {
        return HashMap::new();
    };
    let mut out = HashMap::with_capacity(entries.len());
    for entry in entries {
        let Some(name) = entry.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        // `value` 缺失时存 `Null` 而不是跳过：读侧按「非数值 ⇒ None」回落，
        // 而「键存在但无值」与「键不存在」对默认值的影响本就相同，做成一致更省一层判据。
        out.insert(
            name.to_string(),
            entry.get("value").cloned().unwrap_or(serde_json::Value::Null),
        );
    }
    out
}

/// [`variables_map`] 的字符串入口（DB 的 `variables` 列是 TEXT）。
///
/// 解析失败 ⇒ 空表（读侧全回落），并 warn 一条：**「变量表读不出来」必须留痕**，
/// 否则面板改了没反应的缺陷会在这里静默重生（那正是这批要消灭的形态）。
pub fn variables_map_from_str(raw: &str) -> HashMap<String, serde_json::Value> {
    if raw.trim().is_empty() {
        return HashMap::new();
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(table) => variables_map(&table),
        Err(e) => {
            tracing::warn!(
                "[panel_variables] 变量表 JSON 解析失败（{e}）⇒ 全部参数回落 Rust 默认值"
            );
            HashMap::new()
        },
    }
}

/// 整表装入快照，返回装入条数（供调用方把「装了什么」打进启动日志做自证）。
///
/// 语义是**替换**而非合并：变量表里被删掉的键，快照里也必须消失，
/// 否则退役一条声明反而会留下一个「面板已无控件、读侧仍在读旧值」的幽灵键。
pub fn install_panel_variables(map: HashMap<String, serde_json::Value>) -> usize {
    let len = map.len();
    *SNAPSHOT.write() = map;
    len
}

/// [`variables_map_from_str`] + [`install_panel_variables`] 的一步式入口（wiring 常用形态）。
pub fn install_panel_variables_from_str(raw: &str) -> usize {
    install_panel_variables(variables_map_from_str(raw))
}

/// 当前快照的克隆（供「按变量表构造」的落点函数取用）。
///
/// 返回克隆而不是持有读守卫：落点常在持锁之后才调下游（又一次读锁），
/// 传 `&HashMap` 出去可保证「一次构造读到的是一份一致的表」。
pub fn panel_variables() -> HashMap<String, serde_json::Value> {
    SNAPSHOT.read().clone()
}

/// 快照里有多少条（启动日志与自证断言用；0 表示变量表还没装进来）。
pub fn panel_variables_len() -> usize {
    SNAPSHOT.read().len()
}

/// 一个键的**可用整数值**（纯函数版）。
///
/// 判据：必须是 JSON 数值、有限、且**确为整数**（`10.5` 不算）。
/// 非数值（字符串 / null / bool）一律 `None` ⇒ 调用方回落。
/// 这与 v137 那批 `panel_bar_arg` 的「显式失败」同判据，只是这里没有 `Result` 通道
/// （落点在同步纯函数里），于是把「失败」表达成「不给值 ⇒ 用默认」并留 warn 痕迹
/// —— warn 而不是静默：静默回落就是「面板改了没反应」换了个形态复发。
pub fn integer_in(vars: &HashMap<String, serde_json::Value>, name: &str) -> Option<i64> {
    let v = numeric_in(vars, name)?;
    if v.fract() != 0.0 {
        tracing::warn!("[panel_variables] {name} = {v} 不是整数 ⇒ 回落默认值");
        return None;
    }
    Some(v as i64)
}

/// [`integer_in`] 的快照版。
pub fn integer(name: &str) -> Option<i64> {
    integer_in(&panel_variables(), name)
}

/// 一个键的**可用浮点值**（纯函数版）：JSON 数值且有限 ⇒ Some，其余 None。
pub fn numeric_in(vars: &HashMap<String, serde_json::Value>, name: &str) -> Option<f64> {
    match vars.get(name) {
        Some(value) => match value.as_f64() {
            Some(v) if v.is_finite() => Some(v),
            // `as_f64()` 对字符串 / null / bool 返回 None —— 那也是「不可用」，同样 warn 留痕。
            _ => {
                tracing::warn!(
                    "[panel_variables] {name} 的值 {value} 不是可用数值 ⇒ 回落默认值（面板改了不会生效，检查取值范围）"
                );
                None
            },
        },
        None => None,
    }
}

/// [`numeric_in`] 的快照版。
pub fn numeric(name: &str) -> Option<f64> {
    numeric_in(&panel_variables(), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_map_keeps_only_named_entries_and_last_value_wins() {
        let raw = serde_json::json!([
            { "name": "a", "value": 1 },
            { "value": 99 }, // 缺 name ⇒ 丢
            { "name": "b" }, // 缺 value ⇒ 存 Null
            { "name": "a", "value": 2 },
        ]);
        let map = variables_map(&raw);
        assert_eq!(map.len(), 2, "两个具名键（重名合为一个）");
        assert_eq!(map.get("a"), Some(&serde_json::json!(2)), "同名重复以后者为准");
        assert_eq!(map.get("b"), Some(&serde_json::Value::Null), "缺 value 存 Null 而非跳过");
    }

    #[test]
    fn variables_map_tolerates_missing_table() {
        // 模板 `variables` 为 NULL（从未播种）或列里是空串 ⇒ 空表，读侧全回落，不 panic。
        assert!(variables_map(&serde_json::Value::Null).is_empty());
        assert!(variables_map(&serde_json::json!({"not": "an array"})).is_empty());
        assert!(variables_map_from_str("").is_empty());
        assert!(variables_map_from_str("not json").is_empty());
    }

    #[test]
    fn numeric_in_accepts_json_numbers_and_rejects_everything_else() {
        let vars = variables_map(&serde_json::json!([
            { "name": "int", "value": 10 },
            { "name": "float", "value": 8.5 },
            { "name": "text", "value": "10" },
            { "name": "null", "value": null },
            { "name": "bool", "value": true },
        ]));
        assert_eq!(numeric_in(&vars, "int"), Some(10.0));
        assert_eq!(numeric_in(&vars, "float"), Some(8.5));
        for key in ["text", "null", "bool", "absent"] {
            assert_eq!(numeric_in(&vars, key), None, "{key} 不是可用数值 ⇒ 必须让调用方回落");
        }
    }

    #[test]
    fn integer_in_rejects_fractional_values() {
        let vars = variables_map(&serde_json::json!([
            { "name": "pos_max_total", "value": 10 },
            { "name": "half", "value": 10.5 },
        ]));
        assert_eq!(integer_in(&vars, "pos_max_total"), Some(10));
        assert_eq!(integer_in(&vars, "half"), None, "持仓只数不可能是 10.5 ⇒ 视为不可用");
        assert_eq!(integer_in(&vars, "absent"), None);
    }

    /// 全局快照路径：装入 ⇒ 读得到（装入是**整表替换**，故本测试只在自己这一份表里做判据）。
    #[test]
    fn install_then_read_roundtrip() {
        let installed = install_panel_variables_from_str(
            &serde_json::to_string(&serde_json::json!([
                { "name": "harness_test_only_key", "value": 42 }
            ]))
            .unwrap(),
        );
        assert_eq!(installed, 1);
        assert_eq!(numeric("harness_test_only_key"), Some(42.0));
        assert_eq!(integer("harness_test_only_key"), Some(42));
        assert_eq!(panel_variables_len(), 1, "整表替换 ⇒ 旧键不得残留在快照里");
    }
}
