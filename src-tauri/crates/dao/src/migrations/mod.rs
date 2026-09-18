// SPDX-License-Identifier: AGPL-3.0-only
//! Versioned schema migration framework.
//!
//! ## 当前状态（2026-09-16 起）
//!
//! **建表来源只有一个：声明式收敛引擎**（`crate::reconcile::apply::bootstrap_schema`）。
//! 迁移清单 [`MIGRATIONS`] 已清空为 `&[]`，本模块的执行路径因此成为 no-op；但本模块
//! **不能删** —— 它是「老库升级」的唯一落点（存量库的一次性数据搬迁：知识图谱合并、
//! 回填、改名…），那些是实体声明表达不了的逻辑。
//!
//! ## 曾经的形态（已失效，保留作考古）
//!
//! 清空前这里是「**上游基线 + 本地增量**」双层架构：`v100_consolidated` 是上游全部 DDL
//! （表 / 索引 / 触发器 / 种子数据）的单一基线，`v101–v233` 是本地各功能模块的增量。
//! 那些迁移文件已随这次切换删除（可从 `.worktrees/` 的旧整树快照按版本号找回），
//! 所以此处**不再以「文件:行」引用它们** —— 指向已删文件的链接是死的，而
//! `cargo check` / `clippy` **都不查** intra-doc link（只有 `cargo doc` 会报）。
//!
//! ## 约定（清空后）
//!
//! - 建表 / 加列 / 加索引：改**实体声明**或 `crate::reconcile::extras` 的声明，不要新建迁移
//! - 确需数据搬迁的升级：在本模块追加一条 `Migration` —— 这是迁移仅剩的用途
//! - [`CURRENT_VERSION`] 的「保持 233」理由见其自身文档

use sea_orm::{ConnectionTrait, DbBackend, DbErr, Statement};

pub mod pg_ddl;
pub mod schema_diff;
// 上游新 migration：为 workflow_templates 表添加 hooks_config 列（模板级生命周期钩子）。
// 上游编号 v134 与本地 v134_lead_workflow_link 冲突，故作为本地序列下一个版本 v224 追加。

/// 当前 schema 版本号。迁移时代「每次新增 migration 时必须累加此常量」的约定已随
/// 清单清空失效（[`MIGRATIONS`] = `&[]`）。
///
/// 语义仍是「**代码中定义的最新版本号**」（见 [`SchemaStatus::latest_version`]），
/// 但唯一用途只剩 `init/database.rs` 的 `applied_version > latest_version` 判定 ——
/// 它识别「连到了版本号体系不同的下游 fork 库」并触发一次重型 `repair_schema`。
///
/// ⚠ **必须保持 233，不得改成 0**：版本表是存量库的既成事实（生产库最后一行是 233），
/// 改成 0 会让**每一个**存量库都满足 `applied > latest` ⇒ 每次启动跑一次全量自愈。
///
/// 2026-09-15 修正：原为 `231`，而数组早已含 `v232` ⇒ 两处漂移。
///
/// ⚠ 2026-09-16 清空迁移清单时，下面两条旧描述**同时失效**，故一并改写（不是删除结论，
/// 而是说明它们为何不再成立）：
/// * 「必须等于 `MIGRATIONS` 数组里的最大 `version`」—— 空数组没有最大值，该约束
///   已无对象。它当年的目的是「防偏小 ⇒ 误判 fork 库」，那个目的现在由「保持 233」
///   直接承担。
/// * 「用于 `repair_schema` 收尾记录」—— `repair_schema` 已不再写版本表
///   （理由见该函数文档）。
///
/// 它与「是否执行某条迁移」从来无关（后者判据是版本表集合成员，见 [`missing_versions`]）。
pub const CURRENT_VERSION: i32 = 233;

/// P2-10: Schema 版本追踪表名。
///
/// 所有 migration 状态查询/写入都通过此常量引用表名，
/// 避免散落的字符串字面量导致重命名时遗漏。
pub const SCHEMA_VERSION_TABLE: &str = "axagent_schema_version";

