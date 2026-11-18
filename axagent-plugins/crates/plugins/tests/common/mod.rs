// SPDX-License-Identifier: AGPL-3.0-only

//! 测试工具和 fixture 生成器。
//!
//! 多个 test binary 共享同一份工具模块,但每个 test 只用其中一部分,
//! 因此允许 `common` 模块内出现未使用的辅助函数。

#![allow(dead_code)]

use axagent_harness::{
    AdjType, KLine, MarketDataProvider, Result as HarnessResult, StockQuote, StockSearchResult,
};
use axagent_plugins::PluginManifest;
use serde_json::json;
use std::path::{Path, PathBuf};

/// 固定行情替身：只验证「宿主 provider 的数据能经插件回到宿主」，不验证真实取数
/// （真实 provider 要联网，不能进测试）。
pub struct FixedQuoteProvider;

#[async_trait::async_trait]
impl MarketDataProvider for FixedQuoteProvider {
    async fn get_quote(&self, stock_code: &str) -> HarnessResult<StockQuote> {
        Ok(StockQuote {
            code: stock_code.to_string(),
            name: "测试股".to_string(),
            price: 1500.25,
            pre_close: 1490.0,
            open: 1495.0,
            high: 1510.0,
            low: 1488.0,
            volume: 1_000_000.0,
            amount: 1_500_000_000.0,
            change_pct: 0.68,
            turnover_rate: 0.5,
            pe: Some(12.3),
            pb: Some(1.8),
            total_mv: Some(1.0e11),
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: "2026-11-04T15:00:00+08:00".to_string(),
        })
    }

    async fn get_klines(
        &self,
        _stock_code: &str,
        _period: &str,
        _limit: u32,
        _adj_type: Option<AdjType>,
    ) -> HarnessResult<Vec<KLine>> {
        Ok(Vec::new())
    }

    async fn search_stock(&self, _keyword: &str) -> HarnessResult<Vec<StockSearchResult>> {
        Ok(Vec::new())
    }
}

/// 固定工作流执行替身：实现 `list_active` 与 `list_recent`（任务源取数走后者）。
/// 其余方法**沿用 trait 默认实现**（回 `Provider("… 未实现")`），
/// 测试若误用会立刻拿到错误，而不是拿到一份假回执。
///
/// 返回值固定，让端到端断言钉得住「宿主引擎视图 → 插件行记录 → 回传宿主」这一整条链。
/// `list_recent` 比 `list_active` 多给一行**已结束**的执行（`exec-0` /
/// `partially_completed` + `finished_at`）：任务中心要的就是这一行，插件侧的耗时列
/// 只由它验证得到；选 `partially_completed` 而不是 `completed`，是因为它同时钉住
/// 「有产出的终态判成 succeeded」这条最容易判错的翻译。
pub struct FixedWorkflowRunProvider;

#[async_trait::async_trait]
impl axagent_harness::WorkflowRunProvider for FixedWorkflowRunProvider {
    async fn list_active(
        &self,
        owner: &str,
        _workflow_id: Option<&str>,
        _limit: usize,
    ) -> axagent_harness::core_error::Result<axagent_harness::WorkflowRunActivePage> {
        Ok(axagent_harness::WorkflowRunActivePage {
            executions: vec![axagent_harness::WorkflowRunActiveView {
                execution_id: "exec-1".to_string(),
                workflow_id: "wf-1".to_string(),
                status: axagent_harness::workflow_types::ExecutionStatus::Running,
                current_node_id: Some("node-2".to_string()),
                owner_plugin_id: owner.to_string(),
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_001_000,
                finished_at: None,
            }],
            total: 1,
        })
    }

