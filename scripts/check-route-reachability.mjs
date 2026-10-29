#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// 路由可达性门：每条注册路由必须至少有一个人口。
//
// 为什么需要（2026-10-28 立）：
//   `pageRegistry` 声明了约 48 条路由，而侧栏只挂了 19 项。差额页面里有一批
//   只能靠深链或命令面板到达（`/replay-workbench`、`/memory`、`/learning-graph` …）。
//   一次性切换路由（`PLAN-frontend-information-architecture.md` D2）时，这类页面
//   最容易被切成「功能还在、入口没了」。
//
// 本门只做静态文本提取，不解析 TSX：
//   - 路由：`ContentArea.tsx` 的 `<Route path=…>` + `pageRegistry` + `domainMeta`
//   - 入口：`navRegistry`、`CommandPalette`、全仓 `navigate("…")` / `to="…"` / `href="…"`
//
// 用法：
//   node scripts/check-route-reachability.mjs            # 报告模式，总是 exit 0
//   node scripts/check-route-reachability.mjs --strict   # 门禁模式，有未达路由 / 已删键引用 / 已删路由跳转则 exit 1

import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = process.cwd();
const STRICT = process.argv.includes("--strict");
const JSON_OUT = process.argv.includes("--json");

/** 内部功能：允许没有导航入口，只保留深链。 */
const INTERNAL_ROUTES = new Set([
  "/dynamic-ui/:schemaId",
  // 快速操作：独立浮层窗口，由全局快捷键 `show_quickbar` 加载（`__route=quickbar`），
  // 不走侧栏导航，2026-10-28 误接入导航后移除。
  "/chat/quickbar",
]);

/** 兼容 / 重定向 / 兜底路由：不是功能页，不要求入口。 */
const COMPAT_ROUTES = new Set([
  "/devtools", // redirectToDevTools
  "/link", // /gateway 的别名（同一组件），侧栏入口在 /gateway
  "/opc/industry/:packId", // 旧链 → /opc/domain/:packId
  "*", // NotFound
]);

function read(rel) {
  return readFileSync(join(ROOT, rel), "utf8");
}

/** 剥掉注释，避免把示例路径当成真实路由。
 *
 * 顺序很关键：**先剥行注释，再剥块注释**。
 * 反例（2026-10-28 实测）：`navRegistry.tsx` 的行注释里写了 `/opc/*`，
 * 若先跑块注释正则，`/*` 会被当成块注释起点，连同后面几十行代码一起吞掉，
 * 导致所有侧栏入口解析为 0 —— 门禁自己先失灵。
 */
export function stripComments(src) {
  return src
    .replace(/(^|[^:])\/\/[^\n]*/g, "$1")
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "));
}

/** `pageRegistry.ts` 的 `key: "/path"`。动态展开由 domainMeta 单独处理。 */
export function parsePageRegistry(src) {
  const map = new Map();
  const re = /"([A-Za-z0-9_-]+)"\s*:\s*"(\/[^"]*)"|([A-Za-z0-9_-]+)\s*:\s*"(\/[^"]*)"/g;
  for (const m of src.matchAll(re)) {
    const key = m[1] ?? m[3];
    const path = m[2] ?? m[4];
    map.set(key, path);
  }
  return map;
}

/** `domainMeta.ts` 的 `id: { path: "/…" }`。 */
export function parseDomainPaths(src) {
  const map = new Map();
  const re = /(\w+)\s*:\s*\{\s*path:\s*"([^"]+)"/g;
  for (const m of src.matchAll(re)) {
    map.set(m[1], m[2]);
  }
  return map;
}

