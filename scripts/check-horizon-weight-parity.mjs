#!/usr/bin/env node
/**
 * 四周期「分析师权重表」等式门禁（四周期科学化 Phase A）。
 *
 * ## 为什么需要它
 *
 * 同一张「周期 × 分析师 → 权重」表有**两处载体**：
 *
 * | 载体 | 位置 | 机制 |
 * |---|---|---|
 * | **单一真相源** | `src-tauri/crates/analysis-engine/src/evidence_weight.rs` 的 `get_horizon_base_weights` | Rust `HashMap` 逐档显式 insert |
 * | 前端离线兜底 | `src/lib/stock-analysis-utils.ts` 的 `ANALYST_TIME_HORIZON_WEIGHT` | 手抄（`compute_evidence_weights` 命令失败时的共识兜底） |
 *
 * 兜底侧的读取函数 `getAnalystWeight` 对**缺键**的处理是 `weights.default ?? 1.0` ——
 * 也就是说「表里少一行」不会报错，只会**静默把该分析师的档位权重变成 1.0**。
 * 2026-09-29 实测：前端表比后端少 6 项（ultra_short 缺 `a-technical`/`macro`/`a-sector`，
 * short 缺 `macro`/`a-sector`/`research-mgr`），而 `mid`/`long` 两档完全一致 ——
 * 典型的「看起来同步、实际半张表失效」。本门禁把两侧逐字段对齐，并把桥表也纳入：
 *
 * | 第三处 | 位置 | 为什么一起查 |
 * |---|---|---|
 * | 决策腿 → 分析师桥 | `evidence_weight.rs` 的 `DECISION_LEG_ANALYST` | 桥里写错分析师名 ⇒ 该腿的证据域**没有属主**（同一域可能被计两次方向，或整条腿查无此人）。注：2026-10-04 起这张桥不再被投影成「逐档乘数表」（那张表随 R-11 退役），本判据锁的是属主与闭合两件事 |
 *
 * ## 判据
 *
 * ① 后端每档的每个分析师，前端必须都有且值相等（缺键 / 多键 / 值不等都算红）；
 * ② 桥表引用的每个分析师 id，必须在**四档**后端表里都存在；
 * ③ 扫描面非零（4 档 × 每档 ≥ 11 键），否则视为解析失效（exit 2）——
 *    「扫到 0 条」的门等于没有门（本仓已多次为这类假绿付过代价）；
 * ④ **四档显示名不得再手抄天数**：持有天数的唯一来源是 Rust
 *    `harness/src/holding_period.rs` 的 `default_holding_days`（2/5/28/90）。
 *    实证缺陷（2026-09-29 审计）：`stockAnalysis.reflection.horizonUltraShort` 在 11 个语言里
 *    写「(1-3天)」，而权威是 **2 交易日** —— 抄本腐烂了，且没有任何门会响。
 *    所以判据不是「数字要对」而是「标签里不许出现数字」：要显示天数，就渲染
 *    `byHorizon[].holdingDays`（后端权威表经 DTO 出边界），别再抄一遍。
 *
 * 用法：
 * ⑦ **三处键集合必须等于权威分析师清单**（后端权重表每档、前端兜底表每档，都严格等于
 *    `evidence_weight.rs` 的 `EVIDENCE_ANALYST_IDS`），且清单里每个 id 在 seed 里真有一个
 *    同名 Agent 节点。存在理由（2026-10-03 实测）：表里一度并存 `a-market`/`a-technical`/
 *    `capital`/`macro`/`fundamental`/`sentiment` 六个图中不存在的名字，而后端按裸节点 id
 *    精确查表 ⇒ 逐档权重静默退 1.0；判据①②只在「自选名」之间互相印证，抓不到这种漂移。
 *
 * ⑧ **闭合**：`DECISION_LEG_ANALYST` 桥到的分析师 ∪ `UNBRIDGED_ANALYST_IDS` = 权威清单，
 *    且两边不重叠。存在理由：「新增分析师
 *    只补了权重表、没接腿」在这套体系里不会报错，只会让那个分析师的证据**永远不进任何一档的
 *    融合** —— 与缺陷 ⑦ 同族但方向相反，
 *    必须登记成显式的「无腿 + 理由」（Rust 侧另有同名闭合测试，本判据保证不跑 cargo 也红）。
 *
 *   node scripts/check-horizon-weight-parity.mjs            # 比对
 *   node scripts/check-horizon-weight-parity.mjs --selftest # 正负对照（证明它会红）
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");
const BACKEND = "src-tauri/crates/analysis-engine/src/evidence_weight.rs";
const FRONTEND = "src/lib/stock-analysis-utils.ts";
const HOLDING = "src-tauri/crates/harness/src/holding_period.rs";
const SEED = "src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs";
const LOCALE_DIR = "src/i18n/locales";
const LABEL_KEYS = ["horizonUltraShort", "horizonShort", "horizonMid", "horizonLong"];
const TIERS = ["ultra_short", "short", "mid", "long"];

/** 相对仓库根读文件；读不到返回 null ⇒ 判据报「覆盖面失效」，不静默放行。 */
function readRel(rel) {
  try { return fs.readFileSync(path.join(ROOT, rel), "utf8"); } catch { return null; }
}

