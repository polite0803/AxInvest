#!/usr/bin/env node
/**
 * 验证窗口集（T+N）的**唯一权威门**（B3，`PLAN-llm-horizon-decision-and-parity.md` §6.4 / 拍板 7）。
 *
 * ## 为什么要有这道门
 * `[5, 20, 60]` 那组数此前抄在四处：写侧默认值、两处文档注释、以及三个**跨行池化**的统计读点
 * （`compute_validation_report` / `reco_ic_stats` / `reco_loop`）。抄一份就多一个真相：窗口集换掉
 * 而漏改某处时那里不会报错，只会把「20 天的命中」和「28 天的命中」并成同一个分母 —— 而
 * `compute_hit_rate_report` 的 by_action / by_style 正是跨窗口池化的，混池后果直接进面板。
 *
 * ## 规则
 *  · R1 权威存在且**现场推导**：`harness::holding_period` 里两个函数都在，且函数体取自
 *     `default_holding_days`（把 2/5/28/90 字面量抄进权威 = 第四套参数，同样红）。
 *  · R2 全仓 `.rs` 的**代码行**不得再出现手抄的旧窗口数组（文档里的历史说明不算，见 codeOnly）。
 *  · R3 三个统计读点各自都必须调用 `is_validation_window`（少一个 = 那一条链继续混池）。
 *  · R4 写侧默认值必须来自 `validation_windows`，不许退回 `vec![…]`。
 *  · 每条规则都要求「定位得到被检函数/文件」——改名或搬走即红，登记表不腐烂。
 *
 * ## 自证（`--selftest`）
 * M1 权威改名 / M2 一个读点不筛 / M3 权威体改手抄 / M4 写侧退回旧数组 / M5 注释里提旧数组**不得**红。
 * 每个破坏样本都断言「只有该红的规则红」，M4 与 M5 成对证明剥注释没把代码一起吞掉。
 *
 * ## 实现纪律
 * 本文件不出现正则与转义（同 `check-generation-scope-registry.mjs`：heredoc 落盘的双反斜杠会被
 * 双层归一吃掉 ⇒ 模式恒不匹配，自证与首跑同时全红）。全部用 `indexOf` / `startsWith` 扫描。
 *
 * 用法：node scripts/check-validation-window-authority.mjs [--selftest] [--dump]
 * 退出码：0 = 通过；1 = 有违规 / 定位失败；2 = 参数错误。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

const HARNESS = "src-tauri/crates/harness/src/holding_period.rs";
const WRITE_SIDE = "src-tauri/src/commands/backtest_validation.rs";
/** 三个跨行池化的统计读点（按**文件**列，一条一个分母）。 */
const READ_POINTS = [
  { name: "compute_validation_report", file: "src-tauri/src/commands/backtest_validation.rs" },
  { name: "reco_ic_stats", file: "src-tauri/src/commands/stock_analysis.rs" },
  { name: "reco_loop", file: "src-tauri/crates/analysis-engine/src/recommender/reco_loop.rs" },
];
/** 旧窗口集字面量的两种写法（dprint 会在逗号后补空格，两种都要拦）。 */
const STALE_ARRAYS = ["[5, 20, 60]", "[5,20,60]"];

/** 递归收集 `.rs` 源文件，跳过构建产物与本地副本目录（副本会让按文件扫描失真）。 */
function listRustFiles(dir, acc) {
  for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
    if (ent.name === "target" || ent.name === "output" || ent.name.startsWith(".")) {
      continue;
    }
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      listRustFiles(full, acc);
    } else if (ent.name.endsWith(".rs")) {
      acc.push(full);
    }
  }
  return acc;
}

/**
 * 剥注释骨架：只按**行首标记**剥（`//` / `//!` / `/*` / `*`）。
 * M4（代码行含旧数组 ⇒ 红）与 M5（注释行含 ⇒ 不红）成对自证，缺一即恒绿或恒红。
 */
function codeOnly(src) {
  return src
    .split("\n")
    .filter((ln) => {
      const t = ln.trim();
      return !(t.startsWith("//") || t.startsWith("/*") || t.startsWith("*"));
    })
    .join("\n");
}

/** 取某个函数定义之后、到**它自己那行收尾 `}`** 之前的片段。
 * ⚠ 不能找 `\n}`：Rust 的 impl 方法收尾是缩进的 `    }`，找 `\n}` 会一路吞到 impl 块末，
 * 于是「改坏第一个函数体」的破坏样本仍能在后面函数里读到 `default_holding_days` ⇒ 恒绿分支
 * （M3 头一次跑就是这样漏掉 R1 的）。 */
