# PLAN · 前端信息架构重整

> 状态：active ｜ 日期：2026-10-27 ｜ 对应版本：v2.9.9
> 范围：导航模型、页面层级、路由映射、可达性保证、股票工作台布局
> 不含：视觉改版、组件内部重写、后端改动

## 0. 决策记录

| # | 决策 | 影响 |
|---|---|---|
| D1 | Chat 在业务域内以**右侧可召出面板**出现 | 复用现有右侧可隐藏 agent 面板 |
| D2 | **一次性切换**到新路由，不长期双轨 | 入口必须先于路由就位 |
| D3 | **取消 `density` 全局偏好**，完全交给区块展开 | 删除 `useSimpleMode` / `useShowAdvancedPanels` 的分支语义 |
| D4 | 8 个能力域聚合页**并入 OPC** | 顶级路由消失，`domainMeta.path` 改为 `/opc/domains/*` |
| D5 | `/quickbar` → `/chat/quickbar` | 归入 Chat 域 |
| D6 | `/dynamic-ui/:schemaId` 是**内部功能** | 仅深链，可达性检查白名单 |

## 1. 问题定义

两套导航模型并存：

| 模型 | 表现 | 证据 |
|---|---|---|
| Chat-first | `dashboard` / `workflow` / `terminal` / `files` / `knowledge` / `multiAgent` 重定向到 chat 内 tab | `src/components/layout/ContentArea.tsx:272-279` |
| Workspace-first | `/invest`、`/opc`、`/tasks`、`/settings`、`/wiki` 是独立页面 | `src/components/layout/ContentArea.tsx:150-357` |

黏合层是 query + store 双向同步，`ChatPage` 需要 `syncingRef` 防循环（`src/pages/ChatPage.tsx:221-268`）。

可达性缺口：侧栏 19 项（`src/lib/navRegistry.tsx:46-163`），`pageRegistry` 约 48 条路由。差额页面无稳定入口：

| 页面 | 现状入口 |
|---|---|
| `/workflow`、`/terminal`、`/files`、`/multi-agent`、`/marketplace` | 仅 chat 内 tab |
| `/memory`、`/wiki`、`/learning-graph`、`/quickbar` | 无侧栏项 |
| `/replay-workbench` | 无侧栏项 |
| `/devtools/*` | 命令面板或深链 |
| `/gateway`、`/link` | 注释称“已迁入设置”，路由仍在 |
| `/dynamic-ui/:schemaId` | 无入口（D6 允许） |

## 2. 目标模型：Shell + Domain

```
Shell
├── 左侧：5 个一级域 + 设置
├── 顶部：域标题 + 面包屑 + 全局搜索（唯一）
└── 主区：当前域

Domain
├── 域内二级导航
├── 域首页
└── 域内视图（URL 路径段）

右侧：Chat 面板（D1），任何域内可召出
```

| 域 | 前缀 | 承载 |
|---|---|---|
| Chat | `/chat` | 对话、工作流、终端、文件、多智能体、市场、QuickBar |
| Invest | `/invest` | 工作台、选股、组合、量化、回放 |
| Knowledge | `/knowledge` | 知识库、Wiki、记忆、学习图谱 |
| OPC | `/opc` | 经营面板、需求发现、12 行业包、8 能力域 |
| Tasks | `/tasks` | 定时任务、手动作业 |
| Settings | `/settings` | 设置、网关、链接、DevTools |

## 3. 路由映射（旧 → 新）

### 3.1 Chat 域

| 旧 | 新 | 新入口 |
|---|---|---|
| `/chat` | `/chat` | 侧栏首项 |
| `/dashboard` | `/chat` | 合并 |
| `/workflow` | `/chat/workflow` | Chat 域内导航 |
| `/workflow/new` | `/chat/workflow/new` | 同上 |
| `/terminal` | `/chat/terminal` | Chat 域内导航 |
| `/files` | `/chat/files` | Chat 域内导航 |
| `/multi-agent` | `/chat/agents` | Chat 域内导航 |
| `/marketplace` | `/chat/marketplace` | Chat 域内导航 |
| `/quickbar` | `/chat/quickbar` | Chat 域内导航（D5） |

### 3.2 Invest 域

