// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 列表（侧栏左列）—— 纯展示组件：数据与回调全部经 props 注入，
 * 不读任何 store（组合由 `pages/BotsPage` 负责）。
 */

import type { Bot } from "@/types";
import { Badge, Button, theme, Typography } from "antd";
import { Bot as BotIcon, Plus } from "lucide-react";
import type { KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

export interface BotListProps {
  bots: Bot[];
  activeBotId: string | null;
  /** P2-a：botId → 未读通知数（>0 时在列表项上渲染角标；页面层经 props 注入） */
  unreadByBot?: Map<string, number>;
  onSelect: (botId: string) => void;
  onCreate: () => void;
}

export function BotList({ bots, activeBotId, unreadByBot, onSelect, onCreate }: BotListProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>, botId: string) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(botId);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, minHeight: 0 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "0 4px" }}>
        <Text strong>{t("bots.title")}</Text>
        <Button type="text" size="small" icon={<Plus size={16} />} onClick={onCreate} data-testid="bot-create-btn">
          {t("bots.newBot")}
        </Button>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4, overflowY: "auto" }}>
        {bots.map((bot) => {
          const active = bot.id === activeBotId;
          return (
            <div
              key={bot.id}
              role="button"
              tabIndex={0}
              data-testid="bot-list-item"
              data-bot-id={bot.id}
              aria-pressed={active}
              onClick={() => onSelect(bot.id)}
              onKeyDown={(e) => handleKeyDown(e, bot.id)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "8px 10px",
                borderRadius: 8,
                cursor: "pointer",
                background: active ? token.colorFillSecondary : "transparent",
                border: `1px solid ${active ? token.colorBorderSecondary : "transparent"}`,
              }}
            >
              <Badge
                count={unreadByBot?.get(bot.id) ?? 0}
                size="small"
                offset={[-4, 4]}
                title={t("bots.unreadBadge", { count: unreadByBot?.get(bot.id) ?? 0 })}
              >
                <span style={{ display: "inline-flex", width: 20, justifyContent: "center", fontSize: 16 }}>
                  {bot.icon ? bot.icon : <BotIcon size={16} />}
                </span>
              </Badge>
              <span style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
                <Text ellipsis style={{ fontWeight: active ? 600 : 400 }}>
                  {bot.name}
                </Text>
                {bot.title
                  ? (
                    <Text type="secondary" ellipsis style={{ fontSize: 12 }}>
                      {bot.title}
                    </Text>
                  )
                  : null}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
