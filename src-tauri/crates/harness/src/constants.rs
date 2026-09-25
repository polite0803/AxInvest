// SPDX-License-Identifier: AGPL-3.0-only

//! 共享字符串常量
//!
//! 消除代码中的魔法字符串。按语义分组为子模块。
//! 非用户可见的内部标识符，不需要 i18n 翻译。

/// 消息/对话角色
pub mod role {
    pub const USER: &str = "user";
    pub const SYSTEM: &str = "system";
    pub const ASSISTANT: &str = "assistant";
    pub const TOOL: &str = "tool";
}

/// 状态标记
pub mod status {
    pub const RUNNING: &str = "running";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const PENDING: &str = "pending";
    pub const PROCESSING: &str = "processing";
    pub const SKIPPED: &str = "skipped";
    pub const ARCHIVED: &str = "archived";
    pub const PAUSED: &str = "paused";
    pub const CANCELLED: &str = "cancelled";
    pub const PARTIAL: &str = "partial";
    pub const ERROR: &str = "error";
    pub const READY: &str = "ready";
    pub const INDEXING: &str = "indexing";
}

/// JSON/API 字段名 — 前后端协议字段
pub mod api_field {
    pub const CONVERSATION_ID: &str = "conversationId";
    pub const MESSAGE_ID: &str = "messageId";
    pub const STREAMING_MESSAGE_ID: &str = "streamingMessageId";
    pub const ASSISTANT_MESSAGE_ID: &str = "assistantMessageId";
    pub const TEMPLATE_ID: &str = "templateId";
    pub const TEMPLATE_NAME: &str = "templateName";
    pub const SERVER_ID: &str = "serverId";
    pub const PROVIDER_ID: &str = "providerId";
    pub const SKILL_ID: &str = "skill_id";
    pub const ERROR: &str = "error";
    pub const CODE: &str = "code";
    pub const MESSAGE: &str = "message";
    pub const TYPE: &str = "type";
    pub const NAME: &str = "name";
    pub const ID: &str = "id";
    pub const INPUT: &str = "input";
    pub const OUTPUT: &str = "output";
    pub const CONTENT: &str = "content";
    pub const TEXT: &str = "text";
    pub const REASON: &str = "reason";
    pub const SUCCESS: &str = "success";
    pub const HIT: &str = "hit";
    pub const RESULT: &str = "result";
    pub const SOURCE: &str = "source";
    pub const TARGET: &str = "target";
    pub const POSITION: &str = "position";
    pub const TOOL_USE: &str = "tool_use";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const TOOL_USE_ID: &str = "tool_use_id";
    pub const TOOL_NAME: &str = "tool_name";
    pub const IS_ERROR: &str = "is_error";
    pub const IMAGE_URL: &str = "image_url";
    pub const FUNCTION: &str = "function";
    pub const WEB_SEARCH: &str = "web_search";
    pub const DOCUMENT_ID: &str = "documentId";
    pub const CONVERSATION_ID_SHORT: &str = "conversation_id";
    pub const MESSAGE_COUNT: &str = "message_count";
    pub const RESPONSE_STYLE: &str = "response_style";
    pub const CONTENT_ADJUSTMENTS: &str = "content_adjustments";
    pub const SKILL_SUGGESTIONS: &str = "skill_suggestions";
    pub const IMPROVED: &str = "improved";
    pub const IS_RUNNING: &str = "is_running";
    pub const STATS: &str = "stats";
    pub const ENABLED: &str = "enabled";
    pub const SKIPPED: &str = "skipped";
}

/// 传输类型
pub mod transport {
    pub const STREAMABLE_HTTP: &str = "streamableHttp";
    pub const STDIO: &str = "stdio";
    pub const SSE: &str = "sse";
    pub const BUILTIN: &str = "builtin";
    pub const HTTP: &str = "http";
}

