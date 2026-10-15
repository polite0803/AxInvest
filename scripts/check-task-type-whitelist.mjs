// SPDX-License-Identifier: AGPL-3.0-only
//
// 内置 task_type 白名单的**跨语言对账门**（2026-10-15，PLAN-monitor-chain-closure 裁定 17）。
//
// ── 为什么必须有这道门 ──
// 同一个 task_type 要同时登记在三处手抄清单里：
//   ① 后端 `src-tauri/src/commands/scheduled_task.rs::BUILTIN_TASK_TYPES`（「立即执行」白名单）
//   ② 前端 `src/pages/TasksCenterPage/taskGrouping.ts::BUILTIN_EXECUTABLE_TYPES`（①的手抄副本，
//      `canExecuteNow` 用它决定按钮灰不灰）
//   ③ 前端 `taskGrouping.ts::INVEST_TASK_TYPES` / `SYSTEM_TASK_TYPES`（任务中心分组）
// 2026-10-15 实测 `lesson-validation` 漏登 ②③ 两处**数月**：
//   · 漏 ② ⇒ 手动执行报 `MANUAL_RUN_NOT_SUPPORTED`，而**到点自动执行照常** ⇒ 只在手动触发时露头；
//   · 漏 ③ ⇒ 任务归进「自定义」组，删除确认也不再提示「重启会重建」。
// 三处之间此前只有 Rust 侧的「种子 ⇒ ①」真值对表（在 `stock_automation_cron_seed.rs` 的
// `every_builtin_job_is_manually_executable`），**②③ 与 ① 之间没有任何东西在管**。
//
// ── 为什么不在 `check-contracts.mjs` 里 ──
// 那份脚本当前**不在 CI 的 35 道门里**（只 `npm run contracts` 手动跑）⇒ 写在那里等于没牙齿。
//
// ── 判据 ──
//   I1 ①的值集合 ≡ ②的值集合（双向，逐值点名：少一条报「手动执行会报错」、多一条报「前端放行后端拒绝」）
//   I2 ①的每个值都能被任务中心三组之一接走（投资 / `opc` 前缀 / 系统），不得落「自定义」兜底
//   I3 ①的每个值都能在 Rust 源码里逐字定位（`*_TASK_TYPE` 常量定义或字面量）—— 解析面自证
//   任一解析锚点失效 ⇒ 报「判据没电」判红，**不许由「解析不到」推成「没问题」**
//
// ── 用法 ──
//   node scripts/check-task-type-whitelist.mjs             # 门禁（0/1）
//   node scripts/check-task-type-whitelist.mjs --selftest  # 负控全集（五个坏样本，逐条必须真红）
//
// 退出码：0 通过 ｜ 1 违规（含判据没电）

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const TASK_REL = "src-tauri/src/commands/scheduled_task.rs";
const GROUPING_REL = "src/pages/TasksCenterPage/taskGrouping.ts";
const SRC_TAURI = join(ROOT, "src-tauri");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

/** 全部 `.rs`（用于解析常量定义与 I3 的逐字来源），排除构建产物与备份目录。 */
function collectRustSources() {
  const out = [];
  const skip = new Set(["target", "output", "node_modules", ".git", ".worktrees"]);
  const walk = (dir) => {
    let names;
    try {
      names = readdirSync(dir);
    } catch {
      return;
    }
    for (const name of names) {
      if (skip.has(name)) {
        continue;
      }
      const full = join(dir, name);
      const st = statSync(full);
      if (st.isDirectory()) {
        walk(full);
      } else if (name.endsWith(".rs")) {
        out.push([full.slice(SRC_TAURI.length + 1).replaceAll("\\", "/"), readFileSync(full, "utf8")]);
      }
    }
  };
  walk(SRC_TAURI);
  return out;
}

