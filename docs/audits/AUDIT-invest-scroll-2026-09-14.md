# AUDIT — 投资中心缺少垂直滚动条 / 下部内容被折叠（2026-09-14）

## 1. 现象

用户报告：投资中心页（`/invest`）无垂直滚动条，工作区等视图下部内容被折叠不可见（截图：工作区分析视图底部「看空/看多」比例条被硬裁）。

## 2. 根因

`src/hooks/useGlobalOverlayScrollbars.ts` 以全局 CSS 选择器 `.overflow-y-auto` 自动把**所有** Tailwind 滚动容器交给 OverlayScrollbars 接管，且传入 `elements: { viewport: el }` —— host 与 viewport 指向**同一元素**。

OverlayScrollbars 只在**初始化那一刻**读一次元素的计算 `overflow`，经内部
`overflowCssValueToOverflowStyle(auto, n)` 折算为 `scroll`（需要滚动条）或 `hidden`（不需要），
写入 `data-overlayscrollbars-viewport` 属性，**随后直接以该结果改写元素自身的 overflow**。

React 下「容器先挂载、内容后渲染」是常态：

1. 容器挂载时内容为空 → OS 判「无需滚动」→ 写入 `overflowYHidden` → 元素 `overflow: hidden`；
2. 容器高度此后不再变化（内容变化不改变容器自身尺寸）→ **OS 永不重算**；
3. 内容再高也被硬裁，页面上永远出不来垂直滚动条。

这不是 OS 的 bug，也不是选择器写错，而是**接管时机早于内容到达**导致的一次性判定被永久锁死。

## 3. 量化证据

环境：Vite dev（浏览器模式）+ Playwright/Chromium，视口 1080×599，向 `.sa-main` 注入 2600px 内容块。

| 状态 | 容器 computed `overflow-y` | `data-overlayscrollbars-viewport` | 内容 / 可视 | 实测 `maxScroll` |
| --- | --- | --- | --- | --- |
| 修复前 | `hidden` | `scrollbarHidden overflowXHidden overflowYHidden` | 2664 / 281 | **0（滚不动）** |
| 修复后 | `scroll` | `scrollbarHidden overflowXHidden overflowYScroll` | 2664 / 281 | **2383（可到底）** |

**对照组（关键反证）**：

| 容器 | 选择器 | computed `overflow-y` | 是否可滚 |
| --- | --- | --- | --- |
| `screener` / `quant` 滚动容器 | `overflow-auto` —— **不在 hook 选择器内** | `auto` | 可滚（maxScroll 98 / 342） |
| 工作区视图容器 | `overflow-y-auto` —— 被 hook 接管 | `hidden` | **不可滚** |

同页动态新建一个空 `.overflow-y-auto` 容器**再**填入 2000px 内容，OS 判定为 `overflowYScroll` 且可滚 1802px —— 进一步证明问题出在时序（初始化早于内容），而非 OS 本身或选择器本身。

## 4. 修复

| # | 文件 | 改动 |
| --- | --- | --- |
| 1 | `src/hooks/useGlobalOverlayScrollbars.ts` | 新增 `refreshExisting()`：每轮 DOM 变化后（`scanAndInit` 内）对全部已存在实例 `inst.update(true)` 强制重算；带 120ms 节流（聊天流式输出时 DOM 每帧变化，否则退化成每帧对全部滚动容器强制布局）；`cleanup` 同步清理定时器 |
| 2 | `src/components/stock-analysis/ScreenshotDiagnosisDashboard.tsx` | 根节点 `<div style={{padding:16}}>` → 加 `flex-1 min-h-0 overflow-auto`（原**根本没有滚动入口**，实测 pane 419 / 内容 466，被裁 47px） |
| 3 | `src/components/stock-analysis/PaperPortfolioDashboard.tsx` | 同上（空数据下恰好不溢出，属同一缺陷模式的潜在复发点） |

第 2、3 处刻意使用 `overflow-auto` 而非 `overflow-y-auto`：后者会被上述全局选择器接管，与 `screener` / `quant` 既有模式保持一致。

## 5. 验证

| 项 | 结果 |
| --- | --- |
| 工作区注入 2600px 内容 | `overflowYScroll`，maxScroll 2383（修复前恒 0） |
| 强制重算是否重置滚动位置 | scrollTop 900 → 连做两次 update 后仍 900；滚到底 2783 → 仍 2783 ✔ |
| 投资中心 6 个 tab 全景扫描 | `screenshot-diagnosis` 由 `paneClips=true`（466/419）→ **false**；`paper-portfolio` 滚动入口就位；其余 tab 无回归；`clipRisk` 为空 |
| `tsc --noEmit` | 0 错误 |
| `dprint check` | 通过 |
| 单测 `App.d2.test.tsx` | 已 `vi.mock("@/hooks/useGlobalOverlayScrollbars")`，不受影响（**同时也说明该 hook 目前无任何测试覆盖**） |

复现/验证脚本留存于 `output/`：`layout-probe4.mjs`（注入压测 + OS 状态）、`layout-probe5.mjs`（CDP 追 overflow 来源）、`layout-probe6.mjs`（新容器对照实验）、`layout-probe7.mjs`（滚动位置保持）、`layout-probe3.mjs`（全 tab 扫描）。

