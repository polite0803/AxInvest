#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 逐档决策腿的「键级生产者-消费者对账」门 —— 防死腿（2026-10-18，PLAN-mover-coverage §0-A）。
//
// ## 为什么要有这道门
// 触发实证：as-of 2026-09-28 百洋股份（002696）判「观望 3.3%」，其后 12 交易日 +65.3%。
// 复盘取数发现该样本两条腿**恒缺席**（现网 138 条有效决策行里各 80/80 缺席）：
//   · `sectorRotation`  ← 消费键 `industryRankPercentile`，全仓**零生产者**（只有 leg_signal.rs
//     的消费臂和它自己的测试夹具提到这个键）；
//   · `microstructure`  ← 消费键 `inLimitUpPool`，四份档脚本的 `raw` 输入对象里**没有这个键**
//     （涨停池工具本身在跑、`sealRate`/`breakCount` 都接了，唯独成员判定没接）。
// 这类断裂的特征是：**腿声明在、实现在、数据源也在，只有键名对不上** ⇒ 编译过、测试过、
// 面板只把该腿显示成「缺席」。`leg_signal.rs` 里那条「齐备态夹具」测试（`supported_factors_equal_the
// _branch_source_table`）之所以查不出，是因为它自带一份**手写全键 json**，从不经过真实的
// `raw` 对象 —— 测试把门焊死了，焊的是空气。
//
// ## 规则（消费面 = leg_signal.rs 代码域，生产面 = 四份 portfolio-mgr-h-*.rhai 的 `raw` 键）
// L1 每个被消费的输入键，必须至少在一份档脚本的 `raw` 里有生产者；缺 ⇒ 红（点名因子与键）。
// L2 每个已登记因子，其全部消费键都无生产者 ⇒ 该因子是**整条死腿**，红。
// L3 反手抄：`SUPPORTED_FACTORS`（信号实现侧）与 `VERDICT_FACTOR_SOURCES`（来源表侧）
//    必须逐名同域 —— 有人为了过本门**删腿**而不是接线，这里当场红。
// L4 齐备性下限：消费键数、因子数、扫描到的档脚本份数都不得低于现场常量，
//    解析失效导致「空集 ⇒ 恒绿」一律红（空扫描面不是通过）。
//
// 自证：`--selftest` 含①拆生产者（删掉 mid 的 `pePercentile`）②拼错键名（改名仍须命中）
//       ③删腿骗门（从来源表摘掉一个因子）④注入无生产者的假消费键 ⑤扫描面清空即红。
//
// 用法：node scripts/check-leg-producers.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// 复用 as-of 接线门的剥注释实现（禁区 12：不另造一份状态机）
import { stripRustComments } from "./check-asof-node-scope.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const LEG_SIGNAL_FILE = "src-tauri/crates/analysis-engine/src/leg_signal.rs";
const SOURCE_TABLE_FILE = "src-tauri/crates/analysis-engine/src/evidence_weight.rs";
const WIRING_FILE = "src-tauri/src/commands/stock_workflow/hooks.rs";
const BRANCH_SCRIPTS = [
  "src-tauri/src/commands/portfolio-mgr-h-ultra-short.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-short.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-mid.rhai",
  "src-tauri/src/commands/portfolio-mgr-h-long.rhai",
];

// L4 下限常量：写明「为什么是这个数」而不是随手拍 ——
// 消费键数 = 现网 16（12 因子去重后）；因子数 = 现网 12；档脚本 = 4。
// 取「现值的一半」作下限：低于它只可能是解析坏了，不可能是真实收缩（真收缩要同批改 L3 两侧名单）。
const MIN_CONSUMER_KEYS = 8;
const MIN_FACTORS = 6;
const MIN_BRANCH_SCRIPTS = 4;

