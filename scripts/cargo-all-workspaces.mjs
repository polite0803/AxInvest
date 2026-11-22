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
import fs from "node:fs";
import path from "node:path";
import { ensurePluginBuildRoots, pluginBuildRootsDir } from "./lib/plugin-build-roots.mjs";
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
 * nextest 前必须真实暂存的插件产物（顺序即串行构建顺序）。
 *
 * 一份规格同时驱动 staging 与自检：新增「靠环境变量门控」的 bundle E2E 时，
 * 必须在这里登记插件名与环境变量，否则自检会因测试面出现未登记变量而失败。
 * 这样 CI 不会因为漏设变量而把 `SKIP` 读成通过。
 */
const BUNDLE_STAGING = [
  { plugin: "stock", env: "AXAGENT_STOCK_BUNDLE_DIR" },
  { plugin: "opc", env: "AXAGENT_OPC_BUNDLE_DIR" },
  { plugin: "opc-accounting", env: "AXAGENT_ACCOUNTING_BUNDLE_DIR" },
  { plugin: "opc-sales-growth", env: "AXAGENT_SALES_BUNDLE_DIR" },
  { plugin: "opc-content-media", env: "AXAGENT_CONTENT_BUNDLE_DIR" },
  { plugin: "opc-ai-research", env: "AXAGENT_AI_RESEARCH_BUNDLE_DIR" },
  { plugin: "opc-consulting", env: "AXAGENT_CONSULTING_BUNDLE_DIR" },
  { plugin: "opc-design", env: "AXAGENT_DESIGN_BUNDLE_DIR" },
  { plugin: "opc-ecommerce", env: "AXAGENT_ECOMMERCE_BUNDLE_DIR" },
  { plugin: "opc-education", env: "AXAGENT_EDUCATION_BUNDLE_DIR" },
  { plugin: "opc-finance-invest", env: "AXAGENT_FINANCE_INVEST_BUNDLE_DIR" },
  { plugin: "opc-game-dev", env: "AXAGENT_GAME_DEV_BUNDLE_DIR" },
  { plugin: "opc-geospatial", env: "AXAGENT_GEOSPATIAL_BUNDLE_DIR" },
  { plugin: "opc-project-management", env: "AXAGENT_PROJECT_MANAGEMENT_BUNDLE_DIR" },
  { plugin: "opc-security", env: "AXAGENT_SECURITY_BUNDLE_DIR" },
  { plugin: "opc-software-dev", env: "AXAGENT_SOFTWARE_DEV_BUNDLE_DIR" },
  { plugin: "wiki", env: "AXAGENT_WIKI_BUNDLE_DIR" },
];
const BUNDLE_ENV_BY_PLUGIN = new Map(BUNDLE_STAGING.filter((item) => item.env).map((item) => [item.plugin, item.env]));
const E2E_NON_BUNDLE_GATES = new Set([
  // 通用子进程开关：本入口没有生成通用测试 worker，CI 仍诚实 SKIP，不伪造通过。
  "AXAGENT_TEST_PLUGIN_SUBPROCESS",
]);
const E2E_GATED_BUNDLE_VARS = new Set([
  ...BUNDLE_STAGING.map((item) => item.env).filter(Boolean),
  // stock 工具 E2E 与 bundle 安装 E2E 共用同一份 worker 文件。
  "AXAGENT_STOCK_WORKER_BIN",
]);

function platformWorkerName() {
  return process.platform === "win32" ? "axagent-plugin-stock.exe" : "axagent-plugin-stock";
}

