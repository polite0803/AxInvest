// 此文件由 schema-gen 自动生成，请勿手动编辑
// 修改 Rust DTO 后请重新运行: cargo run -p schema-gen -- frontend-dto
//
// 与同目录 `ipc.ts` 的分工：那一份只做「后端 DTO 有没有变」的同步校验（类型不加
// export，无人消费）；本文件是**前端真正消费**的类型 —— 权威定义在后端结构体，
// `src/types/index.ts` 直接 re-export，不再维护第二份手写定义。
//
// 映射口径（由 ts-rs 按 serde 属性推导，禁止在本文件手改）：
// - `#[serde(rename_all = "camelCase")]` ⇒ 字段名 camelCase；
// - `Option<T>` ⇒ `T | null`；带 `#[serde(default)]` 的 ⇒ 可选字段 `?:`（只写 `skip_serializing_if` 不算）；
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


// ============================================================================
// 股票域（`axagent-harness::market_data` / `stock_market_types`）
// ============================================================================

export type StockQuote = { code: string, name: string, price: number, 
/**
 * 昨收价
 */
preClose: number, open: number, high: number, low: number, volume: number, amount: number, changePct: number, turnoverRate: number, pe: number | null, pb: number | null, totalMv: number | null, circulatingMv?: number | null, 
/**
 * 涨停价
 *
 * `default` 是给 ts-rs 看的：它只在 `default` 存在时把字段判为可选（`?:`），
 * 单有 `skip_serializing_if` 仍生成必填 —— 而线上载荷确实可能缺这个键。
 */
limitUp?: number | null, 
/**
 * 跌停价
 */
limitDown?: number | null, 
/**
 * 是否ST股票（含*ST）
 */
isSt: boolean, timestamp: string, };

export type KLine = { date: string, open: number, high: number, low: number, close: number, volume: number, amount: number, turnoverRate: number | null, 
/**
 * 累计复权因子 (R3-A); None 表示未应用复权
 */
adjFactor: number | null, };

export type StockSearchResult = { code: string, name: string, market: string, };

export type ConsensusEPS = { stockCode: string, consensusEps: number | null, consensusTargetPrice: number | null, ratingAvg: string | null, ratingCount: number | null, year: string, 
/**
 * 该 EPS 是否为**估算值**（非真实一致预期 / 财报数据）。
 *
 * `true` 只出现在 vendor 全部失败的兜底分支（`lib.rs` 的 C-fallback）：
 * 按挂牌板块取常数（科创/创业 0.40、北交所 0.25、其余主板 0.55）。
 *
 * **下游不得用估算值做「超预期 / 不及预期」类判定** —— 那是拿一个假基准
 * 算出来的相对量（`tools/src/tools/finance.rs` 的 `detect_earnings` 即此类消费）。
 * 形态与 `is_fallback_anchor`（`mcp_tools.rs` 的 DCF 锚点标记）一致：
 * bool 标记 + 消费端数值降级 / 拒判。
 *
 * 为什么必须是**显式标记**而不是让下游猜：常数 0.55 完全可能是某只股票的真实
 * 一致预期，数值层面不可区分 —— 只有产出端知道自己是不是编的。
 */
isEstimated: boolean, 
/**
 * 估算来源，仅 `is_estimated = true` 时有值（如 `"board_constant:star"`）。
 *
 * 留痕用：使「这条数据是编的」可归因到具体兜底规则，而非只留一句 `warn!` 日志。
 */
estimateSource?: string | null, };

export type IndexQuote = { code: string, name: string, price: number, preClose: number, changePct: number, volume: number, amount: number, };

export type EarningsEvent = { stockCode: string, stockName: string, 
/**
 * YYYY-MM-DD
 */
eventDate: string, 
/**
 * "preliminary" | "express" | "formal" | "shareholders_meeting" | "other"
 */
eventType: string, 
/**
 * 财报期间（"2025Q3" / "2025年报"）
 */
period: string | null, 
/**
 * 摘要/标题
 */
detail: string | null, 
/**
 * vendor 标识（"cninfo" / "ths"）
 */
source: string | null, createdAt: number, };

export type DashboardIndexQuote = { 
/**
 * 指数名称（如 "上证指数"）
 */
name: string, 
/**
 * 点位
 */
price: number, 
/**
 * 涨跌幅（百分比）
 */
changePct: number, };

// Total DTO types: 13