/** 解析后端 `get_horizon_base_weights`：`"tier" => { w.insert("id", 1.3); … }`。 */
function parseBackend(src) {
  const start = src.indexOf("fn get_horizon_base_weights");
  if (start < 0) { return { ok: false, reason: "未见 get_horizon_base_weights（函数改名？）" }; }
  // 只取到下一个顶层 `///` 文档块为止，避免吃进后面的桥表/乘数函数。
  const tail = src.indexOf("\n/// 计算市场周期调节系数", start);
  const body = src.slice(start, tail > start ? tail : src.length);
  const out = {};
  const tierRe = /"([a-z_]+)"\s*=>\s*\{([\s\S]*?)\n\s{8}\}/g;
  let m;
  while ((m = tierRe.exec(body)) !== null) {
    const row = {};
    const insRe = /w\.insert\(\s*"([a-z\-]+)"\s*,\s*([0-9.]+)\s*\)/g;
    let im;
    while ((im = insRe.exec(m[2])) !== null) { row[im[1]] = Number(im[2]); }
    out[m[1]] = row;
  }
  // `mid` 档在源码里写成 `"mid" => {…}` 之外还有 `_ =>` 兜底；兜底档不参与比对。
  const bridge = [];
  const bStart = src.indexOf("DECISION_LEG_ANALYST");
  if (bStart >= 0) {
    const bBody = src.slice(bStart, src.indexOf("];", bStart));
    const legRe = /\(\s*"([a-z0-9_]+)"\s*,\s*(Some\(\s*"([a-z\-]+)"\s*\)|None)\s*\)/g;
    let bm;
    while ((bm = legRe.exec(bBody)) !== null) {
      bridge.push({ leg: bm[1], analyst: bm[3] ?? null });
    }
  }
  return { ok: true, tiers: out, bridge };
}

/** 解析前端 `ANALYST_TIME_HORIZON_WEIGHT`：`tier: { "id": 1.3, … }`（`default` 键除外）。 */
function parseFrontend(src) {
  const start = src.indexOf("ANALYST_TIME_HORIZON_WEIGHT: Record");
  if (start < 0) { return { ok: false, reason: "未见 ANALYST_TIME_HORIZON_WEIGHT（常量改名？）" }; }
  const end = src.indexOf("\n};", start);
  if (end < 0) { return { ok: false, reason: "前端权重表收尾定位失败" }; }
  const body = src.slice(start, end);
  const out = {};
  const tierRe = /\n\s{2}(ultra_short|short|mid|long):\s*\{([\s\S]*?)\n\s{2}\},/g;
  let m;
  while ((m = tierRe.exec(body)) !== null) {
    const row = {};
    const kvRe = /"([a-z\-]+)"\s*:\s*([0-9.]+)/g;
    let km;
    while ((km = kvRe.exec(m[2])) !== null) { row[km[1]] = Number(km[2]); }
    out[m[1]] = row;
  }
  return { ok: true, tiers: out };
}

/** 解析后端权威清单 `pub const EVIDENCE_ANALYST_IDS: &[&str] = &[…]`。 */
function parseAuthority(src) {
  const start = src.indexOf("pub const EVIDENCE_ANALYST_IDS");
  if (start < 0) { return { ok: false, reason: "未见 EVIDENCE_ANALYST_IDS（常量改名？）" }; }
  const end = src.indexOf("];", start);
  if (end < 0) { return { ok: false, reason: "权威清单收尾定位失败" }; }
  const ids = [];
  const re = /"([a-z0-9-]+)"/g;
  let m;
  while ((m = re.exec(src.slice(start, end))) !== null) { ids.push(m[1]); }
  return { ok: true, ids };
}

