#!/usr/bin/env node
/**
 * 「产出 → 被消费」门（C2，`PLAN-chain-closure-remediation.md` §4）。
 *
 * ## 为什么要有这道门
 * 本仓现有门禁全部停在**定义点**：`check-orphan-modules.mjs` 自陈「门绿 ≠ 全仓无死码 ——
 * 本门只判有没有被编译，不判编译了但没人调」。于是一类缺陷可以长绿：
 * 函数在、编得过、测试也过，但**生产调用方永远给它传空**。
 * 实证（2026-10-20 三链路核查）：`evidence_weight` 的 `historical_weights` 入参在 Rust 侧
 * 只有定义 + 测试给过值，生产唯一注入口是前端 `computeEvidenceDrivenConsensus` 的第 4 参，
 * 而调用点硬传 `null` ⇒ 进化学到的权重永远进不了个股共识。这道门若早在，那天就红。
 *
 * ## 规则
 *  · R1 登记表非空，且每条的**生产者锚点**与**消费者锚点**都必须在代码域里定位得到；
 *     定位不到即红（改名/搬走 ⇒ 门不许退化成「查不到 = 没问题」）。
 *  · R2 消费侧必须真的读到该产物：只认**代码域**字面量（剥注释；注释里提一句不算消费）。
 *     带 `nullArgIndex` 的面更严 —— 调用块里那一位是字面量 `null` 即判「从未被消费」。
 *  · R3 一个条目的多个消费面各须独立成立（少一面 = 那条链继续断）。
 *  · R4 显式豁免当**完成计数器**用：豁免条目若已全接通 ⇒ 红「应撤豁免」。
 *     豁免不改变红绿（否则一道门可以永久拖着 CI），但**每次运行都打印**其日期与理由。
 *
 * ## 自证（`--selftest`）
 * 判据是纯函数、读文件由调用方注入 ⇒ 破坏样本**不落盘**（往合成文件集里喂），
 * 每个样本只让它自己那一格红：M1 变量注册锚点消失 / M2 提示词占位消失 /
 * M3 权重消费面字面量消失 / M4 豁免条目被接通 ⇒ 改口要求撤豁免 /
 * M5 键名只出现在注释里 **不得**算消费 / M6 登记表为空 ⇒ R1 红 /
 * M7 消费面整份文件消失 ⇒ R1 红（不是 R2，定位与语义要分得开）。
 * M9 面板撤掉按轮次读回 ⇒ 只红 mover_recall_runs / M10 进化聚合撤掉轮次表读取 ⇒ 红在另一面。
 *
 * 用法：node scripts/check-reflection-output-consumers.mjs [--selftest] [--dump]
 * 退出码：0 = 通过（豁免条目按登记如实豁免并打印）；1 = 有违规；2 = 参数错误。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

// ── 登记表：产物 → 生产锚点 → 消费锚点 ──────────────────────────────────────
// 每条都要能一句话说清「哪段代码读它」；说不出来的条目不该登记在这里。
const ENTRIES = [
  {
    key: "reco_strategy_weights",
    why: "荐股 IC 闭环产出的逐格权重，必须被荐股/智选出票评分读取",
    producer: {
      file: "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs",
      literal: "reco_strategy_weights",
    },
    consumers: [
      {
        file: "src-tauri/crates/analysis-engine/src/recommender/mod.rs",
        literal: "reco_strategy_weights",
        note: "按 key 解析 served vars 里被闭环覆盖后的权重表",
      },
      {
        file: "src-tauri/src/commands/stock_workflow/serenity.rs",
        literal: "load_reco_served_vars_db",
        note: "趋势智选取同一份 served vars（口径统一，2026-10-02）",
      },
    ],
  },
  {
    key: "stock_lessons",
    why: "反思产出的教训必须进下一次分析的提示词",
    producer: { file: "src-tauri/src/commands/stock_workflow/hooks.rs", literal: "stock_lessons" },
    consumers: [
      {
        file: "src-tauri/agency_experts/stock-analysis/trader.md",
        literal: "{{stock_lessons}}",
        note: "提示词槽位真实存在（不是只在代码里拼）",
      },
      {
        file: "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs",
        literal: "stock_lessons",
        note: "变量注册在案",
      },
    ],
  },
  {
    key: "serenity_validation_summary",
    why: "智选后验的只读汇总必须真被面板调用（B1 之前它连命令都没有，产物只进 tracing 日志）",
    producer: {
      file: "src-tauri/src/commands/backtest_validation.rs",
      literal: "pub async fn serenity_validation_summary",
    },
    consumers: [
      {
        file: "src/components/stock-analysis/SerenityValidationPanel.tsx",
        literal: "serenity_validation_summary",
        note: "面板 invoke 该命令，且 generation 跟着时间锚点传（不是硬传 null 的死参）",
      },
    ],
  },
  {
    key: "mover_recall_runs",
    why: "核查轮次落库后必须被按轮次读回 —— 只写不读就是「任务中心点了立即执行，界面看不到那一次的结果」（2026-10-20 实机反馈）",
    producer: {
      file: "src-tauri/src/commands/mover_recall.rs",
      literal: "persist_mover_recall_run",
    },
    consumers: [
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "get_mover_recall_run_view",
        note: "面板按 runId 读回那一轮的载荷（D-5，不是现场重算）",
      },
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "list_mover_recall_runs",
        note: "轮次下拉的真取数点（列表为空时面板另有成句，不会假称「本轮无事件」）",
      },
      {
        file: "src/pages/TasksCenterPage/ScheduledJobsTab.tsx",
        literal: "MOVER_RUN_ID_PATTERN",
        note: "执行历史把 run_id 提成深链，跳回面板并锁定该轮",
      },
      {
        file: "src-tauri/src/commands/evolution_proposal.rs",
        literal: "list_runs_since",
        note: "20:00 进化聚合读同一张轮次表（第二个消费者，与面板互不顶替）",
      },
    ],
  },
  {
    key: "mover_recall_events.asof_diff",
    why: "as-of 荐股回放的差异证据必须既进反思提示词、又在面板可见 —— 只落库不读等于「回放跑完没人看」",
    producer: {
      file: "src-tauri/src/commands/mover_recall_analysis.rs",
      literal: "asof_diff_json",
    },
    consumers: [
      {
        file: "src-tauri/src/commands/stock_workflow/reflection.rs",
        literal: "asof_evidence_block",
        note: "反思提示词把 as-of 层/池/截断/分歧逐项喂给模型（与 live 层分列）",
      },
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "asofDiffJson",
        note: "事件详情展示 as-of 层与池/出票/截断（与 live 层并列）",
      },
    ],
  },
  {
    key: "mover_recall_runs.asof_diff_summary",
    why: "轮级差异摘要是荐股闭环的数值信号来源（C-1/C-5），必须被命令与面板同时读到",
    producer: {
      file: "src-tauri/crates/dao/src/repo/mover_recall_runs.rs",
      literal: "asof_diff_summary_json",
    },
    consumers: [
      {
        file: "src-tauri/src/commands/stock_analysis.rs",
        literal: "get_mover_asof_summary",
        note: "命令读最新一轮的六桶摘要（错误/缺失各自成句）",
      },
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "get_mover_asof_summary",
        note: "面板首屏渲染 as-of 摘要行",
      },
    ],
  },
  {
    key: "stock_reflections.asof_layer",
    why: "逐档反思行的 as-of 归因是面板过滤与复核的权威列（miss_layer 历史列混有两种口径）",
    producer: {
      file: "src-tauri/src/commands/stock_workflow/reflection.rs",
      literal: "Column::AsofLayer",
    },
    consumers: [
      {
        file: "src-tauri/crates/dao/src/repo/stock_lesson_queries.rs",
        literal: "asofLayer",
        note: "反思行投影带上 as-of 层与批次键",
      },
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "asofLayer",
        note: "面板把 as-of 层与 live 层分列显示（缺证据时另有成句）",
      },
    ],
  },
  {
    key: "historical_weights",
    why: "进化/闭环学到的权重必须进个股证据共识",
    producer: {
      file: "src-tauri/crates/analysis-engine/src/evidence_weight.rs",
      literal: "historical_weights",
    },
    consumers: [
      {
        file: "src/stores/feature/stockAnalysisStore.ts",
        literal: "computeEvidenceDrivenConsensus",
        note: "生产唯一调用点；第 4 参传字面量 null 即视为未被消费",
        nullArgIndex: 3,
      },
    ],
    // ⚠ 显式豁免（带日期，失效即红）。理由不是「还没做」，而是**前提不成立**：
    // 该入参的键域是 analyst_id，而本仓唯一的历史权重产物 `strategy_weight_history`
    // 的键域是 (style_id, period) —— 两域不相交，硬接只会让每条查找恒落 1.0，
    // 把「已接通」演给门禁看（比现在的 null 更坏）。撤豁免的前提是先有一个
    // **按分析师**的历史表现权威源。详见 PLAN 的 C1 条目。
    exempt: {
      since: "2026-10-20",
      reason: "键域不相交（analyst_id vs style×period），本仓无分析师级历史权重权威源",
    },
  },
  {
    key: "mover_recall_events.miss_layer",
    why: "漏检归因层是三条链优化的唯一定位依据 —— 只落库不读，反思与面板都只会说「有漏检」而说不出「为什么漏」",
    producer: {
      file: "src-tauri/src/commands/mover_recall.rs",
      literal: "miss_layer",
    },
    consumers: [
      {
        file: "src-tauri/src/commands/stock_workflow/reflection.rs",
        literal: "miss_layer",
        note: "漏检专项反思把层名与三问一起注入提示词（attr 层不可判时如实写「无法归因」）",
      },
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "missLayer",
        note: "面板逐行显示层，并按层计数分组（A-2 后每链各自成表）",
      },
    ],
  },
  {
    key: "stock_reflections.mover_label",
    why: "妖股标签是「命中/漏检/按时/未达标」四态的唯一落点；只写不读等于反思里最贵的一段结论没人用",
    producer: {
      file: "src-tauri/src/commands/stock_workflow/reflection.rs",
      literal: "mover_label",
    },
    consumers: [
      {
        file: "src/components/stock-analysis/ReflectionPanel.tsx",
        literal: "moverLabel",
        note: "反思列表把四态渲染成标签（值域由 `mover_label_for` 单源，前端不再自判）",
      },
      {
        file: "src-tauri/src/commands/evolution_proposal.rs",
        literal: "mover_label",
        note: "20:00 进化聚合按标签分池统计（漏检/命中不能混成一个成功率）",
      },
    ],
  },
  {
    key: "mover_recall_events.reflow_pending",
    why: "反思判定「本应入池」的结论必须既可复核又被消费 —— 只写清单不消费＝又一条只写不读的链",
    producer: {
      file: "src-tauri/src/commands/stock_workflow/reflection.rs",
      literal: "mark_reflow_pending",
    },
    consumers: [
      {
        file: "src-tauri/src/commands/stock_analysis.rs",
        literal: "refresh_reflow_seed",
        note: "荐股扫描前读清单，按「人工开关 + 样本下限」决定是否并入种子（默认 shadow）",
      },
      {
        file: "src/components/stock-analysis/MoverRecallPanel.tsx",
        literal: "list_mover_reflow_candidates",
        note: "面板三态展示候选与转正就绪度（可复核）",
      },
    ],
  },
  {
    key: "MoverCellSignal",
    why: "漏检降权信号是荐股闭环「只降不升」的输入；不消费则 `l3_scored_out` 的账永远不落到权重上",
    producer: {
      file: "src-tauri/crates/analysis-engine/src/mover_recall.rs",
      literal: "pub async fn load_mover_cell_signal",
    },
    consumers: [
      {
        file: "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs",
        literal: "MoverCellSignal",
        note: "compute_loop_cell_weights 逐格取 penalty（按 (style, period) 累加分子分母）",
      },
    ],
  },
];

// ── 剥注释（只认代码域）──────────────────────────────────────────────────
/**
 * 逐行剥离行注释与块注释。刻意**不**处理字符串内的 `//`（URL 之类）——
 * 本门判据字面量常出现在字符串里（提示词占位符就是），宁可宽报也不把代码一起吞掉；
 * M5 负控专门锁「键名只在注释里 ⇒ 不得算消费」。
 */
