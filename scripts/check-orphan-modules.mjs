#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 孤儿模块门（PLAN-reflection-optimization-closure §四.2）
 *
 * 要拦的根因：一个 `.rs` 文件可以整份存在、自带单测、甚至写着「待裁决后删除或接线」，
 * 但只要没有任何 `mod`/`pub mod`/`include!` 指向它，它就**不参与编译** ——
 * `cargo check` / `clippy` / `test` 全都不会抱怨，门禁全绿，代码却在树上假装活着。
 * 实测先例：`crates/analysis-engine/src/serenity_hit_rate_backtest.rs`
 * （智选 T+N 超额命中判据，11 个单测从未跑过，2026-10-15 才收编）。
 *
 * 口径（刻意笨、单调）：
 *   根 = 每个 package 的 `src/lib.rs`、`src/main.rs`、`src/bin/*.rs`
 *   边 = 文件里的 `mod NAME;` / `pub(crate) mod NAME;` / `include!("PATH")`
 *   解析 = 同名 `NAME.rs` 或 `NAME/mod.rs`（相对声明者所在目录）
 *   不可达 = 该 package `src/` 下存在、却从任何根都走不到的 `.rs`
 *   扫描面排除：`tests/` `examples/` `benches/`（它们各自是独立 target，不由 lib 声明）
 *
 * ⚠ 门绿 ≠ 全仓无死码：本门只判「有没有被编译」，不判「编译了但没人调」。
 *    豁免必须写理由 + 日期，且每次运行都打印（豁免悄悄长大 = 门悄悄失效）。
 */
import fs from "node:fs";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "..");
const PKG_DIRS = [
  path.join(ROOT, "src-tauri"),
  ...fs
    .readdirSync(path.join(ROOT, "src-tauri", "crates"), { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => path.join(ROOT, "src-tauri", "crates", d.name)),
];

/** 显式豁免：{ 相对路径, 理由, 日期 }。空表是目标状态。 */
const EXEMPT = [
  {
    file: "src-tauri/src/register_commands.rs",
    reason: "build.rs 生成的注册表，经 lib.rs 的 include! 消费；无 mod 声明属预期",
    since: "2026-10-15",
  },
];

/**
 * 既存基线（棘轮）：本门 2026-10-15 上线时首次查出的 11 个未声明文件，均**不在本轮范围**
 * （网关 / agent / kit，非股票业务），由各自归属会话处置。
 *
 * ⚠ 门绿 ≠ 待修面已清：这 11 个文件今天仍然不参与编译，基线只保证「不再长出第 12 个」。
 *    清单只减不增；减一条就把日期与处置提交号写进本注释。
 */
const BASELINE = [
  "src-tauri/crates/agent/src/fine_tune/candle_trainer.rs",
  "src-tauri/crates/agent/src/proactive_mode_20260706_170316_620.rs",
  "src-tauri/crates/kit/src/harness_impls.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/dingtalk.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/discord.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/feishu.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/qq.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/slack.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/telegram.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/wechat.rs",
  "src-tauri/crates/runtime/src/message_gateway/platforms/whatsapp.rs",
];

const SKIP_DIR_NAMES = new Set(["tests", "examples", "benches"]);
const MOD_RE = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;/gm;
const INCLUDE_RE = /include!\s*\(\s*"([^"]+)"\s*\)/g;
const MAIN_RE = /\bfn\s+main\s*\(/;

/** 剥掉注释后的代码文本（判据只看代码域：注释里提一遍文件名不算接线）。 */
function codeOnly(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");
}

function listRustFiles(dir, acc = []) {
  if (!fs.existsSync(dir)) return acc;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (SKIP_DIR_NAMES.has(entry.name)) continue;
      listRustFiles(abs, acc);
    } else if (entry.isFile() && entry.name.endsWith(".rs")) {
      acc.push(abs);
    }
  }
  return acc;
}

/** 把一条 `mod NAME;` 解析成它应当指向的文件（两种 Rust 模块布局都试）。 */
function resolveMod(declaringFile, name) {
  const dir = path.dirname(declaringFile);
  // src/foo.rs 里的 `mod bar;` → src/bar.rs 或 src/bar/mod.rs
  // src/foo/mod.rs 里的 `mod bar;` → src/foo/bar.rs 或 src/foo/bar/mod.rs
  const candidates = [path.join(dir, `${name}.rs`), path.join(dir, name, "mod.rs")];
  // `src/foo.rs` 里声明的 `mod bar;` 也可解析到 `src/foo/bar.rs`（2018 起的子目录形态）。
  // 漏掉这两条会把 `message_gateway.rs` 里的 `mod platforms;` 判成孤儿（实测踩过 16 个假阳）。
  const base = path.basename(declaringFile, ".rs");
  if (base !== "mod" && base !== "lib" && base !== "main") {
    candidates.push(path.join(dir, base, `${name}.rs`), path.join(dir, base, name, "mod.rs"));
  }
  for (const c of candidates) if (fs.existsSync(c)) return c;
  // `#[path]` 属性本门不解析（出现即应进豁免表并写明理由）
  return null;
}

