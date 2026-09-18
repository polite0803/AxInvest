// SPDX-License-Identifier: AGPL-3.0-only

//! **P5 出口判据**：声明式引擎能不能**独立**承担建表 —— 两个方言都要能。
//!
//! # 为什么需要它
//!
//! 「删掉全部版本化迁移文件」只有一个硬前提：新机制能独立建出全部表。而这件事
//! **不能靠读源码断言** —— `expected::build` 产出的模型是声明，`render` 产出的是文本，
//! 真正要证明的是「这些文本执行完之后，`introspect` 读回来的东西 == 声明」。
//!
//! # ⚠ 方言不由本探针决定
//!
//! 连接由用户在设置里选（`sqlite` 或 `postgresql`）。因此每条判据都必须**两个方言都跑
//! 一遍**，而不是在 SQLite 上证完就当证过了：
//!
//! | 模式 | 用法 | 连的库 | 写库 |
//! |---|---|---|---|
//! | 全新库 · SQLite | `--fresh-sqlite` | `sqlite::memory:`（空内存库） | 是（建表） |
//! | 全新库 · PG | `--fresh-pg <url>` | **必须空库**（非空即拒绝） | 是（建表） |
//! | 存量库 diff | `--diff <url>` | 任意（方言由连接决定） | **否**（只发 SELECT） |
//! | **已存在的 SQLite 库** | `--takeover <url> [--want-index=<名>]` | 任意**真实存在**的 SQLite 文件 | 是（**跑生产启动路径**；需 `AX_SCHEMA_APPLY_CONFIRM`） |
//!
//! `--fresh-*` 是**正面证明**：不经任何迁移，只靠 `apply::bootstrap_schema` 把库建到
//! 声明形态，并证明收敛（二次 diff 为空）。`--diff` 是**风险测量**：在真实存量库上算差集，
//! 报出哪些条目会被 bootstrap **跳过**（= 本方言没有原生 DDL 的那几类，该库因此永不收敛）。
//!
//! `--takeover` 是**第三极**（2026-09-17 加）：前两者一个只测空库、一个只发 SELECT，
//! 于是「**用户机器上那个已经存在的库**会不会被接管成功」长期无人测。它跑的是
//! **生产启动路径** `db::create_pool`（PRAGMA → 完整性自愈 → 引擎收敛 → 种子），
//! 而不是直接调 `apply` —— 差的那一层正是「启动方能不能起来」。
//! ⚠ 它**真执行 DDL**，且**只接 `sqlite:`**：PG 侧已有 `p4_apply_probe <url> --execute`，
//! 这里拒绝 PG 是为了让「URL 拿错」不可能发生。正确用法是**先拷副本、对副本跑**。
//!
//! ⚠ 「存量库有方言缺口」**不再**意味着启动会被拒 —— 2026-09-16 改判，见
//! `apply::SkipReason::UnsupportedDialect`。这个数字现在是「还剩多少收敛不了」的量化，
//! 不是「装不上」的预警。
//!
//! # ⚠ 已退休的模式：`--legacy-sqlite`（2026-09-17 删除）
//!
//! 它做的是「用迁移建库 → 让引擎接管」。删它的依据是**实测**，不是它自己文档里的
//! 那句「使命结束，应随之退休」—— 实测结论恰好**否定**了「它已退化成 `--fresh-sqlite`
//! 的等价品」这个说法：
//!
//! | | `--fresh-sqlite` | `--legacy-sqlite`（删前） |
//! |---|---|---|
//! | 连的库 | `sqlite::memory:` | **文件库**（`%TEMP%/p5-legacy-*.db`） |
//! | 跑迁移 | **不跑**（刻意） | 跑 `ddl::run_initialization` |
//! | 起始实况 | 0 表 | **1 表**（`axagent_schema_version`） |
//!
//! ⇒ 两者是**不同形态**，删前它的 8 条判据仍全绿（`LEGACY_EXIT=0`）。真正让它该退休的是
//! **覆盖面重叠**：它测的「文件库 + 一个既存元表」已被 `--takeover` 完全包含，而后者的库
//! 就是真实文件库、走的还是**生产启动路径**（`db::create_pool`），比它只调 `apply` 更外层。
//!
//! **自给自足的替代用法**（`mode=rwc` 会自动建库，故不需要先备一份）：
//!
//! ```text
//! AX_SCHEMA_APPLY_CONFIRM=1 p5_engine_takeover --takeover "sqlite:<不存在的路径>?mode=rwc"
//! ```
//!
//! # ⚠ `--fresh-pg` 会写库，且拒绝在非空库上跑
//!
//! 它执行真 DDL。安全措施是「非空即拒」：只要 `introspect` 读出**任何一张**表就退出，
//! 所以它只可能作用在一个全新的（或专门准备的空）库上。**不要**拿生产库试。
//!
//! # 退出码
//!
//! 0 = 判据全满足；1 = 判据不满足；2 = 参数 / 连库 / 前置条件失败。

