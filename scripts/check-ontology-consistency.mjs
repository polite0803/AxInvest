#!/usr/bin/env node
/**
 * check-ontology-consistency.mjs — 领域本体「权威源 ↔ 副本」一致性门禁
 *
 * 存在理由
 * --------
 * `harness/src/domain_ontology.rs` 声明了瓶颈三力的**唯一权威源**（权重 / 分档阈值 / 公理）。
 * 但 `.rhai` 脚本**读不到 Rust 常量**（只能注入模板变量），所以运行时必须保留一份 fallback；
 * prompt 与注释里也各有一份散文口径。
 *
 * ⇒ **声明了权威源 ≠ 副本会跟着走。** 没有比对机制时，改一处、另两处不动，
 * 结果是「同一个概念在不同节点上给出不同分数」——而**没有任何东西会红**。
 * 本脚本就是那个比对机制。
 *
 * 分两档（刻意的，勿合并）
 * ----------------------
 * | 档 | 对象 | 处置 |
 * |---|---|---|
 * | **硬拦** | 可执行字面量（`.rhai` 顶层权重兜底 / **三力内部权重** / 分档阈值）、接线存在性、站点清单完整性、**引用类字段的指向必须真实存在** | 不一致 ⇒ **exit 1** |
 * | **报告** | 口径分歧（`OPEN_DIVERGENCES`：定义式 / 分档数值）与散文注释 | 只打印，**不判失败** |
 *
 * 覆盖的**四类**现场（2026-09-15 扩充后）：
 *  1. 顶层权重（`BOTTLENECK_WEIGHTS`）在 2 个 `.rhai` 里的兜底字面量；
 *  2. 分档阈值（`READINESS_BANDS`）在 2 个 `.rhai` 里的 `>=` 字面量；
 *  3. **三力内部权重**（`*_PARTS`）在 2 个 `.rhai` 里的加权和字面量
 *     —— 扩充前 `parseParts()` 的返回值只进 null 检查 / 自洽校验 / info 打印，
 *     **从未进入 `problems.push`**（判据 #197「悬空解析」）；
 *  4. **权威源自身的引用类字段**：`Axiom::enforced_by` 指向的 `#[test] fn` 必须存在、
 *     `evidence` / `Divergence::site` 指向的文件必须存在。
 *     这三个字段都是 `&'static str` 自由文本，Rust 侧断言只查「非空」⇒
 *     改名/删除后「指向的东西不存在」与「指向的东西通过了」在门禁看来是同一件事。
 *
 * 为什么口径分歧不硬拦：两侧可能都在「对」的位置上（LLM 产出 vs 脚本复算），
 * 消除它需要**产品裁决**。硬拦会逼人删掉登记项 —— 信号不是被解决，是被消灭
 * （与 `check-domain-semantics.mjs` 的 `--strict-plan` 同一条纪律）。
 * 需要时用 `--strict-divergence` 把报告档也纳入失败。
 *
 * 设计纪律（对应铁律 7「审计脚本自身会撒谎」）
 * ------------------------------------------
 *  1. **权威源解析不出来 ⇒ exit 3**，绝不静默按 0 条通过（否则门禁变成永久绿灯）。
 *  2. **站点清单是契约**：`SITES` 里声明的每个站点若一条都抓不到 ⇒ exit 1 并提示更新清单。
 *     抓不到 = 代码被重构了（或模式腐烂），**不是「没问题」**。
 *  3. **数字渲染约定两侧必须一致**：Rust `format!("{v:.2}")` ↔ 本脚本 `toFixed(2)`。
 *     `format!("{}", 0.30)` 会给 `"0.3"` —— 那会让种子产物静默变化。
 *  4. `--selftest` 全用**纯函数 + 畸形输入**做正负对照，含「比较器必须能判失败」。
 *  5. **扫到 0 个文件 ⇒ 非 0 退出**。
 *  6. **只验到文件级、不验行号**：行号必然随重构漂移（判据 #186–#189），
 *     锁行号会把门禁变成噪声源。`evidence` 的 `:行号` 尾巴一律剥掉再判存在性。
 *
 * 已知残余缺口（**刻意保留，勿「顺手修」**）
 * --------------------------------------
 *  · 三力内部权重只比**权重序列**、不比**分项名**：同一条式子里两个同值权重互换
 *    （如 supply 的 `0.30/0.40/0.30` 首尾对调）检测不到。理由见 `compareForceParts` 文档。
 *  · 公理的守护者全是**本模块单元测试**，无生产消费者 ⇒ 通过只代表「常量表自洽」，
 *    **不代表** `.rhai` 真按公理计算（详见 `domain_ontology.rs` 模块头的接线状态表）。
 *  · 权重数字只保证「没抄错」，不保证「被消费」—— 本体在 Rust 侧的生产消费方仍然是 0。
 *
 * 用法
 * ----
 *   node scripts/check-ontology-consistency.mjs                    # 门禁（含硬拦）
 *   node scripts/check-ontology-consistency.mjs --list             # 同上，**额外**打印引用类现场（不改判据）
 *   node scripts/check-ontology-consistency.mjs --strict-divergence # 口径分歧也拦（人工巡检）
 *   node scripts/check-ontology-consistency.mjs --selftest
 *   node scripts/check-ontology-consistency.mjs --json
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

const argv = process.argv.slice(2);
const ARG = (k) => argv.some((a) => a === `--${k}` || a.startsWith(`--${k}=`));
const LIST = ARG("list");
const JSON_OUT = ARG("json");
const SELFTEST = ARG("selftest");
const STRICT_DIV = ARG("strict-divergence");

const EPS = 1e-9;
const near = (a, b) => Math.abs(a - b) < EPS;

// ── 权威源 ────────────────────────────────────────────────────────────
const AUTHORITY_REL = "src-tauri/crates/harness/src/domain_ontology.rs";

// ── 副本站点清单（**契约**：抓不到 ⇒ 失败，不是通过）──────────────────────
/**
 * 每个站点声明「文件 + 抽取器 + 期望条数」。
 * 期望条数是**声明集合**：与 `check-contracts.mjs` 的 `SILENT_RESULT_BASELINE` 同一思路 —
 * 数量对不上说明代码结构变了，必须人工确认（可能是好事，也可能正是漏抓）。
 */
const SITES = [
  {
    file: "src-tauri/src/commands/bottleneck-calc.rhai",
    kind: "rhai-weight-fallback",
    expect: 3,
    note: "顶层权重兜底（w_supply/w_demand/w_irreplace）",
  },
  {
    file: "src-tauri/src/commands/strategy-scorer.rhai",
    kind: "rhai-weight-fallback",
    expect: 3,
    note: "第二份脚本，独立一份兜底",
  },
  {
    file: "src-tauri/src/commands/bottleneck-calc.rhai",
    kind: "rhai-band-from-ontology",
    note: "分档来自本体 band_for_score（readiness_signal 一行，无独立阈值字面量）",
  },
  {
    file: "src-tauri/src/commands/strategy-scorer.rhai",
    kind: "rhai-band-from-ontology",
    note: "readiness_signal 经 band_for_score 读阈值（§7-a b1，档名映射回 _signal，6 处产出不改）",
  },
  // ── 三力**内部**权重（`*_PARTS`）→ Rust 权威函数收敛后的调用断言 ─────
  //
  // 2026-09-20 收敛后：两个 `.rhai` 不再内联 `let xxx_score = ... * 0.30...+...`
  // 计分行了，三力内部权重（0.30/0.40/0.30、0.60/0.40、0.30/0.40/0.30）的唯一
  // 权威实现迁到 Rust 的 `stock_workflow/rhai_bottleneck.rs::bottleneck_node_score`。
  //
  // 但注意：**真实权威算分已不在本体 `*_PARTS` 里被消费**（本体侧生产消费方本为 0，
  // 见 `AUDIT-ontology-vs-palantir-2026-09-15.md` §2.2）。收敛后用户真正跑在 Arc 上的是
  // Rust 侧的字面量 —— 为保证这点，脚本还硬校验 Rust 源码里的内部权重字面量就是权威值
  // （见下方 `rhai-calls-shared` 分支与 `checkRustForcePartsLiterals`）。
  {
    file: "src-tauri/src/commands/bottleneck-calc.rhai",
    kind: "rhai-calls-shared",
    note: "三力内部权重已收敛进 Rust 权威函数 bottleneck_node_score（rhai_bottleneck.rs），脚本只负责调用与拼装输出",
  },
  {
    file: "src-tauri/src/commands/strategy-scorer.rhai",
    kind: "rhai-calls-shared",
    note: "同上：三力内部权重收敛进 bottleneck_node_score，脚本改为调用共享权威函数",
  },
];

