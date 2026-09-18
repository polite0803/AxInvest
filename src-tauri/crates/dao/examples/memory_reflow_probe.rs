// SPDX-License-Identifier: AGPL-3.0-only

//! Memory → Knowledge 回流：**真 PG 端到端探针**（2026-09-17，`PLAN-memory-kb-reflow-id-space.md` §6 验收）。
//!
//! # 为什么需要它
//!
//! §6 的 5 条验收判据里有 4 条只有**真跑生产函数**才能答：
//! 「哨兵行是否真被建出」「回流第一轮是否真写入」「第二轮是否**不**线性增长」
//! 「`mention_count` 是否按预期累加」—— SQLite 单测答不了 PG 方言侧，
//! 而 `axinvest` 存量库里 `importance >= 0.7` 的记忆条目**为 0 条**（实测），
//! 所以必须在**独立测试库**上造数据。
//!
//! # 与既有探针的**关键区别**：连接串不进 argv
//!
//! `wiki_edge_relation_probe` / `p4_apply_probe` 等走
//! `"$(node scripts/pg-connect.mjs url)"` 形态 ⇒ 口令落在 **argv** 里，
//! 并被 shell / cargo 的 `Running … <exe> 'postgres://user:<口令>@…'` 行**回显**
//! —— 这正是 `PLAN-declarative-schema-sync.md:2598` 记录过的那次泄漏。
//!
//! 本探针改为**自己在进程内调 `pg-connect.mjs` 并读它的 stdout**：
//! 口令只存在于本进程内存，**不进 argv、不进环境变量、不进任何日志**。
//! 脚本仅在错误分支里报告「返回的不是 PG 连接串」，永不回显连接串本身。
//!
//! # 用法
//!
//! ```text
//! cargo run -p axagent-dao --example memory_reflow_probe --
//! cargo run -p axagent-dao --example memory_reflow_probe -- --db=axagent_pg_migtest
//! cargo run -p axagent-dao --example memory_reflow_probe -- --node=C:/path/to/node.exe
//! cargo run -p axagent-dao --example memory_reflow_probe -- --inject-failure   # 加验「失败可见」
//! ```
//!
//! ⚠ **默认库 = `axagent_pg_migtest`**（安全侧）：不显式指定就**不会**碰 `axinvest`。
//!    探针会**写入**（造 3 条记忆条目 + 建 1 个命名空间），结束时**清理**。
//!
//! # 它做六件事（加 `--inject-failure` 则多做一件：**失败可见**）
//!
//! | # | 动作 | 期望 |
//! |---|---|---|
//! | ① | `seed::ensure_sentinels` | `knowledge_bases` 出现 `__sys_memory_reflow__`（`enabled = 0`） |
//! | ② | **再调一次** `ensure_sentinels` | 行数不变 —— 幂等 |
//! | ③ | 造 3 条 `importance >= 0.7` 的记忆条目 | 第 1 轮流回应写 3 行 |
//! | ④ | 第 2 轮回流 | **行数仍为 3**（验的是「不线性增长」，不是「第 1 轮成功」） |
//! | ⑤ | 读回 `mention_count` | 每行 == 2 —— 证明走的是 **UPDATE** 而非 INSERT |
//! | ⑤b | `--inject-failure`：用**不存在**的 KB id 调一次 | 逐条失败（不是整体 `Err`）· `entities_created == 0` · `failures == items_read` · stderr 出现**恰好一条**计数型 warn |
//! | ⑥ | 清理探针自造的数据 | 命名空间删掉（CASCADE 清条目）+ 该 KB 下实体删掉 |
//!
//! ⚠ ① 的**区分力说明**：`create_pool` → `initialize_schema` 会在**探针读「before」之前**
//! 就把哨兵行播种出来（实测：先把该行 DELETE 掉、再跑本探针，`before` 仍显示已存在）。
//! 所以 ① 的判据**不靠 before/after 的差**，而是靠**配合 SQL 的删除-重建对照**：
//! `DELETE` 后库里只剩 `__sys_trajectory__`，跑一次探针 ⇒ 该行被重建。
//!
//! ④ 是本探针的**区分力所在**：若 `upsert_entity` 仍是「新生成 id + `on_conflict(Id)`」
//! 那套写法，④ 会看到 **6 行**（而 ①②③⑤ 全都照常通过）。
//!
//! # 退出码
//!
//! 0 = 全部符合预期；1 = 有反例；2 = 参数 / 连库 / 脚本执行失败。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axagent_dao::db::{DatabaseConnection, create_pool};
use axagent_dao::repo::knowledge_graph as kg;
use axagent_entities::{knowledge_bases, knowledge_entities, memory_items, memory_namespaces};
use axagent_harness::constants::sentinel::{
    MEMORY_REFLOW_KB_ENABLED, MEMORY_REFLOW_KB_ID, TRAJECTORY_KB_ID, TRAJECTORY_MEM_NS_ID,
};
use sea_orm::{ColumnTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set};

