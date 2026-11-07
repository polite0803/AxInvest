# PLAN — 智能体工作台完善建议

> 状态：active ｜ 日期：2026-09-15 ｜ 对应版本：v2.9.7


> 时点：2026-09-15 ｜ 性质：**建议 / 提案**（未施工）
> 范围：`/chat` 工作台（`WorkspaceHub` + `WorkspaceSwitcher` + `workspaceTabStore`）
> 立论前提（用户观点）：工作台把功能收进一个窗口，符合从传统软件（IDE / 浏览器）迁移的使用习惯 —— 本方案围绕**把「像」变成「是」**展开。

---

## 0. 结论摘要

工作台目前是**「单路由 + 内存态单选 Tab」**：`/chat` 是唯一真实路由，其余 7 个功能页被 `redirectToChat` 压成它的 Tab，Tab 状态只活在内存 store 里。

| 判断 | 说明 |
|---|---|
| ✅ 方向对 | 「一个窗口装多个功能视图」确实是 IDE 式迁移路径，8 个 Tab 的划分也合理 |
| ❌ 差距在**基础设施**，不在 Tab 清单 | 传统软件标签页的 9 条习惯，工作台只满足 0 条（见 §3.1） |
| ⚠ 有 3 个 P0 级可达性/信息丢失缺陷 | 不是「不够好」，是「用不通」（见 §2） |
| 🎯 最小有效改动 = 4 个文件 | `?ws=` URL 化 + persist + 快捷键 + 侧栏/命令面板入口（见 §4 阶段一） |
| 🚫 不建议**加 Tab**，也不建议**上 IDE 停靠框架** | 见 §5 反面清单 |

一句话：**先修「Tab 不可深链、不可恢复、不可达」，再谈「Tab 可关闭、可保活」—— 前者是 bug 级，后者是体验级。**

---

## 1. 现状测绘

### 1.1 架构事实（全部经源码核对）

```
ContentArea.tsx:200  <Routes>
├─ "/"                      → Navigate to DEFAULT_HOME                     (pageRegistry.ts:21 = "/chat")
├─ /dashboard /workflow /terminal /files /knowledge /multi-agent
│  /marketplace /llm-wiki /devtools ×6  → redirectToChat(tab)   ← 旧路由全被压平
│                                                                  (ContentArea.tsx:225-232, 356-361)
├─ /chat                    → SafeLazyPage(LazyWorkspaceHub)              (ContentArea.tsx:217-221)
│                             └─ switch(activeTab) 渲染 8 选 1             (WorkspaceHub.tsx:60-79)
├─ /finance /automation /devops /... (8 域) → DomainHubPage              (ContentArea.tsx:204-214)
└─ /invest /opc/industry/* …   → 业务页（AxInvest 本体）

App.tsx:387  <WorkspaceSwitcherBar />   ← 仅在 pathname ∈ {/chat, /, ""} 时渲染 (App.tsx:142-153)
App.tsx:388  <GlobalTabBar />           ← 仅 /chat **且 activeTab==="chat"** 时渲染 (App.tsx:124-137)
```

### 1.2 两条平行导航轴（这是理解一切问题的钥匙）

| 轴 | 载体 | 组织方式 | 项数 | 权威源 |
|---|---|---|---|---|
| **业务轴** | 侧栏 `Sidebar` | 按 8 个能力域分组，项=域聚合入口/行业包 | **18** | `navRegistry.tsx:48-193` + `domainMeta.ts:109-137` |
| **功能轴** | 工作台 `WorkspaceSwitcher` | 扁平工具列 | **8** | `WorkspaceSwitcher.tsx:14-23` |

**两轴零交集**（除 chat）：

- `NAV_ITEM_DOMAIN_MAP`（`domainMeta.ts:109-137`）里**没有任何**工作台功能 key（dashboard/workflow/terminal/files/knowledge/multiAgent/devtools），只有 `chat: "general"`。
- 侧栏 18 项的 `path` **没有一条**指向 `/dashboard`、`/workflow`、`/terminal`、`/files`、`/knowledge`、`/multi-agent`、`/devtools`（`navRegistry.tsx` 全量核对）。

### 1.3 对照：仓库里已有一套「成熟的 Tab」

| 维度 | 会话 Tab（`useTabStore`） | 工作台 Tab（`useWorkspaceTabStore`） |
|---|---|---|
| 定义 | `stores/shared/tabStore.ts:19-47` | `stores/shared/workspaceStore.ts:9-33` |
| 持久化 | ✅ `persist` + localStorage（`tabStore.ts:59-61`） | ❌ 无 |
| 多开/关闭 | ✅ `openTab / closeTab / closeOtherTabs / closeTabsToRight` | ❌ 固定 8 选 1 |
| 排序 | ✅ `moveTab` | ❌ |
| 失效清理 | ✅ `pruneInvalidTabs` | ❌ |
| 快捷键 | ✅ `Ctrl+Tab / Ctrl+Shift+Tab`（`useKeyboardShortcuts.ts:24-40`） | ❌ |
| 活动指示 | 有（流式/执行态经 `streamStore`/`executionStore`） | ❌ |
| Action 总数 | **11** | **1**（`setActiveTab`） |

**结论：工作台 Tab 是一个「原型级」状态容器，与会话 Tab 差一个量级。** 完善工作台 = 把会话 Tab 已验证的语义搬上去，而不是重新发明。

---

## 2. 已确认缺陷（按优先级，含证据）

### P0-1｜7 个工作台 Tab **只有唯一入口**，且该入口在离开 `/chat` 后消失

- **证据**
  - `useWorkspaceTabStore.setActiveTab` 全项目调用点仅 3 处：`WorkspaceSwitcher.tsx:62`（用户点击）、`WorkspaceHub.tsx:45`（消费 `?prompt` 等 chat 参数）、`WorkspaceHub.tsx:55`（消费 `location.state.tab`）。**无任何业务跳转能切工作台 Tab。**
  - 侧栏 18 项无一条指向这 7 个路径（§1.2）。
  - `CommandPalette.tsx:102-221` 的命令注册表里全是「去设置/去对话/新建会话/切换侧栏」，**无一条工作台 Tab 命令**。
  - `WorkspaceSwitcherBar` 被 `isChatPage` 门控（`App.tsx:142-153`）。
- **用户实际遭遇**：在「金融投研行业包」页想开终端 → 切换栏不在 → 侧栏里找不到「终端」→ 只能先点侧栏「对话」，再点顶部 Tab。
- **root cause**：功能视图（Tab）被实现为「`/chat` 路由的私有状态」，而不是「全局可寻址的视图」。
- **fix**：见 §4 阶段一（URL 化）+ 阶段三（多入口）。

### P0-2｜DevTools 的 5 条子路由全部压成同一个 Tab，**路径信息被静默丢弃**

