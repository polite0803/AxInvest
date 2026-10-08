#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 「跨行聚合入口」的**按代筛样登记门**（#31，PLAN §七十六/§七十七，2026-10-05）。
//
// ## 为什么要有这道门
// 「按版本同代筛样」这件事散在十几处跨行聚合里（统计分母 / 注入语料 / 呈现 / 单条取数）。
// 只靠人记得去改，下一个人加读者时就会无声漏掉 —— 漏掉的后果不是报错，而是**判据静默混池**。
//
// ## 规则（按**四类**分列，不是「都调同一个 helper」）
// PLAN §五十一-③ 原写的是「每个登记项都调同一个 helper」。普查 9 个入口后确认**这是错的**：
// 那张单子混了三种名目，统一套起算代下限会让「呈现」与「单条取数」两类别无端丢行 ——
//   · 统计（分母类）：该筛 —— 断言函数体里出现起算代常量；
//   · 注入语料：该筛（且要声明被筛条数）—— 同上断言；
//   · 呈现：**不该筛**（筛了藏数据）—— 断言函数体里**不出现**该常量；
//   · 单条取数：**不该筛**（筛了反而制造缺席）—— 同上。
// 另有两类必须打印的理由：
//   · pending：源表尚无代际列（需加列或 join）—— 断言**未**假装筛过，每次运行打印；
//   · 每个条目找不到函数（改名/搬走）⇒ 红（保持登记表不腐烂）。
//
// ## 自证
// `--selftest` 用合成夹具证明两个方向都有区分力：
//   · 统计类夹具**缺**常量 ⇒ 必须报出；
//   · 呈现类/单条取数/pending 夹具**含**常量 ⇒ 必须报出（把下限套到这三类上是本门要拦的形态）；
//   · 找不到函数 ⇒ 必须报出。
//
// 常量名按**域**给：缺省 `HORIZON_BRANCH_GENERATION_FLOOR`（工作流域，唯一权威在
// `harness::holding_period`）；荐股流域的条目带 `marker: RECO_GENERATION_FLOOR`
// （权威在 `analysis-engine::recommender`）。两条链的代际整数互不相干 —— 拿 125 去比
// `reco_picks.reco_version` 恒假、拿 1 去比模板版本恒真，所以「引用了某个起算代常量」
// 不够，必须引用**自己那域**的那个。
//
// ⚠ 实现纪律：本文件**不出现任何正则与转义**。第一版用 `new RegExp` 拼模式，但文件是用
//   heredoc 落盘的 —— 双反斜杠被双层归一吃掉（两个反斜杠变一个、再变没有）⇒ 正则恒不匹配，
//   自证与首跑**同时全红**。改成纯 `indexOf` 扫描后没有转义面。
//
// 用法：node scripts/check-generation-scope-registry.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
/** 默认 = **工作流域**起算代（权威 `harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR`）。 */
const MARKER = "HORIZON_BRANCH_GENERATION_FLOOR";
/**
 * **荐股流域**起算代（权威 `analysis-engine::recommender::RECO_GENERATION_FLOOR`）。
 * 为什么要按条目分域：两条链的代际是两套互不相干的整数 —— 荐股链不跑工作流模板，
 * 它的版本载体是 `reco_picks.reco_version`。把 125 拿去比 reco_version 是**恒假**
 * （所有荐股样本都被判成「早于起算代」），反过来拿 1 去筛工作流域则是**恒真**
 * （模板版本从来 >= 1）。⇒ 每个条目必须声明自己那域的常量，`marker` 缺省为工作流域。
 */
const MARKER_RECO = "RECO_GENERATION_FLOOR";
/** 全部已知域的起算代常量 —— 负方向（呈现/单条取数）一次查完，否则换一域下手检不出。 */
const ALL_MARKERS = [MARKER, MARKER_RECO];