function codeOnly(src) {
  const out = [];
  let inBlock = false;
  for (const line of src.split("\n")) {
    const t = line.trim();
    if (inBlock) {
      if (t.includes("*/")) inBlock = false;
      continue;
    }
    if (t.startsWith("/*")) {
      if (!t.includes("*/")) inBlock = true;
      continue;
    }
    if (t.startsWith("//") || t.startsWith("*")) continue;
    const ci = t.indexOf(" //");
    out.push(ci >= 0 ? t.slice(0, ci) : t);
  }
  return out.join("\n");
}

/** 从调用点起按括号深度配平，取出整个调用块文本。 */
function callBlock(text, callee) {
  const at = text.indexOf(callee + "(");
  if (at < 0) return null;
  let depth = 0;
  for (let i = at + callee.length; i < text.length; i += 1) {
    const c = text[i];
    if (c === "(") depth += 1;
    else if (c === ")") {
      depth -= 1;
      if (depth === 0) return text.slice(at, i + 1);
    }
  }
  return null;
}

/** 按顶层逗号切实际参数（跳过嵌套的 () [] {}）。 */
function splitArgs(block) {
  const inner = block.slice(block.indexOf("(") + 1, block.lastIndexOf(")"));
  const args = [];
  let depth = 0;
  let cur = "";
  for (const c of inner) {
    if (c === "(" || c === "[" || c === "{") depth += 1;
    else if (c === ")" || c === "]" || c === "}") depth -= 1;
    if (c === "," && depth === 0) {
      args.push(cur.trim());
      cur = "";
      continue;
    }
    cur += c;
  }
  if (cur.trim()) args.push(cur.trim());
  return args;
}

