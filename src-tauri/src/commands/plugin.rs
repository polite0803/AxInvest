// SPDX-License-Identifier: AGPL-3.0-only

use axagent_agent_macro::agent_command;

use axagent_harness::CapabilityIndexer;
use tauri::{State, command};
use tracing::warn;

use crate::app_state::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::plugin as plugin_err;

#[agent_command(domain = plugin, safety = Safe, call_mode = StateOnly, description = "列出所有插件")]
#[command]
pub async fn plugin_list(state: State<'_, AppState>) -> Result<Vec<PluginSummaryDto>, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = plugin_manager.blocking_read();
        manager
            .list_plugins()
            .map(|plugins| {
                plugins
                    .into_iter()
                    .map(|p| {
                        let id = p.metadata.id.clone();
                        let event_stats =
                            manager.plugin_event_stats(&id).map(PluginEventStatsDto::from);
                        PluginSummaryDto {
                            id,
                            name: p.metadata.name,
                            version: p.metadata.version,
                            description: p.metadata.description,
                            kind: p.metadata.kind.to_string(),
                            enabled: p.enabled,
                            tools: p.tool_names,
                            mcp_servers: p.mcp_server_names,
                            skills: p.skill_names,
                            commands: p
                                .commands
                                .into_iter()
                                .map(|c| PluginCommandDto {
                                    name: c.name,
                                    description: c.description,
                                })
                                .collect(),
                            event_stats,
                        }
                    })
                    .collect()
            })
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })
    })
    .await
    .map_err(|e| format!("plugin list task panicked: {e}"))?
}

#[agent_command(domain = plugin, safety = Safe, call_mode = StateInput, description = "验证插件源并获取元数据")]
#[command]
pub async fn plugin_validate_source(
    state: State<'_, AppState>,
    source: String,
) -> Result<PluginManifestDto, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = plugin_manager.blocking_read();
        let manifest = manager.validate_plugin_source(&source).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
        Ok(PluginManifestDto {
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            permissions: manifest.permissions.iter().map(|p| p.as_str().to_string()).collect(),
            default_enabled: manifest.default_enabled,
            hooks: {
                let mut hooks = serde_json::Map::new();
                hooks.insert(
                    "PreToolUse".to_string(),
                    serde_json::Value::Array(
                        manifest
                            .hooks
                            .pre_tool_use
                            .iter()
                            .map(|s| serde_json::Value::String(s.clone()))
                            .collect(),
                    ),
                );
                hooks.insert(
                    "PostToolUse".to_string(),
                    serde_json::Value::Array(
                        manifest
                            .hooks
                            .post_tool_use
                            .iter()
                            .map(|s| serde_json::Value::String(s.clone()))
                            .collect(),
                    ),
                );
                hooks
            },
            tools: manifest
                .tools
                .iter()
                .map(|t| ToolDto { name: t.name.clone(), description: t.description.clone() })
                .collect(),
            mcp_servers: manifest
                .mcp_servers
                .iter()
                .map(|m| McpServerDto { name: m.name.clone(), command: m.command.clone() })
                .collect(),
            skills: manifest
                .skills
                .iter()
                .map(|s| SkillDto { name: s.name.clone(), path: s.path.clone() })
                .collect(),
            capabilities: manifest
                .capabilities
                .iter()
                .map(|c| PluginCapabilityDto {
                    seam: c.seam.clone(),
                    capability_type: c.capability_type.clone(),
                    version: c.version.clone(),
                    description: c.description.clone(),
                })
                .collect(),
        })
    })
    .await
    .map_err(|e| format!("plugin validate task panicked: {e}"))?
}

/// 插件源码静态审计（PLAN §12.2）：对**内联源码**做形式合规检查。
///
/// 与 [`plugin_validate_source`] 是两件事：后者处理「来源定位符」（URL / 包名 / 路径），
/// 最终读目录里的 manifest JSON，**从不接触 Rust 源码**；本命令的输入是源码文本本身，
/// 输出是禁项报告（`passed` / `violations` / `annotations`）。故二者不构成重复定义。
///
/// ⚠ 审计**不是安全边界**：`build.rs` 与 proc-macro 在编译期以宿主用户身份执行任意
/// 代码，任何源码级检查都能被绕过。它必须与 §12.3 的隔离构建（离线 + 独立
/// `--target-dir` + 无网络）配合使用 —— 详见 `crates/plugins/src/source_audit.rs`。
#[agent_command(
    domain = plugin,
    safety = Safe,
    call_mode = StateInput,
    description = "审计插件源码的禁项清单"
)]
#[command]
pub fn plugin_source_audit(
    files: Vec<axagent_plugins::SourceFile>,
) -> Result<axagent_plugins::SourceAuditReport, String> {
    Ok(axagent_plugins::audit_sources(&files))
}

