#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// cargo 状态探针 —— 判「cargo 到底在推进，还是卡住了 / 在排队等锁」，附各 target 目录概览。
//
// ## 为什么需要它（2026-09-21 实测沉淀）
//
// 「cargo 好像不动了」有三个**错判据**，都是测错了观察面：
//   ① 看 `target/debug/deps` 的 mtime —— build script 阶段的产物**不在 deps**，
//      而在 `build/<crate>-<hash>/out`；`deps` 不动属正常。
//   ② 数 `rustc.exe` 进程（=0 就判卡死）—— build script 进程名是
//      `build-script-main.exe`，按 rustc 数根本数不到。
//   ③ 看日志有没有新行 —— cargo 非 tty 下每个 crate 只打一行 `Compiling`，
//      只剩一个慢 crate 时必然长时间静止。
//
// **唯一正判据**：间隔若干秒测两次 `target/**/build/*/out` 的**最新 mtime**；
// 在推进 ⇒ 正常，去等，别杀（Windows 上的 choke 点通常是 `aws-lc-sys`，
// 完整 C+汇编构建，实测 `Compiling` 状态可挂 10 分钟以上）。
//
// 另有一类**不是卡死**的情形：日志里只有
//   `Blocking waiting for file lock on build directory`
// —— 那是**在排队等锁**，而持锁方**可能不是你自己**（本机实测同时跑着别人的
// `cargo clippy -p <某 crate>` 和一个长驻的 `cargo run`）。等锁**不耗 CPU**，
// 所以「保留排队任务 + 另起独立 target 作第二条腿」是划算的。
//
// ## 用法
//
//   node scripts/probe-cargo-state.mjs              # 默认等 30s 测推进
//   node scripts/probe-cargo-state.mjs --wait=0     # 只做瞬时快照，不等待
//   node scripts/probe-cargo-state.mjs --wait=60
//
// ## 查进程归属（Windows）—— 本脚本不代查，避免与「禁止乱杀进程」冲突
//
//   Get-CimInstance Win32_Process -Filter "Name='cargo.exe'" |
//     Select-Object ProcessId,CreationDate,CommandLine | Format-List
//
// **判归属只看 `CommandLine`**：排队队列里的命令可能一条都不是你的。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CARGO_ROOT = path.join(ROOT, "src-tauri");

function fail(msg) {
  process.stderr.write(`probe-cargo-state: ${msg}\n`);
  process.exit(1);
}

function argOf(name, dflt) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((a) => a.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : dflt;
}

const waitSec = Math.max(0, Number.parseInt(argOf("wait", "30"), 10) || 0);
const rel2 = (p) => path.relative(ROOT, p).replace(/\\/g, "/");

function targetDirs() {
  if (!fs.existsSync(CARGO_ROOT)) return [];
  return fs
    .readdirSync(CARGO_ROOT, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith("target"))
    .map((e) => path.join(CARGO_ROOT, e.name));
}

/** 一个 target 的 debug/deps 概况（`deps` 是全量依赖产物的落点）。 */
function depsInfo(dir) {
  const deps = path.join(dir, "debug", "deps");
  let entries;
  try {
    entries = fs.readdirSync(deps);
  } catch {
    return null;
  }
  let n = 0;
  let rmeta = 0;
  let rlib = 0;
  let newest = 0;
  for (const f of entries) {
    n++;
    if (f.endsWith(".rmeta")) rmeta++;
    if (f.endsWith(".rlib")) rlib++;
    try {
      const m = fs.statSync(path.join(deps, f)).mtimeMs;
      if (m > newest) newest = m;
    } catch {
      /* 瞬态文件，忽略 */
    }
  }
  return { n, rmeta, rlib, newest };
}

/**
 * 跨全部 target 取 `debug/build/<crate>-<hash>/out` 里的最新产物 —— **推进判据的观察面**。
 *
 * ⚠ 本 JSDoc 里**不要**写带星号通配的路径：星号紧跟斜杠会**提前闭合块注释**，
 *   把后面的注释行变成代码（2026-09-21 实测：`SyntaxError: Unexpected identifier`）。
 *   行注释 `//` 形式没有这个问题。
 *
 * 刻意不只看某一个 crate：慢的是 build script（如 `aws-lc-sys`），
 * 而它可能落在任一 target 目录下。
 */
function latestBuildArtifact(dirs) {
  let best = { t: 0, p: "(none)" };
  for (const dir of dirs) {
    const buildDir = path.join(dir, "debug", "build");
    let subs;
    try {
      subs = fs.readdirSync(buildDir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of subs) {
      if (!e.isDirectory()) continue;
      const out = path.join(buildDir, e.name, "out");
      let files;
      try {
        files = fs.readdirSync(out);
      } catch {
        continue;
      }
      for (const f of files) {
        try {
          const m = fs.statSync(path.join(out, f)).mtimeMs;
          if (m > best.t) {
            best = { t: m, p: `${path.basename(dir)}/debug/build/${e.name}/out/${f}` };
          }
        } catch {
          /* 瞬态文件，忽略 */
        }
      }
    }
  }
  return best;
}

function diskFreeGB(p) {
  try {
    const st = fs.statfsSync(p);
    return ((st.bavail * st.bsize) / 1024 ** 3).toFixed(1);
  } catch {
    return null;
  }
}

const dirs = targetDirs();
if (dirs.length === 0) fail(`在 ${rel2(CARGO_ROOT)} 下没找到 target* 目录`);

console.log(`仓库根 ${rel2(ROOT)}`);
console.log(`target 目录 ${dirs.length} 个：${dirs.map((d) => path.basename(d)).join(", ")}\n`);

for (const d of dirs) {
  const info = depsInfo(d);
  if (!info) {
    console.log(`  ${path.basename(d)}  （无 debug/deps）`);
    continue;
  }
  const ageMin = ((Date.now() - info.newest) / 60000).toFixed(1);
  console.log(
    `  ${path.basename(d).padEnd(16)} deps=${String(info.n).padEnd(6)} rmeta=${String(info.rmeta).padEnd(6)} rlib=${String(info.rlib).padEnd(6)} 最新 ${ageMin} 分钟前`,
  );
}
const free = diskFreeGB(ROOT);
if (free) console.log(`\n磁盘可用 ${free} GB`);

const t1 = latestBuildArtifact(dirs);
console.log(`\nbuild 产物最新：${t1.p}`);
console.log(`  时间 ${new Date(t1.t).toISOString()}`);

if (waitSec === 0) {
  console.log("\n（--wait=0，跳过推进复测）");
  process.exit(0);
}

console.log(`\n等待 ${waitSec}s 后复测（推进判据）…`);
await new Promise((r) => setTimeout(r, waitSec * 1000));
const t2 = latestBuildArtifact(dirs);
console.log(`build 产物最新：${t2.p}`);
console.log(`  时间 ${new Date(t2.t).toISOString()}`);

if (t2.t > t1.t) {
  const delta = ((t2.t - t1.t) / 1000).toFixed(1);
  console.log(`\n✅ PROGRESS=YES  窗口内推进了 ${delta}s ⇒ **正常编译中**，去等，别杀。`);
} else {
  console.log("\n⚠ PROGRESS=NO  窗口内无新 build 产物 ⇒ **不是编译中**，可能：");
  console.log("   · 某进程长驻持锁（典型：别的会话在跑 `cargo run` 起应用）；");
  console.log("   · 你的 cargo 在排队 —— 日志里会有 `Blocking waiting for file lock`；");
  console.log("   · 真有挂死的 cargo。");
  console.log("   先按文件头的 PowerShell 片段查 CommandLine 判归属，再决定动不动它。");
}
