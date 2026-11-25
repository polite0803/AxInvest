// SPDX-License-Identifier: AGPL-3.0-only
/**
 * Bot P1 IPC 契约审计（供 `scripts/check-contracts.mjs --only=bot` 调用）。
 *
 * ## 为什么单独一段、且带 selftest
 *
 * `contracts` 的 A 段只检查「`#[tauri::command]` 定义 ⊆ generate_handler!」——
 * 它能抓到「漏注册」，但抓不到「命令改名」「参数改名」「错误码漏语言」「DTO 少了
 * `rename_all`」「前端传了 snake_case 参数」。这五类都不会让 typecheck / 单测变红，
 * 只会在运行时表现为「前端 invoke 404」「字段永远是 null」。本段把
 * `docs/bot-integration/IPC.md` 的命令表变成**可执行清单**。
 *
 * 另一个必须解决的假绿：仓库现在**还没有** `bot.rs`。如果检查器「文件不存在就跳过」，
 * 它会永远绿。所以缺文件 = 红，且 selftest 用合成源码证明每种破坏都会红。
 *
 * 判据来源：`docs/bot-integration/IPC.md`（命令/参数/返回/错误码）、`PLAN.md` T3（前端类型）。
 */

import { decorateWithP3 } from "./bot-ipc-audit-p3-fixture.mjs";
import { decorateWithP4 } from "./bot-ipc-audit-p4-fixture.mjs";
import { decorateWithP5 } from "./bot-ipc-audit-p5-fixture.mjs";

/** P1 契约清单。顺序与 IPC.md §1 一致；任何字段变更都应先改文档再改这里。 */
export const BOT_IPC = {
  commands: [
    {
      name: "list_bots",
      params: [{ name: "include_disabled", type: "Option<bool>" }],
      returns: "Result<Vec<Bot>, String>",
    },
    {
      name: "get_bot",
      params: [{ name: "bot_id", type: "String" }],
      returns: "Result<Bot, String>",
    },
    {
      name: "create_bot",
      params: [{ name: "input", type: "CreateBotInput" }],
      returns: "Result<Bot, String>",
    },
    {
      name: "update_bot",
      params: [
        { name: "bot_id", type: "String" },
        { name: "input", type: "UpdateBotInput" },
      ],
      returns: "Result<Bot, String>",
    },
    {
      name: "delete_bot",
      params: [{ name: "bot_id", type: "String" }],
      returns: "Result<DeleteBotResult, String>",
    },
    {
      name: "list_bot_threads",
      params: [
        { name: "bot_id", type: "String" },
        { name: "include_archived", type: "Option<bool>" },
      ],
      returns: "Result<Vec<Conversation>, String>",
    },
    {
      name: "read_bot_soul",
      params: [{ name: "bot_id", type: "String" }],
      returns: "Result<BotSoul, String>",
    },
    {
      name: "write_bot_soul",
      params: [
        { name: "bot_id", type: "String" },
        { name: "content", type: "String" },
      ],
      returns: "Result<BotSoul, String>",
    },
    {
      name: "read_bot_memory",
      params: [{ name: "bot_id", type: "String" }],
      returns: "Result<BotMemory, String>",
    },
  ],
  structs: [
    {
      name: "Bot",
      derive: "Serialize, Deserialize",
      fields: [
        ["id", "String"],
        ["name", "String"],
        ["title", "Option<String>"],
        ["description", "Option<String>"],
        ["soul", "String"],
        ["soul_hash", "String"],
        ["soul_drift", "bool"],
        ["icon", "String"],
        ["color", "Option<String>"],
        ["section", "String"],
        ["model_id", "Option<String>"],
        ["provider_id", "Option<String>"],
        ["effort", "Option<String>"],
        ["workdir", "Option<String>"],
        ["approval_mode", "String"],
        ["agent_profile_id", "Option<String>"],
        ["is_enabled", "bool"],
        ["sort_order", "i32"],
        ["created_at", "i64"],
        ["updated_at", "i64"],
      ],
    },
    {
      name: "CreateBotInput",
      // 权威定义在 harness：`CreateBot`（命令层 `pub use ... as CreateBotInput`）。
      aliases: ["CreateBot"],
      derive: "Deserialize",
      fields: [
        ["name", "String"],
        ["title", "Option<String>"],
        ["description", "Option<String>"],
        ["soul", "Option<String>"],
        ["icon", "Option<String>"],
        ["color", "Option<String>"],
        ["section", "Option<String>"],
        ["model_id", "Option<String>"],
        ["provider_id", "Option<String>"],
        ["effort", "Option<String>"],
        ["workdir", "Option<String>"],
        ["approval_mode", "Option<String>"],
        ["agent_profile_id", "Option<String>"],
      ],
    },
    {
      name: "UpdateBotInput",
      // 权威定义在 harness：`UpdateBotPatch`（命令层别名为 `UpdateBotInput`）。
      aliases: ["UpdateBotPatch"],
      derive: "Deserialize",
      fields: [
        ["name", "Option<String>"],
        ["title", "Option<Option<String>>"],
        ["description", "Option<Option<String>>"],
        ["soul", "Option<String>"],
        ["icon", "Option<String>"],
        ["color", "Option<Option<String>>"],
        ["section", "Option<String>"],
        ["model_id", "Option<Option<String>>"],
        ["provider_id", "Option<Option<String>>"],
        ["effort", "Option<Option<String>>"],
        ["workdir", "Option<Option<String>>"],
        ["approval_mode", "Option<String>"],
        ["agent_profile_id", "Option<Option<String>>"],
        ["is_enabled", "Option<bool>"],
        ["sort_order", "Option<i32>"],
      ],
    },
    {
      name: "BotSoul",
      derive: "Serialize",
      fields: [
        ["content", "String"],
        ["hash", "String"],
        ["drift", "bool"],
      ],
    },
    {
      name: "MemoryBudget",
      derive: "Serialize",
      fields: [
        ["max_lines", "i64"],
        ["max_bytes", "i64"],
      ],
    },
    {
      name: "MemoryTopic",
      derive: "Serialize",
      fields: [
        ["relative_path", "String"],
        ["category", "String"],
        ["title", "String"],
        ["updated_at", "i64"],
      ],
    },
    {
      name: "BotMemory",
      derive: "Serialize",
      fields: [
        ["index_content", "String"],
        ["line_count", "i64"],
        ["byte_count", "i64"],
        ["budget", "MemoryBudget"],
        ["topics", "Vec<MemoryTopic>"],
      ],
    },
    {
      name: "DeleteBotResult",
      derive: "Serialize",
      fields: [
        ["detached_threads", "i64"],
        ["memory_path", "String"],
      ],
    },
    // ── P5（IPC-P5 §2/§5）──
    {
      name: "BotCostQuota",
      aliases: ["BotCostQuotaDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["scope", "String"],
        ["scope_id", "String"],
        ["max_total_tokens", "i64"],
        ["max_cost_usd", "f64"],
        ["window_hours", "i32"],
        ["is_default", "bool"],
        ["updated_at", "Option<i64>"],
      ],
    },
    {
      name: "BotCostQuotaInput",
      aliases: ["BotCostQuotaInputDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["scope", "String"],
        ["scope_id", "String"],
        ["max_total_tokens", "i64"],
        ["max_cost_usd", "f64"],
        ["window_hours", "i32"],
      ],
    },
    {
      name: "BotUsageEntry",
      aliases: ["BotUsageEntryDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["id", "String"],
        ["bot_id", "String"],
        ["department_id", "Option<String>"],
        ["conversation_id", "Option<String>"],
        ["execution_id", "Option<String>"],
        ["workflow_run_id", "Option<String>"],
        ["assignment_id", "Option<String>"],
        ["model_id", "String"],
        ["origin", "String"],
        ["input_tokens", "i64"],
        ["output_tokens", "i64"],
        ["cache_creation_tokens", "i64"],
        ["cache_read_tokens", "i64"],
        ["total_tokens", "i64"],
        ["estimated_cost_usd", "f64"],
        ["pricing_source", "String"],
        ["created_at", "i64"],
      ],
    },
    {
      name: "BotUsageFilter",
      aliases: ["BotUsageFilterDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["bot_id", "Option<String>"],
        ["department_id", "Option<String>"],
        ["origin", "Option<String>"],
        ["workflow_run_id", "Option<String>"],
        ["assignment_id", "Option<String>"],
        ["since", "Option<i64>"],
        ["until", "Option<i64>"],
      ],
    },
    {
      name: "BotUsageSummary",
      aliases: ["BotUsageSummaryDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["scope", "String"],
        ["scope_id", "String"],
        ["window_hours", "i32"],
        ["input_tokens", "i64"],
        ["output_tokens", "i64"],
        ["cache_creation_tokens", "i64"],
        ["cache_read_tokens", "i64"],
        ["total_tokens", "i64"],
        ["estimated_cost_usd", "f64"],
        ["entry_count", "i64"],
        ["breached", "bool"],
        ["pricing_note", "String"],
      ],
    },
    {
      name: "BotSkill",
      aliases: ["BotSkillDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["name", "String"],
        ["description", "String"],
        ["source", "String"],
        ["enabled", "bool"],
        ["shadowed_global", "bool"],
        ["missing", "bool"],
        ["preset_id", "Option<String>"],
        ["updated_at", "Option<i64>"],
      ],
    },
    {
      name: "BotSkillPreset",
      aliases: ["BotSkillPresetDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["version", "String"],
        ["name", "String"],
        ["description", "String"],
        ["skills", "Vec<BotSkillPresetItem>"],
      ],
    },
    {
      name: "BotSkillPresetItem",
      aliases: ["BotSkillPresetItemDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["name", "String"],
        ["description", "String"],
        ["content", "String"],
        ["category", "Option<String>"],
        ["tags", "Option<Vec<String>>"],
        ["requires_toolsets", "Option<Vec<String>>"],
      ],
    },
    {
      name: "BotSkillPresetSkipped",
      aliases: ["BotSkillPresetSkippedDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["name", "String"],
        ["reason", "String"],
        ["detail", "String"],
      ],
    },
    {
      name: "BotSkillPresetResult",
      aliases: ["BotSkillPresetResultDto"],
      derive: "Serialize, Deserialize",
      fields: [
        ["preset_id", "String"],
        ["imported", "Vec<String>"],
        ["skipped", "Vec<BotSkillPresetSkipped>"],
      ],
    },
  ],
  /** IPC.md §5 + IPC-P2 §3 + IPC-P2B §4 + IPC-P3 §4：31 个错误码，必须逐字出现在 error_code.rs 的 `pub mod bot` 中。 */
  errorCodes: [
    "BOT_NOT_FOUND",
    "BOT_NAME_INVALID",
    "BOT_FIELD_TOO_LONG",
    "BOT_SOUL_TOO_LARGE",
    "BOT_APPROVAL_MODE_INVALID",
    "BOT_MODEL_PAIR_INVALID",
    "BOT_WORKDIR_INVALID",
    "BOT_ICON_INVALID",
    "BOT_DELETE_FAILED",
    "BOT_MEMORY_READ_FAILED",
    "BOT_SOUL_WRITE_FAILED",
    // P2-a（IPC-P2 §3）：Bot 例程错误码 ×2
    "BOT_SCHEDULE_CRON_INVALID",
    "BOT_SCHEDULE_TIMEZONE_INVALID",
    // P2-b（IPC-P2B §4）：部门错误码 ×2（T2''）；委派错误码 ×4 由 T4'' 追加
    "BOT_DEPARTMENT_NOT_FOUND",
    "BOT_DEPARTMENT_MEMBER_EXISTS",
    "BOT_ASSIGNMENT_NOT_FOUND",
    "BOT_DELEGATE_SELF",
    "BOT_DELEGATE_BUSY",
    "BOT_DELEGATE_TIMEOUT",
    // P3（IPC-P3 §4）：编排 10 个 + 多层委派 2 个
    "BOT_DELEGATE_CYCLE",
    "BOT_DELEGATE_MAX_DEPTH",
    "BOT_WORKFLOW_NOT_FOUND",
    "BOT_WORKFLOW_STEP_INVALID",
    "BOT_WORKFLOW_CYCLE",
    "BOT_WORKFLOW_DEPARTMENT_REQUIRED",
    "BOT_WORKFLOW_MEMBER_CHANGED",
    "BOT_WORKFLOW_RUN_NOT_FOUND",
    "BOT_WORKFLOW_RUN_BUSY",
    "BOT_WORKFLOW_STEP_BOT_MISSING",
    "BOT_WORKFLOW_TIMEOUT",
    "BOT_WORKFLOW_CANCEL_INVALID",
    // P4（IPC-P4 §4）：配额错误码 ×2（待答错误码 ×3 由 B3 追加）
    "BOT_QUOTA_INVALID",
    "BOT_QUOTA_EXCEEDED",
    // P4-b（IPC-P4 §4）：待答请求错误码 ×3
    "BOT_INPUT_REQUEST_NOT_FOUND",
    "BOT_INPUT_REQUEST_NOT_PENDING",
    "BOT_INPUT_ANSWER_EMPTY",
    // P5（IPC-P5 §4）：费用配额 ×2 + 技能归属 ×2
    "BOT_COST_QUOTA_INVALID",
    "BOT_COST_QUOTA_EXCEEDED",
    "BOT_SKILL_NOT_FOUND",
    "BOT_SKILL_PRESET_INVALID",
  ],
  /** IPC.md §6：P1 不新增事件；P2-b / P3 冻结事件为例外（见下方白名单）。 */
  forbiddenEventPrefix: "bot-",
  /** T3：前端类型文件里的导出（缺失 ⇒ 组件只能 `any`，契约形同虚设）。 */
  frontendTypes: [
    "ApprovalMode",
    "Bot",
    "CreateBotInput",
    "UpdateBotInput",
    "BotSoul",
    "MemoryBudget",
    "MemoryTopic",
    "BotMemory",
    "DeleteBotResult",
  ],
  localeCount: 11,
};

