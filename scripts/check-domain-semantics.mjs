#!/usr/bin/env node
/**
 * check-domain-semantics.mjs — 领域概念语义一致性扫描（O2 语义出口校验 · report-only）
 *
 * 存在理由
 * --------
 * 本项目有 20+ 契约脚本，清一色是**结构检查**（引用是否为零 / i18n key 是否齐全 /
 * 行尾是否 LF / 命令是否注册）。**没有一条在问「这个字段在当前代码里到底是什么量纲」**。
 * 后果（已实证）：`PLAN-semantica-borrowings.md` 登记的「语义病点」在 2026-09-14 复检时
 * 发现——1 条我写错、1 条早已修掉、1 条查无实据。**没有权威源的概念，其"现状"会腐烂，
 * 而没有任何机制会告诉你它腐烂了。**
 *
 * 本脚本做两件事
 * --------------
 * 【A】量纲锚点扫描：对一组领域概念收集代码中的「量纲锚点」并聚类，区分
 *      **域内不一致**（真问题）与**跨域同名**（同名不同概念，提示级）。
 * 【B】PLAN 断言复检：对 PLAN 登记的病点逐条实检，报「仍存在 / 已消失 / 查无实据 /
 *      PLAN 写错」，把「文档腐烂」变成可重复运行的判定。
 *
 * 设计纪律（对应铁律 7：审计脚本自身会撒谎）
 * ------------------------------------------
 *  1. 扫到 0 文件 ⇒ **非 0 退出**，绝不静默通过
 *  2. **扫描面自证**：打印文件数/锚点数，「0 命中」与「没扫到」必须可区分
 *  3. `--selftest` **正负对照**：含 RATIO/PERCENT/MONEY 三类正例 + 变量比较/注释/测试
 *     上下文三类反例
 *  4. **概念必须绑定「域」**：`confidence` 在 `crates/agent`（LLM 置信度 0–1）与
 *     `analysis-engine`（决策置信度 0–100）是**两个不同概念**。不分域 ⇒ 216 条假阳性。
 *     ⇒ 本版把「域内冲突」与「跨域同名」分成两个报告段，不做合并。
 *  5. **识别并跳过测试上下文**（`#[cfg(test)]` / `mod tests` / `describe(`）——
 *     Rust 测试模块与 `assert!` 内的字面量不是生产口径证据，不跳过会淹没信号。
 *  6. **词边界允许字段访问**：`p.position_pct` / `self.confidence` 是最常见形式，
 *     用 `(?<![\w.])` 会把它们全拒掉（本脚本 v1 的真实 bug）。
 *     正确边界 = `(?<![A-Za-z0-9_])…(?![A-Za-z0-9_])`。
 *  7. **换算点（SCALE）与量纲簇分开**：`posterior * 100.0` 是合法换算，
 *     把它算进「百分比簇」会凭空造出冲突（v1 的真实 bug）。
 *  8. **注释行不参与**：`旧式 (dqi-50)/50 把…` 是对历史的描述，不是现状（v1 的真实 bug）。
 *  9. **门禁只拦「代码态」，不拦「待办态」**：A 段（未登记量纲）由代码推出 ⇒ 可拦；
 *     PLAN 病点是**待办清单**，按定义就记着未修项 ⇒ 拦它等于逼人删条目，信号反而消失。
 *     需要把文档腐烂也纳入门禁时显式加 `--strict-plan`。
 * 10. **结案项要转成正向回归断言**：O-claim-3 这类「PLAN 原文写错了」的检查在 PLAN
 *     更正后会永远报同一句话（垃圾信号）。结案时应改写为「不得再漂回去」的正向守卫。
 *
 * 用法
 * ----
 *   node scripts/check-domain-semantics.mjs                  # report-only，恒 exit 0
 *   node scripts/check-domain-semantics.mjs --strict         # 未登记量纲 > 0 ⇒ exit 1（CI 门禁用）
 *   node scripts/check-domain-semantics.mjs --strict-plan    # 上述 + PLAN 病点 > 0 也拦（人工巡检用）
 *   node scripts/check-domain-semantics.mjs --json
 *   node scripts/check-domain-semantics.mjs --selftest
 *   node scripts/check-domain-semantics.mjs --concept=posterior
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

const argv = process.argv.slice(2);
const ARG = (k) => argv.some((a) => a === `--${k}` || a.startsWith(`--${k}=`));
const ARGV = (k, d = null) => {
  const hit = argv.find((a) => a.startsWith(`--${k}=`));
  return hit ? hit.slice(k.length + 3) : d;
};

const STRICT = ARG("strict");
const STRICT_PLAN = ARG("strict-plan");
const REPORT_ONLY = !STRICT && !STRICT_PLAN;
const JSON_OUT = ARG("json");
const SELFTEST = ARG("selftest");
const ONLY = ARGV("concept");

// ── 扫描面 ────────────────────────────────────────────────────────────
const SCAN_ROOTS = ["src-tauri/crates", "src-tauri/src", "src"];
const EXTS = new Set([".rs", ".ts", ".tsx", ".rhai"]);
const SKIP_SEG = [
  "/target/", "/node_modules/", "/dist/", "/output/", "/.git/",
  "/backup-", "/__tests__/", "/fixtures/", "/.workbuddy/",
];

// ── 概念注册表（权威源在 harness，本脚本不再自带清单）─────────────────────
/**
 * **为什么不在脚本里写清单** —— 那正是 v1 的根因：
 * 每个表面名只能有一个 `expect`（期望量纲），而现实里 `confidence` 一个名字
 * 承载 9 个概念、两种量纲（0–1 与 0–100）⇒ **数据结构无法表达现实**，
 * 必然把其中一半的出现误报成「冲突」。
 *
 * 现在改为读 `crates/harness/src/domain_semantics.rs` 的 `CONCEPTS`：
 * 名字 → 概念集合（各自带 unit / 值域 / 承载者 / 依据）。
 *
 * 判据随之升级：
 *   · 现场量纲簇**被登记概念的量纲覆盖** ⇒ 已知多义，**不是缺陷**；
 *   · 现场量纲簇**不被任何登记概念覆盖** ⇒ 真异常（新量纲未经登记）。
 */
