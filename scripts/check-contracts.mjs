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
 *   G. 事件名契约:          TS 侧 listen("evt") 必须命中后端枚举 IpcEventName 的成员,
 *                            且每个成员须有发射引用; 生成物 events.ts 须与枚举一致
 *                            (防「前端监听永远收不到」的静默失效; R2-2 起由枚举驱动)
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
import { sanitize, testModuleRanges } from "./lib/rust-source-scan.mjs";

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
  {
    field: "horizon_results_json",
    container: "stock_reflections::Model",
    declaredIn: "crates/entities/src/stock_reflections.rs",
    rationale:
      "四周期反思结果 JSON（PLAN-stock-reflection-four-horizon）—— 反思写入点须落库、统计展开点须读它，"
      + "不得只声明；两侧任一缺失都会退化成「四周期面板恒空」而无人察觉",
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

// ---------- G. 事件名契约（枚举单一来源） ----------
/**
 * R2-2 之前，事件名由两侧各写字符串字面量：后端 `.emit("agent-done", ..)`、
 * 前端 `listen("agent-done")`。任一侧改名或删除，另一侧**静默失效** ——
 * 前端监听器永远收不到消息，不报错、单测也过（本仓已在
 * `agent-plan-ready-for-approval` 上真实踩到该形态）。
 *
 * 现在事件名由后端枚举 `axagent_harness::IpcEventName` 单一来源驱动：
 * 发射点写 `IpcEventName::AgentDone.as_str()`，前端 `listen` 的形参类型是
 * schema-gen 生成的 `@/types/generated/events` 联合类型。
 *
 * 于是「改名 / 删除」由编译器拦住，本段只校验**编译期覆盖不到的四条边**：
 *
 *   G1 前端 `listen("X")` 的 X 必须在枚举里 —— 监听一个后端不存在的事件名。
 *   G2 每个枚举成员必须有 ≥1 处发射引用（`IpcEventName::Variant`）——
 *      G1 只保证名字合法，G2 才保证它真被发出去（「有声明无入边」）。
 *   G3 已知死链白名单自检：白名单项一旦被接上（出现发射引用）必须移出，
 *      否则本段对该事件名**永久失明**。
 *   G4 `src/types/generated/events.ts` 与枚举逐项一致 —— 生成物腐烂时
 *      前端会继续接受已被删除的事件名（typecheck 假绿）。
 */
const EVENT_NAME_ENUM = join(SRC_TAURI, "crates", "harness", "src", "event_names.rs");
const EVENT_NAMES_TS = join(FRONTEND, "types", "generated", "events.ts");

/**
 * 已确认断链、但**仍在前端监听**的事件名（2026-09-12 建立，R2-2 起改为枚举成员）。
 *
 * 这 6 个名字在 src-tauri 中零发射引用（grep 确认），前端却仍在 `listen`，
 * 属真断链债务。它们必须留在枚举里（否则前端 `listen` 会 typecheck 报错），
 * 故在本段以「无发射引用白名单」登记。语义是「已知债务，必须与 PLAN 同步」，
 * 不是「永久豁免」：一旦补上发射端，G3 会要求把它从白名单移出。
 * 逐个处置方向见 `PLAN-evoflow-borrowings.md` 的 P1-D。
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

/** 解析 `event_names.rs` 的 `as_str()`，返回 variant 名 → 事件通道名 */
function readEventNameEnum() {
  if (!existsSync(EVENT_NAME_ENUM)) {
    fail("[G] 事件名枚举文件不存在: crates/harness/src/event_names.rs —— 本段失去判据来源");
    return new Map();
  }
  const src = read(EVENT_NAME_ENUM).replace(/\/\/.*$/gm, "");
  const map = new Map();
  for (const m of src.matchAll(/IpcEventName::(\w+)\s*=>\s*"([^"]+)"/g)) {
    map.set(m[1], m[2]);
  }
  if (map.size === 0) {
    fail(
      "[G] 未能从 event_names.rs 解析出任何 variant —— 枚举形态已变"
        + "（as_str 不再由 match 字面量构成），本段须同步改造",
    );
  }
  return map;
}

