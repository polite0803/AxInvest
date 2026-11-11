// SPDX-License-Identifier: AGPL-3.0-only

export { useAgentPanelStore } from "./shared/agentPanelStore";
export type { AgentContext, AgentPanelTab, AgentSelection } from "./shared/agentPanelStore";
export { useArtifactStore } from "./shared/artifactStore";
export { type TabItem, useTabStore } from "./shared/tabStore";
export { useUIStore } from "./shared/uiStore";
export { useWorkspaceStore as useWorkspaceTabStore, type WorkspaceTab } from "./shared/workspaceStore";

export {
  type AnimationMode,
  initAnimationPreference,
  isAnimationEnabled,
  useAnimationStore,
} from "./shared/animationStore";
export { useBackendStatusStore } from "./shared/backendStatusStore";
export { useErrorNotificationStore } from "./shared/errorNotificationStore";

export { useDecompositionStore } from "./feature/decompositionStore";
export { useDynamicUIStore } from "./feature/dynamicUIStore";
export { useKnowledgeSourceStore } from "./feature/knowledgeSourceStore";
export { useLlmWikiStore } from "./feature/llmWikiStore";
export { useProactiveStore } from "./feature/proactiveStore";
export { useStyleStore } from "./feature/styleStore";
export { useThemeStore } from "./feature/themeStore";
export { useWikiStore } from "./feature/wikiStore";

export { useEvaluatorStore } from "./devtools/evaluatorStore";
export { useFineTuneStore } from "./devtools/fineTuneStore";
export { useRecommendationStore } from "./devtools/recommendationStore";
export { useTracerStore } from "./devtools/tracerStore";
export { useCompressStore } from "./domain/compressStore";
export { useConversationStore } from "./domain/conversationStore";
export { useMultiModelStore } from "./domain/multiModelStore";
export { usePreferenceStore } from "./domain/preferenceStore";
export { useStreamStore } from "./domain/streamStore";
export { useAgentRoleStore } from "./feature/agentRoleStore";
export { setupAgentEventListeners, useAgentStore } from "./feature/agentStore";
export { useAppConfigStore } from "./feature/appConfigStore";
export type { FeatureFlags, ModelTier as AppConfigModelTier } from "./feature/appConfigStore";
export { useApprovalStore } from "./feature/approvalStore";
export { useBackupStore } from "./feature/backupStore";
export { useBuddyStore } from "./feature/buddyStore";
export type { BuddyInfo, BuddyMessage, BuddyMood } from "./feature/buddyStore";
export { useCacheStore } from "./feature/cacheStore";
export { useCapabilityStore } from "./feature/capabilityStore";
export { useCategoryStore } from "./feature/categoryStore";
export { ensureRouteEventListening, useCognitiveRouteStore } from "./feature/cognitiveRouteStore";
export { useContextSourceStore } from "./feature/contextSourceStore";
export { formatCny, useCurrencyStore, useFormatCny } from "./feature/currencyStore";
export { useDeviceSyncStore } from "./feature/deviceSyncStore";
export { setupDreamEventListeners, useDreamStore } from "./feature/dreamStore";
export { useEvolutionStore } from "./feature/evolutionStore";
export type { ABTestResult, EngineLog, EngineStatus, EvolutionEvent, SkillVersion } from "./feature/evolutionStore";
export { useExecutionStore } from "./feature/executionStore";
export { useExpertStore } from "./feature/expertStore";
export { useGatewayLinkStore } from "./feature/gatewayLinkStore";
export { initGatewayStatusListener, useGatewayStore } from "./feature/gatewayStore";
export { useKnowledgeGraphStore } from "./feature/knowledgeGraphStore";
export { useKnowledgeStore } from "./feature/knowledgeStore";
export { useLocalToolStore } from "./feature/localToolStore";
export { useMcpStore } from "./feature/mcpStore";
export type { DiscoveredMcpServer } from "./feature/mcpStore";
export { useMemoryStore } from "./feature/memoryStore";
export { useMultiAgentStore } from "./feature/multiAgentStore";
export { useNudgeStore } from "./feature/nudgeStore";
export { useOfficeStore } from "./feature/officeStore";
export { useOnboardingStore } from "./feature/onboardingStore";
export { usePaperStore } from "./feature/paperStore";
export { setupPlanEventListeners, usePlanStore } from "./feature/planStore";
export { usePlatformStore } from "./feature/platformStore";
export { usePluginStore } from "./feature/pluginStore";
export { usePromptTemplateStore } from "./feature/promptTemplateStore";
export { useProviderStore } from "./feature/providerStore";
export { useReadingListStore } from "./feature/readingListStore";
export { useRlTrainingStore } from "./feature/rlTrainingStore";
export type { CheckpointInfo, RLTrainingConfig, TrainingMetrics } from "./feature/rlTrainingStore";
export { useSchedulerStore } from "./feature/schedulerStore";
export { useSearchStore } from "./feature/searchStore";
export { useSecurityStore } from "./feature/securityStore";
export { useSettingsStore } from "./feature/settingsStore";
export type { GlobalShortcutDiagnostic, GlobalShortcutStatus } from "./feature/settingsStore";
export { useSkillExtensionStore } from "./feature/skillExtensionStore";
export { useSkillStore } from "./feature/skillStore";
export { useSourceStore } from "./feature/sourceStore";
export { setupTaskShapeApprovalListeners, useTaskShapeApprovalStore } from "./feature/taskShapeApprovalStore";
export type { TaskShapeApprovalEvent } from "./feature/taskShapeApprovalStore";
export { useTerminalStore } from "./feature/terminalStore";
export { useTopicGroupStore } from "./feature/topicGroupStore";
export { useTrajectoryStore } from "./feature/trajectoryStore";
export { useTTSChannelStore } from "./feature/ttsChannelStore";
export { useUserProfileStore } from "./feature/userProfileStore";
export type { AvatarType } from "./feature/userProfileStore";
export { useVoicePreferenceStore } from "./feature/voicePreferenceStore";
export { useWorkEngineStore } from "./feature/workEngineStore";
export type { PausedExecutionInfo } from "./feature/workEngineStore";
export { useWorkflowEditorStore } from "./feature/workflowEditorStore";
export type { SaveSkillWorkflowResponse, SimilarWorkflow } from "./feature/workflowEditorStore";
export { useWorkflowStore } from "./feature/workflowStore";

// ── AxInvest 投资域 ──
// ⚠ 域 store 不再经本 barrel 转发（宿主 barrel 只留跨域共享 DTO）：
// crossMarket / paperPortfolio / quant 三个 store 请直接从 `@/stores/feature/<file>` 取；
// stockAnalysisStore 的 PortfolioStressResult / SimulationScenarioRow / SimulationSnapshot
// 三个类型请直接从 `@/stores/feature/stockAnalysisStore` 取（域类型归域，见 AGENTS.md 禁区 3）。
// `useStockAnalysisStore` 与 `usePipelineStore` 仍保留导出：前者约 48 处经 barrel 消费，后者被
// `PipelineJobCard` 经 barrel 使用 —— 收口需另批迁移，勿在本处顺手摘除。
export { usePipelineStore } from "./feature/pipelineStore";
export { useStockAnalysisStore } from "./feature/stockAnalysisStore";
export { useTaskCenterStore } from "./feature/taskCenterStore";
export type { RunFilters, RunTimeRange } from "./feature/taskCenterStore";
export { useWorkspaceStore, type WorkspaceView } from "./feature/workspaceStore";