- **证据**
  - `ContentArea.tsx:356-361`：`/devtools`、`/devtools/trace-explorer`、`/devtools/benchmark`、`/devtools/tool-recommender`、`/devtools/fine-tune`、`/devtools/rl-training` **六条路由全部** `redirectToChat("devtools")`。
  - `DevToolsPage.tsx:58` 用 `defaultActiveKey="trace-explorer"`，**全文不读 `location` / `searchParams`**（已 grep 确认零匹配）。
- **后果**：打开 `/devtools/benchmark` 永远落在「追踪浏览器」。6 个 `BUILTIN_PAGE_PATH` key（`pageRegistry.ts:76-81`）+ 6 条路由 = **零有效出口的死路径**。
- **根因归类**：**悬空解析** —— 上游解析了子路径语义，下游没有任何消费端比对。
- **fix**：`?ws=devtools&sub=<key>`，`DevToolsPage` 受控读 `sub`；或短期直接删掉这 5 条无消费者的路由（若确认无外部链接引用）。

### P0-3｜点侧栏「对话」**不会**回到对话页，而是停在上次的功能 Tab

- **证据**：`WorkspaceHub` 的两个 effect 分别只依赖 `location.search`（:41-47）和 `location.state`（:49-57），**均不依赖 `pathname`**；`activeTab` 是内存态、不随路由重置。侧栏「对话」项 `navigate(BUILTIN_PAGE_PATH.chat)`（`Sidebar.tsx` → `NavItemButton`）。
- **后果**：`activeTab === "terminal"` 时点侧栏「对话」，URL 回到 `/chat`，画面仍是终端 —— 用户会认为导航坏了。
- **说明**：该结论由代码路径推得，建议实测确认一次（预期：稳定复现）。
- **fix**：进入 `/chat` 且**无** `?ws=` / `state.tab` 时，显式重置或回落到上次持久化值；侧栏「对话」项语义应明确为「回工作台」而非「回对话」。

### P1-1｜Tab 状态不入 URL、不持久化

- **失点**：① 刷新即回 `chat`；② 浏览器前进/后退对 Tab 完全无效；③ 无法深链/书签/分享「工作台 + 终端」；④ 崩溃恢复后上下文归零。
- **证据**：`workspaceStore.ts:30-33` 裸 `create`，无 `persist`（对比 `tabStore.ts:59` 有）。
- **注**：这也让 P0-3 的「记忆上次 Tab」变得无从下手 —— 没有持久化就没有正确的回落值来源。

### P1-2｜切换 Tab 即卸载并重建，页面状态全丢

- **证据**：`WorkspaceHub.tsx:60-79` 是 `switch → return <Page />` 的**互斥渲染**，无 `keep-alive` / `display:none` 保活。
- **后果**：工作流画布缩放/未保存草稿、终端滚动区、文件浏览器路径、知识源筛选条件、仪表盘时间范围 —— 一切页面级状态在切走时蒸发。**这与「传统软件习惯」直接冲突**（VS Code 切标签不丢状态）。
- **注意**：`DevToolsPage` 内部 antd `Tabs` 用了 `destroyOnHidden`（:63），同族问题叠加一层。

### P1-3｜工作台 Tab 无快捷键

- 会话 Tab 有 `Ctrl+Tab`（`useKeyboardShortcuts.ts:26-40`），工作台 Tab 无任何绑定；`ShortcutAction`（`shortcuts.ts:6-18`）16 个 action 中零个与工作台相关。
- 传统软件习惯：`Cmd/Ctrl+1..8` 直切视图。
- **坑**：浏览器模式下 `Ctrl+1..9` 被浏览器本身占用（切浏览器标签），需按运行环境 fallback（见 §4 阶段一）。

### P1-4｜Tab 无活动/状态指示

终端有命令在跑、工作流正在执行、多智能体有活跃会话、知识源在索引 —— 顶栏一律毫无提示。IDE 的标签页活动点（dot）是这类场景的标准解。

### P2 级（顺手项）

| 编号 | 问题 | 证据 |
|---|---|---|
| P2-1 | **两个同名 store**，靠别名掩盖：`stores/shared/workspaceStore.ts`（Tab）与 `stores/feature/workspaceStore.ts`（股票工作区），`stores/index.ts:8` 把前者别名成 `useWorkspaceTabStore`，`:110` 才导出后者 | `stores/index.ts:8,110` |
| P2-2 | 调试残留 `console.warn` ×3，其中 `:59` 在**渲染路径**上（每次渲染都打） | `WorkspaceHub.tsx:51,54,59` |
| P2-3 | 死 key：`sidebar.sectionWorkspace`（「工作台」）11 个语言包全有，**代码零引用**（侧栏已改域轴） | `zh-CN.json:6579` 等 11 处 |
| P2-4 | SPDX 头拼写错误 `Identifier-Identifier`（4 处） | `WorkspaceHub.tsx:1`、`WorkflowRefPropertyPanel.tsx:1`、`SwarmPropertyPanel.tsx:1`、`StoragePropertyPanel.tsx:1` |
| P2-5 | 移动端 `overflowX: auto` 横向滚动，无折叠/抽屉 | `WorkspaceSwitcher.tsx:51` |

---

## 3. 立论回应：工作台离「传统软件习惯」还差什么

### 3.1 习惯对照表（这是完善工作的验收清单）

| # | 传统标签页习惯 | 工作台现状 | 差距类型 |
|---|---|---|---|
| 1 | 标签常驻、可见 | ✅ 已做到 | — |
| 2 | 快捷键切标签（`Cmd+1..8`） | ❌ | 体验 |
| 3 | 切换不丢状态 | ❌ 卸载重建 | **体验（高频痛点）** |
| 4 | 标签有活动指示 | ❌ | 体验 |
| 5 | 状态可恢复（重开记得） | ❌ 无持久化 | **基础设施** |
| 6 | 可深链 / 书签 / 分享 | ❌ | **基础设施** |
| 7 | 命令面板可达（`Cmd+Shift+P` 输入「终端」） | ❌ | **基础设施** |
| 8 | 标签可关闭 / 新建 / 固定 / 拖拽排序 | ❌ 固定 8 选 1 | 可扩展性 |
| 9 | 右键菜单（关闭其他 / 关闭右侧） | ❌ | 可扩展性 |

**8 条待补中，4 条属基础设施（5/6/7 + 2），它们同时是 P0-1/P0-3 的解。** 这就是为什么建议「先补基础设施，再做标签交互」。

### 3.2 一个必须想清楚的设计张力

**侧栏（业务轴，二维之一）× 工作台（功能轴，二维之二），但 URL 是一维的。**

- VS Code 的对应解：Activity Bar（左，切视图容器）× Editor Tabs（中，切文件）—— 两者都常驻、都可见。
- AxInvest 当前解：侧栏常驻；工作台 Tab **只在 `/chat` 时可见**。

因此「离开 `/chat` 就看不到工作台 Tab」在**分层语义上可以成立**（业务页 ≠ 工作台），但必须满足一个前提：**用户随时能一键回到工作台，且回来时 Tab 不丢**。当前 P0-1 + P0-3 + P1-1 三条恰好把这个前提全破坏了。