use std::collections::BTreeMap;

use axagent_dao::db::{connect_without_initialization, create_pool};
use axagent_dao::reconcile::plan::{self, PlanOptions};
use axagent_dao::reconcile::{Dialect, RenderError, apply, expected, introspect, render};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement};

/// 连接串的来源（**不进 argv**，避免 `cargo run` 把口令回显到日志）。
const ENV_DB_URL: &str = "AX_SCHEMA_DB_URL";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Option<String> = args.iter().find(|a| !a.starts_with("--")).cloned();
    let url = positional
        .or_else(|| std::env::var(ENV_DB_URL).ok().map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty());

    let mut ok = true;
    let mut ran_any = false;

    if args.is_empty() || args.iter().any(|a| a == "--fresh-sqlite") {
        ran_any = true;
        let db = match Database::connect("sqlite::memory:").await {
            Ok(d) => d,
            Err(e) => exit2(format!("连内存库失败: {e}")),
        };
        ok &= or_exit2(fresh_and_verify(&db, Dialect::Sqlite, "全新 SQLite（内存库）").await);
    }

    if args.iter().any(|a| a == "--fresh-pg") {
        ran_any = true;
        let Some(url) = url.as_deref() else {
            exit2("`--fresh-pg` 需要一个连接串（位置参数或环境变量），见 usage");
        };
        let db = match connect_without_initialization(url).await {
            Ok(h) => h.conn,
            Err(e) => exit2(format!("连库失败: {e}")),
        };
        ok &= or_exit2(fresh_and_verify(&db, Dialect::Postgres, "全新 PG").await);
    }

    if args.iter().any(|a| a == "--diff") {
        ran_any = true;
        let Some(url) = url.as_deref() else {
            exit2("`--diff` 需要一个连接串（位置参数或环境变量），见 usage");
        };
        // ⚠ 只读连接：`create_pool` 会建表 + 收敛，用它取连接就等于在声称「只发 SELECT」
        // 的同时改了库。
        let db = match connect_without_initialization(url).await {
            Ok(h) => h.conn,
            Err(e) => exit2(format!("连库失败: {e}")),
        };
        ok &= or_exit2(diff_report(&db, &redact(url)).await);
    }

    if args.iter().any(|a| a == "--takeover") {
        ran_any = true;
        let Some(url) = url.as_deref() else {
            exit2("`--takeover` 需要一个连接串（位置参数或环境变量），见 usage");
        };
        // `--want-index=<名>` 可选：给了就把它当成「本轮必须建出的那个对象」逐字取证。
        // ⚠ 写成**具名**入口而不是「跑完自己看」：聚合类读数（表/索引计数）无法回答
        // 「这条索引到底建没建出来」，而它恰恰是本次要证的那件事。
        let want = args.iter().find_map(|a| a.strip_prefix("--want-index=")).map(str::to_string);
        ok &= or_exit2(takeover_existing(url, want.as_deref()).await);
    }

    if !ran_any {
        usage();
        std::process::exit(2);
    }
    if !ok {
        std::process::exit(1);
    }
}