/**
 * 判据⑦：后端每档 / 前端每档的键集合 **严格等于** 权威清单，
 * 且清单里每个 id 在 seed 中以 `"id"` 形态出现（防清单自己腐烂成新的幽灵名）。
 */
function checkAuthority(backend, frontend, authority, seedSrc) {
  const errs = [];
  if (!authority || !authority.ok) {
    return [`⑦ ${authority ? authority.reason : "权威清单读不到"} ⇒ 判据失效`];
  }
  if (!seedSrc) { return [`⑦ 读不到 ${SEED} ⇒ 存在性无法核对`]; }
  const ids = authority.ids;
  if (ids.length < 12) { errs.push(`⑦ 权威清单仅 ${ids.length} 项（< 12）⇒ 解析或清单本身失效`); }
  for (const t of TIERS) {
    for (const [side, src] of [["后端", backend.tiers[t]], ["前端", frontend.tiers[t]]]) {
      if (!src) { continue; }
      const keys = Object.keys(src);
      const missing = ids.filter((k) => !(k in src));
      const extra = keys.filter((k) => !ids.includes(k));
      if (missing.length) { errs.push(`⑦ ${t} ${side}表缺权威清单里的键: ${missing.join(", ")}`); }
      if (extra.length) {
        errs.push(`⑦ ${t} ${side}表有权威清单外的键（图中无此分析师 ⇒ 精确查表会静默退 1.0）: ${extra.join(", ")}`);
      }
    }
  }
  for (const id of ids) {
    if (!seedSrc.includes(`"${id}"`)) {
      errs.push(`⑦ 权威清单里的 '${id}' 在 seed 里找不到同名节点 ⇒ 清单已腐烂成幽灵 id，请核对节点是否改名`);
    }
  }
  return errs;
}

/** 解析后端 `pub const UNBRIDGED_ANALYST_IDS`（刻意无决策腿的分析师登记）。 */
function parseUnbridged(src) {
  const start = src.indexOf("pub const UNBRIDGED_ANALYST_IDS");
  if (start < 0) { return { ok: false, reason: "未见 UNBRIDGED_ANALYST_IDS" }; }
  const end = src.indexOf("];", start);
  if (end < 0) { return { ok: false, reason: "无腿清单收尾定位失败" }; }
  const ids = [];
  const re = /"([a-z0-9\-]+)"/g;
  let m;
  while ((m = re.exec(src.slice(start, end))) !== null) { ids.push(m[1]); }
  return { ok: true, ids };
}

/**
 * 判据⑧：有腿 ∪ 无腿 = 权威清单（闭合）。
 * Rust 侧同有 `bridged_plus_unbridged_covers_every_analyst`，这里是**不跑 cargo 也能红**的那一层
 * —— 因为「新增分析师时权重表补了、腿没接」恰好是最容易漏的一种，而它的表现是乘数永不命中。
 */
function checkLegClosure(backend, authority, unbridged) {
  const errs = [];
  if (!authority || !authority.ok) { return [`⑧ ${authority ? authority.reason : "权威清单读不到"} ⇒ 闭合判据失效`]; }
  if (!unbridged || !unbridged.ok) { return ["⑧ 未见 UNBRIDGED_ANALYST_IDS ⇒ 「没有决策腿」退化成查表落空，必须显式登记"]; }
  const bridged = [...new Set((backend.bridge ?? []).filter((r) => r.analyst).map((r) => r.analyst))];
  for (const id of unbridged.ids) {
    if (bridged.includes(id)) { errs.push(`⑧ '${id}' 既有腿又被登记为无腿 ⇒ 归因有歧义`); }
  }
  for (const id of authority.ids) {
    if (!bridged.includes(id) && !unbridged.ids.includes(id)) {
      errs.push(`⑧ '${id}' 未桥到任何腿、也未登记为无腿 ⇒ 其逐档权重静默不影响融合`);
    }
  }
  for (const id of bridged) {
    if (!authority.ids.includes(id)) { errs.push(`⑧ 桥表引用 '${id}' 不在权威清单 ⇒ 该腿无属主（该域证据进不了融合，或同一域被计两次方向）`); }
  }
  return errs;
}

