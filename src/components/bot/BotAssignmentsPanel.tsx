// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 详情页「委派」区（P2-b T5''）。
 *
 * 数据面走 IPC-P2B §1 冻结契约：`create_bot_assignment` / `list_bot_assignments` /
 * `cancel_bot_assignment`。consult 由前端禁用（IPC-P2B §2.3：consult 是 assignee
 * 轮内工具语义，person 只发 dispatch），mode 固定 dispatch。
 *
 * 刷新路径（PLAN-P2B §5 ②）：`bot-assignment-updated` 事件到达即刷新 + 15s 轮询兜底
 * （与 BotRoutinesPanel 同节奏；事件经 invoke 层订阅——浏览器模式走内存事件总线，
 * e2e 可真实触发；Tauri 模式待后端 T4'' 枚举落定后 typecheck 收口）。
 */

import { backendErrorText } from "@/lib/errorI18n";
import { listen } from "@/lib/invoke";
import { useBotStore } from "@/stores/feature/botStore";
import type { Bot, BotAssignment, BotAssignmentStatus } from "@/types";
import type { IpcEventName } from "@/types/generated/events";
import { Alert, App, Badge, Button, Card, Empty, Form, Input, Popconfirm, Select, Spin, Tag, Typography } from "antd";
import { Plus, Send } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;
const { TextArea } = Input;

/** 列表自动刷新间隔（与 BotRoutinesPanel 同节奏）。 */
const AUTO_REFRESH_INTERVAL_MS = 15_000;

/**
 * IPC-P2B §5 事件名（冻结字面量）。后端 T4'' 把 `BotAssignmentUpdated` 落进
 * `IpcEventName` 枚举后，schema-gen 生成物即含此名，这里的断言收窄为零成本
 * （类型来源切换到生成物，字面量不动）。
 */
export const BOT_ASSIGNMENT_UPDATED_EVENT = "bot-assignment-updated" as IpcEventName;

export interface BotAssignmentsPanelProps {
  /** 发起方视角的 Bot（列表 = 该 Bot 发起 + 指派给该 Bot 的单）。 */
  bot: Bot;
  /** 全量 Bot 清单（assignee 下拉）。 */
  bots: Bot[];
}

/** 状态 → 徽章色（在途蓝、终态绿灰、失败红）。导出供单测直断映射。 */
export function statusBadgeColor(
  status: BotAssignmentStatus,
): "default" | "processing" | "success" | "error" | "warning" {
  switch (status) {
    case "queued":
      return "default";
    case "running":
      return "processing";
    case "completed":
      return "success";
    case "failed":
      return "error";
    case "cancelled":
      return "warning";
  }
}

/** 状态 → i18n 键（statusQueued / statusRunning / …）。导出供单测直断映射。 */
export function statusI18nKey(status: BotAssignmentStatus): string {
  return `bot.assignment.status${status.charAt(0).toUpperCase()}${status.slice(1)}`;
}

/** 终态（取消按钮禁用判据，IPC-P2B §1：queued/running 可取消）。 */
export function isTerminalStatus(status: BotAssignmentStatus): boolean {
  return status === "completed" || status === "failed" || status === "cancelled";
}

interface AssignmentFormValues {
  assigneeBotId: string;
  taskText: string;
}

