// SPDX-License-Identifier: AGPL-3.0-only

use std::path::PathBuf;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, ConnectionTrait, Database, DbBackend, DbErr,
    EntityTrait, QueryFilter, Set, Statement,
};
use tracing::{info, warn};

use crate::repo::provider;
use axagent_entities::providers;
use axagent_harness::core_error::Result;
use axagent_harness::types::*;
use axagent_harness::util_fns::now_ts;

// 再导出 sea-orm 的连接类型，使 axagent-harness 可以基于此定义 Persistence trait，
// 消费者（agent/tools/runtime）只需 `use axagent_harness::DatabaseConnection`，
// 无需在自己的 Cargo.toml 中直接依赖 sea-orm。
pub use sea_orm::DatabaseConnection;

pub struct DbHandle {
    pub conn: DatabaseConnection,
    pub path: String,
}

impl axagent_harness::Persistence for DbHandle {
    fn connection(&self) -> &axagent_harness::DatabaseConnection {
        &self.conn
    }

    fn db_path(&self) -> &str {
        &self.path
    }
}

/// 连接串 → `(实际 URL, 是否 SQLite)`。**纯函数**，好让方言分派可被单测守住。
///
/// ## 为什么必须把它提出来（2026-09-16 实测缺陷）
///
/// 原先这段是内联的，判据是 `is_sqlite = db_path.starts_with("sqlite:")`，而 `else`
/// 分支把**其余任何输入**都拼成 `sqlite:{}?mode=rwc`。两者不一致：
///
/// * `create_pool_for_profile`（多 profile 模式）传的是**裸文件路径**
///   （`default_db_path()` → `…/profiles/<name>/data/axagent.db`）⇒ 连接真的落在 SQLite，
///   但 `is_sqlite` 是 `false` ⇒ 下面那整段 PRAGMA（`foreign_keys=ON` / WAL /
///   `busy_timeout` / `synchronous` / `cache_size`）**被跳过**。`foreign_keys=ON` 尤甚：
///   跳过它等于库自己的外键约束静默失效。
/// * 空串 ⇒ URL 变成 `sqlite:?mode=rwc`，而 SQLite 把**空文件名**当**临时库**：写进去的
///   数据随后消失。此时既不报错也不落盘 —— 是「数据不知去哪了」那一类。
/// * 拼错的 scheme（`mysql://…`）同样落进 else，被拼成 `sqlite:mysql://…?mode=rwc`。
///
/// ## 语义（"用户设置里选 sqlite 还是 postgresql"，两边同等一等公民）
///
/// | 输入 | URL | is_sqlite |
/// |---|---|---|
/// | `postgres://…` / `postgresql://…` | 原样 | `false` |
/// | `sqlite:…` | 追加 `?mode=rwc`（已带 `?` 则不加） | `true` |
/// | 其它**非空**且不含 `://`（裸文件路径 / 裸文件名） | `sqlite:<path>?mode=rwc` | `true` |
/// | 空 / 纯空白 | —— **报错** | |
/// | 含 `://` 但不是上面两种 scheme | —— **报错** | |
///
/// 后两行是**刻意不猜**：此处退回 SQLite 会把「连错库 / 没读到配置」伪装成「连上了」，
/// 而这两种事故的表现都是「数据不见了」。
///
/// 返回 `DbErr` 而不是 harness 的 `AxAgentError`：本函数是**纯字符串分类**，与 DB 层无关；
/// 调用方 `?` 一下就能升成 `AxAgentError`（`From<DbErr>` 已实现）。
fn resolve_db_url_from_path(db_path: &str) -> std::result::Result<(String, bool), DbErr> {
    let raw = db_path.trim();
    if raw.is_empty() {
        return Err(DbErr::Custom(
            "db_path 是**空串** —— 不猜。空串会被 SQLite 当成**临时库**（写入的数据随后消失），\
             静默接受它等于把「配置没读到」变成「数据不知去哪了」。请显式给出 \
             `postgres://…` / `sqlite:…` / 一个数据库文件路径。"
                .into(),
        ));
    }
    if raw.starts_with("postgres://") || raw.starts_with("postgresql://") {
        return Ok((raw.to_string(), false));
    }
    if raw.starts_with("sqlite:") {
        // 已带查询串（如调用方自己写了 `?mode=rwc`）⇒ 不再追加，避免出现两个 `?`。
        if raw.contains('?') {
            return Ok((raw.to_string(), true));
        }
        return Ok((format!("{raw}?mode=rwc"), true));
    }
    if let Some(scheme) = raw.split("://").next().filter(|_| raw.contains("://")) {
        return Err(DbErr::Custom(format!(
            "不支持连接的 scheme `{scheme}`（只有 `postgres://` / `postgresql://` / `sqlite:`）。\
             此处**刻意不退回 SQLite**：退回会把「连错库」伪装成「连上了」。"
        )));
    }
    // 裸文件路径 ⇒ SQLite 文件库。⚠ **必须同时把 is_sqlite 置真**，否则 PRAGMA 段跳过
    // （这一条正是上面记录的缺陷：多 profile 模式下外键约束静默失效）。
    Ok((format!("sqlite:{raw}?mode=rwc"), true))
}

