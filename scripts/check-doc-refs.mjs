#!/usr/bin/env node
// scripts/check-doc-refs.mjs — 文档「文件:行号」引用的段对齐门禁（2026-11-04）
//
// 补的是什么盲区：`AGENTS.md`「文档中的源码引用写法（强制）」立了规则 —— 文档里的
//   `文件:行号` 必须**段对齐**（完整路径 / 唯一后缀 / 唯一 basename），禁止从文件名
//   **中间**截断或换同义词。但这条规则**此前没有任何自动检查**，只靠人撞：
//   `docs/plans/PLAN-shell-plus-plugins.md` §99.4 手工抓到过 `worker.rs:3798`
//   实际已差 492 行 —— 行号引用**会随源码编辑持续腐烂**，而编译 / clippy / 单测 / 现有
//   任何门禁都不会红（它们不读文档）。
//
// 为什么必须是独立门禁（而不是并进 `check-single-source-facts.mjs`）：
//   那个脚本自己声明「**不扫** Markdown 文档」—— 它的 `--dump-doc-refs` 是**取证模式**，
//   刻意不产出 failure / 不进退出码（`.md` 里存在大量「故意保留的历史行号」，见其头部）。
//   本门禁是**判据**：按桶分类后只硬拦客观错，历史存量走棘轮基线冻结。
//   引用抽取 / 定位原语与它**共用** `scripts/lib/doc-refs.mjs`（同一份实现，判据不分叉）。
//
// 扫描面：`docs/**/*.md`（**全部**，不限 `docs/INDEX.md` 登记过的）——
//   未登记的文档同样会被阅读，同样会腐烂；不扫它们等于「未检查」（判据 #7：
//   未检查与检查通过长得一样）。`--selftest` 走临时目录，不依赖仓内真实文档。
//
// 分桶（每条引用 token 一个判定，同一文档内重复引用去重）：
//   · OK        —— 唯一可定位（完整路径 / 唯一后缀 / 唯一 basename），且行号在文件行数内
//   · NONLOC    —— 同名多份、路径后缀也不能消歧（如裸 `mod.rs`）⇒ **未检查**，不是通过
//   · STALE     —— 文件存在但行号越界（含区间终点越界）⇒ **红**
//   · BROKEN    —— 仓内不存在该文件，且找不到被截断/换词的候选 ⇒ **红**
//   · ABBREV    —— 不存在，但**自证**到它缩写的真实文件（段内截断 / 同义词换词）⇒ **只报告**
//                  每条必须打印候选；找不到候选的**不许进本桶**（否则就成了猜）
//   · EXTERNAL  —— 跨仓引用（路径带 `name-x.y.z/` 版本目录，或本机绝对路径）⇒ 只报告
//   · EPHEMERAL —— 落在**不入版本控制 / 环境相关**的目录（`.workbuddy/`、`output/` …）⇒ 只报告
//                  理由：这些目录在 CI 检出里**不存在**，硬拦会让「本地绿、CI 红」，
//                  而那不是文档缺陷，是环境差异（同 `check-single-source-facts` 的 `.rs→.md` 豁免）
//   · MALFORMED —— 抽取出来的串根本不是路径（散文 / 表格残片）⇒ 丢弃，只计数
//                  实证：`bash.rs:266/:293` 这类写法旁边若还有裸 `:293`，归并时会拼出含
//                  反引号 / 中文的怪物串。判成 BROKEN 只会给基线灌垃圾（桶就指不回真实风险了）。
//
// 退出码：0 = 无新增 BROKEN / STALE；1 = 有新增（或 selftest 失败）；
//         2 = 脚本自身失效（扫描面 0 篇文档 / 基线不可读）。
//   ⚠ ABBREV 首轮**只报告**（存量未知且量级可能很大）；判据未放宽 —— 它只是还没被提升为红。
//
// 棘轮基线 `scripts/doc-refs-baseline.json`（**只减不增**）：
//   键 = `<文档相对路径>::<引用串>:<起始行>[-<结束行>]`（**不含文档行号** —— 文档增删会让
//   文档行号整体漂移，那正是本门禁要抓的腐烂形态，拿它当键会自己先烂）。
//   · 常规运行：基线内的存量不拦，基线外新增 → 红；基线内已消失 → 打印「可收紧」，不红；
//   · `--init-baseline`：**仅当基线文件不存在**时建立（首轮冻结存量；存在则拒绝，不许洗白）；
//   · `--update-baseline`：收紧（先跑常规运行确认无新增，否则拒绝执行）。
//
// 用法：
//   node scripts/check-doc-refs.mjs
//   node scripts/check-doc-refs.mjs --selftest
//   node scripts/check-doc-refs.mjs --dump-doc-refs        # 逐条打印 + 判定 + 命中的真实文件
//   node scripts/check-doc-refs.mjs --init-baseline        # 首轮冻结存量（仅当基线不存在）
//   node scripts/check-doc-refs.mjs --update-baseline      # 存量修复后收紧基线

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  abbrevCandidates,
  buildBasenameIndex,
  extractRefs,
  indexFromFileList,
  locateRef,
  synonymCandidates,
  walkRepoFiles,
} from "./lib/doc-refs.mjs";
import { discoverWorkspaceRoots, repoRel as repoRelOf } from "./lib/workspace-roots.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const BASELINE_PATH = path.join(HERE, "doc-refs-baseline.json");
const DOCS_DIR = path.join(ROOT, "docs");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const DUMP = args.includes("--dump-doc-refs");
const INIT_BASELINE = args.includes("--init-baseline");
const UPDATE_BASELINE = args.includes("--update-baseline");

