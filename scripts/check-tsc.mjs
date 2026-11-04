#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * TypeScript 类型检查的唯一入口：`npm run typecheck` / `npm run lint` / pre-commit 都指向这里
 * （替代裸 `tsc --noEmit`）。
 *
 * 为什么不能直接用 npm/npx：
 *   从 UNC 路径（`\\host\share\...`）发起 git hook 时，npm/npx 经 cmd.exe 启动，
 *   cmd.exe 不支持 UNC cwd ⇒ 退到 `C:\Windows` ⇒ tsc 根本不在仓库里跑。
 *
 * 为什么不能只换成 `node node_modules/typescript/bin/tsc --noEmit`：
 *   本机 TypeScript 7（tsgo）在 UNC cwd 下解析 tsconfig 会**发现 0 个文件且 EXIT=0**
 *   （任务 #111，实测）：`-p \\host\share\...\tsconfig.json` 0 文件；
 *   相对 `-p tsconfig.json` 0 文件；`-p D:\...\tsconfig.json` 7480 个文件。
 *   所以这里先把仓库根换算成盘符等价路径，并在真实类型检查前**显式校验文件发现数**，
 *   绝不让「0 文件」静默通过。
 *
 * 行为：
 *   1. 仓库在 UNC 时，扫描盘符找同一仓库根的盘符等价路径；找不到 ⇒ 硬失败；
 *   2. `--listFilesOnly` 统计 src 源文件数，少于 MIN_PROJECT_FILES ⇒ 硬失败；
 *   3. 执行真实 `--noEmit`，退出码原样透传（不吞、不降级）。
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..");
/** 当前仓库 src 下被 tsc 发现的源文件约 1400+；低于此值视为「扫描面残缺」而非代码问题。 */
const MIN_PROJECT_FILES = 100;

function fail(message) {
  console.error(`[check-tsc] ${message}`);
  process.exit(1);
}

/** 仓库在 UNC（`\\host\share\...`）时，找到指向同一仓库根的盘符路径（如 `D:\...`）。 */
function findDriveMirror(uncRoot) {
  const uncPkg = path.join(uncRoot, "package.json");
  if (!fs.existsSync(uncPkg) || !fs.existsSync(path.join(uncRoot, "tsconfig.json"))) return undefined;
  const parts = uncRoot.replace(/^\\\\/, "").split("\\").filter(Boolean);
  const drives = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"].filter((letter) => fs.existsSync(`${letter}:\\`));
  // 盘符可能映射到共享根，也可能映射到更深目录：逐层尝试 UNC 路径的每一段后缀，
  // 用 package.json 逐字相等（而不只是 size）确认是同一个仓库根。
  // 从最深后缀往前找：优先选「映射得离仓库最近」的盘符，得到最短路径
  // （本机同时存在 Z: → `\\host\Shared Folders` 与 D: → 同目录，前者会多出一层 `Z:\D\...`）。
  for (let i = parts.length - 1; i >= 1; i -= 1) {
    const suffix = parts.slice(i).join("\\");
    for (const letter of drives) {
      const candidate = `${letter}:\\${suffix}`;
      const candidatePkg = path.join(candidate, "package.json");
      if (!fs.existsSync(candidatePkg) || !fs.existsSync(path.join(candidate, "tsconfig.json"))) continue;
      try {
        if (fs.readFileSync(candidatePkg, "utf8") === fs.readFileSync(uncPkg, "utf8")) return candidate;
      } catch {
        // 不可读的候选盘符直接跳过
      }
    }
  }
  return undefined;
}

let runtimeRoot = repoRoot;
if (/^\\\\/.test(repoRoot)) {
  const mirror = findDriveMirror(repoRoot);
  if (!mirror) {
    fail(
      "仓库位于 UNC 路径，且未找到等价的盘符路径。\n" +
        "  本机 TypeScript 7（tsgo）在 UNC cwd 下会「发现 0 个文件仍 EXIT=0」（任务 #111），\n" +
        "  无法可靠执行类型检查 ⇒ 拒绝把这条门判成通过。\n" +
        `  仓库: ${repoRoot}\n` +
        "  处理: 先为该共享映射盘符（如 D:）后从盘符路径提交，或从本地盘路径提交。",
    );
  }
  runtimeRoot = mirror;
}

const tscBin = path.join(runtimeRoot, "node_modules", "typescript", "bin", "tsc");
const project = path.join(runtimeRoot, "tsconfig.json");
if (!fs.existsSync(tscBin)) fail(`找不到 TypeScript 入口: ${tscBin}（node_modules 未安装？）`);
if (!fs.existsSync(project)) fail(`找不到 tsconfig: ${project}`);

function runTsc(extraArgs, options) {
  return spawnSync(process.execPath, [tscBin, ...extraArgs, "-p", project], {
    cwd: runtimeRoot,
    maxBuffer: 64 * 1024 * 1024,
    ...options,
  });
}

// 1) 先验扫描面：0 文件 / 明显残缺一律红。
const listing = runTsc(["--noEmit", "--listFilesOnly"], { encoding: "utf8" });
if (listing.error) fail(`启动 tsc 失败: ${listing.error.message}`);
if (listing.status !== 0) {
  process.stderr.write(listing.stderr || "");
  process.stdout.write(listing.stdout || "");
  fail(`tsc --listFilesOnly 退出码 ${listing.status}`);
}
const projectFiles = (listing.stdout || "")
  .split(/\r?\n/)
  .filter((line) => /[\\/]src[\\/].+\.(ts|tsx|mts|cts)$/.test(line)).length;
if (projectFiles < MIN_PROJECT_FILES) {
  fail(
    `tsc 只发现 ${projectFiles} 个 src 源文件（阈值 ${MIN_PROJECT_FILES}）⇒ 视为未真正检查，拒绝通过。\n` +
      `  项目: ${project}\n` +
      "  若确属 tsconfig include 收缩，请同步调整 MIN_PROJECT_FILES 并说明理由。",
  );
}

// 2) 真实类型检查，退出码原样透传。
console.log(`[check-tsc] tsc 发现 src 源文件 ${projectFiles} 个，开始 --noEmit 类型检查…`);
const checked = runTsc(["--noEmit"], { stdio: "inherit" });
if (checked.error) fail(`启动 tsc 失败: ${checked.error.message}`);
process.exit(checked.status ?? 1);