// SPDX-License-Identifier: AGPL-3.0-only

// 通知铃铛 — 显示 Agent 生命周期通知和未读计数
// 数据读写统一走 lib/notification（写入受通知偏好闸门控制）

/* eslint-disable react-refresh/only-export-components */
import { DropdownMenu } from "@/components/layout/DropdownMenu";
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard";
import {
  addNotification,
  clearAllNotifications,
  dismissNotification,
  filterNotificationsByBot,
  getNotifications,
  listNotificationBotIds,
  markAllAsRead,
  readNotification,
  type Notification,
} from "@/lib/notification";
import { BellOutlined } from "@ant-design/icons";
import { Badge, Empty, theme, Typography } from "antd";
import { Check, CheckCheck, Copy, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

const { Text } = Typography;

/** 推送一条通知（兼容旧 API：归属字段可选，缺省行为与 P1 完全一致） */
export function pushNotification(
  type: Notification["type"],
  message: string,
  attribution?: { botId?: string | null; conversationId?: string | null },
) {
  addNotification({
    type,
    title: message,
    // 事件 payload 带了 conversationId 才写归属；botId 解析不到（映射查不到）时
    // 不写该字段，通知仍落 —— IPC-P2 §4。
    ...(attribution?.conversationId ? { conversationId: attribution.conversationId } : {}),
    ...(attribution?.botId ? { botId: attribution.botId } : {}),
  });
}

export function NotificationBell() {
  const [notifications, setNotifications] = useState<Notification[]>(getNotifications);
  const [open, setOpen] = useState(false);
  const { t, i18n } = useTranslation();
  const { token } = theme.useToken();
  const { copy } = useCopyToClipboard();
  const navigate = useNavigate();
  const [copiedId, setCopiedId] = useState<string | null>(null);
  /** 归属过滤：null = 全部（P1 行为；旧通知无 botId 也在这里可见） */
  const [botFilter, setBotFilter] = useState<string | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // 监听 CustomEvent 实时更新
  const handleNotificationEvent = useCallback(() => {
    setNotifications(getNotifications());
  }, []);

  useEffect(() => {
    window.addEventListener("axagent:notification", handleNotificationEvent);
    return () => {
      window.removeEventListener("axagent:notification", handleNotificationEvent);
    };
  }, [handleNotificationEvent]);

  // 过滤与徽章都按归属口径：botFilter=null 计全部（IPC-P2 §4「旧通知不丢」）
  const visibleNotifications = filterNotificationsByBot(notifications, botFilter);
  const unreadCount = visibleNotifications.filter((n) => !n.read).length;
  const botFilterOptions = listNotificationBotIds(notifications);

  const handleMarkAllRead = useCallback(() => {
    setNotifications(markAllAsRead());
  }, []);

  const handleClearAll = useCallback(() => {
    setNotifications(clearAllNotifications());
  }, []);

  /** 逐条点击已读 + 跳转到产生通知的线程（/chat?conversationId= 为 ChatPage 挂载时消费的契约参数）。 */
  const handleOpenThread = useCallback(
    (n: Notification) => {
      readNotification(n.id);
      setNotifications(getNotifications());
      if (n.conversationId) {
        setOpen(false);
        navigate(`/chat?conversationId=${encodeURIComponent(n.conversationId)}`);
      }
    },
    [navigate],
  );

  /** 删除单条（铃铛逐条清理入口；localStorage 同步更新） */
  const handleDismiss = useCallback((id: string) => {
    setNotifications(dismissNotification(id));
  }, []);

  const handleCopy = useCallback(
    async (n: Notification) => {
      const text = [n.title, n.message].filter(Boolean).join("\n");
      const ok = await copy(text);
      if (!ok) {
        return;
      }
      setCopiedId(n.id);
      if (copyTimerRef.current) {
        clearTimeout(copyTimerRef.current);
      }
      copyTimerRef.current = setTimeout(() => {
        setCopiedId((cur) => (cur === n.id ? null : cur));
      }, 1500);
    },
    [copy],
  );

  useEffect(() => {
    return () => {
      if (copyTimerRef.current) {
        clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  const actionItems = notifications.length === 0 ? [] : [
    ...(botFilterOptions.length > 0
      ? [
        { key: "filter-all", label: <span>{t("notification.allBots")}</span>, onClick: () => setBotFilter(null) },
        ...botFilterOptions.map((id) => ({
          key: `filter-bot-${id}`,
          label: <span>{t("notification.filterByBot")} · {id.slice(0, 8)}</span>,
          onClick: () => setBotFilter(id),
        })),
        { key: "divider-filter", divider: true },
      ]
      : []),
    {
      key: "markAllRead",
      label: <span>{t("notification.markAllRead")}</span>,
      icon: <CheckCheck size={14} />,
      onClick: handleMarkAllRead,
      disabled: unreadCount === 0,
    },
    {
      key: "clearAll",
      label: <span style={{ color: token.colorError }}>{t("notification.clear")}</span>,
      icon: <Trash2 size={14} color={token.colorError} />,
      onClick: handleClearAll,
      danger: true,
    },
    { key: "divider-actions", divider: true },
  ];

  const notificationItems = visibleNotifications.length === 0
    ? [
      {
        key: "empty",
        label: (
          <Empty
            description={t("notification.empty")}
            image={Empty.PRESENTED_IMAGE_SIMPLE}
          />
        ),
        disabled: true,
      },
    ]
    : visibleNotifications.slice(0, 20).map((n) => ({
      key: n.id,
      label: (
        <div
          role={n.conversationId ? "button" : undefined}
          tabIndex={n.conversationId ? 0 : undefined}
          onClick={n.conversationId ? () => handleOpenThread(n) : undefined}
          onKeyDown={n.conversationId
            ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                handleOpenThread(n);
              }
            }
            : undefined}
          style={{
            display: "flex",
            alignItems: "flex-start",
            gap: 8,
            maxWidth: 320,
            padding: "4px 0",
            cursor: n.conversationId ? "pointer" : undefined,
          }}
        >
          <div style={{ flex: 1, minWidth: 0 }}>
            <Text
              style={{
                fontSize: 12,
                color: n.type === "error"
                  ? token.colorError
                  : n.type === "warning"
                  ? token.colorWarning
                  : token.colorSuccess,
              }}
            >
              {n.type === "error" ? "❌" : n.type === "warning" ? "⚠️" : "✅"} {n.title}
            </Text>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {new Date(n.timestamp).toLocaleTimeString(i18n.language)}
              </Text>
              {n.conversationId && (
                <Text
                  type="secondary"
                  style={{ fontSize: 12, textDecoration: "underline" }}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleOpenThread(n);
                  }}
                >
                  {t("notification.viewThread")}
                </Text>
              )}
              <span
                role="button"
                tabIndex={0}
                aria-label={t("notification.clear")}
                title={t("notification.clear")}
                onClick={(e) => {
                  e.stopPropagation();
                  handleDismiss(n.id);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.stopPropagation();
                    e.preventDefault();
                    handleDismiss(n.id);
                  }
                }}
                style={{ color: token.colorTextQuaternary, fontSize: 11, cursor: "pointer" }}
              >
                ✕
              </span>
            </div>
          </div>
          <span
            role="button"
            tabIndex={0}
            aria-label={t("common.copy")}
            title={t("common.copy")}
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
              void handleCopy(n);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.stopPropagation();
                e.preventDefault();
                void handleCopy(n);
              }
            }}
            style={{
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              flexShrink: 0,
              marginTop: 2,
              color: copiedId === n.id
                ? token.colorSuccess
                : token.colorTextQuaternary,
            }}
          >
            {copiedId === n.id ? <Check size={13} /> : <Copy size={13} />}
          </span>
        </div>
      ),
    }));

  const items = [...actionItems, ...notificationItems];

  return (
    <DropdownMenu items={items} open={open} onOpenChange={setOpen} trigger={["click"]}>
      <Badge count={unreadCount} size="small" offset={[-2, 2]}>
        <BellOutlined style={{ fontSize: 16, cursor: "pointer", padding: 4 }} />
      </Badge>
    </DropdownMenu>
  );
}