**建议明确语义**：侧栏「对话」项 → 更名/重定位为 **「工作台」**（切回 `/chat` + 恢复上次 Tab），并在其下（或侧栏底部）保持工作台 Tab 的可发现性。

---

## 4. 分阶段建议

### 阶段一（基础设施，收益最大、改动最小）—— 推荐先做

| 项 | 做法 | 涉及文件 |
|---|---|---|
| **1. Tab URL 化** | 双向绑定 `/chat?ws=<tab>`：<br>读 —— WorkspaceHub 挂载时从 `location.search` 取 `ws` 并 `setActiveTab`；<br>写 —— `setActiveTab` 同时 `navigate({ search })`。<br>保留 `state.tab` 旧机制不动（向后兼容）；`ws=chat` 可省略参数 | `WorkspaceHub.tsx`、`WorkspaceSwitcher.tsx`、`lib/pageRegistry.ts`（新增 `WORKSPACE_TAB_PARAM` 常量） |
| **2. Tab 持久化** | 给 `shared/workspaceStore` 加 `persist`（照抄 `tabStore.ts:59-61` 的 `createJSONStorage`），key 如 `axinvest-workspace-tab`。**落实 P0-3 的正确回落值**：URL 无 `ws` 时用持久化值 | `stores/shared/workspaceStore.ts` |
| **3. 快捷键** | `Ctrl/Cmd+1..8` 硬编码分支（与 `Ctrl+Tab` 同层，不走用户可配的 `SHORTCUT_ACTIONS`）。<br>**按环境分流**：Tauri 桌面用 `Cmd/Ctrl`；浏览器模式退回 `Ctrl+Alt+1..8`（避开浏览器标签快捷键） | `hooks/useKeyboardShortcuts.ts`、`WorkspaceSwitcher.tsx`（Tab 上显示快捷键提示） |
| **4. 入口补齐** | a. 侧栏「对话」→「工作台」，语义 = 回 `/chat` + 恢复上次 Tab；<br>b. 命令面板新增 8 条「工作台：{{tab}}」命令 | `navRegistry.tsx`、`CommandPalette.tsx`、11 个语言包 |
| **5. P0-3 语义修正** | 明确「点击工作台入口」与「页面内跳转 `/chat`」的不同期望：前者恢复 Tab，后者可指定 `?ws=` | `WorkspaceHub.tsx`、`Sidebar.tsx` |

**风险点**：`WorkspaceHub` 的 `state.tab` + `handledStateRef` 逻辑（:28-57）比较绕，加 `?ws=` 后存在**双写来源冲突**（`ws` / `state.tab` / 内存态）。建议定优先级：`state.tab` > `ws` > 持久化 > 默认 `chat`，并为此写测试。

### 阶段二（结束 P0-2）

- `DevToolsPage` 受控化：新增 `sub` 参数（`?ws=devtools&sub=benchmark`），`Tabs` 从 `defaultActiveKey` 改 `activeKey` + `onChange` 回写 URL。
- 5 条 devtools 子路由由 `redirectToChat("devtools")` 改为透传 `sub`。
- 若确认 5 条子路由无外部引用，**也可直接删除**（更符合「裁剪优于重构」）—— 建议先 grep 全仓 + 检查历史跳转调用方再决定。

### 阶段三（Tab 保活 + 活动指示）

- **保活**：`WorkspaceHub` 由「互斥渲染」改为「首次访问后挂载、之后 `display:none` 隐藏」。**注意内存与后台副作用**（Terminal 的 PTY、Workflow 画布的定时器/订阅、Monaco），建议按 Tab 配置 `keepAlive: boolean`，重页面默认不保活或提供设置开关。
- **活动指示**：为每个 Tab 定义活动信号（数据已存在，只需读）：
  | Tab | 信号源 |
  |---|---|
  | chat | 流式中 / 未读（`streamStore`） |
  | workflow | 有执行中的 run（`executionStore`） |
  | terminal | 有运行中命令（terminal store） |
  | multiAgent | 活跃会话数（`agentStore`） |
  | knowledge | 索引任务进行中 |
- 呈现：Tab 右上角 dot（active / error 两色）；不建议用数字 badge（噪声）。

### 阶段四（可扩展性，按需）

- Tab 配置抽成**注册表**（对齐 `pageRegistry` / `navRegistry` 的既有范式），支持 `gate`（权限/feature flag）、`order`、`keepAlive`、`badge`。
- 引入**可关闭的临时 Tab**（如「工作流编辑器 → 某流程」在 Tab 中打开），复用 `tabStore` 的 `closeTab / closeOtherTabs / moveTab` 语义。
- 插件页签（当前动态页面已移到设置/扩展分组，若将来需要，注册表可直接承接）。

---

## 5. 反面清单（不建议做的事）

1. **不要继续加 Tab。** 8 个已够；问题在基础设施不在清单长度。加 Tab 会加剧两轴重叠。
2. **不要把 7 个功能 Tab 塞进侧栏域分组。** 侧栏是「业务轴」，塞工具项会让 8 个域语义污染（`NAV_ITEM_DOMAIN_MAP` 的兜底 `general` 会变成垃圾桶）。要补入口就补**一个统一的「工作台」入口** + 命令面板。
3. **不要引入停靠/浮层框架**（rc-dock、golden-layout、flexlayout 等）。当前需求是「记住 + 可寻址 + 不丢状态」，几十行可控状态即可，架构冗余是负资产。
4. **不要把会话 Tab 与工作台 Tab 合并成一层。** 两者语义正交（会话维度 vs 功能维度），合并会重演 `CHAT_ONLY_PARAMS` 那类参数打架的 hack。应保持两层，但**共享同一套 Tab 语义实现**。
5. **不要为过 P0-3 而在 `/chat` 无条件重置 Tab。** 那会让「深链 `?ws=terminal`」失效。重置必须以「无显式 Tab 诉求」为前提。

---

## 6. 验收判据

| # | 判据 | 验证方式 |
|---|---|---|
| 1 | 刷新后停留原 Tab | 手工：切到终端 → F5 → 仍在终端 |
| 2 | 可深链 | 手工：直接打开 `/chat?ws=devtools&sub=benchmark` → 落在「基准测试」 |
| 3 | 前进/后退可切 Tab | 手工：连续切 3 个 Tab → 后退 2 次 → 回到第 1 个 |
| 4 | 快捷键生效 | 手工：`Ctrl+1..8` 逐一切换；浏览器模式下确认 fallback 组合 |
| 5 | 命令面板可达 | 手工：`Cmd/Ctrl+K` 输入「终端」→ 回车进入工作台终端 Tab |
| 6 | P0-3 已修 | 手工：在终端 Tab 点侧栏「工作台」→ 回到工作台（保持终端）；URL 显式 `?ws=chat` 时 → 回对话 |
| 7 | 回归 | `npm run typecheck` + `npm run test:run -- src/components/layout/__tests__/WorkspaceHub.test.tsx` |
| 8 | 新增测试 | ① `ws` 参数 ↔ store 双向同步；② 优先级（`state.tab` > `ws` > 持久化）；③ 快捷键分发；④ DevTools `sub` 深链（对照 §2 P0-2） |
| 9 | 门禁 | `npm run format`(dprint) + `npm run lint:eslint`(oxlint) 通过 |