/// 参数 / 前置条件失败 ⇒ 退出码 2（与判据不满足的 1 区分开）。
///
/// 参数写成 `impl Display` 而不是 `&str`：这样字符串字面量与 `DbErr` 都能直接传进来。
fn exit2(msg: impl std::fmt::Display) -> ! {
    eprintln!("失败: {msg}");
    std::process::exit(2);
}

/// 把「连库/构建失败」与「判据不满足」分成两种退出码。
///
/// ⚠ 不能写成 `.unwrap_or_else(exit2)`：`unwrap_or_else` 要 `FnOnce(E) -> bool`，而
/// `fn(..) -> !` **不是** `fn(..) -> bool` 的子类型（`!` 到 `bool` 的强制发生在**表达式**位置，
/// 不发生在函数项类型上）。用 `match` 让那条强制回到表达式位置。
fn or_exit2(r: Result<bool, DbErr>) -> bool {
    match r {
        Ok(v) => v,
        Err(e) => exit2(e),
    }
}

fn usage() {
    eprintln!("用法:");
    eprintln!("  p5_engine_takeover --fresh-sqlite         # 空内存库：只靠引擎建出全部表并验收敛");
    eprintln!(
        "  p5_engine_takeover --fresh-pg <url>       # ⚠ 会写库；目标**必须为空库**，非空即拒"
    );
    eprintln!(
        "  p5_engine_takeover --diff <url>           # 只读：真实存量库的差集 + 本方言表达不了的条目"
    );
    eprintln!(
        "  p5_engine_takeover --takeover <sqlite-url> [--want-index=<名>]   # ⚠ 真执行 DDL；跑**生产启动路径**"
    );
    eprintln!();
    eprintln!(
        "  ⚠ 原 `--legacy-sqlite`（迁移建库 → 引擎接管）已于 2026-09-17 **退休**。\
         删它的依据是实测：`migrations::MIGRATIONS` 清空后它建出的库只剩 `axagent_schema_version` \
         一张元表，与用户机器上的存量库形态相差太远，而它的覆盖面已被 `--takeover` 完全包含。\
         **自给自足的替代**：`--takeover sqlite:<不存在的路径>?mode=rwc`（会自动建库）。"
    );
    eprintln!(
        "  `--takeover` 只接 `sqlite:` 且要求环境变量 `AX_SCHEMA_APPLY_CONFIRM` 非空；\
         PG 请用 `p4_apply_probe <url> --execute`。**先拷副本再跑**。"
    );
    eprintln!("  p5_engine_takeover                        # 等价于 --fresh-sqlite");
    eprintln!();
    eprintln!("  连接串也可以走环境变量 `{ENV_DB_URL}`（**不进 argv**，口令不会被 cargo 回显）。");
    eprintln!("  方言不由本探针决定 —— 由连接串决定（`sqlite:…` / `postgres://…`）。");
}

/// 连接串**脱敏**：只保留 `@` 之后的部分（`host:port/db`）。日志是要给人看、给报告引的。
fn redact(url: &str) -> String {
    url.rsplit('@').next().unwrap_or("(?)").to_string()
}

// ═══════════════════════════════════════════════════════════════════════════
// 模式 1/2：全新库 —— 「引擎能独立建表」的正面证明
// ═══════════════════════════════════════════════════════════════════════════

