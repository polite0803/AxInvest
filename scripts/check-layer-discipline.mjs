// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 模块级分层护栏（L1）—— 依赖方向 / 越层门禁
 *
 * 为什么需要它（问题形态）：
 *   我方分层纪律**只写在 `AGENTS.md`（规则文件）** —— `eslint.config.js` 无 boundaries、
 *   `scripts/` 下的契约脚本零个查依赖方向、CI 无对应门禁。
 *   规则文件拦不住任何人：**越界是静态可判的，却没有任何机器在看**。
 *   反面实证：Semantica 无任何分层护栏 + Python 无编译期约束 ⇒
 *   长成星型扇出（cli 扇出 22）+ 3 个跨包环 + 具体类直连 6 处 + `core/` 零抽象基类。
 *   ⇒ **没有模块级护栏，crate 边界内的病会扩散成整仓的病。**
 *
 * 五条规则（判据全部由代码本身推出 ⇒ 可硬拦；存量违规走棘轮）：
 *   1. commands-no-direct-db      `src/commands/**` 不得直连 `sea_orm` / `axagent_entities`
 *                                 （须经 dao / service 层）。含 `use` 与**行内全路径** ——
 *                                 后者正是「函数体内延迟 import 规避层次」的形态。
 *   2. commands-no-sibling-call   `src/commands/**` 不得跨模块调 `crate::commands::<别的模块>::`
 *                                 （命令之间横向互调 ⇒ 依赖图退化成网）
 *   3. harness-fanout-zero        `crates/harness/src/**` 不得出现任何 `axagent_*` 依赖
 *                                 （锁死扇出 = 0。**当前合规 ⇒ 直接 error 级硬拦**，防未来破坏）
 *   4. crate-has-consumer         每个 `crates/*` 必须被 workspace 内 ≥1 个 Cargo.toml 引用
 *                                 （零消费方的 crate = 死代码）
 *   5. stores-domain-no-feature   `src/stores/domain/**` 不得反向 import `feature/`
 *                                 （store 四层单向：domain 是最底层）
 *
 * 棘轮（ratchet）语义 —— **只减不增**：
 *   规则 1 / 2 / 4 / 5 有存量违规，**不可直接 error**（一刀切只会逼出刷分式修改）。
 *   基线落在 `scripts/layer-discipline-allowlist.json`，**按「规则 + 文件」记录计数**：
 *     · 某文件的违规数 **超过** 基线 ⇒ fail（已有的坑不许再挖深）
 *     · 出现基线里**没有的新文件**   ⇒ fail（新代码一律不许越界）
 *     · 某文件违规数 **少于** 基线   ⇒ 提示下调基线（棘轮向下走）
 *   为什么不按精确行号：行会随格式化漂移 ⇒ 基线天天失配、最终被人关掉。
 *   按文件计数既防增长，又对格式化免疫。
 *
 * 用法：
 *   node scripts/check-layer-discipline.mjs                    # 体检（超基线 ⇒ exit 1）
 *   node scripts/check-layer-discipline.mjs --list             # 列出违规点（file:line）
 *   node scripts/check-layer-discipline.mjs --selftest         # 检测器正负对照（不依赖仓库现状）
 *   node scripts/check-layer-discipline.mjs --update-baseline  # 重算基线（**仅在减少违规后**）
 *
 * 退出码：0 = 通过；1 = 超基线 / 硬规则违规 / 扫描面为 0
 */
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SRC_TAURI = join(ROOT, "src-tauri");
const COMMANDS_DIR = join(SRC_TAURI, "src", "commands");
const HARNESS_SRC = join(SRC_TAURI, "crates", "harness", "src");
const CRATES_DIR = join(SRC_TAURI, "crates");
const STORES_DOMAIN = join(ROOT, "src", "stores", "domain");
const BASELINE_PATH = join(ROOT, "scripts", "layer-discipline-allowlist.json");
const ROOT_CARGO = join(SRC_TAURI, "Cargo.toml");

const argv = process.argv.slice(2);
const ARG = (f) => argv.includes(`--${f}`);

