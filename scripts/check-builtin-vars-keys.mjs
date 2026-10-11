#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
// 内建变量（as-of → prompt `{{key}}`）一致门（2026-10-11）。
//
// ## 为什么要有这道门
// 2026-10-11 查「趋势智选是否支持 as-of」时抓到同族的一条：**三处文档都在说**
// 「主 crate 在 as-of 模式下通过 WorkEngine.set_builtin_vars_provider 注入
// data_freshness / as_of_date / is_replay / data_scope」，但
//   ① `WorkEngine` 上根本没有这个方法（只有 `AgentExecutor` 的槽位 setter），
//   ② 全仓零调用点，
//   ③ 连 `as_of::data_freshness_description()` 也只有自己的测试在调。
// ⇒ 分析师提示词从来不知道自己跑在回放里，而文档把这说成已具备的能力（三份手抄一起说谎）。
//
// ## 规则
// R1 硬：实现里 `vars.insert("…")` 的键集 == 权威清单 `BUILTIN_VAR_KEYS`（两侧现场抽取，
//    缺一个或多一个都算红）。
// R2 硬：**除权威文件外**，任何 `.rs` 的注释里不得再并列 ≥2 个清单名目 —— 手抄清单是
//    「文档与实现各处说一套」的源头；判据只认注释里以反引号 + " / " 并列的**手抄形态**，
//    散装提及不算 —— `as_of_date` / `data_scope` 在 astock-data 里是真实字段名（第一版按
//    「出现过」判，把 as_of.rs / lib.rs 甚至权威文件自己都报成了手抄）。
// R3 硬：`WorkEngine::set_builtin_vars_provider` 必须有 ≥1 个**主 crate** 调用点。
//    只锁清单不锁接线 = 允许「声明了键、没人注册」重新长回来。
// R4 信息（不红）：逐键扫模板槽 `{{key}}`（专家 md / rhai / 种子模板）。零消费 ⇒ 打印出来，
//    避免「线接上了」被读成「效果生效了」。当前四个键**全部零引用**，加占位符属提示词裁定。
// R5 自证：`--selftest` 正样本 + 九条负控 —— 抽不到清单/实现必须返回 null（不得静默当空集）、
//    少一个 insert 要被抽出、手抄形态必须命中、散装提及与非清单键并列不得命中、
//    真注册调用必须命中、文档自指与注释掉的调用不得命中。
//
// 用法：node scripts/check-builtin-vars-keys.mjs [--selftest] [--dump]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SERVICES = "src-tauri/src/init/services.rs";
const CONST_RE = /pub\(crate\) const BUILTIN_VAR_KEYS: \[&str; \d+\] =\s*\[([^\]]*)\]/;
const INSERT_RE = /vars\.insert\(\s*"([a-z_]+)"\.to_string\(\)/g;

function read(rel) {
  return fs.readFileSync(path.join(ROOT, rel), "utf8");
}

/** 从权威文件抽出清单常量里的键（现场抽取，不写死在本脚本里）。 */
export function extractDeclaredKeys(text) {
  const m = CONST_RE.exec(text.replace(/\s+/g, " "));
  if (!m) return null;
  return m[1]
    .split(",")
    .map((s) => (s.match(/"([a-z_]+)"/) || [])[1])
    .filter(Boolean);
}

/**
 * 从 `pub(crate) fn asof_builtin_vars` 的函数体抽出实际提供的键。
 * 用括号计数取函数边界 —— 按固定字符数截会漏/会串到下一个条目。
 */
export function extractImplementedKeys(text) {
  const at = text.indexOf("fn asof_builtin_vars()");
  if (at === -1) return null;
  let i = text.indexOf("{", at);
  if (i === -1) return null;
  let depth = 0;
  let end = -1;
  for (; i < text.length; i += 1) {
    if (text[i] === "{") depth += 1;
    else if (text[i] === "}") {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end === -1) return null;
  const body = text.slice(at, end + 1);
  const keys = [];
  let m;
  INSERT_RE.lastIndex = 0;
  while ((m = INSERT_RE.exec(body)) !== null) keys.push(m[1]);
  return keys;
}

/** 收集仓内 `.rs` 文件（排除 target / worktrees / output）。 */
function listRustFiles(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (["target", "node_modules", ".worktrees", ".workbuddy", "output", "tests"].includes(entry.name)) {
      continue;
    }
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) listRustFiles(p, out);
    else if (entry.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

/**
 * R2 的注释面：只认**手抄清单的字面形态** —— 注释里用反引号包住、以 " / " 并列的 ≥2 个键名。
 * 不能退化成「注释里出现过这些标识符就算」：`as_of_date` / `data_scope` 本身是 astock-data 里
 * 真实存在的字段名，那样判会把正常文档行全报成手抄（本门第一版就误伤三处，含权威文件自己）。
 */
export function copycatLists(fileText, keys) {
  const found = [];
  const keySet = new Set(keys);
  for (const line of fileText.split("\n")) {
    const t = line.trim();
    if (!t.startsWith("//")) continue;
    for (const run of t.matchAll(/(?:`[a-z_]+`)(?:\s*\/\s*`[a-z_]+`)+/g)) {
      const names = [...run[0].matchAll(/`([a-z_]+)`/g)].map((m) => m[1]);
      if (names.length >= 2 && names.every((n) => keySet.has(n))) found.push(names.join(" / "));
    }
  }
  return found;
}

/** R4 的模板槽消费面：扫 `{{key}}`。 */
function slotConsumers(keys) {
  const dirs = ["src-tauri/agency_experts", "src-tauri/src/commands", "src-tauri/crates"];
  const found = new Map(keys.map((k) => [k, []]));
  const walk = (p) => {
    if (!fs.existsSync(p)) return;
    for (const entry of fs.readdirSync(p, { withFileTypes: true })) {
      if (["target", "node_modules", ".worktrees"].includes(entry.name)) continue;
      const full = path.join(p, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      if (!/\.(md|rhai|rs)$/.test(entry.name)) continue;
      // 「是否被消费」只看**代码/模板域**：`.rs` 里的行注释常写「供 prompt {{key}} 注入」，
      // 那是自述不是消费方 ⇒ 剥掉行注释再扫，否则 R4 会把文档自己报成有人在用。
      let txt = fs.readFileSync(full, "utf8");
      if (entry.name.endsWith(".rs")) {
        txt = txt
          .split("\n")
          .map((l) => (l.trim().startsWith("//") ? "" : l))
          .join("\n");
      }
      for (const k of keys) {
        if (txt.includes(`{{${k}}}`)) found.get(k).push(path.relative(ROOT, full));
      }
    }
  };
  for (const d of dirs) walk(path.join(ROOT, d));
  return found;
}

/**
 * 是否真的**注册**了提供器：剥掉行注释后，匹配 `.set_builtin_vars_provider(` 调用形态。
 * 只认带前导点的调用 ⇒ 文档里的 `WorkEngine::set_builtin_vars_provider`（自指写法）
 * 与注释掉的示例都不会被算成接线。
 * ⚠ 行注释剥离按整行 `//` 与行尾 `//` 处理，不做字符串状态机：本判据的靶子是
 *    「某行是否发起了一次方法调用」，URL 出现在同一行的概率与代价都低，写明即可。
 */
export function hasRegistrationCall(text) {
  const code = text
    .split("\n")
    .map((l) => (l.trim().startsWith("//") ? "" : l.replace(/\/\/.*$/, "")))
    .join("\n");
  return /\.set_builtin_vars_provider\(/.test(code);
}

export function check() {
  const violations = [];
  const services = read(SERVICES);
  const declared = extractDeclaredKeys(services);
  if (!declared || declared.length === 0) {
    return { violations: [`判据失效: 抽不到 BUILTIN_VAR_KEYS 清单（权威面 ${SERVICES} 改了形态？）`], declared: [], implemented: [], consumers: new Map() };
  }
  const implemented = extractImplementedKeys(services);
  if (!implemented) {
    return { violations: ["判据失效: 抽不到 asof_builtin_vars 的函数体（实现面改了名字？）"], declared, implemented: [], consumers: new Map() };
  }

  // R1
  const dset = new Set(declared);
  const iset = new Set(implemented);
  for (const k of iset) if (!dset.has(k)) violations.push(`R1: 实现给了 ${k}，权威清单里没有`);
  for (const k of dset) if (!iset.has(k)) violations.push(`R1: 清单声明了 ${k}，实现里没给 ⇒ 模板会渲染成未解析`);

  // R2：手抄清单（只看注释行，且要求同一文件并列 ≥2 个）
  const copycats = [];
  for (const file of listRustFiles(path.join(ROOT, "src-tauri"))) {
    // 路径归一：Windows 下 path.relative 给反斜杠，与 SERVICES 的正斜杠字面不等 ⇒
    // 不归一就会把权威文件自己报成手抄（本门第一版踩过）。
    const rel = path.relative(ROOT, file).split(path.sep).join("/");
    if (rel === SERVICES) continue;
    for (const list of copycatLists(fs.readFileSync(file, "utf8"), declared)) {
      copycats.push(rel + " —— 注释里并列了 " + list);
    }
  }
  for (const c of copycats) violations.push(`R2: 手抄第二份键清单（唯一权威是 ${SERVICES}::BUILTIN_VAR_KEYS）：${c}`);

  // R3：接线必须在主 crate，且要认**调用形态**（`.set_builtin_vars_provider(`）——
  // 裸标识符匹配会把文档里的「应通过 WorkEngine.set_builtin_vars_provider 注入」也算成调用点，
  // 那正是本门要抓的那种说谎。
  const mainCallsites = listRustFiles(path.join(ROOT, "src-tauri/src"))
    .filter((f) => !path.basename(f).startsWith("register_commands"))
    .filter((f) => hasRegistrationCall(fs.readFileSync(f, "utf8")));
  if (mainCallsites.length === 0) {
    violations.push("R3: 主 crate 没有任何 set_builtin_vars_provider 调用点 ⇒ 提供器又是「声明了没注册」");
  }

  return { violations, declared, implemented, consumers: slotConsumers(declared), callsites: mainCallsites.map((f) => path.relative(ROOT, f)) };
}

function selftest() {
  const fails = [];

  const good = `pub(crate) const BUILTIN_VAR_KEYS: [&str; 4] =
    ["data_freshness", "as_of_date", "is_replay", "data_scope"];
pub(crate) fn asof_builtin_vars() -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert("data_freshness".to_string(), s);
    vars.insert("as_of_date".to_string(), d);
    vars.insert("is_replay".to_string(), r);
    vars.insert("data_scope".to_string(), sc);
    vars
}`;
  const declared = extractDeclaredKeys(good);
  const implemented = extractImplementedKeys(good);
  if (!declared || declared.length !== 4) fails.push(`清单抽取失败: ${declared}`);
  if (!implemented || implemented.length !== 4) fails.push(`实现抽取失败: ${implemented}`);

  // 负控 1：抽不到清单（换了形态就必须给 null，不能静默当空集）
  if (extractDeclaredKeys("fn x() {}") !== null) fails.push("无清单样本没返回 null（判据会静默放行）");
  if (extractImplementedKeys("const A=[\"z\"];") !== null) fails.push("无实现样本没返回 null");

  // 负控 2：少一个 insert ⇒ 两侧集合必须不等
  const shortBody = good.replace('    vars.insert("is_replay".to_string(), r);\n', "");
  const dec = extractDeclaredKeys(shortBody);
  const imp = extractImplementedKeys(shortBody);
  if (!imp || imp.length !== 3 || dec.length !== 4) fails.push(`少一个 insert 没被抽出（${imp}）`);

  // 负控 3：手抄清单形态 —— 注释里以反引号 + " / " 并列 ≥2 个清单键要命中
  const copycat = "// 注入 `data_freshness` / `as_of_date` / `is_replay` 等状态\n";
  if (copycatLists(copycat, dec).length !== 1) fails.push("手抄清单形态没被判出（R2 恒假）");
  // 负控 3b：散装提及（无并列形态）不算手抄 —— as_of_date / data_scope 是别处真实字段名，
  // 第一版按「出现过」判，误伤三处。
  const scattered = "// as_of_date 字段与 data_scope 枚举\nfn f() {}\n";
  if (copycatLists(scattered, dec).length !== 0) fails.push("散装提及被误判成手抄（R2 误伤）");
  // 负控 3c：并列的不是清单里的键 ⇒ 不报（防把任意反引号列表当清单）
  const otherRun = "// `vendor` / `method` / `reason` 三个字段\n";
  if (copycatLists(otherRun, dec).length !== 0) fails.push("非清单键的并列被误判成手抄");

  // 负控 4：注册判据要认**调用形态** —— 真调用命中；文档自指与注释掉的调用都不算
  if (!hasRegistrationCall("    state.work_engine.set_builtin_vars_provider(provider).await;\n")) {
    fails.push("真注册调用没被判出（R3 恒假）");
  }
  if (hasRegistrationCall("/// 应通过 WorkEngine.set_builtin_vars_provider(provider) 注入\n")) {
    fails.push("文档里的自指写法被判成调用点（R3 会被说谎喂绿）");
  }
  if (hasRegistrationCall("// engine.set_builtin_vars_provider(p); // 已移除\n")) {
    fails.push("注释掉的调用仍被判成调用点（剥注释没生效）");
  }

  if (fails.length > 0) {
    console.error("❌ 自证失败：");
    for (const f of fails) console.error("  - " + f);
    process.exit(1);
  }
  console.log(
    "✅ 自证通过（九条负控：缺清单→null / 缺实现→null / 少 insert / 手抄形态命中 / 散装提及不报 / " +
      "非清单键不报 / 真调用命中 / 文档自指不报 / 注释掉的调用不报）",
  );
  process.exit(0);
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) selftest();
  const r = check();
  console.log(
    `权威清单 ${r.declared.length} 键（实现 ${r.implemented.length} 键）；主 crate 注册调用点 ` +
      `${(r.callsites ?? []).length} 处：${(r.callsites ?? []).join(", ") || "无"}`,
  );
  for (const [k, files] of r.consumers) {
    if (files.length === 0) {
      console.log(`  ⚠ {{${k}}} 全仓零模板引用 ⇒ 已接线但未进任何提示词（加占位符属提示词裁定）`);
    } else if (args.includes("--dump")) {
      console.log(`  {{${k}}} ← ${files.join(", ")}`);
    }
  }
  if (r.violations.length > 0) {
    console.error(`❌ 违规 ${r.violations.length} 条：`);
    for (const v of r.violations) console.error("  - " + v);
    process.exit(1);
  }
  console.log("✅ 内建变量：清单=实现、无手抄副本、注册点在位");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
