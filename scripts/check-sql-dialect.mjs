#!/usr/bin/env node

/**
 * SQL 方言守卫 —— 检出「只在一种方言上能跑」的代码。
 *
 * ## 为什么需要它
 *
 * 本仓同时跑 SQLite（开发）与 PostgreSQL（生产）。`DbBackend::Sqlite` 在代码里出现
 * **98 次**是正常的（大量是双方言分支与本地库），所以**不能**用「出现即拦」——
 * 那种门禁会因噪声被人关掉（项目既有判据：噪声淹没真问题 ⇒ 门禁必然被关）。
 *
 * 真正只在 PG 上才暴露的缺陷形态（2026-09-17 的 `42702`、2026-09-19 的
 * `get_conversation_lineage`）共同点是：
 *   ① 一段 raw SQL / 一个函数**假设了唯一后端**，
 *   ② 而它**不在任何方言分支的保护下**，
 *   ③ 于是它在另一种方言上要么语法错、要么静默走错路径。
 *
 * ## 两段判据（强度不同，处置不同）
 *
 * | 段 | 判据 | 处置 |
 * |---|---|---|
 * | **A** | **SQLite 专有 SQL 语法**（`sqlite_master` / `PRAGMA` / `AUTOINCREMENT` / `json_extract(` / `INSERT OR IGNORE` / `group_concat(` / `USING fts5` …）出现在**非方言感知**的函数/文件里 ⇒ 该 SQL 换库必崩 | **硬拦**（`--strict`），不在豁免名单 ⇒ exit 1 |
 * | **B** | raw SQL 调用点硬编码 `DbBackend::X` 字面量，而函数名/文件名均无方言标识、函数内也无方言感知 ⇒ 疑似「方言无关函数里写死了方言」 | **只报告**（待人工核对清单） |
 *
 * A 段是**可静态确证**的（专有语法非标准 SQL，换库必错）；
 * B 段是**启发式**（专用实现与共用函数在静态层面同形）⇒ 按项目惯例只报告、不拦。
 *
 * ## 已内建排除（不是豁免，是本来就不该管）
 *
 * - 注释（行 + 块）—— 否则「修复注释里写的旧写法」会让字面量恒命中（判据 #128）
 * - 测试上下文：路径含 `/tests/`、`/examples/`，文件名含 `test`，函数名含 `test`，
 *   或命中行位于 `#[cfg(test)]` 之后
 * - 文件名含方言标识（`pg_ddl.rs` / `introspect/sqlite.rs` / `fts5.rs`）—— 该文件本身
 *   就是**某一方言的实现**，写死方言是它的职责
 * - 函数名含方言标识（`sqlite_*` / `*_postgres` / `*_pragma`）—— 同上
 * - 函数内**同时**出现两种后端、或出现 `get_database_backend()` / `backend` 参数 /
 *   `match … DbBackend` ⇒ 它已是**方言分派点**，不是假设单方言
 *
 * ## 用法
 *
 * ```bash
 * node scripts/check-sql-dialect.mjs            # 报告 A/B 两段，A 段非空 ⇒ exit 1
 * node scripts/check-sql-dialect.mjs --list     # 打印全部命中（含 B 段），始终 exit 0
 * node scripts/check-sql-dialect.mjs --selftest # 正负对照（判据自身的判据）
 * ```
 *
 * ## 豁免
 *
 * `scripts/sql-dialect-allowlist.json`：`{ "files": {"<rel-path>": "理由"}, "sites": {"<rel-path>:<line>": "理由"} }`
 * **只允许为本文件鉴定的「合法单方言模块」开豁免**（如本地 SQLite 库），
 * 且必须写明理由 —— 没有理由的豁免等于关掉门禁。
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const SCAN_DIRS = [resolve(root, "src-tauri")];
const ALLOWLIST_PATH = resolve(__dirname, "sql-dialect-allowlist.json");

const ARGV = (name) => (process.argv.includes(`--${name}`) ? true : undefined);
const LIST = ARGV("list");
const SELFTEST = ARGV("selftest");

// ── 规则定义 ────────────────────────────────────────────────

/** A 段：SQLite 专有 SQL 语法（非标准 SQL，在 PG 上必然语法错或语义不同）。
 *
 * `lineGuard`：额外要求命中行**本身像 SQL**。用于 `sqlite_master` 这类
 * 既能出现在 SQL 里、也能出现在日志文案里的词 —— 不加会把
 * `"integrity_check passed but sqlite_master query failed: {e}"` 这种
 * **错误消息**当成 SQL（2026-09-19 实测的假阳性）。
 */
