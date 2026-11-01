#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 在**每一个** Cargo workspace 根执行同一条 cargo 命令。
 *
 * 为什么需要（问题形态）：
 *   L2 逻辑拆分后仓库根下有 8 个 workspace（axagent-core / -kit / -ai / -infra /
 *   -quant / -plugins / -runtime / src-tauri）。任何仍只在 `src-tauri` 里执行的
 *   `cargo fmt --all --check` / `cargo clippy --workspace` / `cargo check --workspace`
 *   都只覆盖根包 + axagent-mobile，**其余 34 个 crate 完全不检查** —— 门禁照样绿。
 *   这与「搬走 crate ⇒ 扫描面变小 ⇒ 假绿」是同一种失效模式，只是发生在 cargo 侧。
 *
 * 口径：
 *   workspace 根由 `scripts/lib/workspace-roots.mjs` 自动探测（与各静态门禁同源），
 *   不手抄名单；新拆出的 workspace 自动纳入。
 *
 * 用法：
 *   node scripts/cargo-all-workspaces.mjs fmt-check
 *   node scripts/cargo-all-workspaces.mjs clippy
 *   node scripts/cargo-all-workspaces.mjs check
 *   node scripts/cargo-all-workspaces.mjs nextest
 *   node scripts/cargo-all-workspaces.mjs --list      # 只打印将执行的命令，不跑
 *   node scripts/cargo-all-workspaces.mjs --selftest  # 只验命令组装（纯 Node，不起 cargo）
 *
 * 额外参数会原样追加到每个 workspace 的 cargo 命令末尾（`--` 之后的部分照常传）。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { discoverWorkspaceRoots, repoRel } from "./lib/workspace-roots.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..");
const MODES = {
  "fmt-check": { args: ["fmt", "--all", "--check"] },
  clippy: {
    args: ["clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"],
  },
  check: { args: ["check", "--workspace"] },
  nextest: { args: ["nextest", "run", "--workspace"] },
};

const argv = process.argv.slice(2);
const listOnly = argv.includes("--list");
const mode = argv.find((a) => !a.startsWith("--"));
const extra = argv.filter((a) => a !== mode && a !== "--list" && a !== "--selftest");

/**
 * 不变量：cargo 自己的参数（含 `--manifest-path`）必须全部落在**第一个 `--` 之前**。
 * `--` 之后的每一个 token 都交给 rustc / clippy-driver，cargo 不认。
 */
function cargoSideArgsOrdered(args) {
  const sep = args.indexOf("--");
  const mp = args.indexOf("--manifest-path");
  if (mp === -1) return false;
  return sep === -1 || mp < sep;
}

/** 2026-11-01 之前的写法（无条件追加）—— 只用于负控，证明检法不是没电。 */
function legacyFlatten(modeName, root) {
  return [...MODES[modeName].args, "--manifest-path", path.join(root, "Cargo.toml")];
}

/**
 * 自检：9 条正负对照，全部在纯 Node 里比对**组装出来的 argv**，不起 cargo。
 *
 * 为什么这道门值得存在：`--manifest-path` 落到 rustc 侧时，**已缓存的 crate 不调 rustc
 * ⇒ 不报错**，只有真需要编译的才炸。于是暖缓存的机器上一路绿，而空缓存的 CI runner
 * 第一个 crate 就编译 ⇒ 8 根的 clippy 门禁整体不可信（实测 `axagent-core` `EXIT=101`，
 * `error: Unrecognized option: 'manifest-path'`）。四种模式共用一行拼接，正是这种失效
 * 的藏身处 —— 只有 clippy 的 args 带 `--`，所以另外三种模式从来没露过面。
 */