/** 接线存在性检查 —— 「接了线」与「真接活」是两件事（L1 的教训）。 */
const WIRING = [
  {
    file: "src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs",
    mustContain: ["domain_ontology::BOTTLENECK_WEIGHTS"],
    note: "seeder 必须引用权威源，而不是自己再写一份",
  },
  {
    file: "src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs",
    mustContain: ["domain_ontology::format_weight(bw.supply)"],
    note: "描述文本的数字必须经 format_weight 渲染（否则尾零丢失会改种子产物）",
  },
  // 权重默认值「不得回退成硬编码」由 `findHardcodedWeightValues()` 单独检查
  // （跨行结构 + 必须验锚点存在，单行正则会误报无关变量的 `json!(0.0)`）。
];

// ── 纯函数：解析 ──────────────────────────────────────────────────────

/** 解析 `BOTTLENECK_WEIGHTS`。返回 null 表示**解析失败**（调用方必须据此失败）。 */
export function parseWeights(text) {
  const m =
    /pub const BOTTLENECK_WEIGHTS:\s*BottleneckWeights\s*=\s*\n?\s*BottleneckWeights\s*\{\s*supply:\s*([0-9.]+)\s*,\s*demand:\s*([0-9.]+)\s*,\s*irreplaceability:\s*([0-9.]+)\s*\}/.exec(
      text,
    );
  if (!m) return null;
  return { supply: Number(m[1]), demand: Number(m[2]), irreplaceability: Number(m[3]) };
}

/** 解析某个 `ForceParts` 常量（形如 `pub const X: ForceParts = &[("a", 0.30), …];`）。 */
export function parseParts(text, name) {
  const head = new RegExp(`pub const ${name}:\\s*ForceParts\\s*=\\s*\\n?\\s*&\\[`).exec(text);
  if (!head) return null;
  const rest = text.slice(head.index + head[0].length);
  const end = rest.indexOf("];");
  if (end < 0) return null;
  const body = rest.slice(0, end);
  const out = [];
  for (const m of body.matchAll(/\("([a-z_]+)"\s*,\s*([0-9.]+)\)/g)) {
    out.push({ part: m[1], weight: Number(m[2]) });
  }
  return out.length ? out : null;
}

/** 解析 `READINESS_BANDS`（保持声明顺序 = 降序）。 */
export function parseBands(text) {
  const head = /pub const READINESS_BANDS:\s*&\[Band\]\s*=\s*&\[/.exec(text);
  if (!head) return null;
  const rest = text.slice(head.index + head[0].length);
  const end = rest.indexOf("];");
  if (end < 0) return null;
  const out = [];
  for (const m of rest.slice(0, end).matchAll(/Band\s*\{\s*id:\s*"([a-z_]+)"\s*,\s*min:\s*([0-9.]+)\s*,/g)) {
    out.push({ id: m[1], min: Number(m[2]) });
  }
  return out.length ? out : null;
}

/** 解析 `OPEN_DIVERGENCES` 的 `{id, site}`（只用于报告）。 */
export function parseDivergences(text) {
  const head = /pub const OPEN_DIVERGENCES:\s*&\[Divergence\]\s*=\s*&\[/.exec(text);
  if (!head) return null;
  const rest = text.slice(head.index + head[0].length);
  const end = rest.indexOf("\n];");
  if (end < 0) return null;
  const block = rest.slice(0, end);
  const out = [];
  for (const m of block.matchAll(/Divergence\s*\{([\s\S]*?)\n\s*\},/g)) {
    const body = m[1];
    const id = /id:\s*"([^"]+)"/.exec(body);
    const concept = /concept:\s*"([^"]+)"/.exec(body);
    const site = /site:\s*"([^"]+)"/.exec(body);
    const claim = /claim:\s*"([^"]+)"/.exec(body);
    const status = /status:\s*"([^"]+)"/.exec(body);
    if (id) {
      out.push({
        id: id[1],
        concept: concept ? concept[1] : "",
        site: site ? site[1] : "",
        claim: claim ? claim[1] : "",
        status: status ? status[1] : "",
      });
    }
  }
  return out.length ? out : null;
}

/**
 * 解析 `AXIOMS` 的 `{id, enforced_by}`。
 *
 * ⚠ 与其它解析器的一处刻意差异：**缺 `enforced_by` 的条目仍然返回**（`enforcedBy: ""`），
 * 而不是丢弃。丢弃会让「有人加了公理但忘了写守护者」变成**少一条**，
 * 而少一条在这里没有任何检查会发现 —— 那正是本函数要治的病。
 * 空 `enforced_by` 由 `checkEnforcedByTargets` 报出。
 */
export function parseAxioms(text) {
  const head = /pub const AXIOMS:\s*&\[Axiom\]\s*=\s*&\[/.exec(text);
  if (!head) return null;
  const rest = text.slice(head.index + head[0].length);
  const end = rest.indexOf("\n];");
  if (end < 0) return null;
  const out = [];
  for (const m of rest.slice(0, end).matchAll(/Axiom\s*\{([\s\S]*?)\n\s*\},/g)) {
    const body = m[1];
    const id = /id:\s*"([^"]+)"/.exec(body);
    if (!id) continue;
    const by = /enforced_by:\s*"([^"]+)"/.exec(body);
    out.push({ id: id[1], enforcedBy: by ? by[1] : "" });
  }
  return out.length ? out : null;
}

// ── 纯函数：校验 ──────────────────────────────────────────────────────

export function weightsSumTo1(w) {
  return !!w && near(w.supply + w.demand + w.irreplaceability, 1);
}

export function partsSumTo1(parts) {
  if (!parts || parts.length === 0) return false;
  return near(parts.reduce((s, p) => s + p.weight, 0), 1);
}

export function bandsAreTotal(bands, metricMax) {
  if (!bands || bands.length === 0) return false;
  if (bands[0].min > metricMax) return false;
  for (let i = 1; i < bands.length; i++) {
    if (bands[i - 1].min <= bands[i].min) return false;
  }
  return Math.abs(bands[bands.length - 1].min) < 1e-12;
}

/**
 * 比较一组现场权重与权威权重。
 * **这是本脚本的核心判据**，抽成纯函数以便用畸形输入做负向对照。
 */
export function compareWeights(authority, found) {
  const problems = [];
  for (const f of found) {
    const want = authority[f.key];
    if (want === undefined) {
      problems.push({ ...f, want: NaN, reason: `站点变量 \`${f.key}\` 不在权威源里（权威源字段被改名？）` });
      continue;
    }
    if (!near(f.value, want)) {
      problems.push({ ...f, want, reason: `期望 ${want}，实际 ${f.value}` });
    }
  }
  return problems;
}

/** 比较分档阈值序列与权威分档的下界序列。 */
export function compareBands(authorityBands, foundMins) {
  const want = authorityBands.filter((b) => b.min > 0).map((b) => b.min);
  const problems = [];
  if (foundMins.length !== want.length) {
    problems.push({
      want: want.join("/"),
      got: foundMins.join("/"),
      reason: `分档数不一致：期望 ${want.length} 个阈值，实际 ${foundMins.length} 个`,
    });
    return problems;
  }
  for (let i = 0; i < want.length; i++) {
    if (!near(foundMins[i], want[i])) {
      problems.push({ want: want.join("/"), got: foundMins.join("/"), reason: `第 ${i + 1} 档阈值不一致` });
      break;
    }
  }
  return problems;
}

/**
 * 权重序列的**规范渲染**：固定 2 位小数。
 *
 * 与 Rust 侧 `format_weight`（`{:.2}`）同一条约定 —— 否则 `[0.6, 0.4]` 会 `join` 成
 * `"0.6/0.4"`，而权威源里写的是 `0.60 / 0.40`。报告里显示成 `0.6` 会诱导人把数字
 * 抄成 `0.6`（数字等价，但会让「两侧书写形态一致」这条约定悄悄失效）。
 */
export function fmtSeq(values) {
  return values.map((v) => Number(v).toFixed(2)).join("/");
}

/**
 * 比较三力**内部**权重（`*_PARTS`）与 `.rhai` 现场。
 *
 * 只比**权重序列**，不比分项名 —— 本体侧叫 `expansion_cycle`、`.rhai` 侧叫
 * `adjusted_cycle_score`，两侧命名本来就允许不同（语义别名）。**残留缺口**（已知、已登记）：
 * 同一条式子里两个**同值**权重互换位置检测不到（如 supply 的 0.30 / 0.40 / 0.30
 * 首尾对调）。这是刻意的取舍：把分项名也锁死会让本检查耦合 `.rhai` 变量名，
 * 而变量名比数字更容易改 —— 那会把「数字漂移」这道门禁变成「改名就红」的噪声源。
 *
 * @param {Record<string, Array<{part: string, weight: number}>>} authorityParts 权威分项
 * @param {Array<{force: string, lhs: string, site: string, weights: number[]}>} found 现场
 * @param {number} expectPerForce 每力**应**找到的站点数（SITES 的 `expectPerForce`）
 */