const REGISTRY_REL = "src-tauri/crates/harness/src/domain_semantics.rs";

/** 解析 Rust 侧 `CONCEPTS` 常量表（格式固定：每字段一行、以 `},` 收尾）。 */
function parseRegistry() {
  const abs = path.join(ROOT, REGISTRY_REL);
  let text;
  try {
    text = fs.readFileSync(abs, "utf8");
  } catch (e) {
    throw new Error(`概念注册表不可读：${REGISTRY_REL} —— ${e.message}`);
  }
  return parseRegistryText(text);
}

/** 纯解析：从源码文本提取 `ConceptDecl { … }` 条目（拆出来是为了能用畸形输入做负向对照）。 */
function parseRegistryText(text) {
  const out = [];
  let cur = null;
  for (const raw of text.split(/\r?\n/)) {
    const t = raw.trim();
    if (!cur) {
      if (/^ConceptDecl\s*\{/.test(t)) cur = {};
      continue;
    }
    if (/^\},?$/.test(t)) {
      if (cur.id && cur.name && cur.unit) out.push(cur);
      cur = null;
      continue;
    }
    const m = /^(id|name|carrier|unit|min|max|meaning|evidence):\s*(.+)$/.exec(t);
    if (!m) continue;
    const key = m[1];
    const v = m[2].trim().replace(/,$/, "").trim();
    if (key === "unit") {
      cur.unit = v.replace(/^Unit::/, "");
      continue;
    }
    if (key === "min" || key === "max") {
      cur[key] = v === "f64::INFINITY" ? Infinity : Number(v);
      continue;
    }
    cur[key] = v.replace(/^"(.*)"$/s, "$1");
  }
  return out;
}

/** 量纲 → 该量纲下「可接受的观察簇」。ONE / ZERO 无判别力，各量纲下均视为通过。 */
const UNIT_CLUSTERS = {
  Ratio: new Set(["RATIO", "ONE", "ZERO"]),
  Percent: new Set(["SCALED", "ONE", "ZERO"]),
  Cny: new Set(["LARGE", "ONE", "ZERO"]),
  // 损失/误差值：越小越好、无上界 ⇒ 任意非负数都合法（含哨兵大值）
  Loss: new Set(["RATIO", "ONE", "SCALED", "LARGE", "ZERO"]),
};

/** 按表面名聚合注册表 —— 每个名字一个条目，携带它的全部概念。 */
function buildConcepts() {
  const registry = parseRegistry();
  const byName = new Map();
  for (const d of registry) {
    if (!byName.has(d.name)) byName.set(d.name, []);
    byName.get(d.name).push(d);
  }
  return [...byName.entries()].map(([name, decls]) => ({
    name,
    decls,
    units: [...new Set(decls.map((d) => d.unit))],
    coveredClusters: new Set(decls.flatMap((d) => [...(UNIT_CLUSTERS[d.unit] || [])])),
    // scope 已废弃：用路径前缀近似「域」必然误判
    // （workflow_ai/generate.rs 位于 src/commands/ 下，但它是节点推荐、不是投资决策域）
    scope: [],
  }));
}

const CONCEPTS = buildConcepts();

// 注册表解析为 0 条 ⇒ 扫描什么都查不到，会静默「零命中」。显式拦下（铁律 7）。
if (CONCEPTS.length === 0) {
  console.error(`✖ 注册表解析出 0 个概念（${REGISTRY_REL}）⇒ 扫描无意义，结论不可信。非 0 退出。`);
  process.exit(3);
}

const DIM_LABEL = {
  RATIO: "比率 (0,1)",
  ONE: "边界 1（歧义：比率满值 or 1 元/1 天）",
  SCALED: "百分比/分数 (1,100]",
  LARGE: "金额/计数 (>100)",
  ZERO: "零（无判别力）",
};

function classify(v) {
  const a = Math.abs(v);
  if (a === 0) return "ZERO";
  if (a === 1) return "ONE";
  if (a < 1) return "RATIO";
  if (a <= 100) return "SCALED";
  return "LARGE";
}