// ── 文档面扩展名白名单 ───────────────────────────────────────────────
// 比源码面（`DEFAULT_REF_EXTS`）宽：文档引用的目标常是 `.json` / `.css` / `.html` /
// `.lock` / `.proto`。抽取器本体仍只有一份（`lib/doc-refs.mjs`），这里只换白名单。
const DOC_REF_EXTS = [
  "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "sh", "ps1", "bat", "cmd",
  "json", "toml", "lock", "md", "rhai", "py", "sql", "yml", "yaml",
  "css", "html", "proto", "txt", "csv",
].join("|");

// ── 目标文件索引的排除口径（覆盖面判据，改动前必须量）──────────────
// 检出副本（`.worktrees` / `.qoder/worktrees`）与产物副本（`output/**`、`.tmp`）里装着
// 整棵树的副本 ⇒ 同一 basename 撑成「同名 N 份」⇒ 本该可定位的引用退化成 NONLOC（= 未检查）。
// `docs` **不排除**：文档之间的互引（`AUDIT-x.md:12`）同样要被检查。
const SKIP_DIRS = new Set([
  "target", "node_modules", ".git", "dist", "coverage", "knowledge-sources",
  ".output", ".vite", ".monaco", ".worktrees", ".qoder", "output", ".tmp",
]);

// ── 环境相关目录（CI 检出里不存在 ⇒ 不判红）────────────────────────
// 判据是**路径首段**（或路径中某段）命中；这些目录都不在版本控制里
// （`.workbuddy/memory` 是本地记忆机制，`output/` 是产物/备份，`.tmp/` 是运行日志）。
// ⚠ 这只是「不判红」，仍然**计数并打印** —— 别把「免检」读成「检查通过」。
const EPHEMERAL_PREFIXES = [
  ".workbuddy/", "workbuddy/", ".qoder/", "qoder/", ".tmp/", "tmp/", "output/", "target/",
  "node_modules/", "dist/", "coverage/", ".vite/", ".monaco/",
  "knowledge-sources/", ".worktrees/",
];

/** 第三方 crate / 跨仓源码的版本目录形态：`sea-orm-2.0.2/src/…`。 */
const EXTERNAL_PATH_RE = /(?:^|\/)[A-Za-z0-9_.-]+-\d+\.\d+\.\d+\//;
/** 本机绝对路径（`D:\…`、`\\host\…`、`/home/…`）—— 换机器即失效，属环境相关。 */
const ABSOLUTE_PATH_RE = /^(?:[A-Za-z]:[\\/]|\\\\|\/)/;

/** 遍历 `docs/` 收全部 `.md`（绝对路径，排序保证输出稳定）。 */
export function listDocFiles(root) {
  const out = [];
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return; // 目录不存在 = 安全 no-op（调用方另有「扫描面为 0 即红」的判据）
    }
    for (const e of entries) {
      if (e.name === "node_modules" || e.name === ".git") continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile() && e.name.endsWith(".md")) out.push(p);
    }
  };
  walk(path.join(root, "docs"));
  return out.sort();
}

/**
 * 仓库文件清单（绝对路径）。**优先 `git ls-files`**：
 *   · 本机是 VMware 共享目录，递归 `readdir` 整个仓要数分钟（实测首版卡死）；
 *     git 索引读取是秒级，且 CI（Linux 本地盘）同样可用；
 *   · 口径恰好是「已入版本控制 + 未忽略的新文件」⇒ 本地与 CI 检出**同一集合**，
 *     不会因为某台机器上多了一份检出副本 / 产物目录而改变判定（那是环境差异，不是缺陷）；
 *   · 未跟踪但**未被忽略**的新文件（正在写的文档 / 源码）也在内（`--others --exclude-standard`）。
 * 拿不到 git（无 `.git` / 未安装）时退回文件系统遍历，并在输出里自陈口径。
 * @returns {{files:string[], source:"git"|"walk"}}
 */
