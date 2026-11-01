// SPDX-License-Identifier: AGPL-3.0-only
// 两份能力/L1 清点脚本的**唯一**共享口径：命令注册表解析 + 功能域归并规则。
// 存在理由：AGENTS.md 禁区 12「禁止重复定义」—— 同一指标只能有一套解析规则，
// 否则 `scripts/audit-capability-inventory.mjs` 与 `scripts/audit-l1-blockers.mjs` 会各报各的命令数。
// ⚠ 域归并是**命名级**的（正则匹配模块名前缀），命名不规范者落「其它」；它是审计口径，不是插件边界口径。

export const REG_PATH = 'src-tauri/src/register_commands.rs';

/** 逐行解析 build.rs 生成的 `tauri::generate_handler![...]` 清单。 */
export function parseRegistry(text) {
  const entries = [];
  for (const l of text.split('\n')) {
    const m = l.trim().match(/^([a-z_0-9]+(?:::[a-z_0-9]+)*)::([a-z_0-9]+),?$/);
    if (!m) continue;
    const parts = m[1].split('::');
    entries.push({
      mod: parts[0] === 'commands' ? parts[1] || 'root' : parts.join('::'),
      cmd: m[2],
    });
  }
  return entries;
}

export const DOMAIN_RULES = [
  ['工作流引擎与编辑器', /^workflows?$|^workflow_|^work_engine|^wf_des/],
  ['股票分析与管线', /^stock_|^analyst|^recommendation|^mover_|^universe|^pool_scan|^market_(close|mainline)|^demand|^narrative|^price_alert|^conditional_order|^paper|^portfolio|^trade_intent/],
  ['量化与仿真', /^quant_|^backtest|^market_sim|^paper_portfolio/],
  ['OPC 公司运行时（口径已废，盘上未收口）', /^opc/],
  ['智能体引擎与编排', /^agent|^sub_agent|^multi_agent|^orchestrator|^fleet|^cognitive|^smart_router|^plan$|^personality|^profile$|^user_profile|^nudge|^proactive|^reminder|^agency_expert|^decision_gates/],
  ['任务调度与后台执行', /^scheduler|^background_tasks|^parallel_execution/],
  ['上下文与提示词工程', /^context_breakdown|^context_sources|^prompt_templates|^system_prompt|^memory_context/],
  ['对话与消息', /^conversation|^message|^chat|^session|^quickbar|^compress|^branches|^time_travel/],
  ['模型提供方', /^provider|^llm_|^model|^local_model/],
  ['知识库 / RAG / Wiki', /^knowledge|^unified_knowledge|^llm_wiki|^wiki|^retrieval|^sources$|^ingest|^index_jobs|^dynamic_ui|^research/],
  ['技能与工具', /^skill|^generated_tool|^tool_|^local_tool|^marketplace|^capability|^dojo|^sandbox|^approval_rules|^hooks/],
  ['MCP 与插件体系', /^mcp|^plugin|^platform_integration/],
  ['网关与互联', /^gateway/],
  ['通知与定时', /^notification|^scheduled|^cron|^nl_to_cron|^webhook|^reminder/],
  ['终端 / 桌面控制 / 多媒体', /^terminal|^pty|^desktop|^device|^screenshot|^screen_vision|^computer_control|^browser|^image_gen|^chart_generator|^document_export|^voice|^tts/],
  ['数据与文件', /^files?|^file_|^storage|^backup|^db_config|^migration|^artifacts|^sources/],
  ['轨迹 / 反思 / 进化 / RL', /^evolution|^dependency_metrics|^reflection|^trajectory|^rl_|^fine_tune|^evaluator|^tracer|^benchmark|^learning|^dream|^analyst_/],
  ['设置 / 安全 / 应用外壳', /^settings|^app_config|^master_key|^credential|^security|^error|^health|^onboarding|^help|^theme|^locale|^crash|^crdt|^search$|^webdav|^cloud_workspace|^memory|^task|^tray/],
  ['仪表盘与动态 UI', /^dashboard|^dynamic/],
];

export function domainOf(mod) {
  for (const [d, rx] of DOMAIN_RULES) if (rx.test(mod)) return d;
  return '其它（未归类，需人工）';
}

// 「其它」桶在 2026-11-01 已被人工认领归零；用它当回归哨兵：非空说明模块命名或规则漂移了。
export function unassigned(entries) {
  const mods = new Set(entries.map((e) => e.mod));
  return [...mods].filter((m) => domainOf(m) === '其它（未归类，需人工）');
}