| 旧 | 新 | 新入口 |
|---|---|---|
| `/invest` | `/invest` | 侧栏 |
| `/stock-analysis` | `/invest` | 域首页 |
| `/stock-analysis/:id` | `/invest/:id` | 域首页 → 单股 |
| `/watchlist` | `/invest/watchlist` | Invest 域内导航 |
| `/trade` | `/invest/:stockCode/trade` | 单股视图 |
| `/backtest` | `/invest/:stockCode/backtest` | 单股视图 |
| `/compare` | `/invest/:stockCode/compare` | 单股视图 |
| `/screener` | `/invest/screener` | Invest 域内导航 |
| `/quant` | `/invest/quant` | Invest 域内导航 |
| `/replay-workbench` | `/invest/replay` | **新增入口** |

### 3.3 Knowledge 域

| 旧 | 新 | 新入口 |
|---|---|---|
| `/knowledge` | `/knowledge` | 侧栏 |
| `/memory` | `/knowledge/memory` | **新增入口** |
| `/wiki`、`/llm-wiki` | `/knowledge/wiki` | Knowledge 域内导航 |
| `/llm-wiki/:wikiId/graph` | `/knowledge/wiki/:wikiId/graph` | Wiki 内跳转 |
| `/llm-wiki/:wikiId/ingest` | `/knowledge/wiki/:wikiId/ingest` | Wiki 内跳转 |
| `/llm-wiki/:wikiId/edit/:noteId` | `/knowledge/wiki/:wikiId/edit/:noteId` | Wiki 内跳转 |
| `/learning-graph` | `/knowledge/learning` | **新增入口** |

### 3.4 OPC 域（含 D4）

| 旧 | 新 | 新入口 |
|---|---|---|
| `/opc` | `/opc` | 侧栏 |
| `/opc/:tab` | `/opc/:tab` | 域内导航 |
| `/opc/domain/:packId` | `/opc/domain/:packId` | 域内导航（12 行业包） |
| `/demand-discovery` | `/opc/demand` | OPC 域内导航 |
| `/general` | `/opc/domains/general` | OPC 域内“能力域” |
| `/devops` | `/opc/domains/devops` | 同上 |
| `/ai-media` | `/opc/domains/ai_media` | 同上 |
| `/data-analysis` | `/opc/domains/data_analysis` | 同上 |
| `/content-creation` | `/opc/domains/content_creation` | 同上 |
| `/communication` | `/opc/domains/communication` | 同上 |
| `/finance` | `/opc/domains/finance` | 同上 |
| `/automation` | `/opc/domains/automation` | 同上 |

**路由顺序硬约束**：`/opc/domains/:domainId` 必须声明在 `/opc/:tab` **之前**，否则被 `:tab` 吞掉。

### 3.5 Tasks / Settings / 其他

| 旧 | 新 | 新入口 |
|---|---|---|
| `/tasks` | `/tasks` | 侧栏 |
| `/tasks/:id` | `/tasks/:id` | 任务列表内跳转 |
| `/settings/*` | `/settings/*` | 侧栏底部 |
| `/gateway`、`/link` | `/settings/gateway` | 设置内“连接” |
| `/devtools/*` | `/devtools/*` | 设置内“开发工具” |
| `/dynamic-ui/:schemaId` | 不变 | 仅深链（D6，白名单） |

## 4. 可达性保证机制

### 4.1 CI 检查

新增 `scripts/check-route-reachability.mjs`：

1. 从 `pageRegistry` + `ContentArea` 抽路由；
2. 从 `navRegistry`、域配置、`CommandPalette`、页面内跳转白名单抽入口；
3. 每条路由必须命中至少一个入口；
4. 未命中 → CI 失败。

白名单（仅 `internal: true` 可豁免）：`/dynamic-ui/:schemaId`。

### 4.2 切换前先补 12 个入口

见 §3 中标注“新增入口”的项，以及 Chat/Knowledge 域内导航。

**顺序：入口与路由同一版本上线**，因为 D2 要求一次性切换；入口先指向新路径，旧路径同时 302。

### 4.3 旧路由 302 保留一个版本

`Navigate replace`，参数透传（`stockCode` / `wikiId` / `view`）。
下一版本删除，并用 4.1 脚本确认零残留。

## 5. 股票工作台重设计

