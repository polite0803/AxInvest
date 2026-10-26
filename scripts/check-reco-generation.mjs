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
// R5（2026-10-18 追加，PLAN-mover-coverage §0-C）：写侧段内 `seed_pool_snapshot` 的**前向邻域**
//          出现 `unwrap_or_else` 或字面量 `"[]"` ⇒ 红。`None`（来源不可得）折成 `"[]"`（池真的空）
//          是把取数缺痕写成一条「该票未进候选池」的结论 —— 现网回放侧 3520 行全是 `"[]"`，
//          而真实候选行数 = 0，这个矛盾在库里正因为被折空而读不出来。
//          判据必须落在表达式邻域：`start_cron_scheduler` 这类巨型函数里 `unwrap_or_else`
//          会为别的目的出现，整段共现即假阳（首跑实测误报过一次，已收窗口并补负控 9）。
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
 * R10（2026-10-24，PLAN G-7）：`purpose` 是「谁跑的这次回放」的登记值域。
 *
 * 判据两条，都是机械可判的恒空/串批形态：
 * - **live 谓词 + 回放用途**（`AsOfDate.is_null()` 与 `purpose::MOVER_REPLAY` 同段）⇒ 恒空；
 * - **按回放用途读候选池却不带 `generated_at`** ⇒ 同一天多次回放会串批（差异证据必须
 *   钉到某一次回放，而不是「那天所有回放」）。
 *
 * 值域本身的唯一权威在 `crates/dao/src/repo/reco_pool.rs` 的 `pub mod purpose`；
 * 本表只是**登记**（新增值必须先来这里），运行时会与源码逐值对表（见 main）。
 */
const PURPOSE_REGISTRY = [
  "legacy_replay",
  "live",
  "manual_replay",
  "mover_replay",
  "time_travel_replay",
];
const PURPOSE_REPLAY_FORMS = [
  "purpose::MOVER_REPLAY",
  "purpose::LEGACY_REPLAY",
  "purpose::TIME_TRAVEL_REPLAY",
  "purpose::MANUAL_REPLAY",
];

/** R10 的段级判据（无 `Column::Purpose` 的段不参与）。 */
function purposeCheck(body) {
  // 只判**读侧**：`update_many` 的用途回填（`backfill_legacy_replay_purpose`）是写操作，
  // 要求它带 generated_at 是无对象的（它按 `purpose IS NULL AND as_of_date IS NOT NULL` 收敛）。
  if (!body.includes("Column::Purpose") || !body.includes("Entity::find")) return null;
  const replay = PURPOSE_REPLAY_FORMS.find((c) => body.includes(c));
  if (!replay) return null;
  if (body.includes("Column::AsOfDate.is_null()")) {
    return {
      rule: "R10",
      msg: `live 谓词（AsOfDate.is_null）与回放用途（${replay}）同段 ⇒ 查询恒空`,
    };
  }
  if (!body.includes("Column::GeneratedAt")) {
    return {
      rule: "R10",
      msg: `按回放用途读候选池（${replay}）未带 generated_at ⇒ 同日多次回放会串批`,
    };
  }
  return null;
}

