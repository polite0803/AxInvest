#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 分析师节点 id 的**形态门**（B2-1，PLAN-four-horizon-workflow-alignment.md §五十六/§六十一）。
//
// ## 为什么要有这道门
// R-11 之后每档要挂自己的分析师节点，id 形态是 `<base>--<tier>`（`a-sentiment--mid`）。
// 建 23 个节点之前必须先把「谁在按整串 id 比较」收干净：后缀一旦出现，所有裸匹配
// `"a-sentiment"` 的地方会**静默失配**（不报错，只是查不到 ⇒ 权重退 1.0、必采清单放水、
// 专家映射落空）。这类缺陷只能靠门提前量出来，不能等建完节点再找。
//
// ## 三条规则
// R1 唯一 base 域：所有完整的 `"a-<x>"` 字面量必须落在权威 base 集内。
//    权威集**由种子自己的分析师表推导**（`seed_stock_analysis.rs` 的 `ANALYST_*` 清单），
//    不在本文件手抄第二份 —— 手抄的那份会随分析师增删腐烂（本仓「清单由单一权威渲染」的纪律）。
// R2 带档后缀只许由 helper 产出：任何字面量里都不得出现 `<base>--<tier>` 形态
//    （拼 id 的正门是 `holding_period::analyst_node_id` / `stock-analysis-utils.ts::analystNodeId`）。
//    这一条在 B2 前**恒真**（还没有带档节点），它拦的是「提前手抄带档 id」。
// R3 裸匹配基线：`(==|!=)\s*"a-…"` 与 `match … "a-…" =>` 这类「按整串 id 比较」的位置，
//    计数锁成基线，**只许降不许升**；降到 0 时把基线改成 0，它就变成硬拦。
//    ⚠ 首读打在**未修的树上**（这是本仓新门的规矩）：现在基线非 0 是**如实的现状快照**，
//    不是门坏了。把某处改成走 `analyst_base_of` 之后计数才会降。
//
// ## 豁免的两类，都写理由
// · helper 自身（定义分隔符与拼/剥函数的文件）—— 它必然含 `--` 与 `a-` 字面量；
// · 测试夹具（构造样本 id 的地方）—— 它的职责就是写死字符串来验生产剥得对不对。
//
// 用法：node scripts/check-analyst-id-shape.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const p = (...a) => path.join(ROOT, ...a);

// ── 扫描面：股票分析链的分析师 id 载体（**不含 opc_* 域包**，那套 `a-` 是别的子系统的
//    agent id，与本判据无关；把它们卷进来会让门报出一堆与 B2 无能的噪声）──
const SCOPE = [
  "src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs",
  "src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs",
  "src-tauri/src/commands/stock_workflow/decision.rs",
  "src-tauri/src/commands/stock_workflow/rhai_registry.rs",
  "src-tauri/crates/analysis-engine/src/evidence_weight.rs",
  "src-tauri/crates/analysis-engine/src/evidence_citation.rs",
  "src-tauri/crates/analysis-engine/src/blackboard.rs",
  "src-tauri/crates/astock-data/src/quality.rs",
  "src-tauri/crates/entities/src/analyst_feedback.rs",
  "src-tauri/crates/harness/src/holding_period.rs",
  "src-tauri/crates/rt-workflow/src/work_engine/executors/code_executor.rs",
  "src-tauri/crates/rt-workflow/src/work_engine/executors/var_filter.rs",
  "src/lib/stock-analysis-utils.ts",
  "src/lib/dataQualityDiagnosis.ts",
  "src/lib/decisionInputDiagnosis.ts",
  "src/stores/feature/stockWorkflowChatBridge.ts",
  "src/components/stock-analysis/AnalysisProgress.tsx",
  "src/components/stock-analysis/AnalystReportGrid.tsx",
  "src/components/settings/AgentProfileList.tsx",
  "src/stores/feature/__tests__/stockAnalysisStore.test.ts",
  "src/stores/__tests__/stockAnalysisStore.test.ts",
  "src/components/stock-analysis/__tests__/AnalystDataQualityModal.scope.test.tsx",
  "src/components/stock-analysis/__tests__/EvidenceCitationPanel.test.tsx",
];