/// 建表 + 声明式收敛的**唯一入口**。
///
/// ## 为什么必须只有一份（2026-09-16 实测缺陷）
///
/// 此前两个调用方各自写了一份初始化：
///
/// | 调用方 | 做过的事 |
/// |---|---|
/// | [`create_pool`]（生产 / 降级） | `ddl::run_initialization` + 声明式收敛 |
/// | [`create_test_pool`]（38 处测试） | **只有** `ddl::run_initialization` |
///
/// 在「迁移还在」的年代两者等价（迁移能建出全部表）。但**迁移清单清空后**，测试侧会
/// **一张业务表都没有** —— 而它是 38 个调用点（`company-runtime` / `trajectory` /
/// `analysis-engine` / `dao/tests/*` …）的地基。这类「两份实现、只同步了一份」的缺陷
/// 不会在改动当场暴露，会在删迁移那一刻集中爆发。
///
/// ## 两个方言同等一等公民
///
/// 方言**不由这里决定**：`db_path` 由用户在设置里选（`resolve_db_url_from_path`），
/// 这里只负责让「不管连的是哪个库」都走同一条建表链。收敛本身由
/// `reconcile::apply::bootstrap_schema` → `dialect_of(conn)` 读连接自身的 backend，
/// **任何硬编码方言的写法在这里都是缺陷**。
///
/// ## 被拒即中止启动
///
/// 与 `run_initialization` 报错同语义：迁移清空后引擎是唯一的建表来源，被拒意味着库形态
/// 与本版本代码不匹配 —— 硬撑着启动只会把「起不来」变成更难查的「跑得起来但到处出错」。
///
/// ## ⚠ 但「本方言没原生 DDL」**不是**被拒
///
/// 2026-09-16 改判：`SET DEFAULT` / `ADD FK` / `ADD CHECK` / `ADD UNIQUE` 在 SQLite 上
/// 没有原生 `ALTER TABLE` 形态，`apply` 现在把它们记成
/// [`SkipReason::UnsupportedDialect`]（只跳过本条），**不再**凑成 `refusal`。
/// 改判的直接证据是本函数自己：`create_test_pool` 会走同一条链，而这些条目会让
/// **每一个**迁移建出的 SQLite 库（含真实用户的 `.axagent/data/axagent.db`）启动中止。
/// 原因与代价见 `reconcile::apply::bootstrap_schema` 的文档。
async fn initialize_schema(conn: &DatabaseConnection) -> Result<()> {
    // 历史迁移：负责**存量库的一次性数据搬迁**（知识图谱合并、回填、改名…），
    // 那是实体声明表达不了的逻辑。迁移清单清空后本行成为 no-op（数组为空），
    // 但**不能删**：它是「老库升级」的唯一落点。
    crate::ddl::run_initialization(conn).await?;

    // 声明式收敛 —— 「引擎接管建表」的落点，见 `reconcile::apply::bootstrap_schema`。
    // 必须在 `seed_builtin_providers` **之前**：播种要往表里写行，表得先存在。
    let out = crate::reconcile::apply::bootstrap_schema(conn).await?;
    if let Some(r) = &out.refusal {
        return Err(DbErr::Custom(format!(
            "声明式 schema 收敛被拒（{}）：{} | 计划条目 {} 条、已执行 {} 条。\
             被拒的三类是熔断、缺导出证据、渲染器真报错（payload 与 kind 不匹配）；\
             若确实是渲染器缺陷，那是代码问题，不能靠重试绕过。",
            r.kind_str(),
            r.reason(),
            out.items.len(),
            out.executed
        ))
        .into());
    }
    // ⚠ 必须打 WARN 而不是静默跳过：这几条会在**每一轮**启动时重现，而「没被执行」
    // 若不出现在日志里，运维就只能靠「为什么这个库的表少一个约束」去反推。
    let unsupported = out.unsupported();
    if !unsupported.is_empty() {
        warn!(
            "声明式 schema 收敛：{} 条无法在 {} 上表达（只跳过，不阻断启动）——\
             该库结构不会因这几条而收敛，直到实现重建表流程（PLAN §3.1）。前几条：{}",
            unsupported.len(),
            crate::reconcile::apply::dialect_of(conn)
                .map(|d| format!("{d:?}"))
                .unwrap_or_else(|_| "(未知方言)".into()),
            unsupported
                .iter()
                .take(8)
                .map(|i| format!("{} {}", i.kind.as_str(), i.object))
                .collect::<Vec<_>>()
                .join("、")
        );
    }
    // ⚠ **执行期失败（`aborted`）必须单独可见** —— 它比上面那条 `unsupported` 更严重。
    //
    // 判据 #493：`fail-stop` 只接在 `refusal` 上（它在上面被转成 `Err`），执行期的 `aborted`
    // **不阻断启动**。不阻断是**有意**的（理由见下方注释），但「不阻断」若同时「不可见」，
    // 就成了 fail-open —— 库停在「改了一半」的状态，而启动日志里一个字都没有。
    if let Some(a) = &out.aborted {
        warn!(
            "声明式 schema 收敛**未完成**：{a} | 其后 {} 条被跳过（AbortedAfterError），\
             失败那条本身也未生效 ⇒ 本库结构本轮只改到失败点为止。\
             影响面与上一条不同：`UnsupportedDialect` 只影响它自己，这一类会牵连其后全部条目。\
             排查：审计表有逐条 error 原文（run_id={}）；\
             `cargo run -p axagent-dao --example p5_engine_takeover -- --diff <db_url>` 可复现。\
             常见成因：把唯一约束加到**已有重名行**的存量表上（先清存量、再建约束）。",
            out.aborted_skips().len(),
            out.run_id
        );
    }
    // 为什么**不**把 `aborted` 也转成 `Err`（不做 fail-stop）：
    // `refusal` 表达的是「本版本代码与库形态**根本不匹配**」（熔断 / 缺导出证据 / 渲染器真报错），
    // 那是启动前就该拦下的；而 `aborted` 多由**存量数据**触发（唯一索引撞上重名行即典型），
    // 拦下来只会把「结构没收敛完」升级成「应用起不来」，对存量库（含移动端）代价过大。
    // 换成 warn + 带上「牵连条数」，运维才有手工收敛的落点。
    if out.executed > 0 {
        info!(
            "声明式 schema 收敛：{}/{} 条纯新增变更已应用（run_id={}）",
            out.executed,
            out.items.len(),
            out.run_id
        );
    }

    // 持久初始行 —— 「表建好之后还必须存在的那几行」。
    // 必须在 `bootstrap_schema` **之后**（表得先存在），且必须在这条链上而不是
    // `create_pool` 里：本函数是生产与测试**共用**的建表链（`create_test_pool`
    // 也走这里），把它挂在链上才能保证「凡是库建出来过，哨兵就在」。
    // 具体是谁、为什么缺了会静默返空，见 `crate::seed` 的模块文档。
    crate::seed::ensure_sentinels(conn).await?;
    Ok(())
}

