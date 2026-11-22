# PLAN — URL 作为真相（路由 / 深链收敛）

> 状态：active ｜ 日期：2026-10-27 ｜ 对应版本：v2.9.9


> 关联：`PLAN-agent-workbench-enhancement.md`（工作台 Tab 基础设施已落地，本文只处理其 §10.8 遗留的「两套 redirect 并存」与「Tab 状态未完全由 URL 表达」）。
> 范围：仅前端路由层。不动 Rust、不动业务组件内部结构、不动 `src-tauri/`。

## 0. 结论摘要

当前 `/chat` 的 `?ws=` **是半真相**：有它就用它，没有它就用 localStorage。会话身份（`conversationId`）**完全不进 URL**，只在挂载时被读一次随即抹掉。于是「刷新 / 后退 / 复制链接」三种行为分别依赖三种不同机制，用户无法建立空间记忆。

本计划的目标不是「把所有东西塞进 URL」，而是**让每个 URL 参数有唯一 owner**，并让缺省值被显式补齐。分 4 个阶段，每阶段可独立上线、可独立回滚：

| 阶段 | 内容 | 风险 | 前置 |
|---|---|---|---|
| 1 | `?ws=` 缺省补齐（URL 归一化） | 低 | 无 |
| 2a | 会话深链读侧（后退/外链能切会话） | 低 | 阶段 1 |
| 2b | 会话深链写侧（当前会话镜像进 URL） | 中 | 2a |
| 3 | 页面内 Tab 参数统一（`?tab=` 收口） | 低 | 无 |
| 4 | 旧路由重定向表声明化 + 修 `window.location` 读法 | 低 | 无 |

阶段 1、3、4 互不依赖，可并行施工；2a/2b 必须串行。

---

## 1. 事实基线（逐条经源码核对）

### 1.1 `?ws=` 缺席时 URL 不描述当前视图

- `WorkspaceHub` 只在 `parseWorkspaceTab` 返回非 null 时写 store（`src/components/layout/WorkspaceHub.tsx:113-116`）；返回 null 时**保留 store 现值**。
- 持久化在 `localStorage`（`src/stores/shared/workspaceStore.ts:30-51`，key `axinvest-workspace-tab`）。
- 反例（可稳定复现）：上次停在终端 → 打开 `/chat`（无参数）→ 画面是终端，地址栏是 `/chat`。此时**刷新能恢复**靠的是 localStorage，不是 URL；把该地址发给别人，对方看到的是他自己的上次位置。

### 1.2 会话身份不进 URL

- `ChatPage` 只在**挂载时执行一次**读取 `conversationId`（`src/pages/ChatPage.tsx:31-92`，`urlInitDoneRef` + 空依赖数组 `:92`）。
- 读完后立即从 URL 抹掉（`:71` 与 `:87`，走 `withoutParams(prev, CHAT_ONLY_QUERY_PARAMS)`，该名单含 `conversationId`，见 `src/lib/workspaceTabs.tsx:144-149`）。
- 全仓**没有任何**代码把当前会话写回 URL（检索 `set("conversationId")` 零命中）。切换会话只改 store。

### 1.3 会话深链在当前实现下不可后退

对话 pane 是保活常驻的（`src/lib/workspaceTabs.tsx:68`，`keepAlive: true`），配合 1.2 的 mount-only 逻辑：从 `/chat?conversationId=A` 导航到 `/chat?conversationId=B` 时 `ChatPage` **不会重新挂载**，于是 effect 不重跑，会话仍是 A。

这是 2a 的直接依据，也是「前进/后退行为取决于落在哪一侧」的技术成因之一。

### 1.4 三套 Tab 命名空间并存

| 语义 | 参数 | 消费方 |
|---|---|---|
| 工作台功能 Tab | `?ws=` | `src/lib/workspaceTabs.tsx:84` |
| 投资中心业务 Tab | `?tab=` | `src/components/invest/InvestHub.tsx:98,108` |
| 能力包页业务 Tab | `?tab=` | `src/components/opc/CapabilityPackHub.tsx:67,77` |
| 任务中心 Tab | `?tab=` | `src/pages/TasksCenterPage/TasksCenterPage.tsx:59,68` |
| OPC 管理面板 | `/opc/:tab` 路径段 | `src/components/layout/ContentArea.tsx:305` |

`?tab=` 在三个文件里各写了一遍「读—校验—写」逻辑，且 `/opc/:tab` 又是第四种表达。

### 1.5 旧路由重定向是 JSX 内联代码

- `redirectToChat`（`src/components/layout/ContentArea.tsx:140-143`，2026-10-28 已删除、仅留墓碑）删除前用 **`window.location.search`** ——非响应式读取，且与同类 `redirectToDevTools`（`:148-154`）形态不一致，同一件事两套写法。
- 约 20 条兼容路由散落在 `:272-279`、`:420-449`，无法单测、无法统一加「来路」标记。
- 该问题已被 `PLAN-agent-workbench-enhancement.md` §10.8 登记为未做项。