export function listRepoFiles(root, skipDirs) {
  const r = spawnSync("git", ["-C", root, "ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
    maxBuffer: 256 * 1024 * 1024,
    encoding: "buffer",
  });
  if (!r.error && r.status === 0) {
    const files = r.stdout
      .toString("utf8")
      .split("\0")
      .filter(Boolean)
      .map((rel) => path.join(root, rel.split("/").join(path.sep)));
    if (files.length > 0) return { files, source: "git" };
  }
  return { files: walkRepoFiles(root, skipDirs), source: "walk" };
}

/**
 * 基于**文件清单集合**的路径解析器（与 `resolveRepoFile` 同序候选，但零 I/O）。
 *
 * 为什么不用 `resolveRepoFile`：它逐条 `stat` 候选路径，SMB 上实测 ~190ms/条
 * （1.2 万条引用 ⇒ 38 分钟，首版就是这么卡住的）。这里把候选路径前缀预先展开，
 * 只在 `Set` 里查存在性 —— 微秒级。候选顺序与 lib 逐字对齐：
 *   ① 仓库根 + rel ｜ ② 各 workspace 根 + rel ｜ ③ 各 workspace 根 + `crates/` + rel
 */
export function makeFileSetResolver(root, files) {
  const relSet = new Set(files.map((abs) => repoRelOf(root, abs)));
  const roots = discoverWorkspaceRoots(root).map((r) => repoRelOf(root, r));
  const prefixes = ["", ...roots.map((r) => `${r}/`), ...roots.map((r) => `${r}/crates/`)];
  return (refFile) => {
    const rel = refFile.split("\\").join("/").replace(/^\.\//, "");
    for (const pre of prefixes) {
      const cand = `${pre}${rel}`;
      if (relSet.has(cand)) return path.join(root, cand.split("/").join(path.sep));
    }
    return null;
  };
}

/** 纯函数：该引用串是否落在环境相关目录（见 `EPHEMERAL_PREFIXES`）。 */
export function isEphemeralRef(refFile) {
  const p = refFile.replace(/^\.\//, "");
  return EPHEMERAL_PREFIXES.some((pre) => p.startsWith(pre) || p.includes("/" + pre));
}

/** 纯函数：该引用串是否跨仓 / 本机绝对路径（见上方两条正则）。 */
export function isExternalRef(refFile) {
  return EXTERNAL_PATH_RE.test(refFile) || ABSOLUTE_PATH_RE.test(refFile);
}

/**
 * 纯函数：该「引用串」是否根本不是路径（散文 / 表格残片）。
 *
 * 实证（首版）：`bash.rs:266/:293` 这类**多行清单**写法配上一行里后续的裸 `:293`，
 * 归并后曾拼出 `266/:293`` 用` 这种既含反引号又含空白/中文的字符串。它不是引用，
 * 是抽取器的边界噪声 —— 判成 BROKEN 只会给基线灌垃圾（`BROKEN` 桶就指不回真实风险了）。
 * 判据是**形态**、不是白名单：出现路径字符集之外的字符 / 以 `/` 收尾 / 空段 ⇒ 不是路径。
 */
export function isMalformedRef(refFile) {
  if (!refFile || refFile.length > 300) return true;
  // 允许前导点 / 省略号 / 斜杠 / 连字符：文档里常见「承前省略」写法
  // （`portfolio-mgr-h-ultra-short.rhai:303`、`-short.rhai:215`、`.../sqlite/table.rs:108`）
  // —— 它们**仍是引用**，交给 ABBREV 的候选自证去认领，不能在这里丢掉。
  if (!/^[.\-/]*[A-Za-z0-9_][A-Za-z0-9_./-]*$/.test(refFile)) return true;
  if (refFile.endsWith("/") || refFile.includes("//")) return true;
  return false;
}

/**
 * 行数（按 `\n` 切；末尾换行**不多算一行** —— 否则 `:N+1` 会被当成合法行）。
 * 结果缓存由调用方持有（同一目标文件常被多条引用命中）。
 */
function lineCount(abs, cache) {
  if (!cache.has(abs)) {
    let text;
    try {
      text = fs.readFileSync(abs, "utf8");
    } catch (e) {
      // 文件索引来自 `git ls-files --cached`：工作区里**已删除但尚未提交**的文件仍在索引中
      // （实测：opc 子树搬迁期间，文档仍指向旧路径 ⇒ readFileSync 抛 ENOENT 把整道门禁打崩）。
      // 盘上不存在 = 引用不可定位 ⇒ 与「仓内无此文件」同判 BROKEN，不能抛异常。
      if (e.code !== "ENOENT") throw e;
      cache.set(abs, null);
      return null;
    }
    const lines = text.split(/\r?\n/);
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
    cache.set(abs, lines.length);
  }
  return cache.get(abs);
}

/**
 * 单条引用判定（纯函数：文件系统只经 `lineCount` / `index` 访问）。
 * @returns {{bucket:string, target?:string, candidates?:string[], kind?:string, lines?:number}}
 */
export function classifyRef(ref, ctx) {
  const { refFile, start, end } = ref;
  if (isMalformedRef(refFile)) return { bucket: "MALFORMED" };
  if (isEphemeralRef(refFile)) return { bucket: "EPHEMERAL" };
  if (isExternalRef(refFile)) return { bucket: "EXTERNAL" };

  const loc = ctx.locate(refFile);
  if (loc.kind === "N") return { bucket: "NONLOC", why: loc.why };
  if (loc.kind === "B") {
    // 自证：先找「段内截断」（字符级后缀），再找「换词」（词干集合包含）
    const trunc = abbrevCandidates(refFile, ctx.index, ctx.root);
    if (trunc.length > 0) return { bucket: "ABBREV", kind: "截断", candidates: trunc };
    const syn = synonymCandidates(refFile, ctx.index, ctx.root);
    if (syn.length > 0) return { bucket: "ABBREV", kind: "同义词", candidates: syn };
    // 找不到候选 ⇒ 明确「无候选」，绝不猜（猜出来的桶指向不了真实风险，见 AGENTS.md）
    return { bucket: "BROKEN", candidates: [] };
  }
  const n = lineCount(loc.abs, ctx.lineCache);
  if (n === null) return { bucket: "BROKEN", candidates: [] };
  const overflow = start > n || (end && end > n);
  return {
    bucket: overflow ? "STALE" : "OK",
    target: ctx.repoRel(loc.abs),
    lines: n,
  };
}

/**
 * 引用串的**左边界补全**。
 *
 * 为什么需要：共用的抽取器（`lib/doc-refs.mjs` 的 `extractRefs`）要求文件名的首字符是
 * `[A-Za-z0-9_]`，于是文档里常见的**点开头路径**会被截短：
 *   `.github/workflows/ci.yml:12` → 抽出 `github/workflows/ci.yml:12`（少了前导点）
 *   `.workbuddy/memory/X.md:3`    → 抽出 `workbuddy/memory/X.md:3`
 * 截短后的串在本仓**不存在** ⇒ 会被判 BROKEN（假红），而它指向的文件明明在。
 * 修法放在**本门禁侧**而不是改共用抽取器：改抽取器会改变 `check-single-source-facts.mjs`
 * 的 `.rs` 面读数（那边有棘轮基线，凭空多出命中会直接红），风险远大于收益。
 *
 * 判据：从匹配起点向左吃掉连续的路径字符（含 `.`），只影响**前导**，不改后缀。
 */
function expandRefFile(ref) {
  const line = ref.text ?? "";
  // ⚠ 只对**带文件名的锚**生效：裸 `:293`（承接同一行的前一个文件名）在原文里
  //   并没有 `ref.file` 这几个字符，盲目向左吃会把前一段行号（`266/`）和文件名拼成
  //   `266/:293`` 用` 这类怪物串（首版实测，全是假 BROKEN）。判据：该列区间内的原文
  //   必须**逐字等于** refFile，否则原样返回。
  if (line.slice(ref.col, ref.col + ref.file.length) !== ref.file) return ref.file;
  let i = ref.col;
  while (i > 0 && /[A-Za-z0-9_./-]/.test(line[i - 1])) i--;
  if (i === ref.col) return ref.file;
  return line.slice(i, ref.col + ref.file.length);
}

/**
 * 扫描 + 判定（纯函数：不写盘、不 exit）。selftest 与正式跑共用这一条路径。
 *
 * @param {{root:string, docs:string[], index:Map<string,string[]>, repoRel:(abs:string)=>string,
 *          resolve?:(refFile:string)=>(string|null)}} opts
 *   `resolve` 缺省走 `locateRef` 的逐条 stat 解析（selftest 用）；批量跑必须传内存解析器
 *   （见 `makeFileSetResolver`，理由：SMB 上逐条 stat ~190ms）。
 * @returns {{refs:object[], counts:Record<string,number>, docs:number}}
 */
export function auditDocs({ root, docs, index, repoRel, resolve }) {
  const lineCache = new Map();
  // 同一条引用常被多份文档重复引用（登记表 / 汇总页）⇒ 解析结果按引用串缓存，
  // 否则同一目标会被 stat（或查集合）上千次。
  const locCache = new Map();
  const locate = (refFile) => {
    if (!locCache.has(refFile)) {
      locCache.set(refFile, locateRef(refFile, index, root, resolve));
    }
    return locCache.get(refFile);
  };
  const ctx = { root, index, repoRel, lineCache, locate };
  const seen = new Set();
  const refs = [];
  for (const abs of docs) {
    const docRel = repoRel(abs);
    const text = fs.readFileSync(abs, "utf8");
    // `extractRefs` 把 `:A-B` 拆成两条（起点 + 终点），`:A/B` 拆成两条多行清单。
    // 同一个 token（同一列区间）归并成**一条**引用：区间终点只是边界，不是另一条引用。
    const toks = new Map();
    for (const r of extractRefs(text, DOC_REF_EXTS)) {
      const k = `${r.srcLine}:${r.col}`;
      if (!toks.has(k)) {
        toks.set(k, { file: expandRefFile(r), srcLine: r.srcLine, start: r.line, end: r.line });
      }
      const t = toks.get(k);
      t.start = Math.min(t.start, r.line);
      t.end = Math.max(t.end, r.line);
    }
    for (const t of [...toks.values()].sort((a, b) => a.srcLine - b.srcLine || a.start - b.start)) {
      const refFile = t.file;
      const key = `${docRel}::${refFile}:${t.start}${t.end !== t.start ? `-${t.end}` : ""}`;
      if (seen.has(key)) continue; // 同一文档内重复引用 ⇒ 只判一次（输出不被淹）
      seen.add(key);
      const verdict = classifyRef({ refFile, start: t.start, end: t.end }, ctx);
      refs.push({ key, doc: docRel, docLine: t.srcLine + 1, refFile, start: t.start, end: t.end, ...verdict });
    }
  }
  const counts = {};
  for (const r of refs) counts[r.bucket] = (counts[r.bucket] ?? 0) + 1;
  return { refs, counts, docs: docs.length };
}

/** 纯函数：与基线比对。返回 {newViolations, fixedCount, baseSize}。 */
export function diffAgainstBaseline(baseKeys, violations) {
  const cur = new Set(violations.map((v) => v.key));
  return {
    newViolations: violations.filter((v) => !baseKeys.has(v.key)),
    fixedCount: [...baseKeys].filter((k) => !cur.has(k)).length,
    baseSize: baseKeys.size,
  };
}

/** 只有 BROKEN / STALE 判红（ABBREV / NONLOC / EXTERNAL / EPHEMERAL 只报告）。 */
export function hardViolations(refs) {
  return refs.filter((r) => r.bucket === "BROKEN" || r.bucket === "STALE");
}

// ── selftest：正负对照（临时目录，不依赖仓内真实文件）────────────────
function runSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refs-selftest-"));
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    // 目标文件：三档合法引用各自有对应真实文件；另有 trunc/synonym/越界/同名多份的靶子
    write("src/a.rs", "fn a() {}\n");
    write("src/commands/b.rs", "fn b() {}\n");
    write("src/c.rs", "l1\nl2\nl3\n");
    write("src/init/state.rs", "// s\n");
    write("src/other/state.rs", "// s\n");
    write("src/x/m.rs", "// m\n");
    write("src/y/m.rs", "// m\n");
    write("src/opc_admin_actions.rs", "// 候选（段内截断）\n");
    write("src/opc_capability_pack_bridge.rs", "// 候选（换词）\n");
    write("src/short.rs", "l1\nl2\nl3\n");
    // 点开头路径：抽取器会截掉前导点 ⇒ 本门禁必须靠左边界补全把它救回来（不得判 BROKEN）
    write(".github/workflows/ci.yml", "jobs: {}\n");

    // 正控三档 + 负控五类（段内截断 / 同义词 / 无候选 / 越界 / 同名多份不可消歧）
    write(
      "docs/guide.md",
      [
        "见 `src/a.rs:1`（完整路径）",
        "见 `commands/b.rs:1`（唯一后缀）",
        "见 `c.rs:3`（唯一 basename）",
        "见 `init/state.rs:1`（唯一后缀消歧）",
        "见 `admin_actions.rs:4`（段内截断）",
        "见 `domain_pack_bridge.rs:5`（同义词换词）",
        "见 `ghost_file.rs:6`（无候选）",
        "见 `src/short.rs:99`（行号越界）",
        "见 `m.rs:1`（同名多份且后缀不可消歧）",
        "见 `sea-orm-2.0.2/src/schema/entity.rs:10`（跨仓）",
        "见 `.workbuddy/memory/MEMORY.md:3`（环境相关）",
        "见 `src/short.rs:1-99`（区间终点越界）",
        "见 `.github/workflows/ci.yml:1`（点开头路径：抽取器会截掉前导点，必须补全）",
        "",
      ].join("\n"),
    );

    check(listDocFiles(tmp).length === 1, "扫描面：临时目录应收到 1 篇文档", listDocFiles(tmp).length);
    // 负控（面试）：docs 不存在时扫描面必须为 0 ⇒ 调用方据此判「脚本失效」，不许静默绿
    check(listDocFiles(path.join(tmp, "nope")).length === 0, "扫描面为 0 时必须能被识别（不得静默通过）");

    const index = buildBasenameIndex(tmp, SKIP_DIRS);
    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const { refs, counts } = auditDocs({ root: tmp, docs: listDocFiles(tmp), index, repoRel });

    // 期望：OK 5 ｜ ABBREV 2 ｜ BROKEN 1 ｜ STALE 2（`:99` 与 `:1-99`）｜ NONLOC 1 ｜ EXTERNAL 1 ｜ EPHEMERAL 1
    check(
      counts.OK === 5,
      "正控：完整路径 / 唯一后缀 / 唯一 basename / 后缀消歧 / 点开头路径 应判 OK=5",
      counts.OK,
    );
    check(counts.ABBREV === 2, "段内截断 + 同义词换词 应判 ABBREV=2", counts.ABBREV);
    check(counts.BROKEN === 1, "无候选的引用应判 BROKEN=1", counts.BROKEN);
    check(counts.STALE === 2, "行号越界（单行 + 区间终点）应判 STALE=2", counts.STALE);
    check(counts.NONLOC === 1, "同名多份且后缀不可消歧应判 NONLOC=1", counts.NONLOC);
    check(counts.EXTERNAL === 1, "版本目录形态的跨仓引用应判 EXTERNAL=1", counts.EXTERNAL);
    check(counts.EPHEMERAL === 1, "环境相关目录的引用应判 EPHEMERAL=1", counts.EPHEMERAL);

    // ABBREV 的「自证」必须点到真实文件 —— 找不到候选就不许落这个桶
    const abbrev = refs.filter((r) => r.bucket === "ABBREV");
    const trunc = abbrev.find((r) => r.refFile === "admin_actions.rs");
    check(!!trunc, "段内截断样本必须落在 ABBREV 桶");
    check(
      !!trunc && trunc.candidates.includes("src/opc_admin_actions.rs"),
      "段内截断必须自证到 src/opc_admin_actions.rs",
      trunc?.candidates,
    );
    check(trunc?.kind === "截断", "段内截断应标 kind=截断", trunc?.kind);
    const syn = abbrev.find((r) => r.refFile === "domain_pack_bridge.rs");
    check(
      !!syn && syn.candidates.includes("src/opc_capability_pack_bridge.rs"),
      "同义词换词必须自证到 src/opc_capability_pack_bridge.rs",
      syn?.candidates,
    );
    check(syn?.kind === "同义词", "同义词换词应标 kind=同义词", syn?.kind);
    const ghost = refs.find((r) => r.refFile === "ghost_file.rs");
    check(ghost?.bucket === "BROKEN" && ghost.candidates.length === 0, "无候选必须明确报「无候选」");

    // 回归钉：文件索引由 git 索引构建，工作区里「已删除但未提交」的文件仍在其中。
    // 这时 locate 会返回一个盘上不存在的绝对路径 —— 必须判 BROKEN，不许 readFileSync 打崩整道门。
    const vanishedCtx = {
      locate: () => ({ kind: "L", abs: path.join(tmp, "src", "vanished.rs") }),
      index,
      root: tmp,
      repoRel,
      lineCache: new Map(),
    };
    check(
      classifyRef({ refFile: "vanished.rs", start: 1 }, vanishedCtx).bucket === "BROKEN",
      "索引内有、盘上已删（搬迁未提交）必须判 BROKEN 而非抛 ENOENT",
    );

    // 红面 = BROKEN + STALE（ABBREV 只报告，不许进红）
    const hard = hardViolations(refs);
    check(hard.length === 3, "判红面应恰为 BROKEN 1 + STALE 2 = 3", hard.length);
    check(!hard.some((r) => r.bucket === "ABBREV"), "ABBREV 不得进红面（首轮只报告）");

    // 形态护栏：不是路径的串必须被丢弃（否则给基线灌垃圾，BROKEN 桶就指不回真实风险）
    check(isMalformedRef("266/:293` 用") === true, "散文残片（含反引号 / 中文）必须判 MALFORMED");
    check(isMalformedRef("src/a.rs") === false, "正常路径不得误判 MALFORMED");
    check(isMalformedRef("a//b.rs") === true, "空路径段必须判 MALFORMED");

    // 棘轮正负控：基线内存量不红；新增一条 ⇒ 红
    const base = new Set(hard.map((r) => r.key));
    check(diffAgainstBaseline(base, hard).newViolations.length === 0, "基线内存量不得判红");
    const plusOne = [...hard, { key: "docs/guide.md::new_file.rs:1" }];
    check(diffAgainstBaseline(base, plusOne).newViolations.length === 1, "基线外新增必须判红");
    check(diffAgainstBaseline(new Set(), hard).newViolations.length === 3, "空基线（首轮）必须全部判红");
    check(diffAgainstBaseline(new Set(["gone"]), hard).fixedCount === 1, "基线内已消失应计入可收紧");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  if (fail > 0) return false;
  console.log(
    "SELFTEST OK：三档合法（完整 / 唯一后缀 / 唯一 basename）+ 后缀消歧 ⇒ OK；" +
      "段内截断 / 同义词换词 ⇒ ABBREV 且自证候选；无候选 ⇒ BROKEN；" +
      "单行与区间终点越界 ⇒ STALE；同名多份不可消歧 ⇒ NONLOC；跨仓 / 环境相关免检；棘轮正负控通过",
  );
  return true;
}

// ── main ────────────────────────────────────────────────────────────
if (SELFTEST) process.exit(runSelftest() ? 0 : 1);

const repoFiles = listRepoFiles(ROOT, SKIP_DIRS);
const index = indexFromFileList(repoFiles.files);
const docs = listDocFiles(ROOT);
if (docs.length === 0) {
  console.error("✖ 扫描面为空 —— docs/**/*.md 一篇都没收到（路径约定变了？），不是「没有引用」");
  process.exit(2);
}
const repoRel = (abs) => path.relative(ROOT, abs).split(path.sep).join("/");
const resolveRef = makeFileSetResolver(ROOT, repoFiles.files);
const { refs, counts } = auditDocs({ root: ROOT, docs, index, repoRel, resolve: resolveRef });
const hard = hardViolations(refs);

const BASELINE_NOTE =
  "棘轮基线：键 = <文档>::<引用串>:<起始行>[-<结束行>]（不含文档行号，防漂移）。" +
  "只许减少不许增加；新增 BROKEN/STALE 必须改文档引用，不许改基线洗白。";
function loadBaselineKeys() {
  if (!fs.existsSync(BASELINE_PATH)) return new Set();
  try {
    const b = JSON.parse(fs.readFileSync(BASELINE_PATH, "utf8"));
    return new Set(b.entries ?? []);
  } catch (e) {
    console.error(`✖ 基线不可读（${BASELINE_PATH}）：${e.message}`);
    process.exit(2);
  }
}
function writeBaseline(keys) {
  const countsNow = {};
  for (const r of refs) if (r.bucket === "BROKEN" || r.bucket === "STALE") countsNow[r.bucket] = (countsNow[r.bucket] ?? 0) + 1;
  fs.writeFileSync(
    BASELINE_PATH,
    JSON.stringify({ note: BASELINE_NOTE, version: 1, counts: countsNow, entries: [...keys].sort() }, null, 2) + "\n",
    "utf8",
  );
}

const summary =
  `扫描 ${docs.length} 篇文档 · 引用 ${refs.length} 条` +
  ` ｜ OK ${counts.OK ?? 0} ｜ NONLOC（未检查）${counts.NONLOC ?? 0} ｜ BROKEN ${counts.BROKEN ?? 0}` +
  ` ｜ ABBREV ${counts.ABBREV ?? 0} ｜ STALE ${counts.STALE ?? 0}` +
  ` ｜ EXTERNAL ${counts.EXTERNAL ?? 0} ｜ EPHEMERAL ${counts.EPHEMERAL ?? 0}` +
  ` ｜ MALFORMED ${counts.MALFORMED ?? 0}`;

if (DUMP) {
  for (const r of refs) {
    const at = `${r.doc}:${r.docLine}`;
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    let detail = "";
    if (r.bucket === "OK") detail = `→ ${r.target}`;
    else if (r.bucket === "STALE") detail = `→ ${r.target}（共 ${r.lines} 行，行号越界）`;
    else if (r.bucket === "ABBREV") detail = `→ 疑似${r.kind}（候选，需人判）：${r.candidates.join(" / ")}`;
    else if (r.bucket === "BROKEN") detail = "→ 无候选（仓内不存在该文件）";
    else if (r.bucket === "NONLOC") detail = `→ ${r.why}`;
    else if (r.bucket === "EXTERNAL") detail = "→ 跨仓 / 绝对路径（免检，只报告）";
    else if (r.bucket === "EPHEMERAL") detail = "→ 环境相关目录（免检，只报告）";
    else if (r.bucket === "MALFORMED") detail = "→ 不是路径形态（散文 / 表格残片，已丢弃）";
    console.log(`${at}  ${ref.padEnd(64)} ${r.bucket.padEnd(9)} ${detail}`);
  }
  console.log(`\n${summary}`);
  process.exit(0);
}

const baseKeys = loadBaselineKeys();
const { newViolations, fixedCount, baseSize } = diffAgainstBaseline(baseKeys, hard);

if (INIT_BASELINE) {
  if (fs.existsSync(BASELINE_PATH)) {
    console.error(`✖ --init-baseline 拒绝执行：${BASELINE_PATH} 已存在。收紧存量请用 --update-baseline，不要重建基线洗白。`);
    process.exit(1);
  }
  writeBaseline(new Set(hard.map((r) => r.key)));
  console.log(`\n${summary}`);
  console.log(`基线已建立：冻结 BROKEN/STALE 存量 ${hard.length} 条（之后只减不增）。`);
  process.exit(0);
}

if (UPDATE_BASELINE) {
  if (newViolations.length > 0) {
    console.error(`\n✖ --update-baseline 拒绝执行：当前有 ${newViolations.length} 条基线外新增，先修引用（不许用基线洗白）：`);
    for (const v of newViolations.slice(0, 20)) console.error(`  + ${v.key}（${v.doc}:${v.docLine}）`);
    process.exit(1);
  }
  writeBaseline(new Set(hard.map((r) => r.key)));
  console.log(`\n${summary}`);
  console.log(`基线已收紧：${baseSize} → ${hard.length} 条（本轮修复 ${fixedCount} 条）。`);
  process.exit(0);
}

console.log(`\n${summary}`);
console.log(`目标索引来源：${repoFiles.source === "git" ? "git ls-files（已跟踪 + 未忽略的新文件）" : "文件系统遍历（未找到 git，口径可能与 CI 不同）"}`);
if (baseSize > 0) console.log(`棘轮基线 ${baseSize} 条；本轮已消失 ${fixedCount} 条（可择机 --update-baseline 收紧）。`);

// ABBREV 逐条自证（报告面）：每条都必须点到它缩写的真实文件，找不到候选的不许进本桶
const abbrev = refs.filter((r) => r.bucket === "ABBREV");
if (abbrev.length > 0) {
  console.log(`\n~ ABBREV ${abbrev.length} 条（段内截断 / 同义词，**只报告**；按 AGENTS.md 改为段对齐写法即可消除）：`);
  for (const r of abbrev.slice(0, 50)) {
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    console.log(`   ~ ${r.doc}:${r.docLine} → ${ref}（疑似${r.kind}）：${r.candidates.join(" / ")}`);
  }
  if (abbrev.length > 50) console.log(`   … 其余 ${abbrev.length - 50} 条省略（--dump-doc-refs 看全量）`);
}
const nonloc = refs.filter((r) => r.bucket === "NONLOC");
if (nonloc.length > 0) {
  console.log(`\nℹ NONLOC ${nonloc.length} 条**未检查**（裸同名文件无法唯一定位；补上路径才进检查）：`);
  for (const r of nonloc.slice(0, 10)) console.log(`   ℹ ${r.doc}:${r.docLine} → ${r.refFile}:${r.start}`);
  if (nonloc.length > 10) console.log(`   … 其余 ${nonloc.length - 10} 条省略`);
}

if (newViolations.length > 0) {
  console.error(`\n✖ ${newViolations.length} 条基线外新增（BROKEN / STALE —— 文档引用的客观错，修法唯一：改引用）：`);
  for (const v of newViolations.slice(0, 40)) {
    const ref = v.end !== v.start ? `${v.refFile}:${v.start}-${v.end}` : `${v.refFile}:${v.start}`;
    const why = v.bucket === "STALE" ? `行号越界（目标共 ${v.lines} 行）` : "仓内不存在该文件（无截断/同义词候选）";
    console.error(`   ✖ ${v.doc}:${v.docLine} → ${ref} —— ${why}`);
  }
  if (newViolations.length > 40) console.error(`   … 其余 ${newViolations.length - 40} 条省略（--dump-doc-refs 看全量）`);
  console.error(
    `\n  · BROKEN：把引用改成**段对齐**写法（完整路径 / 唯一后缀 / 唯一 basename）；\n` +
      `  · STALE ：按目标文件当前行数订正行号（引用腐烂，编译与单测都看不见）。`,
  );
  process.exit(1);
}

console.log(`PASS: 无新增 BROKEN / STALE。`);
process.exit(0);