/** 从 dao 源码取 purpose 值域（登记表必须与它逐值相等）。 */
function readPurposeDomain() {
  const file = path.join(ROOT, "src-tauri/crates/dao/src/repo/reco_pool.rs");
  if (!fs.existsSync(file)) return [];
  const src = fs.readFileSync(file, "utf8");
  const block = src.match(/pub mod purpose \{([\s\S]*?)\n\}/);
  if (!block) return [];
  return [...block[1].matchAll(/pub const \w+: &str = "([^"]+)";/g)]
    .map((m) => m[1])
    .sort();
}

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
    "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs|load_pick_versions",
    "R1：按 pick 主键集合回查 `reco_version` —— 本函数读的就是代际列本身（供 IC 筛样），再加代际谓词是把同一判据写两遍",
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
    // R5（2026-10-18，PLAN-mover-coverage §0-C）：写候选池的段内**不得**把「池来源不可得」
    // 折成空数组字面量。`None` 与 `"[]"` 含义相反（前者=取数侧没给，后者=池子真的空），
    // 而 `mover_recall::code_in_pool_snapshot` 对两者同判「不在池」⇒ 折空即把取数缺痕
    // 写成一条「该票未进候选」的结论。
    // ⚠ 判据必须落在**表达式邻域**而不是整段共现：`start_cron_scheduler` 这类巨型函数里
    //   `unwrap_or_else` 会为别的目的出现，整段共现 ⇒ 假阳（2026-10-18 首跑实测就误报了它）。
    //   折空形态永远是 `seed_pool_snapshot…` 后面紧跟 `.unwrap_or_else(…)` 或 `"[]"`，
    //   所以取 `seed_pool_snapshot` 命中点起的前向窗口即可；窗口宽度按 rustfmt 折行余量取 200。
    const foldAt = (body, needle) => {
      let at = body.indexOf(needle);
      while (at !== -1) {
        const win = body.slice(at, at + 200);
        if (win.includes("unwrap_or_else") || win.includes('"[]"')) return needle;
        at = body.indexOf(needle, at + needle.length);
      }
      return null;
    };
    const foldsPool = foldAt(body, "seed_pool_snapshot");
    if (foldsPool) {
      out.push({ rule: "R5", form: `${foldsPool} 后随 unwrap_or_else / "[]"`, ok: false });
    }
  }
  const vread = anyIn(body, VALIDATION_READ_FORMS);
  if (vread) out.push({ rule: "R3", form: vread, ok: !!anyIn(body, R3_PREDICATES) });
  const pc = purposeCheck(body);
  if (pc) out.push({ rule: "R10", form: pc.msg, ok: false });
  return out;
}

// ── R6/R7/R8（2026-10-18，PLAN-mover-coverage-and-reflection-exit §P-g）──────────────
// `reco_picks.synthetic=1` 是**降级产物**（capital 无资金流数据 ⇒ K线代理；watchlist ⇒ 初筛无技术
// 信号）：价格与置信都真，但**不是该策略自己做的判断**。每个读点必须先归一类再决定要不要排除它
// —— 四类口径互斥，混用等于把两套计数域并成一个分母：
//   A 必须排除（算分母 / 记策略信号史 / 当对照先验）⇒ 缺 `Synthetic` 谓词即 R6 红；
//   B 必须包含（候选池覆盖证据，漏检归因靠它）⇒ 加谓词即 R7 红（会把「进过池」读成「没进池」）；
//   C 与降级无关（按主键回查元数据）⇒ 加谓词即 R7 红（同一判据写两遍）；
//   P 呈现类（荐股页缓存回读）⇒ 必须包含，降级声明在 reasons/risk_notes，抹掉即丢证据。
// R8：出现**未登记**的读点 ⇒ 红。名单腐烂比缺门更坏 —— 新读点必须先判语义再登记，不能静默通过。
const POOL_A = new Map([
  ["src-tauri/src/commands/stock_analysis.rs|backtest_reco_strategies_inner", "正负样本分母"],
  ["src-tauri/src/commands/stock_analysis.rs|get_reco_signal_history", "策略信号明细"],
  ["src-tauri/src/commands/stock_workflow/hooks.rs|fetch_reco_prior", "两链对照先验"],
  ["src-tauri/src/commands/pool_scan.rs|load_pool_selection", "池分析分母"],
  ["src-tauri/src/commands/backtest_validation.rs|run_decision_backtest_inner", "验证样本分母"],
  ["src-tauri/src/commands/backtest_validation.rs|run_serenity_backtest_pass", "智选命中率分母"],
]);
const POOL_B = new Map([
  [
    "src-tauri/crates/analysis-engine/src/mover_recall.rs|load_picks_in_range",
    "池覆盖证据：排除降级会把「进过池」读成「没进池」",
  ],
]);
const POOL_C = new Map([
  [
    "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs|load_pick_versions",
    "按 pick_id 主键回查 reco_version，与降级无关",
  ],
  ["src-tauri/src/commands/backtest_validation.rs|compute_validation_report", "报告侧主键回查"],
  ["src-tauri/src/commands/stock_workflow/reco_history.rs|get_reco_detail", "按主键取详情"],
  ["src-tauri/src/commands/stock_workflow/reco_history.rs|batch_delete_reco_history", "按批次键删除"],
  ["src-tauri/src/commands/stock_workflow/reco_history.rs|delete_serenity_pick", "按主键删单行"],
  [
    "src-tauri/crates/dao/src/repo/reco_pool.rs|latest_seed_pool_codes",
    "按批次读 seed_pool_json（扫描阶段的输入池快照），与该行自身是否降级产物无关 ⇒ 排除降级只会让收盘归档多一条「池快照为空，跳过」，不提升选码质量",
  ],
  [
    "src-tauri/crates/dao/src/repo/reco_pool.rs|find_existing_pick_id",
    "幂等键（as_of_date/period/stock_code/style/purpose）查重回查既有行 id，用途是覆盖写而非分母 ⇒" +
      "与 synthetic 降级无关；加谓词会把既有行判成「不存在」⇒ 覆盖写变新增、id 换新致下游引用悬空",
  ],
]);
const POOL_P = new Map([
  [
    "src-tauri/src/commands/stock_analysis.rs|get_cached_recommendation",
    "荐股页缓存回读：降级候选必须显示，声明已在 reasons/risk_notes",
  ],
]);
/** A 类待补（显式豁免、每次打印，不是清零）：serenity 行由写入侧固定 `synthetic=0`
 *  （`serenity.rs` 写 `reco_picks::ActiveModel` 那处），所以这两处现在加谓词是**零行为变更**；登记为债是为了防止
 *  将来有别的产端往 style='serenity' 写降级行时，这里静默混进分母。 */
