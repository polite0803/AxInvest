// SPDX-License-Identifier: AGPL-3.0-only

//! **P4 出口判据**：apply 编排 + 渲染覆盖。
//!
//! # 为什么需要它（并且为什么它必须能在没有 PG 的机器上跑）
//!
//! P3 的出口判据是「plan 与预期一致」，可以在**只读**层面回答。P4 的出口判据变成
//! 「apply 之后指纹 == 期望指纹」，它必须真连库、真跑编排。而 CI 上没有 PG
//! （`render` 也只实现了 PG）—— 如果出口判据只能在有 PG 的机器上跑，它就永远没被跑过。
//!
//! 所以判据被这样切开：**能自动跑的判据尽量不依赖 PG**。
//!
//! | 模式 | 用法 | 做什么 | 连库 | 写库 |
//! |---|---|---|---|---|
//! | 空 SQLite 冒烟 | `--empty-sqlite` | 空内存库 → `apply::cycle` 两遍 → 比对 | 内存 | 只写元表 |
//! | 空模型渲染覆盖 | `--render-coverage` | 空 PG **模型**（不连库）→ 全量 render | **否** | 否 |
//! | 真库只读渲染 | `--pg-render <url>` | introspect 真库 → diff → render 全部变更 | 是 | **否**（零写入） |
//! | 真库 apply | `<pg-url> [--execute]` | 真跑编排；默认 dry-run | 是 | 默认只写元表 |
//!
//! ⚠ 「真库只读渲染」是**后加的**，加它的理由值得记下来：apply 路径即使 dry-run 也会建出
//! 四张元表（那是刻意的，见 `safety` 模块文档），于是「想在真库数据上验证渲染器」就附带
//! 了一次真实写入。把这件事切成两半 —— **渲染器可以在生产库上被真实验证，而生产库连一张
//! 表都不会多** —— 靠的只是「这一路不调 `apply`，只调 `render`」。
//!
//! ⚠「空模型渲染覆盖」**替代**了原计划的「空 PG」。原计划里 `--empty-pg` 用
//! `SchemaModel::new(Postgres)` 代表空 schema —— 那对 P3 成立（只读），对 P4 不成立：
//! apply 必须连库，**没有「虚拟空库」可以执行 DDL**。把它改成「不连库、只做渲染覆盖」
//! 才是能自动化的判据。这个偏离是刻意的，写在这里以免下次有人按旧计划找 `--empty-pg`。
//!
//! # ⚠ `--execute` 是**不可逆**的
//!
//! 它要在真实库上执行 DDL。因此需要两样东西同时成立才会执行：
//! 命令行给 `--execute` **且** 环境里有非空的 `AX_SCHEMA_APPLY_CONFIRM`。
//! 少一样就停在 dry-run。默认（什么都不加）**永远是 dry-run**。
//! 而 `--pg-render` 那一路**根本不接受** `--execute` —— 它没有执行分支。
//!
//! # 退出码
//!
//! 0 = 判据全满足；1 = 判据不满足（渲染失败 / 执行了不该执行的 / 未收敛 / 被熔断）；
//! 2 = 参数或连库失败。

use std::collections::{BTreeMap, BTreeSet};

use axagent_dao::db::connect_without_initialization;
use axagent_dao::reconcile::{
    Dialect, SchemaModel, apply, evidence, expected, introspect, plan, render, safety,
};