**⚠ 测试必须真注入 URL**：`WorkspaceHub.test.tsx` 现有 4 个用例全部走 `MemoryRouter` + store 直改（`setActiveTab`），**没有任何用例经过 `location.search`** —— 阶段一新增的 URL 通路若照此写测试，会出现「单端单测绿、链路不通」。新增用例应以 `initialEntries={["/chat?ws=terminal"]}` 起步并在断言前**对照 URL**。

---

## 7. 改动清单汇总（供排期）

| 优先级 | 文件 | 改动 |
|---|---|---|
| P0 阶段一 | `src/stores/shared/workspaceStore.ts` | + `persist` |
| P0 阶段一 | `src/components/layout/WorkspaceHub.tsx` | + 读 `ws`；定优先级；清调试 `console.warn`（:51,54,59）；修 SPDX（:1） |
| P0 阶段一 | `src/components/layout/WorkspaceSwitcher.tsx` | + 写 URL；+ 快捷键提示；移动端折叠 |
| P0 阶段一 | `src/hooks/useKeyboardShortcuts.ts` | + `Ctrl/Cmd+1..8` 分支（含环境 fallback） |
| P0 阶段一 | `src/lib/pageRegistry.ts` | + `WORKSPACE_TAB_PARAM` 常量 |
| P0 阶段一 | `src/lib/navRegistry.tsx` | 「对话」→「工作台」措辞/语义 |
| P0 阶段一 | `src/components/layout/CommandPalette.tsx` | + 8 条工作台命令 |
| P0 阶段一 | `src/i18n/locales/*.json` ×11 | + 新 key；- 死 key `sidebar.sectionWorkspace` |
| P0 阶段二 | `src/pages/DevTools/DevToolsPage.tsx` | 受控 `activeKey` + 读 `sub` |
| P0 阶段二 | `src/components/layout/ContentArea.tsx` | devtools 子路由透传 `sub`（或删除） |
| P1 阶段三 | `WorkspaceHub.tsx` + 各页面 store | 保活 + 活动指示信号 |

**注**：SPDX 拼写（P2-4）另 3 处（`WorkflowRefPropertyPanel.tsx:1`、`SwarmPropertyPanel.tsx:1`、`StoragePropertyPanel.tsx:1`）可顺手同修。

---

## 8. 实施记录 — 阶段一（2026-09-15 完成）

### 8.1 改动清单（9 个文件）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `src/lib/workspaceTabs.tsx` **（新增）** | 8 个 Tab 的**唯一真相源**：类型、`WORKSPACE_TABS`、图标/色、`isWorkspaceTab` 守卫、`CHAT_ONLY_QUERY_PARAMS`、`buildWorkspaceTabSearch` / `parseWorkspaceTab` |
| 2 | `src/lib/workspaceShortcuts.ts` **（新增）** | 快捷键**判定↔提示共用同一逻辑**（避免"提示 Ctrl+1、按下无效"）；门控判定 `isWorkspaceTabGated` |
| 3 | `src/hooks/useWorkspaceTabNavigator.ts` **（新增）** | 切 Tab 唯一入口：**同时写 store 与 URL**，保证二者不漂移 |
| 4 | `src/stores/shared/workspaceStore.ts` | 加 `persist`（key `axinvest-workspace-tab`）+ **脏值防线** `merge`（非法持久化值回落默认） |
| 5 | `src/components/layout/WorkspaceHub.tsx` | ① 读 `?ws=` 并定义优先级；② 清理 3 处调试 `console.warn`；③ 修 SPDX 拼写 |
| 6 | `src/components/layout/WorkspaceSwitcher.tsx` | 改用 `WORKSPACE_TABS`；点击走 navigator；`title` 带快捷键；移动端 icon-only |
| 7 | `src/hooks/useKeyboardShortcuts.ts` | 新增 `Ctrl/Cmd+1..8`（浏览器端 `Ctrl+Alt+1..8`）分支，含门控联动 |
| 8 | `src/components/layout/CommandPalette.tsx` | 新增 8 条「工作台：X」命令；`go-chat` 改为显式切 chat Tab |
| 9 | `src/lib/navRegistry.tsx` + 11 个 `locales/*.json` | 侧栏首项文案改「工作台」（`nav.workbench`）；新增 `commandPalette.workbench`；**删除死 key `sidebar.sectionWorkspace`**（译文直接复用 ⇒ 零翻译成本） |

### 8.2 实施中暴露、原方案未预见的 3 个缺陷（均已修）

| # | 缺陷 | 现象（实测） | 修法 |
|---|---|---|---|
| A | **`state.tab` 与 `?ws=` 双源冲突** | `state.tab=workflow` + `?ws=terminal` 最终渲染 **terminal**：`state.tab` 只生效一次，effect 因 `activeTab` 变化重跑时走 URL 分支把它翻转 | 消费 `state.tab` 时**归一化写入 URL**（`replace`）。附带收益：旧路由 `/dashboard` → 现在落成 `/chat?ws=dashboard`，同样可深链/刷新恢复 |
| B | **业务页查询串污染工作台** | 从 `/invest?tab=workspace&view=trade` 切 Tab 会把 `tab`/`view` 带进 `/chat`，被下游误读为工作台语义 | navigator 判断来源：非工作台路由切 Tab 时**不继承**其查询串 |
| C | **effect 依赖 `activeTab` ⇒ "直改 store 被 URL 拉回"** | 2 个既有用例（store 直改切 Tab）失败：URL 里的旧 `ws` 把 Tab 拽回 | 依赖数组**移除 `activeTab`** —— 该 effect 只在"地址变化"时同步一次；同值 `setActiveTab` 幂等，无需比较 |

> C 的教训：**「URL→store」的同步 effect 一旦依赖被同步的 state，就退化成双向绑定**，
> 任何不经 URL 的写入都会被 URL 旧值覆盖。这类 effect 的依赖只能是"外部输入"（location）。

### 8.3 两处原「待裁决」已按最小惊讶原则决策

1. **侧栏首项改名「工作台」**（`navRegistry` 的 chat 项 → `labelKey: nav.workbench`）。理由：不改名就保留"点『对话』却停在终端"的困惑；改名 + 保留 `nav.chat` 供 Tab/命令复用，是零语义损失的最小改动。
2. **快捷键 fallback = 浏览器端 `Ctrl+Alt+1..8`**。理由：`Ctrl+1..9` 被浏览器占用（切浏览器标签），不 fallback 等于该环境完全不可用。

### 8.4 验证记录（全部实跑）

