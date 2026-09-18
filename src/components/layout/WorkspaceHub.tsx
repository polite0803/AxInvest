// SPDX-License-Identifier: AGPL-3.0-only

import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import {
  buildWorkspaceTabSearch,
  CHAT_ONLY_QUERY_PARAMS,
  isWorkspaceTab,
  parseWorkspaceTab,
  shouldRenderTab,
  WORKSPACE_TABS,
  type WorkspaceTab,
} from "@/lib/workspaceTabs";
import { ChatPage } from "@/pages/ChatPage";
import { DashboardPage } from "@/pages/DashboardPage";
import { DevToolsPage } from "@/pages/DevTools/DevToolsPage";
import { FilesPage } from "@/pages/FilesPage";
import { KnowledgeHubPage } from "@/pages/KnowledgeHubPage";
import { MultiAgentPage } from "@/pages/MultiAgentPage";
import { TerminalPage } from "@/pages/TerminalPage";
import { WorkflowPage } from "@/pages/WorkflowPage";
import { useWorkspaceTabStore } from "@/stores";
import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";

/**
 * 工作台 Hub：/chat 路由的统一渲染器。
 * 内容区根据当前功能 Tab 渲染对应页面组件。
 * 顶部的 WorkspaceSwitcher 在 App.tsx 中渲染（WorkspaceSwitcherBar）。
 *
 * Tab 来源优先级（高 → 低）：
 *   1. `location.state.tab` —— 旧路由（/dashboard 等）通过 <Navigate state={{ tab }} /> 重定向携带
 *   2. `?ws=<tab>`          —— 显式深链 / 切换栏 / 快捷键 / 命令面板写入
 *   3. store 持久化值        —— 上次工作位置（**无显式诉求时不得重置**）
 *   4. 默认 chat
 *
 * ⚠ 第 3 条是「点侧栏工作台入口不回弹到对话」的关键。凡语义为「我要对话」的跳转
 *   必须显式带 `?ws=chat`，只写 `navigate("/chat")` 会被解读为「回上次位置」。
 *
 * ── Tab 渲染策略（保活） ──
 * 逐 Tab 由 `keepAlive` 决定（见 lib/workspaceTabs 的字段说明）：
 *   - `keepAlive: true`  —— **首次被访问后常驻挂载**，非活跃时 `display: none` 隐藏。
 *     切换栏回来时页面内的状态/连接/滚动位置全在（终端不重连 PTY、工作流执行不中断）。
 *   - `keepAlive: false` —— 仅活跃时挂载，切走即卸载（等价于保活改造前的旧行为）。
 *
 * 用 `display: none` 隐藏（而非 `visibility: hidden`）：它会**让容器尺寸变成 0**，
 * 这正是依赖尺寸测量的页面必须自带 0 尺寸守卫的原因（终端已加，见 IntegratedTerminal）。
 * `visibility: hidden` 会让隐藏层继续参与布局、撑开父容器，故不采用。
 *
 * ⚠ 已知限制：Modal / Drawer 默认 portal 到 `document.body`，**不在**本 pane 的 DOM 子树内，
 *   因此隐藏一个「开着的 Modal」所在 Tab 时，该 Modal 仍会浮在界面上（切走前会卸载页面、
 *   连带清掉 portal，是保活改造后才出现的新行为）。回到该 Tab 时 Modal 状态仍在。
 */