// ── 棘轮基线（带日期，每次运行都打印）────────────────────────────────
// 本门建立在 2026-10-18 的**未修树**上，首读 3 条红（2 个死键 + 1 条整腿恒缺席），
// 与现网库内实测逐一对上：138 条有效决策行的 4 档 ×552 格里
// `sectorRotation` 缺席 80/80（长档全缺席）、`microstructure` 缺席 80/80。
// 登记基线**不是**把红清零：它是接线批的完成计数器 —— 每修一条必须从本表**摘掉一行**，
// 摘光即接线完成。表内条目若已不再命中，运行时会提示「可摘除」而不是报错（否则下次真修完反而红）。
const BASELINE = [
  {
    kind: "L1",
    token: "inLimitUpPool",
    since: "2026-10-18",
    why: "涨停池工具在跑（sealRate/breakCount 已接），成员判定未进 raw ⇒ 阶段 0 接线批修",
  },
  {
    kind: "L1",
    token: "industryRankPercentile",
    since: "2026-10-18",
    why: "get_industry_ranking 只喂 a-sector LLM 分析师，未落档脚本 raw ⇒ 阶段 1 逆势/板块批修",
  },
  {
    kind: "L2",
    token: "sectorRotation",
    since: "2026-10-18",
    why: "同上（该因子唯一消费键就是 industryRankPercentile，故整条腿恒缺席）",
  },
];

/** 红条是否落在基线内（按「规则号 + 键/因子名」双条件，拼进别处的同名串不算豁免）。 */
function baselineHit(red) {
  const kind = red.slice(0, 2);
  return BASELINE.find((b) => b.kind === kind && red.includes(`'${b.token}'`));
}

/** 读文件；缺失即抛（调用方按「扫描面坏」处理，绝不当空集通过）。 */
function read(rel) {
  const abs = path.join(ROOT, rel);
  if (!fs.existsSync(abs)) throw new Error(`扫描面缺文件：${rel}`);
  return fs.readFileSync(abs, "utf8");
}

/**
 * 剥 Rust 注释后，**再剥掉 `#[cfg(test)] mod tests { … }` 整块**。
 *
 * 为什么不剥就会假绿：`leg_signal.rs` 的测试模块里那份齐备态夹具把 16 个键全写了一遍，
 * 直接扫原文 ⇒ 消费面会混入测试侧拼写，而生产面（rhai）永远不含它们，两边走偏就查不出。
 * 同理，测试夹具里的键也**不能**当作生产者（禁区 12 的「测试替身不算接线」）。
 */
function rustCodeOnly(src, rel) {
  const stripped = stripRustComments(src);
  const marker = "#[cfg(test)]";
  const at = stripped.indexOf(marker);
  if (at === -1) return stripped;
  // 从标记处按大括号配平找到该 mod 的结尾，之后的内容整体丢弃
  let depth = 0;
  let i = stripped.indexOf("{", at);
  if (i === -1) return stripped.slice(0, at);
  for (; i < stripped.length; i++) {
    if (stripped[i] === "{") depth++;
    else if (stripped[i] === "}") {
      depth--;
      if (depth === 0) return stripped.slice(0, at) + stripped.slice(i + 1);
    }
  }
  throw new Error(`${rel}: #[cfg(test)] 块大括号不配平，无法可靠剥离测试面`);
}

/** 剥 Rhai `//` 行注释（块注释在本仓档脚本里不用于代码域，保守按行处理）。 */
function rhaiCodeOnly(src) {
  return src
    .split("\n")
    .filter((l) => !l.trim().startsWith("//"))
    .join("\n");
}