const POOL_A_PENDING = new Map([
  ["src-tauri/src/commands/stock_workflow/serenity.rs|do_refresh_exit_signals", "退出信号刷新；同上"],
  ["src-tauri/src/commands/stock_workflow/serenity.rs|do_feedback_loop", "反馈闭环；同上"],
]);
const POOL_READ_FORMS = [
  "reco_picks::Entity::find()",
  "reco_picks::Entity::find_by_id(",
  "reco_picks::Entity::delete_many()",
];

/** 该段是否含 `reco_picks` 读形态（含则返回该形态，用于 R8/R9 的命中记账）。 */
export function poolReadForm(body) {
  return POOL_READ_FORMS.find((f) => body.includes(f)) || null;
}

/** 键级归类判定：无读形态返回 null（不参与本组规则）。 */
export function poolClassCheck(key, body) {
  const form = POOL_READ_FORMS.find((f) => body.includes(f));
  if (!form) return null;
  const hasPred = body.includes("Column::Synthetic");
  if (POOL_A.has(key)) {
    return hasPred
      ? null
      : { rule: "R6", msg: `A 类读点（${POOL_A.get(key)}）缺 Synthetic 谓词 ⇒ 降级产物被计进分母` };
  }
  const mustNot = POOL_B.get(key) || POOL_C.get(key) || POOL_P.get(key);
  if (mustNot) {
    return hasPred ? { rule: "R7", msg: `非分母读点却加了 Synthetic 谓词（应属类：${mustNot}）` } : null;
  }
  if (POOL_A_PENDING.has(key)) {
    return hasPred ? null : { pending: POOL_A_PENDING.get(key) };
  }
  return { rule: "R8", msg: `未登记的 reco_picks 读点（读形态：${form}）⇒ 必须先判 A/B/C/P 再登记` };
}

/**
 * 扫一遍，返回 `{ violations, hits, examined, exemptHits, rows }`。
 * 判据只看**剥注释后**的骨架：把守卫写进注释不算接线（负控自证）。
 */
