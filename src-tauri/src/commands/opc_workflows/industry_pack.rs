// SPDX-License-Identifier: AGPL-3.0-only

//! 行业数据资产包（Industry Pack）引擎
//!
//! 行业 = 数据资产包，非代码。每个行业一个独立目录：
//! `config/opc/industries/{industry_id}/`
//!   ├── manifest.yaml     # id / name / icon / version / enabled
//!   ├── roles.yaml        # 行业角色映射（opc-cfo 等 → 专家/工具白名单）
//!   └── workflows/*.yaml  # 工作流模板（纯数据，节点/边/prompt）
//!
//! 启动扫描注册到 `opc_industries` 表，支持单独启用/禁用/导出/导入。
//! 行业级版本号取代全局 OPC_TEMPLATE_VERSION，行业间互不影响。

use axagent_harness::util_fns::now_ts;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 行业包根目录（相对仓库根）
pub const INDUSTRIES_DIR: &str = "config/opc/industries";

// ── manifest.yaml schema ──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct IndustryManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 分析配置文件（P0-4 四件套之一），缺省 "analysis.yaml"，None 表示无分析配置
    /// 供 `analysis_schema::load_industry_analysis` 读取（industry_pack.rs:286）
    #[serde(default = "default_analysis_file")]
    pub analysis: String,
    /// 学习配置文件（P0-4 四件套之一），缺省 "learning.yaml"；读取见
    /// `opc_industry_actions::industry_learning_config_path`
    #[allow(dead_code)]
    #[serde(default = "default_learning_file")]
    pub learning: String,
}

fn default_analysis_file() -> String {
    "analysis.yaml".into()
}
fn default_learning_file() -> String {
    "learning.yaml".into()
}

fn default_icon() -> String {
    "🏢".into()
}
fn default_version() -> i32 {
    1
}
fn default_true() -> bool {
    true
}

// ── 包加载 ────────────────────────────────────────────────────────

/// 扫描行业包目录，返回所有 manifest（含是否启用）。
pub fn scan_industry_packs(base_dir: &Path) -> Vec<IndustryManifest> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(base_dir) else { return out };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("manifest.yaml");
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else { continue };
        match serde_yaml::from_str::<IndustryManifest>(&raw) {
            Ok(m) => {
                out.push(m);
            },
            Err(e) => {
                tracing::warn!("[industry-pack] {} manifest 解析失败: {e}", dir.display());
            },
        }
    }
    out
}

// P0-4 定义 schema；Phase 1 数据接入层已消费 data_sources/quality_precheck。
// strategies/risk 字段由 P2（分析策略维度）消费，当前未被读取的字段交给编译器逐项报告。
pub mod analysis_schema {
    use serde::Deserialize;
    use std::path::{Path, PathBuf};

    use super::IndustryManifest;

    /// 行业分析配置（`analysis.yaml`，由 manifest.analysis 字段引用，缺省同名文件）。
    ///
    /// 供数据接入层（OpIndustryVendor 路由）、分析层（策略维度）与
    /// 质量预检（QualityPrecheck 源清单）消费。P0-4 先定义 schema 与加载，
    /// 执行逻辑在 P1/P2 接入。
    // analysis.yaml 是配置契约（yaml 文件字段齐全），P2 分析策略维度接入后消费
    // strategies/risk/quality_precheck，届时移除豁免。
    #[derive(Debug, Clone, Default, Deserialize)]
    #[allow(dead_code)]
    pub struct IndustryAnalysisConfig {
        #[serde(default)]
        pub version: u32,
        #[serde(default)]
        pub industry_id: String,
        /// 数据源声明（vendor 链按优先级）
        #[serde(default)]
        pub data_sources: Vec<AnalysisDataSource>,
        /// 分析策略（行业专属分析维度）
        #[serde(default)]
        pub strategies: Vec<AnalysisStrategy>,
        /// 风控参数（对齐 position_limits 的行业版）
        #[serde(default)]
        pub risk: AnalysisRisk,
        /// 质量预检源清单（对齐 stock QualityPrecheck 的行业版）
        #[serde(default)]
        pub quality_precheck: Vec<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct AnalysisDataSource {
        pub id: String,
        /// vendor 链（按优先级：db / cache / web / file / astock）
        #[serde(default)]
        pub chain: Vec<String>,
        /// 是否纳入质量预检
        #[serde(default)]
        pub quality_precheck: bool,
    }

