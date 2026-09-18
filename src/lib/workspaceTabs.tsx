// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 智能体工作台「功能 Tab」的**唯一真相源**。
 *
 * 消费方（禁止各自硬编码 Tab 列表，否则新增/删除 Tab 必漏一处）：
 *   - WorkspaceSwitcher      顶部切换栏渲染
 *   - WorkspaceHub           switch 渲染对应页面
 *   - workspaceShortcuts     Ctrl/Cmd+1..N 序号绑定 + UI 提示文案
 *   - CommandPalette         命令面板「工作台：X」命令
 *   - workspaceStore(shared) 持久化值的合法性校验
 *
 * 与侧栏（业务轴）的关系：侧栏按能力域组织**业务入口**，工作台按功能组织**工具视图**，
 * 两轴正交。**不要**把这些功能 key 补进 NAV_ITEM_DOMAIN_MAP —— 那会让 8 个业务域
 * 的语义被工具项污染（此前 `domainForNavKey` 的 general 兜底正承担这个风险）。
 *
 * URL 契约：`/chat?ws=<tab>`，参数名见 WORKSPACE_TAB_PARAM。
 * devtools Tab 另有**内部子页参数** `sub`（`/chat?ws=devtools&sub=benchmark`），
 * 定义在 lib/devtoolsSubTabs.ts —— 别在本文件重复子页列表。
 */

import { Database, Folder, FolderTree, Grid, MessageSquare, SquareTerminal, Users, Wrench } from "lucide-react";
import { DEVTOOLS_SUB_PARAM } from "./devtoolsSubTabs";

/** 工作台功能 Tab key */
export type WorkspaceTab =
  | "chat"
  | "dashboard"
  | "workflow"
  | "terminal"
  | "knowledge"
  | "files"
  | "multiAgent"
  | "devtools";

export interface WorkspaceTabMeta {
  key: WorkspaceTab;
  /** i18n key（与侧栏共用同一批 nav.* 文案） */
  labelKey: string;
  /**
   * 切走后是否**保活**（保留挂载、仅 `display:none` 隐藏，而非卸载重建）。
   *
   * 语义 = 「切走再回来，页面内状态与连接不丢」：
   *   - 终端：xterm 实例不销毁、PTY 不重连、scrollback 保留
   *   - 工作流：画布视口与编辑态保留，且**执行中的 run 不中断**
   *   - 对话：输入草稿、滚动位置、右侧面板状态保留
   *
   * ⚠ 声明为**必填**（无默认值）：新增 Tab 时编译器强制表态，避免
   * 「忘写 = 静默继承某个默认」这类隐性行为（Hub 侧判定为 `=== true`，
   * 即漏写等价于 `false`，但这里要求显式写出来）。
   *
   * ⚠ 前置条件（保活即 `display:none`，容器尺寸会变 0，任何依赖尺寸测量的
   * 页面**必须**自带 0 尺寸守卫，否则会把 0 尺寸当真值算出去）：
   *   - 终端：已给 IntegratedTerminal 的 ResizeObserver 加「容器 0 尺寸跳过 fit」守卫
   *     （否则 FitAddon 会算出 MINIMUM 2×1 并真的把 resize(2,1) 发给 PTY）
   */
  keepAlive: boolean;
}

/**
 * Tab 定义，**顺序即快捷键序号**（下标 + 1 ↔ Ctrl/Cmd+N）。
 * 门控 Tab（见 GATED_WORKSPACE_TABS）必须排在末尾，否则开关切换会让其余 Tab 的
 * 序号整体前移，导致「记住的快捷键」错位。
 *
 * ⚠ 保活页只会在**首次被访问时**挂载（见 WorkspaceHub 的 visited 集合），
 * 因此未访问过的 Tab 不占内存 —— 「全部保活」不等于「首屏挂载 8 个页面」。
 */
export const WORKSPACE_TABS: readonly WorkspaceTabMeta[] = [
  { key: "chat", labelKey: "nav.chat", keepAlive: true },
  { key: "dashboard", labelKey: "nav.dashboard", keepAlive: true },
  { key: "workflow", labelKey: "nav.workflow", keepAlive: true },
  { key: "terminal", labelKey: "nav.terminal", keepAlive: true },
  { key: "files", labelKey: "nav.files", keepAlive: true },
  { key: "knowledge", labelKey: "nav.knowledge", keepAlive: true },
  { key: "multiAgent", labelKey: "nav.multiAgent", keepAlive: true },
  { key: "devtools", labelKey: "nav.devTools", keepAlive: true },
];

export const DEFAULT_WORKSPACE_TAB: WorkspaceTab = "chat";

/** 受设置项 showDeveloperTools 门控的 Tab（必须位于 WORKSPACE_TABS 末尾） */
export const GATED_WORKSPACE_TABS: readonly WorkspaceTab[] = ["devtools"];

/** 工作台 Tab 的 URL 查询参数名（`/chat?ws=terminal`） */
export const WORKSPACE_TAB_PARAM = "ws";

/** 图标（lucide）。用 Record 保证新增 Tab 时编译器穷举报错。 */
export const WORKSPACE_TAB_ICONS: Record<WorkspaceTab, typeof MessageSquare> = {
  chat: MessageSquare,
  dashboard: Grid,
  workflow: FolderTree,
  terminal: SquareTerminal,
  files: Folder,
  knowledge: Database,
  multiAgent: Users,
  devtools: Wrench,
};

