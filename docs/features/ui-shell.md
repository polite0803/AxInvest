# UI 外壳与前端规范

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：ui-shell

## 定位

这条链路定义「用户看到的外壳」：页面从哪来、路由在哪注册、顶栏多高、文案从哪种语言来、组件之间允许怎样的依赖、浮层触发器如何定位、主题与语法高亮如何收口。它不是某个业务页面，而是所有页面共用的**约束层**——违反这些约束不会立刻白屏，但会在跨语言、跨模块、跨浮层时集中爆雷。

## 当前行为

### 1. 路由与页面注册

路由是**显式注册**，不是文件系统扫描：`ContentArea` 维护一张 `<Route path=... element=...>` 列表，页面组件从 `src/pages/` 直接 import；`useNavigate` 在同一个组件里负责交互跳转（`src/components/layout/ContentArea.tsx:157`、`:213-347`）。动态页面（插件 / dynamicUI）走最后一条带参数的通配路由（`:347`），不会覆盖静态路由。

页面注册集中在一处的好处是「可达性」可审计：新增页面必须同时给出路径与入口，不能只加文件不加路由。导航模型的上游计划见 `PLAN-frontend-information-architecture.md`（v2.9.9）。

（对应版本：v2.9.6（导航 / 页面注册重构，`CHANGELOG.md:734`）；v3.0.0（任务中心 v2 统一运行视图接入路由表，`bd2532a71`；路由门禁注释剥离感知，`CHANGELOG.md:364`））

### 2. 布局与 `AppHeader` 40px 规范

`AppHeader` 是全局顶栏的唯一实现（`src/components/layout/AppHeader.tsx:45`），由 `ContentArea` 挂载（`src/components/layout/ContentArea.tsx:3`、`:208`）。高度是硬规范：`height: 40` + `minHeight: 40`（`src/components/layout/AppHeader.tsx:68-69`），所有页面内容区的可用高度都按这 40px 扣减。

**按页面抑制**：对话页不显示 `AppHeader`（`src/components/layout/AppHeader.tsx:59`），股票页也不显示（`:61`）；股票工作台另有自己的 `sa-header`（`:20`），不要再叠一层全局顶栏。新增页面若自带顶栏，必须同时确认全局顶栏在该路由被抑制，否则会出现双顶栏 + 内容区被压两遍。

（对应版本：v3.0.0（顶栏高度与路由 / 首字节预算统一收口，`CHANGELOG.md:195`））

### 3. i18n：11 种语言与门禁

默认语言是 zh-CN，且**只有 zh-CN 同步 bundle**；其余 10 种语言（en-US、zh-TW、ja、ko、fr、de、es、ru、hi、ar）全部动态 `import`（`src/i18n/index.ts:10-21`、`:51-54`）。`changeLanguage` 被拦截：切换前先 `await loadLocale`，避免切过去瞬间显示裸 key（`:68-76`）；加载失败不缓存，允许下一次重试。

**回退语言**：代码里 `fallbackLng` 是 `"en-US"`（`src/i18n/index.ts:56`），而 `AGENTS.md` 写的是「默认 / 回退均为 zh-CN」。两者当前不一致；实际行为以代码为准（en-US 回退），该差异列入「待确认」。

**错误文案**：后端错误码翻译平铺在顶层 `error` 段，键就是后端 `code`（如 `error.CONVERSATION_NOT_FOUND`），与 `error.network` 等 14 个 UI 错误同段；不允许散落到 `chat.workflow.errorDetail` 之类子段。

**四道门禁**（均已在 `scripts/ci-check.mjs` 与 CI 注册；带 `--selftest` 的脚本先跑自检）：

| 门禁 | 查什么 | 基线 |
|---|---|---|
| `scripts/check-i18n-key-parity.mjs` | 11 种语言键集对齐（zh-CN 为源） | 零容忍 |
| `scripts/check-i18n-exemption-budget.mjs` | 文件级 `i18n-exempt` 必须带 until；行级 allowlist 的 reason 不得是默认措辞；总量只减不增 | 142 条 |
| `scripts/check-jsx-text-i18n.mjs` | JSX children 未翻译字面量（CJK 即违规；英文词须命中某 locale 值） | 51 条 |
| `scripts/check-i18n-orphan-diff.mjs` | 删 `src/**` 代码后遗留的 locale key（只扫 `git diff --name-status HEAD`；变更面过大时显式降级 report-only） | 无（机械判据） |

（对应版本：v3.0.0（i18n 门禁 + 豁免预算接线，59533a05e，`CHANGELOG.md:198`）；v2.9.7（补全 9 个 i18n key 至全部 locale，`CHANGELOG.md:671`））