// P5 命令在 BOT_IPC.commands 中不强制“src/** 非 mock 调用点”警告（本批后端契约先行，
// 前端页签并行接入；由 p5 段独立核对 + 前端补齐后自然满足）。这里单独登记命令清单给 p5 段。
export const BOT_P5_COMMANDS = [
  ["get_bot_cost_quota", "bot_cost"],
  ["set_bot_cost_quota", "bot_cost"],
  ["reset_bot_cost_quota", "bot_cost"],
  ["list_bot_usage", "bot_cost"],
  ["get_bot_usage_summary", "bot_cost"],
  ["list_bot_skills", "bot_skill"],
  ["set_bot_skill_enabled", "bot_skill"],
  ["import_bot_skill_preset", "bot_skill"],
];

// ── 基础解析 ─────────────────────────────────────────────────────────────

/** 去掉 Rust 注释但保留换行，避免行号/邻接判断失真。 */
function stripRustComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/\/\/[^\n]*/g, (m) => m.replace(/[^\n]/g, " "));
}

/** 去掉 TS 注释但保留换行。 */
function stripTsComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/(^|[^:])\/\/[^\n]*/g, (m, p1) => p1 + " ".repeat(m.length - p1.length));
}

function matchBracket(src, openIndex) {
  const open = src[openIndex];
  const close = open === "(" ? ")" : open === "{" ? "}" : open === "[" ? "]" : "";
  if (!close) {
    return -1;
  }
  let depth = 0;
  for (let i = openIndex; i < src.length; i += 1) {
    if (src[i] === open) {
      depth += 1;
    } else if (src[i] === close) {
      depth -= 1;
      if (depth === 0) {
        return i;
      }
    }
  }
  return -1;
}

function splitTopLevel(text, sep = ",") {
  const out = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if ("(<[{".includes(ch)) {
      depth += 1;
    } else if (")>]}".includes(ch)) {
      depth -= 1;
    } else if (ch === sep && depth === 0) {
      out.push(text.slice(start, i));
      start = i + 1;
    }
  }
  out.push(text.slice(start));
  return out.map((s) => s.trim()).filter(Boolean);
}

