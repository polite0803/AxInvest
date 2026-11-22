#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 插件源钉版清单漂移检查（2026-11-22）
 *
 * 背景：主仓 `axagent-plugins/crates/` 下的 17 个业务插件副本已删除，权威源码是
 * `github.com/polite0803/axinvest-plugin-<plugin>` 这 17 个独立仓。CI 不再「连带」主仓
 * 副本一起构建，而是按 `scripts/lib/plugin-sources.json` 里钉死的 commit SHA 检出。
 *
 * 为什么钉版而不是跟分支：跟分支意味着「同一份 commit 的 CI 结果不可复现」——插件仓
 * 一推新提交，主仓上次绿的证据就作废。钉版后发布前手工 bump（只改清单，不自动改），
 * 本脚本负责把「清单 ↔ 实际取源」和「清单 ↔ 远端默认分支」两处漂移**报告出来**。
 *
 * 三条校验：
 *   A. 清单 ↔ 复合动作 `.github/actions/checkout-plugin-repos/action.yml`
 *      每个 repository/ref/path 必须与清单逐项一致（离线、确定性）；
 *   B. 清单 ↔ 各 workflow 顶层 `AXAGENT_PLUGIN_REPOS_ROOT`（离线、确定性）
 *      值必须是 `${{ github.workspace }}/.plugin-repos`，与动作的 path 默认值同目录；
 *   C. 清单 SHA ↔ 远端 `refs/heads/<defaultBranch>` 当前 HEAD（联网、只报告）。
 *      漂移 = 插件仓有了新提交，需在下次发布前手工 bump 清单后重跑。
 *
 * 退出码：
 *   默认      0 —— 漂移/网络失败只报告（本地与 pre-push 不因网络红）
 *   --strict  1 —— A/B 不一致或 C 检出漂移时红（定时任务与发布前用；网络失败仍只警告）。
 *
 * 用法：
 *   node scripts/check-plugin-source-pins.mjs              # 全量报告
 *   node scripts/check-plugin-source-pins.mjs --strict     # 漂移即红
 *   node scripts/check-plugin-source-pins.mjs --offline    # 只做离线校验（A/B）
 *   node scripts/check-plugin-source-pins.mjs --json       # 机器可读
 *   node scripts/check-plugin-source-pins.mjs --selftest   # 纯函数自检
 */
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PLUGIN_NAMES } from "./lib/plugin-repos.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST_REL = "scripts/lib/plugin-sources.json";
const ACTION_REL = ".github/actions/checkout-plugin-repos/action.yml";
// 所有会把插件源码当构建输入的 workflow（stage-plugin-bundles / check-plugin-contract
// / nextest 前置 staging 都在这些工作流里跑）。
const WORKFLOW_FILES = ["ci.yml", "release.yml", "test-build.yml", "test-windows-build.yml"];
const EXPECTED_ROOT = "${{ github.workspace }}/.plugin-repos";
const EXPECTED_ACTION_DIR = ".plugin-repos";
const SHA_RE = /^[0-9a-f]{40}$/;
const QUOTE = String.fromCharCode(39);

const argv = new Set(process.argv.slice(2));
const STRICT = argv.has("--strict");
const OFFLINE = argv.has("--offline");
const JSON_OUT = argv.has("--json");
const SELFTEST = argv.has("--selftest");

function read(rel) {
  return readFileSync(path.join(ROOT, rel), "utf8");
}

function unquote(value) {
  const v = value.trim();
  if (v.length >= 2 && (v[0] === QUOTE || v[0] === '"') && v[v.length - 1] === v[0]) {
    return v.slice(1, -1);
  }
  return v;
}

/** 从动作 YAML 提取每个 `actions/checkout` 步骤的 repository/ref/path（不引 YAML 依赖）。 */
export function parseActionCheckouts(text) {
  const out = [];
  let cur = null;
  let inWith = false;
  for (const raw of text.split("\n")) {
    const line = raw.replace(/\s+$/, "");
    const item = /^\s*-\s+(.*)$/.exec(line);
    if (item) {
      // 新的列表项 = 新步骤开始；上一个 checkout 步骤到此结束
      if (cur) out.push(cur);
      cur = null;
      inWith = false;
      const u = /^uses:\s*(actions\/checkout@\S+)\s*$/.exec(item[1]);
      if (u) cur = { uses: u[1], repository: null, ref: null, path: null };
      continue;
    }
    const usesOnly = /^\s+uses:\s*(actions\/checkout@\S+)\s*$/.exec(line);
    if (usesOnly) {
      if (cur) out.push(cur);
      cur = { uses: usesOnly[1], repository: null, ref: null, path: null };
      inWith = false;
      continue;
    }
    if (!cur) continue;
    if (/^\s*with:\s*$/.test(line)) {
      inWith = true;
      continue;
    }
    if (!inWith) continue;
    const kv = /^\s+(repository|ref|path):\s*(.+?)\s*$/.exec(line);
    if (kv) cur[kv[1]] = unquote(kv[2]);
  }
  if (cur) out.push(cur);
  return out;
}

