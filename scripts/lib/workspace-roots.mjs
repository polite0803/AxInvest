// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 多 workspace 根探测 —— 门禁脚本的唯一真相源。
 *
 * 为什么需要它（问题形态）：
 *   逻辑拆分（PLAN-repo-split L1/L2）把 crate 从 `src-tauri/crates/*` 搬到
 *   `axagent-{core,kit,ai,infra,quant,plugins,runtime}/crates/*`。任何门禁脚本只要
 *   硬编码 `src-tauri` + `axagent-core` 两个根，**搬走的 crate 就从扫描面消失** ——
 *   扫面变小、存量违规跟着消失 ⇒ 门禁**假绿**（未检查 ≠ 通过）。
 *   实测：kit 搬走后孤儿模块扫面 1988 → 1950（-38，正好是 kit 源码数），门仍 exit 0。
 *
 * 口径：
 *   workspace 根 = 仓库根目录下**含 `[workspace]` 段**的 `Cargo.toml` 所在目录
 *   （另含仓库根本身，若它也声明了 `[workspace]`）。只探一层子目录 + 根，不递归；
 *   `target/`、`node_modules/`、`.git/` 等不参与。
 *   ⇒ 新拆出的 workspace **自动纳入**，脚本不再逐组改名单。
 *
 * 注意：本文件只回答「根在哪 / 哪些文件属于根」。**语义边界仍由各门自己定**
 *   —— 例如应用壳层 `src-tauri/src/commands` 不是 crate 根，不得因本文件而扩大扫描面。
 */
import fs from "node:fs";
import path from "node:path";

/** 目录名级跳过表（探测与遍历共用）。 */
const SKIP_DIR_NAMES = new Set([
  "target",
  "node_modules",
  ".git",
  ".worktrees",
  "output",
  ".qoder",
  ".workbuddy",
  "dist",
  "coverage",
]);

const WORKSPACE_HEADER_RE = /^[ \t]*\[workspace\][ \t]*(?:#.*)?$/m;

/** 该目录的 `Cargo.toml` 是否声明了 `[workspace]`（`[workspace.dependencies]` 不算）。 */
export function isWorkspaceRoot(dir) {
  const manifest = path.join(dir, "Cargo.toml");
  try {
    return WORKSPACE_HEADER_RE.test(fs.readFileSync(manifest, "utf8"));
  } catch {
    return false;
  }
}

/**
 * 探测仓库内全部 Cargo workspace 根（绝对路径，按目录名排序 ⇒ 输出稳定）。
 * @param {string} repoRoot 仓库根绝对路径
 * @returns {string[]}
 */
export function discoverWorkspaceRoots(repoRoot) {
  const roots = [];
  if (isWorkspaceRoot(repoRoot)) roots.push(path.resolve(repoRoot));
  let entries = [];
  try {
    entries = fs.readdirSync(repoRoot, { withFileTypes: true });
  } catch {
    return roots;
  }
  for (const e of entries) {
    if (!e.isDirectory() || SKIP_DIR_NAMES.has(e.name)) continue;
    const dir = path.join(repoRoot, e.name);
    if (isWorkspaceRoot(dir)) roots.push(path.resolve(dir));
  }
  return roots.sort((a, b) => a.localeCompare(b));
}

/** 某个 workspace 根下的 `crates/*` 目录（只取一层子目录，不要求已声明 member）。 */
export function listCrateDirs(workspaceRoot) {
  const cratesDir = path.join(workspaceRoot, "crates");
  let entries = [];
  try {
    entries = fs.readdirSync(cratesDir, { withFileTypes: true });
  } catch {
    return [];
  }
  return entries
    .filter((e) => e.isDirectory())
    .map((e) => path.join(cratesDir, e.name))
    .sort((a, b) => a.localeCompare(b));
}

/** 通用递归遍历：按谓词收文件；跳过构建产物 / 检出副本目录。 */
export function walkFiles(dir, predicate, out = []) {
  let entries = [];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of entries) {
    if (SKIP_DIR_NAMES.has(e.name)) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      walkFiles(p, predicate, out);
    } else if (e.isFile() && predicate(e.name, p)) {
      out.push(p);
    }
  }
  return out;
}

