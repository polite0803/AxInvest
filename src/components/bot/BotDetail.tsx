// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 详情壳：概览 + SOUL + 记忆 + 线程。
 *
 * 线程列表数据源是 `list_bot_threads`（Bot 归属的唯一权威），不是会话总表的本地过滤 ——
 * 这样「Bot 详情线程数 = 该 Bot 会话数」由后端过滤保证，不依赖前端缓存完整性。
 */

import { useBotStore } from "@/stores/feature/botStore";
import type { Bot, Conversation } from "@/types";
import { Alert, Button, Card, List, Popconfirm, Spin, Tabs, theme, Typography } from "antd";
import { Bot as BotIcon, MessageSquarePlus, Pencil, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BotMemoryPanel } from "./BotMemoryPanel";
import { BotSoulPanel } from "./BotSoulPanel";

const { Text, Title } = Typography;

/**
 * Bot 五档 → 会话三档展示映射（ADR-P2-4 的展示面：custom → Prompt ⇒ 会话侧
 * 最接近的是「默认」（逐轮确认），故映射到 permissionMode.readOnly 同级的保守档；
 * auto → workspaceWrite（Q1 建议值）。仅做 UI 展示，执行语义在后端解析。
 */
const APPROVAL_TO_SESSION_MODE: Record<Bot["approvalMode"], "readOnly" | "allow" | "workspaceWrite" | "fullAccess"> = {
  ask: "readOnly",
  edits: "workspaceWrite",
  auto: "workspaceWrite",
  full: "fullAccess",
  custom: "readOnly",
};

export interface BotDetailProps {
  bot: Bot;
  deleting?: boolean;
  onEdit: () => void;
  onDelete: () => void;
  onCreateThread: () => void;
  onOpenThread: (conversationId: string) => void;
  /**
   * P2-a：「例程」Tab 内容（页面层注入 —— 组件中立性门禁：components/bot 不得
   * import pages/**；未传则不渲染该 Tab，组件在无例程依赖场景保持可用）。
   */
  routinesPanel?: React.ReactNode;
  /**
   * P2-b：「委派」Tab 内容（页面层注入，组件中立性同 routinesPanel；未传则不渲染）。
   */
  assignmentsPanel?: React.ReactNode;
}

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", gap: 8, minWidth: 0 }}>
      <Text type="secondary" style={{ flexShrink: 0, width: 96 }}>
        {label}
      </Text>
      <Text ellipsis={{ tooltip: value }} style={{ flex: 1 }} copyable={false}>
        {value}
      </Text>
    </div>
  );
}

export function BotDetail({
  bot,
  deleting = false,
  onEdit,
  onDelete,
  onCreateThread,
  onOpenThread,
  routinesPanel,
  assignmentsPanel,
}: BotDetailProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const listThreads = useBotStore((s) => s.listThreads);
  const [threads, setThreads] = useState<Conversation[]>([]);
  const [threadsLoading, setThreadsLoading] = useState(true);
  const [threadsFailed, setThreadsFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    setThreadsLoading(true);
    setThreadsFailed(false);
    setThreads([]);
    listThreads(bot.id)
      .then((value) => {
        if (alive) {
          setThreads(value);
          setThreadsLoading(false);
        }
      })
      .catch(() => {
        if (alive) {
          setThreadsFailed(true);
          setThreadsLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [bot.id, listThreads]);

  const modelLabel = bot.providerId && bot.modelId ? `${bot.providerId} / ${bot.modelId}` : t("bots.modelPlaceholder");

  return (
    <div
      style={{ display: "flex", flexDirection: "column", gap: 12, minHeight: 0, overflowY: "auto", paddingRight: 4 }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 12 }}>
        <span style={{ fontSize: 32, lineHeight: 1 }}>{bot.icon ? bot.icon : <BotIcon size={30} />}</span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <Title level={4} style={{ margin: 0 }} ellipsis>
            {bot.name}
          </Title>
          {bot.title
            ? (
              <Text type="secondary" ellipsis>
                {bot.title}
              </Text>
            )
            : null}
        </div>
        <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
          <Button
            icon={<MessageSquarePlus size={16} />}
            type="primary"
            onClick={onCreateThread}
            data-testid="bot-new-thread"
          >
            {t("bots.newThread")}
          </Button>
          <Button icon={<Pencil size={16} />} onClick={onEdit} data-testid="bot-edit-btn">
            {t("common.edit")}
          </Button>
          <Popconfirm
            title={t("bots.deleteConfirm")}
            okText={t("bots.delete")}
            cancelText={t("common.cancel")}
            okButtonProps={{ danger: true, loading: deleting }}
            onConfirm={onDelete}
          >
            <Button danger icon={<Trash2 size={16} />} data-testid="bot-delete-btn">
              {t("bots.delete")}
            </Button>
          </Popconfirm>
        </div>
      </div>

      {bot.description
        ? (
          <Text type="secondary" style={{ whiteSpace: "pre-wrap" }}>
            {bot.description}
          </Text>
        )
        : null}

      <Card size="small" title={t("bots.title")}>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <MetaRow label={t("bots.role")} value={bot.title ?? "—"} />
          <MetaRow label={t("bots.model")} value={modelLabel} />
          <MetaRow label={t("bots.workdir")} value={bot.workdir ?? "—"} />
          <MetaRow label={t("bots.approvalMode")} value={t(`bots.approval.${bot.approvalMode}`)} />
          <MetaRow
            label={t("bots.approvalEffective")}
            value={t(`permissionMode.${APPROVAL_TO_SESSION_MODE[bot.approvalMode]}`)}
          />
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t("bots.approvalEffectiveHint")}
          </Text>
        </div>
      </Card>

      <BotSoulPanel botId={bot.id} />
      <BotMemoryPanel botId={bot.id} />

      <Tabs
        defaultActiveKey="threads"
        destroyInactiveTabPane={false}
        items={[
          {
            key: "threads",
            label: `${t("bots.threads")}${threads.length > 0 ? ` (${threads.length})` : ""}`,
            children: (
      <Card
        size="small"
        title={t("bots.threads")}
        data-testid="bot-threads-panel"
      >
        {threadsLoading
          ? <Spin size="small" />
          : threadsFailed
          ? <Alert type="error" showIcon message={t("error.loadFailed")} />
          : threads.length === 0
          ? (
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <Text type="secondary">{t("bots.threadsEmpty")}</Text>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("bots.threadsEmptyDescription")}
              </Text>
            </div>
          )
          : (
            <List
              size="small"
              dataSource={threads}
              renderItem={(conv) => (
                <List.Item
                  style={{ cursor: "pointer", padding: "6px 4px", borderRadius: 6 }}
                  onClick={() => onOpenThread(conv.id)}
                  data-testid="bot-thread-item"
                >
                  <div style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
                    <Text ellipsis>{conv.title}</Text>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {new Date(conv.updatedAt * 1000).toLocaleString()}
                    </Text>
                  </div>
                  <Text type="secondary" style={{ fontSize: 12, color: token.colorTextTertiary }}>
                    {conv.messageCount}
                  </Text>
                </List.Item>
              )}
            />
          )}
      </Card>
            ),
          },
          ...(routinesPanel
            ? [{ key: "routines", label: t("bots.tabRoutines"), children: routinesPanel }]
            : []),
          ...(assignmentsPanel
            ? [{ key: "assignments", label: t("bot.assignment.title"), children: assignmentsPanel }]
            : []),
        ]}
      />
    </div>
  );
}