// ── 文件遍历 / 读取 ───────────────────────────────────────────────────
function walk(dir, exts, out = []) {
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir)) {
    const p = join(dir, e);
    let s;
    try {
      s = statSync(p);
    } catch {
      continue; // 并发进程正在写 / 断链 ⇒ 跳过，不让它把整轮扫描带崩
    }
    if (s.isDirectory()) walk(p, exts, out);
    else if (exts.some((x) => p.endsWith(x))) out.push(p);
  }
  return out;
}
const rel = (p) => relative(ROOT, p).split(sep).join("/");
function readLines(p) {
  try {
    return readFileSync(p, "utf8").split(/\r?\n/);
  } catch {
    return null;
  }
}

// ── 行级过滤驱动（**规则与自检共用同一份实现**） ──────────────────────
//
// ⚠ 设计要点：自检**必须**调用这里的同一函数，不能另抄一份判定逻辑。
//   抄一份 = 测的是副本 ⇒ 副本对、真错了，自检照样全绿（「审计工具自造绿灯」）。

/** 返回「测试起始行号」，其后所有行按测试对待（不参与生产口径判定）。 */
export function testStarts(lines) {
  const start = [];
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    if (/^#\[cfg\(test\)\]/.test(t) || /^#\[cfg\(any\(test/.test(t) || /^mod tests\b/.test(t)) {
      start.push(i);
    }
  }
  return start;
}
/** 注释行：`//` 整行注释 / `*` 块注释续行 / `#`。 */
export function isCommentLine(line) {
  const t = line.trim();
  return t.startsWith("//") || t.startsWith("*") || t.startsWith("#");
}
/**
 * 标记「处于多行原始字符串内」的行（Rust 的 `r"…"` / `r#"…"#`）。
 * **必须排除**：prompt 模板里写着代码示例，形态与真代码一致 ⇒ 否则是假锚点。
 * ⚠ 词边界 `(?<![A-Za-z0-9_])` 必需，否则 `let s = "four";` 的 `four"` 被误判为 raw string 开头。
 */
export function rawStringLines(lines) {
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
        closer = cl;
        break;
      }
    }
  }
  return inRaw;
}

/**
 * 剥掉**双引号字符串字面量的内容**（用空格顶替，保持字符串长度以便定位）。
 *
 * 为什么必须做：`crates/harness/src/capability_registry.rs` 里有
 * `"axagent_harness::ToolRegistry"` 这类**能力 ID / 类型名字符串**，
 * 形态与真实路径一模一样 ⇒ 不剥字符串就会把 13 处「自我描述的字符串」
 * 判成「harness 依赖了 axagent_*」，**整条规则全错**。
 *
 * 只处理普通 `"…"`（含 `\"` 转义）；跨行 raw string 已由 [`rawStringLines`] 整体排除。
 */
export function stripStringLiterals(line) {
  let out = "";
  let inStr = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (!inStr) {
      if (c === '"') {
        inStr = true;
        out += " ";
      } else out += c;
    } else if (c === "\\") {
      out += "  "; // 转义对整体屏蔽，避免 `\"` 被当成字符串结束
      i++;
    } else if (c === '"') {
      inStr = false;
      out += " ";
    } else out += " ";
  }
  return out;
}

/**
 * 逐行驱动：自动排除注释行 / 测试块 / raw string，再把「活代码行」交给 `onLine`。
 * 所有行级规则都必须经它 —— 排除逻辑只有一处实现。
 *
 * @param {string[]} lines
 * @param {(line:string, idx:number)=>void} onLine
 * @returns {number} 被跳过的行数（供自陈覆盖范围 / 调试）
 */
export function driveLines(lines, onLine) {
  const ts = testStarts(lines);
  const raw = rawStringLines(lines);
  let skipped = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (ts.some((s) => i >= s) || raw[i] || isCommentLine(line)) {
      skipped++;
      continue;
    }
    onLine(line, i);
  }
  return skipped;
}

