// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 例程执行分支（docs/bot-integration/P2/ADR-P2-2 / ADR-P2-4）。
//!
//! cron executor 识别 `job.bot_id` 后进入本模块：按 Bot 档非交互跑一轮
//! agent_query 等价路径（与 `init/agent_turn_adapter.rs` 的 headless 先例同构：
//! prompter = None，ask 类裁决走 `PermissionPolicy` 自动 Deny），结果由调用方
//! 经 `record_run` 落 `cron_job_history`，一次性任务跑完置 Disabled。
//!
//! 不走独立进程 / 独立会话池：调度器 30s tick 单派发、在途去重、leader 租约
//! 已是现成并发护栏（ADR-P2-2）。完成/失败经既有 agent-done / agent-error
//! 事件进入前端通知管线（ADR-P2-3，零新增管线）。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axagent_agent::{AxAgentApiClient, ChannelPermissionPrompter, SessionManager};
use axagent_harness::resolve_base_url_for_type;
use axagent_harness::runtime_types::conversation::ApiClient;
use axagent_harness::runtime_types::permissions::PermissionPolicy;
use axagent_harness::types::provider_model::resolve_provider_proxy;
use axagent_harness::{ProviderAdapter, ProviderRequestContext};
use axagent_runtime::harness::RuntimeHarness;
use axagent_runtime_core::{
    ConversationRuntimeFactoryArgs, RuntimeFeatureConfig, create_conversation_runtime,
};
use axagent_tools::registry::UnifiedToolRegistry;
use axagent_tools::tools::bot_delegate::{
    DELEGATE_CONTEXT, DelegateContext, MAX_DELEGATE_DEPTH, with_delegate_context,
};
use tokio::sync::Mutex;

/// 例程轮的执行依赖（executor 闭包捕获一次，每轮复用）。
pub struct BotRoutineContext {
    pub harness: RuntimeHarness,
    pub session_manager: Arc<SessionManager>,
}

/// P5-b B2：一轮非交互 Bot 执行回传的用量（ADR-P5-8）。
#[derive(Debug, Clone, PartialEq)]
pub struct BotTurnUsage {
    /// 本轮实际使用的模型 id（四类分价查表用，ADR-P5-4）。
    pub model_id: String,
    /// 本轮执行 id（账本 execution_id 快照）。
    pub execution_id: String,
    pub usage: axagent_harness::conversation_model::TokenUsage,
}

/// P5-b B2：带回传的结果。`usage=None` 表示本路径不产生用量（Fake/无模型），账本不写行。
#[derive(Debug, Clone, PartialEq)]
pub struct BotTurnOutcome {
    pub text: String,
    pub usage: Option<BotTurnUsage>,
}

/// 组装 Bot 例程的 system prompt：SOUL/MEMORY/技能注入（复用 `bot_context`，
/// 与 `agent_query` 同一真相源；ADR-P2-2 / P5 C1-2）。
async fn build_bot_system_prompt(harness: &RuntimeHarness, bot_id: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut has_soul = false;
    if let Some(ctx) =
        crate::bot_context::resolve_bot_context_for_bot(harness.db(), Some(bot_id)).await
    {
        if let Some(seg) = ctx.soul_segment() {
            has_soul = true;
            parts.push(seg);
        }
        if let Some(seg) = ctx.memory_segment() {
            parts.push(seg);
        }
    }
    // P5 C1-2：仅列名字+一行描述，不注入全文；无技能时不追加段。
    if let Some(seg) =
        crate::bot_skill_service::bot_skills_prompt_segment(harness.db(), bot_id).await
    {
        parts.push(seg);
    }
    if !has_soul {
        let has_memory = parts.iter().any(|p| p.starts_with("<bot-memory>"));
        let has_skills = parts.iter().any(|p| p.starts_with("<bot-skills>"));
        if parts.is_empty() {
            // 无 SOUL/MEMORY/技能：保持既有兜底。
            parts.push(format!(
                "<bot-soul>\n（Bot {bot_id} 未配置 SOUL，按通用助手身份执行本例程。）\n</bot-soul>"
            ));
        } else if has_skills && !has_memory {
            // 有技能但无 SOUL/MEMORY：补身份边界，避免失去归属语义。
            parts.insert(
                0,
                format!(
                    "<bot-soul>\n（Bot {bot_id} 未配置 SOUL，按通用助手身份执行本例程。）\n</bot-soul>"
                ),
            );
        }
        // memory-only（±skills）：保持原行为，不强插 fallback。
    }
    parts.join("\n\n")
}

