// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 薄 re-export —— 实现已于 2026-11-07（任务 #193，组件中立性 B 类）下沉到
 * `src/components/shared/ChartPreview.tsx`。
 *
 * 为什么搬：本组件被 `shared/ChartRenderer`（中性目录）与 `viz/VizBlockRenderer`
 * 两个方向引用；`shared/` 反向 import 特性目录会触发组件中立性门禁。组件本身只依赖
 * hooks / lib / theme，不读任何 feature store ⇒ 属跨 feature 共用基元。
 *
 * 为什么保留本文件：`docs/plans/PLAN-shell-plus-plugins.md` 仍按
 * `ArtifactPreview/ChartPreview.tsx:33` 记录「iframe + srcdoc 隔离渲染」的先例位置；
 * 直接删文件会让 `check-doc-refs` 判 BROKEN。文档改指属另一批（docs 不在本任务可改范围）。
 *
 * 实现要点（详情见新位置）：
 *   · echarts option → iframe srcdoc 的渲染壳，图例色板取宿主类目色板并按明暗档解析；
 *   · iframe 是独立文档，宿主 CSS 变量到不了 ⇒ 底色/前景在宿主侧解析成具体色值后注入；
 *   · `sandbox="allow-scripts"`，不授予 same-origin；脚本走 CDN + SRI 校验；
 *   · srcdoc 里那一处**裸 echarts 初始化调用**是 `src/lib/__tests__/echartsInstance.test.ts`
 *     「绕开 ensureECharts 的初始化站点清单」的登记豁免项（搬家后豁免路径已同步改到 shared/）。
 *
 * 早先为什么放在 chat/ArtifactPreview：它最初只服务聊天 artifact 预览的图表分支，
 * 后来 `viz` 的图表块面板复用同一实现（见 `VizBlockRenderer` 里的 `<ChartPreview />`），
 * 于是从「聊天内件」变成了跨域共用件 —— 这是搬迁的判据。
 *
 * ⚠ 本文件只剩转发，不再承载实现。新增代码请直接 import
 * `@/components/shared/ChartPreview`：中立性门禁判的是 import 字面量指向的目录，
 * 留在这里的引用会让「viz / shared 依赖 chat」这条账重新记上。
 *
 * ⚠ 若后续要删本 shim：必须同批把 `docs/plans/PLAN-shell-plus-plugins.md` 的那条引用
 * 改指 `src/components/shared/ChartPreview.tsx`，并重跑 `node scripts/check-doc-refs.mjs`。
 */
export { ChartPreview } from "@/components/shared/ChartPreview";