// 已知缺陷的**带日期豁免**（每次运行都会打印，不做静默豁免）。与 `.i18n-allowlist.json` 同形态：
// 条目必须写清「为什么现在不算违规、修它归谁」，否则门就从「找缺陷」退化成「给现状背书」。
// 修好后删掉条目，R1 立刻接管。
const KNOWN_FINDINGS = [
  {
    file: "src-tauri/crates/analysis-engine/src/evidence_citation.rs",
    ids: ["a-technical", "a-macro"],
    // 本门首读实测（2026-10-05）：这两个 id **不在**种子权威 base 表（10 个）里，
    // 表里对应名目是 `a-market-analyst` / `a-policy`；它们与专家短名写在同一个 match 的或侧
    // （`"a-technical" | "market-analyst" =>`）= 两套拼法共存。
    // 2026-10-05 更新：**「表内每个权威 id 必须有显示名」已经变成判据**（Rust 侧
    // `evidence_citation::tests::every_authority_analyst_has_a_display_name`）—— 本豁免原先
    // 只盯【表外名字】，而补之前 `a-lockup`/`a-catalyst`/`a-research`/`a-policy`/`a-market-analyst`
    // 五个【表内现役】 id 一个都没登记、门却全绿 ⇒ 那种「看不见表内缺口」的形态已被测试接管。
    // 剩下的豁免理由只有一条：删这两臂要先普查「谁往 citation.analystId 写值」
    // （`backtest_feedback.rs` 的测试夹具也用 `a-technical`，那是另一条链的 id 空间）。
    owner: "citation analystId 名目收编（普查写值方后删旧臂）；与 B2-2 建点批无关",
  },
];

// helper 自身 + 夹具：R1/R2 豁免（理由见文件头），但仍**计入 R3**（它们也该改走 base）。
const HELPER_OR_FIXTURE = new Set([
  "src-tauri/crates/harness/src/holding_period.rs",
  "src/lib/stock-analysis-utils.ts",
]);

// 归一见证窗口：命中点上方几行内必须有剥后缀的调用（0 = 关闭见证判据，仅自测用）。
const WITNESS_WINDOW = 6;

const TIER_SNAKE = ["ultra_short", "short", "mid", "long"];

