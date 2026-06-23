// SPDX-License-Identifier: AGPL-3.0-only

import { recoverIpcConnection } from "@/lib/invoke";
import { Alert, Button, Spin } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  /** false=断开, true=健康, undefined=初始加载 */
  healthy: boolean | undefined;
}

/**
 * IPC 断连时显示的横幅，提供"重试连接"按钮。
 * 通常放在页面顶部，不影响其他组件。
 */
export function IpcReconnectBanner({ healthy }: Props) {
  const { t } = useTranslation();
  const [reconnecting, setReconnecting] = useState(false);

  if (healthy !== false) { return null; }

  const handleRetry = async () => {
    setReconnecting(true);
    try {
      await recoverIpcConnection();
    } finally {
      setReconnecting(false);
    }
  };

  return (
    <Alert
      type="error"
      showIcon
      message={t("ipc.disconnected")}
      description={t("ipc.disconnectedHint")}
      action={
        <Button size="small" onClick={handleRetry} disabled={reconnecting}>
          {reconnecting
            ? (
              <>
                <Spin size="small" /> {t("ipc.reconnecting")}
              </>
            )
            : t("ipc.retry")}
        </Button>
      }
      closable={false}
      style={{ marginBottom: 8 }}
    />
  );
}
