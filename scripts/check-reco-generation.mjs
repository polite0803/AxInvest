#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 荐股候选池「代际一致」门 —— 防回放行与实盘行同表后被混读混写（2026-10-13）。
//
// ## 为什么要有这道门
// 2026-10-12 裁定：智能荐股与趋势智选在 as-of（回放）下取到的候选**必须持久化**，
// 否则回放结束后没有样本可供反思。实现形态是「同表 + 每行带 `as_of_date` 代际标记」，
// 而不是恢复 2026-10-11 那轮的「回放跳过写库」守卫 —— 后者满足不混码，但不满足可反思。
//
// 代价是：从此一切「只看实盘」的读侧都必须显式带代际谓词，漏一处就等于把历史口径的
// 候选当成今天真出的票（缓存回读、趋势智选历史、退出信号/反馈、池分析、漏检核查都是）。
// 这类"新增消费者忘了过滤"的断裂在 UI 上读不出来，只能由门守着。
//
// ## 规则（粒度 = 顶层函数段，剥注释后段内空白归一）
// R1 读侧：段内出现 `reco_picks` / `reco_scan_audit` 的取数形态 ⇒ 同段必须出现代际谓词。
// R2 写侧：段内出现 `persist_reco_picks(` / `persist_reco_scan_audit(` / `pick.insert(db)`
//          ⇒ 同段必须出现 `as_of_date`（写入字段或参数），否则等于落库后无法归代。
// R3 闭环：段内出现 `decision_validations::Entity::find()` ⇒ 同段必须能读出处置代际
//          （谓词或逐行 `as_of_date`），防两套计数域被并成一个命中率分母。
// 豁免：必须写明理由，**每次运行都打印**；豁免不是清零，是带日期的显式债务。
// 自证：`--selftest` 含拆谓词负控、"谓词只写在注释里"负控、假改造（包成 Some(..) 但没过滤）
//       仍须命中、以及扫描面为空即红。
//
// 用法：node scripts/check-reco-generation.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// 复用 as-of 接线门的剥注释与分段实现（禁区 12：不另造一份状态机）
import { normalize, splitTopLevelFns, stripRustComments } from "./check-asof-node-scope.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCAN_DIRS = ["src-tauri/src", "src-tauri/crates"];
const SKIP_DIRS = new Set([
  "node_modules",
  "dist",
  "target",
  "target2",
  ".worktrees",
  "output",
  ".qoder",
  ".git",
  "test-results",
  "playwright-report",
]);

/** R1：取 reco_picks / reco_scan_audit 的调用形态。 */
const READ_FORMS = [
  "reco_picks::Entity::find()",
  "reco_picks::Entity::find_by_id(",
  "reco_scan_audit::Entity::find()",
  "FROM reco_picks",
];
/** R1/R3 认的代际谓词或代际读出形态。 */
const GENERATION_FORMS = [
  "Column::AsOfDate.is_null()",
  "Column::AsOfDate.is_not_null()",
  "Column::AsOfDate.eq(",
  "as_of_date IS NULL",
  "as_of_date IS NOT NULL",
  "as_of_date =",
  "as_of_date:",
  "as_of_date.",
  "as_of_date,",
];
/** R2：写 reco_picks / reco_scan_audit 的落库形态。 */
const WRITE_FORMS = [
  "persist_reco_picks(",
  "persist_reco_scan_audit(",
  "pick.insert(db)",
];
/**
 * R2 的证据必须落在**调用点前后各一窗**内，不是"整段里某处出现过"。
 * 方向要双向：落库前构造 ActiveModel（`as_of_date: Set(...)`）在 `insert` 之前，
 * 而 cron 入口传的代际实参在调用括号里 —— 只查一侧会把合法形态判成违规。
 * 之所以不许"段内任意位置"：裸 `None` 与"忘了传代际"完全同形，
 * 只有落在调用点近邻的具名代际常量或 `as_of*` 实参，才证明这里想过代际。
 */
const CALL_WINDOW = 400;
function evidenceAtCallSite(body, form) {
  const at = body.indexOf(form);
  if (at === -1) return null;
  const win = body.slice(Math.max(0, at - CALL_WINDOW), at + CALL_WINDOW);
  return win.includes("as_of") || win.includes("GENERATION") ? "调用点近邻含代际" : null;
}
/** R3：读决策验证表（荐股 IC/权重闭环的样本源）。 */
const VALIDATION_READ_FORMS = ["decision_validations::Entity::find()"];

/**
 * 显式豁免（键 = 仓库相对路径 + 函数名）。每条都必须带理由；门每次运行打印全表。
 * 加一条豁免 = 承认一处未收口的债务，不是让它消失。
 */
const EXEMPT = new Map([
  [
    "src-tauri/src/commands/backtest_validation.rs|write_decision_validations",
    "R3：幂等查重按 (pick_id, t_plus_n)，pick_id 全局唯一 ⇒ 代际已由键本身决定，再加谓词是重复表达",
  ],
  [
    "src-tauri/src/commands/stock_workflow/reco_history.rs|get_reco_detail",
    "R1：按 generated_at 取整批 —— 批次键即代际（实盘与回放各写各的批），不重复加谓词",
  ],
  [
    "src-tauri/src/commands/stock_workflow/reco_history.rs|batch_delete_reco_history",
    "R1：同上，删除按批次键",
  ],
  [
    "src-tauri/src/commands/stock_workflow/reco_history.rs|delete_serenity_pick",
    "R1：find_by_id 单行读删，主键即代际",
  ],
]);