/// 插件源码的隔离构建（PLAN §12.3 + §14.3）：前置探测 → 内容寻址 → 离线编译 → 落盘产物。
///
/// 与 [`plugin_source_audit`] 的分工：审计是**纯函数**（同输入同输出、秒级返回）；本命令会
/// **真的调起 `cargo build`**（分钟级、有副作用），故必须 `spawn_blocking` 而非占用 async
/// 工作线程。
///
/// 产物是一个**可直接安装的插件目录**：`<build_root>/<plugin_id>/<artifact_hash>/`，
/// 内含 `plugin.json`（根目录，供既有 `plugin_install` 装载）、`bin/<binary>` 与 `source/`
/// （源码快照）。`artifact_hash` 由源码 + 协议版本 + 目标平台 + 编译参数决定，**同 hash
/// 命中即跳过编译**（`cache_hit = true`）—— 这是 §13.1 内容寻址的兑现点。
///
/// ⚠ 隔离是**进程级**而非 OS 级：构建以 `--offline` 与独立 `--target-dir` 运行，但不阻断
/// `build.rs` 主动外联。真正的沙箱需要 OS 级能力（见 `crates/plugins/src/source_build.rs`
/// 头部「诚实边界」）。
#[agent_command(
    domain = plugin,
    safety = Caution,
    call_mode = StateInput,
    description = "隔离构建插件源码"
)]
#[command]
pub async fn plugin_source_build(
    state: State<'_, AppState>,
    request: axagent_plugins::SourceBuildRequest,
) -> Result<axagent_plugins::SourceBuildOutcome, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let build_root = plugin_manager.blocking_read().plugin_build_root();
        axagent_plugins::build_plugin_source(&build_root, &request).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error_with_code(
                source_build_error_code(&e),
                &e,
                source_build_category(&e),
            ))
        })
    })
    .await
    .map_err(|e| format!("plugin source build task panicked: {e}"))?
}

/// 把构建失败映射到前端可翻译的错误码（PLAN §12.3 / §14.3）。
///
/// 只有「工具链缺失」与「资源不足」值得单列错误码 —— 它们是**环境问题**，用户能自己去修
/// （装 rustup target / 清磁盘）；其余失败（编译报错、路径非法、产物缺失）都归入通用的
/// `SOURCE_BUILD_FAILED`，具体原因在 `detail` 里（`stderr` 尾部）。
fn source_build_error_code(error: &axagent_plugins::SourceBuildError) -> String {
    use axagent_plugins::SourceBuildError as E;
    match error {
        E::ToolchainUnavailable { .. } | E::TargetNotInstalled { .. } => {
            plugin_err::SOURCE_BUILD_TOOLCHAIN_UNAVAILABLE.to_string()
        },
        E::ResourceLow { .. } => plugin_err::SOURCE_BUILD_RESOURCE_LOW.to_string(),
        _ => plugin_err::SOURCE_BUILD_FAILED.to_string(),
    }
}

/// 构建失败的分类：决定前端是「提示修正输入」`Validation` / 「建议稍后重试」`Retryable`
/// 还是「显示错误并停止」`Unrecoverable`。
///
/// `ResourceLow` 归 `Retryable` 而非 `Unrecoverable`：磁盘/内存是**可恢复的暂时状态**
/// （关掉别的编译进程、清缓存后就能过）。
fn source_build_category(
    error: &axagent_plugins::SourceBuildError,
) -> crate::commands::error::ErrorCategory {
    use axagent_plugins::SourceBuildError as E;
    match error {
        E::InvalidPluginId { .. } | E::InvalidBinaryName { .. } => {
            crate::commands::error::ErrorCategory::Validation
        },
        E::ResourceLow { .. } => crate::commands::error::ErrorCategory::Retryable,
        _ => crate::commands::error::ErrorCategory::Unrecoverable,
    }
}