/// 在**已知为空**的库上只靠引擎建表，并逐条验证。
///
/// ⚠ 全程**不碰** `run_migrations`：这正是要证明的事 —— 去掉迁移之后引擎还能不能建出
/// 全部表。若这里先跑了迁移，然后再 diff，那份「零残余」就是迁移的功劳，与引擎无关。
async fn fresh_and_verify(
    db: &DatabaseConnection,
    dialect: Dialect,
    label: &str,
) -> Result<bool, DbErr> {
    println!("═══ {label}（只用声明式引擎，不跑任何迁移）═══");
    let want = expected::build(dialect)?;

    // 前置：库必须是空的 —— 否则「建出全部表」分不清是引擎建的还是本来就有。
    let before = introspect::read(db).await?;
    if !before.tables.is_empty() {
        eprintln!(
            "!! 目标库已有 {} 张表 —— 本模式要求空库（它执行真 DDL）。请换一个空库/空 schema。",
            before.tables.len()
        );
        return Ok(false);
    }

    let out = apply::bootstrap_schema(db).await?;
    println!("{}", out.report(&format!("P5 · {label}")));

    let mut ok = true;
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("  [{}] {name} —— {detail}", if pass { "✓" } else { "✗" });
        if !pass {
            ok = false;
        }
    };

    check(
        "未被拒（refusal 为空）",
        out.refusal.is_none(),
        match &out.refusal {
            None => "无整批拒绝".to_string(),
            Some(r) => format!("[{}] {}", r.kind_str(), r.reason()),
        },
    );
    check("真的执行了 DDL", out.executed > 0, format!("executed = {}", out.executed));
    // ⚠ 这个数字在**全新库**上必须为 0：空库建表只用得到 `CreateTable` / `CreateIndex`，
    // 两者都在 `render::sqlite_supports_natively` 的白名单里。非空即说明白名单漏了一类
    // ⇒ 空库会静默少建，而上面「期望表逐张存在」也可能因为只漏了少数形态而蒙混过关。
    // （同一个数字在**存量库**上含义相反 —— 那里非空是已知缺口，不是失败。见 `--diff`。）
    let gaps = out.unsupported();
    check(
        "无「本方言无原生 DDL」条目",
        gaps.is_empty(),
        if gaps.is_empty() {
            "零缺口".to_string()
        } else {
            format!(
                "{} 条：{:?}",
                gaps.len(),
                gaps.iter()
                    .take(8)
                    .map(|i| format!("{} {}", i.kind.as_str(), i.object))
                    .collect::<Vec<_>>()
            )
        },
    );

    let after = introspect::read(db).await?;
    // ⚠ 逐表核而不是只核总数：总数对得上仍可能「多建几张、少建几张」互相抵消。
    let missing: Vec<&str> =
        want.tables.iter().map(|t| t.name.as_str()).filter(|n| after.table(n).is_none()).collect();
    check(
        "期望表逐张存在（不是只核总数）",
        missing.is_empty(),
        format!(
            "期望 {} 张 / 实况 {} 张 / 缺 {} 张{}",
            want.tables.len(),
            after.tables.len(),
            missing.len(),
            if missing.is_empty() {
                String::new()
            } else {
                format!("：{:?}", missing.iter().take(10).collect::<Vec<_>>())
            }
        ),
    );

    // 二次 diff = 收敛判据。语义是「要把实况变成期望还差什么」⇒ 空 plan 即收敛。
    // **这是唯一能证明「建出来的形态 == 声明形态」的判据**：「语句执行成功」只证明语法对，
    // 不证明 introspect 读回来的东西与声明一致（类型归一 / 主键形态 / 约束名 / 表达式文本
    // 任何一处对不上都会在这里现形 —— 而它们在语法层面全是合法的）。
    //
    // ⚠ 按「是否纯新增」分栏看：bootstrap 用 `additive_only` 计划，收窄掉的**非纯新增**
    // 条目根本不会被渲染，因此也不会让它被拒 ⇒ 那部分残余是**设计内**的（启动路径不碰
    // 收缩类变更）。只有「纯新增残余」才说明 bootstrap 该做而没做成。
    let p2 = plan::diff_with(&want, &after, dialect, &PlanOptions::default());
    let (additive_left, non_additive) = split_residual(&p2);
    check(
        "纯新增类别已收敛",
        additive_left.is_empty(),
        if additive_left.is_empty() {
            "零残余".to_string()
        } else {
            format!("残余 {additive_left:?}")
        },
    );
    if !non_additive.is_empty() {
        println!("  :: 非纯新增残余 {non_additive:?}（启动路径刻意不碰，属设计内，非失败）");
    }
    println!("  :: 孤儿候选 = {}", p2.orphans().len());
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
// 模式 3：存量库 diff —— 「升级会不会被拒」的风险测量
// ═══════════════════════════════════════════════════════════════════════════