### 1.6 设置分区不入 URL

`CommandPalette` 的 6 条设置命令全部是 `navigate("/settings")` 后调 `setSettingsSection(...)`（`src/components/layout/CommandPalette.tsx:138-226`）。用户无法把「设置 → MCP」发成链接，刷新后也回不到该分区。

---

## 2. 目标契约

原则：**一个 URL 参数只有一个 owner；缺省值必须被显式补齐。**

| URL | 渲染 | owner | 说明 |
|---|---|---|---|
| `/chat?ws=<tab>` | 工作台对应 Tab | WorkspaceHub | `ws` 永远存在；缺失时首帧后 `replace` 补齐 |
| `/chat?ws=chat&conversationId=<id>` | 该会话 | ChatPage | 会话深链；`ws!=chat` 时该参数被清掉 |
| `/invest?tab=<key>&stockCode=&view=` | 投资中心对应 Tab | InvestHub | 页面内参数，不改公开形态 |
| `/chat?conversationId=&prompt=&workflow=` | 一次性指令 | ChatPage | 消费后清除，不可作为稳定链接 |

配套约束（写进代码注释，防回归）：

- `ws` 与 `conversationId` **不得互相删除**。现有 `buildWorkspaceTabSearch` 会清 `conversationId`（切走 chat 时正确），因此**归一化不能复用它**，需另配一个只写 `ws` 的纯函数。
- 所有写入走 `setSearchParams(prev => ...)` 函数式更新。禁止用 `navigate({search: 构造串})` 从 `location.search` 反推——那会覆盖另一个 owner 刚写入的键（见 §3.1 施工注意）。

---

## 3. 分阶段方案

### 阶段 1：`?ws=` 缺省补齐

**改什么**

1. `src/lib/workspaceTabs.tsx` 新增纯函数：

   ```ts
   /** 只写 ws，保留其余参数（尤其 conversationId）。与 buildWorkspaceTabSearch 的区别：
    *  后者用于「用户主动切 Tab」，需要清理 chat 专属参数；本函数用于「URL 归一化」。 */
   export function withWorkspaceTab(currentSearch: string, tab: WorkspaceTab): string
   ```

2. `src/components/layout/WorkspaceHub.tsx`：在 Tab 解析 effect（`:93-117`）末尾，若 `parseWorkspaceTab(location.search) === null`，则以 `setSearchParams(prev => withWorkspaceTab(prev, activeTabFromStore), { replace: true })` 补齐。

**为什么这么切**

只补写、不改解析优先级，store 从「优先级 3」降级为「冷启动默认值」，行为在缺省路径上完全不变，只是地址栏开始说真话。

**施工注意**

- 必须用 `setSearchParams` 函数式更新。`ChatPage` 同一时刻也在 `replace` 掉 `conversationId`（`:71/:87`），若两边都从 `location.search` 构造完整串，会互相覆盖，出现「归一化把 conversationId 抹掉」或「ChatPage 把 ws 抹掉」。
- 补齐动作要幂等：`parseWorkspaceTab` 已是 null 才写，写完 effect 重跑时自然短路。
- 不要把这个 effect 加到 `location.pathname` 依赖里，否则离开 `/chat` 会触发无意义写入。

**验收**

- 裸访问 `/chat`（持久化值为 terminal）→ 地址栏变为 `/chat?ws=terminal`，画面不变。
- `/chat?conversationId=X`（无 ws）→ 变为 `/chat?ws=chat&conversationId=X`（或该参数已被消费，则只剩 `ws`），**不得**出现两参数互相删除。
- `/chat?ws=garbage` → 回落到持久化值并改写地址。

**测试**：`src/lib/__tests__/workspaceTabs.test.ts` 增 `withWorkspaceTab` 4 条；`src/components/layout/__tests__/WorkspaceHub.test.tsx` 增 2 条（补齐 / 不重复写）。

---

### 阶段 2a：会话深链读侧

**改什么**

`src/pages/ChatPage.tsx:33-92` 的初始化 effect：

- 由「mount-only」改为「监听 `conversationId` 变化」，以 `conversationId` 为主键做 diff：URL 值 ≠ store 值 → `setActiveConversation(urlId)`。
- `prompt` / `workflow` 仍保持**一次性**语义：用 `consumedKeysRef: Set<string>` 记录已消费的 `<workflow|prompt>` 组合，避免后退时重复建会话、重复发送。
- `urlInitDoneRef` 只继续用于 `prompt/workflow` 的首次闸门，不再拦截 `conversationId`。

**为什么这么切**

