# AUDIT — 滚动指示器跨主题不可见（2026-09-14）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


> 承接 `AUDIT-invest-scroll-2026-09-14.md`：上一轮修好了「内容被裁、无法滚动」，
> 本轮处理紧随其后的第二个缺陷 —— **滚动条本身在多数主题下几乎看不见**。

## 1. 现象

投资中心内容区滚动生效后，滚动指示器在多种主题下都难以辨认（用户原话：
「换了几种主题都极难发现这个指示器」）。

## 2. 根因

**全站没有任何滚动条样式**（`::-webkit-scrollbar` 仅出现在 21th 预设、输入框、
以及若干刻意 `display: none` 的横向 tab 条上）。未声明时滚动条完全由浏览器/系统决定，
**颜色不跟随应用主题**，于是三种形态都不可用：

| 形态 | 触发条件 | 实测 thumb 与背景对比度 |
|---|---|---|
| Fluent / overlay（WebView2 默认） | 不占布局宽度、自动淡出 | **1.05:1**（深色）/ 3.25:1（浅色） |
| Windows classic | 占位 15px、轨道用系统色 | thumb vs 轨道 **1.17:1**（深色主题下轨道是近白 `#fcfcfc`） |
| 21th 预设自定义 | `[data-theme-preset*="21th"]` | **1.9:1**（21th-dark，`--border` 过暗）/ **1.5:1**（21th-light，`--border` 是纯白压在浅灰底上） |

WCAG 非文本对比度下限为 3:1，上述全部不达标。

第二处独立根因：**OverlayScrollbars 的 handle 用 `--accent-deco` 的 8% 透明度**
（`index.css` 原 `.os-theme-axagent`），实测 handle 与背景对比度 **1.05:1**；
叠加 `autoHide: "scroll"`（滚动后 600ms 淡出），凡是走 `.overflow-y-auto` /
`[data-os-scrollbar]` 的容器同样"看不见滚动条"。

第三处：`AgentExecutionPanel.css` 的 thumb 用 `var(--color-fill-tertiary)`，
**该变量全项目无定义**（只有消费端、没有定义端）⇒ 属性失效退回浏览器默认色。

## 3. 量化证据（有头浏览器，视口 1072×679，注入 3000px 内容）

统一取 `--scrollbar-thumb = color-mix(in srgb, var(--fg-2) 80%, transparent)` 后：

| 主题 | 修复前对比度 | 修复后对比度 | 滚动条宽 | thumb 实测色 / 背景 |
|---|---|---|---|---|
| deep-dusk（默认） | 1.05 | **5.86** | 10px 常驻 | `145,142,138` / `18,15,12` |
| light-dawn | 3.25 | **4.12** | 10px 常驻 | `120,116,111` / `244,241,237` |
| 21th-dark | 1.9 | **6.61** | 10px 常驻 | `149,149,149` / `10,10,10` |
| 21th-light | 1.5 | **3.72** | 10px 常驻 | `96,97,97` / `197,201,201` |
| oceanic-dark | 1.06 | **5.94** | 10px 常驻 | `137,142,147` / `8,12,16` |
| cyberpunk-dark | 1.05 | **5.86** | 10px 常驻 | `145,142,138` / `18,15,12` |

OverlayScrollbars 容器（`.overflow-y-auto`，handle 走 `--os-handle-bg`）同样达标：
deep-dusk **5.86** / light-dawn **4.12** / 21th-dark **6.61**。

采样自检：thumb 像素高度实测 69~71px，与理论值 `clientH² / scrollH = 71px` 吻合，
确认采到的是 thumb 而非轨道。

### ⚠ 测量方法本身踩过两个坑（判据 #162）

1. **headless 浏览器不渲染滚动条**（Playwright 默认带 `--hide-scrollbars`）：
   同一份页面在 headless 下四组候选样式实测结果**完全恒等**，`offsetWidth-clientWidth`
   恒为 0，看起来像"所有方案都无效"。改用 headed 后立刻区分出差异
   （宽度 15→10、thumb 由系统色变为主题色）。
