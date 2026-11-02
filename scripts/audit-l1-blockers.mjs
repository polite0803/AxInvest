#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// L1 功能包（股票 / OPC）拆分阻塞面清点 —— PLAN-shell-plus-plugins.md §0.10 的数字来源。
// 用法：node scripts/audit-l1-blockers.mjs
// 口径：
//   反向阻塞 = 「非域自有文件」import 域 crate 的引用点（这些要逐条改写）；
//   域自有文件（commands/stock_*、commands/opc_* 等）随插件整体迁出，不计入阻塞。
//   H 桶 = `scripts/l1-host-owned-allowlist.json` 里**逐行点名**豁免的宿主持有引用点：
//   它们不参与「降到 0」的终态判据（终态 = 待改写归零 + H 桶等于白名单），
//   但每一行都要能被 anchorText 复核 ⇒ 白名单是**可审的**，扩正则不可审（PLAN §54 R5）。
//   命令数同时给两套口径：**插件边界口径**（本脚本 ownRe/cmdRe）与**审计 19 域口径**（lib/command-domains.mjs），
//   两者不相等是规则本身不同（例：`demand_*` 在审计里归股票域、在插件边界里归 OPC），不是数据漂移。
// 用法：
//   node scripts/audit-l1-blockers.mjs                    生成审计快照
//   node scripts/audit-l1-blockers.mjs --check-allowlist  只校验白名单（锚点腐烂 / 重复 / 文件缺失 ⇒ EXIT=1）
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { REG_PATH, parseRegistry, domainOf } from './lib/command-domains.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// ⚠ 用**本地日**而不是 `toISOString()`（PLAN §54 R8）：UTC 日在本地 00:00–08:00 会退到昨天，
// 于是「今天的重跑」覆盖昨天的快照，git 里表现为 modify 而非 add，与 AGENTS「审计报告 = 时点快照」的命名语义冲突。
// 同日二次跑追加 -HHMM，两份快照同时留档。
const LOCAL = new Intl.DateTimeFormat('sv-SE', { dateStyle: 'short', timeStyle: 'short' }).format(new Date()).split(' ');
const NOW_BASE = LOCAL[0].slice(0, 10);
const NOW_HHMM = (LOCAL[1] || '').replace(':', '');
const ALLOWLIST_PATH = 'scripts/l1-host-owned-allowlist.json';
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
// 四个桶：foreign（主壳待改写）/ hostOwned（白名单点名的宿主持有，H 桶）/ cross（功能包之间的口，要走 call_seam）/ owned（同域内部，随迁走）
const ownedByAny = (f) => Object.entries(DOMAINS).find(([, d]) => d.ownedRe.test(f))?.[0] || null;
const allowPath = path.join(repo, ALLOWLIST_PATH);
const allowDoc = fs.existsSync(allowPath) ? JSON.parse(fs.readFileSync(allowPath, 'utf8')) : { entries: [] };
const allowByFile = new Map();
for (const e of allowDoc.entries || []) {
  if (!allowByFile.has(e.file)) allowByFile.set(e.file, []);
  allowByFile.get(e.file).push({ ...e, matched: 0 });
}
const allowMatch = (f, txt) => (allowByFile.get(f) || []).find((e) => e.anchorText && txt.includes(e.anchorText)) || null;
const edges = {};
for (const k of Object.keys(DOMAINS)) edges[k] = { owned: [], foreign: [], cross: [], hostOwned: [] };
for (const f of files) {
  if (!f.endsWith('.rs')) continue;
  const fileOwner = ownedByAny(f);
  for (const [dk, d] of Object.entries(DOMAINS)) {
    const crateHit = d.crates.some((c) => f.includes('crates/' + c.split('axagent-')[1] + '/'));
    if (crateHit) continue; // 域自身 crate 内部引用不算反向阻塞
    const src = fs.readFileSync(path.join(repo, f), 'utf8').split('\n');
    const hits = [];
    const hHits = [];
    src.forEach((l, i) => {
      const t = l.trim();
      if (t.startsWith('//') || t.startsWith('///')) return;
      if (!d.useRe.test(l)) return;
      const ent = allowMatch(f, l);
      if (ent) {
        ent.matched++;
        hHits.push({ line: i + 1, reason: ent.reason || '（白名单未写 reason）' });
      } else hits.push(i + 1);
    });
    if (hHits.length) edges[dk].hostOwned.push({ file: f, n: hHits.length, hits: hHits });
    if (!hits.length) continue;
    const rec = { file: f, n: hits.length, lines: hits.slice(0, 8) };
    if (fileOwner === dk) edges[dk].owned.push(rec);
    else if (fileOwner) edges[dk].cross.push(rec);
    else edges[dk].foreign.push(rec);
  }
}