这是唯一能在「不重挂载」前提下让后退生效的改法。先只做读侧，写侧不动，失败面被限制在「URL 想切会话」这一种输入。

**风险与回滚**

- 风险：URL 与 store 不一致时会出现一帧抖动（先渲染旧会话再切）。回滚 = 恢复 `urlInitDoneRef` 语义，单点改动。
- 风险：若外部仍存在 `/chat?conversationId=X` 的一次性跳转，改造后它会变成「可持续的深链」——这是期望行为，但需确认 `WorkflowPage` 等跳转方不会因参数留存而重复触发（`consumedKeysRef` 兜住）。

**验收**

- `/chat?ws=chat&conversationId=A` → `/chat?ws=chat&conversationId=B` → 按后退 → 回到 A。
- 带 `prompt` 的链接后退时**不得**重复发送消息。

---

### 阶段 2b：会话深链写侧

**改什么**

`src/pages/ChatPage.tsx`：`activeConversationId` 变化 → 仅当 `ws=chat` 时，把 `conversationId` 写入 URL（`replace`）；为 null 时删除该键。

**约束（必须写进注释）**

URL 是 `activeConversationId` 的**镜像**，不参与 tab↔conversation 判定。现有 `syncingRef`（`:222`、`:252-301`）继续只管 `TabStore ↔ ConversationStore` 两个方向，URL 方向只由这一条 effect 单向写出。

否则三方互相同步会退化成需要第三个 `syncingRef` 状态，正是当前 `DecisionHeroBar` 一类组件失控的同源问题。

**风险与回滚**

- 风险：每次切会话多一次 `history.replace`，在 `Ctrl+Tab` 高频切换时可能有感。回滚 = 删掉写侧 effect，读侧仍可用（2a 独立生效）。
- 风险：新建会话时 id 异步生成，若在 id 到达前写 URL 会写入空值。实现上用 `if (!id) return;`。

**验收**

- A → B：地址栏从 `conversationId=A` 变 B；刷新停在 B。
- 复制该地址到新窗口 → 落到同一会话（依赖 2a）。
- 切到 `ws=terminal` → `conversationId` 被清（复用 `buildWorkspaceTabSearch` 既有行为）。
- 删除当前会话 → 参数被清，不残留死 id。

---

### 阶段 3：页面内 Tab 参数统一

**改什么**

1. 新增 `src/lib/pageTabs.ts`：

   ```ts
   export const PAGE_TAB_PARAM = "tab";
   export function usePageTab<K extends string>(valid: readonly K[], fallback: K): [K, (k: string) => void];
   ```

   语义与 `InvestHub.tsx:97-118`、`CapabilityPackHub.tsx:67-80`、`TasksCenterPage.tsx:59-70` 完全一致：非法值回退 fallback，切换时 `set` + 保留其它参数 + `replace`。

2. 三处各自删除本地实现，改用 hook（净减约 40 行重复逻辑）。

3. `/opc/:tab`（`ContentArea.tsx:305`）：保留路由不动，`CapabilityPackHub` 内改为「URL 查询串优先，路径段兜底」，避免改变已公开的链接形态。

**不做**：不改 `/invest` / `/opc` 的参数名（公开 URL，改名收益小于断链成本）。

**验收**

- 三个页面切换 Tab 后刷新均保持；非法 `?tab=` 回退默认且不报错。
- 单测覆盖 hook 的三条分支（合法 / 非法 / 缺省）。

---

### 阶段 4：旧路由重定向表声明化

**改什么**

1. 新增 `src/lib/legacyRoutes.tsx`：一张表 + 一个 `<LegacyRedirect />` 组件，列形如

   ```ts
   { from: "/dashboard", to: "/chat", tab: "dashboard" },
   { from: "/stock-analysis/:id", to: "/invest", tab: "workspace", view: "analysis", passThrough: ["analysisId"] },
   ```

2. `src/components/layout/ContentArea.tsx` 的 `:272-279` 与 `:420-449` 改为 `LEGACY_REDIRECTS.map(...)` 生成，删除 `redirectToChat` / `redirectToDevTools` / `RedirectToChatWithParams` / `RedirectToInvest` 四套形态（统一成一个）。

3. **顺带修复** `:179` 的 `window.location.search`：统一改用 `useLocation()`，使重定向对 router 状态响应式。

4. 表项增加可选 `legacyOf` 字段，供 `AppHeader` 的 `resolvePageLabel`（`src/components/layout/AppHeader.tsx:12-78`）在过渡期显示来路语义（解决 `/marketplace → ws=workflow`、`/llmWiki → ws=knowledge` 的语义丢失）。此项**可后置**，不与 4 主体绑定。

**收益**