/**
 * 命中数下限（**不是**「门绿的充分条件」，只是「扫描面没缩水」的必要条件）。
 * 基线取自 2026-10-13 现网：改动前用 `--dump` 复算，若低于下限说明扫描面或判据被削弱。
 */
const MIN_HITS = { R1: 14, R2: 6, R3: 5 };

function listRustFiles(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      out.push(...listRustFiles(path.join(dir, entry.name)));
    } else if (entry.name.endsWith(".rs")) {
      out.push(path.join(dir, entry.name));
    }
  }
  return out;
}

function anyIn(body, forms) {
  return forms.find((f) => body.includes(f)) ?? null;
}

/**
 * R3 的证据只认真正的**按代筛**，不认「把字段读出来」：
 *  - SQL 下推形态：`Column::AsOfDate.*`
 *  - 内存切分形态：`as_of_date.is_none()` / `is_some()` / `as_of_date.as_deref() ==`
 *    （报表要同时给出「库里另有几条回放样本」，就必须读全表再按代切 —— 只允许 SQL
 *     谓词会逼出「两次查询拼一个数」，反而更难自证）
 * 带出代际 ≠ 处置代际：只把 `as_of_date` 拷进 DTO、仍把两代算进同一个分母，两族形态
 * 都不出现 ⇒ 判红（自测负控 7 锁的就是这个）。 */
const R3_PREDICATES = [
  "Column::AsOfDate.is_null()",
  "Column::AsOfDate.is_not_null()",
  "Column::AsOfDate.eq(",
  "as_of_date.is_none()",
  "as_of_date.is_some()",
  "as_of_date.as_deref() ==",
];

/**
 * 段级判据的**唯一实现** —— `checkSources` 与 `selftest` 都走这里。
 * 分成两份各写一遍的代价是：自检测的是旧口径，现网跑的是新口径，两边都绿却互不证明。
 */
function evaluateBody(body) {
  const out = [];
  const read = anyIn(body, READ_FORMS);
  if (read) out.push({ rule: "R1", form: read, ok: !!anyIn(body, GENERATION_FORMS) });
  const write = anyIn(body, WRITE_FORMS);
  if (write) {
    out.push({ rule: "R2", form: write, ok: !!evidenceAtCallSite(body, write) });
  }
  const vread = anyIn(body, VALIDATION_READ_FORMS);
  if (vread) out.push({ rule: "R3", form: vread, ok: !!anyIn(body, R3_PREDICATES) });
  return out;
}

/**
 * 扫一遍，返回 `{ violations, hits, examined, exemptHits, rows }`。
 * 判据只看**剥注释后**的骨架：把守卫写进注释不算接线（负控自证）。
 */
export function checkSources(files, dump) {
  const violations = [];
  const rows = [];
  const hits = { R1: 0, R2: 0, R3: 0 };
  const exemptHits = [];
  let examined = 0;

  for (const abs of files) {
    const rel = path.relative(ROOT, abs).replace(/\\/g, "/");
    const src = fs.readFileSync(abs, "utf8");
    const stripped = stripRustComments(src);
    for (const fn of splitTopLevelFns(stripped)) {
      examined += 1;
      const body = fn.body;
      const key = `${rel}|${fn.name}`;

      for (const { rule, form, ok } of evaluateBody(body)) {
        hits[rule] += 1;
        if (ok) {
          if (dump) rows.push(`  ok ${rule} ${key} ← ${form}`);
          continue;
        }
        if (EXEMPT.has(key)) {
          exemptHits.push(`${rule} ${key} — 豁免：${EXEMPT.get(key)}`);
          continue;
        }
        violations.push(`${rule} ${key}：读到/写到候选池（${form}）却没有任何代际处置`);
      }
    }
  }
  return { violations, hits, rows, examined, exemptHits };
}

