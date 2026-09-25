// SPDX-License-Identifier: AGPL-3.0-only

//! 运行时能力注册表 — 「一切皆插件」重构的核心接缝。
//!
//! 目标：让**内置实现**与**外部插件**以同一路径注册/检索可替换能力，
//! 对应 DeepSeek Harness 的「内置核心也是插件」（无特权核心）思想。
//!
//! 与 `capability.rs`（能力发现元数据 / 护照）互补但不同：
//! - `capability.rs`：面向检索/路由的**元数据描述**（[`crate::CapabilityPassport`]）
//! - 本模块：面向运行时的**实现注册与类型化检索**（[`CapabilityRegistry`]）
//!
//! 三件套对应 DeepSeek Harness 的 Capability Seam：
//! - **ServiceDefinition**：声明某个能力接缝的接口契约（id / 版本 / 契约路径 / 描述）
//! - **Provider**：实现该接缝的具体对象（类型擦除后以 `Arc<dyn Any + Send + Sync>` 持有）
//! - **Consumer**：通过接缝对应的类型化入口（如 [`CapabilityRegistry::get_agent_turn_runner`]）取回
//!
//! **存储模型（P1）**：全部实现只写**一张表** `inner`，取回时按 trait object 向下转型。
//! 插件的「能力声明」不写入该表，另存 `declarations`（声明 ≠ 实现，见 [`DeclarationEntry`]）。
//!
//! 所有注册都是可逆的（返回 [`crate::EffectHandle`]），支持运行期热插拔与隔离回滚。

use crate::agent_turn_runner::AgentTurnRunner;
use crate::reversible_effect::{EffectHandle, EffectScope};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 能力接缝的接口契约声明（ServiceDefinition）。
///
/// 只声明「*有* 这样一个可替换能力、其契约是什么」，不包含实现。
#[derive(Debug, Clone)]
pub struct ServiceDefinition {
    /// 全局唯一能力 ID（如 `"model.provider"`、`"agent.loop"`）。
    pub id: String,
    /// 契约版本（语义化）。
    pub version: String,
    /// 权威 trait 的完整路径（文档与调试用，如 `"axagent_harness::ProviderAdapter"`）。
    pub contract: String,
    /// 人类可读描述。
    pub description: String,
    /// 本接缝**依赖**的其他接缝 ID（依赖收敛，见 [`CapabilityRegistry::register_consumer`]）。
    ///
    /// 空 = 无依赖（预置构造函数默认如此）。装配时用于校验顺序与诊断：
    /// 依赖未就绪的消费者会被记入 `pending_consumers`，依赖补齐后自动就绪。
    pub requires: Vec<String>,
}

impl ServiceDefinition {
    /// 构造一个能力接缝声明。
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        contract: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            contract: contract.into(),
            description: description.into(),
            requires: Vec::new(),
        }
    }

    /// 声明本接缝依赖的其他接缝 ID（链式构造）。
    ///
    /// 与 [`CapabilityRegistry::register_consumer`] 配套：被依赖的接缝未注册时，
    /// 消费方会被记入 `pending_consumers`，而不是静默拿到 `None`。
    pub fn with_requires(mut self, ids: &[&str]) -> Self {
        self.requires = ids.iter().map(|s| (*s).to_string()).collect();
        self
    }

    /// 预置 agent-loop 接缝（Agent 主循环）。
    pub fn agent_loop() -> Self {
        Self::new(
            "agent.loop",
            "1.0",
            "axagent_harness::AgentTurnRunner",
            "Agent 主循环：实现 AgentTurnRunner，驱动单轮或多轮 ReAct 执行",
        )
    }

    /// 预置 tool-set 接缝（工具集）。
    ///
    /// contract 为 [`crate::ToolSetProvider`]（**提供者视角**：贡献一组工具）；
    /// 消费方视角的查询/执行面是 `crate::ToolRegistry`，由工具注册表自行实现，
    /// 不经本接缝注册。
    pub fn tool_set() -> Self {
        Self::new(
            "tool.set",
            "1.0",
            "axagent_harness::ToolSetProvider",
            "工具集：实现 ToolSetProvider，向工具注册表贡献一组工具",
        )
    }

    /// 预置 model-provider 接缝（LLM Provider 适配器，按 provider 类型名注册多实例）。
    ///
    /// 注册 ID 为 `model.provider.{provider_type}`（如 `model.provider.deepseek`）。
    /// 内置 14 家适配器与外部插件适配器走同一入口（「一切皆插件」缺口 G4）。
    pub fn model_provider(provider_type: &str) -> Self {
        Self::new(
            format!("model.provider.{provider_type}"),
            "1.0",
            "axagent_harness::ProviderAdapter",
            format!("LLM Provider 适配器：实现 ProviderAdapter，接入 {provider_type} 提供商"),
        )
    }

    /// 预置 system-prompt 接缝（系统提示词段，按段名注册多实例）。
    ///
    /// 注册 ID 为 `system.prompt.{section_name}`。每个实现贡献一段在每轮 LLM
    /// 调用前动态注入的文本（「段（section）可插拔」）—— 内置注入器与外部
    /// 插件走同一入口（「一切皆插件」缺口 G4）。
    pub fn system_prompt_section(section_name: &str) -> Self {
        Self::new(
            format!("system.prompt.{section_name}"),
            "1.0",
            "axagent_harness::ContextContributor",
            format!("系统提示词段：实现 ContextContributor，每轮调用前注入 {section_name} 段"),
        )
    }

    /// 预置 session-store 接缝（会话状态存储）。
    ///
    /// 单实例接缝：实现 [`crate::SessionStateStore`]（会话作用域 KV，含 TTL），
    /// 由 `dao` 层提供（`DaoSessionStateStore`）。工具侧原先经 `OnceLock` + setter
    /// 各自注入同一实例，现统一从本接缝取回（「一切皆插件」缺口 G4）。
    pub fn session_state_store() -> Self {
        Self::new(
            "session.store",
            "1.0",
            "axagent_harness::SessionStateStore",
            "会话状态存储：实现 SessionStateStore，提供会话作用域 KV 读写",
        )
    }

    /// 预置 sandbox 接缝（工作流沙箱）。
    pub fn sandbox() -> Self {
        Self::new(
            "workflow.sandbox",
            "1.0",
            "axagent_harness::WorkflowSandbox",
            "工作流沙箱：实现 WorkflowSandbox，隔离执行工作流基因并校验结果",
        )
    }

    /// 预置 workflow-reflector 接缝（工作流反思）。
    pub fn workflow_reflector() -> Self {
        Self::new(
            "workflow.reflector",
            "1.0",
            "axagent_harness::WorkflowReflector",
            "工作流反思：实现 WorkflowReflector，在工作流完成后复盘/沉淀模式",
        )
    }

    /// 预置 workflow-evolver 接缝（工作流进化）。
    pub fn workflow_evolver() -> Self {
        Self::new(
            "workflow.evolver",
            "1.0",
            "axagent_harness::WorkflowEvolver",
            "工作流进化：实现 WorkflowEvolver，驱动新一代工作流变异与验证",
        )
    }

    /// 预置 workflow-optimizer 接缝（工作流优化）。
    pub fn workflow_optimizer() -> Self {
        Self::new(
            "workflow.optimizer",
            "1.0",
            "axagent_harness::WorkflowOptimizer",
            "工作流优化：实现 WorkflowOptimizer，对执行记录给出优化建议",
        )
    }

    /// 预置 business-rule 接缝（业务规则评估）。
    pub fn business_rule() -> Self {
        Self::new(
            "workflow.business_rule",
            "1.0",
            "axagent_harness::BusinessRuleEvaluator",
            "业务规则评估：实现 BusinessRuleEvaluator，dispatch 前评估节点输入",
        )
    }

    /// 预置 message-callback 接缝（消息平台入站回调）。
    pub fn message_callback() -> Self {
        Self::new(
            "message.callback",
            "1.0",
            "axagent_harness::PlatformMessageCallback",
            "消息平台入站回调：实现 PlatformMessageCallback，统一处理平台消息并返回回复",
        )
    }

    /// 预置 webhook-dispatch 接缝（Webhook 事件派发）。
    pub fn webhook_dispatch() -> Self {
        Self::new(
            "webhook.dispatch",
            "1.0",
            "axagent_harness::WebhookDispatch",
            "Webhook 事件派发：实现 WebhookDispatch，把事件投递给订阅端点",
        )
    }

    /// 预置 event-dispatch 接缝（类型化事件派发总线，P2 事件化）。
    pub fn event_dispatch() -> Self {
        Self::new(
            "event.dispatch",
            "1.0",
            "axagent_harness::EventDispatchBus",
            "类型化事件派发总线：注册事件订阅者，支持 emit/waterfall/parallel/serial 四派发模式",
        )
    }

    /// 预置 session.log.invariant 接缝（会话日志不变量，缺陷 #3 05 项）。
    pub fn session_log_invariant() -> Self {
        Self::new(
            "session.log.invariant",
            "1.0",
            "axagent_harness::SessionLogInvariant",
            "会话日志不变量：记录模型可见内容并可重建成模型所见（Model-visible means logged）",
        )
    }

    /// 预置 platform-adapter 接缝（消息平台适配器，按平台名注册多实例）。
    pub fn platform_adapter(platform_name: &str) -> Self {
        Self::new(
            format!("platform.adapter.{platform_name}"),
            "1.0",
            "axagent_harness::MessagePlatformAdapter",
            format!("消息平台适配器：实现 MessagePlatformAdapter，接入 {platform_name} 平台"),
        )
    }
}

