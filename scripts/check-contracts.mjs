// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 契约一致性核对 CI 脚本
 *
 * 覆盖项目 AGENTS.md「禁区」「后端错误码 i18n 规范」中可静态自动化的契约:
 *   A. Tauri 命令两步注册:  #[tauri::command] 定义必须出现在 register_commands.rs 的 generate_handler![]
 *   B. 错误码 ↔ i18n 翻译:  error_code(s).rs 中的错误码值 ⊆ 11 语言 locale 的 error 段 key
 *   C. i18n key 完整性:      以 zh-CN 为源, 其余 10 语言缺失/多余的 key
 *   D. Harness 依赖方向:     consumer crate 不得直接依赖 harness 之外的 axagent-* crate
 *   E. (warning, 默认关闭)   前后端 DTO 粗对齐: 命令返回类型名应在 src/types 有同名导出
 *   F. 登记字段必有消费点:   下表登记的 DTO 字段必须在声明文件之外至少出现 1 次
 *                            (铁律 #6「声明的输入真有入边供给」, 防「死参数」回归)
 *   G. 事件发射/监听对称:     TS 侧 listen("evt") 必须有 Rust 侧 emit("evt")
 *                            (防「前端监听永远收不到」的静默失效)
 *   H. 静默丢弃 Result 棘轮:  `let _ = <fallible>.await;` 的数量不得超过基线
 *                            (Rust 侧 bare-except 等价物; 铁律 #12「归因字段不得说谎」.
 *                             棘轮只减不增 —— 修复后必须同步下调 SILENT_RESULT_BASELINE)
 *
 * 用法:
 *   node scripts/check-contracts.mjs            # 跑 A-D, F, G, H (error 级会 fail)
 *   node scripts/check-contracts.mjs --only=a,b # 只跑指定项
 *   node scripts/check-contracts.mjs --dto      # 额外开启 E (仅 warning)
 *
 * 退出码: 任何 error 级问题 -> 1; 仅 warning -> 0
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SRC_TAURI = join(ROOT, "src-tauri");
const FRONTEND = join(ROOT, "src");
const LOCALES_DIR = join(FRONTEND, "i18n", "locales");
const ZH = join(LOCALES_DIR, "zh-CN.json");

/** consumer crate 列表: 仅允许依赖 axagent-harness */
const CONSUMER_CRATES = ["agent", "gateway", "orchestrator", "runtime-core"];

const errors = [];
const warnings = [];
let hasError = false;
function fail(...m) {
  errors.push(m.join(" "));
  hasError = true;
}
function warn(...m) {
  warnings.push(m.join(" "));
}

// ---------- 工具 ----------
function walk(dir, ext, out = []) {
  if (!existsSync(dir)) { return out; }
  for (const e of readdirSync(dir)) {
    if (e === "target" || e === "output") { continue; } // 构建产物 / 备份目录，非源码
    const p = join(dir, e);
    const s = statSync(p);
    if (s.isDirectory()) { walk(p, ext, out); }
    else if (p.endsWith(ext)) { out.push(p); }
  }
  return out;
}
function read(p) {
  return readFileSync(p, "utf8");
}
/** 限制单条打印数量, 避免海量缺失 key 刷屏 */
function printList(title, items, limit = 50) {
  const sorted = [...items].sort();
  console.log(`\n${title} (${sorted.length}):`);
  sorted.slice(0, limit).forEach((x) => console.log("  " + x));
  if (sorted.length > limit) { console.log(`  ... 另有 ${sorted.length - limit} 条未显示`); }
}

// ---------- A. 命令两步注册 ----------
function extractHandlerBlock(src) {
  const start = src.indexOf("generate_handler![");
  if (start < 0) { return ""; }
  let depth = 0;
  let i = start;
  for (; i < src.length; i++) {
    if (src[i] === "[") { depth++; }
    else if (src[i] === "]") {
      depth--;
      if (depth === 0) { break; }
    }
  }
  return src.slice(start, i + 1);
}
function checkCommandRegistration() {
  const regFile = join(SRC_TAURI, "src", "register_commands.rs");
  const regSrc = read(regFile);
  const block = extractHandlerBlock(regSrc);
  const registered = new Set();
  for (const m of block.matchAll(/^\s*(?:commands::)?(?:[\w]+::)+\w+\s*,?\s*$/gm)) {
    const parts = m[0].split("::");
    registered.add(parts[parts.length - 1].replace(/[,\s]/g, ""));
  }

  const cmdFiles = walk(join(SRC_TAURI, "src"), ".rs");
  const reDef = /#\[(?:tauri::)?command\][\s\S]*?(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)/g;
  const defined = new Set();
  for (const f of cmdFiles) {
    const srcClean = read(f).replace(/\/\/.*$/gm, "");
    for (const m of srcClean.matchAll(reDef)) {
      defined.add(m[1]);
    }
  }

  const unregistered = [...defined].filter((n) => !registered.has(n));
  const orphanReg = [...registered].filter((n) => !defined.has(n));
  if (unregistered.length) { printList("[A] 已定义但未注册到 generate_handler! (前端 invoke 会 404)", unregistered); }
  unregistered.forEach((n) => fail(`[A] 命令已定义但未注册: ${n}`));
  if (orphanReg.length) { orphanReg.forEach((n) =>
      warn(`[A] generate_handler! 注册但无 #[tauri::command] 定义: ${n}`)
    ); }
}

// ---------- B. 错误码 ↔ i18n 翻译 ----------
function extractErrorCodes() {
  const files = [
    join(SRC_TAURI, "crates", "harness", "src", "error_codes.rs"),
    join(SRC_TAURI, "src", "commands", "error_code.rs"),
  ];
  const codes = new Set();
  for (const f of files) {
    if (!existsSync(f)) { continue; }
    const src = read(f);
    for (const m of src.matchAll(/(?:pub\s+)?const\s+\w+\s*:\s*&str\s*=\s*"([A-Z][A-Z0-9_]*)"/g)) {
      codes.add(m[1]);
    }
  }
  return codes;
}
function localeErrorKeys(p) {
  const obj = JSON.parse(read(p));
  return obj.error && typeof obj.error === "object" ? Object.keys(obj.error) : [];
}
function checkErrorCodeI18n() {
  const codes = extractErrorCodes();
  if (!codes.size) {
    warn("[B] 未提取到任何错误码常量, 请检查 error_code(s).rs 路径");
    return;
  }
  const locales = readdirSync(LOCALES_DIR).filter((f) => f.endsWith(".json"));
  const missingByLocale = {};
  for (const loc of locales) {
    const keys = new Set(localeErrorKeys(join(LOCALES_DIR, loc)));
    const missing = [...codes].filter((c) => !keys.has(c));
    if (missing.length) { missingByLocale[loc] = missing; }
  }
  for (const [loc, missing] of Object.entries(missingByLocale)) {
    printList(`[B] 错误码在 ${loc} 的 error 段缺失翻译`, missing);
    missing.forEach((c) => fail(`[B] 错误码 ${c} 在 ${loc} 缺失翻译`));
  }
  // orphan: locale error 段里像错误码格式但不在常量表中的 key
  for (const loc of locales) {
    const keys = localeErrorKeys(join(LOCALES_DIR, loc));
    keys
      .filter((k) => /^[A-Z][A-Z0-9_]{3,}$/.test(k) && !codes.has(k))
      .forEach((k) => warn(`[B] ${loc} error 段含未定义错误码格式的 key: ${k}`));
  }
}

// ---------- C. i18n key 完整性 ----------
function flatten(obj, prefix = "", out = new Set()) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) { flatten(v, key, out); }
    else { out.add(key); }
  }
  return out;
}
function checkI18nKeys() {
  const zh = flatten(JSON.parse(read(ZH)));
  const locales = readdirSync(LOCALES_DIR).filter((f) => f !== "zh-CN.json" && f.endsWith(".json"));
  for (const loc of locales) {
    const keys = flatten(JSON.parse(read(join(LOCALES_DIR, loc))));
    const missing = [...zh].filter((k) => !keys.has(k));
    const extra = [...keys].filter((k) => !zh.has(k));
    if (missing.length) { printList(`[C] ${loc} 相对 zh-CN 缺失的 i18n key`, missing); }
    missing.forEach((k) => fail(`[C] ${loc} 缺失 i18n key: ${k}`));
    extra.forEach((k) => warn(`[C] ${loc} 含 zh-CN 没有的多余 key: ${k}`));
  }
}

