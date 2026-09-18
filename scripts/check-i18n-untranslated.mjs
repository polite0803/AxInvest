#!/usr/bin/env node
/**
 * i18n **未翻译值** 检查（P2-2 门禁）
 *
 * ── 为什么需要本脚本 ──
 * 现有 i18n 检查各有一段盲区：
 *   · `check-hardcoded-i18n.sh`  只查**代码里的硬编码文案**，不查 locale 值
 *   · `check_i18n.py`            只查 JSON 语法 / zh-CN 空值 / **key 是否存在**
 * 于是「key 存在、11 语言全对齐，但 de/fr/es/ru/hi/ko/ar 的值仍是中文」这一类
 * 缺陷可以对全部现有门禁保持全绿 —— 本仓库曾实测 8 种语言的 `stockAnalysis`
 * 中出现大量中文值，全部逃过检查。
 *
 * ── 判据 ──
 * 对**非汉字语言**（见 CJK_LOCALES 白名单之外的所有 locale），递归遍历全部命名空间，
 * 断言「字符串值不含 CJK 汉字」。含汉字即判违规。
 *
 * ── 覆盖范围自陈（重要）──
 * 本脚本**只**检查「值是否含汉字」。它**不**检查：
 *   · key 是否缺失 / 拼写错误（由 `check_i18n.py` 覆盖）
 *   · 值是否为空串（由 `check_i18n.py` 覆盖其中 zh-CN 部分）
 *   · 语法是否真的正确（只检查**语义质感**，不检查译文质量 —— 机器判不了）
 *   · 汉字语言（zh-CN / zh-TW / ja）自身 —— 它们含汉字是正常的
 *   · 「英文字符串但其实是中文语义」（如 "Rerun" 写成 "rerun"）—— 机器判不了
 *
 * ── 基线机制 ──
 * 存量违规量级很大（八种语言各有约 1.8k），一次性清零不现实。故采用**棘轮**：
 *   · `scripts/i18n-untranslated-baseline.json` 记录已知未翻译项（只减不增）
 *   · 默认模式：**只有新增违规**才非 0 退出；基线项被修好只提示、不报错
 *   · `--update-baseline` 重写基线（修完一批后收紧棘轮）
 *   · `--strict-all` 忽略基线，任何违规都非 0（全部修完后切到此模式）
 *
 * 用法：
 *   node scripts/check-i18n-untranslated.mjs              # CI 用（棘轮模式）
 *   node scripts/check-i18n-untranslated.mjs --verbose    # 打印全部违规明细
 *   node scripts/check-i18n-untranslated.mjs --update-baseline
 *   node scripts/check-i18n-untranslated.mjs --strict-all
 *   node scripts/check-i18n-untranslated.mjs --self-test  # 自证判据有效
 *
 * 退出码：0 = 通过；1 = 有新增违规 / 环境异常；2 = 参数错误
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const LOCALES_DIR = path.join(ROOT, "src", "i18n", "locales");
const BASELINE_PATH = path.join(__dirname, "i18n-untranslated-baseline.json");
/**
 * 豁免清单：明确「**有意保留**中文」的值（正则，对**值**做部分匹配）。
 *
 * 典型且**合法**的保留场景（否则门禁会永久恒红，最终被禁用）：
 *   · 语言自称 —— 语言选择器惯例显示各语言 endonym（`简体中文`/`繁體中文`/`日本語`）；
 *     其中日语自称本身就用汉字，正则**无法**与中文区分。
 *   · 需用户**原样输入**的命令串（翻译后系统无法识别）。
 * 每一条都必须在清单里写明 `reason` —— 这是「有意保留」，不是「懒得翻译」。
 */
const ALLOWLIST_PATH = path.join(__dirname, "i18n-untranslated-allowlist.json");

/**
 * 汉字语言白名单 —— 这些语言的值**允许**出现 CJK 汉字。
 * ⚠️ 判据本身**无法**区分「中文汉字」与「日文汉字」：`様`/`子`/`見` 都落在
 *    CJK 统一表意文字区，`様子見` 会被判据命中。所以日语只能靠**白名单豁免**，
 *    不能靠正则区分。白名单因此是判据的一部分，不是可选项。
 *    代价：日语 locale 内若混入**简体中文特有**用词（如「买入」而非「買入」），
 *    本脚本判不出来，需人工抽查 —— 这是已知覆盖边界，已在输出中自陈。
 */