/** 主比对：返回问题清单（空 = 绿）。 */
function compare(backend, frontend) {
  const problems = [];
  for (const t of TIERS) {
    const b = backend.tiers[t];
    const f = frontend.tiers[t];
    if (!b) { problems.push(`后端缺档 ${t}`); continue; }
    if (!f) { problems.push(`前端缺档 ${t}`); continue; }
    const missing = Object.keys(b).filter((k) => !(k in f)).map((k) => `${k}=${b[k]}`);
    const extra = Object.keys(f).filter((k) => !(k in b));
    const drift = Object.keys(b).filter((k) => k in f && Math.abs(b[k] - f[k]) > 1e-9)
      .map((k) => `${k}: 后端 ${b[k]} / 前端 ${f[k]}`);
    if (missing.length) { problems.push(`${t} 前端缺键（会被 default 静默吸收成 1.0）: ${missing.join(", ")}`); }
    if (extra.length) { problems.push(`${t} 前端多出后端没有的键: ${extra.join(", ")}`); }
    if (drift.length) { problems.push(`${t} 两侧值不一致: ${drift.join(", ")}`); }
  }
  for (const { leg, analyst } of backend.bridge ?? []) {
    if (!analyst) { continue; }
    for (const t of TIERS) {
      const row = backend.tiers[t];
      if (row && !(analyst in row)) {
        problems.push(`桥表腿 ${leg} → 分析师 ${analyst} 在后端 ${t} 档不存在 ⇒ 该腿属主在该档无权重（桥表与权威表分叉）`);
      }
    }
  }
  return problems;
}

/** 扫描面自证：解析到 0 条 = 门失效，必须比「值漂移」更早报出来。 */
function surfaceCheck(backend, frontend) {
  const errs = [];
  for (const t of TIERS) {
    const bn = Object.keys(backend.tiers[t] ?? {}).length;
    const fn = Object.keys(frontend.tiers[t] ?? {}).length;
    if (bn < 11) { errs.push(`后端 ${t} 仅解析到 ${bn} 键（< 11）⇒ 解析器失效，结论不可信`); }
    if (fn < 11) { errs.push(`前端 ${t} 仅解析到 ${fn} 键（< 11）⇒ 解析器失效，结论不可信`); }
  }
  if ((backend.bridge ?? []).length < 12) {
    errs.push(`桥表仅解析到 ${(backend.bridge ?? []).length} 条（< 12）⇒ 解析器失效`);
  }
  return errs;
}

/** ④ 天数唯一来源：Rust 权威表 `default_holding_days`（用于确认它还在、还解析得出）。 */
function parseAuthorityDays(src) {
  const start = src.indexOf("pub fn default_holding_days");
  if (start < 0) { return { ok: false, reason: "未见 default_holding_days（改名或搬家？）" }; }
  const body = src.slice(start, start + 900);
  const re = /Period::(UltraShort|Short|Mid|Long)\s*=>\s*(\d+)/g;
  const days = {};
  let m;
  while ((m = re.exec(body)) !== null) { days[m[1].toLowerCase()] = Number(m[2]); }
  const missing = ["ultrashort", "short", "mid", "long"].filter((k) => !(k in days));
  if (missing.length) { return { ok: false, reason: `权威表只解析到 ${Object.keys(days).length} 档，缺 ${missing.join("/")}` }; }
  return { ok: true, days };
}

/**
 * ④ 四档显示名不得手抄天数：命中「值里出现数字」即红。
 * 天数要显示就渲染 `byHorizon[].holdingDays`（权威表出边界的那一份）。
 */
function checkLabelsCarryNoDayRange(localeObjs) {
  const errs = [];
  for (const [lang, obj] of Object.entries(localeObjs)) {
    for (const key of LABEL_KEYS) {
      const v = obj[key];
      if (typeof v !== "string") { errs.push(`${lang}: reflection.${key} 缺失或非字符串`); continue; }
      if (/\d/.test(v)) { errs.push(`${lang}: reflection.${key} = ${JSON.stringify(v)} 里手抄了天数 ⇒ 权威表一改就静默腐烂`); }
    }
  }
  return errs;
}

/** ⑤ pending 行只能由共享构造器建（三处建点各抄默认值 = 缺陷 ④ 的成因）。 */
const PENDING_SITES = [
  "src-tauri/src/commands/stock_workflow/core.rs",
  "src-tauri/src/commands/stock_workflow/hooks.rs",
  "src-tauri/src/bin/axagent-batch-rerun.rs",
];
const REFLECTION_RS = "src-tauri/src/commands/stock_workflow/reflection.rs";
const REFLECTION_MD = "src-tauri/agency_experts/stock-analysis/reflection.md";
const SETUP_MOD = "src-tauri/src/commands/stock_analysis_setup/mod.rs";

