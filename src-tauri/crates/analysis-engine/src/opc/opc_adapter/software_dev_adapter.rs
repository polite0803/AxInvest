// SPDX-License-Identifier: AGPL-3.0-only

//! 软件开发域包适配器
//!
//! `software_dev` 域包的手写 `CapabilityPackAdapter`。步骤**硬编码进 Rust**：
//! - `decompose_mission`：按任务关键词路由 SDLC 流程（18 步）或大型项目重构流程（批次）
//! - `preset_steps`：返回 SDLC 骨架步骤作无 mission 兜底
//!
//! 步骤原料来源：`config/opc/domain_packs/software_dev/runtime.yaml` 的 `workflow_steps` 段。

use async_trait::async_trait;
use std::sync::Arc;

use axagent_harness::{
    capability_pack_orchestration::types::{
        AcceptanceCriterion, CapabilityPackLearningConfig, EvolutionConstraints, ReflectionTemplate,
    },
    CapabilityPackAdapter, CapabilityPackContext, GeneratedSubGraph, MissionType,
    OrchestrationError, OrchestrationStrategy, PresetWorkflowStep, SubTask,
};

use super::skeleton::{build_plan, select_strategy_keywords, DEFAULT_DEBATE_KEYWORDS};

/// 软件开发域包适配器
pub struct SoftwareDevCapabilityPackAdapter {
    domain_pack_id: String,
    capability_pack_name: String,
    reflection_template: ReflectionTemplate,
    evolution_constraints: EvolutionConstraints,
    acceptance_criteria: Vec<AcceptanceCriterion>,
    learning_config: CapabilityPackLearningConfig,
}

impl SoftwareDevCapabilityPackAdapter {
    pub fn new() -> Self {
        Self {
            domain_pack_id: "software_dev".to_string(),
            capability_pack_name: "软件开发".to_string(),
            reflection_template: Self::dev_reflection_template(),
            evolution_constraints: EvolutionConstraints::default(),
            acceptance_criteria: vec![
                AcceptanceCriterion {
                    id: "sd-code-quality".to_string(),
                    name: "代码质量".to_string(),
                    description: "代码质量达标，无阻塞缺陷".to_string(),
                    dimension: "quality".to_string(),
                    threshold: 0.8,
                    is_critical: true,
                    weight: 0.4,
                },
                AcceptanceCriterion {
                    id: "sd-test-coverage".to_string(),
                    name: "测试覆盖".to_string(),
                    description: "关键路径测试覆盖充分".to_string(),
                    dimension: "quality".to_string(),
                    threshold: 0.7,
                    is_critical: false,
                    weight: 0.3,
                },
                AcceptanceCriterion {
                    id: "sd-delivery".to_string(),
                    name: "交付完整".to_string(),
                    description: "功能交付完整、文档齐备".to_string(),
                    dimension: "completion".to_string(),
                    threshold: 0.8,
                    is_critical: true,
                    weight: 0.3,
                },
            ],
            learning_config: CapabilityPackLearningConfig::default(),
        }
    }

    fn dev_reflection_template() -> ReflectionTemplate {
        ReflectionTemplate {
            id: "software-dev-default".to_string(),
            name: "软件开发反思模板".to_string(),
            prompts: vec![
                "代码是否满足功能需求且无阻塞缺陷？".to_string(),
                "测试覆盖是否充分，关键路径是否有验证？".to_string(),
                "交付物（代码/文档/部署）是否完整可交接？".to_string(),
            ],
            ..Default::default()
        }
    }