/// 文件名
pub mod file_name {
    pub const SKILL_MD: &str = "SKILL.md";
    pub const MANIFEST_JSON: &str = "manifest.json";
    pub const SKILL_MANIFEST_JSON: &str = "skill-manifest.json";
    pub const FRONTEND_JSON: &str = "frontend.json";
    pub const MASTER_KEY: &str = "master.key";
    pub const AXAGENT_DB: &str = "axagent.db";
    pub const SETTINGS_JSON: &str = "settings.json";
    pub const INSTALLED_JSON: &str = "installed.json";
    pub const PLUGIN_JSON: &str = "plugin.json";
    pub const CLAUDE_PLUGIN_JSON: &str = ".claude-plugin/plugin.json";
    pub const WINDOW_STATE_JSON: &str = "window-state.json";
    pub const USER_MD: &str = "USER.md";
    pub const AGENTS_MD: &str = "AGENTS.md";
    pub const CLAUDE_MD: &str = "CLAUDE.md";
    /// 3.2 P2:长期记忆索引文件路径(原 `.axagent/memory.md` 单文件已废弃)
    ///
    /// 现在是 200 行硬限制的索引文件,索引 `.axagent/memory/` 下四类分目录
    /// 的主题文件。详见 `memory::MEMORY_INDEX`。
    pub const MEMORY_MD: &str = ".axagent/MEMORY.md";
    pub const CRASH_LOG: &str = "axagent-crash.log";
    pub const STARTUP_PHASE: &str = ".startup_phase";
    pub const PRICING_TOML: &str = "pricing.toml";
    pub const SCREEN_PNG: &str = "screen.png";
}

/// 目录名/路径片段
pub mod dir_name {
    pub const AXAGENT: &str = ".axagent";
    pub const CLAUDE: &str = ".claude";
    pub const SKILLS: &str = "skills";
}

/// Unity 改造 Feature Flag 常量（与前端 FEATURE_FLAGS 双写对齐）
pub mod unity_feature_flag {
    /// P0: 任务形态分类器（原则三标尺：上下文保留成本 × 安全隔离需求）
    pub const UNITY_P0_TASK_SHAPE: &str = "UNITY_P0_TASK_SHAPE";
}

/// 3.1 P2:投机执行影子目录常量
///
/// 投机执行(CoW 覆盖文件系统轻量方案):等待用户确认时,后台投机执行
/// 工具调用,写入 `.axagent/shadow/{session_id}/` 影子目录。用户确认后
/// diff 应用到真实目录;用户拒绝时删除影子目录回滚。
///
/// 与 FUSE/驱动级 CoW 相比,此方案无需系统级支持,跨平台兼容。
pub mod shadow {
    /// 影子目录根目录(相对于项目根)
    pub const SHADOW_DIR: &str = ".axagent/shadow";
    /// 单个文件 diff 最大大小(字节,1MB)
    pub const DIFF_FILE_SIZE_LIMIT: usize = 1024 * 1024;
    /// 影子目录最大文件数(防止失控)
    pub const SHADOW_MAX_FILES: usize = 1000;
}

/// 3.2 P2:长期记忆文件级四类分目录常量
///
/// 对齐 Claude Code MEMORY.md 模型,在 `.axagent/memory/` 下按四类分目录:
/// - `user/`       — 用户偏好/信息(技术栈、沟通风格、工作习惯)
/// - `feedback/`   — 用户反馈(显式喜好/排斥、纠正记录)
/// - `project/`    — 项目相关(架构决策、约定、命令)
/// - `reference/`  — 参考资料(外部链接、文档索引)
///
/// `.axagent/MEMORY.md` 为索引文件(200 行硬限制),始终加载,
/// 索引四类主题文件的相对路径与一句话摘要。
pub mod memory {
    /// 记忆根目录(相对于项目根)
    pub const MEMORY_DIR: &str = ".axagent/memory";
    /// 索引文件路径(相对于项目根,始终加载,200 行硬限制)
    pub const MEMORY_INDEX: &str = ".axagent/MEMORY.md";
    /// 索引文件最大行数(硬限制)
    pub const MEMORY_INDEX_MAX_LINES: usize = 200;
    /// 文件级检索时选取的最相关文件数上限
    pub const MEMORY_RELEVANT_FILES_LIMIT: usize = 5;
    /// 单个记忆主题文件大小上限(字节,256KB)
    pub const MEMORY_FILE_SIZE_LIMIT: usize = 256 * 1024;

    /// 用户偏好/信息子目录
    pub const USER_DIR: &str = "user";
    /// 用户反馈子目录
    pub const FEEDBACK_DIR: &str = "feedback";
    /// 项目相关子目录
    pub const PROJECT_DIR: &str = "project";
    /// 参考资料子目录
    pub const REFERENCE_DIR: &str = "reference";

    /// 四类分目录列表(供扫描器遍历)
    pub const ALL_DIRS: &[&str] = &[USER_DIR, FEEDBACK_DIR, PROJECT_DIR, REFERENCE_DIR];
}

