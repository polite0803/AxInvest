// 面板「技术指标」8 个可调变量的**域归属 + 默认值对账 + 消费者存在**门（裁定 2，PLAN §一○五）
//
// ── 为什么必须有这道门 ──
// 这 8 个变量此前**只有两处**：`seed_variables.rs` 的声明 + 设置面板一个可编辑分组。
// 全仓没有任何节点把它们写进 `input_mapping` ⇒ 面板改值对评分零影响，属本仓登记的
// 「配置项空接线」族（同族先例：`value_dcf_*` 的 C2 路径 Z、`is_async` 的 B-2b 前置 1）。
//
// 裁定 2 是「先加设施再接线」。接线之后新增了一条**边界**，而边界两侧都可能坏：
//   · **漏接** ⇒ 回到空接线（面板改了没反应）；
//   · **多接** ⇒ 把窗口域五条接到四档评分节点上 ⇒ 四档的指标窗口重新焊成同一份，
//     正是 #41 片 A 刚拆掉的缺陷（工具侧会显式失败，但「失败」发生在运行时而不是 CI）。
// ⇒ 所以本门是**双向**的：接了要能看见、接错要能拦住。
//
// ── 三条判据 ──
//   P1 默认值对账：面板 `b("<变量>", <默认值>, …)` 的默认值 ≡ `IndicatorConfig::default()`
//      同名字段 ⇒ 保证「接线」本身在默认态**不改现网数值**（面板那份是手抄的，抄错就分叉）。
//   P2 域归属：**窗口域 5 条**只出现在主图（`ind_*` 参数 + 身份映射），且**不得**出现在
//      档模板、也不得被父扇出带进子图；**阈值域 3 条**必须主图 + 档模板**两边都在**，
//      且四条父扇出各带一条同名身份映射（少一条 = 整档子执行失败，不是那条不生效）。
//   P3 消费者存在：8 个变量名各自至少有一个 `("ind_<名>", "<名>")` 配对 ⇒ 「声明了但没人读」复发即红。
//
// ── 用法 ──
//   node scripts/check-indicator-config-scope.mjs            # 门禁模式（退出码 0/1）
//   node scripts/check-indicator-config-scope.mjs --selftest # 五条负控，每条必须真的能红
//   node scripts/check-indicator-config-scope.mjs --dump     # 打三份抽取结果，排漂移时用
//
// ⚠ 扫描面是**这 8 个具名变量**（定向门），不是「所有声明变量」的全域门 —— 全域反向判据
//   （任何种子变量都得有消费者）会把「由 Rust 在播种期直接读走的变量」误报成无源，
//   那类消费点在本仓有数十处且形态不一（`debate_max_rounds` / `kline_limit` …）。
//   解析失败一律当**红**（不当「没有违规」）；两份手抄源（面板 TS 与 Rust default）任一取不到字段即红。

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(resolve(ROOT, rel), "utf8");

const PANEL_REL = "src/components/settings/StockAnalysisConfigPanel.tsx";
const CFG_REL = "src-tauri/crates/astock-data/src/indicators.rs";
const SEED_REL = "src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs";
const BUILDER_REL = "src-tauri/src/commands/stock_analysis_setup/horizon_tier_template.rs";

/// 窗口域（「几根 bar」）：档侧由 `ScaleWindowPlan` 决定 ⇒ **只作用日线链**。
const WINDOW_VARS = ["macd_fast", "macd_slow", "macd_signal", "boll_period", "volume_lookback"];
/// 阈值域（比值/倍数）：§九十六(2) 裁「阈值不随尺度缩」⇒ **全链接受**。
const THRESHOLD_VARS = ["boll_stddev", "volume_surge_ratio", "volume_shrink_ratio"];
const ALL_VARS = [...WINDOW_VARS, ...THRESHOLD_VARS];

const count = (hay, needle) => hay.split(needle).length - 1;