// 白名单腐烂判据三条，任一即「白名单在说谎」：
//   ① 锚点在文件原文里找不到（那一行被改写或删除）；② 同 (file, anchorText) 重复；
//   ③ 锚点还在、但那一行今天不再计入 H 桶（文件被划成域自有 / 行成了注释 / 不再 import 域 crate）= 豁免空转。
const allowFindings = [];
const seenAnchor = new Set();
for (const e of allowDoc.entries || []) {
  const dupKey = JSON.stringify([e.file, e.anchorText]);
  if (seenAnchor.has(dupKey)) allowFindings.push({ kind: '重复条目', file: e.file, anchor: e.anchorText, why: '同一 (file, anchorText) 出现两次，计数会被重复豁免' });
  seenAnchor.add(dupKey);
  const fp = path.join(repo, e.file || '');
  if (!fs.existsSync(fp)) {
    allowFindings.push({ kind: '文件缺失', file: e.file || '(空)', anchor: e.anchorText || '', why: '白名单指向的文件不在盘上' });
    continue;
  }
  if (!e.anchorText || !fs.readFileSync(fp, 'utf8').includes(e.anchorText)) {
    allowFindings.push({ kind: '锚点腐烂', file: e.file, anchor: e.anchorText || '(空)', why: 'anchorText 在该文件原文里已找不到' });
  } else if (e.matched === 0) {
    allowFindings.push({ kind: '豁免空转', file: e.file, anchor: e.anchorText, why: '锚点在，但那一行今天不再计入 H 桶（文件已域自有 / 行成注释 / 不再 import 域 crate）' });
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

const sumH = (dk) => edges[dk].hostOwned.reduce((x, r) => x + r.n, 0);
if (process.argv.includes('--check-allowlist')) {
  console.log('H 桶白名单逐条锚点复扫：登记 ' + (allowDoc.entries || []).length + ' 条 / 今天实际豁免 ' +
    (sumH('STOCK') + sumH('OPC')) + ' 处（STOCK ' + sumH('STOCK') + ' · OPC ' + sumH('OPC') + '）');
  for (const f of allowFindings) console.log('FAIL [' + f.kind + '] ' + f.file + ' | ' + f.anchor + ' —— ' + f.why);
  if (allowFindings.length) {
    console.log('FAIL: 白名单腐烂 ' + allowFindings.length + ' 条 ⇒ 逐条改写锚点或删除条目，不许留假话');
    process.exit(1);
  }
  console.log('PASS: 白名单每条锚点都能在盘上原文里找到，且都在计入 H 桶');
  process.exit(0);
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

// 同日二次跑不覆盖既有快照（R8）：已存在当日文件就追加 -HHMM。
let SNAP = NOW_BASE;
if (fs.existsSync(path.join(repo, 'docs/audits/AUDIT-l1-blockers-' + SNAP + '.md'))) SNAP = NOW_BASE + '-' + NOW_HHMM;
const sum = (a) => a.reduce((x, r) => x + r.n, 0);
const L = [];
L.push('# L1 功能包拆分阻塞面实测（' + SNAP + '）');
L.push('');
L.push('本文件由 `node scripts/audit-l1-blockers.mjs` 生成，是 PLAN §0.10 的数字来源。**改判据请改脚本，不要手抄。**');
L.push('');
L.push('| 度量 | 股票 STOCK | OPC |');
L.push('|---|---|---|');
L.push('| 命令条数 / 命令模块数（**插件边界口径**，本脚本 `cmdRe`） | ' + cmdStats.STOCK.total + ' / ' + cmdStats.STOCK.modules + ' | ' + cmdStats.OPC.total + ' / ' + cmdStats.OPC.modules + ' |');
L.push('| 命令条数 / 模块数（**审计 19 域口径**，`lib/command-domains.mjs`） | ' + auditCmd.STOCK.total + ' / ' + auditCmd.STOCK.modules + '（域「股票分析与管线」） | ' + auditCmd.OPC.total + ' / ' + auditCmd.OPC.modules + '（域「OPC 公司运行时」） |');
const unionFiles = (a, b) => new Set([...a.map((r) => r.file), ...b.map((r) => r.file)]).size;
L.push('| **Rust 反向阻塞（待改写，H 桶除外）** | **' + sum(edges.STOCK.foreign) + ' 处 / ' + edges.STOCK.foreign.length + ' 文件** | **' + sum(edges.OPC.foreign) + ' 处 / ' + edges.OPC.foreign.length + ' 文件** |');
L.push('| Rust H 桶（`l1-host-owned-allowlist.json` 逐行点名的宿主持有，不参与「归零」） | ' + sumH('STOCK') + ' 处 / ' + edges.STOCK.hostOwned.length + ' 文件 | ' + sumH('OPC') + ' 处 / ' + edges.OPC.hostOwned.length + ' 文件 |');
L.push('| 同上**合计**（= 待改写 + H 桶，与旧 152 口径可比） | ' + (sum(edges.STOCK.foreign) + sumH('STOCK')) + ' 处 / ' + unionFiles(edges.STOCK.foreign, edges.STOCK.hostOwned) + ' 文件 | ' + (sum(edges.OPC.foreign) + sumH('OPC')) + ' 处 / ' + unionFiles(edges.OPC.foreign, edges.OPC.hostOwned) + ' 文件 |');
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
  L.push('### ' + label + '：H 桶 ' + sumH(dk) + ' 处（白名单逐行点名豁免；每行都要能拿 anchorText 在盘上复核）');
  L.push('');
  for (const r of edges[dk].hostOwned.sort((a, b) => b.n - a.n)) {
    for (const h of r.hits) L.push('- `' + r.file + ':' + h.line + '` —— ' + h.reason);
  }
  if (!edges[dk].hostOwned.length) L.push('- （空：本域尚无点名豁免）');
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
L.push('7. **H 桶 = 白名单逐行点名豁免，不是正则扩面**（PLAN §54 R5）：`node scripts/audit-l1-blockers.mjs --check-allowlist` 会拿每条 `anchorText` 回盘上原文复核，');
L.push('   找不到 / 重复 / 计入不了 H 桶（锚点在但该行今天不算反向阻塞）三类一律 EXIT=1。');
L.push('   ⇒ 股票迁移的终态判据是「**待改写归零 + H 桶等于白名单**」，不是「152 打到 0」；合计行与旧 152 口径可比，不许拿它当进度。');
L.push('');
L.push('## 白名单复扫（' + (allowDoc.entries || []).length + ' 条登记 / ' + (allowFindings.length ? '腐烂 ' + allowFindings.length + ' 条' : '逐条锚点复核通过') + '）');
L.push('');
for (const f of allowFindings) L.push('- ⚠ [' + f.kind + '] `' + f.file + '` | anchor `' + f.anchor + '` —— ' + f.why);
fs.writeFileSync(path.join(repo, 'docs/audits/AUDIT-l1-blockers-' + SNAP + '.md'), L.join('\n') + '\n');
console.log(L.slice(0, 17).join('\n'));
console.log('\n已落 docs/audits/AUDIT-l1-blockers-' + SNAP + '.md（' + L.length + ' 行）');
