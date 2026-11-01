// SPDX-License-Identifier: AGPL-3.0-only

//! 域包适配器模块
//!
//! 提供域包适配器核心 trait 和注册表，定义域包动态编排、反思、进化接口。

pub mod plan;
pub mod subgraph;
pub mod types;

use async_trait::async_trait;
use std::sync::Arc;

// ── 重导出所有类型以便外部访问 ──
pub use plan::{
    DecompositionPlan, OrchestrationError, OrchestrationStrategy, SubTask, SubTaskStatus,
};
pub use subgraph::{DynamicSubGraph, GeneratedSubGraph};
pub use types::{
    AcceptanceCriterion, AcceptanceResult, AutoReflectTrigger, AutoTriggerConfig,
    CapabilityPackContext, CapabilityPackLearningConfig, CriterionResult, DependencyType,
    EvolutionConfig, EvolutionConstraints, ForbiddenOptimization, MissionType, PresetWorkflowStep,
    ProtectedStep, QualityThresholds, QualityWeights, ReflectionCheckpoint, ReflectionConfig,
    ReflectionTemplate, ReinforcementLearningConfig, RewardWeightConfig, SelfImprovementConfig,
    SkillEvolverConfig, StepDependency, WorkflowEvolverConfig,
};

// ── CapabilityPackAdapter trait ──────────────────────────────────────────

/// 域包适配器核心 trait
///
/// 每个域包实现此 trait，提供域包特定的：
/// - 动态任务分解策略
/// - 反思模板
/// - 进化约束
/// - 验收标准定义
#[async_trait]
pub trait CapabilityPackAdapter: Send + Sync {
    /// 域包唯一标识
    fn domain_pack_id(&self) -> &str;

    /// 域包显示名称
    fn capability_pack_name(&self) -> &str;

    /// 将用户意图分解为动态任务 DAG
    async fn decompose_mission(
        &self,
        mission: &str,
        context: &CapabilityPackContext,
    ) -> Result<GeneratedSubGraph, OrchestrationError>;

    /// 检测任务类型
    fn detect_mission_type(&self, mission: &str) -> MissionType;

    /// 获取域包特定反思模板
    fn reflection_template(&self) -> &ReflectionTemplate;

    /// 获取域包特定进化约束
    fn evolution_constraints(&self) -> &EvolutionConstraints;

    /// 获取域包特定验收标准定义
    fn acceptance_criteria(&self) -> &[AcceptanceCriterion];

    /// 获取域包学习配置
    fn learning_config(&self) -> &CapabilityPackLearningConfig;

    /// 获取域包预设工作流步骤
    ///
    /// 返回域包的标准工作流步骤模板，用于初始化工作流编排。
    /// 默认实现返回空列表，域包适配器可覆盖此方法。
    fn preset_steps(&self) -> Vec<PresetWorkflowStep> {
        Vec::new()
    }
}

// ── CapabilityPackAdapterRegistry ────────────────────────────────────────

/// 域包适配器注册表
///
/// 管理所有域包适配器的实例，提供按 ID 查找功能。
pub struct CapabilityPackAdapterRegistry {
    adapters: Vec<Arc<dyn CapabilityPackAdapter>>,
}

impl CapabilityPackAdapterRegistry {
    /// 创建空注册表
    pub fn new() -> Self {
        Self { adapters: Vec::new() }
    }

    /// 注册域包适配器
    pub fn register(&mut self, adapter: Arc<dyn CapabilityPackAdapter>) {
        self.adapters.push(adapter);
    }

    /// 按域包 ID 查找适配器
    pub fn get(&self, domain_pack_id: &str) -> Option<&Arc<dyn CapabilityPackAdapter>> {
        self.adapters.iter().find(|a| a.domain_pack_id() == domain_pack_id)
    }

    /// 获取所有已注册域包 ID 列表
    pub fn list_industries(&self) -> Vec<&str> {
        self.adapters.iter().map(|a| a.domain_pack_id()).collect()
    }

    /// 获取所有已注册域包适配器引用
    pub fn all(&self) -> &[Arc<dyn CapabilityPackAdapter>] {
        &self.adapters
    }

    /// 获取已注册域包数量
    pub fn count(&self) -> usize {
        self.adapters.len()
    }
}

impl Default for CapabilityPackAdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