// ── 规则定义 ──────────────────────────────────────────────────────────
const R1_RE = /(?<![A-Za-z0-9_])(sea_orm|axagent_entities)\s*::/;
const R2_RE = /crate::commands::([a-z_][a-z0-9_]*)\s*::/g;
const R3_RE = /(?<![A-Za-z0-9_])axagent_[a-z0-9_]+\s*::/;
const R5_RE = /from\s+["'][^"']*(?:\.\.\/feature|@\/stores\/feature|\.\.\/\.\.\/feature)[^"']*["']/;

/**
 * 规则 2 的**共享基础设施模块**豁免 —— 调这些模块不算「命令之间横向互调」。
 *
 * 判据：它们不承载业务、被绝大多数命令当**公共库**用（错误类型 / 错误码 / 常量 /
 * 共享状态 / 协议常量 / 上下文 / 任务守卫）。
 *
 * ⚠ **这是人工决定，不是从代码推出的** —— 已写进「覆盖范围自陈」。
 * 不豁免它们会把 2635 处「错误处理样板」全报成越界 ⇒ 信号被噪声淹没 ⇒ 门禁必然被关掉。
 */
const SHARED_INFRA_MODULES = new Set([
  "error",
  "error_code",
  "error_classification",
  "constants",
  "_shared_state",
  "_workflow_ai_protocol",
  "provider_ctx",
  "spawn_guard",
]);

/**
 * 每条规则 = { id, title, level, filesOf(ctx), hitsInLines(lines, ctx) }
 * `level:"hard"` = 违规即 fail（仅用于当前已合规者，用来上锁）
 * `level:"ratchet"` = 与基线比，只减不增
 */
const RULES = [
  {
    id: "commands-no-direct-db",
    title: "`src/commands/**` 不得直连 sea_orm / axagent_entities（须经 dao / service）",
    level: "ratchet",
    filesOf: (ctx) => ctx.commandFiles,
    hitsInLines(lines) {
      const hits = [];
      driveLines(lines, (line, i) => {
        const m = R1_RE.exec(stripStringLiterals(line)); // 剥字符串：`format!("sea_orm::X")` 不算依赖
        if (m) hits.push({ line: i + 1, text: line.trim(), tag: m[1] });
      });
      return hits;
    },
  },
  {
    id: "commands-no-sibling-call",
    title: "`src/commands/**` 不得跨模块调 `crate::commands::<别的模块>::`",
    level: "ratchet",
    filesOf: (ctx) => ctx.commandFiles,
    hitsInLines(lines, ctx) {
      const hits = [];
      const own = ownCommandModule(ctx.currentFile);
      if (own === null) return hits; // 归属不可知 ⇒ 不判（宁缺勿滥，避免满屏假阳性）
      driveLines(lines, (line, i) => {
        for (const m of stripStringLiterals(line).matchAll(R2_RE)) {
          if (m[1] === own) continue; // 调自己模块的东西不算越界
          if (SHARED_INFRA_MODULES.has(m[1])) continue; // 公共基础设施不算横向互调
          hits.push({ line: i + 1, text: line.trim(), tag: `${own} → ${m[1]}` });
        }
      });
      return hits;
    },
  },
  {
    id: "harness-fanout-zero",
    title: "`crates/harness/**` 不得依赖任何 `axagent_*`（扇出锁 0）",
    level: "hard",
    filesOf: (ctx) => ctx.harnessFiles,
    hitsInLines(lines) {
      const hits = [];
      driveLines(lines, (line, i) => {
        if (R3_RE.test(stripStringLiterals(line))) hits.push({ line: i + 1, text: line.trim() });
      });
      return hits;
    },
  },
  {
    id: "crate-has-consumer",
    title: "每个 **workspace 成员** crate 必须被 ≥1 个 Cargo.toml 引用（FFI 产物除外）",
    level: "ratchet",
    filesOf: () => [], // 非行级规则：自己解析 Cargo.toml
    hitsInLines() {
      return [];
    },
    hitsInContext(ctx) {
      const hits = [];
      const allCargos = [ROOT_CARGO, ...ctx.cargos].filter((p) => existsSync(p));
      const texts = allCargos.map((p) => ({ path: p, text: readFileSync(p, "utf8") }));
      for (const { name, dir, isMember, ffiOnly } of ctx.crateNames) {
        // ① 非成员目录由 crate-dir-not-orphan 负责，避免同一件事报两遍
        if (!isMember) continue;
        // ② `crate-type = ["staticlib"]` / cdylib 是**移动端/动态库产物**：
        //    由构建工具链消费，**本来就不会有 Cargo 消费方** ⇒ 不算缺陷。
        //    （实证：`crates/axagent-mobile` 是 staticlib，供 Android/iOS 链接。）
        if (ffiOnly) continue;
        // 依赖可能写成 `axagent-foo = { path = … }` 或 `axagent-foo.workspace = true`
        const re = new RegExp(`^[ \\t]*${escapeRe(name)}[ \\t]*[.=]`, "m");
        const ok = texts.some((t) => dirname(t.path) !== dir && re.test(t.text));
        if (!ok) {
          hits.push({
            file: rel(dir),
            line: 1,
            text: `${name} 是 workspace 成员，但无任何 Cargo.toml 引用它`,
            tag: "零消费方",
          });
        }
      }
      return hits;
    },
  },
  {
    id: "crate-dir-not-orphan",
    title: "`crates/*` 目录必须在 workspace `members` 里（防孤儿 crate）",
    level: "ratchet",
    filesOf: () => [],
    hitsInLines() {
      return [];
    },
    hitsInContext(ctx) {
      return ctx.crateNames
        .filter((c) => !c.isMember)
        .map((c) => ({
          file: rel(c.dir),
          line: 1,
          text: `${c.name} 不在 workspace members 内 ⇒ 不参与构建、也无人依赖（孤儿目录）`,
          tag: "孤儿目录",
        }));
    },
  },
  {
    id: "stores-domain-no-feature",
    title: "`src/stores/domain/**` 不得反向 import `feature/`",
    level: "ratchet",
    filesOf: (ctx) => ctx.storeFiles,
    hitsInLines(lines) {
      const hits = [];
      // TS 侧不做测试块排除（`__tests__` 目录本身已天然分离），但仍排除注释行
      for (let i = 0; i < lines.length; i++) {
        if (isCommentLine(lines[i])) continue;
        if (R5_RE.test(lines[i])) hits.push({ line: i + 1, text: lines[i].trim() });
      }
      return hits;
    },
  },
];

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
/**
 * 该文件「自己的」命令模块名 —— 规则 2 用它判定是否跨模块。
 *
 * ⚠ 返回 `null` = **不在 `commands/` 下、归属不可知**。调用方必须据此跳过判定。
 * 为什么不能硬编码仓库相对前缀：那样「能否正确归属」会依赖调用方传入的路径形态
 * （`rel()` 对绝对路径/跨盘路径给出的结果不同）⇒ 归属算错时**不报错**，
 * 而是把每个 `crate::commands::X::` 都当成「跨模块」⇒ **满屏假阳性**。
 * 这与「选择函数依赖入参有序」同族：把隐含前置条件藏进实现。**宁可判不出，不可猜。**
 */
export function ownCommandModule(file) {
  const r = relative(COMMANDS_DIR, file).split(sep).join("/");
  if (r.startsWith("..")) return null; // 不在 commands 下 ⇒ 归属不可知，交由调用方跳过
  const parts = r.split("/");
  if (parts.length === 1) return parts[0].replace(/\.rs$/, "");
  return parts[0]; // 目录型模块（如 stock_workflow/core.rs）归属其目录
}

// ── 扫描 ──────────────────────────────────────────────────────────────
function buildContext() {
  const cargos = walk(CRATES_DIR, ["Cargo.toml"]);
  // workspace members ⇒ 用于「孤儿目录」判定（不采信目录存在 = 成员）
  const members = new Set();
  if (existsSync(ROOT_CARGO)) {
    const m = /^[ \t]*members[ \t]*=[ \t]*\[([\s\S]*?)\]/m.exec(readFileSync(ROOT_CARGO, "utf8"));
    if (m) {
      for (const s of m[1].matchAll(/"([^"]+)"/g)) members.add(s[1].replace(/\\/g, "/"));
    }
  }
  const crateNames = [];
  for (const c of cargos) {
    const text = readFileSync(c, "utf8");
    const m = /^[ \t]*name[ \t]*=[ \t]*"([^"]+)"/m.exec(text);
    if (!m) continue;
    const dir = dirname(c);
    // 路径成员写法有两种：`crates/foo` 与 `crates/foo/`（含尾斜杠）
    const relDir = relative(SRC_TAURI, dir).split(sep).join("/");
    // `crate-type = ["staticlib", …]` / cdylib ⇒ 由 FFI 工具链消费，无 Cargo 消费方是设计
    const ct = /^[ \t]*crate-type[ \t]*=[ \t]*\[([^\]]*)\]/m.exec(text);
    const ffiOnly = !!ct && /"(staticlib|cdylib)"/.test(ct[1]);
    crateNames.push({ name: m[1], dir, isMember: members.has(relDir), ffiOnly });
  }
  return {
    commandFiles: walk(COMMANDS_DIR, [".rs"]),
    harnessFiles: walk(HARNESS_SRC, [".rs"]),
    storeFiles: walk(STORES_DOMAIN, [".ts", ".tsx"]),
    cargos,
    crateNames,
    currentFile: "",
  };
}