// 全限定导入（不写 `use apply::{…}`）—— 让 `use` 的首段永远是 crate 名，
// 避免与「当前模块里恰好也叫 apply 的东西」产生解析歧义。
use axagent_dao::reconcile::apply::{ApplyOptions, ApplyRefusal, SkipReason};
use axagent_dao::reconcile::evidence::ExportOptions;
use axagent_dao::reconcile::plan::{Change, ChangeKind, ChangePayload, PlanOptions};
use axagent_dao::reconcile::render::{RenderError, Rendered};
use sea_orm::{ConnectionTrait, Database, DbBackend, DbErr, Statement};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional = args.iter().find(|a| !a.starts_with("--")).cloned();

    let result = if args.iter().any(|a| a == "--empty-sqlite") {
        empty_sqlite().await
    } else if args.iter().any(|a| a == "--render-coverage") {
        render_coverage()
    } else if args.iter().any(|a| a == "--pg-render") {
        match resolve_pg_url(positional.as_deref()) {
            Ok(Some(url)) => pg_render(&url).await,
            Ok(None) => {
                eprintln!("`--pg-render` 需要一个 pg-url（位置参数或 `{ENV_PG_URL}`）");
                std::process::exit(2);
            },
            Err(e) => {
                eprintln!("失败: {e}");
                std::process::exit(2);
            },
        }
    } else {
        // 真库 apply 分支：位置参数与 `AX_SCHEMA_PG_URL` **任一**给出目标即进入；
        // 两个都给 = `resolve_pg_url` 报歧义（fail-closed，见其文档）。
        match resolve_pg_url(positional.as_deref()) {
            Ok(Some(url)) => {
                production(
                    &url,
                    args.iter().any(|a| a == "--execute"),
                    args.iter().any(|a| a == "--export-evidence"),
                )
                .await
            },
            Ok(None) => {
                usage();
                std::process::exit(2);
            },
            Err(e) => {
                eprintln!("失败: {e}");
                std::process::exit(2);
            },
        }
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
    eprintln!("用法:");
    eprintln!("  p4_apply_probe --empty-sqlite        # 空内存库跑 apply 编排（dry-run）");
    eprintln!("  p4_apply_probe --render-coverage     # 不连库，空 PG 模型全量 render");
    eprintln!(
        "  p4_apply_probe --pg-render <pg-url>  # 真库**只读**：introspect + diff + render，零写入"
    );
    eprintln!("  p4_apply_probe <pg-url> [--execute] [--export-evidence]");
    eprintln!("        # 真库 apply；默认 dry-run，--execute 需 AX_SCHEMA_APPLY_CONFIRM");
    eprintln!("        # --export-evidence 会先把 42 条丢数据变更导出来再执行（证据产出端）");
    eprintln!();
    eprintln!("  连接串也可以走环境变量 `{ENV_PG_URL}` —— **生产轮请用这条**：");
    eprintln!("        # 位置参数会进 argv，而 `cargo run` 会把整条 argv 原样回显");
    eprintln!("        # ⇒ **口令会进日志**（2026-09-16 实测泄漏过，见 ENV_PG_URL 文档）。");
    eprintln!("        # 两个来源同时给出 ⇒ 报错退出（连接目标有歧义，不静默选一个）。");
    eprintln!(
        r#"例:   AX_SCHEMA_PG_URL="$(node scripts/pg-connect.mjs url)" cargo run -p axagent-dao --example p4_apply_probe -- --pg-render"#
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 模式 1：空 SQLite 冒烟
// ═══════════════════════════════════════════════════════════════════════════

/// ⚠ **连接目标必须在 `create_pool` 之前**校验是不是 PG。
///
/// 为什么不能在「连上之后查 backend」：`create_pool` 的分派是
/// `starts_with("sqlite:")` → `starts_with("postgres://")` → **否则一律当成 SQLite 路径**
/// （`crates/dao/src/db.rs:38-45`）。所以一个空串或手误的连接串会被它**静默当成 SQLite
/// 文件**，然后在那儿跑一遍 PRAGMA + 迁移。实测症状：
///
/// ```text
/// 失败: Custom Error: 连库失败: Database error: Query Error:
///       error returned from database: (code: 1) no such table: axagent_schema_version
/// ```
///
/// 这条错误看起来是「目标库缺表」，实为「连的根本不是目标库」。等连上再查 backend
/// 已经晚了 —— 那时迁移已经跑过了。**必须在连库之前拦住。**
///
/// （第 5 次踩「shell 变量/命令替换拿到空值且不报错」这个坑了。上一次的形式是
/// `URL=$(cmd)` 静默退化；这次是探针自己没做前置校验 —— 校验一次，两个形式都堵住。）
fn require_pg_url(url: &str) -> Result<(), DbErr> {
    if url.trim().is_empty() {
        return Err(DbErr::Custom(
            "pg-url 是**空串** —— 通常意味着 shell 变量/命令替换没取到值。\
             空串会被 `create_pool` 当作 SQLite 路径并**在那里跑迁移**，\
             所以必须在连库之前拦住（本函数就在做这件事）。"
                .into(),
        ));
    }
    if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
        // 只显示 scheme 段：连接串里可能带口令，绝不能整条进日志。
        let scheme: String = url.split(':').next().unwrap_or("").chars().take(24).collect();
        return Err(DbErr::Custom(format!(
            "本探针只接受 `postgres://` 连接串，收到的 scheme 是 `{scheme}`。\
             非 PG 串会被 `create_pool` **静默降级成 SQLite 文件**并在那里跑迁移。"
        )));
    }
    Ok(())
}

/// 目标标签（**脱敏**）：只保留最后一个 `@` 之后的部分，即 `host:port/db`。
///
/// 打出来的用处在「日志里能看到自己连的到底是哪个库」—— 上面那条 `no such table`
/// 之所以耗时间，就是因为日志里没有任何一行写过目标。
fn pg_target_label(url: &str) -> String {
    url.rsplit('@').next().unwrap_or("(?)").to_string()
}

/// 连接串的**不进 argv** 的来源。
///
/// ## 为什么必须有这么一条路（2026-09-16 真库实测挖出）
///
/// 本文件的脱敏做得很足：`pg_target_label` 只打 `host:port/db`，`require_pg_url` 的
/// 报错只回显 scheme。但**脱敏只管本进程的输出，管不了调用方式** ——
/// `cargo run -- <url>` 会把整条 argv 原样回显：
///
/// ```text
/// Running `target\debug\examples\p4_apply_probe.exe 'postgres://user:****@host:5432/db'`
/// ```
///
/// 实测泄漏点：`output/tmp-p1-dryrun-prod.log:26`（口令明文，落进日志文件）。
/// ⇒ **脱敏写在代码里、泄漏发生在传输层，两者互不相识。** 环境变量不会被回显。
///
/// ## 两个来源同时给出 ⇒ *拒绝执行*（fail-closed）
///
/// 不是因为「谁优先」难定，而是因为**连接目标存在歧义**这件事，在 `--execute` 下
/// 最不能容忍：位置参数说 A 库、环境变量说 B 库，静默选一个就可能对着错的库执行 DDL。
/// 所以这里不选，直接拒 —— 与 `require_pg_url` 拒空串是同一种处置。
pub const ENV_PG_URL: &str = "AX_SCHEMA_PG_URL";

/// 定连接串从哪来。`Ok(None)` = 两个来源都没给（调用方自己决定是 usage 还是报错）。
///
/// 优先级不是「谁赢」，而是「同时给 = 报错」：见 [`ENV_PG_URL`] 的文档。
fn resolve_pg_url(positional: Option<&str>) -> Result<Option<String>, DbErr> {
    // 空串/纯空白一律当「没给」—— shell 变量取空是高频事故（见 `require_pg_url` 注释）。
    let from_env =
        std::env::var(ENV_PG_URL).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    match (from_env, positional) {
        (Some(_), Some(p)) => Err(DbErr::Custom(format!(
            "`{ENV_PG_URL}` 与命令行位置参数**同时**给了连接串 —— 连接目标有歧义，拒绝执行。\
             请只保留一个（建议用 `{ENV_PG_URL}`：不走 argv ⇒ cargo 回显时不会泄漏口令）。\
             位置参数指向的是 `{}`。",
            pg_target_label(p)
        ))),
        (Some(e), None) => {
            println!("-- 连接串来自 `{ENV_PG_URL}`（不进 argv，不进 cargo 回显）--");
            Ok(Some(e))
        },
        (None, Some(p)) => {
            // 不是错误，但必须说清楚代价：这条路上的调用会被 cargo 连口令一起回显。
            eprintln!(
                "!! 连接串来自**命令行位置参数** ⇒ `cargo run` 会把 argv 原样回显，\
                 口令会进日志。生产轮请改用 `{ENV_PG_URL}` 传。"
            );
            Ok(Some(p.to_string()))
        },
        (None, None) => Ok(None),
    }
}

/// 冒烟用的渲染器。
///
/// 为什么不直接用 `render`：本冒烟要观察的是「**空 SQLite 库的期望模型用到了哪些
/// `ChangeKind`**」，而真渲染器对 SQLite 原生支持 6 类、其余 15 类一律
/// [`RenderError::UnsupportedDialect`] —— 用它就只能看到那 6 类，观察面被削掉一半。
///
/// ⚠ 旧版注释写的是「`render` 在 SQLite 上恒返回 `UnsupportedDialect`」，
/// 那是 P4 时期的状态，**已过期**（SQLite 适配后不是恒返回了）。留着这种与代码事实
/// 相反的注释比没有注释更糟：它会让人以为「SQLite 上什么都渲染不了」而绕过真渲染器。
///
/// ⚠ 它对**其它类别一律报错**，不返回「无害的假语句」：返回假语句会让「空 SQLite 的
/// 期望模型用到了哪些类别」这个信息被掩盖掉 —— 而那正是这个模式要观察的东西。
fn sqlite_smoke_render(c: &Change, _d: Dialect) -> Result<Rendered, RenderError> {
    match c.kind {
        // 建表：真 DDL（从 `Table` 载荷取列）
        ChangeKind::CreateTable => {
            let ChangePayload::Table(t) = &c.payload else {
                return Err(RenderError::PayloadMismatch {
                    kind: c.kind,
                    object: c.object.clone(),
                    want: "Table",
                    got: "其它",
                });
            };
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|col| {
                    format!(
                        "\"{}\" {} {}",
                        col.name,
                        col.sql_type,
                        if col.nullable { "" } else { "NOT NULL" }
                    )
                })
                .collect();
            Ok(Rendered {
                statements: vec![format!("CREATE TABLE \"{}\" ({})", c.object, cols.join(", "))],
                notes: vec![],
            })
        },
        // 建索引：真 DDL（表名与列要从 `Index` 载荷取 —— `object` 只有索引名）
        ChangeKind::CreateIndex => {
            let ChangePayload::Index { table, def } = &c.payload else {
                return Err(RenderError::PayloadMismatch {
                    kind: c.kind,
                    object: c.object.clone(),
                    want: "Index",
                    got: "其它",
                });
            };
            let cols: Vec<String> = def.cols.iter().map(|x| format!("\"{x}\"")).collect();
            let head = if def.unique {
                "CREATE UNIQUE INDEX"
            } else {
                "CREATE INDEX"
            };
            Ok(Rendered {
                statements: vec![format!(
                    "{head} \"{}\" ON \"{}\" ({})",
                    c.object,
                    table,
                    cols.join(", ")
                )],
                notes: vec![],
            })
        },
        other => Err(RenderError::Unrenderable {
            kind: other,
            object: c.object.clone(),
            why: "空 SQLite 的期望模型里本不该出现这一类（出现即说明计划变了，\
                  该给冒烟渲染器补上对应 DDL）"
                .into(),
        }),
    }
}