// ── 判据（纯函数；读文件动作由调用方注入，负控因此不必碰工作树）──────────────
/** @returns {{violations: string[], exemptions: string[]}} */
/**
 * R5 语义断言（2026-10-24，PLAN G-6）：登记表管「谁读它」，这里管「读法对不对」的三条
 * 曾被实际踩到过的形态 —— 它们都不是「少一个调用」而是「接上了但语义被覆盖」。
 */
const EVENTS_DAO = "src-tauri/crates/dao/src/repo/mover_recall_events.rs";
const REFLECTION = "src-tauri/src/commands/stock_workflow/reflection.rs";
const SEMANTIC_CHECKS = [
  {
    name: "盖章不覆盖回灌清单",
    file: EVENTS_DAO,
    fnAnchor: "pub async fn stamp_reflected",
    forbidden: ["ReflowPending"],
  },
  {
    name: "事件档反思行必须真被合并",
    file: REFLECTION,
    need: ["merge_extra_horizons"],
  },
  {
    name: "差异证据必须进反思提示词",
    file: REFLECTION,
    need: ["asof_evidence_block"],
  },
];

/** 取代码域里某个锚点之后的窗口（函数体足够短时等价于「该函数内」）。 */
function codeWindow(code, anchor, len = 1200) {
  const at = code.indexOf(anchor);
  return at === -1 ? null : code.slice(at, at + len);
}