const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
/** 正确词边界：允许 `.` 前缀（字段访问），拒绝标识符字符（含 `_`，故 `final_position_pct` 内的 `position_pct` 不匹配） */
const wordBound = (name) => `(?<![A-Za-z0-9_])${esc(name)}(?![A-Za-z0-9_])`;

// ── 文件收集 ──────────────────────────────────────────────────────────
function collectFiles() {
  const out = [];
  const walk = (dir) => {
    let entries;
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const e of entries) {
      const full = path.join(dir, e.name);
      const rel = path.relative(ROOT, full).replace(/\\/g, "/");
      const norm = "/" + rel + (e.isDirectory() ? "/" : "");
      if (SKIP_SEG.some((s) => norm.includes(s))) continue;
      if (e.isDirectory()) walk(full);
      else if (EXTS.has(path.extname(e.name))) out.push(full);
    }
  };
  for (const r of SCAN_ROOTS) {
    const abs = path.join(ROOT, r);
    if (fs.existsSync(abs)) walk(abs);
  }
  return out;
}

// ── 锚点提取 ──────────────────────────────────────────────────────────
/**
 * 四类锚点：
 *   CMP   `名称 <op> 字面量`                     —— 比较阈值，最强证据
 *   CLAMP `clamp(lo, hi)` 且本行出现该名称        —— 边界，强证据（上界即量纲）
 *   CONS  `名称: 字面量` / `名称 = 字面量`        —— 构造/赋值实际值
 *   SCALE `名称 *|/ 字面量` / 反向               —— **换算点**；合法，只报告不判冲突
 *
 * ⚠ 只认**字面量**：`confidence > threshold` 不产生锚点（静态层面无法定量纲，强猜会造假阳性）。
 */
function findAnchors(concept, line, file, lineNo) {
  const hits = [];
  const wb = wordBound(concept);
  const wbRe = new RegExp(wb);

  const cmp = new RegExp(`${wb}\\s*(>=|<=|==|!=|>|<)\\s*(-?\\d+(?:\\.\\d+)?)`, "g");
  for (const m of line.matchAll(cmp)) {
    hits.push({ kind: "CMP", op: m[1], value: Number(m[2]), cluster: classify(Number(m[2])), file, line: lineNo, text: line.trim() });
  }
  const cmpR = new RegExp(`(-?\\d+(?:\\.\\d+)?)\\s*(>=|<=|==|!=|>|<)\\s*${wb}`, "g");
  for (const m of line.matchAll(cmpR)) {
    hits.push({ kind: "CMP", op: m[2], value: Number(m[1]), cluster: classify(Number(m[1])), file, line: lineNo, text: line.trim() });
  }

  const clamp = /\.clamp\(\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*\)/g;
  for (const m of line.matchAll(clamp)) {
    if (!wbRe.test(line)) continue;
    const lo = Number(m[1]), hi = Number(m[2]);
    hits.push({ kind: "CLAMP", op: "clamp", value: hi, lo, cluster: classify(hi), file, line: lineNo, text: line.trim() });
  }

  const scaleA = new RegExp(`${wb}\\s*([*/])\\s*(\\d+(?:\\.\\d+)?)`, "g");
  for (const m of line.matchAll(scaleA)) {
    hits.push({ kind: "SCALE", op: m[1], value: Number(m[2]), cluster: classify(Number(m[2])), file, line: lineNo, text: line.trim() });
  }
  const scaleB = new RegExp(`(\\d+(?:\\.\\d+)?)\\s*([*/])\\s*${wb}`, "g");
  for (const m of line.matchAll(scaleB)) {
    hits.push({ kind: "SCALE", op: m[2], value: Number(m[1]), cluster: classify(Number(m[1])), file, line: lineNo, text: line.trim() });
  }

  const cons = new RegExp(`${wb}\\s*[:=]\\s*(-?\\d+(?:\\.\\d+)?)\\b`, "g");
  for (const m of line.matchAll(cons)) {
    const after = line.slice(m.index);
    if (/^[A-Za-z0-9_]*\s*==/.test(after)) continue; // 排除 `x == 1`
    hits.push({ kind: "CONS", op: "=", value: Number(m[1]), cluster: classify(Number(m[1])), file, line: lineNo, text: line.trim() });
  }
  return hits;
}

// ── 测试上下文识别 ────────────────────────────────────────────────────
/** 返回「测试起始行号集合」，之后的行按测试对待（不参与生产口径判定）。 */
function testBlocks(lines) {
  const start = [];
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    if (
      /^#\[cfg\(test\)\]/.test(t) ||
      /^#\[cfg\(any\(test/.test(t) ||
      /^mod tests\b/.test(t) ||
      /^describe\(/.test(t)
    ) start.push(i);
  }
  return start;
}
const inTestBlock = (starts, i) => starts.some((s) => i >= s);

