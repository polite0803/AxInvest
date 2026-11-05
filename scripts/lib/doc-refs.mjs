// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 「文件:行号」引用的公共解析原语 —— **唯一实现**，供两道门禁共用：
 *   · `scripts/check-single-source-facts.mjs`（`.rs` 注释 / rustdoc 里的出处引用）
 *   · `scripts/check-doc-refs.mjs`（`docs/**\/*.md` 里的引用，见 AGENTS.md「文档中的源码引用写法」）
 *
 * 为什么抽成 lib 而不是各写一份（判据：AGENTS.md 禁区 12「禁止重复定义」）：
 *   两处判据必须**逐字同源**，否则同一份行号漂移在两个扫描面上会给出不同结论 ——
 *   那是判据口径分歧，比不检查更糟。抽取前它们在同一份脚本里（单源），抽取后仍单源。
 *
 * ⚠ 为什么不能直接 `import` `check-single-source-facts.mjs` 复用：
 *   那个脚本是**顶层可执行过程**（无 main 守卫），被 import 时会整段跑一遍扫描并
 *   `process.exit` ⇒ 无法作为库使用。故把纯函数下沉到这里，那边改为**转调**。
 *
 * 本文件只放**无副作用**的纯函数 + 常量；不读 `process.argv`、不写盘、不 exit。
 */
import fs from "node:fs";
import path from "node:path";
import { repoRel, resolveRepoFile } from "./workspace-roots.mjs";

/**
 * 引用串里认作「文件」的扩展名白名单。
 * 默认值 = `check-single-source-facts.mjs` 的历史口径（改动会改变 `.rs` 面读数，勿轻改）；
 * 文档面可传更宽的白名单（见 `check-doc-refs.mjs`），但**抽取器本体只有这一份**。
 */
export const DEFAULT_REF_EXTS = "rs|ts|tsx|js|mjs|sh|json|toml|md|rhai|py|sql|yml|yaml";

/** 行号允许 `:N`、`:N/M`（同一文件多行，如 `wiki_compiler.rs:671/782`）、`:N-M`（区间）。 */
export const LINE_LIST = String.raw`\d+(?:[/-]\d+)*`;

/**
 * 从一段文本里抽出全部引用。纯函数 ⇒ 可被 `--selftest` 复用（判据 #147②）。
 *
 * @param {string} text 任意文本（按行切）
 * @param {string} [exts] 扩展名白名单（`a|b|c`，不带点）。缺省用 `DEFAULT_REF_EXTS`。
 * @returns {{file:string, line:number, text:string, srcLine:number, col:number, end:number, blankOk:boolean}[]}
 *   `text` = 该引用所在的整行；`blankOk` = 区间 / 多行清单里的**非首个**行号
 *   （端点只是边界，允许落在空行 —— 硬判它恒定假红）。
 */
export function extractRefs(text, exts = DEFAULT_REF_EXTS) {
  const fileAnchor = new RegExp(
    String.raw`([A-Za-z0-9_][A-Za-z0-9_./-]*\.(?:${exts}))\s*:\s*(${LINE_LIST})`,
    "g",
  );
  // 裸 `:行号`（承接同一段里前一个文件名）。负向先行断言排除 `13:43` 这类时刻：
  // 时刻的 `:` 前面是数字，而引用写法的 `:` 前面是标点 / 空白 / 文件名。
  const bareAnchor = new RegExp(String.raw`(?<![A-Za-z0-9]):\s*(${LINE_LIST})`, "g");

  const out = [];
  for (const [srcLine, raw] of text.split(/\r?\n/).entries()) {
    // 两个正则都跑在**整行**上，再按 index 归并 —— 必须整行，不能切片：
    // 切片会让负向先行断言丢掉左边界上下文，`:4:5`（cargo 诊断的 `行:列`）里
    // 第二个 `:` 会因为在切片里处于是「字符串开头」而被误当成裸引用，
    // 于是 `--> src/main.rs:4:5` 被读成 `src/main.rs:5`（实测踩到，判据 #7）。
    const toks = [];
    for (const a of raw.matchAll(fileAnchor)) {
      toks.push({ i: a.index, e: a.index + a[0].length, file: a[1], lines: a[2] });
    }
    for (const b of raw.matchAll(bareAnchor)) {
      toks.push({ i: b.index, e: b.index + b[0].length, file: null, lines: b[1] });
    }
    toks.sort((x, y) => x.i - y.i);

    let lastFile = null;
    let covered = -1;
    for (const t of toks) {
      if (t.i < covered) continue; // 落在前一个带文件名匹配的内部
      covered = t.e;
      if (t.file) lastFile = t.file;
      else if (!lastFile) continue; // 裸引用但本行还不知道文件名 ⇒ 无从判定
      const f = t.file ?? lastFile;
      for (const part of t.lines.split("/")) {
        const mm = part.match(/^(\d+)(?:-(\d+))?$/);
        if (!mm) continue;
        // `col` / `end` = 该引用 token 在**本行**里的列区间。软判据用它把「引号值」归给
        // 文本上最近的那条引用（同一行常有多条引用，见 single-source-facts 的
        // `quotedTokensOwnedBy`）。
        out.push({
          file: f,
          line: Number(mm[1]),
          text: raw,
          srcLine,
          col: t.i,
          end: t.e,
          blankOk: false,
        });
        if (mm[2]) {
          out.push({
            file: f,
            line: Number(mm[2]),
            text: raw,
            srcLine,
            col: t.i,
            end: t.e,
            blankOk: true,
          });
        }
      }
    }
  }
  return out;
}

