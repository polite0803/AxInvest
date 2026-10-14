#!/usr/bin/env node
// 后端错误码 i18n 守卫（Phase 4.1 → 2026-10-14 加宽）
//
// 禁止 src-tauri/src/commands 下出现「不带错误码」的 map_err 兜底串。
// 后端命令错误必须携带错误码（ErrorResponse）返回，供前端按 error.{CODE} 翻译，
// 否则该错误在切语言后会退回原始英文/中文串、破坏 i18n（AGENTS.md「后端错误码 i18n 规范」）。
//
// 两族判据，处置强度**不同**：
//   ① 身份式 `map_err(|e| e.to_string())` —— **硬拦零容忍**。
//   ② 中文裸串式 `map_err(|e| format!("…: {e}"))` —— **棘轮**（只许降不许升）。
//      基线在 `raw-map-err-baseline.json`：按文件记当前处数；超过基线 / 新文件带违规 ⇒ 红；
//      低于基线 ⇒ 绿但打印「可下调」，避免基线悄悄长回去。
//      为什么不一并硬拦：2026-10-14 实测这一族有 479 处、散在 90 个文件里，每一处的修法
//      都要为那次失败**选一个业务域错误码**（域内已有码优先复用；无合适码退
//      COMMON_INTERNAL，但那样等于把「塌成一个码」的旧问题从 i18n 层搬到语义层）。
//      一次改完不成立；而放任不管就等于「门只看得见 4 处、其余 475 处自称合规」——
//      原门只认①，正是这个形态（2026-10-14 报出 4 处时其余全为静默）。
//
// 正确写法（按优先级）：
//   1) 复用已有错误码 + 技术详情（推荐，粒度最细）：
//      .map_err(|e| crate::commands::error::ErrorResponse::err_with_detail(
//          crate::commands::error_code::<业务域>::<CODE>, format!("中文上下文: {e}")))?
//   2) 兜底退化（错误码恒为 COMMON_INTERNAL，仅在你确实没有合适业务码时用）：
//      .map_err(|e| crate::commands::error::CommandError::from_error(
//          e, crate::commands::error::ErrorCategory::Unrecoverable))?
//
// 优先「复用」而不是新增码：新增错误码必须同步 11 语言 error 段，
// 否则 check-errorcode-alignment.mjs 会红。改完这两个脚本都要跑。
//
// 见 AGENTS.md「后端错误码 i18n 规范（强制）」。
//
// 用法：
//   node scripts/check-rust-raw-map-err.mjs                    # 门禁（CI 命令行未变）
//   node scripts/check-rust-raw-map-err.mjs --selftest         # 判据自身正负对照
//   node scripts/check-rust-raw-map-err.mjs --update-baseline  # 降 debt 后收紧基线

import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const target = join(root, "src-tauri", "src", "commands");
const BASELINE_PATH = join(__dirname, "raw-map-err-baseline.json");

// ① 身份式：map_err(|IDENT| IDENT.to_string())
const RE_IDENTITY = /map_err\(\|(\w+)\|\s*\1\.to_string\(\)\)/;
// ② 中文裸串式：闭体**直接**是 format!(…) ⇒ 返回裸字符串，无码。
//    `map_err(|e| ErrorResponse::…(CODE, format!(…)))` 里 format! 在实参位、不在闭体首位 ⇒ 不误报。
const RE_RAW_FORMAT = /map_err\(\|(\w+)\|\s*format!\(/;

/** 一行的分类结论（纯函数，--selftest 复用）。 */
function classifyLine(line) {
  if (RE_IDENTITY.test(line)) { return "identity"; }
  if (RE_RAW_FORMAT.test(line)) { return "raw-format"; }
  return "clean";
}

/** 棘轮比较（纯函数）：该文件应否为红 + 可下调量。 */
function ratchetFor(rel, count, baseline) {
  if (count === 0) { return { red: false, drift: 0, known: baseline[rel] !== undefined }; }
  const allowed = baseline[rel];
  if (allowed === undefined) { return { red: true, drift: 0, known: false }; } // 新文件 ⇒ 不许带违规
  if (count > allowed) { return { red: true, drift: 0, known: true }; }
  return { red: false, drift: allowed - count, known: true };
}

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) { walk(p, out); }
    else if (/\.rs$/.test(name)) { out.push(p); }
  }
}

function readBaseline() {
  if (!existsSync(BASELINE_PATH)) { return {}; }
  try {
    const j = JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
    return j.files && typeof j.files === "object" ? j.files : {};
  } catch (e) {
    console.error(`::error::基线文件读不到或不是合法 JSON（${e.message}）⇒ 现有面按「未登记的新违规」处理，不静默放行`);
    return {};
  }
}

function scan() {
  const files = [];
  walk(target, files);
  const identityHits = [];
  const formatCounts = new Map();
  for (const f of files) {
    // 用 relative() 而不是 `f.replace(root + "/")`：后者在 win32 下 join() 产出反斜杠、
    // 拼的是正斜杠 ⇒ 永不命中，本地跑出来的是整条绝对路径（CI 是 Linux 才碰巧对）。
    const rel = relative(root, f).replaceAll("\\", "/");
    const lines = readFileSync(f, "utf8").split(/\r?\n/);
    lines.forEach((line, i) => {
      const kind = classifyLine(line);
      if (kind === "identity") { identityHits.push({ rel, line: i + 1, text: line.trim() }); }
      else if (kind === "raw-format") { formatCounts.set(rel, (formatCounts.get(rel) || 0) + 1); }
    });
  }
  return { identityHits, formatCounts };
}