/// SECURITY (S9): 远程插件源（Git URL、npm 包）安装时无 SHA-256 完整性校验或签名验证。
/// 用户应从可信源（如官方 AxHub 市场）安装插件，避免安装来源不明的远程插件。
/// 前端应在安装前通过 `plugin_validate_source` 验证插件元数据。
#[agent_command(domain = plugin, safety = Caution, call_mode = StateInput, description = "安装插件")]
#[command]
pub async fn plugin_install(
    state: State<'_, AppState>,
    source: String,
) -> Result<InstallOutcomeDto, String> {
    // 安全日志：记录远程插件源安装（无完整性校验）
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
        || source.starts_with('@')
    {
        warn!(
            "SECURITY: Installing plugin from remote source without integrity verification: {}",
            source
        );
    }
    let plugin_manager = state.plugin_manager.clone();
    let outcome =
        tauri::async_runtime::spawn_blocking(move || -> Result<InstallOutcomeDto, String> {
            let mut manager = plugin_manager.blocking_write();
            let outcome = manager.install(&source).map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
            Ok(InstallOutcomeDto {
                plugin_id: outcome.plugin_id,
                version: outcome.version,
                install_path: outcome.install_path.display().to_string(),
            })
        })
        .await
        .map_err(|e| format!("plugin install task panicked: {e}"))??;

    // 插件表声明（PLAN-shell-plus-plugins §69.3）：安装成功后再收敛，失败则把
    // 「插件已装、建表失败」如实报给用户（下一次启用 / 启动会重试同一份声明）。
    crate::init::plugin_schema::reconcile_installed(
        state.harness.db(),
        &state.plugin_manager,
        Some(&outcome.plugin_id),
    )
    .await
    .map_err(|e| format!("插件已安装，但 schema.json 收敛失败：{e}"))?;
    // 插件 UI 贡献（任务 #149 第 2 步）：`ui.json` → `dynamic_ui_schemas`（origin=plugin）。
    // 与建表同批挂载，但**失败不回滚安装** —— 表声明失败会让插件跑不起来，UI 失败只是
    // 少一张表页；错误照样交给用户，下一次启用 / 启动会重试。
    crate::init::plugin_ui_schema::reconcile_ui_schemas(
        state.harness.db(),
        &state.plugin_manager,
        Some(&outcome.plugin_id),
    )
    .await
    .map_err(|e| format!("插件已安装，但 ui.json 收敛失败：{e}"))?;
    Ok(outcome)
}

#[agent_command(domain = plugin, safety = Caution, call_mode = StateInput, description = "启用指定插件")]
#[command]
pub async fn plugin_enable(state: State<'_, AppState>, plugin_id: String) -> Result<(), String> {
    let plugin_manager = state.plugin_manager.clone();
    let capability_indexer = state.capability_indexer.clone();
    let plugin_id_for_task = plugin_id.clone();
    let (enable_result, passports) = tauri::async_runtime::spawn_blocking(move || {
        let mut manager = plugin_manager.blocking_write();
        let result = manager.enable(&plugin_id_for_task).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        });
        // 护照构造为纯函数（同 manifest 稳定），启用后用于注册能力发现索引
        let passports = manager.passports_for_plugin(&plugin_id_for_task);
        (result, passports)
    })
    .await
    .map_err(|e| format!("plugin enable task panicked: {e}"))?;
    enable_result?;
    // 索引同步：把该插件的能力护照写入能力发现索引（async 上下文）
    for passport in &passports {
        if let Err(e) = capability_indexer.index_passport(passport).await {
            warn!("SECURITY: 插件 `{plugin_id}` 护照 `{}` 注册失败: {e}", passport.capability_id);
        }
    }
    // 插件表声明（PLAN-shell-plus-plugins §69.3）：覆盖「安装时建表失败」与
    // 「库被换掉」两种情况；已存在且一致时是幂等的（0 条执行）。
    crate::init::plugin_schema::reconcile_installed(
        state.harness.db(),
        &state.plugin_manager,
        Some(&plugin_id),
    )
    .await
    .map_err(|e| format!("插件已启用，但 schema.json 收敛失败：{e}"))?;
    // 插件 UI 贡献（任务 #149 第 2 步）：同样覆盖「安装时 UI 写入失败」与「库被换掉」。
    crate::init::plugin_ui_schema::reconcile_ui_schemas(
        state.harness.db(),
        &state.plugin_manager,
        Some(&plugin_id),
    )
    .await
    .map_err(|e| format!("插件已启用，但 ui.json 收敛失败：{e}"))?;
    Ok(())
}

#[agent_command(domain = plugin, safety = Caution, call_mode = StateInput, description = "禁用指定插件")]
#[command]
pub async fn plugin_disable(state: State<'_, AppState>, plugin_id: String) -> Result<(), String> {
    let plugin_manager = state.plugin_manager.clone();
    let capability_indexer = state.capability_indexer.clone();
    let plugin_id_for_task = plugin_id.clone();
    let (disable_result, ids) = tauri::async_runtime::spawn_blocking(move || {
        // 先取护照 ID（目录尚在），再禁用，供索引回滚
        let manager = plugin_manager.blocking_read();
        let ids: Vec<String> = manager
            .passports_for_plugin(&plugin_id_for_task)
            .into_iter()
            .map(|p| p.capability_id)
            .collect();
        drop(manager);
        let mut manager = plugin_manager.blocking_write();
        let result = manager.disable(&plugin_id_for_task).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        });
        (result, ids)
    })
    .await
    .map_err(|e| format!("plugin disable task panicked: {e}"))?;
    disable_result?;
    // 索引同步：移除该插件的能力护照（async 上下文）
    for id in ids {
        if let Err(e) = capability_indexer.remove_index(&id).await {
            warn!("SECURITY: 插件 `{plugin_id}` 护照 `{id}` 回滚失败: {e}");
        }
    }
    Ok(())
}