/** 面板那侧的 `b("<名>", <默认值>, …)` ⇒ { 名: 值 }；一个都没取到 ⇒ null（判据没电） */
export function panelDefaults(src) {
  const out = {};
  const re = /\bb\("([a-z_]+)",\s*(-?[0-9.]+)\s*,/g;
  let m;
  while ((m = re.exec(src)) !== null) out[m[1]] = Number(m[2]);
  return Object.keys(out).length > 0 ? out : null;
}

/** `impl Default for IndicatorConfig` 里的标量字段 ⇒ { 名: 值 }；取不到 ⇒ null */
export function rustDefaults(src) {
  const at = src.indexOf("impl Default for IndicatorConfig");
  if (at < 0) return null;
  // 只扫 `Self { … }` 那一段（收尾是 8 空格的 `}`）：扫到文件末尾会把别的结构的
  // 同缩进字段行也算进来 ⇒ 一份本不该存在的第二权威，P1 会比错东西。
  const tail = src.slice(at);
  const end = tail.indexOf("\n        }");
  if (end < 0) return null;
  const body = tail.slice(0, end);
  const out = {};
  const re = /^\s{12}([a-z_]+):\s*(-?[0-9.]+),\s*$/gm;
  let m;
  while ((m = re.exec(body)) !== null) out[m[1]] = Number(m[2]);
  return Object.keys(out).length > 0 ? out : null;
}

/** 三条判据一起跑，返回问题串数组（空 = 通过）。四个入参可替换（自证用）。 */
export function checkAll(panelSrc, cfgSrc, seedSrc, builderSrc) {
  const problems = [];
  const panel = panelDefaults(panelSrc);
  const rust = rustDefaults(cfgSrc);
  if (!panel) problems.push("P1 判据没电：面板 b(\"<名>\", <数字>, …) 一条都没取到");
  if (!rust) problems.push("P1 判据没电：impl Default for IndicatorConfig 的标量字段一条都没取到");

  // ── P1 默认值对账 ──
  if (panel && rust) {
    for (const v of ALL_VARS) {
      if (!(v in panel)) problems.push(`P1 面板里没有 ${v} 的默认值（分组被改名/摘掉？）`);
      if (!(v in rust)) problems.push(`P1 IndicatorConfig::default() 里没有 ${v}`);
      if (v in panel && v in rust && panel[v] !== rust[v]) {
        problems.push(
          `P1 ${v}：面板手抄 ${panel[v]} ≠ Rust 默认 ${rust[v]} ⇒ 现网默认态会被这次接线改掉`
        );
      }
    }
  }

  // ── P2 域归属 ──
  for (const v of WINDOW_VARS) {
    const argPair = `("ind_${v}", "${v}")`;
    const identity = `("${v}", "${v}")`;
    if (count(seedSrc, argPair) < 1) problems.push(`P2 主图缺窗口域参数接入：${argPair}`);
    if (count(builderSrc, `"ind_${v}"`) > 0) {
      problems.push(
        `P2 档模板接了窗口域参数 ind_${v} ⇒ 四档指标窗口会被面板焊成同一份（档侧窗口归 ScaleWindowPlan）`
      );
    }
    if (count(seedSrc, identity) > 0) {
      problems.push(`P2 主图把窗口变量 "${v}" 身份映射进了子图（扇出不得带窗口域）`);
    }
  }
  for (const v of THRESHOLD_VARS) {
    const argPair = `("ind_${v}", "${v}")`;
    const identity = `("${v}", "${v}")`;
    if (count(seedSrc, argPair) < 1) problems.push(`P2 主图缺阈值域参数接入：${argPair}`);
    if (count(builderSrc, `"ind_${v}"`) < 1) {
      problems.push(`P2 档模板缺阈值域参数 ind_${v} ⇒ 面板对四档仍是空接线`);
    }
    const n = count(seedSrc, identity);
    if (n !== 4) {
      problems.push(
        `P2 阈值变量 "${v}" 的父扇出身份映射有 ${n} 条（应为 4 条=四条扇出各一条）⇒ 少一条那一档整段子执行失败`
      );
    }
  }

  // ── P3 消费者存在（与 P2 的 argPair 同一条面，但按变量逐个点名）──
  for (const v of ALL_VARS) {
    if (!seedSrc.includes(`("${"ind_" + v}", "${v}")`) && !builderSrc.includes(`("${"ind_" + v}", "${v}")`)) {
      problems.push(`P3 变量 ${v} 声明了却没有任何 input_mapping 读它 ⇒ 面板对它仍是空接线`);
    }
  }
  return problems;
}

function report(problems) {
  if (problems.length === 0) {
    console.log(
      `✅ 面板技术指标 8 个变量：默认值对账 + 两域归属（窗口 ${WINDOW_VARS.length} 条只日线 / 阈值 ${THRESHOLD_VARS.length} 条全链）+ 消费者存在 全部通过`
    );
    return 0;
  }
  for (const p of problems) console.log("❌ " + p);
  console.log(`\n共 ${problems.length} 条`);
  return 1;
}

function selftest() {
  const panel = read(PANEL_REL);
  const cfg = read(CFG_REL);
  const seed = read(SEED_REL);
  const builder = read(BUILDER_REL);
  const clean = checkAll(panel, cfg, seed, builder);
  if (clean.length > 0) {
    console.error("❌ 正控失败（现网就该通过）：\n" + clean.join("\n"));
    return 1;
  }
  const cases = [
    [
      "档模板塞进窗口域参数 ⇒ 必须点名 ind_macd_fast 焊档",
      checkAll(panel, cfg, seed, builder.replace(
        'input_mapping.insert("period".to_string(), "scoring_period".to_string());',
        'input_mapping.insert("period".to_string(), "scoring_period".to_string());\n    input_mapping.insert("ind_macd_fast".to_string(), "macd_fast".to_string());'
      )).some((p) => p.includes("ind_macd_fast") && p.includes("焊成同一份")),
    ],
    [
      "面板默认值手抄漂移 ⇒ 必须点名是哪个变量",
      checkAll(panel.replace('b("boll_period", 20,', 'b("boll_period", 30,'), cfg, seed, builder).some(
        (p) => p.startsWith("P1 boll_period")
      ),
    ],
    [
      "四条扇出少一条阈值映射 ⇒ 必须点名条数",
      checkAll(panel, cfg, seed.replace('("volume_shrink_ratio", "volume_shrink_ratio"),', ""), builder).some(
        (p) => p.startsWith("P2 阈值变量 \"volume_shrink_ratio\"") && p.includes("3 条")
      ),
    ],
    [
      "主图摘掉窗口参数接入 ⇒ 必须点名缺接入",
      checkAll(panel, cfg, seed.replace('("ind_macd_signal", "macd_signal"),', ""), builder).some(
        (p) => p.includes("P2 主图缺窗口域参数接入") && p.includes("ind_macd_signal")
      ),
    ],
    [
      "Rust default 整块取不到 ⇒ 必须报「判据没电」而不是静默通过",
      checkAll(panel, cfg.replace("impl Default for IndicatorConfig", "impl XyDefaultZz for IndicatorConfig"), seed, builder).some(
        (p) => p.startsWith("P1 判据没电")
      ),
    ],
  ];
  const failed = cases.filter((c) => !c[1]).map((c) => c[0]);
  if (failed.length > 0) {
    console.error("❌ 自证失败：" + failed.join(" / "));
    return 1;
  }
  console.log(`✅ 自证通过（${cases.length} 条负控 + 1 条正控，含「判据没电」一类）`);
  return 0;
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) process.exit(selftest());
  const problems = checkAll(read(PANEL_REL), read(CFG_REL), read(SEED_REL), read(BUILDER_REL));
  if (args.includes("--dump")) {
    console.log("面板手抄默认值：", JSON.stringify(panelDefaults(read(PANEL_REL)), null, 0)?.slice(0, 400));
    console.log("Rust 默认值：", JSON.stringify(rustDefaults(read(CFG_REL)), null, 0)?.slice(0, 400));
    const seed = read(SEED_REL);
    for (const v of ALL_VARS) console.log(`  ${v}: arg 配对=${count(seed, `("ind_${v}", "${v}")`)} 身份=${count(seed, `("${v}", "${v}")`)}`);
  }
  process.exit(report(problems));
}

if (process.argv[1] && process.argv[1].endsWith("check-indicator-config-scope.mjs")) main();
