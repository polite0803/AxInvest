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
//   · SYMDRIFT —— （2026-11-05 追加，任务 #182 ②）**独立的第二层计数，不改动上面任何一个桶**：
//                  上面各桶只回答「**指不指得到**」（文件存在 + 行号在范围内），
//                  不回答「**那一行是不是所述符号**」。实测反例：
//                    `docs/plans/PLAN-shell-plus-plugins.md:1936` 引 `src/types/index.ts:2123`
//                    并称那是 `PluginPagesInfoDto` —— 该声明实际在 `:2135`，`:2123` 落在
//                    `PluginPageAssetLocationDto` 体内 ⇒ 行号仍在文件行数内 ⇒ 既有判据报
//                    `PASS`（长期假绿）。判据与保守口径见下方「行内符号一致性」注释块。
//
// 退出码：0 = 无新增 BROKEN / STALE；1 = 有新增（或 selftest 失败）；
//         2 = 脚本自身失效（扫描面 0 篇文档 / 基线不可读）。
//   ⚠ ABBREV 首轮**只报告**（存量未知且量级可能很大）；判据未放宽 —— 它只是还没被提升为红。
//   ⚠ SYMDRIFT 同样**默认只报告**（存量已实测，见下），且**不进棘轮基线**（基线键面仍是
//     BROKEN/STALE，不改）；要让它退非 0 必须显式给 `--require-symbol-match`，
//     升与不升由对账批裁定 —— CI 与 `ci-check.mjs` 的命令行**都不带**该开关。
//
// ── 行内符号一致性（SYMDRIFT）的判据 ────────────────────────────────
// 五条，**逐条都是往「宁可漏报」的方向收敛**（散文里绝大多数反引号不是符号，
// 硬要比对就是结构性假阳性）：
//   ① 只检既有桶为 **OK** 的引用（STALE / BROKEN / NONLOC / … 已有自己的语义，不叠第二层红）；
//   ② 符号必须**可机械比对**：与引用**同一行**、列距离 ≤ `MAX_SYM_GAP` 的反引号簇里，
//      剥掉前导属性 `#[...]`、把 `::` 归一为 `.` 后**恰好只剩一个**标识符链
//      （`foo` / `Foo::bar` / `pub fn foo` / `list_x()`）；
//      含数字 / 含 `/` 或 `\` / 以文件扩展名结尾 / 剩下 ≥2 个标识符 / 含非 ASCII
//      ⇒ 判「句中无可比对符号」⇒ **不检也不判红**（最重要的一条护栏）；
//      ⚠ **点号链 `notify.channel` 一类默认也排除**（判据修正，2026-11-05 实测）：本仓的
//      能力登记表把这类**接缝名**（= `scripts/capability-register.json` 的键）放在表格首列、
//      `文件:行` 放在次列，它不是「该行有此符号」的断言；当成符号比对的实测后果是把
//      `docs/plans/PLAN-shell-plus-plugins.md` 等整表拖进红（占该档读数的主要部分）。
//      故默认走「接缝名 ⇒ 未检」，只计数自陈（`--sym-include-dotted` 可放回，供对账）；
//      `::` 路径形态（`Foo::bar`）**不受此影响**，仍按符号比对。
//   ③ 该符号必须**在目标文件里存在**（零存在 ⇒ 跳过：那是重命名或口语，属另一道题，
//      不是「行号漂了但没出界」）；
//   ④ 命中窗口 = `[start, end + WINDOW_BELOW]`，默认 **2 行**，理由：rustfmt / prettier 的
//      折行只会把名字推到紧邻的下一行（`impl Trait for Type` 断行、长泛型 `where`），
//      再往下进函数体了；**上方不留 0 行** —— `start-1` 通常是属性行或 doc 注释，
//      算它命中会让「符号只写在注释里」的引用白赚。窗口可用 `--sym-window=N` 现场放大复核；
//   ⑤ 多个候选符号里**任一命中即视为一致**（保守），全不命中才成一条 SYMDRIFT。
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
//   node scripts/check-doc-refs.mjs --require-symbol-match # 把 SYMDRIFT 提升为红（**默认不开**）
//   node scripts/check-doc-refs.mjs --sym-include-dotted   # 把点号接缝名也算符号（读数虚高，仅对账）
//   node scripts/check-doc-refs.mjs --sym-window=5         # 放大折行窗口复核（只影响 SYMDRIFT 读数）

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
// SYMDRIFT 的**唯一**升硬开关。默认关 ⇒ 既有退出码与桶计数逐字不变（CI 与 ci-check.mjs
// 的命令行都不带它，所以本地绿 / CI 绿口径不受本批影响）。
const REQUIRE_SYMBOL_MATCH = args.includes("--require-symbol-match");
// 把 `notify.channel` 这类**点号接缝名**也当符号比对（默认 **否**，理由见 `comparableSymbolSpan`）。
// 只是读数口径开关：开着它 SYMDRIFT 会虚高（本仓实测主要是能力登记表那一族），不进 CI 命令行。
const INCLUDE_DOTTED = args.includes("--sym-include-dotted");
// 折行窗口（下方行数）。`--sym-window=N` 只放大 SYMDRIFT 的**读数**，不改其它任何判据；
// 非法值（格式不对 / >50）⇒ 退 2 自陈，不静默按默认跑（判据 #7：未检查与通过长得一样）。
// 解析放在下方常量块之后（`WINDOW_BELOW_DEFAULT` 的 TDZ），真正取值在 main 里。
const WINDOW_FLAG = args.find((a) => a.startsWith("--sym-window"));

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
 * ⚠ 2026-11-05 起 `lineCache` 缓存的是**行数组**（`readLines`）而非行数 —— 符号一致性判据
 * 需要逐行内容。语义逐字不变：读不到（ENOENT）⇒ `null` ⇒ 调用方判 BROKEN。
 */
