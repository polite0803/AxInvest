// SPDX-License-Identifier: AGPL-3.0-only

//! **P3 只读出口判据**：算出 reconcile plan 并打印，**不生成也不执行任何 DDL**。
//!
//! # 为什么需要它
//!
//! P3 的出口判据是「在**空库** + **生产库副本**上，plan 与预期一致」。这条判据无法
//! 用单测回答：单测只能证明 `diff` 这个纯函数在构造模型上的行为，而「plan 与预期
//! 一致」说的是**真实库的结构**与 L1+L2 声明的差集能被逐条归因。
//!
//! # 三种模式
//!
//! | 模式 | 用法 | 做什么 | 是否写库 |
//! |---|---|---|---|
//! | 生产只读 | `p3_plan_probe <pg-url>` | introspect 真库 → diff → 打印 | **否**（只有 SELECT） |
//! | 空 PG | `p3_plan_probe --empty-pg` | 空 `SchemaModel(Postgres)` vs 期望 → diff | 否（**不连库**） |
//! | 空 SQLite | `p3_plan_probe --empty-sqlite` | `sqlite::memory:` 真连空库 → introspect → diff | 否（内存库） |
//!
//! 为什么「空 PG」不连库：`introspect` 在**空 schema** 上返回的就是一个空
//! `SchemaModel`（表集为空、无列无索引）。这个等价关系不是假设 —— `--empty-sqlite`
//! 会在同一个二进制里**真连一张空库**并断言 `introspect` 返回 0 张表。
//! 两者一起构成「空库」的完整证据：SQLite 侧是**实测**，PG 侧由实测撑起的等价式。
//!
//! # 只读的自我证明
//!
//! ⚠ **本段已随 P4 订正（2026-09-16）**。原文写的是「`reconcile` 在 P3 的模块集合里
//! 没有 render 模块，也就是说这份二进制在结构上没有能力把 plan 变成 DDL 文本」——
//! P4-2 给 `reconcile` 加了 `render` 模块之后，这句话**变成了假话**。
//!
//! 本二进制仍然不能写库，但那不再靠「某个模块不存在」保证，而是靠**自身不引用任何
//! 渲染/执行入口**：`reconcile::mod::tests::read_only_probe_cannot_execute_or_render`
//! 会剥掉本文件的注释后扫 `execute_unprepared(` / `reconcile::render` / `safety::`
//! 这些禁词，出现即红。
//!
//! 这同时是一条**方法教训**：把「只读」的证明写成「相邻模块不存在」，会随相邻阶段的
//! 推进**静默失真**（判据 K 组「自证注释污染」—— 注释还在，但它描述的世界已经没了）。
//! 可靠形态是「本文件不出现这些调用」，因为它只依赖本文件自己。
//!
//! # 退出码
//!
//! 0 = 正常；1 = 自身不变量被违反（空库实况非空、CREATE 数 ≠ 期望表数）；2 = 参数/连库失败。

use axagent_dao::db::connect_without_initialization;
use axagent_dao::reconcile::{Dialect, expected, introspect, plan};

use sea_orm::{Database, DbBackend, DbErr};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let result = if args.iter().any(|a| a == "--empty-pg") {
        empty_pg()
    } else if args.iter().any(|a| a == "--empty-sqlite") {
        empty_sqlite().await
    } else if let Some(url) = args.first() {
        production(url).await
    } else {
        usage();
        std::process::exit(2);
    };

    match result {
        Ok(true) => {},
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("失败: {e}");
            std::process::exit(2);
        },
    }
}

fn usage() {
    eprintln!("用法: p3_plan_probe <pg-url> | --empty-pg | --empty-sqlite");
    eprintln!(
        r#"例:   cargo run -p axagent-dao --example p3_plan_probe -- "$(node scripts/pg-connect.mjs url)""#
    );
}

