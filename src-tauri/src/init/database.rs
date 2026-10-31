// SPDX-License-Identifier: AGPL-3.0-only

use std::path::{Path, PathBuf};

/// 限制密钥文件权限为仅当前用户可访问。
/// - Unix: 0o600 (owner rw)
/// - Windows: icacls 移除继承权限，仅保留当前用户
pub fn restrict_file_permissions(#[allow(unused_variables)] path: &Path) -> Result<(), String> {
    #[cfg(all(unix, not(mobile)))]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("failed to set file permissions: {}", e))?;
    }
    #[cfg(windows)]
    {
        // Windows: 使用 icacls 移除继承权限，仅保留当前用户
        let username = std::env::var("USERNAME").unwrap_or_else(|_| "SYSTEM".into());
        let mut scmd = std::process::Command::new("icacls");
        scmd.arg(path.as_os_str())
            .arg("/inheritance:r")
            .arg("/grant")
            .arg(format!("{}:(R,W)", username));
        #[cfg(windows)]
        axagent_kit::utils::hide_window(&mut scmd);
        let result = scmd.output().map_err(|e| format!("failed to run icacls: {}", e))?;
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            tracing::warn!(
                "icacls restricted permissions reported non-zero exit: stderr={}",
                stderr
            );
        }
    }
    #[cfg(mobile)]
    {
        let _ = path; // 移动端无文件权限细化需求
    }
    #[cfg(all(not(mobile), not(unix), not(windows)))]
    {
        let _ = path; // unsupported platform, skip
    }
    Ok(())
}

pub struct DatabaseInitResult {
    pub db_handle: axagent_dao::db::DbHandle,
    pub db_path: String,
    pub master_key: [u8; 32],
    pub app_dir: PathBuf,
}

/// 使用预先解析的 app_dir 初始化数据库。
///
/// Android：主线程已调用 `axagent_home()` + `create_dir_all()`，
/// 子线程中 `dirs::data_dir()` 因缺少 JNI 上下文不可用。
/// 此函数跳过路径解析，直接使用传入的目录。
/// 解析数据库连接配置（DB 外持久化于 `{app_dir}/db_config.json`）。
///
/// 返回 `(连接 URL, 是否 SQLite)`。未配置文件时回退到默认本地 SQLite。
fn resolve_db_url(app_dir: &Path, master_key: &[u8; 32]) -> Result<(String, bool), String> {
    let cfg_path = app_dir.join("db_config.json");
    if !cfg_path.exists() {
        return Ok((format!("sqlite:{}/axagent.db", app_dir.display()), true));
    }
    let content = std::fs::read_to_string(&cfg_path).map_err(|e| e.to_string())?;
    let cfg: axagent_dao::config::DbConfig =
        serde_json::from_str(&content).map_err(|e| e.to_string())?;
    build_db_url(&cfg, app_dir, master_key)
}

/// 根据 DbConfig 解析数据库连接 URL 与是否 SQLite。
///
/// PostgreSQL 密码优先使用明文 `pg_password`（连接测试/前端回传），
/// 缺失时回退解密 `pg_password_enc`（启动时从盘读取）。
/// 供 `init_database_with_dir`（启动）与 `test_db_connection` 命令共用，
/// 避免解析逻辑重复。
pub(crate) fn build_db_url(
    cfg: &axagent_dao::config::DbConfig,
    app_dir: &Path,
    master_key: &[u8; 32],
) -> Result<(String, bool), String> {
    if cfg.db_type == "postgres" {
        // 校验在拼 URL **之前**：host/database 是**裸拼**进 URL 的（不像 user/password
        // 有 pg_url_encode 兜底），host 里一个 `@` 就把 userinfo 边界改写掉、
        // database 里一个 `/` 就伪造出多级 path ⇒ 连错库 / 解析错 host ⇒ 假降级。
        // `None` 走文档化默认值；`Some("")` **不猜** —— 与 dao
        // `resolve_db_url_from_path` 对空串报错的纪律同源：把「配置没填好」
        // 伪装成「连上了默认库」正是「数据不知去哪了」的事故形态。
        let host = match &cfg.pg_host {
            Some(h) => {
                validate_pg_host(h)?;
                h.clone()
            },
            None => "localhost".to_string(),
        };
        let port = cfg.pg_port.unwrap_or(5432);
        let database = match &cfg.pg_database {
            Some(d) => {
                validate_pg_database(d)?;
                d.clone()
            },
            None => "axagent".to_string(),
        };
        let user = cfg.pg_user.clone().unwrap_or_else(|| "postgres".to_string());
        let password = match &cfg.pg_password {
            Some(pw) if !pw.is_empty() => pw.clone(),
            _ => match &cfg.pg_password_enc {
                Some(enc) => axagent_crypto::decrypt_key(enc, master_key)
                    .map_err(|e| format!("解密数据库密码失败: {}", e))?,
                None => String::new(),
            },
        };
        let sslmode = if cfg.use_ssl.unwrap_or(false) {
            "require"
        } else {
            "disable"
        };
        let mut url = format!(
            "postgres://{}:{}@{}:{}/{}?sslmode={}",
            pg_url_encode(&user),
            pg_url_encode(&password),
            host,
            port,
            database,
            sslmode
        );
        if let Some(schema) = &cfg.pg_schema {
            if !schema.is_empty() {
                url.push_str(&format!("&search_path={}", pg_url_encode(schema)));
            }
        }
        Ok((url, false))
    } else {
        let path =
            cfg.sqlite_path.clone().unwrap_or_else(|| format!("{}/axagent.db", app_dir.display()));
        Ok((format!("sqlite:{}", path), true))
    }
}