function judge(entries, readFile) {
  const violations = [];
  const exemptions = [];
  if (!entries.length) {
    return { violations: ["R1 登记表为空 ⇒ 本门恒绿，等于没有门"], exemptions: [] };
  }
  for (const e of entries) {
    const local = [];
    const psrc = readFile(e.producer.file);
    if (psrc === null) {
      violations.push(`R1 ${e.key}：生产者文件定位不到 ${e.producer.file}`);
      continue;
    }
    if (!codeOnly(psrc).includes(e.producer.literal)) {
      violations.push(`R1 ${e.key}：生产者锚点 ${e.producer.literal} 不在 ${e.producer.file} 的代码域里`);
      continue;
    }

    let wired = 0;
    for (const c of e.consumers) {
      const src = readFile(c.file);
      if (src === null) {
        local.push(`R1 ${e.key}：消费面文件定位不到 ${c.file}（${c.note}）`);
        continue;
      }
      const code = codeOnly(src);
      if (!code.includes(c.literal)) {
        local.push(`R3 ${e.key}：消费面锚点 ${c.literal} 不在 ${c.file} 的代码域里（${c.note}）`);
        continue;
      }
      if (c.nullArgIndex === undefined) {
        wired += 1;
        continue;
      }
      const block = callBlock(code, c.literal);
      if (!block) {
        local.push(`R3 ${e.key}：${c.file} 里找不到 ${c.literal}( 调用块`);
        continue;
      }
      const args = splitArgs(block);
      const arg = args[c.nullArgIndex];
      if (arg === undefined) {
        local.push(`R3 ${e.key}：${c.literal} 调用只有 ${args.length} 个实际参数，取不到第 ${c.nullArgIndex + 1} 个`);
        continue;
      }
      if (arg === "null" || arg === "undefined") {
        local.push(
          `R2 ${e.key}：${c.file} 把第 ${c.nullArgIndex + 1} 参硬传 ${arg} ⇒ 该产物从未进入消费方`,
        );
        continue;
      }
      wired += 1;
    }

    if (e.exempt) {
      if (wired === e.consumers.length) {
        // 豁免是完成计数器，不是免检牌：接通了还不撤，这条豁免就会替未来的静默失败说话。
        violations.push(`R4 ${e.key}：豁免（自 ${e.exempt.since}）已失效 —— 消费面全部接通，请撤销豁免`);
      } else {
        exemptions.push(`${e.key}：自 ${e.exempt.since} 豁免，理由：${e.exempt.reason}`);
      }
      continue;
    }
    violations.push(...local);
  }
  for (const c of SEMANTIC_CHECKS) {
    const src = readFile(c.file);
    if (src === null) {
      violations.push(`R5 ${c.name}：文件定位不到 ${c.file}`);
      continue;
    }
    const code = codeOnly(src);
    if (c.fnAnchor) {
      const win = codeWindow(code, c.fnAnchor);
      if (win === null) {
        violations.push(`R5 ${c.name}：${c.file} 里找不到 ${c.fnAnchor}`);
        continue;
      }
      for (const bad of c.forbidden ?? []) {
        if (win.includes(bad)) {
          violations.push(
            `R5 ${c.name}：${c.fnAnchor} 内出现 ${bad} ⇒ 盖章会把回灌清单覆盖掉（清单恒空）`,
          );
        }
      }
    }
    for (const need of c.need ?? []) {
      if (!code.includes(need)) {
        violations.push(`R5 ${c.name}：${c.file} 代码域缺 ${need}`);
      }
    }
  }
  return { violations, exemptions };
}