#[agent_command(domain = plugin, safety = Dangerous, call_mode = StateInput, description = "卸载指定插件")]
#[command]
pub async fn plugin_uninstall(state: State<'_, AppState>, plugin_id: String) -> Result<(), String> {
    let plugin_manager = state.plugin_manager.clone();
    let capability_indexer = state.capability_indexer.clone();
    let plugin_id_for_task = plugin_id.clone();
    let (uninstall_result, ids) = tauri::async_runtime::spawn_blocking(move || {
        // 先取护照 ID（卸载会删除插件目录），再卸载，供索引回滚
        let manager = plugin_manager.blocking_read();
        let ids: Vec<String> = manager
            .passports_for_plugin(&plugin_id_for_task)
            .into_iter()
            .map(|p| p.capability_id)
            .collect();
        drop(manager);
        let mut manager = plugin_manager.blocking_write();
        let result = manager.uninstall(&plugin_id_for_task).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        });
        (result, ids)
    })
    .await
    .map_err(|e| format!("plugin uninstall task panicked: {e}"))?;
    uninstall_result?;
    // 索引同步：移除该插件的能力护照（async 上下文）
    for id in ids {
        if let Err(e) = capability_indexer.remove_index(&id).await {
            warn!("SECURITY: 插件 `{plugin_id}` 护照 `{id}` 回滚失败: {e}");
        }
    }
    // UI 贡献撤销：删掉该插件注入的动态 UI Schema（origin=plugin 且 owner_id=pluginId）。
    // 失败只告警不回滚卸载：插件目录已删，留下孤儿 Schema 比让卸载「失败」更可接受
    // —— 用户可以手动在动态 UI 管理页删掉它们。
    match crate::commands::dynamic_ui::revoke_schemas_owned_by(state.harness.db(), &plugin_id).await
    {
        Ok(0) => {},
        Ok(n) => {
            tracing::info!(plugin_id = %plugin_id, count = n, "已撤销插件注入的动态 UI Schema")
        },
        Err(e) => warn!("插件 `{plugin_id}` 的 UI 贡献撤销失败（孤儿 Schema 需手动清理）: {e}"),
    }
    Ok(())
}

/// 插件 UI action 回流：把动态 UI 里声明的动作送回**插件自己的 worker 进程**。
///
/// 方向与「宿主 → 能力接缝」的调用相反（PLAN §10.5-2）：schema 由插件注入
/// （`origin = "plugin"`、`owner_id = pluginId`），用户在界面上触发的 action
/// 需要回到该插件处理，故这里按 `pluginId` 取 worker 调用面直接发一帧。
/// 插件未声明 `worker`（或当前未启用）时返回 `PLUGIN_UI_ACTION_UNAVAILABLE`。
#[agent_command(
    domain = plugin,
    safety = Caution,
    call_mode = StateInput,
    description = "把动态 UI 动作回流传给插件 worker"
)]
#[command]
pub async fn plugin_ui_action(
    state: State<'_, AppState>,
    plugin_id: String,
    action: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 取到调用面即释放读锁：帧往返要等插件算完（可能很慢甚至卡住），
        // 持锁等待会把整个 PluginManager 的读写一起拖住。
        let invoker = {
            let manager = plugin_manager.blocking_read();
            manager
                .plugin_invoker(&plugin_id)
                .ok_or_else(|| ErrorResponse::err(plugin_err::UI_ACTION_UNAVAILABLE))?
        };
        invoker
            .invoke(axagent_plugins::OP_UI_ACTION, action)
            .map_err(|e| ErrorResponse::err_with_detail(plugin_err::UI_ACTION_FAILED, e))
    })
    .await
    .map_err(|e| format!("plugin ui action task panicked: {e}"))?
}

/// 读取插件仪表盘面板声明的前端资产 —— 面板渲染宿主（沙箱 iframe）的内容来源。
///
/// 路径形状、canonicalize 逃逸校验、大小上限全部收在 `PluginManager::read_panel_asset` 内；
/// 本命令只做**错误归类**：「这个面板没有内容」与「内容被安全策略拒绝」的用户动作相反
/// （前者是插件的配置态，后者需要插件作者改 manifest），故分两个码。
#[agent_command(
    domain = plugin,
    safety = Safe,
    call_mode = StateInput,
    description = "读取插件仪表盘面板的前端资产"
)]
#[command]
pub fn plugin_read_panel_asset(
    state: State<'_, AppState>,
    plugin_id: String,
    panel_id: String,
) -> Result<axagent_plugins::PluginPanelAsset, String> {
    let plugin_manager = state.plugin_manager.clone();
    let manager = plugin_manager.blocking_read();
    manager.read_panel_asset(&plugin_id, &panel_id).map_err(|e| {
        use crate::commands::error::ErrorCategory as Cat;
        let (code, category) = match &e {
            axagent_plugins::PluginError::PermissionDenied(_) => {
                (plugin_err::PANEL_ASSET_BLOCKED, Cat::PermissionDenied)
            },
            axagent_plugins::PluginError::IntegrityUnpinned(_) => {
                (plugin_err::PANEL_ASSET_UNPINNED, Cat::Validation)
            },
            axagent_plugins::PluginError::IntegrityMismatch(_) => {
                (plugin_err::PANEL_ASSET_TAMPERED, Cat::Unrecoverable)
            },
            _ => (plugin_err::PANEL_ASSET_UNAVAILABLE, Cat::Validation),
        };
        String::from(ErrorResponse::from_error_with_code(code, e, category))
    })
}