    /// P2（分析策略维度）接入后消费
    #[derive(Debug, Clone, Deserialize)]
    #[allow(dead_code)]
    pub struct AnalysisStrategy {
        pub id: String,
        #[serde(default)]
        pub name: String,
        /// 分析维度（如 cash_flow_health / tax_risk）
        #[serde(default)]
        pub dimensions: Vec<String>,
    }

    /// P2（风控参数维度）接入后消费
    #[derive(Debug, Clone, Default, Deserialize)]
    #[allow(dead_code)]
    pub struct AnalysisRisk {
        /// 超阈值告警线（0-1）
        #[serde(default)]
        pub max_kpi_warning_pct: f64,
        /// 关键 KPI 清单（越界触发风控拦截）
        #[serde(default)]
        pub critical_kpis: Vec<String>,
    }

    /// 读取行业包内分析配置（`{manifest.analysis}`，缺省 analysis.yaml）。
    /// 文件缺失返回 None（向后兼容：旧行业包无分析配置）。
    pub fn load_industry_analysis(
        industry_dir: &Path,
        manifest: &IndustryManifest,
    ) -> Option<IndustryAnalysisConfig> {
        let path = industry_dir.join(&manifest.analysis);
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_yaml::from_str(&raw) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!("[industry-pack] {} analysis 解析失败: {e}", path.display());
                None
            },
        }
    }

    /// 行业包完整资产（P0-4：manifest + analysis；workflows 已迁移至手动 seed 文件）
    /// analysis/pack_dir 当前仅 load_industry_pack 装配，由 P1 数据接入层完整消费后移除豁免。
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub struct IndustryPackBundle {
        pub manifest: IndustryManifest,
        pub analysis: Option<IndustryAnalysisConfig>,
        /// 学习配置在 `{industry_dir}/{manifest.learning}`（P4-3 已迁入行业包），
        /// 此处不重复解析，读取走 `opc_industry_actions::industry_learning_config_path`
        pub pack_dir: PathBuf,
    }

    /// 加载单个行业包目录的完整资产（manifest 解析失败返回 None）。
    pub fn load_industry_pack(dir: &Path) -> Option<IndustryPackBundle> {
        let manifest_path = dir.join("manifest.yaml");
        let raw = std::fs::read_to_string(&manifest_path).ok()?;
        let manifest: IndustryManifest = serde_yaml::from_str(&raw).ok()?;
        let analysis = load_industry_analysis(dir, &manifest);
        Some(IndustryPackBundle { manifest, analysis, pack_dir: dir.to_path_buf() })
    }
}

// ── 注册与 seed ───────────────────────────────────────────────────