function lineCount(abs, cache) {
  const lines = readLines(abs, cache);
  return lines === null ? null : lines.length;
}

/**
 * 读目标文件正文并按行切（LRU，上限 `LINES_CACHE_MAX` 个文件 —— 不设上限等于把整仓读进内存）。
 * 负缓存（读不到的文件记 `null`）与 `lineCount` 原有一致，不重复抛。
 */
export function readLines(abs, cache) {
  if (cache.has(abs)) {
    const hit = cache.get(abs);
    cache.delete(abs); // 重新插入 ⇒ 移到队尾（LRU）
    cache.set(abs, hit);
    return hit;
  }
  let lines = null;
  try {
    const text = fs.readFileSync(abs, "utf8");
    lines = text.split(/\r?\n/);
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  } catch (e) {
    // 文件索引来自 `git ls-files --cached`：工作区里**已删除但尚未提交**的文件仍在索引中
    // （实测：opc 子树搬迁期间，文档仍指向旧路径 ⇒ readFileSync 抛 ENOENT 把整道门禁打崩）。
    // 盘上不存在 = 引用不可定位 ⇒ 与「仓内无此文件」同判 BROKEN，不能抛异常。
    if (e.code !== "ENOENT") throw e;
    lines = null;
  }
  cache.set(abs, lines);
  if (cache.size > LINES_CACHE_MAX) cache.delete(cache.keys().next().value);
  return lines;
}

// ── 行内符号一致性（SYMDRIFT）───────────────────────────────────────
// 判据全文见文件头「行内符号一致性」注释块。这里只放**纯函数**，便于 selftest 逐条命中。

/** 折行窗口下方行数（④）。取 2：rustfmt / prettier 只把名字推到紧邻下一行，再往下是函数体。 */
const WINDOW_BELOW_DEFAULT = 2;
/** ②：符号簇与引用 token 的最大列距离。表格行「引 | 符号」的间隔实测 5 列，24 列覆盖中文连接词。 */
const MAX_SYM_GAP = 24;
/** 缓存的目标文件数上限（LRU）。 */
const LINES_CACHE_MAX = 400;
/** 单个符号最多记多少个出现行（超出即截断，只影响「实际在第几行」的展示，不影响命中判定）。 */
const MAX_OCCURRENCES = 400;

/** 以文件扩展名结尾的簇（`index.ts` / `lib.rs`）是**文件名**不是符号。 */
const EXT_TAIL_RE = new RegExp(String.raw`^(?:.+\.)?(?:${DOC_REF_EXTS})$`, "i");

/**
 * 修饰词 / 关键字表：反引号簇里**只剩一个**非本表标识符时才算「给出了可比对符号」。
 * 用途见 `comparableSymbolSpan`（②）。表里没有 `new` / `default` 这类**可能真是符号名**的词。
 * ⚠ 后半段是 **prelude / 内置类型名**（`None` / `Option` / `Vec` / `Result` …）：
 *   它们在仓内几乎每个 `.rs` 里都「存在于别处」，判据③ 的零存在护栏拦不住，
 *   而散文里写 `` `None` `` 通常是在说**值**而不是说「该行定义了这个符号」。
 *   实测（2026-11-05，1046 条读数里）这一族贡献 28 条（`None` 16 / `Result` 4 / `Option` 3 /
 *   `Err` 2 / `Some` 1 / `Vec` 1 / `Arc` 1），故一并排除。
 */
const CODE_KEYWORDS = new Set(
  `pub crate async unsafe extern let var val const static fn func function class interface type enum
   struct trait impl mod macro record union abstract this self Self return if else for while loop match
   mut ref use where as dyn in of get set true false null undefined nil void string number boolean
   export import declare from throw try catch finally package private protected override virtual
   await yield typeof instanceof public internal
   none some ok err vec option result boxed box arc rc cell mutex rwlock hashmap hashset btreemap
   set list map dict tuple unit char float int uint isize usize i8 i16 i32 i64 u8 u16 u32 u64 f32 f64`
    .split(/\s+/)
    .filter(Boolean),
);

