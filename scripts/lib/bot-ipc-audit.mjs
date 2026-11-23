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
  ],
  /** IPC.md §6：P1 不新增事件。出现任何 `bot-*` 事件名即违约。 */
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

  // 6) 事件：P1 不新增 bot-* 事件；P2-b 唯一例外 `bot-assignment-updated`（IPC-P2B §5）。
  const P2B_ALLOWED_EVENT = "bot-assignment-updated";
  const P2B_ALLOWED_VARIANT = "BotAssignmentUpdated";
  const eventSources = [
    ["event_names.rs", sources.eventsEnum ?? ""],
    ["types/generated/events.ts", sources.eventsTs ?? ""],
  ];
  for (const [label, src] of eventSources) {
    const re = new RegExp(`"${spec.forbiddenEventPrefix}[a-z0-9-]*"`, "g");
    const hits = [...src.matchAll(re)].map((m) => m[0]);
    const bad = hits.find((h) => h !== `"${P2B_ALLOWED_EVENT}"`);
    if (bad) {
      errors.push(`${label} 出现 P1 禁止的新事件 \`${bad}\`（IPC.md §6：P1 不新增事件）`);
    }
    const variants = [...src.matchAll(/\bBot[A-Z][A-Za-z0-9]*\b/g)].map((m) => m[0]);
    const badVariant = variants.find((v) => v !== P2B_ALLOWED_VARIANT);
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
    const sources = generateValidSources();
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

  // 正控：未破坏时必须 0 错（否则所有破坏都「命中」= 假绿）
  {
    const { errors } = auditBotIpc(generateValidSources());
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

  const failed = cases.filter((c) => !c.pass);
  return { cases, failed };
}
