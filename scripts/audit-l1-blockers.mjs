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
//   node scripts/audit-l1-blockers.mjs                    打印阻塞面读数（默认**只读、不落盘**）
//   node scripts/audit-l1-blockers.mjs --emit-doc         归档快照到 docs/audits/，并由脚本自动在
//                                                        docs/INDEX.md「审计（AUDIT）」主节登记新行（active）
//                                                        + 把同族旧 active 行降为 superseded；登记失败 ⇒ 整体拒写
//   node scripts/audit-l1-blockers.mjs --fe-json          前端反向读数的机器可读输出 {stock,opc,perFile}
//                                                        （第五道门的复算器就是本模式 —— 权威 §3 判据自身，禁止第二套扫描器）
//   node scripts/audit-l1-blockers.mjs --fe-json --before <commit>
//                                                        同上报数，但扫描面与内容取自 <commit> 树（迁层/搬迁批净 0 证明用）
//   node scripts/audit-l1-blockers.mjs --check-allowlist  只校验白名单（锚点腐烂 / 重复 / 文件缺失 ⇒ EXIT=1）
//   node scripts/audit-l1-blockers.mjs --selftest         纯函数正负对照（scanFeDomain + registerAuditRow，零盘、先于任何 IO 短路）
// ⚠ 任务 #207：默认落盘是缺陷（每次跑都产出一份未登记快照，2026-11-05 单日 18 份 + 11-04 一份，共 19 份未进 INDEX）。
//   「产物落盘但索引没登记」与「产物没落盘但文档抄了数」都是缺陷 ⇒ 落盘改为显式 --emit-doc，且登记由脚本代劳。
import fs from 'node:fs';
import cp from 'node:child_process';
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

// ── 运行模式解析（任务 #207：默认只读出数、不落盘；--fe-json 出机器可读前端反向读数）──
const ARGS = process.argv.slice(2);
const SELFTEST = ARGS.includes('--selftest');
const FE_JSON = ARGS.includes('--fe-json');
const EMIT_DOC = ARGS.includes('--emit-doc');
const BEFORE_REF = ARGS.includes('--before') ? String(ARGS[ARGS.indexOf('--before') + 1] ?? '') : null;
const feDie = (m) => {
  console.error('FAIL: ' + m);
  process.exit(2);
};
const FE_FILE_RE = /^src\/.*\.(ts|tsx)$/;
const sum = (a) => a.reduce((x, r) => x + r.n, 0);