function scanAll(ctx) {
  return RULES.map((rule) => {
    const hits = [];
    if (rule.hitsInContext) {
      hits.push(...rule.hitsInContext(ctx));
    }
    for (const f of rule.filesOf(ctx)) {
      const lines = readLines(f);
      if (!lines) continue;
      for (const h of rule.hitsInLines(lines, { ...ctx, currentFile: f })) {
        hits.push({ ...h, file: rel(f) });
      }
    }
    const byFile = new Map();
    for (const h of hits) byFile.set(h.file, (byFile.get(h.file) ?? 0) + 1);
    return { rule, hits, byFile };
  });
}

// ── 棘轮判定（纯函数 ⇒ 可自证，含「默认放行」格） ─────────────────────
/**
 * @param {{file:string,count:number,base:number}[]} over 超过基线的（含新文件）
 * @param {number} hardCount  level=hard 规则的违规数
 * @param {number} scannedFiles 扫描面（0 ⇒ 配置错误，不得装绿）
 */
export function gateExit(over, hardCount, scannedFiles) {
  if (scannedFiles === 0) return 1; // 扫不到文件 ⇒ 判据失效，宁可红
  if (hardCount > 0) return 1;
  if (over.length > 0) return 1;
  return 0;
}

// ── 基线读写 ──────────────────────────────────────────────────────────
function loadBaseline() {
  if (!existsSync(BASELINE_PATH)) return { rules: {} };
  try {
    return JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
  } catch (e) {
    console.error(`✖ 基线文件解析失败（${rel(BASELINE_PATH)}）：${e.message}`);
    process.exit(1);
  }
}