## 6. 登记（本轮未改）

- **`.nav-sidebar` 溢出被裁**：`src/index.css:2074` 显式 `overflow: hidden`，实测 clientHeight 567 / scrollHeight 673 ⇒ 侧边栏底部 106px 不可见且无滚动条。与 OverlayScrollbars 无关，属独立缺陷；修法取决于「折叠态 / 展开态是否应有滚动」的产品决策。
- **`PipelinePage` 在浏览器 mock 模式命中 PageErrorBoundary**：显示「页面错误」。其根容器自带 `flex-1 overflow-auto`，滚动链本身完好，推测为 mock 环境专有崩溃，未处理。

---

## 7. 第二轮：真正的根因（同日，用户反馈「修改了半天没有任何改善」后）

### 7.1 第一轮为何无效

第一轮修的全是**叶子容器**。但投资中心内容区的最外层
`.ax-fill-tabs > .ant-tabs-body-holder > .ant-tabs-body > .ant-tabs-content`
自身写着 `overflow: hidden`：**只要 tab 内容超出这一层的高度，超出部分就在这里被裁掉，且这一层不产生滚动条**。
叶子容器修得再对，也改变不了「内容在外层就被吃掉」——所以用户看到的画面**像素级零变化**
（佐证：修复前后同视口同 tab 的截图字节数完全相同，screener 40964=40964、workspace 45474=45474）。

> **判据**：报告「内容被裁 / 看不到滚动条」时，**先量最外层容器的 overflow**，再往下修叶子。
> 只修叶子会被外层吞掉，对外表现为「修了但毫无改善」。

### 7.2 决定性证据（视口 1072×679，向激活 tab 注入 3000px 内容块）

| 状态 | `.ant-tabs-content` computed `overflow-y` | 鼠标滚轮 6×300px 后 `scrollTop` | 3000px 内容块可见高度 |
| --- | --- | --- | --- |
| 修复前 | `hidden` | **0（完全滚不动）** | **26px**（其余 2974px 永久不可达） |
| 修复后 | `auto` | **1500** | **679px（完整可达）** |

注意 `overflow: hidden` 的元素**仍可被程序置 `scrollTop`**（脚本读到的 `maxScroll` 是 3000），
但**用户滚轮/拖拽完全无效**且**不显示滚动条** —— 这正是「缺滚动条 + 下部内容被折叠」的成因。
只看 `scrollHeight > clientHeight` 或 `maxScroll > 0` 会把这种容器误判成「可滚」，
**必须用真实滚轮事件验证**。

### 7.3 本轮改动

| # | 文件 | 改动 |
| --- | --- | --- |
| 4 | `src/index.css`（`.ax-fill-tabs … .ant-tabs-content`） | `overflow: hidden` → `overflow-x: hidden; overflow-y: auto` —— **核心修复**；9 个使用 `ax-fill-tabs` 的页面共同受益 |
| 5 | `src/index.css`（`.nav-sidebar`） | `overflow: hidden` → `overflow-x: hidden; overflow-y: auto`（消化第一轮登记项） |
| 6 | `src/lib/browserMock.ts` | 补 `get_pipeline_history`（`[]`）、`get_pipeline_run_detail` / `run_stock_pipeline`（`null`）显式分支 |
| 7 | `src/stores/feature/pipelineStore.ts` | `fetchHistory` 加 `Array.isArray` 契约防御 |

**第 6 项根因**：mock 的 `default` 分支对 `get_*` 一律兜底返回 `{}`，而 `get_pipeline_history`
的消费端按数组使用（`pipelineRuns.filter/.length`），收到 `{}` 即抛 `TypeError`
（`PipelinePage.tsx:105`），把「投资中心 → 管道」整个 tab 兜成「页面错误」（消化第一轮登记项）。
第 7 项使该崩溃在**真实后端异常返回**时也不会整页白屏。

### 7.4 验证（视口 1072×679，投资中心全部 6 个 tab）

| 项 | 结果 |
| --- | --- |
| 注入 3000px 后滚轮 | `scrollTop` 0 → 1500，内容块可见 679px ✔ |
| 6 tab 扫描 | 全部 `overflow-y: auto`；**残留「裁切且不可滚」容器 = 0**（修复前为 `nav-sidebar`）✔ |
| 管道 tab | 错误边界不再触发、js 错误 0（修复前 `pipelineRuns.filter is not a function`）✔ |
| 侧边栏 | 可视 647 / 内容 673，滚轮后 `scrollTop` = 26 ✔ |
| 副作用 | 内容未溢出时不产生滚动条（`auto` 语义）；9 个 `ax-fill-tabs` 页面抽查无异常 ✔ |
| `tsc --noEmit` / `dprint check` | 均通过 ✔ |

复现脚本留存于 `output/`：`stress-scroll.mjs`（注入压测）、`wheel-test.mjs`（滚轮对照）、
`dump-chain.mjs`（DOM 高度链 + 错误捕获）、`final-verify.mjs`（全量回归）。

### 7.5 说明

浏览器 mock 数据量偏小，多数 tab 内容不满一屏，**故「修复后仍看不到滚动条」属正常**
（`auto` 仅在溢出时才出现滚动条）。真实数据下的溢出场景由上述注入压测等价覆盖。