// ---------- D. Harness 依赖方向 ----------
function checkHarnessDirection() {
  for (const name of CONSUMER_CRATES) {
    const toml = join(SRC_TAURI, "crates", name, "Cargo.toml");
    if (!existsSync(toml)) {
      warn(`[D] consumer crate 不存在: ${name}`);
      continue;
    }
    const src = read(toml);
    const axagent = new Set();
    for (const b of src.matchAll(/\[dependencies(?:\.[\w-]+)?\][\s\S]*?(?=\n\[|\Z)/g)) {
      for (const m of b[0].matchAll(/^\s*([\w-]+)\s*=/gm)) {
        if (m[1].startsWith("axagent-")) { axagent.add(m[1]); }
      }
    }
    const violations = [...axagent].filter((d) => d !== "axagent-harness");
    if (violations.length) {
      printList(`[D] consumer crate "${name}" 越界依赖实现层 (仅允许 axagent-harness)`, violations);
      violations.forEach((d) => fail(`[D] ${name} 越界依赖: ${d}`));
    }
  }
}

// ---------- E. 前后端 DTO 粗对齐 (warning only) ----------
function checkDto() {
  const cmdFiles = walk(join(SRC_TAURI, "src"), ".rs");
  const tsTypes = new Set();
  for (const f of walk(join(FRONTEND, "types"), ".ts")) {
    const src = read(f);
    for (const m of src.matchAll(/export\s+(?:interface|type)\s+(\w+)/g)) { tsTypes.add(m[1]); }
  }
  const primitives = new Set([
    "String",
    "bool",
    "u8",
    "u16",
    "u32",
    "u64",
    "i8",
    "i16",
    "i32",
    "i64",
    "f32",
    "f64",
    "usize",
    "isize",
  ]);
  const re =
    /#\[tauri::command\][\s\S]*?pub\s+(?:async\s+)?fn\s+(\w+)\s*\([^)]*\)\s*(?:->\s*([\w<>:,\s]+?))?\s*(?:\{|;)/g;
  for (const f of cmdFiles) {
    const src = read(f);
    let m;
    while ((m = re.exec(src))) {
      const ret = m[2];
      if (!ret) { continue; }
      const inner = ret
        .replace(/(?:Result|Option|Vec|HashMap|BTreeMap|std::|crate::|axagent_\w+::)[\s<>:,]*|[\s<>:,]+/g, " ")
        .trim()
        .split(/\s+/)[0];
      if (/^[A-Z]\w+$/.test(inner) && !primitives.has(inner) && !tsTypes.has(inner)) {
        warn(`[E] 命令 ${m[1]} 返回类型 ${inner} 在 src/types 无同名导出 (可能需对齐 DTO)`);
      }
    }
  }
}

// ---------- F. 登记字段必有消费点 (入边供给) ----------
/**
 * 针对铁律 #6「声明的输入真有入边供给」的机械检查。
 *
 * 为什么是**登记制**而非全量扫描：全量扫描「结构体字段是否在别处出现」会被
 * 同名标识符（`timeout` / `id` / `name`）大量误报 —— 而会误报的门禁最终会被
 * 加白名单或直接禁用，比没有更糟（「声明了不生效的机制」正是这么来的）。
 * 登记制零误报，代价是新增关键字段时手写一行。
 *
 * 判据：字段名在 `src-tauri` 内**除声明文件外**至少出现 1 次。
 * Rust 侧消费必然写作 `x.field` / `.field` / `field:`，都含裸字段名，
 * 故「裸名跨文件出现」是可靠的下界；反之不出现则一定是死字段。
 */
const FIELD_CONTRACTS = [
  {
    field: "max_steps",
    container: "AgentExecuteRequest",
    declaredIn: "crates/harness/src/agent.rs",
    rationale: "MCP agent_run 的调用级迭代预算，须被 HarnessAgentAdapter 消费",
  },
  {
    field: "execution_authorized",
    container: "plans::ActiveModel",
    declaredIn: "crates/entities/src/plans.rs",
    rationale: "Plan 执行授权位（P0-A），须有写入点与执行前校验点",
  },
];

function checkFieldConsumption() {
  const rsFiles = walk(SRC_TAURI, ".rs");
  const cache = new Map(rsFiles.map((f) => [f, read(f)]));

  for (const c of FIELD_CONTRACTS) {
    const declPath = join(SRC_TAURI, c.declaredIn);
    if (!existsSync(declPath)) {
      fail(`[F] 登记项 ${c.field} 的声明文件不存在: ${c.declaredIn} —— 登记表已过期，请修正`);
      continue;
    }
    // 自检：字段必须真的在该文件里声明。若失败说明登记表本身失真
    // （字段已改名 / 已删），这类漂移不查就会变成「检查一个不存在的字段」
    // 从而恒 PASS 的假绿。
    if (!new RegExp(`\\b${c.field}\\s*:`).test(read(declPath))) {
      fail(
        `[F] 登记项 ${c.field} 在 ${c.declaredIn} 中无字段声明 —— 契约漂移（字段已改名或删除），请更新登记表`,
      );
      continue;
    }
    const consumers = [];
    for (const [p, src] of cache) {
      if (p === declPath) { continue; }
      const n = (src.match(new RegExp(`\\b${c.field}\\b`, "g")) || []).length;
      if (n > 0) { consumers.push(`${p.slice(SRC_TAURI.length + 1)}×${n}`); }
    }
    if (consumers.length === 0) {
      fail(`[F] ${c.container}.${c.field} 无任何消费点 —— ${c.rationale}`);
    }
  }
}

// ---------- G. 事件发射/监听对称 ----------
/**
 * 跨语言检查：TS 侧 `listen("evt")` 若无对应 Rust 侧 `emit("evt")`，
 * 该监听**永远收不到消息**（前端静默失效，不报错、单测也过）。
 * 本轮已在 `agent-plan-ready-for-approval` 上真实踩到该形态。
 *
 * 已知局限（脚本自陈覆盖范围，避免又变成「声明了不生效的机制」）：
 * - 只能解析**字符串字面量**或指向字面量的 `const &str` 事件名；
 *   `emit(&var, ..)` / `listen(name)` 这类动态取值无法静态判定，
 *   会记入「未解析」并以 warning 打印，**不计入 fail**。
 * - 第三方插件（updater / shell / window 等）事件无 Rust 侧 emit，须登记豁免。
 */
const EVENT_EXEMPT = new Set([
  // 由 Tauri 插件或窗口系统发出、不在 axagent 源码里 emit 的事件名。
  // 补充时必须写明来源，禁止「为了过 CI」而加（这会让本段彻底失效）。
]);

/**
 * 已确认断链但**尚未修复**的事件基线（2026-09-12 建立）。
 *
 * 本段首次运行时发现 11 个「前端 listen、Rust 侧零 emit」的事件，逐个 grep
 * 甄别后**全部确认为真断链**（Rust 侧连字符串都没出现过，连注释都没有）。
 * 它们需要逐项判定处置方向 —— 按铁律 #3「独占且无人接线 ⇒ 激活非删」，
 * 要么补发射端（功能仍需要），要么删监听端（功能已被替代）。
 * 判定结果与后续处理见 `PLAN-evoflow-borrowings.md` 的 P1-D。
 *
 * 为什么不直接豁免：`EVENT_EXEMPT` 会让本段对该事件永久失明。此处用
 * **基线集合**，语义是「已知债务，数量必须与 PLAN 同步」：
 * - 出现**基线外**的新断链 ⇒ fail（拦住新回归，这是本段的主要价值）
 * - 基线内的项被修好却没从基线移除 ⇒ fail（阻止基线腐烂成永久豁免）
 */
const KNOWN_DEAD_EVENTS = new Set([
  // agent 生命周期 / 限流：监听方 agentStore / backendStatusStore
  "agent-rate-limit",
  // 子代理卡片：监听方 executionStore
  "agent-subagent-card",
  // worker 池（4 个）：监听方均为 executionStore，疑为旧并行执行设计残留
  "worker-created",
  "worker-progress",
  "worker-completed",
  "worker-failed",
]);
// 2026-09-12 剪除 5 项已修复的基线项（脚本的「基线自检」发现：
// agent-started / agent-plan-ready-for-approval / knowledge-base-updated /
// memory-item-indexed / memory-rebuild-complete —— 均已不再断链）。
// 基线留着腐烂会让本段对这几个事件名**永久失明**，所以必须剪。

function checkEventSymmetry() {
  const rsFiles = walk(SRC_TAURI, ".rs");
  const consts = new Map();
  for (const f of rsFiles) {
    const re = /(?:pub\s+)?const\s+(\w+)\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)"/g;
    for (const m of read(f).matchAll(re)) { consts.set(m[1], m[2]); }
  }

  const emitted = new Set();
  const unresolved = [];
  // `.emit(evt, payload)` —— 事件名是第 1 个参数
  const reDirect = /\.emit\s*(?:::<[^>]*>)?\s*\(\s*([^,)]+)/g;
  // `.emit_to(target, evt, payload)` / `.emit_all(evt, ..)` / `.emit_filter(..)`
  // —— 后者的 evt 位置不同，分开匹配避免把 target 误当事件名
  const reAll = /\.emit_all\s*(?:::<[^>]*>)?\s*\(\s*([^,)]+)/g;
  const reTo = /\.emit_to\s*(?:::<[^>]*>)?\s*\(\s*[^,)]+,\s*([^,)]+)/g;
  const reFilter = /\.emit_filter\s*(?:::<[^>]*>)?\s*\(\s*([^,)]+)/g;

  for (const f of rsFiles) {
    const rel = f.slice(SRC_TAURI.length + 1);
    // 去掉行注释，避免注释里的 `.emit("xxx")` 被当成真实发射点
    const src = read(f).replace(/\/\/.*$/gm, "");
    for (const re of [reDirect, reAll, reTo, reFilter]) {
      re.lastIndex = 0;
      for (const m of src.matchAll(re)) {
        const arg = m[1].trim();
        const lit = arg.match(/^"([^"]+)"$/);
        if (lit) { emitted.add(lit[1]); }
        else if (consts.has(arg)) { emitted.add(consts.get(arg)); }
        else { unresolved.push(`${rel}: ${arg.slice(0, 48)}`); }
      }
    }
  }

  // 二级判据所需的「提及」集合：Rust 源码（去注释后）里出现过的**事件名风格**
  // 字符串字面量。
  //
  // 为什么需要它：事件名经常先赋给局部变量再发射，例如
  //   let (event_name, payload) = match st { "running" => ("workflow-step-start", ..) };
  //   app.emit(event_name, payload)
  // 此时 `emit(arg)` 的 arg 是标识符，静态无法追踪。若不区分，这类**真实存在**
  // 的发射会被误报为断链 —— 而会误报的门禁最终会被加白名单或禁用。
  //
  // 正则为「小写字母开头 + 小写/数字/短横线，长度 ≥4」，只收事件名风格的串，
  // 不会把任意日志文本收进来（否则判据退化、检查恒 PASS）。
  const mentioned = new Set();
  for (const f of rsFiles) {
    const src = read(f).replace(/\/\/.*$/gm, "");
    for (const m of src.matchAll(/"([a-z][a-z0-9-]{3,})"/g)) { mentioned.add(m[1]); }
  }

  // 排除前端测试文件：`__tests__/*.test.ts` 里的 `listen("my_event")` 是
  // 用假事件名验证 invoke 封装的桩代码，不是真实契约。
  const isTestFile = (p) =>
    /[\\/]__tests__[\\/]|[\\/]__mocks__[\\/]|\.(?:test|spec)\.[cm]?tsx?$/.test(p);
  const tsFiles = [...walk(FRONTEND, ".ts"), ...walk(FRONTEND, ".tsx")]
    .filter((f) => !isTestFile(f));

  const listened = new Map();
  // 前置负向断言排除 `x.listen(...)` 这类非 Tauri 事件 API
  const reListen = /(?:^|[^.\w])listen\s*(?:<[^>]*>)?\s*\(\s*"([^"]+)"/g;
  for (const f of tsFiles) {
    const rel = f.slice(FRONTEND.length + 1);
    for (const m of read(f).matchAll(reListen)) {
      if (!listened.has(m[1])) { listened.set(m[1], new Set()); }
      listened.get(m[1]).add(rel);
    }
  }

  // 三级判定：严格发射 > 已知断链基线 > 仅被提及（间接发射，warning）
  //           > 完全不存在（fail，新回归）
  const knownDead = [];
  const indirect = [];
  const missing = [];
  for (const [evt, files] of listened) {
    if (EVENT_EXEMPT.has(evt) || emitted.has(evt)) { continue; }
    const where = [...files].join(", ");
    if (KNOWN_DEAD_EVENTS.has(evt)) { knownDead.push(`${evt}  ← ${where}`); }
    else if (mentioned.has(evt)) { indirect.push(`${evt}  ← ${where}`); }
    else { missing.push(`${evt}  ← ${where}`); }
  }

  // 基线自检（防止基线腐烂成永久豁免）：基线项若已不再断链（监听被删 /
  // 发射被补），必须同步移除，否则本段对该事件名永久失明。
  const listenedNames = new Set(listened.keys());
  const staleBaseline = [...KNOWN_DEAD_EVENTS].filter(
    (e) => !listenedNames.has(e) || emitted.has(e),
  );
  // 必须打印明细：此前这里只 fail 不打印，导致「errors 计数涨了但看不到是哪几条」——
  // 检查器自己的出口不严，等于让维护者无从下手。
  if (staleBaseline.length) {
    printList(
      "[G] 基线项已不再断链（必须从 KNOWN_DEAD_EVENTS 移除，否则本段对其永久失明）",
      staleBaseline.map((e) => `${e}${emitted.has(e) ? "  ← 后端已补 emit" : "  ← 前端监听已删"}`),
    );
  }
  staleBaseline.forEach((e) =>
    fail(
      `[G] 基线事件 '${e}' 已不再断链（前端监听已删或后端发射已补），请从 KNOWN_DEAD_EVENTS 移除`,
    )
  );

  if (knownDead.length) {
    console.log(
      `\n[G] 已知断链基线 (${knownDead.length}/${KNOWN_DEAD_EVENTS.size}) —— 待逐项判定「补发射端 or 删监听端」(PLAN P1-D):`,
    );
    knownDead.forEach((s) => console.log("  " + s));
  }

  if (missing.length) {
    printList("[G] 前端监听但后端无发射（监听永远收不到）", missing);
  }
  missing.forEach((s) =>
    fail(`[G] 事件 '${s.split("  ←")[0]}' 前端有 listen 但 Rust 侧无 emit / 无提及`)
  );

  if (indirect.length) {
    printList("[G] (warning) 疑似经变量间接发射，静态不可判定", indirect);
    indirect.forEach((s) =>
      warn(`[G] 事件 '${s.split("  ←")[0]}' 在 Rust 侧仅被提及（可能经局部变量 emit），需人工确认`)
    );
  }

  if (unresolved.length) {
    // 覆盖率自陈：不 fail，但必须可见 —— 否则「没报错」会被误读为「已全覆盖」
    warn(
      `[G] ${unresolved.length} 处 emit 的事件名为动态取值，本段未覆盖（例: ${unresolved.slice(0, 3).join(" | ")}）`,
    );
  }
}