- 新增兼容路由只改数据不改 JSX；整张表可被单测逐条断言「目标 pathname + search 合法」。
- 关闭 `PLAN-agent-workbench-enhancement.md` §10.8 登记的未做项。

**验收**

- 表驱动测试：每条 `from` 渲染后都得到期望目标，且目标能被 `parseWorkspaceTab` / 业务 Tab 校验通过。
- 手工：`/marketplace`、`/llmWiki`、`/stock-analysis/123`、`/screener` 逐个访问，地址栏与画面符合预期。

---

## 4. 待拍板的设计决策

| # | 问题 | 建议 | 影响 |
|---|---|---|---|
| D1 | 会话深链参数名用 `conversationId` 还是更短的 `c` | 保留 `conversationId` | 零兼容成本；短名省 10 字符但需同时支持两种 |
| D2 | 同一 `/chat` 内切换会话是否产生 history 记录 | **push**（现有切 Tab 是 replace，保持不变） | 只有 push，后退键才等于「回上一个会话」；否则 2a 的收益只剩外链 |
| D3 | `?ws=` 补齐是否也覆盖 `/`（根路径） | 覆盖，先 `Navigate` 到 `/chat` 再补 | 根路径当前已 redirect 到 `/chat` |
| D4 | 设置分区是否本轮入 URL（`/settings/:section`） | 暂不做，登记为独立事项 | 涉及 `SettingsPage` + 6 处调用方，超出「URL 收敛」最小范围 |

D2 必须在使用前确认：若选 replace，则后退行为不变，2a 的价值主要落在「外链/刷新」，需在文档里如实标注。

---

## 5. 明确不做（本计划范围外）

- **不合并** `/dashboard`、`/workflow` 等 Tab 与独立页（`ContentArea.tsx:272-279` 的 coexistence）。这是产品信息架构决策，需要单独提案。
- **不收敛** Sidebar / AppHeader / CommandPalette 的重叠入口。
- **不拆分** `DecisionBanner`（3129 行）等大组件。
- **不统一** 内联 style → antd token / Tailwind。
- **不动** `components/stock-analysis/` 与 `components/stock-workspace/` 的目录合并。

以上五项各自独立，混入本计划会放大单次改动面，违背「改动尽量小」。

---

## 6. 验证与门禁

按既有节奏：**各阶段施工期不跑编译**，每阶段结束跑一次合并验证。

```powershell
# 阶段内（改完即跑，一次性）
npm run typecheck
npx vitest run src/lib/__tests__/workspaceTabs.test.ts src/components/layout/__tests__/WorkspaceHub.test.tsx
npx vitest run src/hooks/__tests__/useWorkspaceTabNavigator.test.tsx
npm run format
```

- 本计划不触碰 Rust，**无需** `cargo check` / `clippy` / `test`。
- CI 口径与 `package.json` 一致（前端 lint 走 oxlint，不是 ESLint）。

**手工验收清单**（自动化测不到的部分）

1. 裸开 `/chat` → 地址栏出现 `ws`；刷新后位置不变。
2. `/chat?ws=chat&conversationId=A` → 切到 B → 后退 → 回 A（若 D2 选 push）。
3. 带 `?prompt=` 的链接按后退 → **不重复发送**。
4. 从 `/invest?tab=workspace&stockCode=600519` 切到终端 → 地址为 `/chat?ws=terminal`，不带 `stockCode`。
5. `/marketplace`、`/llmWiki`、`/stock-analysis/123` → 目标地址正确、可刷新恢复。

---

## 7. 改动清单汇总

| 阶段 | 文件 | 类型 | 预估 diff |
|---|---|---|---|
| 1 | `src/lib/workspaceTabs.tsx` | 新增纯函数 | +20 |
| 1 | `src/components/layout/WorkspaceHub.tsx` | 归一化 effect | +15 / -2 |
| 1 | `src/lib/__tests__/workspaceTabs.test.ts`、`src/components/layout/__tests__/WorkspaceHub.test.tsx` | 测试 | +60 |
| 2a | `src/pages/ChatPage.tsx` | 初始化 effect 重构 | +30 / -15 |
| 2b | `src/pages/ChatPage.tsx` | 写侧 effect | +25 |
| 3 | `src/lib/pageTabs.ts`（新） | 共享 hook | +45 |
| 3 | `InvestHub.tsx`、`CapabilityPackHub.tsx`、`TasksCenterPage.tsx` | 删除本地实现 | -40 |
| 4 | `src/lib/legacyRoutes.tsx`（新） | 声明式表 | +120 |
| 4 | `src/components/layout/ContentArea.tsx` | 删除 4 套 redirect | -90 |
| — | `src/i18n/locales/*.json` | **无新增 UI 文本**，预期零改动 | 0 |

总规模约 **+300 / -150 行**，集中在 6 个生产文件 + 3 个测试文件。