function checkEventSymmetry() {
  const variantToName = readEventNameEnum();
  if (variantToName.size === 0) { return; }
  const nameToVariant = new Map([...variantToName].map(([v, n]) => [n, v]));
  const enumNames = new Set(nameToVariant.keys());

  // 发射引用统计口径：src-tauri 下全部 .rs（去行注释），排除枚举定义文件本身。
  const bodies = walk(SRC_TAURI, ".rs")
    .filter((f) => f !== EVENT_NAME_ENUM)
    .map((f) => read(f).replace(/\/\/.*$/gm, ""));
  const refCount = (variant) => {
    const re = new RegExp(`\\bIpcEventName::${variant}\\b`, "g");
    let n = 0;
    for (const src of bodies) { n += (src.match(re) || []).length; }
    return n;
  };

  // ── G1: 前端 listen 只能取枚举成员 ──
  // 排除前端测试文件：`__tests__/*.test.ts` 里的事件名是验证 invoke 封装的桩代码，
  // 不是真实契约。
  const isTestFile = (p) =>
    /[\\/]__tests__[\\/]|[\\/]__mocks__[\\/]|\.(?:test|spec)\.[cm]?tsx?$/.test(p);
  const tsFiles = [...walk(FRONTEND, ".ts"), ...walk(FRONTEND, ".tsx")]
    .filter((f) => !isTestFile(f));

  const listened = new Map();
  // 前置负向断言排除 `_unlisten(...)` 这类局部变量名。
  // 泛型参数用**有界惰性**匹配（`{0,N}?`）：`<[^>]*>` 会被载荷类型里的嵌套 `>`
  // （如 `Record<string, unknown>`）提前闭合，于是「多行泛型 + 字面量」的调用整条漏掉。
  // 2026-09-25 实测该形态漏了 5 个真实事件名（agent-render-ui / agent-update-ui /
  // serenity-screening-completed / workflow-completed）—— 门禁漏报比没有门禁更糟。
  const reListen = /(?:^|[^.\w])listen\s*(?:<[\s\S]{0,800}?>)?\s*\(\s*"([^"]+)"/g;
  for (const f of tsFiles) {
    const rel = f.slice(FRONTEND.length + 1);
    for (const m of read(f).matchAll(reListen)) {
      if (!listened.has(m[1])) { listened.set(m[1], new Set()); }
      listened.get(m[1]).add(rel);
    }
  }
  const unknownNames = [...listened]
    .filter(([evt]) => !enumNames.has(evt))
    .map(([evt, files]) => `${evt}  ← ${[...files].join(", ")}`);
  if (unknownNames.length) {
    printList("[G1] 前端 listen 的事件名不在 IpcEventName 中（后端无此契约，监听永远收不到）", unknownNames);
  }
  unknownNames.forEach((s) =>
    fail(`[G1] 事件 '${s.split("  ←")[0]}' 前端有 listen 但不在 axagent_harness::IpcEventName 中`)
  );

  // ── G2: 每个枚举成员必须有发射引用 ──
  const noEmitRef = [];
  for (const [variant, name] of variantToName) {
    if (refCount(variant) === 0 && !KNOWN_DEAD_EVENTS.has(name)) {
      noEmitRef.push(`${name}  (IpcEventName::${variant})`);
    }
  }
  if (noEmitRef.length) {
    printList("[G2] 枚举成员无任何发射引用（有声明无入边）", noEmitRef);
  }
  noEmitRef.forEach((s) =>
    fail(`[G2] 事件 '${s.split("  (")[0]}' 是枚举成员但全仓无发射引用 —— 补发射端或删成员`)
  );

  // ── G3: 白名单自检（防基线腐烂成永久豁免）──
  const staleBaseline = [];
  for (const name of KNOWN_DEAD_EVENTS) {
    const variant = nameToVariant.get(name);
    if (!variant) { staleBaseline.push(`${name}  ← 已不在枚举中（监听已删？）`); continue; }
    if (refCount(variant) > 0) { staleBaseline.push(`${name}  ← 已补上发射引用`); }
  }
  if (staleBaseline.length) {
    printList("[G3] 已知死链白名单已失效（必须从 KNOWN_DEAD_EVENTS 移除，否则本段对其永久失明）", staleBaseline);
  }
  staleBaseline.forEach((s) => fail(`[G3] 白名单事件 '${s.split("  ←")[0]}' ${s.split("←")[1].trim()}，请从 KNOWN_DEAD_EVENTS 移除`));

  // ── G4: 生成物与枚举一致 ──
  if (!existsSync(EVENT_NAMES_TS)) {
    fail("[G4] 生成物不存在: src/types/generated/events.ts —— 请运行 cargo run -p schema-gen -- event-names");
  } else {
    const listed = new Set([...read(EVENT_NAMES_TS).matchAll(/"([^"]+)"/g)].map((m) => m[1]));
    const missingInTs = [...enumNames].filter((n) => !listed.has(n));
    const extraInTs = [...listed].filter((n) => !enumNames.has(n));
    if (missingInTs.length) { printList("[G4] events.ts 缺这些枚举成员", missingInTs); }
    if (extraInTs.length) { printList("[G4] events.ts 多出枚举中没有的名字（生成物已腐烂）", extraInTs); }
    if (missingInTs.length || extraInTs.length) {
      fail(
        `[G4] src/types/generated/events.ts 与 IpcEventName 不一致`
          + `（缺 ${missingInTs.length} / 余 ${extraInTs.length}）`
          + " —— 运行 cargo run -p schema-gen -- event-names 重新生成",
      );
    }
  }

  console.log(
    `\n[G] 事件名契约: 枚举 ${variantToName.size} 个成员 / 前端监听 ${listened.size} 个名字`
      + ` / 已知死链白名单 ${KNOWN_DEAD_EVENTS.size} 个`,
  );
}