async fn empty_sqlite() -> Result<bool, DbErr> {
    let db = Database::connect("sqlite::memory:").await?;
    let opts = ApplyOptions { renderer: sqlite_smoke_render, ..ApplyOptions::default() };
    assert!(opts.config.dry_run, "冒烟必须默认停在 dry-run");

    let first = apply::cycle(&db, &opts).await?;
    println!("{}", first.report("P4 apply · 空 SQLite（编排冒烟，dry-run）"));

    let second = apply::cycle(&db, &opts).await?;

    // 元表确实建起来了（且是 4 张）
    let meta = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name".to_string(),
        ))
        .await?;
    let meta_names: Vec<String> =
        meta.iter().map(|r| r.try_get::<String>("", "name").unwrap()).collect();

    // apply 之后实况里**只有**元表，且它们对 introspect 不可见
    let seen = introspect::read(&db).await?;
    let visible = seen.table_names();

    // 实际用到哪些类别 —— **实测后打印**，不写死。
    // 写死就是「自证注释污染」：那句「只用了几类」不会随被测对象变化，
    // 计划一变它就静静变成假话（本仓已记录过这一类）。
    let mut kinds: Vec<&str> = first.items.iter().map(|i| i.kind.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();

    println!("-- 判据 --");
    let mut ok = true;
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("  [{}] {name} —— {detail}", if pass { "✓" } else { "✗" });
        if !pass {
            ok = false;
        }
    };

    check(
        "零渲染失败",
        first.refusal.is_none(),
        match &first.refusal {
            // 冒烟渲染器对「非这两类」一律报错 ⇒ 这里通过就等于「类别集 ⊆ 这两类」，
            // 而打印出来的是**实测**的集合（不是断言的那两句）。
            None => format!("空 SQLite 的期望模型实际只用了：{}", kinds.join(" / ")),
            Some(r) => format!("[{}] {}", r.kind_str(), r.reason()),
        },
    );
    check(
        "一条 DDL 都没执行",
        first.executed == 0,
        format!("executed = {}（dry-run）", first.executed),
    );
    check(
        "每条变更都写了审计",
        first.audit_rows == first.items.len(),
        format!("audit_rows = {}｜变更 = {}", first.audit_rows, first.items.len()),
    );
    check(
        "全部条目都被判为 dry-run",
        first.items.iter().all(|i| i.skip == Some(SkipReason::DryRun)),
        format!(
            "非 dry-run 的条目 = {}",
            first.items.iter().filter(|i| i.skip != Some(SkipReason::DryRun)).count()
        ),
    );
    check("库里没有用户表", visible.is_empty(), format!("introspect 可见表 = {visible:?}"));
    check(
        "四张元表都已建出",
        safety::META_TABLES.iter().all(|t| meta_names.contains(&(*t).to_string())),
        format!("sqlite_master = {meta_names:?}"),
    );
    check(
        "重复运行结果一致（确定性）",
        first.items == second.items,
        "两遍 cycle 的逐条结果逐字段相同".to_string(),
    );
    check(
        "dry-run 不判收敛",
        first.converged.is_none(),
        format!("converged = {:?}", first.converged),
    );

    println!(
        "  结论 = {}",
        if ok {
            "符合预期"
        } else {
            "不符合预期，见上"
        }
    );
    Ok(ok)
}

