// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 行业数据接入层命令
//!
//! 与股票业务同架构：从 analysis-engine 引擎的 opc::vendors 模块
//! 获取 OpIndustryVendor / OpIndustryClient 实现，命令层只负责调用。
//!
//! 注：vendor 装配与 OpIndustryClient 构造的权威实现在
//! `opc_industry_runtime::build_data_client`（内联 db/cache/web/file 装配），
//! 本文件只保留 analysis.yaml 读取入口。

use crate::commands::opc_workflows::IndustryAnalysisConfig;

// ── 行业包加载 ─────────────────────────────────────────────────

/// 读取行业包 analysis.yaml（命令直读入口）
pub fn load_analysis_config(dir: &std::path::Path) -> Result<IndustryAnalysisConfig, String> {
    let path = dir.join("analysis.yaml");
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    serde_yaml::from_str(&raw).map_err(|e| format!("解析 {} 失败: {e}", path.display()))
}