/// 将行业包注册进 opc_industries 表（存在则按 version 判断是否升级）。
pub async fn upsert_industry_registry(
    db: &DatabaseConnection,
    m: &IndustryManifest,
) -> Result<(), String> {
    use axagent_entities::opc_industries;
    use sea_orm::*;

    let now = now_ts();
    // P1-5：保留用户手动禁用状态——DB 已有记录时以 DB enabled 为准，
    // manifest.enabled 仅首次插入生效（否则重启会把用户禁用的行业自动重新启用）。
    let existing = opc_industries::Entity::find_by_id(&m.id).one(db).await.ok().flatten();
    let effective_enabled = existing.map(|e| e.enabled != 0).unwrap_or(m.enabled);
    let am = opc_industries::ActiveModel {
        id: Set(m.id.clone()),
        name: Set(m.name.clone()),
        icon: Set(m.icon.clone()),
        description: Set(m.description.clone()),
        version: Set(m.version),
        enabled: Set(effective_enabled as i32),
        pack_path: Set(format!("{INDUSTRIES_DIR}/{}", m.id)),
        installed_at: Set(now),
        updated_at: Set(now),
    };
    opc_industries::Entity::insert(am)
        .on_conflict(
            sea_query::OnConflict::column(opc_industries::Column::Id)
                .update_column(opc_industries::Column::Name)
                .update_column(opc_industries::Column::Icon)
                .update_column(opc_industries::Column::Description)
                .update_column(opc_industries::Column::Version)
                .update_column(opc_industries::Column::Enabled)
                .update_column(opc_industries::Column::PackPath)
                .update_column(opc_industries::Column::UpdatedAt)
                .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .map_err(|e| format!("upsert industry: {e}"))?;
    Ok(())
}

/// 行业包完整 seed：扫描目录 → 注册表（opc_industries）。
///
/// ⚠️ 架构变更：行业工作流已迁移至手动定义的 seed 文件（见 mod.rs `seed_opc_industries_from_seed_files`），
/// 本函数仅负责 manifest 注册（opc_industries 表），不再从 YAML 加载工作流。
///
/// 返回 seed 的行业 id 列表。
pub async fn ensure_opc_industries_seeded(
    db: &DatabaseConnection,
    base_dir: &Path,
) -> Result<Vec<String>, String> {
    use axagent_entities::opc_industries;
    use sea_orm::EntityTrait;

    let manifests = scan_industry_packs(base_dir);
    let mut seeded = Vec::new();

    for m in manifests {
        // 版本判断：读 DB 现有记录（seed 前，避免 registry upsert 自引用）
        let existing = opc_industries::Entity::find_by_id(&m.id).one(db).await.ok().flatten();
        // P1-5：生效 enabled 以 DB 为准（用户手动禁用优先于 manifest），manifest 仅首装生效
        let effective_enabled = existing.as_ref().map(|e| e.enabled != 0).unwrap_or(m.enabled);
        let already_seeded = existing.as_ref().map(|e| e.version >= m.version).unwrap_or(false);

        // 注册表 upsert（记录当前包状态，enabled 保留 DB 用户状态）
        upsert_industry_registry(db, &m).await?;

        if already_seeded {
            seeded.push(m.id.clone());
            continue;
        }

        if !effective_enabled {
            tracing::info!("[industry-pack] {} 已禁用，跳过注册", m.id);
            continue;
        }

        // 行业工作流已由手动定义的 seed 文件生成（seed_opc_industries_from_seed_files），
        // 此处仅注册 manifest 到 opc_industries 表。
        tracing::info!(
            "[industry-pack] {} manifest 注册完成（v{}，工作流由手动 seed 文件提供）",
            m.id,
            m.version
        );
        seeded.push(m.id.clone());
    }
    Ok(seeded)
}

/// 供测试/工具使用：给定行业 id 的包目录路径。
pub fn industry_pack_dir(base_dir: &Path, id: &str) -> PathBuf {
    base_dir.join(id)
}

// ── .opcip 导出/导入 ─────────────────────────────────────────────
//
// .opcip = Industry Pack 的 zip 归档（manifest.yaml + workflows/*.yaml）。
// 导出：打包行业目录 → zip 文件；导入：解包 → 注册 → seed。

/// 导出行业包为 .opcip 归档。
/// 返回生成的文件路径。
pub async fn export_industry_pack(
    base_dir: &Path,
    id: &str,
    out_dir: &Path,
) -> Result<String, String> {
    let src = industry_pack_dir(base_dir, id);
    if !src.is_dir() {
        return Err(format!("行业包不存在: {}", src.display()));
    }

    let file_path = out_dir.join(format!("{id}.opcip"));
    let file = std::fs::File::create(&file_path).map_err(|e| format!("创建归档失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 递归打包目录（zip 内部用正斜杠相对路径）
    fn add_dir(
        zip: &mut zip::ZipWriter<std::fs::File>,
        opts: &zip::write::SimpleFileOptions,
        _base: &Path,
        dir: &Path,
        prefix: &str,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let zip_name = format!("{prefix}{name}");
            if path.is_dir() {
                add_dir(zip, opts, _base, &path, &format!("{zip_name}/"))?;
            } else {
                let content = std::fs::read(&path).map_err(|e| format!("读取文件失败: {e}"))?;
                zip.start_file(zip_name, *opts).map_err(|e| format!("写入归档失败: {e}"))?;
                zip.write_all(&content).map_err(|e| format!("写入归档失败: {e}"))?;
            }
        }
        Ok(())
    }

    // 打包：zip 内路径以 {id}/ 为前缀（如 "finance_invest/manifest.yaml"），
    // 保证导入时能识别单一顶层行业目录。
    add_dir(&mut zip, &opts, &src, &src, &format!("{id}/"))
        .map_err(|e| format!("打包失败: {e}"))?;
    zip.finish().map_err(|e| format!("归档完成失败: {e}"))?;
    tracing::info!("[industry-pack] 导出 {id} → {}", file_path.display());
    Ok(file_path.to_string_lossy().to_string())
}

/// 导入 .opcip 行业包：解包到 app_dir/config/opc/industries/{id}/ 并注册 seed。
/// 返回导入的行业 id。
pub async fn import_industry_pack(
    db: &DatabaseConnection,
    app_dir: &Path,
    archive_path: &Path,
) -> Result<String, String> {
    // P1-12：兼容目录导入（市场页把行业目录路径当归档传）。
    // 目录内应含 manifest.yaml（或其子目录含），直接拷贝到 industries/ 并 seed。
    if archive_path.is_dir() {
        let Some(id) = archive_path.file_name().map(|s| s.to_string_lossy().to_string()) else {
            return Err("无法从目录名确定行业 id".to_string());
        };
        let industries_root = app_dir.join(INDUSTRIES_DIR);
        let target = industries_root.join(&id);
        // 源目录可能是 {id}/（含 manifest）或 {id}/workflows 的父目录，先探测 manifest 位置
        let manifest_candidate = if archive_path.join("manifest.yaml").is_file() {
            archive_path.to_path_buf()
        } else if archive_path.parent().map(|p| p.join("manifest.yaml").is_file()).unwrap_or(false)
        {
            archive_path.parent().unwrap().to_path_buf()
        } else {
            return Err(format!(
                "{} 目录内未找到 manifest.yaml，不是有效的行业包目录",
                archive_path.display()
            ));
        };
        super::copy_dir_recursive(&manifest_candidate, &target)
            .map_err(|e| format!("拷贝行业包目录失败: {e}"))?;
        tracing::info!("[industry-pack] 目录导入 {id} → {}", target.display());
        let seeded = ensure_opc_industries_seeded(db, &industries_root).await?;
        if !seeded.contains(&id) {
            tracing::info!("[industry-pack] {id} 已存在（版本一致），视为导入成功");
        }
        return Ok(id);
    }

    let file = std::fs::File::open(archive_path).map_err(|e| format!("打开归档失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("解析归档失败: {e}"))?;

    // 目标目录：app_dir/config/opc/industries/{id}
    let target_root = app_dir.join(INDUSTRIES_DIR);
    std::fs::create_dir_all(&target_root).map_err(|e| format!("创建目录失败: {e}"))?;

    // 解包所有条目，记录顶层目录（行业 id，通常只有一个）
    let mut top_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut has_manifest = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("读取条目失败: {e}"))?;
        let entry_name = entry.name().to_string();
        // P2-3：zip-slip 防护——拒绝绝对路径与 `..` 穿越（恶意 .opcip 可写任意目录）
        let normalized = entry_name.replace('\\', "/");
        if std::path::Path::new(&normalized).is_absolute()
            || normalized.split('/').any(|c| c == "..")
        {
            return Err(format!("归档内存在非法路径，已拒绝解包: {entry_name}"));
        }
        if entry.is_dir() {
            continue;
        }
        // 顶层目录 = 行业 id（zip_name 形如 "finance_invest/manifest.yaml"）
        let top = entry_name.split('/').next().unwrap_or("").to_string();
        if top.is_empty() {
            continue;
        }
        top_dirs.insert(top.clone());
        if entry_name.ends_with("manifest.yaml") {
            has_manifest = true;
        }
        let out_path = target_root.join(&entry_name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(|e| format!("创建文件失败: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("解包失败: {e}"))?;
    }

    if !has_manifest {
        return Err("归档内未找到 manifest.yaml，不是有效的 .opcip 行业包".to_string());
    }
    if top_dirs.len() != 1 {
        return Err(format!("归档应只含一个行业包目录，实际 {} 个: {top_dirs:?}", top_dirs.len()));
    }
    let id = top_dirs.into_iter().next().unwrap();
    tracing::info!("[industry-pack] 导入 {id} → {}", target_root.display());

    // 注册 + seed（行业工作流现已由 Rust 代码生成，仅注册 manifest）
    let seeded = ensure_opc_industries_seeded(db, &target_root).await?;
    if !seeded.contains(&id) {
        tracing::info!("[industry-pack] {id} 已存在，跳过 seed");
    }
    Ok(id)
}

// ── 股票工具白名单（P4-2：金融行业吃 astock-data 工具链）────────

/// 从 astock-data stock_mcp_tools 匹配工具名 → ToolDef 列表。
/// 工具已由 init/services.rs ToolResolver 接通执行路径（execute_mcp_tool），
/// 工作流 AgentNode 只要 exposed_tools 含工具名即可调用。
pub fn stock_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    let mut out = Vec::new();
    for tool in axagent_astock_data::mcp_tools::stock_mcp_tools() {
        let Some(name) = tool.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if !names.iter().any(|n| n == name) {
            continue;
        }
        let description = tool.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
        // parameters：把 inputSchema json 转 ToolDef.parameters（JsonSchema）
        let parameters =
            tool.get("inputSchema").and_then(|v| serde_json::from_value(v.clone()).ok());
        out.push(axagent_harness::workflow_types::ToolDef {
            name: name.to_string(),
            description,
            parameters,
        });
    }
    out
}

// ── OPC 工具白名单（一人公司业务：内容营销/电商等行业吃 Opc 工具链）────/// 从 tools crate 内置 OPC 工具匹配工具名 → ToolDef 列表。
/// 工具已注册进本地工具注册表（UnifiedToolRegistry），
/// init/services.rs ToolResolver 的 `known` 分支即可接通执行路径，
/// 工作流 AgentNode 只要 exposed_tools 含工具名即可调用。
pub fn opc_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    use axagent_tools::Tool;
    let candidates: Vec<Arc<dyn Tool>> = vec![
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListInvoicesTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcCreateInvoiceTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcTransitionInvoiceTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListCustomersTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcCreateCustomerTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListProjectsTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcCreateProjectTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcAddMilestoneTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcGetDashboardTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListLandingPagesTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListBlogPostsTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcCreateLandingPageTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcCreateBlogPostTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListContactsTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcSendNotificationTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcRecordKpiTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcListKpisTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcSearchWikiTool),
        std::sync::Arc::new(axagent_tools::tools::opc::OpcGetFinancialReportTool),
    ];
    let mut out = Vec::new();
    for tool in candidates {
        if !names.iter().any(|n| n == tool.name()) {
            continue;
        }
        // parameters：把 input_schema()（serde_json::Value）转 ToolDef.parameters（JsonSchema）
        let parameters = serde_json::from_value(tool.input_schema()).ok();
        out.push(axagent_harness::workflow_types::ToolDef {
            name: tool.name().to_string(),
            description: Some(tool.description().to_string()),
            parameters,
        });
    }
    out
}

// ── 通用本机工具白名单（P1-1：software_dev 等行业声明 FileRead/Bash/Grep 等）──

/// 从 tools crate 内置通用工具匹配工具名 → ToolDef 列表。
/// 与 stock_tool_defs / opc_tool_defs 并列，构成完整工具注入白名单。
pub fn local_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    use axagent_tools::Tool;
    let candidates: Vec<Arc<dyn Tool>> = vec![
        std::sync::Arc::new(axagent_tools::tools::file_read::FileReadTool),
        std::sync::Arc::new(axagent_tools::tools::file_write::FileWriteTool),
        std::sync::Arc::new(axagent_tools::tools::file_edit::FileEditTool),
        std::sync::Arc::new(axagent_tools::tools::bash::BashTool),
        std::sync::Arc::new(axagent_tools::tools::grep::GrepTool),
        std::sync::Arc::new(axagent_tools::tools::glob::GlobTool),
        std::sync::Arc::new(axagent_tools::tools::file_system::ListDirectoryTool),
        std::sync::Arc::new(axagent_tools::tools::web_search::WebSearchTool),
    ];
    let mut out = Vec::new();
    for tool in candidates {
        if !names.iter().any(|n| n == tool.name()) {
            continue;
        }
        let parameters = serde_json::from_value(tool.input_schema()).ok();
        out.push(axagent_harness::workflow_types::ToolDef {
            name: tool.name().to_string(),
            description: Some(tool.description().to_string()),
            parameters,
        });
    }
    out
}