// ═══════════════════════════════════════════════════════════════════════════
// 模式 2：渲染覆盖（不连库）
// ═══════════════════════════════════════════════════════════════════════════

/// 把整份 plan 渲染一遍并报告结果。返回 `true` = 判据全满足。
///
/// **三个模式共用它**：判据口径只写一处，才不会出现「一个模式修好了、另一个还在按旧
/// 口径放行」这种漂移（口径漂移是本仓已记录过的一类缺陷）。
///
/// `empty_actual`：实况侧是否**空库**。只有空库才该满足「建表数 == 期望表数且零孤儿」；
/// 真库上孤儿是正常的（`orphans()` 非空不等于出错），所以那个判据必须能被关掉 ——
/// 否则真库模式会因为「有孤儿」而永远红，而人就会去放宽它，把渲染判据一起放宽。
fn render_report(
    p: &plan::Plan,
    dialect: Dialect,
    expected_tables: usize,
    title: &str,
    empty_actual: bool,
) -> bool {
    println!("{}", p.render(title));

    let mut per_kind: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let mut errs: Vec<String> = Vec::new();
    for c in &p.changes {
        let slot = per_kind.entry(c.kind.as_str()).or_insert((0, 0));
        match render::render(c, dialect) {
            Ok(r) if r.statements.is_empty() => {
                slot.1 += 1;
                errs.push(format!("{} {}：渲染产出 0 条语句", c.kind.as_str(), c.object));
            },
            Ok(_) => slot.0 += 1,
            Err(e) => {
                slot.1 += 1;
                errs.push(format!("{} {}：{e}", c.kind.as_str(), c.object));
            },
        }
    }

    println!("-- 逐类渲染结果（成功 / 失败）--");
    for (k, (ok_n, err_n)) in &per_kind {
        println!("  {k:<14} {ok_n:>5} / {err_n}");
    }
    let exercised: BTreeSet<&str> = per_kind.keys().copied().collect();
    let missing: Vec<&str> =
        ChangeKind::ALL.iter().map(|k| k.as_str()).filter(|s| !exercised.contains(s)).collect();
    println!(
        "-- ChangeKind 覆盖：本份 plan 用到 {} / {} 类 --",
        exercised.len(),
        ChangeKind::ALL.len()
    );
    if !missing.is_empty() {
        // ⚠ 必须把**没测到**的类别打出来。只报「用到 1 / 21 类」而不列剩下的 20 类，
        // 读的人很容易把「渲染零失败」当成「21 类都验证过了」—— 那正好相反。
        println!("   ⚠ 本份 plan 未触及的 {} 类（它们的渲染路径这次没被验证）：", missing.len());
        println!("     {}", missing.join(" / "));
    }

    let creates = p.changes.iter().filter(|c| c.kind == ChangeKind::CreateTable).count();
    let loses = p.changes.iter().filter(|c| c.loses_data).count();
    let drops = p.changes.iter().filter(|c| c.kind == ChangeKind::DropTable).count();
    let deferred_preview = p.changes.iter().filter(|c| c.kind == ChangeKind::DropTable).count();

    println!("-- 判据 --");
    println!("  期望表数          = {expected_tables}");
    println!("  CREATE TABLE 条数 = {creates}");
    println!("  孤儿候选          = {}", p.orphans().len());
    println!("  丢数据变更        = {loses}（真跑时每条都要导出证据，缺一条即整批被拒）");
    println!(
        "  DROP TABLE        = {drops}（真跑时首见只登记，共 {deferred_preview} 条会先被延迟）"
    );
    println!("  渲染失败          = {}", errs.len());
    for e in errs.iter().take(20) {
        println!("    ✗ {e}");
    }

    // 渲染失败**恒为**判据（这是 P4-2 的出口判据）；空库那两条只在 `empty_actual` 时判。
    let render_ok = errs.is_empty();
    let empty_ok = !empty_actual || (creates == expected_tables && p.orphans().is_empty());
    let ok = render_ok && empty_ok;
    println!(
        "  结论              = {}",
        match (render_ok, empty_ok) {
            (true, true) => "渲染零失败".to_string(),
            (false, _) => format!("渲染有 {} 条失败，见上", errs.len()),
            (true, false) => "渲染通过，但空库的两条判据不满足，见上".to_string(),
        }
    );
    ok
}

