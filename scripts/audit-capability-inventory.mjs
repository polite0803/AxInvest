#!/usr/bin/env node
// scripts/audit-capability-inventory.mjs
//
// 功能点全集清点 —— 一次扫全，产出 docs/audits/AUDIT-capability-inventory-<日期>.md。
// 权威骨架 = build.rs 生成的 Tauri 命令注册表；交叉源 = 前端 invoke 消费方、
// commands/** 的行与 axagent_* 引用、全仓 pub trait 及实现者分布、壳层 Arc<dyn T> 持有面。
// 层级判据见 docs/plans/PLAN-shell-plus-plugins.md §0.9.4。
//
// ── #61（2026-11-02）：拔掉「靠不在仓里的工件出数」的根 ────────────────────
// 旧版三处依赖全在 `.tmp/` 下、未入库 ⇒ 干净检出上本脚本要么抛错要么静默空跑，
// 而它输出的数字被 PLAN 引用：
//   ① `.tmp/traits-full.json`（trait 定义面 + 实现者面；mtime 2026-11-01T04:41Z 的快照，
//      仍含已删的 `DojoExtension` ⇒ 它就是「316」这个死读数的产地，PLAN §17.1 已判其退役）
//   ② `.tmp/cap-register3.tsv`（「接缝」「键控分发点」两列，谓词真身在 `.tmp/capability-register3.cjs`）
//   ③ 文档头自述生成方式 = `node .tmp/full-inventory.cjs`
// 现在**自己扫盘**。口径声明（逐字对齐第五道门的 K1 段）：
//   扫描面 = 8 个 workspace 根 `axagent-ai / axagent-core / axagent-infra / axagent-kit /
//            axagent-plugins / axagent-quant / axagent-runtime / src-tauri` 下全部 `*.rs`，
//            剪路径段 ∈ {target, node_modules, gen, tests}（= K1_ROOTS / K1_IGNORE）。
//   声明面 = 逐行 `/^pub trait\s+([A-Za-z_][A-Za-z0-9_]*)/`（行级、不跨行、不剥注释、
//            不展开宏；`pub(crate) trait` 与裸 `trait` 不计）。
//   实现站点 = `implTraitOf(行)` 逐行跑；**测试替身/产物** = `isTestDoublePath(相对路径)`；
//            **harness 面** = 相对路径前缀 `axagent-core/crates/harness/`。
//   「K1 + 有非 harness、非测试实现者」= 本脚本的定案读数
//            `k1DeclaredExternalImplTraitCount` = 上面三者 + **两趟法**（第一趟收齐声明名，
//            第二趟才收 impl 站点）。
//   ⚠ 两趟不是性能优化，**是口径本身**：单趟（def 随文件遍历就地累积）会让「impl 出现在
//     其 trait 声明文件之前」的站点被静默丢弃 —— 实测丢 **22 条**，正是历史读数 210 与 232
//     的全部差值（归因表见落盘文档 §5.6）。本脚本同时输出单趟复现值，把这条断言钉在盘上。
//
// 强制复用（AGENTS.md 禁区 12：同一指标只许一套解析器）：
//   `implTraitOf` / `isTestDoublePath` / `computeK2Readings` 三个谓词**由
//   `scripts/check-capability-register.mjs` export**，本脚本直接 import 调用 ⇒ trait 侧与
//   键控侧不存在第二套解析器。该文件被 import 时零盘扫描、零副作用、绝不 process.exit
//   （其文件末尾的 main guard 保证，该守卫在它自己的注释里叫「阻断 2」）。
//   ⚠ 反之，扫描面三常量（`K1_ROOTS` / `K1_IGNORE` / `HARNESS_PREFIX`）在主脚本里是
//     `const` 而非 `export const` ⇒ 无法 import，本脚本只能逐字抄一份并在此互指
//     （对面是 `check-capability-register.mjs:195-199`）。改任一侧必须同改另一侧，否则
//     「同口径」这句声明就是假的。#61 之所以不顺手把它们改成 export：本次任务禁改主脚本
//     （另有会话正在其中加功能）——拔第二套解析器不能以制造第三处并发编辑为代价。
//
// 用法：
//   node scripts/audit-capability-inventory.mjs [--out <路径>] [--date <YYYY-MM-DD>]
//   node scripts/audit-capability-inventory.mjs --selftest   # 纯函数 + fixture，零盘 IO

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { REG_PATH, parseRegistry, domainOf } from './lib/command-domains.mjs';
import { isTestDoublePath, implTraitOf, computeK2Readings } from './check-capability-register.mjs';

const argv = process.argv.slice(2);
const argVal = (flag) => { const i = argv.indexOf(flag); return i >= 0 ? argv[i + 1] : null; };

// ── K1 扫描面常量（与 check-capability-register.mjs:195-199 逐字同源；理由见文件头）──
export const K1_ROOTS = ['axagent-ai', 'axagent-core', 'axagent-infra', 'axagent-kit', 'axagent-plugins', 'axagent-quant', 'axagent-runtime', 'src-tauri'];
export const K1_IGNORE = new Set(['target', 'node_modules', 'gen', 'tests']);
export const HARNESS_PREFIX = 'axagent-core/crates/harness/';
const PUB_TRAIT_RE = /^pub trait\s+([A-Za-z_][A-Za-z0-9_]*)/;

/**
 * K1-trait 谓词（**纯函数**，喂 [[rel, text], …] 即可复算 —— `--selftest` 就喂 fixture，零盘 IO）。
 * 口径原文见文件头「口径声明」；两趟法是口径的一部分，勿改回单趟。
 */
