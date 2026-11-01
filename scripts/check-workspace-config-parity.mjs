#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 多 workspace 根「构建配置逐字同源」校验（PLAN-repo-split L2）。
 *
 * 为什么需要（问题形态，2026-11-01 实测）：
 *   L2 把 41 个 crate 搬到 7 个新 workspace 根时，只搬了代码与依赖，**没搬根级构建配置**：
 *   `[profile.dev*]` 只在「发起构建的那个 workspace 根」生效，写在成员里 cargo 直接忽略。
 *   7 个子根缺它 ⇒ 退回默认 `debug = true` / `incremental = true` ⇒
 *     · 同一份依赖按 metadata hash 在各根各编一遍（`axagent_harness` 实测 36 个增量会话）；
 *     · 僵尸增量会话目录涨到 **879 个 / 68G**（src-tauri 2026-08-31 关增量时记录的是 415 个 / 55G）；
 *     · 全量 clippy 从单根 **9m24s** 变成 8 根串行 **67m53s** —— 拆分的立项目标（缩短构建时间）反被击穿。
 *   而 `cargo check` / `clippy` / 任何既有门禁**都不会因此变红**：配置少一段，编译照样过。
 *   这是「搬家只搬代码不搬配置」的静默回归，必须有机检。
 *
 * 口径（与 `scripts/lib/workspace-roots.mjs` 同源自动探测，不手抄根名单 ⇒ 新拆出的根自动纳入）：
 *   1. `[profile.dev]` / `[profile.dev.build-override]`：键值在所有根**完全一致**，并额外硬拦两个
 *      地板值 —— `[profile.dev].incremental` 必须显式为 `false`、`debug` 必须存在且不为 `true`。
 *      （这两个值就是上面那 68G 与链接耗时的来源，产品可以自由改别的键，但不能退回默认。）
 *   2. `[workspace.lints.clippy]`：所有根都存在 `disallowed_types` 与 `module_name_repetitions`
 *      且取值一致 —— 否则禁区 8（无 std::sync 锁）在某些根**静默失去执行面**。
 *   3. `clippy.toml` / `rustfmt.toml` / `rust-toolchain.toml`：每个根都必须有，内容必须一致
 *      （缺 `rustfmt.toml` ⇒ 该根按默认 100 列以外口径格式化，两边永远互相改回）。
 *   4. `.cargo/config.toml` 的 `[build] target-dir`：定义了就不得互相打架；
 *      未定义的根只报告不拦（应用壳层 `src-tauri` 本就是共享 target 的锚点）。
 *
 * 解析局限（刻意）：只认「一行一个 `key = value`」的平凡表，不实现完整 TOML
 *   —— 多行数组 / inline table 会让比对失真，宁可就所需表做窄解析，扩展面由需求来定。
 *
 * 用法：
 *   node scripts/check-workspace-config-parity.mjs            # 判定
 *   node scripts/check-workspace-config-parity.mjs --selftest  # 正负对照（判据本身会不会漏）
 */
import fs from "node:fs";
import path from "node:path";
import { discoverWorkspaceRoots, repoRel } from "./lib/workspace-roots.mjs";