/** posix 分隔的绝对路径（后缀对账用；`norm` 这个名字在旧脚本里已沿用）。 */
export const normPath = (p) => p.split(path.sep).join("/");

/**
 * 由**文件路径清单**建「basename → 全部真实路径」索引（单一定义）。
 * 文件清单的**来源**由调用方决定：`buildBasenameIndex` 走文件系统遍历，
 * 门禁也可传 `git ls-files` 的产物（VMware 共享目录 / 大仓下遍历要几分钟，
 * 而 git 索引读取是秒级；两者对「已入版本控制 + 未忽略的新文件」等价）。
 */
export function indexFromFileList(files) {
  const index = new Map();
  for (const p of files) {
    const name = path.basename(p);
    if (!index.has(name)) index.set(name, []);
    index.get(name).push(p);
  }
  return index;
}

/** 递归收全部文件（跳过 `skipDirs` 里的目录名），供 `buildBasenameIndex` 与门禁复用。 */
export function walkRepoFiles(repoRoot, skipDirs) {
  const out = [];
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      if (skipDirs.has(e.name)) continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile()) out.push(p);
    }
  };
  walk(repoRoot);
  return out;
}

/**
 * 按 basename 建「同名文件 → 全部真实路径」索引（walk 整个 repoRoot）。
 *
 * `skipDirs` 由调用方给：**它是覆盖面判据的一部分** —— 检出副本（`.worktrees` / `.qoder`）
 * 与产物副本（`output/**`）会把同一 basename 撑成「同名 N 份」，让本该可定位的引用
 * 退化成「无法定位 = 未检查」。故两种口径都在各调用方就地写明理由，不在本文件里静默决定。
 */
export function buildBasenameIndex(repoRoot, skipDirs) {
  return indexFromFileList(walkRepoFiles(repoRoot, skipDirs));
}

/**
 * 定位一个引用指向的文件。三分法：
 *   · `L` LOCATED   —— 唯一可定位（显式路径能解析，或同名唯一 / 路径后缀可消歧）
 *   · `N` NONLOC    —— 同名多份且后缀不能消歧（如裸 `mod.rs`）⇒ **未检查，不是通过**
 *   · `B` BROKEN    —— 仓内确实没有这个名字
 *
 * @param {string} refFile 引用串原样
 * @param {Map<string, string[]>} index `buildBasenameIndex` 的产物
 * @param {string} repoRoot 仓库根
 * @param {(refFile:string)=>(string|null)} [resolve] 可选的「路径→绝对路径」解析器。
 *   缺省用 `resolveRepoFile`（逐条 stat，SMB 上每条 ~190ms —— 实测 1.2 万条引用要 38 分钟）。
 *   批量扫描的调用方应传一个**基于文件清单集合**的解析器（内存查找，微秒级）：
 *   `check-doc-refs.mjs` 就是这么做的（清单来自 `git ls-files`，与 CI 检出同集合）。
 */
