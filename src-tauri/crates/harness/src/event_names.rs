// SPDX-License-Identifier: AGPL-3.0-only

//! Tauri IPC 事件名契约 —— **前端监听（`listen`）与后端发射（`emit`）的单一来源**。
//!
//! ## 为什么需要它
//!
//! 事件名此前在两侧各写一遍字符串：后端 `.emit("agent-done", ..)`、前端 `listen("agent-done")`。
//! 任一侧改名或删除，另一侧**静默失效** —— 前端监听器永远收不到消息，不报错、单测也过，
//! `scripts/check-contracts.mjs` 项 G 只能事后比对字符串集合（本仓已在
//! `agent-plan-ready-for-approval` 上真实踩到该形态）。
//!
//! 现在改为：本枚举是唯一来源，发射点写 `IpcEventName::AgentDone.as_str()`，
//! 前端 `listen` 的形参类型是 schema-gen 由本枚举生成的联合类型
//! （`src/types/generated/events.ts`）。于是：
//!
//! - **删掉一个成员** ⇒ 所有引用它的发射点 `cargo check` 失败（编译期穷举）；
//! - 前端监听一个非成员名 ⇒ `npm run typecheck` 报错；
//! - 成员存在但无任何发射点 ⇒ `scripts/check-contracts.mjs` 项 G 报错（有声明无入边）。
//!
//! ## 收录边界（重要）
//!
//! 本枚举 = **前端监听过的事件名全集**，不是「后端 emit 全集」。
//! 后端仅推送、前端从未监听的事件名（如 `index-job-*` / `dream-*` / `stock-t0-*`）
//! **不在此列** —— 它们不构成前端契约，纳入只会让「成员必有发射点」这条判据失去意义。
//! 判据是：**能被前端 `listen` 的名字，必须在这里；在这里的名字，必须有发射点。**
//!
//! ## 生成物与门禁
//!
//! - 前端联合类型由 `cargo run -p schema-gen -- event-names` 生成，**不要手改**生成文件；
//! - `scripts/check-contracts.mjs` 项 G 校验「前端 listen ⊆ 本枚举」「本枚举成员 ⊇ 发射点」
//!   与「生成物与本枚举一致」三条；
//! - 本模块不改动任何运行期行为：`as_str()` 返回的字符串与改造前的字面量逐字相同。

/// 前端监听过的事件名（契约全集）。
///
/// `as_str()` 的返回值即 Tauri 事件通道名，与改造前的字符串字面量逐字一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpcEventName {
    // ── agent 生命周期与流式 ──
    AgentAskUser,
    AgentCancelled,
    AgentDone,
    AgentError,
    AgentMessageId,
    AgentPaused,
    AgentPermissionRequest,
    AgentPermissionTimeout,
    AgentRenderUi,
    AgentUpdateUi,
    AgentRemoveUi,
    AgentResumed,
    AgentStarted,
    AgentStatus,
    AgentStreamText,
    AgentStreamThinking,
    AgentToolResult,
    AgentToolStart,
    AgentToolUse,
    // ── 会话流式与标题 ──
    ChatStreamChunk,
    ChatStreamError,
    ConversationTitleGenerating,
    ConversationTitleUpdated,
    RagContextRetrieved,
    PromptCacheEvent,
    // ── 计划 / 工作流 ──
    PlanAuthorizationChanged,
    PlanExecutionComplete,
    PlanGenerated,
    PlanStepUpdate,
    TaskShapeApprovalRequest,
    WorkflowAiChatChunk,
    WorkflowAiChatError,
    WorkflowComplete,
    WorkflowCompleted,
    WorkflowStepDone,
    WorkflowApprovalRequested,
    WorkflowExecutionCompleted,
    WorkflowNodeStatusChanged,
    WorkflowStateChanged,
    WorkflowStepStart,
    WorkflowStepComplete,
    WorkflowStepDelta,
    WorkflowStepError,
    WorkflowError,
    PipelineStep,
    // ── 知识库 / 索引 / 记忆 / wiki ──
    KnowledgeBaseUpdated,
    KnowledgeChunkReindexed,
    KnowledgeDocumentIndexed,
    KnowledgeRebuildComplete,
    MemoryItemIndexed,
    MemoryRebuildComplete,
    WikiNoteIndexed,
    WikiRebuildComplete,
    // ── 技能 / 网关 ──
    SkillProposal,
    SkillStateChanged,
    SkillFileChanged,
    GatewayStatusChanged,
    // ── 任务 / 终端 / 行情 / 系统 ──
    AppCloseRequested,
    BackgroundTaskCreated,
    BackgroundTaskUpdated,
    FilePermissionRequest,
    PtyExit,
    PtyOutput,
    PriceAlertTriggered,
    StockMonitorT0RerunRequested,
    StockQuoteUpdate,
    SimulationReady,
    SerenityScreeningStep,
    SerenityScreeningCompleted,
    // ── 进化 / 反思 / 认知路由 ──
    EvolutionConsentRequest,
    DreamConsolidationCompleted,
    DreamConsolidationStarted,
    CognitiveRouteEvent,
    // ── 确认死链（前端监听、后端无发射） ──
    AgentRateLimit,
    AgentSubagentCard,
    WorkerCreated,
    WorkerProgress,
    WorkerCompleted,
    WorkerFailed,
}