/** 反引号簇 `` `...` ``（不跨行）。返回内容与其**列区间**（不含两侧反引号）。 */
export function backtickSpans(line) {
  const out = [];
  for (const m of line.matchAll(/`([^`\r\n]+)`/g)) {
    out.push({ text: m[1], col: m.index + 1, endCol: m.index + 1 + m[1].length });
  }
  return out;
}

/**
 * 纯函数（判据②）：一个反引号簇是否是「可机械比对的符号」，并给出它的**形态**。
 *
 * @returns {{form:string, syms:string[]}}
 *   `form`：`ident`（裸标识符）｜ `decl`（`pub fn foo` 这类带关键字的声明形态）｜
 *   `path`（`Foo::bar` 路径）｜ `dotted`（`notify.channel` 点号链）｜ `none`（不是符号）
 *   `syms`：比对用的标识符（**末段在前**，路径首段作兜底）。
 *
 * ⚠ `dotted` 形态在本仓**不是「该行有这个符号」的断言**（实测 2026-11-05）：
 *   `docs/plans/PLAN-shell-plus-plugins.md` 的能力登记表把 `notify.channel` / `workflow.reflector` /
 *   `platform.adapter` / `session.store` 这类**接缝名**放在表格首列，`文件:行` 在次列 ——
 *   这些点号串是 `scripts/capability-register.json` 里的**登记表键**（逐条可查），
 *   不是目标文件里的符号。把它们当符号比对 ⇒ 整表成批假阳性。
 *   故默认（判据②）**排除 `dotted`**，只把 `::` 路径形态当作路径符号；
 *   需要把它们也计入时用 `--sym-include-dotted`（读数会虚高，仅供对账）。
 */
export function comparableSymbolSpan(spanText) {
  const none = { form: "none", syms: [] };
  const raw = spanText.trim();
  if (raw.length < 2 || raw.length > 80) return none;
  if (!/^[\x20-\x7e]+$/.test(raw)) return none; // 含中文 / 全角 ⇒ 散文
  if (/[\\/]/.test(raw)) return none; // 含路径分隔 ⇒ 是文件
  if (/\d/.test(raw)) return none; // 含数字 ⇒ 行号 / 版本 / 序号，不作符号比对（保守）
  const stripped = raw.replace(/^#\[[^\]]*\]\s*/, "");
  const s = stripped.replace(/::/g, "."); // 路径归一为点，便于整条链当一个符号
  const chains = s.match(/[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*/g) ?? [];
  const kept = chains.filter((c) => {
    const segs = c.split(".");
    const last = segs[segs.length - 1];
    if (CODE_KEYWORDS.has(last.toLowerCase())) return false;
    return !EXT_TAIL_RE.test(c); // `mod.rs` 这种「点号链」其实是文件名
  });
  if (kept.length !== 1) return none; // 0 = 散文；≥2 = 归属不明 ⇒ 不猜
  const chain = kept[0].split(".");
  const primary = chain[chain.length - 1];
  if (primary.length < 3) return none; // `id` / `x` 这类短名撞车率太高
  let form;
  if (chain.length > 1) form = raw.includes("::") ? "path" : "dotted";
  else form = /\s/.test(stripped.trim()) ? "decl" : "ident";
  const first = chain[0];
  const syms = first !== primary ? [primary, first] : [primary];
  return { form, syms };
}

/** 兼容包装：只要符号列表（selftest 与调用方按数组用）。 */
export function comparableSymbols(spanText) {
  return comparableSymbolSpan(spanText).syms;
}

/**
 * 纯函数（判据②的归属）：给**同一行**里的每条引用 token 归因「它自己那簇反引号里给出的符号」。
 * 规则：跳过与任何引用列区间重叠的簇（那就是引用本体 / 同行另一条引用）；其余簇归给**列距离最近**
 * 且 ≤ `MAX_SYM_GAP` 的那条引用；超出距离 ⇒ 归属不明 ⇒ 谁都不给（宁可漏报）。
 * 就地写 `r.symCandidates`（数组，每条 = `{ text, form, syms }`：原簇文本 / 形态 / 比对标识符）。
 */
export function attributeSymbolsToLine(refsOnLine) {
  if (refsOnLine.length === 0) return;
  const line = refsOnLine[0].docText ?? "";
  for (const r of refsOnLine) r.symCandidates = [];
  for (const sp of backtickSpans(line)) {
    let owner = null;
    let best = Infinity;
    for (const r of refsOnLine) {
      if (r.col === undefined) continue;
      if (sp.col < r.colEnd && r.col < sp.endCol) continue; // 与引用本体重叠 ⇒ 不是符号簇
      const gap = sp.endCol <= r.col ? r.col - sp.endCol : sp.col - r.colEnd;
      if (gap <= MAX_SYM_GAP && gap < best) {
        best = gap;
        owner = r;
      }
    }
    if (!owner) continue;
    const { form, syms } = comparableSymbolSpan(sp.text);
    if (syms.length > 0) owner.symCandidates.push({ text: sp.text, form, syms });
  }
}

/** 词边界匹配（`_`/字母/数字之外才算边界）；同一符号只编译一次。 */
const wordReCache = new Map();
function wordRegex(tok) {
  if (!wordReCache.has(tok)) {
    wordReCache.set(tok, new RegExp(`(?<![A-Za-z0-9_])${tok.replace(/[$^.*+?()|{}[\]\\]/g, "\\$&")}(?![A-Za-z0-9_])`));
  }
  return wordReCache.get(tok);
}

/** 符号在目标文件里的出现行（1-based，上限 `MAX_OCCURRENCES`）；按 (文件, 符号) 缓存。 */
function occurrenceLines(lines, abs, tok, occCache) {
  const k = `${abs}\u0000${tok}`;
  if (occCache.has(k)) return occCache.get(k);
  const re = wordRegex(tok);
  const out = [];
  for (let i = 0; i < lines.length && out.length < MAX_OCCURRENCES; i++) {
    if (re.test(lines[i])) out.push(i + 1);
  }
  occCache.set(k, out);
  return out;
}

/**
 * 第二层判据（SYMDRIFT）。就地给命中的引用写 `r.symDrift = { symbols, window, actual, forms }`。
 * @param {{refs:object[], windowBelow?:number, lineCache?:Map<string, string[]|null>,
 *          includeDotted?:boolean}} opts
 * @returns {{drift:object[], withSymbol:number, checked:number, absent:number, dottedSkipped:number,
 *            byForm:Record<string,number>, windowBelow:number}}
 *   · `withSymbol`     —— 句中给出可比对符号的引用条数（判据②，**不含**被排除的点号接缝名）
 *   · `checked`        —— 其中符号确实存在于目标文件、真正进入行窗比对的条数（判据③）
 *   · `absent`         —— 符号在目标文件里零出现 ⇒ **跳过**（不是「通过」，是「未检」）
 *   · `dottedSkipped`  —— 只给出 `notify.channel` 这类点号接缝名的引用条数 ⇒ 判据② 排除（未检）
 *   · `byForm`         —— 漂移按形态计数（`ident` / `decl` / `path` / `dotted`），供对账升硬时取舍
 */
export function auditSymbolDrift({
  refs,
  windowBelow = WINDOW_BELOW_DEFAULT,
  lineCache = new Map(),
  includeDotted = false,
}) {
  const occCache = new Map();
  attributeAllLines(refs);
  const byTarget = new Map();
  const dottedOnly = [];
  for (const r of refs) {
    if (r.bucket !== "OK" || !r.abs) continue; // classifyRef 只给可定位的引用写 abs
    if (!r.symCandidates || r.symCandidates.length === 0) continue;
    const usable = r.symCandidates.filter((c) => includeDotted || c.form !== "dotted");
    if (usable.length === 0) {
      r.symDottedOnly = true; // 只给了 `notify.channel` 这类接缝名 ⇒ 未检，不是通过
      dottedOnly.push(r);
      continue;
    }
    if (!byTarget.has(r.abs)) byTarget.set(r.abs, []);
    byTarget.get(r.abs).push({ r, usable });
  }
  const drift = [];
  const byForm = {};
  let withSymbol = 0;
  let checked = 0;
  let absent = 0;
  for (const [abs, list] of byTarget) {
    const lines = readLines(abs, lineCache);
    if (lines === null) continue; // 盘上已删 ⇒ 既有桶已判 BROKEN，不在此叠第二层
    for (const { r, usable } of list) {
      withSymbol++;
      const toks = [...new Set(usable.flatMap((c) => c.syms))];
      const occ = new Map(toks.map((t) => [t, occurrenceLines(lines, abs, t, occCache)]));
      const present = toks.filter((t) => occ.get(t).length > 0);
      if (present.length === 0) {
        absent++;
        r.symAbsent = true;
        continue; // 判据③：文件里根本没这个符号 ⇒ 不是行号漂移，另属「重命名/口语」那道题
      }
      checked++;
      const lo = Math.max(1, r.start);
      const hi = Math.min(lines.length, Math.max(r.end, r.start) + windowBelow);
      // 判据⑤：任一候选命中即视为一致（保守）
      if (present.some((t) => occ.get(t).some((l) => l >= lo && l <= hi))) continue;
      const forms = [...new Set(usable.map((c) => c.form))];
      // 按该条**第一个候选簇**的形态计数（一条只计一次；一行常同时给 ident + path 两簇，
      // 逐形态各计会让 ΣbyForm > drift.length，读数就对不上账了）。
      byForm[forms[0]] = (byForm[forms[0]] ?? 0) + 1;
      const actual = occ
        .get(present[0])
        .slice()
        .sort((a, b) => Math.abs(a - r.start) - Math.abs(b - r.start))
        .slice(0, 3);
      r.symDrift = { symbols: present, window: [lo, hi], actual, forms, spans: usable.map((c) => c.text) };
      drift.push(r);
    }
  }
  return { drift, withSymbol, checked, absent, dottedSkipped: dottedOnly.length, byForm, windowBelow };
}

/** 按（文档, 文档行）分组后逐行做符号归属（`attributeSymbolsToLine` 需要同行上下文）。 */
function attributeAllLines(refs) {
  const byLine = new Map();
  for (const r of refs) {
    if (r.srcLine === undefined) continue;
    const k = `${r.doc}\u0000${r.srcLine}`;
    if (!byLine.has(k)) byLine.set(k, []);
    byLine.get(k).push(r);
  }
  for (const list of byLine.values()) attributeSymbolsToLine(list);
}

/** 纯函数：SYMDRIFT 是否进退出码。**默认关** ⇒ 恒空数组 ⇒ 退码与改前逐字相同。 */
export function symbolHardViolations(drift, requireSymbolMatch) {
  return requireSymbolMatch ? drift : [];
}

/** 纯函数：`--sym-window=N` 的解析（非法 ⇒ null ⇒ main 退 2 自陈）。 */
export function parseSymWindowFlag(flag) {
  if (flag === undefined) return WINDOW_BELOW_DEFAULT;
  const m = flag.match(/^--sym-window=(\d+)$/);
  if (!m || Number(m[1]) > 50) return null;
  return Number(m[1]);
}

/**
 * 单条引用判定（纯函数：文件系统只经 `lineCount` / `index` 访问）。
 * @returns {{bucket:string, target?:string, abs?:string, candidates?:string[], kind?:string, lines?:number}}
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
    // `abs` 只给第二层（SYMDRIFT）判据定位目标文件用；**不参与**桶判定、键、输出行 ⇒ 既有读数不变
    abs: loc.abs,
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
 * @returns {{refs:object[], counts:Record<string,number>, docs:number, lineCache:Map<string,string[]|null>}}
 *   ⚠ `counts` 的**桶口径不含 SYMDRIFT** —— 第二层判据（`auditSymbolDrift`）只往引用对象上挂
 *   `symDrift` / `symAbsent` 字段，**不改 `bucket`**，所以既有各桶计数与退出码逐字不变。
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
    const lineTexts = new Map();
    for (const r of extractRefs(text, DOC_REF_EXTS)) {
      const k = `${r.srcLine}:${r.col}`;
      if (!toks.has(k)) {
        // `col` / `colEnd` = 引用 token 在本行里的列区间（第二层判据要靠它做符号归属）；
        // `docText` 只截前 2000 字符，够覆盖一行散文，也避免把整篇文档留在内存里。
        toks.set(k, {
          file: expandRefFile(r),
          srcLine: r.srcLine,
          start: r.line,
          end: r.line,
          col: r.col,
          colEnd: r.end,
          text: (r.text ?? "").slice(0, 2000),
        });
      }
      const t = toks.get(k);
      t.start = Math.min(t.start, r.line);
      t.end = Math.max(t.end, r.line);
      lineTexts.set(r.srcLine, t.text);
    }
    for (const t of [...toks.values()].sort((a, b) => a.srcLine - b.srcLine || a.start - b.start)) {
      const refFile = t.file;
      const key = `${docRel}::${refFile}:${t.start}${t.end !== t.start ? `-${t.end}` : ""}`;
      if (seen.has(key)) continue; // 同一文档内重复引用 ⇒ 只判一次（输出不被淹）。
      // ⚠ 被丢掉的重复 token 也就不参与第二层的「同行符号归属」上下文 —— 它判定与幸存那条完全相同，
      //   归属只会落到同文件的同一行号上，不影响结论。
      seen.add(key);
      const verdict = classifyRef({ refFile, start: t.start, end: t.end }, ctx);
      refs.push({
        key,
        doc: docRel,
        docLine: t.srcLine + 1,
        srcLine: t.srcLine,
        refFile,
        start: t.start,
        end: t.end,
        col: t.col,
        colEnd: t.colEnd,
        docText: lineTexts.get(t.srcLine) ?? "",
        ...verdict,
      });
    }
  }
  const counts = {};
  for (const r of refs) counts[r.bucket] = (counts[r.bucket] ?? 0) + 1;
  // `lineCache` 一并返回：第二层（SYMDRIFT）复用同一份正文缓存，避免 SMB 上把目标文件读两遍。
  return { refs, counts, docs: docs.length, lineCache };
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

// ── selftest（第二层）：行内符号一致性 SYMDRIFT ────────────────────
// 独立的临时目录，**不碰上面那套桶判据的靶面** ⇒ 既有用例的每个期望值都原样不动。
function runSymbolSelftest() {
  let fail = 0;
  const check = (cond, msg, extra) => {
    if (!cond) {
      console.error(`SELFTEST FAIL：${msg}${extra === undefined ? "" : `（实际 ${JSON.stringify(extra)}）`}`);
      fail++;
    }
  };
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "doc-refs-symtest-"));
  try {
    const write = (rel, content) => {
      const abs = path.join(tmp, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content, "utf8");
    };
    // rust 靶（行号逐字可数）
    write(
      "src/engine.rs",
      [
        "// 头部注释", // 1
        "pub struct EngineState {", // 2
        "    pub tick: u64,", // 3
        "}", // 4
        "", // 5
        "impl EngineState {", // 6
        "    pub fn snapshot_all(&self) -> u64 {", // 7
        "        self.tick", // 8
        "    }", // 9
        "}", // 10
        "", // 11
        "pub fn resolve_gap_width(a: u64, b: u64) -> u64 {", // 12
        "    a.max(b)", // 13
        "}", // 14
        "", // 15
        "impl NamedThing", // 16
        "    for EngineState", // 17  ← rustfmt 折行：名字在引用行的**下一行**
        "{", // 18
        "    fn wide(x: u64) -> u64 { x }", // 19
        "}", // 20
        "",
      ].join("\n"),
    );
    // ts 靶（复刻 leader 实测的 `src/types/index.ts` 表格行形态）
    write(
      "src/pages.ts",
      [
        "/**", // 1
        " * 返回形状。", // 2
        " */", // 3
        "export interface PluginPagesInfoDto {", // 4
        "  pages: PluginPageDto[];", // 5
        "  tick: number;", // 6
        "}", // 7
        "",
      ].join("\n"),
    );
    write(
      "docs/syms.md",
      [
        "见 `src/engine.rs:12` 的 `resolve_gap_width`（正控：符号确在所指行）", // 1
        "见 `src/engine.rs:7` 的 `resolve_gap_width`（负控：漂到同文件另一处 ⇒ 须点名 12）", // 2 ⇒ 漂移
        "见 `src/engine.rs:16` 的 `EngineState`（折行窗口内命中 17 行 ⇒ 不报）", // 3
        "见 `src/engine.rs:13` 的 `EngineState`（出窗 ⇒ 报，实际最近 17）", // 4 ⇒ 漂移
        "见 `src/engine.rs:8` 的 `EngineState::tick`（rust 路径形态：末段 tick 命中）", // 5
        "见 `src/engine.rs:2` 的 `rename_all = camelCase`（一簇里剩 2 个标识符 ⇒ 归属不明，不检）", // 6
        "见 `src/engine.rs:3` 的 `TotallyAbsentIface`（符号在目标文件零出现 ⇒ 跳过，不是漂移）", // 7
        "见 `src/engine.rs:5`（句中没有可机械比对符号 ⇒ 不检也不许退红）", // 8
        "| `src/pages.ts:4` | `PluginPagesInfoDto`（ts 正控） |", // 9
        "| `src/pages.ts:7` | `PluginPagesInfoDto`，`pages` 复用 `PluginPageDto` |（ts 负控）", // 10 ⇒ 漂移
        "见 `src/pages.ts:6` 的 `engine.tick`（点号接缝名形态 ⇒ 判据② 默认排除，不检）", // 11
        "见 `src/engine.rs:2-4` 的 `EngineState`（区间引用：窗口覆盖全区间）", // 12
        "见 `src/engine.rs:19-20` 的 `EngineState`（区间内确实没有 ⇒ 报，实际最近 17）", // 13 ⇒ 漂移
        "见 `src/engine.rs:9` 与一段中文填充文字，跨度超过二十四个字符才出现的符号 `resolve_gap_width` 不该被归属", // 14
        "| `src/pages.ts:2` | 接缝 `pages.tick`（同为点号形态；`--sym-include-dotted` 时才报） |", // 15
        "",
      ].join("\n"),
    );

    const index = buildBasenameIndex(tmp, SKIP_DIRS);
    const repoRel = (abs) => path.relative(tmp, abs).split(path.sep).join("/");
    const symDocs = [path.join(tmp, "docs", "syms.md")];
    const { refs, counts, lineCache } = auditDocs({ root: tmp, docs: symDocs, index, repoRel });
    const sym = auditSymbolDrift({ refs, windowBelow: 2, lineCache });

    // 前置：全部引用必须仍是 OK（第二层**不许**改桶），既有判红面仍是 0 ⇒ 默认退码不变
    check(counts.OK === 15, "靶面：15 条引用都应仍判 OK（SYMDRIFT 不进桶）", counts);
    check(hardViolations(refs).length === 0, "第二层判据不得改动桶：既有判红面必须仍为 0");
    check(symRefsAllOk(refs), "SYMDRIFT 条目自身 bucket 仍为 OK（独立计数，不与 BROKEN/STALE 混桶）");

    // 抽样面计数：可比对符号 / 进入行窗比对 / 零出现跳过 / 点号接缝名排除
    check(sym.withSymbol === 10, "判据②：10 条引用给出可比对符号（多标识符 / 无符号 / 超距离 / 点号接缝名 共 5 条不检）", sym.withSymbol);
    check(sym.absent === 1, "判据③：符号零出现的那条应记为「跳过（未检）」而非漂移", sym.absent);
    check(sym.checked === 9, "真正进入行窗比对的应恰为 9 条", sym.checked);
    check(sym.dottedSkipped === 2, "点号接缝名形态（`engine.tick` / `pages.tick`）必须被排除并**计数自陈**", sym.dottedSkipped);

    // 负控：行号漂到同文件另一处 ⇒ 报进新桶，且点名实际行
    check(sym.drift.length === 4, "漂移应恰为 4 条（rs 两条 + ts 表格行一条 + 区间一条）", sym.drift.length);
    check(sym.byForm.ident === 4 && Object.keys(sym.byForm).length === 1, "漂移按形态计数：4 条全是裸标识符", sym.byForm);
    const d2 = sym.drift.find((r) => r.start === 7);
    check(!!d2 && d2.symDrift.actual[0] === 12, "负控：漂移到同文件另一处必须点名实际行 12", d2?.symDrift?.actual);
    const d4 = sym.drift.find((r) => r.start === 13);
    check(!!d4 && d4.symDrift.actual[0] === 17, "负控：出窗样本必须点名最近的真实出现行 17", d4?.symDrift?.actual);
    const d10 = sym.drift.find((r) => r.docLine === 10);
    check(!!d10 && d10.symDrift.symbols.includes("PluginPagesInfoDto"), "ts 表格行负控必须被抓到（leader 实测形态）");
    check(!!d10 && d10.symDrift.actual[0] === 4, "ts 负控：实际声明行应点名 4", d10?.symDrift?.actual);

    // 正控：三类「不许报」
    for (const [at, why] of [
      [1, "符号确在所指行"],
      [3, "折行窗口内命中（下一行）"],
      [5, "rust 路径形态（::）取末段命中"],
      [9, "ts 正控"],
      [12, "区间引用覆盖全区间"],
      [14, "符号离引用超 24 列 ⇒ 不归属"],
    ]) {
      check(!sym.drift.some((r) => r.docLine === at), `正控（第 ${at} 行，${why}）不得报漂移`);
    }
    // 最重要的一条：句中无可比对符号 ⇒ 不检、也不许退红
    check(!sym.drift.some((r) => r.docLine === 8), "句中无可比对符号 ⇒ 不检（这条若误报，整篇散文都会红）");
    check(!sym.drift.some((r) => r.docLine === 6), "一簇里剩 ≥2 个标识符 ⇒ 归属不明，不检");
    check(!sym.drift.some((r) => r.docLine === 7), "符号在目标文件零出现 ⇒ 判「跳过」不判漂移");
    check(!sym.drift.some((r) => r.docLine === 11), "点号接缝名形态默认排除 ⇒ 不检（本仓实测主要是它，误检会整表假阳性）");

    // 读数口径开关：`--sym-include-dotted` 只把点号形态放回来（第 15 行由此转红，第 11 行仍命中）
    const wide = auditSymbolDrift({ refs: [...refs], windowBelow: 2, lineCache, includeDotted: true });
    check(wide.dottedSkipped === 0, "打开 --sym-include-dotted 后不应再有「接缝名排除」");
    check(wide.withSymbol === 12, "打开后给出符号的引用应是 12 条（多出第 11、15 两行）", wide.withSymbol);
    check(
      wide.drift.length === 5 && wide.drift.some((r) => r.docLine === 15),
      "打开后只应多报第 15 行那条（第 11 行的 tick 确在所指行 ⇒ 仍不报）",
      wide.drift.map((r) => r.docLine),
    );
    check(wide.byForm.dotted === 1, "新增那条必须记在 dotted 形态下（自陈它是接缝名而非符号）", wide.byForm);

    // 窗口收紧只多报不少报（折行窗口取值 2 的反证：0 行时第 3 条正控会变红）
    check(
      auditSymbolDrift({ refs: [...refs], windowBelow: 0, lineCache }).drift.length === 5,
      "窗口取 0 时只应多报那 1 条折行样本（⇒ 默认取 2 是在往漏报方向收敛）",
    );

    // 升硬开关：关 ⇒ 违规恒空（退码与改前逐字相同）；开 ⇒ 全部成为违规
    check(symbolHardViolations(sym.drift, false).length === 0, "--require-symbol-match 关闭时 SYMDRIFT 不进退出码");
    check(symbolHardViolations(sym.drift, true).length === 4, "--require-symbol-match 打开时 SYMDRIFT 全部判红（⇒ 退非 0）");
    check(parseSymWindowFlag(undefined) === 2, "缺省窗口 = 2");
    check(parseSymWindowFlag("--sym-window=5") === 5, "--sym-window=5 应解析成 5");
    check(parseSymWindowFlag("--sym-window=x") === null, "非法窗口值必须返回 null（main 据此退 2）");
    check(parseSymWindowFlag("--sym-window=99") === null, "超出 0..50 的窗口值必须返回 null");

    // 符号形态单测（判据②的护栏逐条）
    check(comparableSymbols("PluginPagesInfoDto").join() === "PluginPagesInfoDto", "单个 PascalCase 标识符 = 可比对符号");
    check(comparableSymbols("pub fn resolve_gap_width").join() === "resolve_gap_width", "声明形态取名字");
    check(comparableSymbols("EngineState::tick")[0] === "tick", "rust 路径取末段");
    check(comparableSymbols("engine.tick")[0] === "tick", "点号链取末段（形态由 comparableSymbolSpan 判）");
    check(comparableSymbols("list_plugin_pages()")[0] === "list_plugin_pages", "带括号调用形态");
    check(comparableSymbols("src/types/index.ts:2123").length === 0, "引用本体（含路径与数字）不是符号");
    check(comparableSymbols("mod.rs").length === 0, "文件名（以扩展名结尾）不是符号");
    check(comparableSymbols("rename_all = camelCase").length === 0, "剩 2 个标识符 ⇒ 归属不明");
    check(comparableSymbols("无 新增").length === 0, "含非 ASCII ⇒ 散文");
    check(comparableSymbols("v2").length === 0, "短于 3 字符的末段不比对（撞车率太高）");
    check(comparableSymbols("impl EngineState").join() === "EngineState", "impl 形态剥掉关键字");
    check(comparableSymbolSpan("notify.channel").form === "dotted", "点号链 = 接缝名形态（默认排除的依据）");
    check(comparableSymbolSpan("SessionManager::new").form === "path", ":: 路径形态仍按符号比对");
    check(comparableSymbolSpan("pub fn foo").form === "decl", "带关键字的声明形态");
    check(comparableSymbolSpan("PluginPagesInfoDto").form === "ident", "裸标识符形态");
    check(comparableSymbolSpan("见 foo 说明").form === "none", "散文形态 ⇒ none");
    check(comparableSymbolSpan("Foo::Bar::baz").syms.join() === "baz,Foo", "多级路径：末段在前 + 首段兜底");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }

  if (fail > 0) return false;
  console.log(
    "SELFTEST OK（第二层 SYMDRIFT）：符号在行 ⇒ 不报；漂到同文件另一处 ⇒ 报并点名实际行；" +
      "折行窗口内命中 ⇒ 不报（窗口取 0 时该条转红，故 2 是漏报方向收敛）；" +
      "句中无符号 / 多标识符 / 超距离 / 符号零出现 / 点号接缝名 ⇒ 不检也不退红；rs 与 ts 两形态各覆盖；" +
      "--sym-include-dotted 只放回接缝名读数；开关关闭 ⇒ 违规恒空（既有退码不变），打开 ⇒ 全部判红；" +
      "桶计数仍为 OK（不与 BROKEN/STALE 混桶）",
  );
  return true;
}

/** 纯函数：SYMDRIFT 条目是否**没有**污染既有桶（第二层的语义护栏）。 */
export function symRefsAllOk(refs) {
  return refs.every((r) => !r.symDrift || r.bucket === "OK");
}

// ── main ────────────────────────────────────────────────────────────
if (SELFTEST) process.exit(runSelftest() && runSymbolSelftest() ? 0 : 1);

const SYM_WINDOW = parseSymWindowFlag(WINDOW_FLAG);
if (SYM_WINDOW === null) {
  console.error(`✖ 参数非法：${JSON.stringify(WINDOW_FLAG)} —— 只接受 --sym-window=<0..50>（第二层判据的折行窗口）`);
  process.exit(2);
}

const repoFiles = listRepoFiles(ROOT, SKIP_DIRS);
const index = indexFromFileList(repoFiles.files);
const docs = listDocFiles(ROOT);
if (docs.length === 0) {
  console.error("✖ 扫描面为空 —— docs/**/*.md 一篇都没收到（路径约定变了？），不是「没有引用」");
  process.exit(2);
}
const repoRel = (abs) => path.relative(ROOT, abs).split(path.sep).join("/");
const resolveRef = makeFileSetResolver(ROOT, repoFiles.files);
const { refs, counts, lineCache } = auditDocs({ root: ROOT, docs, index, repoRel, resolve: resolveRef });
const hard = hardViolations(refs);
// 第二层（SYMDRIFT）：复用同一份正文缓存；**不改桶、不改键**，只往引用对象上挂 symDrift/symAbsent。
const sym = auditSymbolDrift({ refs, windowBelow: SYM_WINDOW, lineCache, includeDotted: INCLUDE_DOTTED });
const symHard = symbolHardViolations(sym.drift, REQUIRE_SYMBOL_MATCH);

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
  ` ｜ MALFORMED ${counts.MALFORMED ?? 0}` +
  // 第二层是**并列独立计数**（上面各桶的读数与它无关）：给出可比对符号的引用里，
  // 符号不在所指行（±窗口）的条数。窗口 0 行时上面这些数字不变，只有这个数会变。
  ` ｜ SYMDRIFT ${sym.drift.length}（第二层·符号不在所指行${REQUIRE_SYMBOL_MATCH ? "**判红**" : "只报告"}；` +
  `可比对符号 ${sym.withSymbol} 条 / 进入比对 ${sym.checked} 条 / 符号零存在跳过 ${sym.absent} 条 /` +
  ` 点号接缝名排除 ${sym.dottedSkipped} 条 / 窗口 +${sym.windowBelow} 行` +
  `；漂移按形态 ${Object.entries(sym.byForm).map(([k, v]) => `${k} ${v}`).join(" / ") || "无"}）`;

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
    // 第二层附注（不改上面任何桶的读数）：符号一致 / 符号漂移 / 符号零存在（未检）/ 仅接缝名（未检）
    if (r.symDrift) {
      detail +=
        ` ⇒ SYMDRIFT 簇 \`${r.symDrift.spans.join("` / `")}\`（形态 ${r.symDrift.forms.join("/")}）` +
        `的符号 ${r.symDrift.symbols.join("/")} 不在 ${r.symDrift.window.join("-")} 行` +
        `（实际 ${r.symDrift.actual.join("/")}`;
      detail += r.symDrift.symbols.length > 1 ? `；候选共 ${r.symDrift.symbols.length} 个）` : "）";
    } else if (r.symAbsent) detail += ` ⇒ 符号零存在（未检，属重命名/口语那道题）`;
    else if (r.symDottedOnly) detail += ` ⇒ 仅点号接缝名（未检，见 comparableSymbolSpan 注释）`;
    else if (r.symCandidates && r.symCandidates.length > 0 && r.bucket === "OK") detail += ` ⇒ 符号一致`;
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

