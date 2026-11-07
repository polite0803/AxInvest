// SPDX-License-Identifier: AGPL-3.0-only

import { useMcpStore } from "@/stores/feature/mcpStore";

/**
 * mcpStore 的 chat 域菜单/命令读面（CommandSuggest / McpMenu 两个 chat 组件
 * 共用，组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 mcpStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * mcpStore（组件中立性），统一从这里取。内部为逐条 selector 订阅
 * （与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致。
 */

/** MCP 工具描述符清单（CommandSuggest 专用，供 @ 提及建议列表）。 */
export function useMcpToolDescriptors() {
  return useMcpStore((s) => s.toolDescriptors);
}

/** MCP 服务器菜单面：清单 + 新建/更新动作引用（McpMenu 专用，组件侧按原名解构转发）。 */
export function useMcpServerMenu() {
  const servers = useMcpStore((s) => s.servers);
  const createServer = useMcpStore((s) => s.createServer);
  const updateServer = useMcpStore((s) => s.updateServer);
  return { servers, createServer, updateServer };
}