/// 对 PostgreSQL 连接 URL 中的 user/password 做最小 percent-encode，
/// 转义 `@ : / % & # ?` 及空格，避免破坏 URL 结构。
fn pg_url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '@' | ':' | '/' | '%' | '&' | '#' | '?' | ' ' => {
                out.push_str(&format!("%{:02X}", c as u8));
            },
            _ => out.push(c),
        }
    }
    out
}

/// 校验 `pg_host` 输入。**纯函数**，好在拼 URL 之前就把结构性破坏拒掉。
///
/// 为什么是**拒绝**而不是 percent-encode：host 是寻址语义，编码修不了歧义 ——
/// `ho@st` 编码成 `ho%40st` 后 libpq 会当主机名解析，照样连不上；而 `ho@st`
/// 不编码则 userinfo 边界被改写（`parse_pg_host_port` 按「最后一个 `@`」取
/// host，会把 host 取成 `st`）。两种都产生假故障，唯一正确处理是让用户
/// 在入口就看到可操作的报错。
///
/// 规则（host 出现在 URL authority 段，`{host}:{port}` 由 `build_db_url` 的
/// `format!` 拼接）：
/// * 非空、无首尾空白（首尾空白几乎必是输入失误，静默 trim 是在猜）
/// * 白名单 `[A-Za-z0-9._-]` —— 主机名的合法字符集本来就小，黑名单追不完
/// * IPv6 **必须**方括号形式 `[...]`：内部只允许十六进制 / `:` / `.`（兼容
///   v4 映射 `::ffff:192.168.0.1`）；`]` 之后不允许再带东西（端口有自己的
///   输入框，`[::1]:5432` 会拼成 `...@[::1]:5432:5432/...`）
/// * 裸 `::1` 被白名单拒绝并提示改用 `[::1]` —— 不拒绝的话
///   `postgres://u:p@::1:5432/db` 是结构歧义的，且
///   `parse_pg_host_port` 对它解析出空 host
fn validate_pg_host(host: &str) -> Result<(), String> {
    let bad = |reason: String| Err(format!("pg_host 非法：`{host}` —— {reason}"));
    if host.is_empty() {
        return bad("不能为空。留空请在配置里删掉该字段（会回退 localhost），或填一个主机名".into());
    }
    if host.trim() != host {
        return bad("含首尾空白，请去掉空格".into());
    }
    if let Some(inner) = host.strip_prefix('[') {
        let Some((addr, tail)) = inner.split_once(']') else {
            return bad("方括号未闭合，IPv6 应写作 `[::1]`".into());
        };
        if !tail.is_empty() {
            return bad(
                "`]` 之后不应再带内容（端口请在「端口」输入框单独填，不要写进主机名）".into()
            );
        }
        if addr.is_empty() || !addr.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return bad("方括号内只允许 IPv6 字符（十六进制、`:`、`.`）".into());
        }
        return Ok(());
    }
    if let Some(c) =
        host.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    {
        let hint = if c == ':' {
            "IPv6 请用方括号形式，如 `[::1]`"
        } else {
            "主机名只允许字母、数字、`.`、`-`、`_`"
        };
        return bad(format!("含非法字符 `{c}`（{hint}）"));
    }
    Ok(())
}

/// 校验 `pg_database` 输入。**纯函数**，规则黑名单：database 落在 URL path 段，
/// 会破坏结构的是 `/ ? # @ %` 与空白（`@` 尤其要拒 —— 本文件 `parse_pg_host_port`
/// 按「最后一个 `@`」切 userinfo，库名带 `@` 会把自己的诊断解析也带歪；
/// `%` 会被 URL 解析器当 percent-escape，非法序列直接让连接失败且报错指向无关处）。
/// 不做白名单：PG 标识符在带引号时合法字符很多，黑名单漏拒的顶多连不上
/// （服务端报错），白名单误杀的是合法库。
fn validate_pg_database(name: &str) -> Result<(), String> {
    let bad = |reason: String| Err(format!("pg_database 非法：`{name}` —— {reason}"));
    if name.is_empty() {
        return bad("不能为空。留空请在配置里删掉该字段（会回退 axagent）".into());
    }
    if name.trim() != name {
        return bad("含首尾空白，请去掉空格".into());
    }
    if name.chars().any(char::is_whitespace) {
        return bad("不能含空白字符（需要带空格的库名请在 PG 里改用不含空格的名字）".into());
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '/' | '?' | '#' | '@' | '%')) {
        return bad(format!(
            "含非法字符 `{c}`（会破坏连接串结构：`/` 伪造 path 层级，`?`/`#` 伪造查询与片段，\
             `@` 干扰 userinfo 解析，`%` 落进 URL 会被当 percent-escape 解码）"
        ));
    }
    Ok(())
}