function collectRoots(pkgDir) {
  const src = path.join(pkgDir, "src");
  const roots = [];
  for (const cand of [path.join(src, "lib.rs"), path.join(src, "main.rs")]) {
    if (fs.existsSync(cand)) roots.push(cand);
  }
  // 第二入口：src 下任何自带 `fn main()` 的文件都是一个可执行根（如 server_main.rs），
  // 它们不必被 lib.rs 声明也在编译。
  for (const f of listRustFiles(src)) {
    if (f.endsWith("/mod.rs") || f.endsWith("\\mod.rs")) continue;
    try {
      if (MAIN_RE.test(codeOnly(fs.readFileSync(f, "utf8")))) roots.push(f);
    } catch {
      /* 读不到就交给后续判定 */
    }
  }
  const binDir = path.join(src, "bin");
  if (fs.existsSync(binDir)) {
    for (const f of fs.readdirSync(binDir)) {
      if (f.endsWith(".rs")) roots.push(path.join(binDir, f));
      // bin/<name>/main.rs 形态
      const nested = path.join(binDir, f, "main.rs");
      if (fs.existsSync(nested)) roots.push(nested);
    }
  }
  return roots;
}

function reachFrom(roots) {
  const seen = new Set();
  const queue = [...roots];
  while (queue.length > 0) {
    const file = queue.pop();
    if (seen.has(file)) continue;
    seen.add(file);
    let text;
    try {
      text = fs.readFileSync(file, "utf8");
    } catch {
      continue;
    }
    for (const m of text.matchAll(MOD_RE)) {
      const resolved = resolveMod(file, m[1]);
      if (resolved) queue.push(resolved);
    }
    for (const m of text.matchAll(INCLUDE_RE)) {
      const p = path.resolve(path.dirname(file), m[1]);
      // include! 的目标可能在 OUT_DIR（构建期生成），存在才收
      if (fs.existsSync(p)) queue.push(p);
    }
  }
  return seen;
}

const orphans = [];
let scanned = 0;
let macroDeclared = 0;
for (const pkg of PKG_DIRS) {
  const src = path.join(pkg, "src");
  if (!fs.existsSync(src)) continue;
  const files = listRustFiles(src);
  scanned += files.length;
  const reachable = reachFrom(collectRoots(pkg));
  // 兜底判据：宏生成的模块清单（如 `entity_modules!(...)` / `$m!(name);`）在文本上
  // 不是 `mod name;`，图走会漏。故再查一次「可达文件的代码域里是否出现该 basename」——
  // 命中即认定已被宏声明。注释已剥，所以「只在注释里提过文件名」不算接线
  // （serenity_hit_rate_backtest 当年的情形正是如此）。
  const reachableCode = [...reachable]
    .map((f) => {
      try {
        return codeOnly(fs.readFileSync(f, "utf8"));
      } catch {
        return "";
      }
    })
    .join("\n");
  for (const f of files) {
    if (reachable.has(f)) continue;
    const rel = path.relative(ROOT, f).split(path.sep).join("/");
    if (EXEMPT.some((e) => e.file === rel) || BASELINE.includes(rel)) continue;
    const base = path.basename(f, ".rs");
    if (new RegExp(`\\b${base}\\b`).test(reachableCode)) {
      macroDeclared += 1;
      continue;
    }
    orphans.push(rel);
  }
}

for (const e of EXEMPT) {
  console.log(`豁免: ${e.file} —— ${e.reason}（自 ${e.since}）`);
}

if (orphans.length > 0) {
  console.error(`孤儿模块 ${orphans.length} 个（存在但没有任何 mod/include 指向 ⇒ 不参与编译）：`);
  for (const o of orphans) console.error(`  ${o}`);
  console.error(
    "处置二选一：① 在所属 crate 的 lib.rs/mod.rs 里声明它并跑通其单测；② 删除文件。",
  );
  console.error("（不许第三种：留着不声明 = 门禁永远看不见它，本次先例见 PLAN §四）");
  process.exit(1);
}
console.log(
  `结论: 扫描 ${scanned} 个 .rs 文件，无新增孤儿模块（宏清单兜底命中 ${macroDeclared} 个；豁免 ${EXEMPT.length} 条已逐条打印；既存基线 ${BASELINE.length} 条未处置，见文件头说明）`,
);

// ── 自测：判据本身要能红 ──
// 只看「基线/豁免的排除逻辑会不会把该报的也吞了」。基线写成全量放行 = 本门恒真，
// 所以这里必须拿一个**不在基线里的真实类案例**断言它仍会被上报。
{
  const inBaseline = (rel) => EXEMPT.some((e) => e.file === rel) || BASELINE.includes(rel);
  // 类案例：本轮收编的智选胜率线文件（曾被查出、现已声明，绝不允许进基线）
  if (inBaseline("src-tauri/crates/analysis-engine/src/serenity_hit_rate_backtest.rs")) {
    console.error("自测失败：serenity 胜率线被放进了基线/豁免 —— 那等于本门对它永久失明");
    process.exit(1);
  }
  if (!inBaseline(BASELINE[0])) {
    console.error("自测失败：基线首条未被识别（BASELINE 与排除逻辑不同型）");
    process.exit(1);
  }
  // 空基线不是目标态的提醒（打印，不影响退出码）
  console.log(`自测: 基线排除逻辑有效（serenity 类案例仍会上报）；基线条数=${BASELINE.length}`);
}