const REPO_ROOT = path.resolve(import.meta.dirname, "..");
const SHARED_FILES = ["clippy.toml", "rustfmt.toml", "rust-toolchain.toml"];
/** 平凡表：一行一个 `key = value`。表名 → Map(key → 原样值文本)。 */
const KV_LINE_RE = /^([A-Za-z0-9_.\-"]+)[ \t]*=[ \t]*(.+?)[ \t]*$/;

/**
 * 只解析目标表（其余表跳过但记录表头，用于「表不存在」的判断）。
 * @returns {{tables: Map<string, Map<string,string>>, seen: Set<string>}}
 */
export function parseTables(text, wanted) {
  const tables = new Map();
  const seen = new Set();
  let current = null;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.replace(/#(?![^"]*").*$/, "").trim();
    if (!line) continue;
    const header = /^\[([^\]]+)\]$/.exec(line);
    if (header) {
      const name = header[1];
      seen.add(name);
      current = wanted.includes(name) ? new Map() : null;
      if (current) tables.set(name, current);
      continue;
    }
    if (!current) continue;
    const kv = KV_LINE_RE.exec(line);
    if (!kv) {
      // 多行数组 / inline table 等非平凡写法：标出来，不猜。
      current.set(`__unparsed__${current.size}`, line);
      continue;
    }
    if (!current.has(kv[1])) current.set(kv[1], kv[2]);
  }
  return { tables, seen };
}

const PROFILES = ["profile.dev", "profile.dev.build-override"];
const LINTS = ["workspace.lints.clippy"];

/** 判据①②：清单内的 profile / lints 表跨根一致 + 两个地板值。返回问题列表。 */
export function checkManifestRules(rootManifests) {
  const problems = [];
  const parsed = new Map();
  for (const { rel, text } of rootManifests) {
    const { tables, seen } = parseTables(text, [...PROFILES, ...LINTS]);
    parsed.set(rel, { tables, seen });
  }

  for (const table of [...PROFILES, ...LINTS]) {
    const keysByRoot = new Map();
    for (const [rel, p] of parsed) {
      if (!p.seen.has(table)) {
        problems.push(`${rel} 缺 [${table}] 表`);
        continue;
      }
      keysByRoot.set(rel, p.tables.get(table));
    }
    // 跨根逐键对拍：以任意一根为基准即可（一致 ⇒ 谁当基准都一样）。
    const baseRel = [...keysByRoot.keys()][0];
    if (!baseRel) continue;
    const base = keysByRoot.get(baseRel);
    for (const [rel, map] of keysByRoot) {
      if (rel === baseRel) continue;
      for (const key of base.keys()) {
        if (!map.has(key)) {
          problems.push(`${rel} 的 [${table}] 缺键 \`${key}\`（${baseRel} 有 = ${base.get(key)}）`);
        } else if (map.get(key) !== base.get(key)) {
          problems.push(`${rel} 的 [${table}] \`${key}\` = ${map.get(key)} ≠ ${baseRel} 的 ${base.get(key)}`);
        }
      }
      for (const key of map.keys()) {
        if (!base.has(key)) problems.push(`${rel} 的 [${table}] 多出键 \`${key}\` = ${map.get(key)}（${baseRel} 无）`);
      }
    }
  }

  // 地板值：这两条是「退回 cargo 默认」的唯一有害形态，单独点名。
  for (const [rel, p] of parsed) {
    const dev = p.tables.get("profile.dev");
    if (!dev) continue;
    if (dev.get("incremental") !== "false") {
      const incVal = dev.get("incremental") ?? '"(缺，即默认 true)"';
      problems.push(`${rel} 的 [profile.dev].incremental = ${incVal} —— 必须显式 false，否则每编一遍落一个僵尸增量会话（实测 879 个 / 68G）`);
    }
    const dbg = dev.get("debug");
    if (dbg === undefined || dbg === "true" || dbg === '""' || dbg === '"false"') {
      problems.push(`${rel} 的 [profile.dev].debug = ${dbg ?? "(缺，即默认 true)"} —— 完整符号表按实测比 line-tables-only 慢 30-50%（链接慢 50%+）`);
    }
    const bo = p.tables.get("profile.dev.build-override");
    if (bo && bo.get("incremental") !== "false") {
      problems.push(`${rel} 的 [profile.dev.build-override].incremental = ${bo.get("incremental") ?? "(缺，即默认 true)"} —— 构建脚本的增量会话同样 GC 不掉`);
    }
  }

  for (const [rel, p] of parsed) {
    const lints = p.tables.get("workspace.lints.clippy");
    if (!lints) continue;
    for (const key of ["disallowed_types", "module_name_repetitions"]) {
      if (!lints.has(key)) {
        problems.push(`${rel} 的 [workspace.lints.clippy] 缺 \`${key}\` —— 未继承该规则的 crate 会静默失去对应执行面`);
      }
    }
  }
  return problems;
}

/** 判据③：根级共享配置文件必须存在且内容一致。 */
export function checkSharedFiles(roots) {
  const problems = [];
  for (const file of SHARED_FILES) {
    const contents = new Map();
    for (const root of roots) {
      const p = path.join(root, file);
      if (!fs.existsSync(p)) {
        problems.push(`${repoRel(REPO_ROOT, p)} 缺失（其余根都有 ⇒ 该根的 lint/格式/钉版口径与别的根打架）`);
        continue;
      }
      contents.set(repoRel(REPO_ROOT, p), fs.readFileSync(p, "utf8").replace(/\r\n/g, "\n"));
    }
    const entries = [...contents];
    if (entries.length < 2) continue;
    const [baseRelPath, baseText] = entries[0];
    for (const [rel, text] of entries.slice(1)) {
      if (text !== baseText) {
        problems.push(`${rel} 与 ${baseRelPath} 内容不一致（同名根级配置出现分叉 ⇒ 逐字对齐，别挑一边）`);
      }
    }
  }
  return problems;
}

/** 判据④：`.cargo/config.toml` 的 target-dir 不得互相打架。 */
export function checkTargetDir(roots) {
  const problems = [];
  const defined = new Map();
  const undef = [];
  for (const root of roots) {
    const p = path.join(root, ".cargo", "config.toml");
    if (!fs.existsSync(p)) {
      undef.push(repoRel(REPO_ROOT, path.dirname(p)));
      continue;
    }
    const { tables } = parseTables(fs.readFileSync(p, "utf8"), ["build"]);
    const v = tables.get("build")?.get("target-dir");
    if (v === undefined) undef.push(repoRel(REPO_ROOT, path.dirname(p)));
    else defined.set(repoRel(REPO_ROOT, p), v);
  }
  const entries = [...defined];
  const [baseName, baseVal] = entries[0] ?? [];
  for (const [name, val] of entries.slice(1)) {
    if (val !== baseVal) problems.push(`${name} 的 target-dir = ${val} ≠ ${baseName} 的 ${baseVal} ⇒ 两根不再共享同一份增量缓存`);
  }
  return { problems, notes: undef.length > 0 ? [`${undef.length} 个根未定义 target-dir（各自建 target/）：${undef.join(", ")}`] : [] };
}

/** 自检：判据本身会不会漏（正样本 + 逐条负控）。 */
export function selftest() {
  let pass = 0;
  const fails = [];
  const t = (label, cond) => (cond ? (pass += 1) : fails.push(label));

  const good = `
[workspace]
[profile.dev]
opt-level = 0
debug = "line-tables-only"
incremental = false
split-debuginfo = "unpacked"
[profile.dev.build-override]
opt-level = 0
codegen-units = 16
debug = false
incremental = false
[workspace.lints.clippy]
disallowed_types = "deny"
module_name_repetitions = "allow"
`;
  t("正样本：两份一致 ⇒ 0 问题", checkManifestRules([{ rel: "a/Cargo.toml", text: good }, { rel: "b/Cargo.toml", text: good }]).length === 0);

  const missingProfile = good.replace(/\[profile\.dev\][\s\S]*?split-debuginfo = "unpacked"\n/, "");
  t("负控1：子根整段缺 [profile.dev] ⇒ 红", checkManifestRules([{ rel: "a/Cargo.toml", text: good }, { rel: "b/Cargo.toml", text: missingProfile }]).some((p) => p.includes("缺 [profile.dev] 表")));

  const incrementalOn = good.replace("incremental = false\nsplit-debuginfo", "incremental = true\nsplit-debuginfo");
  t("负控2：incremental 退回 true ⇒ 红（68G 僵尸会话那格）", checkManifestRules([{ rel: "a/Cargo.toml", text: good }, { rel: "b/Cargo.toml", text: incrementalOn }]).some((p) => p.includes("incremental = true")));

  const incrementalAbsent = good.replace(/^incremental = false$\n/m, "").replace(/^incremental = false$\n/m, "");
  t("负控3：不写 incremental（=默认 true）同样红", checkManifestRules([{ rel: "a/Cargo.toml", text: incrementalAbsent }]).some((p) => p.includes("缺，即默认 true")));

  const debugDefault = good.replace('debug = "line-tables-only"', "debug = true");
  t("负控4：debug = true ⇒ 红", checkManifestRules([{ rel: "a/Cargo.toml", text: debugDefault }]).some((p) => p.includes("debug = true")));

  const lintsGone = good.replace('disallowed_types = "deny"\n', "");
  t("负控5：某根缺 disallowed_types ⇒ 红（禁区 8 失守那格）", checkManifestRules([{ rel: "a/Cargo.toml", text: good }, { rel: "b/Cargo.toml", text: lintsGone }]).some((p) => p.includes("disallowed_types")));

  const valueDrift = good.replace('codegen-units = 16', 'codegen-units = 256');
  t("负控6：同一键两边取值不同 ⇒ 红且点出两边值", checkManifestRules([{ rel: "a/Cargo.toml", text: good }, { rel: "b/Cargo.toml", text: valueDrift }]).some((p) => p.includes("codegen-units") && p.includes("256")));

  const tmp = path.join(REPO_ROOT, ".tmp", "parity-selftest");
  fs.mkdirSync(tmp, { recursive: true });
  const dirA = fs.mkdtempSync(path.join(tmp, "rootA-"));
  const dirB = fs.mkdtempSync(path.join(tmp, "rootB-"));
  fs.mkdirSync(path.join(dirA, ".cargo"), { recursive: true });
  fs.mkdirSync(path.join(dirB, ".cargo"), { recursive: true });
  fs.writeFileSync(path.join(dirA, "clippy.toml"), "a = 1\n");
  fs.writeFileSync(path.join(dirB, "clippy.toml"), "a = 2\n");
  fs.writeFileSync(path.join(dirA, ".cargo", "config.toml"), '[build]\ntarget-dir = "../t1"\n');
  fs.writeFileSync(path.join(dirB, ".cargo", "config.toml"), '[build]\ntarget-dir = "../t2"\n');
  t("负控7：共享文件内容分叉 ⇒ 红", checkSharedFiles([dirA, dirB]).some((p) => p.includes("clippy.toml")));
  t("负控8：target-dir 打架 ⇒ 红", checkTargetDir([dirA, dirB]).problems.some((p) => p.includes("增量缓存")));
  fs.rmSync(tmp, { recursive: true, force: true });

  console.log(`[workspace-config-parity --selftest] ${pass} 项通过 / ${fails.length} 项失败`);
  for (const f of fails) console.log(`  ✖ ${f}`);
  return fails.length === 0 ? 0 : 1;
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--selftest")) process.exit(selftest());

  const roots = discoverWorkspaceRoots(REPO_ROOT);
  if (roots.length === 0) {
    console.error("未探测到任何 Cargo workspace 根 —— 拒绝以「0 个根」当作通过");
    process.exit(2);
  }
  const rootManifests = roots
    .map((r) => ({ rel: repoRel(REPO_ROOT, path.join(r, "Cargo.toml")), text: fs.readFileSync(path.join(r, "Cargo.toml"), "utf8") }))
    .filter((m) => /^\[workspace\]/m.test(m.text));

  const target = checkTargetDir(roots);
  const problems = [...checkManifestRules(rootManifests), ...checkSharedFiles(roots), ...target.problems];
  const notes = target.notes;

  console.log(`扫描 ${roots.length} 个 workspace 根：${roots.map((r) => repoRel(REPO_ROOT, r) || ".").join(" / ")}`);
  for (const n of notes) console.log(`  · 报告（不拦）：${n}`);
  if (problems.length > 0) {
    for (const p of problems) console.log(`  ✖ ${p}`);
    console.error(`\n[workspace-config-parity] ${problems.length} 处跨根配置漂移 —— 少一段 profile 不会让编译变红，但会让同一依赖按 hash 各编一遍（实测全量 9m24s → 67m53s、僵尸增量会话 68G）。`);
    process.exit(1);
  }
  console.log(`✔ ${roots.length} 个根的 [profile.dev*] / [workspace.lints.clippy] / ${SHARED_FILES.join(" / ")} 逐字同源，target-dir 不冲突`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.filename)) main();
