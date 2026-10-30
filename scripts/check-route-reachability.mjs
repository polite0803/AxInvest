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
 *
 * ⚠ **第二次同族复发（2026-10-30）**：上面的「调换顺序」只解决了**注释里**出现 `/*`
 * 的形态，管不住**模板串里**出现的 `/*`。实测 `ContentArea.tsx` 的
 * `` path={`${BUILTIN_PAGE_PATH.settings}/*`} `` —— splat 通配 `/*` 落在模板串内，
 * 块注释正则把它配到文件后面最近的「块注释结束符」，**吞掉其间全部内容**：
 *
 * | 症状 | 实测 |
 * |---|---|
 * | `BUILTIN_PAGE_PATH.settings` 的模板串被截断（闭合反引号被吃） | 该 Route 解析不出 |
 * | `BUILTIN_PAGE_PATH.llmWiki` 整段落在被吞区间内 | 那 4 条 llm-wiki 路由全部未被解析 |
 * | ⇒ `parseRoutes` **静默少收集 5 条路由** | 它们既不被查可达性、也不会报孤儿 |
 *
 * 代价：`--strict` 曾把 `/settings`、`/llm-wiki` 误报成孤儿键（真因是根本没解析到路由），
 * 同时 `/llm-wiki/:wikiId/ingest` 这个**真缺陷**因为从未进 `routes` 而**隐身**。
 *
 * ⇒ 判据：**注释剥离必须感知字符串 / 模板串字面量**，否则「注释语法出现在字面量里」
 * 这一类会持续复发（与判据 #206「扫描面 ≠ 影响面」同族：解析器的盲区会被当成事实）。
 *
 * 实现：逐字符扫描并跟踪状态；非注释区间**原样复制**，注释字符替换为空格以保住行号。
 */
