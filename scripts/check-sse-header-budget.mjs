#!/usr/bin/env node

/**
 * 首字节（HTTP 响应头）预算口径门。
 *
 * ── 为什么必须有这道门 ──
 * 2026-10-27 改造前：首字节等待只有 OpenAI 兼容路径有一道闸门，且是「10s 固定 +
 * 按请求体积每 64KB 加 5s、封顶 60s」；Anthropic / Gemini / OpenAI-Responses /
 * transport 四条路径**没有闸门**。同一上游在不同协议下等待时长完全不同，而闸门
 * 本身也没有任何锁 —— 改回去不会有任何门报错。失效形态是「改了 openai.rs 的
 * 公式，另外四条路径没跟上」「把默认 45/90 抄成第二份」「文案改了但三处消费点
 * 还在按老短语匹配」。
 *
 * ── 权威是谁 ──
 * `src-tauri/crates/providers/src/stream_budget.rs` —— 首字节预算（软/硬两档）
 * 的**唯一**定义处（AGENTS.md 规则 12：共享契约的权威定义在 harness/implementor
 * 一侧，其余面一律被对账）。默认值常量与 `resolve_header_budget` 都只此一份。
 *
 * ── 判据 ──
 *   B1 权威唯一：预算默认值常量与 `resolve_header_budget` 各只允许在
 *      stream_budget.rs 定义一处（别处出现同名定义 ⇒ 红）。
 *   B2 五条路径同口径：openai / anthropic / gemini / openai_responses /
 *      transport::chat_completions 五条流式路径**每一条**都必须经由
 *      `send_with_header_budget`；缺任一条 ⇒ 红（口径分叉的根因）。
 *   B3 旧闸门退役：providers crate 内不得再出现 `response_headers_timeout`
 *      与旧的「10s + 每 64KB 加 5s」缩放公式。
 *   B4 错误文案契约：`response headers not received within` 的产端必须只在
 *      stream_budget.rs 一处；两处消费点（harness 的 llm_executor 按它降日志级别、
 *      rt-workflow 的 is_gateway_header_timeout 按它归 LLM_CALL_FAILED）必须仍在
 *      匹配同一短语 —— 产端改文案而消费点没跟上 ⇒ 红。
 *   B5 可配置链路接通：`ModelParamOverrides.header_timeout_secs`、
 *      `ProviderRequestContext.header_timeout_secs`、`build_provider_request_context_for_model`
 *      （从模型级那一格取值）、工作流主链调用 `_for_model`、前端 TS 类型与设置格，
 *      六处必须同时在位 —— 少一环就是「面板能填但不生效」。
 *
 * 用法：
 *   node scripts/check-sse-header-budget.mjs            # 正常检查
 *   node scripts/check-sse-header-budget.mjs --selftest  # 负控自检（不落盘）
 *   node scripts/check-sse-header-budget.mjs --dump      # 打印各判据读数
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const SRC_TAURI = resolve(root, "src-tauri");

const SKIP_DIRS = new Set([
  "target",
  "node_modules",
  ".git",
  ".worktrees",
  "output",
  ".qoder",
  ".workbuddy",
  "dist",
  "coverage",
]);

// 五条必须同口径的流式路径
const STREAM_PATHS = [
  "crates/providers/src/openai.rs",
  "crates/providers/src/anthropic.rs",
  "crates/providers/src/gemini.rs",
  "crates/providers/src/openai_responses.rs",
  "crates/providers/src/transport/chat_completions.rs",
];

const AUTHORITY_FILE = "crates/providers/src/stream_budget.rs";

const BUDGET_CONSTS = [
  "DEFAULT_SOFT_HEADER_TIMEOUT_SECS",
  "DEFAULT_HARD_HEADER_TIMEOUT_SECS",
  "MAX_SOFT_HEADER_TIMEOUT_SECS",
  "MAX_HARD_HEADER_TIMEOUT_SECS",
  "MIN_CONFIGURED_HEADER_TIMEOUT_SECS",
  "MAX_CONFIGURED_HEADER_TIMEOUT_SECS",
  "HEADER_BUDGET_BODY_STEP_BYTES",
  "HEADER_BUDGET_BODY_STEP_SECS",
];

const ERR_PHRASE = "response headers not received within";

const CONSUMERS = [
  "crates/harness/src/llm_executor.rs",
  "crates/rt-workflow/src/work_engine/executors/agent_executor.rs",
];

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) walk(full, out);
    else if (entry.endsWith(".rs")) out.push(full);
  }
  return out;
}

function loadSources() {
  const sources = new Map();
  for (const file of walk(SRC_TAURI)) {
    sources.set(relative(SRC_TAURI, file).replace(/\\/g, "/"), readFileSync(file, "utf8"));
  }
  return sources;
}

function loadFrontendSources() {
  const sources = new Map();
  const files = ["src/types/index.ts", "src/components/settings/ProviderDetail.tsx"];
  for (const rel of files) {
    sources.set(rel, readFileSync(resolve(root, rel), "utf8"));
  }
  return sources;
}

function countConstDefinitions(sources, name) {
  const re = new RegExp(`(?:pub\\s+)?const\\s+${name}\\s*:`, "g");
  const hits = [];
  for (const [path, text] of sources) {
    const n = (text.match(re) || []).length;
    if (n > 0) hits.push({ path, n });
  }
  return hits;
}

function countFnDefinitions(sources, name) {
  const re = new RegExp(`(?:pub\\s+)?(?:async\\s+)?fn\\s+${name}\\s*[(<]`, "g");
  const hits = [];
  for (const [path, text] of sources) {
    const n = (text.match(re) || []).length;
    if (n > 0) hits.push({ path, n });
  }
  return hits;
}

function occurrences(sources, needle, pathPrefix) {
  const hits = [];
  for (const [path, text] of sources) {
    if (pathPrefix && !path.startsWith(pathPrefix)) continue;
    const n = text.split(needle).length - 1;
    if (n > 0) hits.push({ path, n });
  }
  return hits;
}

// ── 判据 ────────────────────────────────────────────────────────────

const RULES = [
  {
    id: "B1",
    desc: "首字节预算默认值与解析函数只有一处定义",
    check(sources) {
      const bad = [];
      for (const name of BUDGET_CONSTS) {
        const hits = countConstDefinitions(sources, name);
        // 按**定义处总数**判，不是按文件数 —— 同一文件里抄两份同样要红。
        const total = hits.reduce((acc, h) => acc + h.n, 0);
        if (total === 0) {
          bad.push({ msg: `${name} 没有任何定义处（权威文件被删？）` });
        } else if (total > 1) {
          bad.push({
            msg: `${name} 有 ${total} 处定义（${hits.map((h) => `${h.path}×${h.n}`).join(", ")}）—— 权威只允许 ${AUTHORITY_FILE} 一处`,
          });
        } else if (hits[0].path !== AUTHORITY_FILE) {
          bad.push({
            msg: `${name} 定义在 ${hits[0].path}，权威应在 ${AUTHORITY_FILE}`,
          });
        }
      }
      const fnHits = countFnDefinitions(sources, "resolve_header_budget");
      const fnTotal = fnHits.reduce((acc, h) => acc + h.n, 0);
      if (fnTotal !== 1) {
        bad.push({ msg: `resolve_header_budget 应恰好 1 处定义，实得 ${fnTotal}` });
      } else if (fnHits[0].path !== AUTHORITY_FILE) {
        bad.push({
          msg: `resolve_header_budget 定义在 ${fnHits[0].path}，权威应在 ${AUTHORITY_FILE}`,
        });
      }
      return bad;
    },
  },
  {
    id: "B2",
    desc: "五条流式路径都经由 send_with_header_budget",
    check(sources) {
      const bad = [];
      for (const rel of STREAM_PATHS) {
        const text = sources.get(rel);
        if (text === undefined) {
          bad.push({ msg: `${rel} 读不到（路径改名？）` });
        } else if (!text.includes("send_with_header_budget")) {
          bad.push({ msg: `${rel} 未调用 send_with_header_budget（口径分叉）` });
        }
      }
      return bad;
    },
  },
  {
    id: "B3",
    desc: "旧的 10s 固定闸门已退役",
    check(sources) {
      const bad = [];
      const oldFormula = /\(\s*10\s*\+\s*\(\s*body_size\s*\/\s*\(\s*64\s*\*\s*1024\s*\)/;
      for (const [path, text] of sources) {
        if (!path.startsWith("crates/providers/")) continue;
        if (text.includes("response_headers_timeout")) {
          bad.push({ msg: `${path} 仍出现 response_headers_timeout（旧闸门变量名）` });
        }
        if (oldFormula.test(text)) {
          bad.push({ msg: `${path} 仍出现旧的「10s + 每 64KB 加 5s」缩放公式` });
        }
      }
      return bad;
    },
  },
  {
    id: "B4",
    desc: "响应头超时错误文案的产端唯一、消费点仍在匹配",
    check(sources) {
      const bad = [];
      const providerHits = occurrences(sources, ERR_PHRASE, "crates/providers/src/");
      const providerPaths = new Set(providerHits.map((h) => h.path));
      if (!providerPaths.has(AUTHORITY_FILE)) {
        bad.push({ msg: `${AUTHORITY_FILE} 不再产出错误短语「${ERR_PHRASE}」` });
      }
      for (const p of providerPaths) {
        if (p !== AUTHORITY_FILE) {
          bad.push({ msg: `${p} 也在合成错误短语「${ERR_PHRASE}」（产端应唯一）` });
        }
      }
      for (const rel of CONSUMERS) {
        const text = sources.get(rel);
        if (text === undefined) {
          bad.push({ msg: `${rel} 读不到` });
        } else if (!text.includes(ERR_PHRASE)) {
          bad.push({
            msg: `${rel} 不再匹配「${ERR_PHRASE}」（产端改文案后消费点没跟上）`,
          });
        }
      }
      return bad;
    },
  },
  {
    id: "B5",
    desc: "首字节超时可配置链路六处在位",
    check(sources, frontend) {
      const bad = [];
      const need = [
        ["crates/harness/src/types/provider_model.rs", "pub header_timeout_secs: Option<u64>", "ModelParamOverrides 缺 header_timeout_secs 字段"],
        ["crates/harness/src/provider.rs", "pub header_timeout_secs: Option<u64>", "ProviderRequestContext 缺 header_timeout_secs 字段"],
        ["crates/harness/src/context_builder.rs", "build_provider_request_context_for_model", "缺 build_provider_request_context_for_model（模型级取值入口）"],
        ["crates/harness/src/context_builder.rs", "o.header_timeout_secs", "build_provider_request_context_for_model 未读取模型级那一格"],
        ["crates/rt-workflow/src/work_engine/executors/agent_executor.rs", "build_provider_request_context_for_model", "agent_executor 主链未走 _for_model（面板填了不生效）"],
        ["crates/rt-workflow/src/work_engine/executors/llm_executor.rs", "build_provider_request_context_for_model", "llm_executor 主链未走 _for_model（面板填了不生效）"],
      ];
      for (const [rel, needle, msg] of need) {
        const text = sources.get(rel);
        if (text === undefined) bad.push({ msg: `${rel} 读不到` });
        else if (!text.includes(needle)) bad.push({ msg });
      }
      const tsType = frontend.get("src/types/index.ts") ?? "";
      if (!tsType.includes("headerTimeoutSecs?: number;")) {
        bad.push({ msg: "src/types/index.ts 的 ModelParamOverrides 缺 headerTimeoutSecs" });
      }
      const ui = frontend.get("src/components/settings/ProviderDetail.tsx") ?? "";
      if (!ui.includes("editHeaderTimeoutSecs")) {
        bad.push({ msg: "ProviderDetail.tsx 缺首字节超时的编辑状态" });
      }
      if (!ui.includes("settings.headerTimeoutSecs")) {
        bad.push({ msg: "ProviderDetail.tsx 缺首字节超时的设置格" });
      }
      return bad;
    },
  },
];

function runAll(sources, frontend) {
  const bad = [];
  for (const rule of RULES) {
    for (const b of rule.check(sources, frontend)) bad.push({ rule: rule.id, ...b });
  }
  return bad;
}

// ── 负控自检（不落盘、不碰共享树）────────────────────────────────────

function selftest() {
  const sources = loadSources();
  const frontend = loadFrontendSources();
  const baseline = runAll(sources, frontend);
  if (baseline.length) {
    console.error("自检失败：基线本身就不绿，先修实现再谈负控");
    for (const b of baseline) console.error(`  ${b.rule} · ${b.msg}`);
    process.exit(1);
  }

  const mutate = (path, from, to) => {
    const copy = new Map(sources);
    const text = copy.get(path);
    if (text === undefined || !text.includes(from)) {
      throw new Error(`自检变异锚点未命中：${path} ← ${from}`);
    }
    // 必须替换**全部**出现处 —— 只换第一个会漏掉其余调用点，负控就成假电。
    copy.set(path, text.split(from).join(to));
    return copy;
  };

  const cases = [
    {
      name: "B1：在别处复制一份默认常量",
      run: () => runAll(mutate(AUTHORITY_FILE, "pub const DEFAULT_SOFT_HEADER_TIMEOUT_SECS: u64 = 45;", "pub const DEFAULT_SOFT_HEADER_TIMEOUT_SECS: u64 = 45;\npub const DEFAULT_HARD_HEADER_TIMEOUT_SECS: u64 = 90;"), frontend),
      expect: "B1",
    },
    {
      name: "B2：摘掉 gemini 路径的预算调用",
      run: () => {
        const copy = new Map(sources);
        const p = "crates/providers/src/gemini.rs";
        copy.set(p, (copy.get(p) ?? "").replace("send_with_header_budget", "legacy_send"));
        return runAll(copy, frontend);
      },
      expect: "B2",
    },
    {
      name: "B3：把旧闸门公式放回 openai.rs",
      run: () => runAll(mutate("crates/providers/src/openai.rs", "let budget =", "let response_headers_timeout = std::time::Duration::from_secs((10 + (body_size / (64 * 1024)) as u64 * 5).min(60));\n            let budget ="), frontend),
      expect: "B3",
    },
    {
      name: "B4：消费点不再匹配错误短语",
      run: () => runAll(mutate(CONSUMERS[0], ERR_PHRASE, "response headers gave up"), frontend),
      expect: "B4",
    },
    {
      name: "B5：主链退回旧的 ctx 构造（面板填了不生效）",
      run: () => runAll(mutate("crates/rt-workflow/src/work_engine/executors/agent_executor.rs", "build_provider_request_context_for_model", "build_provider_request_context"), frontend),
      expect: "B5",
    },
  ];

  let failed = 0;
  for (const c of cases) {
    const bad = c.run();
    const fired = bad.some((b) => b.rule === c.expect);
    const others = bad.filter((b) => b.rule !== c.expect);
    console.log(
      `${fired ? "OK  " : "FAIL"} ${c.name} → 期望 ${c.expect} 触发；实得 ${bad.length} 处违规` +
        (others.length ? `（另有 ${others.length} 处：${[...new Set(others.map((b) => b.rule))].join(",")}）` : ""),
    );
    if (!fired) failed += 1;
  }
  if (failed) {
    console.error(`\n结论: ${failed} 个负控未触发 —— 门有电性缺口`);
    process.exit(1);
  }
  console.log("\n结论: 全部负控均被本门捕获");
  process.exit(0);
}

// ── 入口 ────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
if (args.includes("--selftest")) {
  selftest();
}

const sources = loadSources();
const frontend = loadFrontendSources();

if (args.includes("--dump")) {
  for (const rule of RULES) {
    const bad = rule.check(sources, frontend);
    console.log(`${rule.id} ${rule.desc} → ${bad.length} 处违规`);
    for (const b of bad) console.log(`  · ${b.msg}`);
  }
  console.log(`扫描 .rs 文件数: ${sources.size}`);
  process.exit(0);
}

const bad = runAll(sources, frontend);
const grouped = new Map();
for (const b of bad) {
  if (!grouped.has(b.rule)) grouped.set(b.rule, []);
  grouped.get(b.rule).push(b);
}
for (const rule of RULES) {
  const list = grouped.get(rule.id) || [];
  if (!list.length) {
    console.log(`OK  ${rule.id} ${rule.desc}`);
    continue;
  }
  console.log(`\n${rule.id} ${rule.desc} —— ${list.length} 处`);
  for (const b of list) console.log(`  · ${b.msg}`);
}
if (!bad.length) {
  console.log("\n结论: 首字节预算口径一致（五条路径同源、权威唯一、旧闸门已退役、可配置链路接通）");
  process.exit(0);
}
console.log(`\n结论: ${bad.length} 处违规`);
process.exit(1);