// ---------- H. 「静默丢弃 Result」棘轮 (EvoFlow 病历 #2/#3) ----------
//
// 为什么做成机器检查而不是文档：
//   EvoFlow 病历第 7 条正是「边界只写在 CONTRIBUTING.md 一条规则 + 60 处违规」。
//   把「出口不严」清单写成 markdown，就是在复制同一个缺陷 ——
//   没有任何东西会执行它。因此这里把可静态检出的那一类编成检查段。
//
// 运行位置：本段随 `npm run contracts` 一起跑，CI 在 `.github/workflows/ci.yml:510`
//   调 `npm run contracts -- --only=a,c,d,f,g,h` ⇒ 带牙齿（I 段当年移出去的理由不再成立）。
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
 *
 * 2026-09-20 下调 274 → 263（实测值，即脚本自己在 warn 里要求的数）：
 *   11 × set_status 一次性清完 —— `src-tauri/src/init/services.rs` 的 11 处 cron 分支
 *   `if !recurring { let _ = store.set_status(&job_id, CronJobStatus::Disabled).await; }`
 *   全部改为 `if !store.set_status(..).await { tracing::warn!(..) }`。
 *   这几处**不是**「丢弃 Result」语义：`set_status` 返 `bool`（`crates/runtime-core/src/cron_job.rs:621`），
 *   false = 未找到该任务或持久化失败 ⇒ 一次性 cron 任务仍留 Active ⇒ 下次到点重复跑。
 *   274 − 11 = 263，与 11 处一一对应（复跑实测：`共 263 处，已排除 47 处豁免调用`）。回滚 = 改回 274。
 *   2026-10-15 阶段 A（`docs/plans/PLAN-monitor-chain-closure.md`）262 → 261：
 *   `src-tauri/src/init/monitor_emitter.rs` 的 `let _ = dispatcher.dispatch_alert(..).await`
 *   改为读 `NotificationDispatchSummary`，零渠道时显式 warn「通知不外推」、有渠道但失败时按字段 warn。
 *   同批删掉的 `AppMonitorEmitter` 里那处 `let _ = self.app.emit(..)` **不带 `.await`**，
 *   本就不在判据 H 的识别形态内 —— 所以净 −1 而不是 −2（复跑实测：`共 261 处，已排除 47 处豁免调用`）。
 *   2026-10-15 阶段 F（同一 PLAN §6）261 → 259：条件单接线批退役两只「丢弃装载失败」的手 ——
 *   `risk_inspection.rs:62` 的 5 分钟热加载 `let _ = load_from_db(..)`（整文件退役）与
 *   `commands/conditional_order.rs:112` 列表命令里的同名丢弃（改成读新表并 `?` 传播）。
 *   两处都是「装载失败仍然返回成功」的形态：前者让风控拿着上一次的规则继续跑，
 *   后者让列表在表读不通时显示空集 —— 与本轮「拿不到不得伪装成没有」同一条线。
 *
 * 2026-10-28 下调 259 → 246（**改的是判据口径，不是被检代码**）：
 *   CI 报 259 → 262，新增三处全在 `crates/providers/src/stream_budget.rs:321/327/329`
 *   —— 那是 `#[cfg(test)] mod http_behavior_tests` 里假上游 TCP server 的
 *   `let _ = socket.read/write_all/flush().await`，测试替身不关心返回值，不是生产出口。
 *   真缺陷在本段的**取源口径漏了一格**：只按路径排除测试面（`/tests/`、`_test.rs`），
 *   内联 `#[cfg(test)] mod` 从没被排除过 —— 于是同一份判据里已经躺着 14 处测试丢弃
 *   被当成生产出口计进了基线（`src-tauri/crates/agent/src/steer_manager.rs` 等），
 *   再加 2 处**注释里抄写的旧代码形态**（`src-tauri/crates/trajectory/src/storage.rs:1568`
 *   文档注释、`src-tauri/src/indexing.rs:950` 行注释「此前 `let _ =` 丢掉错误」）。
 *   259 这个数里 16 处是假的。
 *   修法：逐文件先 `sanitize`（注释与字符串按**等长空白**剥，行号不变），再用
 *   `testModuleRanges` 的区间剔除测试模块；两条都在 `scripts/lib/rust-source-scan.mjs`，
 *   与 `check-port-axiom-wiring.mjs` 共用同一份剥离器（禁区 12）。
 *   ⚠ 途中踩到一个更狠的坑：`testModuleRanges` 原写法「属性往后找最近的 `mod X {`」，
 *   会把**贴在 `use` 上的** `#[cfg(test)]`（全仓 1018 处属性里 67 处不紧贴 `mod X {`）
 *   一路圈到那个模块闭合 —— 实测在 `src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs`
 *   圈出 38–7365、在 `src-tauri/src/commands/conversations/mod.rs` 圈出 11–1800。
 *   照它出数会得到 235，即**静默豁免 11 处生产丢弃**（`resolve_approval_for_timeout` ×2、
 *   `update_document_status_with_error`、`delete_by_id` ×2 等）、把棘轮一次放松 24 格。
 *   已改成只认紧邻 `mod X {` 的形态；顺带让 port-axiom 门多认出 2 个此前被误圈的写路径
 *   （`seed_reflection_workflow_template` / `seed_event_triggered_decision_template`，均已接门禁）。
 *   负控：`node scripts/check-contracts.mjs --selftest` 8 条全绿；三种变异各红对应的格
 *   （撤测试剔除→1 红、撤 sanitize→2 红、换回宽松区间→1 红）。
 *   回滚 = 把本数改回 259 并撤掉 sanitize 两步。
 */
