// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件声明的 `ui.json` key → 动态 UI 库里的**行 id**。
 *
 * 后端行 id 恒为 `plugin.{pluginId}.{key}`（`axagent-plugins/crates/plugins/src/ui_schema.rs`
 * 的 `row_id`）；插件清单只带 **key**（如 `order-form`），拼全 id 归宿主 —— 跨插件引用因此
 * 在声明面上就写不出来。
 *
 * 前端有两处消费，必须共用这一份，各自内联拼串就会长出第三套口径：
 * - 任务中心详情槽（`TasksCenterPage/adapters/pluginTaskSource.adapter.ts` 的 `detailSchema`）；
 * - 插件 schema 页（`components/plugin/PluginPageHost.tsx` 的 `pages[].entry`）。
 *
 * key 的合法形状不含 `.`（后端 `is_declared_key_shape`），所以恒拼一次既确定又不会双前缀。
 */
export function pluginSchemaRowId(pluginId: string, key: string): string {
  return `plugin.${pluginId}.${key}`;
}