| 检查 | 命令 | 结果 |
|---|---|---|
| 类型检查 | `tsc --noEmit` | **EXIT=0**，日志 0 字节 |
| 工作台专项测试 | `vitest run` × 3 文件 | **25/25 passed** |
| 全量测试 | `scripts/vitest-run.mjs run` | **1731 passed / 1 failed**（152 文件） |
| ↳ 该失败项复跑 | `vitest run ScreenerToWorkspace.integration.test.tsx` | **2/2 passed（4.1s）** ⇒ 全量失败系并发资源竞争（`deadlock detected` + lazy import 20s 超时），**与本次改动无关** |
| 格式化 | `dprint check` | **EXIT=0**（曾 2 文件 import 序不符，`fmt` 后复检通过） |
| 前端 lint | `oxlint src --max-warnings=0` | 5 warnings，**全部在 `KnowledgeBaseDocuments.tsx`（既存，非本次改动文件）**；本次 9 个文件 clean |
| i18n 硬编码 | `i18n-scan.mjs --strict` | 5 项 PASS |
| i18n 未翻译 | `check-i18n-untranslated.mjs` | 0 新增 |
| i18n key 对齐 | `check_i18n.py --strict` | 11 locale 语法正常、zh-CN 无空值、代码引用 key 全有定义 |

新增测试（25 个）分布：`workspaceTabs.test.ts` 11 · `useWorkspaceTabNavigator.test.tsx` 4 · `WorkspaceHub.test.tsx` 10（原 4 + 新 6）。

### 8.5 阶段一**未覆盖**的残留（已知，勿当已修）

| 项 | 说明 | 归属 |
|---|---|---|
| 其它 `navigate("/")` 未带 `?ws=chat` | `Ctrl+F`（搜索会话）、`Ctrl+W`（关标签后）等语义为"去对话"，落在非 chat Tab 时不会切回。⚠ `Ctrl+F` 场景下 ChatPage 未渲染，`.chat-sidebar-search` 选择器必然找不到（被既有 `setTimeout 50ms` 掩盖） | ✅ `Ctrl+F` 已于**阶段二**收口（见 §9.3）；`Ctrl+W` 语义为"关会话标签"，不涉及切工作台 Tab，无需改 |
| `.sa-header-back` 返回按钮 | 4 处（Trade/Watchlist/Compare/Backtest）+ `PageHeader`，文案 `t("nav.chat")`＝"对话"，行为 `navigate("/chat")` ⇒ 会恢复上次 Tab。**改文案还是改行为属产品语义决策**（按纪律只登记不动手） | 待裁决 |
| P0-2 devtools 子路由被压平 | `/devtools/benchmark` 等 6 条仍全压成同一 Tab | ✅ 已于**阶段二**修复（见 §9） |
| 保活 / 活动指示 | 切 Tab 仍卸载重建；Tab 无活动点 | ✅ 已于**阶段三**完成（见 §10） |
| `KnowledgeBaseDocuments.tsx` 5 个 lint 警告 | `unicorn(no-useless-fallback-in-spread)`，CI 为 `--max-warnings=0` ⇒ **会让 CI 变红**。与本次无关，但需单独修 | 独立小任务 |

### 8.6 手工验收（需人在应用里确认，自动化测不到）

1. 切到终端 → F5 → 仍在终端
2. 直接打开 `/chat?ws=devtools` → 落在开发工具
3. 侧栏点「工作台」（先在终端 Tab）→ 回到工作台且**仍是终端**；命令面板 `Ctrl+K` 输「工作台: 对话」→ 切到对话
4. `Ctrl+1..8` 逐一切换；浏览器模式确认 `Ctrl+Alt+1..8`
5. 旧链接 `/dashboard` → 地址栏变成 `/chat?ws=dashboard`
6. 从 `/invest` 切 Tab → 地址为 `/chat?ws=X`（不带 `tab=workspace`）
7. **按 `Ctrl+F`**（先在终端 Tab）→ 切回对话、左侧搜索框**展开且光标在框内**

---

## 9. 实施记录 — 阶段二（2026-09-15 完成）

本阶段两件事：① 收口阶段一遗留的 `Ctrl+F` 断链；② 修 P0-2（6 条 devtools 子路由被压平）。

### 9.1 改动清单（7 个文件，1 新增）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `src/stores/shared/uiStore.ts` | + `chatSearchFocusRequest`（计数）+ `requestChatSearchFocus` / `consumeChatSearchFocus` |
| 2 | `src/hooks/useKeyboardShortcuts.ts` | `Ctrl+F` 改走 `switchWorkspaceTab("chat")` + 登记聚焦请求；**删除** `navigate("/")` 与 `setTimeout 50ms` + DOM 查询 |
| 3 | `src/components/chat/ChatSidebar.tsx` | + 消费聚焦请求（两段 effect：先补齐渲染前提 → 再聚焦并归零）；搜索框加 `ref` |
| 4 | `src/lib/devtoolsSubTabs.ts` **（新增）** | devtools 子页唯一真相源：`DevToolsSub`、`DEVTOOLS_SUBS`、`DEVTOOLS_SUB_PATHS`、`parseDevToolsSub`、`buildDevToolsSubSearch` |
| 5 | `src/pages/DevTools/DevToolsPage.tsx` | `defaultActiveKey` → **受控** `activeKey` + `onChange` 写回 URL `sub`；子页表改 `Record<DevToolsSub, …>` 穷举 |
| 6 | `src/components/layout/ContentArea.tsx` | 6 条 devtools 旧路由 → `redirectToDevTools(sub)`（产出 `/chat?ws=devtools&sub=X`） |
| 7 | `src/lib/workspaceTabs.tsx` | `buildWorkspaceTabSearch` 在**离开 devtools 时清除 `sub`** |

### 9.2 `Ctrl+F` 的真实断链是三层，原方案只看到最外层

| 层 | 代码事实 | 后果 |
|---|---|---|
| L1 | `navigate("/")` 不带 `?ws=chat` | 停在终端/工作流时按 Ctrl+F **画面不动**（WorkspaceHub 按"无显式诉求"保持上次 Tab） |
| L2 | 搜索框渲染条件是 `!showArchived && searchVisible && !multiSelectMode`，而 `searchVisible` **默认 `false`** | 默认状态下 DOM 里**根本没有** `.chat-sidebar-search input` |
| L3 | `document.querySelector(...)?.focus()` + 固定 `setTimeout 50ms` | 失败被 `?.` **静默吞掉**；即便展开过搜索框，非对话 Tab 时 `ChatSidebar` 未挂载，仍然拿不到元素 |

**修法**：`Ctrl+F` → ① `switchWorkspaceTab("chat")`（唯一入口，同时写 store + URL）；② 往 `uiStore` 登记一个**计数式请求**；③ `ChatSidebar` 挂载后自取并消费。