/** 把 `let raw = #{ … };` 的键抽出来（生产面）。返回 { keys, tiers }。 */
function branchRawKeys(rhaiSrc, rel) {
  const code = rhaiCodeOnly(rhaiSrc);
  const start = code.indexOf("let raw = #{");
  if (start === -1) throw new Error(`${rel}: 找不到 'let raw = #{'，生产面解析口径已变，门必须重定位`);
  // 从 { 起配平到对应 }，只在 literal 内找键（嵌套 #{} 里的键也一起算，那是同一份输入对象）
  let depth = 0;
  let i = code.indexOf("{", start);
  const begin = i;
  for (; i < code.length; i++) {
    if (code[i] === "{") depth++;
    else if (code[i] === "}") {
      depth--;
      if (depth === 0) break;
    }
  }
  if (depth !== 0) throw new Error(`${rel}: raw 输入对象大括号不配平`);
  const body = code.slice(begin, i);
  const keys = new Set();
  for (const m of body.matchAll(/^\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*:/gm)) keys.add(m[1]);
  return keys;
}

/** 消费面：从 leg_signal.rs 的代码域抽 (因子, 键) 对。 */
function consumedPairs(src, rel) {
  const code = rustCodeOnly(src, rel);
  const pairs = [];
  const lines = code.split("\n");
  let factor = null;
  for (let n = 0; n < lines.length; n++) {
    const arm = lines[n].match(/^\s{8}"([a-zA-Z]+)"\s=>/);
    if (arm) factor = arm[1];
    if (!factor) continue;
    const hits = [
      ...lines[n].matchAll(/\b(?:num|text)\(\s*in_\s*,\s*"([a-zA-Z0-9_]+)"/g),
      ...lines[n].matchAll(/in_\.get\(\s*"([a-zA-Z0-9_]+)"/g),
    ];
    for (const h of hits) pairs.push({ factor, key: h[1], line: n + 1 });
  }
  return pairs;
}

/** 抽 `const SUPPORTED_FACTORS: &[&str] = &[ … ];` 的名单。 */
function supportedFactors(codeOnly) {
  const at = codeOnly.indexOf("SUPPORTED_FACTORS");
  if (at === -1) throw new Error(`${LEG_SIGNAL_FILE}: 找不到 SUPPORTED_FACTORS`);
  const seg = codeOnly.slice(at, at + 1200);
  return [...seg.matchAll(/"([a-zA-Z]+)"/g)].map((m) => m[1]).slice(0, 16);
}

/** 抽 `pub const VERDICT_FACTOR_SOURCES: &[(&str,&str,&str)] = &[ … ];` 的第一列。 */
function sourceTableFactors(codeOnly) {
  const at = codeOnly.indexOf("VERDICT_FACTOR_SOURCES");
  if (at === -1) throw new Error(`${SOURCE_TABLE_FILE}: 找不到 VERDICT_FACTOR_SOURCES`);
  const seg = codeOnly.slice(at, codeOnly.indexOf("];", at));
  return [...seg.matchAll(/\(\s*"([a-zA-Z]+)"\s*,/g)].map((m) => m[1]);
}

/** 抽来源表里 kind=="injectedVar" 的行 ⇒ [(因子, 注入变量名)]。 */
function injectedVarChannels(sourceTableCodeOnly) {
  const at = sourceTableCodeOnly.indexOf("VERDICT_FACTOR_SOURCES");
  if (at === -1) throw new Error(`${SOURCE_TABLE_FILE}: 找不到 VERDICT_FACTOR_SOURCES`);
  const seg = sourceTableCodeOnly.slice(at, sourceTableCodeOnly.indexOf("];", at));
  return [...seg.matchAll(/\(\s*"([a-zA-Z]+)"\s*,\s*"injectedVar"\s*,\s*"([a-zA-Z0-9_]+)"\s*\)/g)].map(
    (m) => ({ factor: m[1], channel: m[2] }),
  );
}

/** 主检：返回红条数组（空 = 绿）。ctx 可被 selftest 用来注入变异。 */
function runCheck(ctx) {
  const reds = [];
  const pairs = consumedPairs(ctx.legSignalSrc, LEG_SIGNAL_FILE);
  const consumerKeys = new Set(pairs.map((p) => p.key));

  // L4 齐备性下限（解析失效不得伪装成通过）
  if (Object.keys(ctx.producers).length < ctx.minBranchScripts) {
    reds.push(`L4 扫到的档脚本只有 ${Object.keys(ctx.producers).length} 份（下限 ${ctx.minBranchScripts}）`);
  }
  if (consumerKeys.size < ctx.minConsumerKeys) {
    reds.push(`L4 抽到的消费键只有 ${consumerKeys.size} 个（下限 ${ctx.minConsumerKeys}）⇒ 消费面解析失效`);
  }
  const sup = ctx.supported;
  const src = ctx.sources;
  if (sup.length < ctx.minFactors || src.length < ctx.minFactors) {
    reds.push(`L4 因子名单过短（实现侧 ${sup.length} / 来源表侧 ${src.length}，下限 ${ctx.minFactors}）`);
  }

  // ⚠ 不能用 Object.values(producers).flat() —— `flat()` 只展开数组，Set 会被当成**元素**
  //   塞进结果（`[object Set]`），produced 于是恒空 ⇒ 每个因子都被判死腿（假红 12 条）。
  const produced = new Set();
  for (const keys of Object.values(ctx.producers)) for (const k of keys) produced.add(k);

  // L1 键级：每个消费键都要有生产者
  for (const key of [...consumerKeys].sort()) {
    const hitTiers = Object.entries(ctx.producers)
      .filter(([, ks]) => ks.has(key))
      .map(([t]) => path.basename(t).replace("portfolio-mgr-h-", "").replace(".rhai", ""));
    if (hitTiers.length === 0) {
      const owners = [...new Set(pairs.filter((p) => p.key === key).map((p) => `${p.factor}(${p.line})`))];
      reds.push(
        `L1 消费键 '${key}' 无任何生产者（消费方：${owners.join("、")}；档侧：${Object.keys(ctx.producers).length} 份 raw 输入对象均无此键）`,
      );
    }
  }

  // L2 因子级：整条死腿单独点名（L1 的按因子归并，报出来才看得出「这条腿从来没参与过」）
  const deadFactors = [...new Set(pairs.map((p) => p.factor))].filter(
    (f) => pairs.filter((p) => p.factor === f).every((p) => !produced.has(p.key)),
  );
  for (const f of deadFactors) reds.push(`L2 因子 '${f}' 的全部消费键都无生产者 ⇒ 整条腿恒缺席`);

  // L5（2026-10-19）：`injectedVar` 类因子**没有分析师属主**（`evidence_weight.rs` 的属主断言
  //   对它就是豁免的），"这条腿有没有人生产"因此不受那条测试保护 —— 由本规则在源码层钉：
  //   注入变量的通道名必须能在 wiring 层找到真实 push（`name: "<channel>".into()`）。
  //   负控 ⑥：把 wiring 侧那行摘掉 ⇒ 必须命中（否则豁免就成了藏身处）。
  for (const { factor, channel } of ctx.injected) {
    const pushed = ctx.wiringCode.includes(`name: "${channel}".into()`);
    if (!pushed) {
      reds.push(
        `L5 因子 '${factor}' 声明为 injectedVar，但 ${WIRING_FILE} 里没有 name: "${channel}" 的注入 ⇒ 该腿恒缺席`,
      );
    }
  }

  // L3 反手抄：两侧名单逐名同域
  for (const f of sup) if (!src.includes(f)) reds.push(`L3 信号实现侧有 '${f}'，来源表 VERDICT_FACTOR_SOURCES 没有`);
  for (const f of src) if (!sup.includes(f)) reds.push(`L3 来源表有 '${f}'，信号实现侧 SUPPORTED_FACTORS 没有`);

  return { reds, consumerKeys, produced, pairs };
}

/** 造检查上下文（selftest 在这里注入变异）。 */
function buildContext(mutate) {
  const legSignalSrcRaw = read(LEG_SIGNAL_FILE);
  const sourceSrcRaw = read(SOURCE_TABLE_FILE);
  const producers = {};
  for (const rel of BRANCH_SCRIPTS) {
    producers[rel] = branchRawKeys(read(rel), rel);
  }
  const sourceTableCode = rustCodeOnly(sourceSrcRaw, SOURCE_TABLE_FILE);
  const ctx = {
    injected: injectedVarChannels(sourceTableCode),
    wiringCode: rustCodeOnly(read(WIRING_FILE), WIRING_FILE),
    legSignalSrc: legSignalSrcRaw,
    producers,
    supported: supportedFactors(rustCodeOnly(legSignalSrcRaw, LEG_SIGNAL_FILE)),
    sources: sourceTableFactors(sourceTableCode),
    minBranchScripts: MIN_BRANCH_SCRIPTS,
    minConsumerKeys: MIN_CONSUMER_KEYS,
    minFactors: MIN_FACTORS,
  };
  return mutate ? mutate(ctx) : ctx;
}

/** 把红条分成「基线外（真失败）」与「基线内（已知待修面）」，并报已修好但没摘基线的条目。 */
function classify(result) {
  // 既收红条数组也收 runCheck 的整个结果对象：selftest 里五处变异都是直接喂结果对象的，
  // 让调用方记「取哪个字段」只会漏，漏了就是 `reds is not iterable` 这种门自己崩。
  const reds = Array.isArray(result) ? result : result.reds;
  const newReds = [];
  const hitTokens = new Set();
  for (const r of reds) {
    const b = baselineHit(r);
    if (b) hitTokens.add(`${b.kind}|${b.token}`);
    else newReds.push(r);
  }
  const stale = BASELINE.filter((b) => !hitTokens.has(`${b.kind}|${b.token}`));
  return { newReds, baselineReds: reds.length - newReds.length, stale };
}

function selfTest() {
  const failures = [];
  let assertions = 0;
  const want = (name, cond) => {
    assertions += 1;
    if (!cond) failures.push(name);
    else console.log(`  ✓ ${name}`);
  };

  // 基线：未修树上这两条必须已经在名单里（本门就是为它们而写的）
  const base = runCheck(buildContext());
  const baseCls = classify(base.reds);
  want("基线可读（消费键 ≥ 下限）", base.consumerKeys.size >= MIN_CONSUMER_KEYS);
  want("基线外为零红（未修树只该有登记的待修面）", baseCls.newReds.length === 0);
  want("基线内确有登记待修面（门没空转）", baseCls.baselineReds === BASELINE.length);

  // ① 拆生产者：把 mid/long 的 pePercentile 摘掉 ⇒ 必须新增一条基线外红，且只针对该键
  const m1 = classify(
    runCheck(
      buildContext((c) => {
        c.producers["src-tauri/src/commands/portfolio-mgr-h-mid.rhai"].delete("pePercentile");
        // valuationBand 只在 mid/long 用，两条都摘才算死键
        c.producers["src-tauri/src/commands/portfolio-mgr-h-long.rhai"].delete("pePercentile");
        return c;
      }),
    ),
  ).newReds;
  want("① 拆掉 pePercentile 生产者 ⇒ 基线外红", m1.some((r) => r.includes("'pePercentile'")));
  want("① 只让 valuationBand 那一格红", m1.filter((r) => r.includes("pePercentile")).length === 1);

  // ② 拼错键名：生产者改名 pePercentile → pePercentileV2 ⇒ 原键仍须判无生产者
  const m2 = classify(
    runCheck(
      buildContext((c) => {
        for (const t of Object.keys(c.producers)) {
          const s = c.producers[t];
          if (s.has("pePercentile")) {
            s.delete("pePercentile");
            s.add("pePercentileV2");
          }
        }
        return c;
      }),
    ),
  ).newReds;
  want("② 生产侧改名 ⇒ 假接线仍须命中", m2.some((r) => r.includes("'pePercentile'")));

  // ③ 删腿骗门：从来源表摘掉一个因子 ⇒ L3 必须红
  const m3 = classify(
    runCheck(
      buildContext((c) => {
        c.sources = c.sources.filter((f) => f !== "sectorRotation");
        return c;
      }),
    ),
  ).newReds;
  want("③ 从来源表删腿 ⇒ 红（接线才是修法，摘名字不是）", m3.some((r) => r.includes("sectorRotation")));

  // ④ 注入无生产者的假消费键 ⇒ 必须出现在红名单里
  const m4 = classify(
    runCheck(
      buildContext((c) => {
        c.legSignalSrc = c.legSignalSrc.replace(
          '"trendStrength" => {',
          '"trendStrength" => { let _ = num(in_, "__no_such_leg_key__");',
        );
        return c;
      }),
    ),
  ).newReds;
  want("④ 新增无生产者消费键 ⇒ 红", m4.some((r) => r.includes("__no_such_leg_key__")));

  // ⑤ 扫描面清空即红（解析失效不得恒绿）
  const m5 = classify(
    runCheck(
      buildContext((c) => {
        c.producers = {};
        return c;
      }),
    ),
  ).newReds;
  want("⑤ 档脚本扫描面为空 ⇒ 红", m5.some((r) => r.startsWith("L4")));

  // ⑥ L5 负控：injectedVar 的通道在 wiring 层没有注入点 ⇒ 必须命中
  const m6 = classify(
    runCheck(
      buildContext((c) => {
        c.wiringCode = "fn unrelated() {}";
        return c;
      }),
    ),
  ).newReds;
  want(
    "⑥ injectedVar 无 wiring 注入点 ⇒ 红",
    m6.some((r) => r.startsWith("L5")) && m6.some((r) => r.includes("relative_strength")),
  );

  if (failures.length > 0) {
    console.error("❌ 自检失败：");
    for (const f of failures) console.error("  - " + f);
    process.exit(1);
  }
  // 断言条数**自数**：手写死数字会在加了一条负控后继续报旧数，那是门在说谎。
  console.log(`✅ 自检通过（${assertions} 项断言全部有效）；基线内待修面 ${baseCls.baselineReds}/${BASELINE.length} 条`);
}

function main() {
  const dump = process.argv.includes("--dump");
  if (process.argv.includes("--selftest")) {
    selfTest();
    return;
  }
  let res;
  try {
    res = runCheck(buildContext());
  } catch (e) {
    console.error(`❌ 门自身跑不起来（扫描面/解析口径问题，不是被测代码问题）：${e.message}`);
    process.exit(1);
  }
  if (dump) {
    console.log("消费键：", [...res.consumerKeys].sort().join(", "));
    console.log("生产键：", [...res.produced].sort().join(", "));
  }
  const cls = classify(res.reds);
  // 计数器每次都要出声：基线不是把红藏起来，是把「还剩几条接线」写死在文件里。
  if (BASELINE.length > 0) {
    console.log(`⚠ 基线待修面 ${cls.baselineReds}/${BASELINE.length} 条（棘轮只准缩短）：`);
    for (const b of BASELINE) console.log(`   · ${b.kind} '${b.token}'（登记 ${b.since}）${b.why}`);
  }
  for (const b of cls.stale) {
    console.log(`   ▶ '${b.token}' 已不再命中 ⇒ 接线已完成，请把该行从 BASELINE 摘掉`);
  }
  if (cls.newReds.length > 0) {
    console.error(`❌ 决策腿键级对账**新增**违规 ${cls.newReds.length} 条（基线外）：`);
    for (const r of cls.newReds) console.error("  - " + r);
    console.error("  修法＝在生产侧把键接上（档脚本 raw 对象 + 上游节点产出），不是从名单里摘掉这条腿。");
    process.exit(1);
  }
  console.log(
    BASELINE.length > 0
      ? `✅ 门绿（无新增断裂）；⚠ 待修面仍剩 ${cls.baselineReds} 条 —— 门绿≠接线已完成`
      : "✅ 决策腿输入键两侧对齐（消费键全有生产者、两侧因子名单同域）",
  );
}

main();