export function locateRef(refFile, index, repoRoot, resolve) {
  // ① 显式相对路径（含 `/`）：按仓库内的多种基准目录试（多 workspace 根，见 lib）
  if (refFile.includes("/")) {
    const hit = resolve ? resolve(refFile) : resolveRepoFile(repoRoot, refFile);
    if (hit) return { kind: "L", abs: hit };
  }
  const base = refFile.slice(refFile.lastIndexOf("/") + 1);
  const cands = index?.get(base) ?? [];
  if (cands.length === 0) return { kind: "B" };
  if (cands.length === 1) return { kind: "L", abs: cands[0] };
  // ② 同名多份 ⇒ 用「引用串的路径后缀」消歧（`init/state.rs` 只对应一个真实文件）。
  //    要求**路径段对齐**：`"…/opc_domain_pack_actions.rs".endsWith("/actions.rs")` 为 false
  //    ⇒ 段内截断的写法不可能靠这里蒙混过关（它归 ABBREV 桶，见 `abbrevCandidates`）。
  const want = refFile.replace(/^\.\//, "");
  const suffix = cands.filter((p) => normPath(p).endsWith("/" + want));
  if (suffix.length === 1) return { kind: "L", abs: suffix[0] };
  return { kind: "N", why: `同名 ${cands.length} 份、路径后缀也不能消歧（如 \`mod.rs\`/\`lib.rs\`）` };
}

/**
 * 「**段内截断**」候选 —— 纯函数。
 *
 * 问题形态：文档把 `opc_domain_pack_actions.rs` 写成 `actions.rs`（砍掉中间段）。
 * `locateRef` 的后缀消歧要求路径段对齐 ⇒ 永不命中；但它**确实是**真实 basename 的字符级
 * 后缀（`"opc_domain_pack_actions.rs".endsWith("actions.rs")` 为 true）。
 * 实测代价（2026-09-18）：`BROKEN（真待查）` 12 条里 11 条是这种写法 ⇒ 该桶指向不了任何真实风险。
 *
 * ⚠ 反向护栏：只在候选 basename **严格更长**且 `endsWith` 时才算 —— 等长命中不是截断。
 *
 * @returns {string[]} 疑似被截断的真实文件（仓库相对路径，已排序）
 */
export function abbrevCandidates(refFile, index, repoRoot) {
  const b = refFile.slice(refFile.lastIndexOf("/") + 1);
  if (!b || !index) return [];
  const out = [];
  for (const [name, arr] of index) {
    if (name.length > b.length && name.endsWith(b)) for (const p of arr) out.push(repoRoot ? repoRel(repoRoot, p) : p);
  }
  return out.sort();
}

/**
 * 「**疑似同义词 / 换词**」候选 —— 纯函数。比 `abbrevCandidates` 更宽：
 * 引用串未必是真实 basename 的字符后缀，而是**换掉了其中一段**。
 * 实证形态（AGENTS.md 记载）：`domain_pack_bridge.rs` 代 `opc_capability_pack_bridge.rs`
 * —— 字符后缀不成立（`domain` ≠ `capability`），但同一族文件的 `pack` / `bridge` 词干都在。
 *
 * 口径（刻意保守，宁可少报）：
 *   · 扩展名必须相同；
 *   · 空名 / 单段名（无 `_`、`-`）不参与 —— 短名撞词的概率太高，会把「真不存在」洗成候选；
 *   · 候选须与引用名**共享 ≥ max(2, 词干数−1) 个词干**（即最多换掉/补上一段），
 *     且候选**多出**词干（是缩写而非同义收缩）；
 *   · 结果上限 5 条（只给行动面，不枚举全族）。
 *
 * 实证：`domain_pack_bridge.rs` ↔ `opc_capability_pack_bridge.rs`
 *   —— 共享 `pack` / `bridge`（2 = 3−1），候补多出 `opc` / `capability` ⇒ 判同义候选。
 *
 * @returns {string[]} 仓库相对路径（已排序）
 */
export function synonymCandidates(refFile, index, repoRoot) {
  const b = refFile.slice(refFile.lastIndexOf("/") + 1);
  if (!b || !index) return [];
  const dot = b.lastIndexOf(".");
  if (dot <= 0) return [];
  const stem = b.slice(0, dot);
  const ext = b.slice(dot);
  const toks = stem.split(/[_-]/).filter(Boolean);
  if (toks.length < 2) return []; // 单段名不判同义（噪声 > 收益）
  const need = Math.max(2, toks.length - 1);
  const out = new Set();
  for (const [name, arr] of index) {
    if (name === b || !name.endsWith(ext)) continue;
    const cStem = name.slice(0, name.length - ext.length);
    // 字符级后缀已由 abbrevCandidates 负责，这里只补「换词 / 插词」形态
    if (cStem.length > stem.length && cStem.endsWith(stem)) continue;
    const cToks = cStem.split(/[_-]/).filter(Boolean);
    const set = new Set(cToks);
    const shared = toks.filter((t) => set.has(t)).length;
    if (shared < need) continue;
    // 多出的词干不得多于共享的（否则是「同一族里两个不同东西」，如 `2026-08.md` ↔
    // `AUDIT-code-review-2026-08-14.md` —— 共享 2 个日期词干、多出 4 个，纯噪声）
    if (cToks.length - shared > shared) continue;
    if (cToks.length <= shared) continue; // 必须**多出**词干 ⇒ 是缩写而非同义收缩
    for (const p of arr) out.add(repoRoot ? repoRel(repoRoot, p) : p);
  }
  return [...out].sort().slice(0, 5);
}