// ── 自检：**调用真代码路径**的正负对照 ────────────────────────────────
export function selftest() {
  let pass = 0;
  let fail = 0;
  const T = (name, cond) => {
    if (cond) pass++;
    else {
      fail++;
      console.log(`  ✖ ${name}`);
    }
  };
  const ruleById = (id) => RULES.find((r) => r.id === id);
  /** 用**规则自己的** hitsInLines 判定（不另抄一份逻辑） */
  const probe = (id, lines, ctx = {}) => ruleById(id).hitsInLines(lines, { currentFile: "/probe.rs", ...ctx });

  // ── 规则 1 ──
  T("R1+ `use sea_orm::DatabaseConnection;`", probe("commands-no-direct-db", ["use sea_orm::DatabaseConnection;"]).length === 1);
  T("R1+ 行内全路径 `sea_orm::Database::connect`（延迟 import 形态）", probe("commands-no-direct-db", ["let d = sea_orm::Database::connect(u)?;"]).length === 1);
  T("R1+ `use axagent_entities::prelude::*;`", probe("commands-no-direct-db", ["use axagent_entities::prelude::*;"]).length === 1);
  T("R1- 注释行不算", probe("commands-no-direct-db", ["// 曾经 use sea_orm::DatabaseConnection;"]).length === 0);
  T("R1- `mod tests` 之后不算", probe("commands-no-direct-db", ["use axagent_dao::X;", "mod tests {", "    use sea_orm::Y;"]).length === 0);
  T("R1- raw string（prompt 模板）内不算", probe("commands-no-direct-db", ['let p = r#"', "use sea_orm::X;", '"#;']).length === 0);
  T("R1- `sea_ormx::` 不得命中（词边界）", probe("commands-no-direct-db", ["let a = sea_ormx::y();"]).length === 0);
  T("R1- `axagent_dao::` 是合规出口，不得命中", probe("commands-no-direct-db", ["use axagent_dao::Repo;"]).length === 0);

  // ── 规则 2 ──
  // ⚠ 必须传**真实的 commands 路径**：`ownCommandModule` 的归属由「相对 commands 目录」决定，
  //   传假路径会走「归属不可知」分支 ⇒ 测出来的是另一条路径，等于没测。
  const r2 = (line, own) => probe("commands-no-sibling-call", [line], { currentFile: join(COMMANDS_DIR, `${own}.rs`) }).length;
  T("R2+ 跨模块命中", r2("let x = crate::commands::plan::load();", "cognitive") === 1);
  T("R2- 同模块不命中", r2("let x = crate::commands::cognitive::inner();", "cognitive") === 0);
  T("R2- 注释不命中", r2("// crate::commands::plan::load()", "cognitive") === 0);
  T("R2- 归属不可知时不判（**宁可判不出，不可猜**）", probe("commands-no-sibling-call", ["let x = crate::commands::plan::load();"], { currentFile: "/elsewhere/foo.rs" }).length === 0);
  T("R2- 共享基础设施豁免：`crate::commands::error::`", r2("let e = crate::commands::error::ErrorResponse::new();", "plan") === 0);
  T("R2- 共享基础设施豁免：`crate::commands::error_code::`", r2("use crate::commands::error_code::common::INTERNAL;", "plan") === 0);
  T("R2- 共享基础设施豁免：`crate::commands::constants::`", r2("let n = crate::commands::constants::MAX;", "plan") === 0);
  T("R2+ 豁免名单外的业务模块仍要命中", r2("let x = crate::commands::stock_workflow::run();", "plan") === 1);
  T("R2- 字符串里的路径不算（剥字符串）", r2('let s = "crate::commands::plan::load";', "plan") === 0);
  T("R2 归属：`stock_workflow/core.rs` → `stock_workflow`", ownCommandModule(join(COMMANDS_DIR, "stock_workflow", "core.rs")) === "stock_workflow");
  T("R2 归属：`plan.rs` → `plan`", ownCommandModule(join(COMMANDS_DIR, "plan.rs")) === "plan");
  T("R2 归属：`agent/mod.rs` → `agent`", ownCommandModule(join(COMMANDS_DIR, "agent", "mod.rs")) === "agent");
  T("R2 归属：`stock_analysis_setup/seed_x.rs` → 目录", ownCommandModule(join(COMMANDS_DIR, "stock_analysis_setup", "seed_x.rs")) === "stock_analysis_setup");
  T("R2 归属：目录外 ⇒ null（不返回垃圾值）", ownCommandModule(join(ROOT, "src-tauri", "src", "lib.rs")) === null);

  // ── 规则 3 ──
  T("R3+ `use axagent_dao::X;`", probe("harness-fanout-zero", ["use axagent_dao::X;"]).length === 1);
  T("R3+ 行内 `axagent_kit::x()`", probe("harness-fanout-zero", ["let a = axagent_kit::x();"]).length === 1);
  T("R3- 注释不算", probe("harness-fanout-zero", ["// 见 axagent_dao::X"]).length === 0);
  T("R3- 非路径形态（`axagent_foo` 无 `::`）不算", probe("harness-fanout-zero", ["let s = \"x\";"]).length === 0);
  // ⚠ 这条是本轮实测踩到的**假阳性根源**：`capability_registry.rs` 用 `"axagent_harness::X"`
  //   当能力 ID 字符串 ⇒ 不剥字符串会把 13 处自我描述判成依赖，整条规则全错。
  T("R3- **字符串字面量内的名字不算**（能力 ID `\"axagent_harness::ToolRegistry\"`）", probe("harness-fanout-zero", ['    "axagent_harness::ToolRegistry",']).length === 0);
  T("R3- `format!` 内的字符串同样不算", probe("harness-fanout-zero", ['let s = format!("axagent_dao::{}", n);']).length === 0);
  T("R3- 字符串外仍有真依赖时**照样命中**（不能因剥字符串而漏报）", probe("harness-fanout-zero", ['let s = "axagent_dao::X"; let v = axagent_dao::Real::new();']).length === 1);

  // ── stripStringLiterals 本体 ──
  const s1 = 'a"b::c"d'; // 8 字符：a " b : : c " d ⇒ 输出应为 a + 6 空格 + d（**长度不变**）
  T("剥字符串：长度不变（便于行内定位）", stripStringLiterals(s1).length === s1.length);
  T("剥字符串：串内屏蔽、串外保留", stripStringLiterals(s1) === `a${" ".repeat(6)}d`);
  T("剥字符串：转义引号不提前结束", stripStringLiterals('let s = "a\\"b::c";').includes("::") === false);
  T("剥字符串：无引号时原样返回", stripStringLiterals("use sea_orm::X;") === "use sea_orm::X;");
  T("剥字符串：未闭合引号也屏蔽到行尾", stripStringLiterals('let s = "sea_orm::X;').includes("sea_orm") === false);

  // ── 规则 4 ──
  const re4 = (name) => new RegExp(`^[ \\t]*${escapeRe(name)}[ \\t]*[.=]`, "m");
  T("R4+ `axagent-crdt = { path = … }` 视为引用", re4("axagent-crdt").test('[dependencies]\naxagent-crdt = { path = "../crdt" }') === true);
  T("R4+ `axagent-crdt.workspace = true` 视为引用", re4("axagent-crdt").test("axagent-crdt.workspace = true") === true);
  T("R4- 注释里的名字不算引用", re4("axagent-crdt").test("# 曾用 axagent-crdt") === false);
  T("R4- `axagent-crdt-extra` 不得误判（词边界）", re4("axagent-crdt").test('axagent-crdt-extra = { path = "x" }') === false);
  T("R4- 缩进无关（tab / 空格均可）", re4("axagent-crdt").test("\taxagent-crdt.path = \"x\"") === true);

  // ── 规则 5 ──
  T("R5+ `from \"../feature/agentStore\"`", probe("stores-domain-no-feature", ['import { x } from "../feature/agentStore";']).length === 1);
  T("R5+ 别名 `@/stores/feature/...`", probe("stores-domain-no-feature", ['import { x } from "@/stores/feature/agentStore";']).length === 1);
  T("R5- `from \"../shared/ui\"` 合规", probe("stores-domain-no-feature", ['import { x } from "../shared/ui";']).length === 0);
  T("R5- `../domain/other` 同层合规", probe("stores-domain-no-feature", ['import { x } from "../domain/other";']).length === 0);
  T("R5- 注释里的 import 不算", probe("stores-domain-no-feature", ['// import { x } from "../feature/a";']).length === 0);

  // ── driveLines 驱动本身 ──
  T("driveLines：正常行被交付", (() => {
    let n = 0;
    driveLines(["let a = 1;", "// c", "let b = 2;"], () => n++);
    return n === 2;
  })());
  T("driveLines：空文件交付 0 行", (() => {
    let n = 0;
    driveLines([], () => n++);
    return n === 0;
  })());

  // ── 门禁判定（**含「默认放行」那一格**） ──
  T("门禁：0 文件 ⇒ 红（配置错误不得装绿）", gateExit([], 0, 0) === 1);
  T("门禁：无违规 + 有扫描面 ⇒ **绿**（默认放行格）", gateExit([], 0, 100) === 0);
  T("门禁：硬规则违规 ⇒ 红", gateExit([], 1, 100) === 1);
  T("门禁：超基线 ⇒ 红", gateExit([{ file: "a.rs", count: 2, base: 1 }], 0, 100) === 1);
  T("门禁：硬违规 + 超基线 ⇒ 红", gateExit([{ file: "a.rs", count: 2, base: 1 }], 1, 100) === 1);

  console.log(`\n── 自检 ──────────────────────────────────`);
  console.log(`  检测器正负对照 + 门禁判定 : ${pass} passed / ${fail} failed`);
  console.log(fail === 0 ? "  ✔ 自检通过 —— 检测器与门禁判定可信" : "  ✖ 自检失败 —— 后续结论一律不可信");
  return fail === 0;
}

