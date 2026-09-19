// SPDX-License-Identifier: AGPL-3.0-only

export type IndexingStatus = "pending" | "indexing" | "ready" | "failed";
export type MemoryScope = "global" | "project";
export type MemorySource = "manual" | "auto_extract";

/**
 * 知识库类型：
 * - `indexed`: 默认，KB 内容存于本地 data 目录，走 RAG 索引
 * - `connected_vault`: 指针型，指向外部 Obsidian vault，agent 通过 9 个 obsidian_* 工具直接读写 live 文件
 * - `connected_linked` / `connected_subagent`: 保留枚举位，本期不实现
 */
export type KbKind =
  | "indexed"
  | "connected_vault"
  | "connected_linked"
  | "connected_subagent";

export type KnowledgeBase = {
  id: string;
  name: string;
  description?: string;
  embeddingProvider?: string;
  enabled: boolean;
  iconType?: string;
  iconValue?: string;
  sortOrder: number;
  embeddingDimensions?: number;
  retrievalThreshold?: number;
  retrievalTopK?: number;
  chunkSize?: number;
  chunkOverlap?: number;
  separator?: string;
  /** 知识库类型，默认 `indexed`；`connected_vault` 时通过 obsidian_* 工具直接读写 vault */
  kind?: KbKind;
  /** ConnectedVault 类型时的 vault 根路径（绝对路径），其他类型为 undefined */
  vaultPath?: string;
};

export type KnowledgeDocument = {
  id: string;
  knowledgeBaseId: string;
  title: string;
  sourcePath: string;
  mimeType: string;
  sizeBytes: number;
  indexingStatus: IndexingStatus;
  docType: string;
  /** 源文件内容 sha256（十六进制小写）；空串 = 未记录 / 不可读（旧数据） */
  contentHash: string;
  indexError?: string;
  sourceConversationId?: string;
  // 后端 KnowledgeDocumentDto 实际返回（repo_dtos.rs），此前缺失
  createdAt?: number;
  updatedAt?: number;
};

export type ImportDirectoryError = {
  path: string;
  error: string;
  /** 可选错误码（对应后端 `error_code` 常量），前端可按码走 i18n 翻译；缺省时回退显示 error 原文 */
  code?: string | null;
};

/** 目录导入遇到「目标文档已存在」时的冲突处理策略（对应后端 `ConflictPolicy`） */
export type ConflictPolicy = "skip" | "overwrite";

/** 目录预扫描结果中的单个可导入文件（对应后端 `DirectoryScanFile`） */
export type DirectoryScanFile = {
  /** 文件绝对路径（压缩包内部文件为解包后的临时路径，导入时按此路径读取） */
  path: string;
  /** 相对目录根的路径（POSIX 风格），导入时作为文档标题 */
  relPath: string;
  /** 无点小写扩展名（如 `md`、`pdf`） */
  extension: string;
  /** 文件大小（字节） */
  sizeBytes: number;
  /** 是否来自压缩包解包（true 时 path 指向临时解包目录） */
  fromArchive: boolean;
  /** KB 中是否已存在相同 sourcePath 的文档（导入时按 conflict 策略处理） */
  exists: boolean;
};

/** 目录预扫描结果（导入前预览：文件清单 + 统计 + 与 KB 现有文档的重叠情况） */
export type DirectoryScanResult = {
  directoryPath: string;
  recursive: boolean;
  /** 可导入文件总数（含压缩包解包出的文件） */
  totalCount: number;
  /** 被跳过的文件数（隐藏项 / 不支持的扩展名 / ignore 命中 / 解包失败） */
  skippedCount: number;
  skipped: string[];
  files: DirectoryScanFile[];
  /** KB 中已存在相同 sourcePath 的文档数（conflict=skip 时这些文件将被跳过） */
  existingCount: number;
  /** 实际使用的嵌入模型 provider（null 表示未配置，导入后不会自动索引） */
  embeddingProvider: string | null;
};

export type ImportDirectoryResult = {
  baseId: string;
  importedCount: number;
  skippedCount: number;
  errorCount: number;
  imported: KnowledgeDocument[];
  skipped: string[];
  errors: ImportDirectoryError[];
};