export function compareForceParts(authorityParts, found, expectPerForce) {
  const problems = [];
  for (const force of Object.keys(authorityParts)) {
    const want = authorityParts[force];
    const hits = (found || []).filter((f) => f.force === force);
    const wantSeq = fmtSeq(want.map((p) => p.weight));
    if (hits.length !== expectPerForce) {
      problems.push({
        force,
        want: wantSeq,
        got: `${hits.length} 处`,
        reason:
          `应找到 ${expectPerForce} 处 \`${force}\` 加权和，实际 ${hits.length} 处` +
          ` ⇒ 变量被改名或结构变化（**勿当作通过**，请同步 FORCE_LHS_ALIASES 与 SITES）`,
      });
      continue;
    }
    for (const h of hits) {
      if (h.weights.length !== want.length) {
        problems.push({
          force,
          site: h.site,
          want: wantSeq,
          got: fmtSeq(h.weights),
          reason: `分项个数不一致：期望 ${want.length} 项，实际 ${h.weights.length} 项`,
        });
        continue;
      }
      const badIdx = want.findIndex((p, k) => !near(h.weights[k], p.weight));
      if (badIdx >= 0) {
        problems.push({
          force,
          site: h.site,
          want: wantSeq,
          got: fmtSeq(h.weights),
          reason: `第 ${badIdx + 1} 项权重不一致：期望 ${want[badIdx].weight.toFixed(2)}（${want[badIdx].part}），实际 ${h.weights[badIdx].toFixed(2)}`,
        });
      }
    }
  }
  return problems;
}

/**
 * 校验 `Axiom::enforced_by` 指向的函数**真实存在且确实是 `#[test]`**。
 *
 * 为什么必须机器校验：`enforced_by` 是 `&'static str`（自由文本），Rust 侧断言只有
 * `!a.enforced_by.is_empty()` ⇒ 测试一改名/删除，公理**静默失去守护者**，
 * 而「守护者不存在」与「守护者通过」在门禁看来是同一件事。
 * 同族判据 #197（悬空解析）／#130–#150（引用卫生）。
 *
 * 为什么连 `#[test]` 一起验：只验「存在同名 fn」会被一个**同名的普通函数**满足，
 * 那公理仍然无人守护。
 */
export function checkEnforcedByTargets(axioms, text) {
  const problems = [];
  const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  for (const a of axioms) {
    if (!a.enforcedBy) {
      problems.push({ ...a, reason: `公理 ${a.id} 未声明 \`enforced_by\` ⇒ 无守护者` });
      continue;
    }
    const hasTest = new RegExp(`#\\[test\\][\\s\\S]{0,200}?\\bfn\\s+${esc(a.enforcedBy)}\\s*\\(`).test(text);
    if (!hasTest) {
      problems.push({
        ...a,
        reason:
          `公理 ${a.id} 的 \`enforced_by = "${a.enforcedBy}"\` 在本文件里找不到对应的 ` +
          `\`#[test] fn\` ⇒ 该公理已失去守护者（改名/删除即静默腐烂）`,
      });
    }
  }
  return problems;
}

// ── 纯函数：抽取副本现场 ──────────────────────────────────────────────

const isCommentLine = (l) => /^\s*(\/\/|#)/.test(l);

/** `.rhai`：顶层权重兜底字面量（跳过注释行 —— 注释是散文，属报告档）。 */
export function extractRhaiWeightFallbacks(text) {
  const KEY = { w_supply: "supply", w_demand: "demand", w_irreplace: "irreplaceability" };
  const out = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((line, i) => {
    if (isCommentLine(line)) return;
    const hit = /\b(w_supply|w_demand|w_irreplace)\b/.exec(line);
    if (!hit) return;
    // 兜底值 = 该行最后一个浮点字面量（`else { 0.35 }` 或 `to_f64(w_supply, 0.35)`）
    const nums = [...line.matchAll(/(?<![\w.])(\d+\.\d+)(?![\w.])/g)].map((m) => Number(m[1]));
    if (nums.length === 0) return;
    out.push({ key: KEY[hit[1]], site: `${i + 1}`, value: nums[nums.length - 1], text: line.trim() });
  });
  return out;
}

/** 分档标签族（`.rhai` 里 readiness 档位的输出标签形态）。 */
const BAND_LABEL_RE = /"(strong|potential|weak|no)_(signal|bottleneck)"/;

/**
 * `.rhai`：readiness 分档阈值序列。
 *
 * **自锚定**，不接受外部传入锚点。早期版本让调用方猜锚点（`"line"` / `"fn"`），
 * 实测踩中：`strategy-scorer.rhai` 里有 **6 处** `"readiness_signal":` 调用点
 * （:250/:296/:342/:388/:446/:494）与 1 处 `fn` 定义（:64），
 * `"line"` 锚点撞上 :250 那个**没有 `>=`** 的调用点 ⇒ 返回 null
 * ⇒ 报出「代码可能已重构」。**那是脚本自身的脆弱，被伪装成了代码问题。**
 *
 * 现在改为：找「同时含 `>=` 与分档标签」的第一行，再向后连续收集，
 * 直到攒满 3 个阈值或链断（下一行既无 `>=` 也无 `else`）。
 */
export function extractRhaiBandThresholds(text) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((l) => l.includes(">=") && BAND_LABEL_RE.test(l));
  if (start < 0) return null;
  const mins = [];
  for (let k = start; k < Math.min(start + 8, lines.length); k++) {
    for (const m of lines[k].matchAll(/>=\s*(\d+\.\d+)/g)) mins.push(Number(m[1]));
    if (mins.length >= 3) break;
    const next = lines[k + 1];
    if (!next || !(next.includes(">=") || /\belse\b/.test(next))) break;
  }
  return mins.length ? mins : null;
}

const WEIGHT_VAR_NAMES = ["w_supply", "w_demand", "w_irreplace"];

// ── 三力**内部**权重（`*_PARTS`）现场 ──────────────────────────────────

/**
 * 三力加权和在 `.rhai` 里的**左值别名**（**契约**）。
 *
 * 两个脚本各写一份三力加权和，变量名不同：`bottleneck-calc.rhai` 用长名，
 * `strategy-scorer.rhai` 用缩写。某力改名 ⇒ 抓不到该力的站点 ⇒
 * `compareForceParts` 按「站点数不符」报红，**不会**静默当成 0 条通过。
 */
const FORCE_LHS_ALIASES = {
  SupplyRigidity: ["supply_rigidity_score", "srs"],
  DemandElasticity: ["demand_elasticity_score", "des"],
  Irreplaceability: ["irreplaceability_score", "irs"],
};

/**
 * `.rhai`：三力加权和的分项权重现场。
 *
 * **自锚定**（不靠行号、不靠外部传入锚点 —— 分档抽取器的教训见
 * `extractRhaiBandThresholds` 的文档）。形态：
 *   `let <左值> = <操作数> * <浮点字面量> [+ <操作数> * <浮点字面量> …];`
 *
 * 只认「操作数 * 浮点字面量」这一种项，故：
 *  · `x >= 0.0` 之类比较不产生项（不是 `*` 项）；
 *  · 顶层加权和（`… * ws`，权重是变量）不产生项 ⇒ 本函数只管**内部**权重，
 *    顶层权重归 `extractRhaiWeightFallbacks`；
 *  · 少于 2 项 ⇒ 不是加权和（透传 / 兜底），不算现场。
 */
export function extractRhaiForceParts(text) {
  const out = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((line, i) => {
    if (isCommentLine(line)) return;
    const m = /^\s*let\s+([A-Za-z_]\w*)\s*=\s*(.+?);\s*$/.exec(line);
    if (!m) return;
    const lhs = m[1];
    const force = Object.keys(FORCE_LHS_ALIASES).find((f) => FORCE_LHS_ALIASES[f].includes(lhs));
    if (!force) return;
    const terms = [...m[2].matchAll(/([A-Za-z_][\w.]*)\s*\*\s*(\d+\.\d+)/g)];
    if (terms.length < 2) return;
    out.push({
      force,
      lhs,
      site: `${i + 1}`,
      weights: terms.map((t) => Number(t[2])),
      text: line.trim(),
    });
  });
  return out;
}

/**
 * Rust 权威函数 `bottleneck_node_score` 内部权重字面量现场（2026-09-20 收敛后新增）。
 *
 * 收敛后两个 `.rhai` 不再内联三力加权和，真实算分迁到 `rhai_bottleneck.rs`。
 * 用与 `extractRhaiForceParts` 同构的抽取逻辑（`let <三力左值> = … * 0.xx + …;`）
 * 从 Rust 源码里抓出权重字面量，再与本体 `*_PARTS` 比对 —— 保证「用户真正跑在
 * Arc 上的 Rust 权重」就是权威值（本体 `*_PARTS` 生产消费方本为 0，收敛后更须
 * 盯住 Rust 侧字面量）。
 *
 * 支持跨行：Rust 的 `let xxx_score =\n        a * 0.30 + b * 0.40 + c * 0.30;`
 * 会把项拆到后续行，需从 `=` 之后连续收集到分号。
 */