**为什么不用 `CustomEvent`**（仓内已有 `axagent:escape` 先例）：派发时刻 `ChatSidebar` 常常**尚未挂载**（ChatPage 是切 Tab 后才渲染的），一次性事件必然丢。放进 store 后「挂载时自取」与「已挂载时响应」同一套代码路径。
**为什么用「消费归零」而非 `useRef` 记已消费值**：`ChatSidebar` 在 Tab 间切换会反复卸载，`ref` 随卸载重置 ⇒ 同一个旧请求会在每次重挂载时重复抢焦点。

### 9.3 `sub` 为什么不能走 `location.state`

阶段一让 `WorkspaceHub` 把 `state.tab` 归一化进 URL（`replace`），但那段逻辑**只认 `tab`**。若把 `sub` 一并塞进 `state`，会在归一化那一刻被丢掉——**与 P0-2 原来的失败方式一模一样**。因此 `redirectToDevTools` **不经 state**，直接产出 `/chat?ws=devtools&sub=<sub>`。
副作用是 `redirectToChat` 与 `redirectToDevTools` 两套并存：前者有 15 个调用点（dashboard/workflow/terminal/files/knowledge/multiAgent/marketplace/llmWiki 等旧路由），本阶段**刻意不动**以控制回归面。

### 9.4 施工事故：同文件并发 `Edit` 会被覆盖（两种形态，第二种更隐蔽）

把「同一文件的多个 `Edit`」放进**同一批次**发送时，每处编辑各自基于**旧快照**写入，**后者覆盖前者**。本次撞到两次：

| 形态 | 现场 | 捕获方式 |
|---|---|---|
| **A. 改动完全消失** | `workspaceTabs.tsx`（改函数体 + 加 import）、`useKeyboardShortcuts.ts`（改 `case "f"` + 加 import）→ 函数体被覆盖，只留下 import | `tsc` 报 `TS6133 'useXxx' is declared but its value is never read` —— **「新增 import 却没人用」正是「函数体没改上」的指纹** |
| **B. 只剩一半（更隐蔽）** | `ChatSidebar.tsx` 一次发了三处：加 `InputRef` import、加 `searchInputRef` + 两段 effect、给 `<Input>` 加 `ref={searchInputRef}` → **只有第三处丢了** | ⚠ `tsc` **抓不到**：`searchInputRef` 被声明**且被读**（effect 里 `.current`）⇒ 无 `TS6133`；但它**从未被写入**（没挂到 DOM）⇒ 运行时恒 `null` ⇒ 搜索框会展开、**永不聚焦**、请求永不消费。靠**逐文件 grep 复核**才发现 |

**判据（形态 B 的通用检法）**：
> **「被读」不等于「被接上」。** 新增一个 ref / handler / 回调后，必须 grep 它的**写入端**
> （`ref={x}`、`onX={x}`、`x(...)` 的实际调用点），而不只是确认声明存在。
> 只 grep 声明与读取，会漏掉「声明了、读了、但没人喂数据」这一类 —— 类型系统对它完全沉默。

**纪律**：同文件 `Edit` 必须**串行**；每组改动收尾时对**全部改动文件**做一次 grep 复核实际内容（本次若只跑 `tsc` + 测试，形态 B 会带着「Ctrl+F 不聚焦」上线）。

> **形态 B 已补修**：`ref={searchInputRef}` 已挂到 `<Input>`（`ChatSidebar.tsx:2130`），
> 补修后重跑了全套门禁 —— 见 §9.5「补 `ref` 前后各跑一次」。

### 9.5 验证记录（全部实跑）

| 检查 | 命令 | 结果 |
|---|---|---|
| 类型检查 | `tsc --noEmit` | **EXIT=0**，日志 0 字节（补 `ref` 前后各跑一次，均干净） |
| 本次相关测试 | 7 个文件（含 3 新增） | **65/65 passed** |
| ↳ 其中 `DevToolsPage.test.tsx` | 5 用例 | ✅ `?sub=benchmark` 现在落在「基准测试」（此前必落首个 Tab） |
| ↳ 其中 `useKeyboardShortcuts.test.tsx` | 3 用例 | ✅ Ctrl+F 同时改 Tab 与登记请求；输入框内不触发 |
| 全量测试 | `scripts/vitest-run.mjs run`（跑了 **2 次**：补 `ref` 前 / 后各一次） | 两次均 **1753 passed / 2 failed**（155 文件），且**两次失败的是不同用例**（① `McpServerSettings` 5s + `DemandDiscoveryPage` 15s；② `DemandDiscoveryPage` 两个用例 15s）⇒ 抖动特征，非稳定失败 |
| ↳ 失败项单独复跑 | `vitest run DemandDiscoveryPage.test.tsx` | **11/11 passed（29.1s）** ⇒ 均为全量并发 CPU 争抢的 **timeout**，**与本次改动无关**。⚠ 该文件是慢测试：11 用例 29s，单用例 5.2–6.3s，本身贴 15s 上限 |
| 格式化 | `dprint fmt` + `dprint check` | **EXIT=0**（改动后复跑，fmt 本次改动 4 文件） |
| 前端 lint | `oxlint src` | 5 warnings / **0 errors**，**仍全在 `KnowledgeBaseDocuments.tsx`（既存）**；本次 7 个文件 clean |

新增测试 23 个：`devtoolsSubTabs.test.ts` 10 · `DevToolsPage.test.tsx` 5 · `useKeyboardShortcuts.test.tsx` 3 · `uiStore.test.ts` +3 · `workspaceTabs.test.ts` +2。

### 9.6 一处**没能自动化**的闭环（手工验收，别当已测）

`uiStore 请求 → ChatSidebar 展开搜索框并聚焦` 这一段**无法在 vitest 里跑**：`ChatSidebar` 的依赖树会拉到 `@emoji-mart/data/sets/15/native.json`，该包被 vitest **外部化**（走 node 原生 ESM），报
`needs an import attribute of "type: json"`。实测 `vi.mock` 对该模块**无效**（mock 拦不到被外部化的模块），修它要动 `vite.config.ts` 的 `test.server.deps.inline`，属全局配置改动，超出本阶段范围。

⇒ 因此 §8.6 的第 7 条（按 Ctrl+F 的实际观感）**必须人工确认**。
⇒ 逻辑侧已由 3 个测试锁住（请求登记 / 累加 / 消费归零），UI 侧是机械接线；若人工验证发现搜索框没展开，先查 `ChatSidebar` 那两段 effect 的依赖数组。

### 9.7 阶段二**未覆盖**的残留

| 项 | 说明 | 归属 |
|---|---|---|
| devtools 子页缺「直达入口」 | 子页现在可深链、可刷新恢复，但**命令面板仍只有 1 条「工作台：开发工具」**（落到上次子页）。若希望 `Ctrl+K` 直达「基准测试」，需补 5 条命令 | 可选，未做 |
| `redirectToChat` / `redirectToDevTools` 两套并存 | 前者 15 个调用点仍走 `location.state`。统一成 URL 直写是更干净的方向，但会牵动 15 条路由 + `WorkspaceHub` 的 state 分支与其测试 | 可延后 |
| 保活 / 活动指示 | 切 Tab 仍卸载重建；Tab 无活动点 | ✅ 已于**阶段三**完成（见 §10） |