// ── 自证 ────────────────────────────────────────────────────────────────────
function collectFiles() {
  const set = new Set();
  for (const e of ENTRIES) {
    set.add(e.producer.file);
    for (const c of e.consumers) set.add(c.file);
  }
  // R5 语义断言单独读的文件也要进合成集，否则任何一条破坏样本都会额外红一条
  // 「文件定位不到」，把「只红这一格」的负控变成噪声。
  for (const c of SEMANTIC_CHECKS) set.add(c.file);
  return set;
}

function realReader() {
  return (rel) => {
    const p = path.join(ROOT, rel);
    return fs.existsSync(p) ? fs.readFileSync(p, "utf8") : null;
  };
}

/** 在生产真文上做**单点**破坏，喂给同一批判据；不落盘、不碰工作树。 */
function synthetic(mutate) {
  const files = new Map();
  const read = realReader();
  for (const rel of collectFiles()) files.set(rel, read(rel) ?? "");
  mutate(files);
  return (rel) => (files.has(rel) ? files.get(rel) : null);
}

const PANEL = "src/components/stock-analysis/SerenityValidationPanel.tsx";
const STORE = "src/stores/feature/stockAnalysisStore.ts";
const MOD = "src-tauri/crates/analysis-engine/src/recommender/mod.rs";
const SEEDVARS = "src-tauri/src/commands/stock_analysis_setup/seed_variables.rs";
const TRADER = "src-tauri/agency_experts/stock-analysis/trader.md";
const MR_PANEL = "src/components/stock-analysis/MoverRecallPanel.tsx";
const SCHED = "src/pages/TasksCenterPage/ScheduledJobsTab.tsx";
const EVO = "src-tauri/src/commands/evolution_proposal.rs";