/// 空 PG **模型** vs 期望模型 ⇒ 计划里应当包含期望结构的全部建表动作。
///
/// 判据是「**在真实数据规模上**，render 对计划里出现的每个类别都成功」。单测用的是手搓
/// 的小模型（21 个类别各一条），这里用的是**真实**的 L1 + L2 声明 —— 后者才可能出现
/// 「某个类别的真实载荷形态让渲染器失败」这类问题。
///
/// ⚠ **本模式实测只能触及 `CreateTable` 一类**（实况侧是空模型 ⇒ 只可能产出建表；
/// 首次跑就自己报了「用到 1 / 21 类」，见 `output/tmp-p4-probe-render-cov.log`）。
/// 所以它证明的是「真实规模下的**建表**渲染没问题」，**不是**「21 类都没问题」。
/// 后者的证据来自另外两处：单测的穷举用例（集合比对 `ChangeKind::ALL`）与
/// 「真库只读渲染」（实况非空 ⇒ 才会产出 DROP / ALTER / RENAME 那些类）。
/// 把这条写在函数上，是因为「渲染零失败」这四个字极容易被读成后者。
fn render_coverage() -> Result<bool, DbErr> {
    let want = expected::build(Dialect::Postgres)?;
    let actual = SchemaModel::new(Dialect::Postgres);
    let p = plan::diff(&want, &actual, Dialect::Postgres);
    Ok(render_report(
        &p,
        Dialect::Postgres,
        want.tables.len(),
        "P4 render coverage · 空 PG（模型等价式，不连库）",
        true,
    ))
}