function fnBody(src, sig) {
  const at = src.indexOf(sig);
  if (at < 0) {
    return null;
  }
  const lines = src.slice(at).split("\n");
  const body = [lines[0]];
  for (const ln of lines.slice(1)) {
    if (ln.trim() === "}") {
      break;
    }
    body.push(ln);
  }
  return body.join("\n");
}

/**
 * 一次检查的全部输入注入进来 ⇒ 负控能拆在**真正决定该断言的那道守卫**上，
 * 而不是靠改磁盘（改共享树会碰别人的未提交文件）。
 */
function runChecks(files) {
  const out = [];
  const harness = files[HARNESS];
  if (harness === undefined) {
    return [{ rule: "R1", msg: `定位不到权威文件 ${HARNESS}` }];
  }

  // R1 权威存在且现场推导
  for (const sig of ["pub fn validation_windows()", "pub fn is_validation_window("]) {
    if (!harness.includes(sig)) {
      out.push({ rule: "R1", msg: `权威函数缺失或改名：${HARNESS} 里找不到 \`${sig}\`` });
    }
    const body = fnBody(harness, sig);
    if (body !== null && !body.includes("default_holding_days")) {
      out.push({ rule: "R1", msg: `\`${sig}\` 的函数体没走 default_holding_days ⇒ 窗口集变成手抄的第二套参数` });
    }
  }

  // R2 全仓**代码行**不得再手抄旧窗口数组
  for (const [rel, raw] of Object.entries(files)) {
    if (!rel.endsWith(".rs") || rel === HARNESS) {
      continue;
    }
    const src = codeOnly(raw);
    for (const stale of STALE_ARRAYS) {
      if (src.includes(stale)) {
        out.push({ rule: "R2", msg: `${rel} 的代码里仍有手抄窗口集 ${stale}` });
      }
    }
  }

  // R3 三个读点都在分母上筛
  for (const rp of READ_POINTS) {
    const src = files[rp.file];
    if (src === undefined) {
      out.push({ rule: "R3", msg: `定位不到读点 ${rp.name} 的文件 ${rp.file}` });
      continue;
    }
    if (!src.includes("is_validation_window")) {
      out.push({
        rule: "R3",
        msg: `${rp.name}（${rp.file}）没调 is_validation_window ⇒ 该分母仍会把新旧窗口混池`,
      });
    }
  }

  // R4 写侧默认值必须**在那条语句上**取权威：只看文件里有没有 `validation_windows` 字样会恒绿 ——
  // 该字样也可能只出现在日志/注释里，而真正的默认值早已退回手抄数组。
  // ⚠ 按**语句窗口**读而不是按单行：rustfmt 会把这条链式调用折成三行
  // （`let t_plus_n_list = request` / `.t_plus_n_list` / `.unwrap_or_else(…)`），
  // 按行匹配当场失明（本门的 R4 第一次红就是这么来的 —— 源码是对的，检法读不到）。
  const ws = files[WRITE_SIDE];
  if (ws === undefined) {
    out.push({ rule: "R4", msg: `定位不到写侧文件 ${WRITE_SIDE}` });
  } else {
    const lines = codeOnly(ws).split("\n");
    const starts = [];
    for (let i = 0; i < lines.length; i += 1) {
      if (lines[i].includes("let t_plus_n_list")) {
        starts.push(i);
      }
    }
    if (starts.length === 0) {
      out.push({ rule: "R4", msg: `${WRITE_SIDE} 里找不到 t_plus_n_list 的默认值语句（写侧被搬走？）` });
    }
    for (const i of starts) {
      const stmt = lines.slice(i, i + 4).join(" ");
      if (!stmt.includes("validation_windows")) {
        out.push({ rule: "R4", msg: `写侧默认 T+N 未走 validation_windows()：${stmt.trim()}` });
      }
    }
  }

  return out;
}

function collectDiskFiles() {
  const files = {};
  for (const p of listRustFiles(path.join(ROOT, "src-tauri"), [])) {
    const rel = path.relative(ROOT, p).replace(/\\/g, "/");
    files[rel] = fs.readFileSync(p, "utf8");
  }
  // 权威与读点必须真的在扫描面里，否则「零命中」是扫描面漏了而不是没违规
  for (const rel of [HARNESS, WRITE_SIDE, ...READ_POINTS.map((r) => r.file)]) {
    if (files[rel] === undefined) {
      throw new Error(`扫描面不含 ${rel} ⇒ 本门读数不可信`);
    }
  }
  return files;
}