---

## 附：待裁决（产品语义，需用户拍板）

> 第 1、4 项已在阶段一按最小惊讶原则决策并实施（见 §8.3），保留原文以便回溯。

1. **侧栏「对话」是否改名「工作台」？** 涉及 28 处历史认知与 11 语言包，语义变更需明确。
2. **Tab 是否要做「可关闭」？** 若做，需要回答「关掉后从哪重新打开」（注册表 + 固定项）。
3. ~~**保活范围**~~ —— 已在**阶段三**决策为「8 个 Tab 全部保活」，并保留 `keepAlive` 逐 Tab 开关（见 §10.2）。
4. **快捷键 fallback 策略**：浏览器模式用 `Ctrl+Alt+1..8`，还是不提供？

---

## 10. 实施记录 — 阶段三（2026-09-15 完成）

本阶段三件事：① 修一个**阶段一引入的 URL 契约回归**；② Tab 保活；③ Tab 活动指示。
外加一个**保活前置条件**：终端 xterm 的 0 尺寸守卫（不先修它，保活会把 PTY 尺寸搞坏）。

### 10.1 前置修复：`setSearchParams({})` 会抹掉 `ws`（阶段一回归）

阶段一把 Tab 状态搬进 URL（`/chat?ws=<tab>`）后，仓内既有的**两处全量清空**变成了隐患：

| 位置 | 触发条件 | 后果 |
|---|---|---|
| `ChatPage.tsx:70,86` | URL 带 `conversationId` / `prompt` / `workflow`（业务页「打开会话」跳转） | 清空后地址退回 `/chat`，`ws` 丢失 |
| `WorkflowPage.tsx:50` | URL 带 `template` / `industry` | 同上 |

画面**仍是对的**（Hub 的 store 分支不重置），但地址栏不再可寻址 —— 「Tab 可深链 / 可刷新恢复」静默失效。
（两处都有「无相关参数则提前 return」的守卫，所以**只在带这些参数跳转时**才踩，这也是它没在阶段一被发现的原因。）

**修法**：`workspaceTabs.tsx` 新增 `withoutParams(prev, keys)`（纯函数，保留其余参数），两处改为
`setSearchParams((prev) => withoutParams(prev, KEYS), { replace: true })`。

### 10.2 Tab 保活

**判定逻辑**（`lib/workspaceTabs.tsx` 的 `shouldRenderTab`，单一实现）：

```
保活页（keepAlive: true） ：isActive || mountedTabs.includes(tab)  ⇒ 常驻挂载，隐藏而非卸载
非保活页（keepAlive: false）：isActive                              ⇒ 仅活跃时挂载（旧行为）
```

- `mountedTabs` 是 Hub 内的「已访问集合」：**首次访问才挂载** ⇒ 「全部保活」≠「首屏挂 8 个页面」。
- 隐藏用 `display: none`，非隐藏态用 `display: contents` —— 后者不产生盒子，**页面元素直接成为 Hub 的 flex item**，
  与「改造前直接渲染该页」布局完全等价（父容器 `ContentArea.tsx:214-216` 本就是 flex column）。
- `keepAlive` 在注册表里是**必填**字段：新增 Tab 时编译器强制表态，避免「忘写 = 静默继承默认」。
- 不用 `visibility: hidden`：它会让隐藏层继续参与布局、撑开父容器。

**⚠ 为什么必须先修终端的 0 尺寸守卫（本阶段最有价值的一次取证）**

读 `node_modules/@xterm/addon-fit/src/FitAddon.ts` 源码确认：

```ts
public proposeDimensions() {
  ...
  if (dims.css.cell.width === 0 || dims.css.cell.height === 0) return undefined; // 只守卫「字符尺寸」
  const parentElementWidth = Math.max(0, parseInt(parentElementStyle.getPropertyValue('width')));
  const availableWidth = parentElementWidth - elementPaddingHor - scrollbarWidth;
  cols: Math.max(MINIMUM_COLS, Math.floor(availableWidth / dims.css.cell.width)),  // ← 容器 0 尺寸时兜成 2×1
  rows: Math.max(MINIMUM_ROWS, Math.floor(availableHeight / dims.css.cell.height)),
}
```

**容器宽高为 0 时它不返回 undefined，而是把 cols/rows 兜到 `MINIMUM_*`(2×1) 并成功执行 `terminal.resize(2, 1)`**，
再经 `xterm.onResize` → `resizeSession` 把 `resize(2,1)` 发给后端 PTY ⇒ shell 按 2 列重排、输出错乱。

⇒ `IntegratedTerminal` 的 `ResizeObserver` 回调加守卫：容器 `offsetWidth/offsetHeight === 0` 时跳过 `fit()`
（重新可见时 ResizeObserver 会因尺寸变化再触发一次正常 fit）。这是保活终端的**前置条件**，同时修掉一个既存潜伏缺陷
（窗口最小化 / 容器折叠时同样会误发 resize）。

**⚠ 施工中撞出的缺陷：首帧「无 active pane」空白帧**

`mountedTabs` 初值一度写成「URL 目标 Tab 优先」，但首帧 `activeTab` 仍是 store 现值 ⇒ 两者不同时，那个 pane
既不在 `mountedTabs` 里、又不是 active ⇒ **一个 active pane 都没有，画面空一帧**。
修法：`shouldRenderTab` 保活分支加 `isActive` 兜底（`isActive` 必然属于「要显示」的集合，是安全超集），
`mountedTabs` 初值回归最简单的 `[activeTab]`。

### 10.3 Tab 活动指示

信号判据（写进 `useWorkspaceTabActivity.ts` 文件头，防将来凭直觉"补齐 5 个"）：
**① 状态在全局 store（不依赖该页是否挂载）② 会自行归零 ③ 语义是「有正在进行、会结束的工作」而非「有东西存在」。**

| Tab | 信号 | 来源 |
|---|---|---|
| chat | `streaming` | `useStreamStore` |
| workflow | `isExecuting` | `useWorkflowStore` |
| multiAgent | `delegating` | `useMultiAgentStore` |
| devtools | `status === "running"` | `useRlTrainingStore` |
| terminal / knowledge / files / dashboard | **无信号，刻意不配** | 见下 |

原方案（§3 阶段三）列的 5 个信号里，**有 2 个在代码里不成立**，已按实测修正：

- **terminal**：唯一候选是 `sessions[].status === "running"`，但 PTY 会话一旦创建就长期 running（直到 exited）
  ⇒ **恒亮**，是噪声不是信号。终端 store 里**没有**「命令执行中」这一维状态。
- **knowledge**：`useKnowledgeSourceStore` 只有 `loading`（全局唯一、瞬时的请求态），**没有**索引/抓取任务的进行中状态。