### 4. 组件中立性

规则是**目录级**的：`src/components/<A>/**` 不得 import `@/stores/feature/<B>Store`（B≠A）或 `@/components/<B>/**`（B≠A）；`@/` 别名与 `../` 相对路径统一 resolve 后判定。中性目录（`layout` / `common` / `shared` / `dynamicUI`）天然跨 feature，不算违规；新增中性目录必须登记进脚本 `NEUTRAL_PREFIXES` 并写理由。

白名单是 `scripts/component-neutrality-allowlist.json`，键 = 文件 + 精确 import 串（**不含行号**，行号随编辑漂移），登记必须带 reason；现存唯一条目是 `shared/PageContextProvider.tsx`（边界聚合器）。棘轮基线 `scripts/component-neutrality-baseline.json` 首建 **212 条**存量跨 feature 引用，**只减不增**：新违规必须当场修（下沉 / 注入 / 事件化）或显式登记；存量修复后跑 `--update-baseline` 收紧，存在新违规时该命令拒绝执行。

（对应版本：v3.0.0（组件中立性门禁落地，`d2652a980`、`CHANGELOG.md:197`））

### 5. 浮层 ref 透传

任何自定义组件被 antd 浮层（Popover / Popconfirm / Tooltip / Dropdown / Select）作为 children 时，必须 `forwardRef` 并把 ref 透传到真实 DOM。否则 rc-trigger 无法定位触发器，弹层会落到视口左下角并立即消失——React 19 已移除 `findDOMNode`，没有兜底。标准实现是 `src/components/layout/Tooltip.tsx`：`forwardRef`（`:77`）、`setTriggerRef` 合并外部 ref 与内部触发 ref（`:118-122`）、真实元素挂 ref（`:162`）。

新增这类组件时，先 `grep` 它是否被浮层包裹，再决定是否转发 ref。`Tooltip.tsx` 的合并写法可以直接照抄，不要自己写第二套。

（对应版本：v2.9.x（`Tooltip` 触发 ref 统一实现已在用）；v3.0.0（浮层组件规范进入门禁口径））

### 6. 主题与 Tailwind 4 / Monaco 白名单

主题走 Ant Design theme token + Tailwind 工具类，**不新建** `tailwind.config.js` / `postcss.config.js`；Tailwind 4 用 `@tailwindcss/vite` 插件（`vite.config.ts:1`、`:117`）。主题 token 在 v3.0.0 统一收口（`ff9c46f39`，`CHANGELOG.md:195`），新增颜色 / 圆角优先用 token，不要写死色值。

Monaco 的语法高亮是**白名单制**：新增语言必须在 `vite.config.ts` 的 `SHIKI_ALLOWED_LANGS` 里加，否则不会被打包（`vite.config.ts:32`）。这不会报编译错，只会在运行时表现为「语言识别了但没有高亮」。

（对应版本：v3.0.0（主题 token 统一 + Monaco 高亮白名单收口，`ff9c46f39`、`CHANGELOG.md:195`））

## 关键代码入口

- 路由 / 顶栏：
  - `src/components/layout/ContentArea.tsx:3`（`AppHeader` import）、`:157`（`useNavigate`）、`:208`（`<AppHeader />`）、`:213-347`（路由表）
  - `src/components/layout/AppHeader.tsx:20`（股票工作台头）、`:45`（组件导出）、`:59-61`（按页面抑制）、`:68-69`（40px）
- i18n：
  - `src/i18n/index.ts:10-21`（10 种懒加载语言）、`:51-56`（`lng` / `fallbackLng`）、`:68-76`（`changeLanguage` 拦截）
  - `scripts/check-i18n-key-parity.mjs`、`scripts/check-i18n-exemption-budget.mjs`、`scripts/check-jsx-text-i18n.mjs`、`scripts/check-i18n-orphan-diff.mjs`
- 组件中立性：
  - `scripts/check-component-neutrality.mjs`
  - `scripts/component-neutrality-allowlist.json`、`scripts/component-neutrality-baseline.json`
- 浮层 ref：
  - `src/components/layout/Tooltip.tsx:77`、`:118-122`、`:162`
- 主题 / Monaco：
  - `vite.config.ts:1`、`:32`、`:117`

## 配置与开关