/** 清单 ↔ 动作逐项对账；返回问题列表（空数组 = 通过）。 */
export function verifyPins(pins, entries) {
  const problems = [];
  const expected = new Map(pins.map((p) => [p.plugin, p]));
  const seen = new Map();
  const pathRe = /^\$\{\{\s*inputs\.path\s*\}\}\/([A-Za-z0-9._-]+)$/;
  for (const e of entries) {
    const m = pathRe.exec(e.path ?? "");
    if (!m) {
      problems.push("动作里出现非常规 path：" + String(e.path));
      continue;
    }
    const plugin = m[1];
    if (seen.has(plugin)) problems.push(plugin + "：动作里重复检出");
    seen.set(plugin, e);
  }
  for (const [plugin, e] of seen) {
    const pin = expected.get(plugin);
    if (!pin) {
      problems.push(plugin + "：动作检出了清单外的仓");
      continue;
    }
    const repo = pin.owner + "/" + pin.repo;
    if (e.repository !== repo) {
      problems.push(plugin + "：repository " + String(e.repository) + " ≠ 清单 " + repo);
    }
    if (e.ref !== pin.sha) {
      problems.push(plugin + "：ref " + String(e.ref) + " ≠ 清单 SHA " + pin.sha);
    }
  }
  for (const plugin of expected.keys()) {
    if (!seen.has(plugin)) problems.push(plugin + "：动作里没有检出（缺源即失败，禁止回退主仓副本）");
  }
  return problems;
}

/** 从 workflow 文本提取顶层 `AXAGENT_PLUGIN_REPOS_ROOT` 的声明值。 */
export function parseWorkflowRoots(text) {
  const out = [];
  for (const line of text.split("\n")) {
    const m = /^\s*AXAGENT_PLUGIN_REPOS_ROOT:\s*(.+?)\s*$/.exec(line);
    if (m) out.push(unquote(m[1]));
  }
  return out;
}

function remoteHead(pin, branch) {
  const url = "https://github.com/" + pin.owner + "/" + pin.repo + ".git";
  const r = spawnSync("git", ["ls-remote", "--heads", url, "refs/heads/" + branch], {
    encoding: "utf8",
    timeout: 30000,
    windowsHide: true,
  });
  if (r.error) return { sha: null, error: r.error.message };
  if (r.status !== 0) {
    return {
      sha: null,
      error: String(r.stderr || "").trim().split("\n")[0].slice(0, 160) || "exit " + r.status,
    };
  }
  const m = /^([0-9a-f]{40})\s+refs\/heads\//m.exec(r.stdout ?? "");
  return m ? { sha: m[1], error: null } : { sha: null, error: "ls-remote 无输出" };
}

function selftest() {
  const t = (label, ok) => {
    console.log((ok ? "  PASS " : "  FAIL ") + label);
    if (!ok) process.exitCode = 1;
  };
  const action = [
    "runs:",
    "  using: composite",
    "  steps:",
    "    - name: Checkout plugin-stock",
    "      uses: actions/checkout@v5",
    "      with:",
    "        repository: polite0803/axinvest-plugin-stock",
    "        ref: " + "a".repeat(40),
    "        path: ${{ inputs.path }}/stock",
    "        fetch-depth: 1",
    "        persist-credentials: false",
    "    - shell: bash",
    "      run: echo ok",
  ].join("\n");
  const parsed = parseActionCheckouts(action);
  t("动作解析：取到 1 个 checkout", parsed.length === 1);
  t("动作解析：repository/ref/path 齐全", parsed[0]?.repository === "polite0803/axinvest-plugin-stock" && parsed[0]?.ref === "a".repeat(40) && parsed[0]?.path === "${{ inputs.path }}/stock");
  t("动作解析：后续 shell 步骤不污染当前条目", parsed[0]?.ref === "a".repeat(40));

  const pins = [{ plugin: "stock", owner: "polite0803", repo: "axinvest-plugin-stock", sha: "a".repeat(40) }];
  t("对账：一致时零问题", verifyPins(pins, parsed).length === 0);
  t("对账：ref 漂移被抓", verifyPins(pins, [{ ...parsed[0], ref: "b".repeat(40) }]).length === 1);
  t("对账：repository 漂移被抓", verifyPins(pins, [{ ...parsed[0], repository: "x/y" }]).length === 1);
  t("对账：缺仓被抓", verifyPins(pins, []).length === 1);
  const extra = parseActionCheckouts(action.split("stock").join("opc"));
  t("对账：清单外的仓被抓", verifyPins(pins, extra).some((m) => m.includes("清单外")));

  const roots = parseWorkflowRoots("env:\n  AXAGENT_PLUGIN_REPOS_ROOT: " + EXPECTED_ROOT + "\n");
  t("workflow env 解析", roots.length === 1 && roots[0] === EXPECTED_ROOT);

  const manifest = JSON.parse(read(MANIFEST_REL));
  t("清单：17 条且插件名唯一", manifest.plugins.length === 17 && new Set(manifest.plugins.map((p) => p.plugin)).size === 17);
  t("清单：SHA 均为 40 位小写十六进制", manifest.plugins.every((p) => SHA_RE.test(p.sha)));
  t("清单：插件名与契约一致", JSON.stringify([...manifest.plugins.map((p) => p.plugin)].sort()) === JSON.stringify([...PLUGIN_NAMES].sort()));

  console.log((process.exitCode ? "FAIL" : "PASS") + " check-plugin-source-pins --selftest");
}

