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

// ── 智能体工具面（`#[agent_command(...)]` 属性标注）扫描口径 ────────────────
// 存在理由：`#[agent_command]` 的注册走 `inventory::submit!`（axagent-agent-macro），工具名由宏从
//   Rust fn 名推导，源码里没有任何字符串字面量 ⇒ 「grep 命令名找不到消费方」不能证明无人使用。
//   命令注册面（parseRegistry）之外必须有这组读数，否则删一批标注门一声不响。
// ⚠ 检测式里的标注名一律用字符串**拼接**构造，不许写成完整的字面量 —— 本判据扫的是源码文本，
//   将来扫描面若从 `.rs` 扩到脚本自身（`.mjs`/`.json`/PLAN），留在实现里的完整字面量会把自己
//   数成一次命中（同仓 Rust 侧踩过的坑：计数型钉桩要用 `concat!` 排除自身，此处是同构问题）。
const AC_ATTR_HEAD = '#' + '[' + 'agent' + '_' + 'command';
// 行首（允许缩进）必须紧跟 `#[agent_command` 才算属性标注。这一条同时排除三类假命中：
//   ① `//` / `///` / `//!` 注释与文档示例（如 axagent-agent-macro/src/lib.rs 的示例行）；
//   ② 字符串字面量里的提及（如 generate_command_docs.rs 的 println!/push_str）；
//   ③ 裸词 `agent_command`（宏定义本身 `pub fn agent_command(` 行首不是 `#[`）。
const AC_LINE_HEAD_RE = new RegExp('^\\s*' + AC_ATTR_HEAD.replace("[", "\\["));
const AC_DOMAIN_QUOTED_RE = /domain\s*=\s*"([^"]+)"/;
const AC_DOMAIN_BARE_RE = /domain\s*=\s*([A-Za-z_][A-Za-z0-9_:]*)/;
// 仓内真实形态两种：`domain = "finance"`（带引号）与 `domain = market_sim`（不带引号），都要认；
// 未声明 domain 的标注归入该桶名（当前实测应为 0 条，若出现说明宏默认值被消费了）。
export const AC_NO_DOMAIN_BUCKET = '(未声明 domain)';

/** 从标注头命中的下标起做**括号配平**（跨行；跳过字符串字面量与行注释），返回属性体文本或 null。 */
function extractAttrBody(text, headIdx) {
  const lineEnd = text.indexOf('\n', headIdx);
  const openIdx = text.indexOf('(', headIdx);
  let i = openIdx;
  // 裸 `#[agent_command]` 无参数体：不存在 domain，不计入本口径。
  // ⚠ 开括号**必须落在标注行内**：若只判 indexOf 是否 >=0，无参数的标注行会跨过换行去吞**后面某个
  //   无关左括号**（下一个命令的属性、乃至函数体的 `(`），凭空造出一条不存在的读数。当前树实测
  //   0 条裸标注 ⇒ 本分支不改现值，改的是「将来出现裸标注时门不会静默虚高」。
  if (openIdx < 0 || (lineEnd >= 0 && openIdx > lineEnd)) { return null; }
  let depth = 0;
  let inStr = false;
  for (; i < text.length; i++) {
    const c = text[i];
    if (inStr) {
      if (c === '\\') { i++; } else if (c === '"') { inStr = false; }
      continue;
    }
    if (c === '"') { inStr = true; continue; }
    if (c === '/' && text[i + 1] === '/') { const nl = text.indexOf('\n', i); i = nl < 0 ? text.length : nl; continue; }
    if (c === '(') { depth++; } else if (c === ')') { depth--; if (depth === 0) { return text.slice(openIdx + 1, i); } }
  }
  return null; // 括号不闭合（理论上不会发生）⇒ 宁可不计数，不许数错
}

/**
 * 智能体工具面读数（纯函数，零盘 IO；与 check-capability-register.mjs 的 --selftest 共用同一实现）。
 * @param {Iterable<[string, string]>} sources —— `[仓库相对路径(正斜杠), 文件全文]`，调用方负责只喂 `.rs`。
 * @returns {{ total:number, host:number, nonHost:number, domainCount:number, byDomain:Object<string,number>,
 *            byRoot:Object<string,number>, roots:Object<string,{total:number,byDomain:Object}> }}
 *   宿主 = 根段 `src-tauri`；其余每个顶层根单独计（`axagent-core` / `axagent-runtime` / …）；
 *   插件根 `axagent-plugins` 当前应为 0 —— 这个 0 就是要钉住的锚，插件侧真接入时必须能显式看到变化。
 */
export function scanAgentCommandFace(sources) {
  const byDomain = {};
  const roots = {};
  const add = (root, dom) => {
    roots[root] ??= { total: 0, byDomain: {} };
    roots[root].total++;
    roots[root].byDomain[dom] = (roots[root].byDomain[dom] || 0) + 1;
    byDomain[dom] = (byDomain[dom] || 0) + 1;
  };
  for (const [rel, text] of sources) {
    const root = rel.split('/')[0];
    const lines = text.split('\n');
    // 逐行找「行首即标注」的头，再从该处对**全文**做括号配平（跨行属性照常闭合）。
    let offset = 0;
    for (const line of lines) {
      if (AC_LINE_HEAD_RE.test(line)) {
        const body = extractAttrBody(text, offset);
        if (body !== null) {
          const flat = body.replace(/\s+/g, ' ');
          const dm = AC_DOMAIN_QUOTED_RE.exec(flat) ?? AC_DOMAIN_BARE_RE.exec(flat);
          add(root, dm ? dm[1] : AC_NO_DOMAIN_BUCKET);
        }
      }
      offset += line.length + 1;
    }
  }
  let total = 0;
  let host = 0;
  const byRoot = {};
  for (const [r, v] of Object.entries(roots)) {
    byRoot[r] = v.total;
    total += v.total;
    if (r === 'src-tauri') { host += v.total; }
  }
  return { total, host, nonHost: total - host, domainCount: Object.keys(byDomain).length, byDomain, byRoot, roots };
}