// ── 多行字符串识别 ────────────────────────────────────────────────────
/**
 * 标记「处于多行原始字符串内」的行 —— Rust 里只有 `r"…"` / `r#"…"#` / `br"…"` 能跨行。
 *
 * **为什么必须排除**：`src/commands/workflow_ai/helpers.rs` 有 3 个 prompt 模板
 * （27–112 / 189–457 / 460–542，共 435 行），里面写着「Rhai 示例 — 数据转换:
 * `let result = items.filter(|i| i.score > 0.8);`」这类**文档示例**。
 * 它们不是代码，但形态与代码一致 ⇒ 会被当成真实锚点（v1/v2 遗留噪声源）。
 *
 * ⚠ 词边界 `(?<![A-Za-z0-9_])` 是必需的：否则 `let s = "four";` 里 `four"` 的
 * `r"` 会被误判为 raw string 开头（`four`/`color`/`their`/`our` 都很常见）。
 */
function rawStringBlocks(lines) {
  const inRaw = new Array(lines.length).fill(false);
  let closer = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (closer) {
      inRaw[i] = true;
      if (line.includes(closer)) closer = null;
      continue;
    }
    const re = /(?<![A-Za-z0-9_])b?r(#{0,255})"/g;
    let m;
    while ((m = re.exec(line))) {
      const cl = '"' + m[1];
      if (!line.slice(m.index + m[0].length).includes(cl)) {
        closer = cl; // 本行未闭合 ⇒ 之后的行都算字符串内容
        break;
      }
    }
  }
  return inRaw;
}

// ── 主体扫描 ──────────────────────────────────────────────────────────
function scan(files, concepts) {
  const perConcept = new Map();
  for (const c of concepts) perConcept.set(c.name, { inScope: [], outScope: [] });

  const codeLines = []; // 非注释行全文，供 PLAN 断言复检（不扫注释）
  let skippedFiles = 0;

  for (const f of files) {
    let text;
    try { text = fs.readFileSync(f, "utf8"); } catch { skippedFiles++; continue; }
    const lines = text.split(/\r?\n/);
    const rel = path.relative(ROOT, f).replace(/\\/g, "/");
    const tStarts = testBlocks(lines);
    const rawIn = rawStringBlocks(lines);

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const trimmed = line.trim();
      const isComment = trimmed.startsWith("//") || trimmed.startsWith("#") || trimmed.startsWith("*");
      if (isComment) continue;              // 注释不参与（v1 bug：注释被当现状）
      if (rawIn[i]) continue;               // 多行字符串内的文档示例不参与（v3 修：prompt 模板被当代码）
      if (inTestBlock(tStarts, i)) continue; // 测试不参与生产口径（v1 bug：assert! 淹没信号）
      codeLines.push(line);

      for (const c of concepts) {
        if (ONLY && c.name !== ONLY) continue;
        if (!line.includes(c.name)) continue;
        const hits = findAnchors(c.name, line, rel, i + 1);
        if (!hits.length) continue;
        const scoped = c.scope.length === 0 || c.scope.some((p) => rel.startsWith(p));
        const bucket = perConcept.get(c.name);
        (scoped ? bucket.inScope : bucket.outScope).push(...hits);
      }
    }
  }
  return { perConcept, codeLines, skippedFiles };
}

// ── 判定 ──────────────────────────────────────────────────────────────
const STRONG = new Set(["CMP", "CLAMP", "CONS"]);

/**
 * 判定：只看强锚点（CMP/CLAMP/CONS），换算点（SCALE）不参与（v1 bug）。
 * ONE / ZERO 不构成冲突（歧义 / 无判别力）。
 *
 * **v2 判据升级**：不再拿「一个名字一个期望量纲」比对 —— 那是 v1 的根因
 * （一个名字可承载多个概念，故不存在「该名字的期望量纲」这种量）。
 * 改为拿「注册表里该名字**全部概念**的量纲并集」比对：
 *   · `uncovered`（不被任何登记量纲覆盖）⇒ **真异常**（未登记的新量纲）
 *   · 差异本身 ⇒ **已知多义**，不是缺陷（静态无法判定每一处的归属）
 */
function judgeByName(concept, anchors) {
  const strong = anchors.filter((a) => STRONG.has(a.kind));
  const clusters = {};
  for (const a of strong) if (a.cluster !== "ZERO") clusters[a.cluster] = (clusters[a.cluster] || 0) + 1;
  const uncovered = strong.filter((a) => !concept.coveredClusters.has(a.cluster));
  return { strongCount: strong.length, clusters, uncovered };
}