/// 提供商类型/注册表 key
pub mod provider {
    pub const OPENAI: &str = "openai";
    pub const OPENAI_RESPONSES: &str = "openai_responses";
    pub const ANTHROPIC: &str = "anthropic";
    pub const GEMINI: &str = "gemini";
    pub const OLLAMA: &str = "ollama";
    pub const OPENCLAW: &str = "openclaw";
    pub const HERMES: &str = "hermes";
}

/// 默认 API 端点
pub mod default_url {
    pub const OPENAI_BASE: &str = "https://api.openai.com/v1";
    pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com/v1";
    pub const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
    pub const OLLAMA_HOST: &str = "http://localhost:11434";
    pub const OPENCLAW_HOST: &str = "http://localhost:8100";
    pub const HERMES_HOST: &str = "http://localhost:8642";
    pub const REPLICATE_API: &str = "https://api.replicate.com/v1/predictions";
    pub const SKILLS_HUB_API: &str = "https://api.agentskills.io";
    pub const HONCHO_API: &str = "https://api.honcho.ai";
    pub const MEM0_API: &str = "https://api.mem0.ai";
    pub const NPM_REGISTRY: &str = "https://registry.npmjs.org";
    pub const OTEL_ENDPOINT: &str = "http://localhost:4318";
}

/// 平台名称
pub mod platform {
    pub const TELEGRAM: &str = "telegram";
    pub const DISCORD: &str = "discord";
    pub const SLACK: &str = "slack";
    pub const WECHAT: &str = "wechat";
    pub const FEISHU: &str = "feishu";
    pub const DINGTALK: &str = "dingtalk";
    pub const WHATSAPP: &str = "whatsapp";
    pub const QQ: &str = "qq";
}

/// 默认语言
pub mod locale {
    pub const ZH_CN: &str = "zh-CN";
    pub const EN_US: &str = "en-US";
}

/// Android 平台消息
pub mod android_msg {
    pub const BROWSER_NOT_AVAILABLE: &str = "Browser automation is not available on Android";
    pub const COMPUTER_CONTROL_NOT_AVAILABLE: &str = "Computer control is not available on Android";
    pub const UI_AUTOMATION_NOT_SUPPORTED: &str = "UI automation is not supported on Android";
    pub const GIT_TOOLS_NOT_AVAILABLE: &str = "Git tools are not available on Android";
    pub const SANDBOX_NOT_AVAILABLE: &str = "Sandbox execution is not available on Android";
}

/// 窗口配置默认值
pub mod window {
    pub const MIN_WIDTH: f64 = 640.0;
    pub const MIN_HEIGHT: f64 = 480.0;
    pub const MARGIN: f64 = 100.0;
}

/// 超时/间隔默认值（秒）
pub mod timing {
    pub const AUTO_BACKUP_INTERVAL_HOURS: u64 = 24;
    pub const MEMORY_MAINTENANCE_SECS: u64 = 7200;
    pub const INSIGHT_INTERVAL_SECS: u64 = 600;
    pub const PATTERN_LEARN_SECS: u64 = 900;
    pub const CROSS_SESSION_LEARN_SECS: u64 = 1800;
    pub const RL_INTERVAL_SECS: u64 = 1200;
    pub const BATCH_PROCESS_SECS: u64 = 3600;
    pub const USER_PROFILE_PERSIST_SECS: u64 = 600;
    pub const SKILL_EVOLUTION_SECS: u64 = 2700;
    pub const MEMORY_DECAY_SECS: u64 = 3600;
    pub const AUTO_TOOL_OBSERVE_SECS: u64 = 3600;
    pub const TEXT_GRAD_ANALYSIS_SECS: u64 = 7200;
    pub const CRON_POLL_SECS: u64 = 30;
    pub const TRAJECTORY_CLEANUP_SECS: u64 = 86400;
    pub const SHUTDOWN_TIMEOUT_SECS: u64 = 5;
}

/// 嵌入相关常量
pub mod embed {
    pub const BATCH_SIZE: usize = 256;
    pub const MAX_RETRIES: u32 = 3;
    pub const RETRY_BASE_DELAY_MS: u64 = 500;
    pub const RAG_CACHE_TTL_SECS: u64 = 30;