2. **`scrollbar-width` / `scrollbar-color` 与 `::-webkit-scrollbar` 互斥**：
   Chromium 中标准属性优先，一旦声明前者，后者整套失效。项目原
   `.axagent-input-textarea { scrollbar-width: thin !important; … }` 正是这种自废武功的写法。

## 4. 改动

| 文件 | 改动 |
|---|---|
| `src/index.css` `:root` | 新增 `--scrollbar-thumb` / `--scrollbar-thumb-hover`，全站滚动指示器的**唯一颜色来源** |
| `src/index.css` 全局 | 新增 `::-webkit-scrollbar` / `-track` / `-thumb` / `-thumb:hover` / `-corner`；宽度 10px、圆角 5px、轨道透明 |
| `src/index.css` 21th 预设 | 删掉自带颜色（对比度 1.5~1.9 不达标），只保留方角风格 |
| `src/index.css` 输入框 | 移除自废武功的 `scrollbar-width/color`，thumb 改用同一变量 |
| `src/index.css` `.os-theme-axagent` | handle 三态色由 `--accent-deco` 8%/18%/28% → `--scrollbar-thumb` / `--scrollbar-thumb-hover` |
| `src/index.css` `.chat-scroll-indicator` | 自绘指示器改用同一变量（同一提示在不同主题下亮度一致） |
| `src/components/chat/AgentExecutionPanel.css` | thumb 由悬空变量 `--color-fill-tertiary` → `--scrollbar-thumb` |

设计取舍：颜色选 `--fg-2` 混 80% 而不是 `--muted` —— 后者在 21th-light（浅灰底
oklch 83%）上仅 1.9:1，`--fg-2` 在全部 14 个预设下均 ≥ 3.3:1。

## 5. 验证

- 6 个主题实测对比度 3.72 ~ 6.61，全部 ≥ 3:1；OS 容器 3 个主题同样达标
- `sbWidth` 恒为 10px ⇒ 滚动条**常驻占位**（不再依赖自动淡出），跨主题形态一致
- 截图人工复核：深色为浅灰圆角竖条、浅色为中灰圆角竖条，均已清晰可辨
- 刻意隐藏滚动条的位置不受影响（`.nav-sidebar` / `.title-bar-drag` / `.msg-list-scroll-box`
  用 `scrollbar-width: none`，标准属性会覆盖全局自定义，语义保持）
- CSS 无 TS/类型面影响；dprint 不覆盖 CSS（`dprint check src/index.css` 报 no files）

## 6. 未处理 / 待裁决

1. **滚动条改为常驻占位后内容区窄 10px**（原 overlay 形态不占位）。这是"能看见"的
   必然代价；若嫌宽可降到 8px，需产品定。
2. **OverlayScrollbars 的 `autoHide: "scroll"`（600ms 淡出）未动** —— 属行为设计，
   不在本次颜色修复范围；若希望 handle 常驻，改 `useGlobalOverlayScrollbars.ts` 的
   `autoHide` 即可。
3. **悬空 CSS 变量（同类缺陷，未修）**：`--color-fill-tertiary` / `--color-fill-secondary`
   全项目只有消费端、无定义端，涉及 `WorkflowExecutor.tsx:470`、`NLParserPanel.tsx:108`、
   `IndustryTabLayout.tsx:294`、`Sidebar.tsx:155`、`FrontendEditorModal.tsx:170`、
   `TaskPanel.tsx:388` 等 —— 属性静默失效（`TrajectoryReplay.css` 有 fallback 故正常）。
4. 本轮实测环境为 Chromium/Edge 152（与 WebView2 Runtime 152 同内核），
   建议在真实 Tauri 窗口内对 21th-dark / light-dawn 各扫一眼确认。