// ── PLAN 断言复检 ─────────────────────────────────────────────────────
const PLAN_CLAIMS = [
  {
    id: "O-claim-1",
    text: "`total_mv` 单位「元」(B 侧) / 「万元」(C 侧) 混用",
    check: (ctx) => {
      const scale = ctx.anchorsOf("total_mv").filter((a) => a.kind === "SCALE" && (a.value === 10000 || a.value === 1e4));
      return scale.length === 0
        ? { verdict: "查无实据", detail: "全仓无 total_mv↔10000/1e4 换算锚点；且 get_total_portfolio_value = price×shares ⇒ 单位统一为元" }
        : { verdict: "仍存在", detail: `${scale.length} 处换算锚点：${scale.slice(0, 5).map((a) => `${a.file}:${a.line}`).join(", ")}` };
    },
  },
  {
    id: "O-claim-2",
    text: "`f6=(dqi-50)/50` 负值域翻转",
    check: (ctx) => {
      const live = ctx.codeMatch(/\(dqi\s*-\s*50\)\s*\/\s*50/); // 只在非注释、非测试行上找
      return live.length === 0
        ? { verdict: "已消失", detail: "非注释/非测试代码中无 `(dqi-50)/50`；portfolio-mgr.rhai:684 载明 V75(2026-09-13) 已改「只惩罚不给分」" }
        : { verdict: "仍存在", detail: `活跃表达式 ${live.length} 处` };
    },
  },
  {
    // O-claim-3 已**结案**：PLAN 原文的 `position ∈ [0,1]` 已更正为 `[0,100]`
    // （PLAN 第 173 行 ⚠ 注）。结案后不再保留「文档腐烂检查」——那种检查在文档修好后
    // 会永远报同一句话，是垃圾信号。按本仓库惯例，结案项**转成正向回归守卫**。
    id: "O-claim-3",
    text: "`position_pct` 恒为 0–100（**结案，转正向守卫**：不得再出现 0–1 口径）",
    check: (ctx) => {
      const p = ctx
        .anchorsOf("position_pct")
        .filter((a) => a.kind === "CLAMP" || a.kind === "CMP" || a.kind === "CONS");
      const hi100 = p.filter((a) => a.kind === "CLAMP" && a.value === 100);
      const hi1 = p.filter((a) => a.kind === "CLAMP" && a.value === 1);
      if (hi1.length)
        return {
          verdict: "仍存在",
          detail: `出现 0–1 口径 ${hi1.length} 处（如 ${hi1[0].file}:${hi1[0].line}）⇒ 与注册表 Unit::Percent 冲突`,
        };
      if (!hi100.length)
        return { verdict: "待人工确认", detail: `未找到 clamp(.,100) 证据（强锚点 ${p.length} 条）` };
      return {
        verdict: "已结案",
        detail: `clamp(.,100) ${hi100.length} 处、未见 0–1 口径 ⇒ 与注册表 Unit::Percent 一致（PLAN 第 173 行已更正为 [0,100]）`,
      };
    },
  },
  {
    id: "O-claim-4",
    text: "`posterior` 一物三义",
    check: (ctx) => {
      const p = ctx.anchorsOf("posterior");
      const strong = p.filter((a) => STRONG.has(a.kind));   // 排除换算点（v1 bug）
      const ratio = strong.filter((a) => ["RATIO", "ONE"].includes(a.cluster)).length;
      const scaled = strong.filter((a) => a.cluster === "SCALED").length;
      const large = strong.filter((a) => a.cluster === "LARGE").length;
      if (scaled + large === 0)
        return { verdict: "查无实据（单义）", detail: `强锚点 ${strong.length} 条全部落入比率簇（RATIO/ONE=${ratio}）⇒ 未发现第二种量纲；换算点另有 ${p.length - strong.length} 处属合法 ×100` };
      return { verdict: "仍存在", detail: `比率簇 ${ratio} / 百分比簇 ${scaled} / 金额簇 ${large}` };
    },
  },
];

// ── 门禁判定（纯函数） ────────────────────────────────────────────────
/**
 * 抽出成纯函数是**为了能自检「这个门禁真的会红吗」** —— 从不 fail 的门禁
 * 与没有门禁等价（本仓库反复出现的「声明了不生效的机制」）。
 *
 * 语义（见文件头设计纪律第 9 条）：
 * - A 段（未登记量纲）由代码推出 = 硬事实 ⇒ 一律拦截；
 * - PLAN 病点属**待办态** ⇒ 默认只报告，`strictPlan` 时才拦。
 */
function gateExit(anomalousCount, planBadCount, strictPlan) {
  return anomalousCount > 0 || (strictPlan && planBadCount > 0) ? 1 : 0;
}