const SQLITE_ONLY_SQL = [
  { id: "sqlite-master", re: /\bsqlite_master\b/gi, lineGuard: /\b(SELECT|FROM|INSERT|UPDATE|DELETE|CREATE|DROP|ALTER|JOIN)\b/i },
  { id: "pragma", re: /\bPRAGMA\s+[a-z_]+/gi },
  { id: "autoincrement", re: /\bAUTOINCREMENT\b/gi },
  { id: "json_extract", re: /\bjson_extract\s*\(/gi },
  { id: "insert-or-ignore", re: /\bINSERT\s+OR\s+IGNORE\b/gi },
  { id: "insert-or-replace", re: /\bINSERT\s+OR\s+REPLACE\b/gi },
  { id: "group_concat", re: /\bgroup_concat\s*\(/gi },
  { id: "using-fts5", re: /\bUSING\s+fts5\b/gi },
  { id: "strftime", re: /\bstrftime\s*\(/gi },
  { id: "ifnull", re: /\bifnull\s*\(/gi },
  { id: "datetime-now", re: /\bdatetime\s*\(\s*'now'/gi },
  { id: "glob-op", re: /\bGLOB\b/g },
];

/** 文件级/函数级方言标识 ⇒ 该处本就是某方言的专用实现。 */
const DIALECT_MARKER = /sqlite|postgres|\bpg\b|pg_|_pg|fts5|mysql|pragma/i;

/** 测试辅助函数的命名（`create_test_pool` / `test_db`）⇒ 测试上下文，不参与判定。 */
const TEST_FN = /(?:^|_)(?:test|tests|testing)(?:_|$)/;

const RAW_SQL_BACKEND = /\bDbBackend::(?:Sqlite|Postgres)\b/;

// ── 工具 ────────────────────────────────────────────────────

/**
 * 剥掉注释，**保留字符串字面量的内容**（A 段要匹配字符串里的 SQL 语法）。
 * 状态机处理行注释 / 块注释 / 普通字符串 / raw 字符串（`r#"…"#`）。
 */
function stripComments(src) {
  let out = "";
  let i = 0;
  let inLine = false;
  let inBlock = false;
  let inStr = false;
  let inRaw = false;
  let hashes = 0;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (inLine) {
      if (c === "\n") {
        inLine = false;
        out += c;
      } else out += " ";
      i++;
      continue;
    }
    if (inBlock) {
      if (c === "*" && n === "/") {
        inBlock = false;
        i += 2;
      } else {
        if (c === "\n") out += c;
        i++;
      }
      continue;
    }
    if (inRaw) {
      if (c === '"' && src.slice(i + 1, i + 1 + hashes) === "#".repeat(hashes)) {
        inRaw = false;
        i += 1 + hashes;
      } else {
        out += c;
        i++;
      }
      continue;
    }
    if (inStr) {
      if (c === "\\") {
        out += c + (n ?? "");
        i += 2;
        continue;
      }
      if (c === '"') inStr = false;
      out += c;
      i++;
      continue;
    }
    if (c === "/" && n === "/") {
      inLine = true;
      i += 2;
      continue;
    }
    if (c === "/" && n === "*") {
      inBlock = true;
      i += 2;
      continue;
    }
    if (c === "r") {
      const m = /^r(#*)"/.exec(src.slice(i));
      if (m) {
        hashes = m[1].length;
        inRaw = true;
        i += 1 + hashes + 1;
        continue;
      }
    }
    if (c === '"') {
      inStr = true;
      out += c;
      i++;
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

/**
 * 逐行建立「该行属于哪个函数」与「到该行为止是否已进入 `#[cfg(test)]`」。
 *
 * 用「最近的一个 `fn` 定义行」作为归属，不做花括号平衡 —— 平衡法会被字符串里的
 * `{where_clause}` 这类花括号带偏（2026-09-19 实测：正因此漏掉了真命中）。
 */
function annotateLines(lines) {
  const meta = [];
  let curFn = "";
  let curFnLine = 0;
  let inTestMod = false;
  let testModDepth = -1;
  let braceDepth = 0;
  for (let i = 0; i < lines.length; i++) {
    const L = lines[i];
    if (/#\[cfg\(test\)\]/.test(L)) {
      inTestMod = true;
      testModDepth = -1;
    }
    const fnM = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z0-9_]+)/.exec(
      L,
    );
    if (fnM) {
      curFn = fnM[1];
      curFnLine = i + 1;
      if (testModDepth === -1 && inTestMod) testModDepth = braceDepth;
    }
    const opens = (L.match(/\{/g) || []).length;
    const closes = (L.match(/\}/g) || []).length;
    meta.push({
      fn: curFn,
      fnLine: curFnLine,
      inTest: inTestMod,
      inTestMod: testModDepth >= 0 && braceDepth >= testModDepth,
    });
    braceDepth += opens - closes;
  }
  return meta;
}

function walk(dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name === "target" || e.name === "node_modules" || e.name === ".git") continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

function isTestPath(rel) {
  return /\/tests\/|\/examples\//.test(rel) || /(^|\/)test[_-]/.test(rel) || /_test\.rs$/.test(rel);
}

function loadAllowlist() {
  if (!existsSync(ALLOWLIST_PATH)) return { files: {}, sites: {} };
  try {
    const j = JSON.parse(readFileSync(ALLOWLIST_PATH, "utf8"));
    return { files: j.files || {}, sites: j.sites || {} };
  } catch (e) {
    console.error(`✗ 豁免名单不可读（JSON 解析失败）：${ALLOWLIST_PATH}\n  ${e.message}`);
    process.exit(1);
  }
}

// ── 扫描 ────────────────────────────────────────────────────

/** 对单个文件做判定（纯函数，便于自检用畸形输入喂它）。 */
export function scanText(rel, rawText, allow) {
  // 测试 / 示例代码本来就可以写死方言 ⇒ 在**判定函数内部**排除，
  // 而不是只在 main 的扫描面里排除：否则「单文件判定」会误报，
  // 且扫描面一旦漏配就静默放过（自检用例锁住这条）。
  if (isTestPath(rel)) return { hits: [], raw: [] };
  const src = stripComments(rawText);
  const lines = src.split("\n");
  const meta = annotateLines(lines);
  const fileDialect = DIALECT_MARKER.test(rel);
  const hits = [];

  for (let i = 0; i < lines.length; i++) {
    const L = lines[i];
    const m = meta[i];

    // 内建排除
    if (m.inTest || m.inTestMod) continue;
    if (/^\s*\/\//.test(rawText.split("\n")[i] ?? "")) continue;

    // ── A 段 ──
    for (const rule of SQLITE_ONLY_SQL) {
      rule.re.lastIndex = 0;
      const mm = rule.re.exec(L);
      if (!mm) continue;
      if (rule.lineGuard && !rule.lineGuard.test(L)) continue;
      if (fileDialect || DIALECT_MARKER.test(m.fn)) continue;
      if (TEST_FN.test(m.fn)) continue;
      if (bodyIsDialectAware(meta, lines, m.fnLine)) continue;
      hits.push({
        seg: "A",
        file: rel,
        line: i + 1,
        rule: rule.id,
        fn: m.fn,
        text: L.trim().slice(0, 90),
        why: "SQLite 专有 SQL 语法出现在非方言感知的函数里 ⇒ 换 PG 必错",
      });
    }

    // ── B 段 ──
    if (RAW_SQL_BACKEND.test(L)) {
      const fnDialect = DIALECT_MARKER.test(m.fn);
      if (
        !fileDialect &&
        !fnDialect &&
        !TEST_FN.test(m.fn) &&
        !bodyIsDialectAware(meta, lines, m.fnLine)
      ) {
        hits.push({
          seg: "B",
          file: rel,
          line: i + 1,
          rule: "hardcoded-backend-literal",
          fn: m.fn,
          text: L.trim().slice(0, 90),
          why: "方言无关的函数里硬编码了后端 ⇒ 疑似假设单方言（启发式，待人工核对）",
        });
      }
    }
  }

  // 文件级豁免对 A/B 两段都生效 —— 同一模块「合法单方言」的性质对两段都成立。
  const kept = hits.filter((h) => !allow.files[rel] && !allow.sites[`${rel}:${h.line}`]);
  return { hits: kept, raw: hits };
}

/**
 * 函数体是否已是方言分派点。
 *
 * ⚠ 必须看**整个函数体**，不能只看「到当前行为止的前缀」—— 分派代码（`DbBackend::Postgres`
 * 分支）常写在 SQL 使用点**之后**，只看前缀会把它误判成「假设单方言」
 * （2026-09-19 自检实测：正是这一条被自检抓出）。
 */
function bodyIsDialectAware(meta, lines, fnLine) {
  let end = meta.length;
  for (let i = fnLine; i < meta.length; i++) {
    if (meta[i].fnLine !== fnLine) {
      end = i;
      break;
    }
  }
  const body = lines.slice(fnLine - 1, end).join("\n");
  // 两种后端同现 ⇒ 已是分派点
  if (/\bDbBackend::Postgres\b/.test(body) && /\bDbBackend::Sqlite\b/.test(body)) return true;
  // 显式方言探测。`.be()` 是本项目的后端访问器惯例（`VectorStore::be`），
  // 出现它即表示「该处的后端是运行期决定的」—— 不认它会把 `vector_store.rs`
  // 的 sqlite_master 查询误判成单方言（2026-09-19 实测的假阳性）。
  if (/get_database_backend|\bDialect::|is_postgres|is_sqlite|\.be\(\)/.test(body)) return true;
  // 后端作为参数传入（`fn placeholders(backend: DbBackend, …)` 形态）
  if (/fn\s+\w+\s*\([^)]*\bbackend\w*\s*:/.test(body)) return true;
  return false;
}

// ── 自检（判据自身的判据） ──────────────────────────────────

function runSelftest() {
  const t = (name, fn) => {
    try {
      fn();
      console.log(`  ✓ ${name}`);
      return 0;
    } catch (e) {
      console.log(`  ✗ ${name}\n      ${e.message}`);
      return 1;
    }
  };
  let bad = 0;
  const empty = { files: {}, sites: {} };

  bad += t("正向：方言无关函数里写 sqlite_master ⇒ A 段命中", () => {
    const src = `async fn lineage() {\n    let sql = "SELECT 1 FROM sqlite_master";\n    db.query_all_raw(sql).await;\n}\n`;
    const r = scanText("src/a.rs", src, empty);
    const a = r.hits.filter((h) => h.seg === "A");
    if (a.length !== 1) throw new Error(`期望 A 段 1 条，实际 ${a.length}`);
  });

  bad += t("负向：同样的 SQL 在 `fn sqlite_helper()` 里 ⇒ 不报（专用实现）", () => {
    const src = `async fn sqlite_helper() {\n    let sql = "SELECT 1 FROM sqlite_master";\n}\n`;
    const r = scanText("src/a.rs", src, empty);
    if (r.hits.length !== 0) throw new Error(`期望 0 条，实际 ${r.hits.length}`);
  });

  bad += t("负向：方言分派函数（两种后端都出现）⇒ 不报", () => {
    const src = `fn ddl(backend: DbBackend) {\n  let a = "PRAGMA x";\n  let b = DbBackend::Postgres;\n  let c = DbBackend::Sqlite;\n  let d = "PRAGMA y";\n}\n`;
    const r = scanText("src/a.rs", src, empty);
    if (r.hits.length !== 0) throw new Error(`期望 0 条，实际 ${r.hits.length}`);
  });

  bad += t("负向：注释里的 sqlite_master ⇒ 不报（剥注释）", () => {
    const src = `// 旧写法：SELECT 1 FROM sqlite_master\nfn plain() {}\n`;
    const r = scanText("src/a.rs", src, empty);
    if (r.hits.length !== 0) throw new Error(`期望 0 条，实际 ${r.hits.length}`);
  });

  bad += t("负向：tests/ 目录下的方言写死 ⇒ 不报", () => {
    const src = `fn helper() {\n    let sql = "PRAGMA x";\n}\n`;
    const r = scanText("src-tauri/crates/dao/tests/x.rs", src, empty);
    if (r.hits.length !== 0) throw new Error(`期望 0 条，实际 ${r.hits.length}`);
  });

  bad += t("★ 负向：函数体里有字符串 `{where_clause}` 时，归属不得跨函数（花括号干扰回归）", () => {
    const src = `fn other() { let s = "WHERE {where_clause}"; }
fn plain() {
    let sql = "PRAGMA x";
}
`;
    const r = scanText("src/a.rs", src, empty);
    const a = r.hits.filter((h) => h.seg === "A");
    if (a.length !== 1) throw new Error(`期望 A 段 1 条，实际 ${a.length}`);
    if (a[0].fn !== "plain") throw new Error(`归属函数应为 plain，实际 ${a[0].fn}`);
  });

  bad += t("正向：豁免名单按文件生效", () => {
    const src = `fn open() {\n    let sql = "PRAGMA journal_mode";\n}\n`;
    const r1 = scanText("src/disk.rs", src, empty);
    const r2 = scanText("src/disk.rs", src, { files: { "src/disk.rs": "本地 SQLite 库" }, sites: {} });
    if (r1.hits.length === 0) throw new Error("未豁免时应报");
    if (r2.hits.length !== 0) throw new Error("已豁免时不应报");
  });

  bad += t("正向：B 段能认出「硬编码后端」", () => {
    const src = `async fn lineage() {\n    let sql = "SELECT 1 WHERE id = ?";\n    db.query_one_raw(Statement::from_sql_and_values(DbBackend::Sqlite, sql, v)).await;\n}\n`;
    const r = scanText("src/a.rs", src, empty);
    const b = r.hits.filter((h) => h.seg === "B");
    if (b.length === 0) throw new Error("B 段应命中");
  });

  bad += t("★ 反向护栏：扫描面不得为 0 文件（0 ⇒ 配置错误，非「干净」）", () => {
    const n = countScannedFiles();
    if (n < 200) throw new Error(`只扫到 ${n} 个 .rs 文件（应为数千）⇒ 路径配置失效`);
  });

  console.log(bad === 0 ? "\n✔ 自检通过" : `\n✗ 自检失败：${bad} 项`);
  return bad;
}

function countScannedFiles() {
  let n = 0;
  for (const d of SCAN_DIRS) if (existsSync(d)) n += walk(d).length;
  return n;
}

// ── 主流程 ──────────────────────────────────────────────────

function main() {
  if (SELFTEST) {
    console.log("── 自检（正负对照）──────────────────────────");
    process.exit(runSelftest() === 0 ? 0 : 1);
  }

  const allow = loadAllowlist();
  const files = [];
  for (const d of SCAN_DIRS) if (existsSync(d)) files.push(...walk(d));
  if (files.length === 0) {
    console.error(`✗ 扫描面为 0 个 .rs 文件 ⇒ 路径配置失效（不许当作「干净」）`);
    process.exit(1);
  }

  const all = [];
  let scanned = 0;
  for (const f of files) {
    const rel = relative(root, f).split(sep).join("/");
    if (isTestPath(rel)) continue;
    scanned++;
    const r = scanText(rel, readFileSync(f, "utf8"), allow);
    all.push(...r.hits);
  }

  const segA = all.filter((h) => h.seg === "A");
  const segB = all.filter((h) => h.seg === "B");

  if (LIST) {
    console.log(`── 全量命中（扫描 ${scanned} 个 .rs）──\n`);
    for (const seg of ["A", "B"]) {
      const rows = all.filter((h) => h.seg === seg);
      console.log(`【${seg} 段】${rows.length} 条`);
      for (const h of rows) console.log(`  ${h.file}:${h.line}  [${h.rule}]  fn ${h.fn}()\n      ${h.text}`);
      console.log("");
    }
    process.exit(0);
  }

  console.log(`── SQL 方言守卫 ──`);
  console.log(`  扫描         : ${scanned} 个 .rs（已排除 tests/examples）`);
  console.log(`  A 段（硬拦） : ${segA.length} 条  ← 专有语法 + 非方言感知`);
  console.log(`  B 段（报告） : ${segB.length} 条  ← 硬编码后端字面量（启发式，待人工核对）`);

  if (segB.length) {
    console.log(`\n  B 段清单（不影响退出码）：`);
    for (const h of segB.slice(0, 20)) console.log(`    · ${h.file}:${h.line}  fn ${h.fn}()  ${h.text}`);
    if (segB.length > 20) console.log(`    … 另 ${segB.length - 20} 条（--list 看全部）`);
  }

  if (segA.length) {
    console.log(`\n  ✗ A 段命中（换 PG 必错）：`);
    for (const h of segA) {
      console.log(`\n    ${h.file}:${h.line}  [${h.rule}]  fn ${h.fn}()`);
      console.log(`      ${h.text}`);
    }
    console.log(
      `\n  处置：① 改成方言分派（\`db.get_database_backend()\` / 双分支）；` +
        `② 若该模块本就只支持一种方言，进 \`scripts/sql-dialect-allowlist.json\` 并写理由。`,
    );
    process.exit(1);
  }

  console.log("\n  ✔ A 段 0 条");
  process.exit(0);
}

main();
