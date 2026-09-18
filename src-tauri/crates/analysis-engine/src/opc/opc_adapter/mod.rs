// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包动态编排适配器
//!
//! 14 个域包各一个手写 `CapabilityPackAdapter`，接入 harness 的 `DynamicSubGraph`
//! 动态编排。步骤/工具**硬编码进 Rust**（不读 runtime.yaml 的 workflow_steps 段），
//! `decompose_mission` 按任务意图动态选策略生成 DAG，`preset_steps` 作无 mission 兜底。
//!
//! - `skeleton`：统一编排样板（`build_plan` / `to_workflow_template_data` / 关键词路由）
//! - `software_dev_adapter` 等：每域专属 adapter + `create_xxx_adapter()` 工厂
//! - `register_opc_capability_pack_adapters`：启动时把全部域包 adapter 注册进注册表

pub mod skeleton;

pub mod software_dev_adapter;

pub use skeleton::to_workflow_template_data;
pub use software_dev_adapter::create_software_dev_adapter;

/// 注册全部 OPC 域包 adapter 到注册表（幂等、静态全量注册）。
///
/// 各域的 `create_xxx_adapter()` 由对应 adapter 模块提供。
pub fn register_opc_capability_pack_adapters(
    registry: &mut axagent_harness::CapabilityPackAdapterRegistry,
) {
    registry.register(create_software_dev_adapter());
    // 其余域包 adapter 依次在此注册（见各域模块）
}
