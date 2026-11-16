// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 「插件页」工作台 Tab 的 pages 层薄中转。
 *
 * 为什么多这一层：组件中立性门禁（`scripts/check-component-neutrality.mjs`）禁止
 * `components/layout/**` 直引 `components/plugin/**`（后者不在 NEUTRAL_PREFIXES）。
 * 容器本体留在 `components/plugin/PluginPageExplorer.tsx`，本文件只做一层转发 ——
 * 与 WorkspaceHub 里其它 8 个 Tab 一律经 `@/pages/XxxPage` 挂载的形态保持同构。
 */
import { PluginPageExplorer } from "@/components/plugin/PluginPageExplorer";

export function PluginPageExplorerPage() {
  return <PluginPageExplorer />;
}