function checkPendingBuilderSingleSource(read) {
  const errs = [];
  const builder = read(REFLECTION_RS);
  if (builder === null) { return [`⑤ ${REFLECTION_RS} 读不到 ⇒ 判据失效`]; }
  if (!builder.includes("pub fn build_pending_reflection_rows_for")) {
    errs.push("⑤ 共享构造器 build_pending_reflection_rows_for 不见了 ⇒ 判据失去锚点（改名请同步本门）");
  }
  for (const f of PENDING_SITES) {
    const src = read(f);
    if (src === null) { errs.push(`⑤ ${f} 读不到 ⇒ 该建点已失踪还是搬家？`); continue; }
    const n = (src.match(/stock_reflections::ActiveModel\s*\{/g) ?? []).length;
    if (n > 0) {
      errs.push(`⑤ ${f} 有 ${n} 处 stock_reflections::ActiveModel { 字面量构造 ⇒ 必须走 build_pending_reflection_rows（否则三处默认值/档口径再度分叉）`);
    }
  }
  return errs;
}

/** ⑥ 反思 prompt 必须「单档」：复盘档进了 prompt，且不许再要求跨档产出。 */
const BANNED_CROSS_HORIZON = [/对四个周期分别判断/, /必须.*四.*周期.*分别/];

function checkReflectionPromptSingleHorizon(read) {
  const errs = [];
  const md = read(REFLECTION_MD);
  const mod = read(SETUP_MOD);
  if (md === null) { return [`⑥ ${REFLECTION_MD} 读不到 ⇒ 判据失效`]; }
  if (mod === null) { return [`⑥ ${SETUP_MOD} 读不到 ⇒ 判据失效`]; }
  if (!md.includes("{{review_horizon}}")) {
    errs.push("⑥ reflection.md 未引用 {{review_horizon}} ⇒ 复盘档没进生成层（缺陷 ① 复发）");
  }
  for (const re of BANNED_CROSS_HORIZON) {
    if (re.test(md)) {
      errs.push(`⑥ reflection.md 命中跨档产出指令 ${re} ⇒ 与「一次反思只复盘一档」裁定冲突（缺陷 ②）`);
    }
  }
  // 两处 prompt 同源：节点内联 system_prompt 与 input_mapping 都要接上复盘档
  const hits = (mod.match(/review_horizon/g) ?? []).length;
  if (hits < 2) {
    errs.push(`⑥ ${SETUP_MOD} 仅 ${hits} 处引用 review_horizon（system_prompt + input_mapping 至少各 1）⇒ 漏一处即 VARIABLE_NOT_FOUND`);
  }
  return errs;
}

/** 读 11 个语言里 reflection 段的四个档名（解析不出 ⇒ 护栏）。 */
function readHorizonLabels() {
  const dir = path.join(ROOT, LOCALE_DIR);
  const out = {};
  for (const f of fs.readdirSync(dir).filter((x) => x.endsWith(".json"))) {
    const lang = f.slice(0, -5);
    let d;
    try { d = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")); }
    catch { out[lang] = { __broken: true }; continue; }
    const ref = d?.stockAnalysis?.reflection ?? {};
    out[lang] = Object.fromEntries(LABEL_KEYS.map((k) => [k, ref[k]]));
  }
  return out;
}

function run() {
  const beSrc = fs.readFileSync(path.join(ROOT, BACKEND), "utf8");
  const be = parseBackend(beSrc);
  if (!be.ok) { console.log(`❌ 后端解析失败: ${be.reason}`); return 2; }
  const fe = parseFrontend(fs.readFileSync(path.join(ROOT, FRONTEND), "utf8"));
  if (!fe.ok) { console.log(`❌ 前端解析失败: ${fe.reason}`); return 2; }
  const surface = surfaceCheck(be, fe);
  if (surface.length) {
    surface.forEach((e) => console.log(`❌ ${e}`));
    return 2;
  }
  const problems = compare(be, fe);
  // ⑦ 两侧每档键集合必须严格等于权威清单，且清单每个 id 在 seed 里真有一个同名节点。
  const authority = parseAuthority(beSrc);
  problems.push(...checkAuthority(be, fe, authority, readRel(SEED)));
  problems.push(...checkLegClosure(be, authority, parseUnbridged(beSrc)));
  // ④ 天数唯一来源 = Rust 权威表；标签里出现数字即红
  const auth = parseAuthorityDays(fs.readFileSync(path.join(ROOT, HOLDING), "utf8"));
  if (!auth.ok) {
    console.log(`❌ 权威天数表解析失败: ${auth.reason}`);
    return 2;
  }
  const builderErrs = checkPendingBuilderSingleSource(readRel);
  const promptErrs = checkReflectionPromptSingleHorizon(readRel);
  if (builderErrs.length || promptErrs.length) {
    [...builderErrs, ...promptErrs].forEach((e) => console.log(`❌ ${e}`));
    return 2;
  }
  const labels = readHorizonLabels();
  const labelErrs = checkLabelsCarryNoDayRange(labels);
  if (Object.keys(labels).length < 11) {
    console.log(`❌ 只读到 ${Object.keys(labels).length} 个语言文件（< 11）⇒ 判据 ④ 覆盖面失效`);
    return 2;
  }
  const keys = TIERS.reduce((n, t) => n + Object.keys(be.tiers[t]).length, 0);
  console.log(
    `比对面：4 档 × 后端合计 ${keys} 键 | 桥表 ${be.bridge.length} 腿 | 权威分析师清单 ${(authority.ids ?? []).length} 个 | 无腿登记 ${(parseUnbridged(beSrc).ids ?? []).length} 个 | 档名 ${Object.keys(labels).length} 语言 × ${LABEL_KEYS.length} 键（权威天数 ${auth.days.ultrashort}/${auth.days.short}/${auth.days.mid}/${auth.days.long} 交易日）`,
  );
  if (problems.length === 0 && labelErrs.length === 0) {
    console.log("✅ 通过：前后端逐字段一致、桥表分析师四档均有权重、两侧键集合等于权威清单且清单可落地 seed、有腿∪无腿闭合、档名标签未手抄天数");
    return 0;
  }
  problems.forEach((p) => console.log(`✖ ${p}`));
  labelErrs.forEach((p) => console.log(`✖ ${p}`));
  console.log("\n❌ 存在不一致（权重真相源 = 后端 get_horizon_base_weights；天数真相源 = holding_period.rs 的 default_holding_days）");
  return 1;
}

/**
 * 正负对照：证明本门**会红**，而不是恒报绿。
 * 三个正控都用「真实缺陷形态」在内存里复刻（缺键 / 值漂移 / 桥表写错分析师），
 * 负控用磁盘上的当前形态（应无问题）。
 */
function selftest() {
  const readRelDisk = (rel) => {
    try { return fs.readFileSync(path.join(ROOT, rel), "utf8"); } catch { return null; }
  };
  const be = parseBackend(fs.readFileSync(path.join(ROOT, BACKEND), "utf8"));
  const fe = parseFrontend(fs.readFileSync(path.join(ROOT, FRONTEND), "utf8"));
  if (!be.ok || !fe.ok) {
    console.log("SELFTEST FAIL：真实文件解析失败 ⇒ 无从对照");
    return 1;
  }
  const cases = [];
  const clone = (o) => structuredClone(o);

  cases.push({ name: "负控 磁盘现状 ⇒ 应无问题", got: compare(be, fe).length, want: 0 });

  const dropKey = clone(fe);
  delete dropKey.tiers.ultra_short["a-catalyst"];
  cases.push({ name: "正控① 前端删掉一个键 ⇒ 必须报缺键", got: compare(be, dropKey).length, want: 1 });

  const drift = clone(fe);
  drift.tiers.long["value-investor"] = 1.0;
  cases.push({ name: "正控② 前端改错一个值 ⇒ 必须报值不一致", got: compare(be, drift).length, want: 1 });

  const badBridge = clone(be);
  badBridge.bridge = [...badBridge.bridge, { leg: "f99", analyst: "a-nonexistent-analyst" }];
  cases.push({ name: "正控③ 桥表引用不存在的分析师 ⇒ 必须报 4 档各一条", got: compare(badBridge, fe).length, want: 4 });

  // ── 判据 ⑦：三处键集合必须等于权威分析师清单，且清单每个 id 在 seed 里真有一个同名节点 ──
  // 正控全部用「2026-10-03 实测过的真实缺陷形态」在内存里复刻（表里留着图中不存在的名字 /
  // 节点改名而清单没跟），不改生产码也不改磁盘文件。
  const beSrcDisk = fs.readFileSync(path.join(ROOT, BACKEND), "utf8");
  const authority = parseAuthority(beSrcDisk);
  const seedDisk = readRel(SEED);
  cases.push({
    name: "⑦负控 磁盘现状（两侧每档 = 12 id 清单，且清单可落地到 seed）⇒ 应无问题",
    got: checkAuthority(be, fe, authority, seedDisk).length,
    want: 0,
  });
  const phantom = clone(fe);
  phantom.tiers.mid["a-technical"] = 1.0;
  cases.push({
    name: "⑦正控 前端塞回一个图中不存在的名字 ⇒ 必须报 1 处",
    got: checkAuthority(be, phantom, authority, seedDisk).length,
    want: 1,
  });
  const thin = clone(be);
  delete thin.tiers.long["a-hot-money"];
  cases.push({
    name: "⑦正控 后端整档少一个权威 id ⇒ 必须报 1 处",
    got: checkAuthority(thin, fe, authority, seedDisk).length,
    want: 1,
  });
  const seedMissing = String(seedDisk ?? "").replace(/"a-lockup"/g, '"a-lockup-renamed"');
  cases.push({
    name: "⑦正控 seed 节点改名、清单没跟 ⇒ 必须报 1 处（清单腐烂成幽灵 id）",
    got: checkAuthority(be, fe, authority, seedMissing).length,
    want: 1,
  });
  const rottenList = { ok: true, ids: [...authority.ids, "a-phantom"] };
  cases.push({
    name: "⑦正控 清单自己掺一个幽灵 id ⇒ 必须报 9 处（两侧 × 4 档缺键 8 + seed 找不到 1）",
    got: checkAuthority(be, fe, rottenList, seedDisk).length,
    want: 9,
  });
  cases.push({
    name: "⑦护栏 常量改名 ⇒ 必须报判据失效（不能静默放行）",
    got: checkAuthority(be, fe, parseAuthority("pub const RENAMED_AWAY: &[&str] = &[];"), seedDisk).length,
    want: 1,
  });
  cases.push({
    name: "⑦护栏 seed 读不到 ⇒ 必须报存在性无法核对",
    got: checkAuthority(be, fe, authority, null).length,
    want: 1,
  });
  cases.push({
    name: "⑦真实清单非空且为 12 个 id（不是空表充当一致）",
    got: authority.ok && authority.ids.length === 12 ? 1 : 0,
    want: 1,
  });

  // ── 判据 ⑧：有腿 ∪ 无腿 = 权威清单 ──
  const unbridged = parseUnbridged(beSrcDisk);
  cases.push({
    name: "⑧负控 磁盘现状（8 个有腿 + 4 个登记无腿 = 12）⇒ 应无问题",
    got: checkLegClosure(be, authority, unbridged).length,
    want: 0,
  });
  cases.push({
    name: "⑧正控 无腿清单少登记一个 ⇒ 必须报 1 处（该分析师的逐档权重会静默落空）",
    got: checkLegClosure(be, authority, { ok: true, ids: unbridged.ids.filter((x) => x !== "a-news") }).length,
    want: 1,
  });
  const doubleBridged = { ...be, bridge: [...be.bridge, { leg: "f98", analyst: "a-news" }] };
  cases.push({
    name: "⑧正控 把登记为无腿的分析师又接上腿 ⇒ 必须报重叠 1 处",
    got: checkLegClosure(doubleBridged, authority, unbridged).length,
    want: 1,
  });
  cases.push({
    name: "⑧正控 桥表引用清单外的人 ⇒ 必须报 1 处",
    got: checkLegClosure({ ...be, bridge: [...be.bridge, { leg: "f97", analyst: "a-ghost" }] }, authority, unbridged).length,
    want: 1,
  });
  cases.push({
    name: "⑧护栏 常量失踪 ⇒ 必须报判据失效",
    got: checkLegClosure(be, authority, parseUnbridged("pub const RENAMED_AWAY: &[&str] = &[];")).length,
    want: 1,
  });

  const emptyParse = { tiers: {}, bridge: [] };
  cases.push({ name: "★护栏 解析面为 0 ⇒ surfaceCheck 必须报错", got: surfaceCheck(emptyParse, emptyParse).length > 0 ? 1 : 0, want: 1 });

  // ── 判据 ④：档名标签不得手抄天数（天数唯一来源 = Rust 权威表）──
  const realLabels = readHorizonLabels();
  cases.push({ name: "④负控 磁盘现状（标签已剥离天数）⇒ 应无问题", got: checkLabelsCarryNoDayRange(realLabels).length, want: 0 });

  const rebaked = structuredClone(realLabels);
  rebaked["zh-CN"].horizonUltraShort = "超短线 (1-3天)";
  cases.push({ name: "④正控 把「(1-3天)」抄回一个标签 ⇒ 必须报 1 处", got: checkLabelsCarryNoDayRange(rebaked).length, want: 1 });

  const allBad = {};
  for (const lang of Object.keys(realLabels)) {
    allBad[lang] = { horizonUltraShort: "超短线 (1-3天)", horizonShort: "短线 (5天)", horizonMid: "中线 (28天)", horizonLong: "长线 (90天)" };
  }
  cases.push({ name: "④规模 11 语言全抄 ⇒ 必须报 44 处（不是只报第一处）", got: checkLabelsCarryNoDayRange(allBad).length, want: 44 });

  // ⑤/⑥ 新门自证：用**修复前的真实形态**喂进去，必须红
  const fakeRead = (map) => (rel) => (rel in map ? map[rel] : null);
  const cleanSites = Object.fromEntries(PENDING_SITES.map((f) => [f, "let x = 1;"]));
  cases.push({
    name: "⑤负控 三建点均走共享构造器 ⇒ 应无问题",
    got: checkPendingBuilderSingleSource(fakeRead({ [REFLECTION_RS]: "pub fn build_pending_reflection_rows_for", ...cleanSites })).length,
    want: 0,
  });
  cases.push({
    name: "⑤正控 某建点退回字面量构造 ⇒ 必须报 1 处",
    got: checkPendingBuilderSingleSource(
      fakeRead({
        [REFLECTION_RS]: "pub fn build_pending_reflection_rows_for",
        [PENDING_SITES[0]]: "let _ = stock_reflections::ActiveModel { id: Set(x) };",
        ...PENDING_SITES.slice(1).reduce((m, f) => ({ ...m, [f]: "" }), {}),
      }),
    ).length,
    want: 1,
  });
  cases.push({
    name: "⑤护栏 构造器改名/失踪 ⇒ 必须报（判据不能静默放行）",
    got: checkPendingBuilderSingleSource(fakeRead({ [REFLECTION_RS]: "", ...cleanSites })).length,
    want: 1,
  });
  cases.push({
    name: "⑥负控 磁盘现状 ⇒ 应无问题",
    got: checkReflectionPromptSingleHorizon(readRelDisk).length,
    want: 0,
  });
  cases.push({
    name: "⑥正控 还原批次 3 的跨档指令 ⇒ 必须报 4 处（2 条跨档指令 + 缺 review_horizon + 内联 prompt 未接）",
    got: checkReflectionPromptSingleHorizon(
      fakeRead({
        [REFLECTION_MD]: "1. 逐周期分析：必须对四个周期分别判断决策对错。",
        [SETUP_MOD]: "input_mapping 只有 actual_market_text",
      }),
    ).length,
    want: 4,
  });

  cases.push({
    name: "④护栏 权威表解析失效（改名）⇒ 必须 ok:false",
    got: parseAuthorityDays("pub fn renamed_away() {}").ok ? 1 : 0,
    want: 0,
  });
  cases.push({
    name: "④护栏 权威表真实可读（2/5/28/90）",
    got: (() => {
      const a = parseAuthorityDays(fs.readFileSync(path.join(ROOT, HOLDING), "utf8"));
      return a.ok && a.days.ultrashort === 2 && a.days.long === 90 ? 1 : 0;
    })(),
    want: 1,
  });

  let failed = 0;
  console.log("=== 门禁自检（正负对照）===");
  for (const c of cases) {
    const pass = c.got === c.want;
    if (!pass) { failed++; }
    console.log(`  ${pass ? "✅" : "❌"} ${c.name}（实得 ${c.got}，期望 ${c.want}）`);
  }
  console.log(failed === 0 ? "\nSELFTEST PASS" : `\nSELFTEST FAIL (${failed})`);
  return failed === 0 ? 0 : 1;
}

const argv = process.argv.slice(2);
if (process.argv[1] && process.argv[1].endsWith("check-horizon-weight-parity.mjs")) {
  process.exit(argv.includes("--selftest") ? selftest() : run());
}