// ── 自检（正负对照） ──────────────────────────────────────────────────
function selftest() {
  const cases = [
    { name: "posterior 阈值=RATIO", line: "if posterior > 0.63 { buy() }", concept: "posterior", wantHit: true, wantCluster: "RATIO" },
    { name: "confidence 误用比率", line: "if confidence < 0.5 { skip() }", concept: "confidence", wantHit: true, wantCluster: "RATIO" },
    { name: "clamp 上界=量纲", line: "let p = p.position_pct.clamp(0.0, 100.0);", concept: "position_pct", wantHit: true, wantCluster: "SCALED" },
    { name: "★字段访问必须抓到", line: "if self.confidence > 100.0 { a() }", concept: "confidence", wantHit: true, wantCluster: "SCALED" },
    { name: "★后缀名不得误抓", line: "if final_position_pct > 100.0 { a() }", concept: "position_pct", wantHit: false },
    { name: "变量比较无锚点", line: "if confidence > threshold { ok() }", concept: "confidence", wantHit: false },
    { name: "纯赋值无字面量", line: "let confidence = other_value;", concept: "confidence", wantHit: false },
  ];
  let pass = 0, fail = 0;
  for (const [i, c] of cases.entries()) {
    const hits = findAnchors(c.concept, c.line, "<self>", i + 1);
    const got = hits.length > 0;
    const clusterOk = !c.wantCluster || hits.some((h) => h.cluster === c.wantCluster);
    const ok = got === c.wantHit && (c.wantHit ? clusterOk : true);
    if (ok) pass++;
    else {
      fail++;
      console.log(`  ✖ ${c.name}`);
      console.log(`      ${c.line}`);
      console.log(`      期望 wantHit=${c.wantHit}${c.wantCluster ? ` cluster=${c.wantCluster}` : ""}，实际 ${JSON.stringify(hits.map((h) => [h.kind, h.value, h.cluster]))}`);
    }
  }
  // ── 注册表解析自证（防「权威源读不到 ⇒ 全表静默 0 命中」—— 铁律 7）──
  const reg = parseRegistry();
  const confDecls = reg.filter((d) => d.name === "confidence");
  const confUnits = new Set(confDecls.map((d) => d.unit));
  const confEntry = CONCEPTS.find((c) => c.name === "confidence");

  const goodEntry = [
    "ConceptDecl {",
    '    id: "t.x",',
    '    name: "x",',
    '    carrier: "C",',
    "    unit: Unit::Ratio,",
    "    min: 0.0,",
    "    max: 1.0,",
    '    meaning: "m",',
    '    evidence: "a.rs:1",',
    "  },",
  ].join("\n");
  const badEntryNoUnit = [
    "ConceptDecl {",
    '    id: "t.y",',
    '    name: "y",',
    '    carrier: "C",',
    "    min: 0.0,",
    "    max: 1.0,",
    '    meaning: "m",',
    '    evidence: "a.rs:1",',
    "  },",
  ].join("\n");

  const regChecks = [
    ["注册表解析出 0 条", reg.length > 0, `${reg.length} 条`],
    ["条目缺字段（id/name/unit/carrier）", reg.every((d) => d.id && d.name && d.unit && d.carrier), `${reg.filter((d) => !(d.id && d.name && d.unit && d.carrier)).length} 条`],
    ["confidence 未同时登记 Ratio 与 Percent", confUnits.has("Ratio") && confUnits.has("Percent"), [...confUnits].join("+") || "无"],
    ["coveredClusters 未覆盖 RATIO", !!confEntry && confEntry.coveredClusters.has("RATIO"), confEntry ? [...confEntry.coveredClusters].join(",") : "无该名字"],
    ["名字数未少于概念数（多义未生效）", CONCEPTS.length < reg.length, `${CONCEPTS.length} 名字 / ${reg.length} 概念`],
    ["min/max 未解析为数字", reg.every((d) => typeof d.min === "number" && typeof d.max === "number"), "ok"],
    ["★空文本应解析出 0 条（负向对照）", parseRegistryText("").length === 0, "ok"],
    ["★缺 unit 的条目应被丢弃（负向对照）", parseRegistryText(badEntryNoUnit).length === 0, "ok"],
    ["★完整条目应被接受（正向对照）", parseRegistryText(goodEntry).length === 1, "ok"],
  ];
  let rfail = 0;
  for (const [label, good, detail] of regChecks) {
    if (!good) {
      rfail++;
      console.log(`  ✖ 注册表：${label}  (${detail})`);
    }
  }

  // ── 多行字符串识别自证（含 `four"` 误判的负向对照）──
  const rawCases = [
    ["★raw string 跨行块内应被标记", ["const P: &str = r#\"", "  let s = 0.8;", "\"#;", "let real = 0.9;"], [false, true, true, false]],
    ["★普通字符串不得误判（four/color）", ['let s = "four";', 'let c = "color";'], [false, false]],
    ["★同行开闭的 raw string 不跨行", ['let a = r#"x"#;', "let b = 1;"], [false, false]],
    ["★字节串 br\"…\" 也应识别", ["let b = br#\"", "  data", "\"#;"], [false, true, true]],
  ];
  let wfail = 0;
  for (const [label, input, want] of rawCases) {
    const got = rawStringBlocks(input);
    if (!(got.length === want.length && got.every((v, i) => v === want[i]))) {
      wfail++;
      console.log(`  ✖ ${label}`);
      console.log(`      期望 ${JSON.stringify(want)}，实际 ${JSON.stringify(got)}`);
    }
  }

  const files = collectFiles();

  // ── 门禁判定自证：门禁必须**真的会红**，且默认不拦待办态 ──
  const gateCases = [
    ["A 段 0 + PLAN 0 ⇒ 放行", gateExit(0, 0, false), 0],
    ["★A 段 > 0 ⇒ 拦截（否则门禁等于没装）", gateExit(1, 0, false), 1],
    ["A 段 0 + PLAN > 0 ⇒ 默认放行（待办态不拦）", gateExit(0, 3, false), 0],
    ["★PLAN > 0 + --strict-plan ⇒ 拦截", gateExit(0, 3, true), 1],
    ["A 段 > 0 + --strict-plan ⇒ 拦截", gateExit(2, 1, true), 1],
  ];
  let gfail = 0;
  for (const [label, got, want] of gateCases) {
    if (got !== want) {
      gfail++;
      console.log(`  ✖ 门禁判定：${label}  期望 exit ${want}，实际 ${got}`);
    }
  }

  console.log("── 自检 ──────────────────────────────────");
  console.log(`  检测器正负对照  : ${pass} passed / ${fail} failed`);
  console.log(`  注册表解析自证  : ${regChecks.length - rfail}/${regChecks.length} 通过（${reg.length} 概念 / ${CONCEPTS.length} 表面名）`);
  console.log(`  多行字符串自证  : ${rawCases.length - wfail}/${rawCases.length} 通过`);
  console.log(`  门禁判定自证    : ${gateCases.length - gfail}/${gateCases.length} 通过`);
  console.log(`  扫描面自证      : ${files.length} 个源文件（0 ⇒ 配置错误）`);
  const ok = fail === 0 && rfail === 0 && wfail === 0 && gfail === 0 && files.length > 0;
  console.log(`  ${ok ? "✔ 自检通过 —— 扫描结论可信" : "✖ 自检失败 —— 扫描结论不可信，勿采信"}`);
  process.exit(ok ? 0 : 1);
}