/// 迁移函数签名：所有 `up()` 都遵循这个接口。
///
/// `DatabaseConnection` 是 `Arc<DbConnection>` 的 newtype，clone
/// 是引用计数 +1，零拷贝。所以 `up` 接收 owned 是 trivial 的：
/// 调用方在每次 invoke 时 clone 一份即可。
///
/// 用 owned 而非 `&DatabaseConnection` 是为了让 boxed future 不带
/// 借用——`Pin<Box<dyn Future + 'static>>` 可以装进 `const MIGRATIONS`
/// 数组（fn pointer 自身要求 'static）。
///
/// `Send` 是为了让 `run_migrations` 能在 multi-threaded runtime 中
/// 被调用（生产环境 `tokio::main` 默认是 multi_thread）。不需要
/// `Sync`：future 只在 await 期间被一个 task 持有，不存在共享。
pub type MigrationFn =
    fn(
        sea_orm::DatabaseConnection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DbErr>> + Send>>;

struct Migration {
    version: i32,
    description: &'static str,
    up: MigrationFn,
}

/// 版本化迁移的**注册表** —— 2026-09-16 起**刻意保持为空**。
///
/// 建表职责已交给声明式引擎（`reconcile::apply::bootstrap_schema`，由
/// `crate::db::initialize_schema` 调用）。空表不是「忘了填」，而是本轮的**终点状态**：
/// 一旦往这里重新加迁移，它就会和引擎**同时**是建表来源，而两套真相源的差异
/// （列类型、索引命名 `idx_x_y` vs 引擎的 `idx-x-y`、约束写法）不会在改动当场暴露，
/// 只会在某次启动时集中爆发成难以归因的故障。要恢复迁移机制，必须先撤掉引擎接管 ——
/// 二者只能有一个 owner。
///
/// ## 数组清空后本模块仍然要留下的东西（**不可**跟着一起删）
///
/// 删掉任一项都会各自造成一类缺陷：
///
/// 1. `SCHEMA_VERSION_TABLE` + `record_version` + `get_schema_status`：
///    版本表是**存量库的既成事实**（生产库有 74 行）。`init/database.rs` 用
///    `applied_version > latest_version` 判定「版本超前」并触发全量自愈 ——
///    把这套记账删掉，所有老库都会被误判成「下游 fork 库」，每次启动跑一次重型自愈。
/// 2. `CURRENT_VERSION`：**必须保留 233，不得改成 0**。它是上面那条判定的另一半；
///    改成 0 会让「已应用 233 的库」看起来超前 233 个版本 ⇒ 同样每次启动全量自愈。
/// 3. `schema_diff::heal_all`（`repair_schema` 内调用）：启动期**唯一的列类型加宽**
///    通道 —— `AlterColumnType` 不在引擎的纯新增白名单里，引擎不做这件事。
///
/// ## 与「逐条迁移」一起消失的能力（**已知代价，已登记**）
///
/// 迁移时代逐条记录的「这条迁移修了什么」不复存在 ⇒ 存量库若落后于 `CURRENT_VERSION`，
/// 不再有任何**数据修复**通道（引擎只做结构收敛，不做数据搬迁）。在用的库都已在
/// 其迁移轨道上执行完，故本轮接受此代价；详见 PLAN §十一。
const MIGRATIONS: &[Migration] = &[];

/// 执行所有尚未应用的 schema 迁移。
///
/// 启动时调用；幂等，多次调用结果相同。
///
/// 「尚未应用」的判据是**版本表集合成员**，不是 `MAX(version)` 高水位线 ——
/// 两者在「中间缺口」场景下结论相反，理由见函数体内注释与 `missing_versions`。
///
/// 第一步（建 version tracking 表）使用 `&impl ConnectionTrait`——这是
/// ConnectionTrait 的稳定接口，ddl.rs shim 可以直接转发。第二步（实际跑 up()）
/// 需要 `&DatabaseConnection`，所以顶层 API 接收 `&DatabaseConnection`；
/// ddl.rs shim 已经更新成强类型。
///
/// ## 为什么每条迁移**没有**包事务（2026-09-13 实测结论）
///
/// 「给每条迁移包事务」长期挂在待办上：判据从高水位线改成集合成员后，失败的迁移
/// 会被**重试**，于是「中途失败留下的 partial apply 被反复叠加」成为真实风险。
///
/// 但**当前 API 形状下做不到**，实测依据（不是推测）：
///
/// - `sea-orm 2.0.2` 的 `DatabaseConnectionType`（即 `DatabaseConnection` 的内层枚举，
///   定义在 `sea-orm-2.0.2/src/database/db_connection.rs:47`）只有**连接池**变体：
///   `SqlxMySqlPoolConnection` / `SqlxPostgresPoolConnection` /
///   `SqlxSqlitePoolConnection` / `RusqliteSharedConnection` /
///   `MockDatabaseConnection` / `ProxyDatabaseConnection` / `Disconnected`。
///   **没有 `Transaction` 变体**；
/// - 也不存在 `impl From<DatabaseTransaction> for DatabaseConnection`
///   （全 crate grep 零命中）；
/// - 而 [`MigrationFn`] 的签名是 `fn(DatabaseConnection) -> …`（**owned**）。
///
/// ⇒ 事务对象无法作为 `DatabaseConnection` 传进 `up()`。
///
/// 唯一的做法是：把 [`MigrationFn`] 与**全部 68 个迁移**的 `up()` 签名从
/// `DatabaseConnection` 改成 `&DatabaseTransaction`（并连带改它们调用的每一个
/// 辅助函数的签名），再在 `db.transaction(|txn| …)` 的闭包里调用。
/// 那是**跨 68 个文件的重构 + 双方言真机全量回归**，不应与别的修复混做一轮。
///
/// ⚠ 2026-09-17 版本号订正：上面引的 `sea-orm 2.0.1` 是旧快照 —— **版本真源是
/// `src-tauri/Cargo.lock`**（`sea-orm = 2.0.2`），已改。两版该文件**逐字节相同**
/// （均 927 行，`:47` 恰为 `pub enum DatabaseConnectionType {`，变体清单一致）
/// ⇒ 行号与结论均不变。⚠ 这类缺陷**存在性检查查不出来**：registry 长期留历史版本，
/// 「指向 2.0.1 的路径」照样解析成功，而它证明的已不是我们在用的那份代码。
///
/// ⚠ 2026-09-16：迁移清单已清空（[`MIGRATIONS`] = `&[]`），**上面这段论述已无对象**。
/// 保留它（而不是删掉）是为了让「为什么当初没给迁移包事务」这个结论不随代码消失 ——
/// 它正是下面两件事的成因：版本表按**集合成员**判定而非 `MAX(version)`、
/// 以及 `repair_schema` 那条「失败的迁移下次启动会重试」的承诺。
/// 读这段时要把它当**历史依据**，不要当成待办。
///
/// 当前的实际防护是「**迁移自身幂等**」：`CREATE TABLE IF NOT EXISTS` /
/// `INSERT … ON CONFLICT DO NOTHING` / 先查 PRAGMA 或 information_schema 再 ALTER
/// —— 幂等则重试无害。**若要恢复迁移机制，新增迁移必须沿用这个约定**；
/// 若某条迁移无法写成幂等，它才需要上面那个重构来兜底。
///
/// ⚠ 但在往 [`MIGRATIONS`] 里加回任何一条之前，先读它的文档：迁移与声明式引擎
/// **只能有一个**建表 owner。两者并存时差异（列类型、索引命名 `idx_x_y` vs
/// 引擎的 `idx-x-y`、约束写法）不会在改动当场暴露，只会在某次启动时集中爆发。
pub async fn run_migrations(db: &sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();

    // 1) 确保 version tracking 表存在（ANSI DDL，SQLite/PG 通用）
    db.execute_unprepared(&format!(
        "CREATE TABLE IF NOT EXISTS {SCHEMA_VERSION_TABLE} (\
         version INTEGER NOT NULL PRIMARY KEY, \
         applied_at INTEGER NOT NULL, \
         description TEXT)"
    ))
    .await?;

    // 2) 读已应用版本集合
    //
    // 刻意用「集合成员判定」而不是 `MAX(version)` 高水位线。高水位线有两个缺陷：
    // ① 判不出**中间缺口** —— 后补进清单的中间版本号在既有库上被永久静默跳过
    //    （生产实证见 `missing_versions` 注释）；
    // ② 让 `repair_schema` 里「下次启动 run_migrations 将重试失败的迁移」这句
    //    承诺落空 —— 失败迁移自身没写版本号，但比它大的版本号可能已写入，
    //    `MAX` 于是越过它，永不重试。
    let applied: std::collections::HashSet<i32> =
        read_applied_versions(db).await?.into_iter().collect();

    // 3) 按注册顺序补跑未应用 migration
    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }
        // db.clone() 是 Arc +1，up() 内部 await 时持有一个 owned 副本。
        //
        // 失败时**必须带上版本号**再向上传：裸传 DbErr 会让调用方只看到一句 SQL
        // 错误（如 `字段 "enabled" 的类型为 integer, 但表达式的类型为 boolean`），
        // 却不知道是清单里的哪一条（写下这段时是 68 条）—— 排障要逐个翻文件。归因信息缺失与归因说谎
        // 同样有害（2026-09-12 生产事故：应用弹「数据库初始化失败」，错误串里
        // 完全没有 v101 的踪迹）。
        //
        // 这里刻意**保持 fail-fast 语义**（不改成 warn 吞掉）：迁移失败意味着
        // schema 与代码不匹配，硬撑着启动只会把故障从"起不来"变成更难查的
        // "跑得起来但到处出错"。要做的是让错误**说得清**，而不是让它沉默。
        (m.up)(db.clone()).await.map_err(|e| {
            tracing::error!(
                version = m.version,
                description = m.description,
                error = %e,
                "[run_migrations] 迁移执行失败，数据库初始化中止"
            );
            DbErr::Custom(format!(
                "迁移 v{} 执行失败: {e} —— 该迁移此前未应用过。\
                 若它是首次在 PostgreSQL 上执行，请优先核对 DDL 是否为 PG 兼容语法\
                 （SQLite 专有写法如 AUTOINCREMENT / INSERT OR IGNORE / 布尔字面量插整数列，PG 均不接受）。",
                m.version
            ))
        })?;
        record_version(db, backend, m.version, m.description).await?;
    }

    Ok(())
}