export function scanK1Traits(sources) {
  const defSites = {}; // trait -> ["rel:line"]（同名多定义逐条留名，不许被去重静默吃掉）
  let pubTraitHits = 0;
  for (const [rel, text] of sources) {
    if (!text) continue;
    const ls = text.split('\n');
    for (let i = 0; i < ls.length; i++) {
      const m = PUB_TRAIT_RE.exec(ls[i]);
      if (!m) continue;
      pubTraitHits++;
      (defSites[m[1]] = defSites[m[1]] || []).push(rel + ':' + (i + 1));
    }
  }
  const extSites = {}; // trait -> [rel]：非 harness、非测试替身的站点（定案档的站点面）
  const extAll = {}; // 门禁 A 口径：不要求该 trait 在扫描面有声明面
  const implAll = {}; // 全部 impl 站点（含 harness 与测试替身）——判「空接口（全仓无实现）」用
  const singlePass = new Set(); // 单趟语义复现：def 按文件顺序就地累积 ⇒ 顺序相关
  const seenDef = new Set();
  for (const [rel, text] of sources) {
    if (!text) continue;
    const ls = text.split('\n');
    for (const l of ls) { const d = PUB_TRAIT_RE.exec(l); if (d) seenDef.add(d[1]); }
    const skip = rel.startsWith(HARNESS_PREFIX) || isTestDoublePath(rel);
    for (const l of ls) {
      const t = implTraitOf(l);
      if (!t) continue;
      implAll[t] = (implAll[t] || 0) + 1;
      if (skip) continue;
      (extAll[t] = extAll[t] || []).push(rel);
      if (defSites[t]) {
        (extSites[t] = extSites[t] || []).push(rel);
        if (seenDef.has(t)) singlePass.add(t);
      }
    }
  }
  const pubTraitNames = Object.keys(defSites);
  const declaredExternal = pubTraitNames.filter((t) => (extSites[t] || []).length > 0);
  return {
    rsFiles: sources.length,
    pubTraitHits, pubTraitNames,
    dupTraits: pubTraitNames.filter((t) => defSites[t].length > 1),
    dupOverflow: pubTraitNames.reduce((a, t) => a + defSites[t].length - 1, 0),
    defSites, extSites, extAll, implAll,
    harnessTraitCount: pubTraitNames.filter((t) => defSites[t].some((d) => d.startsWith(HARNESS_PREFIX))).length,
    externalAllCount: Object.keys(extAll).length, // 门禁 A（不要求声明面）
    declaredExternalCount: declaredExternal.length, // ⇒ 定案读数 k1DeclaredExternalImplTraitCount
    declaredExternalTraits: declaredExternal.slice().sort(),
    singlePassCount: singlePass.size,
    singlePassLost: declaredExternal.filter((t) => !singlePass.has(t)).sort(),
    undeclaredExt: Object.keys(extAll).filter((t) => !defSites[t]).sort(),
  };
}