export function stripComments(src) {
  let out = "";
  let state = "code"; // code | line | block | dq | sq | tpl
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    const next = src[i + 1];
    if (state === "code") {
      if (c === "/" && next === "/") { state = "line"; out += "  "; i++; continue; }
      if (c === "/" && next === "*") { state = "block"; out += "  "; i++; continue; }
      if (c === '"') { state = "dq"; }
      else if (c === "'") { state = "sq"; }
      else if (c === "`") { state = "tpl"; }
      out += c;
      continue;
    }
    if (state === "line") {
      if (c === "\n") { state = "code"; out += c; } else { out += " "; }
      continue;
    }
    if (state === "block") {
      if (c === "*" && next === "/") { state = "code"; out += "  "; i++; continue; }
      out += c === "\n" ? "\n" : " ";
      continue;
    }
    // 字面量内：转义序列整体放过，遇到同类引号收尾。
    // 模板串刻意**不做 `${}` 嵌套解析** —— 把它整段当字面量更安全：
    // 模板串里的 `//` 与 `/*` 本来就只是文本，不该被当注释。
    if (c === "\\") { out += c + (next ?? ""); i++; continue; }
    if ((state === "dq" && c === '"') || (state === "sq" && c === "'") || (state === "tpl" && c === "`")) {
      state = "code";
    }
    out += c;
  }
  return out;
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
    // 「纯重定向路由」：渲染结果就是一个 `<Navigate>`。
    //
    // 这类路由**不是目的地**，它存在的唯一理由是接住旧链接 —— 要求它有入口等于
    // 要求「有人链接到一条重定向」，语义上就错了（`COMPAT_ROUTES` 的注释亦是此意）。
    // 2026-10-30 收紧父路径规则后，它们不再被 `"/"` 顺带覆盖，会暴露出来。
    //
    // ⚠ 这里**不静默豁免**：main() 把它们单列进 `redirects` 桶并打印。
    // 剔除「重定向无需入口」是判据的语义修正；把它藏起来才是削判据。
    const isRedirect = /element=\{<\s*Navigate\b/.test(attrs);
    const tag = isRedirect ? "redirect:" : "";
    const literal = attrs.match(/\bpath="([^"]+)"/);
    if (literal) {
      routes.set(literal[1], tag + "literal");
      continue;
    }
    const template = attrs.match(
      /\bpath=\{`\$\{BUILTIN_PAGE_PATH(?:\.([A-Za-z0-9_]+)|\["([^"]+)"\])\}([^`]*)`\}/,
    );
    if (template) {
      const key = template[1] ?? template[2];
      const base = registry.get(key);
      if (base) { routes.set(base + template[3], tag + "template:" + key); }
      continue;
    }
    const bracket = attrs.match(/\bpath=\{BUILTIN_PAGE_PATH\["([^"]+)"\]\}/);
    if (bracket) {
      const base = registry.get(bracket[1]);
      if (base) { routes.set(base, tag + "registry[" + bracket[1] + "]"); }
      continue;
    }
    const member = attrs.match(/\bpath=\{BUILTIN_PAGE_PATH\.([A-Za-z0-9_]+)\}/);
    if (member) {
      const base = registry.get(member[1]);
      if (base) { routes.set(base, tag + "registry:" + member[1]); }
      continue;
    }
    if (/\bpath=\{domain\.path\}/.test(attrs)) {
      for (const [id, p] of domainPaths) { routes.set(p, tag + "domain:" + id); }
      continue;
    }
    if (/\bpath=\{BUILTIN_PAGE_PATH\[pageKey\]\}/.test(attrs)) {
      for (const key of domainRouteKeys) {
        const base = registry.get(key);
        if (base) { routes.set(base, tag + "domainRoutes:" + key); }
      }
      continue;
    }
  }
  return routes;
}

/**
 * 找出 `pageRegistry` 里有键、但 `ContentArea` 里**没有对应 `<Route>`** 的孤儿键
 * （2026-10-30 立）。
 *
 * 为什么必须是门禁：本门的正向判据是「路由 → 入口」，它**看不见反向**——
 * 一个键若根本没有注册成路由，就永远不会出现在 `routes` 里，于是既不会被判
 * 「不可达」也不会出现在任何报告里，**静默隐身**。调用方 `navigate(BUILTIN_PAGE_PATH.wiki)`
 * 会跳到 `path="*"` 的 404 页，而 `typecheck` 全绿（`Record<string, string>` 的键
 * 存在性不受 TS 保护）。
 *
 * 实证（2026-10-30）：`src/lib/pageRegistry.ts:73` 的 `wiki: "/wiki"` 在
 * `ContentArea.tsx` 中零 `<Route>` 命中，唯一引用是 `e2e/wiki.spec.ts:5` 的
 * `page.goto("/wiki")` ⇒ 该 e2e 断言的是一个必 404 的地址。
 *
 * 豁免来源：`ORPHAN_REGISTRY_KEYS`（必须写明理由，禁止为过门禁而扩表）。
 */
export function findOrphanRegistryKeys(registry, routes, whitelist) {
  const known = [...routes.keys()];
  const orphans = [];
  for (const [key, path] of registry) {
    if (whitelist.has(key)) { continue; }
    // 该 path 被某条已注册路由**消费**的判据：把路由模式当正则去匹配这个具体路径。
    // ⚠ 不能用朴素的 `r === path || r.startsWith(path + "/")` —— 参数化路由会漏判：
    // `/chat/workflow` 是被 `/chat/:tab` 消费的，但两者字符串前缀关系不成立，
    // 朴素判据会把它误报成孤儿（实测：会一口气误报 `/chat/*` 全部 6 个键）。
    const consumed = known.some((r) => routeToRegex(r).test(path));
    if (!consumed) { orphans.push({ key, path }); }
  }
  return orphans;
}

/** 孤儿键豁免表：仅登记「有意保留、不注册为路由」的键。 */
const ORPHAN_REGISTRY_KEYS = new Map([
  // 例：{ key: "xxx", reason: "..." }
]);

