// SPDX-License-Identifier: AGPL-3.0-only

import { useCapabilityStore } from "@/stores/feature/capabilityStore";
import { usePluginStore } from "@/stores/feature/pluginStore";

/**
 * 插件市场读面：pluginStore（清单 + 安装/启停/卸载）与 capabilityStore（能力注册表）。
 *
 * 抽成 hook 是为了让 chat（PluginMarketplace）组件不直接依赖这两个 store
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 * 返回形态与原组件的 selector 完全一致，不改任何逻辑。
 */
export function usePluginGallery() {
  // 从 usePluginStore 获取状态和方法
  const plugins = usePluginStore((s) => s.plugins);
  const loading = usePluginStore((s) => s.loading);
  const installing = usePluginStore((s) => s.installing);
  const validating = usePluginStore((s) => s.validating);
  const loadPlugins = usePluginStore((s) => s.loadPlugins);
  const validateSource = usePluginStore((s) => s.validateSource);
  const installPlugin = usePluginStore((s) => s.installPlugin);
  const enablePlugin = usePluginStore((s) => s.enablePlugin);
  const disablePlugin = usePluginStore((s) => s.disablePlugin);
  const uninstallPlugin = usePluginStore((s) => s.uninstallPlugin);

  // 能力注册表（capability_registry_dump，缺陷 #6：前端消费后端检视命令）
  const registry = useCapabilityStore((s) => s.registry);
  const registryLoading = useCapabilityStore((s) => s.isLoading);
  const loadRegistry = useCapabilityStore((s) => s.loadRegistry);

  return {
    plugins,
    loading,
    installing,
    validating,
    loadPlugins,
    validateSource,
    installPlugin,
    enablePlugin,
    disablePlugin,
    uninstallPlugin,
    registry,
    registryLoading,
    loadRegistry,
  };
}