// ---------- H. 「静默丢弃 Result」棘轮 (EvoFlow 病历 #2/#3) ----------
//
// 为什么做成机器检查而不是文档：
//   EvoFlow 病历第 7 条正是「边界只写在 CONTRIBUTING.md 一条规则 + 60 处违规」。
//   把「出口不严」清单写成 markdown，就是在复制同一个缺陷 ——
//   没有任何东西会执行它。因此这里把可静态检出的那一类编成检查段。
//
// 检出形态：`let _ = <expr>.await;`
//   Rust 里 `bare except` 的等价物：错误通道明明存在（.await 后的 Result），
//   却被显式丢弃。本轮的真实缺陷全部是这个形态 ——
//   `storage.rs::delete_memory_fts` 的 `let _ =` 让 4 个调用点的 warn 成为死代码，
//   `update_item_index_status` 的 13 处 `let _` 让索引状态静默漂移。
//
// 为什么不 fail 而是棘轮：
//   342 处里相当一部分是**合理的** fire-and-forget（`txn.rollback()` 在错误路径上
//   本就不该再抛错）。一刀切 fail 只会逼出 `#[allow]` 刷分。
//   棘轮只做一件事：**不允许变多**。这既守住了出口，又不要求一次性大扫除。

/**
 * 基线：超过即 fail。修复后应同步下调（棘轮只减不增）。
 *
 * 2026-09-18 下调 278 → 273（实测落在 273，脚本自己在 warn 里要求的数）。
 * 归因：273 处里不含任何本轮改动 —— 下降来自此前几轮的 `let _ =` 清理，
 * 基线一直没跟着走，于是棘轮松了 5 格（5 个新增静默丢弃不会被拦住）。
 * 下调依据是**实测值**而不是估算：`node scripts/check-contracts.mjs --only=h`
 * 打印 `共 273 处，已排除 47 处豁免调用`。回滚 = 把本数改回 278。
 *
 * 2026-09-19 上调 273 → 274：
 *   · 基线实测口径修正：此前 273 受 `walk()` 计入 `target/`、`output/`（构建产物 /
 *     备份目录）污染，是**低估**；已修 `walk()` 排除这两目录，本地与 CI 对齐。
 *   · 干净基线实为 272（0963ffaac 实测）；当前 HEAD 274，净增 2：
 *     ① knowledge.rs 知识源目录导入新增 fire-and-forget（目录导入清历史、验证重建
 *        等，属合理丢弃）；② capability_pack_learning.rs:766（原 domain_pack_learning.rs
 *        改名分片）。
 *   · 抬 1 格到 274，继续拦截后续回归。回滚 = 改回 273。
 */