/** 登记表：`fn` 必须在 `file` 里唯一可定位。class 决定断言方向。 */
const REGISTRY = [
  // ── 统计（分母类）：该筛 ──
  {
    file: "src-tauri/crates/analysis-engine/src/reflection_stats.rs",
    fn: "build_hitrate_stats",
    class: "statistics",
    note: "命中率/IC/半衰期的主分母（§七十三）",
  },
  {
    file: "src-tauri/crates/analysis-engine/src/evolution_drift.rs",
    fn: "load_performance_window",
    class: "statistics",
    note: "权重线的样本窗口（§七十五）",
  },
  {
    file: "src-tauri/crates/analysis-engine/src/backtest.rs",
    fn: "optimize_weights",
    class: "statistics",
    note: "自适应评分权重；计数与取数同条件（§七十七）",
  },
  {
    file: "src-tauri/crates/analysis-engine/src/key_levels.rs",
    fn: "backtest_key_levels_with_config",
    class: "statistics",
    note: "关键位命中率（§七十七）",
  },
  {
    file: "src-tauri/src/commands/stock_analysis.rs",
    fn: "backtest_all_history",
    class: "statistics",
    note: "批量回测统计（§七十七）",
  },
  // ── 注入语料：该筛 + 声明被筛条数 ──
  {
    file: "src-tauri/src/commands/stock_workflow/core.rs",
    fn: "fetch_similar_cases",
    class: "injection",
    note: "同股失败案例进 prompt（§七十七）",
  },
  {
    file: "src-tauri/src/commands/stock_workflow/core.rs",
    fn: "fetch_stock_lessons",
    class: "injection",
    note: "历史反思/规则教训进 prompt（§七十一）",
  },
  // ⚠ 不登记 `stock_lesson_queries::generation_floor_or_null`：它是**比较符 helper**（常量由
  //   调用方给），没有「聚合入口」语义 ⇒ 断言它引用常量是假要求（首跑据此改过一次）。
  //   它被 `fetch_stock_lessons`/`fetch_rule_lessons` 覆盖（那两个入口必须引用常量）。
  // ── 呈现：不该筛（筛了藏数据） ──
  {
    file: "src-tauri/crates/analysis-engine/src/monthly_report.rs",
    fn: "generate_monthly_report",
    class: "presentation",
    note: "月度报告：按日期区间列分析，代际应标注而不是过滤",
  },
  {
    file: "src-tauri/src/commands/stock_workflow/misc.rs",
    fn: "query_decision_backtest",
    class: "presentation",
    note: "决策↔结果对账列表（观测/回放用）",
  },
  {
    file: "src-tauri/src/commands/backtest_validation.rs",
    fn: "list_decision_validations",
    class: "presentation",
    note: "验证记录分页列表：同一张表的**逐条**呈现，筛了就是藏行 —— 只有分母类才筛",
  },
  // ── 单条取数：不该筛（筛了制造缺席） ──
  {
    file: "src-tauri/crates/analysis-engine/src/trade_review.rs",
    fn: "get_trade_review",
    class: "single_lookup",
    note: "取卖出前最近一条分析当事前预测",
  },
  {
    file: "src-tauri/src/commands/backtest_validation.rs",
    fn: "sync_outcomes_to_stock_analyses",
    class: "single_lookup",
    marker: MARKER_RECO,
    note: "逐条验证记录回写 stock_analyses.outcome —— 按代筛它等于把旧代样本说成「未验证」",
  },
  // ── 统计（**荐股流域**代际）：该筛，但比的是 `RECO_GENERATION_FLOOR` 而非工作流域的 125 ──
  // 两条链的代际载体不同（`workflow_templates.version` vs `reco_picks.reco_version`），
  // 所以这两条带 `marker`。#49：`decision_validations` 自己没有版本列，代际由它 join 的
  // `reco_picks.reco_version` 提供 ⇒ 读侧筛样、写侧不筛（盖章门在 seed_consistency_tests）。
  {
    file: "src-tauri/src/commands/backtest_validation.rs",
    fn: "compute_validation_report",
    class: "statistics",
    marker: MARKER_RECO,
    note: "决策验证命中率/IC 的分母（§七十九 A2 / #49）",
  },
  {
    file: "src-tauri/src/commands/backtest_validation.rs",
    fn: "run_decision_backtest_inner",
    class: "statistics",
    marker: MARKER_RECO,
    note: "同一个报告的第二聚合入口（cron 自动触发），与上一条共用筛样判据 ⇒ 一个名目一套分母",
  },
  // ── pending：本批**没**筛它，并说明为什么 —— 断言它未假装筛过，每次运行打印 ──
  // #49 只交付「命中率报告」这一对入口；下面两条同表的统计读者属 #12（PLAN §七十九 C2），
  // 前置 = 跑一次荐股循环让新 pick 带章。不登记就会让 pending 桶看着是空的 = 谎报全清。
  {
    file: "src-tauri/src/commands/stock_analysis.rs",
    fn: "reco_ic_stats",
    class: "pending",
    marker: MARKER_RECO,
    note: "#12：荐股逐档 rank IC 面板（读 decision_validations 全表，尚未按 reco_version 筛）",
  },
  {
    file: "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs",
    fn: "load_reco_loop_samples",
    class: "pending",
    marker: MARKER_RECO,
    note: "#12：自动降权的样本装载 —— 未筛 ⇒ 跨代配对直接落在**写回**路径上",
  },
];

