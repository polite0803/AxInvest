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
//   node scripts/check-route-reachability.mjs --strict   # 门禁模式，有未达路由则 exit 1

import { readFileSync, readdirSync, statSync } from "node:fs";
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

  const result = {
    routes: routes.size,
    entries: entries.size,
    unreachable,
    inlineOnly,
    internal,
    compat,
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
    if (unreachable.length === 0 && inlineOnly.length === 0) {
      console.log("所有路由均有导航级入口。");
    }
  }

  if (STRICT && unreachable.length > 0) {
    process.exitCode = 1;
  }
}

main();