/// 能力来源 — 内置实现与外部插件平权的关键标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityOrigin {
    /// 内置实现（第一方，随二进制分发）。
    BuiltIn,
    /// 外部插件（经 axagent-plugins 加载）。
    ExternalPlugin,
}

impl CapabilityOrigin {
    /// 来源的字符串表示。
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityOrigin::BuiltIn => "builtin",
            CapabilityOrigin::ExternalPlugin => "external_plugin",
        }
    }
}

/// 外部插件声明的能力描述（声明式，P3 外部插件注册）。
///
/// 外部插件以 shell 脚本分发，无法跨进程提供 Rust trait 对象，因此在插件
/// 启用时以「声明描述」形式注册进能力注册表（[`CapabilityOrigin::ExternalPlugin`]），
/// 供检视 / 路由 / 编排使用；禁用或卸载时经 [`EffectHandle::undo`] 可逆回滚。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityDescriptor {
    /// 能力接缝 ID（如 `"platform.adapter.telegram"`、`"tool.set.myplugin"`）。
    pub seam_id: String,
    /// 来源插件 ID。
    pub plugin_id: String,
    /// 能力类型标识（如 `"platform_adapter"`、`"tool_set"`）。
    pub capability_type: String,
    /// 契约版本（语义化）。
    pub version: String,
    /// 人类可读描述。
    pub description: String,
    /// 能力配置快照（插件声明的可选配置，JSON）。
    pub config: serde_json::Value,
}

impl PluginCapabilityDescriptor {
    /// 构造一个插件能力描述。
    pub fn new(
        seam_id: impl Into<String>,
        plugin_id: impl Into<String>,
        capability_type: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            seam_id: seam_id.into(),
            plugin_id: plugin_id.into(),
            capability_type: capability_type.into(),
            version: version.into(),
            description: description.into(),
            config: serde_json::Value::Null,
        }
    }
}

/// 能力注册错误。
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    /// 能力 ID 已被占用。
    #[error("capability `{id}` already registered")]
    Duplicate { id: String },
    /// 注册的接缝声明了依赖，但依赖的接缝未注册。
    ///
    /// 依赖收敛：`ServiceDefinition::requires` 中列出的接缝必须已注册，
    /// 否则该接缝一旦被消费方取到，就会静默拿到 `None`。
    #[error("capability `{id}` requires unregistered seam(s): {missing}")]
    MissingRequirement { id: String, missing: String },
    /// 能力不存在。
    #[error("capability `{id}` not found")]
    NotFound { id: String },
    /// 类型不匹配（向下转型失败）。
    #[error("capability `{id}` type mismatch: expected {expected}")]
    TypeMismatch { id: String, expected: &'static str },
}

/// 一条已注册的能力（定义 + 来源 + 类型擦除后的 Provider）。
struct CapabilityRegistration {
    definition: ServiceDefinition,
    origin: CapabilityOrigin,
    provider: Arc<dyn std::any::Any + Send + Sync>,
}

/// 运行时能力注册表 — 内置与未来 B 层插件统一的注册 / 类型化检索入口。
///
/// 线程安全：内部由 `RwLock<HashMap>` 保护；每条注册都是可逆的，
/// 可通过 [`CapabilityRegistry::rollback_all`] 或单条句柄撤销。
#[derive(Clone)]
pub struct CapabilityRegistry {
    /// **唯一实现表** —— 全部接缝实现（内置 / 未来 B 层插件）只写这一张表。
    ///
    /// `provider` 为 `Arc<dyn Any + Send + Sync>`；接缝实现以 `Arc<Arc<dyn Trait>>`
    /// 形式存入，取回时经 `Arc::downcast::<Arc<dyn Trait>>()` 还原（见各 `get_*`）。
    /// 之所以可行：全部接缝 trait 均声明 `std::any::Any + Send + Sync` 超 trait
    /// ⇒ `Arc<dyn Trait>: Any + Send + Sync`，满足本字段的类型约束。
    inner: Arc<RwLock<HashMap<String, CapabilityRegistration>>>,
    /// 纯声明型能力索引（插件护照声明）—— **与实现表严格分离**。
    ///
    /// 键 = `(plugin_id, seam_id)`：同一插件对同一接缝只声明一次，不同插件可各自
    /// 声明同一接缝（互不覆盖）。声明**不写入 `inner`**（缺陷 1 的修法），
    /// 理由见 [`CapabilityRegistry::register_plugin_capability`]。
    declarations: Arc<RwLock<HashMap<(String, String), DeclarationEntry>>>,
    /// 依赖收敛：已声明 `requires` 但依赖尚未就绪的消费者。
    ///
    /// 键 = 消费者 ID，值 = 其声明的依赖清单。任一注册路径成功后自动重查，
    /// 依赖补齐即移出本表（见 `refresh_pending_consumers`）。
    pending_consumers: Arc<RwLock<HashMap<String, PendingConsumer>>>,
    effects: EffectScope,
}

