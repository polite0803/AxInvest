#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 决策 alpha 离线报告入口（只读，不调任何 LLM）。
//
//   node scripts/decision-alpha-report.mjs          # 打印原始 JSON
//   node scripts/decision-alpha-report.mjs --md     # 打印 Markdown 表
//
// SQL 在 `scripts/decision-alpha-report.sql`（与脚本同目录，改口径只改那里）。
// 连接复用 `scripts/pg-connect.mjs` —— 口令从 `~/.axagent` 解密，本文件不含任何密钥。
//
// ## 为什么存在
//
// 2026-10-28 的结论是「没有可用的正期望子集」，但那是**一次性查询**：样本、口径、
// 分桶全在对话里，谁都无法复跑。固化成本文件后，只要 `decision_validations` 有新结算，
// 同一条命令就能重算，日/周对比才算得出来。
//
// ## 判读纪律
//
// · `span.days` < 60 时，任何分桶结论都只是假设（横截面厚 ≠ 时间维度够）；
// · `rules[].days_positive / days` 是防过拟合的关键：某条规则靠 2–3 天撑起来的
//   `avg_excess` 不可用；
// · `avg_excess` 与 `avg_ret` 要一起看：超额为正但绝对收益为负，说明它只是「跌得比
//   同组少」，不能当买入信号。

import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SQL_PATH = join(HERE, "decision-alpha-report.sql");
const PG_CONNECT = join(HERE, "pg-connect.mjs");

function fail(msg) {
  process.stderr.write(`decision-alpha-report: ${msg}\n`);
  process.exit(1);
}

function runSql() {
  // SQL 走 `--file` 而不是 argv：Windows 下 argv 里的中文会被控制台代码页转坏
  // （实测 `invalid byte sequence for encoding "UTF8"`），`-f` 才是可靠路径。
  const res = spawnSync(process.execPath, [PG_CONNECT, "sql", `--file=${SQL_PATH}`], {
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  if (res.error) fail(`调用 pg-connect 失败：${res.error.message}`);
  if (res.status !== 0) {
    fail(`pg-connect 退出码 ${res.status}\n${res.stderr || ""}`);
  }
  return res.stdout;
}

/** psql `-A -F tab` 会先打一行列名（这里是 `jsonb_pretty`），剥掉后才是 JSON。 */
function extractJson(stdout) {
  const text = stdout.replace(/\r\n/g, "\n").trim();
  const start = text.indexOf("{");
  if (start < 0) fail(`输出里没有 JSON：\n${text.slice(0, 400)}`);
  return JSON.parse(text.slice(start));
}

function fmt(v, digits = 2) {
  if (v === null || v === undefined) return "—";
  if (typeof v === "number") return v.toFixed(digits);
  return String(v);
}

function renderMarkdown(r) {
  const out = [];
  const span = r.span ?? {};
  out.push("## 样本覆盖");
  out.push(
    `已结算 ${span.rows_settled} 条，进基准 ${span.rows_with_bench} 条；`
      + `覆盖 **${span.days} 个交易日**（${span.first_day} → ${span.last_day}），`
      + `持有期 ${span.trade_days}。`,
  );
  if ((span.days ?? 0) < 60) {
    out.push("");
    out.push(`> ⚠ 时间维度只有 ${span.days} 天，分桶结论一律按假设处理。`);
  }

  const b = r.baseline ?? {};
  out.push("");
  out.push("## 基线");
  out.push("| n | 绝对收益 | 超额 | 绝对胜率 | 胜基准 | t |");
  out.push("|---:|---:|---:|---:|---:|---:|");
  out.push(
    `| ${b.n} | ${fmt(b.avg_ret)}% | ${fmt(b.avg_excess)}% | ${fmt(b.win_abs, 1)}% | `
      + `${fmt(b.win_vs_day, 1)}% | ${fmt(b.t_stat)} |`,
  );

  const section = (title, rows, cols) => {
    if (!rows?.length) return;
    out.push("");
    out.push(`## ${title}`);
    out.push(`| ${cols.map((c) => c.label).join(" | ")} |`);
    out.push(`|${cols.map(() => "---:").join("|")}|`);
    for (const row of rows) {
      out.push(`| ${cols.map((c) => fmt(row[c.key], c.digits)).join(" | ")} |`);
    }
  };

  section("按持有期", r.by_horizon, [
    { key: "t", label: "T+N", digits: 0 },
    { key: "n", label: "n", digits: 0 },
    { key: "avg_ret", label: "绝对收益" },
    { key: "win_abs", label: "胜率", digits: 1 },
  ]);

  section("按周期档", r.by_period, [
    { key: "period", label: "档" },
    { key: "t", label: "T+N", digits: 0 },
    { key: "n", label: "n", digits: 0 },
    { key: "avg_ret", label: "绝对收益" },
    { key: "avg_excess", label: "超额" },
    { key: "t_stat", label: "t" },
  ]);

  section("按置信度", r.by_confidence, [
    { key: "band", label: "置信" },
    { key: "n", label: "n", digits: 0 },
    { key: "avg_excess", label: "超额" },
    { key: "t_stat", label: "t" },
  ]);

  section("规则候选", r.rules, [
    { key: "rule", label: "规则" },
    { key: "n", label: "n", digits: 0 },
    { key: "avg_ret", label: "绝对收益" },
    { key: "avg_excess", label: "超额" },
    { key: "win_abs", label: "胜率", digits: 1 },
    { key: "days_positive", label: "正收益天数", digits: 0 },
    { key: "days", label: "总天数", digits: 0 },
  ]);

  out.push("");
  out.push("> 判读：`avg_excess > 0` 但 `avg_ret < 0` ＝「跌得比同组少」，不是买入信号；");
  out.push("> `days_positive / days` 低于六成的规则，视为少数交易日撑起的噪声。");
  return out.join("\n");
}

const result = extractJson(runSql());

if (process.argv.includes("--md")) {
  process.stdout.write(renderMarkdown(result) + "\n");
} else {
  process.stdout.write(JSON.stringify(result, null, 2) + "\n");
}
