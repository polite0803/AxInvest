#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// L1 功能包（股票 / OPC）拆分阻塞面清点 —— PLAN-shell-plus-plugins.md §0.10 的数字来源。
// 用法：node scripts/audit-l1-blockers.mjs
// 口径：
//   反向阻塞 = 「非域自有文件」import 域 crate 的引用点（这些要逐条改写）；
//   域自有文件（commands/stock_*、commands/opc_* 等）随插件整体迁出，不计入阻塞。
//   命令数同时给两套口径：**插件边界口径**（本脚本 ownRe/cmdRe）与**审计 19 域口径**（lib/command-domains.mjs），
//   两者不相等是规则本身不同（例：`demand_*` 在审计里归股票域、在插件边界里归 OPC），不是数据漂移。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { REG_PATH, parseRegistry, domainOf } from './lib/command-domains.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const NOW = new Date().toISOString().slice(0, 10);
const SKIP = new Set(['target', 'node_modules', 'output', '.git', '.tmp', 'dist', 'gen', 'build', 'coverage', '.workbuddy', 'examples']);

const files = [];
(function walk(d) {
  let es;
  try {
    es = fs.readdirSync(d, { withFileTypes: true });
  } catch {
    return;
  }
  for (const e of es) {
    if (SKIP.has(e.name)) continue;
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p);
    else if (/\.(rs|ts|tsx|toml)$/.test(e.name)) files.push(p.slice(repo.length + 1).split(path.sep).join('/'));
  }
})(repo);

// ── 域归属：域自有文件（迁出时整体搬走）─────────────────────
const DOMAINS = {
  STOCK: {
    crates: ['axagent-astock-data', 'axagent-analysis-engine', 'axagent-quant', 'axagent-market-sim'],
    useRe: /axagent_(astock_data|analysis_engine|quant|market_sim)::/,
    ownedRe: /^src-tauri\/src\/(commands|init|bin)\/(stock|market_|backtest|quant|pool_scan|universe_scan|time_travel|paper|trade|conditional_order|price_alert|analyst|recommendation_cron|mover_recall|limit_up|signal|reco|wf_des)/,
    feRe: /stock|quote|kline|technical|portfolio|trading|advis|invest|quant|market/i,
    cmdRe: /^(stock|quote|kline|quant|backtest|portfolio|market_|pool|universe|reco|signal|north|margin|dragon|limit_up|time_travel|advis|paper|trade|conditional_order|price_alert|analyst|fundamental|techn|wf_des)/,
  },
  OPC: {
    crates: ['axagent-company-runtime'],
    useRe: /axagent_company_runtime::/,
    ownedRe: /^src-tauri\/src\/(commands|init)\/(opc|capability_pack|agency|domain_pack|horizon|dojo|demand_discovery)/,
    feRe: /opc|capability-?pack|agency|horizon|domain-?pack|dojo/i,
    cmdRe: /^(opc|capability_pack|agency|domain|horizon|dojo|demand_)/,
  },
};

// ── 1) Rust：所有文件 import 域 crate 的行级命中 ────────────
// 三个桶：foreign（主壳要改写）/ cross（两个功能包之间的口，要走 call_seam）/ owned（同域内部，随迁走）
const ownedByAny = (f) => Object.entries(DOMAINS).find(([, d]) => d.ownedRe.test(f))?.[0] || null;
const edges = {};
for (const k of Object.keys(DOMAINS)) edges[k] = { owned: [], foreign: [], cross: [] };
for (const f of files) {
  if (!f.endsWith('.rs')) continue;
  const fileOwner = ownedByAny(f);
  for (const [dk, d] of Object.entries(DOMAINS)) {
    const crateHit = d.crates.some((c) => f.includes('crates/' + c.split('axagent-')[1] + '/'));
    if (crateHit) continue; // 域自身 crate 内部引用不算反向阻塞
    const src = fs.readFileSync(path.join(repo, f), 'utf8').split('\n');
    const hits = [];
    src.forEach((l, i) => {
      const t = l.trim();
      if (t.startsWith('//') || t.startsWith('///')) return;
      if (d.useRe.test(l)) hits.push(i + 1);
    });
    if (!hits.length) continue;
    const rec = { file: f, n: hits.length, lines: hits.slice(0, 8) };
    if (fileOwner === dk) edges[dk].owned.push(rec);
    else if (fileOwner) edges[dk].cross.push(rec);
    else edges[dk].foreign.push(rec);
  }
}