impl CapabilityRegistry {
    /// 创建空能力注册表。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            declarations: Arc::new(RwLock::new(HashMap::new())),
            pending_consumers: Arc::new(RwLock::new(HashMap::new())),
            effects: EffectScope::new(),
        }
    }

    /// 注册一条**实现型**接缝 —— 全部实现注册路径（通用 [`Self::register`] 与
    /// 各类型化 `register_*`）的唯一写入点。
    ///
    /// `provider` 为 trait object 时以 `Arc<Arc<dyn Trait>>` 存入单表，
    /// 取回时由各 `get_*` 向下转型还原。
    ///
    /// **为何抽成一处**：注册路径都要写 `inner`，若各自内联，「依赖收敛」的触发点
    /// 就会有多个 —— 漏掉任何一个都不会有编译期提示，后果是消费者永远停在
    /// `pending`（静默失效）。故此处既是去重，也是唯一不变量。
    fn register_seam(
        &self,
        definition: ServiceDefinition,
        provider: Arc<dyn std::any::Any + Send + Sync>,
        suffix: &str,
    ) -> Result<EffectHandle, CapabilityError> {
        let id = definition.id.clone();
        self.commit_generic_registration(definition, CapabilityOrigin::BuiltIn, provider)?;
        // 可逆效果：撤销时从唯一实现表移除该接缝。
        let undo = self.inner.clone();
        let undo_id = id.clone();
        Ok(self.effects.register(format!("capability:{id}:{suffix}"), move || {
            undo.write().remove(&undo_id);
        }))
    }

    /// 从唯一实现表按 ID 取回类型化 provider（向下转型；类型不符或未注册均为 `None`）。
    ///
    /// `T` 即注册时存入的**内层**类型：trait object 接缝为 `dyn SomeTrait`，
    /// 具体类型接缝为 `EventDispatchBus` 等。实现以 `Arc<T>` 存入单表，
    /// 故此处先向下转型到 `Arc<T>`，再取其内层 `Arc<T>` 返回
    /// （单表持有额外一层 `Arc` 仅为满足 `Arc<dyn Any>` 的 `Sized` 要求）。
    fn provider_as<T: ?Sized + Send + Sync + 'static>(&self, id: &str) -> Option<Arc<T>> {
        let stored: Arc<Arc<T>> =
            self.inner.read().get(id)?.provider.clone().downcast::<Arc<T>>().ok()?;
        Some((*stored).clone())
    }

    /// 提交一条注册 —— 依赖校验 → 重复检查 → 写入唯一实现表 → 触发依赖收敛重查。
    fn commit_generic_registration(
        &self,
        definition: ServiceDefinition,
        origin: CapabilityOrigin,
        provider: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), CapabilityError> {
        let id = definition.id.clone();
        // 依赖收敛：接缝自己声明的依赖必须先就绪（见 `ServiceDefinition::requires`），
        // 否则消费方取到本接缝后会静默拿到 `None`。
        if !definition.requires.is_empty() {
            let missing = self.missing_seams(&definition.requires);
            if !missing.is_empty() {
                return Err(CapabilityError::MissingRequirement {
                    id: id.clone(),
                    missing: missing.join(", "),
                });
            }
        }
        // 重复检查：单表内同一 ID 只允许一条注册（含内置，先注册者胜出）。
        {
            let mut slot = self.inner.write();
            if slot.contains_key(&id) {
                return Err(CapabilityError::Duplicate { id });
            }
            slot.insert(id.clone(), CapabilityRegistration { definition, origin, provider });
        }
        self.release_ready_consumers(&id);
        Ok(())
    }

    /// 依赖收敛：本接缝补上后重查等待它的消费者，并把「刚就绪」记入日志。
    ///
    /// 消费方的激活动作由 wiring 层自行决定，这里只推进状态。
    fn release_ready_consumers(&self, seam_id: &str) {
        for consumer_id in self.refresh_pending_consumers() {
            tracing::info!(
                consumer = %consumer_id,
                seam = %seam_id,
                "依赖已就绪，消费者可激活（依赖收敛）"
            );
        }
    }

    /// 注册一个类型擦除的 Provider（通用入口，用于无专用 trait 的接缝，如 `tool.set`）。
    ///
    /// 返回可单独撤销的句柄；重复注册同一 ID 返回 [`CapabilityError::Duplicate`]。
    pub fn register(
        &self,
        definition: ServiceDefinition,
        origin: CapabilityOrigin,
        provider: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<EffectHandle, CapabilityError> {
        let id = definition.id.clone();
        let undo_id = id.clone();
        self.commit_generic_registration(definition, origin, provider)?;

        // 注册一个可逆效果：撤销时从注册表移除该能力。
        let this = self.clone();
        let undo = this.inner.clone();
        Ok(self.effects.register(format!("capability:{id}"), move || {
            let mut guard = undo.write();
            guard.remove(&undo_id);
        }))
    }

    /// 登记插件声明的一条能力（**声明型，非实现**）。
    ///
    /// 外部插件以 shell 脚本分发，无法跨进程提供 Rust trait object ⇒ 其清单里的
    /// `capabilities[].seam` 只能表达「我声称提供该接缝」，**不构成任何实现**。
    ///
    /// **为何不写入实现表 `inner`（缺陷 1）**：一旦写入，`contains(seam)` 为真
    /// 而全部 `get_*()` 返回 `None` —— 接缝「看起来有提供者、运行时已死」，
    /// 且随 `seam` 是自由字符串（无校验）而与内置接缝同名时，
    /// 旧实现还会先撤销内置（缺陷 2）。故声明改存独立的 `declarations` 索引：
    /// - 不影响 `contains` / `get_*` / `list_platform_adapters`（这些只看实现表）
    /// - 经 [`Self::list_with_details`] 可见，并以 `implemented == false` 标注
    /// - 禁用 / 卸载插件时经 [`EffectHandle::undo`] 可逆移除
    ///
    /// 键为 `(plugin_id, seam_id)` ⇒ 同一插件重复声明同一条返回
    /// [`CapabilityError::Duplicate`]；不同插件声明同一接缝互不覆盖。
    pub fn register_plugin_capability(
        &self,
        descriptor: PluginCapabilityDescriptor,
    ) -> Result<EffectHandle, CapabilityError> {
        let key = (descriptor.plugin_id.clone(), descriptor.seam_id.clone());
        let entry = DeclarationEntry {
            definition: ServiceDefinition::new(
                &descriptor.seam_id,
                &descriptor.version,
                "axagent_harness::PluginCapabilityDescriptor",
                &descriptor.description,
            ),
            plugin_id: descriptor.plugin_id.clone(),
        };
        {
            let mut decls = self.declarations.write();
            if decls.contains_key(&key) {
                return Err(CapabilityError::Duplicate { id: descriptor.seam_id });
            }
            decls.insert(key.clone(), entry);
        }
        let decls = self.declarations.clone();
        let (plugin_id, seam_id) = key.clone();
        Ok(self.effects.register(format!("capability-decl:{plugin_id}:{seam_id}"), move || {
            decls.write().remove(&key);
        }))
    }

    /// 注销一个能力（直接移除实现，不走可逆效果）。
    pub fn unregister(&self, id: &str) -> Result<(), CapabilityError> {
        let mut slot = self.inner.write();
        if slot.remove(id).is_none() {
            return Err(CapabilityError::NotFound { id: id.to_string() });
        }
        drop(slot);
        self.pending_consumers.write().remove(id);
        Ok(())
    }

    // ───────────────────────── 依赖收敛（§4.3 G5） ─────────────────────────

    /// 声明一个消费者及其依赖的接缝 ID（依赖收敛）。
    ///
    /// 返回**尚未就绪**的接缝 ID 清单（空 = 依赖全部就绪，可直接激活）：
    /// - 全部就绪 ⇒ 立即返回空清单，不进入 `pending_consumers`
    /// - 存在缺失 ⇒ 记入 `pending_consumers`（供 [`Self::pending_consumers_snapshot`] 检视），
    ///   并把缺失清单返回给调用方 —— **调用方据此决定日志 / 降级 / 阻断，而非静默 `None`**
    ///
    /// 重复调用同一 `id` 会覆盖其旧的依赖声明。
    pub fn register_consumer(&self, id: &str, requires: &[&str]) -> Vec<String> {
        let owned: Vec<String> = requires.iter().map(|s| (*s).to_string()).collect();
        let missing = self.missing_seams(&owned);
        let mut pending = self.pending_consumers.write();
        if missing.is_empty() {
            pending.remove(id);
        } else {
            pending.insert(id.to_string(), PendingConsumer { id: id.to_string(), requires: owned });
        }
        missing
    }

    /// 重查全部 pending 消费者（[`Self::register`] 成功后自动调用）。
    ///
    /// 返回**本次刚变为就绪**的消费者 ID 清单（调用方据此激活 / 记日志）。
    pub fn refresh_pending_consumers(&self) -> Vec<String> {
        // 先取已注册接缝快照，再取 pending 写锁 —— 保持与 register() 一致的锁序
        // （inner → pending_consumers），避免反向持锁造成死锁。
        let registered: HashSet<String> = self.inner.read().keys().cloned().collect();
        let mut ready = Vec::new();
        let mut pending = self.pending_consumers.write();
        pending.retain(|consumer_id, consumer| {
            if consumer.requires.iter().all(|r| registered.contains(r)) {
                ready.push(consumer_id.clone());
                false
            } else {
                true
            }
        });
        ready
    }

    /// 当前仍被依赖阻塞的消费者快照（装配诊断 / dump-config 用）。
    pub fn pending_consumers_snapshot(&self) -> Vec<PendingConsumer> {
        let mut list: Vec<PendingConsumer> =
            self.pending_consumers.read().values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// 从 `requires` 中筛出尚未注册的接缝 ID。
    fn missing_seams(&self, requires: &[String]) -> Vec<String> {
        let registered = self.inner.read();
        let mut missing = Vec::new();
        for seam in requires {
            if !registered.contains_key(seam) {
                missing.push(seam.clone());
            }
        }
        missing
    }

    /// 按 ID 取回类型擦除的 Provider。
    pub fn get(&self, id: &str) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.inner.read().get(id).map(|r| r.provider.clone())
    }

    /// 按 ID + 类型约束向下转型取回 Provider（Consumer 入口）。
    ///
    /// `T` 必须是具体类型（`Sized`），如 `Arc<String>`；需要取回 trait object 时，
    /// 请使用对应的类型化入口（如 [`CapabilityRegistry::get_agent_turn_runner`]）——
    /// 它们经 [`Self::provider_as`] 从同一张表向下转型还原。
    pub fn get_typed<T: std::any::Any + Send + Sync>(&self, id: &str) -> Option<Arc<T>> {
        self.get(id)?.downcast::<T>().ok()
    }

    /// 注册 Agent 主循环（`agent.loop` 接缝）。
    ///
    /// consumers 可通过 [`CapabilityRegistry::get_agent_turn_runner`] 取回。
    pub fn register_agent_loop(
        &self,
        runner: Arc<dyn AgentTurnRunner>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(ServiceDefinition::agent_loop(), Arc::new(runner), "agent-loop")
    }

    /// 取回 agent-loop 接缝上的 Agent 主循环（若已注册）。
    pub fn get_agent_turn_runner(&self) -> Option<Arc<dyn AgentTurnRunner>> {
        self.provider_as::<dyn AgentTurnRunner>("agent.loop")
    }

    // ── sandbox 接缝 ────────────────────────────────────────────────────────

    /// 注册一个 WorkflowSandbox（`workflow.sandbox` 接缝）。
    pub fn register_sandbox(
        &self,
        sandbox: Arc<dyn crate::WorkflowSandbox>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(ServiceDefinition::sandbox(), Arc::new(sandbox), "sandbox")
    }

    /// 取回 sandbox 接缝上的工作流沙箱（若已注册）。
    pub fn get_sandbox(&self) -> Option<Arc<dyn crate::WorkflowSandbox>> {
        self.provider_as::<dyn crate::WorkflowSandbox>("workflow.sandbox")
    }

    // ── workflow-reflector 接缝 ─────────────────────────────────────────────

    /// 注册一个 WorkflowReflector（`workflow.reflector` 接缝）。
    pub fn register_workflow_reflector(
        &self,
        reflector: Arc<dyn crate::WorkflowReflector>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::workflow_reflector(),
            Arc::new(reflector),
            "workflow-reflector",
        )
    }

    /// 取回 workflow-reflector 接缝上的反思实现（若已注册）。
    pub fn get_workflow_reflector(&self) -> Option<Arc<dyn crate::WorkflowReflector>> {
        self.provider_as::<dyn crate::WorkflowReflector>("workflow.reflector")
    }

    // ── workflow-evolver 接缝 ───────────────────────────────────────────────

    /// 注册一个 WorkflowEvolver（`workflow.evolver` 接缝）。
    pub fn register_workflow_evolver(
        &self,
        evolver: Arc<dyn crate::WorkflowEvolver>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::workflow_evolver(),
            Arc::new(evolver),
            "workflow-evolver",
        )
    }

    /// 取回 workflow-evolver 接缝上的进化实现（若已注册）。
    pub fn get_workflow_evolver(&self) -> Option<Arc<dyn crate::WorkflowEvolver>> {
        self.provider_as::<dyn crate::WorkflowEvolver>("workflow.evolver")
    }

    // ── workflow-optimizer 接缝 ─────────────────────────────────────────────

    /// 注册一个 WorkflowOptimizer（`workflow.optimizer` 接缝）。
    pub fn register_workflow_optimizer(
        &self,
        optimizer: Arc<dyn crate::WorkflowOptimizer>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::workflow_optimizer(),
            Arc::new(optimizer),
            "workflow-optimizer",
        )
    }

    /// 取回 workflow-optimizer 接缝上的优化实现（若已注册）。
    pub fn get_workflow_optimizer(&self) -> Option<Arc<dyn crate::WorkflowOptimizer>> {
        self.provider_as::<dyn crate::WorkflowOptimizer>("workflow.optimizer")
    }

    // ── business-rule 接缝 ──────────────────────────────────────────────────

    /// 注册一个 BusinessRuleEvaluator（`workflow.business_rule` 接缝）。
    pub fn register_business_rule(
        &self,
        evaluator: Arc<dyn crate::BusinessRuleEvaluator>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(ServiceDefinition::business_rule(), Arc::new(evaluator), "business-rule")
    }

    /// 取回 workflow.business_rule 接缝上的规则评估器（若已注册）。
    pub fn get_business_rule(&self) -> Option<Arc<dyn crate::BusinessRuleEvaluator>> {
        self.provider_as::<dyn crate::BusinessRuleEvaluator>("workflow.business_rule")
    }

    // ── message.callback 接缝 ───────────────────────────────────────────────

    /// 注册一个 PlatformMessageCallback（`message.callback` 接缝）。
    ///
    /// consumers 可通过 [`CapabilityRegistry::get_message_callback`] 取回。
    pub fn register_message_callback(
        &self,
        callback: Arc<dyn crate::PlatformMessageCallback>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::message_callback(),
            Arc::new(callback),
            "message-callback",
        )
    }

    /// 取回 message.callback 接缝上的消息回调（若已注册）。
    pub fn get_message_callback(&self) -> Option<Arc<dyn crate::PlatformMessageCallback>> {
        self.provider_as::<dyn crate::PlatformMessageCallback>("message.callback")
    }

    // ── webhook.dispatch 接缝 ───────────────────────────────────────────────

    /// 注册一个 WebhookDispatch（`webhook.dispatch` 接缝）。
    ///
    /// consumers 可通过 [`CapabilityRegistry::get_webhook_dispatch`] 取回。
    pub fn register_webhook_dispatch(
        &self,
        dispatcher: Arc<dyn crate::WebhookDispatch>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::webhook_dispatch(),
            Arc::new(dispatcher),
            "webhook-dispatch",
        )
    }

    /// 取回 webhook.dispatch 接缝上的派发器（若已注册）。
    pub fn get_webhook_dispatch(&self) -> Option<Arc<dyn crate::WebhookDispatch>> {
        self.provider_as::<dyn crate::WebhookDispatch>("webhook.dispatch")
    }

    // ── event.dispatch 接缝（单例类型化事件派发总线） ──────────────────────

    /// 注册类型化事件派发总线（`event.dispatch` 接缝）。
    pub fn register_event_dispatcher(
        &self,
        dispatcher: Arc<crate::EventDispatchBus>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::event_dispatch(),
            Arc::new(dispatcher),
            "event-dispatch",
        )
    }

    /// 取回 event.dispatch 接缝上的类型化事件派发总线（若已注册）。
    pub fn get_event_dispatcher(&self) -> Option<Arc<crate::EventDispatchBus>> {
        self.provider_as::<crate::EventDispatchBus>("event.dispatch")
    }

    // ── session.log.invariant 接缝（单例会话日志不变量） ─────────────────────

    /// 注册会话日志不变量（`session.log.invariant` 接缝）。
    pub fn register_session_log_invariant(
        &self,
        log: Arc<dyn crate::SessionLogInvariant>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(ServiceDefinition::session_log_invariant(), Arc::new(log), "session-log")
    }

    /// 取回 session.log.invariant 接缝上的会话日志不变量（若已注册）。
    pub fn get_session_log_invariant(&self) -> Option<Arc<dyn crate::SessionLogInvariant>> {
        self.provider_as::<dyn crate::SessionLogInvariant>("session.log.invariant")
    }

    // ── platform.adapter 接缝（多实例，按平台名注册） ──────────────────────

    /// 注册一个消息平台适配器（`platform.adapter` 接缝）。
    ///
    /// `platform_name` 即平台唯一名称（如 `"telegram"`），注册 ID 为
    /// `platform.adapter.{platform_name}`。consumers 可通过
    /// [`CapabilityRegistry::get_platform_adapter`] 按名称取回。
    pub fn register_platform_adapter(
        &self,
        platform_name: &str,
        adapter: Arc<dyn crate::MessagePlatformAdapter>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::platform_adapter(platform_name),
            Arc::new(adapter),
            "platform-adapter",
        )
    }

    /// 按平台名取回一个平台适配器（若已注册）。
    pub fn get_platform_adapter(
        &self,
        platform_name: &str,
    ) -> Option<Arc<dyn crate::MessagePlatformAdapter>> {
        let id = format!("platform.adapter.{platform_name}");
        self.provider_as::<dyn crate::MessagePlatformAdapter>(&id)
    }

    /// 列出所有已注册的平台名（去掉 `platform.adapter.` 前缀；仅含**实现**，不含声明）。
    pub fn list_platform_adapters(&self) -> Vec<String> {
        self.inner
            .read()
            .keys()
            .filter_map(|k| k.strip_prefix("platform.adapter.").map(|s| s.to_string()))
            .collect()
    }

    // ── model.provider 接缝（多实例，按 provider 类型名注册） ───────────────

    /// 注册一个 LLM Provider 适配器（`model.provider.{provider_type}` 接缝）。
    ///
    /// `provider_type` 即适配器的类型名（如 `"deepseek"`），注册 ID 为
    /// `model.provider.{provider_type}`。consumers 可通过
    /// [`CapabilityRegistry::get_model_provider`] 按类型名取回。
    pub fn register_model_provider(
        &self,
        provider_type: &str,
        adapter: Arc<dyn crate::ProviderAdapter>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::model_provider(provider_type),
            Arc::new(adapter),
            "model-provider",
        )
    }

    /// 按 provider 类型名取回一个 Provider 适配器（若已注册）。
    pub fn get_model_provider(
        &self,
        provider_type: &str,
    ) -> Option<Arc<dyn crate::ProviderAdapter>> {
        let id = format!("model.provider.{provider_type}");
        self.provider_as::<dyn crate::ProviderAdapter>(&id)
    }

    /// 列出所有已注册的 provider 类型名（去掉 `model.provider.` 前缀；仅含**实现**）。
    pub fn list_model_providers(&self) -> Vec<String> {
        self.inner
            .read()
            .keys()
            .filter_map(|k| k.strip_prefix("model.provider.").map(|s| s.to_string()))
            .collect()
    }

    // ── tool.set 接缝（工具集贡献者，单实例） ───────────────────────────────

    /// 注册一个工具集贡献者（`tool.set` 接缝）。
    ///
    /// 内置工具集与外部实现走同一入口；消费方（工具注册表初始化）通过
    /// [`CapabilityRegistry::get_tool_set`] 取回。
    pub fn register_tool_set(
        &self,
        provider: Arc<dyn crate::ToolSetProvider>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(ServiceDefinition::tool_set(), Arc::new(provider), "tool-set")
    }

    /// 取回 tool.set 接缝上的工具集贡献者（若已注册）。
    pub fn get_tool_set(&self) -> Option<Arc<dyn crate::ToolSetProvider>> {
        self.provider_as::<dyn crate::ToolSetProvider>("tool.set")
    }

    // ── system.prompt 接缝（多实例，按段名注册） ────────────────────────────

    /// 注册一个系统提示词段贡献者（`system.prompt.{section_name}` 接缝）。
    ///
    /// consumers 通过 [`CapabilityRegistry::list_system_prompt_sections`] 一次取回全部段。
    pub fn register_system_prompt_section(
        &self,
        section_name: &str,
        contributor: Arc<dyn crate::ContextContributor>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::system_prompt_section(section_name),
            Arc::new(contributor),
            "system-prompt",
        )
    }

    /// 取回全部已注册的系统提示词段贡献者，**按接缝 ID 字典序**排列。
    ///
    /// 顺序确定性：注入顺序会影响提示词内容布局，故不暴露 `HashMap` 迭代的
    /// 偶然顺序 —— 按段名排序保证同一组注册在不同运行中产出同一顺序。
    pub fn list_system_prompt_sections(&self) -> Vec<Arc<dyn crate::ContextContributor>> {
        let ids: Vec<String> = {
            let guard = self.inner.read();
            let mut ids: Vec<String> =
                guard.keys().filter(|k| k.starts_with("system.prompt.")).cloned().collect();
            ids.sort();
            ids
        };
        ids.iter().filter_map(|id| self.provider_as::<dyn crate::ContextContributor>(id)).collect()
    }

    /// 注册会话状态存储（`session.store` 接缝，单实例）。
    pub fn register_session_state_store(
        &self,
        store: Arc<dyn crate::SessionStateStore>,
    ) -> Result<EffectHandle, CapabilityError> {
        self.register_seam(
            ServiceDefinition::session_state_store(),
            Arc::new(store),
            "session-store",
        )
    }

    /// 取回会话状态存储。
    pub fn get_session_state_store(&self) -> Option<Arc<dyn crate::SessionStateStore>> {
        self.provider_as::<dyn crate::SessionStateStore>("session.store")
    }

    /// 是否已注册指定 ID。
    pub fn contains(&self, id: &str) -> bool {
        self.inner.read().contains_key(id)
    }

    /// 已注册能力数量。
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 列出全部能力定义（用于检视 / `dump-config`）。
    pub fn list_definitions(&self) -> Vec<ServiceDefinition> {
        self.inner.read().values().map(|r| r.definition.clone()).collect()
    }

    /// 列出全部能力来源（用于检视 / `dump-config`）。
    pub fn list_origins(&self) -> Vec<(String, CapabilityOrigin)> {
        self.inner.read().values().map(|r| (r.definition.id.clone(), r.origin)).collect()
    }

    /// 列出带详细来源的能力条目（用于检视 / `dump-config`）。
    ///
    /// **合并两个来源，并区分二者**（缺陷 1：声明不得冒充实现）：
    /// - 实现表 `inner` 的条目：`implemented == true`，`plugin_id` 恒为 `None`
    /// - 插件声明索引 `declarations` 的条目：`implemented == false`，标注来源插件 ID
    ///
    /// 前端据此把「声称提供」与「运行时可用」分开展示；按 `plugin_id` 过滤即可
    /// 得到「某插件声明的全部接缝」（`plugin_profile_dump` 的用法）。
    pub fn list_with_details(&self) -> Vec<CapabilityRegistrationDetail> {
        let mut list: Vec<CapabilityRegistrationDetail> = self
            .inner
            .read()
            .values()
            .map(|r| CapabilityRegistrationDetail {
                definition: r.definition.clone(),
                origin: r.origin,
                plugin_id: None,
                implemented: true,
            })
            .collect();
        list.extend(self.declarations.read().values().map(|d| CapabilityRegistrationDetail {
            definition: d.definition.clone(),
            origin: CapabilityOrigin::ExternalPlugin,
            plugin_id: Some(d.plugin_id.clone()),
            implemented: false,
        }));
        list
    }

    /// 逆序回滚全部注册（清空注册表并回放所有撤销闭包）。
    pub fn rollback_all(&self) {
        self.effects.rollback_all();
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CapabilityRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 内部为共享锁容器，无法 derive Debug；仅暴露检视友好的摘要。
        f.debug_struct("CapabilityRegistry").field("len", &self.len()).finish()
    }
}