/// 解析例程轮的 provider/model：Bot 自带（`bots.provider_id`/`model_id`）优先，
/// 未配置时回落第一个启用且含可用 key 的提供商（与 `agent_turn_adapter` 同口径）。
async fn resolve_routine_provider(
    harness: &RuntimeHarness,
    bot_provider_id: Option<&str>,
    bot_model_id: Option<&str>,
) -> Result<
    (
        axagent_harness::types::ProviderConfig,
        Arc<dyn ProviderAdapter>,
        ProviderRequestContext,
        String,
    ),
    String,
> {
    let db = harness.db();
    let providers =
        axagent_dao::repo::provider::list_providers(db).await.map_err(|e| e.to_string())?;

    let prov = match bot_provider_id {
        Some(pid) => providers
            .into_iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| format!("Bot 指定的 provider 不存在或已禁用: {pid}"))?,
        None => providers
            .into_iter()
            .find(|p| p.enabled && p.keys.iter().any(|k| k.enabled))
            .ok_or_else(|| "没有启用的模型提供商".to_string())?,
    };

    let key = prov
        .keys
        .iter()
        .find(|k| k.enabled)
        .ok_or_else(|| format!("provider '{}' 没有可用的 API key", prov.id))?;
    let api_key = axagent_crypto::decrypt_key(&key.key_encrypted, harness.master_key())
        .map_err(|e| e.to_string())?;

    let settings = axagent_dao::repo::settings::get_settings(db).await.unwrap_or_default();
    let ctx = ProviderRequestContext {
        header_timeout_secs: None,
        api_key,
        key_id: key.id.clone(),
        provider_id: prov.id.clone(),
        base_url: Some(resolve_base_url_for_type(&prov.api_host, &prov.provider_type)),
        api_path: prov.api_path.clone(),
        proxy_config: resolve_provider_proxy(&prov.proxy_config, &settings),
        custom_headers: prov.custom_headers.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };

    // 经 RuntimeHarness 取 adapter（与 agent_query / agent_turn_adapter 同一入口，
    // tool_adaptation 包裹语义一致）。
    let adapter: Arc<dyn ProviderAdapter> = harness
        .get_adapter_for_provider(&prov)
        .await
        .ok_or_else(|| format!("无适配器可用: {:?}", prov.provider_type))?;

    let model = match bot_model_id {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => prov.models.first().map(|m| m.model_id.clone()).unwrap_or_default(),
    };

    Ok((prov, adapter, ctx, model))
}

/// 轮次类型：例程/根轮恒保留委派工具；assignee 轮按深度放行（ADR-P3-2 ①）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKind {
    Routine,
    Assignee,
}

/// 每轮独立工具表（独立实例避免与主 registry 的 tokio Mutex 死锁）。
/// `Agent`/`RemoteTrigger` 恒禁；`delegate_to_bot` 在 depth<3 的 assignee 轮出示。
pub fn build_turn_tool_registry(round_kind: TurnKind, depth: u8) -> UnifiedToolRegistry {
    let mut registry = UnifiedToolRegistry::new();
    registry.init_all();
    for name in registry.list_tools() {
        let hide_delegate = round_kind == TurnKind::Assignee && depth >= MAX_DELEGATE_DEPTH;
        if matches!(name.as_str(), "Agent" | "RemoteTrigger")
            || (hide_delegate && name == "delegate_to_bot")
        {
            registry.disable_tool(&name);
        }
    }
    registry
}

/// 跑一轮 Bot 例程（非交互）。
///
/// 这是 P2-a 原有函数的薄包装：不注入委派任务卡、不改工具表 —— 行为与
/// P2-a 逐字节一致（既有 `bot_routine` 测试锁旧行为）。委派执行器走
/// [`run_bot_non_interactive_turn`] 并显式传任务卡。
pub async fn run_bot_routine_turn(
    ctx: &BotRoutineContext,
    bot_id: &str,
    bot_approval_mode: &str,
    bot_provider_id: Option<&str>,
    bot_model_id: Option<&str>,
    prompt: &str,
) -> Result<String, String> {
    run_bot_non_interactive_turn_with_usage(
        ctx,
        bot_id,
        bot_approval_mode,
        bot_provider_id,
        bot_model_id,
        prompt,
        None,
        DelegateContext::default(),
    )
    .await
    .map(|o| o.text)
}