/** 取 `fn <name>` 的函数体（花括号配平）。找不到 ⇒ null。纯 indexOf，无正则、无转义。 */
export function extractFnBody(text, fnName) {
  const needle = "fn " + fnName;
  let at = text.indexOf(needle);
  while (at >= 0) {
    const after = text[at + needle.length];
    if (after === "(" || after === "<") {
      break;
    }
    at = text.indexOf(needle, at + 1);
  }
  if (at < 0) {
    return null;
  }
  const open = text.indexOf("{", at);
  if (open < 0) {
    return null;
  }
  let depth = 0;
  for (let i = open; i < text.length; i += 1) {
    if (text[i] === "{") {
      depth += 1;
    } else if (text[i] === "}") {
      depth -= 1;
      if (depth === 0) {
        return text.slice(open, i + 1);
      }
    }
  }
  return null;
}

/**
 * 剥掉行注释（`//` 到行尾）。`String.fromCharCode(10)` 而不是 `"\n"` —— 见文件头「实现纪律」：
 * 本文件不许出现转义（历史上 heredoc 落盘把 `\\n` 吃成空 ⇒ 判据恒不匹配）。
 *
 * 为什么必须剥：statistics/injection 类断言的是**代码**引用了起算代常量。注释里抄一遍常量名
 * 也能让 `body.includes(MARKER)` 为真 —— 那正是本门要拦的「假装筛过」。
 * 反过来 presentation/single_lookup 类断言「不引用」，注释泄漏同样会误报红。
 *
 * 只剥行注释、不处理字符串里的 `//`（URL 形态）。自证方式：现网 12 条真实入口跑完仍全绿
 * ⇒ 剥法没有吃掉任何真代码（吃了就会当场红，不会静默）。
 */
function codeOnly(bodyText) {
  const nl = String.fromCharCode(10);
  return bodyText
    .split(nl)
    .map((line) => {
      const at = line.indexOf("//");
      return at < 0 ? line : line.slice(0, at);
    })
    .join(nl);
}