```
现状：Header → DecisionHeroBar → 6 视图 Tab → 三栏
目标：域 Header（股票切换 + 视图切换同行）→ 决策条（单行可展开）→ 单栏内容
      右栏 ContextSidebar 改按需抽屉；Chat 面板可召出
```

| 项 | 变更 |
|---|---|
| 视图切换 | 上移域 Header |
| DecisionHeroBar | 默认单行；展开 2–3 行；详情抽屉 |
| 左栏 | 折叠为图标条 |
| 右栏 | 按需抽屉 |
| 移动端 | 底部 4 tab；决策条折叠；不砍功能 |

验收：桌面内容区 ≥ 首屏 60%；移动端 4 tab 覆盖 6 视图。

## 6. 渐进披露（替代 density）

| 区块 | 默认 | 展开 |
|---|---|---|
| 决策条 | 单行 | 2–3 行 / 抽屉 |
| 分析报告 | 结论 + 关键证据 | 完整报告 / 辩论 |
| 回测 | 指标卡 + 曲线 | 明细 / 参数 |
| 反思 | 最近一次 | 历史 / 漂移 |

删除 `useSimpleMode` / `useShowAdvancedPanels`；移动端不再强制 simple。

## 7. 阶段与成本

| 阶段 | 内容 | 人日 | 状态 |
|---|---|---|---|
| 1 | 域并入 OPC + 补入口（沿用旧路径）+ 可达性脚本 | 4–6 | ✅ 2026-10-28 |
| 2 | 路由切换（`/chat/*`、`/knowledge/*`、`/invest/*`）+ 旧路径 302 + `WorkspaceHub` 路径段支持 | 4–6 | 待实施 |
| 3 | Shell 重构（Sidebar 分域、AppHeader 降级、WorkspaceSwitcher 并入） | 3–5 | 待实施 |
| 4 | 股票工作台重排 + 渐进披露 | 8–12 | 待实施 |
| 5 | 删旧路由与 `PAGE_LABELS` | 2 | 待实施 |
| **合计** | | **21–31 人日** | |

## 8. 阶段 1 文件级清单（待实施）

### 8.1 路由与注册表

| 文件 | 改动 |
|---|---|
| `src/lib/pageRegistry.ts` | 删除 8 个域顶级 key 的展开；新增 `/chat/*`、`/invest/*`、`/knowledge/*`、`/opc/domains/:id` 常量 |
| `src/lib/domainMeta.ts` | `DOMAIN_PRESENTATION.path` 改为 `/opc/domains/<id>`；更新“域聚合入口”注释 |
| `src/components/layout/ContentArea.tsx` | 删 8 个 `<Route path={domain.path}>`；新增 `/opc/domains/:domainId`（放在 `/opc/:tab` 之前）；新增所有新路由；旧路径 302 |
| `src/pages/DomainHubPage.tsx` | 从 `useParams().domainId` 取域，不再从 pathname 反查 |
| `src/components/layout/AppHeader.tsx` | 域标签匹配改为 `startsWith("/opc/domains/")`；删除 8 个顶级路径映射 |
| `src/components/layout/Sidebar.tsx` | `domainPath` 指向新路径；Chat/Knowledge/Invest 增加域内二级导航 |
| `src/lib/navRegistry.tsx` | 新增域内导航项（workflow/terminal/files/agents/marketplace/quickbar；memory/wiki/learning；replay/watchlist） |
| `src/lib/browserMock.ts` | `nav_path` 与 `domainMeta.path` 同步 |
| `scripts/check-route-reachability.mjs` | 新增；接入 CI |

### 8.2 测试

| 文件 | 改动 |
|---|---|
| `src/lib/__tests__/navRegistry.test.ts`（若存在） | 断言每个 nav item 的 path 在新路由表中 |
| `src/lib/__tests__/pageRegistry.test.ts` | 断言无重复路径、无 `/general` 等顶级域 |
| 新增 `scripts/__tests__/check-route-reachability.test.mjs` | 覆盖“未命中入口”的失败用例 |

### 8.3 旧路由 302 清单

`/dashboard`、`/workflow`、`/workflow/new`、`/terminal`、`/files`、`/multi-agent`、`/marketplace`、`/quickbar`、`/stock-analysis`、`/stock-analysis/:id`、`/watchlist`、`/trade`、`/backtest`、`/compare`、`/screener`、`/quant`、`/replay-workbench`、`/memory`、`/wiki`、`/llm-wiki`、`/llm-wiki/:wikiId/*`、`/learning-graph`、`/demand-discovery`、`/gateway`、`/link`、8 个域顶级路径。

