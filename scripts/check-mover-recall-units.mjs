// SPDX-License-Identifier: AGPL-3.0-only
//
// 漏检补分析阈值**单位一致性门**（2026-10-22，PLAN-mover-recall-defect-closure 阶段 1）。
//
// ── 为什么必须有这道门 ──
// `mover_recall_runs.rates_json.unexplainedShare` 是 `compute_rates` 输出的**百分数**
// （0-100，例如 45.45），而补分析请求的 `max_unexplained_share` 是 **0-1**（默认 0.5）。
// 2026-10-22 实测：`select_events` 直接比较两者 ⇒ 实际阈值变成 0.5%（而非 50%），
// 10-21 批次 total=0、10-22 批次只选 6 条、467 条 reco 事件 0 处理。
// 空态文案还把 `0.5 * 100` 显示成「≥ 50%」，把单位错配藏成了合理阈值。
//
// ── 判据（读源码结构，不锁实现写法）──
//   U1 `load_run_unexplained_shares` 必须经 `normalize_unexplained_share(...)` 出值
//   U2 `normalize_unexplained_share` 函数体必须有归一化常量 `/ 100.0` 或 `* 0.01`
//   U3 `select_events` 必须经 `share_blocks(...)` 判阈值，不得直接拿 share 与请求阈值比
//   U4 两处自证测试必须在（`unexplained_share_is_normalized_to_unit_interval` /
//      `share_threshold_is_unit_interval`）——判据没电时不能恒绿
//
// ── 用法 ──
//   node scripts/check-mover-recall-units.mjs             # 门禁（0/1）
//   node scripts/check-mover-recall-units.mjs --selftest  # 三条负控 + 原样正控
//
// 退出码：0 通过 ｜ 1 违规（含判据没电）

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DAO_REL = "src-tauri/crates/dao/src/repo/mover_recall_events.rs";
const CMD_REL = "src-tauri/src/commands/mover_recall_analysis.rs";

const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

/** 取函数体：从 `fn NAME(` 到行首的 `}`（函数内嵌套块都带缩进）。 */
export function bodyOf(src, signature) {
  const start = src.indexOf(signature);
  if (start < 0) {
    return null;
  }
  const rel = src.slice(start);
  const end = rel.indexOf("\n}");
  return end < 0 ? null : rel.slice(0, end);
}

/**
 * 纯函数判定：返回问题清单（空数组 = 一致）。
 * 拆成纯函数是为了负控能在内存里造破坏样本、不落盘。
 */
export function checkUnits({ dao, cmd }) {
  const problems = [];

  const loadFn = bodyOf(dao, "pub async fn load_run_unexplained_shares");
  if (!loadFn) {
    return ["判据没电：DAO 里解析不到 load_run_unexplained_shares（锚点失效或改名）"];
  }
  if (!loadFn.includes("normalize_unexplained_share(")) {
    problems.push(
      "U1 load_run_unexplained_shares 没走 normalize_unexplained_share ⇒ rates_json 的百分数可能原样泄漏给 0-1 阈值比较",
    );
  }

  const normFn = bodyOf(dao, "fn normalize_unexplained_share");
  if (!normFn) {
    return [...problems, "判据没电：解析不到 normalize_unexplained_share（归一化入口被删或改名）"];
  }
  if (!normFn.includes("/ 100.0") && !normFn.includes("/100.0") && !normFn.includes("* 0.01")) {
    problems.push("U2 normalize_unexplained_share 没有归一化常量（/ 100.0 / * 0.01 至少一个）");
  }

  const selectFn = bodyOf(cmd, "async fn select_events");
  if (!selectFn) {
    return [...problems, "判据没电：解析不到 select_events（锚点失效或改名）"];
  }
  if (!selectFn.includes("share_blocks(")) {
    problems.push("U3 select_events 没经 share_blocks 判阈值 ⇒ 单位可能再次直接相撞");
  }
  if (selectFn.includes(">= request.max_unexplained_share")) {
    problems.push("U3 select_events 出现 share 与 request.max_unexplained_share 的直接比较（绕过归一化入口）");
  }

  for (const name of [
    "unexplained_share_is_normalized_to_unit_interval",
    "share_threshold_is_unit_interval",
  ]) {
    if (!dao.includes(`fn ${name}`) && !cmd.includes(`fn ${name}`)) {
      problems.push(`U4 缺自证测试 ${name} ⇒ 判据是否真的有电无法验证`);
    }
  }

  return problems;
}

function selftest() {
  const dao = read(DAO_REL);
  const cmd = read(CMD_REL);
  const cases = [];

  cases.push({
    name: "正控：当前源码必须通过",
    input: { dao, cmd },
    want: 0,
  });

  const noNormalize = dao.replace("/ 100.0", " * 1.0");
  if (noNormalize === dao) {
    throw new Error("负控锚点未命中：/ 100.0");
  }
  cases.push({
    name: "负控 U2：删掉归一化常量 ⇒ 必须红",
    input: { dao: noNormalize, cmd },
    want: 1,
  });

  const directCompare = cmd.replace(
    ".all(|m| share_blocks(shares.get(&m.run_id).copied(), request.max_unexplained_share))",
    ".all(|m| shares.get(&m.run_id).is_some_and(|s| *s >= request.max_unexplained_share))",
  );
  if (directCompare === cmd) {
    throw new Error("负控锚点未命中：share_blocks 调用行");
  }
  cases.push({
    name: "负控 U3：比较点绕开 share_blocks ⇒ 必须红",
    input: { dao, cmd: directCompare },
    want: 2,
  });

  const noTest = dao.replace("fn unexplained_share_is_normalized_to_unit_interval", "fn zzz_gone");
  if (noTest === dao) {
    throw new Error("负控锚点未命中：dao 自证测试名");
  }
  cases.push({
    name: "负控 U4：删掉归一化自证测试 ⇒ 必须红",
    input: { dao: noTest, cmd },
    want: 1,
  });

  let failed = 0;
  for (const c of cases) {
    const problems = checkUnits(c.input);
    const ok = problems.length === c.want;
    console.log(`${ok ? "✓" : "✗"} ${c.name}（问题数 ${problems.length}，期望 ${c.want}）`);
    if (!ok) {
      failed += 1;
      console.log(`   ${problems.join("\n   ")}`);
    }
  }
  if (failed > 0) {
    console.error(`\n单位门自检失败：${failed}/${cases.length}`);
    process.exit(1);
  }
  console.log(`\n单位门自检通过：${cases.length}/${cases.length}`);
}

if (process.argv.includes("--selftest")) {
  selftest();
} else {
  const problems = checkUnits({ dao: read(DAO_REL), cmd: read(CMD_REL) });
  if (problems.length > 0) {
    console.error("mover_recall 阈值单位门：违规");
    for (const p of problems) {
      console.error(`  - ${p}`);
    }
    process.exit(1);
  }
  console.log("mover_recall 阈值单位门：通过（U1–U4）");
}