export function checkOne(sourceText, entry) {
  const body = extractFnBody(sourceText, entry.fn);
  if (body === null) {
    return { ok: false, why: "找不到函数 " + entry.fn + "（改名/搬走 ⇒ 登记表须同步）" };
  }
  const marker = entry.marker ?? MARKER;
  const code = codeOnly(body);
  switch (entry.class) {
    case "statistics":
    case "injection": {
      const has = code.includes(marker);
      return has
        ? { ok: true }
        : { ok: false, why: entry.class + " 类必须引用 " + marker + "（否则判据静默混池）" };
    }
    case "presentation":
    case "single_lookup": {
      // 负方向查**所有域**的常量：呈现/单条取数被任一代际常量筛过都是藏行/制造缺席，
      // 只盯本域常量的话，换一域下手就检不出。
      const hit = ALL_MARKERS.filter((m) => code.includes(m));
      return hit.length === 0
        ? { ok: true }
        : {
            ok: false,
            why:
              entry.class + " 类**不该**引用 " + hit.join(" / ") + " —— 筛了会藏数据/制造缺席",
          };
    }
    case "pending": {
      const has = code.includes(marker);
      return has
        ? { ok: false, why: "pending 类不得假装已筛（源表没有代际列时引用常量＝假合规）" }
        : { ok: true };
    }
    default:
      return { ok: false, why: "未知 class " + entry.class };
  }
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    const good = "fn f(a: u32) { let x = " + MARKER + "; }";
    const recoGood = "fn f(a: u32) { let x = " + MARKER_RECO + "; }";
    const bad = "fn f(a: u32) { let x = 1; }";
    const cases = [
      ["统计类缺常量必须红", checkOne(bad, { fn: "f", class: "statistics" }).ok === false],
      ["统计类有常量必须绿", checkOne(good, { fn: "f", class: "statistics" }).ok === true],
      ["注入类有常量必须绿", checkOne(good, { fn: "f", class: "injection" }).ok === true],
      ["统计类引用**别域**的起算代常量必须红", checkOne(good, { fn: "f", class: "statistics", marker: MARKER_RECO }).ok === false],
      ["统计类引用本域(荐股)起算代常量必须绿", checkOne(recoGood, { fn: "f", class: "statistics", marker: MARKER_RECO }).ok === true],
      ["只在注释里写起算代常量必须红（引用要落在代码上）", checkOne("fn f(a: u32) { let x = 1; // " + MARKER + " }", { fn: "f", class: "statistics" }).ok === false],
      ["剥注释不得吃掉同行代码", checkOne("fn f(a: u32) { let x = " + MARKER + "; // " + "备注 }", { fn: "f", class: "statistics" }).ok === true],
      ["pending 类引用本域(荐股)常量必须红", checkOne(recoGood, { fn: "f", class: "pending", marker: MARKER_RECO }).ok === false],
      ["呈现类带常量必须红", checkOne(good, { fn: "f", class: "presentation" }).ok === false],
      ["呈现类引用**荐股域**常量也必须红（负方向查所有域）", checkOne(recoGood, { fn: "f", class: "presentation" }).ok === false],
      ["单条取数引用荐股域常量也必须红", checkOne(recoGood, { fn: "f", class: "single_lookup" }).ok === false],
      ["呈现类无常量必须绿", checkOne(bad, { fn: "f", class: "presentation" }).ok === true],
      ["单条取数带常量必须红", checkOne(good, { fn: "f", class: "single_lookup" }).ok === false],
      ["pending 带常量必须红", checkOne(good, { fn: "f", class: "pending" }).ok === false],
      ["找不到函数必须红", checkOne(bad, { fn: "nope", class: "statistics" }).ok === false],
      ["配平只取本函数体", extractFnBody(bad + "fn g() { " + good + " }", "f").includes(MARKER) === false],
    ];
    const failed = cases.filter((c) => !c[1]).map((c) => c[0]);
    if (failed.length > 0) {
      console.error("❌ 自证失败：" + failed.join(" / "));
      process.exit(1);
    }
    console.log("✅ 自证通过（" + cases.length + " 条对照，含两方向负控）");
    process.exit(0);
  }

  const problems = [];
  const byClass = new Map();
  for (const e of REGISTRY) {
    const src = fs.readFileSync(path.join(ROOT, e.file), "utf8");
    const r = checkOne(src, e);
    if (!r.ok) {
      problems.push(e.file + " 的 " + e.fn + "：" + r.why);
    }
    byClass.set(e.class, (byClass.get(e.class) ?? 0) + 1);
  }
  const summary = [...byClass.entries()].map((kv) => kv[0] + " " + kv[1]).join(" · ");
  console.log("扫描：登记 " + REGISTRY.length + " 项（" + summary + "）");
  for (const e of REGISTRY) {
    if (e.class === "pending") {
      console.log("  ⚠ pending(未筛)：" + e.fn + " —— " + e.note);
    }
  }
  if (args.includes("--dump")) {
    for (const e of REGISTRY) {
      const marker = e.marker ? " (marker=" + e.marker + ")" : "";
      console.log("  [" + e.class + "] " + e.fn + " @ " + e.file + marker + " —— " + e.note);
    }
  }
  if (problems.length > 0) {
    console.error("❌ " + problems.length + " 处违规：");
    for (const p of problems) {
      console.error("  " + p);
    }
    process.exit(1);
  }
  console.log("✅ 登记表与四类断言一致（presentation/single_lookup 已确认**没有**被误筛）");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