impl IpcEventName {
    /// 事件通道名（Tauri `emit` / `listen` 的第一个参数）。
    pub fn as_str(&self) -> &'static str {
        match self {
            // ── agent 生命周期与流式 ──
            IpcEventName::AgentAskUser => "agent-ask-user",
            IpcEventName::AgentCancelled => "agent-cancelled",
            IpcEventName::AgentDone => "agent-done",
            IpcEventName::AgentError => "agent-error",
            IpcEventName::AgentMessageId => "agent-message-id",
            IpcEventName::AgentPaused => "agent-paused",
            IpcEventName::AgentPermissionRequest => "agent-permission-request",
            IpcEventName::AgentPermissionTimeout => "agent-permission-timeout",
            IpcEventName::AgentRenderUi => "agent-render-ui",
            IpcEventName::AgentUpdateUi => "agent-update-ui",
            IpcEventName::AgentRemoveUi => "agent-remove-ui",
            IpcEventName::AgentResumed => "agent-resumed",
            IpcEventName::AgentStarted => "agent-started",
            IpcEventName::AgentStatus => "agent-status",
            IpcEventName::AgentStreamText => "agent-stream-text",
            IpcEventName::AgentStreamThinking => "agent-stream-thinking",
            IpcEventName::AgentToolResult => "agent-tool-result",
            IpcEventName::AgentToolStart => "agent-tool-start",
            IpcEventName::AgentToolUse => "agent-tool-use",
            // ── 会话流式与标题 ──
            IpcEventName::ChatStreamChunk => "chat-stream-chunk",
            IpcEventName::ChatStreamError => "chat-stream-error",
            IpcEventName::ConversationTitleGenerating => "conversation-title-generating",
            IpcEventName::ConversationTitleUpdated => "conversation-title-updated",
            IpcEventName::RagContextRetrieved => "rag-context-retrieved",
            IpcEventName::PromptCacheEvent => "prompt-cache-event",
            // ── 计划 / 工作流 ──
            IpcEventName::PlanAuthorizationChanged => "plan-authorization-changed",
            IpcEventName::PlanExecutionComplete => "plan-execution-complete",
            IpcEventName::PlanGenerated => "plan-generated",
            IpcEventName::PlanStepUpdate => "plan-step-update",
            IpcEventName::TaskShapeApprovalRequest => "task-shape-approval-request",
            IpcEventName::WorkflowAiChatChunk => "workflow-ai-chat-chunk",
            IpcEventName::WorkflowAiChatError => "workflow-ai-chat-error",
            IpcEventName::WorkflowComplete => "workflow-complete",
            IpcEventName::WorkflowCompleted => "workflow-completed",
            IpcEventName::WorkflowStepDone => "workflow-step-done",
            IpcEventName::WorkflowApprovalRequested => "workflow:approval-requested",
            IpcEventName::WorkflowExecutionCompleted => "workflow:execution-completed",
            IpcEventName::WorkflowNodeStatusChanged => "workflow:node-status-changed",
            IpcEventName::WorkflowStateChanged => "workflow:state-changed",
            IpcEventName::WorkflowStepStart => "workflow-step-start",
            IpcEventName::WorkflowStepComplete => "workflow-step-complete",
            IpcEventName::WorkflowStepDelta => "workflow-step-delta",
            IpcEventName::WorkflowStepError => "workflow-step-error",
            IpcEventName::WorkflowError => "workflow-error",
            IpcEventName::PipelineStep => "pipeline-step",
            // ── 知识库 / 索引 / 记忆 / wiki ──
            IpcEventName::KnowledgeBaseUpdated => "knowledge-base-updated",
            IpcEventName::KnowledgeChunkReindexed => "knowledge-chunk-reindexed",
            IpcEventName::KnowledgeDocumentIndexed => "knowledge-document-indexed",
            IpcEventName::KnowledgeRebuildComplete => "knowledge-rebuild-complete",
            IpcEventName::MemoryItemIndexed => "memory-item-indexed",
            IpcEventName::MemoryRebuildComplete => "memory-rebuild-complete",
            IpcEventName::WikiNoteIndexed => "wiki-note-indexed",
            IpcEventName::WikiRebuildComplete => "wiki-rebuild-complete",
            // ── 技能 / 网关 ──
            IpcEventName::SkillProposal => "skill-proposal",
            IpcEventName::SkillStateChanged => "skill-state-changed",
            IpcEventName::SkillFileChanged => "skill:file-changed",
            IpcEventName::GatewayStatusChanged => "gateway-status-changed",
            // ── 任务 / 终端 / 行情 / 系统 ──
            IpcEventName::AppCloseRequested => "app-close-requested",
            IpcEventName::BackgroundTaskCreated => "background-task:created",
            IpcEventName::BackgroundTaskUpdated => "background-task:updated",
            IpcEventName::FilePermissionRequest => "file-permission-request",
            IpcEventName::PtyExit => "pty_exit",
            IpcEventName::PtyOutput => "pty_output",
            IpcEventName::PriceAlertTriggered => "price-alert-triggered",
            IpcEventName::StockMonitorT0RerunRequested => "stock-monitor-t0-rerun-requested",
            IpcEventName::StockQuoteUpdate => "stock-quote-update",
            IpcEventName::SimulationReady => "simulation-ready",
            IpcEventName::SerenityScreeningStep => "serenity-screening-step",
            IpcEventName::SerenityScreeningCompleted => "serenity-screening-completed",
            // ── 进化 / 反思 / 认知路由 ──
            IpcEventName::EvolutionConsentRequest => "evolution-consent-request",
            IpcEventName::DreamConsolidationCompleted => "dream-consolidation-completed",
            IpcEventName::DreamConsolidationStarted => "dream-consolidation-started",
            IpcEventName::CognitiveRouteEvent => "cognitive-route-event",
            // ── 确认死链（前端监听、后端无发射） ──
            IpcEventName::AgentRateLimit => "agent-rate-limit",
            IpcEventName::AgentSubagentCard => "agent-subagent-card",
            IpcEventName::WorkerCreated => "worker-created",
            IpcEventName::WorkerProgress => "worker-progress",
            IpcEventName::WorkerCompleted => "worker-completed",
            IpcEventName::WorkerFailed => "worker-failed",
        }
    }

    /// 全部成员，顺序与本文件声明序一致（schema-gen 依赖该顺序生成前端联合类型）。
    pub const ALL: &'static [IpcEventName] = &[
        IpcEventName::AgentAskUser,
        IpcEventName::AgentCancelled,
        IpcEventName::AgentDone,
        IpcEventName::AgentError,
        IpcEventName::AgentMessageId,
        IpcEventName::AgentPaused,
        IpcEventName::AgentPermissionRequest,
        IpcEventName::AgentPermissionTimeout,
        IpcEventName::AgentRenderUi,
        IpcEventName::AgentUpdateUi,
        IpcEventName::AgentRemoveUi,
        IpcEventName::AgentResumed,
        IpcEventName::AgentStarted,
        IpcEventName::AgentStatus,
        IpcEventName::AgentStreamText,
        IpcEventName::AgentStreamThinking,
        IpcEventName::AgentToolResult,
        IpcEventName::AgentToolStart,
        IpcEventName::AgentToolUse,
        IpcEventName::ChatStreamChunk,
        IpcEventName::ChatStreamError,
        IpcEventName::ConversationTitleGenerating,
        IpcEventName::ConversationTitleUpdated,
        IpcEventName::RagContextRetrieved,
        IpcEventName::PromptCacheEvent,
        IpcEventName::PlanAuthorizationChanged,
        IpcEventName::PlanExecutionComplete,
        IpcEventName::PlanGenerated,
        IpcEventName::PlanStepUpdate,
        IpcEventName::TaskShapeApprovalRequest,
        IpcEventName::WorkflowAiChatChunk,
        IpcEventName::WorkflowAiChatError,
        IpcEventName::WorkflowComplete,
        IpcEventName::WorkflowCompleted,
        IpcEventName::WorkflowStepDone,
        IpcEventName::WorkflowApprovalRequested,
        IpcEventName::WorkflowExecutionCompleted,
        IpcEventName::WorkflowNodeStatusChanged,
        IpcEventName::WorkflowStateChanged,
        IpcEventName::WorkflowStepStart,
        IpcEventName::WorkflowStepComplete,
        IpcEventName::WorkflowStepDelta,
        IpcEventName::WorkflowStepError,
        IpcEventName::WorkflowError,
        IpcEventName::PipelineStep,
        IpcEventName::KnowledgeBaseUpdated,
        IpcEventName::KnowledgeChunkReindexed,
        IpcEventName::KnowledgeDocumentIndexed,
        IpcEventName::KnowledgeRebuildComplete,
        IpcEventName::MemoryItemIndexed,
        IpcEventName::MemoryRebuildComplete,
        IpcEventName::WikiNoteIndexed,
        IpcEventName::WikiRebuildComplete,
        IpcEventName::SkillProposal,
        IpcEventName::SkillStateChanged,
        IpcEventName::SkillFileChanged,
        IpcEventName::GatewayStatusChanged,
        IpcEventName::AppCloseRequested,
        IpcEventName::BackgroundTaskCreated,
        IpcEventName::BackgroundTaskUpdated,
        IpcEventName::FilePermissionRequest,
        IpcEventName::PtyExit,
        IpcEventName::PtyOutput,
        IpcEventName::PriceAlertTriggered,
        IpcEventName::StockMonitorT0RerunRequested,
        IpcEventName::StockQuoteUpdate,
        IpcEventName::SimulationReady,
        IpcEventName::SerenityScreeningStep,
        IpcEventName::SerenityScreeningCompleted,
        IpcEventName::EvolutionConsentRequest,
        IpcEventName::DreamConsolidationCompleted,
        IpcEventName::DreamConsolidationStarted,
        IpcEventName::CognitiveRouteEvent,
        IpcEventName::AgentRateLimit,
        IpcEventName::AgentSubagentCard,
        IpcEventName::WorkerCreated,
        IpcEventName::WorkerProgress,
        IpcEventName::WorkerCompleted,
        IpcEventName::WorkerFailed,
    ];
}

impl std::fmt::Display for IpcEventName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