/** 某目录下全部 `.rs`（含 tests/examples/benches；不含 target 等）。 */
export function listRustFiles(dir) {
  return walkFiles(dir, (name) => name.endsWith(".rs"));
}

/** 某目录下全部 `Cargo.toml`（不含 target 等）。 */
export function listCargoManifests(dir) {
  return walkFiles(dir, (name) => name === "Cargo.toml");
}

/** 仓库相对路径（posix 分隔，便于跨平台对账与基线键）。 */
export function repoRel(repoRoot, absPath) {
  return path.relative(repoRoot, absPath).split(path.sep).join("/");
}

/**
 * 把「仓库相对路径」解析成真实文件（多根感知）。
 *
 * 依次尝试：
 *   1. 仓库根 + relPath（`src-tauri/src/...`、`src/...`、`docs/...` 等仍走这条）
 *   2. 各 workspace 根 + relPath（`crates/...`）
 *   3. 各 workspace 根 + `crates/` + relPath（裸 crate 相对写法 `dao/src/x.rs`）
 * 找不到返回 null ⇒ 调用方必须报「覆盖面失效」，不得静默放行。
 */
export function resolveRepoFile(repoRoot, relPath) {
  if (!relPath) return null;
  if (path.isAbsolute(relPath)) {
    return fs.existsSync(relPath) ? relPath : null;
  }
  const rel = relPath.split(/[\\/]/).join(path.sep);
  const roots = discoverWorkspaceRoots(repoRoot);
  const candidates = [
    path.join(repoRoot, rel),
    ...roots.map((r) => path.join(r, rel)),
    ...roots.map((r) => path.join(r, "crates", rel)),
  ];
  for (const c of candidates) {
    try {
      if (fs.statSync(c).isFile()) return c;
    } catch {
      /* 继续试下一个 */
    }
  }
  return null;
}

/**
 * 按 **crate 目录名** 定位文件，返回仓库相对路径（找不到返回 null）。
 *
 * 用途：单点路径型门禁（「权威文件是 analysis-engine/src/x.rs」）在 crate 换 workspace
 * 后不该失效。目录名在拆分中保持不变，故按 `<root>/crates/<name>/<relPath>` 探测即可。
 * 同名 crate 出现两处时返回 null 并在 stderr 提示 —— 宁可红，不可静默选错。
 */
export function crateRepoRel(repoRoot, crateDirName, relPath = "") {
  const roots = discoverWorkspaceRoots(repoRoot);
  const hits = roots
    .map((r) => path.join(r, "crates", crateDirName))
    .filter((dir) => fs.existsSync(path.join(dir, "Cargo.toml")));
  if (hits.length > 1) {
    console.error(
      `[workspace-roots] crate 目录名 "${crateDirName}" 在多个 workspace 中重复：` +
        hits.map((d) => repoRel(repoRoot, d)).join(" / ") +
        " —— 拒绝猜测，按未找到处理",
    );
    return null;
  }
  if (hits.length === 0) return null;
  return repoRel(repoRoot, path.join(hits[0], relPath.split(/[\\/]/).join(path.sep)));
}

/** `crateRepoRel` 的绝对路径版本（找不到返回 null）。 */
export function crateFile(repoRoot, crateDirName, relPath = "") {
  const rel = crateRepoRel(repoRoot, crateDirName, relPath);
  return rel ? path.join(repoRoot, rel) : null;
}

/** 按 package 名（Cargo.toml `name`）找包目录；找不到返回 null。 */
export function findPackageDir(repoRoot, packageName) {
  const roots = discoverWorkspaceRoots(repoRoot);
  const candidates = [];
  for (const r of roots) {
    candidates.push(r);
    candidates.push(...listCrateDirs(r));
  }
  for (const dir of candidates) {
    try {
      const text = fs.readFileSync(path.join(dir, "Cargo.toml"), "utf8");
      const m = /^[ \t]*name[ \t]*=[ \t]*"([^"]+)"/m.exec(text);
      if (m && m[1] === packageName) return dir;
    } catch {
      /* 没有 manifest 的目录跳过 */
    }
  }
  return null;
}