/// 只读地报出真实库的差集，并高亮会让启动期 bootstrap 中止的那批条目。
///
/// 判据取自**渲染器本身**（调 `render::render(c, dialect)` 看是不是
/// `UnsupportedDialect`），不另抄一份「哪个方言支持哪几类」的清单 —— 抄一份就是给漂移
/// 留位置（清单变了，抄的那份不会红）。
///
/// 本模式**不做通过/失败判定**：存量库有漂移是事实，不是缺陷。它给的是决策用的数字。
async fn diff_report(db: &DatabaseConnection, target: &str) -> Result<bool, DbErr> {
    let dialect = apply::dialect_of(db)?;
    println!("═══ 存量库 diff · 目标 {target} · 方言 {dialect:?}（只发 SELECT）═══");

    let actual = introspect::read(db).await?;
    let want = expected::build(dialect)?;
    if actual.tables.is_empty() {
        eprintln!("!! 实况 0 张表 —— 目标库看起来是空的；本模式的意义是在**真实数据**上量差集。");
        return Ok(false);
    }
    let p = plan::diff_with(&want, &actual, dialect, &PlanOptions::default());
    println!("  实况表数 = {}｜期望表数 = {}", actual.tables.len(), want.tables.len());
    println!("  计划变更 = {}｜孤儿候选 = {}", p.changes.len(), p.orphans().len());

    // (总数, 属纯新增, 其中会被 bootstrap 拒绝)
    let mut per_kind: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    let mut aborting: Vec<String> = Vec::new();
    for c in &p.changes {
        let slot = per_kind.entry(c.kind.as_str()).or_insert((0, 0, 0));
        slot.0 += 1;
        if !c.kind.is_purely_additive() {
            continue;
        }
        slot.1 += 1;
        if matches!(render::render(c, dialect), Err(RenderError::UnsupportedDialect { .. })) {
            slot.2 += 1;
            if aborting.len() < 15 {
                aborting.push(format!("{} {}", c.kind.as_str(), c.object));
            }
        }
    }
    println!("  ── 逐类（总数 / 属纯新增 / 其中 bootstrap 会拒）──");
    for (k, (total, add_n, abort_n)) in &per_kind {
        println!("     {k:<18} {total:>4} / {add_n:>6} / {abort_n:>4}");
    }
    if aborting.is_empty() {
        println!("  ✓ 无「纯新增 + 该方言无原生 DDL」的条目 ⇒ 该库会被完整收敛到声明形态");
    } else {
        // ⚠ 2026-09-16 改判后这里的结论变了：这些条目**不再**让 bootstrap 整批拒绝、
        // 不再中止启动。它们被记成 `SkipReason::UnsupportedDialect` —— 只跳过本条，
        // 其余照跑。代价是该库在这几条上永不收敛（直到实现重建表流程，PLAN §3.1）。
        // 旧文案写的「整批拒绝、启动中止」已过期，留着会误导人去做一次不必要的数据重建。
        println!(
            "  ⚠ 有 {} 条「纯新增 + 该方言无原生 DDL」⇒ bootstrap 会**跳过**它们（不阻断本轮、\
             不阻断启动），该库在这几条上**永不收敛**。要真正收敛需走重建表流程（PLAN §3.1）：",
            aborting.len()
        );
        for a in &aborting {
            println!("      {a}");
        }
    }
    Ok(true)
}