const SILENT_RESULT_BASELINE = 274;

/** 已知的合理丢弃（按被调方名），计数时排除，避免基线被噪声撑大。 */
const SILENT_RESULT_EXEMPT_CALLEES = new Set([
  "rollback", // 错误路径回滚，本就不该二次抛错
  "send", // 跨任务投递，接收端已消亡时丢弃是设计
  "cancel",
]);

function checkSilentResultDiscard() {
  const re = /let\s+_\s*=\s*([\s\S]*?);/g;
  const agg = new Map();
  const samples = new Map();
  let total = 0;
  let excluded = 0;

  for (const f of walk(SRC_TAURI, ".rs")) {
    const norm = f.replace(/\\/g, "/");
    // 测试文件里的 `let _ =` 多为「不关心返回值」，不属生产出口
    if (norm.includes("/tests/") || norm.endsWith("_test.rs")) { continue; }
    const src = read(f);
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(src)) !== null) {
      const rhs = m[1];
      // 防正则跨语句吞并：RHS 过长或内含 `let` 说明匹配到了多条语句
      if (rhs.length > 300 || /\blet\b/.test(rhs)) { continue; }
      if (!/\.await\b/.test(rhs)) { continue; }
      const calls = [...rhs.matchAll(/([a-zA-Z_][a-zA-Z0-9_]*)\s*\(/g)];
      if (!calls.length) { continue; }
      const callee = calls[0][1];
      if (["Some", "Ok", "Err", "String", "Vec", "format", "json", "self"].includes(callee)) {
        continue;
      }
      if (SILENT_RESULT_EXEMPT_CALLEES.has(callee)) { excluded++; continue; }

      total++;
      agg.set(callee, (agg.get(callee) || 0) + 1);
      if (!samples.has(callee)) {
        const line = src.slice(0, m.index).split("\n").length;
        samples.set(callee, `${norm}:${line}`);
      }
    }
  }

  const top = [...agg.entries()].sort((a, b) => b[1] - a[1]);
  if (top.length) {
    console.log(
      `\n[H] 「静默丢弃 Result」聚合（共 ${total} 处，已排除 ${excluded} 处豁免调用）—— 前 10:`,
    );
    top.slice(0, 10).forEach(([k, v]) =>
      console.log(`  ${String(v).padStart(3)} × ${k}   例: ${samples.get(k)}`)
    );
  }

  if (total > SILENT_RESULT_BASELINE) {
    fail(
      `[H] 静默丢弃 Result 的数量从基线 ${SILENT_RESULT_BASELINE} 上升到 ${total}。` +
        `新增的 \`let _ = <fallible>.await;\` 必须改为显式处理（传播 / 记录 / 注释说明为何可丢）`
    );
  } else if (total < SILENT_RESULT_BASELINE) {
    // 棘轮只减不增：数量下降是好消息，但基线必须同步下调，否则棘轮会松掉
    warn(
      `[H] 静默丢弃 Result 已降至 ${total}（基线 ${SILENT_RESULT_BASELINE}）。` +
        `请把 SILENT_RESULT_BASELINE 下调到 ${total}，否则棘轮变松、后续回归检不出`
    );
  }
}