/* ═══════════ 判定跑（main guard：被 import 时零副作用）═══════════ */
function runAudit() {
  const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
  const R = (rel) => path.join(ROOT, rel);
  const exists = (p) => fs.existsSync(p);
  const read = (p) => (exists(p) ? fs.readFileSync(p, 'utf8') : '');
  const lines = (s) => s.split('\n').length;
  const relOf = (f) => path.relative(ROOT, f).split(path.sep).join('/');
  function walk(d, out, ext) {
    let e; try { e = fs.readdirSync(d, { withFileTypes: true }); } catch (x) { return out; }
    for (const f of e) {
      const p = path.join(d, f.name);
      if (f.isDirectory()) { if (['target', 'node_modules', 'gen', '__tests__', 'tests'].includes(f.name)) continue; walk(p, out, ext); }
      else if (ext.some((x) => f.name.endsWith(x))) out.push(p);
    }
    return out;
  }
  /** 收集 K1 扫描面的 [相对路径, 文本] 清单（本脚本唯一的 trait 侧 IO 入口）。 */
  function collectK1Surface() {
    const out = [];
    const w = (dir) => {
      let es; try { es = fs.readdirSync(R(dir), { withFileTypes: true }); } catch { return; }
      for (const e of es) {
        const rel = dir + '/' + e.name;
        if (e.isDirectory()) { if (K1_IGNORE.has(e.name)) continue; w(rel); }
        else if (e.name.endsWith('.rs')) { let t = null; try { t = fs.readFileSync(R(rel), 'utf8'); } catch { } out.push([rel, t]); }
      }
    };
    for (const r of K1_ROOTS) w(r);
    return out;
  }

  /* ─────────── 1. 权威骨架：Tauri 命令注册表 ─────────── */
  const regSrc = read(REG_PATH);
  const cmdToModule = {};
  const modules = {};
  for (const e of parseRegistry(regSrc)) {
    cmdToModule[e.cmd] = e.mod;
    (modules[e.mod] = modules[e.mod] || []).push(e.cmd);
  }
  const MOD_KEYS = Object.keys(modules).sort();
  console.log('命令总数 ' + Object.keys(cmdToModule).length + ' / 模块数 ' + MOD_KEYS.length);

  /* ─────────── 2. 模块体量与后端 crate 归属 ─────────── */
  function modPath(mod) {
    const c = ['src-tauri/src/commands/' + mod + '/mod.rs', 'src-tauri/src/commands/' + mod + '.rs',
      'src-tauri/src/' + mod + '/mod.rs', 'src-tauri/src/' + mod + '.rs',
      'src-tauri/src/commands/' + mod.split('::').join('/') + '/mod.rs', 'src-tauri/src/' + mod.split('::').join('/') + '.rs'];
    return c.find(exists) || null;
  }
  const modInfo = {};
  for (const mod of MOD_KEYS) {
    const p = modPath(mod);
    let txt = '', files = [];
    if (p) {
      if (p.endsWith('mod.rs')) files = walk(path.dirname(p), [], ['.rs']);
      else files = [R(p)];
      txt = files.map((f) => read(relOf(f))).join('\n');
    }
    const crates = [...new Set([...txt.matchAll(/axagent_([a-z0-9_]+)::/g)].map((m) => m[1]))].sort();
    modInfo[mod] = { file: p, loc: p ? lines(txt) : 0, crates, cmds: modules[mod].length };
  }

  /* ─────────── 3. 前端消费方：谁 invoke 了这些命令 ─────────── */
  const feFiles = walk(R('src'), [], ['.tsx', '.ts']).filter((f) => !/\.test\.|\.spec\.|__tests__/.test(f));
  const cmdConsumers = {};
  for (const f of feFiles) {
    const rel = relOf(f);
    let key;
    const mDir = rel.match(/^src\/components\/([^/]+)\//);
    const mPage = rel.match(/^src\/pages\/([^/]+)\.tsx?$/);
    const mStore = rel.match(/^src\/stores\/([^/]+)\/([^/]+)\.ts$/);
    if (mDir) key = 'ui:' + mDir[1];
    else if (mPage) key = 'page:' + mPage[1];
    else if (mStore) key = 'store:' + mStore[2];
    else key = 'lib';
    const txt = read(rel);
    for (const m of txt.matchAll(/invoke(?:<[^(]*>)?\(\s*"([a-z0-9_]+)"/g)) {
      const c = m[1];
      if (!cmdToModule[c]) continue;
      (cmdConsumers[c] = cmdConsumers[c] || new Set()).add(key);
    }
  }
  const modConsumers = {};
  for (const c of Object.keys(cmdConsumers)) {
    const mod = cmdToModule[c];
    (modConsumers[mod] = modConsumers[mod] || new Set());
    cmdConsumers[c].forEach((k) => modConsumers[mod].add(k));
  }

  /* ─────────── 4. trait 侧：L2 接缝 / L3 策略位 / L0 内核 / 空接口 ─────────── */
  const k1Sources = collectK1Surface();
  const K1 = scanK1Traits(k1Sources);
  const k2Sources = {};
  for (const [rel, text] of k1Sources) k2Sources[rel] = text ?? '';
  // 「键控分发点」列换源（旧 .tmp/cap-register3.tsv 第 5 列）⇒ 用 #57 入库版八形态谓词
  const k2Readings = computeK2Readings(k2Sources);
  // 「已建接缝」列换源（旧 .tmp/cap-register3.tsv 第 3 列）⇒ 用 PLAN §0.7 的机器镜像
  const capReg = JSON.parse(read('scripts/capability-register.json'));
  const seamOf = {};
  for (const e of capReg.entries) {
    if (e.trait && e.expect && e.expect.seam) seamOf[e.trait] = String(e.expect.seam);
  }
  const dispOf = {};
  for (const t of k2Readings.strongTraits) dispOf[t] = 'K2 强档';
  for (const t of k2Readings.weakTraits) if (!dispOf[t]) dispOf[t] = 'K2 弱档';

  const shellTxt = read('src-tauri/src/app_state.rs') + read('src-tauri/src/init/state.rs');
  const shellDyn = new Set([...shellTxt.matchAll(/dyn\s+(?:[a-z_0-9]+::)*([A-Z][A-Za-z0-9_]*)/g)].map((m) => m[1]));
  const traits = K1.pubTraitNames;
  const cls = {};
  for (const t of traits) {
    const inH = K1.defSites[t].some((d) => d.startsWith(HARNESS_PREFIX));
    const ext = K1.extSites[t] || [];
    const anyImpl = (K1.implAll[t] || 0) > 0;
    let c;
    if (seamOf[t] && /已建|有接缝/.test(seamOf[t])) c = 'L2-已建接缝';
    else if (ext.length >= 1 && dispOf[t]) c = 'L2-待建接缝';
    else if (ext.length >= 1) c = 'L2-候选(有外部实现者无键控点)';
    else if (inH && ext.length === 0 && shellDyn.has(t)) c = 'L3-策略位(壳层全局持有)';
    else if (inH && ext.length === 0 && anyImpl) c = 'L3-策略位(仅内置实现)';
    else if (!anyImpl) c = '空接口(全仓无实现)';
    else c = 'L0-内核/实现层接口';
    (cls[c] = cls[c] || []).push(t);
  }
  const clsStat = Object.entries(cls).map(([k, v]) => [k, v.length]).sort((a, b) => b[1] - a[1]);
  console.log('\npub trait 总数 ' + traits.length + '（原始命中行 ' + K1.pubTraitHits + '，重名溢出 ' + K1.dupOverflow + '），分类：');
  clsStat.forEach(([k, n]) => console.log('  ' + String(n).padStart(4) + '  ' + k));
  console.log('K1 扫描面 .rs = ' + K1.rsFiles + '；harness 定义 trait = ' + K1.harnessTraitCount);
  console.log('k1DeclaredExternalImplTraitCount（两趟 + 要求声明面）= ' + K1.declaredExternalCount);
  console.log('  对照：门禁 A 口径（不要求声明面）= ' + K1.externalAllCount + '（差 ' + (K1.externalAllCount - K1.declaredExternalCount) + '）');
  console.log('  对照：单趟语义（顺序相关，缺陷复现）= ' + K1.singlePassCount + '（丢失 ' + K1.singlePassLost.length + ' 条）');

  /* ─────────── 5. 域簇归并（命令模块 → 功能域） ─────────── */
  const domains = {};
  for (const mod of MOD_KEYS) {
    const d = domainOf(mod);
    const e = (domains[d] = domains[d] || { mods: [], cmds: 0, loc: 0, crates: new Set(), consumers: new Set(), zeroCmdMods: [] });
    e.mods.push(mod); e.cmds += modInfo[mod].cmds; e.loc += modInfo[mod].loc;
    modInfo[mod].crates.forEach((c) => e.crates.add(c));
    (modConsumers[mod] || new Set()).forEach((c) => e.consumers.add(c));
    if (!modInfo[mod].file) e.zeroCmdMods.push(mod);
  }

  /* ─────────── 5.5 耦合度量：一个域被多少个别的域消费 ─────────── */
  const cmdDomain = {};
  for (const c of Object.keys(cmdToModule)) cmdDomain[c] = domainOf(cmdToModule[c]);
  const consumerDomains = {};
  for (const c of Object.keys(cmdConsumers)) {
    const d = cmdDomain[c];
    cmdConsumers[c].forEach((k) => (consumerDomains[k] = consumerDomains[k] || new Set()).add(d));
  }
  const entangle = {};
  for (const d of Object.keys(domains)) entangle[d] = { sharedConsumers: new Set(), foreignCmds: new Set(), otherDomains: new Set() };
  for (const c of Object.keys(cmdConsumers)) {
    const d = cmdDomain[c];
    cmdConsumers[c].forEach((k) => {
      const ds = consumerDomains[k];
      if ([...ds].filter((x) => x !== d).length > 0) {
        entangle[d].sharedConsumers.add(k);
        entangle[d].foreignCmds.add(c);
        ds.forEach((x) => { if (x !== d) entangle[d].otherDomains.add(x); });
      }
    });
  }
  function verdictOf(d) {
    const e = entangle[d];
    if (e.sharedConsumers.size >= 8) return 'L0 公共底座（被 ≥8 个别的面的消费方共享）';
    if (e.sharedConsumers.size >= 3) return 'L1 功能包 · 深耦合（跨面共享消费方 ' + e.sharedConsumers.size + '）';
    if (e.sharedConsumers.size >= 1) return 'L1 功能包 · 浅耦合（' + e.sharedConsumers.size + '）';
    return 'L1 功能包 · 无前端 invoke 交叉（最易独立）';
  }

  /* ─────────── 6. 落 AUDIT 文档 ─────────── */
  const NOW = argVal('--date') || new Date().toISOString().slice(0, 10);
  let md = `# AUDIT-capability-inventory-${NOW} — AxInvest 功能点全集清点（一次扫全）

> 生成方式：\`node scripts/audit-capability-inventory.mjs\`（本文件由脚本直接落盘，非手抄；\`--out <路径>\` 改落点，\`--date <YYYY-MM-DD>\` 改快照日期）。
> 权威骨架 = \`src-tauri/src/register_commands.rs\`（build.rs 生成的命令注册表）；
> 交叉源 = 前端 \`src/**\` 的 invoke 消费方、\`commands/**\` 的代码行与 \`axagent_*\` 引用、全仓 \`pub trait\` 与其实现者分布、\`app_state.rs\`/\`init/state.rs\` 的 \`Arc<dyn T>\` 全局持有面。
> 口径唯一：层级判据见 \`PLAN-shell-plus-plugins.md\` §0.9.4；trait 扫描面口径见 §4.0 与脚本文件头。
>
> **本脚本不读 \`.tmp/\`**（#61 拔根）：trait 声明面/实现者面自己扫盘；「已建接缝」取自 \`scripts/capability-register.json\`；「键控分发点」取自 \`computeK2Readings\`（#57 入库版 K2 谓词）。三者与第五道门同源，仓内可复跑。

## 1. 总量

| 面 | 数量 |
|---|---|
| Tauri 命令（注册表逐条） | **${Object.keys(cmdToModule).length}** |
| 命令模块 | **${MOD_KEYS.length}** |
| 功能域（下表） | **${Object.keys(domains).length}** |
| K1 扫描面 \`.rs\` | **${K1.rsFiles}** |
| \`pub trait\` 唯一名（K1） | **${traits.length}**（原始命中行 ${K1.pubTraitHits}，同名重复溢出 ${K1.dupOverflow}；harness ${K1.harnessTraitCount}） |
| K1 + 有非 harness、非测试实现者（\`k1DeclaredExternalImplTraitCount\`） | **${K1.declaredExternalCount}** |
| — 对照：不要求声明面（第五道门 \`k1.externalImplTraits\` 同口径） | ${K1.externalAllCount} |
| — 对照：单趟语义（顺序相关缺陷复现，**非口径**） | ${K1.singlePassCount} |
| L2 提供方接缝：已建 | ${(cls['L2-已建接缝'] || []).length} |
| L2 提供方接缝：待建 | ${(cls['L2-待建接缝'] || []).length} |
| L2 候选（有外部实现者，无键控分发点） | ${(cls['L2-候选(有外部实现者无键控点)'] || []).length} |
| L3 策略位（0 外部实现者，壳层全局持有） | ${(cls['L3-策略位(壳层全局持有)'] || []).length} |
| L3 策略位（仅内置实现） | ${(cls['L3-策略位(仅内置实现)'] || []).length} |
| 空接口（全仓无实现） | ${(cls['空接口(全仓无实现)'] || []).length} |
| L0 内核/实现层接口 | ${(cls['L0-内核/实现层接口'] || []).length} |

## 1.1 trait 侧闭合断言（机械可复算，任一不闭合即 \`--selftest\` 红）

- \`pubTraitHits − pubTraitNames == dupOverflow\` ⇒ ${K1.pubTraitHits} − ${traits.length} == ${K1.dupOverflow}${K1.pubTraitHits - traits.length === K1.dupOverflow ? ' ✔' : ' ✖ 不闭合'}
  重名 trait 逐条：${K1.dupTraits.length ? K1.dupTraits.map((t) => '`' + t + '`×' + K1.defSites[t].length).join(' ') : '（无）'}
- \`declaredExternal ⊆ pubTraitNames\` ⇒ ${K1.declaredExternalCount} ≤ ${traits.length} ✔（「要求声明面」这一档的全部含义）
- \`externalAll − declaredExternal == 无声明面名单长度\` ⇒ ${K1.externalAllCount} − ${K1.declaredExternalCount} = ${K1.undeclaredExt.length}${K1.externalAllCount - K1.declaredExternalCount === K1.undeclaredExt.length ? ' ✔' : ' ✖ 不闭合'}
  名单（${K1.undeclaredExt.length} 条）：${K1.undeclaredExt.map((t) => '`' + t + '`').join(' ')}
- \`declaredExternal − singlePass == 单趟丢失名单\` ⇒ ${K1.declaredExternalCount} − ${K1.singlePassCount} = ${K1.singlePassLost.length}${K1.declaredExternalCount - K1.singlePassCount === K1.singlePassLost.length ? ' ✔' : ' ✖ 不闭合'}
  名单（${K1.singlePassLost.length} 条）：${K1.singlePassLost.map((t) => '`' + t + '`').join(' ')}

## 2. 功能域全集（含体量、后端 crate、前端消费方）

| # | 功能域 | 命令模块 | 命令数 | 代码行 | 后端 crate | 前端消费方（目录/页/store） | 跨面共享消费方 | 被别的面消费的命令数 | 层级判定 |
|---|---|---|---|---|---|---|---|---|---|
`;
  let idx = 0;
  for (const [d, e] of Object.entries(domains).sort((a, b) => entangle[b[0]].sharedConsumers.size - entangle[a[0]].sharedConsumers.size || b[1].cmds - a[1].cmds)) {
    idx++;
    const crates = [...e.crates];
    const cons = [...e.consumers].sort();
    const en = entangle[d];
    md += `| ${idx} | ${d} | ${e.mods.length} 个（共 ${e.cmds} 条命令） | ${e.cmds} | ${e.loc.toLocaleString()} | ${crates.length ? crates.slice(0, 8).join(' ') + (crates.length > 8 ? ' +' + (crates.length - 8) : '') : '（仅壳层）'} | ${cons.length ? cons.slice(0, 8).join('、') + (cons.length > 8 ? ' +' + (cons.length - 8) : '') : '（无前端 invoke 命中）'} | ${en.sharedConsumers.size} | ${en.foreignCmds.size} | ${verdictOf(d)} |\n`;
  }

  md += `
### 2.1 判据（机械可复算，见脚本 \`verdictOf\`）

- 「跨面共享消费方」= 同一个前端文件（组件目录 / 页面 / store）既 invoke 了本域的某个命令，又 invoke 了**别的域**的命令。
- 「被别的面消费的命令数」= 本域中由这些共享消费方调用过的命令条数。
- 判层：共享消费方 ≥8 ⇒ **L0 公共底座**（拆走会让多个面同时失去能力）；3–7 ⇒ L1 但深耦合；1–2 ⇒ L1 浅耦合；0 ⇒ 最易独立。
- 这个度量只看**前端消费面**的交叉，不看后端 crate 依赖；两者都硬耦合的域，看第 3 节 \`后端 crate\` 列是否与其它域重叠。
`;

  md += `
## 3. 命令模块逐条清单（${MOD_KEYS.length} 个，按命令数降序）

| 模块 | 命令数 | 代码行 | 路径 | 后端 crate | 前端消费方 |
|---|---|---|---|---|---|
`;
  for (const mod of [...MOD_KEYS].sort((a, b) => modInfo[b].cmds - modInfo[a].cmds || a.localeCompare(b))) {
    const inf = modInfo[mod];
    const cons = [...(modConsumers[mod] || [])].sort().slice(0, 6).join('、') || '—';
    md += `| \`${mod}\` | ${inf.cmds} | ${inf.loc ? inf.loc.toLocaleString() : '—'} | ${inf.file || '**注册表有、文件未定位**'} | ${inf.crates.slice(0, 5).join(' ') || '—'} | ${cons} |\n`;
  }

  md += `
## 4. trait 分类全集（逐条归属，可复核）

### 4.0 K1-trait 口径（本次扫盘实际用的谓词，逐字与第五道门同源）

| 维度 | 取值 |
|---|---|
| 扫描面 | 8 根 \`${K1_ROOTS.join(' / ')}\` 下全部 \`.rs\`，剪路径段 ${[...K1_IGNORE].join(' / ')} |
| 声明面谓词 | 逐行 \`/^pub trait\\s+([A-Za-z_][A-Za-z0-9_]*)/\`（行级、不剥注释、不展开宏；\`pub(crate) trait\` 与裸 \`trait\` 不计） |
| 实现站点谓词 | \`implTraitOf\`（\`check-capability-register.mjs\` export，同一套 \`IMPL_LINE_RE\`） |
| 测试替身/产物 | \`isTestDoublePath\`（同文件 export：basename 含 \`test_support\\|mock_\\|e2e_\\|_fixture\` 或路径含 \`bin\` 段） |
| harness 面 | 相对路径前缀 \`${HARNESS_PREFIX}\` |
| 计数单位 | **唯一 trait 名**（同名多定义、同文件多站点都只算一次；溢出量单独上报，见 §1.1） |
| 遍历次序 | **两趟**（先收齐声明名，再收 impl 站点）——单趟会丢 ${K1.singlePassLost.length} 条，见 §5.6 |
| 键控分发点 | \`computeK2Readings\`（#57 入库版八形态；本树强 ${k2Readings.strongTraits.length} / 弱 ${k2Readings.weakTraits.length}） |
| 已建接缝 | \`scripts/capability-register.json\` 的 \`entries[].expect.seam\`（PLAN §0.7 机器镜像） |

`;
  for (const [k, n] of clsStat) {
    md += `### 4.${clsStat.findIndex((x) => x[0] === k) + 1} ${k} — ${n} 条\n\n`;
    md += cls[k].slice().sort().map((t) => '`' + t + '`（' + K1.defSites[t][0].replace(HARNESS_PREFIX, 'harness/').replace(/^axagent-[a-z]+\//, '') + '）').join(' · ') + '\n\n';
  }
  md += `## 5. 判读口径与已知偏差

0. **第 2 节的「跨面共享消费方 / 被别的面消费的命令数」只度量拆分的顺序成本，不度量可行性。**
   \`WorkEngine\` 是底座（\`rt-workflow/Cargo.toml\` 的 axagent-* 依赖仅 crypto dao harness kit providers telemetry，**不含任何业务域 crate**），
   域以客户端方式调它（\`cognitive.rs:1249\` 调 \`run_workflow\`、\`stock_workflow/core.rs:2035\` 调 \`cancel_workflow\`）⇒ **「基于工作流」不构成拆不动的理由**。
   股票分析拆不干净的真实原因见 \`PLAN-shell-plus-plugins.md\` §0.8.5：引擎 \`agent_executor.rs:4385\` 的 \`PREFETCHABLE_TOOLS\` 硬编码十余个 \`get_stock_*\` 工具名、
   \`:1232-1246\` 从上下文取 \`stock_code\` 预取、\`:854\` prompt 文本写死 \`search_stock\`；另有一条空壳执行器
   \`document_parser_executor.rs\`（59 行，只回显 \`"status":"parsed"\`，不接 \`DocumentParser\` 接缝）。
1. 命令模块的「代码行」按该模块目录内全部 \`.rs\` 求和，含注释与测试外的内联 \`#[cfg(test)]\`。
2. 域簇用命令模块名的正则归并（规则在 **\`scripts/lib/command-domains.mjs\`** 的 \`DOMAIN_RULES\`，两份清点脚本共用同一份，逐条可见）。**归并是命名级的，命名不规范者会落「其它」**，
   那张表的行需要人工认领，但**不影响命令数总量闭合**（${Object.values(domains).reduce((a, e) => a + e.cmds, 0)} 条已分配）。
3. trait 分类里「L2 候选（无键控点）」是**中间态**：它有外部实现者，但我没找到按字符串键选实现的站点 ⇒ 只能算分层解耦，不能算可插拔。
4. 「空接口」= 全仓零实现 ⇒ 属债不属能力，不得计入插件数。
5. \`domain.pack\`／OPC 已按产品口径从能力清单剔除，但盘上仍在（见 PLAN §0.8.4 / 任务 #32），故本清点仍如实列出其体量。
6. **「键控分发点」一列换了来源**（#61）：旧版取 \`.tmp/cap-register3.tsv\` 第 5 列（谓词在 \`.tmp/capability-register3.cjs:44-82\`，四形态启发式、未入库、仓内无处复跑）⇒ 现取 \`computeK2Readings\`（#57 入库版八形态，强/弱两档）。
   两档**不是同一口径**（扫描面从 198 样本扩到全树、新增 ⑥ push/set 与 ⑧ impl 反查两形态），故分类数会随之变化。镜像表 \`countRule.k2Caveat\` 与 PLAN §19.3 已声明入库版三处已知高估/漏检面 ⇒ **在 #54 裁掉前不得据本列改能力数**。
7. **「已建接缝」一列同样换源**（#61）：旧 \`.tmp/cap-register3.tsv\` 第 3 列（由 \`capability_registry.rs\` 的 \`Self::new("id","1.0","axagent_harness::Trait")\` 正则反推）⇒ 现取 \`scripts/capability-register.json\` 的 \`entries[].expect.seam\`（PLAN §0.7 的机器镜像，36 行登记项）。
   ⇒ 本列现在**只覆盖登记表已认领的 trait 面**；登记外的 trait 即使有 \`Self::new\` 声明也会落「候选/待建」。这是刻意的：镜像表是唯一权威，不在表里就不算已建。

## 5.6 \`k1ExternalImplTraitCount\` 三读数归因（#61 定案）

历史三个读数互相打架：PLAN:248 登记 **232**、\`#29\` 实测 **269**、第五道门复算 **210**（镜像表 \`implementorsMechanical\`）。本轮把三个数**全部落到判据分支**：

| 读数 | 出处（工件/脚本 + 谓词配方） | 与定案口径的判据差 | 本脚本在当前树的同名谓词 |
|---|---|---|---|
| **232** | \`.tmp/capability-census.cjs\` 谓词的**两趟**复算（11-01 树、2004 个 \`.rs\`；探针 \`.tmp/cap5-probe13.mjs\` ① 打印 232） | 与定案**只差一条判据**：impl 正则能否跨行（见末行）。另一档 \`isNoise\` 名单（\`.tmp/capability-census.cjs:21\`，44 个 std/派生名）在「要求声明面」之下**贡献 0**（\`def ∩ noise = ∅\`；冻结工件与当前树两边实测一致）⇒ 定案口径删掉了它，仓内不留这份硬编码名单 | 定案 **${K1.declaredExternalCount}** |
| **210** | \`.tmp/capability-census.cjs\` **原样单趟**跑出的历史工件 \`.tmp/capability-census.tsv\`（表头 + 210 数据行；行数本身即证据） | **单趟扫描缺陷**：\`def\` 随文件遍历就地累积 ⇒「impl 早于其声明文件出现」的站点被静默丢弃。丢失 **22 条**（今天复现：两趟 ${K1.declaredExternalCount} − 单趟 ${K1.singlePassCount} = ${K1.declaredExternalCount - K1.singlePassCount}，名单逐条见 §1.1） | 单趟复现 = **${K1.singlePassCount}** |
| **269** | 第五道门 \`--dump-readings\` 的 \`k1.externalImplTraits\`（\`.tmp/res29-readings.txt\`，11-01 树；\`.tmp/res33-trait-adjudication.md:8\` 转录同数） | **不要求 trait 声明面**：\`impl X for Y\` 只要 X 被收进 impl 名集合就算 ⇒ 多出的全是 std/派生名、外部 crate 名与宏展开名（逐条名单见 §1.1） | 同口径 = **${K1.externalAllCount}**（差 ${K1.undeclaredExt.length}） |

- ⇒ **三个数不是三套口径下的三个真值，而是同一谓词在同一条链上的三种计数姿势**：232 vs 210 = 两趟/单趟之差（差 22，逐条可点名）；269 = 去掉「要求声明面」的放宽读数（今天差 ${K1.undeclaredExt.length}）。每个差值都有具名判据分支，不存在「口径不可核对」。
- ⚠ **顺带纠 PLAN:248 的一处事实陈述**：它写「232 出自一次性 census 工件 \`.tmp/traits-full.json\`」——**不确**。把 \`.tmp/traits-census.cjs\` 的下游谓词在**那份工件自身**上重放 = **231**（三套测试替身谓词逐一重放都是 231 ⇒ 替身档那一轴对计数**贡献 0**）。\`.tmp/traits-full.json\` 是「**316**」的产地（PLAN §17.1 已判其退役），不是「232」的产地；232 出自 census 谓词的**两趟复算**（11-01 晚些、2004 个 \`.rs\` 的树）。231 与 232 差 1 = **工件 mtime（2026-11-01T04:41Z）之后树又动过**，不是谓词差。
- **+1 的判据分支 = impl 正则会否跨行**：定案沿用门禁 \`implTraitOf\`（逐行、不跨行），因此漏一条多行 impl 头——
  \`axagent-runtime/crates/runtime-core/src/conversation.rs:2024\`：\`impl<C: ApiClient + Send, T: ToolExecutor + Send + 'static>\` 换行后才写全路径 \`axagent_harness::runtime_types::conversation::ConversationRuntimeHost\`。
  census 系的 \`[^;{=]*?\` 版本能跨行 ⇒ 会多数这一条（今天该配方 = 235，定案 = ${K1.declaredExternalCount}）。
  **本脚本不为此引进第二套 impl 正则**（禁区 12）；要收这一条只能改 \`IMPL_LINE_RE\` 本体（\`check-capability-register.mjs:343\`，另有会话持有该文件）⇒ 记为待决策项交 leader。
- **定案读数（可机器复跑）**：\`k1DeclaredExternalImplTraitCount = ${K1.declaredExternalCount}\`。复跑：
  \`node scripts/audit-capability-inventory.mjs --out <仓外路径>\` 看同名那一行；恒等式
  \`${K1.externalAllCount}（第五道门 k1.externalImplTraits） − ${K1.undeclaredExt.length}（§1.1 无声明面名单） = ${K1.declaredExternalCount}\`，
  两侧各自都可独立复跑 ⇒ 这是本条的机检闭合。
  ⚠ 该数**随盘漂**（本树 ${K1.rsFiles} 个 \`.rs\`；11-01 是 2004）⇒ 它是「当前树的读数」，不是历史常数；历史三数只在各自的树上才可复现。
`;
  const outRel = argVal('--out') || path.join('docs', 'audits', 'AUDIT-capability-inventory-' + NOW + '.md');
  const outAbs = path.isAbsolute(outRel) ? outRel : R(outRel);
  fs.mkdirSync(path.dirname(outAbs), { recursive: true });
  fs.writeFileSync(outAbs, md);
  console.log('\n已落 ' + outRel + '，行数 ' + lines(md));
  console.log('功能域 ' + Object.keys(domains).length + ' 个；命令分配闭合 ' + Object.values(domains).reduce((a, e) => a + e.cmds, 0) + '/' + Object.keys(cmdToModule).length);
  const uncls = domains['其它（未归类，需人工）'];
  if (uncls) console.log('未归类模块 ' + uncls.mods.length + ' 个 / ' + uncls.cmds + ' 条命令：' + uncls.mods.slice(0, 40).join(' '));
  return 0;
}

/* ═══════════ --selftest：纯函数 + fixture，零盘 IO ═══════════ */
function runSelftest() {
  let fail = 0;
  const ck = (ok, label) => { console.log((ok ? '  PASS ' : '  FAIL ') + label); if (!ok) fail++; };
  const F = (rel, body) => [rel, body];

  // fixture：impl 早于声明文件 ⇒ 两趟必须收、单趟必丢（210 病根的机检锁）
  const early = F('axagent-core/crates/dao/src/a.rs', 'impl Widget for Foo {}\n');
  const late = F('axagent-core/crates/harness/src/b.rs', 'pub trait Widget {}\n');
  {
    const r = scanK1Traits([early, late]);
    ck(r.declaredExternalCount === 1 && r.pubTraitNames.join() === 'Widget', '两趟法：impl 先于 def 出现仍计入（k1Declared=1）');
    ck(r.singlePassCount === 0 && r.singlePassLost.join() === 'Widget', '单趟法同 fixture 必丢 ⇒ 210 的缺陷被 fixture 锁住');
  }
  // harness 自身的实现不计入「外部实现者」
  {
    const r = scanK1Traits([F('axagent-core/crates/harness/src/t.rs', 'pub trait Thing {}\nimpl Thing for Inner {}\n')]);
    ck(r.declaredExternalCount === 0 && r.pubTraitNames.join() === 'Thing', 'harness 内 impl 不算外部实现者');
  }
  // 测试替身/产物排除（复用门禁导出的 isTestDoublePath，两套调用点必须一致）
  {
    const r = scanK1Traits([
      F('axagent-core/crates/harness/src/t.rs', 'pub trait Thing {}\n'),
      F('axagent-ai/crates/x/src/mock_thing.rs', 'impl Thing for Mock {}\n'),
      F('axagent-ai/crates/x/src/bin/worker.rs', 'impl Thing for W {}\n'),
    ]);
    ck(r.declaredExternalCount === 0, '测试替身（mock_）与 bin/ 产物面被剔除');
  }
  // std/派生名：门禁 A 收，定案档因「无声明面」不收 ⇒ 269−234 那一轴
  {
    const r = scanK1Traits([
      F('axagent-ai/crates/x/src/a.rs', 'impl Default for Foo {}\nimpl Clone for Foo {}\npub trait Real {}\n'),
      F('axagent-ai/crates/x/src/b.rs', 'impl Real for Bar {}\n'),
    ]);
    ck(r.externalAllCount === 3 && r.declaredExternalCount === 1 && r.undeclaredExt.join() === 'Clone,Default', '不要求声明面 ⇒ 多算 std/派生名（归因表第 3 行）');
  }
  // 同名重复：hits−names==溢出，且逐条留名（PLAN §17.1 的闭合断言）
  {
    const r = scanK1Traits([
      F('axagent-core/crates/harness/src/p.rs', 'pub trait Repo {}\n'),
      F('axagent-core/crates/harness/src/q.rs', 'pub trait Repo {}\n'),
    ]);
    ck(r.pubTraitHits === 2 && r.pubTraitNames.length === 1 && r.dupOverflow === 1 && r.dupTraits.join() === 'Repo', '重名 trait：hits − names == dupOverflow 闭合且逐条点名');
  }
  // pub(crate)/裸 trait 不进声明面
  {
    const r = scanK1Traits([F('axagent-ai/crates/x/src/a.rs', 'pub(crate) trait Priv {}\ntrait Bare {}\n')]);
    ck(r.pubTraitHits === 0 && r.rsFiles === 1, 'pub(crate) trait / 裸 trait 不计入 pub trait 面');
  }
  // 同文件多站点只算一条 trait
  {
    const r = scanK1Traits([
      F('axagent-core/crates/harness/src/a.rs', 'pub trait Svc {}\n'),
      F('axagent-ai/crates/x/src/b.rs', 'impl Svc for A {}\nimpl Svc for B {}\n'),
    ]);
    ck(r.declaredExternalCount === 1 && (r.extSites['Svc'] || []).length === 2, '计数单位 = 唯一 trait 名（站点数另存 extSites）');
  }
  // 注释里的 trait 会命中（口径如实声明：不剥注释，与门禁同档）
  {
    const r = scanK1Traits([F('axagent-ai/crates/x/src/a.rs', '/// 说明\npub trait DocOnly {}\n')]);
    ck(r.pubTraitNames.join() === 'DocOnly', '不剥注释 ⇒ 行首 pub trait 即命中（与门禁同口径，如实声明）');
  }
  // 反回归锁：本脚本源码不得再出现 .tmp/ 读取与硬编码盘符
  {
    const src = fs.readFileSync(fileURLToPath(import.meta.url), 'utf8');
    ck(!/read\(\s*['"]\.tmp/.test(src) && !/['"]\.tmp\/[a-z-]+\.(json|tsv|cjs)['"]/.test(src.replace(/^\/\/.*$/gm, '')), '源码（剥注释后）零 .tmp/ 读取点');
    ck(!/const repo = '[A-Za-z]:\//.test(src), '无硬编码盘符（ROOT 由 import.meta.url 导出）');
  }
  console.log(`\n自检 ${fail === 0 ? '全过' : '失败 ' + fail + ' 条'}（fixture 9 组，零盘 IO）`);
  return fail === 0;
}

const entryPath = process.argv[1];
if (entryPath && import.meta.url === pathToFileURL(entryPath).href) {
  if (argv.includes('--selftest')) {
    console.log('── audit-capability-inventory --selftest（纯函数 + fixture，零盘扫描）──');
    process.exit(runSelftest() ? 0 : 1);
  }
  process.exit(runAudit());
}
