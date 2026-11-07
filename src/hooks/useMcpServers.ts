// SPDX-License-Identifier: AGPL-3.0-only

import { useMcpStore } from "@/stores/feature/mcpStore";
import { useEffect } from "react";

/**
 * MCP 服务器清单的挂载期预热（自 InputArea 抽出，组件中立性消键）：
 * 清单为空时拉取一次（空依赖 effect，自 InputArea 逐字搬入）。
 *
 * 活源是 mcpStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * mcpStore（组件中立性），统一从这里取。
 */
export function useMcpServersBootstrap(): void {
  useEffect(() => {
    if ((useMcpStore.getState().servers ?? []).length === 0) {
      useMcpStore.getState().loadServers();
    }
  }, []);
}
