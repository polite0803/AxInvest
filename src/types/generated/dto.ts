// 此文件由 schema-gen 自动生成，请勿手动编辑
// 修改 Rust DTO 后请重新运行: cargo run -p schema-gen -- frontend-dto
//
// 与同目录 `ipc.ts` 的分工：那一份只做「后端 DTO 有没有变」的同步校验（类型不加
// export，无人消费）；本文件是**前端真正消费**的类型 —— 权威定义在后端结构体，
// `src/types/index.ts` 直接 re-export，不再维护第二份手写定义。
//
// 映射口径（由 ts-rs 按 serde 属性推导，禁止在本文件手改）：
// - `#[serde(rename_all = "camelCase")]` ⇒ 字段名 camelCase；
// - `Option<T>` ⇒ `T | null`；带 `skip_serializing_if` 的 ⇒ 可选字段 `?:`；
// - `i64/u64` ⇒ `number`（ts-rs 默认 `bigint`，与 JSON 线上类型不符，生成器已改）。

// ============================================================================
// Provider 域（`axagent-harness::types::provider_model`）
// ============================================================================

export type ProviderType = "openai" | "openai_responses" | "anthropic" | "gemini" | "openclaw" | "hermes" | "ollama" | "llama_cpp" | "typesafe";

export type ModelType = "Chat" | "Voice" | "Embedding" | "Decision";

export type ModelCapability = "TextChat" | "Vision" | "FunctionCalling" | "Reasoning" | "RealtimeVoice";

export type ProviderKey = { id: string, providerId: string, keyEncrypted: string, keyPrefix: string, enabled: boolean, lastValidatedAt: number | null, lastError: string | null, rotationIndex: number, createdAt: number, };

export type ProviderProxyConfig = { proxyType: string | null, proxyAddress: string | null, proxyPort: number | null, };


// ============================================================================
// Workflow 域（`axagent-harness::workflow_types`）
// ============================================================================

export type SubWorkflowOrigin = { 
/**
 * 父执行的 `execution_id`
 */
parentExecutionId: string, 
/**
 * 父图里发起本子执行的那个 SubWorkflow 节点 id
 */
parentNodeId: string, };

// Total DTO types: 6