/// 安全侧默认：显式传 `--db=axinvest` 才会碰生产库。
const DEFAULT_DB: &str = "axagent_pg_migtest";
const DEFAULT_NODE: &str = "node";

/// 探针自造的命名空间（结束后删除；`memory_items.namespace_id` 对它有 `ON DELETE CASCADE`）。
const PROBE_NS: &str = "__probe_memory_reflow__";
/// 探针自造条目的 id / content 前缀 —— 用于「只看得见探针自造的行」的断言与清理。
const PROBE_PREFIX: &str = "probe_memory_reflow_";

fn flag<'a>(args: &'a [String], prefix: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(prefix))
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// 取连接串 —— **不进 argv**（本文件头「与既有探针的关键区别」）。
///
/// `cargo run --example` 的 cwd 可能是仓库根 / `src-tauri` / crate 目录三种之一，
/// 故按候选列表探测脚本位置；路径本身不是敏感值，可以出现在诊断里。
fn resolve_pg_url(db_name: &str, node: &str) -> Result<String, String> {
    let candidates = [
        "scripts/pg-connect.mjs",
        "../scripts/pg-connect.mjs",
        "../../scripts/pg-connect.mjs",
        "../../../scripts/pg-connect.mjs",
    ];
    let script = candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .ok_or_else(|| format!("找不到 scripts/pg-connect.mjs（试过 {candidates:?}）"))?;

    let out = std::process::Command::new(node)
        .arg(script)
        .arg("url")
        .arg(format!("--db={db_name}"))
        .output()
        .map_err(|e| format!("无法执行 `{node}` —— `--node=<绝对路径>` 可指定解释器: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "pg-connect 非 0 退出: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    // ⚠ 不把 url 打进任何错误信息。
    if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
        return Err("pg-connect 返回的不是 PG 连接串（未回显其内容）".to_owned());
    }
    Ok(url)
}

/// `knowledge_bases` 的全部 id（排序后）。
async fn kb_ids(db: &DatabaseConnection) -> Result<Vec<String>, DbErr> {
    let mut v: Vec<String> =
        knowledge_bases::Entity::find().all(db).await?.into_iter().map(|m| m.id).collect();
    v.sort();
    Ok(v)
}

/// 指定 KB 下的实体行数。
async fn entity_count(db: &DatabaseConnection, kb: &str) -> Result<u64, DbErr> {
    knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(kb))
        .count(db)
        .await
}

/// 造 3 条高重要度记忆条目（幂等：先删同前缀的旧行再插）。
async fn seed_probe_items(db: &DatabaseConnection) -> Result<usize, DbErr> {
    let ns = memory_namespaces::ActiveModel {
        id: Set(PROBE_NS.to_owned()),
        name: Set("Probe namespace (memory reflow)".to_owned()),
        scope: Set("system".to_owned()),
        sort_order: Set(0),
        ..Default::default()
    };
    memory_namespaces::Entity::insert(ns)
        .on_conflict_do_nothing()
        .exec_without_returning(db)
        .await?;

    // 清掉上一轮残留（同一进程外的历史残留也一并清）
    memory_items::Entity::delete_many()
        .filter(memory_items::Column::NamespaceId.eq(PROBE_NS))
        .exec(db)
        .await?;

    let stamp = now_ms().to_string();
    let importances = [0.80_f64, 0.85, 0.90];
    for (i, imp) in importances.iter().enumerate() {
        let item = memory_items::ActiveModel {
            id: Set(format!("{PROBE_PREFIX}{i}")),
            namespace_id: Set(PROBE_NS.to_owned()),
            title: Set(format!("probe item {i}")),
            // `upsert_entity` 用 content 前 100 字当实体名 ⇒ 前缀可控便于清理/断言
            content: Set(format!("{PROBE_PREFIX}{i} 高重要度记忆条目（回流幂等性验收用）")),
            source: Set("probe".to_owned()),
            index_status: Set("pending".to_owned()),
            updated_at: Set(stamp.clone()),
            tier: Set("working".to_owned()),
            importance: Set(*imp),
            access_count: Set(0),
            memory_nature: Set("semantic".to_owned()),
            tags: Set("[]".to_owned()),
            confirmed: Set(0),
            ..Default::default()
        };
        memory_items::Entity::insert(item)
            .on_conflict_do_nothing()
            .exec_without_returning(db)
            .await?;
    }
    Ok(importances.len())
}