// ═══════════════════════════════════════════════════════════════════════════
// 模式 4：接管**已存在**的库（生产启动路径真执行）
//   ⚠ 原编号为「模式 5」。原「模式 4：存量库路径」（`--legacy-sqlite`）已于 2026-09-17
//     退休删除（理由见模块文档的「已退休的模式」段），这里接续编号以免留下空号。
// ═══════════════════════════════════════════════════════════════════════════

/// 对一个**真实存在**的 SQLite 库跑生产启动路径（`db::create_pool`），并取证「结构到底改了没改」。
///
/// ## 为什么 `--diff` 取代不了它
///
/// - `--diff` **只读**：它报「计划里有 N 条 `CREATE INDEX`」，但**不证明这些语句在真实库上
///   执行得成功** —— 「声明 → 渲染 → 执行 → 回读」最后一跳它根本没走。而历史遗留库上
///   恰恰是最后一跳会炸（唯一约束撞同名多行）。
/// - `--fresh-*` 连的是**空库 / 内存库**，`--diff` 又不写 ⇒ 「**用户机器上那个已经存在的库**
///   被接管时会不会出事」在 2026-09-17 之前**无人测**。
/// - （原先还有一极 `--legacy-sqlite`，它建自己的临时库；已于 2026-09-17 退休，理由见模块
///   文档。本模式连的库就是**真实文件库**，是它的严格上位。）
///
/// ## 两条闸（它真执行 DDL）
///
/// 1. **只接 `sqlite:`** —— PG 侧已有 `p4_apply_probe <url> --execute`。这里拒绝 PG 不是能力问题，
///    而是让「URL 拿错」**不可能发生**：一个探针不该有能力顺手改生产的 PG 库。
/// 2. **`AX_SCHEMA_APPLY_CONFIRM` 非空** —— 与 P4 同款约定，挡掉「手滑跑一下」。
///
/// ⚠ 正确用法是**先 `copyFileSync` 出副本、对副本跑**；本探针不替你挡这件事。
///
/// ## 四条判据
///
/// 1. **启动不中止**：`create_pool` 返回 `Ok`。这是「用户能不能起来」，比 `apply` 的返回值更外层。
/// 2. **结构真被改**：前后的 `sqlite_master` 计数有可归因增量 —— 否则「没报错」可能只是
///    「什么都没干」。
///
///    ⚠ `executed` 是**计数**：中止与完成会给它**同一个值**。2026-09-16 那个探针的第一版
///    就栽在这里 —— 一次 fail-stop 中止让 `executed` 停在非零 ⇒「有真执行」判真；第二遍又因
///    **同一颗雷在第一条就炸** 而 `executed == 0` ⇒「幂等」也判真。两条判据全绿，而库其实
///    一条新表都没建起来，探针却打了「符合预期」。教训：守「跑完了」必须直接断言**中止字段
///    本身**，而不是它的一个可能重合的派生量（判据要锚定被测对象自身形态）。
/// 3. **目标对象真建出且** DDL **原文可读**：`--want-index` 给的那个索引必须出现在
///    `sqlite_master`，并把它的 `sql` **原样打印** —— 「对象存在」是最弱的一档，
///    谓词丢了会退化成**整表唯一**（那是比缺索引更糟的形态，症状是插入失败）。
/// 4. **幂等**：第二次 `create_pool` 不再产生新的结构计数增量。
///
/// ⚠ 判据 1、2 必须**分开**断言：`create_pool` 返回 `Ok` 而结构零增量，正是「静默空转」的形态。
async fn takeover_existing(url: &str, want_index: Option<&str>) -> Result<bool, DbErr> {
    // 判据 1 的观测手段：执行期失败走的是 `db::initialize_schema` 里的 `warn!`。
    // 不接 tracing ⇒ 那条 WARN 无处落地 ⇒ 「失败可见」这条判据**验不成**。
    // （本仓 `memory_reflow_probe` 第一次跑就踩到同一个坑：注入失败后 stderr 一片空白。）
    // 默认过滤 `axagent_dao=warn`，用 `RUST_LOG` 可覆盖。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("axagent_dao=warn")),
        )
        .try_init();

    println!("═══ 存量库接管（生产启动路径 create_pool 真执行）═══");
    println!("  目标 = {}", redact(url));

    // ── 闸 1/2 ──
    if !url.starts_with("sqlite:") {
        exit2(format!(
            "`--takeover` 只接受 `sqlite:` 连接串，收到 `{}`。\
             PG 请用 `p4_apply_probe <url> --execute` —— 本探针不接受 PG 是刻意的\
             （少一条路能改生产库，就少一次拿错 URL 的事故）。",
            redact(url)
        ));
    }
    let confirmed =
        std::env::var("AX_SCHEMA_APPLY_CONFIRM").map(|v| !v.trim().is_empty()).unwrap_or(false);
    if !confirmed {
        exit2(
            "`--takeover` 会**真执行 DDL**。请在环境里给出非空的 `AX_SCHEMA_APPLY_CONFIRM` \
             （任意值即可，与 P4 同款约定）；并确认目标是**副本**而不是原库。",
        );
    }

    let mut ok = true;
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("  [{}] {name} —— {detail}", if pass { "✓" } else { "✗" });
        if !pass {
            ok = false;
        }
    };

    // ── 前态 ──
    let before = sqlite_peek(url, want_index).await;
    println!(
        "  前态：表 {} / 索引 {}{}",
        before.tables,
        before.indexes,
        match &before.index_ddl {
            Some(s) => format!("；`{}` 已存在：{s}", want_index.unwrap_or("-")),
            None if want_index.is_some() => {
                format!("；`{}` **不存在**", want_index.unwrap_or("-"))
            },
            None => String::new(),
        }
    );
    if !before.ok {
        println!("  ⚠ 前态读数不完整：{}", before.note);
    }

    // ── 真执行：生产启动路径 ──
    // 直连 URL（不再走 `resolve_db_url_from_path`）：本探针要的正是「用户配的那个 URL 原样」，
    // 而 `create_pool` 内部会再解析一次，两条路的口径由此对齐。
    let t0 = std::time::Instant::now();
    let started = match create_pool(url).await {
        Ok(h) => {
            let _ = h.conn.close().await;
            Ok(())
        },
        Err(e) => Err(e.to_string()),
    };
    let elapsed = t0.elapsed();
    check(
        "启动不中止（create_pool 返回 Ok）",
        started.is_ok(),
        match &started {
            Ok(()) => format!("Ok，用时 {:?}", elapsed),
            Err(e) => format!("**Err**：{e}"),
        },
    );

    // ── 后态 + 判据 2/3 ──
    let after = sqlite_peek(url, want_index).await;
    println!("  后态：表 {} / 索引 {}", after.tables, after.indexes);
    let d_tables = after.tables - before.tables;
    let d_indexes = after.indexes - before.indexes;
    check(
        "结构真被改（有可归因增量）",
        d_tables > 0 || d_indexes > 0,
        format!("Δ表 = {d_tables}｜Δ索引 = {d_indexes}"),
    );

    if let Some(name) = want_index {
        match &after.index_ddl {
            Some(ddl) => {
                println!("  `{name}` 的 DDL 原文 = {ddl}");
                // 谓词存活是**独立**于「对象存在」的一条：丢了谓词 ⇒ 整表唯一 ⇒ 检查点同名多行
                // 会被挡（插入失败），归因方向与「约束没生效」正好相反。
                let has_predicate = ddl.to_ascii_uppercase().contains("WHERE");
                check(
                    "目标索引的排除谓词存活（不是退化成整表唯一）",
                    has_predicate,
                    if has_predicate {
                        "DDL 里含 WHERE".to_string()
                    } else {
                        format!("**DDL 里没有 WHERE** —— 已退化成整表唯一：{ddl}")
                    },
                );
            },
            None => {
                check("目标索引真建出", false, format!("`{name}` **仍不在 `sqlite_master` 里**"))
            },
        }
    }

    // ── 判据 4：幂等 ──
    let second_ok = match create_pool(url).await {
        Ok(h) => {
            let _ = h.conn.close().await;
            true
        },
        Err(e) => {
            println!("  !! 第二遍 create_pool 报错：{e}");
            false
        },
    };
    let twice = sqlite_peek(url, want_index).await;
    check(
        "第二遍幂等（结构计数不再变）",
        second_ok && twice.tables == after.tables && twice.indexes == after.indexes,
        format!("Δ表 = {}｜Δ索引 = {}", twice.tables - after.tables, twice.indexes - after.indexes),
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

/// SQLite 侧的**原始**结构读数（表/索引计数 + 指定索引的 DDL 原文）。
///
/// 为什么绕开 `introspect` 直接读 `sqlite_master`：本判据要的是「这条索引的 `sql` 原文」——
/// 那是**谓词是否存活**的唯一直接证据。`introspect` 给的是归一化后的模型，归一化
/// 恰恰会把「写法差异」抹平，于是「谓词被改坏成另一个仍合法的谓词」在它那里不可见。
///
/// ⚠ 读失败**不返回 Err** 而是记进 `note`：探针在读数环节死掉会让「结构没改」这类结论
/// 被误报成「跑不起来」，两者要分开。
struct SqlitePeek {
    ok: bool,
    note: String,
    tables: i64,
    indexes: i64,
    index_ddl: Option<String>,
}

async fn sqlite_peek(url: &str, want_index: Option<&str>) -> SqlitePeek {
    let bad =
        |note: String| SqlitePeek { ok: false, note, tables: -1, indexes: -1, index_ddl: None };
    let handle = match connect_without_initialization(url).await {
        Ok(h) => h,
        Err(e) => return bad(format!("连库失败：{e}")),
    };
    let conn = &handle.conn;

    let tables = sqlite_count(conn, "table").await;
    let indexes = sqlite_count(conn, "index").await;

    let mut note = String::new();
    if tables < 0 || indexes < 0 {
        note.push_str("sqlite_master 计数读取失败；");
    }

    let index_ddl = match want_index {
        None => None,
        Some(name) => conn
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT sql FROM sqlite_master WHERE type='index' AND name='{name}'"),
            ))
            .await
            .ok()
            .flatten()
            .and_then(|r| r.try_get::<String>("", "sql").ok()),
    };

    let _ = handle.conn.close().await;
    SqlitePeek { ok: note.is_empty(), note, tables, indexes, index_ddl }
}