/** 从 `taskGrouping.ts` 读一个 `const NAME = new Set([...])` 的字面量集合。 */
export function readTsSet(src, name) {
  const m = src.match(new RegExp(`const\\s+${name}\\s*=\\s*new Set\\(\\[([\\s\\S]*?)\\]\\)`));
  if (!m) {
    return null;
  }
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** 从 `scheduled_task.rs` 读 `BUILTIN_TASK_TYPES` 的条目（字面量或常量路径）。 */
export function readBackendTaskTypes(src) {
  const m = src.match(/const BUILTIN_TASK_TYPES:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/);
  if (!m) {
    return null;
  }
  return [...m[1].matchAll(/"([^"]+)"|([A-Za-z_][\w:]*)\s*(?=,|\n)/g)]
    .map((x) => (x[1] ? { literal: x[1] } : { constPath: x[2] }))
    .filter((x) => x.literal || /::/.test(x.constPath ?? ""));
}

/** 把常量名（`X::Y::NAME` 的末段）解析成 `pub const NAME: &str = "值";` 的值。 */
export function resolveConstValue(name, sources) {
  const re = new RegExp(`pub const ${name}\\s*:\\s*&str\\s*=\\s*"([^"]+)"`);
  for (const [, src] of sources) {
    const m = src.match(re);
    if (m) {
      return m[1];
    }
  }
  return null;
}

/**
 * 纯函数判定：给定三份源码文本，返回问题清单（空数组 = 一致）。
 * 拆成纯函数是为了负控能**在内存里**造破坏样本、不落盘（selftest 与门禁走同一份判据）。
 */
export function checkWhitelist({ taskSrc, groupingSrc, rustSources }) {
  const problems = [];
  const entries = readBackendTaskTypes(taskSrc);
  if (!entries || entries.length === 0) {
    return ["判据没电：后端 BUILTIN_TASK_TYPES 解析为空（锚点失效或被改名）"];
  }
  const execTypes = readTsSet(groupingSrc, "BUILTIN_EXECUTABLE_TYPES");
  const investTypes = readTsSet(groupingSrc, "INVEST_TASK_TYPES");
  const systemTypes = readTsSet(groupingSrc, "SYSTEM_TASK_TYPES");
  if (!execTypes || !investTypes || !systemTypes) {
    return ["判据没电：前端三个 Set 至少有一个解析失败（锚点失效或被改名）"];
  }

  const backend = [];
  const unresolved = [];
  for (const e of entries) {
    if (e.literal) {
      backend.push(e.literal);
      continue;
    }
    const name = e.constPath.split("::").pop();
    const value = resolveConstValue(name, rustSources);
    if (value === null) {
      unresolved.push(`${e.constPath}（${name} 的 pub const 定义没找到）`);
    } else {
      backend.push(value);
    }
  }
  if (unresolved.length) {
    return [`判据没电：后端白名单里这些条目解析不到值 ⇒ ${unresolved.join("；")}`];
  }
  if (backend.length === 0) {
    return ["判据没电：后端白名单解析为空"];
  }

  // I1 双向相等
  const beSet = new Set(backend);
  const feSet = new Set(execTypes);
  const missingInFrontend = backend.filter((v) => !feSet.has(v));
  const extraInFrontend = execTypes.filter((v) => !beSet.has(v));
  if (missingInFrontend.length) {
    problems.push(
      `后端「立即执行」白名单里的 ${missingInFrontend.join(", ")} 不在前端 BUILTIN_EXECUTABLE_TYPES ⇒ ` +
        `手动执行会报 MANUAL_RUN_NOT_SUPPORTED（到点自动执行照常，只在手动触发时露头）`,
    );
  }
  if (extraInFrontend.length) {
    problems.push(`前端 BUILTIN_EXECUTABLE_TYPES 多了后端没有的 ${extraInFrontend.join(", ")} ⇒ 前端放行、后端拒绝`);
  }

  // I2 归组：投资 / opc 前缀 / 系统，三者之一
  const ungrouped = backend.filter(
    (v) => !investTypes.includes(v) && !v.startsWith("opc") && !systemTypes.includes(v),
  );
  if (ungrouped.length) {
    problems.push(
      `这些 task_type 在任务中心归不进任何内置组（会落「自定义」兜底、删除确认不再提示会重建）：` +
        ungrouped.join(", "),
    );
  }

  // I3 逐字来源（常量被删/改名时，值在源码里就找不到字面量）
  const allRust = rustSources.map(([, s]) => s).join("\n");
  const unattested = backend.filter((v) => !allRust.includes(`"${v}"`));
  if (unattested.length) {
    problems.push(`这些值在 Rust 源码里找不到字面量来源（常量定义被删或改名）：${unattested.join(", ")}`);
  }

  return problems;
}

function selftest() {
  const taskSrc = read(TASK_REL);
  const groupingSrc = read(GROUPING_REL);
  const rustSources = collectRustSources();
  const patch = (src, from, to) => {
    if (!src.includes(from)) {
      throw new Error(`负控锚点未命中：${from.slice(0, 60)}`);
    }
    return src.replace(from, to);
  };

  const clean = checkWhitelist({ taskSrc, groupingSrc, rustSources });
  if (clean.length > 0) {
    console.error("❌ 正控失败（现网就该绿）：\n" + clean.join("\n"));
    return 1;
  }

  const cases = [
    [
      "后端白名单删一条 ⇒ 前端多出该值",
      checkWhitelist({
        // 锚点须与 `scheduled_task.rs` 的**当前常量路径**逐字一致：
        // `LESSON_VALIDATION_TASK_TYPE` 已从 `stock_workflow::reflection` 挪到 `commands::constants`
        // （2026-10-15 `e3ea43eca` 下沉 dao 那批），旧路径锚点会直接抛「负控锚点未命中」。
        taskSrc: patch(taskSrc, "    crate::commands::constants::LESSON_VALIDATION_TASK_TYPE,\n", ""),
        groupingSrc,
        rustSources,
      }).some((p) => p.includes("多了后端没有的") && p.includes("lesson-validation")),
    ],
    [
      "前端可执行集删一条 ⇒ 后端那条没用处",
      checkWhitelist({
        taskSrc,
        groupingSrc: patch(
          groupingSrc,
          // 锚点必须带上下文：两个 Set 的结尾三行**逐字相同**（`"portfolio-metrics", "intent-expiry", ]);`），
          // 只写那三行会替换到投资组那份 —— 断言就会指向另一条判据（当晚实测）
          '  // 报 MANUAL_RUN_NOT_SUPPORTED，而到点自动执行照常 ⇒ 缺陷只在手动触发时露头）\n  "lesson-validation",\n  "portfolio-metrics",\n  "intent-expiry",\n]);',
          '  // 报 MANUAL_RUN_NOT_SUPPORTED，而到点自动执行照常 ⇒ 缺陷只在手动触发时露头）\n  "lesson-validation",\n  "portfolio-metrics",\n]);',
        ),
        rustSources,
      }).some((p) => p.includes("不在前端 BUILTIN_EXECUTABLE_TYPES")),
    ],
    [
      "前端投资组删一条 ⇒ 归不进任何内置组",
      checkWhitelist({
        taskSrc,
        groupingSrc: patch(groupingSrc, '  "portfolio-metrics",\n', ""),
        rustSources,
      }).some((p) => p.includes("归不进任何内置组")),
    ],
    [
      "常量值被改名 ⇒ 两侧集合不再相等（且点名新值）",
      checkWhitelist({
        taskSrc,
        groupingSrc,
        rustSources: rustSources.map(([rel, src]) =>
          rel.endsWith("commands/constants.rs") ? [rel, patch(src, '"portfolio-metrics";', '"portfolio-metrics-x";')] : [rel, src]
        ),
      }).some((p) => p.includes("portfolio-metrics-x")),
    ],
    [
      "锚点被破坏（`&[&str]` 改名）⇒ 报「判据没电」而不是静默通过",
      checkWhitelist({
        taskSrc: patch(taskSrc, "const BUILTIN_TASK_TYPES: &[&str] = &[", "const BUILTIN_TASK_TYPES: &[String] = &["),
        groupingSrc,
        rustSources,
      }).some((p) => p.startsWith("判据没电")),
    ],
  ];

  let pass = 0;
  for (const [label, ok] of cases) {
    console.log(`${ok ? "✓" : "✗"} 负控 ${label}（期望红｜实测${ok ? "红" : "绿"}）`);
    if (ok) {
      pass += 1;
    }
  }
  console.log(`\n负控 ${pass}/${cases.length} 通过`);
  return pass === cases.length ? 0 : 1;
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1].replaceAll("\\", "/")}`).href) {
  const selftestMode = process.argv.includes("--selftest");
  if (selftestMode) {
    process.exitCode = selftest();
  } else {
    const problems = checkWhitelist({
      taskSrc: read(TASK_REL),
      groupingSrc: read(GROUPING_REL),
      rustSources: collectRustSources(),
    });
    if (problems.length) {
      console.log(`❌ task_type 白名单不一致（${problems.length} 处）：`);
      problems.forEach((p) => console.log(`  · ${p}`));
      process.exitCode = 1;
    } else {
      console.log("✅ 内置 task_type 白名单三处一致（后端「立即执行」≡ 前端可执行 ⊆ 任务中心三组）");
    }
  }
}