// ── 自证 ────────────────────────────────────────────────────────────────────
function selftest() {
  const disk = collectDiskFiles();
  const clone = () => ({ ...disk });
  let pass = 0;
  let fail = 0;

  /** `wantRules` 为空数组 = 必须全绿；否则要求「每条 want 都命中，且没有别的规则命中」。 */
  const expect = (name, mutated, wantRules) => {
    const vios = runChecks(mutated);
    const missing = wantRules.filter((r) => !vios.some((v) => v.rule === r));
    const extra = vios.filter((v) => !wantRules.includes(v.rule));
    const ok = missing.length === 0 && extra.length === 0;
    if (ok) {
      pass += 1;
      console.log(`OK   ${name}（期望红：${wantRules.join("+") || "无"}；实得 ${vios.length} 条）`);
    } else {
      fail += 1;
      console.log(`FAIL ${name}: 漏 ${missing.join("+") || "无"} / 多 ${extra.map((v) => `[${v.rule}] ${v.msg}`).join(" ; ") || "无"}`);
    }
  };

  expect("基线：当前源码零违规", disk, []);

  const m1 = clone();
  m1[HARNESS] = m1[HARNESS].replace("pub fn is_validation_window(", "pub fn renamed_window_p(");
  // 权威改名 ⇒ R1（找不到签名）；读点里那句 `Period::is_validation_window` 变成悬空调用，
  // 但 R3 只看**读点文件**的字面量，故同批必须也能从 R1 抓到 —— 这条证明「改名不会静默过关」。
  expect("M1 权威函数改名", m1, ["R1"]);

  const m2 = clone();
  m2[READ_POINTS[1].file] = m2[READ_POINTS[1].file].replaceAll("is_validation_window", "other_thing");
  expect("M2 一个读点不筛窗口", m2, ["R3"]);

  const m3 = clone();
  m3[HARNESS] = m3[HARNESS].replace(
    "Self::ALL.iter().map(|p| p.default_holding_days() as i32).collect()",
    "vec![2, 5, 28, 90]",
  );
  expect("M3 权威体改成手抄字面量", m3, ["R1"]);

  const m4 = clone();
  m4[WRITE_SIDE] = m4[WRITE_SIDE].replace(
    ".unwrap_or_else(axagent_harness::holding_period::Period::validation_windows)",
    ".unwrap_or_else(|| vec![5, 20, 60])",
  );
  expect("M4 写侧退回手抄旧窗口集", m4, ["R2", "R4"]);

  const m6 = clone();
  m6[WRITE_SIDE] = m6[WRITE_SIDE].replace("let t_plus_n_list = request", "let something_else = request");
  expect("M6 默认值语句被搬走（定位失败，不许静默过关）", m6, ["R4"]);

  const m5 = clone();
  m5[WRITE_SIDE] = `// 历史说明：旧默认是 [5, 20, 60]，现已收编到四档窗口集\n${m5[WRITE_SIDE]}`;
  expect("M5 只在注释里提旧数组：不得红（证明剥注释没吞代码）", m5, []);

  console.log(`\n自证：${pass} 绿 / ${fail} 红`);
  return fail === 0 ? 0 : 1;
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    return selftest();
  }
  if (args.some((a) => a !== "--dump" && a !== "--help")) {
    console.error(`未知参数：${args.join(" ")}`);
    return 2;
  }
  const files = collectDiskFiles();
  if (args.includes("--dump")) {
    console.log(`扫描 .rs ${Object.keys(files).length} 个；读点 ${READ_POINTS.length} 个`);
  }
  const vios = runChecks(files);
  if (vios.length === 0) {
    console.log("OK   T+N 窗口集唯一权威（R1 现场推导 / R2 代码无手抄 / R3 三读点同判据 / R4 写侧取权威）");
    return 0;
  }
  for (const v of vios) {
    console.log(`FAIL [${v.rule}] ${v.msg}`);
  }
  console.log(`\n结论: ${vios.length} 处窗口集权威被绕过`);
  return 1;
}

// 只有被**直接执行**时才退出：被别的脚本 import 时必须交出控制权，
// 否则 import 副作用里的 process.exit(0) 会把调用方的整段自证吞掉（= 假绿）。
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main());
}