/// 带用量回传的例程入口（P5-b B2）：行为与 `run_bot_routine_turn` 一致，
/// 仅多回传本轮 TokenUsage（ADR-P5-8）。
pub async fn run_bot_routine_turn_with_usage(
    ctx: &BotRoutineContext,
    bot_id: &str,
    bot_approval_mode: &str,
    bot_provider_id: Option<&str>,
    bot_model_id: Option<&str>,
    prompt: &str,
) -> Result<BotTurnOutcome, String> {
    run_bot_non_interactive_turn_with_usage(
        ctx,
        bot_id,
        bot_approval_mode,
        bot_provider_id,
        bot_model_id,
        prompt,
        None,
        DelegateContext::default(),
    )
    .await
}

/// 非交互 Bot 轮的唯一实现（例程 P2-a + 委派执行器 P2-b 共用，ADR-P2-2）。
///
/// `task_card` 非空时注入 `<bot-delegation-task>` 段（任务正文 + requester 身份 +
/// 结果回传边界），不注入 requester 的 SOUL/MEMORY。例程入口传 `None`，
/// 系统提示词与旧实现逐字一致。
pub async fn run_bot_non_interactive_turn(
    ctx: &BotRoutineContext,
    bot_id: &str,
    bot_approval_mode: &str,
    bot_provider_id: Option<&str>,
    bot_model_id: Option<&str>,
    prompt: &str,
    task_card: Option<&str>,
) -> Result<String, String> {
    let chain = DELEGATE_CONTEXT.try_with(|v| v.clone()).unwrap_or_default();
    run_bot_non_interactive_turn_with_context(
        ctx,
        bot_id,
        bot_approval_mode,
        bot_provider_id,
        bot_model_id,
        prompt,
        task_card,
        chain,
    )
    .await
}

/// 带链上下文的非交互轮（P3）。内部链字段经 task-local 在工具调用期注入，
/// 不进工具 JSON schema；例程入口仍走上面的无上下文包装，行为与 P2-a 一致。
#[allow(clippy::too_many_arguments)]
pub async fn run_bot_non_interactive_turn_with_context(
    ctx: &BotRoutineContext,
    bot_id: &str,
    bot_approval_mode: &str,
    bot_provider_id: Option<&str>,
    bot_model_id: Option<&str>,
    prompt: &str,
    task_card: Option<&str>,
    chain: DelegateContext,
) -> Result<String, String> {
    run_bot_non_interactive_turn_with_usage(
        ctx,
        bot_id,
        bot_approval_mode,
        bot_provider_id,
        bot_model_id,
        prompt,
        task_card,
        chain,
    )
    .await
    .map(|o| o.text)
}