    /// embedding 配置的**确定性错误**标记①：**没有**配置 provider。
    ///
    /// 由 `axagent_search::rag::resolve_default_embedding_provider` 产出；
    /// 该模块 `pub use` 本常量，因此 `axagent_search::rag::ERR_NO_EMBEDDING_PROVIDER`
    /// 这一既有路径仍然可用，历史调用点无需改动。
    ///
    /// 消费点唯一：`axagent_lib::index_queue` 的 R9 通道 —— 命中即把作业直接置为
    /// `failed` 终态，不消耗 `max_retries` 次指数退避。
    pub const ERR_NO_EMBEDDING_PROVIDER: &str = "EMBEDDING_PROVIDER_NOT_CONFIGURED";

    /// embedding 配置的**确定性错误**标记②：配置了 provider，但**指向的 provider
    /// 已不存在**（悬空引用 —— provider 被删除，或重建后换了 id）。
    ///
    /// #### 为什么两个标记必须定义在同一处
    ///
    /// 它们被**同一个判断**消费，分开定义必然漂移。这正是 2026-09-12 修掉的缺陷：
    /// R9 通道原本只匹配标记①，于是悬空引用的作业照常走完 `max_retries` 次指数
    /// 退避。生产实证：两个 `index_memory` 作业因 `Not found: Provider af052547-…`
    /// 停在 `retrying` 并持续刷 WARN。
    ///
    /// #### 为什么要与「运行时抖动」区分
    ///
    /// provider 不存在是**确定性**的，重试多少次结果都一样 —— 重试一个确定性无效
    /// 的动作不是韧性，只是日志噪音。两者的**用户修复动作也不同**：标记①是
    /// 「去配一个」，本标记是「去重新绑定」。
    pub const ERR_EMBEDDING_PROVIDER_GONE: &str = "EMBEDDING_PROVIDER_GONE";

    /// 全部「embedding 配置确定性错误」标记的**唯一清单**。
    ///
    /// 新增标记时的唯一动作是**加进这个数组** —— [`is_deterministic_config_error`]
    /// 从本数组派生判定，因此不存在「定义了标记但忘了加判断分支」的可能。
    /// 这正是 2026-09-12 的缺陷形态：标记② 有了，判定还是 `if contains(标记①)`。
    pub const DETERMINISTIC_CONFIG_MARKERS: &[&str] =
        &[ERR_NO_EMBEDDING_PROVIDER, ERR_EMBEDDING_PROVIDER_GONE];

    /// 错误消息是否属于「embedding 配置确定性错误」⇒ 重试结果必然相同，不应重试。
    ///
    /// 判定按 [`DETERMINISTIC_CONFIG_MARKERS`] 派生，调用方（`axagent_lib::index_queue`
    /// 的 R9 通道）不要自行写 `contains(某个标记)`，否则又会回到漏判。
    pub fn is_deterministic_config_error(err_msg: &str) -> bool {
        DETERMINISTIC_CONFIG_MARKERS.iter().any(|m| err_msg.contains(m))
    }
}

/// Agent 执行默认迭代上限。各层（coordinator / agent_runtime / conversation_runtime）
/// 引用此常量以避免三层默认值不一致。coordinator 层可在 AgentConfig 中覆写。
pub const DEFAULT_MAX_ITERATIONS: usize = 50;

/// OPC 域包（Domain Pack）协议 / 钩子的共享标识符。
///
/// 此前这两个值分别定义在 `commands/opc_workflow_kpi_hook.rs` 与
/// `commands/opc_workflows/` 内 —— 即**命令层**。后果是每个消费方都必须
/// 「跨命令模块 import」，命令依赖图退化成网（分层门禁 `commands-no-sibling-call` 会拦）。
/// 它们是不承载业务的纯标识符，权威源理应在 harness：
/// 写入端与消费端共用同一份，改名时只有一处可改。
pub mod capability_pack {
    /// 工作流 `ctx.input` 里承载域包归属的键名。
    ///
    /// 写入端 `opc_capability_pack_actions::run_template_via_engine` 与消费端
    /// `opc_workflow_kpi_hook` 必须同名 —— 不同名则 KPI **静默不落库**。
    pub const INPUT_KEY: &str = "domain_pack_id";

    /// 内容媒体域包的 KPI 落库生命周期钩子名。
    ///
    /// 模板 `hooks_config.post_exec` 按此名引用（见 `seed_content_media`）；
    /// 名称不一致 ⇒ 钩子注册了却永不触发 ⇒ 同样表现为 KPI 静默不落库。
    pub const KPI_HOOK_NAME: &str = "content-media-kpi-persist";
}