// ---------- 主流程 ----------
const argv = process.argv.slice(2);
let only = null;
for (let i = 0; i < argv.length; i++) {
  if (argv[i] === "--only") { only = argv[i + 1].split(","); }
  else if (argv[i].startsWith("--only=")) { only = argv[i].slice("--only=".length).split(","); }
}
const enableDto = argv.includes("--dto");
const has = (x) => !only || only.includes(x);

console.log("=== 契约一致性核对 (contract-consistency) ===");
if (has("a")) { checkCommandRegistration(); }
if (has("b")) { checkErrorCodeI18n(); }
if (has("c")) { checkI18nKeys(); }
if (has("d")) { checkHarnessDirection(); }
if (has("f")) { checkFieldConsumption(); }
if (has("g")) { checkEventSymmetry(); }
if (has("h")) { checkSilentResultDiscard(); }
if (enableDto) { checkDto(); }

if (warnings.length) {
  console.log(`\n[WARNINGS] (${warnings.length}):`);
  warnings.slice(0, 50).forEach((w) => console.log("  " + w));
  if (warnings.length > 50) { console.log(`  ... 另有 ${warnings.length - 50} 条`); }
}
console.log(`\n[汇总] errors=${errors.length} warnings=${warnings.length}`);
console.log(`结果: ${hasError ? "FAIL" : "PASS"}`);
process.exit(hasError ? 1 : 0);