/// 解析插件逐页声明（`manifest.pages[]`，§0.10.2 T3）条目的**绝对资产路径**。
///
/// 这是「通道 A = 插件自带 bundle 经 asset 协议装载」缺的那块输入（任务 #43）：
/// 前端把宿主绑定的 `(pluginId, pageId)` 交给后端，后端按清单解析出绝对路径，
/// 前端再交给 Tauri 的 `convertFileSrc` 换成 asset URL 给沙箱容器。
///
/// 与 [`plugin_read_panel_asset`] 的分工：那条把文本资产**读进内存**给 iframe `srcdoc`
/// （兜底通道 B，有大小上限）；本条只回答「内容在磁盘哪儿」，不读文件。两者共用
/// `PluginManager` 里同一套防线（清单形状 → 安装锚点 → canonicalize 包含性），
/// 且**都不接收前端传来的路径** —— 前端串改 `entry` 探测目录在此无处下手。
///
/// ⚠ 返回的 `absolutePath` 已额外校验「落在安装根内」：`tauri.conf.json` 的
/// `assetProtocol.scope` 只放行 `$HOME/.axagent/plugins/installed/*/**`，
/// 而 `external_dirs` 发现形态的插件不在该根里 —— 那种页面只能继续走 srcdoc 兜底，
/// 由本命令显式拒绝，比 webview 里一条 403 更可定位。
/// ⚠ 错误码暂时**复用面板那四个**（`PLUGIN_PANEL_ASSET_*`）：本批禁止改 11 份 locale
/// （新增码会全语言显示原始串，比措辞偏面板更糟）。待 locale 空出来应补
/// `PLUGIN_PAGE_ASSET_*` 一组并把这里换过去；精确原因在 `detail` 里。
#[agent_command(
    domain = plugin,
    safety = Safe,
    call_mode = StateInput,
    description = "解析插件页面声明的资产绝对路径"
)]
#[command]
pub fn plugin_resolve_page_asset(
    state: State<'_, AppState>,
    plugin_id: String,
    page_id: String,
) -> Result<axagent_plugins::PluginPageAssetLocation, String> {
    let plugin_manager = state.plugin_manager.clone();
    let manager = plugin_manager.blocking_read();
    manager.resolve_page_asset(&plugin_id, &page_id).map_err(|e| {
        use crate::commands::error::ErrorCategory as Cat;
        let (code, category) = match &e {
            axagent_plugins::PluginError::PermissionDenied(_) => {
                (plugin_err::PANEL_ASSET_BLOCKED, Cat::PermissionDenied)
            },
            axagent_plugins::PluginError::IntegrityUnpinned(_) => {
                (plugin_err::PANEL_ASSET_UNPINNED, Cat::Validation)
            },
            axagent_plugins::PluginError::IntegrityMismatch(_) => {
                (plugin_err::PANEL_ASSET_TAMPERED, Cat::Unrecoverable)
            },
            _ => (plugin_err::PANEL_ASSET_UNAVAILABLE, Cat::Validation),
        };
        String::from(ErrorResponse::from_error_with_code(code, e, category))
    })
}