export function checkSources(files, dump) {
  const violations = [];
  const rows = [];
  const hits = { R1: 0, R2: 0, R3: 0, R5: 0, R6: 0, R7: 0, R8: 0, R9: 0, R10: 0 };
  const exemptHits = [];
  let examined = 0;
  const poolSeen = new Set();

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
      // ⚠ 记账必须在**任何判定之前**：`poolClassCheck` 对合规的 A 类点返回 null，
      //   若只在「有判定结果」时 add，则所有**已经合规**的登记点都会被 R9 判成名单过期
      //   （2026-10-18 首跑实测：15 条假红，全是已加谓词的 A 类）。
      if (poolReadForm(body)) poolSeen.add(key);
      const pc = poolClassCheck(key, body);
      if (pc) {
        if (pc.pending) exemptHits.push(`R6 ${key} — 豁免：${pc.pending}`);
        else {
          hits[pc.rule] += 1;
          violations.push(`${pc.rule} ${key}：${pc.msg}`);
        }
      }
    }
  }
  // R9：登记表里的键若本轮一个读点都没命中 ⇒ 名单已腐烂（函数改名 / 读点搬迁），
  // 而门会安静地对一个不存在的对象「保证」它合规。与 R8 成对：R8 抓漏登记，R9 抓过期登记。
  for (const [tbl, name] of [
    [POOL_A, "A"],
    [POOL_B, "B"],
    [POOL_C, "C"],
    [POOL_P, "P"],
    [POOL_A_PENDING, "A(待补)"],
  ]) {
    for (const key of tbl.keys()) {
      if (!poolSeen.has(key)) {
        hits.R9 += 1;
        violations.push(`R9 ${key}：登记为 ${name} 类，但本轮扫描找不到它的 reco_picks 读点 ⇒ 名单过期，须重定位或删除`);
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

  // 负控 11（2026-10-20）：代际字样藏在**尾随的 `#[cfg(test)] mod tests`** 里 ⇒ R1 仍须命中。
  // 触发实例：给 dao `reco_pool.rs` 补了形参叫 `as_of_date` 的测试助手后，分段器把整个测试模块
  // 吞进文件最后一个生产函数的体 ⇒ 生产侧那行谓词删掉门照样绿。测试文本不算生产接线。
  const testModTail = `
pub async fn load_pool_selection(db: &DatabaseConnection) -> Result<(), String> {
    let rows = reco_picks::Entity::find().all(db).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    async fn insert_row(db: &DatabaseConnection, as_of_date: Option<&str>) {
        insert_picks(db, &[], None, None, "2026-01-01", as_of_date, None).await;
    }
}
`;
  cases.push({ name: "负控·尾随测试模块里的代际字样不算接线", fns: mk(testModTail), expect: 1 });

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

  // ── R5（2026-10-18，PLAN-mover-coverage-and-reflection-exit §0-C）──
  // 负控 8：写候选池的段内把「池来源不可得」折成空数组 ⇒ 必须命中。
  // 该段已带 as_of_date（R2 通过），所以唯一红来自 R5 —— 负控拆在真正决定它的那道守卫上。
  const foldPool = `
async fn persist_reco_response_picks(state: &AppState, response: &RecoResponse) -> usize {
    let seed_pool_json = response.seed_pool_snapshot.clone().unwrap_or_else(|| "[]".to_string());
    let written = persist_reco_picks(db, &flat, Some(seed_pool_json), weights, &generated_at, as_of_date).await;
    written
}
`;
  cases.push({ name: "负控·池快照折成空数组", fns: mk(foldPool), expect: 1 });

  // 正控：原样透传 Option（拿不到就是 NULL）⇒ 不得报
  const honestPool = foldPool
    .replace('.unwrap_or_else(|| "[]".to_string())', "")
    .replace("Some(seed_pool_json)", "seed_pool_json");
  cases.push({ name: "正控·池快照原样透传", fns: mk(honestPool), expect: 0 });

  // 负控 9：同段远处偶遇 unwrap_or_else 不得算折空（巨型函数假阳的那次实测反例）
  const distant = `
async fn start_cron_scheduler_big(db: &Db) -> usize {
    let other = lookup(db).unwrap_or_else(|e| 0);
    let _ = other;
    let written = persist_reco_picks(db, &picks, scan.seed_pool_snapshot.clone(), weights, &generated_at, as_of_date).await;
    written
}
`;
  cases.push({ name: "负控·同段远处偶遇不算折空", fns: mk(distant), expect: 0 });

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
  // ── R6/R7/R8（2026-10-18，PLAN §P-g）：键级归类是纯函数，负控拆在它自己的守卫上 ──
  const A_KEY = "src-tauri/src/commands/pool_scan.rs|load_pool_selection";
  const B_KEY = "src-tauri/crates/analysis-engine/src/mover_recall.rs|load_picks_in_range";
  const FIND = "reco_picks::Entity::find()";
  const PRED = "reco_picks::Column::Synthetic.eq(0)";
  const pcChecks = [
    { name: "R6 A 类缺谓词 ⇒ 红", got: poolClassCheck(A_KEY, FIND), want: "R6" },
    { name: "R6 A 类有谓词 ⇒ 不报", got: poolClassCheck(A_KEY, `${FIND}.filter(${PRED})`), want: null },
    { name: "R7 池覆盖证据被加谓词 ⇒ 红", got: poolClassCheck(B_KEY, `${FIND}.filter(${PRED})`), want: "R7" },
    { name: "R8 未登记读点 ⇒ 红", got: poolClassCheck("x.rs|brand_new_reader", FIND), want: "R8" },
    { name: "R8 无读形态的段 ⇒ 不参与", got: poolClassCheck("x.rs|anything", "let a = 1;"), want: null },
  ];
  for (const c of pcChecks) {
    const got = c.got === null ? null : c.got.rule || "pending";
    const ok = got === c.want;
    if (!ok) failed += 1;
    console.log(`  ${ok ? "✅" : "❌"} ${c.name}（实得 ${got}）`);
  }
  // R9 的负控已经在真跑里自证过一轮：登记两个不存在的键时它报出 2 条假登记
  //（2026-10-18 实测 R9 15 → 摘掉过期键后 0），这里只锁它的记账依据还在。
  // ── R10（2026-10-24，PLAN G-7）：用途与代际的矛盾形态 ──
  const r10Cases = [
    {
      name: "R10 live 谓词 + 回放用途 ⇒ 红",
      src: `
pub async fn live_reader(db: &DatabaseConnection) -> usize {
    let q = reco_picks::Entity::find()
        .filter(reco_picks::Column::AsOfDate.is_null())
        .filter(reco_picks::Column::Purpose.eq(purpose::MOVER_REPLAY))
        .filter(reco_picks::Column::GeneratedAt.eq("x"));
    let n = q.count(db).await;
    n as usize
}`,
      want: 1,
    },
    {
      name: "R10 回放用途缺 generated_at ⇒ 红",
      src: `
pub async fn diff_reader(db: &DatabaseConnection) -> usize {
    let q = reco_picks::Entity::find()
        .filter(reco_picks::Column::AsOfDate.eq("2026-09-04"))
        .filter(reco_picks::Column::Purpose.eq(purpose::MOVER_REPLAY));
    let n = q.count(db).await;
    n as usize
}`,
      want: 1,
    },
    {
      name: "R10 回放用途 + 代际 + generated_at ⇒ 绿",
      src: `
pub async fn diff_reader_ok(db: &DatabaseConnection) -> usize {
    let q = reco_picks::Entity::find()
        .filter(reco_picks::Column::AsOfDate.eq("2026-09-04"))
        .filter(reco_picks::Column::Purpose.eq(purpose::MOVER_REPLAY))
        .filter(reco_picks::Column::GeneratedAt.eq("x"));
    let n = q.count(db).await;
    n as usize
}`,
      want: 0,
    },
  ];
  for (const c of r10Cases) {
    const got = mk(c.src).reduce(
      (acc, fn) => acc + evaluateBody(fn.body).filter((row) => row.rule === "R10").length,
      0,
    );
    const ok = got === c.want;
    if (!ok) failed += 1;
    console.log(`  ${ok ? "✅" : "❌"} ${c.name}（实得 ${got}）`);
  }
  if (readPurposeDomain().length === 0) {
    console.log("  ❌ R10 取不到 purpose 值域 ⇒ 登记对表恒绿");
    failed += 1;
  }

  if (!poolReadForm(FIND)) {
    console.log("  ❌ poolReadForm 认不出标准读形态 ⇒ R8/R9 的记账依据已失效");
    failed += 1;
  } else {
    console.log("  ✅ poolReadForm 认出入口读形态（R8/R9 记账依据在位）");
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
      `R1 ${r.hits.R1} 处读侧、R2 ${r.hits.R2} 处写侧、R3 ${r.hits.R3} 处闭环读侧、R5 ${r.hits.R5} 处折空、R6 ${r.hits.R6} 处漏排除、R7 ${r.hits.R7} 处误排除、R8 ${r.hits.R8} 处未登记、R9 ${r.hits.R9} 处名单过期、R10 ${r.hits.R10} 处用途/代际矛盾`,
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
  // G-7：purpose 值域登记对表（新增用途却忘了登记 ⇒ 红；删了用途但登记表没跟 ⇒ 也红）。
  const domain = readPurposeDomain();
  if (domain.length === 0) {
    violations.push("R10 取不到 purpose 值域（dao 的 `pub mod purpose` 未找到）⇒ 登记对表失效");
  } else if (JSON.stringify(domain) !== JSON.stringify(PURPOSE_REGISTRY)) {
    violations.push(
      `R10 purpose 值域与登记表不一致：源码 ${JSON.stringify(domain)} / 登记 ${JSON.stringify(PURPOSE_REGISTRY)}`,
    );
  }
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