/** 目录增量同步结果（对齐后端 `SyncDirectoryResult`：base_id + added/updated/deleted/skipped 计数） */
export type SyncDirectoryResult = {
  baseId: string;
  addedCount: number;
  updatedCount: number;
  deletedCount: number;
  skippedCount: number;
  errorCount: number;
  added: string[];
  updated: string[];
  deleted: string[];
  skipped: string[];
  errors: ImportDirectoryError[];
};

export type RetrievalHit = {
  id: string;
  conversationId: string;
  messageId: string;
  knowledgeBaseId: string;
  documentId: string;
  chunkRef: string;
  score: number;
  preview: string;
};

export type CreateKnowledgeBaseInput = {
  name: string;
  description?: string;
  embeddingProvider?: string;
  enabled?: boolean;
  /** KB 类型，默认 `indexed` */
  kind?: KbKind;
  /** ConnectedVault 类型时的 vault 根路径（绝对路径） */
  vaultPath?: string;
};

export type UpdateKnowledgeBaseInput = Partial<CreateKnowledgeBaseInput> & {
  iconType?: string | null;
  iconValue?: string | null;
  updateIcon?: boolean;
  embeddingDimensions?: number;
  updateEmbeddingDimensions?: boolean;
  retrievalThreshold?: number;
  updateRetrievalThreshold?: boolean;
  retrievalTopK?: number;
  updateRetrievalTopK?: boolean;
  chunkSize?: number;
  updateChunkSize?: boolean;
  chunkOverlap?: number;
  updateChunkOverlap?: boolean;
  separator?: string;
  updateSeparator?: boolean;
};

// ── RAG Pipeline Config ───────────────────────────────────

export type EnhancementConfig = {
  enabled: boolean;
  strategy: "none" | "hyde" | "multi_query" | "decomposition" | "auto";
  maxVariants: number;
  combinedCall: boolean;
};

/**
 * 重排配置。字段形状与后端 `crates/harness/src/rag_config.rs::RerankConfig` 一一对应。
 *
 * ⚠ 曾有一个 `ollamaEndpoint: string | null` 字段（2026-09-15 删）：后端 `RerankConfig`
 * **没有**该字段，写入后会被 serde 当作未知键忽略（不报错，但也没有任何作用），
 * 属「幽灵字段」—— 依 AGENTS.md 禁区 13（TS 类型须与后端 DTO 对齐）删除。
 * 需要 Ollama 端点的是 `SelfRagConfig.ollamaEndpoint`（后端确有该字段）。
 *
 * ⚠ `backend` 的 wire 名就是 `backend`：后端字段名为 `backend`，
 * 靠 `#[serde(rename_all = "camelCase")]` 输出同名。曾有一个 `#[serde(rename = "type")]`
 * 把它改成 `type`，导致本类型写出的 JSON 后端解析失败（见
 * `rag_config.rs::tests::frontend_shaped_config_must_parse` 的注释）。
 */
export type RerankConfig = {
  enabled: boolean;
  backend: "rule" | "cross_encoder" | "pipeline";
  crossEncoderModel: string | null;
  topN: number;
  candidateK: number;
  ruleFilterKeep: number;
  scoreThreshold: number | null;
};

export type SelfRagConfig = {
  enabled: boolean;
  judgeModel: string;
  ollamaEndpoint: string;
  relevanceThreshold: number;
  qualityThreshold: number;
  maxRetryRounds: number;
};

export type HybridConfig = {
  enabled: boolean;
  vectorWeight: number;
  bm25Weight: number;
  sparseWeight: number;
  fusion: "rrf" | "weighted";
  rrfK: number;
};

/// Graph RAG 增强检索（实体图谱）。
///
/// `enabled=true` 时后端会向 `RAGPipeline` 注入 `EntityGraphProvider`，
/// 阶段 4 用 `graph_enhanced_search` 取「实体 + 关系 + 邻居」并合入注入 prompt 的上下文。
/// 默认 `false`（图检索会改变注入内容，属可见行为变更）。
export type EntityGraphConfig = {
  enabled: boolean;
};

export type RAGPipelineConfig = {
  queryEnhancement: EnhancementConfig;
  rerank: RerankConfig;
  selfRag: SelfRagConfig;
  /// 多引擎 RAG：混合检索权重与融合算法（后端 `HybridConfig`）
  hybrid?: HybridConfig;
  /// Graph RAG 增强检索（后端 `EntityGraphConfig`）
  entityGraph?: EntityGraphConfig;
};