// ── 2) Cargo.toml 构建边 ────────────────────────────────────
const cargoEdges = { STOCK: [], OPC: [] };
for (const f of files.filter((x) => path.basename(x) === 'Cargo.toml')) {
  const own = f.match(/crates\/([a-z-]+)\/Cargo.toml$/);
  const txt = fs.readFileSync(path.join(repo, f), 'utf8');
  for (const [dk, d] of Object.entries(DOMAINS)) {
    for (const c of d.crates) {
      for (const m of txt.matchAll(new RegExp('^\\s*"?' + c + '"?\\s*=', 'gm'))) {
        const selfName = own ? 'axagent-' + own[1] : f;
        if (selfName === c) continue;
        cargoEdges[dk].push(f + '  → ' + c);
      }
    }
  }
}

// ── 3) 前端 import 域文件 ───────────────────────────────────
const fe = { STOCK: { owned: [], foreign: [] }, OPC: { owned: [], foreign: [] } };
for (const f of files.filter((x) => /^src\/.*\.(ts|tsx)$/.test(x))) {
  for (const [dk, d] of Object.entries(DOMAINS)) {
    const own = d.feRe.test(f);
    const src = fs.readFileSync(path.join(repo, f), 'utf8').split('\n');
    const hits = [];
    src.forEach((l, i) => {
      for (const m of l.matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)) if (d.feRe.test(m[1])) hits.push(i + 1);
    });
    if (!hits.length) continue;
    (own ? fe[dk].owned : fe[dk].foreign).push({ file: f, n: hits.length, lines: hits.slice(0, 8) });
  }
}

// ── 4) 命令面 / 表 / i18n ───────────────────────────────────
const reg = fs.readFileSync(path.join(repo, REG_PATH), 'utf8');
// 与 audit-capability-inventory.mjs **共用同一份解析与域归并规则**（scripts/lib/command-domains.mjs）
const regEntries = parseRegistry(reg);
const modNames = regEntries.map((e) => e.mod);
const cmdStats = {};
for (const [dk, d] of Object.entries(DOMAINS)) {
  const c = {};
  modNames.forEach((m) => {
    if (d.cmdRe.test(m)) c[m] = (c[m] || 0) + 1;
  });
  cmdStats[dk] = { total: Object.values(c).reduce((a, b) => a + b, 0), modules: Object.keys(c).length, top: Object.entries(c).sort((a, b) => b[1] - a[1]).slice(0, 8) };
}
// 审计 19 域口径（同一份 DOMAIN_RULES，另一套归属）—— 与上面的插件边界口径并列，避免读者以为只有一个数
const AUDIT_DOMAIN_OF = { STOCK: '股票分析与管线', OPC: 'OPC 公司运行时（口径已废，盘上未收口）' };
const auditCmd = {};
for (const [dk, name] of Object.entries(AUDIT_DOMAIN_OF)) {
  const c = {};
  modNames.forEach((m) => {
    if (domainOf(m) === name) c[m] = (c[m] || 0) + 1;
  });
  auditCmd[dk] = { total: Object.values(c).reduce((a, b) => a + b, 0), modules: Object.keys(c).length };
}
// 两套口径的差集：哪些模块被归到不同边
const diffMods = [];
for (const m of new Set(modNames)) {
  const mine = Object.entries(DOMAINS).find(([, d]) => d.cmdRe.test(m))?.[0];
  const ad = Object.entries(AUDIT_DOMAIN_OF).find(([, n]) => domainOf(m) === n)?.[0];
  if (mine !== ad && (mine || ad)) diffMods.push(m + '（边界口径 ' + (mine || '—') + ' / 审计口径 ' + (ad || '—') + '）');
}
const ent = {};
for (const [dk, d] of Object.entries(DOMAINS)) {
  ent[dk] = files.filter((f) => f.includes('crates/entities/src/') && d.feRe.test(path.basename(f))).map((f) => path.basename(f));
}
const zhPath = path.join(repo, 'src/i18n/locales/zh-CN.json');
const countKeys = (o, re, p) => {
  let n = 0;
  for (const [k, v] of Object.entries(o)) {
    const q = p ? p + '.' + k : k;
    if (v && typeof v === 'object') n += countKeys(v, re, q);
    else if (re.test(q)) n++;
  }
  return n;
};
const zh = fs.existsSync(zhPath) ? JSON.parse(fs.readFileSync(zhPath, 'utf8')) : {};
const i18n = {
  STOCK: countKeys(zh, /stock|quote|kline|portfolio|quant|indicator/i, ''),
  OPC: countKeys(zh, /opc|capability|agency|horizon|domainPack/i, ''),
  整段: Object.keys(zh).filter((k) => /stock|opc|invest|quant|trade/i.test(k)),
};
// 前端体量：域自有 .ts/.tsx 文件数与行数（PLAN §0.10.1 引用的 60 757 行等数字出自这里）
const feVolume = {};
for (const [dk, d] of Object.entries(DOMAINS)) {
  let f = 0, ln = 0;
  for (const x of files.filter((p) => /^src\/.*\.(ts|tsx)$/.test(p) && d.feRe.test(p))) {
    f++;
    ln += fs.readFileSync(path.join(repo, x), 'utf8').split('\n').length;
  }
  feVolume[dk] = { files: f, lines: ln };
}

