// SPDX-License-Identifier: AGPL-3.0-only

//! Orchestrator — high-level task decomposition, subgraph generation,
//! execution monitoring, and replanning for multi-agent workflows.
//!
//! The OrchestratorExecutor receives a high-level mission description,
//! decomposes it into subtasks using LLM reasoning, generates a DAG
//! subgraph of Worker nodes, submits the subgraph to the work engine,
//! monitors execution progress, and replans on failures.
//!
//! # Architecture
//!
//! ```text
//! Mission → decompose() → SubTask[] → build_subgraph() → WorkflowGraph
//!                                                              ↓
//!                                engine.execute(subgraph) → monitor() → replan() ↻
//! ```

pub mod decomposer;
pub mod domain_pack_adapters;
pub mod domain_pack_learning;
pub mod dynamic_subgraph;
pub mod executor;
pub mod task_context;
pub mod task_shape_classifier;
pub mod token_budget;
pub mod types;

pub use domain_pack_adapters::types::ReinforcementLearningConfig;
pub use domain_pack_adapters::types::{
    AcceptanceCriterion, DomainPackContext, DomainPackLearningConfig, EvolutionConstraints,
    MissionType, ReflectionTemplate, RewardWeightConfig,
};
pub use domain_pack_adapters::{DomainPackAdapter, DomainPackAdapterRegistry};
pub use domain_pack_learning::{
    DimensionScore, DomainPackLearningEngine, EvolutionRequest, EvolutionResult,
    ExperiencePoolStats, LlmInferencePort, RLExperience, RLPolicyUpdate, ReflectionRequest,
    ReflectionResult, SelfImprovementRequest, SelfImprovementResult,
};
pub use dynamic_subgraph::{DynamicSubGraph, GeneratedSubGraph};
pub use executor::{OrchestratorExecutor, OrchestratorState};
pub use task_context::{
    DomainPackContextManager, DomainPackTaskContext, TaskContextState, TaskContextSummary,
};
pub use task_shape_classifier::{DefaultTaskShapeClassifier, classify_hybrid, classify_input};
pub use token_budget::{
    BudgetDecision, CompactionResult, DomainPackTokenBudgetManager, DomainPackTokenConfig,
    DomainPackTokenStats, DryLeafEntry, TokenUsageSnapshot,
};
pub use types::{
    DecompositionPlan, OrchestrationError, OrchestrationEvent, OrchestrationStrategy,
    StructuredHandover, SubTask, SubTaskStatus, WorkerAssignment,
};