/// 一个声明了依赖、但依赖接缝尚未就绪的消费者（依赖收敛）。
///
/// 由 [`CapabilityRegistry::register_consumer`] 记入，依赖补齐后自动移出。
/// 用途：把「漏注册只静默拿 `None`」变成**可检视的显式阻塞状态**。
#[derive(Debug, Clone)]
pub struct PendingConsumer {
    /// 消费者 ID（消费方模块名，用于诊断定位）。
    pub id: String,
    /// 其声明依赖的接缝 ID 清单。
    pub requires: Vec<String>,
}

/// 一条**纯声明型**能力（插件护照声明，无运行时实现）。
///
/// 由 [`CapabilityRegistry::register_plugin_capability`] 写入独立的 `declarations`
/// 索引，**不进入实现表** —— 声明只表示「插件声称提供某接缝」，与「该接缝在运行时
/// 可被取到」是两件事（缺陷 1 的根因即把二者混为一谈）。
#[derive(Debug, Clone)]
struct DeclarationEntry {
    /// 声明对应的接缝定义（id / 版本 / 契约 / 描述）。
    definition: ServiceDefinition,
    /// 声明方插件 ID。
    plugin_id: String,
}

/// 一条带详细来源的运行时能力条目（用于检视 / `dump-config`）。
#[derive(Debug, Clone)]
pub struct CapabilityRegistrationDetail {
    /// 能力接缝声明。
    pub definition: ServiceDefinition,
    /// 来源（内置 / 外部插件）。
    pub origin: CapabilityOrigin,
    /// 若该条目来自插件声明，则为来源插件 ID；实现型条目为 `None`。
    pub plugin_id: Option<String>,
    /// 是否**已有运行时实现**（`false` = 仅插件声明，消费方取不到该接缝）。
    pub implemented: bool,
}