/** 合成骨架自证：判据必须抓得到真缺失，且不被注释/假改造骗过。 */
export function selftest() {
  const mk = (src) => splitTopLevelFns(stripRustComments(src));

  const cases = [];

  // 正控：带代际谓词的读侧 ⇒ 不得报违规
  const good = `
pub async fn load_pool_selection(db: &DatabaseConnection) -> Result<(), String> {
    let q = reco_picks::Entity::find().filter(reco_picks::Column::AsOfDate.is_null());
    let rows = q.all(db).await;
    Ok(())
}
`;
  cases.push({ name: "正控·谓词在位", fns: mk(good), expect: 0 });

  // 负控 1：拆掉谓词 ⇒ 必须命中一条 R1
  const bad = good.replace(".filter(reco_picks::Column::AsOfDate.is_null())", "");
  cases.push({ name: "负控·拆掉谓词", fns: mk(bad), expect: 1 });

  // 负控 2：谓词只写在注释里 ⇒ 仍须命中（注释不算接线）
  const commentOnly = `
pub async fn load_x(db: &DatabaseConnection) -> Result<(), String> {
    // 代际过滤：reco_picks::Column::AsOfDate.is_null()
    let rows = reco_picks::Entity::find().all(db).await;
    Ok(())
}
`;
  cases.push({ name: "负控·谓词只在注释里", fns: mk(commentOnly), expect: 1 });

  // 负控 3：假改造（读了 as_of_date 字段却没过滤/没写）⇒ R2 仍须命中
  const fakeWrite = `
pub async fn persist_reco_picks2(db: &DatabaseConnection) -> usize {
    let pick = pick_data();
    let _ = pick.insert(db).await;
    0
}
`;
  cases.push({ name: "负控·落库无代际", fns: mk(fakeWrite), expect: 1 });

  // 负控 4：整段没有候选池读写 ⇒ 不得命中（防判据过宽变成噪声门）
  const unrelated = `
pub async fn nothing_here(db: &DatabaseConnection) -> u32 {
    let rows = stock_analyses::Entity::find().all(db).await;
    rows.len() as u32
}
`;
  cases.push({ name: "负控·无关段不误报", fns: mk(unrelated), expect: 0 });

  // 负控 6：代际证据确实存在，但落在离调用点 >窗口的地方 ⇒ R2 仍须命中
  //（锁住"段里某处提过 as_of 就算数"这种凑证据形态）
  const filler = Array.from({ length: 24 }, (_, i) => `helper_${i}(db) +`).join(" ");
  const farEvidence = `
pub async fn persist_thing(db: &DatabaseConnection) -> usize {
    let as_of_date = compute_as_of();
    let _ = as_of_date;
    let long_winded = ${filler} 0;
    let _ = long_winded;
    let pick = build_pick();
    let _ = pick.insert(db).await;
    0
}
`;
  cases.push({ name: "负控·证据在调用点窗口外", fns: mk(farEvidence), expect: 1 });

  // 负控 7：读验证表时只**读出** `as_of_date` 字段、没有任何谓词 ⇒ R3 仍须命中
  //（带出代际 ≠ 处置代际：报表完全可以带着它把两代算进同一个分母）
  const readOutOnly = `
pub async fn report(db: &DatabaseConnection) -> u32 {
    let rows = decision_validations::Entity::find().all(db).await;
    let v = rows
        .into_iter()
        .map(|m| PickValidation { as_of_date: m.as_of_date })
        .count();
    v as u32
}
`;
  cases.push({ name: "负控·只读出代际字段不设谓词", fns: mk(readOutOnly), expect: 1 });

  let failed = 0;
  for (const c of cases) {
    const got = c.fns.reduce(
      (acc, fn) => acc + evaluateBody(fn.body).filter((r) => !r.ok).length,
      0,
    );
    const ok = got === c.expect;
    if (!ok) failed += 1;
    console.log(`  ${ok ? "✅" : "❌"} ${c.name}：期望命中 ${c.expect}，实得 ${got}`);
  }
  // 负控 5：骨架为空 ⇒ 分段器失效必须立刻暴露（否则整道门空转报绿）
  if (mk("").length !== 0) {
    console.log("  ❌ 空骨架应切出 0 段");
    failed += 1;
  } else {
    console.log("  ✅ 空骨架切出 0 段（分段器行为可预期）");
  }
  if (failed > 0) {
    console.error(`❌ 自检未通过：${failed} 条`);
    process.exit(1);
  }
  console.log("✅ 自检通过（判据会红、注释不算数、无关不误报）");
}

function main() {
  const files = SCAN_DIRS.flatMap((d) => listRustFiles(path.join(ROOT, d)));
  const dump = process.argv.includes("--dump");
  const r = checkSources(files, dump);

  if (dump && r.rows.length > 0) {
    console.log("命中明细：");
    for (const line of r.rows) console.log(line);
  }
  console.log(
    `扫描：${files.length} 个文件 / ${r.examined} 个顶层函数段；` +
      `R1 ${r.hits.R1} 处读侧、R2 ${r.hits.R2} 处写侧、R3 ${r.hits.R3} 处闭环读侧`,
  );
  if (r.exemptHits.length > 0) {
    console.log(`显式豁免 ${r.exemptHits.length} 条（债务，不是清零）：`);
    for (const e of r.exemptHits) console.log("  - " + e);
  }

  const thin = [];
  for (const [rule, min] of Object.entries(MIN_HITS)) {
    if (r.hits[rule] < min) thin.push(`${rule} 命中 ${r.hits[rule]} < 下限 ${min}（扫描面或判据被削弱）`);
  }
  const violations = [...r.violations, ...thin];
  if (violations.length > 0) {
    console.error(`❌ 代际一致违规 ${violations.length} 条：`);
    for (const v of violations) console.error("  - " + v);
    process.exit(1);
  }
  console.log("✅ 代际一致（读侧有谓词、写侧带标记、闭环分得开两套计数域）");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) selftest();
  main();
}