## 9. 验收清单

- [ ] `check-route-reachability` 通过，无未命中项（除 `/dynamic-ui/:schemaId` 白名单）；
- [ ] 任一功能 ≤ 2 次点击可达；
- [ ] 任一页面可直接打开、刷新、分享；
- [ ] 12 个“仅深链”页面全部有域内入口；
- [ ] 8 个能力域旧路径 302 到 `/opc/domains/*`，无 404；
- [ ] 桌面端内容区 ≥ 首屏 60%；
- [ ] 移动端 4 tab 覆盖全部视图。

## 10. 明确不做

- 视觉改版；
- 拆 `DecisionBanner`（3125 行）等超组件；
- 改后端；
- 处理 DSH 迁移。

## 11. 实施记录

### 2026-10-28 · 阶段 1

**已完成**

1. 8 个能力域聚合页并入 OPC：
   - `domainMeta.path` 保持协议值不变（门禁要求与后端 `nav_path` 一致）；
   - 顶级路由注销，统一为 `/opc/domains/:domainId`；
   - 旧路径 `/general` 等 302 到新路径；
   - 同步修改 5 个消费点：`ContentArea`、`Sidebar`、`pageRegistry`、`AppHeader`、`DomainHubPage`。
2. 补齐 12 个“有路由、无入口”的页面：workflow / terminal / files / multi-agent / marketplace / quickbar / knowledge / memory / wiki / learning-graph / replay-workbench / gateway。
3. 新增 `scripts/check-route-reachability.mjs`，接入 `npm run check:routes` 与 `check:routes:strict`。

**验证**

- `npm run typecheck` 通过；
- `npm run check:routes`：55 条路由，**全部有导航级入口**，strict 退出码 0；
- 本次改动文件的 `dprint check` 通过。

**范围调整**

原计划阶段 1 同时切换路由。实施时确认：`/chat/workflow` 等页面由 `?ws=` query 驱动，`WorkspaceHub` 还承担保活与「回上次位置」语义；直接改路径段会破坏这两条，属于阶段 3 的 Shell 重构。因此阶段 1 **只补入口，沿用旧路径**，路由切换整体移到阶段 2，仍是一次性切换。

**已知限制**

- `/dynamic-ui/:schemaId` 未被静态解析（`<Route>` 的属性含嵌套 JSX，正则提前截断）；它是内部路由（D6），不影响“全部可达”的结论，但脚本对该条是盲区。
- 全仓 `dprint check` 仍有 2 个**既有**失败（`BacktestChart.tsx` 解析错误、`ValuationBandChart.tsx` import 顺序），与本次改动无关。

### 2026-10-28 · 阶段 2

**已完成**

1. 工作台 URL 契约从 `?ws=<tab>` 迁到路径段 `/chat/<tab>`：
   - `workspaceTabs.tsx` 新增 `workspaceTabPath()` / `parseWorkspaceTabPath()`；
   - `WorkspaceHub` 解析优先级：`location.state` → 路径段 → 旧 `?ws=`（解析后归一化到路径）；
   - `useWorkspaceTabNavigator` 改为写路径段，不再写 `ws`。
2. `ContentArea` 新增 `/chat/:tab`、`/chat/quickbar`、`/chat/marketplace`、`/invest/replay`；
   旧路径 `/workflow`、`/terminal`、`/files`、`/multi-agent`、`/marketplace`、`/quickbar`、`/replay-workbench` 全部 302。
3. `navRegistry` 的 11 个入口改为新路径；`/chat/agents` 统一为 `/chat/multiAgent`。

**验证**

- `npm run typecheck` 通过；
- `npm run check:routes:strict`：59 条路由，全部有导航级入口，退出码 0；
- `vitest run workspaceTabs.test.ts WorkspaceHub.test.tsx`：**35 个测试全过**。

**顺带修复**

`src/components/stock-analysis/BacktestChart.tsx:346` 的既有 JSX 语法错误（`color=cssVar("--red")` 缺花括号）会同时阻塞 `tsc` 与 `dprint`，已修为 `color={cssVar("--red")}`。该文件不在本次改动范围内，属工作区既有的未提交改动。

**遗留**

