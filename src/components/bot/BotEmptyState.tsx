// SPDX-License-Identifier: AGPL-3.0-only

/** Bot 列表空态：无 Bot 时的首屏引导。 */

import { Button, Empty, Typography } from "antd";
import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

export interface BotEmptyStateProps {
  onCreate: () => void;
}

export function BotEmptyState({ onCreate }: BotEmptyStateProps) {
  const { t } = useTranslation();

  return (
    <div
      data-testid="bot-empty-state"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 12,
        padding: 32,
      }}
    >
      <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("bots.empty")} />
      <Text type="secondary" style={{ textAlign: "center", maxWidth: 420 }}>
        {t("bots.emptyDescription")}
      </Text>
      <Button type="primary" icon={<Plus size={16} />} onClick={onCreate}>
        {t("bots.createFirst")}
      </Button>
    </div>
  );
}