pub async fn init_database_with_dir(app_dir: PathBuf) -> Result<DatabaseInitResult, String> {
    axagent_storage::storage_paths::ensure_documents_dirs().unwrap_or_else(|e| {
        tracing::warn!(
            "Failed to create documents storage dirs (non-critical, will retry later): {}",
            e
        );
    });

    let key_path = app_dir.join("master.key");
    let master_key = load_or_create_master_key(&key_path, &app_dir)?;

    // 解析数据库连接配置（DB 外持久化于 {app_dir}/db_config.json）
    let (db_url, is_sqlite) = resolve_db_url(&app_dir, &master_key)?;

    // ── Postgres 可用性探测 + 降级 ──
    // 当 db_type=postgres 但连不上时（如服务宕机 / 密码错 / 网络不通），
    // 默认降级到本地 SQLite 以保住启动可用性。
    // 通过 `AXAGENT_DB_FALLBACK_TO_SQLITE=0` / db_config.fallback_to_sqlite=false 可关闭降级，
    // 关闭时连接失败直接报错回上层窗口。
    let pg_failure = if !is_sqlite {
        pg_unreachable(&db_url).await
    } else {
        None
    };
    let (db_url, is_sqlite) = if let Some(reason) = pg_failure {
        if !fallback_enabled(&app_dir) {
            return Err(format!(
                "Postgres 不可达且降级被禁用: {}（如需启用降级，请在 db_config.json 设 \
                 \"fallback_to_sqlite\": true，或设置环境变量 AXAGENT_DB_FALLBACK_TO_SQLITE=1）",
                reason
            ));
        }
        tracing::error!(
            target: "startup",
            "{}",
            format_pg_fallback_log(&reason, &db_url)
        );
        write_fallback_marker(&app_dir, "pg_unreachable", &reason).ok();
        // ⚠ **不要在这里拼 `?mode=rwc`**。
        //
        // dao 侧 `resolve_db_url_from_path`（crates/dao/src/db.rs:110-115）对
        // `sqlite:` 开头且不带查询串的输入**自己就会追加** `?mode=rwc`，`create_pool`
        // 拿到的 URL 与拼过的完全一致。而这里多拼的这一段会原样进
        // `global_state::set_db_path`（见下方），于是整个进程生命周期内
        // `db_path()` 都返回一个**带查询串的假路径**，下游 4 处只做了
        // `strip_prefix("sqlite:")` 就把它当文件名用：
        //   · src/commands/backup.rs:80        → 恢复时 restore_sqlite_backup 打不开
        //   · src/commands/webdav.rs:280       → std::fs::copy 失败 ⇒「已中止恢复」
        //   · src/init/state.rs:196            → FTS5 建表失败 ⇒ 全文检索静默降级
        //   · crates/tools/src/tools/knowledge.rs:181 → 文本兜底搜索打不开库
        // （Windows 上 `?` 是非法文件名字符，所以是直接报错；Linux 上会落地一个
        //   名叫 `axagent.db?mode=rwc` 的空库 —— 连错库那种「数据不知去哪了」。）
        // 正确形态见 crates/dao/src/integrity.rs:90：strip prefix 后还要 `.split('?')`。
        // 在这里去掉 `?mode=rwc` 是**一处改动同时修掉上面四处**，不必去改四个文件。
        let sqlite_url = format!("sqlite:{}/axagent.db", app_dir.display());
        (sqlite_url, true)
    } else {
        (db_url, is_sqlite)
    };

    // 仅 SQLite 注册 sqlite-vec 扩展。在 Android 上默认跳过（见 vector_store.rs），
    // 在桌面平台用 catch_unwind 防止 FFI 异常 panic。
    if is_sqlite {
        let vec_registration = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            axagent_search::vector_store::register_sqlite_vec_extension();
        }));
        if let Err(e) = vec_registration {
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "unknown panic payload".to_string()
            };
            tracing::error!(
                "sqlite-vec extension registration panicked: {} — vector search will be unavailable",
                msg
            );
        }
    }
    axagent_tools::global_state::set_db_path(&db_url);

    // 直接使用当前 tokio runtime，不再创建嵌套 Runtime
    let db_handle = axagent_dao::db::create_pool(&db_url).await.map_err(|e| {
        format!("database initialization failed: {}", explain_pg_error(&e.to_string()))
    })?;

    // ── Schema 自愈：适配用户选择的任意数据库 ─────────────────────────
    // 用户可在 UI（设置 → 数据库）中选择任何 PostgreSQL/SQLite 库，程序必须
    // 适配所选库，而不是要求库适配代码。
    //
    // 场景：连接的是下游 fork 库（如 AxInvest 的 axinvest 库），其迁移版本号
    // 体系与主线不同（fork 用 v200+ 编号），`applied_max` 恒大于主线
    // `CURRENT_VERSION`。
    //
    // 这里检测版本超前场景，自动触发 `repair_schema`，无需用户手动点击
    // "修复 Schema"按钮。
    //
    // ⚠ 2026-09-16 迁移清单清空后，`repair_schema` 做的事只剩
    // `schema_diff::heal_all`（以实体声明为权威补缺失列 + 修类型错配）—— 它
    // **不再无条件重跑所有已注册迁移**（清单为空，无迁移可跑）也不再写版本表。
    // 本块对「版本超前」场景的兜底价值因此收缩为「补齐列级缺口」，不再兼有
    // 「重跑全部 IF NOT EXISTS 的迁移」这一层。
    //
    // 2026-09-12 更新：`run_migrations` 的原判据 `MAX(version)` 高水位线**判不出
    // 中间缺口**（生产库 `applied == latest == 227` 而 v101 / v139 从未执行，
    // 见 `axagent_dao::migrations::missing_versions` 注释），该判据已改为
    // **版本表集合成员判定**。⚠ 清单清空后集合成员判定同样无对象
    // （`MIGRATIONS` 为空 ⇒ 无缺口可补），于是本块**只剩「版本超前」这一个用途**：
    // 它是 `applied_version` / `latest_version` 两个字段在全仓的**唯一逻辑消费者**
    // —— 删掉本块，那两个字段就没有存在理由了（见 `SchemaStatus` 的字段文档）。
    if let Ok(status) = axagent_dao::migrations::get_schema_status(&db_handle.conn).await {
        if status.applied_version > status.latest_version {
            tracing::warn!(
                "[DB] 检测到 schema 版本超前（applied={} > latest={}，疑似下游 fork 库），\
                 自动执行 repair_schema 补齐缺失列...",
                status.applied_version,
                status.latest_version
            );
            match axagent_dao::migrations::repair_schema(&db_handle.conn).await {
                Ok(report) => {
                    // ⚠ 补的是**列**不是表：`heal_all` 对本库里不存在的表直接跳过
                    // （建表是引擎的事），所以这里别说成「补齐缺失表」。
                    if report.errors.is_empty() {
                        tracing::info!(
                            "[DB] repair_schema 自动自愈完成: 对照 {} 张表，补列 {} 个，类型修复 {} 个",
                            report.tables_scanned,
                            report.columns_added.len(),
                            report.types_healed.len()
                        );
                    } else {
                        // 部分未完成也是「没做完」：这些表的列账不完整，启动日志必须
                        // 说得清，否则它和上面那句 info 长得一样、被当成完全成功。
                        tracing::warn!(
                            "[DB] repair_schema 自动自愈**部分完成**: {} 张表对照完成，\
                             另有 {} 张未对照完（其缺失列无从判断），补列 {} 个，类型修复 {} 个；未完成: {:?}",
                            report.tables_scanned,
                            report.errors.len(),
                            report.columns_added.len(),
                            report.types_healed.len(),
                            report.errors
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!("[DB] repair_schema 自动自愈失败（不阻塞启动）: {}", e);
                },
            }
        }
    }

    // MCP 预设服务器播种（依赖 mcp_client，在 core 中）
    if let Err(e) = axagent_dao::repo::mcp_server::ensure_preset_servers(&db_handle.conn).await {
        tracing::warn!("[DB] MCP 预设服务器迁移失败: {e}");
    }

    // 硬编码路径 → 模板变量迁移（已迁入 storage，直接调用）
    // 注意：此函数已从 path_vars 移除，迁移逻辑由各模块自行处理
    // axagent_storage::path_vars::migrate_hardcoded_paths(&db_handle.conn).await;

    // 注册 SeaORM 连接
    axagent_tools::global_state::set_sea_db(std::sync::Arc::new(db_handle.conn.clone()));

    // 将 dao 实现的 agent / workflow 系列 repository 注册进 harness 全局服务注册表，
    // 供 consumer crate（rt-workflow 等）通过 trait 访问器获取，避免直接依赖 axagent-entities。
    axagent_dao::agent_repositories::register_repositories(&db_handle.conn);

    // 需求精评 LLM 桥：把首个启用的 provider 注入 tools 全局态，
    // 供 run_discovery_scan 对高分候选做 LLM 精评（未配置 provider 时精评静默跳过）。
    // 必须在 register_repositories 之后 —— build_llm_bridge_from_db 走
    // provider_repository() 全局访问器，ServiceRegistry 未注册时会 panic。
    crate::commands::demand_llm_refine::register_demand_llm_bridge(&master_key).await;

    Ok(DatabaseInitResult { db_handle, db_path: db_url, master_key, app_dir })
}

pub(crate) fn load_or_create_master_key(
    key_path: &Path,
    app_dir: &Path,
) -> Result<[u8; 32], String> {
    if key_path.exists() {
        let mut bytes =
            std::fs::read(key_path).map_err(|e| format!("failed to read master key: {}", e))?;
        if bytes.len() != 32 {
            return Err(format!(
                "master.key is corrupted: expected 32 bytes, got {}. Delete the file to regenerate.",
                bytes.len()
            ));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        // Security: securely zero the temporary buffer before dropping.
        // Using a helper that inhibits compiler optimization of the clear.
        secure_zero(&mut bytes);
        // key is returned (copy), bytes is zeroed and dropped
        Ok(key)
    } else {
        let db_file = app_dir.join("axagent.db");
        if db_file.exists() {
            return Err(format!(
                "FATAL: axagent.db exists at '{}' but master.key is missing from '{}'.\n\
                 Generating a new master key would render all encrypted database \
                 contents permanently unrecoverable.\n\n\
                 Options:\n\
                 • Restore master.key from a backup and restart.\n\
                 • Remove axagent.db (and axagent.db-shm / axagent.db-wal if present) \
                   to start fresh — ALL DATA WILL BE LOST.",
                db_file.display(),
                key_path.display()
            ));
        }
        let key = axagent_crypto::generate_master_key();
        std::fs::write(key_path, key).map_err(|e| format!("failed to write master key: {}", e))?;
        restrict_file_permissions(key_path)?;
        Ok(key)
    }
}

/// Securely zero a byte buffer, inhibiting compiler optimization of the clear.
/// Uses volatile writes + compiler fence to ensure the memory is actually overwritten before drop.
#[inline(never)]
fn secure_zero(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        // SAFETY: byte is a valid mutable reference obtained from buf.iter_mut();
        // write_volatile is used to prevent compiler optimization from eliding the
        // zeroing of sensitive key material; this is the standard pattern for secure
        // memory clearing.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    // SECURITY (C8): compiler_fence 防止编译器将上述 volatile 写入视为"死存储"而优化掉。
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

// ──────────────────────────────────────────────────────────────────────
// Postgres 可用性探测 + 降级 helper（init/database.rs 私有）
// ──────────────────────────────────────────────────────────────────────

/// 构造「Postgres 不可达，降级到 SQLite」这条启动日志的文本。
///
/// **纯函数，好让「日志里有没有明文密码」可被单测守住。** 这是本次事故的形态：
/// 降级日志把含明文口令的连接串写进了日志文件。历史上本文件还自带一份脱敏副本
/// 并取第一个 `@`，密码含未编码 `@` 时后半段直接外泄。
///
/// 现在脱敏口径的**唯一真源**是 `axagent_dao::db::redact_db_url`（取最后一个 `@`），
/// 本文件不再有第二份实现 —— 两份实现正是分叉的根源。
fn format_pg_fallback_log(reason: &str, db_url: &str) -> String {
    format!(
        "⚠️ Postgres 不可达，降级到本地 SQLite。原因: {} | 原 URL: {}",
        reason,
        axagent_dao::db::redact_db_url(db_url)
    )
}

/// 从 Postgres URL 里解析出 `(host, port)`。纯函数，好让「取哪个 `@`」这条判据可被单测守住。
///
/// **必须取最后一个 `@`**：密码含未编码 `@`（如 `p@ss`）时，取第一个会把 host
/// 解析成 `ss@host:5432` ⇒ TCP 必然连不上 ⇒ 误报「Postgres 不可达」⇒ 触发降级
/// 并打印 URL。与脱敏口径 `axagent_dao::db::redact_db_url` 同口径。
///
/// **IPv6 字面量必须走方括号分支**（RFC 3986 §3.2.2）：host 以 `[` 开头时，端口
/// 分隔符是 `]` **之后**那个 `:`，而不是 host 内部的 `:`。用朴素的
/// `split_once(':')` 会把 `[::1]:5432` 解析成 host=`[`、port 回退 5432 ⇒ TCP 连
/// 一个叫 `[` 的主机名必然失败 ⇒ 误报「Postgres 不可达」⇒ 触发降级 + 打印 URL。
/// IPv6 回环（`[::1]`）是本地自建 PG 的常见写法，比「密码含 `@`」更容易触发。
///
/// 返回的 host **不带方括号**：`tokio::net::TcpStream::connect((host, port))` 走
/// `ToSocketAddrs`，它要的是 `::1` 这种裸地址，`[::1]` 会解析失败。
fn parse_pg_host_port(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("postgres://").or_else(|| url.strip_prefix("postgresql://"))?;
    // 跳过 user:pass@
    let after_auth = rest.rsplit_once('@')?.1;
    let host_port = after_auth.split('/').next()?;
    // IPv6 字面量：`[::1]` / `[::1]:5432` / `[2001:db8::1]`
    if let Some(inner) = host_port.strip_prefix('[') {
        let (addr, tail) = inner.split_once(']')?;
        let port = tail.strip_prefix(':').and_then(|p| p.parse::<u16>().ok()).unwrap_or(5432);
        return Some((addr.to_string(), port));
    }
    Some(match host_port.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(5432)),
        None => (host_port.to_string(), 5432),
    })
}

/// 探测 Postgres 是否**真的可用**：先 3 秒 TCP 快探排除网络不通与超时，
/// 再走一次完整握手（`connect_without_initialization`：只连库，不建表、不收敛）。
///
/// ⚠ 为什么不能停在 TCP 层（2026-10-28 实测）：此前只判 TCP，于是
/// `pg_hba.conf` 拒绝 / 口令错误 / 库不存在这三类都会「探测通过」，直到
/// `create_pool` 才失败 —— 而它们全是**认证阶段的 FATAL**，服务端此时按
/// `lc_messages` 的 locale 编码消息。中文 Windows 上 initdb 默认
/// `Chinese (Simplified)_China.936`，sqlx 无法按 UTF-8 解码 ⇒ 只剩
/// `non-UTF-8 string for its error message` 这一句，真正的
/// `FATAL: no pg_hba.conf entry for host "…"` 被整条吞掉，且降级逻辑不触发。
/// 握手失败同样计入「不可达」，让既有降级/报错路径给出可操作结论。
async fn pg_unreachable(url: &str) -> Option<String> {
    // 解析 host:port
    let (host, port) = parse_pg_host_port(url)?;
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    {
        Ok(Ok(_stream)) => {},
        Ok(Err(e)) => return Some(format!("TCP 连接失败: {}", e)),
        Err(_) => return Some("连接超时（3s）".to_string()),
    }
    // TCP 通 ≠ 可用：hba 拒绝 / 口令错误 / 库不存在都在握手阶段才暴露。
    match axagent_dao::db::connect_without_initialization(url).await {
        Ok(_handle) => None,
        Err(e) => Some(explain_pg_error(&e.to_string())),
    }
}

/// 读取 db_config.fallback_to_sqlite 与 AXAGENT_DB_FALLBACK_TO_SQLITE 环境变量。
/// env 优先于配置文件；配置文件缺省为 true（即允许降级）。
fn fallback_enabled(app_dir: &Path) -> bool {
    if let Ok(v) = std::env::var("AXAGENT_DB_FALLBACK_TO_SQLITE") {
        match v.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "no" | "off" => return false,
            "1" | "true" | "yes" | "on" => return true,
            _ => {},
        }
    }
    let cfg_path = app_dir.join("db_config.json");
    if let Ok(content) = std::fs::read_to_string(&cfg_path) {
        if let Ok(cfg) = serde_json::from_str::<axagent_dao::config::DbConfig>(&content) {
            return cfg.fallback_to_sqlite.unwrap_or(true);
        }
    }
    true
}