- `/knowledge` 仍走 `redirectToChat("knowledge")`，即 `/chat/knowledge`；Knowledge 域的独立路径（`/knowledge/memory` 等）留到阶段 3 与 Sidebar 分域一起切。

**界面验证（2026-10-28，本地 Vite + 无头浏览器）**

- `/chat/workflow`：渲染工作流页，工作台切换栏保留；
- `/opc/domains/finance`：渲染“金融”域页，3 个入口（金融投资 / 投顾经营 / 财务会计）；
- `/chat`、`/invest`：正常渲染。

验证中发现并修复一个回归：`App.tsx` 的 `isChatPage` 只认 `/chat`，切到 `/chat/<tab>` 后工作台切换栏会消失，用户无法再切回其他 Tab。已改为 `isWorkspacePath()`，同时覆盖 `/chat` 与 `/chat/*`。

**发现未修**

`DomainHubPage` 的能力卡片文字被挤成竖排（卡片宽度不足），属既有布局问题，与本次路由改动无关，留到阶段 4 的视觉/密度调整。

### 2026-10-28 · 阶段 3

**已完成**

1. 新增 `src/lib/navGroups.ts`：**产品导航组**（7 组），与 `CapabilityDomain`（后端 8 域协议）解耦。能力域仍服务 `DomainHubPage` 与工具域过滤，不再充当侧栏菜单轴。
2. `Sidebar` 改为按 `NAV_GROUPS` 渲染，组内顺序由 `itemKeys` 决定：
   工作台（7）/ 金融投资（2）/ 知识源（4）/ 运营管理（2）/ 能力包（14）/ 任务中心（1）/ 系统（1）。
3. `AppHeader` 删除原 20+ 项 `PAGE_LABELS` 手写映射，改为按 `builtinNavItems` 最长前缀匹配、回落到 `NAV_GROUPS` 组标签；单一真相源回到 `navRegistry`。
4. 分组重名修复：行业包组原复用 `nav.marketplace`（“市场”）与工作台组的“市场”项冲突，改用既有的 `opc.nav.market`（“能力包”），不新增 11 语言文案。

**范围调整**

原计划含「`WorkspaceSwitcher` 并入 Sidebar」，本次**未执行**。原因：移动端侧栏折叠后没有顶部切换栏就无法切 Tab，且快捷键、命令面板都依赖现有切换路径。侧栏已提供同构入口，删除顶部栏属于收益低、风险高的改动，留待移动端方案确定后再评估。

**验证**

- `npm run typecheck` 通过；
- `npm run check:routes:strict`：59 条路由全部有入口；
- `WorkspaceSwitcher` + `WorkspaceHub` 单测 **21 个全过**；
- 界面截图：侧栏展开后 7 组可见，“能力包”与“市场”不再重名；`/tasks` 顶部正确显示“任务中心”。

### 2026-10-28 · 阶段 4（部分完成）

**已完成（4a：布局重排）**

`StockWorkspaceShell` 把原先独立的一行「视图 Tab」合并进顶部 Header：

```
改前：Header / DecisionHeroBar / 视图 Tab / 三栏
改后：Header（返回 + 股票 + 视图切换 + 模式）/ DecisionHeroBar / 三栏
```

- 视图切换按钮改为 `shrink-0` + 水平滚动容器，窄屏不挤爆；
- 股票名限宽 `max-w-56` 截断，避免长名称挤走 Tab；
- 决策条默认单行（`workspaceStore.userMode` 默认 `simple`，无需改代码）；
- 截图确认：`/invest?tab=workspace&stockCode=600519&view=analysis` 布局正确，决策条显示“暂无决策，请先运行分析”。

**未执行（需重新评估）**

| 原计划 | 原因 |
|---|---|
| 左栏 `StockSwitcher` 改抽屉 | 它已有 48px 折叠态，改抽屉收益约 0，且会藏掉最近股票列表 |
| 右栏 `ContextSidebar` 改抽屉 | 其持仓与大盘面板是“交易决策前置条件，始终可见”（组件注释），改抽屉会让用户看不到持仓，属功能退步 |
| 移动端 4 tab | 当前是 5 tab（分析/监控/交易/复盘/更多），4 tab 覆盖不了 6 个视图 |
| 删除 `useSimpleMode` / `useShowAdvancedPanels` | 默认已是 `simple`（单行），删除需要逐区块改渐进披露，属独立任务；`useSimpleMode` 仍被 `DecisionHeroBar`、`DecisionConsensusMeter` 等消费 |