    /// SDLC 全流程步骤（18 步，对齐 runtime.yaml workflow_steps）
    fn sdlc_sub_tasks() -> Vec<SubTask> {
        vec![
            SubTask::new(
                "req".into(),
                "需求分析".into(),
                "分析并澄清用户需求".into(),
                "analyst".into(),
            ),
            SubTask::new(
                "feasibility".into(),
                "可行性评审".into(),
                "评估技术可行性、资源和风险".into(),
                "analyst".into(),
            ),
            SubTask::new(
                "arch".into(),
                "架构设计".into(),
                "设计系统架构和技术栈".into(),
                "architect".into(),
            ),
            SubTask::new(
                "data".into(),
                "数据模型设计".into(),
                "设计数据库实体、关系和索引".into(),
                "data_agent".into(),
            ),
            SubTask::new(
                "api".into(),
                "API 设计".into(),
                "设计 RESTful API 接口".into(),
                "developer".into(),
            ),
            SubTask::new(
                "setup".into(),
                "项目环境搭建".into(),
                "初始化开发环境".into(),
                "developer".into(),
            ),
            SubTask::new(
                "code".into(),
                "编码实现".into(),
                "按设计实现代码".into(),
                "developer".into(),
            ),
            SubTask::new("cr".into(), "代码审查".into(), "审查代码质量".into(), "reviewer".into()),
            SubTask::new(
                "fix".into(),
                "缺陷修复".into(),
                "根据审查意见修复缺陷".into(),
                "developer".into(),
            )
            .with_dependencies(vec!["cr".into()]),
            SubTask::new(
                "doc".into(),
                "文档编写".into(),
                "生成设计与开发文档".into(),
                "developer".into(),
            ),
            SubTask::new(
                "unit".into(),
                "单元测试".into(),
                "为核心模块编写单元测试".into(),
                "developer".into(),
            ),
            SubTask::new(
                "itest".into(),
                "集成测试".into(),
                "执行集成与端到端测试".into(),
                "developer".into(),
            ),
            SubTask::new("sec".into(), "安全审查".into(), "执行安全审计".into(), "security".into()),
            SubTask::new(
                "deploy".into(),
                "部署上线".into(),
                "执行部署、构建、迁移".into(),
                "executor".into(),
            ),
            SubTask::new(
                "handoff".into(),
                "运维交接".into(),
                "生成运维文档与交接包".into(),
                "executor".into(),
            )
            .with_dependencies(vec!["deploy".into()]),
        ]
    }

    /// 大型项目重构流程
    fn refactor_sub_tasks() -> Vec<SubTask> {
        vec![
            SubTask::new(
                "scan".into(),
                "代码资产盘点".into(),
                "统计文件数与模块结构".into(),
                "analyst".into(),
            ),
            SubTask::new(
                "dep".into(),
                "依赖关系分析".into(),
                "构建依赖图、识别循环依赖".into(),
                "analyst".into(),
            ),
            SubTask::new(
                "cx".into(),
                "复杂度扫描".into(),
                "圈复杂度热点识别".into(),
                "analyst".into(),
            ),
            SubTask::new(
                "smell".into(),
                "坏味道检测".into(),
                "长方法/上帝类/重复代码检测".into(),
                "analyst".into(),
            ),
            SubTask::new(
                "strategy".into(),
                "重构策略制定".into(),
                "选择渐进/绞杀式重构模式".into(),
                "planner".into(),
            ),
            SubTask::new(
                "baseline".into(),
                "质量基线建立".into(),
                "测试覆盖/性能基准/规范".into(),
                "developer".into(),
            ),
            SubTask::new(
                "execute".into(),
                "分批执行".into(),
                "逐模块重构并持续集成".into(),
                "developer".into(),
            )
            .with_dependencies(vec!["baseline".into()]),
            SubTask::new(
                "regress".into(),
                "回归验证".into(),
                "全量回归测试与性能对比".into(),
                "reviewer".into(),
            )
            .with_dependencies(vec!["execute".into()]),
            SubTask::new(
                "gate".into(),
                "质量门禁".into(),
                "覆盖/复杂度/性能达标".into(),
                "reviewer".into(),
            ),
            SubTask::new(
                "handoff".into(),
                "运维交接".into(),
                "运行手册/监控告警/回滚预案".into(),
                "executor".into(),
            )
            .with_dependencies(vec!["gate".into()]),
        ]
    }

    fn select_strategy(&self, mission: &str) -> OrchestrationStrategy {
        select_strategy_keywords(mission, DEFAULT_DEBATE_KEYWORDS)
    }
}

impl Default for SoftwareDevCapabilityPackAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityPackAdapter for SoftwareDevCapabilityPackAdapter {
    fn domain_pack_id(&self) -> &str {
        &self.domain_pack_id
    }

    fn capability_pack_name(&self) -> &str {
        &self.capability_pack_name
    }