/// 已注册迁移的版本号（升序，含重复）。
///
/// 供启动自愈判定使用：**仅凭 `MAX(version)`（高水位线）判不出「中间缺口」**。
///
/// 生产实证（2026-09-12，axagent PG 库）：源码 68 个版本 vs 版本表 66 行，
/// `applied_max = 227 == CURRENT_VERSION`，而下面两条从未执行 ——
///   - **v101**（`trajectory_*` 合并 + `__sys_trajectory_memory__` sentinel）：
///     三项产物全部缺失（sentinel KB / sentinel NS 无行，旧表
///     `trajectory_entities|memories|relationships` 仍存在）；
///   - **v139**（建 `session_events`）：`to_regclass` 为 NULL，而
///     `DbSessionEventSink::emit` 每次 INSERT 都失败并被 `tracing::warn!` 吞掉
///     ⇒ 事件流持久化 100% 静默降级。
///
/// 成因：`run_migrations` 只看 `MAX(version)`，任何**后补进清单的中间版本号**
/// 在既有库上被永久静默跳过（新库按序跑则正常）。这是「新装/升级不一致」类缺陷。
pub fn registered_versions() -> Vec<i32> {
    MIGRATIONS.iter().map(|m| m.version).collect()
}

/// 「已注册但版本表里没有」的版本号（升序去重）。空 = 真正追平。
///
/// 与 `read_max_version` 的区别：后者是**高水位线**，判不出缺口。
/// 实现刻意复用 `registered_versions()`（而不是再次遍历 `MIGRATIONS`）：
/// 让「注册清单」只有一处推导，避免两个函数日后对清单的理解漂移。
///
/// ⚠ 2026-09-16 迁移清单清空后，本函数**恒返回空**（生产调用点也随之消失）。
/// 保留它（而不是删掉）是为了让「**为什么版本判定必须用集合成员而不是 `MAX(version)`**」
/// 这个结论不随代码消失 —— 它绑着上面那条生产实证（v101/v139 被高水位线永久跳过、
/// 事件流 100% 静默降级）。删掉它，下次有人想「简化」`run_migrations` 时就只剩
/// 无人记得的理由。测试 `missing_versions_is_empty_after_migration_purge` 负责在
/// 有人往 [`MIGRATIONS`] 里加回迁移时变红，提醒他重读本函数与 `run_migrations` 的文档。
pub fn missing_versions(applied: &[i32]) -> Vec<i32> {
    let applied: std::collections::HashSet<i32> = applied.iter().copied().collect();
    let mut out: Vec<i32> =
        registered_versions().into_iter().filter(|v| !applied.contains(v)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// 读出全部已应用迁移的版本号（不排序、不去重）。
///
/// 与 `read_max_version` 并列存在：两者语义不同，别互相替代 ——
/// `read_max_version` 只回答「最高跑到哪」，本函数回答「跑了哪些」。
async fn read_applied_versions(db: &sea_orm::DatabaseConnection) -> Result<Vec<i32>, DbErr> {
    let rows = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            format!("SELECT version FROM {SCHEMA_VERSION_TABLE}"),
        ))
        .await?;
    Ok(rows.iter().filter_map(|r| r.try_get_by::<i32, _>("version").ok()).collect())
}

async fn read_max_version(db: &sea_orm::DatabaseConnection) -> Result<i32, DbErr> {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            format!("SELECT COALESCE(MAX(version), 0) AS v FROM {SCHEMA_VERSION_TABLE}"),
        ))
        .await?;
    match row {
        None => Ok(0),
        Some(r) => {
            // COALESCE 在空表返回 0，因此总能解析为 i32
            let v: i32 = r.try_get_by("v").unwrap_or(0);
            Ok(v)
        },
    }
}

// ── P2-9: Schema 状态查询 ─────────────────────────────────────────────────
//
// 暴露给 Tauri 命令层，让前端可以查询「库结构离代码声明还差多少、谁会去修」。
//
// ⚠ 2026-09-16 起本节的形态变了：迁移清单清空后，旧的「已应用版本 + pending 迁移
// 数量 + 已应用迁移列表」三件套**全部失真**（详见 `SchemaStatus` 的字段文档）。
// 结构面改由只读探测 `reconcile::status::probe` 提供，版本面只剩 legacy 记账。

/// 数据库结构状态（**引擎视角**）。
///
/// ## 为什么替换了 `SchemaMigrationStatus`
///
/// 迁移清单清空后，旧结构的两个字段同时失效：`pending_count` 恒 0（`MIGRATIONS` 为空）、
/// `applied_version` 只剩遗留记账。前端用 `pending_count > 0` 决定显示「滞后」还是
/// 「已是最新」⇒ 它恒走「已是最新」那一支，且那句里的版本号是常量 —— **面板在说谎**。
/// 现在改为把「库结构离声明还差多少、谁会去修」直接报出来。
///
/// 每个结构面字段都来自**只读**探测（`reconcile::status::probe`），不写 DDL、不写审计。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SchemaStatus {
    /// 方言：`"sqlite"` / `"postgres"`。
    pub dialect: String,
    /// 声明侧 / 实况侧的表数。
    pub tables_expected: i32,
    pub tables_actual: i32,
    /// 引擎下次启动会**自动补**的变更条数。
    pub pending_apply: i32,
    /// 纯新增、但本方言**没有原生 DDL** 的条数（已知限制，每轮重现）。
    pub pending_unsupported: i32,
    /// 启动期**刻意不动**、需人工处理的条数（收缩 / 改类型）。
    pub pending_manual: i32,
    /// 声明漂移（advisory）条数 —— 只有表达式文本差异，不产出 DDL。
    pub advisories: i32,
    /// 异常备注（正常为空）。
    pub notes: Vec<String>,
    /// 版本表高水位线 `MAX(version)`。
    ///
    /// ⚠ **遗留记账**：迁移时代的产物，迁移清单已清空。它**不是**「schema 跑到哪了」的
    /// 答案（那个问题现在由上面的结构面字段回答）。保留它只为一件事：
    /// `init/database.rs:231-252` 用 `applied_version > latest_version` 识别「连到了版本号
    /// 更高的下游 fork 库」并触发重型自愈。**UI 不展示** —— 它正是用户要求去掉的那个
    /// 「数据迁移标记」。
    pub applied_version: i32,
    /// 本程序认识的最高版本号（`CURRENT_VERSION`）。含义见 `applied_version`。
    ///
    /// ⚠ `CURRENT_VERSION` **必须保持 233**，不要改成 0。
    pub latest_version: i32,
    /// 结构面探测失败的原因。`Some` ⇒ 上面**结构面**各字段全为 0 且不可信
    /// （含 `dialect`：它是空串，因为方言也由那次失败的探测读出）；
    /// 版本面字段（`applied_version` / `latest_version`）仍然有效。
    pub probe_error: Option<String>,
}

