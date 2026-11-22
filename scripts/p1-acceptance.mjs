#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * AxInvest Bot 集成 P1 —— 一条命令的标准验收。
 *
 * 为什么需要它：P1 的门禁分散在 npm scripts、Python、Rust 与 Playwright 四处，
 * 手工逐条跑容易漏项；更糟的是「跑了一半就以为全绿」。本脚本按固定顺序跑，
 * 记录每一步的**命令原文 / 退出码 / 耗时 / 输出尾部**，最后给一张判定表，
 * 并把完整日志写到 `docs/bot-integration/evidence/`。
 *
 * 用法：
 *   node scripts/p1-acceptance.mjs            # 默认：静态门禁 + typecheck + 单测
 *   node scripts/p1-acceptance.mjs --quick    # 只跑静态门禁（秒级，改文案时用）
 *   node scripts/p1-acceptance.mjs --full     # 再加 cargo（migration/dao/agent）+ Bot e2e
 *   node scripts/p1-acceptance.mjs --e2e      # 只加 Bot e2e
 *
 * 退出码：0 = 全绿；1 = 任一红项。
 */

import { spawnSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const QUICK = argv.includes("--quick");
const FULL = argv.includes("--full");
const WITH_E2E = FULL || argv.includes("--e2e");

const STATIC_STEPS = [
  {
    name: "契约（A/C/D/F/G/H + BOT）",
    cmd: "node scripts/check-contracts.mjs --only=a,c,d,f,g,h,bot",
  },
  {
    name: "错误码 ↔ 11 语言",
    cmd: "node scripts/check-errorcode-alignment.mjs",
  },
  {
    name: "i18n 严格模式（zh-CN 空值 / t() 引用）",
    cmd: "python3 scripts/check_i18n.py --strict",
  },
  {
    name: "i18n key 集合（zh-CN ↔ 其余 10 语言）",
    cmd: "node scripts/check-i18n-key-parity.mjs",
  },
  {
    name: "i18n Bot 必需 key（11 语言）",
    cmd: "node scripts/check-bot-i18n-keys.mjs",
  },
  {
    name: "分层纪律（命令层不得直连 SeaORM）",
    cmd: "node scripts/check-layer-discipline.mjs",
  },
  {
    name: "Serde 注解一致性",
    cmd: "node scripts/check-serde-annotations.ts",
  },
  {
    name: "路由可达性（只加 Route 不加 nav 会红）",
    cmd: "npm run check:routes",
  },
];

const CODE_STEPS = [
  { name: "TypeScript typecheck", cmd: "npm run typecheck" },
  { name: "Vitest 全量单测", cmd: "node scripts/vitest-run.mjs run" },
];

const CARGO_STEPS = [
  {
    name: "Rust: migration（声明式建表）",
    cmd: "cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-migration",
  },
  {
    name: "Rust: schema_compliance（bots 表 / bot_id 列登记）",
    cmd: "cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance",
  },
  {
    name: "Rust: axagent-agent 单测（BotWorkspace / 提示词 slot）",
    cmd: "cargo test --manifest-path axagent-ai/Cargo.toml -p axagent-agent",
  },
];

const E2E_STEP = {
  name: "Playwright: Bot 归属层（浏览器模式）",
  cmd: "npx playwright test e2e/bots.spec.ts --project=chromium --grep-invert \"@heavy\" --timeout=120000",
};

const steps = [
  ...(QUICK ? [] : CODE_STEPS),
  ...STATIC_STEPS,
  ...(FULL ? CARGO_STEPS : []),
  ...(WITH_E2E ? [E2E_STEP] : []),
];

const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const evidenceDir = join(ROOT, "docs", "bot-integration", "evidence");
if (!existsSync(evidenceDir)) {
  mkdirSync(evidenceDir, { recursive: true });
}
const logPath = join(evidenceDir, `p1-acceptance-${stamp}.log`);
appendFileSync(logPath, `# AxInvest Bot P1 acceptance — ${new Date().toISOString()}\n`);
appendFileSync(logPath, `# cwd: ${ROOT}\n\n`);

function runStep(step) {
  const started = Date.now();
  console.log(`\n▶ ${step.name}\n  $ ${step.cmd}`);
  const res = spawnSync(step.cmd, {
    cwd: ROOT,
    shell: true,
    encoding: "utf8",
    timeout: 30 * 60 * 1000,
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, CI: "1" },
  });
  const ms = Date.now() - started;
  const out = `${res.stdout ?? ""}${res.stderr ?? ""}`;
  const code = res.status ?? 1;
  const tail = out.trimEnd().split("\n").slice(-25).join("\n");
  appendFileSync(logPath, `$ ${step.cmd}\nexit=${code}  ${(ms / 1000).toFixed(1)}s\n${out}\n\n`);
  if (tail) {
    console.log(tail);
  }
  console.log(`  → exit ${code}（${(ms / 1000).toFixed(1)}s）`);
  return { ...step, code, ms };
}

console.log(`AxInvest Bot P1 验收${QUICK ? "（quick）" : FULL ? "（full）" : ""}`);
console.log(`日志: ${logPath}`);

const results = steps.map(runStep);

const width = Math.max(...results.map((r) => r.name.length), 12);
console.log(`\n=== P1 验收判定（${results.length} 项）===`);
for (const r of results) {
  console.log(
    `  ${r.code === 0 ? "PASS" : "FAIL"}  ${r.name.padEnd(width)}  exit=${r.code}  ${(r.ms / 1000).toFixed(1)}s`,
  );
}
const failed = results.filter((r) => r.code !== 0);
console.log(
  `\n结果: ${failed.length === 0 ? "PASS —— P1 门禁全绿" : `FAIL —— ${failed.length} 项红：${failed.map((r) => r.name).join(" / ")}`}`,
);
console.log(`证据日志: ${logPath}`);
process.exit(failed.length === 0 ? 0 : 1);