/// `sqlite_master` 里某一类对象的计数。读不出来返回 `-1`（**不是 0** —— 「读失败」与
/// 「真是 0 个」必须可区分，否则前态/后态相减会凭空造出「增量」或抹掉真实增量）。
async fn sqlite_count(conn: &DatabaseConnection, what: &str) -> i64 {
    conn.query_one_raw(Statement::from_string(
        DbBackend::Sqlite,
        format!("SELECT count(*) AS c FROM sqlite_master WHERE type='{what}'"),
    ))
    .await
    .ok()
    .flatten()
    .and_then(|r| r.try_get::<i64>("", "c").ok())
    .unwrap_or(-1)
}

// ═══════════════════════════════════════════════════════════════════════════
// 共用
// ═══════════════════════════════════════════════════════════════════════════

/// 把残余 plan 拆成「纯新增」与「非纯新增」两栏。理由见 `fresh_and_verify` 里那段注释。
fn split_residual(
    p: &plan::Plan,
) -> (BTreeMap<&'static str, usize>, BTreeMap<&'static str, usize>) {
    let mut additive: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut other: BTreeMap<&'static str, usize> = BTreeMap::new();
    for c in &p.changes {
        let slot = if c.kind.is_purely_additive() {
            &mut additive
        } else {
            &mut other
        };
        *slot.entry(c.kind.as_str()).or_insert(0) += 1;
    }
    (additive, other)
}