/// P2-9: 查询当前 schema 状态。
///
/// 返回**结构面**（离声明形态还差多少、分别由谁去修）与**版本面**（遗留记账）两组字段。
/// 失败时返回 `DbErr`，调用方（Tauri 命令）转 `String`。
///
/// ⚠ **本函数不再报 `pending_count`**：`MIGRATIONS` 清空后它恒为 0，留着就是 UI 谎报
/// （旧实现在 `pending_count == 0` 时显示「Schema 已是最新 v233」，而那句话既依赖一个
/// 恒真条件、又依赖一个与库状态无关的常量）。「还差多少」现在由 `pending_apply` /
/// `pending_unsupported` / `pending_manual` 三个桶回答，它们的口径见
/// [`crate::reconcile::status::EngineStatus`]。
///
/// 结构面探测失败**不**让整个调用失败：版本面字段仍有效，而 `init/database.rs` 的
/// 「版本超前」判定只依赖版本面 ⇒ 探测失败不该连带让它失效。失败原因进 `probe_error`。
pub async fn get_schema_status(db: &sea_orm::DatabaseConnection) -> Result<SchemaStatus, DbErr> {
    // 1) 版本面：读已应用的最大版本号（legacy 记账，见 `applied_version` 文档）
    let applied_version = read_max_version(db).await?;

    let mut status = SchemaStatus {
        dialect: String::new(),
        tables_expected: 0,
        tables_actual: 0,
        pending_apply: 0,
        pending_unsupported: 0,
        pending_manual: 0,
        advisories: 0,
        notes: Vec::new(),
        applied_version,
        latest_version: CURRENT_VERSION,
        probe_error: None,
    };

    // 2) 结构面：只读探测（不执行 DDL / 不写审计 / 不建元表）
    match crate::reconcile::status::probe(db).await {
        Ok(e) => {
            status.dialect = e.dialect;
            status.tables_expected = e.tables_expected as i32;
            status.tables_actual = e.tables_actual as i32;
            status.pending_apply = e.pending_apply as i32;
            status.pending_unsupported = e.pending_unsupported as i32;
            status.pending_manual = e.pending_manual as i32;
            status.advisories = e.advisories as i32;
            status.notes = e.notes;
        },
        Err(e) => {
            // 必须留痕：结构面字段全 0 时，UI 显示的是「什么都不缺」——
            // 若这里静默，一次探测失败会被读成「库完全健康」。
            tracing::warn!(
                "[get_schema_status] 结构面探测失败（版本面字段仍有效，结构面各字段置 0 且不可信）: {e}"
            );
            status.probe_error = Some(e.to_string());
        },
    }

    Ok(status)
}

/// 修复结果 —— `schema_diff::heal_all` 的**真实产出**。
///
/// ⚠ 这是本函数**唯一**还在做的事。旧版本返回的 `(fixed, total)` 语义是「重跑了几个
/// 迁移」，注册表清空后恒为 `(0, 0)`；命令层把它 `format!("{}", added)` 成 `"0"` 回给
/// 前端，UI 于是永远显示「补了 0 个缺失字段」—— **而 heal_all 真补了多少列从不回传**。
/// 现在直接回传它。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SchemaRepairReport {
    /// **对照完成**的实体表数（表缺失的、对照失败的不计入 —— 见 `errors`）。
    pub tables_scanned: i32,
    /// 补上的列（`表.列`）。
    pub columns_added: Vec<String>,
    /// 修好的列类型错配（`表.列`）。
    pub types_healed: Vec<String>,
    /// 逐实体对照失败的原因（`表: 原始错误`），正常为空。
    ///
    /// 非空 ⇒ 那几张表**没对照完**（`heal_entity` 的列循环遇错即中断，见
    /// `schema_diff::DiffReport::errors`），因此 `columns_added` / `types_healed`
    /// 对它们而言是**不完整**的账，不能读成「它们没有缺列」。
    ///
    /// ⚠ 这是「部分未完成」，与整个调用失败是两件事：前者 `repair_schema` 仍返回
    /// `Ok`（其余表确实修了），由调用方按本字段显示「部分未完成」并列出原因；
    /// 只有连列快照都拿不到（`heal_all` 返回 `Err`）才算整体失败。
    pub errors: Vec<String>,
}