/// 认知编排执行链共享常量（护照投影 ↔ agent 工具 ↔ 循环终止辅助共用，禁止散落字面量）
pub mod capability_chain {
    /// RunWorkflow agent 工具的注册名。
    ///
    /// 护照 tool_ref（`workflow_types::WorkflowTemplatePassportParams`）与
    /// tools crate 的注册名（`tools::tools::run_workflow::RunWorkflowTool`）
    /// 必须共用此常量，否则「看得到调不动」。
    pub const RUN_WORKFLOW_TOOL: &str = "RunWorkflow";

    /// 检索类工具「未命中」输出的机器可读标记。
    ///
    /// DiscoverSkills / CapabilityBrowse 未命中时写入输出文本，
    /// runtime-core 的循环终止辅助凭此计数（连续 N 次未命中 → 注入停止检索提示），
    /// 避免跨 crate 文本匹配魔法字符串。
    pub const SEARCH_MISS_MARKER: &str = "[capability-search-miss]";
}

/// 智能路由层级
pub mod routing_tier {
    pub const BUDGET: &str = "budget";
    pub const BALANCED: &str = "balanced";
    pub const PREMIUM: &str = "premium";
}

/// 集合名前缀
pub mod collection_prefix {
    pub const KNOWLEDGE_BASE: &str = "kb";
    pub const MEMORY: &str = "mem";
    pub const WIKI: &str = "wiki";
}

/// 错误类别标签
pub mod error_category {
    pub const FACT: &str = "fact";
    pub const PREFERENCE: &str = "preference";
    pub const PROCEDURE: &str = "procedure";
    pub const CONTEXT: &str = "context";
}

/// 全局契约式约束文本（通用 prompt 契约的四模块模板）。
pub mod general_contract {
    pub const TEXT: &str = "\
# 交付物规范
- 明确输出格式：按任务类型决定输出结构（代码/报告/分析/计划），不允许无结构漫谈
- 输出必须覆盖任务中所有明确提出的要求点，不得遗漏
- 多步骤任务 must provide step-by-step logs: step number, operation type, status, output

# 禁区
- 不可编造或猜测数据——不确定的信息 must be marked as \"待确认\" or \"推测\"
- 不可跳过验证环节直接交付——each deliverable must be self-verified before output
- 不可引入与当前任务无关的额外变更或抽象
- 不可在未确认的情况下覆盖或删除现有功能
- 不可输出伪代码、占位符或\"略\"——must output complete executable content

# 证据规则
- 每个关键数据点/结论 must have a source (URL, file path, tool name)
- 统计数据 must include data caliber (time range, scope, unit)
- 代码变更 must map to specific requirements in the task description
- 多个来源数据冲突时，must present side-by-side with noted differences

# 自验环节（pre-output checklist）
Before output, verify each item:
- [ ] 是否覆盖了任务中的所有要求点？
- [ ] 是否有未经标注的推测或假设？
- [ ] 关键数据是否都有来源标注？
- [ ] 是否已完成所有步骤（无遗漏）？
- [ ] 输出格式是否满足任务要求？";
}

/// DB 内建（sentinel）行的固定主键 —— 系统自动创建、非用户可见。
///
/// # 为什么必须收敛到这一处（AGENTS.md 禁区 12）
///
/// 这些值是**跨 crate 的 DB 协议**：迁移负责 `INSERT`，repo 与 trajectory
/// 负责按同一 id `SELECT`。收敛前 `__sys_trajectory__` 在 3 个文件、
/// `__sys_trajectory_memory__` 在 3 个文件各自以字面量定义 ——
/// **值当时一致，但改一处不改另两处不会报任何错**，只会让实体图 / 记忆检索
/// **静默返空**（写入的 id 与查询的 id 不再相等）。
///
/// 这类缺陷的表现是「功能没数据」而非崩溃，排查成本极高（不报错、单测也过）。
/// 因此权威定义收敛到 harness（所有 crate 的共同依赖），消费点一律 `use`。
pub mod sentinel {
    /// 轨迹实体知识库的固定 KB id（v101 合并后接管 `trajectory_entities`）。
    pub const TRAJECTORY_KB_ID: &str = "__sys_trajectory__";

    /// 轨迹记忆的固定命名空间 id（v101 合并后接管 `trajectory_memories`）。
    pub const TRAJECTORY_MEM_NS_ID: &str = "__sys_trajectory_memory__";

    /// 上述命名空间的显示名（仅用于建行时的 `name` 列）。
    pub const TRAJECTORY_MEM_NS_NAME: &str = "System Memory (Trajectory)";