/** 测试源码里的 `AXAGENT_*` 门控变量；只认 `env::var` / `var_os` 的实参，不认注释。 */
function bundleGateEnvVarsFromTests() {
  const testsDir = path.join(repoRoot, "axagent-plugins", "crates", "plugins", "tests");
  if (!fs.existsSync(testsDir)) return null;
  const found = new Set();
  for (const file of fs.readdirSync(testsDir)) {
    if (!file.endsWith(".rs")) continue;
    const source = fs.readFileSync(path.join(testsDir, file), "utf8");
    for (const match of source.matchAll(/std::env::var(?:_os)?\(\s*"([A-Z0-9_]+)"/g)) {
      if (match[1].startsWith("AXAGENT_")) found.add(match[1]);
    }
  }
  return found;
}

/** 产物落点：`build-plugin-bundle.mjs` 的缺省输出，源码与产物同在主仓。 */
function bundledPluginDir(plugin) {
  return path.join(repoRoot, "axagent-plugins", "dist", "plugins", plugin);
}

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

  // staging 规格自检：之前这里只验 argv，漏 staging 时测试会 SKIP 而门禁仍绿。
  const plugins = BUNDLE_STAGING.map((item) => item.plugin);
  t(`⑩ staging 插件清单无重复（${plugins.length} 项）`, new Set(plugins).size === plugins.length);
  t(
    "⑪ staging 覆盖 stock、公共 opc 与全部 14 个 OPC 域插件",
    plugins.includes("stock")
      && plugins.includes("opc")
      && plugins.filter((p) => p.startsWith("opc-")).length === 14,
  );
  t(
    `⑫ 门控变量一一对应、${BUNDLE_ENV_BY_PLUGIN.size} 个 bundle E2E 都被暂存`,
    new Set(BUNDLE_ENV_BY_PLUGIN.values()).size === BUNDLE_ENV_BY_PLUGIN.size
      // 额外的唯一变量是 stock worker 文件；声明式根已随 11 域全 worker 退役。
      && BUNDLE_ENV_BY_PLUGIN.size === E2E_GATED_BUNDLE_VARS.size - 1,
  );
  const missingStagedVars = [...E2E_GATED_BUNDLE_VARS].filter((name) => !stagedEnvironment()[name]);
  t(`⑬ 每个 E2E 门控变量都有暂存路径（缺：${missingStagedVars.join(", ") || "无"}）`, missingStagedVars.length === 0);
  const testVars = bundleGateEnvVarsFromTests();
  const unregisteredVars = testVars
    ? [...testVars].filter((name) => !E2E_GATED_BUNDLE_VARS.has(name) && !E2E_NON_BUNDLE_GATES.has(name))
    : [];
  t(`⑭ E2E 源码里的 bundle 门控变量全部已登记或显式豁免（漏：${unregisteredVars.join(", ") || "无"}）`, unregisteredVars.length === 0);
  // 自检跑在纯 Node 的 parity job（干净 checkout，没有 dist 产物），只验路径口径。
  // 产物是否真实存在由 nextest 分支的 `assertStagedArtifacts()` 在 staging 之后 fail-closed。
  const wrongDirs = BUNDLE_STAGING.filter(
    (item) => bundledPluginDir(item.plugin)
      !== path.join(repoRoot, "axagent-plugins", "dist", "plugins", item.plugin),
  ).map((item) => item.plugin);
  t(`⑮ staging 产物路径都锚定主仓 axagent-plugins/dist（偏：${wrongDirs.join(", ") || "无"}）`, wrongDirs.length === 0);
  t("⑯ 负控：删掉一个门控变量后确实能被检出", !stagedEnvironment({ skip: [BUNDLE_STAGING[0].env] })[BUNDLE_STAGING[0].env]);
  const stockWorker = stagedEnvironment().AXAGENT_STOCK_WORKER_BIN;
  t(`⑰ stock worker 路径指向 bundle/bin 下的平台可执行文件（${stockWorker ?? "未生成"}）`, Boolean(stockWorker && stockWorker.endsWith(path.join("bin", platformWorkerName()))));
  t(`⑱ 负控：平台 worker 文件名由平台决定（${platformWorkerName()}）`, ["axagent-plugin-stock", "axagent-plugin-stock.exe"].includes(platformWorkerName()));
  const buildRootsDir = pluginBuildRootsDir(repoRoot);
  t(
    `⑲ 临时构建根与主 workspace 分离、生成器在场（${path.relative(repoRoot, buildRootsDir)}）`,
    Boolean(buildRootsDir) && buildRootsDir !== path.join(repoRoot, "axagent-plugins")
      && fs.existsSync(path.join(repoRoot, "scripts", "generate-plugin-independent-repos.mjs")),
  );

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

/** 由 staging 规格导出 E2E 环境变量；`skip` 只给自检做负控。 */
function stagedEnvironment({ skip = [] } = {}) {
  const env = {};
  for (const item of BUNDLE_STAGING) {
    if (!item.env || skip.includes(item.env)) continue;
    env[item.env] = bundledPluginDir(item.plugin);
  }
  // 同一个 stock worker 同时供 bundle 安装 E2E 与工具/命令帧 E2E 使用；
  // 两个变量必须指向同一份已暂存产物，避免后者因缺 env 静默 SKIP。
  if (env.AXAGENT_STOCK_BUNDLE_DIR && !skip.includes("AXAGENT_STOCK_WORKER_BIN")) {
    env.AXAGENT_STOCK_WORKER_BIN = path.join(env.AXAGENT_STOCK_BUNDLE_DIR, "bin", platformWorkerName());
  }
  return env;
}

/** staging 完成后 fail-closed：任一产物缺失就停，不让 E2E 静默 SKIP 冒充通过。 */
function assertStagedArtifacts() {
  const missing = BUNDLE_STAGING.filter((item) => !fs.existsSync(bundledPluginDir(item.plugin)))
    .map((item) => item.plugin);
  if (missing.length) {
    console.error(`[cargo ${mode}] staging 后产物缺失：${missing.join(", ")}`);
    process.exit(1);
  }
  const stockWorker = stagedEnvironment().AXAGENT_STOCK_WORKER_BIN;
  if (!stockWorker || !fs.existsSync(stockWorker)) {
    console.error(`[cargo ${mode}] stock worker 可执行文件缺失：${stockWorker ?? "未生成"}`);
    process.exit(1);
  }
}

if (mode === "nextest" && !listOnly) {
  // bundle E2E 在没有产物时只会打印 SKIP 并返回成功。这里逐插件串行 staging，
  // 任一失败立即中止，确保 CI 里每个门控 E2E 都有真实产物可用。
  // 打包脚本与 worker 模板源权威都在主仓 `axagent-plugins/`。CI 只 checkout 本仓，
  // 不能依赖 sibling 独立插件仓；先用生成器从模板源物化临时构建根（与主仓 workspace
  // 隔离的 exclude crate 需要自己的根 Cargo.toml），再用主仓脚本构建 + `--check` 握手验收。
  const buildRoots = ensurePluginBuildRoots(repoRoot, BUNDLE_STAGING.map((item) => item.plugin));
  for (const { plugin } of BUNDLE_STAGING) {
    const staged = spawnSync(
      process.execPath,
      [
        "axagent-plugins/scripts/build-plugin-bundle.mjs",
        "--plugin", plugin,
        "--build-root", buildRoots.get(plugin),
        "--check",
      ],
      { cwd: repoRoot, stdio: "inherit", env: process.env },
    );
    if (staged.status !== 0) {
      console.error(`[cargo ${mode}] staging ${plugin} bundle 失败`);
      process.exit(staged.status ?? 1);
    }
  }
  assertStagedArtifacts();
  Object.assign(process.env, stagedEnvironment());
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
