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
                    .map(|p| PluginSummaryDto {
                        id: p.metadata.id,
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
                            .map(|c| PluginCommandDto { name: c.name, description: c.description })
                            .collect(),
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
    tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| format!("plugin install task panicked: {e}"))?
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

#[agent_command(domain = plugin, safety = Caution, call_mode = StateInput, description = "更新指定插件")]
#[command]
pub async fn plugin_update(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<UpdateOutcomeDto, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| format!("plugin update task panicked: {e}"))?
}

/// 执行插件声明的命名命令（阶段3-① 分发入口，`PLAN-plugin-gap-closure.md` §2）。
///
/// 走 [`axagent_plugins::PluginManager::execute_plugin_command`]：与 tool 同一
/// ENV 白名单沙箱 + `subprocess_execution` 权限门槛；输入 JSON 经 stdin /
/// `CLAWD_TOOL_INPUT` 注入（复用工具执行链），stdout 原样返回。
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
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })
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
