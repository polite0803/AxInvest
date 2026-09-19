// SPDX-License-Identifier: AGPL-3.0-only
//!
//! v227（中文全文检索）的 PostgreSQL 端到端验证。
//!
//! ⚠ 2026-09-16：`v227_cjk_fts.rs` 已随 74 个迁移文件删除；`ax_cjk_ngram()` 的**唯一**
//! 定义处现在是 `dao/src/cjk_ngram.rs` + `dao/src/sql/ax_cjk_ngram.sql`（引擎按
//! `reconcile::extras` 的 `FUNCTIONS` 声明建出）。本文件的测试名与第 1 步已相应改判。
//!
//! ## 为什么必须有这个测试
//!
//! 中文全文检索修复由三部分组成，**任何一部分单独"看起来对"都不够**：
//!   1. `ax_cjk_ngram()` 函数存在且产出正确的 n-gram；
//!   2. `notes` / `memory_items` / `vec_*_meta` 的生成列真的改用了它，且 GIN 索引存在；
//!   3. 在**真实数据**上中文查询真的能命中。
//!
//! 只有第 1 条被 `cargo test -p axagent-search` 与
//! `scripts/check-ngram-consistency.mjs` 覆盖；2、3 必须对着真实 PG 才能验。
//! 缺了它们，"迁移写了但重建索引时静默失败"或"函数对了但列没改"都会溜过去 ——
//! 而这两种情况的表现都是「查询返回 0 行」，与"确实没有匹配内容"无法区分。
//!
//! ## 运行
//!
//! 需要真实 PG 连接串（本测试会**实际执行迁移**，与生产启动时的行为一致）：
//!
//! ```bash
//! AXAGENT_TEST_PG_URL='postgres://user:pass@host:port/db' \
//!   cargo test -p axagent-dao --test pg_cjk_fts -- --nocapture
//! ```
//!
//! 未设置该变量时测试**诚实跳过**（打印 SKIP 与原因）—— CI 没有生产 PG。
//! 注意：跳过不是通过。
//!
//! ## 变量名为什么必须与 `search/tests/pg_integration.rs` 完全一致
//!
//! （2026-09-16 补：本节原标题还点了 `pg_migrations.rs`。那个文件已随 74 个版本化迁移
//! 一起**退休**——它做的事是「跑全量迁移建全新库」，迁移清单清空后已无对象。
//! 下面这段事故记述**保留不改**，因为它是「为什么必须统一变量名」最有力的证据，
//! 只是请把 `pg_migrations.rs` 读成「一个已退休的、当年腐烂过的 PG 测试文件」。）
//!
//! 本仓库曾同时存在三种写法（`AXAGENT_TEST_PG_URL` / `AXAGENT_PG_TEST_URL` /
//! `AXINVEST_PG_TEST`）。后果不是「麻烦」，而是**这 4 个 PG 测试在 CI 里一条都没跑过**
//! —— 每个文件各自 gate，谁也没注意到隔壁用的不是同一个变量名。
//! 而「从未执行过的测试」会稳定腐烂：`pg_migrations.rs` 曾断言 `MAX(version) == 100`
//! （注册表早已到 v227）且断言一张 `v101` 会 DROP 掉的表「应存在」，跑一次就会红
//! —— 但因为没人跑，所以没人知道，直到 v101 在 PG 上把应用启动彻底搞挂。
//! 统一变量名是这条链上最便宜的一环；真正的根治是让 CI 起一个 PG service 去跑它们。

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

/// 与 `search/tests/pg_integration.rs` **必须一致**（原先还要求与 `pg_migrations.rs` 一致，
/// 该文件已于 2026-09-16 退休）。
const ENV_PG_URL: &str = "AXAGENT_TEST_PG_URL";

/// 连接测试库；未配置环境变量时返回 `None`（由调用方跳过）。
async fn connect_or_skip() -> Option<DatabaseConnection> {
    let url = match std::env::var(ENV_PG_URL) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!(
                "SKIP: 未设置 {ENV_PG_URL}，无法验证 v227 在真实 PG 上的落地效果。\
                 \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不具备条件。"
            );
            return None;
        },
    };
    Some(Database::connect(&url).await.expect("应能连接 PostgreSQL"))
}