/// 清理探针自造的一切（保留哨兵行 —— 那是**生产代码该建的**，不是污染）。
async fn cleanup(db: &DatabaseConnection) -> Result<(), DbErr> {
    knowledge_entities::Entity::delete_many()
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(MEMORY_REFLOW_KB_ID))
        .filter(knowledge_entities::Column::Name.starts_with(PROBE_PREFIX))
        .exec(db)
        .await?;
    memory_namespaces::Entity::delete_by_id(PROBE_NS.to_owned()).exec(db).await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    // ⚠ 必须先把 tracing 接上：否则 `reflow_memory_to_knowledge` 的计数型 warn
    //    **无处落地** ⇒ 「失败可见」这条判据只能靠 stats 字段推断，验不成。
    //    （第一次跑本探针就踩到：注入失败后 stderr 一片空白。）
    //    默认过滤 `axagent_dao=warn`，用 `RUST_LOG` 可覆盖。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("axagent_dao=warn")),
        )
        .try_init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let db_name = flag(&args, "--db=").unwrap_or(DEFAULT_DB).to_owned();
    let node = flag(&args, "--node=").unwrap_or(DEFAULT_NODE).to_owned();

    println!("=== memory_reflow_probe ===");
    println!("目标库     : {db_name}");
    println!("注意       : 本探针会写入并清理；连接串不从 argv 读取");

    let url = match resolve_pg_url(&db_name, &node) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[环境错误] {e}");
            std::process::exit(2);
        },
    };
    let handle = match create_pool(&url).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[环境错误] 连库失败: {e}");
            std::process::exit(2);
        },
    };
    let db = &handle.conn;

    let mut fails: Vec<String> = Vec::new();

    // ── P1 哨兵行 ────────────────────────────────────────────────────────
    let before = match kb_ids(db).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[环境错误] 读 knowledge_bases 失败: {e}");
            std::process::exit(2);
        },
    };
    let had_reflow_kb = before.iter().any(|id| id == MEMORY_REFLOW_KB_ID);
    println!("\n[P1] ensure_sentinels 之前 knowledge_bases = {before:?}");
    if let Err(e) = axagent_dao::seed::ensure_sentinels(db).await {
        eprintln!("[环境错误] ensure_sentinels 失败: {e}");
        std::process::exit(2);
    }
    let after = kb_ids(db).await.unwrap_or_default();
    println!("[P1] ensure_sentinels 之后 knowledge_bases = {after:?}");
    println!("[P1] 回流 KB 探针前已存在? {had_reflow_kb}");

    for want in [TRAJECTORY_KB_ID, MEMORY_REFLOW_KB_ID] {
        if !after.iter().any(|id| id == want) {
            fails.push(format!("P1 哨兵 KB `{want}` 在 ensure_sentinels 之后仍不存在"));
        }
    }
    match knowledge_bases::Entity::find_by_id(MEMORY_REFLOW_KB_ID).one(db).await {
        Ok(Some(m)) => {
            println!(
                "[P1] 回流 KB: name={:?} enabled={} description={:?}",
                m.name, m.enabled, m.description
            );
            if m.enabled != MEMORY_REFLOW_KB_ENABLED {
                fails.push(format!(
                    "P1 回流 KB 的 enabled = {}，期望 {MEMORY_REFLOW_KB_ENABLED}",
                    m.enabled
                ));
            }
            if m.name.trim().is_empty() {
                fails.push("P1 回流 KB 的 name 为空".to_owned());
            }
        },
        Ok(None) => fails.push("P1 回流 KB 查询返回 None".to_owned()),
        Err(e) => fails.push(format!("P1 读回流 KB 失败: {e}")),
    }
    if !after.iter().any(|id| id == TRAJECTORY_MEM_NS_ID) {
        // 命名空间在另一张表；这里只提示，不判失败（P1 的错误已由 KB 断言覆盖）
        println!("[P1] 提示: 记忆命名空间哨兵 `{TRAJECTORY_MEM_NS_ID}` 未在 KB 表中（预期如此）");
    }

    // ── P2 幂等：再调一次，行数必须不变 ────────────────────────────────
    if let Err(e) = axagent_dao::seed::ensure_sentinels(db).await {
        fails.push(format!("P2 第二次 ensure_sentinels 失败: {e}"));
    }
    let after2 = kb_ids(db).await.unwrap_or_default();
    println!("\n[P2] 第二次 ensure_sentinels 之后 = {after2:?}");
    if after2.len() != after.len() {
        fails.push(format!("P2 不幂等：行数从 {} 变为 {}", after.len(), after2.len()));
    }

    // ── 造数据 ───────────────────────────────────────────────────────────
    let made = match seed_probe_items(db).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[环境错误] 造探针数据失败: {e}");
            std::process::exit(2);
        },
    };
    println!("\n[P3] 已造 {made} 条 importance>=0.7 的记忆条目（命名空间 {PROBE_NS}）");

    // 先清空该 KB 下探针历史残留，保证 P3/P4 的行数判据有区分力
    if let Err(e) = knowledge_entities::Entity::delete_many()
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(MEMORY_REFLOW_KB_ID))
        .exec(db)
        .await
    {
        eprintln!("[环境错误] 清空回流 KB 历史实体失败: {e}");
        std::process::exit(2);
    }

    // ── P3 第一轮回流 ────────────────────────────────────────────────────
    let s1 = match kg::reflow_memory_to_knowledge(
        db,
        MEMORY_REFLOW_KB_ID,
        kg::REFLOW_IMPORTANCE_THRESHOLD,
        kg::REFLOW_MAX_ITEMS,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[环境错误] 第一轮回流失败: {e}");
            std::process::exit(2);
        },
    };
    println!(
        "[P3] 第 1 轮: items_read={} entities_created={} failures={}",
        s1.items_read, s1.entities_created, s1.failures
    );
    let n1 = entity_count(db, MEMORY_REFLOW_KB_ID).await.unwrap_or(0);
    println!("[P3] 回流 KB 下实体行数 = {n1}");
    if s1.failures != 0 {
        fails.push(format!(
            "P3 failures = {}（期望 0）—— 单条回流失败，见日志里的 [memory_reflow] warn",
            s1.failures
        ));
    }
    if s1.items_read != made {
        fails.push(format!(
            "P3 items_read = {}，期望 {made}（库里应只有探针自造的高重要度条目）",
            s1.items_read
        ));
    }
    if n1 as usize != made {
        fails.push(format!("P3 第 1 轮后行数 = {n1}，期望 {made}"));
    }

    // ── P4 第二轮回流：**行数不增长**（本探针的区分力所在）────────────────
    let s2 = match kg::reflow_memory_to_knowledge(
        db,
        MEMORY_REFLOW_KB_ID,
        kg::REFLOW_IMPORTANCE_THRESHOLD,
        kg::REFLOW_MAX_ITEMS,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[环境错误] 第二轮回流失败: {e}");
            std::process::exit(2);
        },
    };
    println!(
        "\n[P4] 第 2 轮: items_read={} entities_created={} failures={}",
        s2.items_read, s2.entities_created, s2.failures
    );
    let n2 = entity_count(db, MEMORY_REFLOW_KB_ID).await.unwrap_or(0);
    println!("[P4] 回流 KB 下实体行数 = {n2}");
    if n2 != n1 {
        fails.push(format!("P4 **线性增长**：第 1 轮 {n1} 行 → 第 2 轮 {n2} 行（去重未生效）"));
    }
    if n2 as usize != made {
        fails.push(format!("P4 两轮后行数 = {n2}，期望 {made}"));
    }
    if s2.failures != 0 {
        fails.push(format!("P4 第 2 轮 failures = {}", s2.failures));
    }

    // ── P5 mention_count == 2（证明走的是 UPDATE 而非 INSERT）─────────────
    match knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(MEMORY_REFLOW_KB_ID))
        .order_by_asc(knowledge_entities::Column::Name)
        .all(db)
        .await
    {
        Ok(rows) => {
            println!("\n[P5] 两轮后该 KB 下 {} 行：", rows.len());
            for r in &rows {
                println!(
                    "     name={:?} mention_count={} confidence={} entity_type={}",
                    r.name, r.mention_count, r.confidence, r.entity_type
                );
                if r.mention_count != 2 {
                    fails.push(format!(
                        "P5 `{}` 的 mention_count = {}，期望 2 —— 说明第 2 轮可能是 INSERT 而非 UPDATE",
                        r.name, r.mention_count
                    ));
                }
            }
            let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
            let all_probe = names.iter().all(|n| n.starts_with(PROBE_PREFIX));
            println!("[P5] 全部来自探针自造条目? {all_probe}");
            if !all_probe {
                fails.push(
                    "P5 该 KB 下存在非探针自造的行（存量库不该有此 KB 的历史数据）".to_owned(),
                );
            }
        },
        Err(e) => fails.push(format!("P5 读实体失败: {e}")),
    }

    // ── P5b 失败可见（§6 第 5 条）：故意用一个**不存在**的 KB id ──────────
    //
    // 这是本项缺陷本体的对偶：原实现把失败 `debug!` 吞掉 ⇒ 用户看到
    // `{items_read: N, entities_created: 0, failures: N}` 却**不知道原因**。
    // 修好后应满足：① 逐条失败而不是整体 Err ② `failures == items_read`
    // ③ `entities_created == 0` ④ 日志里出现**一条计数型** warn（不是每条一行）。
    // ④ 由**运行者**从 stderr 观察（本探针只打印断言提示，不去解析 tracing 输出）。
    if args.iter().any(|a| a == "--inject-failure") {
        const GHOST_KB: &str = "__probe_nonexistent_kb__";
        println!("\n[P5b] 注入失败场景：kb_id = {GHOST_KB}（该行不存在 ⇒ 外键必拒）");
        match kg::reflow_memory_to_knowledge(
            db,
            GHOST_KB,
            kg::REFLOW_IMPORTANCE_THRESHOLD,
            kg::REFLOW_MAX_ITEMS,
        )
        .await
        {
            Ok(s3) => {
                println!(
                    "[P5b] items_read={} entities_created={} failures={}",
                    s3.items_read, s3.entities_created, s3.failures
                );
                if s3.items_read == 0 {
                    fails.push("P5b items_read = 0 —— 注入场景无效（应先造好数据）".to_owned());
                }
                if s3.entities_created != 0 {
                    fails.push(format!(
                        "P5b entities_created = {}，期望 0（外键应拒绝全部）",
                        s3.entities_created
                    ));
                }
                if s3.failures != s3.items_read {
                    fails.push(format!(
                        "P5b failures = {}，期望 == items_read = {}（每条都应失败）",
                        s3.failures, s3.items_read
                    ));
                }
                println!(
                    "[P5b] 本次调用应已打出**恰好一条**计数型 warn（`[memory_reflow] kb={GHOST_KB} \
                     回流部分失败：读取 N 条，成功 0，失败 N`）—— tracing 走 stderr，见本日志上半部分"
                );
            },
            Err(e) => fails.push(format!(
                "P5b 返回整体 Err（期望「逐条失败 + 计数型 warn」而非整体失败）: {e}"
            )),
        }
        let ghost_rows = entity_count(db, GHOST_KB).await.unwrap_or(999);
        println!("[P5b] 该假 KB 下实体行数 = {ghost_rows}（期望 0）");
        if ghost_rows != 0 {
            fails.push(format!("P5b 假 KB 下出现 {ghost_rows} 行"));
        }
    }

    // ── P6 清理 ──────────────────────────────────────────────────────────
    match cleanup(db).await {
        Ok(()) => {
            let left = entity_count(db, MEMORY_REFLOW_KB_ID).await.unwrap_or(999);
            let ns_left = memory_namespaces::Entity::find_by_id(PROBE_NS.to_owned())
                .count(db)
                .await
                .unwrap_or(999);
            println!("\n[P6] 清理完成：该 KB 下残留实体 = {left}，探针命名空间残留 = {ns_left}");
            if left != 0 || ns_left != 0 {
                fails.push(format!("P6 清理不彻底：实体残留 {left}，命名空间残留 {ns_left}"));
            }
        },
        Err(e) => fails.push(format!("P6 清理失败: {e}")),
    }

    println!("\n=== 结论 ===");
    if fails.is_empty() {
        println!("PASS —— 六项判据全部符合预期（库 {db_name}）");
        std::process::exit(0);
    }
    for f in &fails {
        println!("FAIL · {f}");
    }
    println!("共 {} 项反例（库 {db_name}）", fails.len());
    std::process::exit(1);
}