export function extractRustForceParts(text) {
  const out = [];
  const lines = text.split(/\r?\n/);
  const seen = new Set();
  lines.forEach((line, i) => {
    if (isCommentLine(line)) return;
    const m = /^\s*let\s+(supply_rigidity_score|demand_elasticity_score|irreplaceability_score)\s*=\s*/.exec(line);
    if (!m) return;
    const force = Object.keys(FORCE_LHS_ALIASES).find((f) => FORCE_LHS_ALIASES[f].includes(m[1]));
    if (!force || seen.has(force)) return;
    // 从 `=` 之后收集项，若一行没到分号则续后续行
    let buf = line.slice(m[0].length);
    let k = i;
    while (!buf.includes(";") && k + 1 < lines.length) {
      k += 1;
      buf += " " + lines[k];
    }
    // 截到本声明结束（分号），并去掉 == 的比较式（如需求里的 != 判断不含 * 项，安全）
    buf = buf.split(";")[0];
    const terms = [...buf.matchAll(/([A-Za-z_][\w.]*)\s*\*\s*(\d+\.\d+)/g)];
    if (terms.length < 2) return;
    seen.add(force);
    out.push({
      force,
      lhs: m[1],
      site: `${i + 1}`,
      weights: terms.map((t) => Number(t[2])),
      text: buf.trim(),
    });
  });
  return out;
}

// ── 权威源里的「指针字段」现场 ────────────────────────────────────────

/**
 * 从权威源抽出所有**指针字段**（`evidence` / `site`）的字面量值。
 *
 * 为何单独管：它们是 `&'static str` 自由文本，但**语义是仓库路径**。
 * Rust 侧断言只查「非空」⇒ 文件改名 / 挪目录后**没有任何东西会红**。
 * 这是 `Axiom::enforced_by` 的同族问题（那条已由 `checkEnforcedByTargets` 覆盖）。
 *
 * 值形态：`路径:行`；多锚点用 ` | ` 分隔（实测 `OBJECT_PROPERTIES` 的
 * 三力度量条目即为此形）。
 */
export function extractPointerSites(text) {
  const out = [];
  text.split(/\r?\n/).forEach((line, i) => {
    const m = /^\s*(evidence|site):\s*"([^"]+)"/.exec(line);
    if (!m) return;
    for (const raw of m[2].split("|")) {
      const v = raw.trim();
      if (!v) continue;
      out.push({ field: m[1], raw: v, file: stripLineSuffix(v), site: `${i + 1}` });
    }
  });
  return out;
}

/**
 * 去掉 `:行号` / `:行-行` 尾巴。
 *
 * **只验文件存在、不验行号**：行号必然随重构漂移，锁行号会把门禁变成噪声源
 * （判据 #186–#189）。副作用是每行都可能行号过期 —— 这是刻意接受的残余风险，
 * 因为本检查的收益（文件被挪走/改名会被拦住）远大于行号精度。
 */
export function stripLineSuffix(v) {
  return v.replace(/:\d+(-\d+)?$/, "");
}

/**
 * 校验指针字段指向的文件真实存在。
 *
 * `exists` 由参数注入（而非直接读 `fs`），使自检能用**畸形输入**做负向对照 ——
 * 一道恒真的检查等于没写。
 */
export function checkPointerFilesExist(pointerSites, exists) {
  const problems = [];
  for (const p of pointerSites) {
    if (!p.file || !/[/\\]/.test(p.file)) {
      problems.push({
        ...p,
        reason: `指针字段 \`${p.field}\` 的值 \`${p.raw}\` 不是可核对的仓库路径（本字段契约为「源码锚点」）`,
      });
      continue;
    }
    if (!exists(p.file)) {
      problems.push({ ...p, reason: `指针字段 \`${p.field}\` 指向的文件不存在：\`${p.file}\`` });
    }
  }
  return problems;
}

/**
 * seeder 的「权重默认值**不得**再硬编码」检测（回退守卫）。
 *
 * 为什么不能用单行正则代替：`Variable { name: …, value: … }` 是**跨行**结构，
 * `value:\s*serde_json::json!\(0\.\d+\)` 这种模式会把**无关变量**的
 * `json!(0.0)`（实测：`serenity_min_revenue_growth`）误报成违规。
 * 必须锚在 `name: "w_*"` 上再向后看若干行。
 */
export function findHardcodedWeightValues(text) {
  const out = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((line, i) => {
    const hit = WEIGHT_VAR_NAMES.find((n) => line.includes(`name: "${n}"`));
    if (!hit) return;
    for (let k = i; k < Math.min(i + 8, lines.length); k++) {
      const m = /value:\s*serde_json::json!\(\s*([0-9.]+)\s*\)/.exec(lines[k]);
      if (m) {
        out.push({ name: hit, site: `${i + 1}`, value: Number(m[1]), text: lines[k].trim() });
        break;
      }
    }
  });
  return out;
}

// ── 自检 ──────────────────────────────────────────────────────────────