呈现：小圆点只在**非当前** Tab 上出现（当前 Tab 用户正看着，提示多余），且活动文案并入按钮
`aria-label` / `title`（不能只用一个色点传达状态）。**只做「进行中」一色**，不做「出错」色 ——
错误需要「已读」语义（看过就该熄灭），那要额外维护已读状态，否则退化成常亮噪声。

### 10.4 改动清单（9 个文件，2 新增）

| 文件 | 改动 |
|---|---|
| `src/lib/workspaceTabs.tsx` | 新增 `keepAlive` 字段（必填）、`shouldRenderTab`、`withoutParams` |
| `src/hooks/useWorkspaceTabActivity.ts` | **新增**：活动信号（含判据与反例说明） |
| `src/components/layout/WorkspaceHub.tsx` | 互斥渲染 → 保活渲染（`mountedTabs` + `display: contents/none`）；抽出 `renderWorkspaceTab` |
| `src/components/layout/WorkspaceSwitcher.tsx` | 活动点 + 无障碍 label |
| `src/components/terminal/IntegratedTerminal.tsx` | ResizeObserver 回调加「容器 0 尺寸跳过 fit」守卫 |
| `src/pages/ChatPage.tsx` | 两处 `setSearchParams({})` → `withoutParams(prev, CHAT_ONLY_QUERY_PARAMS)` |
| `src/pages/WorkflowPage.tsx` | 同上（清 `template` / `industry`） |
| 11 个 locale JSON | 新增 `nav.tabActivity`，译文**复用各包自己的 `chat.running`**（零翻译成本） |
| `src/components/layout/__tests__/WorkspaceHub.test.tsx` | 判据从「在不在文档里」改为「哪个 pane 是 active」；+4 保活用例 |
| `src/lib/__tests__/workspaceTabs.test.ts` | +8（`withoutParams` 3 · `shouldRenderTab` 5） |
| `src/hooks/__tests__/useWorkspaceTabActivity.test.ts` | **新增**：7 个信号接线用例 |
| `src/components/layout/__tests__/WorkspaceSwitcher.test.tsx` | **新增**：7 个活动点渲染用例 |

### 10.5 一处**必然过时**的测试判据（值得记住）

保活改造让 4 个既有断言失效：`expect(screen.queryByTestId("page-workflow")).not.toBeInTheDocument()`。
它们在保活后**永远失败**（切走的页面仍在 DOM 里），但**不能**简单删掉 —— 删掉就丢掉了「不回弹」这个回归防线。

⇒ 换成有同等区分力的判据：`activePane()`（`[data-workspace-pane][data-active="true"]` 的 key）。
这也是本仓的一个通用教训：**判据必须锚定被测对象自身形态**（这里是「哪个 pane 活跃」），
而不是借一个**恰好等价但会随实现漂移**的旁证（「元素在不在 DOM 里」）。

### 10.6 已知限制（保活引入，非 bug 但需知晓）

**Modal / Drawer 默认 portal 到 `document.body`，不在 pane 的 DOM 子树内。**
保活改造前，切走 Tab 会**卸载页面**、连带清掉 portal；改造后页面常驻，于是一个「开着的 Modal」在切走后
**仍会浮在界面上**（回到该 Tab 时其状态仍在）。缓解方向（未做）：切 Tab 时通知各页关闭自己的浮层，
或统一个 `getContainer` 指向 pane。属阶段四范畴。

### 10.7 验证记录（全部实跑）

| 检查 | 命令 | 结果 |
|---|---|---|
| 类型检查 | `tsc --noEmit` | **EXIT=0**，日志 0 字节（改动前后各跑一次） |
| 本次相关测试 | 8 个文件（含 3 新增） | **81/81 passed** |
| ↳ 其中 `WorkspaceHub.test.tsx` | 14 用例 | 10 → 14（+4 保活：隐藏而非卸载 / 同一 DOM 节点 / 未访问不进 DOM / 有且只有一个 active） |
| ↳ 其中 `WorkspaceSwitcher.test.tsx` | 7 用例 | 含「活跃 Tab 不显示点」「无信号 Tab 一个点都没有」 |
| ↳ 其中 `useWorkspaceTabActivity.test.ts` | 7 用例 | 逐一验证 4 个信号接到**正确的 store 字段**，并验证归零 |
| 全量测试 | `scripts/vitest-run.mjs run` | **1781 passed / 0 failed**（157 文件，71.8s）—— 本轮**首次零失败**（阶段一/二的全量都有 1~2 个并发 timeout 抖动） |
| 格式化 | `dprint fmt` + `check` | **EXIT=0**（fmt 本次 3 文件） |
| 前端 lint | `oxlint src` | 5 warnings / **0 errors**，**仍全在 `KnowledgeBaseDocuments.tsx`（既存）**；本次文件 clean |
| i18n 硬编码 / 未翻译 / key 对齐 | 三个脚本 | 全部通过（`nav.tabActivity` 11 语言齐备） |

### 10.8 阶段三**未覆盖**的残留

| 项 | 说明 | 归属 |
|---|---|---|
| 活动点缺 4 个 Tab | terminal / knowledge / files / dashboard **没有可用的「进行中」状态**（详见 §10.3）。要补就得先在后端/ store 里造出这一维状态，不是前端能补的 | 需产品+后端 |
| 活动点无「出错」色 | 需要「已读」语义（用户看过即熄灭），要有持久化已读状态 | 阶段四 |
| Modal portal 浮层 | 见 §10.6 | 阶段四 |
| 保活开关无 UI | `keepAlive` 目前只能改注册表；计划中的「设置面板开关」未做（避免又一次 11 语言 + 设置页改动） | 阶段四 |
| `redirectToChat` / `redirectToDevTools` 两套并存 | 阶段二遗留 | 未做 |
| `KnowledgeBaseDocuments.tsx` 5 个 lint 警告 | CI 是 `--max-warnings=0` ⇒ **会让 CI 变红**（既存，与工作台无关） | 独立小任务 |

### 10.9 手工验收（需人在应用里确认，自动化测不到）

自动化测不到的部分集中在一处：**`display: contents` 的真实布局效果**（jsdom 无布局引擎）。
请逐项确认：

1. 切到终端 → 回对话 → 再切终端：**终端内容/滚动位置还在**（保活生效的最直观表现）
2. 终端在**后台**（切到别的 Tab）时敲命令不会有异常输出错乱 ⇒ 验证 0 尺寸守卫真的拦住了 `resize(2,1)`
3. 工作流画布：编辑中切走再回来，**节点位置与视口未塌陷**
4. 各页面内容区**高度/滚动正常**（`display: contents` 若在某些浏览器失效，会表现为页面高度塌陷或内容被裁）
5. 对话中输入草稿 → 切走 → 切回：**草稿还在**
6. 活动点：在对话里发一条消息并立刻切到别的 Tab ⇒ 对话 Tab 右上角出现圆点，流式结束后**熄灭**
7. 开着设置弹窗 → 切 Tab：确认 §10.6 描述的现象是否符合预期（Modal 会浮着）

