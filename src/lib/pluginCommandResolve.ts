// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import type { PluginSummaryDto } from "@/types";

/**
 * 按命令名反查「哪个启用中的插件提供它」。
 *
 * 为什么不让调用方写死 plugin id：命令是插件的数据（`plugin.json` 的 `commands[]`），
 * 插件名与市场来源（bundled / external）都会变；写死 `stock@bundled` 这类字面量会在
 * 换安装来源或改名后**静默失效**（调用方拿到 NotFound 而不是「没装插件」）。
 *
 * 为什么缓存：`plugin_list` 不是纯内存读 —— 宿主每次都会先做一轮内置插件落盘同步
 * （`sync_bundled_plugins`：目录比对 + 哈希），而这里被搜索框每次回车命中。
 * 缓存的是「命令 → 插件 id」。插件启停/卸载后由 `plugin-state-changed` 事件调用
 * [`invalidatePluginCommandCache`] 整体失效 —— 否则禁用后命令反查仍指向旧插件，
 * UI 会继续展示已不可用的命令入口（用户报告的「启停对 UI 无变化」形态之一）。
 */
const resolved = new Map<string, Promise<string | null>>();

/** 插件状态变更后清空反查缓存；下一次解析会重新拉 `plugin_list`。 */
export function invalidatePluginCommandCache(): void {
  resolved.clear();
}

export function resolvePluginForCommand(command: string): Promise<string | null> {
  const cached = resolved.get(command);
  if (cached) {
    return cached;
  }
  const pending = (async () => {
    try {
      const plugins = await invoke<PluginSummaryDto[]>("plugin_list");
      const hit = plugins.find((p) => p.enabled && p.commands.some((c) => c.name === command));
      return hit?.id ?? null;
    } catch {
      // 枚举失败（宿主未就绪 / 浏览器 mock 无插件）不缓存，下次再试。
      resolved.delete(command);
      return null;
    }
  })();
  resolved.set(command, pending);
  return pending;
}