/// 生产库：只读 plan。
///
/// ⚠ 用 [`connect_without_initialization`] 而**不是** `create_pool`：后者是生产入口，
/// 它会建表 + 跑声明式收敛 —— 本模式承诺「只有 SELECT」，借它取连接就等于在承诺的同时
/// 改了库（2026-09-16 因 `initialize_schema` 落地而暴露）。
async fn production(url: &str) -> Result<bool, DbErr> {
    let handle = connect_without_initialization(url)
        .await
        .map_err(|e| DbErr::Custom(format!("连库失败: {e}")))?;
    let db = &handle.conn;
    let dialect = match db.get_database_backend() {
        DbBackend::Postgres => Dialect::Postgres,
        DbBackend::Sqlite => Dialect::Sqlite,
        other => return Err(DbErr::Custom(format!("不支持 backend {other:?}"))),
    };

    let actual = introspect::read(db).await?;
    let want = expected::build(dialect)?;
    let p = plan::diff(&want, &actual, dialect);
    println!("{}", p.render("P3 plan · 生产库只读（无任何 DDL）"));

    println!("-- 表级对账 --");
    let a: std::collections::BTreeSet<&str> =
        actual.tables.iter().map(|t| t.name.as_str()).collect();
    let b: std::collections::BTreeSet<&str> = want.tables.iter().map(|t| t.name.as_str()).collect();
    println!(
        "  实况 {}｜期望 {}｜交集 {}｜仅实况 {}｜仅期望 {}",
        a.len(),
        b.len(),
        a.intersection(&b).count(),
        a.difference(&b).count(),
        b.difference(&a).count()
    );

    println!("\n-- 仅实况（库有表、L1+L2 无声明）--");
    // ⚠ 原文写的是「⇒ 将被 DROP」，**那句话不准**：本段是纯集合差（`a.difference(&b)`），
    // 而真实删除集还要过一层孤儿豁免（L2 静态规则 / 非主库实体 / 运行期白名单 / **L2 虚表**）。
    // 实测（2026-09-17，生产库副本）：本段列 14 张，而 `plan.orphans()` 只有 13 条 ——
    // 差的 `axagent_schema_version` 是豁免表；给 L2 虚表补上豁免之后本段仍列 14 张，
    // 真实删除集降到 9 条。⇒ 「差额清单」与「将被 DROP」是两个口径，本段只提供前者。
    println!(
        "   （差额清单 —— 不等于「都将被 DROP」；豁免会让其中一部分保留，真实删除集见上方「孤儿候选」段）"
    );
    for n in a.difference(&b) {
        println!("  {n}  （{} 列）", actual.table(n).map_or(0, |t| t.columns.len()));
    }
    println!("\n-- 仅期望（L1+L2 有声明、库中无表 ⇒ 将新建）--");
    for n in b.difference(&a) {
        println!("  {n}");
    }
    Ok(true)
}

/// 空 PG：不连库，用空模型代表空 schema。纯计算。
fn empty_pg() -> Result<bool, DbErr> {
    let want = expected::build(Dialect::Postgres)?;
    let actual = empty_actual(Dialect::Postgres);
    report_empty(&want, &actual, Dialect::Postgres, "P3 plan · 空 PG（模型等价式）")
}

/// 空 SQLite：真连一张内存空库并 introspect —— 实证「空库 ⇒ introspect 返回 0 表」。
async fn empty_sqlite() -> Result<bool, DbErr> {
    let db = Database::connect("sqlite::memory:").await?;
    let actual = introspect::read(&db).await?;
    if !actual.is_empty() {
        eprintln!(
            "!! 空 SQLite 库竟读出 {} 张表 —— 「空库 ⇒ 空模型」的前提不成立",
            actual.tables.len()
        );
        return Ok(false);
    }
    println!("-- 前提实证：真连 sqlite::memory: introspect 返回 {} 张表 --", actual.tables.len());
    let want = expected::build(Dialect::Sqlite)?;
    report_empty(&want, &actual, Dialect::Sqlite, "P3 plan · 空 SQLite（真实空库）")
}

/// 空库的实况模型。
///
/// ⚠ 不是「假装」：`introspect` 在一张不存在任何用户表的 schema 上返回的正是这个值
/// （无表、无列、无索引、无约束）。`--empty-sqlite` 用真实空库验证了这一点。
fn empty_actual(dialect: Dialect) -> axagent_dao::reconcile::SchemaModel {
    axagent_dao::reconcile::SchemaModel::new(dialect)
}

/// 空库出口判据：新建侧覆盖全部期望表，孤儿为空集。
fn report_empty(
    want: &axagent_dao::reconcile::SchemaModel,
    actual: &axagent_dao::reconcile::SchemaModel,
    dialect: Dialect,
    title: &str,
) -> Result<bool, DbErr> {
    let p = plan::diff(want, actual, dialect);
    println!("{}", p.render(title));

    let creates = p.changes.iter().filter(|c| c.kind == plan::ChangeKind::CreateTable).count();
    let orphans = p.orphans().len();
    println!("-- 判据 --");
    println!("  期望表数            = {}", want.tables.len());
    println!("  CREATE TABLE 条数   = {creates}");
    println!("  孤儿候选            = {orphans}");

    let ok = creates == want.tables.len() && orphans == 0 && p.n_destructive == 0;
    println!(
        "  结论                = {}",
        if ok {
            "符合预期（空库 = 全量建表，零孤儿，零破坏性）"
        } else {
            "不符合预期，见上"
        }
    );
    Ok(ok)
}