const CJK_LOCALES = new Set(["zh-CN", "zh-TW", "ja"]);

/** 判据：字符串值是否含 CJK 统一表意文字（汉字） */
export function hasChineseChars(s) {
  return typeof s === "string" && /[\u4E00-\u9FFF]/.test(s);
}

/** 递归收集「点分路径 → 含汉字的值」 */
export function collectUntranslated(node, prefix = "", out = []) {
  for (const [k, v] of Object.entries(node ?? {})) {
    const p = prefix ? `${prefix}.${k}` : k;
    if (typeof v === "string") {
      if (hasChineseChars(v)) { out.push({ key: p, value: v }); }
    } else if (v && typeof v === "object") {
      collectUntranslated(v, p, out);
    }
  }
  return out;
}

function loadLocaleFiles() {
  if (!fs.existsSync(LOCALES_DIR)) {
    console.error(`❌ locale 目录不存在: ${LOCALES_DIR}`);
    process.exit(1);
  }
  const files = fs.readdirSync(LOCALES_DIR).filter((f) => f.endsWith(".json"));
  if (files.length === 0) {
    // 铁律：审计脚本「扫到 0 个文件」必须非 0 退出，否则路径写错会静默全绿
    console.error(`❌ ${LOCALES_DIR} 下没有任何 .json locale 文件（路径或后缀可能已变）`);
    process.exit(1);
  }
  return files;
}

/** 读豁免清单。文件缺失 ⇒ 空清单（豁免是可选能力，不因缺文件而失败） */
function loadAllowlist() {
  if (!fs.existsSync(ALLOWLIST_PATH)) { return []; }
  try {
    const raw = JSON.parse(fs.readFileSync(ALLOWLIST_PATH, "utf8"));
    return (raw.patterns ?? []).map((p) => ({
      re: new RegExp(p.pattern),
      reason: p.reason ?? "",
    }));
  } catch (e) {
    console.error(`❌ 豁免清单解析失败 ${ALLOWLIST_PATH}: ${e.message}`);
    process.exit(1);
  }
}

/** 扫描时被豁免的项数（供 main 报告「有意保留」的量级） */
let allowlistHits = 0;

function scan() {
  const result = new Map(); // lang -> [{key, value}]
  const allow = loadAllowlist();
  allowlistHits = 0;
  for (const f of loadLocaleFiles()) {
    const lang = path.basename(f, ".json");
    if (CJK_LOCALES.has(lang)) { continue; }
    let obj;
    try {
      obj = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, f), "utf8"));
    } catch (e) {
      console.error(`❌ ${f} 解析失败: ${e.message}`);
      process.exit(1);
    }
    const hits = collectUntranslated(obj).filter((h) => {
      const hit = allow.find((a) => a.re.test(h.value));
      if (hit) { allowlistHits++; return false; }
      return true;
    });
    result.set(lang, hits);
  }
  if (result.size === 0) {
    console.error("❌ 没有任何非汉字语言 locale 被扫描（CJK_LOCALES 白名单可能写错了）");
    process.exit(1);
  }
  return result;
}

function loadBaseline() {
  if (!fs.existsSync(BASELINE_PATH)) { return null; }
  try {
    return JSON.parse(fs.readFileSync(BASELINE_PATH, "utf8"));
  } catch (e) {
    console.error(`❌ 基线文件解析失败 ${BASELINE_PATH}: ${e.message}`);
    process.exit(1);
  }
}

