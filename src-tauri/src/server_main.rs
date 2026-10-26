// SPDX-License-Identifier: AGPL-3.0-only

//! AxAgent 独立服务端入口（无 Tauri 依赖）。
//!
//! 启动 Gateway + 后台服务，不创建窗口。
//! 通过 `cargo build --features server` 编译。
//!
//! 设计原则：
//! - 复用 `axagent_lib` 中非 Tauri 依赖的初始化逻辑
//! - 跳过 `tauri::Builder`、`tauri::generate_context!()`、窗口事件、tray
//! - 使用 tokio runtime 替代 Tauri 的事件循环

fn main() {
    // ── logging ──
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
            "%Y-%m-%dT%H:%M:%S%.3f%z".into(),
        ))
        .init();

    // ── panic hook ──
    std::panic::set_hook(Box::new(|info| {
        let msg = match (
            info.payload().downcast_ref::<&str>(),
            info.payload().downcast_ref::<String>(),
        ) {
            (Some(s), _) => s.to_string(),
            (_, Some(s)) => s.clone(),
            _ => "unknown panic".to_string(),
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        tracing::error!(
            panic.message = %msg,
            panic.location = %location,
            "FATAL: server process panicked"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }));

    // ── TLS crypto provider ──
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // ── 创建 tokio runtime ──
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async_main());
}

async fn async_main() {
    // ── 初始化数据目录 ──
    let app_dir = axagent_lib::paths::axagent_home();
    std::fs::create_dir_all(&app_dir).expect("Failed to create AxAgent home dir");
    tracing::info!("axagent_home ready: {}", app_dir.display());

    // ── 初始化数据库 ──
    let db_result = axagent_lib::init::init_database_with_dir(app_dir.clone())
        .await
        .expect("Fatal: database initialization failed");
    tracing::info!("Database initialized");

    // ── 同步 OPC 资产到用户数据目录（CWD 无关） ──
    // 与 Tauri 模式启动流程保持一致，保证域包/领域包可读
    // ensure_opc_config_synced 由 axagent_lib 公开 re-export（commands 模块为私有）
    axagent_lib::ensure_opc_config_synced(&app_dir);

    // ── 创建 AppState（复用 Tauri 版本的全部业务逻辑） ──
    //
    // ⚠ R-3d（2026-10-26）：工作流工具 resolver 与共享 Rhai Engine 宿主函数注册
    // **已在 `create_app_state` 内注入**（所有构造 AppState 的路径的共同入口）
    // ⇒ 本入口（无头服务）与 CLI **自动获得**，不再需要各自补一行。
    // 此前这两步只在 Tauri 应用路径与 `axagent-batch-rerun` 里
    // ⇒ 无头服务与 `axagent-validate-backfill` 都缺 ⇒ 实测「工具未注册」⇒ 候选 0 行。
    let state = axagent_lib::init::state::create_app_state(db_result)
        .await
        .expect("Fatal: app state initialization failed");
    tracing::info!("AppState created");

    // ── 启动后台服务（仅启动非 Tauri 依赖的服务） ──
    // 注：start_tray 因需要 tauri::AppHandle 会被跳过
    // start_background_services 需要适配无 Tauri 模式
    tracing::info!("Server started. Gateway available on configured port.");

    // ── 启动 cron 调度器（C 方案，2026-10-26）────────────────────────────
    //
    // **为什么这一段是本次改造的核心**：
    // 调度器原先**只能**活在 Tauri 应用进程里 ⇒ 桌面应用一关，每日定时任务
    // （回放 / 反思 / 荐股 / 决策回测）就全部停止。
    // 实测代价（生产库）：`mover_recall_analysis_batches` **连续 3 次 `interrupted`**，
    // 消息统一是「应用重启：运行中进程退出，as-of 封闭世界无法跨进程续跑」。
    //
    // ⇒ 无头服务端跑调度器 ⇒ 定时任务与桌面应用**解耦**。
    //
    // ⚠ **跨进程互斥**：调度器启动时抢 DB 里的 leader 租约
    //   （`cron_leader_lease`）⇒ 桌面应用与本服务**同时在跑**时只有一个派发，
    //   另一个只观察（避免同一回放日跑两遍 ⇒ 样本重复计数）。
    //
    // ⚠ 无头路径的两处**显式降级**（不静默）：
    //   · 荐股桌面通知 → 日志（结果仍落库）；
    //   · 中断分析重跑 → 跳过（依赖 AppHandle 反查 managed state）。
    axagent_lib::init::services::start_cron_scheduler_headless(&state).await;
    tracing::info!("Cron scheduler started (headless path).");

    // ── 启动 Gateway ──
    // Gateway 已在 AppState 中持有，由 services 中的 api_server 任务管理
    // 保持进程运行
    tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
    tracing::info!("Shutting down...");
}