// ── 第二层（SYMDRIFT）报告面 ───────────────────────────────────────
// 语义与 ABBREV 同档：**默认只报告**，不影响退出码。`--require-symbol-match` 才判红。
// 逐条必须点到「符号实际在第几行」，否则这条判据指不回真实风险（同 ABBREV 的自证要求）。
if (sym.drift.length > 0) {
  console.log(
    `\n~ SYMDRIFT ${sym.drift.length} 条（引用可定位、但**所指那一行不是句中所述符号**，行号仍在范围内 ⇒` +
      `既有 BROKEN/STALE 全绿的那种腐烂。${REQUIRE_SYMBOL_MATCH ? "**已判红**" : "默认只报告，加 --require-symbol-match 才退非 0"}）：`,
  );
  for (const r of sym.drift.slice(0, 30)) {
    const ref = r.end !== r.start ? `${r.refFile}:${r.start}-${r.end}` : `${r.refFile}:${r.start}`;
    console.log(
      `   ~ ${r.doc}:${r.docLine} → ${ref} 簇 \`${r.symDrift.spans.join("` / `")}\`` +
        `（形态 ${r.symDrift.forms.join("/")}）的符号 \`${r.symDrift.symbols.join("` / `")}\`` +
        ` 不在 ${r.symDrift.window[0]}..${r.symDrift.window[1]} 行（实际 ${r.symDrift.actual.join(" / ")}）`,
    );
  }
  if (sym.drift.length > 30) console.log(`   … 其余 ${sym.drift.length - 30} 条省略（--dump-doc-refs 看全量）`);
  console.log(
    `   修法：把行号订正到「实际」那一行（或把符号名改成所指行上的真符号）。\n` +
      `   复核窗口：--sym-window=N 放大折行窗口，可分辨「折行没吃进」与「真漂移」。`,
  );
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

// 升硬开关：**只有显式给了 --require-symbol-match 才会走到这里退 1**（symHard 关时恒空）。
// ⚠ 不进棘轮基线 —— 基线键面仍是 BROKEN/STALE（`writeBaseline` 未改），升硬前需另立符号漂移基线。
if (symHard.length > 0) {
  console.error(
    `\n✖ --require-symbol-match：SYMDRIFT ${symHard.length} 条判红（引用可定位但所指行不是所述符号）。` +
      `上面已逐条点名实际行；不带该开关时本桶只报告、退码与既往逐字相同。`,
  );
  process.exit(1);
}

console.log(`PASS: 无新增 BROKEN / STALE。`);
process.exit(0);