/** `DOMAIN_ROUTES` 的 `["pageKey", LazyX]` 列表。 */
function parseDomainRouteKeys(src) {
  const block = src.match(/DOMAIN_ROUTES[^=]*=\s*\[([\s\S]*?)\];/);
  if (!block) { return []; }
  const keys = [];
  const re = /\[\s*"([A-Za-z0-9_-]+)"\s*,/g;
  for (const m of block[1].matchAll(re)) { keys.push(m[1]); }
  return keys;
}

/** 从 `ContentArea.tsx` 提取路由模式。 */
export function parseRoutes(src, registry, domainPaths) {
  const routes = new Map(); // pattern -> source
  const domainRouteKeys = parseDomainRouteKeys(src);
  const routeRe = /<Route\b([\s\S]*?)\/?>/g;

  for (const m of src.matchAll(routeRe)) {
    const attrs = m[1];
    const literal = attrs.match(/\bpath="([^"]+)"/);
    if (literal) {
      routes.set(literal[1], "literal");
      continue;
    }
    const template = attrs.match(
      /\bpath=\{`\$\{BUILTIN_PAGE_PATH(?:\.([A-Za-z0-9_]+)|\["([^"]+)"\])\}([^`]*)`\}/,
    );
    if (template) {
      const key = template[1] ?? template[2];
      const base = registry.get(key);
      if (base) { routes.set(base + template[3], "template:" + key); }
      continue;
    }
    const bracket = attrs.match(/\bpath=\{BUILTIN_PAGE_PATH\["([^"]+)"\]\}/);
    if (bracket) {
      const base = registry.get(bracket[1]);
      if (base) { routes.set(base, "registry[" + bracket[1] + "]"); }
      continue;
    }
    const member = attrs.match(/\bpath=\{BUILTIN_PAGE_PATH\.([A-Za-z0-9_]+)\}/);
    if (member) {
      const base = registry.get(member[1]);
      if (base) { routes.set(base, "registry:" + member[1]); }
      continue;
    }
    if (/\bpath=\{domain\.path\}/.test(attrs)) {
      for (const [id, p] of domainPaths) { routes.set(p, "domain:" + id); }
      continue;
    }
    if (/\bpath=\{BUILTIN_PAGE_PATH\[pageKey\]\}/.test(attrs)) {
      for (const key of domainRouteKeys) {
        const base = registry.get(key);
        if (base) { routes.set(base, "domainRoutes:" + key); }
      }
      continue;
    }
  }
  return routes;
}

/** 从 `navRegistry.tsx` 与 `CommandPalette.tsx` 提取显式入口。 */
export function parseNavEntries(navSrc, registry) {
  const entries = new Set();
  const member = /path:\s*BUILTIN_PAGE_PATH\.([A-Za-z0-9_]+)/g;
  for (const m of navSrc.matchAll(member)) {
    const p = registry.get(m[1]);
    if (p) { entries.add(p); }
  }
  const bracket = /path:\s*BUILTIN_PAGE_PATH\["([^"]+)"\]/g;
  for (const m of navSrc.matchAll(bracket)) {
    const p = registry.get(m[1]);
    if (p) { entries.add(p); }
  }
  return entries;
}

/**
 * 找出所有引用了 `pageRegistry` 中**已不存在键**的调用点（2026-10-29 立）。
 *
 * 为什么必须是门禁而不是靠人 grep：`BUILTIN_PAGE_PATH` 声明为 `Record<string, string>`，
 * 取一个已删除的键**不会编译报错** —— TS 只保证「是 string」，不保证「键存在」。
 * 于是路由搬迁时删掉一个键，全部调用点静默退化为 `undefined`：
 * `navigate(undefined)` 既不跳转也不抛错，表现为「按钮点了没反应」，
 * 而 `npm run typecheck` / 单测（各自只断言自己拼的那段）全绿。
 *
 * 实证（2026-10-29）：2026-10-28 的「股票旧路由删除」把 `pageRegistry` 的
 * `replay-workbench` 键换成 `investReplay: "/invest/replay"`，
 * `stock-workspace/views/BacktestView.tsx` 仍取旧键 ⇒ 回测视图的「时间旅行回放」
 * 按钮恒 `navigate(undefined)`。
 *
 * 同一类缺陷的另一形态是**自己拼已删除的字面量路径**（`/stock-analysis/${id}` 等），
 * 那种靠 `parseRoutes` 与入口集对不上的方式抓不到，需另设已知死路径清单。
 */
const DEAD_PAGE_PATHS = [
  // 2026-10-28 删除的股票旧顶级路由：记录详情已并入 `/invest?tab=workspace&view=analysis`。
  // 任何仍拼这些字面量的跳转都会落到 `path="*"` 的 404 页（用户 2026-10-29 报的正是这条）。
  "/stock-analysis",
  "/trade",
  "/watchlist",
  "/screener",
  "/quant",
  "/backtest",
  "/compare",
  "/replay-workbench",
  // chat 类旧路由：统一走 `/chat/<tab>`。
  "/terminal",
  "/files",
  "/multi-agent",
  "/marketplace",
  "/quickbar",
  // 工作流编辑器旧链：现在是 `/chat/workflow?template=…`。
  "/workflow/new",
];

/**
 * 扫描源码中的 `BUILTIN_PAGE_PATH` 使用与硬编码旧路径跳转。
 *
 * 返回 `{ missingKeys, deadPathJumps }` 两组问题；两者都是**运行时静默失效**，
 * 静态类型与既有门禁都拦不住。
 */
export function parseRegistryAndDeadPaths(files, registry) {
  const missingKeys = [];
  const deadPathJumps = [];
  // 已记录命中，按 file+target+dead 去重：
  // ⚠ 单个模板串会被多条正则各匹配到一次（`navigate(\`/x/${id}\`)` 既被「整串」规则命中、
  // 也曾被「静态前缀」规则命中）⇒ 不去重会把 1 处缺陷报成 2 处，读数虚高。
  const seenJumps = new Set();
  const seenKeys = new Set();
  // 只认「跳转语义」的出现点：navigate(...) / to="..." / href="..." / path: "..."。
  // 裸字符串（如 i18n 文案、注释里的历史描述）不算 —— 那会把文档也判成缺陷。
  //
  // 模板串只留**一条**规则：`[^`]+` 已能吞下 `${id}` 之类的插值片段并给出完整目标，
  // 再叠一条「静态前缀」规则只会重复命中同一个缺陷。
  const jumpRes = [
    /navigate\(\s*"([^"]+)"/g,
    /navigate\(\s*`([^`]+)`/g,
    /\bto="([^"]+)"/g,
    /\bhref="([^"]+)"/g,
    /\bpath:\s*"([^"]+)"/g,
  ];
  for (const file of files) {
    const rel = relative(ROOT, file);
    const src = stripComments(readFileSync(file, "utf8"));
    for (const m of src.matchAll(/BUILTIN_PAGE_PATH(?:\.([A-Za-z0-9_]+)|\["([^"]+)"\])/g)) {
      const key = m[1] ?? m[2];
      if (registry.has(key)) { continue; }
      const id = `${rel}:${key}`;
      if (seenKeys.has(id)) { continue; }
      seenKeys.add(id);
      missingKeys.push({ file: rel, key });
    }
    for (const re of jumpRes) {
      for (const m of src.matchAll(re)) {
        const target = m[1];
        for (const dead of DEAD_PAGE_PATHS) {
          // 前缀匹配须落在 `/` / `?` / `${` / 串尾边界上：`/trading` 不该命中 `/trade`
          const hit = target === dead || target.startsWith(`${dead}/`)
            || target.startsWith(`${dead}?`) || target.startsWith(`${dead}\${`);
          if (!hit) { continue; }
          const id = `${rel}:${target}:${dead}`;
          if (seenJumps.has(id)) { continue; }
          seenJumps.add(id);
          deadPathJumps.push({ file: rel, target, dead });
        }
      }
    }
  }
  return { missingKeys, deadPathJumps };
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name.startsWith(".")) { continue; }
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) { walk(p, out); }
    else if (/\.tsx?$/.test(name)) { out.push(p); }
  }
  return out;
}

/** 全仓 `navigate("…")` / `to="…"` / `href="…"`；模板字符串取静态前缀。 */
export function parseInlineEntries(files) {
  const entries = new Set();
  for (const file of files) {
    const src = stripComments(readFileSync(file, "utf8"));
    for (const re of [
      /navigate\(\s*"(\/[^"?#]*)"/g,
      /navigate\(\s*`(\/[^`$?#]*)`/g,
      /\bto="(\/[^"?#]*)"/g,
      /\bhref="(\/[^"?#]*)"/g,
    ]) {
      for (const m of src.matchAll(re)) { entries.add(m[1]); }
    }
  }
  return entries;
}

/** 路由模式 → 匹配正则。`:param` / `*` 都按“任意一段”处理。 */
function routeToRegex(pattern) {
  const escaped = pattern
    .replace(/\/\*$/, "")
    .split("/")
    .map((seg) => (seg.startsWith(":") || seg === "*" ? "[^/]+" : seg.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))
    .join("/");
  return new RegExp(`^${escaped}/?$`);
}

/**
 * 判定一条路由是否可达：
 *   - 入口等于路由；
 *   - 或入口是路由的更深路径（`/opc/domain/x` 对 `/opc/domain/:id`）；
 *   - 或路由是入口的更深路径（`/chat` 入口覆盖 `/chat/workflow` 吗？不 —— 反向不算）。
 */
export function isReachable(route, entries) {
  const re = routeToRegex(route);
  const base = route.replace(/\/\*$/, "");
  const parent = route.split("/").slice(0, -1).join("/") || "/";
  for (const entry of entries) {
    if (re.test(entry)) { return true; }
    if (entry.startsWith(base + "/")) { return true; }
    // 入口是路由前缀（例如入口 `/opc` 覆盖 `/opc/:tab` 的导航入口）
    if (route.startsWith(entry + "/") && !route.includes(":")) { return true; }
  }
  // 父路径有入口：`/opc/:tab` 由 `/opc` 的 tab 导航进入。
  if (entries.has(parent)) { return true; }
  return false;
}

function main() {
  const registry = parsePageRegistry(stripComments(read("src/lib/pageRegistry.ts")));
  const domainPaths = parseDomainPaths(stripComments(read("src/lib/domainMeta.ts")));
  const contentArea = stripComments(read("src/components/layout/ContentArea.tsx"));
  const routes = parseRoutes(contentArea, registry, domainPaths);

  const navEntries = parseNavEntries(stripComments(read("src/lib/navRegistry.tsx")), registry);
  const commandPalette = parseInlineEntries([join(ROOT, "src/components/layout/CommandPalette.tsx")]);
  const inline = parseInlineEntries(walk(join(ROOT, "src")));

  const entries = new Set([...navEntries, ...commandPalette, ...inline]);
  // 侧栏域分组标题可点击（`Sidebar.tsx` 的 `domainPath`），
  // 2026-10-28 起指向 `/opc/domains/:id`，因此按 id 展开。
  for (const id of domainPaths.keys()) { entries.add(`/opc/domains/${id}`); }
  // 默认首页 `"/"` 重定向到 `/chat`，视为 `/` 可达。
  entries.add("/");

  if (process.argv.includes("--debug")) {
    const navRaw = read("src/lib/navRegistry.tsx");
    const navClean = stripComments(navRaw);
    console.error("--- navClean 44..68 ---");
    console.error(navClean.split("\n").slice(44, 68).map((l, i) => `${i + 45}: ${JSON.stringify(l)}`).join("\n"));
  }

  const unreachable = [];
  const internal = [];
  const compat = [];
  const inlineOnly = [];
  const strongEntries = new Set([...navEntries, ...commandPalette, ...domainPaths.values()]);

  const compatDomainPaths = new Set(domainPaths.values());

  for (const [route, source] of routes) {
    if (INTERNAL_ROUTES.has(route)) { internal.push({ route, source }); continue; }
    if (COMPAT_ROUTES.has(route) || compatDomainPaths.has(route)) {
      compat.push({ route, source });
      continue;
    }
    if (!isReachable(route, entries)) {
      unreachable.push({ route, source });
      continue;
    }
    // 弱入口：只有页面内跳转，导航与命令面板都到不了。
    if (!isReachable(route, strongEntries) && isReachable(route, inline)) {
      inlineOnly.push({ route, source });
    }
  }

  const srcFiles = walk(join(ROOT, "src"));
  const { missingKeys, deadPathJumps } = parseRegistryAndDeadPaths(srcFiles, registry);

  const result = {
    routes: routes.size,
    entries: entries.size,
    unreachable,
    inlineOnly,
    internal,
    compat,
    missingKeys,
    deadPathJumps,
  };

  if (JSON_OUT) {
    process.stdout.write(JSON.stringify(result, null, 2) + "\n");
  } else {
    console.log(`路由 ${routes.size} 条，入口 ${entries.size} 个`);
    console.log(`内部路由（白名单）${internal.length} 条，兼容/重定向 ${compat.length} 条`);
    if (unreachable.length > 0) {
      console.log(`\n无任何入口 ${unreachable.length} 条：`);
      for (const u of unreachable) { console.log(`  ✗ ${u.route}  (${u.source})`); }
    }
    if (inlineOnly.length > 0) {
      console.log(`\n仅有页面内跳转（无导航/命令面板入口）${inlineOnly.length} 条：`);
      for (const u of inlineOnly) { console.log(`  △ ${u.route}  (${u.source})`); }
    }
    if (missingKeys.length > 0) {
      console.log(`\n引用了 pageRegistry 中已删除的键 ${missingKeys.length} 处（取到 undefined，navigate 静默不跳）：`);
      for (const m of missingKeys) { console.log(`  ✗ BUILTIN_PAGE_PATH[${m.key}]  (${m.file})`); }
    }
    if (deadPathJumps.length > 0) {
      console.log(`\n跳转到已删除的路由 ${deadPathJumps.length} 处（会落到 path="*" 的 404 页）：`);
      for (const d of deadPathJumps) { console.log(`  ✗ ${d.target}  (${d.file}，已删: ${d.dead})`); }
    }
    if (
      unreachable.length === 0 && inlineOnly.length === 0 && missingKeys.length === 0
      && deadPathJumps.length === 0
    ) {
      console.log("所有路由均有导航级入口；无已删键引用、无已删路由跳转。");
    }
  }

  if (STRICT && (unreachable.length > 0 || missingKeys.length > 0 || deadPathJumps.length > 0)) {
    process.exitCode = 1;
  }
}

/**
 * 自证：喂合成源码给上面两个解析函数，断言「坏样本被抓出、好样本零误报」。
 *
 * 为什么必须有：本门是**纯文本提取**，只能证明「已知会失效的两类写法确实报红」才算有效；
 * 一个只会打印「所有路由均有导航级入口」的脚本与没装时无从区分（判据 #7 分母为零的假阴性）。
 *
 * 合成样本写在 `.tmp/` 下（避免污染 `src/`，也不会被本门的全仓扫描二次计入）。
 * ⚠ 样本里的 `navigate(\`/x/${id}\`)` 只能整串匹配一次 —— 曾经叠了「静态前缀」规则
 * 导致同一处缺陷被报成 2 处，自证第一条就把它顶出来了。
 */
function selftest() {
  const tmp = join(ROOT, ".tmp", "route-gate-selftest");
  mkdirSync(tmp, { recursive: true });

  const registry = parsePageRegistry(stripComments(`export const BUILTIN_PAGE_PATH: Record<string, string> = {
    "finance-investment": "/invest",
    investReplay: "/invest/replay",
  };`));

  const bad = join(tmp, "bad.tsx");
  const good = join(tmp, "good.tsx");
  const commented = join(tmp, "commented.tsx");
  writeFileSync(bad, [
    "export function Bad() {",
    "  const navigate = useNavigate();",
    "  return (<>",
    '    <button onClick={() => navigate(BUILTIN_PAGE_PATH["replay-workbench"])} />',
    "    <button onClick={() => navigate(`/stock-analysis/${id}`)} />",
    '    <button onClick={() => navigate("/trade")} />',
    '    <a href="/watchlist" />',
    "  </>);",
    "}",
  ].join("\n"));
  writeFileSync(good, [
    "export function Good() {",
    "  return (<>",
    "    <button onClick={() => navigate(BUILTIN_PAGE_PATH.investReplay)} />",
    '    <button onClick={() => navigate("/invest?tab=workspace&view=analysis")} />',
    '    <a href="/trading" />',
    "    <span>旧 /stock-analysis 已删除</span>",
    "  </>);",
    "}",
  ].join("\n"));
  writeFileSync(commented, [
    '// 历史：navigate("/trade") 曾用于跳交易页，现已改为 /invest?tab=workspace&view=trade',
    "export function Commented() { return null; }",
  ].join("\n"));

  const badHits = parseRegistryAndDeadPaths([bad], registry);
  const goodHits = parseRegistryAndDeadPaths([good], registry);
  const commentedHits = parseRegistryAndDeadPaths([commented], registry);

  let failed = 0;
  const check = (name, ok) => {
    console.log(`${ok ? "PASS" : "FAIL"}  ${name}`);
    if (!ok) { failed += 1; }
  };

  check(
    "取已删键被抓出（1 处）",
    badHits.missingKeys.length === 1 && badHits.missingKeys[0].key === "replay-workbench",
  );
  check("拼已删路由被抓出（3 处：模板串 / 字面量 / href）", badHits.deadPathJumps.length === 3);
  check("坏样本共 4 处，两类不混", badHits.missingKeys.length + badHits.deadPathJumps.length === 4);
  check("正确键不误报", goodHits.missingKeys.length === 0);
  check("合法 /invest 链接不误报", goodHits.deadPathJumps.length === 0);
  check("前缀边界：/trading 不命中 /trade", !goodHits.deadPathJumps.some((d) => d.target === "/trading"));
  check(
    "注释里的历史路径不算缺陷",
    commentedHits.deadPathJumps.length === 0 && commentedHits.missingKeys.length === 0,
  );

  console.log(failed === 0 ? "\nSELFTEST OK" : `\nSELFTEST FAILED (${failed})`);
  process.exitCode = failed === 0 ? 0 : 1;
}

if (process.argv.includes("--selftest")) {
  selftest();
} else {
  main();
}