/// **生产/测试入口**：连库 + **建表 + 声明式收敛**。
///
/// ⚠ **本函数会改库**（`initialize_schema` 真执行 DDL）。凡宣称「零写入 / 只读」的调用方
/// （探针、审计脚本）**不得**用它 —— 用 [`connect_without_initialization`]。此前几处探针
/// 借它取连接，在 `initialize_schema` 落地后那句承诺就变成了假话。
///
/// 方言由 `db_path` 决定（用户在设置里选），见 [`resolve_db_url_from_path`]。
pub async fn create_pool(db_path: &str) -> Result<DbHandle> {
    let (url, is_sqlite) = resolve_db_url_from_path(db_path)?;

    let mut opt = ConnectOptions::new(&url);
    // 8 → 20：主连接池被后台任务（实体提取/索引/RAG 预热等慢操作）与命令共享，
    // 8 连接在实体提取重载时被占满（15s acquire_timeout 后命令失败，见 index_queue WARN）。
    // PG 默认 max_connections=100，20 安全；SQLite 为单文件读写，不受连接数影响。
    opt.max_connections(20)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .sqlx_logging(false);

    let conn = Database::connect(opt).await?;

    // SQLite 专有的完整性检测与 PRAGMA；PostgreSQL 跳过。
    if is_sqlite {
        // 数据库完整性检测与自动恢复（在 PRAGMA 和迁移之前运行）
        crate::integrity::auto_recover(&conn, &url).await?;

        conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA journal_mode=WAL;"))
            .await?;
        conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA foreign_keys=ON;"))
            .await?;
        conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA busy_timeout=5000;"))
            .await?;
        conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA synchronous=NORMAL;"))
            .await?;
        conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA cache_size=-64000;"))
            .await?;
        conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA temp_store=MEMORY;"))
            .await?;
    }

    // 建表 + 收敛（唯一入口，见 `initialize_schema` 的文档）
    initialize_schema(&conn).await?;

    // Seed built-in providers
    seed_builtin_providers(&conn).await?;

    // 数据迁移：硬编码路径 → 模板变量
    // (注：path_vars 迁移在 init/database.rs 中由调用方负责)
    crate::repo::local_tool::migrate_legacy_keys(&conn).await;

    // 注意：预设模板不再在启动时自动播种。
    // 工作流模板按需导入，通过前端工作流管理页面的"从预设导入"按钮触发 seed_preset_templates Tauri 命令。

    info!("Database initialized at {}", db_path);
    Ok(DbHandle { conn, path: db_path.to_string() })
}