/// 插件页面清单 —— 逐页声明（`manifest.pages[]`）的只读投影（任务 #75）。
///
/// 真相源是 `PluginManifest.pages`，每次调用实时扫描已安装插件清单；**不缓存、不建
/// 第二套路由表**（禁区 4、`PLAN-shell-plus-plugins.md` §0.7.6）。安装 / 启停 / 卸载
/// 仍走既有 `plugin_*` 命令（自带护照索引同步与 UI 贡献撤销），此处不造第二套生命周期。
#[agent_command(domain = plugin, safety = Safe, call_mode = StateOnly, description = "列出插件页面")]
#[command]
pub async fn plugin_list_pages(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_plugins::types::PluginPagesInfo>, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = plugin_manager.blocking_read();
        manager.list_plugin_pages().map_err(|e| {
            String::from(ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
    })
    .await
    .map_err(|e| format!("plugin list pages task panicked: {e}"))?
}

/// 任务中心任务源清单 —— `manifest.taskSources[]` 的只读投影（任务 #148，PLAN §97.4）。
///
/// 与 [`plugin_list_pages`] 同形：真相源是 `PluginManifest.task_sources`，每次调用实时扫描，
/// **不缓存、不建第二份源注册表**（禁区 4）。`enabled` 位原样透出，
/// 启停过滤与「与宿主源重名时宿主优先」的裁决都留给消费侧
/// （`src/pages/TasksCenterPage/adapters/registry.ts`）。
#[agent_command(
    domain = plugin,
    safety = Safe,
    call_mode = StateOnly,
    description = "列出插件贡献的任务源"
)]
#[command]
pub async fn plugin_list_task_sources(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_plugins::types::PluginTaskSourcesInfo>, String> {
    let plugin_manager = state.plugin_manager.clone();
    let mut infos = tauri::async_runtime::spawn_blocking(move || {
        let manager = plugin_manager.blocking_read();
        manager.list_plugin_task_sources().map_err(|e| {
            String::from(ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
    })
    .await
    .map_err(|e| format!("plugin list task sources task panicked: {e}"))??;
    // 运行期复核 `detailSchema` 的归属（任务 #149 第 2 步）：安装期校验只证明「当时声明过」，
    // 库被换掉 / 行被剪掉 / ui.json 被删都会让那份 key 变成悬空引用。复核不过就置 `None`
    // —— 详情槽退化成「无详情」，绝不把 key 原样交给前端去加载一张别人的或不存在的地表。
    crate::init::plugin_ui_schema::retain_owned_detail_schemas(state.harness.db(), &mut infos)
        .await?;
    Ok(infos)
}

#[agent_command(domain = plugin, safety = Caution, call_mode = StateInput, description = "更新指定插件")]
#[command]
pub async fn plugin_update(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<UpdateOutcomeDto, String> {
    let plugin_manager = state.plugin_manager.clone();
    let outcome =
        tauri::async_runtime::spawn_blocking(move || -> Result<UpdateOutcomeDto, String> {
            let mut manager = plugin_manager.blocking_write();
            let outcome = manager.update(&plugin_id).map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
            Ok(UpdateOutcomeDto {
                plugin_id: outcome.plugin_id,
                old_version: outcome.old_version,
                new_version: outcome.new_version,
                install_path: outcome.install_path.display().to_string(),
            })
        })
        .await
        .map_err(|e| format!("plugin update task panicked: {e}"))??;

    // 新版本可能新增表声明；旧表**不删**（additive-only，见 PLAN §69.2）。
    crate::init::plugin_schema::reconcile_installed(
        state.harness.db(),
        &state.plugin_manager,
        Some(&outcome.plugin_id),
    )
    .await
    .map_err(|e| format!("插件已更新，但 schema.json 收敛失败：{e}"))?;
    // UI 贡献**会剪枝**（与表不同）：新版 `ui.json` 里删掉的 key 对应的行要一起清掉，
    // 否则升级后旧表单页在界面上残留、点开就是一张已经不存在的表。
    crate::init::plugin_ui_schema::reconcile_ui_schemas(
        state.harness.db(),
        &state.plugin_manager,
        Some(&outcome.plugin_id),
    )
    .await
    .map_err(|e| format!("插件已更新，但 ui.json 收敛失败：{e}"))?;
    Ok(outcome)
}

/// 执行插件声明的命名命令（阶段3-① 分发入口，`PLAN-plugin-gap-closure.md` §2）。
///
/// 走 [`axagent_plugins::PluginManager::execute_plugin_command`]，以下为该链路的**实测事实**，
/// 前端/调用方据此判断风险，不要按字面「沙箱」理解成隔离：
/// **已长驻 worker 的插件（manifest 声明 `worker` 且启用成功）走命令帧通道**（任务 #49）：
/// 复用启用时的常驻子进程、一次调用一帧往返、宿主强制「预算 + 收帧宽限」的等待上限
/// （超时映射 `COMMON_TIMEOUT`，前端可按重试分支处置），输出为命令回执的 JSON 文本。
/// **未长驻 worker 的插件维持 spawn-on-call 通道**，语义逐字节不变，以下为该通道的实测事实：
/// 命令串整团作为程序名交给 `Command::new`（**没有**按空白切分成程序 + 参数，`args` 恒空），
/// 输入 JSON 经 stdin 与 `CLAWD_TOOL_INPUT` 注入，stdout 原样返回。
/// 程序名解析与 tool 侧同源（任务 #47）：带 `./`、`../` 前缀的相对路径声明先按插件安装目录
/// 拼成绝对路径（可执行文件查找只看宿主 cwd，原样传下去必然 spawn 失败），绝对路径逐字保留，
/// **不带前缀的串仍按字面命令交给 PATH 查找**。
/// 沙箱只做 `subprocess_execution` 门槛 + ENV 白名单（`env_clear` 后回填白名单），
/// **不对子进程做任何路径或写权限限制**。
/// 命令声明的 `requiredPermission` 档位是**宿主侧授权门槛（authorization gate）**，
/// 不是执行隔离（containment）：档位不满足时后端返回 `PermissionDenied` 且不启动子进程。
#[agent_command(domain = plugin, safety = Caution, call_mode = StateInput, description = "执行插件命名命令")]
#[command]
pub async fn plugin_execute_command(
    state: State<'_, AppState>,
    plugin_id: String,
    name: String,
    input: Option<serde_json::Value>,
) -> Result<String, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = plugin_manager.blocking_read();
        manager
            .execute_plugin_command(
                &plugin_id,
                &name,
                &input.unwrap_or_else(|| serde_json::json!({})),
            )
            .map_err(|e| String::from(plugin_command_error_response(&e)))
    })
    .await
    .map_err(|e| format!("plugin execute_command task panicked: {e}"))?
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummaryDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub kind: String,
    pub enabled: bool,
    pub tools: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub skills: Vec<String>,
    /// 插件声明的命名命令（阶段3-① 执行分发的可见面，`PLAN-plugin-gap-closure.md` §2）。
    pub commands: Vec<PluginCommandDto>,
    /// 事件投递计数（任务 #135，B3「丢弃计数」的可见面）。
    ///
    /// `None` 与「全零」是两种状态，处置动作不同：
    /// - `None` ⇒ 这个插件**当前没有挂着的事件订阅**（未声明 `event.dispatch`、
    ///   声明了但装载时总线尚未注册、或插件根本没启用）；
    /// - `Some{delivered:0, 各 dropped:0}` ⇒ 订阅挂着，只是一条事件还没来。
    ///
    /// 判据与 `PluginManager::plugin_event_stats` 逐字同源，不在命令层另立。
    pub event_stats: Option<PluginEventStatsDto>,
}

/// [`axagent_plugins::worker::EventDeliveryStats`] 的线上形状。
///
/// `dropped_total` 是 `serialize + invoke + wait` 三桶之和；`dropped_unavailable`
/// 是 `dropped_invoke` 的**子集**（脏闸拒单那一类），所以它**不参与**求和，
/// 前端把它加进总数会算出两倍丢弃。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEventStatsDto {
    pub delivered: u64,
    pub dropped_total: u64,
    pub dropped_serialize: u64,
    pub dropped_invoke: u64,
    /// `dropped_invoke` 的子集：连接脏闸拒单（处置=禁用再启用该插件）。
    pub dropped_unavailable: u64,
    pub dropped_wait: u64,
}

impl From<axagent_plugins::worker::EventDeliveryStats> for PluginEventStatsDto {
    fn from(s: axagent_plugins::worker::EventDeliveryStats) -> Self {
        Self {
            delivered: s.delivered,
            dropped_total: s.dropped_total(),
            dropped_serialize: s.dropped_serialize,
            dropped_invoke: s.dropped_invoke,
            dropped_unavailable: s.dropped_unavailable,
            dropped_wait: s.dropped_wait,
        }
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommandDto {
    pub name: String,
    pub description: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifestDto {
    pub name: String,
    pub version: String,
    pub description: String,
    pub permissions: Vec<String>,
    pub default_enabled: bool,
    pub hooks: serde_json::Map<String, serde_json::Value>,
    pub tools: Vec<ToolDto>,
    pub mcp_servers: Vec<McpServerDto>,
    pub skills: Vec<SkillDto>,
    pub capabilities: Vec<PluginCapabilityDto>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityDto {
    pub seam: String,
    pub capability_type: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDto {
    pub name: String,
    pub description: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerDto {
    pub name: String,
    pub command: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDto {
    pub name: String,
    pub path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcomeDto {
    pub plugin_id: String,
    pub version: String,
    pub install_path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcomeDto {
    pub plugin_id: String,
    pub old_version: String,
    pub new_version: String,
    pub install_path: String,
}

/// 命名命令派发口的错误归类（任务 #49）：`PluginError` → 结构化 `ErrorResponse`。
///
/// 禁止裸字符串：所有分支都产出「码 + 分类 + 详情」三元组，前端按 `category` 做
/// 重试/授权/放弃分支，按 `code` 走 i18n。原先所有失败塌进
/// `from_error(.., Unrecoverable)`（`COMMON_INTERNAL`），命令超时与「插件没跑出来」
/// 无法区分 —— 现在超时是 `COMMON_TIMEOUT` / `general`。
///
/// ⚠ **超时不得标记为 `Retryable`**：宿主只是「没等到」结果，worker 侧的调用可能
/// 仍在执行并已产生副作用（见 `worker.rs:1463` 的「超时不等价于取消」说明）。若归为
/// `Retryable`，前端按分类自动/引导重试即构成**二次执行**。故归默认的 `general`：
/// 仍按 error 级展示，但不进自动重试分支。调用方若确需重试，必须先确认该命令幂等。
///
/// ⚠ **本批刻意不新增错误码**：11 份 locale 文件当前是他会话未提交改动，而
/// `check-errorcode-alignment.mjs` 要求「后端码 ⊆ zh-CN error 段」，只加码不补译 CI
/// 直接红。故复用既有已译码；待 locale 解冻应补 `PLUGIN_COMMAND_TIMEOUT` /
/// `PLUGIN_COMMAND_FAILED` / `PLUGIN_COMMAND_NOT_FOUND` /
/// `PLUGIN_COMMAND_PERMISSION_DENIED` 专码族（清单见任务 #49 交接报告），
/// 只需把下面的常量换成 `error_code::plugin::*`，分类逻辑不动。
///
/// `pub(crate)`（任务 #170）：**智能体侧同一个映射的唯一入口**。此前本函数是模块私有，
/// 于是 `commands/agent/command_bridge.rs::execute_plugin_agent_command` 只能把
/// `PluginError` 拼成裸中文串，LLM 拿到的是「插件命令执行失败：xxx」这种既无码也无
/// 分类的文本 —— 同一个失败，前端路径有结构化错误、智能体路径没有。两处共用一份映射
/// 而不是复制第二份（禁区 12：禁止重复定义）。
pub(crate) fn plugin_command_error_response(
    error: &axagent_plugins::PluginError,
) -> crate::commands::error::ErrorResponse {
    use crate::commands::error::ErrorCategory;
    use crate::commands::error_code::common;
    use axagent_plugins::PluginError;
    let detail = error.to_string();
    match error {
        // 预算耗尽、结果未回：**不等于取消**，worker 可能仍在跑完（副作用已发生），
        // 故归默认 `general`，不引导自动重试；仅在调用方确认命令幂等后才可重试。
        PluginError::CommandTimeout(_) => {
            ErrorResponse::new(common::TIMEOUT).with_category(ErrorCategory::General).with_detail(
                format!("{detail}；超时不等于取消：插件可能仍在执行，重试可能导致副作用重复"),
            )
        },
        // 授权门槛拒绝：引导用户核对插件权限声明，不重试。
        PluginError::PermissionDenied(_) => ErrorResponse::new(common::INTERNAL)
            .with_category(ErrorCategory::PermissionDenied)
            .with_detail(detail),
        // 插件/命令不存在：输入侧可修正（改名、重装、换插件）。
        PluginError::NotFound(_) => ErrorResponse::new(common::INVALID_INPUT)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail),
        // 其余（worker 帧往返失败、worker 自报协议码、形状校验……）：命令跑出了错，
        // 协议码保留在 detail 原文里（如 `PLUGIN_COMMAND_FAILED: …`），不吞。
        other => ErrorResponse::new(common::INTERNAL)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 映射结果必须能按 `ErrorResponse` 解析回来（不是裸字符串），并返回 (code, category)。
    fn mapped(error: &axagent_plugins::PluginError) -> (String, String, Option<String>) {
        let text = String::from(plugin_command_error_response(error));
        let value: serde_json::Value =
            serde_json::from_str(&text).expect("错误串必须是可解析的 ErrorResponse JSON");
        (
            value["code"].as_str().unwrap_or_default().to_string(),
            value["category"].as_str().unwrap_or_default().to_string(),
            value["detail"].as_str().map(str::to_string),
        )
    }

    #[test]
    fn command_timeout_is_not_marked_retryable() {
        let err = axagent_plugins::PluginError::CommandTimeout(
            "插件 `demo@external` 的 `command` 超过宿主等待上限 1200 ms，调用已放弃".to_string(),
        );
        let (code, category, detail) = mapped(&err);
        assert_eq!(code, "COMMON_TIMEOUT");
        assert_eq!(
            category, "general",
            "超时不是「可重试」：worker 可能仍在执行，自动重试会造成副作用重复"
        );
        let detail = detail.expect("超时必须有详情");
        assert!(detail.contains("超过宿主等待上限"));
        assert!(
            detail.contains("超时不等于取消"),
            "detail 必须显式提示超时≠取消，实际为：{detail}"
        );
    }

    #[test]
    fn command_failure_keeps_protocol_code_in_detail() {
        let err = axagent_plugins::PluginError::CommandFailed(
            "与插件 `demo@external` 的通道错误：PLUGIN_COMMAND_FAILED: worker 侧故意失败"
                .to_string(),
        );
        let (code, category, detail) = mapped(&err);
        assert_eq!(code, "COMMON_INTERNAL");
        assert_eq!(category, "unrecoverable");
        assert!(
            detail.expect("失败必须有详情").contains("PLUGIN_COMMAND_FAILED"),
            "协议码必须留在 detail，不得吞掉"
        );
    }

    #[test]
    fn not_found_and_permission_denied_get_distinct_categories() {
        let (code, category, _) =
            mapped(&axagent_plugins::PluginError::NotFound("没有声明命令 x".to_string()));
        assert_eq!(code, "COMMON_INVALID_INPUT");
        assert_eq!(category, "validation");

        let (code, category, _) =
            mapped(&axagent_plugins::PluginError::PermissionDenied("档位不足".to_string()));
        assert_eq!(code, "COMMON_INTERNAL");
        assert_eq!(category, "permission_denied");
    }
}