export function BotAssignmentsPanel({ bot, bots }: BotAssignmentsPanelProps) {
  const { t } = useTranslation();
  const { message: antdMessage } = App.useApp();
  const [form] = Form.useForm();
  const createBotAssignment = useBotStore((s) => s.createBotAssignment);
  const listBotAssignments = useBotStore((s) => s.listBotAssignments);
  const cancelBotAssignment = useBotStore((s) => s.cancelBotAssignment);

  const [assignments, setAssignments] = useState<BotAssignment[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [sending, setSending] = useState(false);
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [formCollapsed, setFormCollapsed] = useState(true);

  const assigneeOptions = useMemo(
    () =>
      bots
        .filter((candidate) => candidate.id !== bot.id)
        .map((candidate) => ({ value: candidate.id, label: candidate.name })),
    [bots, bot.id],
  );

  const load = useCallback(async () => {
    try {
      const rows = await listBotAssignments({ botId: bot.id });
      setAssignments(Array.isArray(rows) ? rows : []);
      setFailed(false);
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  }, [bot.id, listBotAssignments]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setFailed(false);
    setAssignments([]);
    void load();
    const timer = window.setInterval(() => {
      if (alive) {
        void load();
      }
    }, AUTO_REFRESH_INTERVAL_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [load]);

  // IPC-P2B §5：`bot-assignment-updated` 事件到达即刷新（轮询兜底，与例程面板同构）。
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | null = null;
    void listen<BotAssignment>(BOT_ASSIGNMENT_UPDATED_EVENT, () => {
      if (alive) {
        void load();
      }
    })
      .then((fn) => {
        if (alive) {
          unlisten = fn;
        } else {
          fn();
        }
      })
      .catch(() => {
        // 订阅失败不阻断列表（轮询兜底）
      });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [load]);

  const handleSend = useCallback(
    async (values: AssignmentFormValues) => {
      setSending(true);
      try {
        await createBotAssignment({
          assigneeBotId: values.assigneeBotId,
          taskText: values.taskText,
          mode: "dispatch",
        });
        antdMessage.success(t("bot.assignment.create"));
        form.resetFields();
        setFormCollapsed(true);
        await load();
      } catch (e) {
        antdMessage.error(backendErrorText(e));
      } finally {
        setSending(false);
      }
    },
    [createBotAssignment, form, load, t, antdMessage],
  );

  const handleCancel = useCallback(
    async (assignment: BotAssignment) => {
      setBusy((prev) => ({ ...prev, [assignment.id]: true }));
      try {
        await cancelBotAssignment(assignment.id);
        await load();
      } catch (e) {
        antdMessage.error(backendErrorText(e));
      } finally {
        setBusy((prev) => ({ ...prev, [assignment.id]: false }));
      }
    },
    [cancelBotAssignment, load, antdMessage],
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }} data-testid="bot-assignments-panel">
      <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
        <Button
          size="small"
          icon={<Plus size={14} />}
          onClick={() => setFormCollapsed((v) => !v)}
          data-testid="bot-assignment-toggle-form"
        >
          {t("bot.assignment.create")}
        </Button>
      </div>

      {!formCollapsed
        ? (
          <Card size="small" data-testid="bot-assignment-form">
            <Form<AssignmentFormValues>
              form={form}
              layout="vertical"
              onFinish={(values) => {
                void handleSend(values);
              }}
            >
              <Form.Item
                name="assigneeBotId"
                label={t("bot.assignment.assignee")}
                rules={[{ required: true, message: t("bot.assignment.taskRequired") }]}
              >
                <Select
                  placeholder={t("bot.assignment.assignee")}
                  options={assigneeOptions}
                  notFoundContent={t("bot.department.chiefEmpty")}
                  data-testid="bot-assignment-assignee"
                />
              </Form.Item>
              <Form.Item
                name="taskText"
                label={t("bot.assignment.task")}
                rules={[
                  { required: true, message: t("bot.assignment.taskRequired") },
                  { whitespace: true, message: t("bot.assignment.taskRequired") },
                ]}
              >
                <TextArea rows={3} maxLength={20000} data-testid="bot-assignment-task" />
              </Form.Item>
              <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 8 }}>
                <Tag data-testid="bot-assignment-mode">{t("bot.assignment.modeDispatch")}</Tag>
                <Button
                  type="primary"
                  size="small"
                  icon={<Send size={14} />}
                  loading={sending}
                  htmlType="submit"
                  data-testid="bot-assignment-send"
                >
                  {t("bot.assignment.send")}
                </Button>
              </div>
            </Form>
          </Card>
        )
        : null}

      <Card size="small" title={t("bot.assignment.title")} data-testid="bot-assignments-list">
        {loading
          ? <Spin size="small" />
          : failed
          ? <Alert type="error" showIcon message={t("error.loadFailed")} />
          : assignments.length === 0
          ? <Empty description={t("bot.assignment.empty")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
          : (
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              {assignments.map((assignment) => (
                <div
                  key={assignment.id}
                  data-testid="bot-assignment-item"
                  data-status={assignment.status}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: 4,
                    padding: "8px 10px",
                    borderRadius: 8,
                    border: "1px solid rgba(128,128,128,0.25)",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                    <Badge
                      status={statusBadgeColor(assignment.status)}
                      text={
                        <Text strong style={{ fontSize: 13 }}>
                          {t(statusI18nKey(assignment.status))}
                        </Text>
                      }
                    />
                    <Text type="secondary" ellipsis style={{ fontSize: 12, flex: 1 }}>
                      {`${t("bot.assignment.assignee")}: ${assignment.assigneeName}`}
                    </Text>
                    {!isTerminalStatus(assignment.status)
                      ? (
                        <Popconfirm
                          title={t("bot.assignment.cancel")}
                          okText={t("bot.assignment.cancel")}
                          cancelText={t("common.cancel")}
                          okButtonProps={{ danger: true, loading: busy[assignment.id] === true }}
                          onConfirm={() => {
                            void handleCancel(assignment);
                          }}
                        >
                          <Button
                            size="small"
                            danger
                            disabled={busy[assignment.id] === true}
                            data-testid="bot-assignment-cancel"
                          >
                            {t("bot.assignment.cancel")}
                          </Button>
                        </Popconfirm>
                      )
                      : null}
                  </div>
                  <Text ellipsis={{ tooltip: assignment.taskText }} style={{ fontSize: 12 }}>
                    {assignment.taskText}
                  </Text>
                  {assignment.resultText
                    ? (
                      <Text
                        type="secondary"
                        ellipsis={{ tooltip: assignment.resultText }}
                        style={{ fontSize: 12 }}
                      >
                        {assignment.resultText}
                      </Text>
                    )
                    : null}
                  {assignment.errorDetail
                    ? (
                      <Text type="danger" ellipsis={{ tooltip: assignment.errorDetail }} style={{ fontSize: 12 }}>
                        {assignment.errorDetail}
                      </Text>
                    )
                    : null}
                </div>
              ))}
            </div>
          )}
      </Card>
    </div>
  );
}