/// 修复数据库结构：调 `schema_diff::heal_all`，以实体声明为权威补缺失列 / 修类型错配。
///
/// ⚠ 2026-09-16 起**迁移清单已清空**（[`MIGRATIONS`] = `&[]`），本函数因此只剩
/// `heal_all` 一件事。旧文档说的「重跑所有注册的迁移」「与 `run_migrations` 不同，
/// 此函数跳过版本号检查、无条件对所有已注册迁移调用 `up()`」—— 那两段现在都无对象，
/// 连同 `for m in MIGRATIONS` 空循环与「强制写入 `CURRENT_VERSION`」的收尾一起删掉了
/// （后者连语义都不再需要：`repair_schema` 不再改版本表，见下）。
///
/// ## 它补的是引擎**够不着**的那两类
///
/// * **列类型错配**（`AlterColumnType`）：不在 `plan::ChangeKind::is_purely_additive`
///   的白名单里 ⇒ 引擎**永远不做**类型修复，启动期也刻意不做（改类型是破坏性变更）。
///   ⚠ 这一条**只在 PG 上成立**：SQLite 没有 `ALTER COLUMN`，那边既做不了也不需要
///   （动态类型下列声明的类型名不影响解码）。
/// * **缺列**（`AddColumn`）：虽然在白名单里、引擎每轮启动已经会补，但本函数是
///   **手动兜底** —— 用户点「修复 Schema」时不必等下一次启动，且它是引擎那条链之外
///   的独立通道（引擎被拒 / 渲染失败时仍有这条）。
///
/// ## 对库的写入面
///
/// 只做两件事：① `CREATE TABLE IF NOT EXISTS` 版本表（`get_schema_status` 要读它，
/// 不含则读报错）；② `heal_all` 的 `ALTER TABLE ADD COLUMN`（双方言）+ `ALTER TABLE
/// ALTER COLUMN … TYPE`（**只在 PG**：SQLite 没有这条 DDL，2026-09-16 起由
/// `schema_diff::heal_entity` 的方言闸挡住，详见那里）。
/// **不再写版本表**：`record_version(CURRENT_VERSION, …)` 那一步已删。这么删是安全的：
/// `init/database.rs` 的触发条件是 `applied_version > latest_version`，只有「版本号
/// 体系更高的下游 fork 库」才成立；而 `record_version` 是 `INSERT OR IGNORE`
/// （只插不覆盖），它能把 `MAX(version)` 抬到 233，却无法把它抬**过** 233
/// ⇒ 它永远不会**造成**那个触发，只可能**阻止**它。删掉它不影响那个判定。
///
/// ⚠ 与旧版相比有一处**刻意的语义变化**：旧版在 `heal_all` 失败时只 `warn!` 然后照样
/// 返回 `Ok`（fail-open 静默降级）。现在把错误**返回出去** —— 这个按钮的全部职责就是
/// `heal_all`，它失败了却报「成功」等于让 UI 显示「已修复」而实际什么都没做。
///
/// **失败分两层，不要合并**：
/// * **整体失败** ⇒ `Err`：连实际列快照都读不出来（连接/权限/元数据查询坏了），
///   此时一件事都没做，调用方必须报错。
/// * **部分未完成** ⇒ `Ok` + [`SchemaRepairReport::errors`] 非空：单个实体的列对照
///   中断（如一条本方言执行不了的 DDL），其余实体照常修完。把它也升级成 `Err` 会
///   让「修好了 180 张表、2 张没查完」被显示成「修复失败」，从而丢掉已完成的那部分；
///   而把它压成 `warn!` 就又回到了 fail-open。所以它必须**随返回值一起出去**。
pub async fn repair_schema(db: &sea_orm::DatabaseConnection) -> Result<SchemaRepairReport, DbErr> {
    // 确保 version tracking 表存在（`get_schema_status` 会读它，不含则 SELECT 报错）。
    // 注意：这里只建**空表**，不再往里写任何版本行。
    db.execute_unprepared(&format!(
        "CREATE TABLE IF NOT EXISTS {SCHEMA_VERSION_TABLE} (\
         version INTEGER NOT NULL PRIMARY KEY, \
         applied_at INTEGER NOT NULL, \
         description TEXT)"
    ))
    .await?;

    // schema diff 层：迁移重跑只能修「缺表/缺数据」，修不了「有表缺列」
    // （CREATE TABLE IF NOT EXISTS 对已存在的表是 no-op）。这里以实体定义为权威
    // 对照实际库列集，缺失即补；同时修类型错配（如 SQLite 方言迁移在 PG 上产出的
    // real 列 vs 实体 f64 的 DOUBLE PRECISION）。
    match schema_diff::heal_all(db).await {
        Ok(report) => {
            if report.errors.is_empty() {
                tracing::info!(
                    "[repair_schema] schema diff: {} 张实体表对照完成，补列 {} 个，类型修复 {} 个",
                    report.tables_scanned,
                    report.columns_added.len(),
                    report.types_healed.len()
                );
            } else {
                // 刻意不说「完成」：这些表的 `columns_added` 是**部分账**
                tracing::warn!(
                    "[repair_schema] schema diff: {} 张实体表对照完成（另有 {} 张**未对照完**，缺失列无从判断），补列 {} 个，类型修复 {} 个；未完成: {:?}",
                    report.tables_scanned,
                    report.errors.len(),
                    report.columns_added.len(),
                    report.types_healed.len(),
                    report.errors,
                );
            }
            Ok(SchemaRepairReport {
                tables_scanned: report.tables_scanned as i32,
                columns_added: report.columns_added,
                types_healed: report.types_healed,
                errors: report.errors,
            })
        },
        Err(e) => {
            tracing::warn!("[repair_schema] schema diff 失败: {e}");
            Err(e)
        },
    }
}

/// 安全网：确保 agency_experts / agent_profiles 的 category CHECK 约束
/// 包含所有业务值。
///
/// ⚠️ AxInvest fork 分歧点（upstream merge 时必须保留，勿被上游版本覆盖）：
/// 上游 AxAgent 的同名函数只含 9 个通用值，因为上游没有 OPC/荐股业务；
/// AxInvest 的 opc_setup 种子（opc_setup/mod.rs seed_opc_experts 等）与
/// stock profile 会写入 `opc-company`/`opc-experts`/`opc-domain_pack`/
/// `opc-domain`/`stock-analysis`，v200 PHASE 3 与 v223 自愈迁移的约束
/// 列表也包含这些值。2026-08-18 后某次上游合并把本函数覆盖回 9 值版，
/// 导致 ADD CONSTRAINT 被存量 opc-* 行顶回（EXPERT_READ_DIR_FAILED 同期
/// 日志：check constraint "agency_experts_category_check" is violated），
/// 且 DROP 先行执行 → 表约束整个缺失。列表必须与 v200 PHASE 3 保持一致。
pub async fn ensure_category_check_constraints(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), DbErr> {
    let is_pg = db.get_database_backend() == DbBackend::Postgres;
    if !is_pg {
        return Ok(());
    }

    let backend = db.get_database_backend();
    let categories = "'general','development','security','data','finance',\
        'devops','design','writing','business',\
        'opc-company','opc-experts','opc-domain_pack','opc-domain','stock-analysis'";

    // 防护：先校验存量数据再动约束。若先 DROP 后 ADD 失败，表会落得
    // 「无任何 category 约束」的裸奔状态（2026-09-06 实测发生过）。
    for table in ["agency_experts", "agent_profiles"] {
        let sql = format!(
            "SELECT category, count(*)::int AS n FROM {table} \
             WHERE category NOT IN ({categories}) GROUP BY category"
        );
        let rows = db.query_all_raw(Statement::from_string(backend, sql)).await?;
        if !rows.is_empty() {
            let mut parts: Vec<String> = Vec::new();
            for r in &rows {
                let cat: String = r.try_get("", "category").unwrap_or_default();
                let n: i32 = r.try_get("", "n").unwrap_or_default();
                parts.push(format!("{cat}({n})"));
            }
            return Err(DbErr::Custom(format!(
                "{table} 存在不被 category CHECK 允许的值，已保留原约束未替换: {}",
                parts.join(", ")
            )));
        }
    }

    // agency_experts
    let _ = db
        .execute_raw(Statement::from_string(
            backend,
            "ALTER TABLE agency_experts DROP CONSTRAINT IF EXISTS agency_experts_category_check",
        ))
        .await;
    db.execute_raw(Statement::from_string(
        backend,
        format!(
            "ALTER TABLE agency_experts ADD CONSTRAINT agency_experts_category_check \
             CHECK (category IN ({categories}))"
        ),
    ))
    .await?;

    // agent_profiles
    let _ = db
        .execute_raw(Statement::from_string(
            backend,
            "ALTER TABLE agent_profiles DROP CONSTRAINT IF EXISTS agent_profiles_category_check",
        ))
        .await;
    db.execute_raw(Statement::from_string(
        backend,
        format!(
            "ALTER TABLE agent_profiles ADD CONSTRAINT agent_profiles_category_check \
             CHECK (category IN ({categories}))"
        ),
    ))
    .await?;

    tracing::debug!("[schema] category CHECK 约束已确认");
    Ok(())
}