function main() {
  if (SELFTEST) {
    selftest();
    if (process.exitCode) process.exit(process.exitCode);
    return;
  }

  const manifest = JSON.parse(read(MANIFEST_REL));
  const pins = manifest.plugins ?? [];
  const branch = manifest.defaultBranch ?? "main";
  const problems = [];

  if (pins.length !== PLUGIN_NAMES.length) {
    problems.push("清单条数 " + pins.length + " ≠ 契约插件数 " + PLUGIN_NAMES.length);
  }
  const pinNames = pins.map((p) => p.plugin).sort();
  if (JSON.stringify(pinNames) !== JSON.stringify([...PLUGIN_NAMES].sort())) {
    problems.push("清单插件名与契约不一致：" + pinNames.join(","));
  }
  for (const p of pins) {
    if (!SHA_RE.test(p.sha)) problems.push(p.plugin + "：SHA 非法 " + p.sha);
  }

  // A. 清单 ↔ 复合动作
  const actionText = read(ACTION_REL);
  const entries = parseActionCheckouts(actionText);
  const actionProblems = verifyPins(pins, entries);
  const defaultDir = /^\s{2,}default:\s*(\S+)\s*$/m.exec(actionText)?.[1] ?? null;
  if (defaultDir !== EXPECTED_ACTION_DIR) {
    problems.push("动作 path 默认值 " + String(defaultDir) + " ≠ " + EXPECTED_ACTION_DIR);
  }

  // B. 清单 ↔ 各 workflow 的取源根
  const workflowRoots = {};
  for (const name of WORKFLOW_FILES) {
    const found = parseWorkflowRoots(read(".github/workflows/" + name));
    workflowRoots[name] = found;
    if (found.length !== 1) {
      problems.push(name + "：AXAGENT_PLUGIN_REPOS_ROOT 声明数 " + found.length + "（应为 1）");
    } else if (found[0] !== EXPECTED_ROOT) {
      problems.push(name + "：AXAGENT_PLUGIN_REPOS_ROOT = " + found[0] + " ≠ " + EXPECTED_ROOT);
    }
  }

  // C. 清单 ↔ 远端默认分支 HEAD（联网、只报告）
  const drift = [];
  const unverified = [];
  if (!OFFLINE) {
    for (const pin of pins) {
      const head = remoteHead(pin, branch);
      if (head.sha === null) unverified.push({ plugin: pin.plugin, error: head.error });
      else if (head.sha !== pin.sha) drift.push({ plugin: pin.plugin, pinned: pin.sha, head: head.sha });
    }
  }

  const allProblems = [...problems, ...actionProblems];
  const ok = allProblems.length === 0 && drift.length === 0;

  if (JSON_OUT) {
    console.log(JSON.stringify({ ok, pins: pins.length, problems: allProblems, drift, unverified, workflowRoots }, null, 2));
  } else {
    console.log("[plugin-source-pins] 清单 " + pins.length + " 条，默认分支 " + branch);
    for (const name of WORKFLOW_FILES) {
      const r = workflowRoots[name] ?? [];
      console.log("  " + (r.length === 1 && r[0] === EXPECTED_ROOT ? "✓" : "✗") + " " + name);
    }
    console.log("  " + (actionProblems.length === 0 ? "✓" : "✗") + " " + ACTION_REL + "（" + entries.length + " 个 checkout）");
    for (const p of allProblems) console.log("  ✗ " + p);
    if (OFFLINE) console.log("  - 远端比对已跳过（--offline）");
    for (const d of drift) {
      console.log("  ! 漂移 " + d.plugin + "：钉 " + d.pinned.slice(0, 12) + " ≠ 远端 " + d.head.slice(0, 12) + "（发布前手工 bump 清单）");
    }
    for (const u of unverified) console.log("  ? 未验证 " + u.plugin + "：" + u.error);
    if (!OFFLINE && drift.length === 0 && unverified.length === 0) console.log("  ✓ 17 个仓远端 HEAD 与钉版一致");
  }

  const failed = allProblems.length > 0 || (STRICT && drift.length > 0);
  if (failed) {
    if (!JSON_OUT) {
      console.log("[plugin-source-pins] FAIL（" + allProblems.length + " 处不一致" + (drift.length ? "，" + drift.length + " 处漂移" : "") + "）");
    }
    process.exit(1);
  }
  if (!JSON_OUT) console.log("[plugin-source-pins] PASS");
}

main();