// ── 主流程 ────────────────────────────────────────────────────────────
function main() {
  if (ARG("selftest")) process.exit(selftest() ? 0 : 1);

  const ctx = buildContext();
  const scanned = ctx.commandFiles.length + ctx.harnessFiles.length + ctx.storeFiles.length;
  const results = scanAll(ctx);
  const baseline = loadBaseline();

  if (ARG("update-baseline")) {
    const out = {
      generatedAt: new Date().toISOString().slice(0, 10),
      note: "分层护栏棘轮基线：**只减不增**。修复违规后必须用 --update-baseline 下调，否则基线虚高会让棘轮失效。",
      rules: {},
    };
    for (const { rule, byFile } of results) {
      out.rules[rule.id] = {
        total: [...byFile.values()].reduce((a, b) => a + b, 0),
        files: Object.fromEntries([...byFile.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1))),
      };
    }
    writeFileSync(BASELINE_PATH, JSON.stringify(out, null, 2) + "\n", "utf8");
    console.log(`✔ 基线已写入 ${rel(BASELINE_PATH)}`);
    for (const { rule, byFile } of results) {
      const total = [...byFile.values()].reduce((a, b) => a + b, 0);
      console.log(`  ${rule.id.padEnd(28)} total=${String(total).padStart(4)}  files=${byFile.size}`);
    }
    process.exit(0);
  }

  console.log("═══ 模块级分层护栏（L1）═══\n");
  console.log(
    `扫描面  : commands ${ctx.commandFiles.length} / harness ${ctx.harnessFiles.length} / stores-domain ${ctx.storeFiles.length} 个源文件｜crates ${ctx.crateNames.length} 个`,
  );

  const over = [];
  const below = [];
  let hardTotal = 0;

  for (const { rule, hits, byFile } of results) {
    const base = baseline.rules?.[rule.id]?.files ?? {};
    const baseTotal = baseline.rules?.[rule.id]?.total ?? 0;
    const total = hits.length;
    if (rule.level === "hard") hardTotal += total;
    const icon = total === 0 ? "✅" : rule.level === "hard" || total > baseTotal ? "❌" : "⚠️";
    console.log(`${icon} [${rule.level === "hard" ? "硬拦" : "棘轮"}] ${rule.id}  ${total} 处（基线 ${baseTotal}）`);
    console.log(`     ${rule.title}`);

    for (const [file, count] of byFile) {
      const b = base[file] ?? 0;
      if (count > b) over.push({ rule: rule.id, file, count, base: b });
      else if (count < b) below.push({ rule: rule.id, file, count, base: b });
    }
    for (const [file, b] of Object.entries(base)) {
      if (!byFile.has(file)) below.push({ rule: rule.id, file, count: 0, base: b });
    }

    if (ARG("list") && hits.length > 0) {
      for (const h of hits.slice(0, 200)) {
        console.log(`       ${h.file}:${h.line}${h.tag ? `  [${h.tag}]` : ""}  ${h.text.slice(0, 110)}`);
      }
      if (hits.length > 200) console.log(`       … 其余 ${hits.length - 200} 处省略`);
    }
  }

  console.log("\n── 棘轮 ──────────────────────────────────");
  if (over.length === 0) {
    console.log("✅ 无超基线违规（新文件未越界、已有文件未挖深）");
  } else {
    console.log(`❌ 超基线 ${over.length} 处：`);
    for (const o of over.slice(0, 60)) {
      console.log(`   ${o.rule}  ${o.file}  ${o.count} > 基线 ${o.base}${o.base === 0 ? "  ← **新越界文件**" : ""}`);
    }
    if (over.length > 60) console.log(`   … 其余 ${over.length - 60} 处省略`);
  }
  if (below.length > 0) {
    console.log(`\n💡 ${below.length} 个文件已低于基线 ⇒ 跑 \`--update-baseline\` 下调（否则基线虚高，棘轮失效）`);
  }

  console.log("\n── 覆盖范围自陈（**哪些层没查 / 哪些是人工决定**） ──");
  console.log("1. 只查**字面量导入/路径**。经 `pub use` 转发、trait 动态分发、宏展开产生的依赖**判不出**。");
  console.log("2. 只覆盖 Rust 侧 commands / harness 与前端 stores/domain 三处；其余 crate 间的依赖方向未查。");
  console.log("3. 注释按「行首特征」排除（`//` / `*` / `#`）⇒ **行尾注释**内的路径会被误计（宁多勿漏）。");
  console.log("4. 「函数体内延迟 import / 全路径调用」已被规则 1 覆盖（不看是否出现在 use 位置）。");
  console.log("5. 规则 4 只认 `Cargo.toml` 里的依赖声明 ⇒ 未被声明却靠 `extern crate` 之类的用法判不出。");
  console.log(`6. **人工决定**：规则 2 豁免 ${SHARED_INFRA_MODULES.size} 个共享基础设施模块（${[...SHARED_INFRA_MODULES].join(" / ")}）——`);
  console.log("   判据是「它们被当公共库用」；新增此类模块须同步加进 `SHARED_INFRA_MODULES` 并写明理由。");
  console.log("7. **人工决定**：规则 4 豁免 `crate-type` 含 `staticlib`/`cdylib` 的 FFI 产物（无 Cargo 消费方是设计）。");
  console.log("8. 规则 4 只统计 **workspace 成员**；非成员目录由规则 4b（孤儿目录）单独报。");

  const exit = gateExit(over, hardTotal, scanned);
  console.log(`\n模式: ratchet ｜ 结论: ${exit === 0 ? "✅ 通过" : "❌ 未通过"}`);
  process.exit(exit);
}

// 只有被当作入口脚本直接运行时才执行主流程 —— 这样上面的工具函数/检测器可以被 import 复用
// （自检与将来的单测都靠它；无脑 `main()` 会让任何 import 都触发全仓扫描）。
if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main();
}