// ── --selftest：正负对照，证明两族判据都能红、合法形态不误报 ──
if (process.argv.includes("--selftest")) {
  let bad = 0;
  const chk = (name, got, want) => {
    const ok = JSON.stringify(got) === JSON.stringify(want);
    if (!ok) { bad++; }
    console.log(`${ok ? "✔" : "✖"} ${name}${ok ? "" : `  期望 ${JSON.stringify(want)}，实得 ${JSON.stringify(got)}`}`);
  };
  chk("身份式 ⇒ identity", classifyLine("let x = f().map_err(|e| e.to_string())?;"), "identity");
  chk("身份式换变量名也认（反向引用）", classifyLine("f().map_err(|err| err.to_string())"), "identity");
  chk("中文裸串式 ⇒ raw-format", classifyLine('.map_err(|e| format!("核查落库失败: {e}"))?;'), "raw-format");
  chk("下划线变量名的裸串式也算", classifyLine('.map_err(|_| format!("固定文案"))'), "raw-format");
  // ★ 负控（决定「削真」与否的就是这两条）：带码形态里 format! 在**实参位**，不是闭体首项
  chk(
    "带码形态不误报",
    classifyLine('.map_err(|e| ErrorResponse::err_with_detail(CODE, format!("落库失败: {e}")))?;'),
    "clean",
  );
  chk(
    "CommandError 兜底形态不误报",
    classifyLine(".map_err(|e| crate::commands::error::CommandError::from_error(e, ErrorCategory::Unrecoverable))?"),
    "clean",
  );
  chk("普通 map_err 不误报", classifyLine(".map_err(|e| io::Error::new(e.kind(), \"x\"))"), "clean");
  chk("棘轮：等于基线 ⇒ 绿", ratchetFor("a.rs", 2, { "a.rs": 2 }), { red: false, drift: 0, known: true });
  chk("棘轮：超基线 ⇒ 红", ratchetFor("a.rs", 3, { "a.rs": 2 }).red, true);
  chk("棘轮：低于基线 ⇒ 绿但报可下调", ratchetFor("a.rs", 1, { "a.rs": 2 }), { red: false, drift: 1, known: true });
  chk("棘轮：新文件带违规 ⇒ 红（不许新增债）", ratchetFor("new.rs", 1, { "a.rs": 2 }).red, true);
  chk("棘轮：零违规文件不需基线位", ratchetFor("new.rs", 0, {}), { red: false, drift: 0, known: false });
  console.log(bad === 0 ? "\n✔ selftest 全过" : `\n✖ selftest 失败 ${bad} 项`);
  process.exit(bad === 0 ? 0 : 2);
}

const { identityHits, formatCounts } = scan();
const total = (m) => [...m.values()].reduce((s, n) => s + n, 0);
const baseSum = (b) => Object.values(b).reduce((s, n) => s + n, 0);

if (process.argv.includes("--update-baseline")) {
  const files = Object.fromEntries([...formatCounts.entries()].sort());
  const body = {
    "//": "src-tauri/src/commands 下「中文裸串式 map_err」的每文件处数基线（只许降不许升）。"
      + "重生成：node scripts/check-rust-raw-map-err.mjs --update-baseline",
    generated_by: "check-rust-raw-map-err.mjs --update-baseline",
    total: total(formatCounts),
    files,
  };
  writeFileSync(BASELINE_PATH, `${JSON.stringify(body, null, 2)}\n`, "utf8");
  console.log(`基线已写入：${Object.keys(files).length} 个文件 / ${total(formatCounts)} 处`);
  process.exit(0);
}

let fail = false;
for (const h of identityHits) {
  fail = true;
  console.error(`::error file=${h.rel},line=${h.line}::裸 map_err(|x| x.to_string()) 禁止：后端错误必须带错误码返回`);
  console.error(`   ${h.rel}:${h.line}: ${h.text}`);
}

const baseline = readBaseline();
let driftSum = 0;
for (const [rel, count] of [...formatCounts.entries()].sort()) {
  const r = ratchetFor(rel, count, baseline);
  driftSum += r.drift;
  if (r.red) {
    fail = true;
    const why = r.known ? `超出基线 ${baseline[rel]}` : "该文件不在基线里（新增裸串一律拦下）";
    console.error(`::error file=${rel}::中文裸串式 map_err(|e| format!(…)) ${count} 处，${why}：后端错误必须带错误码返回`);
  }
}

if (fail) {
  console.error(`\n发现 ${identityHits.length} 处身份式；中文裸串式当前 ${total(formatCounts)} 处（基线登记面合计 ${baseSum(baseline)} 处）。`);
  console.error("改法（推荐，复用已有错误码）：");
  console.error("  .map_err(|e| crate::commands::error::ErrorResponse::err_with_detail(");
  console.error("      crate::commands::error_code::common::INVALID_INPUT, format!(\"上下文: {e}\")))?");
  console.error("兜底（无合适业务码时，错误码退化为 COMMON_INTERNAL）：");
  console.error("  .map_err(|e| crate::commands::error::CommandError::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable))?");
  console.error("注意：新增错误码须同步 11 语言 error 段，否则 check-errorcode-alignment.mjs 会失败。");
  process.exit(1);
}

console.log(
  `OK: 身份式裸 map_err 0 处；中文裸串式 ${total(formatCounts)} 处 ≤ 基线 ${baseSum(baseline)} 处`
    + `（基线覆盖 ${Object.keys(baseline).length} 个文件）。本行绿≠待修面已清，剩余面以计数为准。`,
);
if (driftSum > 0) {
  console.log(`ℹ 已降到基线以下共 ${driftSum} 处 ⇒ 跑 --update-baseline 收紧，否则后续回潮不会被发现。`);
}