const sum = (a) => a.reduce((x, r) => x + r.n, 0);
const L = [];
L.push('# L1 功能包拆分阻塞面实测（' + NOW + '）');
L.push('');
L.push('本文件由 `node scripts/audit-l1-blockers.mjs` 生成，是 PLAN §0.10 的数字来源。**改判据请改脚本，不要手抄。**');
L.push('');
L.push('| 度量 | 股票 STOCK | OPC |');
L.push('|---|---|---|');
L.push('| 命令条数 / 命令模块数（**插件边界口径**，本脚本 `cmdRe`） | ' + cmdStats.STOCK.total + ' / ' + cmdStats.STOCK.modules + ' | ' + cmdStats.OPC.total + ' / ' + cmdStats.OPC.modules + ' |');
L.push('| 命令条数 / 模块数（**审计 19 域口径**，`lib/command-domains.mjs`） | ' + auditCmd.STOCK.total + ' / ' + auditCmd.STOCK.modules + '（域「股票分析与管线」） | ' + auditCmd.OPC.total + ' / ' + auditCmd.OPC.modules + '（域「OPC 公司运行时」） |');
L.push('| **Rust 反向阻塞（主壳非域文件引用域 crate）** | **' + sum(edges.STOCK.foreign) + ' 处 / ' + edges.STOCK.foreign.length + ' 文件** | **' + sum(edges.OPC.foreign) + ' 处 / ' + edges.OPC.foreign.length + ' 文件** |');
L.push('| Rust 跨包口（一个功能包的文件调另一个功能包的 crate ⇒ 要走 `call_seam`） | ' + sum(edges.STOCK.cross) + ' 处 / ' + edges.STOCK.cross.length + ' 文件 | ' + sum(edges.OPC.cross) + ' 处 / ' + edges.OPC.cross.length + ' 文件 |');
L.push('| Rust 域自有文件内引用（随迁出搬走，不算阻塞） | ' + sum(edges.STOCK.owned) + ' 处 / ' + edges.STOCK.owned.length + ' 文件 | ' + sum(edges.OPC.owned) + ' 处 / ' + edges.OPC.owned.length + ' 文件 |');
L.push('| Cargo.toml 构建边 | ' + cargoEdges.STOCK.length + ' | ' + cargoEdges.OPC.length + ' |');
L.push('| **前端反向阻塞（非域 import 域文件）** | **' + sum(fe.STOCK.foreign) + ' 处 / ' + fe.STOCK.foreign.length + ' 文件** | **' + sum(fe.OPC.foreign) + ' 处 / ' + fe.OPC.foreign.length + ' 文件** |');
L.push('| entities 表文件 | ' + ent.STOCK.length + ' | ' + ent.OPC.length + ' |');
L.push('| zh-CN i18n key | ' + i18n.STOCK + ' | ' + i18n.OPC + ' |');
L.push('| 前端域自有文件 / 行数（迁出时的体量） | ' + feVolume.STOCK.files + ' 文件 / ' + feVolume.STOCK.lines + ' 行 | ' + feVolume.OPC.files + ' 文件 / ' + feVolume.OPC.lines + ' 行 |');
L.push('| 注册表命令总数 / 迁出后主壳剩余 | ' + modNames.length + ' 条 ⇒ 迁完剩 ' + (modNames.length - cmdStats.STOCK.total - cmdStats.OPC.total) + ' 条 | 同左 |');
L.push('');
for (const [dk, label] of [['STOCK', '股票'], ['OPC', 'OPC']]) {
  L.push('## ' + label + '：' + sum(edges[dk].foreign) + ' 处 Rust 反向阻塞逐条（每文件前列 8 个行号）');
  L.push('');
  L.push('| 文件 | 处数 | 行号 |');
  L.push('|---|---|---|');
  for (const r of edges[dk].foreign.sort((a, b) => b.n - a.n)) L.push('| `' + r.file + '` | ' + r.n + ' | ' + r.lines.join(', ') + ' |');
  L.push('');
  L.push('### ' + label + '：跨包口 ' + sum(edges[dk].cross) + ' 处（调用方本身也是待迁功能包的文件）');
  L.push('');
  for (const r of edges[dk].cross.sort((a, b) => b.n - a.n)) L.push('- `' + r.file + '` ' + r.n + ' 处 @' + r.lines.join(','));
  L.push('');
  L.push('### ' + label + '：前端反向阻塞 ' + sum(fe[dk].foreign) + ' 处');
  for (const r of fe[dk].foreign.sort((a, b) => b.n - a.n)) L.push('- `' + r.file + '` ' + r.n + ' 处 @' + r.lines.join(','));
  L.push('');
  L.push('### ' + label + '：Cargo 构建边');
  for (const e of cargoEdges[dk]) L.push('- ' + e);
  L.push('');
  L.push('### ' + label + '：命令模块分布（前 8）');
  L.push(cmdStats[dk].top.map(([k, v]) => '`' + k + '`(' + v + ')').join(' · '));
  L.push('');
  L.push('### ' + label + '：entities 表文件');
  L.push(ent[dk].map((f) => '`' + f + '`').join(' · '));
  L.push('');
}
L.push('## 判读口径');
L.push('');
L.push('1. 「反向阻塞」只统计**非域自有文件**；`commands/stock_*` / `commands/opc_*` 等按「粒度 = 能力」与插件同迁，不计入。');
L.push('2. 行级匹配排除注释行与 `examples/`、`.workbuddy/`、`output/` 目录。');
L.push('3. 前端归属用文件名/路径正则（`feRe`），命名不规范者会误判，需人工认领异常行。');
L.push('4. **OPC 的直接 Rust 阻塞为 0 ≠ OPC 干净**：OPC 的工具面物理住在 `axagent-runtime/crates/tools/src/tools/opc.rs`（41 处），');
L.push('   它 import 的是**股票 crate**（`astock_data` / `analysis_engine`）而非 `company_runtime` ⇒ 这 41 处计在股票名下。');
L.push('   读法：迁 OPC 时要连带处理 `tools/opc.rs` 与 `tools/capability_pack_defs.rs`，迁股票时要处理 `tools/{finance,audit,global_state,astock_data}.rs`。');
L.push('5. 本脚本与 §0.10 的口径**以本脚本为准**（早前一次性脚本把 `commands/opc_*` 误并入股票桶，故报 210/70；正确读数是 ' + sum(edges.STOCK.foreign) + ' / ' + sum(edges.OPC.foreign) + '）。');
L.push('6. **两套命令口径并存且必须并列**：插件边界口径（本脚本）按「迁出后归哪个插件」划分；审计 19 域口径（`scripts/lib/command-domains.mjs`）按模块命名正则划分。');
L.push('   本次差集（' + diffMods.length + ' 个模块被两套规则归到不同边）：' + (diffMods.slice(0, 20).join('；') || '无') + (diffMods.length > 20 ? '…' : '') + '。');
L.push('   ⇒ 引用命令数时必须同时说明是哪套口径；PLAN §0.10 用**边界口径**，`AUDIT-capability-inventory` 用**审计口径**。');
fs.writeFileSync(path.join(repo, 'docs/audits/AUDIT-l1-blockers-' + NOW + '.md'), L.join('\n'));
console.log(L.slice(0, 14).join('\n'));
console.log('\n已落 docs/audits/AUDIT-l1-blockers-' + NOW + '.md（' + L.length + ' 行）');
