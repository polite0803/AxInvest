// SPDX-License-Identifier: AGPL-3.0-only

//! 指纹 A/B —— 让「绝大多数启动」一次哈希就短路（PLAN §四·一、§四·八）。
//!
//! ```text
//! A = 库中记录的「上次收敛时的期望指纹」   （存 _ax_schema_state.fingerprint）
//! B = 当前期望结构（实体 + L2 + engine_rev + dialect）的指纹
//! A == B  ⇒ 跳过 introspect 的深比对与全部 diff
//! ```
//!
//! ## 三个必须进指纹的字段，缺一即静默失效
//!
//! | 字段 | 缺了会怎样 |
//! |---|---|
//! | `engine_rev` | 引擎规则升级（新增一类 DDL 支持）后指纹不变 ⇒ **永不重跑**，旧库停留在旧结构 |
//! | `dialect` | PG 与 SQLite 的期望结构本就不同（GIN vs FTS5）⇒ 共用一个 A 会互相踩 |
//! | `tables[].extras` | L2 声明改了但指纹不变 ⇒ 新增的例外对象永远不被建 |
//!
//! ## 为什么必须**运行时**算，不能放 `build.rs`
//!
//! `EntityTrait` 是运行时 trait，build.rs 拿不到；在源文本层解析宏属性必然与
//! 宏的真实展开漂移（PLAN §四·一）。故指纹的输入是 [`SchemaModel`]，
//! 而它由运行时反射实体得到。

use sha2::{Digest, Sha256};

use crate::reconcile::model::SchemaModel;

/// 计算模型的指纹（SHA-256 十六进制，小写）。
///
/// **要求入参已 `finalize()`** —— 未规范化时同一结构的不同书写顺序会得到不同
/// 指纹，表现为「每次启动都重跑」。本函数开断言把这条变成硬错误。
pub fn fingerprint(model: &SchemaModel) -> String {
    debug_assert!(is_canonical(model), "指纹输入必须是已 finalize() 的模型");
    let mut h = Sha256::new();
    h.update(model.canonical_json().as_bytes());
    hex::encode(h.finalize())
}

/// 模型是否已满足规范化不变量（[`SchemaModel::finalize`] 的第三条：幂等 + 排序）。
///
/// 判据不看「是否排序过」，而是**直接验证不变量本身**：把它 `finalize()` 一遍，
/// 看序列化结果是否改变。这样即使将来 `finalize` 的排序规则变了，本函数仍正确。
pub fn is_canonical(model: &SchemaModel) -> bool {
    let before = model.canonical_json();
    let mut probe = model.clone();
    probe.finalize();
    before == probe.canonical_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::extras::Dialect;
    use crate::reconcile::model::{ColumnModel, TableModel};

    fn sample(dialect: Dialect) -> SchemaModel {
        let mut m = SchemaModel::new(dialect);
        let mut t = TableModel::new("notes");
        t.upsert_column(ColumnModel::new("id", "bigint", false));
        t.upsert_column(ColumnModel::new("title", "text", true));
        t.primary_key = vec!["id".into()];
        m.upsert_table(t);
        m.finalize();
        m
    }

    /// 同一结构、不同书写顺序 ⇒ 同一指纹（否则每次启动都误判为「变了」）。
    #[test]
    fn fingerprint_is_order_independent() {
        let mut a = SchemaModel::new(Dialect::Postgres);
        let mut t = TableModel::new("notes");
        t.upsert_column(ColumnModel::new("id", "bigint", false));
        t.upsert_column(ColumnModel::new("title", "text", true));
        a.upsert_table(t);
        a.finalize();

        let mut b = SchemaModel::new(Dialect::Postgres);
        let mut t2 = TableModel::new("notes");
        t2.upsert_column(ColumnModel::new("title", "text", true));
        t2.upsert_column(ColumnModel::new("id", "bigint", false));
        b.upsert_table(t2);
        b.finalize();

        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    /// ⚠ 方言必须进指纹：PG 与 SQLite 的期望结构不同，共用一个 A 会互相踩。
    #[test]
    fn dialect_is_part_of_fingerprint() {
        let pg = sample(Dialect::Postgres);
        let lite = sample(Dialect::Sqlite);
        assert_ne!(
            fingerprint(&pg),
            fingerprint(&lite),
            "方言不同 ⇒ 指纹必须不同（否则切方言时跳过 reconcile）"
        );
    }

    /// ⚠ `extras`（L2 认领标记）必须进指纹：L2 声明改了也要触发重跑。
    #[test]
    fn l2_extra_claims_are_part_of_fingerprint() {
        let base = sample(Dialect::Sqlite);
        let mut with_extra = base.clone();
        with_extra.table_or_insert("notes").claim_extra("fts5:notes_fts");
        with_extra.finalize();

        assert_ne!(fingerprint(&base), fingerprint(&with_extra));

        let mut other = base.clone();
        other.table_or_insert("notes").claim_extra("trigger:notes_fts_ai");
        other.finalize();
        assert_ne!(
            fingerprint(&with_extra),
            fingerprint(&other),
            "认领标记的内容变了（不只是数量）也必须换指纹"
        );
    }

    /// ⚠ `engine_rev` 进指纹 —— 引擎规则升级必须能触发重跑。
    #[test]
    fn engine_rev_is_part_of_fingerprint() {
        let mut a = sample(Dialect::Postgres);
        let fa = fingerprint(&a);
        a.engine_rev = "999";
        assert_ne!(fingerprint(&a), fa, "engine_rev 变更必须改变指纹");
        assert_ne!(fa.len(), 0);
        assert_eq!(fa.len(), 64, "SHA-256 十六进制应为 64 字符");
    }

    /// 结构真变了 ⇒ 指纹必变（否则短路会把真差异吞掉）。
    #[test]
    fn structural_change_changes_fingerprint() {
        let base = sample(Dialect::Postgres);
        let mut changed = base.clone();
        changed.table_or_insert("notes").upsert_column(ColumnModel::new(
            "created_at",
            "timestamptz",
            true,
        ));
        changed.finalize();
        assert_ne!(fingerprint(&base), fingerprint(&changed));
    }

    /// `is_canonical` 用于把「忘了 finalize」变成硬错误而非静默重跑。
    #[test]
    fn canonical_check_detects_unfinalized_model() {
        let mut m = SchemaModel::new(Dialect::Postgres);
        let mut t = TableModel::new("z_table");
        t.upsert_column(ColumnModel::new("b", "text", true));
        t.upsert_column(ColumnModel::new("a", "text", true));
        m.upsert_table(t);
        let mut t2 = TableModel::new("a_table");
        t2.upsert_column(ColumnModel::new("id", "bigint", false));
        m.upsert_table(t2);

        assert!(!is_canonical(&m), "未排序的模型不应被判为规范形态");
        m.finalize();
        assert!(is_canonical(&m));
    }

    /// 指纹稳定：同一模型重复计算必须逐字相同（否则短路永远不命中）。
    #[test]
    fn fingerprint_is_deterministic() {
        let m = sample(Dialect::Postgres);
        assert_eq!(fingerprint(&m), fingerprint(&m));
        assert_eq!(fingerprint(&m.clone()), fingerprint(&m));
    }
}