async fn record_version(
    db: &sea_orm::DatabaseConnection,
    backend: DbBackend,
    version: i32,
    description: &str,
) -> Result<(), DbErr> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

    // 参数化查询：避免 format! 拼接 SQL 带来的注入风险与转义负担。
    // SQLite 用 `INSERT OR IGNORE`；PostgreSQL 用 `ON CONFLICT DO NOTHING`
    // （二者语义等价：版本号冲突时静默跳过，保证幂等）。
    // 注：表名是编译期常量 `SCHEMA_VERSION_TABLE`，非用户输入，用 format! 拼接安全。
    let stmt = if backend == DbBackend::Postgres {
        Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!(
                "INSERT INTO {SCHEMA_VERSION_TABLE} (version, applied_at, description) \
                 VALUES ($1, $2, $3) ON CONFLICT (version) DO NOTHING"
            ),
            [version.into(), now.into(), description.into()],
        )
    } else {
        Statement::from_sql_and_values(
            DbBackend::Sqlite,
            format!(
                "INSERT OR IGNORE INTO {SCHEMA_VERSION_TABLE} (version, applied_at, description) VALUES (?, ?, ?)"
            ),
            [version.into(), now.into(), description.into()],
        )
    };
    db.execute_raw(stmt).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ═══════════════════════════════════════════════════════════════════════
    // 以下 4 条是「版本化迁移」清除后的判据集合（2026-09-16，P6）。
    //
    // 改判原则：**换被测对象，不换判据**。旧判据是「迁移跑完之后这些表/索引在不在」，
    // 其中迁移只是**手段**，「这些表/索引在不在」才是产品级不变量 ⇒ 手段换了，
    // 不变量继续测。这与第 2 步删掉的那些不同：那些的被测对象（逐条迁移的幂等性、
    // 注册表唯一性、中间缺口补跑）在产品意义上已经不存在，留着只能靠伪造前提通过。
    //
    // * `fresh_db_via_init_chain_has_core_tables_and_no_dead_tables` —— 改判（3 条之一）
    // * `l2_declared_indices_reach_ddl_on_engine_built_db`           —— 改判（3 条之二）
    // * `repair_schema_on_converged_db_reports_no_heal`              —— 改判（3 条之三）：
    //   顶替 `repair_schema_records_current_version`，后者被测对象已消失，删因见下
    // * `missing_versions_is_empty_after_migration_purge`            —— 新增：防回流棘轮
    // ═══════════════════════════════════════════════════════════════════════

    /// 全新库经**完整初始化链**后，核心业务表必须存在，且死表不得被建出来。
    #[tokio::test]
    async fn fresh_db_via_init_chain_has_core_tables_and_no_dead_tables() {
        // 走 `create_test_pool` 而不是自己连 in-memory 库：它走的是与生产**同一条**
        // `initialize_schema`（版本表记账 → 引擎建表 → 哨兵播种）。手连 in-memory
        // 只覆盖链条的一段，会让「链上少了一步」这类缺陷从判据里漏掉。
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        for table in &[
            "messages",
            "conversations",
            "providers",
            "provider_keys",
            "gateway_keys",
            "gateway_usage",
            SCHEMA_VERSION_TABLE,
        ] {
            let row = db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                    [(*table).into()],
                ))
                .await
                .expect("测试应成功");
            assert!(row.is_some(), "table {} should exist", table);
        }

        // ⚠ 死表判据的**依据变了**（且更强了）：旧版靠「v003 把它们 DROP 过」，
        // 现在靠「它们不在任何实体/L2 声明里，引擎根本不认识它们」——
        // 不再依赖某条迁移被执行过。
        for dead in &["categories", "apps", "context_packs"] {
            let row = db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                    [(*dead).into()],
                ))
                .await
                .expect("测试应成功");
            assert!(row.is_none(), "dead table {} should not be created", dead);
        }
    }

    /// L2 声明的索引必须**逐字**落到 DDL 上（多列 + partial 两条形态）。
    ///
    /// 这条覆盖的是**声明 → plan → render → apply** 整条链在「非单列索引」上的落地。
    /// 此前这条链只有 PG 侧探针覆盖，单测里没有 ⇒ 「L2 声明写错了列」这类缺陷
    /// 只在某次改 PG 时才暴露。
    ///
    /// ⚠ 刻意**不**断言 `idx_conversations_updated` / `idx_provider_keys_provider`
    /// 这两个旧名字 —— 是登记过的决定，不是遗忘：
    /// 1. 索引命名有两套约定。迁移用 `idx_{表}_{列}`（且带 `DESC` 方向），而实体
    ///    `#[sea_orm(indexed)]` 派生的是 **`idx-{表}-{列名原文}`**（sea-orm 硬编码，
    ///    `sea-orm-2.0.2/src/schema/entity.rs:158` 的
    ///    `format!("idx-{}-{}", entity.to_string(), column.to_string())`）。
    ///    注意「列名原文」含下划线 ⇒ `updated_at` 派生为
    ///    **`idx-conversations-updated_at`**（连字符与下划线混写，看着像笔误但不是）。
    /// 2. 两个旧名在存量库上的归宿**已被真库探针定论**
    ///    （`output/tmp-p3-probe-prod2.log`，非推测）：
    ///    - `idx_provider_keys_provider` → **RENAME** 成 `idx-provider_keys-provider_id`
    ///      （列集/unique/method 一致，只差名字 ⇒ 改名匹配成立，**无语义损失**）。
    ///    - `idx_conversations_updated`（实况 `cols[updated_at DESC]`）→ **DROP**。
    ///      成因是**方向**而非名字：实况是 `DESC`，实体声明里没有表达方向的维度
    ///      ⇒ 派生出的新索引是升序。⚠ 这不是「换个名字」：`ORDER BY updated_at DESC`
    ///      的查询会失去有序索引。该语义差**无法**用列标志承接（`expected.rs` 里
    ///      没有「索引方向」这一维），已记入 PLAN 待办，本轮不下判断。
    #[tokio::test]
    async fn l2_declared_indices_reach_ddl_on_engine_built_db() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        for idx in &["idx_messages_conv_created", "idx_gateway_usage_key", "idx_messages_branch"] {
            let row = db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "SELECT name FROM sqlite_master WHERE type='index' AND name=?",
                    [(*idx).into()],
                ))
                .await
                .expect("测试应成功");
            assert!(
                row.is_some(),
                "L2 声明的索引 {} 没落到 DDL 上 —— 声明与实际建出的对象脱节",
                idx
            );
        }

        // 实体 `#[sea_orm(indexed)]` 派生的名字是 `idx-{表}-{列名原文}`（sea-orm 硬编码，
        // `sea-orm-2.0.2/src/schema/entity.rs:158`）。取两个名字是为了覆盖
        // **列名里有没有下划线**这两态 —— 假设有人按「名字=全连字符」的直觉去改
        // 派生逻辑，`updated_at` 那一条会当场红（`status` 那一条不会）。
        //
        // ⚠ 刻意把**字面名写死**，不改成「从 `expected::build` 读出来再比对」：读期望模型
        // 会让「有人删掉了 `#[sea_orm(indexed)]` 属性」这类回归在**两侧同时消失** ⇒
        // 判据自证化。字面名锚的是「实况库里真有这个对象」。
        for idx in &["idx-conversations-updated_at", "idx-stock_analyses-status"] {
            let row = db
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "SELECT name FROM sqlite_master WHERE type='index' AND name=?",
                    [(*idx).into()],
                ))
                .await
                .expect("测试应成功");
            assert!(
                row.is_some(),
                "实体 `#[sea_orm(indexed)]` 应派生出索引 {}（列名原文含下划线，故是\
                 连字符+下划线混写），但引擎没建出来 —— 声明与实际建出的对象脱节",
                idx
            );
        }
    }

    /// `trajectory_patterns` 插一行 —— 只把 `id`/`name` 参数化，其余 9 列按实体声明的类型补常量。
    ///
    /// 刻意走 `execute_raw` 而**不**走 `ActiveModel`：本判据要观测的是**库上的约束**，
    /// 过一遍实体层会把 `ActiveModel` 的默认值/校验混进观测面 —— 那时红了也分不清
    /// 是约束没生效还是实体层拦下的。
    async fn insert_pattern(
        db: &sea_orm::DatabaseConnection,
        id: &str,
        name: &str,
    ) -> Result<(), DbErr> {
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO trajectory_patterns \
             (id, name, description, pattern_type, trajectory_ids, frequency, \
              success_rate, average_quality, average_value_score, reward_profile, created_at) \
             VALUES (?, ?, '', '', '', 1, 0.0, 0.0, 0.0, '', '')",
            [id.into(), name.into()],
        ))
        .await?;
        Ok(())
    }

    /// 排除式部分唯一索引必须在**真库**上按谓词生效 —— 「对象建出来了」不等于「语义对了」。
    ///
    /// 为什么单列一条、不并进 `l2_declared_indices_reach_ddl_on_engine_built_db`：那条只断言
    /// 对象**存在**（`sqlite_master` 里查得到）。对 `uq_trajectory_patterns_name` 而言
    /// 「存在」是最弱的一档，它真正要挡的是 C-#2 —— 声明里 `dialect` 被写成
    /// `Some(Postgres)` 时**SQLite 期望集整条消失**，引擎于是根本不建它（不是建错，是建都不建），
    /// 而本表 `name` 列没有 `#[sea_orm(unique)]` ⇒ 唯一性只剩这一条载体。
    ///
    /// ⚠ 必须带**反例**，否则本判据挡不住「谓词丢了」：只测「同名第二行被挡下」时，
    /// 任何一条**整表唯一**的索引都能让它通过 —— 而那正是丢谓词的退化形态
    /// （把 `rl_checkpoint:%` 的同名多行也一起挡掉，而检查点**需要**同名多行）。
    /// 所以第三段反着测：被谓词排除的前缀，同名多行**必须**插得进去。
    ///
    /// ⚠ 它**不**替代生产库验证：本判据跑的是 `create_test_pool` 建出的 SQLite
    /// （与生产同一条 `initialize_schema` 链，但**是另一个库文件**）。
    /// 「生产 SQLite 库里真建出了这条索引」仍属未验证项。
    #[tokio::test]
    async fn partial_unique_index_excludes_checkpoint_names_on_engine_built_sqlite() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        // 1) DDL 形态：谓词必须真的出现在 `sqlite_master.sql` 里。
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?",
                ["uq_trajectory_patterns_name".into()],
            ))
            .await
            .expect("测试应成功")
            .expect(
                "SQLite 上没建出 uq_trajectory_patterns_name —— 该声明 `dialect: None` 被改成 \
                 `Some(..)` 时的症状就是这个：本方言期望集整条消失，且**无任何报错**（缺陷 C-#2）",
            );
        let ddl: String = row.try_get("", "sql").unwrap_or_default();
        assert!(
            ddl.contains("UNIQUE INDEX")
                && ddl.contains("WHERE")
                && ddl.contains("NOT LIKE 'rl_checkpoint:%'"),
            "SQLite 上的 DDL 少了排除谓词 ⇒ 退化成**整表唯一**（连 `rl_checkpoint:%` 的同名多行 \
             也会被挡）。实际 DDL: {ddl}"
        );

        // 2) 正例：非前缀重名必须被挡。
        insert_pattern(db, "p1", "pattern-a").await.expect("首行应插入成功");
        assert!(
            insert_pattern(db, "p2", "pattern-a").await.is_err(),
            "非 `rl_checkpoint:` 前缀的同名第二行**必须**被唯一约束挡下 —— 它插得进去说明 \
             SQLite 侧这条索引没生效"
        );

        // 3) 反例：被谓词排除的前缀**必须**允许同名多行（这才是「排除式」的全部意义）。
        insert_pattern(db, "c1", "rl_checkpoint:epoch-1").await.expect("检查点首行应插入成功");
        insert_pattern(db, "c2", "rl_checkpoint:epoch-1")
            .await
            .expect("`rl_checkpoint:%` 被 WHERE 排除在索引之外 ⇒ 同名多行必须允许");
    }

    /// 迁移清单**必须保持为空** —— 有人加回迁移时这条会红。
    #[test]
    fn missing_versions_is_empty_after_migration_purge() {
        assert!(
            registered_versions().is_empty(),
            "`MIGRATIONS` 被加回了东西（实测 {:?}）—— 这与「引擎是唯一建表来源」冲突。\
             请先重读 `MIGRATIONS` / `run_migrations` / `missing_versions` 三处文档：\
             迁移与声明式引擎只能有一个建表 owner，且它们对版本判定的口径不同。",
            registered_versions()
        );
        assert!(
            missing_versions(&[100, 200, 233]).is_empty(),
            "注册表为空 ⇒ 不存在「已注册但未应用」的版本号；\
             若这里非空，说明 `missing_versions` 的推导不再只依赖 `registered_versions()`"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // `repair_schema_records_current_version` 为什么被删（2026-09-16）
    //
    // 它钉住的是「`repair_schema` 必须把 `CURRENT_VERSION` 写进版本表」，理由是
    // 「否则 `init/database.rs` 的『版本超前』判定会把存量库误判成下游 fork 库并每轮
    // 跑全量自愈」。**这个理由是反的**：那个判定的触发条件是
    // `applied_version > latest_version`，而 `record_version` 是 `INSERT OR IGNORE`
    // （`ON CONFLICT DO NOTHING`），它只能把 `MAX(version)` 抬到 233、不可能抬**过**
    // 233 ⇒ 它永远不会**造成**那个触发，只可能**阻止**它。
    //
    // 更关键的是**被测对象已经消失**：迁移清单清空后，「`repair_schema` 写版本表」这一步
    // 的全部意义就是「让 `get_schema_status` 报出 0 pending」，而 `pending_count` 这个
    // 字段本身已随旧结构一起删除（`MIGRATIONS` 为空时它恒 0）。按本项目纪律
    // 「被测对象消失的用例只能靠伪造前提通过 ⇒ 必须删掉，而不是放宽」，故删。
    //
    // 它留下的那半条真结论（「版本表里是 255 的库进来时不要被误判」）现在由
    // [`CURRENT_VERSION`] 保持不变（233）来承担，见该常量的文档。替身是下面的
    // `repair_schema_on_converged_db_reports_no_heal`：**换被测对象**（`repair_schema`
    // 的真职责从「记账」变成 `heal_all`），**不换判据**（它必须真做了事，且不该报出
    // 不存在的问题）。
    // ═══════════════════════════════════════════════════════════════════════

    /// `repair_schema` 在**已收敛的库**上必须报出「没补任何列」。
    ///
    /// 这条同时钉住三件事：
    /// 1. **全部实体表都对照完成** —— 断言值取自 `expected::expected_table_count`
    ///    （与引擎 `expected::build` 同源），**不写死 183**：那个数是「此刻实体集」
    ///    的快照，会和实体集一起腐烂（加/删一个实体就过期）。这一条同时守住
    ///    「实体集与库表集同步」：少一张 ⇒ 某实体表不存在或该实体的对照被中断。
    /// 2. **没有实体对照失败**（`errors` 为空）—— 2026-09-16 定因后补的：
    ///    当天实测 `tables_scanned=31`（而非 183）的成因不是「表缺了」，而是
    ///    `heal_entity` 发出 SQLite 不支持的 `ALTER COLUMN` 后中断、失败被 `warn!`
    ///    吞掉。**只断言 `columns_added.is_empty()` 是拦不住它的** —— 中断的表的
    ///    补列账恰好也是空的，于是「没查完」长成了「没问题」的样子。第 2 条把
    ///    「测量中断」与「测量结果为 0」分开。
    /// 3. **引擎自建的库里没有列级缺口** —— 这正是 `probe` 报 `pending_apply == 0`
    ///    的同一件事的另一条独立通道（那边靠 diff，这边靠实体逐列对照）。
    ///
    /// ⚠ 第 1 条原来写的是 `tables_scanned > 0`，**太弱**：它拦不住「183 张里只对照完
    /// 31 张」（31 > 0 为真）。当时把它写成反空真断言是对的、写强到「等于期望表数」还
    /// 需要第 2 条同时到位才不至于把两件事混成一条。
    ///
    /// ⚠ 若这条红了：**不要放宽断言**。
    /// * 第 1 条红 ⇒ 把「期望表名集合 − 实际被对照的表名集合」逐条列出来报回，**不许**
    ///   退回 `> 0`、也不许改成 `>= 150` 之类的软阈值：少的那张要么真不存在，要么它的
    ///   对照被中断，两种都是要归因的真发现。
    /// * 第 3 条红 ⇒ `heal_all` 报出了 `columns_added` / `types_healed` 非空，说明引擎
    ///   建出来的库与实体声明的列集/类型之间**真有**差异（引擎漏建列，或两条通道对同一
    ///   份实体得出不同结论），把实际值原样贴出来归因。
    #[tokio::test]
    async fn repair_schema_on_converged_db_reports_no_heal() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        let report = repair_schema(db).await.expect("测试：修复应成功");

        println!(
            "[repair_schema] tables_scanned={} columns_added={:?} types_healed={:?} errors={:?}",
            report.tables_scanned, report.columns_added, report.types_healed, report.errors
        );

        // 1) 全部实体表都对照完成。期望值取自派生函数（与 `expected::build` 同源），
        //    不是手抄的 183 —— 手抄数会和实体集一起腐烂。
        assert_eq!(
            report.tables_scanned as usize,
            crate::reconcile::expected::expected_table_count(
                crate::reconcile::extras::Dialect::Sqlite
            ),
            "对照完成的实体表数应等于本方言的期望表数 —— 少一张说明该实体表不存在，\
             或该实体的对照被中断（后者看下一条）；多一张说明计数口径漂了"
        );
        // 2) 一条都不许「没查完」。必须与第 1 条并存：`errors` 非空时第 1 条也会红，
        //    但两者归因不同（前者是「数对不上」，这里是「数恰好对上但有一张是中断的」）。
        assert!(
            report.errors.is_empty(),
            "有实体未能对照完成：{:?} —— 那些表的列对照是**中断**的，`columns_added` \
             对它们而言只是部分账，不能读成「没有缺列」。先看这些错误的原始 SQL",
            report.errors
        );
        assert!(
            report.columns_added.is_empty(),
            "引擎刚建完的库里 `heal_all` 仍补了列：{:?} —— 两条列声明通道结论不一致",
            report.columns_added
        );
        assert!(
            report.types_healed.is_empty(),
            "引擎刚建完的库里 `heal_all` 仍修了类型：{:?} —— 两条类型声明通道结论不一致",
            report.types_healed
        );
    }
}
