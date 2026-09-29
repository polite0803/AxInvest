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
 * | 决策腿 → 分析师桥 | `evidence_weight.rs` 的 `DECISION_LEG_ANALYST` | 桥里写错分析师名 ⇒ `horizon_leg_multipliers()` 查不到该键，乘数静默退 1.0（只 warn） |
 *
 * ## 判据
 *
 * ① 后端每档的每个分析师，前端必须都有且值相等（缺键 / 多键 / 值不等都算红）；
 * ② 桥表引用的每个分析师 id，必须在**四档**后端表里都存在；
 * ③ 扫描面非零（4 档 × 每档 ≥ 11 键），否则视为解析失效（exit 2）——
 *    「扫到 0 条」的门等于没有门（本仓已多次为这类假绿付过代价）。
 *
 * 用法：
 *   node scripts/check-horizon-weight-parity.mjs            # 比对
 *   node scripts/check-horizon-weight-parity.mjs --selftest # 正负对照（证明它会红）
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const ROOT = path.resolve(import.meta.dirname, "..");
const BACKEND = "src-tauri/crates/analysis-engine/src/evidence_weight.rs";
const FRONTEND = "src/lib/stock-analysis-utils.ts";
const TIERS = ["ultra_short", "short", "mid", "long"];

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
        problems.push(`桥表腿 ${leg} → 分析师 ${analyst} 在后端 ${t} 档不存在 ⇒ 该腿乘数静默退 1.0`);
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

function run() {
  const be = parseBackend(fs.readFileSync(path.join(ROOT, BACKEND), "utf8"));
  if (!be.ok) { console.log(`❌ 后端解析失败: ${be.reason}`); return 2; }
  const fe = parseFrontend(fs.readFileSync(path.join(ROOT, FRONTEND), "utf8"));
  if (!fe.ok) { console.log(`❌ 前端解析失败: ${fe.reason}`); return 2; }
  const surface = surfaceCheck(be, fe);
  if (surface.length) {
    surface.forEach((e) => console.log(`❌ ${e}`));
    return 2;
  }
  const problems = compare(be, fe);
  const keys = TIERS.reduce((n, t) => n + Object.keys(be.tiers[t]).length, 0);
  console.log(`比对面：4 档 × 后端合计 ${keys} 键 | 桥表 ${be.bridge.length} 腿`);
  if (problems.length === 0) {
    console.log("✅ 通过：前后端逐字段一致，且桥表引用的分析师在四档均有权重");
    return 0;
  }
  problems.forEach((p) => console.log(`✖ ${p}`));
  console.log(`\n❌ ${problems.length} 处不一致（真相源 = 后端 get_horizon_base_weights，改前端对齐它）`);
  return 1;
}

/**
 * 正负对照：证明本门**会红**，而不是恒报绿。
 * 三个正控都用「真实缺陷形态」在内存里复刻（缺键 / 值漂移 / 桥表写错分析师），
 * 负控用磁盘上的当前形态（应无问题）。
 */
function selftest() {
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
  delete dropKey.tiers.ultra_short["a-technical"];
  cases.push({ name: "正控① 前端删掉一个键 ⇒ 必须报缺键", got: compare(be, dropKey).length, want: 1 });

  const drift = clone(fe);
  drift.tiers.long["value-investor"] = 1.0;
  cases.push({ name: "正控② 前端改错一个值 ⇒ 必须报值不一致", got: compare(be, drift).length, want: 1 });

  const badBridge = clone(be);
  badBridge.bridge = [...badBridge.bridge, { leg: "f99", analyst: "a-nonexistent-analyst" }];
  cases.push({ name: "正控③ 桥表引用不存在的分析师 ⇒ 必须报 4 档各一条", got: compare(badBridge, fe).length, want: 4 });

  const emptyParse = { tiers: {}, bridge: [] };
  cases.push({ name: "★护栏 解析面为 0 ⇒ surfaceCheck 必须报错", got: surfaceCheck(emptyParse, emptyParse).length > 0 ? 1 : 0, want: 1 });

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