// ── 域归属：域自有文件（迁出时整体搬走）─────────────────────
// ⚠ 插件侧是**迁出目的地**，不是主壳：`plugin-stock` / `stock-core` 随插件产物整体发布，
//   它们引用域 crate 是预期方向（B9-1 起 worker 直接链接 analysis-engine 跑权威实现）。
//   宿主侧运行时 `axagent-plugins/crates/plugins` 仍是主壳，照旧计入 foreign。
// ⚠ `stock-core` **也是域 crate**（迁出目的地 = 插件内 foundation，不是宿主共享契约层）：
//   宿主文件直连它，等于宿主仍在编译期链接股票业务代码 ⇒ 与直连 analysis-engine 同等计入。
//   漏登记它会让「把代码从 analysis-engine 挪进 stock-core」凭空降数（2026-11-08 实测：
//   `bin/axagent-validate-backfill.rs` 3 处会静默消失）⇒ 改 `crates`/`useRe` 必须与
//   `check-capability-register.mjs` 的 DOMAINS 同批（那里逐字抄本表）。
const DOMAINS = {
  STOCK: {
    crates: ['axagent-astock-data', 'axagent-analysis-engine', 'axagent-quant', 'axagent-market-sim', 'axagent-stock-core'],
    useRe: /axagent_(astock_data|analysis_engine|quant|market_sim|stock_core)::/,
    ownedRe: /^src-tauri\/src\/(commands|init|bin)\/(stock|market_|backtest|quant|pool_scan|universe_scan|time_travel|paper|trade|conditional_order|price_alert|analyst|recommendation_cron|mover_recall|limit_up|signal|reco|wf_des)|^axagent-plugins\/crates\/(plugin-stock|stock-core)\//,
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

// ── 前端反向扫描（§3 权威判据，抽成纯函数：注入文件清单 + 内容读取器）────────
// 判定档 / --fe-json / --selftest **共用同一实现** ⇒ 「复算器 = 权威脚本自身」，仓内不存在第二套扫描器
// （PLAN §125.7 加锚次序①；对照教训：k2TraitFaces 因复算器出自一次性工件，至今只能 manual/hard:false）。
// ⚠ 现状口径两条由 --selftest 锁定，改任一必须与 PLAN §0.10 读数同批：
//   ① 归属用文件**整条路径**判 feRe（命名不规范者会误判，PLAN 判读口径 3）；
//   ② import 说明符命中即计，**不剥注释行**（与 Rust 段 §1 剥 `//` 的做法**不同**，56/43、29/11 就是这样数出来的）。
function scanFeDomain(fileList, contentOf) {
  const fe = { STOCK: { owned: [], foreign: [] }, OPC: { owned: [], foreign: [] } };
  for (const f of fileList) {
    if (!FE_FILE_RE.test(f)) continue;
    const src = contentOf(f);
    if (src === null) continue; // --before 语义下改前不存在 / 已删除的文件：跳过不崩
    for (const [dk, d] of Object.entries(DOMAINS)) {
      const hits = [];
      src.split('\n').forEach((l, i) => {
        for (const m of l.matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)) if (d.feRe.test(m[1])) hits.push(i + 1);
      });
      if (!hits.length) continue;
      (d.feRe.test(f) ? fe[dk].owned : fe[dk].foreign).push({ file: f, n: hits.length, lines: hits.slice(0, 8) });
    }
  }
  return fe;
}

const readOrNull = (rel) => {
  try {
    return fs.readFileSync(path.join(repo, rel), 'utf8');
  } catch {
    return null;
  }
};

// docs/INDEX.md 自动登记的三个口径常量。声明必须在 --selftest 短路**之前**：
// 它们是 const（不随函数声明提升），放文件末尾会让自检在 TDZ 里读到 undefined。
const IDX_SECTION_TITLE = '### 审计（AUDIT）';
const IDX_FAMILY_PREFIX = '`AUDIT-l1-blockers-';
// 与现盘同族行逐字一致的三个默认列（INDEX 重整批若改了这些字段，②号拒写判据兜表头侧，这里兜内容侧）
const IDX_DEFAULTS = { version: 'v3.0.0+（未定版，待确认）', feature: '插件体系', source: '本地原创' };

// 自检必须**零盘**且先于任何目录遍历（本仓铁律：判定档之前的自检不许有 IO）
if (SELFTEST) runSelftest();
if (BEFORE_REF !== null && !FE_JSON) feDie('--before 只在 --fe-json 模式下有意义（判定档/落盘档仍是工作树口径）⇒ 拒跑，不做静默忽略');
if (FE_JSON && BEFORE_REF !== null && (!BEFORE_REF || BEFORE_REF.startsWith('-'))) feDie('--before 后面缺 commit 引用（用法：--fe-json --before <commit>）');

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

// --fe-json：机器可读前端反向读数，**只跑 §3**（不扫 Rust、不落盘、不出其它 stdout 字节 ⇒ 供子进程解析）
if (FE_JSON) runFeJson();


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

// ── 3) 前端 import 域文件（判据本体在 scanFeDomain —— 判定档 / --fe-json / --selftest 三方共用）──
const fe = scanFeDomain(files, readOrNull);

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
// （该撞名探测只在 --emit-doc 时有意义；默认档不落盘，见任务 #207。）
let SNAP = NOW_BASE;
if (fs.existsSync(path.join(repo, 'docs/audits/AUDIT-l1-blockers-' + SNAP + '.md'))) SNAP = NOW_BASE + '-' + NOW_HHMM;
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
console.log(L.slice(0, 17).join('\n'));
if (EMIT_DOC) {
  const docRel = 'docs/audits/AUDIT-l1-blockers-' + SNAP + '.md';
  const docPath = path.join(repo, docRel);
  const idxPath = path.join(repo, 'docs/INDEX.md');
  if (fs.existsSync(docPath)) feDie('快照已存在：' + docRel + '（-HHMM 探测后仍撞名 ⇒ 竞态或手工落放）—— 时点快照拒覆盖');
  if (!fs.existsSync(idxPath)) feDie('docs/INDEX.md 不在盘上 ⇒ 无法自动登记 —— 拒绝落盘（否则重演「产物落盘但索引没登记」，任务 #207 的缺陷本体）');
  const reg = registerAuditRow(fs.readFileSync(idxPath, 'utf8'), {
    fileName: 'AUDIT-l1-blockers-' + SNAP + '.md',
    title: 'L1 功能包拆分阻塞面实测（' + SNAP + '）',
    purpose: '本文件由 node scripts/audit-l1-blockers.mjs 生成，是 PLAN §0.10 的数字来源。改判据请改脚本，不要手抄。',
    date: NOW_BASE,
  });
  if (!reg.ok) {
    for (const e of reg.errors) console.error('  - ' + e);
    feDie('docs/INDEX.md 登记被拒 ⇒ 文档与索引**一字未写**（fail-closed：登记不了就不许产文档）');
  }
  fs.writeFileSync(docPath, L.join('\n') + '\n');
  fs.writeFileSync(idxPath, reg.text);
  console.log('\n已落 ' + docRel + '（' + L.length + ' 行），并在 docs/INDEX.md「审计（AUDIT）」主节登记 1 行 active / 同族旧 active 降 superseded ' + reg.superseded + ' 条。');
} else {
  console.log('\n未落盘（默认只读，任务 #207）：要归档时点快照请显式加 --emit-doc —— 脚本会落 docs/audits/ 并自动登记 docs/INDEX.md「审计（AUDIT）」主节（同族旧 active 降 superseded）；登记不成就整体拒写。');
}

// ── --fe-json：机器可读的前端反向读数（复算器 = 本脚本 §3 权威实现自身，PLAN §125.7 加锚次序①）──
function gitOut(argvList) {
  const r = cp.spawnSync('git', argvList, { encoding: 'utf8', maxBuffer: 1 << 28 });
  if (r.error) feDie('git ' + argvList.join(' ') + ' 启动失败：' + r.error.message + '（cwd 是 UNC 路径？请在盘符路径下跑）');
  if (r.status !== 0) feDie('git ' + argvList.join(' ') + ' 退出 ' + r.status + '：' + (r.stderr || '').trim().slice(0, 300));
  return r.stdout;
}

function runFeJson() {
  let commit = null;
  let basis;
  let fileList;
  let contentOf;
  if (BEFORE_REF !== null) {
    basis = 'commit-tree';
    commit = gitOut(['rev-parse', BEFORE_REF]).trim();
    if (!commit) feDie('git rev-parse ' + BEFORE_REF + ' 返回空 ⇒ commit 引用解析不出，拒出数');
    // ⚠ fail-closed：UNC 工作目录下 git 可能「退 0 + 零输出」（本仓实证过的假绿形态）⇒ 空清单宁可拒跑也不出数
    if (!gitOut(['ls-tree', '-r', '--name-only', commit, '--', 'src']).split('\n').filter((p) => FE_FILE_RE.test(p)).length) {
      feDie('git ls-tree ' + commit + ' -- src 返回 0 个 ts/tsx 文件 ⇒ UNC cwd 假成功或引用面错误，拒出数');
    }
    // 一次 ls-tree（带 blob id）+ 一次 `git cat-file --batch` 批量取内容。
    // 逐文件 `git show` 实测在 Windows 上对 ~2 900 个 src 文件要 >120 s（本仓 2026-11-06 实测转后台），
    // 第五道门判定档跑不动 ⇒ 吸收 .tmp/fe-repro.mjs 的覆盖语义时只吸收判据，不吸收它的取数形态。
    const pairs = [];
    for (const l of gitOut(['ls-tree', '-r', commit, '--', 'src']).split('\n')) {
      const tabAt = l.indexOf('\t');
      if (tabAt < 0) continue;
      const p = l.slice(tabAt + 1);
      if (!FE_FILE_RE.test(p)) continue;
      const parts = l.slice(0, tabAt).split(' '); // "<mode> blob <sha>"
      if (parts.length !== 3 || parts[1] !== 'blob') continue;
      pairs.push([p, parts[2]]);
    }
    const raw = cp.spawnSync('git', ['cat-file', '--batch'], { input: Buffer.from(pairs.map((x) => x[1]).join('\n') + '\n', 'utf8'), maxBuffer: 1 << 28 });
    if (raw.status !== 0) feDie('git cat-file --batch 退出 ' + raw.status + '（--before 批量取内容失败，拒出数）');
    const buf = raw.stdout;
    const contents = new Map();
    let pos = 0;
    for (const [p] of pairs) {
      const nl = buf.indexOf(0x0a, pos);
      if (nl < 0) break;
      const header = buf.subarray(pos, nl).toString('utf8');
      pos = nl + 1;
      const hm = /^([0-9a-f]{40}) (\w+) (\d+)$/.exec(header);
      if (!hm || hm[2] !== 'blob') {
        // miss/歧义行（`<sha> missing` 等）⇒ 该文件按不存在处理，scanFeDomain 会跳过
        continue;
      }
      const size = Number(hm[3]);
      contents.set(p, buf.subarray(pos, pos + size).toString('utf8'));
      pos += size + 1; // 内容后紧跟一个 LF
    }
    fileList = pairs.map((x) => x[0]);
    contentOf = (p) => (contents.has(p) ? contents.get(p) : null); // 该树里不存在 ⇒ 跳过不崩
  } else {
    basis = 'worktree';
    fileList = files;
    contentOf = readOrNull;
  }
  const fe = scanFeDomain(fileList, contentOf);
  const perFile = [];
  for (const [side, dk] of [['stock', 'STOCK'], ['opc', 'OPC']]) {
    for (const r of fe[dk].foreign) perFile.push({ domain: side, file: r.file, points: r.n, lines: r.lines });
  }
  perFile.sort((a, b) => b.points - a.points || a.file.localeCompare(b.file));
  console.log(JSON.stringify({
    tool: 'scripts/audit-l1-blockers.mjs --fe-json',
    caliber: '§3 feRe（权威实现自身）：src/**.ts(x) 整条路径不命中域 feRe、而 from/import 说明符命中 ⇒ 计 foreign；注释行不剥（--selftest 锁定，改此口径必须与 PLAN §0.10 读数同批）',
    basis,
    commit,
    stock: { points: sum(fe.STOCK.foreign), files: fe.STOCK.foreign.length },
    opc: { points: sum(fe.OPC.foreign), files: fe.OPC.foreign.length },
    perFile,
  }, null, 1));
  process.exit(0);
}

// ── docs/INDEX.md「审计（AUDIT）」主节自动登记（纯函数：文本进 ⇒ 文本出，--emit-doc 与 --selftest 共用同一实现）──
// 目标主节表头为 9 列（文件名|标题|一句话用途|状态|日期|对应版本|功能点|来源|关联），与既有同族行逐字同构。
// 拒写判据（fail-closed，任一命中 ⇒ INDEX 与文档**都**一字不写）：
//   ① 找不到主节节头（INDEX 结构变了 ⇒ 同批改脚本，不许瞎猜落点）；② 表头列数 ≠ 9 或缺分隔行；
//   ③ 文件名已在节内（防同名二次登记）；④ 节内存在以 `|` 开头但段数 ≠ 11 的畸形行（表格已散架，先修表）。
// ⚠ IDX_SECTION_TITLE / IDX_FAMILY_PREFIX / IDX_DEFAULTS 三个常量声明在文件**前部**（SELFTEST 短路之前）——
//   它们是 const，不随函数声明提升，放这里会让 --selftest 撞上 TDZ。
function registerAuditRow(indexText, meta) {
  const lines = indexText.split('\n');
  const fail = (errors) => ({ ok: false, text: indexText, superseded: 0, errors });
  const secStart = lines.findIndex((l) => l.trim() === IDX_SECTION_TITLE);
  if (secStart === -1) return fail(['找不到主节节头「' + IDX_SECTION_TITLE + '」—— docs/INDEX.md 结构已变，同批改脚本，拒绝猜测落点']);
  let secEnd = lines.length;
  for (let i = secStart + 1; i < lines.length; i++) {
    if (/^#{1,6}\s/.test(lines[i])) {
      secEnd = i;
      break;
    }
  }
  let headerAt = -1;
  for (let i = secStart + 1; i < secEnd; i++) {
    if (/^\|\s*文件名\s*\|/.test(lines[i])) {
      headerAt = i;
      break;
    }
  }
  if (headerAt === -1) return fail(['主节内未见「| 文件名 |」表头（节高塌陷或表被重建）']);
  const cols = lines[headerAt].split('|').slice(1, -1).map((s) => s.trim());
  if (cols.length !== 9) return fail(['审计主节表头为 ' + cols.length + ' 列 ≠ 9（' + cols.join(',') + '）—— 字段格式已变，同改 scripts/audit-l1-blockers.mjs']);
  if (!/^\|[\s|:-]+\|/.test(lines[headerAt + 1] || '')) return fail(['表头下一行不是分隔行']);
  const errors = [];
  const out = lines.slice();
  let superseded = 0;
  for (let i = headerAt + 2; i < secEnd; i++) {
    const l = lines[i];
    if (!l.startsWith('|')) continue;
    const cells = l.split('|');
    if (cells.length !== 11) {
      errors.push('第 ' + (i + 1) + ' 行段数 ≠ 11（畸形行 ⇒ 拒绝猜列）：' + l.slice(0, 80));
      continue;
    }
    if (cells[1].trim() === '`' + meta.fileName + '`') errors.push('文件名 ' + meta.fileName + ' 已在主节登记 ⇒ 拒绝二次登记');
    if (cells[1].trim().startsWith(IDX_FAMILY_PREFIX) && cells[4].trim() === 'active') {
      cells[4] = ' superseded ';
      out[i] = cells.join('|');
      superseded++;
    }
  }
  if (errors.length) return fail(errors);
  out.splice(headerAt + 2, 0, '| `' + meta.fileName + '` | ' + meta.title + ' | ' + meta.purpose + ' | active | ' + meta.date + ' | ' + IDX_DEFAULTS.version + ' | ' + IDX_DEFAULTS.feature + ' | ' + IDX_DEFAULTS.source + ' |  |');
  return { ok: true, text: out.join('\n'), superseded, errors: [] };
}

// ── --selftest（纯函数正负控、零盘；先于任何目录遍历短路。本仓铁律：自检不过，后面的「通过」没有意义）──
function runSelftest() {
  let fail = 0;
  const chk = (cond, msg) => {
    if (!cond) {
      console.error('SELFTEST FAIL：' + msg);
      fail++;
    }
  };

  // —— 一) scanFeDomain：复算器行为锁定（正控 + owned/删除文件/灵敏度 三负控 + 注释口径锁定）——
  const FIX_FE = {
    'src/lib/helper.ts': "import { fmt } from '@/lib/stock-utils';\nimport a from './plain';\n",
    'src/components/stock/Panel.tsx': "import { x } from '@/lib/stock-api';\n",
    'src/pages/Plain.tsx': "// import { q } from '@/lib/market-data';\nimport { q } from '@/lib/market-data';\n",
    'src/lib/opc-thing.ts': "import pack from '@/features/capability-pack/store';\n",
    'src/x/gone.ts': null,
  };
  const fixContent = (p) => (p in FIX_FE ? FIX_FE[p] : null);
  const fe = scanFeDomain(Object.keys(FIX_FE), fixContent);
  const pts = (dk) => fe[dk].foreign.reduce((x, r) => x + r.n, 0);
  chk(pts('STOCK') === 3 && fe.STOCK.foreign.length === 2, '正控：STOCK 反向应为 3 处 / 2 文件（helper 1 + Plain 2），实得 ' + pts('STOCK') + ' / ' + fe.STOCK.foreign.length);
  chk(fe.STOCK.foreign.every((r) => r.file !== 'src/components/stock/Panel.tsx') && fe.STOCK.owned.length === 1, '负控：路径自身命中 feRe 的文件落 owned、不得落 foreign（域自有随迁出整体搬走）');
  chk(fe.OPC.foreign.length === 0 && pts('OPC') === 0 && fe.OPC.owned.length === 1, '负控：opc 路径文件 import capability-pack 说明符 ⇒ 计 owned 侧，OPC foreign 保持空');
  chk(!fe.STOCK.foreign.some((r) => r.file === 'src/x/gone.ts'), '负控：内容读不到的文件（--before 语义下改前已删）跳过、不崩、不计');
  chk((fe.STOCK.foreign.find((r) => r.file === 'src/pages/Plain.tsx') || {}).n === 2, '口径锁定：注释行里的 import **照计** —— 56/43、29/11 就是这样数出来的；改这条必须与 PLAN §0.10 读数同批，不许借自检偷改口径');
  const fe2 = scanFeDomain(['src/pages/Plain.tsx'], () => "import { q } from './plain';\n");
  chk(fe2.STOCK.foreign.length === 0 && fe2.OPC.foreign.length === 0, '灵敏度负控：把说明符换成非域路径 ⇒ foreign 必须归零（判据没跑才会让合成读数原样不动）');

  // —— 二) registerAuditRow：自动登记（正控）+ 四种拒写（负控），全部走合成文本、零真盘 ——
  const IDX_FIX = [
    '# Index',
    '',
    '### 插件体系（`plugin-system`）',
    '',
    '| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |',
    '|---|---|---|---|---|---|---|',
    '| `AUDIT-l1-blockers-2026-11-06.md` | 审计 | 功能视图同名行——自动登记不得触碰 | active | 2026-11-06 | v | 本地原创 |',
    '',
    IDX_SECTION_TITLE,
    '',
    '| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 功能点 | 来源 | 关联 |',
    '|---|---|---|---|---|---|---|---|---|',
    '| `AUDIT-l1-blockers-2026-11-06.md` | T6 | P | active | 2026-11-06 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |',
    '| `AUDIT-l1-blockers-2026-11-05.md` | T5 | P | superseded | 2026-11-05 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 |  |',
    '| `AUDIT-capability-inventory-2026-11-01.md` | I | P2 | active | 2026-11-01 | v3.0.0+（未定版，待确认） | 插件体系 | 本地原创 | `PLAN-x.md` |',
    '',
    '### 参考（REFERENCE）',
    '',
    '| 文件名 | 类型 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |',
  ].join('\n');
  const META = { fileName: 'AUDIT-l1-blockers-2026-11-07-1000.md', title: 'L1 功能包拆分阻塞面实测（2026-11-07-1000）', purpose: 'P0', date: '2026-11-07' };
  const g = registerAuditRow(IDX_FIX, META);
  chk(g.ok === true, '正控：合成 INDEX（9 列主节 + 同族 1 active + 1 superseded + 外域 1 active）应登记成功，errors=' + (g.errors || []).join(' | '));
  if (g.ok) {
    const rows = g.text.split('\n');
    const sepAt = rows.findIndex((l, i) => i > 10 && l === '|---|---|---|---|---|---|---|---|---|');
    const newAt = rows.findIndex((l) => l.includes('`' + META.fileName + '`'));
    chk(newAt === sepAt + 1, '正控：新行落在表体最前（分隔行下一行），实得 newAt=' + newAt + ' sepAt=' + sepAt);
    chk(rows[newAt].split('|').length === 11 && rows[newAt].includes(' | active | 2026-11-07 | ' + IDX_DEFAULTS.version + ' | ' + IDX_DEFAULTS.feature + ' | ' + IDX_DEFAULTS.source + ' |  |'), '正控：新行 9 列、状态/日期/对应版本/功能点/来源与同族既有行逐字同构');
    chk(g.superseded === 1, '正控：同族唯一 active 行应降级恰 1 条，实得 ' + g.superseded);
    chk(g.text.split('\n')[6] === IDX_FIX.split('\n')[6], '边界负控：功能视图 7 列表的同名 active 行一字未动（降级只发生在审计主节内，节外不碰）');
    chk(g.text.split('\n').some((l) => l.includes('AUDIT-capability-inventory') && l.includes(' active ')), '边界负控：外域 active 行不得被同族降级误伤');
    chk(g.text.split('\n').slice(0, 9).join('\n') === IDX_FIX.split('\n').slice(0, 9).join('\n'), '正控：节头以上逐字未动');
    chk(g.text.split('\n').slice(16).join('\n') === IDX_FIX.split('\n').slice(15).join('\n'), '正控：下一节头起（含其前空行）逐字未动');
  }
  const fixLines = IDX_FIX.split('\n');
  const famAt = fixLines.findIndex((l) => l.includes('`AUDIT-l1-blockers-2026-11-05.md`'));
  const hdrAt = fixLines.findIndex((l) => l.startsWith('| 文件名 | 标题 |'));
  const mkBad = (at, rep) => fixLines.map((l, i) => (i === at ? rep : l)).join('\n');
  const refused = [
    ['无节头', mkBad(8, '### 审计 XXX'), registerAuditRow(mkBad(8, '### 审计 XXX'), META)],
    ['表头列数≠9', mkBad(hdrAt, '| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |'), registerAuditRow(mkBad(hdrAt, '| 文件名 | 标题 | 一句话用途 | 状态 | 日期 | 对应版本 | 来源 |'), META)],
    ['重复登记', IDX_FIX, registerAuditRow(IDX_FIX, Object.assign({}, META, { fileName: 'AUDIT-l1-blockers-2026-11-06.md' }))],
    ['畸形行', mkBad(famAt, '| `AUDIT-l1-blockers-2026-11-05.md` | T5 | 少列 |'), registerAuditRow(mkBad(famAt, '| `AUDIT-l1-blockers-2026-11-05.md` | T5 | 少列 |'), META)],
  ];
  for (const [name, input, r] of refused) {
    chk(r.ok === false && r.text === input && (r.errors || []).length > 0, '负控（自动登记）：' + name + ' ⇒ 必须拒写、且返回文本与**当次输入**逐字相等（不许「部分成功」），实得 ok=' + r.ok + ' errors=' + (r.errors || []).join(' | '));
  }

  if (fail > 0) {
    console.error('SELFTEST FAIL 共 ' + fail + ' 条');
    process.exit(1);
  }
  console.log('SELFTEST OK：前端反向复算器（STOCK/OPC 正控 · owned/已删文件/灵敏度 三负控 · 注释行口径锁定）· INDEX 自动登记（正控 6 断 · 无节头/列数不符/重复登记/畸形行 四拒写负控 · 节外边界两条）全部通过');
  process.exit(0);
}