/// 写一个伴生标记文件，UI 重连 PG 时可读取提示用户。
/// 文件格式：`{"reason": "...", "details": "...", "ts": 1234567890}`。
fn write_fallback_marker(app_dir: &Path, reason: &str, details: &str) -> std::io::Result<()> {
    let marker_path = app_dir.join("db_fallback_active.json");
    let body = serde_json::json!({
        "reason": reason,
        "details": details,
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "version": 1,
    });
    std::fs::write(marker_path, serde_json::to_vec_pretty(&body).unwrap_or_default())
}

/// 把 sqlx 的「服务端错误消息不是 UTF-8」翻译成带修法的中文。
///
/// 触发条件：PostgreSQL 服务端 `lc_messages` 是非 UTF-8 兼容 locale（中文 Windows
/// initdb 默认 `Chinese (Simplified)_China.936`）。认证阶段的 FATAL（`pg_hba` 拒绝、
/// 口令错误）按该 locale 编码，客户端按 UTF-8 解码失败 ⇒ 真正原因被掩盖。
///
/// 不命中原样返回：错误原文是排查现场，改写它就查不到了。
fn explain_pg_error(raw: &str) -> String {
    if !raw.contains("non-UTF-8 string for its error message") {
        return raw.to_string();
    }
    format!(
        "{raw}\n\n\
         ↑ 这不是真正原因：PostgreSQL 服务端用非 UTF-8 的 locale（lc_messages）编码了错误消息，\
         客户端无法解码，真实的服务端错误（常见为 pg_hba.conf 未放行该客户端地址、口令错误、\
         库不存在）被吞掉了。\n\
         修法（在数据库服务端执行，二选一）：\n\
         · psql: ALTER SYSTEM SET lc_messages = 'C'; SELECT pg_reload_conf();\n\
         · 改 postgresql.conf 的 lc_messages = 'C' 后重载服务\n\
         改完重试即可看到服务端原文错误。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命中编码特征 → 必须保留原文并给出两条修法；未命中 → 一个字都不许改。
    #[test]
    fn explain_pg_error_only_rewrites_encoding_failures() {
        let raw = "Connection Error: Postgres returned a non-UTF-8 string for its error \
                   message. This is most likely due to an error that occurred during \
                   authentication (sqlx_postgres:message:139)";
        let out = explain_pg_error(raw);
        assert!(out.contains(raw), "原文必须保留，否则现场丢失：{out}");
        assert!(out.contains("lc_messages"), "{out}");
        assert!(out.contains("pg_hba"), "{out}");

        let other = "TCP 连接失败: 由于目标计算机积极拒绝，无法连接。";
        assert_eq!(explain_pg_error(other), other, "非编码类错误不得被改写");
    }

    /// 降级日志里打印的 URL 必须已脱敏（脱敏口径的唯一真源在 dao 侧）。
    ///
    /// 2026-10-31 事故：本文件曾自带一份副本并取第一个 `@`，密码含未编码 `@` 时
    /// 密码后半段会外泄。合并到 `axagent_dao::db::redact_db_url` 后，此路径不可能
    /// 再与 dao 侧分叉。完整的 8 组输入由 `crates/dao/src/db.rs` 的
    /// `redact_db_url_replaces_password_entirely` 守住。
    #[test]
    fn startup_log_url_is_redacted() {
        assert_eq!(
            axagent_dao::db::redact_db_url("postgres://user:p@ss@host/db"),
            "postgres://user:***@host/db",
            "密码里的 @ 之后不得有明文残留"
        );
    }

    /// host:port 解析：密码含未编码 `@` 时，host 不得被解析成 `ss@host`。
    ///
    /// 这条不是洁癖：解析错了 ⇒ TCP 连的是不存在的 host ⇒ 必然超时 ⇒ 误报
    /// 「Postgres 不可达」⇒ 触发降级并把连接串打进日志。
    #[test]
    fn parse_pg_host_port_takes_last_at() {
        // 常规
        assert_eq!(
            parse_pg_host_port("postgres://user:pw@host:5432/db"),
            Some(("host".to_string(), 5432))
        );
        // 无端口 → 默认 5432
        assert_eq!(
            parse_pg_host_port("postgres://user:pw@host/db"),
            Some(("host".to_string(), 5432))
        );
        // 密码含未编码 @ → host 必须仍是 host，而不是 ss@host
        assert_eq!(
            parse_pg_host_port("postgres://user:p@ss@host:5432/db"),
            Some(("host".to_string(), 5432)),
            "取最后一个 @，否则 host 变成 ss@host ⇒ 必然连不上"
        );
        // percent-encode 过的密码
        assert_eq!(
            parse_pg_host_port("postgresql://user:p%40ss@host/db"),
            Some(("host".to_string(), 5432))
        );
        // 非 PG scheme / 无 @ → None
        assert_eq!(parse_pg_host_port("sqlite:/tmp/a.db"), None);
        assert_eq!(parse_pg_host_port("postgres://host/db"), None);
        // 密码含多个未编码 @ → host 仍须是最后一个 @ 之后的部分
        assert_eq!(
            parse_pg_host_port("postgres://u:a@b@c@host:5432/db"),
            Some(("host".to_string(), 5432))
        );
        // 端口不是数字 → 回退默认 5432，host 不得含端口残留
        assert_eq!(
            parse_pg_host_port("postgres://u:pw@host:notaport/db"),
            Some(("host".to_string(), 5432))
        );
    }

    /// IPv6 字面量必须走方括号分支，且返回的 host **不带方括号**。
    ///
    /// 修复前 `split_once(':')` 会把 `[::1]:5432` 切成 host=`[`、port=`::1]:5432`
    /// 解析失败后退回 5432 ⇒ 拿 `[` 当主机名去 connect ⇒ 必然失败 ⇒ 误报
    /// 「Postgres 不可达」⇒ 触发降级并把连接串打进日志。IPv6 回环是本地自建
    /// PG 的常见写法，这条是**当前就会踩**的路径，不是理论洁癖。
    ///
    /// 返回不带方括号是因为 `tokio::net::TcpStream::connect((host, port))` 走
    /// `ToSocketAddrs`，它要 `::1` 裸地址，`[::1]` 会解析失败。
    #[test]
    fn parse_pg_host_port_handles_ipv6_brackets() {
        assert_eq!(
            parse_pg_host_port("postgres://user:pw@[::1]:5432/db"),
            Some(("::1".to_string(), 5432)),
            "端口分隔符必须是 ] 之后那个 :，不是 host 内部的 :"
        );
        assert_eq!(
            parse_pg_host_port("postgres://user:pw@[::1]/db"),
            Some(("::1".to_string(), 5432)),
            "无端口 → 默认 5432"
        );
        assert_eq!(
            parse_pg_host_port("postgresql://user:pw@[2001:db8::1]:6543/db"),
            Some(("2001:db8::1".to_string(), 6543))
        );
        // 密码含未编码 @ 与 IPv6 同时出现：两层解析都得对
        assert_eq!(
            parse_pg_host_port("postgres://user:p@ss@[::1]:5432/db"),
            Some(("::1".to_string(), 5432))
        );
        // 开了方括号却没闭合 → None，不得猜
        assert_eq!(parse_pg_host_port("postgres://user:pw@[::1:5432/db"), None);
    }

    /// host 校验：合法形态放行；结构性破坏逐个拒绝，且**报错必须可操作**
    ///（断言报错里出现指导性关键词，而不是只断言 Err —— 报错不可操作等于
    /// 把用户堵在门外）。
    #[test]
    fn validate_pg_host_accepts_and_rejects() {
        // 合法：常见主机名 / IPv4 / 域名 / IPv6 方括号（含 v4 映射）
        for ok in [
            "localhost",
            "127.0.0.1",
            "db.internal",
            "MY-DB_1",
            "[::1]",
            "[2001:db8::1]",
            "[::ffff:192.168.0.1]",
        ] {
            assert!(validate_pg_host(ok).is_ok(), "应放行：{ok}");
        }
        // 非法：每条给出报错里必须出现的关键词
        let cases: &[(&str, &str)] = &[
            ("", "不能为空"),
            ("ho@st", "非法字符"),   // @ 改写 userinfo 边界
            ("host/db", "非法字符"), // / 伪造 path
            ("host?x", "非法字符"),  // ? 伪造 query
            ("host#x", "非法字符"),  // # 伪造 fragment
            ("ho st", "非法字符"),   // 中间空白
            (" host", "首尾空白"),
            ("host ", "首尾空白"),
            ("::1", "方括号"), // 裸 IPv6：必须提示改用 [::1]
            ("[::1", "未闭合"),
            ("[::1]:5432", "端口"),   // host 字段里带端口 → 会拼出双端口
            ("[::1]x", "端口"),       // ] 之后有残留（提示里点名端口规则）
            ("[zz::1]", "IPv6 字符"), // 方括号内非十六进制
        ];
        for (input, keyword) in cases {
            let e = validate_pg_host(input).expect_err("应拒绝");
            assert!(e.contains(keyword), "输入 {input:?} 的报错应提到 {keyword:?}，实际：{e}");
        }
    }

    /// database 校验：黑名单逐字符命中，报错解释**为什么**这个字符会坏事。
    #[test]
    fn validate_pg_database_accepts_and_rejects() {
        for ok in ["axagent", "axinvest", "my-db", "db_1", "AxAgent2026"] {
            assert!(validate_pg_database(ok).is_ok(), "应放行：{ok}");
        }
        let cases: &[(&str, &str)] = &[
            ("", "不能为空"),
            ("a/b", "伪造 path"),
            ("a?b", "伪造"),
            ("a#b", "伪造"),
            ("a@b", "userinfo"),
            ("a%b", "percent"),
            ("a b", "空白"),
            (" db", "首尾空白"),
            ("db ", "首尾空白"),
        ];
        for (input, keyword) in cases {
            let e = validate_pg_database(input).expect_err("应拒绝");
            assert!(e.contains(keyword), "输入 {input:?} 的报错应提到 {keyword:?}，实际：{e}");
        }
    }

    /// 守住**调用点接线**：降级日志的实际输出里不得出现明文密码。
    ///
    /// 为什么不能只测 dao 侧的脱敏函数：那只证明「函数是对的」，证明不了
    /// 「调用点真的用了它」。若将来有人把降级日志改回直接打 `db_url`，
    /// 函数侧测试依然全绿，而密码又回到日志里 —— 这正是本次事故的形态。
    ///
    /// 判据取**行为**（输出内容）而非源码文本形状：日志文案、格式串、
    /// 参数顺序怎么改都不影响这条断言。
    #[test]
    fn fallback_log_output_contains_no_plaintext_password() {
        // 密码含未编码 @ —— 事故形态（修复前日志里会出现 `ss`）
        let out = format_pg_fallback_log(
            "TCP 连接失败: 由于目标计算机积极拒绝，无法连接。",
            "postgres://user:p@ss@host:5432/db",
        );
        assert!(!out.contains("p@ss"), "日志不得含明文密码：{out}");
        assert!(!out.contains(":p@"), "日志不得含明文密码：{out}");
        assert!(out.contains("postgres://user:***@host:5432/db"), "{out}");
        // 原因文本必须保留，否则排查现场丢失
        assert!(out.contains("TCP 连接失败"), "{out}");

        // 常规密码也不得明文出现
        let out = format_pg_fallback_log("连接超时（3s）", "postgres://postgres:s3cret@h/db");
        assert!(!out.contains("s3cret"), "日志不得含明文密码：{out}");
        assert!(out.contains("postgres://postgres:***@h/db"), "{out}");
    }
}