const SILENT_RESULT_BASELINE = 246;

/** 已知的合理丢弃（按被调方名），计数时排除，避免基线被噪声撑大。 */
const SILENT_RESULT_EXEMPT_CALLEES = new Set([
  "rollback", // 错误路径回滚，本就不该二次抛错
  "send", // 跨任务投递，接收端已消亡时丢弃是设计
  "cancel",
]);

/**
 * 逐源文件抽取「静默丢弃」命中。口径 = **只看生产代码**，两条剥除：
 *
 * ① 注释与字符串字面量里的同形态不算 —— `sanitize` 按**等长空白**剥（换行原样），
 *    所以净化文本的行号与原文一一对应。不算数的理由与「注释里的名字不算引用面」同一条：
 *    写一行注释就能骗过门。实测本仓有两处这样白计 —— `trajectory/src/storage.rs` 的
 *    文档注释抄了修复前的旧代码形态、`src/indexing.rs` 的行注释写了「此前 `let _ =` 丢掉错误」。
 * ② `#[cfg(test)] mod …{…}` 整块不算 —— 与「路径含 `/tests/` 的文件不扫」是同一条理由
 *    （测试里的 `let _ =` 是「不关心返回值」，不是生产出口），但只按**路径**枚举会漏掉
 *    内联测试模块：`crates/providers/src/stream_budget.rs` 的假上游 TCP server 就是这么
 *    把棘轮顶红的（2026-10-28，三处 `let _ = socket.*().await`）。
 *    区间判定复用 `scripts/lib/rust-source-scan.mjs` 的 `testModuleRanges`（禁区 12：
 *    一份剥离器，两个门共用），其负控在 `check-port-axiom-wiring.mjs --selftest`。
 *
 * 返回 `[{ callee, line, exempt }]`，line 是**原文**行号。
 */