function runSelfTest(verboseOut = true) {
  const cases = [
    { input: "买入", expect: true, desc: "纯中文 → 违规" },
    { input: "风险收敛", expect: true, desc: "纯中文 → 违规" },
    { input: "P/E 市盈率", expect: true, desc: "中英混排仍含汉字 → 违规" },
    { input: "自动校准", expect: true, desc: "纯中文工具名 → 违规" },
    { input: "Buy", expect: false, desc: "英文 → 合规" },
    { input: "様子見", expect: true, desc: "日文汉字被判据命中（故 ja 必须靠白名单豁免，非正则可辨）" },
    { input: "관망", expect: false, desc: "韩文谚文 → 合规" },
    { input: "Подождать", expect: false, desc: "西里尔 → 合规" },
    { input: "", expect: false, desc: "空串（空值由 check_i18n.py 管）→ 本脚本不判" },
    { input: "{{count}} Items", expect: false, desc: "含插值占位 → 合规" },
  ];
  let bad = 0;
  for (const c of cases) {
    const got = hasChineseChars(c.input);
    if (got !== c.expect) {
      console.error(`❌ 自检失败: ${c.desc} input=${JSON.stringify(c.input)} expect=${c.expect} got=${got}`);
      bad++;
    }
  }
  // 对照一：递归收集必须能穿透嵌套命名空间（历史漏检正是「只查顶层」）
  const nested = collectUntranslated({ a: { b: { c: "中文" } }, d: "ok" });
  if (nested.length !== 1 || nested[0].key !== "a.b.c") {
    console.error(`❌ 自检失败: 嵌套收集应命中 a.b.c，实际 ${JSON.stringify(nested)}`);
    bad++;
  }
  // 对照二：数组**应当**被穿透（locale 里可能存在字符串数组，其元素同样需要翻译），
  //         非字符串标量（数字/布尔/null）不得被误报。
  const arr = collectUntranslated({ a: { b: ["中文"] }, c: 1, d: null, e: true });
  if (arr.length !== 1 || arr[0].key !== "a.b.0") {
    console.error(`❌ 自检失败: 数组元素应被穿透并命中山 a.b.0 且标量不得误报，实际 ${JSON.stringify(arr)}`);
    bad++;
  }
  // 对照三：白名单必须真的生效（否则 je/zh 会被误报，门禁一上线就恒红）
  if (!CJK_LOCALES.has("ja") || !CJK_LOCALES.has("zh-CN") || !CJK_LOCALES.has("zh-TW") || CJK_LOCALES.has("de")) {
    console.error(`❌ 自检失败: CJK_LOCALES 白名单异常 ${JSON.stringify([...CJK_LOCALES])}`);
    bad++;
  }
  // 对照四：真实 locale 目录必须能被扫到非空语言集合（防「路径写错 ⇒ 静默扫 0 文件」）
  const scanned = scan();
  if (scanned.size === 0) {
    console.error("❌ 自检失败: 未扫描到任何非汉字语言 locale");
    bad++;
  }
  // 对照五：豁免清单必须**既生效又不过宽**。过宽会把真缺陷一起放行（静默削弱门禁）。
  const allow = loadAllowlist();
  if (!allow.some((a) => a.re.test("日本語"))) {
    console.error("❌ 自检失败: 豁免清单未覆盖语言自称「日本語」（它必定被判据命中）");
    bad++;
  }
  if (allow.some((a) => a.re.test("买入"))) {
    console.error("❌ 自检失败: 豁免清单过宽 —— 普通文案「买入」不得被豁免");
    bad++;
  }
  if (allow.some((a) => a.re.test("风险收敛"))) {
    console.error("❌ 自检失败: 豁免清单过宽 —— 普通文案「风险收敛」不得被豁免");
    bad++;
  }
  if (bad === 0 && verboseOut) {
    console.log(
      `✅ 判据自检通过（${cases.length} 例判据 + 5 组对照：正/负对照、嵌套穿透、数组穿透、标量排除、白名单、目录可达、豁免清单生效且不过宽）`,
    );
  }
  return bad;
}

function selfTest() {
  process.exit(runSelfTest() > 0 ? 1 : 0);
}