function selftest() {
  let pass = 0;
  const fails = [];
  const t = (label, cond) => (cond ? (pass += 1) : fails.push(label));
  const FAKE = "X:/repo/axagent-fake";
  const MP = path.join(FAKE, "Cargo.toml");

  const clippy = buildArgs("clippy", FAKE, []);
  t("① clippy：--manifest-path 落在 `--` 之前", cargoSideArgsOrdered(clippy));
  t(
    "② clippy：rustc 侧只剩 `-D warnings`（没被塞进 cargo 参数）",
    clippy.slice(clippy.indexOf("--") + 1).join(" ") === "-D warnings",
  );
  t(
    "③ clippy：清单路径就是该根的 Cargo.toml",
    clippy[clippy.indexOf("--manifest-path") + 1] === MP,
  );
  t(
    "④ 负控：旧的无条件追加写法确实违反不变量（检法有电）",
    !cargoSideArgsOrdered(legacyFlatten("clippy", FAKE)),
  );

  for (const m of ["fmt-check", "check", "nextest"]) {
    const a = buildArgs(m, FAKE, []);
    t(
      `⑤ ${m}：无 \`--\` 时 manifest 追加在末尾`,
      !a.includes("--") && a.lastIndexOf("--manifest-path") === a.length - 2,
    );
  }

  const withExtra = buildArgs("clippy", FAKE, ["--features", "foo"]);
  t(
    "⑥ 额外参数保留，且仍排在 `--` 之前",
    cargoSideArgsOrdered(withExtra) && withExtra.includes("--features"),
  );
  const extraSep = buildArgs("check", FAKE, ["--", "--offline"]);
  t("⑦ 额外参数自带 `--` 时同样守序", cargoSideArgsOrdered(extraSep));

  const roots = discoverWorkspaceRoots(repoRoot);
  t(`⑧ 探测到 workspace 根（本机 ${roots.length} 个；0 根不得当通过）`, roots.length > 0);
  const bad = roots.filter((r) => !cargoSideArgsOrdered(buildArgs("clippy", r, [])));
  t(`⑨ 本机 ${roots.length} 个根的 clippy 命令逐条守序`, bad.length === 0);

  console.log(`[cargo-all-workspaces --selftest] ${pass} 项通过 / ${fails.length} 项失败`);
  for (const f of fails) console.log(`  ✖ ${f}`);
  return fails.length === 0 ? 0 : 1;
}

if (argv.includes("--selftest")) process.exit(selftest());

if (!mode || !MODES[mode]) {
  console.error(`用法: node scripts/cargo-all-workspaces.mjs <${Object.keys(MODES).join("|")}> [--list]`);
  process.exit(2);
}

const roots = discoverWorkspaceRoots(repoRoot);
if (roots.length === 0) {
  console.error("未探测到任何 Cargo workspace 根 —— 拒绝以「0 个根」当作通过");
  process.exit(2);
}

let failed = 0;

/**
 * 组装单个 workspace 的 cargo argv。
 *
 * ⚠ `--manifest-path` **必须落在 `--` 分隔符之前**（它是 cargo 的参数，不是 rustc 的）。
 *   原先写成 `[...MODES[mode].args, "--manifest-path", <清单>, ...extra]` 无条件追加，
 *   而 `clippy` 模式的 args 里带 `--`（其后 `-D warnings` 是给 rustc 的）⇒ `--manifest-path`
 *   落到 rustc 侧，被原样转给 clippy-driver。实测（2026-11-01，`axagent-core`，
 *   CI 同参数）：`error: Unrecognized option: 'manifest-path'` ×N，`EXIT=101`。
 *   失效形态尤其阴险：**已缓存的 crate 不调 rustc ⇒ 不报错**，只有真需要编译的才炸
 *   ⇒ 暖缓存的机器上可能「跑绿」，而空缓存的 CI runner 第一个 crate 就编译 ⇒ **必然红**，
 *   且红的原因是参数位置，不是代码。
 *   `fmt-check` / `check` / `nextest` 三个模式没有 `--`，所以只有 clippy 中招 ——
 *   「四种模式共用一行拼接」正是这类失效得以藏身的地方。
 */
function buildArgs(modeName, root, extraArgs) {
  const base = [...MODES[modeName].args, ...extraArgs];
  const manifest = ["--manifest-path", path.join(root, "Cargo.toml")];
  const sep = base.indexOf("--");
  return sep === -1
    ? [...base, ...manifest]
    : [...base.slice(0, sep), ...manifest, ...base.slice(sep)];
}

for (const root of roots) {
  const relRoot = repoRel(repoRoot, root);
  const args = buildArgs(mode, root, extra);
  console.log(`\n[cargo ${mode}] ${relRoot}`);
  if (listOnly) {
    console.log(`  cargo ${args.join(" ")}`);
    continue;
  }
  // cwd 取 workspace 根本身：`.cargo/config.toml` 的共享 target-dir 与
  // `rust-toolchain.toml` 的版本钉扎都按「最近的清单目录」解析，不能一律在仓库根跑。
  const r = spawnSync("cargo", args, { cwd: root, stdio: "inherit", env: process.env });
  if (r.error) {
    console.error(`[cargo ${mode}] ${relRoot} 启动失败: ${r.error.message}`);
    failed += 1;
    continue;
  }
  if (r.status !== 0) {
    console.error(`[cargo ${mode}] ${relRoot} 失败（exit ${r.status}）`);
    failed += 1;
  }
}

if (listOnly) {
  console.log(`\n共 ${roots.length} 个 workspace 根（--list 只打印，不执行）`);
  process.exit(0);
}
if (failed > 0) {
  console.error(`\n[cargo ${mode}] ${failed}/${roots.length} 个 workspace 失败`);
  process.exit(1);
}
console.log(`\n[cargo ${mode}] ${roots.length} 个 workspace 全部通过`);