export function selftest() {
  const results = [];
  const t = (name, fn) => {
    try {
      const r = fn();
      results.push({ name, ok: r === true, detail: r === true ? "" : String(r) });
    } catch (e) {
      results.push({ name, ok: false, detail: e.message });
    }
  };

  const GOOD = `pub const BOTTLENECK_WEIGHTS: BottleneckWeights =
    BottleneckWeights { supply: 0.35, demand: 0.35, irreplaceability: 0.30 };`;

  // ── 解析：正负对照 ──
  t("解析权重：正常文本", () => {
    const w = parseWeights(GOOD);
    return w && near(w.supply, 0.35) && near(w.irreplaceability, 0.3)
      ? true
      : `得到 ${JSON.stringify(w)}`;
  });
  t("解析权重：★缺字段必须返回 null（不得当成 0）", () => {
    const broken = GOOD.replace(", irreplaceability: 0.30", "");
    return parseWeights(broken) === null ? true : "畸形文本仍解析成功 ⇒ 会静默按错值通过";
  });
  t("解析权重：★字段顺序打乱必须返回 null（正则锁死顺序）", () => {
    const shuffled =
      "pub const BOTTLENECK_WEIGHTS: BottleneckWeights =\n    BottleneckWeights { demand: 0.35, supply: 0.35, irreplaceability: 0.30 };";
    return parseWeights(shuffled) === null ? true : "顺序被容忍 ⇒ 与文档声明的契约不符";
  });
  t("解析分项权重", () => {
    const txt = 'pub const DEMAND_ELASTICITY_PARTS: ForceParts = &[("evidence", 0.60), ("certainty", 0.40)];';
    const p = parseParts(txt, "DEMAND_ELASTICITY_PARTS");
    return p && p.length === 2 && partsSumTo1(p) ? true : `得到 ${JSON.stringify(p)}`;
  });
  t("解析分项权重：★空表返回 null", () => {
    const txt = "pub const X_PARTS: ForceParts = &[];";
    return parseParts(txt, "X_PARTS") === null ? true : "空表被当成合法解析结果";
  });
  t("解析分档", () => {
    const txt = `pub const READINESS_BANDS: &[Band] = &[
    Band { id: "strong_bottleneck", min: 75.0, label: "强" },
    Band { id: "no_signal", min: 0.0, label: "无" },
];`;
    const b = parseBands(txt);
    return b && b.length === 2 && near(b[0].min, 75) ? true : `得到 ${JSON.stringify(b)}`;
  });
  t("★分档校验：有空洞必须判失败", () => {
    const bad = [
      { id: "a", min: 75 },
      { id: "b", min: 10 },
    ];
    return bandsAreTotal(bad, 100) === false ? true : "末档下界 10 留出 [0,10) 空洞却通过";
  });
  t("★分档校验：正例必须通过", () => {
    const good = [
      { id: "a", min: 75 },
      { id: "b", min: 55 },
      { id: "c", min: 0 },
    ];
    return bandsAreTotal(good, 100) === true ? true : "正常分档被判失败 ⇒ 判据写反";
  });

  // ── 核心：比较器必须能判失败（否则整道门禁是恒真的）──
  t("★比较器：一致时零问题", () => {
    const auth = { supply: 0.35, demand: 0.35, irreplaceability: 0.3 };
    const found = [{ key: "supply", site: "1", value: 0.35 }];
    return compareWeights(auth, found).length === 0 ? true : "一致却报问题";
  });
  t("★比较器：不一致必须报出（负向对照）", () => {
    const auth = { supply: 0.35, demand: 0.35, irreplaceability: 0.3 };
    const found = [{ key: "supply", site: "9", value: 0.4 }];
    const p = compareWeights(auth, found);
    return p.length === 1 && near(p[0].want, 0.35) ? true : "0.4 ≠ 0.35 却判通过 ⇒ 门禁恒真";
  });
  t("★比较器：权威源无该字段必须报出（改名陷阱）", () => {
    const auth = { supply: 0.35 };
    const found = [{ key: "irreplaceability", site: "12", value: 0.3 }];
    return compareWeights(auth, found).length === 1 ? true : "字段被改名后静默通过";
  });
  t("★分档比较：数量不一致必须报出", () => {
    // 注意：compareBands 只比 `min > 0` 的档（末档 = 0 是兜底档，`.rhai` 里没有对应阈值）
    // ⇒ 权威必须给 4 档，过滤后剩 3 个阈值，才能与「只找到 2 个」形成数量差。
    const auth = [
      { id: "a", min: 75 },
      { id: "b", min: 55 },
      { id: "c", min: 35 },
      { id: "d", min: 0 },
    ];
    return compareBands(auth, [75, 55]).length === 1 ? true : "阈值个数少了却通过";
  });
  t("★分档比较：数量相同但数值不同必须报出", () => {
    const auth = [
      { id: "a", min: 75 },
      { id: "b", min: 55 },
      { id: "c", min: 35 },
      { id: "d", min: 0 },
    ];
    return compareBands(auth, [80, 60, 35]).length === 1 ? true : "80/60 与 75/55 不同却通过";
  });
  t("★分档比较：末档 0 不参与比对（否则会把兜底档误报成缺失阈值）", () => {
    const auth = [
      { id: "a", min: 75 },
      { id: "b", min: 0 },
    ];
    return compareBands(auth, [75]).length === 0 ? true : "末档 0 被当成一个应存在的阈值";
  });

  // ── 抽取器：正负对照（含「注释不算现场」）──
  t("抽取 .rhai 权重兜底", () => {
    const src = `        let ws = if w_supply != () { w_supply } else { 0.35 };\n        let wd = if w_demand != () { w_demand } else { 0.35 };`;
    const got = extractRhaiWeightFallbacks(src);
    return got.length === 2 && near(got[0].value, 0.35) ? true : `得到 ${JSON.stringify(got)}`;
  });
  t("★抽取器：行尾注释里的数字不得当作现场", () => {
    const src = "        // 默认 w_supply=0.35 w_demand=0.35 w_irreplace=0.30";
    return extractRhaiWeightFallbacks(src).length === 0 ? true : "注释被当成可执行现场 ⇒ 假阳性";
  });
  t("★抽取器：无数字的透传行不得产生现场", () => {
    const src = '    if name == "w_supply" { if w_supply != () { w_supply } else { fallback } }';
    return extractRhaiWeightFallbacks(src).length === 0 ? true : "透传 fallback 被当成字面量";
  });
  t("抽取分档阈值（多行函数体）", () => {
    const src = [
      "fn readiness_signal(score) {",
      '    if score >= 75.0 { "strong_signal" }',
      '    else if score >= 55.0 { "potential_signal" }',
      '    else if score >= 35.0 { "weak_signal" }',
      '    else { "no_signal" }',
      "}",
    ].join("\n");
    const got = extractRhaiBandThresholds(src);
    return got && got.length === 3 && near(got[0], 75) ? true : `得到 ${JSON.stringify(got)}`;
  });
  t("抽取分档阈值（单行三元链）", () => {
    const src =
      '            "readiness_signal": if fc >= 75.0 { "strong_bottleneck" } else if fc >= 55.0 { "potential_bottleneck" } else if fc >= 35.0 { "weak_signal" } else { "no_signal" },';
    const got = extractRhaiBandThresholds(src);
    return got && got.length === 3 && near(got[2], 35) ? true : `得到 ${JSON.stringify(got)}`;
  });
  t("★抽取分档阈值：调用点（含标签但无 >=）不得被当成锚点 —— 实测踩过的 bug", () => {
    // `strategy-scorer.rhai` 有 6 处 `"readiness_signal": readiness_signal(composite),`
    // 含分档名却**不含 `>=`**。旧版按行号猜锚点撞上其中一处 ⇒ 返回 null
    // ⇒ 报出「代码可能已重构」——**把脚本自身的脆弱伪装成代码问题**。
    const src = [
      '            "readiness_signal": readiness_signal(composite),',
      "fn readiness_signal(score) {",
      '    if score >= 75.0 { "strong_signal" }',
      '    else if score >= 55.0 { "potential_signal" }',
      '    else if score >= 35.0 { "weak_signal" }',
      '    else { "no_signal" }',
      "}",
    ].join("\n");
    const got = extractRhaiBandThresholds(src);
    return got && got.length === 3 && near(got[0], 75) ? true : `得到 ${JSON.stringify(got)}`;
  });
  t("★抽取分档阈值：锚点缺失必须返回 null（不得返回空数组冒充成功）", () => {
    const src = "fn something_else() { let x = 1.0; }";
    return extractRhaiBandThresholds(src) === null ? true : "锚点缺失却返回了非 null";
  });
  t("★回退守卫：派生形态（引用权威源）不得报违规", () => {
    const src = ['            name: "w_supply".into(),', "            value: serde_json::json!(bw.supply),"].join("\n");
    return findHardcodedWeightValues(src).length === 0 ? true : "派生形态被误报成硬编码";
  });
  t("★回退守卫：硬编码字面量必须报出（负向对照）", () => {
    const src = ['            name: "w_supply".into(),', "            value: serde_json::json!(0.35),"].join("\n");
    const got = findHardcodedWeightValues(src);
    return got.length === 1 && near(got[0].value, 0.35) ? true : `得到 ${JSON.stringify(got)}`;
  });
  t("★回退守卫：无关变量的 json!(0.0) 不得报出 —— 实测的假阳性", () => {
    const src = [
      '            name: "serenity_min_revenue_growth".into(),',
      "            value: serde_json::json!(0.0),",
    ].join("\n");
    return findHardcodedWeightValues(src).length === 0 ? true : "无关变量被当成权重硬编码";
  });
  t("渲染约定：Rust {:.2} ↔ JS toFixed(2) 一致", () => {
    return (0.3).toFixed(2) === "0.30" && (0.35).toFixed(2) === "0.35"
      ? true
      : "toFixed(2) 行为与 Rust 的 {:.2} 不一致 ⇒ 描述文本比对会假报";
  });
  t("★渲染约定负向：() 渲染会丢尾零（证明该约定有必要）", () => {
    return String(0.3) === "0.3" ? true : "Number→String 已保留尾零，渲染约定需重新评估";
  });

  // ── 三力内部权重（`*_PARTS`）：抽取器 ──
  t("抽取三力内部权重（长名 + 缩写混排）", () => {
    const src = [
      "        let supply_rigidity_score = concentration_score * 0.30 + barrier_score * 0.40 + adjusted_cycle_score * 0.30;",
      "        let demand_elasticity_score = adjusted_evidence * 0.60 + certainty_score * 0.40;",
      "        let srs = cs * 0.30 + bs * 0.40 + acyc * 0.30;",
      "        let des = ae * 0.60 + cers * 0.40;",
    ].join("\n");
    const got = extractRhaiForceParts(src);
    return got.length === 4 && got[1].force === "DemandElasticity" && fmtSeq(got[1].weights) === "0.60/0.40"
      ? true
      : `得到 ${JSON.stringify(got)}`;
  });
  t("★内部权重渲染：0.60 不得显示成 0.6（与 Rust {:.2} 同约定）", () => {
    // 实测踩过：`[0.6, 0.4].join("/")` → `"0.6/0.4"`，而权威源写的是 0.60 / 0.40。
    // 报告里渲染成 0.6 会诱导人把数字抄成 0.6，让「两侧书写形态一致」悄悄失效。
    return fmtSeq([0.6, 0.4]) === "0.60/0.40" ? true : `得到 ${fmtSeq([0.6, 0.4])}`;
  });
  t("★抽取内部权重：注释行不得算现场", () => {
    const src = "        // 默认 supply_rigidity_score = a * 0.30 + b * 0.40 + c * 0.30";
    return extractRhaiForceParts(src).length === 0 ? true : "注释被当成可执行现场 ⇒ 假阳性";
  });
  t("★抽取内部权重：顶层加权和（权重是变量）不得被当成内部权重", () => {
    const src = "        let composite = supply_rigidity_score * ws + demand_elasticity_score * wd + irs * wi;";
    return extractRhaiForceParts(src).length === 0 ? true : "顶层加权和被当成内部权重 ⇒ 概念串台";
  });
  t("★抽取内部权重：同名左值的比较式不得算现场", () => {
    const src = "        let srs = if x >= 0.0 && y >= 0.0 { 1.0 } else { 0.0 };";
    return extractRhaiForceParts(src).length === 0 ? true : "比较式里的 0.0/0.0 被误读成内部权重";
  });
  t("★抽取内部权重：非三力左值（tech_moat_score 的 0.5/0.5）不得算现场", () => {
    const src =
      "        let tech_moat_score = if financial_rnd_score >= 0.0 && financial_roe_score >= 0.0 { financial_rnd_score * 0.5 + financial_roe_score * 0.5 } else { barrier_score };";
    return extractRhaiForceParts(src).length === 0 ? true : "非三力左值被纳入现场";
  });

  // ── Rust 权威函数内部权重（2026-09-20 收敛后新增，跨行、去重的正负对照）──
  t("抽取 Rust 权威函数内部权重（含跨行声明）", () => {
    const src = [
      "        let supply_rigidity_score = concentration_score * 0.30 + barrier_score * 0.40 + adjusted_cycle_score * 0.30;",
      "        let demand_elasticity_score = adjusted_evidence * 0.60 + certainty_score * 0.40;",
      "        let irreplaceability_score =",
      "            supplier_score * 0.30 + tech_moat_score * 0.40 + barrier_score * 0.30;",
    ].join("\n");
    const got = extractRustForceParts(src);
    return got.length === 3 && got[2].force === "Irreplaceability" && fmtSeq(got[2].weights) === "0.30/0.40/0.30"
      ? true
      : `得到 ${JSON.stringify(got)}`;
  });
  t("★抽取 Rust 内部权重：注释行不得算现场", () => {
    const src = "        // let supply_rigidity_score = a * 0.30 + b * 0.40 + c * 0.30";
    return extractRustForceParts(src).length === 0 ? true : "注释被当成可执行现场 ⇒ 假阳性";
  });
  t("★抽取 Rust 内部权重：非三力 let（如 temp * 0.5）不得算现场", () => {
    const src = "        let tech_moat_score = rnd_s * 0.5 + roe_s * 0.5;";
    return extractRustForceParts(src).length === 0 ? true : "非三力左值被当成现场";
  });
  t("★抽取 Rust 内部权重：三力名但少于 2 项不得算现场", () => {
    const src = "        let supply_rigidity_score = barrier_score * 0.40;";
    return extractRustForceParts(src).length === 0 ? true : "单项式子被当成加权和";
  });

  // ── 三力内部权重：比较器（必须能判失败）──
  const AUTH_PARTS = {
    SupplyRigidity: [
      { part: "concentration", weight: 0.30 },
      { part: "barrier", weight: 0.40 },
      { part: "expansion_cycle", weight: 0.30 },
    ],
    DemandElasticity: [
      { part: "evidence", weight: 0.60 },
      { part: "certainty", weight: 0.40 },
    ],
    Irreplaceability: [
      { part: "supplier", weight: 0.30 },
      { part: "tech_moat", weight: 0.40 },
      { part: "barrier", weight: 0.30 },
    ],
  };
  const FOUND_PARTS_OK = [
    { force: "SupplyRigidity", lhs: "srs", site: "1", weights: [0.30, 0.40, 0.30] },
    { force: "DemandElasticity", lhs: "des", site: "2", weights: [0.60, 0.40] },
    { force: "Irreplaceability", lhs: "irs", site: "3", weights: [0.30, 0.40, 0.30] },
  ];
  t("★内部权重比较：一致时零问题", () => {
    return compareForceParts(AUTH_PARTS, FOUND_PARTS_OK, 1).length === 0 ? true : "一致却报问题";
  });
  t("★内部权重比较：数值不一致必须报出（负向对照）", () => {
    const bad = FOUND_PARTS_OK.map((x) => ({ ...x }));
    bad[0] = { ...bad[0], weights: [0.35, 0.40, 0.25] };
    const p = compareForceParts(AUTH_PARTS, bad, 1);
    return p.length === 1 && /第 1 项/.test(p[0].reason) ? true : `得到 ${JSON.stringify(p)}`;
  });
  t("★内部权重比较：站点数不符必须报出（左值改名陷阱）", () => {
    const p = compareForceParts(AUTH_PARTS, FOUND_PARTS_OK.slice(0, 2), 1);
    return p.length === 1 && p[0].force === "Irreplaceability" ? true : `得到 ${JSON.stringify(p)}`;
  });
  t("★内部权重比较：分项个数不符必须报出", () => {
    const bad = FOUND_PARTS_OK.map((x) => ({ ...x }));
    bad[1] = { ...bad[1], weights: [0.60, 0.30, 0.10] };
    const p = compareForceParts(AUTH_PARTS, bad, 1);
    return p.length === 1 && /分项个数不一致/.test(p[0].reason) ? true : `得到 ${JSON.stringify(p)}`;
  });

  // ── 公理守护者（`enforced_by`）──
  const AXIOMS_SRC = `pub const AXIOMS: &[Axiom] = &[
    Axiom {
        id: "A1",
        statement: "x",
        enforced_by: "test_a",
    },
    Axiom {
        id: "A2",
        statement: "y",
        enforced_by: "test_b",
    },
];`;
  t("解析 AXIOMS", () => {
    const got = parseAxioms(AXIOMS_SRC);
    return got && got.length === 2 && got[1].enforcedBy === "test_b" ? true : `得到 ${JSON.stringify(got)}`;
  });
  t("★解析 AXIOMS：缺 enforced_by 仍须返回该条（不得丢弃）", () => {
    const src = `pub const AXIOMS: &[Axiom] = &[
    Axiom {
        id: "A1",
        statement: "x",
    },
];`;
    const got = parseAxioms(src);
    return got && got.length === 1 && got[0].enforcedBy === ""
      ? true
      : `被丢弃或未标空 ⇒ 「加了公理但忘了写守护者」不可见（得到 ${JSON.stringify(got)}）`;
  });
  t("★公理守护者：指向不存在的 fn 必须报出（负向对照）", () => {
    const src = ["#[test]", "fn test_other() {}"].join("\n");
    const p = checkEnforcedByTargets([{ id: "A1", enforcedBy: "test_missing" }], src);
    return p.length === 1 ? true : "守护者不存在却判通过 ⇒ 门禁恒真";
  });
  t("★公理守护者：同名但非 #[test] 的普通 fn 必须报出", () => {
    const p = checkEnforcedByTargets([{ id: "A1", enforcedBy: "test_a" }], "fn test_a() {}");
    return p.length === 1 ? true : "普通函数被当成公理守护者";
  });
  t("★公理守护者：空 enforced_by 必须报出", () => {
    const p = checkEnforcedByTargets([{ id: "A1", enforcedBy: "" }], "#[test]\nfn test_a() {}");
    return p.length === 1 ? true : "无守护者的公理被判通过";
  });
  t("★公理守护者：正常指向必须通过（须容忍中间的文档注释）", () => {
    const src = ["    /// 说明", "    #[test]", "    fn test_a() {"].join("\n");
    return checkEnforcedByTargets([{ id: "A1", enforcedBy: "test_a" }], src).length === 0
      ? true
      : "正常守护者被误报";
  });

  // ── 指针字段（`evidence` / `site`）──
  t("抽取指针字段（evidence / site，多锚点按 | 拆分）", () => {
    const src = [
      '        evidence: "src-tauri/agency_experts/stock-analysis/chain-decomposer.md:1",',
      '        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:183 | src-tauri/src/commands/bottleneck-calc.rhai:211",',
      '        site: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:101",',
    ].join("\n");
    const got = extractPointerSites(src);
    return got.length === 4 && got[1].field === "evidence" && got[3].field === "site"
      ? true
      : `得到 ${JSON.stringify(got)}`;
  });
  t("★指针抽取：行号后缀须剥离（否则恒报文件不存在）", () => {
    return stripLineSuffix("a/b.md:101") === "a/b.md" && stripLineSuffix("a/b.md:1-5") === "a/b.md"
      ? true
      : "行号未剥离 ⇒ 文件存在性判断会全体假红";
  });
  t("★指针文件存在性：文件不存在必须报出（负向对照）", () => {
    const sites = extractPointerSites('        evidence: "src-tauri/nope/missing.md:3",');
    return checkPointerFilesExist(sites, () => false).length === 1 ? true : "文件不存在却判通过 ⇒ 门禁恒真";
  });
  t("★指针文件存在性：文件存在不得报出（正例）", () => {
    const sites = extractPointerSites('        evidence: "src-tauri/ok.md:3",');
    return checkPointerFilesExist(sites, () => true).length === 0 ? true : "正常指针被误报";
  });
  t("★指针文件存在性：非路径值必须报出（字段契约为「源码锚点」）", () => {
    const sites = [{ field: "evidence", raw: "见上文", file: "见上文", site: "1" }];
    return checkPointerFilesExist(sites, () => true).length === 1 ? true : "自由文本被当成合法路径";
  });

  const failed = results.filter((r) => !r.ok);
  return { results, failed };
}