/// 只读地读一次白名单。**表不存在**（此库从未跑过 `apply`）⇒ 视为空。
///
/// ⚠ **不能**为了「保证表存在」去调 `safety::ensure_meta_tables` —— 那会建出 4 张表，
/// 而本模式的承诺是「生产库连一张表都不会多」（见本文件模块文档）。所以这里先
/// `to_regclass` 探一下存在性（纯 SELECT），再决定要不要读。
///
/// 为什么要有这个函数（2026-09-16 实测缺陷）：`pg_render` 此前用三参数 `plan::diff`，
/// **不读白名单** ⇒ 同一天、同一份 plan、同一个库，两条路径给出**互相矛盾**的结论：
///
/// | 路径 | 对 `opc_capability`（324 行、已人工豁免）的结论 |
/// |---|---|
/// | `<pg-url>` 生产 dry-run | 变更合计 **0** —— 白名单生效，不判孤儿 |
/// | `--pg-render` | `DROP TABLE`、`丢数据变更 2`，还打印「真跑时每条都要导出证据…」 |
///
/// 后者的那两句话是**假的**：真跑时这 2 条根本不在 plan 里，既不导证据也不延迟。
/// 只读模式的用途是「在真库数据上验证渲染」，读的人会把它当成真实计划 ⇒ 必须同口径。
async fn read_only_whitelist(
    db: &sea_orm::DatabaseConnection,
) -> Result<BTreeMap<String, String>, DbErr> {
    let probe = db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!(
                "SELECT to_regclass('{}')::text IS NOT NULL AS present",
                safety::META_WHITELIST
            ),
        ))
        .await?;
    let present = probe.and_then(|r| r.try_get::<bool>("", "present").ok()).unwrap_or(false);
    if !present {
        eprintln!(
            "!! `{}` 不存在（此库从未跑过 apply）⇒ 本次按「白名单为空」判孤儿；\
             生产库请确认这是预期的。",
            safety::META_WHITELIST
        );
        return Ok(BTreeMap::new());
    }
    Ok(safety::load_orphan_whitelist(db, safety::now_epoch()).await?.active)
}