function silentDiscardHits(src) {
  const clean = sanitize(src);
  const rawLineCount = src.split("\n").length;
  const lines = clean.split("\n");
  if (lines.length !== rawLineCount) {
    throw new Error("净化器行数不守恒（行号不可信），拒绝出数");
  }
  const ranges = testModuleRanges(lines);
  const re = /let\s+_\s*=\s*([\s\S]*?);/g;
  const hits = [];
  let m;
  while ((m = re.exec(clean)) !== null) {
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
    const zeroBasedLine = clean.slice(0, m.index).split("\n").length - 1;
    if (ranges.some(([a, b]) => zeroBasedLine >= a && zeroBasedLine <= b)) { continue; }
    hits.push({ callee, line: zeroBasedLine + 1, exempt: SILENT_RESULT_EXEMPT_CALLEES.has(callee) });
  }
  return hits;
}

function checkSilentResultDiscard() {
  const agg = new Map();
  const samples = new Map();
  let total = 0;
  let excluded = 0;

  for (const f of walk(SRC_TAURI, ".rs")) {
    const norm = f.replace(/\\/g, "/");
    // 测试文件里的 `let _ =` 多为「不关心返回值」，不属生产出口
    if (norm.includes("/tests/") || norm.endsWith("_test.rs")) { continue; }
    let hits;
    try {
      hits = silentDiscardHits(read(f));
    } catch (e) {
      // 净化器行数不守恒 ⇒ 行号与判据都不可信，指名报错而不是静默跳过该文件
      fail(`[H] ${norm}: ${e.message}`);
      continue;
    }
    for (const h of hits) {
      if (h.exempt) { excluded += 1; continue; }
      total += 1;
      agg.set(h.callee, (agg.get(h.callee) || 0) + 1);
      if (!samples.has(h.callee)) {
        samples.set(h.callee, `${norm}:${h.line}`);
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

// ---------- I. （已移出）内置 task_type 白名单跨语言对账 ----------
//
// 2026-10-15（裁定 17）先写在这一段，随后**整段移出**到 `scripts/check-task-type-whitelist.mjs`：
// 移出当时本脚本不在 CI 里（只 `npm run contracts` 手动跑）⇒ 写在这里等于没牙齿。
// ⚠ 该前提已过半：本脚本现由 `.github/workflows/ci.yml:510` 以
// `--only=a,c,d,f,g,h` 调用 —— 但 `--only` 不带 i，**I 段依旧没有牙齿**，移出仍是正解。
// 留下这段占位是为了让「I 段的判据去哪了」有迹可循，**不要在这里重新实现一遍**
// （两份实现＝两套真相，正是被移出的那条判据要消灭的形态）。

// ---------- 自检（[H] 段取源口径的正负对照）──────────────────────────
//
// 判据改了「算什么不算什么」，就必须能被证伪：下面每条只动一个维度，
// 撤掉对应的剥除步骤时**恰好红那一条**。跑法 `node scripts/check-contracts.mjs --selftest`
function selftestSilentDiscard() {
  const cases = [];
  const add = (name, actual, expected) => {
    cases.push({ name, pass: JSON.stringify(actual) === JSON.stringify(expected), actual, expected });
  };
  const shape = (call) => `let _ = ${call}.await;`;
  const tally = (src) => silentDiscardHits(src).filter((h) => !h.exempt).length;

  add("生产丢弃计入", tally(`fn a() {\n    ${shape("repo.save()")}\n}`), 1);
  add(
    "内联 #[cfg(test)] mod 里的同形态不计入",
    tally(`fn a() {\n    ${shape("repo.save()")}\n}\n#[cfg(test)]\nmod tests {\n    fn t() {\n        ${shape("repo.save()")}\n    }\n}`),
    1,
  );
  add(
    // 这条必须用「属性贴 use + 其后还有生产 mod」的形态：只写 `#[cfg(test)] pub use foo;`
    // 再跟一个裸函数的话，宽松版往后找不到 `mod X {` ⇒ 两种口径同为 1，等于没设防。
    "属性贴在 use 上 ⇒ 其后的生产 mod 不得被误圈",
    tally(`#[cfg(test)]\npub use foo;\nmod storage {\n    fn a() {\n        ${shape("repo.save()")}\n    }\n}`),
    1,
  );
  add(
    "文档注释里抄的旧代码形态不计入",
    tally(`/// 原实现是 \`${shape("f.delete()")}\`\nfn a() {\n    ${shape("repo.save()")}\n}`),
    1,
  );
  add(
    "豁免 callee 单独归类（rollback 不进 total）",
    silentDiscardHits(`fn a() {\n    ${shape("txn.rollback()")}\n}`),
    [{ callee: "rollback", line: 2, exempt: true }],
  );
  add("无 .await 的 `let _ =` 不算丢弃", tally(`fn a() {\n    let _ = repo.save();\n}`), 0);
  add(
    "字符串里的分号不再提前截断 RHS",
    silentDiscardHits(`fn a() {\n    let _ = f(";").await;\n}`).map((h) => h.callee),
    ["f"],
  );
  add("行号指向原文那一行", silentDiscardHits(`fn a() {\n    ${shape("repo.save()")}\n}`)[0]?.line, 2);

  let failed = 0;
  console.log("=== [H] 段自检（静默丢弃判据的正负对照）===");
  for (const c of cases) {
    if (c.pass) console.log(`  ✓ ${c.name}`);
    else {
      failed += 1;
      console.log(`  ✗ ${c.name}\n      期望 ${JSON.stringify(c.expected)}，实得 ${JSON.stringify(c.actual)}`);
    }
  }
  console.log(`\n自检：${cases.length - failed} 通过 / ${failed} 失败`);
  process.exit(failed > 0 ? 1 : 0);
}

// ---------- 主流程 ----------
const argv = process.argv.slice(2);
if (argv.includes("--selftest")) { selftestSilentDiscard(); }
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
// ⚠ 必须打印 errors 明细：此前 errors 数组**只计数、从不输出**（各段内部只对
// 「G 事件」这一处打印过明细，其余段 fail 后静默），于是出现「[汇总] errors=1
// 但通篇看不到是哪一条」—— 与 445 行注释里记录的那个坑是同一个，只是当时只修了
// G 段、没修汇总处。门禁只报计数不报明细 ⇒ 不可归因 ⇒ 收尾时只能猜。
if (errors.length) {
  console.log(`\n[ERRORS] (${errors.length}):`);
  errors.slice(0, 50).forEach((e) => console.log("  " + e));
  if (errors.length > 50) { console.log(`  ... 另有 ${errors.length - 50} 条`); }
}
console.log(`\n[汇总] errors=${errors.length} warnings=${warnings.length}`);
console.log(`结果: ${hasError ? "FAIL" : "PASS"}`);
process.exit(hasError ? 1 : 0);