    /// sentinel KB 的 `enabled` 取值：`0` = 禁用（系统内部 KB，不参与用户可见列表）。
    ///
    /// ⚠ `knowledge_bases.enabled` 是 **INTEGER** 列。PostgreSQL 不接受把布尔
    /// 字面量 `FALSE` 插进 integer 列，会报
    /// 「字段 "enabled" 的类型为 integer, 但表达式的类型为 boolean」
    /// —— 这是 v101 曾在 PG 上令应用**完全无法启动**的直接原因。
    /// 类型标成 `i32` 就是为了让这类写法在编译期就不可能发生。
    pub const TRAJECTORY_KB_ENABLED: i32 = 0;

    /// 「Memory → 知识图谱回流」所用 KB 的固定 id。
    ///
    /// # 为什么必须是**独立**的 KB，而不是复用 memory 的 namespace id
    ///
    /// `memory_namespaces.id` 与 `knowledge_bases.id` 是**两个 id 空间**：两者都是 TEXT，
    /// 所以类型系统拦不住「拿一个当另一个用」。回流路径原先就踩了这个坑
    /// （见 `PLAN-memory-kb-reflow-id-space.md` §1.1），症状是**静默空转**：
    /// `knowledge_entities.knowledge_base_id` 上有指向 `knowledge_bases.id` 的外键
    /// （`entities/src/knowledge_entities.rs` 的 `Relation::KnowledgeBase`，`on_delete = Cascade`），
    /// 在 PG 上写入一个「长得像 id、但不是任何 KB 行」的值 ⇒ **外键违反** ⇒
    /// 该路径一行也写不进去，且失败被 `debug!` 吞掉。
    ///
    /// ⇒ 本常量就是那个「真实存在的 KB 行」的 id。行本身由
    /// `dao/src/seed.rs::ensure_sentinels` 播种（幂等、不分方言），与
    /// [`TRAJECTORY_KB_ID`] **同构**。
    pub const MEMORY_REFLOW_KB_ID: &str = "__sys_memory_reflow__";

    /// 上述 KB 的显示名（仅用于建行时的 `name` 列）。
    pub const MEMORY_REFLOW_KB_NAME: &str = "System Memory Reflow";

    /// 与 [`TRAJECTORY_KB_ENABLED`] 同义：`0` = 系统内部 KB，不参与用户可见列表。
    ///
    /// ⚠ 同样是 `i32` 而非 `bool` —— 理由见 [`TRAJECTORY_KB_ENABLED`]：该列是 INTEGER，
    /// PG 拒绝布尔字面量，曾令应用**完全无法启动**。
    pub const MEMORY_REFLOW_KB_ENABLED: i32 = 0;
}

#[cfg(test)]
mod tests {
    use super::embed;

    /// 清单里的每个标记都必须能被谓词识别，且**嵌在长消息里也要能识别**。
    ///
    /// 断言「嵌在长消息里」是刻意的：调用方传进来的是完整错误串
    /// （如 `EMBEDDING_PROVIDER_GONE: embedding_provider 指向的 provider …`），
    /// 不是裸标记。用裸标记做断言会掩盖「匹配方式写成了 `==`」这类错误。
    #[test]
    fn deterministic_markers_are_recognized_inside_longer_messages() {
        assert!(
            !embed::DETERMINISTIC_CONFIG_MARKERS.is_empty(),
            "清单不能为空 —— 空清单会让谓词恒返 false，R9 通道静默失效"
        );
        for marker in embed::DETERMINISTIC_CONFIG_MARKERS {
            let msg = format!("{marker}: embedding_provider 指向的 provider `x` 不存在");
            assert!(
                embed::is_deterministic_config_error(&msg),
                "标记 `{marker}` 在长消息中未被识别"
            );
        }
    }

    /// 反向断言：运行时抖动不能被误判成确定性配置错误。
    ///
    /// 这条比上面那条更重要 —— 误判会让「网络抖动导致的索引失败」被直接置为
    /// 终态 `failed` 且**永不重试**，属于把可自愈的失败变成永久失败。
    #[test]
    fn transient_errors_are_not_classified_as_deterministic() {
        for msg in [
            "",
            "connection reset by peer",
            "request timed out after 30s",
            "429 Too Many Requests",
            // 只差一个词：`NOT_CONFIGURED` 之外的写法不能被 contains 命中
            "EMBEDDING_PROVIDER_OTHER_REASON",
        ] {
            assert!(
                !embed::is_deterministic_config_error(msg),
                "瞬时错误被误判为确定性配置错误：{msg}"
            );
        }
    }
}