/// DI 契约 — consumer crate 通过此 trait 获取能力注册表，不依赖具体实现。
pub trait HasCapabilityRegistry: Send + Sync {
    /// 返回能力注册表的共享引用。
    fn capability_registry(&self) -> Arc<CapabilityRegistry>;
}

/// 全局能力注册表实例 — 与 [`crate::get_service_registry`] 同构的过渡方案。
///
/// 后续可迁移到显式 DI 注入。
static CAPABILITY_REGISTRY: std::sync::OnceLock<CapabilityRegistry> = std::sync::OnceLock::new();

/// 获取全局能力注册表的引用。
pub fn get_capability_registry() -> &'static CapabilityRegistry {
    CAPABILITY_REGISTRY.get_or_init(CapabilityRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_turn_runner::{AgentTurnRequest, AgentTurnResult};
    use crate::core_error::Result;
    use crate::types::TokenUsage;
    use async_trait::async_trait;
    use std::sync::Arc;

    #[test]
    fn rollback_all_clears_registry() {
        let registry = CapabilityRegistry::new();
        registry.register_agent_loop(Arc::new(StubLoop) as Arc<dyn AgentTurnRunner>).unwrap();
        let _ = registry.register(
            ServiceDefinition::tool_set(),
            CapabilityOrigin::ExternalPlugin,
            Arc::new(String::from("tool")),
        );
        assert_eq!(registry.len(), 2);

        registry.rollback_all();
        assert!(registry.is_empty());
    }

    #[test]
    fn generic_register_and_typed_get_roundtrip() {
        let registry = CapabilityRegistry::new();
        let marker = Arc::new(String::from("hello"));
        let _handle = registry
            .register(
                ServiceDefinition::tool_set(),
                CapabilityOrigin::ExternalPlugin,
                marker.clone(),
            )
            .unwrap();

        let got: Arc<String> = registry.get_typed("tool.set").unwrap();
        assert_eq!(*got, "hello");
        // 用错误类型向下转型（u64 不匹配）应返回 None
        assert!(registry.get_typed::<u64>("tool.set").is_none());
    }

    #[test]
    fn global_registry_is_singleton() {
        let a = get_capability_registry();
        let b = get_capability_registry();
        assert!(std::ptr::eq(a, b));
    }

    /// 最小 AgentTurnRunner 测试替身 — 仅实现 run_turn。
    struct StubLoop;

    #[async_trait]
    impl AgentTurnRunner for StubLoop {
        async fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnResult> {
            Ok(AgentTurnResult {
                content: format!("echo:{}", request.user_input),
                thinking: None,
                tool_calls: vec![],
                usage: TokenUsage::default(),
                iterations: 1,
                stopped_by_limit: false,
            })
        }
    }

    #[test]
    fn register_and_retrieve_agent_loop() {
        let registry = CapabilityRegistry::new();
        let runner: Arc<dyn AgentTurnRunner> = Arc::new(StubLoop);
        let _handle = registry.register_agent_loop(runner.clone()).unwrap();

        assert!(registry.contains("agent.loop"));
        let got = registry.get_agent_turn_runner().unwrap();
        assert!(Arc::ptr_eq(&runner, &got));
    }

    #[test]
    fn agent_loop_duplicate_is_rejected() {
        let registry = CapabilityRegistry::new();
        let runner: Arc<dyn AgentTurnRunner> = Arc::new(StubLoop);
        registry.register_agent_loop(runner.clone()).unwrap();
        let err = registry.register_agent_loop(runner).unwrap_err();
        assert!(matches!(err, CapabilityError::Duplicate { .. }));
    }

    #[test]
    fn agent_loop_handle_undo_removes_capability() {
        let registry = CapabilityRegistry::new();
        let runner: Arc<dyn AgentTurnRunner> = Arc::new(StubLoop);
        let handle = registry.register_agent_loop(runner).unwrap();
        assert_eq!(registry.len(), 1);

        handle.undo();
        assert!(registry.is_empty());
        assert!(!registry.contains("agent.loop"));
        assert!(registry.get_agent_turn_runner().is_none());
    }

    /// 内置接缝端到端闭环（缺陷 #11）：全部内置接缝经注册表注册后，
    /// 能被真实 consumer 检索 API 取回同一实例（`Arc::ptr_eq`），证明
    /// 「内置核心平权」链路是闭环——注册与消费落在同一存储，且对象可被消费方使用。
    #[test]
    fn builtin_seams_end_to_end_register_and_consumable() {
        let registry = CapabilityRegistry::new();

        // 1) agent.loop：注册 → 消费侧 get_agent_turn_runner 取回
        let loop_runner: Arc<dyn AgentTurnRunner> = Arc::new(StubLoop);
        registry.register_agent_loop(loop_runner.clone()).unwrap();
        assert!(Arc::ptr_eq(&loop_runner, &registry.get_agent_turn_runner().unwrap()));

        // 2) sandbox：注册 → get_sandbox 取回
        let sandbox: Arc<dyn crate::WorkflowSandbox> = Arc::new(StubSandbox);
        registry.register_sandbox(sandbox.clone()).unwrap();
        assert!(Arc::ptr_eq(&sandbox, &registry.get_sandbox().unwrap()));

        // 3) session.log.invariant：注册 → get_session_log_invariant 取回
        let log: Arc<dyn crate::SessionLogInvariant> = Arc::new(crate::InMemorySessionLog::new());
        registry.register_session_log_invariant(log.clone()).unwrap();
        assert!(Arc::ptr_eq(&log, &registry.get_session_log_invariant().unwrap()));

        // 4) message.callback：注册 → get_message_callback 取回
        let cb: Arc<dyn crate::PlatformMessageCallback> = Arc::new(StubCallback);
        registry.register_message_callback(cb.clone()).unwrap();
        assert!(Arc::ptr_eq(&cb, &registry.get_message_callback().unwrap()));

        // 5) webhook.dispatch：注册 → get_webhook_dispatch 取回
        let dispatch: Arc<dyn crate::WebhookDispatch> = Arc::new(StubDispatch);
        registry.register_webhook_dispatch(dispatch.clone()).unwrap();
        assert!(Arc::ptr_eq(&dispatch, &registry.get_webhook_dispatch().unwrap()));

        // 6) platform.adapter：注册 → get_platform_adapter 取回
        let platform: Arc<dyn crate::MessagePlatformAdapter> =
            Arc::new(StubPlatform { name: "telegram" });
        registry.register_platform_adapter("telegram", platform.clone()).unwrap();
        assert!(Arc::ptr_eq(&platform, &registry.get_platform_adapter("telegram").unwrap()));

        // 7) event.dispatch：注册 → get_event_dispatcher 取回
        let bus = Arc::new(crate::EventDispatchBus::new());
        registry.register_event_dispatcher(bus.clone()).unwrap();
        assert!(Arc::ptr_eq(&bus, &registry.get_event_dispatcher().unwrap()));

        // 7 个内置接缝全部注册成功，且消费检索能取回同一实例（闭环成立）。
        assert_eq!(registry.len(), 7);
    }

    /// 最小 PlatformMessageCallback 测试替身。
    struct StubCallback;

    #[async_trait]
    impl crate::PlatformMessageCallback for StubCallback {
        async fn on_message(
            &self,
            _platform: &str,
            user_id: &str,
            _username: Option<&str>,
            _chat_id: &str,
            text: &str,
        ) -> Option<String> {
            Some(format!("echo:{user_id}:{text}"))
        }

        async fn save_cursor(&self, _platform: &str, _cursor: i64) {}
    }

    #[test]
    fn register_and_retrieve_message_callback() {
        let registry = CapabilityRegistry::new();
        let cb: Arc<dyn crate::PlatformMessageCallback> = Arc::new(StubCallback);
        let _handle = registry.register_message_callback(cb.clone()).unwrap();

        assert!(registry.contains("message.callback"));
        let got = registry.get_message_callback().unwrap();
        assert!(Arc::ptr_eq(&cb, &got));
    }

    #[test]
    fn message_callback_duplicate_is_rejected() {
        let registry = CapabilityRegistry::new();
        let cb: Arc<dyn crate::PlatformMessageCallback> = Arc::new(StubCallback);
        registry.register_message_callback(cb.clone()).unwrap();
        let err = registry.register_message_callback(cb).unwrap_err();
        assert!(matches!(err, CapabilityError::Duplicate { .. }));
    }

    #[test]
    fn message_callback_handle_undo_removes_capability() {
        let registry = CapabilityRegistry::new();
        let cb: Arc<dyn crate::PlatformMessageCallback> = Arc::new(StubCallback);
        let handle = registry.register_message_callback(cb).unwrap();
        assert_eq!(registry.len(), 1);

        handle.undo();
        assert!(registry.is_empty());
        assert!(!registry.contains("message.callback"));
        assert!(registry.get_message_callback().is_none());
    }

    /// 最小 WebhookDispatch 测试替身。
    struct StubDispatch;

    #[async_trait]
    impl crate::WebhookDispatch for StubDispatch {
        async fn dispatch(
            &self,
            _event: crate::WebhookEvent,
            _data: std::collections::HashMap<String, serde_json::Value>,
        ) -> crate::DispatchResult {
            crate::DispatchResult { success_count: 0, failure_count: 0, errors: Vec::new() }
        }
    }

    #[test]
    fn register_and_retrieve_webhook_dispatch() {
        let registry = CapabilityRegistry::new();
        let d: Arc<dyn crate::WebhookDispatch> = Arc::new(StubDispatch);
        let _handle = registry.register_webhook_dispatch(d.clone()).unwrap();

        assert!(registry.contains("webhook.dispatch"));
        let got = registry.get_webhook_dispatch().unwrap();
        assert!(Arc::ptr_eq(&d, &got));
    }

    #[test]
    fn webhook_dispatch_handle_undo_removes_capability() {
        let registry = CapabilityRegistry::new();
        let d: Arc<dyn crate::WebhookDispatch> = Arc::new(StubDispatch);
        let handle = registry.register_webhook_dispatch(d).unwrap();
        assert_eq!(registry.len(), 1);

        handle.undo();
        assert!(registry.is_empty());
        assert!(!registry.contains("webhook.dispatch"));
        assert!(registry.get_webhook_dispatch().is_none());
    }

    /// 最小 MessagePlatformAdapter 测试替身。
    struct StubPlatform {
        name: &'static str,
    }

    #[async_trait]
    impl crate::MessagePlatformAdapter for StubPlatform {
        fn name(&self) -> &'static str {
            self.name
        }

        fn is_enabled(&self, _config: &crate::platform_config::PlatformConfig) -> bool {
            true
        }

        async fn start(
            &self,
            _config: &crate::platform_config::PlatformConfig,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_connected(&self) -> bool {
            true
        }

        async fn send_message(
            &self,
            _config: &crate::platform_config::PlatformConfig,
            _chat_id: &str,
            _text: &str,
            _parse_mode: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn register_and_retrieve_platform_adapter() {
        let registry = CapabilityRegistry::new();
        let a: Arc<dyn crate::MessagePlatformAdapter> = Arc::new(StubPlatform { name: "foo" });
        let _handle = registry.register_platform_adapter("foo", a.clone()).unwrap();

        assert!(registry.contains("platform.adapter.foo"));
        let got = registry.get_platform_adapter("foo").unwrap();
        assert!(Arc::ptr_eq(&a, &got));
    }

    #[test]
    fn platform_adapter_duplicate_is_rejected() {
        let registry = CapabilityRegistry::new();
        let a: Arc<dyn crate::MessagePlatformAdapter> = Arc::new(StubPlatform { name: "x" });
        registry.register_platform_adapter("x", a.clone()).unwrap();
        let err = registry.register_platform_adapter("x", a).unwrap_err();
        assert!(matches!(err, CapabilityError::Duplicate { .. }));
    }

    #[test]
    fn platform_adapter_handle_undo_removes_capability() {
        let registry = CapabilityRegistry::new();
        let a: Arc<dyn crate::MessagePlatformAdapter> = Arc::new(StubPlatform { name: "y" });
        let handle = registry.register_platform_adapter("y", a).unwrap();
        assert_eq!(registry.len(), 1);

        handle.undo();
        assert!(registry.is_empty());
        assert!(!registry.contains("platform.adapter.y"));
        assert!(registry.get_platform_adapter("y").is_none());
    }

    #[test]
    fn list_platform_adapters_returns_names_without_prefix() {
        let registry = CapabilityRegistry::new();
        let a1: Arc<dyn crate::MessagePlatformAdapter> =
            Arc::new(StubPlatform { name: "telegram" });
        let a2: Arc<dyn crate::MessagePlatformAdapter> = Arc::new(StubPlatform { name: "discord" });
        registry.register_platform_adapter("telegram", a1).unwrap();
        registry.register_platform_adapter("discord", a2).unwrap();

        let mut names = registry.list_platform_adapters();
        names.sort();
        assert_eq!(names, vec!["discord", "telegram"]);
    }

    #[test]
    fn register_plugin_capability_records_declaration_only_and_rolls_back() {
        let registry = CapabilityRegistry::new();
        let handle = registry
            .register_plugin_capability(PluginCapabilityDescriptor::new(
                "platform.adapter.telegram",
                "external:demo",
                "platform_adapter",
                "1.0",
                "demo plugin telegram adapter",
            ))
            .unwrap();

        // 声明**不构成实现**：不占实现表、不出现来源路由、平台适配器列表不含它
        assert!(!registry.contains("platform.adapter.telegram"));
        assert!(registry.is_empty());
        assert!(registry.list_origins().is_empty());
        assert!(registry.list_platform_adapters().is_empty());
        assert!(registry.get_platform_adapter("telegram").is_none());

        // 但可在检视视图中看到，并标注「仅声明、未实现」
        let details = registry.list_with_details();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].definition.id, "platform.adapter.telegram");
        assert_eq!(details[0].plugin_id.as_deref(), Some("external:demo"));
        assert!(!details[0].implemented);

        // 逆序回滚撤销声明
        handle.undo();
        assert!(registry.list_with_details().is_empty());
    }

    #[test]
    fn register_plugin_capability_duplicate_is_rejected_per_plugin() {
        let registry = CapabilityRegistry::new();
        registry
            .register_plugin_capability(PluginCapabilityDescriptor::new(
                "tool.set.demo",
                "external:demo",
                "tool_set",
                "1.0",
                "demo tool set",
            ))
            .unwrap();
        // 同一插件重复声明同一条接缝 → Duplicate
        let err = registry
            .register_plugin_capability(PluginCapabilityDescriptor::new(
                "tool.set.demo",
                "external:demo",
                "tool_set",
                "1.0",
                "conflict",
            ))
            .unwrap_err();
        assert!(matches!(err, CapabilityError::Duplicate { .. }));

        // 不同插件声明同一条接缝互不覆盖（声明按 (plugin_id, seam) 索引）
        registry
            .register_plugin_capability(PluginCapabilityDescriptor::new(
                "tool.set.demo",
                "external:other",
                "tool_set",
                "2.0",
                "another claim",
            ))
            .unwrap();
        let details = registry.list_with_details();
        assert_eq!(details.len(), 2);
        assert!(details.iter().all(|d| !d.implemented));
    }

    /// 最小 WorkflowSandbox 测试替身。
    struct StubSandbox;

    #[async_trait]
    impl crate::WorkflowSandbox for StubSandbox {
        async fn execute(
            &self,
            _genome: &crate::WorkflowGenome,
            _test_input: &serde_json::Value,
        ) -> std::result::Result<crate::SandboxValidationResult, String> {
            Ok(crate::SandboxValidationResult::default())
        }
    }

    #[test]
    fn register_and_retrieve_sandbox_with_undo() {
        let registry = CapabilityRegistry::new();
        let sandbox: Arc<dyn crate::WorkflowSandbox> = Arc::new(StubSandbox);
        let handle = registry.register_sandbox(sandbox.clone()).unwrap();

        assert!(registry.contains("workflow.sandbox"));
        let got = registry.get_sandbox().unwrap();
        assert!(Arc::ptr_eq(&sandbox, &got));

        handle.undo();
        assert!(!registry.contains("workflow.sandbox"));
        assert!(registry.get_sandbox().is_none());
    }

    /// 缺陷 2 回归：插件声明与内置接缝**同名**时，不得打死内置实现。
    ///
    /// 旧行为：`prepare_registration` 见到 `ExternalPlugin` 就先撤销同键内置句柄，
    /// 而插件只写入一份声明 ⇒ 接缝变死（`contains == true` 但 `get_*()` 为 `None`），
    /// 且插件禁用后内置**永不复活**（`seam` 是自由字符串，无校验）。
    /// 现行为：声明与实现分离，声明既不动实现表也不撤销内置。
    #[test]
    fn plugin_declaration_never_evicts_builtin_seam() {
        let registry = CapabilityRegistry::new();
        let builtin: Arc<dyn crate::WorkflowSandbox> = Arc::new(StubSandbox);
        registry.register_sandbox(builtin.clone()).unwrap();

        // 插件声明与内置接缝同名（这正是旧实现里触发 evict 的路径）
        let decl = registry
            .register_plugin_capability(PluginCapabilityDescriptor::new(
                "workflow.sandbox",
                "external:rogue",
                "workflow_sandbox",
                "1.0",
                "claims the builtin seam",
            ))
            .unwrap();

        // 内置实现仍在，且仍是同一实例
        assert!(Arc::ptr_eq(&builtin, &registry.get_sandbox().unwrap()));

        // 撤销声明后内置照旧可用（旧实现此处已永久损坏）
        decl.undo();
        assert!(Arc::ptr_eq(&builtin, &registry.get_sandbox().unwrap()));
    }

    /// 缺陷 2 回归配套：重复**实现**注册一律 `Duplicate`，且不破坏已注册实现。
    #[test]
    fn duplicate_implementation_registration_keeps_existing_one() {
        let registry = CapabilityRegistry::new();
        let first: Arc<dyn AgentTurnRunner> = Arc::new(StubLoop);
        registry.register_agent_loop(first.clone()).unwrap();

        let second: Arc<dyn AgentTurnRunner> = Arc::new(StubLoop);
        let err = registry.register_agent_loop(second).unwrap_err();
        assert!(matches!(err, CapabilityError::Duplicate { .. }));
        assert!(Arc::ptr_eq(&first, &registry.get_agent_turn_runner().unwrap()));
    }

    #[test]
    fn register_and_retrieve_event_dispatcher() {
        let registry = CapabilityRegistry::new();
        let dispatcher = Arc::new(crate::EventDispatchBus::new());
        let handle = registry.register_event_dispatcher(dispatcher.clone()).unwrap();

        assert!(registry.contains("event.dispatch"));
        let got = registry.get_event_dispatcher().unwrap();
        assert!(Arc::ptr_eq(&dispatcher, &got));

        handle.undo();
        assert!(!registry.contains("event.dispatch"));
        assert!(registry.get_event_dispatcher().is_none());
    }

    #[test]
    fn register_and_retrieve_session_log_invariant() {
        let registry = CapabilityRegistry::new();
        let log: Arc<dyn crate::SessionLogInvariant> = Arc::new(crate::InMemorySessionLog::new());
        let handle = registry.register_session_log_invariant(log.clone()).unwrap();

        assert!(registry.contains("session.log.invariant"));
        let got = registry.get_session_log_invariant().unwrap();
        // 记录一条 model-visible 内容并校验可重建，验证接缝取回的实现可用
        got.record_model_visible(
            "s1",
            crate::ModelVisibleContent::from_chat_message(&crate::types::ChatMessage {
                role: "user".into(),
                content: crate::types::ChatContent::Text("可见内容".into()),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            }),
        );
        assert!(got.assert_replayable("s1").is_ok());

        handle.undo();
        assert!(!registry.contains("session.log.invariant"));
        assert!(registry.get_session_log_invariant().is_none());
    }

    // ─────────────────────── 依赖收敛（P0 / §4.3 G5） ───────────────────────

    #[test]
    fn register_consumer_reports_missing_seams_and_records_pending() {
        let registry = CapabilityRegistry::new();
        let missing = registry.register_consumer("orchestrator", &["agent.loop", "tool.set"]);
        assert_eq!(missing, vec!["agent.loop".to_string(), "tool.set".to_string()]);

        let pending = registry.pending_consumers_snapshot();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "orchestrator");
        assert_eq!(pending[0].requires, missing);
    }

    #[test]
    fn register_consumer_with_ready_seams_is_not_pending() {
        let registry = CapabilityRegistry::new();
        let _ = registry
            .register(
                ServiceDefinition::tool_set(),
                CapabilityOrigin::BuiltIn,
                Arc::new(String::from("tool")),
            )
            .unwrap();

        let missing = registry.register_consumer("orchestrator", &["tool.set"]);
        assert!(missing.is_empty());
        assert!(registry.pending_consumers_snapshot().is_empty());
    }

    #[test]
    fn refresh_reports_newly_ready_consumers_once() {
        let registry = CapabilityRegistry::new();
        registry.register_consumer("orchestrator", &["tool.set"]);

        // 直接补上接缝（绕过注册路径的自动重查），以隔离验证 refresh 的返回语义
        registry.inner.write().insert(
            "tool.set".to_string(),
            CapabilityRegistration {
                definition: ServiceDefinition::tool_set(),
                origin: CapabilityOrigin::BuiltIn,
                provider: Arc::new(String::from("tool")),
            },
        );

        assert_eq!(registry.refresh_pending_consumers(), vec!["orchestrator".to_string()]);
        assert!(registry.pending_consumers_snapshot().is_empty());
        // 已就绪者不重复上报
        assert!(registry.refresh_pending_consumers().is_empty());
    }

    #[test]
    fn all_register_paths_release_pending_consumer() {
        // 特化注册路径（register_agent_loop / register_sandbox）也必须触发依赖收敛，
        // 否则消费者会永远停在 pending（静默失效）。
        let registry = CapabilityRegistry::new();
        registry.register_consumer("agent.runner", &["agent.loop"]);
        registry.register_consumer("workflow.runner", &["workflow.sandbox"]);
        assert_eq!(registry.pending_consumers_snapshot().len(), 2);

        registry.register_agent_loop(Arc::new(StubLoop) as Arc<dyn AgentTurnRunner>).unwrap();
        assert_eq!(registry.pending_consumers_snapshot().len(), 1);

        registry.register_sandbox(Arc::new(StubSandbox)).unwrap();
        assert!(registry.pending_consumers_snapshot().is_empty());
    }

    #[test]
    fn register_rejects_seam_whose_requires_are_unregistered() {
        let registry = CapabilityRegistry::new();
        let def =
            ServiceDefinition::new("plugin.x", "1.0", "x", "x").with_requires(&["agent.loop"]);

        let err = registry
            .register(def, CapabilityOrigin::ExternalPlugin, Arc::new(String::from("x")))
            .expect_err("依赖未就绪时应拒绝注册");
        assert!(matches!(err, CapabilityError::MissingRequirement { .. }));

        // 依赖补齐后同一接缝可正常注册
        registry.register_agent_loop(Arc::new(StubLoop) as Arc<dyn AgentTurnRunner>).unwrap();
        let def =
            ServiceDefinition::new("plugin.x", "1.0", "x", "x").with_requires(&["agent.loop"]);
        assert!(
            registry
                .register(def, CapabilityOrigin::ExternalPlugin, Arc::new(String::from("x")))
                .is_ok()
        );
    }

    #[test]
    fn register_consumer_overwrites_previous_requires() {
        let registry = CapabilityRegistry::new();
        registry.register_consumer("orchestrator", &["agent.loop"]);
        registry.register_consumer("orchestrator", &["agent.loop", "tool.set"]);

        let pending = registry.pending_consumers_snapshot();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].requires, vec!["agent.loop".to_string(), "tool.set".to_string()]);
    }

    #[test]
    fn unregister_clears_pending_consumer_entry() {
        let registry = CapabilityRegistry::new();
        let _ = registry
            .register(
                ServiceDefinition::tool_set(),
                CapabilityOrigin::BuiltIn,
                Arc::new(String::from("tool")),
            )
            .unwrap();
        registry.register_consumer("tool.set", &["agent.loop"]);
        assert_eq!(registry.pending_consumers_snapshot().len(), 1);

        registry.unregister("tool.set").unwrap();
        assert!(registry.pending_consumers_snapshot().is_empty());
    }

    /// 缺陷 2 回归（外部**实现型**注册路径）：外部插件经通用 [`CapabilityRegistry::register`]
    /// 注册一条与内置**同名**的接缝时，必须被 [`CapabilityError::Duplicate`] 明确拒绝，
    /// 且内置实现不得被顶掉。
    ///
    /// 与 [`plugin_declaration_never_evicts_builtin_seam`] 的区别：那条走的是**声明**路径
    /// （`register_plugin_capability`，天然不写实现表）；本条的调用方误把外部提供者当作
    /// 实现型注册（旧实现会先撤销内置句柄再写入），故须验证失败是**显式的 `Duplicate`**
    /// 而非静默替换 —— 调用方据此得知「接缝已被占用」，而非事后发现内置实现消失。
    #[test]
    fn external_implementation_on_builtin_seam_is_duplicate_not_replacement() {
        let registry = CapabilityRegistry::new();
        let builtin: Arc<dyn crate::WorkflowSandbox> = Arc::new(StubSandbox);
        registry.register_sandbox(builtin.clone()).unwrap();

        let err = registry
            .register(
                ServiceDefinition::sandbox(),
                CapabilityOrigin::ExternalPlugin,
                Arc::new(String::from("impostor")),
            )
            .expect_err("同名接缝的外部实现注册应被拒绝");
        assert!(matches!(err, CapabilityError::Duplicate { .. }));

        // 内置实现仍是同一实例，且检视视图中它仍是「已实现」条目
        assert!(Arc::ptr_eq(&builtin, &registry.get_sandbox().unwrap()));
        let details = registry.list_with_details();
        assert_eq!(details.len(), 1);
        assert!(details[0].implemented);
        assert!(details[0].plugin_id.is_none());
    }
}