function main() {
  const args = new Set(process.argv.slice(2));
  if (args.has("--self-test")) { selfTest(); }
  const verbose = args.has("--verbose");
  const updateBaseline = args.has("--update-baseline");
  const strictAll = args.has("--strict-all");

  for (const a of args) {
    if (!["--verbose", "--update-baseline", "--strict-all"].includes(a)) {
      console.error(`❌ 未知参数: ${a}`);
      process.exit(2);
    }
  }

  console.log("=".repeat(64));
  console.log("i18n 未翻译值检查（值含 CJK 汉字 ⇒ 判违规）");
  console.log(`汉字语言白名单（含汉字正常）: ${[...CJK_LOCALES].join(", ")}`);
  console.log("=".repeat(64));

  // 门禁先自证判据有效，再验数据 —— 否则「判据坏了」会表现为「全绿」而不是「报错」。
  // 铁律：新增 CI 检查段须自证有效 + 自陈覆盖范围；会误报/漏报的门禁终会被禁用。
  if (!updateBaseline && runSelfTest() > 0) {
    console.error("\n❌ 判据自检未通过，已中止（此时的全绿/全红都不可信）");
    process.exit(1);
  }

  const current = scan();

  if (updateBaseline) {
    const out = {};
    let total = 0;
    for (const [lang, hits] of [...current.entries()].sort()) {
      out[lang] = hits.map((h) => h.key).sort();
      total += hits.length;
    }
    fs.writeFileSync(BASELINE_PATH, JSON.stringify(out, null, 2) + "\n", "utf8");
    console.log(`\n已写入基线: ${path.relative(ROOT, BASELINE_PATH)}`);
    for (const [lang, keys] of Object.entries(out)) { console.log(`  ${lang}: ${keys.length}`); }
    console.log(`  合计 ${total} 项`);
    process.exit(0);
  }

  const baseline = loadBaseline();
  if (!baseline && !strictAll) {
    console.error(
      `\n❌ 基线文件不存在: ${path.relative(ROOT, BASELINE_PATH)}\n` +
        `   首次启用请运行: node scripts/check-i18n-untranslated.mjs --update-baseline`,
    );
    process.exit(1);
  }

  let totalCurrent = 0;
  let totalNew = 0;
  let totalResolved = 0;
  for (const [lang, hits] of [...current.entries()].sort()) {
    const base = new Set(strictAll ? [] : (baseline[lang] ?? []));
    const now = new Set(hits.map((h) => h.key));
    const added = [...now].filter((k) => !base.has(k)).sort();
    const resolved = [...base].filter((k) => !now.has(k)).sort();
    totalCurrent += now.size;
    totalNew += added.length;
    totalResolved += resolved.length;

    const flag = added.length > 0 ? "❌" : "✅";
    console.log(`\n${flag} ${lang}: 未翻译 ${now.size} 项，基线 ${base.size} 项，新增 ${added.length}，已清 ${resolved.length}`);
    if (added.length) {
      const show = verbose ? added : added.slice(0, 15);
      for (const k of show) {
        const v = hits.find((h) => h.key === k)?.value ?? "";
        console.log(`     + ${k} = ${JSON.stringify(v)}`);
      }
      if (!verbose && added.length > show.length) { console.log(`     ... 还有 ${added.length - show.length} 项（--verbose 看全部）`); }
    }
    if (resolved.length && verbose) {
      for (const k of resolved) { console.log(`     - ${k}`); }
    }
  }

  console.log("\n" + "=".repeat(64));
  console.log(`合计: 未翻译 ${totalCurrent} 项 | 新增 ${totalNew} 项 | 已清 ${totalResolved} 项`);
  console.log(`豁免: ${allowlistHits} 项（有意保留中文，见 scripts/i18n-untranslated-allowlist.json）`);
  console.log("覆盖范围: 仅「值是否含汉字」。key 缺失/拼写由 check_i18n.py 覆盖；译文质量机器判不了。");

  if (totalNew > 0) {
    console.log(`\n❌ 出现 ${totalNew} 项**新增**未翻译值。`);
    console.log("   请翻译这些 key，或（若确属临时/待译）用 --update-baseline 显式承认并记录。");
    console.log("   ⚠️ 不要为了过门禁直接把中文抄进非汉字语言 —— 那正是本门禁要防的缺陷。");
    process.exit(1);
  }
  if (totalResolved > 0) {
    console.log(`\nℹ️ 有 ${totalResolved} 项基线项已被翻译，建议收紧棘轮：`);
    console.log("   node scripts/check-i18n-untranslated.mjs --update-baseline");
  }
  console.log("\n✅ 无新增未翻译值");
}

main();