    async fn list_recent(
        &self,
        owner: &str,
        _workflow_id: Option<&str>,
        _limit: usize,
    ) -> axagent_harness::core_error::Result<axagent_harness::WorkflowRunActivePage> {
        let running = axagent_harness::WorkflowRunActiveView {
            execution_id: "exec-1".to_string(),
            workflow_id: "wf-1".to_string(),
            status: axagent_harness::workflow_types::ExecutionStatus::Running,
            current_node_id: Some("node-2".to_string()),
            owner_plugin_id: owner.to_string(),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_001_000,
            finished_at: None,
        };
        let done = axagent_harness::WorkflowRunActiveView {
            execution_id: "exec-0".to_string(),
            workflow_id: "wf-1".to_string(),
            status: axagent_harness::workflow_types::ExecutionStatus::PartiallyCompleted,
            current_node_id: None,
            owner_plugin_id: owner.to_string(),
            created_at: 1_699_999_990_000,
            updated_at: 1_699_999_999_000,
            finished_at: Some(1_699_999_999_000),
        };
        Ok(axagent_harness::WorkflowRunActivePage { executions: vec![running, done], total: 2 })
    }
}

/// 固定单股分析记录替身：只实现 `analysis.record::list`。
/// 两条固定行一条在跑、一条已完成且带完成时刻；一条子行用于锁插件侧根记录收窄。
pub struct FixedAnalysisRecordProvider;

#[async_trait::async_trait]
impl axagent_harness::AnalysisRecordProvider for FixedAnalysisRecordProvider {
    async fn list(
        &self,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> axagent_harness::core_error::Result<axagent_harness::AnalysisRecordListOutcome> {
        use axagent_harness::{AnalysisRecordListOutcome, AnalysisRecordView};
        let record = |id: &str, status: &str, completed_at: Option<i64>| AnalysisRecordView {
            id: id.to_string(),
            stock_code: "300308".to_string(),
            stock_name: "中际旭创".to_string(),
            analysis_date: "2026-11-05".to_string(),
            status: status.to_string(),
            decision_action: None,
            decision_position_state: None,
            decision_position_pct: None,
            analysis_kind: "live".to_string(),
            as_of_date: None,
            parent_analysis_id: None,
            template_id: None,
            template_version: None,
            decision_time_horizon: None,
            decision_horizon_source: None,
            created_at: 1_699_999_000_000,
            updated_at: 1_699_999_999_000,
            completed_at,
            execution_quality: None,
        };
        Ok(AnalysisRecordListOutcome {
            records: vec![
                record("analysis-1", "running", None),
                record("analysis-0", "completed", Some(1_699_999_999_000)),
                AnalysisRecordView { parent_analysis_id: Some("analysis-0".to_string()), ..record("analysis-child", "completed", Some(1_699_999_999_000)) },
            ],
            total: 3,
        })
    }
}

/// 创建测试用临时目录并在 test 完成后自动清理。
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("测试：系统时间应晚于 UNIX EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("plugin-test-{label}-{nanos}"));
        std::fs::create_dir_all(&path).expect("failed to create temp dir");
        TempDir { path }
    }

    /// 在临时目录中写入 plugin.json 并返回完整路径。
    pub fn write_plugin_json(&self, manifest_json: &str) -> PathBuf {
        let plugin_dir = self.path.join("plugin");
        std::fs::create_dir_all(&plugin_dir).expect("failed to create plugin dir");
        let manifest_path = plugin_dir.join("plugin.json");
        std::fs::write(&manifest_path, manifest_json).expect("failed to write plugin.json");
        plugin_dir
    }

    /// 写入一个恶意 plugin.json（语法错误或 JSON 不合法）。
    pub fn write_malformed_plugin_json(&self) -> PathBuf {
        let plugin_dir = self.path.join("malformed-plugin");
        std::fs::create_dir_all(&plugin_dir).expect("failed to create malformed dir");
        let manifest_path = plugin_dir.join("plugin.json");
        std::fs::write(&manifest_path, r#"{"name": "bad, "version": "0.1"}"#)
            .expect("failed to write malformed json");
        plugin_dir
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 生成一个最小有效的 plugin.json content
pub fn minimal_plugin_json(name: &str, version: &str) -> String {
    let json = json!({
        "name": name,
        "version": version,
        "description": format!("Test plugin: {}", name),
        "mcp_servers": [],
        "skills": [],
        "agents": [],
        "permissions": [],
        "defaultEnabled": false
    });
    json.to_string()
}

/// 尝试从 fixture 目录加载 manifest 并返回 result。
pub fn load_manifest(dir: &Path) -> Result<PluginManifest, axagent_plugins::PluginError> {
    axagent_plugins::load_plugin_from_directory(dir)
}