/// P5-b B2：`with_context` 的用量回传变体，唯一改动既有执行路径（ADR-P5-8）。
/// 除了多回传 `BotTurnOutcome.usage`，行为与 `run_bot_non_interactive_turn_with_context` 逐字节一致。
#[allow(clippy::too_many_arguments)]
pub async fn run_bot_non_interactive_turn_with_usage(
    ctx: &BotRoutineContext,
    bot_id: &str,
    bot_approval_mode: &str,
    bot_provider_id: Option<&str>,
    bot_model_id: Option<&str>,
    prompt: &str,
    task_card: Option<&str>,
    chain: DelegateContext,
) -> Result<BotTurnOutcome, String> {
    // 存在性校验（executor 已验过，此处兜底：Bot 被并发删除时不产生幽灵轮）。
    axagent_dao::repo::bot::get_bot(ctx.harness.db(), bot_id).await.map_err(|e| e.to_string())?;

    // ADR-P2-1 第 2 条：非交互源只取 Bot 档（custom -> Prompt 由映射表兜底）。
    let permission_mode = crate::bot_approval::resolve_bot_routine_level(bot_approval_mode);

    let mut system_prompt = build_bot_system_prompt(&ctx.harness, bot_id).await;
    if let Some(card) = task_card.filter(|c| !c.trim().is_empty()) {
        system_prompt
            .push_str(&format!("\n\n<bot-delegation-task>\n{card}\n</bot-delegation-task>"));
    }
    let (prov, adapter, mut req_ctx, model) =
        resolve_routine_provider(&ctx.harness, bot_provider_id, bot_model_id).await?;

    // 工具装配：例程轮恒保留；assignee 轮按 task-local 深度放行（depth=3 禁用）。
    let round_kind = if task_card.is_some() {
        TurnKind::Assignee
    } else {
        TurnKind::Routine
    };
    let mut tool_registry = build_turn_tool_registry(round_kind, chain.depth);
    // P5 C1-2：Bot 会话把 bot_id 注入 tool extra，SkillIndex/SkillView 按此过滤可见性。
    tool_registry =
        tool_registry.with_tool_extra(axagent_tools::context_keys::BOT_ID, bot_id);

    // 每轮新建的临时会话：execution_id 兼作 conversation_id（headless 先例同构）。
    let execution_id = format!("bot-routine-{}", uuid::Uuid::new_v4());
    let session = ctx
        .session_manager
        .get_or_create_session(prov.id.clone(), execution_id.clone())
        .await
        .map_err(|e| format!("创建例程会话失败: {e}"))?;
    let session_id = session.session().session_id.clone();
    req_ctx.conversation = Some(session_id.clone());

    let api_client: Box<dyn ApiClient + Send> =
        Box::new(AxAgentApiClient::new(adapter.clone(), req_ctx.clone()).with_model(model.clone()));
    let tool_executor: Box<dyn axagent_harness::runtime_types::conversation::ToolExecutor + Send> =
        Box::new(tool_registry);

    let runtime = create_conversation_runtime(ConversationRuntimeFactoryArgs::new(
        session.session().clone(),
        api_client,
        tool_executor,
        PermissionPolicy::new(permission_mode),
        vec![system_prompt],
        RuntimeFeatureConfig::default(),
    ));

    let cancel_token = Arc::new(AtomicBool::new(false));
    // prompter = None（空表）：无人在场的轮 ask 类操作按权限策略自动 Deny（ADR-P2-4）。
    let prompters: Arc<Mutex<HashMap<String, ChannelPermissionPrompter>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (summary, _updated) = with_delegate_context(
        chain.clone(),
        ctx.session_manager.run_turn_with_tools(
            &session_id,
            prompt.to_string(),
            runtime,
            execution_id.clone(),
            Some(cancel_token),
            prompters,
        ),
    )
    .await
    .map_err(|e| e.to_string())?;

    // 占位会话即时清理（与 agent_turn_adapter 同一防堆积语义）。
    if let Err(e) = axagent_dao::repo::conversation::delete_placeholder_conversation(
        ctx.harness.db(),
        &execution_id,
    )
    .await
    {
        tracing::warn!(execution_id = %execution_id, "清理例程占位会话失败（不阻塞）: {e}");
    }

    let mut content = String::new();
    for msg in &summary.assistant_messages {
        for block in &msg.blocks {
            if let axagent_harness::conversation_model::ContentBlock::Text { text } = block {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(text.as_str());
            }
        }
    }
    Ok(BotTurnOutcome {
        text: content,
        usage: Some(BotTurnUsage {
            model_id: model,
            execution_id,
            usage: summary.usage,
        }),
    })
}

/// 例程轮结果的统一收尾：`record_run` + 一次性置 Disabled（G-7 三态沿用）。
pub async fn finish_bot_routine(
    store: &Arc<axagent_runtime_core::CronJobStore>,
    job_id: &str,
    recurring: bool,
    started: i64,
    outcome: Result<String, String>,
) {
    let result = axagent_runtime_core::TaskRunResult {
        success: outcome.is_ok(),
        output: outcome.as_ref().ok().cloned(),
        error: outcome.as_ref().err().cloned(),
        duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
        executed_at: started,
        skipped: false,
        partial: false,
    };
    tracing::info!(
        "[CronScheduler] Bot 例程 '{job_id}' 完成: success={} duration={}ms",
        result.success,
        result.duration_ms
    );
    store.record_run(job_id, result).await;
    if !recurring && !store.set_status(job_id, axagent_runtime_core::CronJobStatus::Disabled).await
    {
        tracing::warn!(
            "[CronScheduler] 一次性 Bot 例程 '{job_id}' 置为 Disabled 失败\
             （未找到该任务或持久化失败，下次到点会重复跑）"
        );
    }
}

#[cfg(test)]
mod p2_bot_routine_tests {

