//! Learning & trajectory optimization domain state.
//!
//! Owns the learning engines that operate on trajectory data:
//! text-grad optimization, intrinsic motivation, co-evolution, and
//! process reward modeling.
//!
//! Also owns the domain_pack-specific learning engine and adapter registry
//! for OPC (One-Person Company) vertical domain_pack scenarios.

use std::sync::Arc;
use tokio::sync::Mutex;

use axagent_orchestrator::{DomainPackAdapterRegistry, DomainPackLearningEngine};

#[derive(Clone)]
pub struct LearningEngineState {
    pub text_grad_engine: Arc<Mutex<axagent_trajectory::TextGradEngine>>,
    pub intrinsic_motivation: Arc<Mutex<axagent_trajectory::IntrinsicMotivationEngine>>,
    pub coevolution_env: Arc<Mutex<axagent_trajectory::CoevolutionEnvironment>>,
    pub process_reward_model: Arc<Mutex<axagent_trajectory::ProcessRewardModel>>,
    /// OPC 域包学习引擎 — 实现反思、进化、自我改进
    pub domain_pack_learning_engine: Arc<DomainPackLearningEngine>,
    /// OPC 域包适配器注册表 — 管理 9 个垂直域包的适配器
    pub domain_pack_adapter_registry: Arc<Mutex<DomainPackAdapterRegistry>>,
}

impl LearningEngineState {
    pub fn new(
        text_grad_engine: Arc<Mutex<axagent_trajectory::TextGradEngine>>,
        intrinsic_motivation: Arc<Mutex<axagent_trajectory::IntrinsicMotivationEngine>>,
        coevolution_env: Arc<Mutex<axagent_trajectory::CoevolutionEnvironment>>,
        process_reward_model: Arc<Mutex<axagent_trajectory::ProcessRewardModel>>,
        domain_pack_learning_engine: Arc<DomainPackLearningEngine>,
        domain_pack_adapter_registry: Arc<Mutex<DomainPackAdapterRegistry>>,
    ) -> Self {
        Self {
            text_grad_engine,
            intrinsic_motivation,
            coevolution_env,
            process_reward_model,
            domain_pack_learning_engine,
            domain_pack_adapter_registry,
        }
    }
}