async fn scalar_i64(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql.to_string()))
        .await
        .unwrap_or_else(|e| panic!("查询失败: {sql}\n{e}"))
        .unwrap_or_else(|| panic!("查询无返回行: {sql}"))
        .try_get::<i64>("", "n")
        .unwrap_or_else(|e| panic!("取列 n 失败: {sql}\n{e}"))
}

/// 落地是否生效 + 中文检索是否真的生效。
#[tokio::test]
async fn cjk_ngram_lands_and_enables_chinese_full_text_search() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    assert_eq!(
        db.get_database_backend(),
        DbBackend::Postgres,
        "本测试只验 PG —— 中文全文检索的 SQLite 路径是另一套实现（FTS5），未在 v227 范围内"
    );

    // ── 1. 确保已落地 ──
    //
    // 改走 `bootstrap_schema`（版本化迁移已于 2026-09-16 清空，`run_migrations`
    // 现在是空操作）。引擎从 `reconcile::extras` 的 `FUNCTIONS` + 生成列声明
    // 建出 `ax_cjk_ngram()` 与 `to_tsvector('simple', ax_cjk_ngram(...))` 生成列，
    // 语义与旧版 v227 迁移一致，且**没有**「版本号已到 227 就跳过」那条捷径。
    //
    // ⚠ 语义差异（登记）：旧版靠版本表高水位跳过，本版每轮都真跑一次收敛判定；
    //   在「纯新增类别」白名单下，已存在的对象不会被重复创建。
    //   而生成列**表达式**的变更（`AlterColumnType` 一类）不在纯新增白名单里 ⇒
    //   引擎不会重写存量库里形态不同的生成列，这一项已记入 PLAN。
    let started = std::time::Instant::now();
    axagent_dao::reconcile::apply::bootstrap_schema(&db).await.expect("引擎收敛必须成功");

    // ── 0（置于 bootstrap 之后）. 记录 notes 总量，用于事后对比/空库跳过 ──
    //
    // ⚠ 2026-09-19 修正顺序：原实现把 `count(*) FROM notes` 放在 bootstrap **之前**，
    //   假设连接到「已有业务数据的库」（见旧注释「notes 48590 行」）。
    //   而 CI 起的是 pgvector **全新空库**（`POSTGRES_DB: axagent_test`）⇒ 表尚未建出
    //   ⇒ `relation "notes" does not exist` 直接崩，PG 集成测试在 CI 首次真跑即红。
    //
    //   语义核对：`notes_total` 只用于两句 println 与「空库则跳过效果断言」（见步骤 4），
    //   不参与「迁移前/后」对比 —— 注释里那个 plainto_tsquery 命中 2 行的基线早已不用。
    //   bootstrap 是「纯新增类别」收敛（只建表/函数/索引，不改存量数据），故把计数移到
    //   bootstrap 之后语义不变：存量库里 read 到同一批行，全新库里读到 0（naturally 跳过）。
    let notes_total = scalar_i64(&db, "SELECT count(*)::bigint AS n FROM notes").await;
    println!(
        "[cjk-ngram] bootstrap_schema 耗时 {:?}（notes {} 行）",
        started.elapsed(),
        notes_total
    );

    // ── 2. 函数存在且与 Rust 规范逐字节一致 ──
    //
    // 这里只抽查关键形态；完整 34 条规范由
    // `cargo test -p axagent-search --test ngram_consistency` 与
    // `scripts/check-ngram-consistency.mjs` 用同一份 fixture 覆盖。
    let probes: &[(&str, &str)] = &[
        ("向量索引", "向 量 索 引 向量 量索 索引"),
        ("股票", "股 票 股票"),
        ("股", "股"),
        ("股票股票", "股 票 股 票 股票 票股 股票"),
        ("使用pgvector实现", "使 用 使用 pgvector 实 现 实现"),
        ("你好，世界", "你 好 你好 世 界 世界"),
        ("привет мир", "привет мир"),
        ("", ""),
    ];
    for (input, expected) in probes {
        // 用参数化查询避免转义问题
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT ax_cjk_ngram($1) AS out",
                vec![(*input).into()],
            ))
            .await
            .expect("调用 ax_cjk_ngram 失败");
        let actual: String = row.expect("应有返回行").try_get("", "out").expect("取 out 失败");
        assert_eq!(
            &actual, expected,
            "PG 侧 ax_cjk_ngram({input:?}) 与 Rust 规范不符 —— \
             索引与查询将静默失配（索引里有、查询取不到）"
        );
    }

    // ── 3. 结构断言：生成列真的改用了 ax_cjk_ngram，GIN 索引真的存在 ──
    let mut targets: Vec<(String, String, String)> = vec![
        ("notes".into(), "tsv".into(), "idx_notes_tsv".into()),
        ("memory_items".into(), "content_tsv".into(), "idx_memory_items_tsv".into()),
    ];
    let vec_tables = db
        .query_all_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name ~ '^vec_.*_meta$' ORDER BY table_name"
                .to_string(),
        ))
        .await
        .expect("列举 vec_*_meta 失败");
    for row in &vec_tables {
        let name: String = row.try_get("", "table_name").expect("取 table_name 失败");
        targets.push((name.clone(), "content_tsv".into(), format!("idx_{name}_tsv")));
    }
    assert!(targets.len() >= 2, "至少应覆盖 notes 与 memory_items，实际 {}", targets.len());

    for (table, column, index) in &targets {
        let expression = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT generation_expression AS out FROM information_schema.columns \
                 WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2",
                vec![table.clone().into(), column.clone().into()],
            ))
            .await
            .expect("查询 information_schema 失败");
        let expression = expression
            .unwrap_or_else(|| panic!("{table}.{column} 不存在 —— v227 未生效或该表缺失"))
            .try_get::<String>("", "out")
            .expect("取 generation_expression 失败");
        assert!(
            expression.contains("ax_cjk_ngram"),
            "{table}.{column} 的生成列仍是旧定义，中文检索仍会失效：\n  {expression}"
        );

        let idx_count = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*)::bigint AS n FROM pg_indexes \
                 WHERE schemaname = 'public' AND tablename = $1 AND indexname = $2",
                vec![table.clone().into(), index.clone().into()],
            ))
            .await
            .expect("查询 pg_indexes 失败")
            .expect("应有返回行")
            .try_get::<i64>("", "n")
            .expect("取 n 失败");
        assert_eq!(
            idx_count, 1,
            "{table} 缺少 GIN 索引 {index} —— 查询会退化为全表扫描，且**不报任何错**"
        );
        println!("[v227] ✓ {table}.{column}（索引 {index}）");
    }

    // ── 4. 效果断言：真实数据上中文检索必须真正可用 ──
    //
    // 迁移前实测：notes 48590 行，`tsv @@ plainto_tsquery('simple','股票')`
    // 只命中 2 行（因为整句中文被当作单个词元）。
    // 迁移后：ngram 索引使「股」「票」「股票」都可命中。
    if notes_total == 0 {
        println!("[v227] notes 表为空，跳过效果断言（新库无真实数据可比）");
        return;
    }

    let hits = scalar_i64(
        &db,
        "SELECT count(*)::bigint AS n FROM notes \
         WHERE tsv @@ to_tsquery('simple', '''股'' | ''票'' | ''股票''')",
    )
    .await;
    println!("[v227] notes 中文检索命中 {hits} / {notes_total} 行（修复前同一语义仅命中 2 行）");

    assert!(
        hits > 2,
        "中文检索命中 {hits} 行，未超过修复前的 2 行 —— \
         迁移可能未真正重建索引（生成列改了但数据未重算？）"
    );

    // 反证：旧查询方式在新索引上应命中更少，证明索引内容确实变了
    let legacy_hits = scalar_i64(
        &db,
        "SELECT count(*)::bigint AS n FROM notes \
         WHERE tsv @@ plainto_tsquery('simple', '股票')",
    )
    .await;
    println!("[v227] 旧查询方式（plainto_tsquery 整词）命中 {legacy_hits} 行");
    assert!(
        hits > legacy_hits,
        "ngram 查询命中 {hits} 行未超过旧查询的 {legacy_hits} 行，\
         说明索引未按 ngram 重建"
    );
}
