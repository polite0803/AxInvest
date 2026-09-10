// SPDX-License-Identifier: AGPL-3.0-only

//! 需求精评 LLM 桥的 lib 侧装配
//!
//! `axagent_tools` 定义了窄接口 [`axagent_tools::tools::demand_llm::DemandLlmBridge`]，
//! 本模块用已配置的 `ProviderLlmBridge`（provider 选择 / fallback 均已内置）实现它，
//! 并在启动时注册进 tools 的 global_state —— 此后所有扫描入口（手动扫描、订阅定时、
//! 主动扫描、cron、工作流 `OpcDiscoverLeads` 工具节点）共享同一个精评能力。
//!
//! 未配置任何 LLM provider 时不注册，精评静默跳过（规则评分兜底）。

use axagent_agent::ProviderLlmBridge;
use axagent_tools::tools::demand_llm::DemandLlmBridge;

/// `ProviderLlmBridge` → tools 窄接口的适配器（低温结构化调用）
struct DemandLlmBridgeAdapter(ProviderLlmBridge);

#[async_trait::async_trait]
impl DemandLlmBridge for DemandLlmBridgeAdapter {
    async fn call(&self, system: &str, user: &str) -> Result<String, String> {
        self.0.call_llm_structured(system, user).await
    }
}

/// 从 DB 构建精评桥并注册进 tools 全局态（启动时调用一次）
///
/// `master_key` 用于解密 provider API key；未配置可用 provider 时不注册，
/// 需求精评退化为纯规则评分。
pub(crate) async fn register_demand_llm_bridge(master_key: &[u8; 32]) {
    match axagent_runtime::llm_bridge::build_llm_bridge_from_db(master_key).await {
        Some(bridge) => {
            axagent_tools::global_state::set_demand_llm(std::sync::Arc::new(
                DemandLlmBridgeAdapter(bridge),
            ));
            tracing::info!("[demand_llm] LLM 精评桥已注册（需求线索扫描将叠加 LLM 精评）");
        },
        None => {
            tracing::info!("[demand_llm] 未配置可用 LLM provider，需求精评使用规则评分");
        },
    }
}