    // ADR-P3-2 ①：多层委派按深度放行（P2-b 一层断言自 P3 起 superseded）。
    #[test]
    fn delegate_tool_respects_depth_cap() {
        use super::TurnKind;
        let routine = super::build_turn_tool_registry(TurnKind::Routine, 1);
        assert!(
            routine.list_tools().iter().any(|n| n == "delegate_to_bot"),
            "例程/根轮恒可发起委派"
        );
        for depth in [1u8, 2] {
            let assignee = super::build_turn_tool_registry(TurnKind::Assignee, depth);
            assert!(
                assignee.list_tools().iter().any(|n| n == "delegate_to_bot"),
                "depth={depth} 应可继续委派"
            );
            assert!(
                !assignee.list_tools().iter().any(|n| n == "Agent" || n == "RemoteTrigger"),
                "P2-a 既有的防递归工具禁用不得回归"
            );
        }
        let capped = super::build_turn_tool_registry(TurnKind::Assignee, super::MAX_DELEGATE_DEPTH);
        assert!(
            !capped.list_tools().iter().any(|n| n == "delegate_to_bot"),
            "depth=3 不得再注册委派工具"
        );
    }

    // ADR-P2-4：Bot 五档映射在例程路径的落点 —— 例程函数无会话参数本身就是
    // 约束的一半（约束另一半在 bot_approval 单测）。这里锁定各档到执行档位
    // 的最终映射，防止后续重构把例程路径接到交互映射上。
    #[test]
    fn routine_level_mapping_is_bot_table_only() {
        assert_eq!(
            crate::bot_approval::resolve_bot_routine_level("ask"),
            axagent_harness::runtime_types::permissions::PermissionMode::Prompt
        );
        assert_eq!(
            crate::bot_approval::resolve_bot_routine_level("edits"),
            axagent_harness::runtime_types::permissions::PermissionMode::WorkspaceWrite
        );
        assert_eq!(
            crate::bot_approval::resolve_bot_routine_level("auto"),
            axagent_harness::runtime_types::permissions::PermissionMode::WorkspaceWrite
        );
        assert_eq!(
            crate::bot_approval::resolve_bot_routine_level("full"),
            axagent_harness::runtime_types::permissions::PermissionMode::Allow
        );
        assert_eq!(
            crate::bot_approval::resolve_bot_routine_level("custom"),
            axagent_harness::runtime_types::permissions::PermissionMode::Prompt
        );
    }

    // 一次性例程跑完置 Disabled；循环例程保持 Active（G-7 三态沿用）。
    #[tokio::test]
    async fn finish_bot_routine_disables_one_shot_only() {
        let store = std::sync::Arc::new(axagent_runtime_core::CronJobStore::new_ephemeral());
        let mut one_shot = axagent_runtime_core::CronJob::new("一次性例程", "0 9 * * *", "p", "d");
        let recurring_job = axagent_runtime_core::CronJob::new("循环例程", "0 9 * * *", "p", "d");
        one_shot.recurring = false;
        assert!(recurring_job.recurring);
        store.add(one_shot.clone()).await;
        store.add(recurring_job.clone()).await;

        let started = axagent_runtime_core::cron_job::now_millis();

        // 成功的一次性例程：置 Disabled，历史落 output。
        crate::bot_routine_runner::finish_bot_routine(
            &store,
            &one_shot.id,
            one_shot.recurring,
            started,
            Ok("每日汇报正文".to_string()),
        )
        .await;
        let job = store.get(&one_shot.id).await.expect("job 仍在");
        assert_eq!(job.status, axagent_runtime_core::CronJobStatus::Disabled);
        assert_eq!(job.run_count, 1);
        let res = job.last_result.expect("成功轮落 last_result");
        assert!(res.success);
        assert_eq!(res.output.as_deref(), Some("每日汇报正文"));

        // 失败的循环例程：保持 Active（下轮继续），历史落 error。
        crate::bot_routine_runner::finish_bot_routine(
            &store,
            &recurring_job.id,
            recurring_job.recurring,
            started,
            Err("provider 不可用".to_string()),
        )
        .await;
        let job = store.get(&recurring_job.id).await.expect("job 仍在");
        assert_eq!(job.status, axagent_runtime_core::CronJobStatus::Active);
        assert_eq!(job.run_count, 1);
        let res = job.last_result.expect("失败轮也落 last_result");
        assert!(!res.success);
        assert_eq!(res.error.as_deref(), Some("provider 不可用"));
    }
}