pub fn default_db_path() -> String {
    #[cfg(mobile)]
    let home = dirs::data_dir()
        .or_else(dirs::home_dir)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .or_else(|| std::env::var("ANDROID_DATA").ok())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| {
            tracing::warn!("Could not determine home directory for DB path, using current dir");
            PathBuf::from(".")
        });
    #[cfg(not(mobile))]
    let home = dirs::home_dir().unwrap_or_else(|| {
        tracing::warn!("Could not determine home directory for DB path, using current dir");
        PathBuf::from(".")
    });

    let path = home.join(".axagent").join("data").join("axagent.db");
    path.to_string_lossy().to_string()
}

pub fn profile_db_path(profile_name: &str) -> String {
    #[cfg(mobile)]
    let home = dirs::data_dir()
        .or_else(dirs::home_dir)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .or_else(|| std::env::var("ANDROID_DATA").ok())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| {
            tracing::warn!("Could not determine home directory for DB path, using current dir");
            PathBuf::from(".")
        });
    #[cfg(not(mobile))]
    let home = dirs::home_dir().unwrap_or_else(|| {
        tracing::warn!("Could not determine home directory for DB path, using current dir");
        PathBuf::from(".")
    });

    let path =
        home.join(".axagent").join("profiles").join(profile_name).join("data").join("axagent.db");
    path.to_string_lossy().to_string()
}

/// **只读连接**：连库但**不**跑迁移、**不**跑声明式收敛。
///
/// 存在的唯一理由是让「零写入」这类承诺**可兑现**：`create_pool` 是生产入口，它**会**
/// 建表与收敛；探针（`p3_plan_probe` / `p4_apply_probe` 的只读模式）若借它取连接，
/// 就会在声称「本次只发 SELECT」的同时真改了库。契约分叉 ⇒ 两个函数。
///
/// ⚠ 连上之后**什么都没建**，所以调用方不应假设业务表存在。这个「不做」是刻意的：
/// 只读探针要看的正是**库现在的样子**，而不是被初始化改写过的样子。
pub async fn connect_without_initialization(db_path: &str) -> Result<DbHandle> {
    let (url, _is_sqlite) = resolve_db_url_from_path(db_path)?;
    let mut opt = ConnectOptions::new(&url);
    opt.max_connections(2).min_connections(1).sqlx_logging(false);
    let conn = Database::connect(opt).await?;
    Ok(DbHandle { conn, path: db_path.trim().to_string() })
}

pub async fn create_pool_for_profile(profile_name: &str) -> Result<DbHandle> {
    let db_path = if profile_name == "default" {
        default_db_path()
    } else {
        profile_db_path(profile_name)
    };
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    create_pool(&db_path).await
}

pub struct BuiltinProvider {
    pub builtin_id: &'static str,
    pub name: &'static str,
    pub provider_type: ProviderType,
    pub api_host: &'static str,
    pub models: Vec<(&'static str, &'static str, Vec<ModelCapability>, Option<u32>)>,
}