function selftest() {
  const cases = [
    {
      name: "P0 真实树：仅豁免条目被免除，其余全部有电",
      read: synthetic(() => {}),
      want: (r) => r.violations.length === 0 && r.exemptions.length === 1 && r.exemptions[0].includes("historical_weights"),
    },
    {
      name: "M1 变量注册锚点消失 ⇒ 只红 stock_lessons 一条",
      read: synthetic((f) => f.set(SEEDVARS, f.get(SEEDVARS).replaceAll("stock_lessons", "unrelated_key"))),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("stock_lessons") && r.violations[0].startsWith("R3"),
    },
    {
      name: "M2 提示词占位消失 ⇒ 只红 stock_lessons（且是消费面那条）",
      read: synthetic((f) => f.set(TRADER, f.get(TRADER).replaceAll("{{stock_lessons}}", "{{other_slot}}"))),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("stock_lessons") && r.violations[0].includes("trader.md"),
    },
    {
      name: "M3 权重消费面字面量消失 ⇒ 只红 reco_strategy_weights",
      read: synthetic((f) => f.set(MOD, f.get(MOD).replaceAll("reco_strategy_weights", "zzz_unrelated"))),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("reco_strategy_weights"),
    },
    {
      name: "M4 豁免条目被接通 ⇒ 必须改口要求撤豁免",
      read: synthetic((f) =>
        f.set(STORE, "const report = computeEvidenceDrivenConsensus(reports, regime, horizon, loadedWeights);\n"),
      ),
      want: (r) => r.violations.length === 1 && r.violations[0].startsWith("R4") && r.violations[0].includes("historical_weights") && r.exemptions.length === 0,
    },
    {
      name: "M5 键名只出现在注释里 ⇒ 不得算消费",
      read: synthetic((f) => f.set(MOD, "// 这里只是注释提到 reco_strategy_weights\nexport const unrelated = 1;\n")),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("reco_strategy_weights") && r.violations[0].startsWith("R3"),
    },
    {
      name: "M6 登记表为空 ⇒ R1 红（防恒绿）",
      read: realReader(),
      entries: [],
      want: (r) => r.violations.length === 1 && r.violations[0].startsWith("R1"),
    },
    {
      name: "M7 消费面整份文件消失 ⇒ 报 R1 定位失败，不报 R2 语义缺失",
      read: synthetic((f) => f.delete(MOD)),
      want: (r) => r.violations.length === 1 && r.violations[0].startsWith("R1") && r.violations[0].includes("定位不到"),
    },
    {
      name: "M8 智选后验面板不再调用该命令 ⇒ 只红这一条（B1 的接线不许被悄悄拆掉）",
      read: synthetic((f) => f.set(PANEL, f.get(PANEL).replaceAll("serenity_validation_summary", "some_other_command"))),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("serenity_validation_summary"),
    },
    {
      name: "M9 面板不再按轮次读回 ⇒ 只红 mover_recall_runs（且点名轮次读回那条消费面）",
      read: synthetic((f) => f.set(MR_PANEL, f.get(MR_PANEL).replaceAll("get_mover_recall_run_view", "zzz_gone"))),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("mover_recall_runs") && r.violations[0].includes("get_mover_recall_run_view"),
    },
    {
      name: "M11 事件档反思合并被撤 ⇒ 只红 R5 的那一条语义断言",
      read: synthetic((f) => f.set(REFLECTION, f.get(REFLECTION).replaceAll("merge_extra_horizons", "zzz_gone"))),
      want: (r) =>
        r.violations.length === 1 &&
        r.violations[0].startsWith("R5") &&
        r.violations[0].includes("事件档反思行"),
    },
    {
      name: "M10 进化聚合撤掉轮次表读取 ⇒ 同一条目红在另一面（两个消费者各须独立成立）",
      read: synthetic((f) => f.set(EVO, f.get(EVO).replaceAll("list_runs_since", "zzz_gone"))),
      want: (r) => r.violations.length === 1 && r.violations[0].includes("mover_recall_runs") && r.violations[0].includes("list_runs_since"),
    },
  ];

  let bad = 0;
  for (const c of cases) {
    const r = judge(c.entries ?? ENTRIES, c.read);
    if (!c.want(r)) {
      bad += 1;
      console.error(`❌ ${c.name} —— 实得 violations=${JSON.stringify(r.violations)} exemptions=${JSON.stringify(r.exemptions)}`);
    } else {
      console.log(`✅ ${c.name}`);
    }
  }
  if (bad) {
    console.error(`自检未通过：${bad} 例`);
    process.exit(1);
  }
  console.log(`自检通过（${cases.length} 例，判据不空转）`);
}

// ── 主流程 ──────────────────────────────────────────────────────────────────
function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    selftest();
    return;
  }
  if (args.includes("--dump")) {
    for (const e of ENTRIES) {
      console.log(`${e.key} —— ${e.why}\n  生产：${e.producer.file} :: ${e.producer.literal}`);
      for (const c of e.consumers) {
        const extra = c.nullArgIndex !== undefined ? `（第 ${c.nullArgIndex + 1} 参须非 null）` : "";
        console.log(`  消费：${c.file} :: ${c.literal}${extra}`);
      }
    }
    return;
  }
  const { violations, exemptions } = judge(ENTRIES, realReader());
  for (const x of exemptions) console.log(`· 已登记豁免：${x}`);
  if (violations.length) {
    console.error(`❌ 违规 ${violations.length} 条：`);
    for (const v of violations) console.error(`  · ${v}`);
    console.error("判据口径：只认代码域；「文件在、函数在、参数恒 null」就是没消费。");
    process.exit(1);
  }
  console.log(`✅ 通过（登记 ${ENTRIES.length} 条产物 → 消费面，豁免 ${exemptions.length} 条已如实打印）`);
}

main();