export function WorkspaceHub() {
  const activeTab = useWorkspaceTabStore((s) => s.activeTab);
  const setActiveTab = useWorkspaceTabStore((s) => s.setActiveTab);
  const location = useLocation();
  const navigate = useNavigate();

  // 已挂载过的 Tab（保活集合）：首次访问时加入，之后只隐藏不卸载。
  //
  // 初始只放 store 现值即可 —— 若 URL / state 指向别的 Tab，下面的解析 effect 会把
  // 它写回 store，随后被本集合收录；而 `shouldRenderTab` 的 `isActive` 兜底保证
  // 首帧一定有 pane 处于活跃态（不会出现空白帧）。此处**不要**为省一次挂载而
  // 让初始值优先 URL：那会让 store 现值那个 Tab 在首帧既不渲染、又不是 active，
  // 画面直接空一帧。
  const [mountedTabs, setMountedTabs] = useState<readonly WorkspaceTab[]>(() => [activeTab]);

  // 记录已消费的重定向 state 对象引用，只消费一次。
  // 背景：redirectToChat 用 <Navigate replace state={{ tab }} /> 传目标 Tab，
  // 但 window.history.replaceState 不触发 popstate，router 内部 location.state 永不更新、
  // 残留 {tab}。若 effect 依赖 activeTab，手动切 Tab 后 effect 重跑会读到残留 state 导致回弹。
  // 用 ref 记录已消费的 state 对象引用（createLocation 对 state 透传引用、不拷贝）：
  // 残留的同一 state 引用不再消费；新导航必然产生新引用（即使 tab 值相同）也会正常消费。
  const handledStateRef = useRef<unknown>(null);

  // chat 专属查询参数：由各业务页跳转携带（OPC 行业页、投资面板等），仅 ChatPage 消费。
  // 这些参数出现时必须渲染 ChatPage——否则 activeTab 残留 workflow 等值时，
  // 跳转会落到工作流列表/编辑器，conversationId/prompt 参数被静默无视。
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const hasChatParam = CHAT_ONLY_QUERY_PARAMS.some((k) => params.has(k));
    if (hasChatParam && activeTab !== "chat") {
      setActiveTab("chat");
    }
  }, [location.search, activeTab, setActiveTab]);

  // Tab 来源解析（优先级见文件头注释）。
  // 无 `?ws=` 时保持 store 现值（不重置）—— 这就是「回工作台恢复上次位置」的实现。
  //
  // ⚠ 依赖数组**刻意不含 activeTab**：本 effect 只该在「地址变化」时把 URL 意图同步进 store。
  // 若依赖 activeTab，任何直接写 store 的操作（不经 URL）都会被 URL 旧值翻转回去，
  // 表现为「切了又被拉回」。同值 setActiveTab 是幂等的，无需比较后再设。
  useEffect(() => {
    const state = location.state as { tab?: WorkspaceTab } | null;
    if (state?.tab && state !== handledStateRef.current) {
      handledStateRef.current = state;
      if (isWorkspaceTab(state.tab)) {
        setActiveTab(state.tab);
        // ⚠ 必须把这次性重定向意图归一化进 URL：否则 URL 里的旧 `?ws=` 会在后续
        // 地址变化时把 Tab 翻转回去（实测：state.tab=workflow + ?ws=terminal 渲染成 terminal）。
        // 副作用是旧路由 /dashboard → /chat 会被写成 /chat?ws=dashboard —— 正是我们要的
        // 可深链 / 可刷新恢复。
        navigate(
          {
            pathname: BUILTIN_PAGE_PATH.chat,
            search: buildWorkspaceTabSearch(location.search, state.tab),
          },
          { replace: true },
        );
      }
      return;
    }
    const fromUrl = parseWorkspaceTab(location.search);
    if (fromUrl !== null) {
      setActiveTab(fromUrl);
    }
  }, [location.state, location.search, setActiveTab, navigate]);

  // 把当前 Tab 记入保活集合。必须放在上面两个解析 effect **之后**：
  // 它们可能在此次渲染后改写 activeTab（如从 state/URL 落到 devtools），
  // 若本 effect 先跑会先挂载旧 Tab 再挂载新 Tab，白白多挂一个页面。
  useEffect(() => {
    setMountedTabs((prev) => (prev.includes(activeTab) ? prev : [...prev, activeTab]));
  }, [activeTab]);

  return (
    <div
      className="ax-workspace-hub"
      style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
    >
      {WORKSPACE_TABS.map((meta) => {
        const { key } = meta;
        const isActive = key === activeTab;
        // 保活页：访问过就常驻；非保活页：仅活跃时挂载（切走即卸载）。
        // 判定逻辑在 lib/workspaceTabs.shouldRenderTab（唯一实现，便于单测覆盖两条分支）。
        if (!shouldRenderTab(meta, isActive, mountedTabs)) {
          return null;
        }
        return (
          <div
            key={key}
            className="ax-workspace-pane"
            data-workspace-pane={key}
            data-active={isActive ? "true" : "false"}
            // `contents` → 不产生盒子，页面元素直接成为 Hub 根的 flex item，
            // 布局与「保活改造前直接渲染该页」完全等价；`none` → 隐藏且尺寸归 0。
            style={{ display: isActive ? "contents" : "none" }}
          >
            {renderWorkspaceTab(key)}
          </div>
        );
      })}
    </div>
  );
}

/** Tab → 页面组件。与 WORKSPACE_TABS 逐项对应（新增 Tab 时 switch 会穷举报错）。 */
function renderWorkspaceTab(tab: WorkspaceTab) {
  switch (tab) {
    case "chat":
      return <ChatPage />;
    case "dashboard":
      return <DashboardPage />;
    case "workflow":
      return <WorkflowPage />;
    case "terminal":
      return <TerminalPage />;
    case "files":
      return <FilesPage />;
    case "knowledge":
      return <KnowledgeHubPage />;
    case "multiAgent":
      return <MultiAgentPage />;
    case "devtools":
      return <DevToolsPage />;
    default:
      return <ChatPage />;
  }
}
