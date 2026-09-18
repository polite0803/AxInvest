// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 开发者工具（工作台 `devtools` Tab）内部**子页**的唯一真相源。
 *
 * 背景：`/devtools` 下有 6 条历史子路由（trace-explorer / benchmark / ...），
 * 此前全部被压成同一次重定向 `redirectToChat("devtools")` —— 路径信息在重定向那一刻
 * 就被丢弃，用户打开 `/devtools/benchmark` 永远落在「追踪浏览器」（首个 Tab）。
 *
 * URL 契约：`/chat?ws=devtools&sub=<sub>`。
 * 消费方（禁止各自硬编码子页列表）：
 *   - ContentArea      旧路由 → URL 的重定向
 *   - DevToolsPage     antd Tabs 的受控 activeKey / onChange
 *   - workspaceTabs    切走 devtools 时清理 `sub`（避免泄漏到其它 Tab 的 URL）
 */

import { BUILTIN_PAGE_PATH } from "./pageRegistry";

/** devtools 子页 key（与 DevToolsPage 的 antd Tabs item key 一一对应） */
export type DevToolsSub =
  | "trace-explorer"
  | "benchmark"
  | "tool-recommender"
  | "fine-tune"
  | "rl-training";

export const DEVTOOLS_SUBS: readonly DevToolsSub[] = [
  "trace-explorer",
  "benchmark",
  "tool-recommender",
  "fine-tune",
  "rl-training",
];

/** 无 `sub` 参数时的回落子页（与 DevToolsPage 此前的 defaultActiveKey 一致） */
export const DEFAULT_DEVTOOLS_SUB: DevToolsSub = "trace-explorer";

/** 子页的 URL 查询参数名 */
export const DEVTOOLS_SUB_PARAM = "sub";

/**
 * 子页 → 历史路由路径。
 * 用 `Record<DevToolsSub, string>` 保证新增子页时编译器穷举报错，
 * 并集中引用 pageRegistry 常量，避免路由定义与子页 key 悄悄脱钩。
 */
export const DEVTOOLS_SUB_PATHS: Record<DevToolsSub, string> = {
  "trace-explorer": BUILTIN_PAGE_PATH.devtoolsTraceExplorer,
  "benchmark": BUILTIN_PAGE_PATH.devtoolsBenchmark,
  "tool-recommender": BUILTIN_PAGE_PATH.devtoolsToolRecommender,
  "fine-tune": BUILTIN_PAGE_PATH.devtoolsFineTune,
  "rl-training": BUILTIN_PAGE_PATH.devtoolsRlTraining,
};

/** 类型守卫：校验任意字符串是否为合法子页（用于 URL 脏值） */
export function isDevToolsSub(value: unknown): value is DevToolsSub {
  return typeof value === "string" && (DEVTOOLS_SUBS as readonly string[]).includes(value);
}

/** 从查询串解析子页；无有效值时返回 null（**不**回落到默认值，由调用方决定） */
export function parseDevToolsSub(currentSearch: string): DevToolsSub | null {
  const raw = new URLSearchParams(currentSearch).get(DEVTOOLS_SUB_PARAM);
  return isDevToolsSub(raw) ? raw : null;
}

/** 构造切换子页后的查询串：保留其它参数（含 `ws=devtools`），只改 `sub` */
export function buildDevToolsSubSearch(currentSearch: string, sub: DevToolsSub): string {
  const params = new URLSearchParams(currentSearch);
  params.set(DEVTOOLS_SUB_PARAM, sub);
  return params.toString();
}