// ── 主流程 ────────────────────────────────────────────────────────────
const UNIT_LABEL = { Ratio: "0-1 比", Percent: "0-100 百分数", Cny: "元", Loss: "损失值" };

function main() {
  if (SELFTEST) return selftest();

  const files = collectFiles();
  if (files.length === 0) {
    console.error("✖ 扫描到 0 个文件 ⇒ 目录/扩展名配置错误，结论不可信。非 0 退出。");
    process.exit(2);
  }

  const concepts = CONCEPTS.filter((c) => !ONLY || c.name === ONLY);
  const { perConcept, codeLines, skippedFiles } = scan(files, concepts);

  const results = [];
  const anchorIndex = new Map();
  for (const c of concepts) {
    const b = perConcept.get(c.name);
    const all = [...b.inScope, ...b.outScope];
    anchorIndex.set(c.name, all);
    results.push({ ...c, anchors: all, ...judgeByName(c, all) });
  }

  const ctx = {
    anchorsOf: (n) => anchorIndex.get(n) || [],
    codeMatch: (re) => codeLines.filter((l) => re.test(l)),
  };
  const claims = PLAN_CLAIMS.map((pc) => ({ ...pc, ...pc.check(ctx) }));

  const anomalous = results.filter((r) => r.uncovered.length > 0); // 真异常：未登记量纲
  const ambiguous = results.filter((r) => r.decls.length > 1); // 已知多义（非缺陷）
  const single = results.filter((r) => r.decls.length === 1);
  const planBad = claims.filter((c) => c.verdict === "仍存在" || c.verdict === "PLAN 写错");
  const anchorTotal = [...anchorIndex.values()].reduce((n, a) => n + a.length, 0);
  const strongTotal = results.reduce((n, r) => n + r.strongCount, 0);
  const uncoveredTotal = results.reduce((n, r) => n + r.uncovered.length, 0);
  const explained = strongTotal === 0 ? 1 : 1 - uncoveredTotal / strongTotal;

  if (JSON_OUT) {
    console.log(JSON.stringify({
      scannedFiles: files.length,
      skippedFiles,
      anchorTotal,
      strongTotal,
      registryConcepts: results.reduce((n, r) => n + r.decls.length, 0),
      registryNames: results.length,
      anomalous: anomalous.map((r) => ({
        name: r.name,
        registeredUnits: r.units,
        uncovered: r.uncovered.map((o) => ({ file: o.file, line: o.line, kind: o.kind, value: o.value, cluster: o.cluster, text: o.text })),
      })),
      ambiguousNames: ambiguous.map((r) => ({
        name: r.name,
        conceptCount: r.decls.length,
        units: r.units,
        clusters: r.clusters,
        concepts: r.decls.map((d) => ({ id: d.id, carrier: d.carrier, unit: d.unit, min: d.min, max: d.max, meaning: d.meaning, evidence: d.evidence })),
      })),
      planClaims: claims.map((c) => ({ id: c.id, text: c.text, verdict: c.verdict, detail: c.detail })),
    }, null, 2));
    return process.exit(0);
  }

  const L = [];
  L.push("═══ 领域概念语义扫描（report-only · 权威源 = harness::domain_semantics） ═══");
  L.push("");
  L.push(`扫描面  : ${files.length} 个源文件（测试上下文与注释行已排除）｜读失败 ${skippedFiles}`);
  L.push(`注册表  : ${results.reduce((n, r) => n + r.decls.length, 0)} 个概念 / ${results.length} 个表面名`);
  L.push(`锚点    : 合计 ${anchorTotal}（强锚点 ${strongTotal}）`);
  L.push("");

  L.push("── A. 未登记量纲（**真异常** · 本报告只有这一段是问题） ──");
  L.push("");
  if (!anomalous.length) {
    L.push("✅ 无。现场所有量纲簇都能被注册表解释。");
  } else {
    for (const r of anomalous) {
      L.push(`🔴 ${r.name} —— 注册表登记量纲 ${r.units.map((u) => UNIT_LABEL[u] || u).join(" / ")}，但现场出现未覆盖的量纲簇：`);
      for (const o of r.uncovered.slice(0, 10)) {
        L.push(`     ${o.file}:${o.line}  [${o.kind} ${o.value} ⇒ ${o.cluster}]  ${o.text.slice(0, 88)}`);
      }
      if (r.uncovered.length > 10) L.push(`     … 另有 ${r.uncovered.length - 10} 处（--json 取全量）`);
    }
  }
  L.push("");

  L.push("── B. 一名多义现场（**非缺陷** · 已登记的多义，需人工核对归属） ──");
  L.push("");
  for (const r of ambiguous) {
    L.push(`⚠ ${r.name} —— 注册表登记 ${r.decls.length} 个概念（${r.units.length} 种量纲）`);
    L.push(`    现场观察 : ${Object.entries(r.clusters).map(([k, v]) => `${k} ${v} 处`).join(" ｜ ") || "（无强锚点）"}`);
    for (const d of r.decls) {
      L.push(`      [${(UNIT_LABEL[d.unit] || d.unit).padEnd(13)}] ${d.id.padEnd(44)} ${d.carrier}`);
    }
    L.push("    ⇒ 现场量纲均在注册表登记范围内 ⇒ **已知多义**；静态无法定每一处归属，需按承载者人工核对。");
  }
  if (!ambiguous.length) L.push("（无）");
  L.push("");

  L.push("── C. 量纲单义的名字（对照） ──");
  L.push("");
  for (const r of single) {
    const d = r.decls[0];
    L.push(`${r.strongCount ? "✅" : "⚪"} ${r.name.padEnd(18)} ${(UNIT_LABEL[d.unit] || d.unit).padEnd(13)} 强锚点 ${String(r.strongCount).padStart(3)} ｜ 簇 ${JSON.stringify(r.clusters)}`);
    L.push(`      ${d.meaning}  —  ${d.carrier}  —  ${d.evidence.split("|")[0].trim()}`);
  }
  if (!single.length) L.push("（无）");
  L.push("");

  L.push("── D. PLAN 登记病点实检 + 结案项守卫（文档腐烂检测） ─────");
  L.push("");
  for (const c of claims) {
    const icon = { "仍存在": "🔴", "PLAN 写错": "🔴", "已消失": "✅", "已结案": "✅", "查无实据（单义）": "✅", "查无实据": "🟡", "待人工确认": "🟡" }[c.verdict] || "⚪";
    L.push(`${icon} ${c.id}  ${c.verdict}`);
    L.push(`      PLAN 原文 ${c.text}`);
    L.push(`      实检结论 ${c.detail}`);
  }
  L.push("");

  L.push("── E. 度量 ──");
  L.push("");
  L.push(`注册表覆盖   : ${(explained * 100).toFixed(1)}%（${strongTotal - uncoveredTotal}/${strongTotal} 个强锚点能被登记量纲解释）`);
  L.push(`未登记量纲   : ${anomalous.length} 个名字 ← **真异常**`);
  L.push(`一名多义     : ${ambiguous.length} 个名字（${ambiguous.map((r) => r.name).join(", ") || "无"}）← 待人工核对`);
  L.push(`量纲单义     : ${single.length} 个名字`);
  L.push(`PLAN 病点    : ${planBad.length}/${claims.length} 条经不起实检`);
  L.push("");
  L.push("── 局限（必读 · 防止把本报告误读成 bug 清单） ──");
  L.push("");
  L.push("1. **B 段的「一名多义」不是缺陷** —— 它是注册表已承认的事实（各概念内部自洽）。");
  L.push("   本脚本判不了「某一处出现该归属哪个概念」：那需要类型信息，静态文本做不到。");
  L.push("   ⇒ B 段是**待人工核对的清单**，不是 bug 清单。");
  L.push("2. 真正可机检的是 A 段：现场量纲**不在注册表登记范围内** ⇒ 新概念未登记，或量纲用错。");
  L.push("3. 只认字面量锚点 ⇒ 变量参与的比较（`x > threshold`）一律无法判定（宁缺勿滥，避免假阳性）。");
  L.push("4. 换算点（SCALE）不参与判定：`posterior * 100.0` 是合法换算，不是量纲错误。");
  L.push("5. 测试上下文（`#[cfg(test)]` / `mod tests` / `describe(`）与注释行已整体排除。");
  L.push("6. **A 段报 0 只说明「没发现未登记量纲」**，不说明注册表完备 —— 登记本身可能滞后于代码。");
  L.push("");
  // 门禁说明（决定 CI 语义，改动前先读设计纪律第 9 条）
  const modeLine = REPORT_ONLY
    ? "模式: report-only（恒 exit 0）。"
    : STRICT_PLAN
      ? `模式: strict-plan（未登记量纲 ${anomalous.length} / PLAN 病点 ${planBad.length}，任一 > 0 ⇒ exit 1）。`
      : `模式: strict（**只拦 A 段**未登记量纲 = ${anomalous.length}）；PLAN 病点 ${planBad.length} 条仅报告、不拦截（待办清单不该让 CI 变红），需要拦截时用 --strict-plan。`;
  L.push(modeLine);
  L.push("");

  console.log(L.join("\n"));
  process.exit(REPORT_ONLY ? 0 : gateExit(anomalous.length, planBad.length, STRICT_PLAN));
}

main();