/** 图标语义色（取自 iconColors 的既有色板，保持全应用一致） */
export const WORKSPACE_TAB_ICON_COLORS: Record<WorkspaceTab, string> = {
  chat: "#3b82f6",
  dashboard: "#6366f1",
  workflow: "#8b5cf6",
  terminal: "#22c55e",
  files: "#64748b",
  knowledge: "#10b981",
  multiAgent: "#f59e0b",
  devtools: "#f97316",
};

/** 类型守卫：校验任意字符串是否为合法 Tab（用于 URL / localStorage 脏值） */
export function isWorkspaceTab(value: unknown): value is WorkspaceTab {
  return typeof value === "string" && WORKSPACE_TABS.some((tab) => tab.key === value);
}

/**
 * 该 Tab 此刻是否应渲染在 DOM 中（保活判定的**唯一实现**）。
 *
 * - 保活页（`keepAlive: true`）：**活跃中或曾访问过** ⇒ 一直渲染，切走只是隐藏
 * - 非保活页（`keepAlive: false`）：仅活跃时渲染，切走即卸载
 *
 * ⚠ `isActive` 兜底不可省：首帧的 activeTab 来自 store（上次位置），而 URL / state
 * 里的目标 Tab 要等 Hub 的解析 effect 跑完才写回 store。两者不一致时（如深链
 * `?ws=terminal` 而 store 停在工作流），若只认 `mountedTabs`，首帧会出现
 * **一个 active pane 都没有** ⇒ 空白帧闪烁。`isActive` 必然属于「要显示」的集合，
 * 加上它是安全的超集。
 *
 * ⚠ 入参是 `meta` 而不是 Tab key（内部查表）：这样才能在单测里用**自造的
 * `keepAlive: false` 元数据**直接覆盖那条分支 —— 当前 8 个 Tab 全部保活，
 * 靠真实表永远走不到「关掉保活」的路径（测试数据须自证区分力）。
 */
export function shouldRenderTab(
  meta: WorkspaceTabMeta,
  isActive: boolean,
  mountedTabs: readonly WorkspaceTab[],
): boolean {
  return meta.keepAlive ? isActive || mountedTabs.includes(meta.key) : isActive;
}

/**
 * 仅 ChatPage 消费的查询参数。
 * 切到其它 Tab 时**必须清掉**：WorkspaceHub 有「存在 chat 参数 ⇒ 强制切回对话」的逻辑，
 * 不清会让刚切走的 Tab 立刻被拽回对话（表现为「点了终端却又跳回对话」）。
 */
export const CHAT_ONLY_QUERY_PARAMS: readonly string[] = [
  "conversationId",
  "prompt",
  "workflow",
  "code",
];

/**
 * 构造切 Tab 后的查询串：清掉 chat 专属参数、写入 `ws=<tab>`，其余参数原样保留。
 * 纯函数，便于单测覆盖 URL 契约。
 */
export function buildWorkspaceTabSearch(currentSearch: string, tab: WorkspaceTab): string {
  const params = new URLSearchParams(currentSearch);
  for (const key of CHAT_ONLY_QUERY_PARAMS) {
    params.delete(key);
  }
  // `sub` 是 devtools 的内部子页参数：离开 devtools 时必须清掉。
  // 不清会留下 `/chat?ws=terminal&sub=benchmark` 这类语义悬空 URL（其它 Tab 不消费 sub），
  // 也会让「按 URL 判断当前在哪」的断言产生歧义。
  if (tab !== "devtools") {
    params.delete(DEVTOOLS_SUB_PARAM);
  }
  params.set(WORKSPACE_TAB_PARAM, tab);
  return params.toString();
}

/** 从查询串解析 Tab；无有效值时返回 null（**不要**回落到默认值，否则会覆盖持久化位置） */
export function parseWorkspaceTab(currentSearch: string): WorkspaceTab | null {
  const raw = new URLSearchParams(currentSearch).get(WORKSPACE_TAB_PARAM);
  return isWorkspaceTab(raw) ? raw : null;
}

/**
 * 剔除指定查询参数，**保留其余**（尤其 `ws`）。
 *
 * ⚠ 各页「消费完自己的 URL 参数后回写」**必须**走本函数，禁止直接
 * `setSearchParams({}, { replace: true })` —— 那是**全量清空**，会把工作台
 * Tab 参数 `ws` 一并抹掉，使「Tab 可深链 / 可刷新恢复」静默失效。
 * 实测受害点：`ChatPage`（带 conversationId/prompt/workflow 跳转时）、
 * `WorkflowPage`（带 template/domain_pack 跳转时）—— 清空后 URL 退回 `/chat`，
 * 画面仍对（store 里有值）但地址栏已不可寻址。
 *
 * 入参出参都用 `URLSearchParams`，便于直接喂给 `setSearchParams` 的函数式更新：
 * `setSearchParams((prev) => withoutParams(prev, KEYS), { replace: true })`。
 */
export function withoutParams(
  prev: URLSearchParams,
  keys: readonly string[],
): URLSearchParams {
  const next = new URLSearchParams(prev);
  for (const key of keys) {
    next.delete(key);
  }
  return next;
}