// ── 主流程 ────────────────────────────────────────────────────────────

function readText(rel, readCount) {
  const abs = path.join(ROOT, rel);
  const text = fs.readFileSync(abs, "utf8");
  readCount.n += 1;
  return text;
}

function main() {
  if (SELFTEST) {
    const { results, failed } = selftest();
    for (const r of results) {
      console.log(`${r.ok ? "✔" : "✖"} ${r.name}${r.ok ? "" : ` —— ${r.detail}`}`);
    }
    console.log(`\n自检：${results.length - failed.length} passed / ${failed.length} failed`);
    process.exit(failed.length === 0 ? 0 : 1);
  }

  const readCount = { n: 0 };
  const problems = [];
  const info = [];
  const prose = [];

  // 1. 权威源（解析不出来 ⇒ exit 3，绝不静默通过）
  let authority;
  try {
    authority = readText(AUTHORITY_REL, readCount);
  } catch (e) {
    console.error(`✖ 权威源不可读：${AUTHORITY_REL} —— ${e.message}`);
    process.exit(3);
  }
  const weights = parseWeights(authority);
  const parts = {
    SupplyRigidity: parseParts(authority, "SUPPLY_RIGIDITY_PARTS"),
    DemandElasticity: parseParts(authority, "DEMAND_ELASTICITY_PARTS"),
    Irreplaceability: parseParts(authority, "IRREPLACEABILITY_PARTS"),
  };
  const bands = parseBands(authority);
  const divergences = parseDivergences(authority);
  const axioms = parseAxioms(authority);
  // 指针字段（`evidence` / `site`）—— 与 `enforced_by` 同族：自由文本，语义是仓库路径
  const pointerSites = extractPointerSites(authority);

  if (!weights || !bands || Object.values(parts).some((p) => p === null)) {
    console.error(
      `✖ 权威源解析失败（weights=${!!weights} bands=${!!bands} parts=${Object.entries(parts)
        .filter(([, v]) => !v)
        .map(([k]) => k)
        .join(",") || "ok"}）`,
    );
    console.error("  ⇒ 解析契约见 domain_ontology.rs 的模块文档；此刻**不得**按「无违规」放行。");
    process.exit(3);
  }

  // 2. 权威源自洽（本体测试也查，这里独立复算 —— 门禁不该盲信被测方）
  if (!weightsSumTo1(weights)) {
    problems.push({ where: AUTHORITY_REL, why: `顶层权重和 ≠ 1（${weights.supply + weights.demand + weights.irreplaceability}）` });
  }
  for (const [name, p] of Object.entries(parts)) {
    if (!partsSumTo1(p)) {
      problems.push({ where: AUTHORITY_REL, why: `${name} 分项权重和 ≠ 1` });
    }
  }
  if (!bandsAreTotal(bands, 100)) {
    problems.push({ where: AUTHORITY_REL, why: "分档表不满足「降序 + 末档下界 = 0」" });
  }
  if (divergences === null) {
    problems.push({ where: AUTHORITY_REL, why: "口径分歧表解析为空 —— 确认是已统一还是漏登记" });
  }
  if (axioms === null) {
    problems.push({ where: AUTHORITY_REL, why: "公理表解析为空 —— 确认是已清空还是漏登记（清空即等于取消全部公理约束）" });
  }
  if (pointerSites.length === 0) {
    problems.push({
      where: AUTHORITY_REL,
      why: "指针字段（`evidence` / `site`）一条都没抽到 —— 本检查已成空转（字段改名即静默通过）",
    });
  }

  info.push(`权威权重：supply=${weights.supply} demand=${weights.demand} irreplaceability=${weights.irreplaceability}`);
  for (const [name, p] of Object.entries(parts)) {
    info.push(`${name} 分项：${p.map((x) => `${x.part}=${x.weight}`).join(" ")}`);
  }
  info.push(`分档（降序）：${bands.map((b) => `${b.id}≥${b.min}`).join("  ")}`);
  if (axioms) {
    info.push(`公理 ${axioms.length} 条（守护者全部为 #[test] fn）：${axioms.map((a) => a.id).join("/")}`);
  }

  // 3. 副本站点（清单是契约：抓不到 ⇒ 失败）
  const siteReport = [];
  for (const s of SITES) {
    let text;
    try {
      text = readText(s.file, readCount);
    } catch (e) {
      problems.push({ where: s.file, why: `站点文件不可读：${e.message}` });
      continue;
    }
    // 三力**内部**权重：走专用分支，**不**用通用条数检查 ——
    // 通用检查只会说「抓到 N 条」，而这里必须点名是**哪一力**丢了站点
    // （左值改名时，`got.length` 仍可能是 2+1≠3 之外的各种值，报「3 vs 2」帮不上排查）。
    // `expectPerForce` 已隐含总数约束：每力恰好 1 处 ⇒ 总数必然 = 3。
    if (s.kind === "rhai-calls-shared") {
      // 2026-09-20 收敛后：.rhai 不再内联三力加权和，而是调用共享 Engine 的 Rust
      // 权威函数 bottleneck_node_score。此处断言「脚本确实调用了它」，防止脚本又
      // 退回独立实现（与 rhai-band-from-ontology 同一纪律：「收敛到本体/权威后，
      // 脚本若不再调用才算回归」）。
      const hasCall = text.includes("bottleneck_node_score(");
      siteReport.push({ site: s.file, kind: s.kind, note: s.note, found: hasCall ? "bottleneck_node_score(…)" : "无" });
      if (!hasCall) {
        problems.push({
          where: s.file,
          why: "未发现对 bottleneck_node_score 的调用 —— .rhai 三力评分已退回独立实现（2026-09-20 收敛要求统一走 Rust 权威口径）。若改写了调用形态请同步 SITES",
        });
      }
      continue;
    }

    let got;
    let shape;
    if (s.kind === "rhai-weight-fallback") {
      got = extractRhaiWeightFallbacks(text);
      shape = got.map((g) => ({ key: g.key, value: g.value, site: `${s.file}:${g.site}`, text: g.text }));
    } else if (s.kind === "rhai-band-from-ontology") {
      // P0：分档已收敛到本体，脚本不应再有独立分档字面量。此处断言「调用 band_for_score」存在。
      const hasCall = text.includes("band_for_score(");
      siteReport.push({ site: s.file, kind: s.kind, note: s.note, found: hasCall ? "band_for_score(…)" : "无" });
      if (!hasCall) {
        problems.push({
          where: s.file,
          why: "未发现对 band_for_score 的调用 —— .rhai 分档已退回独立阈值/独立实现（P0 要求从本体分档）。若改写了调用形态请同步 SITES",
        });
      }
      continue;
    } else {
      const mins = extractRhaiBandThresholds(text);
      if (mins === null) {
        problems.push({
          where: s.file,
          why: "分档阈值站点抓不到（未找到「同时含 >= 与分档标签」的行）⇒ 代码可能已重构。请更新 SITES，勿当作通过",
        });
        continue;
      }
      shape = mins;
      got = mins;
    }

    if (got.length !== s.expect) {
      problems.push({
        where: s.file,
        why: `${s.kind}：声明 ${s.expect} 条，实际抓到 ${got.length} 条 ⇒ 结构变化，须人工确认`,
      });
    }

    if (s.kind === "rhai-weight-fallback") {
      const bad = compareWeights(weights, shape);
      for (const b of bad) {
        problems.push({ where: b.site, why: `权重副本与权威源不一致：${b.reason}` });
      }
      siteReport.push({ site: s.file, kind: s.kind, note: s.note, found: shape.map((x) => `${x.key}=${x.value}`).join(" ") });
    } else {
      const bad = compareBands(bands, shape);
      for (const b of bad) {
        problems.push({ where: s.file, why: `分档阈值与权威源不一致：${b.reason}（权威 ${b.want} / 实际 ${b.got}）` });
      }
      siteReport.push({ site: s.file, kind: s.kind, note: s.note, found: shape.join("/") });
    }
  }

  // 3b. Rust 权威函数内部权重字面量 vs 本体 *_PARTS（2026-09-20 收敛后）
  //
  // 两个 .rhai 已收敛为调用 Rust 权威函数 bottleneck_node_score，不再内联三力加权和。
  // 权威算分迁到 rhai_bottleneck.rs；此处的比对从「比对 .rhai 现场」转为
  // 「比对 Rust 源码里真正跑的字面量」，保证用户实际执行的那份权重就是权威值。
  const RS_REL = "src-tauri/src/commands/stock_workflow/rhai_bottleneck.rs";
  try {
    const rustText = readText(RS_REL, readCount);
    const rustParts = extractRustForceParts(rustText);
    const badRust = compareForceParts(parts, rustParts, 1);
    for (const b of badRust) {
      problems.push({
        where: b.site ? `${RS_REL}:${b.site}` : RS_REL,
        why: `Rust 权威函数内部权重与本体 *_PARTS 不一致：${b.reason}（权威 ${b.want} / 实际 ${b.got}）`,
      });
    }
    siteReport.push({
      site: RS_REL,
      kind: "rust-force-parts",
      note: "bottleneck_node_score 内部权重字面量（收敛后真实算分侧）",
      found: rustParts.map((x) => `${x.force}=[${fmtSeq(x.weights)}]`).join(" "),
    });
  } catch (e) {
    problems.push({ where: RS_REL, why: `Rust 权威函数内部权重检查读不到文件：${e.message}` });
  }

  // 4. 接线存在性（「接了线」≠「接活」）
  for (const w of WIRING) {
    let text;
    try {
      text = readText(w.file, readCount);
    } catch (e) {
      problems.push({ where: w.file, why: `接线检查读不到文件：${e.message}` });
      continue;
    }
    for (const need of w.mustContain || []) {
      if (!text.includes(need)) {
        problems.push({ where: w.file, why: `接线缺失：应包含 \`${need}\`（${w.note}）` });
      }
    }
    for (const re of w.mustNotMatch || []) {
      const m = re.exec(text);
      if (m) {
        problems.push({ where: w.file, why: `出现不应存在的形态 \`${m[0]}\`（${w.note}）` });
      }
    }
  }

  // 4b. 回退守卫：seeder 的权重默认值不得再硬编码
  //
  // ⚠ **两个方向都要查**（这正是「空转守卫」的教训，判据 #88）：
  //   · 反例：出现 `value: serde_json::json!(0.xx)` ⇒ 有人把权威源接线改回去了；
  //   · 正例：三个 `Variable` 锚点必须都在 ⇒ 否则名字一改，本守卫**静默通过**，
  //     冒充了一个守卫。
  const SEEDER_REL = "src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs";
  try {
    const seederText = readText(SEEDER_REL, readCount);
    for (const h of findHardcodedWeightValues(seederText)) {
      problems.push({
        where: `${SEEDER_REL}:${h.site}`,
        why: `权重 \`${h.name}\` 回退成硬编码字面量 ${h.value} —— 应引用 domain_ontology 权威源`,
      });
    }
    const missing = WEIGHT_VAR_NAMES.filter((n) => !seederText.includes(`name: "${n}"`));
    if (missing.length > 0) {
      problems.push({
        where: SEEDER_REL,
        why: `检测锚点消失：${missing.join(", ")} —— 本守卫已成空转（改名即静默通过）。请同步更新 WEIGHT_VAR_NAMES`,
      });
    }
  } catch (e) {
    problems.push({ where: SEEDER_REL, why: `回退守卫读不到文件：${e.message}` });
  }

  // 4c. 权威源自身的「引用卫生」：指针字段必须指向**真实存在**的东西
  //
  // 两个字段都是 `&'static str` 自由文本，而 Rust 侧断言只查「非空」：
  //   · `Axiom::enforced_by` —— 测试改名/删除 ⇒ 公理**静默失去守护者**；
  //   · `evidence` / `Divergence::site` —— 文件改名/挪目录 ⇒ 锚点**静默失效**。
  // 共同形态：**「指向的东西不存在」与「指向的东西通过了」在门禁看来是同一件事。**
  // 这是判据 #197「悬空解析」的同族 —— 值解析出来之后必须真的拿去比对。
  if (axioms) {
    for (const b of checkEnforcedByTargets(axioms, authority)) {
      problems.push({ where: AUTHORITY_REL, why: b.reason });
    }
  }
  // 只验文件存在、不验行号（行号必然随重构漂移，见 stripLineSuffix 的文档）
  for (const b of checkPointerFilesExist(pointerSites, (rel) => fs.existsSync(path.join(ROOT, rel)))) {
    problems.push({ where: `${AUTHORITY_REL}:${b.site}`, why: b.reason });
  }

  // 5. 散文档：注释里的权重（只报告）
  let proseScanned = 0;
  for (const f of ["src-tauri/src/commands/bottleneck-calc.rhai", "src-tauri/src/commands/strategy-scorer.rhai"]) {
    let text;
    try {
      text = readText(f, readCount);
    } catch {
      continue;
    }
    text.split(/\r?\n/).forEach((line, i) => {
      if (!isCommentLine(line)) return;
      proseScanned += 1;
      const nums = [...line.matchAll(/(?<![\w.])(\d+\.\d+)(?![\w.])/g)].map((m) => Number(m[1]));
      const claimsWeight = /w_supply|w_demand|w_irreplace/.test(line);
      if (!claimsWeight || nums.length === 0) return;
      const want = [weights.supply, weights.demand, weights.irreplaceability];
      const sameMultiset =
        nums.length === want.length && nums.every((n, k) => near(n, want[k]));
      if (!sameMultiset) {
        prose.push({ site: `${f}:${i + 1}`, note: `注释里的权重与权威源不一致：${nums.join("/")} vs ${want.join("/")}` });
      }
    });
  }

  // 6. 扫描面自证
  if (readCount.n === 0) {
    console.error("✖ 一个文件都没读到 ⇒ 判据失效，宁可红");
    process.exit(1);
  }

  // ── 输出 ──
  const divergencesOk = !STRICT_DIV || (divergences || []).length === 0;
  const failed = problems.length > 0 || !divergencesOk;

  if (JSON_OUT) {
    console.log(
      JSON.stringify(
        {
          authority: { weights, parts, bands, axioms },
          sites: siteReport,
          pointerSites,
          divergences,
          problems,
          prose,
          failed,
        },
        null,
        2,
      ),
    );
  } else {
    console.log("── 权威源（harness/src/domain_ontology.rs）──");
    for (const l of info) console.log(`   ${l}`);

    console.log("\n── 副本现场 ──");
    for (const s of siteReport) {
      console.log(`   ${s.site}`);
      console.log(`     ${s.note}｜实测：${s.found}`);
    }

    // `--list`：额外打印**引用类现场**（指针字段 / 公理守护者）。
    // ⚠ 它只改变**输出详略**，不改变判据 —— 退出码仍由 `problems` 决定，
    //   以免 `--list` 被当成「看一眼就好」的绿灯（本项目禁止 fail-open 入口）。
    if (LIST) {
      console.log("\n── 引用类现场：指针字段（evidence / site，只验文件存在，不验行号）──");
      for (const p of pointerSites) {
        const ok = fs.existsSync(path.join(ROOT, p.file));
        console.log(`   ${ok ? "✔" : "✖"} domain_ontology.rs:${p.site} ${p.field} → ${p.file}`);
      }
      console.log("\n── 引用类现场：公理守护者（enforced_by）──");
      for (const a of axioms || []) {
        const ok = new RegExp(`#\\[test\\][\\s\\S]{0,200}?\\bfn\\s+${a.enforcedBy}\\s*\\(`).test(authority);
        console.log(`   ${ok ? "✔" : "✖"} ${a.id} → ${a.enforcedBy || "（未声明）"}`);
      }
    }

    console.log("\n── 已登记的口径分歧（只报告，不拦）──");
    if (!divergences || divergences.length === 0) {
      console.log("   （无）");
    } else {
      for (const d of divergences) {
        console.log(`   ${d.id} ${d.concept}`);
        console.log(`     现场：${d.site}`);
        console.log(`     声称：${d.claim}`);
        console.log(`     状态：${d.status}`);
      }
    }

    if (prose.length > 0) {
      console.log("\n── 注释档（散文，只报告）──");
      for (const p of prose) console.log(`   ${p.site} —— ${p.note}`);
    }

    console.log(
      `\n扫描面：读取文件 ${readCount.n} 个｜注释行 ${proseScanned} 行｜` +
        `指针字段 ${pointerSites.length} 条｜公理 ${(axioms || []).length} 条`,
    );

    if (problems.length > 0) {
      console.log("\n── ✖ 硬拦问题 ──");
      for (const p of problems) console.log(`   ${p.where}\n     ${p.why}`);
    }

    const hardOk = problems.length === 0;
    console.log(
      `\n结论：${hardOk ? "✅ 副本与权威源一致" : `❌ ${problems.length} 处硬拦问题`}` +
        `${prose.length ? `｜另有 ${prose.length} 处注释档待人工确认` : ""}` +
        `${(divergences || []).length ? `｜口径分歧 ${divergences.length} 项待裁决` : ""}`,
    );
  }

  process.exit(failed ? 1 : 0);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))) {
  main();
}