/// 真库**只读**：introspect → diff → render 全部变更。**一次写入都没有。**
///
/// 为什么单列这个模式（而不是让大家跑 `<pg-url>` 那个 dry-run）：apply 路径即使 dry-run
/// 也会建出四张元表（那是刻意的，见 `safety` 模块文档），于是「想在真库数据上验证渲染器」
/// 就附带了一次真实写入。本模式把这件事切成两半：**渲染器可以在生产库上被真实验证，
/// 而生产库连一张表都不会多**。区别在于它不调 `apply`，只调 `render`。
async fn pg_render(url: &str) -> Result<bool, DbErr> {
    // 必须在 `create_pool` **之前**（见 `require_pg_url`）。
    require_pg_url(url)?;
    println!("-- 目标：{}（本次只发 SELECT，零写入）--", pg_target_label(url));
    let handle = connect_without_initialization(url)
        .await
        .map_err(|e| DbErr::Custom(format!("连库失败: {e}")))?;
    let db = &handle.conn;
    let dialect = apply::dialect_of(db)?;

    let actual = introspect::read(db).await?;
    let want = expected::build(dialect)?;
    // ⚠ 必须走 `diff_with` 并带上白名单：否则本模式与 `apply::cycle` 的口径不一致
    // （见 `read_only_whitelist` 的实测记录）。
    let wl = read_only_whitelist(db).await?;
    println!(
        "-- 白名单：生效 {} 条{}（与 apply 路径同口径；此前本模式不读它，结论因此矛盾）--",
        wl.len(),
        if wl.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = wl.keys().map(String::as_str).collect();
            format!("：{}", names.join("、"))
        }
    );
    let p = plan::diff_with(&want, &actual, dialect, &PlanOptions { orphan_whitelist: wl });

    // 反向断言：如果实况读出来是空库，那「在真实数据上验证过」这句话就是空的。
    // 先把这件事说清楚，再报渲染结果 —— 否则「渲染零失败」可能只是在空模型上成立。
    println!("-- 实况规模：{} 张表（introspect 只发 SELECT，全程零写入）--", actual.tables.len());
    if actual.tables.is_empty() {
        eprintln!(
            "!! 实况 0 张表 —— 目标库看起来是空的；这条模式的意义是「在真实数据上验证渲染」，请确认连的是对的库。"
        );
        return Ok(false);
    }

    Ok(render_report(
        &p,
        dialect,
        want.tables.len(),
        "P4 render coverage · 真库只读（零写入）",
        false,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// 模式 3：真库
// ═══════════════════════════════════════════════════════════════════════════

async fn production(url: &str, execute: bool, export_evidence: bool) -> Result<bool, DbErr> {
    // 必须在 `create_pool` **之前**（见 `require_pg_url`）：空串/手误串会被
    // `create_pool` 静默降级成 SQLite 文件并在那里跑迁移。
    require_pg_url(url)?;
    if execute
        && std::env::var("AX_SCHEMA_APPLY_CONFIRM").map(|v| v.trim().is_empty()).unwrap_or(true)
    {
        eprintln!("!! `--execute` 会在真实库上执行 DDL（不可逆）。");
        eprintln!("!! 确认请设 `AX_SCHEMA_APPLY_CONFIRM=1` 再跑；不加 `--execute` 则只 dry-run。");
        return Ok(false);
    }
    // ⚠ 目标标签**脱敏**（只打 `host:port/db`）—— 连接串里带明文口令，
    // 绝不能整条进日志（日志是要给人看、给报告引的）。
    if execute {
        eprintln!("!! 逃生阀已关闭：本次会**真的**在 {} 上执行 DDL。", pg_target_label(url));
    } else {
        println!("（dry-run：只渲染 + 写审计，不执行任何业务 DDL）目标 {}", pg_target_label(url));
    }

    // ⚠ 用只读连接而**不是** `create_pool`：后者是生产入口，会建表 + 跑声明式收敛。
    // 本探针的语义是「我先看库现在的样子，再由我决定动不动它」—— 若连接本身就先改了库，
    // 那么「dry-run 下 executed == 0」这条判据衡量的就不再是探针的行为，而是运气
    // （取决于 `create_pool` 当时有没有变更要做）。2026-09-16 因 `initialize_schema`
    // 落地而暴露。
    let handle = connect_without_initialization(url)
        .await
        .map_err(|e| DbErr::Custom(format!("连库失败: {e}")))?;
    let db = &handle.conn;

    // 闸配置从环境读（逃生阀默认开、配额 5、延迟、不许基数下降）。
    let mut config = safety::SafetyConfig::from_env();
    if execute {
        config.dry_run = false;
    }
    let mut opts = ApplyOptions { config, ..ApplyOptions::default() };

    // ── 证据导出（补上「导出成功才允许 DROP」缺的**产出端**） ──
    //
    // ⚠ 这里**自己算了一遍 plan**，而 `apply::cycle` 进去还会再算一遍。这是刻意的，
    // 不是省事：`cycle` 的入参 `ApplyOptions::evidence` 是「按 object 索引的证据表」，
    // 而要枚举 object 就必须先有 plan，plan 又在 cycle 内部产生 —— 想不重复就得把
    // cycle 拆开，那会破坏它内建的「白名单在 diff 之前读」顺序保证（见其文档）。
    //
    // 两遍 plan 之间若库被改动，第二遍的 object 集合会变 ⇒ `missing_evidence` 报缺证据
    // ⇒ **整批中止**，不是静默执行。也就是说这个重复是 fail-closed 的。下面还加了一条
    // 显式检查，把「导出后仍被报缺证据」这件事单独喊出来。
    if export_evidence {
        if !execute {
            eprintln!(
                "!! --export-evidence 只在 --execute 路径下有意义（dry-run 不执行，无需证据）"
            );
        } else {
            let dialect = apply::dialect_of(db)?;
            let wl = safety::load_orphan_whitelist(db, safety::now_epoch()).await?;
            let actual = introspect::read(db).await?;
            let want = expected::build(dialect)?;
            let p = plan::diff_with(
                &want,
                &actual,
                dialect,
                &PlanOptions { orphan_whitelist: wl.active.clone() },
            );
            let dir = std::env::var("AX_SCHEMA_EVIDENCE_DIR")
                .unwrap_or_else(|_| "output/schema-evidence".to_string());
            let ex = ExportOptions { dir: dir.into(), ..Default::default() };
            match evidence::export_for_plan(db, &p, &ex).await {
                Ok(rep) => {
                    println!("-- 证据导出 --");
                    println!("  目录   : {}", ex.dir.display());
                    println!("  {}", rep.summary());
                    for d in &rep.dumped {
                        println!(
                            "    {:58} {:>7} 行  {:>10} B  sha256={}…",
                            d.object,
                            d.rows,
                            d.bytes,
                            &d.sha256[..16.min(d.sha256.len())]
                        );
                    }
                    println!();
                    opts.evidence = rep.evidence;
                },
                Err(e) => {
                    eprintln!("!! 证据导出失败，**不执行任何 DDL**：{e}");
                    return Ok(false);
                },
            }
        }
    }

    let out = apply::cycle(db, &opts).await?;
    println!(
        "{}",
        out.report(if execute {
            "P4 apply · 真库（执行）"
        } else {
            "P4 apply · 真库（dry-run）"
        })
    );

    println!("-- 判据 --");
    let mut ok = out.is_clean();
    println!("  [{}] 无假墓碑 / 无中止 / 无整批拒绝", if ok { "✓" } else { "✗" });

    if execute {
        let converged = out.converged == Some(true);
        println!(
            "  [{}] 收敛（实况指纹 == 期望指纹）—— converged = {:?}",
            if converged { "✓" } else { "✗" },
            out.converged
        );
        ok &= converged;

        // ⚠ 已导出证据却仍被报「缺证据」⇒ 两遍 plan 的 object 集合不一致（导出与执行
        // 之间实况变了），或证据键名与 `Change::object` 对不上。必须显式喊出来：
        // 否则它在日志里看起来只是「又缺证据了」，而人会以为是导出器没干活 ——
        // 真正的原因（键名不匹配 / 竞态）就被这句话盖住了。
        if export_evidence && matches!(out.refusal, Some(ApplyRefusal::MissingEvidence(_))) {
            eprintln!(
                "!! 已导出证据却仍报缺证据 ⇒ 检查证据键名是否与 `Change::object` 逐字一致，\
                 以及导出与执行之间实况是否被改动。"
            );
            ok = false;
        }
    } else {
        let none_executed = out.executed == 0;
        println!(
            "  [{}] dry-run 下一条都没执行 —— executed = {}",
            if none_executed { "✓" } else { "✗" },
            out.executed
        );
        let no_render_err = out.refusal.is_none();
        println!(
            "  [{}] 渲染零失败 —— {}",
            if no_render_err { "✓" } else { "✗" },
            match &out.refusal {
                None => "无整批拒绝".to_string(),
                Some(r) => format!("[{}] {}", r.kind_str(), r.reason()),
            }
        );
        ok &= none_executed && no_render_err;
    }
    println!(
        "  结论 = {}",
        if ok {
            "符合预期"
        } else {
            "不符合预期，见上"
        }
    );
    Ok(ok)
}
