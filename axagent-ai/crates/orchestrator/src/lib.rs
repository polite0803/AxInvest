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

pub mod capability_pack_adapters;
pub mod capability_pack_learning;
pub mod decomposer;
pub mod dynamic_subgraph;
pub mod executor;
pub mod task_context;
pub mod task_shape_classifier;
pub mod token_budget;
pub mod types;

pub use capability_pack_adapters::types::ReinforcementLearningConfig;
pub use capability_pack_adapters::types::{
    AcceptanceCriterion, CapabilityPackContext, CapabilityPackLearningConfig, EvolutionConstraints,
    MissionType, ReflectionTemplate, RewardWeightConfig,
};
pub use capability_pack_adapters::{CapabilityPackAdapter, CapabilityPackAdapterRegistry};
pub use capability_pack_learning::{
    CapabilityPackLearningEngine, DimensionScore, EvolutionRequest, EvolutionResult,
    ExperiencePoolStats, LlmInferencePort, RLExperience, RLPolicyUpdate, ReflectionRequest,
    ReflectionResult, SelfImprovementRequest, SelfImprovementResult,
};
pub use dynamic_subgraph::{DynamicSubGraph, GeneratedSubGraph};
pub use executor::{OrchestratorExecutor, OrchestratorState};
pub use task_context::{
    CapabilityPackContextManager, CapabilityPackTaskContext, TaskContextState, TaskContextSummary,
};
pub use task_shape_classifier::{DefaultTaskShapeClassifier, classify_hybrid, classify_input};
pub use token_budget::{
    BudgetDecision, CapabilityPackTokenBudgetManager, CapabilityPackTokenConfig,
    CapabilityPackTokenStats, CompactionResult, DryLeafEntry, TokenUsageSnapshot,
};
pub use types::{
    DecompositionPlan, OrchestrationError, OrchestrationEvent, OrchestrationStrategy,
    StructuredHandover, SubTask, SubTaskStatus, WorkerAssignment,
};