function normalizeType(type) {
  return String(type)
    .replace(/\s+/g, "")
    .replace(/(?:[a-z_][a-z0-9_]*::)+/g, "") // 去掉 crate 前缀
    .replace(/'_/g, "'_");
}

function parseRustParams(paramText) {
  return splitTopLevel(paramText)
    .map((raw) => {
      const withoutAttrs = raw.replace(/#\[[^\]]*\]/g, "").trim();
      if (!withoutAttrs || withoutAttrs.startsWith("&")) {
        return null;
      }
      const idx = withoutAttrs.indexOf(":");
      if (idx < 0) {
        return null;
      }
      let name = withoutAttrs.slice(0, idx).trim();
      if (name.startsWith("mut ")) {
        name = name.slice(4).trim();
      }
      return { name, type: normalizeType(withoutAttrs.slice(idx + 1)) };
    })
    .filter(Boolean);
}

function parseFns(src) {
  const clean = stripRustComments(src);
  const fns = [];
  const re = /\bfn\s+([a-z_][a-z0-9_]*)\s*\(/g;
  let m;
  while ((m = re.exec(clean)) !== null) {
    const open = clean.indexOf("(", m.index);
    const close = matchBracket(clean, open);
    if (close < 0) {
      continue;
    }
    const params = parseRustParams(clean.slice(open + 1, close));
    const after = clean.slice(close + 1);
    const arrow = /^\s*->\s*([\s\S]*?)\s*\{/.exec(after);
    const lineStart = clean.lastIndexOf("\n", m.index) + 1;
    const before = clean.slice(Math.max(0, lineStart - 500), lineStart);
    fns.push({
      name: m[1],
      params,
      returns: arrow ? arrow[1].replace(/\s+/g, " ").trim() : "",
      hasAttr: /#\[(?:tauri::)?command\]\s*$/.test(before.trimEnd()),
    });
  }
  return fns;
}

function parseStructs(src) {
  const clean = stripRustComments(src);
  const out = new Map();
  const re = /\bpub\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{/g;
  let m;
  while ((m = re.exec(clean)) !== null) {
    const open = clean.indexOf("{", m.index);
    const close = matchBracket(clean, open);
    if (close < 0) {
      continue;
    }
    const body = clean.slice(open + 1, close);
    const fields = [];
    for (const fm of body.matchAll(/\bpub\s+([a-z_][a-z0-9_]*)\s*:\s*([^,\n]+),/g)) {
      fields.push([fm[1], normalizeType(fm[2])]);
    }
    const before = clean.slice(Math.max(0, m.index - 500), m.index);
    out.set(m[1], {
      fields,
      camelCase: /#\[serde\([^\]]*rename_all\s*=\s*"camelCase"/.test(before),
      derives: (before.match(/#\[derive\(([^)]*)\)\]/g) || []).join(" "),
    });
  }
  return out;
}

function parseErrorCodeModule(src) {
  const clean = stripRustComments(src);
  const start = clean.search(/\bpub\s+mod\s+bot\s*\{/);
  if (start < 0) {
    return null;
  }
  const open = clean.indexOf("{", start);
  const close = matchBracket(clean, open);
  if (close < 0) {
    return null;
  }
  const body = clean.slice(open + 1, close);
  const codes = [];
  for (const m of body.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    codes.push({ name: m[1], value: m[2] });
  }
  return codes;
}

function extractInvokeArgKeys(src, command) {
  const clean = stripTsComments(src);
  const re = new RegExp(`\\binvoke\\s*(?:<[\\s\\S]{0,500}?>)?\\s*\\(\\s*"${command}"`, "g");
  const keys = [];
  let sawCall = false;
  let m;
  while ((m = re.exec(clean)) !== null) {
    sawCall = true;
    const open = clean.indexOf("(", m.index + m[0].indexOf("invoke"));
    const close = matchBracket(clean, open);
    if (close < 0) {
      continue;
    }
    const inner = clean.slice(open + 1, close);
    const parts = splitTopLevel(inner);
    const args = parts.slice(1).join(",").trim();
    if (!args.startsWith("{")) {
      continue; // 变量实参：无法做键名校验，但调用点已计入
    }
    const objOpen = inner.indexOf("{");
    const objClose = matchBracket(inner, objOpen);
    if (objClose < 0) {
      continue;
    }
    for (const part of splitTopLevel(inner.slice(objOpen + 1, objClose))) {
      const km = /^([A-Za-z_$][\w$]*)\s*:/.exec(part);
      if (km) {
        keys.push(km[1]);
      }
    }
  }
  return { sawCall, keys };
}

// ── 审计主函数（纯函数：真实仓库与 selftest 共用）────────────────────────

/**
 * @param {object} sources
 * @param {string|null} sources.botRs        src-tauri/src/commands/bot.rs
 * @param {string} sources.typesRs           axagent-core/crates/harness/src/types/bot.rs（DTO 权威定义）
 * @param {string} sources.modRs             src-tauri/src/commands/mod.rs
 * @param {string} sources.registerRs        src-tauri/src/register_commands.rs
 * @param {string} sources.errorCodeRs       src-tauri/src/commands/error_code.rs
 * @param {{name:string,json:object}[]} sources.locales
 * @param {string|null} sources.typesTs      src/types/bot.ts
 * @param {{path:string,src:string}[]} sources.frontendFiles
 * @param {string} sources.eventsEnum       axagent-core/crates/harness/src/event_names.rs
 * @param {string} sources.eventsTs          src/types/generated/events.ts
 */
export function auditBotIpc(sources) {
  const errors = [];
  const warnings = [];
  const spec = BOT_IPC;

  if (!sources.botRs) {
    errors.push("缺 `src-tauri/src/commands/bot.rs` —— IPC.md §2 的 9 个命令全部未实现");
  }
  if (!/(^|\n)\s*pub\s+mod\s+bot\s*;/.test(sources.modRs ?? "")) {
    errors.push("`src-tauri/src/commands/mod.rs` 缺 `pub mod bot;` —— 命令模块不会被编译");
  }

  // 1) 命令定义 / 参数 / 返回
  const fns = sources.botRs ? parseFns(sources.botRs) : [];
  const fnByName = new Map(fns.map((f) => [f.name, f]));
  for (const cmd of spec.commands) {
    const fn = fnByName.get(cmd.name);
    if (!fn) {
      errors.push(`缺命令 \`${cmd.name}\`（bot.rs 中没有 \`fn ${cmd.name}\`）`);
      continue;
    }
    if (!fn.hasAttr) {
      errors.push(`命令 \`${cmd.name}\` 缺 \`#[tauri::command]\` —— build.rs 不会注册它`);
    }
    // state / app 是 wiring 注入参数，不参与 IPC 契约参数比对
    const business = fn.params.filter((p) => !["state", "app", "app_handle"].includes(p.name));
    const expected = cmd.params.map((p) => `${p.name}:${p.type}`);
    const actual = business.map((p) => `${p.name}:${p.type}`);
    if (actual.join(",") !== expected.join(",")) {
      errors.push(
        `命令 \`${cmd.name}\` 参数不符：\n        期望 ${expected.join(", ") || "（无）"}\n`
          + `        实得 ${actual.join(", ") || "（无）"}`,
      );
    }
    const expectedRet = normalizeType(cmd.returns);
    if (fn.returns && normalizeType(fn.returns) !== expectedRet) {
      errors.push(
        `命令 \`${cmd.name}\` 返回类型不符：期望 \`${cmd.returns}\`，实得 \`${fn.returns}\``,
      );
    }
  }

  // 2) 注册（generate_handler!）
  const regBlock = (() => {
    const start = (sources.registerRs ?? "").indexOf("generate_handler![");
    if (start < 0) {
      return "";
    }
    const open = sources.registerRs.indexOf("[", start);
    const close = matchBracket(sources.registerRs, open);
    return close < 0 ? "" : sources.registerRs.slice(open + 1, close);
  })();
  for (const cmd of spec.commands) {
    const re = new RegExp(`(?:^|[\\s,:])${cmd.name}\\s*,?\\s*$`, "m");
    if (!re.test(regBlock)) {
      errors.push(`命令 \`${cmd.name}\` 未出现在 register_commands.rs 的 generate_handler! 中`);
    }
  }

  // 3) DTO 结构（字段集合 + rename_all camelCase）
  //    权威定义在 `axagent-core/crates/harness/src/types/bot.rs`；命令层只做别名。
  //    两个文件都扫，先扫 harness（别名映射按它解析）。
  const structs = new Map();
  for (const src of [sources.typesRs, sources.botRs]) {
    if (!src) {
      continue;
    }
    for (const [name, info] of parseStructs(src)) {
      if (!structs.has(name)) {
        structs.set(name, info);
      }
    }
  }
  for (const want of spec.structs) {
    const got = structs.get(want.name)
      ?? (want.aliases ?? []).map((alias) => structs.get(alias)).find(Boolean);
    if (!got) {
      const aliasNote = (want.aliases ?? []).length
        ? `（或别名 ${(want.aliases ?? []).map((a) => `\`${a}\``).join(" / ")}）`
        : "";
      errors.push(`缺 DTO \`${want.name}\`${aliasNote}（IPC.md §2）`);
      continue;
    }
    if (!got.camelCase) {
      errors.push(
        `DTO \`${want.name}\` 缺 \`#[serde(rename_all = "camelCase")]\` —— 前端会拿到 snake_case`,
      );
    }
    const gotNames = got.fields.map(([n]) => n).sort();
    const wantNames = want.fields.map(([n]) => n).sort();
    const missing = wantNames.filter((n) => !gotNames.includes(n));
    const extra = gotNames.filter((n) => !wantNames.includes(n));
    if (missing.length || extra.length) {
      errors.push(
        `DTO \`${want.name}\` 字段不符：缺 [${missing.join(", ")}] 多 [${extra.join(", ")}]`,
      );
    }
    // Option<Option<T>> 三态（IPC.md §3 末尾）
    for (const [fieldName, fieldType] of want.fields) {
      if (!fieldType.startsWith("Option<Option<")) {
        continue;
      }
      const actual = got.fields.find(([n]) => n === fieldName)?.[1];
      if (actual && !actual.startsWith("Option<Option<")) {
        errors.push(
          `\`${want.name}.${fieldName}\` 必须是 \`${fieldType}\`（null=置空 / 缺失=不改），实得 \`${actual}\``,
        );
      }
    }
  }

  // 4) 错误码（11 个，按**码值**比对）
  //    仓库约定是 `pub const NOT_FOUND: &str = "BOT_NOT_FOUND";` —— 常量名可以短，
  //    决定前后端契约的是字符串值。按名字比对会把正确实现误判成 11 个缺失。
  const codes = parseErrorCodeModule(sources.errorCodeRs ?? "");
  if (!codes) {
    errors.push("`commands/error_code.rs` 缺 `pub mod bot { … }`（IPC.md §5）");
  } else {
    const byValue = new Map(codes.map((c) => [c.value, c.name]));
    for (const want of spec.errorCodes) {
      if (!byValue.has(want)) {
        errors.push(`错误码常量缺值 \`${want}\`（error_code.rs 的 pub mod bot）`);
      }
    }
    for (const { name, value } of codes) {
      if (!spec.errorCodes.includes(value)) {
        errors.push(
          `error_code.rs 的 pub mod bot 有清单外错误码 \`${value}\`（常量 ${name}，文档未登记）`,
        );
      }
    }
  }

  // 5) 错误码 × 11 语言
  if ((sources.locales ?? []).length !== spec.localeCount) {
    errors.push(
      `locale 数量是 ${(sources.locales ?? []).length}，契约要求 ${spec.localeCount} 个语言文件`,
    );
  }
  for (const { name, json } of sources.locales ?? []) {
    for (const code of spec.errorCodes) {
      const value = json?.error?.[code];
      if (typeof value !== "string" || value.trim() === "") {
        errors.push(`locale \`${name}\` 缺 \`error.${code}\`（或为空）`);
      }
    }
  }

  // 6) 事件：P1 不新增 bot-* 事件；冻结例外 = P2-b `bot-assignment-updated` + P3 `bot-workflow-run-updated`。
  const ALLOWED_BOT_EVENTS = new Set([
    "bot-assignment-updated", // IPC-P2B §5
    "bot-workflow-run-updated", // IPC-P3 §3
    "bot-input-request-updated", // IPC-P4 §3（P4-b）
  ]);
  const ALLOWED_BOT_VARIANTS = new Set([
    "BotAssignmentUpdated", // IPC-P2B §5
    "BotWorkflowRunUpdated", // IPC-P3 §3
    "BotInputRequestUpdated", // IPC-P4 §3（P4-b）
  ]);
  const eventSources = [
    ["event_names.rs", sources.eventsEnum ?? ""],
    ["types/generated/events.ts", sources.eventsTs ?? ""],
  ];
  for (const [label, src] of eventSources) {
    const re = new RegExp(`"${spec.forbiddenEventPrefix}[a-z0-9-]*"`, "g");
    const hits = [...src.matchAll(re)].map((m) => m[0]);
    const bad = hits.find((h) => !ALLOWED_BOT_EVENTS.has(h.slice(1, -1)));
    if (bad) {
      errors.push(`${label} 出现 P1 禁止的新事件 \`${bad}\`（IPC.md §6：P1 不新增事件）`);
    }
    const variants = [...src.matchAll(/\bBot[A-Z][A-Za-z0-9]*\b/g)].map((m) => m[0]);
    const badVariant = variants.find((v) => !ALLOWED_BOT_VARIANTS.has(v));
    if (badVariant && /IpcEventName|pub\s+enum/.test(src)) {
      errors.push(`${label} 出现 Bot 事件变体 \`${badVariant}\`（P1 不新增事件）`);
    }
  }

  // 7) 前端类型
  if (!sources.typesTs) {
    errors.push("缺 `src/types/bot.ts`（IPC.md §3 的 TS 类型）");
  } else {
    for (const name of spec.frontendTypes) {
      const re = new RegExp(`export\\s+(?:interface|type)\\s+${name}\\b`);
      if (!re.test(sources.typesTs)) {
        errors.push(`src/types/bot.ts 缺 \`export ... ${name}\``);
      }
    }
  }

  // 8) 前端调用点 + 参数形态（每命令至少一个非 mock/测试调用点）
  const isMockOrTest = (p) =>
    /^src\/lib\/browserMock(?:Bots)?\.ts$/.test(p)
    || /(^|\/)(?:__tests__|__mocks__)\//.test(p)
    || /\.(?:test|spec)\.[cm]?tsx?$/.test(p);
  for (const cmd of spec.commands) {
    let callers = 0;
    for (const file of sources.frontendFiles ?? []) {
      if (isMockOrTest(file.path)) {
        continue;
      }
      const { sawCall, keys } = extractInvokeArgKeys(file.src, cmd.name);
      if (sawCall) {
        callers += 1;
      }
      for (const key of keys) {
        if (key.includes("_")) {
          errors.push(
            `前端 \`${file.path}\` 调用 \`${cmd.name}\` 用了 snake_case 参数 \`${key}\`（Tauri v2 要求 camelCase）`,
          );
        }
      }
    }
    if (callers === 0) {
      // P2 命令（如 `write_bot_soul`）与给外部集成方留的纯 API 面都可能没有 UI 调用点，
      // 这不足以判失败；但必须留痕，防止「实现了却永远走不到」悄悄变成死代码。
      warnings.push(`命令 \`${cmd.name}\` 在 src/** 无调用点（mock/测试除外）—— 声明了却没人用`);
    }
  }

  // ── P2-b 段（IPC-P2B §1/§2.2/§6）：8 新命令两步注册 + DTO 字段 + 事件名 ──
  // 仅在真实仓库检查（sources.p2b 显式给出）时执行；selftest 的合成 P1 源不触发。
  const p2b = sources.p2b;
  if (p2b) {
    const P2B_COMMANDS = [
      ["list_bot_departments", "bot_department"],
      ["create_bot_department", "bot_department"],
      ["update_bot_department", "bot_department"],
      ["delete_bot_department", "bot_department"],
      ["add_bot_to_department", "bot_department"],
      ["remove_bot_from_department", "bot_department"],
      ["create_bot_assignment", "bot_assignment"],
      ["list_bot_assignments", "bot_assignment"],
      ["cancel_bot_assignment", "bot_assignment"],
    ];
    const p2bSources = {
      bot_department: p2b.deptRs ?? "",
      bot_assignment: p2b.asmtRs ?? "",
    };
    for (const [name, mod] of P2B_COMMANDS) {
      const src = p2bSources[mod];
      if (!new RegExp(`pub\\s+async\\s+fn\\s+${name}\\b`).test(src)) {
        errors.push(`[P2-b] 缺命令 \`${name}\`（${mod}.rs 中无 fn 定义）`);
      }
      if (!new RegExp(`commands::${mod}::${name}\\b`).test(sources.registerRs ?? "")) {
        errors.push(`[P2-b] \`${name}\` 未出现在 register_commands.rs（两步注册缺一步）`);
      }
    }
    // BotAssignment DTO 权威定义：harness types/bot.rs 与前端 types/bot.ts 双向字段核���。
    const P2B_FIELDS = [
      "id", "root_id", "parent_id", "depth", "origin", "requester_bot_id",
      "requester_conversation_id", "assignee_bot_id", "assignee_name", "mode",
      "task_text", "status", "result_text", "error_code", "error_detail",
      "started_at", "finished_at", "created_at", "updated_at",
    ];
    const rustDto = /pub struct BotAssignment \{([\s\S]*?)\n\}/.exec(sources.typesRs ?? "");
    if (!rustDto) {
      errors.push("[P2-b] harness types/bot.rs 缺 BotAssignment 结构（IPC-P2B §2.2）");
    } else {
      const missing = P2B_FIELDS.filter((f) => !new RegExp(`\\b${f}\\s*:`).test(rustDto[1]));
      if (missing.length) {
        errors.push(`[P2-b] BotAssignment 缺字段: ${missing.join(", ")}`);
      }
      if (!/#\[serde\(rename_all = "camelCase"\)\]\s*\npub struct BotAssignment/.test(sources.typesRs ?? "")) {
        errors.push("[P2-b] BotAssignment 缺 `#[serde(rename_all = \"camelCase\")]`");
      }
    }
    const tsDto = /export interface BotAssignment \{([\s\S]*?)\n\}/.exec(sources.typesTs ?? "");
    if (!tsDto) {
      errors.push("[P2-b] src/types/bot.ts 缺 BotAssignment 接口（前端 DTO 权威镜像）");
    } else {
      const tsMissing = P2B_FIELDS.map((f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase()))
        .filter((f) => !new RegExp(`\\b${f}\\s*:`).test(tsDto[1]));
      if (tsMissing.length) {
        errors.push(`[P2-b] 前端 BotAssignment 缺 camelCase 字段: ${tsMissing.join(", ")}`);
      }
    }
    // 事件：枚举 + 生成物 + 真实发射点（G 节通用判据之外再锁字面量）。
    const EVENT = "bot-assignment-updated";
    if (!(sources.eventsEnum ?? "").includes("BotAssignmentUpdated")) {
      errors.push("[P2-b] IpcEventName 缺 BotAssignmentUpdated（IPC-P2B §5）");
    }
    if (!(sources.eventsTs ?? "").includes(`"${EVENT}"`)) {
      errors.push(`[P2-b] 生成物 events.ts 缺 "${EVENT}"`);
    }
    if (!(p2b.asmtRs ?? "").includes("BotAssignmentUpdated.as_str()")) {
      errors.push(`[P2-b] 无 "${EVENT}" 发射点（命令层/Runner 必须发）`);
    }
  }


  // ── P3 段（IPC-P3 §1/§2/§3/§5）：8 命令 + DTO 字段 + 事件 + 定时分支 ──
  // 仅在真实仓库检查（sources.p3 显式给出）时执行；selftest 的合成 P1 源不触发。
  const p3 = sources.p3;
  if (p3) {
    const P3_COMMANDS = [
      ["create_bot_workflow", "bot_workflow"],
      ["update_bot_workflow", "bot_workflow"],
      ["list_bot_workflows", "bot_workflow"],
      ["delete_bot_workflow", "bot_workflow"],
      ["run_bot_workflow", "bot_workflow"],
      ["list_bot_workflow_runs", "bot_workflow"],
      ["cancel_bot_workflow_run", "bot_workflow"],
      ["create_bot_workflow_schedule", "scheduled_task"],
    ];
    const p3Sources = {
      bot_workflow: p3.workflowRs ?? "",
      scheduled_task: p3.scheduleRs ?? "",
    };
    const registerRs = sources.registerRs ?? "";
    for (const [name, mod] of P3_COMMANDS) {
      const src = p3Sources[mod];
      if (!src.includes("pub async fn " + name + "(")) {
        errors.push("[P3] 缺命令 " + name + "（" + mod + ".rs 中无 fn 定义）");
      }
      if (!registerRs.includes("commands::" + mod + "::" + name + ",")) {
        errors.push("[P3] " + name + " 未出现在 register_commands.rs（两步注册缺一步）");
      }
    }

    const structBody = (src, header) => {
      const start = src.indexOf(header);
      if (start < 0) return null;
      const end = src.indexOf("\n}", start);
      return end < 0 ? null : src.slice(start, end);
    };
    const P3_WORKFLOW_FIELDS = [
      "id", "name", "description", "owner_scope", "department_id", "steps",
      "max_concurrency", "total_timeout_secs", "created_at", "updated_at",
    ];
    const P3_RUN_FIELDS = [
      "id", "workflow_id", "workflow_name", "status", "origin", "requester_bot_id",
      "requester_conversation_id", "anchor_message_id", "context", "completed_steps",
      "total_steps", "error_code", "error_detail", "started_at", "finished_at",
      "created_at", "updated_at",
    ];
    for (const [dtoName, fields] of [["BotWorkflow", P3_WORKFLOW_FIELDS], ["BotWorkflowRun", P3_RUN_FIELDS]]) {
      const rust = structBody(sources.typesRs ?? "", "pub struct " + dtoName + " {");
      if (!rust) {
        errors.push("[P3] harness types/bot.rs 缺 " + dtoName + " 结构（IPC-P3 §2.2/§2.3）");
      } else {
        const missing = fields.filter((f) => !rust.includes(f + ":"));
        if (missing.length) {
          errors.push("[P3] " + dtoName + " 缺字段: " + missing.join(", "));
        }
      }
      const ts = structBody(sources.typesTs ?? "", "export interface " + dtoName + " {");
      if (!ts) {
        errors.push("[P3] src/types/bot.ts 缺 " + dtoName + " 接口（前端 DTO 权威镜像）");
      } else {
        const tsMissing = fields.map((f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase()))
          .filter((f) => !ts.includes(f + ":"));
        if (tsMissing.length) {
          errors.push("[P3] 前端 " + dtoName + " 缺 camelCase 字段: " + tsMissing.join(", "));
        }
      }
    }

    const P3_EVENT = "bot-workflow-run-updated";
    if (!(sources.eventsEnum ?? "").includes("BotWorkflowRunUpdated")) {
      errors.push("[P3] IpcEventName 缺 BotWorkflowRunUpdated（IPC-P3 §3）");
    }
    if (!(sources.eventsTs ?? "").includes('"' + P3_EVENT + '"')) {
      errors.push('[P3] 生成物 events.ts 缺 "' + P3_EVENT + '"');
    }
    const p3Emitters = [p3.workflowRs, p3.runnerRs, p3.servicesRs, p3.scheduleRs]
      .map((v) => v ?? "").join("\n");
    if (!p3Emitters.includes("BotWorkflowRunUpdated.as_str()")) {
      errors.push('[P3] 无 "' + P3_EVENT + '" 发射点（命令/runner/cron 分支任一必须发）');
    }

    if (!(p3.scheduleRs ?? "").includes('pub(crate) const BOT_WORKFLOW_TASK_TYPE: &str = "bot_workflow"')) {
      errors.push('[P3] scheduled_task.rs 缺 task_type="bot_workflow" 字符串常量（IPC-P3 §2.4）');
    }
    if (!(p3.scheduleRs ?? "").includes('"workflowId"')) {
      errors.push("[P3] create_bot_workflow_schedule 缺 params.workflowId（IPC-P3 §2.4）");
    }
    if (!(p3.servicesRs ?? "").includes("is_bot_workflow_job(&job)")) {
      errors.push("[P3] init/services.rs 缺 bot_workflow cron 执行分支（PLAN-P3 §4）");
    }
    // §1/§5：list_bot_assignments 新增 rootId（Rust 名 root_id + 前端 camelCase 键）。
    const asmtFn = /pub\s+async\s+fn\s+list_bot_assignments\s*\(([\s\S]*?)\)\s*->/.exec(p3.assignmentRs ?? "");
    if (!asmtFn || !/root_id\s*:\s*Option<String>/.test(asmtFn[1])) {
      errors.push("[P3] list_bot_assignments 缺 `root_id: Option<String>`（IPC-P3 §1）");
    }
    const asmtCallers = (sources.frontendFiles ?? [])
      .filter((f) => !isMockOrTest(f.path) && f.src.includes('"list_bot_assignments"'));
    if (asmtCallers.length > 0 && !asmtCallers.some((f) =>
      /["']list_bot_assignments["'][\s\S]{0,400}?rootId\s*:/.test(f.src))) {
      errors.push("[P3] 前端调用 list_bot_assignments 未传 camelCase `rootId`（IPC-P3 §1）");
    }
    // §2.6：delegate_to_bot 只暴露公开入参，内部链字段不得进 LLM schema。
    const delegateRs = p3.delegateRs ?? "";
    const schemaStart = /fn\s+input_schema\s*\([^)]*\)\s*->\s*Value\s*\{[\s\S]*?json!\(\s*\{/.exec(delegateRs);
    if (!schemaStart) {
      errors.push("[P3] delegate_to_bot 缺 input_schema() json! 定义（IPC-P3 §2.6）");
    } else {
      const open = delegateRs.indexOf("{", schemaStart.index + schemaStart[0].length - 1);
      const close = matchBracket(delegateRs, open);
      const body = close < 0 ? "" : delegateRs.slice(open + 1, close);
      for (const hidden of ["rootId", "parentId", "depth", "workflowRunId", "stepKey",
        "root_id", "parent_id", "workflow_run_id", "step_key"]) {
        if (body.includes(`"${hidden}":`) || body.includes(`'${hidden}':`)) {
          errors.push("[P3] delegate_to_bot schema 暴露内部字段 `" + hidden + "`（IPC-P3 §2.6）");
        }
      }
      if (!/["']mode["']\s*:/.test(body)) {
        errors.push("[P3] delegate_to_bot schema 缺公开入参 `mode`（IPC-P3 §2.6）");
      }
    }
  }

  // ── P4 段（IPC-P4 §1/§2/§3）：配额 3 命令 + DTO + 超限出口 + agent_respond_ask ──
  // 待答切片（list/answer/cancel 3 命令 + 事件 + DTO）随 B3 卡接入：`p4.inputRs`
  // 缺省即跳过，保证 P4-a / P4-b 两条切片能独立收口（ADR-P4-7）。
  const p4 = sources.p4;
  if (p4) {
    const P4_QUOTA_COMMANDS = [
      ["get_bot_department_quota", "bot_quota"],
      ["set_bot_department_quota", "bot_quota"],
      ["reset_bot_department_quota", "bot_quota"],
    ];
    for (const [name] of P4_QUOTA_COMMANDS) {
      if (!(p4.quotaRs ?? "").includes("pub async fn " + name + "(")) {
        errors.push("[P4] 缺命令 " + name + "（commands/bot_quota.rs 中无 fn 定义）");
      }
      if (!(sources.registerRs ?? "").includes("commands::bot_quota::" + name + ",")) {
        errors.push("[P4] " + name + " 未出现在 register_commands.rs（两步注册缺一步）");
      }
    }

    const p4StructBody = (src, header) => {
      const start = src.indexOf(header);
      if (start < 0) return null;
      const end = src.indexOf("\n}", start);
      return end < 0 ? null : src.slice(start, end);
    };
    const P4_QUOTA_FIELDS = [
      "department_id", "max_concurrency", "total_timeout_secs", "max_steps",
      "max_in_flight_per_depth", "step_timeout_secs", "max_concurrent_runs",
      "is_default", "updated_at",
    ];
    const P4_QUOTA_INPUT_FIELDS = P4_QUOTA_FIELDS
      .filter((f) => f !== "is_default" && f !== "updated_at");
    for (const [dto, fields] of [
      ["BotDepartmentQuota", P4_QUOTA_FIELDS],
      ["BotDepartmentQuotaInput", P4_QUOTA_INPUT_FIELDS],
    ]) {
      const rust = p4StructBody(sources.typesRs ?? "", "pub struct " + dto + " {");
      if (!rust) {
        errors.push("[P4] harness types/bot.rs 缺 " + dto + " 结构（IPC-P4 §2）");
      } else {
        const missing = fields.filter((f) => !rust.includes(f + ":"));
        if (missing.length) {
          errors.push("[P4] " + dto + " 缺字段: " + missing.join(", "));
        }
      }
      // TS 镜像由前端负责（本仓其余线并发写 src/types/bot.ts）：缺失只告警，
      // 一旦存在就按 camelCase 字段核对。
      const tsNames = [dto + "Dto", dto];
      const ts = tsNames
        .map((n) => p4StructBody(sources.typesTs ?? "", "export interface " + n + " {"))
        .find(Boolean);
      if (!ts) {
        warnings.push("[P4] src/types/bot.ts 尚无 " + dto + " 镜像（前端负责，落地后自动核对）");
      } else {
        const camel = fields.map((f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase()));
        const tsMissing = camel.filter((f) => !ts.includes(f + ":"));
        if (tsMissing.length) {
          errors.push("[P4] 前端 " + dto + " 缺 camelCase 字段: " + tsMissing.join(", "));
        }
      }
    }

    if (!(p4.quotaRs ?? "").includes("BOT_QUOTA_INVALID")) {
      errors.push("[P4] set_bot_department_quota 缺 BOT_QUOTA_INVALID 范围校验（IPC-P4 §1）");
    }
    if (!(p4.quotaEnforceRs ?? "").includes("BOT_QUOTA_EXCEEDED")) {
      errors.push("[P4] 缺 BOT_QUOTA_EXCEEDED 出口（定义/触发超限必须可见拒绝，ADR-P4-2）");
    }

    // IPC-P4 §1：agent_respond_ask 签名与返回类型不变（通用聊天零回归）。
    const askFn = parseFns(p4.agentRs ?? "").find((f) => f.name === "agent_respond_ask");
    if (!askFn) {
      errors.push("[P4] agent_respond_ask 不存在（IPC-P4 §1 修改项，不得误删）");
    } else {
      const names = askFn.params.map((p) => p.name);
      const flat = names.includes("ask_id") && names.includes("answer");
      const boxed = names.includes("request");
      if (!flat && !boxed) {
        errors.push("[P4] agent_respond_ask 入参丢了 askId/answer（签名不得变更）");
      }
      if (!askFn.returns || normalizeType(askFn.returns) !== "Result<(),String>") {
        errors.push("[P4] agent_respond_ask 返回类型变了（必须是 Result<(), String>）");
      }
    }

    if (p4.inputRs !== undefined) {
      const P4_INPUT_COMMANDS = [
        "list_bot_input_requests",
        "answer_bot_input_request",
        "cancel_bot_input_request",
      ];
      for (const name of P4_INPUT_COMMANDS) {
        if (!(p4.inputRs ?? "").includes("pub async fn " + name + "(")) {
          errors.push("[P4-b] 缺命令 " + name + "（commands/bot_input.rs 中无 fn 定义）");
        }
        if (!(sources.registerRs ?? "").includes("commands::bot_input::" + name + ",")) {
          errors.push("[P4-b] " + name + " 未出现在 register_commands.rs（两步注册缺一步）");
        }
      }
      const P4_INPUT_FIELDS = [
        "id", "ask_id", "conversation_id", "bot_id", "workflow_run_id", "assignment_id",
        "origin", "question", "options", "status", "answer", "answered_at", "expires_at",
        "created_at", "updated_at",
      ];
      const rust = p4StructBody(sources.typesRs ?? "", "pub struct BotInputRequest {");
      if (!rust) {
        errors.push("[P4-b] harness types/bot.rs 缺 BotInputRequest 结构（IPC-P4 §2）");
      } else {
        const missing = P4_INPUT_FIELDS.filter((f) => !rust.includes(f + ":"));
        if (missing.length) {
          errors.push("[P4-b] BotInputRequest 缺字段: " + missing.join(", "));
        }
      }
      const helper = p4.inputHelperRs ?? "";
      for (const status of ["pending", "answered", "expired", "cancelled"]) {
        if (!(sources.typesRs ?? "").includes('"' + status + '"')) {
          errors.push('[P4-b] 值域缺 "' + status + '"（input_status，IPC-P4 §2）');
        }
      }
      for (const origin of ["person", "routine", "workflow"]) {
        if (!(sources.typesRs ?? "").includes('"' + origin + '"')) {
          errors.push('[P4-b] origin 值域缺 "' + origin + '"（input_origin，IPC-P4 §2）');
        }
      }
      if (!helper.includes("input_status::")) {
        errors.push("[P4-b] 生命周期 helper 未统一走 input_status 值域（字面量漂移风险）");
      }
      if (!(sources.eventsEnum ?? "").includes("BotInputRequestUpdated")) {
        errors.push("[P4-b] IpcEventName 缺 BotInputRequestUpdated（IPC-P4 §3）");
      }
      if (!(sources.eventsTs ?? "").includes('"bot-input-request-updated"')) {
        errors.push('[P4-b] 生成物 events.ts 缺 "bot-input-request-updated"');
      }
      const emitters = [p4.inputRs, p4.inputHelperRs, p4.agentRs]
        .map((v) => v ?? "").join("\n");
      if (!emitters.includes("BotInputRequestUpdated.as_str()")) {
        errors.push('[P4-b] 无 "bot-input-request-updated" 发射点（落库/答复/超时/取消至少一处）');
      }
    }
  }

  // ── P5 段（IPC-P5 §1/§2/§5）：8 命令 + DTO + 错误码出口 + B2 熔断接线 ──
  // 仅在真实仓库检查（sources.p5 显式给出）时执行；合成 P1 源不触发。
  const p5 = sources.p5;
  if (p5) {
    const costRs = p5.costRs ?? "";
    // 错误码出口允许分布在命令层与服务层（命令薄/服务厚分层）；两者合并核对。
    const skillRs = (p5.skillRs ?? "") + "\n" + (p5.skillServiceRs ?? "");
    const quotaEnforceRs = p5.quotaEnforceRs ?? "";
    // 出口既可用错误码字面值（`BOT_X`），也可用 error_code.rs 常量短名（`bot_err::X`）。
    const hasErrExit = (src, code) => src.includes(code) || src.includes(code.replace(/^BOT_/, ""));

    for (const [name, mod] of BOT_P5_COMMANDS) {
      const src = mod === "bot_cost" ? costRs : skillRs;
      if (!new RegExp(`pub\\s+async\\s+fn\\s+${name}\\b`).test(src)) {
        errors.push("[P5] 缺命令 " + name + "（commands/" + mod + ".rs 中无 fn 定义）");
      }
      if (!(sources.registerRs ?? "").includes("commands::" + mod + "::" + name + ",")) {
        errors.push("[P5] " + name + " 未出现在 register_commands.rs（两步注册缺一步）");
      }
    }

    const p5StructBody = (src, header) => {
      const start = src.indexOf(header);
      if (start < 0) return null;
      const end = src.indexOf("\n}", start);
      return end < 0 ? null : src.slice(start, end);
    };

    const P5_DTOS = [
      ["BotCostQuota", ["scope", "scope_id", "max_total_tokens", "max_cost_usd", "window_hours", "is_default", "updated_at"]],
      ["BotCostQuotaInput", ["scope", "scope_id", "max_total_tokens", "max_cost_usd", "window_hours"]],
      ["BotUsageEntry", ["id", "bot_id", "department_id", "conversation_id", "execution_id", "workflow_run_id", "assignment_id", "model_id", "origin", "input_tokens", "output_tokens", "cache_creation_tokens", "cache_read_tokens", "total_tokens", "estimated_cost_usd", "pricing_source", "created_at"]],
      ["BotUsageFilter", ["bot_id", "department_id", "origin", "workflow_run_id", "assignment_id", "since", "until"]],
      ["BotUsageSummary", ["scope", "scope_id", "window_hours", "input_tokens", "output_tokens", "cache_creation_tokens", "cache_read_tokens", "total_tokens", "estimated_cost_usd", "entry_count", "breached", "pricing_note"]],
      ["BotSkill", ["name", "description", "source", "enabled", "shadowed_global", "missing", "preset_id", "updated_at"]],
      ["BotSkillPreset", ["version", "name", "description", "skills"]],
      ["BotSkillPresetItem", ["name", "description", "content", "category", "tags", "requires_toolsets"]],
      ["BotSkillPresetSkipped", ["name", "reason", "detail"]],
      ["BotSkillPresetResult", ["preset_id", "imported", "skipped"]],
    ];
    for (const [dto, fields] of P5_DTOS) {
      const rust = p5StructBody(sources.typesRs ?? "", "pub struct " + dto + " {");
      if (!rust) {
        errors.push("[P5] harness types/bot.rs 缺 " + dto + " 结构（IPC-P5 §2）");
      } else {
        const missing = fields.filter((f) => !rust.includes(f + ":"));
        if (missing.length) {
          errors.push("[P5] " + dto + " 缺字段: " + missing.join(", "));
        }
      }
      // TS 镜像：存在则核对 camelCase 字段，缺失给 warning（前端并行线）。
      const tsNames = [dto + "Dto", dto];
      const ts = tsNames.map((n) => p5StructBody(sources.typesTs ?? "", "export interface " + n + " {")).find(Boolean);
      if (!ts) {
        warnings.push("[P5] src/types/bot.ts 尚无 " + dto + " 镜像（前端负责，落地后自动核对）");
      } else {
        const camel = fields.map((f) => f.replace(/_([a-z])/g, (_, c) => c.toUpperCase()));
        // 冻结稿 IPC-P5 §2 的 TS 镜像用可选字段（`botId?:`）；同时接受必选 `botId:`。
        const tsMissing = camel.filter((f) => !ts.includes(f + ":") && !ts.includes(f + "?:"));
        if (tsMissing.length) {
          errors.push("[P5] 前端 " + dto + " 缺 camelCase 字段: " + tsMissing.join(", "));
        }
      }
    }

    for (const code of ["BOT_COST_QUOTA_INVALID", "BOT_COST_QUOTA_EXCEEDED", "BOT_SKILL_NOT_FOUND", "BOT_SKILL_PRESET_INVALID"]) {
      if (!(sources.errorCodeRs ?? "").includes('"' + code + '"')) {
        errors.push("[P5] error_code.rs 缺错误码 " + code);
      }
    }
    if (!hasErrExit(costRs, "BOT_COST_QUOTA_INVALID")) {
      errors.push("[P5] bot_cost 命令缺 BOT_COST_QUOTA_INVALID 校验出口");
    }
    if (!hasErrExit(quotaEnforceRs, "BOT_COST_QUOTA_EXCEEDED")) {
      errors.push("[P5] 缺 BOT_COST_QUOTA_EXCEEDED 熔断出口（ADR-P5-3 只拦新建 run/轮）");
    }
    if (!hasErrExit(skillRs, "BOT_SKILL_PRESET_INVALID")) {
      errors.push("[P5] bot_skill 缺 BOT_SKILL_PRESET_INVALID 出口");
    }
  }

  return { errors, warnings };
}

// ── selftest：合成源码 + 单点破坏 ────────────────────────────────────────

function renderStruct(s, rustName = s.name) {
  const fields = s.fields.map(([n, t]) => `    pub ${n}: ${t},`).join("\n");
  return `#[derive(${s.derive})]\n#[serde(rename_all = "camelCase")]\npub struct ${rustName} {\n${fields}\n}`;
}

function generateValidSources() {
  const spec = BOT_IPC;
  // 与真实仓库一致：带别名的输入 DTO 定义在 harness（用 rustName），其余在命令层。
  const aliased = spec.structs.filter((s) => (s.aliases ?? []).length > 0);
  const typesRs = [
    spec.structs.find((s) => s.name === "Bot"),
    ...aliased,
  ]
    .filter(Boolean)
    .map((s) => renderStruct(s, (s.aliases ?? [])[0] ?? s.name))
    .join("\n\n");
  const botRs = [
    ...spec.commands.map((c) => {
      const wiring = ["app: AppHandle", "state: State<'_, AppState>"];
      const params = [...wiring, ...c.params.map((p) => `${p.name}: ${p.type}`)].join(", ");
      return `#[tauri::command]\npub async fn ${c.name}(${params}) -> ${c.returns} {\n    todo!()\n}`;
    }),
    ...spec.structs
      .filter((s) => s.name !== "Bot" && !(s.aliases ?? []).length)
      .map((s) => renderStruct(s)),
  ].join("\n\n");

  const registerRs = `pub fn register() {\n    tauri::generate_handler![\n${spec.commands
    .map((c) => `        commands::bot::${c.name},`)
    .join("\n")}\n    ]\n}`;

  // 常量名是短名、值是完整错误码 —— 与 `error_code.rs` 的真实写法一致。
  // 正控因此同时锁定「按值而非按名字比对」。
  const errorCodeRs = `pub mod bot {\n${spec.errorCodes
    .map((c) => `    pub const ${c.replace("BOT_", "")}: &str = "${c}";`)
    .join("\n")}\n}`;

  const locales = Array.from({ length: spec.localeCount }, (_, i) => ({
    name: ["zh-CN", "zh-TW", "en-US", "ja", "ko", "de", "fr", "es", "ru", "ar", "hi"][i],
    json: { error: Object.fromEntries(spec.errorCodes.map((c) => [c, `msg-${c}`])) },
  }));

  const typesTs = spec.frontendTypes
    .map((n) => (n === "ApprovalMode" ? `export type ${n} = "ask" | "full";` : `export interface ${n} {}`))
    .join("\n");

  const frontendArgs = {
    list_bots: "{ includeDisabled: false }",
    get_bot: "{ botId: id }",
    create_bot: "{ input: { name } }",
    update_bot: "{ botId: id, input: { name } }",
    delete_bot: "{ botId: id }",
    list_bot_threads: "{ botId: id, includeArchived: false }",
    read_bot_soul: "{ botId: id }",
    write_bot_soul: "{ botId: id, content }",
    read_bot_memory: "{ botId: id }",
  };
  const frontendFiles = [
    {
      path: "src/stores/feature/botStore.ts",
      src: spec.commands
        .map((c) => `await invoke<unknown>("${c.name}", ${frontendArgs[c.name]});`)
        .join("\n"),
    },
  ];

  const eventsEnum = `pub enum IpcEventName {\n    AgentDone,\n}\n`;
  const eventsTs = `export const IPC_EVENT_NAMES = ["agent-done"];\n`;

  return {
    botRs,
    typesRs,
    modRs: "pub mod bot;\n",
    registerRs,
    errorCodeRs,
    locales,
    typesTs,
    frontendFiles,
    eventsEnum,
    eventsTs,
  };
}

export function botSelftest() {
  const cases = [];
  const add = (name, mutate, expectedPattern, kind = "errors") => {
    const sources = decorateWithP5(decorateWithP4(decorateWithP3(generateValidSources())));
    mutate(sources);
    const { errors, warnings } = auditBotIpc(sources);
    const pool = kind === "warnings" ? warnings : errors;
    const hit = pool.some((e) => expectedPattern.test(e));
    cases.push({
      name,
      pass: hit,
      detail: hit ? "" : `期望命中 ${expectedPattern}，实得 ${JSON.stringify(pool)}`,
    });
  };

  // 反控：变异后「不应」命中给定错误模式（验证口径放宽未误伤/未过度报错）。
  const addNoHit = (name, mutate, expectedPattern, kind = "errors") => {
    const sources = decorateWithP5(decorateWithP4(decorateWithP3(generateValidSources())));
    mutate(sources);
    const { errors, warnings } = auditBotIpc(sources);
    const pool = kind === "warnings" ? warnings : errors;
    const hit = pool.some((e) => expectedPattern.test(e));
    cases.push({
      name,
      pass: !hit,
      detail: hit ? `不应命中 ${expectedPattern}，实得 ${JSON.stringify(pool)}` : "",
    });
  };

  // 正控：未破坏时必须 0 错（否则所有破坏都「命中」= 假绿）
  {
    const { errors } = auditBotIpc(decorateWithP5(decorateWithP4(decorateWithP3(generateValidSources()))));
    cases.push({
      name: "正控：完整契约 ⇒ 0 错",
      pass: errors.length === 0,
      detail: errors.join("; "),
    });
  }

  add("缺命令 delete_bot", (s) => {
    s.botRs = s.botRs.replace(/#\[tauri::command\]\npub async fn delete_bot[\s\S]*?\n}/, "");
    s.registerRs = s.registerRs.replace(/\s*commands::bot::delete_bot,/, "");
  }, /缺命令 `delete_bot`/);

  add("命令漏注册 create_bot", (s) => {
    s.registerRs = s.registerRs.replace(/\s*commands::bot::create_bot,/, "");
  }, /`create_bot` 未出现在 register_commands/);

  add("参数改名 bot_id → botId", (s) => {
    s.botRs = s.botRs.replace("pub async fn get_bot(app: AppHandle, state: State<'_, AppState>, bot_id: String)",
      "pub async fn get_bot(app: AppHandle, state: State<'_, AppState>, botId: String)");
  }, /命令 `get_bot` 参数不符/);

  add("返回类型漂移", (s) => {
    s.botRs = s.botRs.replace("pub async fn get_bot(app: AppHandle, state: State<'_, AppState>, bot_id: String) -> Result<Bot, String>",
      "pub async fn get_bot(app: AppHandle, state: State<'_, AppState>, bot_id: String) -> Result<Option<Bot>, String>");
  }, /命令 `get_bot` 返回类型不符/);

  add("DTO 少 rename_all camelCase", (s) => {
    s.typesRs = s.typesRs.replace('#[derive(Serialize, Deserialize)]\n#[serde(rename_all = "camelCase")]\npub struct Bot {',
      '#[derive(Serialize, Deserialize)]\npub struct Bot {');
  }, /DTO `Bot` 缺 `#\[serde\(rename_all = "camelCase"\)\]`/);

  add("DTO 少字段 soul_hash", (s) => {
    s.typesRs = s.typesRs.replace("    pub soul_hash: String,\n", "");
  }, /DTO `Bot` 字段不符：缺 \[soul_hash\]/);

  add("UpdateBotInput 三态退化成 Option<String>", (s) => {
    s.typesRs = s.typesRs.replace("    pub title: Option<Option<String>>,", "    pub title: Option<String>,");
  }, /UpdateBotInput\.title` 必须是 `Option<Option<String>>`/);

  add("漏一个错误码常量", (s) => {
    s.errorCodeRs = s.errorCodeRs.replace('    pub const DELETE_FAILED: &str = "BOT_DELETE_FAILED";\n', "");
  }, /错误码常量缺值 `BOT_DELETE_FAILED`/);

  add("错误码值漂移（复制粘贴改错）", (s) => {
    s.errorCodeRs = s.errorCodeRs.replace('pub const ICON_INVALID: &str = "BOT_ICON_INVALID"',
      'pub const ICON_INVALID: &str = "BOT_ICON_TOO_LONG"');
  }, /错误码常量缺值 `BOT_ICON_INVALID`/);

  add("某语言漏 error 键", (s) => {
    delete s.locales[7].json.error.BOT_NOT_FOUND;
  }, /locale `es` 缺 `error\.BOT_NOT_FOUND`/);

  add("新增 bot-updated 事件", (s) => {
    s.eventsEnum = `pub enum IpcEventName {\n    AgentDone,\n    BotUpdated,\n}\nimpl IpcEventName { fn as_str(&self) { IpcEventName::BotUpdated => "bot-updated" } }`;
  }, /出现 Bot 事件变体|P1 禁止的新事件/);

  add("DTO 定义文件整体缺失（harness types 丢了）", (s) => {
    s.typesRs = "";
  }, /缺 DTO `Bot`/);

  add("前端类型漏 BotMemory", (s) => {
    s.typesTs = s.typesTs.replace("export interface BotMemory {}\n", "");
  }, /src\/types\/bot\.ts 缺 `export \.\.\. BotMemory`/);

  add("前端用 snake_case 参数", (s) => {
    s.frontendFiles[0].src = s.frontendFiles[0].src.replace("{ botId: id }", "{ bot_id: id }");
  }, /用了 snake_case 参数 `bot_id`/);

  add("前端没有调用 read_bot_memory", (s) => {
    s.frontendFiles[0].src = s.frontendFiles[0].src.replace(
      'await invoke<unknown>("read_bot_memory", { botId: id });',
      "",
    );
  }, /命令 `read_bot_memory` 在 src\/\*\* 无调用点/, "warnings");

  add("P3 缺命令 run_bot_workflow", (s) => {
    s.p3.workflowRs = s.p3.workflowRs.replace("pub async fn run_bot_workflow", "pub async fn removed_run");
  }, /\[P3\] 缺命令 run_bot_workflow/);

  add("P3 list_bot_assignments 丢 root_id", (s) => {
    s.p3.assignmentRs = s.p3.assignmentRs.replace("root_id: Option<String>", "other: Option<String>");
  }, /list_bot_assignments 缺 `root_id/);

  add("P3 delegate schema 泄露 depth", (s) => {
    s.p3.delegateRs = s.p3.delegateRs.replace('"mode"', '"depth"');
  }, /schema 暴露内部字段 `depth`/);

  add("P3 事件变体缺失", (s) => {
    s.eventsEnum = s.eventsEnum.replace("BotWorkflowRunUpdated,", "");
  }, /IpcEventName 缺 BotWorkflowRunUpdated/);

  add("P4 缺命令 get_bot_department_quota", (s) => {
    s.p4.quotaRs = s.p4.quotaRs.replace("pub async fn get_bot_department_quota", "pub async fn removed_quota_get");
  }, /\[P4\] 缺命令 get_bot_department_quota/);

  add("P4 配额命令漏注册 set_bot_department_quota", (s) => {
    s.registerRs = s.registerRs.replace(/\s*commands::bot_quota::set_bot_department_quota,/, "");
  }, /\[P4\] set_bot_department_quota 未出现在 register_commands/);

  add("P4 配额 DTO 缺字段 max_steps", (s) => {
    s.typesRs = s.typesRs.replace("    pub max_steps: String,\n", "    pub max_steps_v2: String,\n");
  }, /\[P4\] BotDepartmentQuota 缺字段: max_steps/);

  add("P4 agent_respond_ask 返回类型漂移", (s) => {
    s.p4.agentRs = s.p4.agentRs.replace("-> Result<(), String>", "-> Result<bool, String>");
  }, /\[P4\] agent_respond_ask 返回类型变了/);

  add("P4-b 事件漏枚举", (s) => {
    s.eventsEnum = s.eventsEnum.replace("BotInputRequestUpdated,", "");
  }, /\[P4-b\] IpcEventName 缺 BotInputRequestUpdated/);

  add("P4-b 缺 answer 命令", (s) => {
    s.p4.inputRs = s.p4.inputRs.replace("pub async fn answer_bot_input_request", "pub async fn removed_answer");
  }, /\[P4-b\] 缺命令 answer_bot_input_request/);

  add("P4 超限出口丢失（静默夹紧）", (s) => {
    s.p4.quotaEnforceRs = s.p4.quotaEnforceRs.replace("BOT_QUOTA_EXCEEDED", "BOT_QUOTA_SOFT_CAP");
  }, /\[P4\] 缺 BOT_QUOTA_EXCEEDED 出口/);

  add("P5 缺命令 set_bot_cost_quota", (s) => {
    s.p5.costRs = s.p5.costRs.replace("pub async fn set_bot_cost_quota", "pub async fn removed_cost_set");
  }, /\[P5\] 缺命令 set_bot_cost_quota/);

  add("P5 技能命令漏注册 import_bot_skill_preset", (s) => {
    s.registerRs = s.registerRs.replace(/\s*commands::bot_skill::import_bot_skill_preset,/, "");
  }, /\[P5\] import_bot_skill_preset 未出现在 register_commands/);

  add("P5 DTO 缺字段 estimated_cost_usd", (s) => {
    s.typesRs = s.typesRs.replace("    pub estimated_cost_usd: String,\n", "    pub estimated_cost_usd_v2: String,\n");
  }, /\[P5\] BotUsageEntry 缺字段: estimated_cost_usd/);

  add("P5 熔断出口丢失", (s) => {
    s.p5.quotaEnforceRs = s.p5.quotaEnforceRs.replace("BOT_COST_QUOTA_EXCEEDED", "BOT_COST_QUOTA_SOFT_CAP");
  }, /\[P5\] 缺 BOT_COST_QUOTA_EXCEEDED 熔断出口/);

  // 新口径回归：可选字段写法（`botId?:`）应被接受（反控：不得报缺）。
  addNoHit("P5 前端 DTO 可选字段不被误判", (s) => {
    s.typesTs = s.typesTs.replace("  botId: string;", "  botId?: string;");
  }, /\[P5\] 前端 BotUsageFilter 缺 camelCase 字段/);

  add("P5 前端 DTO 真缺字段仍须变红", (s) => {
    s.typesTs = s.typesTs.replace("  category: string;\n", "");
  }, /\[P5\] 前端 BotSkillPresetItem 缺 camelCase 字段: category/);

  // 新口径回归：错误码常量短名（`bot_err::COST_QUOTA_INVALID`）应被接受（反控）。
  addNoHit("P5 错误码常量短名不被误判", (s) => {
    s.p5.costRs = s.p5.costRs.replace(/BOT_COST_QUOTA_INVALID/g, "bot_err::COST_QUOTA_INVALID");
  }, /\[P5\] bot_cost 命令缺 BOT_COST_QUOTA_INVALID 校验出口/);

  add("P5 错误码真缺失仍须变红", (s) => {
    s.p5.skillRs = s.p5.skillRs.replace(/BOT_SKILL_PRESET_INVALID/g, "SKILL_PRESET_SOFT");
    s.p5.skillServiceRs = "";
  }, /\[P5\] bot_skill 缺 BOT_SKILL_PRESET_INVALID 出口/);

  const failed = cases.filter((c) => !c.pass);
  return { cases, failed };
}