/** 权威 base 集：从种子的分析师表推导（第一列就是节点 id）。 */
function authoritativeBases() {
  const seed = fs.readFileSync(p("src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs"), "utf8");
  const set = new Set();
  // 表形态：("a-market-analyst", "技术面分析：…", "market-analyst"),
  for (const m of seed.matchAll(/\("(a-[a-z][a-z0-9-]*)"\s*,\s*"[^"]*"\s*,\s*"[a-z0-9-]+ "\.md|\("(a-[a-z][a-z0-9-]*)"\s*,\s*"[^"]*"\s*,\s*"[a-z0-9-]+"/g)) {
    set.add(m[1] ?? m[2]);
  }
  return set;
}

/** 剥掉注释：行注释与块注释都剥。禁词类判据不剥注释就会红在散文上（本仓踩过多次）。 */
function codeOnly(src) {
  const noLine = src
    .split("\n")
    .map((l) => {
      const i = l.indexOf("//");
      return i >= 0 ? l.slice(0, i) : l;
    })
    .join("\n");
  return noLine.replace(/\/\*[\s\S]*?\*\//g, "");
}

const FULL_ID = /"(a-[a-z][a-z0-9-]*)"/g;
const SUFFIXED = /"(a-[a-z][a-z0-9-]*)--(ultra_short|short|mid|long)[a-z_-]*"/g;
// `== Some("a-news")` 这种**包了 Option** 的写法同样是「按整串 id 比较」，必须计入 ——
// 自测的假改造样本就是为了堵这个覆盖面洞（首版漏了它，门会把最典型的 Rust 形态放过）。
const NAIVE_CMP = [
  /(?:==|!=)\s*(?:Some\(\s*)?&?\s*"a-[a-z][a-z0-9-]*"/g,
  /(?:^|[^\w.])"a-[a-z][a-z0-9-]*"\s*=>/gm,
];

/** R3 的「基线」：只许降不许升。数值是**首读打在未修的树上**得到的现状快照（见 --dump）。 */
// 首读（2026-10-05，未修的树）= 27；口径改成「字面量比较点全量」后**仍是 27**
// （接线没让数字下降 —— 它只保证带档 id 出现时不静默失配，见 scan() 里那段自陈）。
// 余量分布：seed 17 / evidence_weight 10，其中生产点 15 处已改走 analyst_base_of、
// 6 处是测试夹具断言产端 id
// （seed 3 + evidence_weight 6，都是 `find(|a| a.analyst_id == "a-…")` 这类对产端 id 的断言）。
// 归零的正解不是把测试改成 `unwrap_or` 糊过去，而是 B2-2 建带档节点时**用 helper 拼期望 id**
// （`analyst_node_id(base, tier)`），断言才有牙齿。届时把本值改 0 ⇒ 规则自动变硬拦。
// ⚠ R3 只量**比较点**。种子内部 `vec!["a-fundamentals".into(), …]` 这类**列表字面量**
//   与权威表同文件，R1/R3 都不覆盖 ⇒ 随 B2-2 建 23 节点那一批同改，由 `check-input-mapping.mjs`
//   的 DAG 面兜住。「门绿」不等于「待修面已清」，这句话必须留在代码里。
const NAIVE_BASELINE = 27;

function scan(code, bases) {
  const r1 = [];
  for (const m of code.matchAll(FULL_ID)) {
    const id = m[1];
    if (!bases.has(id) && !TIER_SNAKE.some((t) => id.endsWith(`--${t}`))) {
      if (/^a-[a-z0-9-]*--/.test(id)) continue; // 带档形态交 R2
      r1.push(id);
    }
  }
  const r2 = [];
  for (const m of code.matchAll(SUFFIXED)) r2.push(m[0]);
  // R3 判的是「拿**未归一**的 id 去比」，不是「字面量出现过」。
  //   归一的认法按**变量名**而非行距：先从全文收集「由 analyst_base_of / analystBaseOf
  //   绑定出来的变量名」，再看每个命中点的**比较主语**是不是其中之一。
  //   为什么不用「上方 N 行内有调用」当见证（首版就是这么写的，实测假阳）：长 match 块
  //   有十个臂，见证只出现在块头，按行距判就把已接线的代码继续算成违规 ——
  //   棘轮于是量不到真实进度，逼人把基线当噪声处理（本仓「门大批报错先否证判据」那条）。
  // R3 目前**只按字面量比较点全量计数**（不含「是否已归一」的判据）。
  //   为什么退回这么笨：想判的其实是「拿未归一的 id 去比」，而我连着写错三版见证——
  //   ① 「命中点上方 6 行内有 analyst_base_of」⇒ 长 match 块（十个臂）把已接线代码算成违规；
  //   ② 按**行**正则认绑定 ⇒ 扛不住 `cargo fmt` 折行，读数从 9 弹回 13；
  //   ③ 按**语句**（split(';')）认绑定 ⇒ 实测检不出绑定、退化成全量。
  //   一个会把进度读没的判据比笨判据更坏（本仓「门大批报错先否证判据，不可信就删门」），
  //   所以这里保留笨而单调的口径：**基线 = 当前实测全量**，只许降不许升；
  //   「已归一」的豁免随 B2-2 建带档节点那一批一起做（届时比较点会整体重写，判据也一并立对）。
  let r3 = 0;
  for (const re of NAIVE_CMP) for (const m of code.matchAll(re)) r3 += 1;
  // 「名字登记面」的识别**按文件内容**，不按文件名清单 —— 登记显示名的地方本来就必须把
  // base 字面量写全（`"a-lockup" => "解禁观察"`），把它算成裸匹配会让 R3 反过来惩罚补登记。
  // 判据本体（表内 id 是否全覆盖）已交给 Rust 测试
  // `evidence_citation::tests::every_authority_analyst_has_a_display_name`。
  // ⚠ 哪天 `analyst_display_name` 改名或搬走，这条豁免**自动失效**（⇒ R3 立刻把它算回去）。
  const registry = code.includes("fn analyst_display_name");
  return { r1: [...new Set(r1)], r2: [...new Set(r2)], r3, registry };
}

/// R3 的实际计入量：登记面豁免，其余全量。单独成函数是自测的靶子
/// （负控要能分别断言「登记面不 count」与「同一段代码放别处必须 count」）。
function r3Counted(row) {
  return row.registry ? 0 : row.r3;
}

function run(files, bases) {
  const out = [];
  for (const rel of files) {
    const abs = p(rel);
    if (!fs.existsSync(abs)) {
      out.push({ rel, missing: true });
      continue;
    }
    const code = codeOnly(fs.readFileSync(abs, "utf8"));
    const s = scan(code, bases);
    const exempt = HELPER_OR_FIXTURE.has(rel);
    out.push({
      rel,
      ...s,
      exempt,
      hit: (!exempt && (s.r1.length || s.r2.length)) || (!s.registry && s.r3 > 0),
    });
  }
  return out;
}

// ── 自证：三条规则都得**有牙**（正样本 + 每条一个坏样本）──
function selftest() {
  const bases = authoritativeBases();
  let fails = 0;
  const chk = (name, cond, got) => {
    console.log(`${cond ? "PASS" : "FAIL"} ${name}${cond ? "" : ` ⇒ ${JSON.stringify(got)}`}`);
    if (!cond) fails += 1;
  };
  chk("权威 base 集非空（推导成功）", bases.size >= 10, [...bases]);
  const badBase = 'let x = "a-not-an-analyst";';
  chk("R1 抓到不在权威集的 base", scan(badBase, bases).r1.includes("a-not-an-analyst"), scan(badBase, bases).r1);
  const badSuffix = 'let y = "a-sentiment--mid";';
  chk("R2 抓到手抄的带档 id", scan(badSuffix, bases).r2.length === 1, scan(badSuffix, bases).r2);
  const badNaive = 'if id == "a-news" {\n  match kind { "a-sector" => 1, _ => 0 }\n}';
  chk("R3 计数含两种裸匹配（==/!= 与 match 分支）", scan(badNaive, bases).r3 === 2, scan(badNaive, bases).r3);
  // 「包成 `Some(..)` 的裸比较」必须仍被正则抓到（首版漏 `Some(` 前缀，门放过了最典型的 Rust 写法）。
  // ⚠ 判据改成「看有没有归一见证」之后，这条样本**不能**在上方带 `analyst_base_of` ——
  // 带了就变成合法写法（另一条控制正是测这个），两类样本要各自守住自己的前提。
  const gone = 'if analyst_id == Some("a-news") { }';
  chk(
    "R3 抓到包成 Some(..) 的未归一比较（正则覆盖面：Option 包裹不算免检）",
    scan(codeOnly(gone), bases).r3 === 1,
    scan(codeOnly(gone), bases).r3,
  );
  // 归一见证：同一个字面量，上方有没有 `analyst_base_of` 决定它算不算违规
  // （样本用数组 join 拼，不写转义换行 —— 脚本自身被生成工具改写过两次，形态要挑最笨的）
  const normalized = [
    "let b = analyst_base_of(id).unwrap_or(id);",
    "match b {",
    '  "a-news" => 1,',
    "  _ => 0",
    "}",
  ].join("\n");
  chk(
    "笨口径如实陈：归一写法**照样计入**（豁免判据待 B2-2，别把它当已实现）",
    scan(codeOnly(normalized), bases).r3 === 1,
    scan(codeOnly(normalized), bases).r3,
  );
  const rawMatch = ["match id {", '  "a-news" => 1,', "  _ => 0", "}"].join("\n");
  chk("未归一的 match 仍计", scan(codeOnly(rawMatch), bases).r3 === 1, scan(codeOnly(rawMatch), bases).r3);
  const noComment = '// if id == "a-news" { 注释里的不算 }\nif id == "a-news" {}';
  // 这里必须**先过 codeOnly**（首版把原文喂给 scan ⇒ 数到 2，是门自己的覆盖面 bug，不是判据错）
  chk("剥注释：散文里的裸匹配不计", scan(codeOnly(noComment), bases).r3 === 1, scan(codeOnly(noComment), bases).r3);
  // 登记面豁免的两侧控制：有 `analyst_display_name` ⇒ 不计；把函数改名 ⇒ 立刻计。
  // 存在理由：豁免若按**文件名**写死，文件改名/函数搬走后会静默失效（变成永久白名单）；
  // 按内容判定就必须能被这条负控抓到。
  const NL = String.fromCharCode(10);
  const regCode = [
    "fn analyst_display_name(id: &str) -> String {",
    '    match id {',
    '        "a-lockup" => "解禁观察".into(),',
    '        _ => id.to_string(),',
    '    }',
    '}',
  ].join(NL);
  chk(
    "登记面（真有 analyst_display_name）的 base 字面量不计 R3",
    r3Counted(scan(regCode, bases)) === 0,
    r3Counted(scan(regCode, bases)),
  );
  chk(
    "同一段代码改个函数名就必须计 R3（豁免按内容不按文件名）",
    r3Counted(scan(regCode.replace("fn analyst_display_name", "fn other_fn"), bases)) === 1,
    r3Counted(scan(regCode.replace("fn analyst_display_name", "fn other_fn"), bases)),
  );

  console.log(fails === 0 ? "\n自证全绿" : `\n自证红 ${fails} 条`);
  process.exit(fails === 0 ? 0 : 1);
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--selftest")) return selftest();
  const bases = authoritativeBases();
  if (bases.size < 10) {
    console.error(`推导失败：权威 base 集只拿到 ${bases.size} 个（种子分析师表形态变了？）`);
    process.exit(1);
  }
  const rows = run(SCOPE, bases);
  let naive = 0;
  const hard = [];
  const notes = [];
  for (const r of rows) {
    if (r.missing) hard.push(`${r.rel}：扫描面里的文件不存在（改名/删除要同步本清单）`);
    naive += r3Counted(r);
    const kn = KNOWN_FINDINGS.filter((k) => k.file === r.rel).flatMap((k) => k.ids);
    const restR1 = r.r1.filter((id) => !kn.includes(id));
    const shadowed = r.r1.filter((id) => kn.includes(id));
    if (shadowed.length) {
      const owner = KNOWN_FINDINGS.find((k) => k.file === r.rel).owner;
      notes.push(`${r.rel} 的 ${JSON.stringify(shadowed)} 走**带日期豁免**（归 ${owner}），不是已修`);
    }
    if (!r.exempt && (restR1.length || r.r2.length)) {
      hard.push(`${r.rel}：R1 非权威 base ${JSON.stringify(restR1)} / R2 手抄带档 id ${JSON.stringify(r.r2)}`);
    }
  }
  if (argv.includes("--dump")) {
    console.log(`权威 base 集（${bases.size}）：${[...bases].sort().join(" ")}`);
    for (const r of rows) {
      const tag = r.exempt ? "[豁免R1R2]      " : r.registry ? "[登记面不计R3] " : "              ";
      console.log(`${tag} ${String(r.r3).padStart(3)}  ${r.rel}`);
    }
  }
  console.log(`\n裸匹配总数（R3）= ${naive}，基线 = ${NAIVE_BASELINE}`);
  if (naive > NAIVE_BASELINE) {
    console.error(`R3 红：裸匹配从 ${NAIVE_BASELINE} 升到 ${naive} ⇒ 新写了一处按整串 id 比较的地方。\n`);
    process.exit(1);
  }
  if (hard.length) {
    console.error(`R1/R2 红 ${hard.length} 条：`);
    for (const h of hard) console.error("  · " + h);
    process.exit(1);
  }
  for (const n of notes) console.log("⚠ " + n);
  console.log(
    naive === 0
      ? "全绿：R1/R2 无违规，R3 = 0（带档 id 已全部走 helper）"
      // ⚠ 这个数值**不是进度条**：27 处里 17 处其实已走 `base_id` 中介，而 R3 只按字面量
      //   全量计数（「是否已归一」的判据试过三版都会把进度读没，见文件头 ①②③）。
      //   所以它约束的是「不许再新增裸比较」，不是「降到 0 才算修完」。
      : `全绿（R3 = ${naive}/${NAIVE_BASELINE || 0}，这是**上限约束**不是进度：数值里含已走 base_id 中介的登记/比较点）`,
  );
}

main();