/**
 * 解析 `devtoolsSubTabs.ts` 的 `DEVTOOLS_SUB_PATHS: Record<DevToolsSub, string> = { "x": BUILTIN_PAGE_PATH.y }`。
 *
 * 为什么必须单独解析：`ContentArea` 里这 5 条路由写成
 * `{DEVTOOLS_SUBS.map((sub) => <Route path={DEVTOOLS_SUB_PATHS[sub]} .../>)}`，
 * `parseRoutes` 的 5 条正则**一条都匹配不上**（既不是字面量，也不是 `BUILTIN_PAGE_PATH.x`）。
 * 于是这 5 条路由对门禁**完全不可见**：正向「路由→入口」漏查（它们从不出现在
 * `routes` 里），反向孤儿键检查则会把它们误报成「有键无路由」的假阳性。
 *
 * 返回「路径 → 子页 key」。
 */
export function parseDevtoolsSubPaths(src, registry) {
  const block = src.match(/DEVTOOLS_SUB_PATHS[^=]*=\s*\{([\s\S]*?)\};/);
  if (!block) { return new Map(); }
  const out = new Map();
  const re = /"([A-Za-z0-9_-]+)"\s*:\s*BUILTIN_PAGE_PATH(?:\.([A-Za-z0-9_]+)|\["([^"]+)"\])/g;
  for (const m of block[1].matchAll(re)) {
    const sub = m[1];
    const key = m[2] ?? m[3];
    const path = registry.get(key);
    if (path) { out.set(path, sub); }
  }
  return out;
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

/** 测试文件 / 测试目录：**不得**充当路由入口的证明。 */
const TEST_DIR_RE = /(^|[\\/])__tests__([\\/]|$)/;
const TEST_FILE_RE = /\.(test|spec)\.[jt]sx?$/;

function isTestPath(p) {
  return TEST_DIR_RE.test(p) || TEST_FILE_RE.test(p);
}

/**
 * 递归收集 `.ts(x)` 文件。
 *
 * `includeTests: false` 用于**入口抽取**：测试里写一句 `navigate("/x")` 只是断言
 * 或夹具，不代表用户能点到 `/x`。2026-10-30 前 `src` 全量扫描把 `__tests__`
 * 也算作入口 ⇒ 一个「无任何导航入口」的页面只要被单测随手 `navigate` 过一次，
 * 门禁就绿了（假阴性）。**死路径扫描**仍保留测试（测试引用已删路由本身就是缺陷）。
 */
function walk(dir, out = [], options = {}) {
  const { includeTests = true } = options;
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name.startsWith(".")) { continue; }
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) { walk(p, out, options); }
    else if (/\.tsx?$/.test(name)) {
      if (!includeTests && isTestPath(p)) { continue; }
      out.push(p);
    }
  }
  return out;
}

/**
 * 全仓 `navigate("…")` / `to="…"` / `href="…"` 入口。
 *
 * ⚠ 模板字符串必须转成**模式**而不是截断成静态前缀（2026-10-30 修）：
 * 旧实现用 `[^`$?#]*` 在 `$` 处截断，`` navigate(`/llm-wiki/${id}/graph`) ``
 * 只留下 `/llm-wiki/`。这在旧判据下无害（反正整棵子树都被父路径吞掉），
 * 但在收紧后会让 `/llm-wiki/:wikiId/graph` 这类**真有按钮入口**的路由误报不可达 ——
 * 把「真实入口存在」判成「没有入口」，是比漏报更难排查的假阳性。
 *
 * 故把每个 `${...}` 归一成 `:__`（= 任意一段），使模板串成为与路由同构的模式：
 * `` `/llm-wiki/${id}/graph` `` → `/llm-wiki/:__/graph`，可被
 * `routeToRegex("/llm-wiki/:wikiId/graph")` 正常命中。
 */