pub fn get_builtin_providers() -> Vec<BuiltinProvider> {
    use ModelCapability::*;

    vec![
        BuiltinProvider {
            builtin_id: "openai",
            name: "OpenAI",
            provider_type: ProviderType::OpenAI,
            api_host: "https://api.openai.com",
            models: vec![
                (
                    "gpt-5.5",
                    "GPT-5.5",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
                ("gpt-5.4", "GPT-5.4", vec![TextChat, Vision, FunctionCalling], Some(1048576)),
                (
                    "gpt-5.4-mini",
                    "GPT-5.4 Mini",
                    vec![TextChat, Vision, FunctionCalling],
                    Some(1048576),
                ),
                ("o4-mini", "o4-mini", vec![TextChat, Reasoning, FunctionCalling], Some(200000)),
            ],
        },
        BuiltinProvider {
            builtin_id: "openai_responses",
            name: "OpenAI Responses",
            provider_type: ProviderType::OpenAIResponses,
            api_host: "https://api.openai.com",
            models: vec![
                (
                    "gpt-5.5",
                    "GPT-5.5",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
                ("gpt-5.4", "GPT-5.4", vec![TextChat, Vision, FunctionCalling], Some(1048576)),
                (
                    "gpt-5.4-mini",
                    "GPT-5.4 Mini",
                    vec![TextChat, Vision, FunctionCalling],
                    Some(1048576),
                ),
                ("o4-mini", "o4-mini", vec![TextChat, Reasoning, FunctionCalling], Some(200000)),
            ],
        },
        BuiltinProvider {
            builtin_id: "gemini",
            name: "Gemini",
            provider_type: ProviderType::Gemini,
            api_host: "https://generativelanguage.googleapis.com",
            models: vec![
                (
                    "gemini-3.5-flash",
                    "Gemini 3.5 Flash",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
                (
                    "gemini-2.5-flash",
                    "Gemini 2.5 Flash",
                    vec![TextChat, Vision, FunctionCalling],
                    Some(1048576),
                ),
                (
                    "gemini-2.5-pro",
                    "Gemini 2.5 Pro",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "anthropic",
            name: "Claude",
            provider_type: ProviderType::Anthropic,
            api_host: "https://api.anthropic.com",
            models: vec![
                (
                    "claude-sonnet-4-6",
                    "Claude Sonnet 4.6",
                    vec![TextChat, Vision, FunctionCalling],
                    Some(200000),
                ),
                (
                    "claude-haiku-4-5",
                    "Claude Haiku 4.5",
                    vec![TextChat, Vision, FunctionCalling],
                    Some(200000),
                ),
                (
                    "claude-opus-4-8",
                    "Claude Opus 4.8",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(200000),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "deepseek",
            name: "DeepSeek",
            provider_type: ProviderType::OpenAI,
            api_host: "https://api.deepseek.com",
            models: vec![
                (
                    "deepseek-v4-flash",
                    "DeepSeek V4 Flash",
                    vec![TextChat, FunctionCalling],
                    Some(1048576),
                ),
                (
                    "deepseek-v4-pro",
                    "DeepSeek V4 Pro",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(1048576),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "qwen",
            name: "通义千问",
            provider_type: ProviderType::OpenAI,
            api_host: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            models: vec![
                (
                    "qwen3.7-max",
                    "Qwen3.7 Max",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
                (
                    "qwen3.6-plus",
                    "Qwen3.6 Plus",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
                (
                    "qwen3.6-flash",
                    "Qwen3.6 Flash",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(1048576),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "kimi",
            name: "Kimi",
            provider_type: ProviderType::OpenAI,
            api_host: "https://api.moonshot.cn/v1",
            models: vec![
                (
                    "kimi-k2.6",
                    "Kimi K2.6",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(262144),
                ),
                (
                    "kimi-k2.5",
                    "Kimi K2.5",
                    vec![TextChat, Vision, FunctionCalling, Reasoning],
                    Some(262144),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "doubao",
            name: "豆包",
            provider_type: ProviderType::OpenAI,
            api_host: "https://ark.cn-beijing.volces.com/api/v3",
            models: vec![
                (
                    "doubao-1.5-pro-256k",
                    "Doubao 1.5 Pro 256K",
                    vec![TextChat, Vision, FunctionCalling],
                    Some(262144),
                ),
                (
                    "doubao-1.5-lite-32k",
                    "Doubao 1.5 Lite 32K",
                    vec![TextChat, FunctionCalling],
                    Some(32768),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "siliconflow",
            name: "硅基流动",
            provider_type: ProviderType::OpenAI,
            api_host: "https://api.siliconflow.cn/v1",
            models: vec![
                (
                    "Pro/deepseek-ai/DeepSeek-R1",
                    "DeepSeek R1 (Pro)",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(65536),
                ),
                (
                    "Pro/deepseek-ai/DeepSeek-V3",
                    "DeepSeek V3 (Pro)",
                    vec![TextChat, FunctionCalling],
                    Some(65536),
                ),
                (
                    "Qwen/Qwen3-235B-A22B",
                    "Qwen3 235B",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(262144),
                ),
                (
                    "Qwen/Qwen3-32B",
                    "Qwen3 32B",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(262144),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "glm",
            name: "GLM",
            provider_type: ProviderType::OpenAI,
            api_host: "https://open.bigmodel.cn/api/paas/v4",
            models: vec![
                ("glm-5", "GLM-5", vec![TextChat, Reasoning, FunctionCalling], Some(128000)),
                ("glm-4-plus", "GLM-4 Plus", vec![TextChat, FunctionCalling], Some(128000)),
                ("glm-4-flash", "GLM-4 Flash", vec![TextChat, FunctionCalling], Some(128000)),
            ],
        },
        BuiltinProvider {
            builtin_id: "minimax",
            name: "MiniMax",
            provider_type: ProviderType::OpenAI,
            api_host: "https://api.minimax.io",
            models: vec![
                (
                    "MiniMax-M3",
                    "MiniMax-M3",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(1000000),
                ),
                ("MiniMax-S1", "MiniMax-S1", vec![TextChat, FunctionCalling], Some(1000000)),
                (
                    "minimaxai/minimax-m2.7",
                    "MiniMax-M2.7",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(1000000),
                ),
            ],
        },
        BuiltinProvider {
            builtin_id: "nvidia",
            name: "NVIDIA",
            provider_type: ProviderType::OpenAI,
            api_host: "https://integrate.api.nvidia.com/v1",
            models: vec![
                (
                    "meta/llama-3.1-405b-instruct",
                    "Llama 3.1 405B",
                    vec![TextChat, FunctionCalling],
                    Some(128000),
                ),
                (
                    "meta/llama-3.1-70b-instruct",
                    "Llama 3.1 70B",
                    vec![TextChat, FunctionCalling],
                    Some(128000),
                ),
                (
                    "nvidia/llama-3.1-nemotron-70b-instruct",
                    "Llama 3.1 Nemotron 70B",
                    vec![TextChat, FunctionCalling],
                    Some(128000),
                ),
                (
                    "nvidia/llama-3.3-nemotron-super-49b-v1",
                    "Llama 3.3 Nemotron Super 49B",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(128000),
                ),
                (
                    "minimaxai/minimax-m2.7",
                    "MiniMax-M2.7",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(1000000),
                ),
                (
                    "zhipuai/glm-4.7",
                    "GLM-4.7",
                    vec![TextChat, Reasoning, FunctionCalling],
                    Some(128000),
                ),
            ],
        },
        // llama.cpp 本地推理供应商（内置但默认未启用）
        // 模型列表由 `fetch_remote_models` 扫描下载目录的 *.gguf 填充
        BuiltinProvider {
            builtin_id: "llama_cpp",
            name: "llama.cpp",
            provider_type: ProviderType::LlamaCpp,
            api_host: "http://localhost:8091",
            models: vec![],
        },
        // TypeSafe **Jev** 决策模型（内置）。
        //
        // 它**不是 chat 供应商**：Jev 只接收 `state` + 调用方预先声明的类型化问题，
        // 返回带概率的结构化判定，故只能被 `llmClassifier` / 勾选「LLM 动态路由」的
        // `condition` 节点选中。`ModelType::Decision` 由 `detect_model_type` 从模型 id
        // 推断（无需在此显式声明），见 `providers/src/typesafe.rs` 的单测
        // `jev_is_typed_as_decision_model`。
        //
        // ⚠ 播种的行**不带 API Key**（key 走 `provider_keys`，由用户在设置页填），
        // 因此播种本身不会让任何链路开始走 Jev：`seed_serenity_fast::resolve_decision_model`
        // 要求「供应商启用 + 存在启用中的 key + 存在启用的 Decision 模型」三者齐备才返回
        // `Some`，否则节点 `model` 留空、静默回落默认 chat 模型。用户填 key 后还须升
        // 对应模板的 `TEMPLATE_VERSION` 重建模板（探测结果已固化进模板 JSON）。
        BuiltinProvider {
            builtin_id: "typesafe",
            name: "TypeSafe Jev",
            provider_type: ProviderType::TypeSafe,
            // 与 `providers/src/typesafe.rs` 的 `DEFAULT_BASE_URL` 同值：adapter 会在其后
            // 拼上 `/alpha/decisions` 组成完整端点，两者不一致会打偏。
            api_host: "https://openrouter.ai/api",
            models: vec![(
                "typesafe/jev-1.13",
                "Jev 1.13 (TypeSafe)",
                // 决策模型不挂 chat 能力标签（同 `TypeSafeAdapter::builtin_models`）。
                vec![],
                Some(32_000),
            )],
        },
    ]
}

async fn seed_builtin_providers(db: &DatabaseConnection) -> Result<()> {
    info!("Seeding built-in providers...");

    let builtins = get_builtin_providers();

    for (idx, bp) in builtins.into_iter().enumerate() {
        // Check if provider with this builtin_id already exists
        let existing = providers::Entity::find()
            .filter(providers::Column::BuiltinId.eq(bp.builtin_id))
            .one(db)
            .await?;

        if let Some(existing_prov) = existing {
            // Update api_host for existing built-in providers if it has a known-broken value
            let old_hosts: &[(&str, &str)] = &[
                ("https://api.minimaxi.com", "https://api.minimax.io"),
                ("https://open.bigmodel.cn/api/paas", "https://open.bigmodel.cn/api/paas/v4"),
            ];
            for (old_host, new_host) in old_hosts {
                if existing_prov.api_host == *old_host {
                    let mut active: providers::ActiveModel = existing_prov.into();
                    active.api_host = Set(new_host.to_string());
                    active.updated_at = Set(now_ts());
                    active.update(db).await?;
                    info!(
                        "Updated api_host for builtin provider '{}': {} -> {}",
                        bp.builtin_id, old_host, new_host
                    );
                    break;
                }
            }
            continue;
        }

        let prov = provider::create_provider(
            db,
            CreateProviderInput {
                name: bp.name.to_string(),
                provider_type: bp.provider_type,
                api_host: bp.api_host.to_string(),
                api_path: None,
                enabled: true,
                builtin_id: Some(bp.builtin_id.to_string()),
            },
        )
        .await?;

        let models: Vec<Model> = bp
            .models
            .into_iter()
            .map(|(model_id, name, caps, max_tokens)| Model {
                provider_id: prov.id.clone(),
                model_id: model_id.to_string(),
                name: name.to_string(),
                group_name: None,
                model_type: axagent_harness::types::provider_model::detect_model_type(model_id),
                capabilities: caps,
                max_tokens,
                max_output_tokens: None,
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            })
            .collect();

        provider::save_models(db, &prov.id, &models).await?;

        // Set sort order based on insertion index
        provider::update_provider(
            db,
            &prov.id,
            UpdateProviderInput { sort_order: Some(idx as i32), ..Default::default() },
        )
        .await?;
    }

    info!("Seeded built-in providers");
    Ok(())
}

pub async fn create_test_pool() -> Result<DbHandle> {
    let unique_id =
        format!("axagent_test_{}_{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let db_path = std::env::temp_dir().join(format!("{}.db", unique_id));
    let url = format!("sqlite:{}?mode=rwc", db_path.display());

    let mut opt = ConnectOptions::new(&url);
    opt.max_connections(1).min_connections(1).sqlx_logging(false);
    let conn = Database::connect(opt).await?;
    conn.execute_raw(Statement::from_string(DbBackend::Sqlite, "PRAGMA foreign_keys=ON;")).await?;

    // ⚠ 必须走与生产**同一个** `initialize_schema`（见其文档：此前这里只跑迁移，
    // 迁移清单清空后本函数会建不出一张表，而它有 38 处调用）。
    // 保留自己的连接配置（`max_connections(1)`）是刻意的：测试用 SQLite 文件库，
    // 多连接会引入锁竞争，而生产主库需要 20。差异只在**连接**，不在建表链。
    initialize_schema(&conn).await?;

    Ok(DbHandle { conn, path: db_path.to_string_lossy().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 方言分派判据 —— 「支持 sqlite 与 postgresql，由用户设置决定」这句话的可测形式。
    ///
    /// 这一组的价值不在于「函数写对了」，而在于**堵住静默降级**：原实现把「认不出的
    /// 输入」一律当 SQLite 文件，于是拼错的 scheme / 读空的配置都变成「连上了」。
    /// 每一条断言都对应一类曾经真实发生（或极易发生）的事故。
    #[test]
    fn db_url_dispatch_never_silently_falls_back_to_sqlite() {
        // ── PostgreSQL：原样透传，且**不许**被当成 SQLite ──
        for pg in ["postgres://u:p@h:5432/db", "postgresql://u@h/db"] {
            let (url, is_sqlite) = resolve_db_url_from_path(pg).expect("PG 串应被接受");
            assert_eq!(url, pg, "PG 连接串必须原样透传（追加查询串会破坏它）");
            assert!(!is_sqlite, "PG 串不能被判成 SQLite：{pg}");
        }

        // ── SQLite：显式 scheme ──
        let (url, is_sqlite) = resolve_db_url_from_path("sqlite:/tmp/a.db").expect("sqlite 串");
        assert_eq!(url, "sqlite:/tmp/a.db?mode=rwc");
        assert!(is_sqlite);
        // 已带查询串时不得再追加一个 `?`（否则 URL 变成 `…?mode=rwc?mode=rwc`）
        let (url, _) =
            resolve_db_url_from_path("sqlite:/tmp/a.db?cache=shared").expect("sqlite 串");
        assert_eq!(url, "sqlite:/tmp/a.db?cache=shared");

        // ── 裸文件路径：**必须**同时把 is_sqlite 置真 ──
        //
        // 这条正是实测缺陷的回归判据：`create_pool_for_profile` 传的就是裸路径，
        // 旧实现下 `is_sqlite == false` ⇒ PRAGMA 段（含 `foreign_keys=ON`）整段跳过，
        // 而连接实际是 SQLite ⇒ 库自己的外键约束静默失效。
        let bare = std::path::Path::new("/home/u/.axagent/profiles/p1/data/axagent.db");
        let (url, is_sqlite) = resolve_db_url_from_path(&bare.to_string_lossy())
            .expect("裸路径应被当成 SQLite 文件库");
        assert!(url.starts_with("sqlite:"), "{url}");
        assert!(url.ends_with("?mode=rwc"), "{url}");
        assert!(is_sqlite, "裸文件路径被判成「非 SQLite」⇒ PRAGMA 段会被跳过（外键约束失效）");

        // ── 空 / 纯空白：报错，不猜 ──
        //
        // SQLite 把**空文件名**当临时库 ⇒ 写入的数据随后消失。静默接受等于把
        // 「配置没读到」变成「数据不知去哪了」。
        for empty in ["", "   ", "\t\n"] {
            let e = resolve_db_url_from_path(empty).expect_err("空串必须报错");
            assert!(e.to_string().contains("空串"), "{e}");
        }

        // ── 其它 scheme：报错，不退回 SQLite ──
        let e = resolve_db_url_from_path("mysql://u@h/db").expect_err("未知 scheme 必须报错");
        assert!(e.to_string().contains("mysql"), "报错要点出是哪个 scheme：{e}");
        assert!(e.to_string().contains("不支持"), "{e}");
    }

    /// **存量 SQLite 库（迁移建出来的形态）必须能初始化成功。**
    ///
    /// ## 这条判据为什么值钱
    ///
    /// 2026-09-16 实测：`create_test_pool` 接上 `initialize_schema`（= 迁移 + 声明式收敛）
    /// 之后，`repo::message::tests::create_message_round_trips_attachment_metadata` 当场报红：
    ///
    /// ```text
    /// 声明式 schema 收敛被拒（render）：19 条变更渲染失败：SET DEFAULT narrative_structures.genre…；
    /// ADD FK gateway_link_activities.link_id… | 计划条目 40 条、已执行 0 条
    /// ```
    ///
    /// 根因不是测试写错：迁移建出的库与实体声明有 40 条差集，其中 19 条（`SET DEFAULT` /
    /// `ADD FK`）在 SQLite 上没有原生 `ALTER TABLE` 形态 ⇒ 旧实现把它们算进「渲染失败」
    /// ⇒ 整批拒绝 ⇒ 本函数返回 Err ⇒ **启动中止**。真实用户的 `.axagent/data/axagent.db`
    /// 走的是同一条链（`create_pool` → `initialize_schema`），所以那时它是一枚上了膛的枪。
    ///
    /// ## ⚠ 这条测试的**职责边界**（别把它当成「引擎接管建表」的证明）
    ///
    /// `create_test_pool` 先跑迁移，表是**迁移**建的 —— 所以本测试只能证明
    /// 「存量库的初始化链不会中止」，不能证明引擎能独立建表。后者由
    /// `reconcile::apply::tests::bootstrap_on_empty_sqlite_has_no_dialect_gaps`
    /// 与 `examples/p5_engine_takeover.rs --fresh-sqlite` 负责。两种输入，两条判据。
    ///
    /// 迁移清单清空后本测试仍然有效（那时它退化成「全新库能建起来」），不随删除而腐烂。
    #[tokio::test]
    async fn existing_sqlite_db_initializes_despite_dialect_gaps() {
        let h = create_test_pool().await.expect("存量形态的 SQLite 库必须能初始化成功");

        // 不能是「没报错但什么都没建」：抽几张业务表逐张存在性检查。
        // ⚠ 用 `query_all_raw`（`query_one` 在本仓的 sea-orm 2.0 上要求 `StatementBuilder`，
        // `Statement` 不实现它 —— 别照 `execute_raw` 的用法类推）。
        for t in ["conversations", "messages", "providers", "workflow_templates"] {
            let rows = h
                .conn
                .query_all_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    format!(
                        "SELECT COUNT(*) AS c FROM sqlite_master WHERE type='table' AND name='{t}'"
                    ),
                ))
                .await
                .expect("查 sqlite_master 应成功");
            let n: i64 = rows.first().and_then(|r| r.try_get("", "c").ok()).unwrap_or(0);
            assert_eq!(n, 1, "业务表 `{t}` 不存在 —— 初始化链虽然没报错，但库是空的");
        }
    }
}
