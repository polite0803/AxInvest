// SPDX-License-Identifier: AGPL-3.0-only

//! ToolContext.extra 标准 key 常量定义
//!
//! 通过 ToolContext.extra HashMap 传递配置给工具，替代硬编码和全局状态。

/// 搜索提供商类型： "ddg"|"tavily"|"brave"|"bing"|"serpapi"|"google_pse"|"zhipu"|"bocha"
pub const SEARCH_PROVIDER_TYPE: &str = "search.provider_type";

/// 搜索 API 端点 URL
pub const SEARCH_ENDPOINT: &str = "search.endpoint";

/// 已解密的搜索 API key（仅内存中传递，不序列化到日志）
pub const SEARCH_API_KEY: &str = "search.api_key";

/// 区域设置 "cn"|"us" 等
pub const SEARCH_REGION: &str = "search.region";

/// 单次搜索结果数量上限
pub const SEARCH_MAX_RESULTS: &str = "search.max_results";

/// 搜索超时毫秒数
pub const SEARCH_TIMEOUT_MS: &str = "search.timeout_ms";

/// 安全搜索级别 0|1|2
pub const SEARCH_SAFE_SEARCH: &str = "search.safe_search";

/// 会话 ID
pub const CONVERSATION_ID: &str = "core.conversation_id";

/// 数据库路径（替代 global_state 的 set_db_path）
pub const SEA_DB_PATH: &str = "core.sea_db_path";

/// 模型上下文窗口（token 数，十进制字符串）。
///
/// 由 wiring 从已解析模型的 `max_tokens` 注入；`ContextRemaining` 据此按窗口换算分量预算。
/// 缺省（未注入）表示窗口未知 —— 工具会明确回报「未知」而**不是**猜一个默认值。
pub const CONTEXT_WINDOW: &str = "core.context_window";

/// 已消耗的上下文 token 数（十进制字符串，可选注入）。
///
/// 工具侧无法自行观测用量（`tools` 是 hybrid crate，读不到 agent session 的实时状态），
/// 故只在 wiring 能提供时注入；缺省时工具只报预算与阈值，不报「已用/剩余」。
///
/// **口径 = 上一轮请求实际发出的上下文占用**（取本会话最近一条有计量的 assistant 消息的
/// `prompt_tokens`，由 `commands/agent/mod.rs` 注入）—— 本轮用量在工具被调用的时刻还不存在。
/// 工作流 turn（`init/agent_turn_adapter.rs`）没有这条读取通路，故只注入窗口、不注入用量，
/// 工具在该路径下如实报「未知」。
pub const CONTEXT_USED_TOKENS: &str = "core.context_used_tokens";
