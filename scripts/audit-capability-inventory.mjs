#!/usr/bin/env node
// scripts/audit-capability-inventory.mjs
//
// 功能点全集清点 —— 一次扫全，产出 docs/audits/AUDIT-capability-inventory-<日期>.md。
// 权威骨架 = build.rs 生成的 Tauri 命令注册表；交叉源 = 前端 invoke 消费方、
// commands/** 的行与 axagent_* 引用、全仓 pub trait 及实现者分布、壳层 Arc<dyn T> 持有面。
// 层级判据见 docs/plans/PLAN-shell-plus-plugins.md §0.9.4。
import fs from 'node:fs';
import path from 'node:path';
import { REG_PATH, parseRegistry, domainOf } from './lib/command-domains.mjs';
const repo = 'D:/OneManager/AxInvest';
const R = (rel) => path.join(repo, rel);
const exists = (p) => fs.existsSync(p);
const read = (p) => (exists(p) ? fs.readFileSync(p, 'utf8') : '');
const lines = (s) => s.split('\n').length;
function walk(d, out, ext) {
  let e; try { e = fs.readdirSync(d, { withFileTypes: true }); } catch (x) { return out; }
  for (const f of e) {
    const p = path.join(d, f.name);
    if (f.isDirectory()) { if (['target', 'node_modules', 'gen', '__tests__', 'tests'].includes(f.name)) continue; walk(p, out, ext); }
    else if (ext.some((x) => f.name.endsWith(x))) out.push(p);
  }
  return out;
}
const relOf = (f) => f.slice(repo.length + 1).split(path.sep).join('/');

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
    if (p.endsWith('mod.rs')) {
      const dir = path.dirname(p);
      files = walk(dir, [], ['.rs']);
    } else files = [R(p)];
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
const cen = JSON.parse(read('.tmp/traits-full.json'));
const H = 'axagent-core/crates/harness/';
const reg3 = read('.tmp/cap-register3.tsv').trim().split('\n').slice(1).map((l) => l.split('\t'));
const seamOf = {}, dispOf = {};
reg3.forEach((c) => { if (c[2] !== '-') seamOf[c[0]] = c[2]; if (c[4] !== '-') dispOf[c[0]] = c[4]; });
const shellTxt = read('src-tauri/src/app_state.rs') + read('src-tauri/src/init/state.rs');
const shellDyn = new Set([...shellTxt.matchAll(/dyn\s+(?:[a-z_0-9]+::)*([A-Z][A-Za-z0-9_]*)/g)].map((m) => m[1]));
const traits = Object.keys(cen.def);
const cls = {};
for (const t of traits) {
  const inH = cen.def[t].some((d) => d.startsWith(H));
  const ext = (cen.implFiles[t] || []).filter((p) => !p.startsWith(H) && !/test_support|mock_|\/tests\/|_fixture|\/bin\/e2e_/.test(p));
  let c;
  if (seamOf[t] && seamOf[t] !== '-') c = 'L2-已建接缝';
  else if (ext.length >= 1 && dispOf[t]) c = 'L2-待建接缝';
  else if (ext.length >= 1) c = 'L2-候选(有外部实现者无键控点)';
  else if (inH && ext.length === 0 && shellDyn.has(t)) c = 'L3-策略位(壳层全局持有)';
  else if (inH && ext.length === 0 && (cen.implFiles[t] || []).length >= 1) c = 'L3-策略位(仅内置实现)';
  else if ((cen.implFiles[t] || []).length === 0) c = '空接口(全仓无实现)';
  else c = 'L0-内核/实现层接口';
  (cls[c] = cls[c] || []).push(t);
}
const clsStat = Object.entries(cls).map(([k, v]) => [k, v.length]).sort((a, b) => b[1] - a[1]);
console.log('\npub trait 总数 ' + traits.length + '，分类：');
clsStat.forEach(([k, n]) => console.log('  ' + String(n).padStart(4) + '  ' + k));

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
const NOW = '2026-11-01';
let md = `# AUDIT-capability-inventory-${NOW} — AxInvest 功能点全集清点（一次扫全）

> 生成方式：\`node .tmp/full-inventory.cjs\`（本文件由脚本直接落盘，非手抄）。
> 权威骨架 = \`src-tauri/src/register_commands.rs\`（build.rs 生成的命令注册表）；
> 交叉源 = 前端 \`src/**\` 的 invoke 消费方、\`commands/**\` 的代码行与 \`axagent_*\` 引用、全仓 \`pub trait\` 与其实现者分布、\`app_state.rs\`/\`init/state.rs\` 的 \`Arc<dyn T>\` 全局持有面。
> 口径唯一：层级判据见 \`PLAN-shell-plus-plugins.md\` §0.9.4。

## 1. 总量

| 面 | 数量 |
|---|---|
| Tauri 命令（注册表逐条） | **${Object.keys(cmdToModule).length}** |
| 命令模块 | **${MOD_KEYS.length}** |
| 功能域（下表） | **${Object.keys(domains).length}** |
| \`pub trait\`（全仓） | **${traits.length}**（harness ${traits.filter((t) => cen.def[t].some((d) => d.startsWith(H))).length}） |
| L2 提供方接缝：已建 | ${(cls['L2-已建接缝'] || []).length} |
| L2 提供方接缝：待建 | ${(cls['L2-待建接缝'] || []).length} |
| L2 候选（有外部实现者，无键控分发点） | ${(cls['L2-候选(有外部实现者无键控点)'] || []).length} |
| L3 策略位（0 外部实现者，壳层全局持有） | ${(cls['L3-策略位(壳层全局持有)'] || []).length} |
| L3 策略位（仅内置实现） | ${(cls['L3-策略位(仅内置实现)'] || []).length} |
| 空接口（全仓无实现） | ${(cls['空接口(全仓无实现)'] || []).length} |
| L0 内核/实现层接口 | ${(cls['L0-内核/实现层接口'] || []).length} |

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

`;
for (const [k, n] of clsStat) {
  md += `### 4.${clsStat.findIndex((x) => x[0] === k) + 1} ${k} — ${n} 条\n\n`;
  md += cls[k].slice().sort().map((t) => '`' + t + '`（' + cen.def[t][0].replace(H, 'harness/').replace(/^axagent-[a-z]+\//, '') + '）').join(' · ') + '\n\n';
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
`;
fs.writeFileSync(R('docs/audits/AUDIT-capability-inventory-' + NOW + '.md'), md);
console.log('\n已落 docs/audits/AUDIT-capability-inventory-' + NOW + '.md，行数 ' + lines(md));
console.log('功能域 ' + Object.keys(domains).length + ' 个；命令分配闭合 ' + Object.values(domains).reduce((a, e) => a + e.cmds, 0) + '/' + Object.keys(cmdToModule).length);
const uncls = domains['其它（未归类，需人工）'];
if (uncls) console.log('未归类模块 ' + uncls.mods.length + ' 个 / ' + uncls.cmds + ' 条命令：' + uncls.mods.slice(0, 40).join(' '));
