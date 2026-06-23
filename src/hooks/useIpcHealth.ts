// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState } from "react";
import {
  onIpcHealthChange,
  recoverIpcConnection,
  startIpcHeartbeat,
  stopIpcHeartbeat,
} from "@/lib/invoke";

/** 应用启动时调用一次，启动 IPC 心跳。 */
export function useIpcHeartbeat() {
  useEffect(() => {
    startIpcHeartbeat();
    return () => stopIpcHeartbeat();
  }, []);
}

/**
 * 监听 IPC 健康状态 + 窗口焦点/可见性变化时自动尝试恢复连接。
 *
 * 返回当前健康状态（undefined=初始未知, true=健康, false=断开）。
 */
export function useIpcHealth() {
  const [healthy, setHealthy] = useState<boolean | undefined>(undefined);

  useEffect(() => {
    // 1. 初始健康检查
    import("@/lib/invoke").then(({ checkIpcHealth }) => {
      checkIpcHealth().then((h) => setHealthy(h.ok));
    });

    // 2. 监听心跳状态变化
    const unsub = onIpcHealthChange((ok) => {
      setHealthy(ok);
    });

    // 3. 窗口焦点恢复时检查连接
    const handleFocus = () => {
      recoverIpcConnection().then((ok) => setHealthy(ok));
    };
    window.addEventListener("focus", handleFocus);

    // 4. 页面可见性变化（从后台切回时）检查连接
    const handleVisibility = () => {
      if (document.visibilityState === "visible") {
        recoverIpcConnection().then((ok) => setHealthy(ok));
      }
    };
    document.addEventListener("visibilitychange", handleVisibility);

    return () => {
      unsub();
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);

  return healthy;
}
