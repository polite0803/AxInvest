#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 「改动前快照」工具 —— 本仓**禁止碰 git**（含只读的 status / log / diff），
// 所以回滚点只能靠文件副本，这个脚本就是那套副本机制。
//
// ## 为什么不是 `cp -r`
//
// 三条理由，都踩过：
//   1. **顺序**：快照必须发生在改动**之前**。事后补备份 = 拿改动后的状态当回滚点，
//      等于没有回滚点。`verify` 子命令就是用来证明这件事的 —— 改动后跑 `verify`
//      **必须**看到 DIFF；若全是 SAME，说明快照晚于改动（2026-09-17 实战：18 个
//      文件里 16 个与工作区逐字节相同，"先备份再改"的顺序被记反，事后才发现）。
//   2. **可核对**：每份副本记录 sha256，不是「我记得备份了」。
//   3. **按清单**：改动清单由调用方显式给，避免 `cp -r` 整目录把 `node_modules`、
//      构建产物一并拖进来。
//
// ## 用法
//
//   node scripts/backup-snapshot.mjs create --manifest=changes.txt
//   node scripts/backup-snapshot.mjs create src/a.ts src/b.rs        # 直接列路径
//   node scripts/backup-snapshot.mjs verify --manifest=changes.txt   # 改动**之后**核对
//   node scripts/backup-snapshot.mjs verify --dest=output/backup-x --manifest=changes.txt
//   node scripts/backup-snapshot.mjs create --force ...              # 强制覆盖同名副本
//
// manifest 格式：一行一个**相对仓库根**的路径；`#` 开头为注释，空行忽略。
//
// 默认快照目录 `output/backup-<YYYY-MM-DD>`。`create` 遇到**已存在且内容不同**的
// 副本会 **WARN + 跳过**（不覆盖）—— 免得把「改动后」的状态盖成「改动前」，
// 让快照静默失效。确实要刷新时用 `--force` 或换 `--dest`。
//
// 退出码：0 正常；1 = 清单里有文件在工作区/快照中缺失（回滚链不完整）。

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function fail(msg) {
  process.stderr.write(`backup-snapshot: ${msg}\n`);
  process.exit(1);
}

function argOf(name, dflt) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((a) => a.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : dflt;
}

const argv = process.argv.slice(2);
const force = argv.includes("--force");
const positional = argv.filter((a) => !a.startsWith("--"));
const mode = positional[0];
if (mode !== "create" && mode !== "verify") {
  fail(
    "用法:\n" +
      "  node scripts/backup-snapshot.mjs create [--dest=<目录>] [--manifest=<文件>] [--force] <路径...>\n" +
      "  node scripts/backup-snapshot.mjs verify [--dest=<目录>] [--manifest=<文件>] <路径...>",
  );
}

// 清单 = manifests 文件内容 + 位置参数（保序去重）
let paths = positional.slice(1);
const manifestArg = argOf("manifest", null);
if (manifestArg) {
  const mp = path.isAbsolute(manifestArg) ? manifestArg : path.join(ROOT, manifestArg);
  if (!fs.existsSync(mp)) fail(`清单文件不存在：${mp}`);
  const fromFile = fs
    .readFileSync(mp, "utf8")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
  paths = [...fromFile, ...paths];
}
paths = [...new Set(paths)];
if (paths.length === 0) fail("没给出任何文件：用 --manifest=<文件> 或直接列路径");

const today = new Date().toISOString().slice(0, 10);
const dest = argOf("dest", `output/backup-${today}`);
const DEST = path.isAbsolute(dest) ? dest : path.join(ROOT, dest);

/** 逐字节 sha256 —— 只认字节，不认 mtime（mtime 会被 checkout / 复制改写）。 */
const sha = (p) => crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex");
const rel2 = (p) => path.relative(ROOT, p).replace(/\\/g, "/");

console.log(`模式=${mode}  快照目录=${rel2(DEST)}  文件数=${paths.length}\n`);

let copied = 0;
let skipped = 0;
let same = 0;
let diff = 0;
let missing = 0;

for (const rel of paths) {
  const src = path.join(ROOT, rel);
  const dst = path.join(DEST, rel);

  if (mode === "create") {
    if (!fs.existsSync(src)) {
      console.log(`MISS  ${rel}  （工作区无此文件，未备份）`);
      missing++;
      continue;
    }
    const srcSha = sha(src);
    if (fs.existsSync(dst) && !force) {
      if (sha(dst) !== srcSha) {
        console.log(
          `WARN  ${rel}  快照已存在且内容不同 ⇒ 跳过（不把改动后状态盖成「改动前」）。要覆盖加 --force`,
        );
      } else {
        console.log(`SAME  ${rel}  （快照已是同一份，跳过）`);
      }
      skipped++;
      continue;
    }
    fs.mkdirSync(path.dirname(dst), { recursive: true });
    fs.copyFileSync(src, dst);
    copied++;
    console.log(`COPY  ${srcSha.slice(0, 12)}  ${rel}`);
    continue;
  }

  // ── verify ──
  if (!fs.existsSync(dst)) {
    console.log(`MISS  ${rel}  （快照里没有 ⇒ 无回滚点）`);
    missing++;
    continue;
  }
  if (!fs.existsSync(src)) {
    console.log(`DEL   ${rel}  （工作区已删除，快照仍在）`);
    diff++;
    continue;
  }
  if (sha(src) === sha(dst)) {
    same++;
    console.log(`SAME  ${rel}`);
  } else {
    diff++;
    console.log(`DIFF  ${rel}`);
  }
}

if (mode === "create") {
  console.log(`\n合计: COPY=${copied} SKIP=${skipped} MISS=${missing}`);
  if (missing > 0) console.log("⚠ 有文件在工作区不存在 —— 清单里的路径写错了？");
  console.log(`快照目录：${rel2(DEST)}`);
  process.exit(missing > 0 ? 1 : 0);
}

console.log(`\n合计: SAME=${same} DIFF=${diff} MISSING=${missing}`);
if (missing > 0) {
  console.log("⚠ 有文件在快照中缺失 ⇒ 这些文件**没有回滚点**，回滚链不完整。");
} else if (diff === 0) {
  console.log(
    "⚠ 全部 SAME ⇒ 要么还没改，要么快照发生在改动**之后**（后者等于没有回滚点，须重做）。",
  );
} else {
  console.log(`✓ ${diff} 个文件与快照不同 ⇒ 快照确为改动前状态，回滚可用。`);
}
process.exit(missing > 0 ? 1 : 0);