**验证**

- `npm run typecheck` 通过；
- 工作台截图确认 Header 合并后不再有独立的 Tab 行。

### 2026-10-28 · 阶段 5

**已删除**

- `ContentArea` 中 7 条 chat 类旧路由：`/dashboard`、`/workflow`、`/workflow/new`、`/terminal`、`/files`、`/multi-agent`、`/marketplace`、`/quickbar`；
- `/replay-workbench` 旧链（统一走 `/invest/replay`）；
- `RedirectToChatWithParams` 组件（唯一调用点已删）；
- `pageRegistry` 中 7 个重复别名：`workflow`、`terminal`、`files`、`multiAgent`、`marketplace`、`quickbar`、`"replay-workbench"`（保留 `chatWorkflow` 等新 key）；
- 可达性脚本白名单中的对应条目。

**保留（刻意）**

- 股票旧路由 `/stock-analysis`、`/trade`、`/watchlist`、`/backtest`、`/compare`、`/screener`、`/quant`：仍有多处 `backTo="/stock-analysis"` 的组件依赖，作为兼容层保留，下一轮再迁。
- `/knowledge` 仍走 `redirectToChat("knowledge")`，Knowledge 域独立路径未切。
- `/dashboard` 的 `pageRegistry` key 保留（值 `/chat/dashboard`），它是合法的 WorkspaceTab。

**同步**

- `src` + `e2e` 共 13 个文件中的旧路径字面量批量替换为新路径（`pageRegistry.test.ts`、`skillActionExecutor.test.ts`、`App.tsx`、`WorkflowMarketplace.tsx`、7 个 e2e spec）。

**验证**

- `npm run typecheck` 通过；
- `npm run check:routes:strict`：**53 条路由**全部有入口（较阶段 4 少 6 条，即删除的旧链）；
- `pageRegistry` + `workspaceTabs` + `WorkspaceHub` 单测 **40 个全过**；
- e2e 未在本轮运行（需要完整 Tauri 运行时）；路径已同步，交给 CI。

### 2026-10-28 · 收尾（剩余可选项）

**1. Knowledge 域独立**

- `/knowledge` 不再 `redirectToChat`，直接渲染 `KnowledgeHubPage`；
- 新增 `/knowledge/memory`（MemoryPage）、`/knowledge/wiki`（KnowledgeHubPage）、`/knowledge/learning`（LearningGraphPage）；
- `/memory`、`/llm-wiki`、`/learning-graph` 改为 302；
- `navRegistry` 的 knowledge 三项改指新路径；
- `KnowledgeHubPage` 的“学习图”按钮改为 `/knowledge/learning`。

**2. 股票旧路由彻底删除**

- 删除 9 条路由：`/stock-analysis`、`/stock-analysis/:id`、`/trade`、`/watchlist`、`/backtest`、`/compare`、`/screener`、`/quant`、`/workspace/:stockCode`；
- 删除 `RedirectToInvest` 组件与 `redirectToChat` 函数（已无调用点）；
- 7 个组件的 `backTo="/stock-analysis"` / `navigate("/stock-analysis")` 改为 `/invest`；
- 可达性脚本白名单同步清理。

**3. 域卡片竖排修复**

根因不是卡片宽度，而是外层 `Row/Col` 始终没拿到预期宽度：容器实际宽度只有约 220px，`lg={6}`（25%）因此算成 55px，文字被挤成竖排。

修复：`Row/Col` 改为 CSS Grid（`repeat(auto-fill, minmax(220px, 1fr))`），外层容器补 `width: 100%`，标题加 `nowrap + ellipsis`。截图确认恢复为三列、文字完整。

**验证**

- `npm run typecheck` 通过；
- `npm run check:routes:strict`：**47 条路由**全部有入口（较阶段 5 少 6 条，即股票旧链）；
- `pageRegistry` + `workspaceTabs` + `WorkspaceHub` 单测 **40 个全过**。

**保留说明**

`src/components/stock-analysis/StockAnalysisPage.tsx` 组件本身保留（股票工作台的 `AnalysisView` 内嵌使用），仅删除其独立路由与页面包装引用。