export function parseInlineEntries(files) {
  const entries = new Set();
  for (const file of files) {
    const src = stripComments(readFileSync(file, "utf8"));
    for (const re of [
      /navigate\(\s*"(\/[^"?#]*)"/g,
      /\bto="(\/[^"?#]*)"/g,
      /\bhref="(\/[^"?#]*)"/g,
    ]) {
      for (const m of src.matchAll(re)) { entries.add(m[1]); }
    }
    // 模板串：整串取出后把插值段换成 `:__`。
    for (const m of src.matchAll(/navigate\(\s*`([^`?#]*)`/g)) {
      const pattern = m[1].replace(/\$\{[^}]*\}/g, ":__");
      if (pattern.startsWith("/")) { entries.add(pattern); }
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
 * 祖先入口能否覆盖该路由：**仅当路由相对入口的剩余段全部是参数段**。
 *
 * 设计意图（`PLAN-frontend-information-architecture.md` §4.1）：一条参数化路由
 * （`/opc/:tab`）与它的父页（`/opc`）是**同一页面的不同 Tab**，父入口点进去后由
 * 页内 tab 导航到达子级 ⇒ 父入口可覆盖它。
 *
 * 反例（2026-10-30 实测坐实的缺陷）：`/chat` 侧栏入口曾把 `/chat/workflow`
 * 判成可达 —— 但两者是**不同页面**（WorkspaceHub vs WorkflowPage），`/chat` 入口
 * 并不能把用户带到工作流页。旧判据只要求「路由不含 `:`」，于是父路径吞掉了整棵
 * 子树；`/chat` 又是侧栏首项 ⇒ 所有 `/chat/*` 恒绿，门禁证明不了它要证明的事。
 *
 * 故判据收紧为「剩余段全参数才放行」：字面量段（`workflow` / `memory`）代表
 * 一个**独立目的地**，必须有它自己的入口。
 */
function paramsCoverRoute(route, entry) {
  const base = entry.replace(/\/\*$/, "");
  if (!route.startsWith(`${base}/`)) { return false; }
  const rest = route.slice(base.length + 1);
  if (!rest) { return false; }
  return rest
    .split("/")
    .filter(Boolean)
    .every((seg) => seg.startsWith(":") || seg === "*");
}

/**
 * 判定一条路由是否可达：
 *   - 入口等于路由（参数段按任意一段匹配）；
 *   - 或入口是路由的更深路径（`/opc/domain/finance-invest` 对 `/opc/domain/:id`）；
 *   - 或**参数化**的祖先入口（`/opc` 覆盖 `/opc/:tab`，见 `paramsCoverRoute`）。
 *
 * ⚠ 这里刻意**不**做「字面量祖先覆盖」（`/chat` → `/chat/workflow`）。
 * 历史上那条规则（`route.startsWith(entry + "/") && !route.includes(":")`）
 * 让父路径吞掉任意深度子树，是本门禁失效的根因。
 *
 * ⚠ 同时**删除**了「父路径有入口」的单段捷径：旧实现 `parent =
 * route.split("/").slice(0, -1).join("/") || "/"` 对单段路由回退成 `"/"`，
 * 而 `"/"` 又是无条件入口 ⇒ 任何顶级路由（`/llm-wiki`、`/learning-graph`、
 * `/dynamic-ui` …）都白拿一个入口。现在走同一条 `paramsCoverRoute` 判据。
 */
export function isReachable(route, entries) {
  const re = routeToRegex(route);
  const base = route.replace(/\/\*$/, "");
  for (const entry of entries) {
    if (re.test(entry)) { return true; }
    if (entry.startsWith(`${base}/`)) { return true; }
    if (paramsCoverRoute(route, entry)) { return true; }
  }
  return false;
}

function main() {
  const registry = parsePageRegistry(stripComments(read("src/lib/pageRegistry.ts")));
  const domainPaths = parseDomainPaths(stripComments(read("src/lib/domainMeta.ts")));
  const contentArea = stripComments(read("src/components/layout/ContentArea.tsx"));
  const routes = parseRoutes(contentArea, registry, domainPaths);
  // `.map()` 展开的 devtools 子路由：`parseRoutes` 抓不到。
  //
  // ⚠ 刻意**只**把它们当作「孤儿键检查的消费证据」，**不**加进 `routes`：
  // 加进去会让这 5 条**历史上从未被门禁检查过**的路由首次进入正向检查，
  // 从而在本次收紧之外产生一批新红，把「收紧是否生效」的信号搅浑。
  // 它们确实是门禁的盲区（已单独登记为待裁决项），但那是**另一个**判据变更。
  const devtoolsSubPaths = parseDevtoolsSubPaths(
    stripComments(read("src/lib/devtoolsSubTabs.ts")),
    registry,
  );

  const navEntries = parseNavEntries(stripComments(read("src/lib/navRegistry.tsx")), registry);
  const commandPalette = parseInlineEntries([join(ROOT, "src/components/layout/CommandPalette.tsx")]);
  // ⚠ 入口抽取**排除测试**（includeTests: false）：测试里的 `navigate("/x")` 不是用户入口。
  const inline = parseInlineEntries(walk(join(ROOT, "src"), [], { includeTests: false }));

  const entries = new Set([...navEntries, ...commandPalette, ...inline]);
  // 侧栏域分组标题可点击（`Sidebar.tsx` 的 `domainPath`），
  // 2026-10-28 起指向 `/opc/domains/:id`，因此按 id 展开。
  for (const id of domainPaths.keys()) { entries.add(`/opc/domains/${id}`); }
  // 默认首页 `"/"` 重定向到 `/chat`，视为 `/` 可达。
  // ⚠ 注意：`"/"` 只是**它自己**的入口，不再顺带覆盖任何路由 —— 旧实现靠
  // 「单段路由的父路径回退成 `"/"`」把这个入口放大成了「所有顶级路由白拿入口」。
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
  const redirects = [];
  const inlineOnly = [];
  const strongEntries = new Set([...navEntries, ...commandPalette, ...domainPaths.values()]);

  const compatDomainPaths = new Set(domainPaths.values());

  for (const [route, source] of routes) {
    if (INTERNAL_ROUTES.has(route)) { internal.push({ route, source }); continue; }
    if (COMPAT_ROUTES.has(route) || compatDomainPaths.has(route)) {
      compat.push({ route, source });
      continue;
    }
    // 纯重定向路由（`element={<Navigate .../>}`）：不是目的地，不要求入口。
    // 单列成 `redirects` 桶并打印 —— 语义修正，不是静默豁免。
    if (source.startsWith("redirect:")) {
      redirects.push({ route, source: source.slice("redirect:".length) });
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

  // ⚠ 死路径扫描**保留测试**（includeTests 默认 true）：
  // 测试引用已删路由本身就是缺陷（它断言一个必 404 的地址），与「入口」语义相反。
  const srcFiles = walk(join(ROOT, "src"));
  const { missingKeys, deadPathJumps } = parseRegistryAndDeadPaths(srcFiles, registry);

  // 孤儿键检查的「已知路由」= 正向检查用的路由 + devtools 子路由（后者仅此处参与，
  // 见 main() 顶部注释：不让它们进入正向检查以免搅浑本次收紧的信号）。
  const routesForOrphan = new Map(routes);
  for (const [path] of devtoolsSubPaths) {
    if (!routesForOrphan.has(path)) { routesForOrphan.set(path, "devtoolsSub"); }
  }

  const orphanKeys = findOrphanRegistryKeys(
    registry,
    routesForOrphan,
    new Set(ORPHAN_REGISTRY_KEYS.keys()),
  );

  const result = {
    routes: routes.size,
    entries: entries.size,
    unreachable,
    inlineOnly,
    internal,
    compat,
    redirects,
    missingKeys,
    deadPathJumps,
    orphanKeys,
  };

  if (JSON_OUT) {
    process.stdout.write(JSON.stringify(result, null, 2) + "\n");
  } else {
    console.log(`路由 ${routes.size} 条，入口 ${entries.size} 个`);
    console.log(
      `内部路由（白名单）${internal.length} 条，兼容/重定向 ${compat.length} 条，`
        + `纯重定向路由 ${redirects.length} 条`,
    );
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
    if (orphanKeys.length > 0) {
      console.log(`\npageRegistry 有键但无对应 <Route> 的孤儿键 ${orphanKeys.length} 处（navigate 会落 404）：`);
      for (const o of orphanKeys) { console.log(`  ✗ ${o.key}: "${o.path}"`); }
    }
    if (
      unreachable.length === 0 && inlineOnly.length === 0 && missingKeys.length === 0
      && deadPathJumps.length === 0 && orphanKeys.length === 0
    ) {
      console.log("所有路由均有导航级入口；无已删键引用、无已删路由跳转、无孤儿注册键。");
    }
  }

  if (
    STRICT
    && (unreachable.length > 0 || missingKeys.length > 0 || deadPathJumps.length > 0
      || orphanKeys.length > 0)
  ) {
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

  // ── 核心判定自证（2026-10-30 补）──────────────────────────────────────────
  // 此前 `parseRoutes` / `parseNavEntries` / `isReachable` **一条都没被测**：
  // 上面 7 条只覆盖 `parseRegistryAndDeadPaths`，于是「父路径吞掉整棵子树」这个
  // 让本门禁**完全失效**的缺陷，在自证全绿的情况下存活了整整一个版本
  // （Lead 2026-10-30 实测：`isReachable('/chat/workflow', {'/chat'})` → true）。
  //
  // ⚠ 这一组的「反例」断言的是**修复后行为**：它们在修复前必然 FAIL。
  // 这正是自证的意义 —— 自证通过与否，本身就是判据是否生效的证据。

  // 反例：字面量祖先不得覆盖子路由（`/chat` 入口带不到 `/chat/workflow` 页）。
  check(
    "反例：/chat/workflow 不该被 /chat 入口覆盖",
    isReachable("/chat/workflow", new Set(["/chat"])) === false,
  );
  check(
    "反例：/invest/screener 不该被 /invest 入口覆盖",
    isReachable("/invest/screener", new Set(["/invest"])) === false,
  );
  check(
    "反例：/knowledge/memory 不该被 /knowledge 入口覆盖",
    isReachable("/knowledge/memory", new Set(["/knowledge"])) === false,
  );
  // 反例：更深的多段字面量同样不得被覆盖（防「只查一段」的假修复）。
  check(
    "反例：/chat/workflow/new 不该被 /chat 入口覆盖",
    isReachable("/chat/workflow/new", new Set(["/chat"])) === false,
  );

  // 正例：参数段路由仍须被父入口覆盖 —— 这是 `:305` 注释写明的**设计意图**，
  // `/opc` 点进去后用页内 tab 导航到 `/opc/:tab`。收紧不得把它一起干掉。
  check(
    "正例：/opc/:tab 仍由 /opc 入口覆盖（设计意图）",
    isReachable("/opc/:tab", new Set(["/opc"])) === true,
  );
  // ⚠ 这条**不能**写成「/opc 覆盖 /opc/domains/:domainId」：
  // `domains` 是字面量段，按新判据 `/opc` 不该覆盖它。该路由的真实入口是
  // `main()` 里按 domain id 展开的 `/opc/domains/<id>`（Sidebar 域标题可点）。
  check(
    "正例：/opc/domains/:domainId 由具体域 id 入口覆盖（而非 /opc）",
    isReachable("/opc/domains/:domainId", new Set(["/opc/domains/finance"])) === true,
  );
  check(
    "反例：/opc/domains/:domainId 不该被裸 /opc 覆盖（有字面量段 domains）",
    isReachable("/opc/domains/:domainId", new Set(["/opc"])) === false,
  );
  // 正例：精确命中（参数段按任意一段匹配）。
  check(
    "正例：入口精确等于路由",
    isReachable("/settings/*", new Set(["/settings"])) === true,
  );
  check(
    "正例：参数化入口可被字面量路径命中",
    isReachable("/opc/domain/:packId", new Set(["/opc/domain/finance-invest"])) === true,
  );

  // 反例：空入口集一律不可达（检测方法本身必须有效，否则前面全是假绿）。
  check("反例：空入口集 ⇒ 不可达", isReachable("/chat/workflow", new Set()) === false);
  check("反例：空入口集 ⇒ /opc/:tab 也不可达", isReachable("/opc/:tab", new Set()) === false);

  // 反例：`"/"` 只能覆盖它自己，不得顺带覆盖任何顶级路由
  //（旧实现 `parent = ... || "/"` + 无条件 `entries.add("/")` 让所有顶级路由白拿入口）。
  check(
    "反例：/llm-wiki 不该被 / 入口覆盖",
    isReachable("/llm-wiki", new Set(["/"])) === false,
  );

  // ── 路由抽取自证：`parseRoutes` 此前零覆盖 ──
  const routeSrc = stripComments([
    "const DOMAIN_ROUTES = [",
    '  ["finance-investment", LazyInvest],',
    "];",
    "export function R() {",
    "  return (<Routes>",
    '    <Route path="/opc/domains/:domainId" element={<X />} />',
    "    <Route path={BUILTIN_PAGE_PATH.chat} element={<X />} />",
    "    <Route path={`${BUILTIN_PAGE_PATH.investReplay}/x`} element={<X />} />",
    '    <Route path="/legacy" element={<Navigate to="/new" replace />} />',
    "    {DOMAIN_ROUTES.map(([pageKey, Page]) => (",
    "      <Route key={pageKey} path={BUILTIN_PAGE_PATH[pageKey]} element={<Page />} />",
    "    ))}",
    "    <Route path=\"*\" element={<NotFound />} />",
    "  </Routes>);",
    "}",
  ].join("\n"));
  const routeRegistry = parsePageRegistry(stripComments(`export const BUILTIN_PAGE_PATH: Record<string, string> = {
    "finance-investment": "/invest",
    investReplay: "/invest/replay",
    chat: "/chat",
  };`));
  const parsed = parseRoutes(routeSrc, routeRegistry, new Map());
  check("抽路由：字面量 path 被抓出", parsed.has("/opc/domains/:domainId"));
  check("抽路由：`BUILTIN_PAGE_PATH.chat` 成员引用被抓出", parsed.has("/chat"));
  check("抽路由：模板串拼接被抓出", parsed.has("/invest/replay/x"));
  check("抽路由：DOMAIN_ROUTES 展开被抓出", parsed.has("/invest"));
  check("抽路由：`*` 兜底被抓出", parsed.has("*"));
  // 重定向路由必须被标记（main() 靠 `redirect:` 前缀把它们从「必须有入口」里分出）。
  check(
    "抽路由：重定向路由被打上 redirect: 标记",
    String(parsed.get("/legacy") ?? "").startsWith("redirect:"),
  );
  check(
    "抽路由：非重定向路由**不**带 redirect: 标记",
    !String(parsed.get("/chat") ?? "").startsWith("redirect:"),
  );

  // ── 入口抽取自证：`parseNavEntries` 此前零覆盖 ──
  const navSrc = stripComments([
    "export const items = [",
    '  { key: "a", path: BUILTIN_PAGE_PATH.chat },',
    '  { key: "b", path: BUILTIN_PAGE_PATH["investReplay"] },',
    '  { key: "c", path: "/hardcoded" },',
    "];",
  ].join("\n"));
  const navParsed = parseNavEntries(navSrc, routeRegistry);
  check("抽入口：成员引用被抓出", navParsed.has("/chat"));
  check("抽入口：方括号引用被抓出", navParsed.has("/invest/replay"));
  check("抽入口：硬编码字面量**不**被当作 registry 入口", !navParsed.has("/hardcoded"));

  // ── 孤儿键自证：`findOrphanRegistryKeys` 是新增判据，必须正负对照 ──
  const orphanRegistry = new Map([
    ["live", "/live"],
    ["liveChild", "/live/child"],
    ["dead", "/dead"],
  ]);
  const orphanRoutes = new Map([["/live", "literal"], ["/live/child", "literal"]]);
  const orphans = findOrphanRegistryKeys(orphanRegistry, orphanRoutes, new Set());
  check(
    "孤儿键：有键无路由被抓出（仅 dead）",
    orphans.length === 1 && orphans[0].key === "dead",
  );
  check(
    "孤儿键：有路由的键不误报",
    !orphans.some((o) => o.key === "live" || o.key === "liveChild"),
  );
  check(
    "孤儿键：豁免表生效",
    findOrphanRegistryKeys(orphanRegistry, orphanRoutes, new Set(["dead"])).length === 0,
  );

  // ── 入口抽取必须排除测试文件（否则测试里的 navigate 就能"造出"入口）──
  check("测试路径识别：__tests__ 目录", isTestPath(join(ROOT, "src/lib/__tests__/x.tsx")));
  check("测试路径识别：*.test.tsx", isTestPath(join(ROOT, "src/a/b.test.tsx")));
  check("测试路径识别：*.spec.ts", isTestPath(join(ROOT, "src/a/b.spec.ts")));
  check("测试路径识别：普通源码不算测试", !isTestPath(join(ROOT, "src/a/b.tsx")));

  // ── 注释剥离必须感知字面量（2026-10-30 第二次同族复发）────────────────────
  // 这组断言在修复前**必然 FAIL**。⚠ 样本末尾**必须**带一个块注释：
  // 旧实现的正则 `/\*[\s\S]*?\*\//` 是非贪婪「配到下一个块注释结束符」，
  // 若源码里后面**没有**结束符，它就匹配不到、什么都不做 —— 那样样本复现不出缺陷，
  // 断言在两种实现下都 PASS（= 无效断言）。本组断言已用负对照验证过区分力。
  const splatSrc = [
    "export function Splat() {",
    "  return (<>",
    "    <Route path={`${BUILTIN_PAGE_PATH.settings}/*`} element={<A />} />",
    '    <Route path={BUILTIN_PAGE_PATH.llmWiki} element={<Navigate to="/knowledge/wiki" replace />} />',
    "  </>);",
    "}",
    "/* 末尾块注释：提供旧实现误吞所需的结束符 */",
  ].join("\n");
  const splatRegistry = parsePageRegistry(stripComments(`export const BUILTIN_PAGE_PATH: Record<string, string> = {
    settings: "/settings",
    llmWiki: "/llm-wiki",
  };`));
  const splatStripped = stripComments(splatSrc);
  const splatRoutes = parseRoutes(splatStripped, splatRegistry, new Map());

  check("注释剥离感知模板串：splat 的 /* 不被当块注释", splatStripped.includes("/*"));
  check(
    "注释剥离感知模板串：其后的路由代码不被吞掉",
    splatStripped.includes("BUILTIN_PAGE_PATH.llmWiki"),
  );
  check("parseRoutes 收得到带 splat 后缀的模板路由", splatRoutes.has("/settings/*"));
  check("parseRoutes 收得到其后的模板路由（不被吞）", splatRoutes.has("/llm-wiki"));
  check(
    "孤儿键：splat 路由对应的键不误报",
    findOrphanRegistryKeys(splatRegistry, splatRoutes, new Set()).length === 0,
  );

  console.log(failed === 0 ? "\nSELFTEST OK" : `\nSELFTEST FAILED (${failed})`);
  process.exitCode = failed === 0 ? 0 : 1;
}

if (process.argv.includes("--selftest")) {
  selftest();
} else {
  main();
}