    async fn decompose_mission(
        &self,
        mission: &str,
        _context: &CapabilityPackContext,
    ) -> Result<GeneratedSubGraph, OrchestrationError> {
        let lower = mission.to_lowercase();
        if lower.contains("重构") || lower.contains("refactor") {
            build_plan(mission, OrchestrationStrategy::Pipeline, Self::refactor_sub_tasks(), 2, 3)
        } else if matches!(self.select_strategy(mission), OrchestrationStrategy::Debate) {
            build_plan(
                mission,
                OrchestrationStrategy::Debate,
                vec![
                    SubTask::new(
                        "pro".into(),
                        "正方架构评审".into(),
                        "从采用角度论证方案".into(),
                        "reviewer".into(),
                    ),
                    SubTask::new(
                        "con".into(),
                        "反方架构评审".into(),
                        "从风险角度提出异议".into(),
                        "reviewer".into(),
                    ),
                    SubTask::new(
                        "arb".into(),
                        "仲裁裁决".into(),
                        "综合双方给出最终决策".into(),
                        "planner".into(),
                    ),
                ],
                2,
                2,
            )
        } else {
            build_plan(mission, OrchestrationStrategy::Pipeline, Self::sdlc_sub_tasks(), 2, 3)
        }
    }

    fn detect_mission_type(&self, mission: &str) -> MissionType {
        let lower = mission.to_lowercase();
        if lower.contains("开发")
            || lower.contains("实现")
            || lower.contains("编码")
            || lower.contains("build")
        {
            MissionType::Generation
        } else if lower.contains("审查") || lower.contains("review") {
            MissionType::Review
        } else if lower.contains("修复")
            || lower.contains("bug")
            || lower.contains("重构")
            || lower.contains("refactor")
        {
            MissionType::Fix
        } else {
            MissionType::Planning
        }
    }

    fn reflection_template(&self) -> &ReflectionTemplate {
        &self.reflection_template
    }

    fn evolution_constraints(&self) -> &EvolutionConstraints {
        &self.evolution_constraints
    }

    fn acceptance_criteria(&self) -> &[AcceptanceCriterion] {
        &self.acceptance_criteria
    }

    fn learning_config(&self) -> &CapabilityPackLearningConfig {
        &self.learning_config
    }

    fn preset_steps(&self) -> Vec<PresetWorkflowStep> {
        vec![
            PresetWorkflowStep::skeleton("req", "需求分析", "分析用户需求", "analyst", 1),
            PresetWorkflowStep::skeleton("arch", "架构设计", "设计系统架构", "architect", 2),
            PresetWorkflowStep::skeleton("code", "编码实现", "实现业务代码", "developer", 3),
            PresetWorkflowStep::skeleton("review", "代码审查", "审查代码质量", "reviewer", 4),
            PresetWorkflowStep::skeleton("test", "测试", "编写并运行测试", "developer", 5),
            PresetWorkflowStep::skeleton("deploy", "部署", "部署到目标环境", "executor", 6),
        ]
    }
}

/// 创建软件开发域包适配器
pub fn create_software_dev_adapter() -> Arc<dyn CapabilityPackAdapter> {
    Arc::new(SoftwareDevCapabilityPackAdapter::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_has_correct_id() {
        let a = SoftwareDevCapabilityPackAdapter::new();
        assert_eq!(a.domain_pack_id(), "software_dev");
        assert_eq!(a.capability_pack_name(), "软件开发");
    }

    #[test]
    fn detects_generation_mission() {
        let a = SoftwareDevCapabilityPackAdapter::new();
        assert_eq!(a.detect_mission_type("帮我开发一个接口"), MissionType::Generation);
    }

    #[test]
    fn detects_refactor_as_fix() {
        let a = SoftwareDevCapabilityPackAdapter::new();
        assert_eq!(a.detect_mission_type("对支付模块重构"), MissionType::Fix);
    }

    #[tokio::test]
    async fn decomposes_refactor_pipeline() {
        let a = SoftwareDevCapabilityPackAdapter::new();
        let r =
            a.decompose_mission("对一个大型项目进行重构", &CapabilityPackContext::default()).await;
        assert!(r.is_ok());
        assert!(r.unwrap().nodes.len() >= 5);
    }

    #[test]
    fn has_acceptance_criteria() {
        let a = SoftwareDevCapabilityPackAdapter::new();
        assert!(a.acceptance_criteria().iter().any(|c| c.is_critical));
    }
}