- 默认语言 `lng: "zh-CN"`，回退语言 `fallbackLng: "en-US"`（`src/i18n/index.ts:51-56`）。
- `partialBundledLanguages: true` 允许 resources 只带部分语言，其余运行时加载（`:59`）。
- 顶栏高度 40px（`src/components/layout/AppHeader.tsx:68-69`）。
- 组件中立性中性目录：`layout` / `common` / `shared` / `dynamicUI`（脚本 `NEUTRAL_PREFIXES`）。
- Monaco 语言白名单：`vite.config.ts` 的 `SHIKI_ALLOWED_LANGS`（`:32`）。
- Tailwind 4：仅 `@tailwindcss/vite` 插件，无独立配置文件（`vite.config.ts:117`）。

## 已知限制与坑

1. **`fallbackLng` 与规范不一致**：代码是 `en-US`，`AGENTS.md` 写「默认 / 回退均为 zh-CN」。当前 zh-CN 键集完整且门禁强制 11 语言对齐，所以 zh-CN 用户走不到回退路径；但**其它语言缺键时会落 en-US**，不是 zh-CN。改规范或改代码必须二选一并同步文档（待确认）。
2. **`AppHeader` 不是「全局一定有」**：对话页、股票页主动不渲染（`src/components/layout/AppHeader.tsx:59-61`）。新页面若假定顶栏存在来做定位 / 高度计算，会在这两类页面错位。
3. **动态路由在最后**：`src/components/layout/ContentArea.tsx:347` 是带参数通配；静态路由必须写在它之前，否则会被吞掉。
4. **组件中立性白名单键不含行号**：这是刻意设计（行号会漂移）；但意味着「同一 import 串」无法按行区分，登记前要确认整条 import 都合理。
5. **`--update-baseline` 不是洗白工具**：存在新违规时脚本拒绝执行；想靠它把新违规写进基线会直接失败（`scripts/check-component-neutrality.mjs`）。
6. **i18n 孤儿门只看 `git diff`**：未提交的删除、或变更面过大降级 report-only 时，孤儿 key 不会被拦；发版前仍需跑全量键集对齐。
7. **Monaco 白名单漏加不报错**：`SHIKI_ALLOWED_LANGS` 未收录的语言只会「无高亮」，不会编译失败。
8. **浮层 ref 透传不可省**：React 19 下 `findDOMNode` 已移除，未透传 ref 的自定义组件被浮层包裹时弹层定位直接失效，且是运行时才暴露。

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-frontend-information-architecture.md` | 待确认（无元信息块） | — | 导航模型、页面层级、路由映射、可达性保证；第 1 节的上游计划 |
| `PLAN-shell-plus-plugins.md` | active | v3.0.0+（未定版，待确认） | 主壳 + 按能力插件；与本文件同属外壳层，边界在「页面注册 vs 插件装载」 |
| `PLAN-repo-split.md` | active | v3.0.0+（未定版，待确认） | 目录迁移；`src/` 路径未变，但脚本与 CI 路径按拆分后口径执行 |
| `PROPOSAL-i18n-hardcoded-strings-cleanup-design.md` | 待审批 | — | i18n 硬编码清理设计；第 3 节门禁的上游依据 |

## 版本变更记录

| 版本 | 日期 | 该功能点在本版本改了什么 | 证据（提交短哈希或 CHANGELOG 版本节） |
|---|---|---|---|
| v2.9.6 | 2026-08-20 | 导航 / 页面注册重构、i18n 同步 | `CHANGELOG.md:734` |
| v2.9.7 | 2026-08-24 | 补全 store 用户可见文案的 9 个 i18n key 至全部 locale | `CHANGELOG.md:671` |
| v3.0.0 | 2026-10-12 | 任务中心 v2 统一运行视图接入 `ContentArea` 路由表 | `bd2532a71` |
| v3.0.0 | 2026-10-28 | 主题 token 统一 + 路由 / 首字节预算门禁 | `ff9c46f39`；`CHANGELOG.md:195` |
| v3.0.0 | 2026-10-29 | 路由门禁改为注释剥离感知字符串字面量 | `CHANGELOG.md:364` |
| v3.0.0 | 2026-10-31 | 新增组件中立性门禁（目录边界 + 白名单 + 212 条棘轮基线） | `d2652a980`；`CHANGELOG.md:197` |
| v3.0.0 | 2026-10-31 | 新增 i18n 三道门禁（豁免预算 / JSX 字面量 / 键集对齐） | `59533a05e`；`CHANGELOG.md:198` |
| v3.0.0 | 2026-10-31 | provider key 解密错误前端分支 i18n 收口 | `acedf88e2` |
| unreleased | 2026-11-01 | 代码按 8 个 workspace 根拆分（`src/` 路径不变） | `8dfaeb4db` |
