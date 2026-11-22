// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 详情页「例程」区（P2-a T4'）。
 *
 * 数据面走 T5' 冻结契约（IPC-P2 §2.1/§2.2）：`create_bot_scheduled_task` /
 * `list_bot_scheduled_tasks`，暂停/恢复/删除复用既有 scheduled_task 命令。
 * 本面板属页面层（pages/BotsPage/），可直读 botStore；不复用 TasksCenterPage
 * 内部模块，避免页面层相互 import（组件中立性门禁同源约束）。
 *
 * 不新建第 5 层 store：列表自持 useState + 15s 轮询（与 ScheduledJobsTab 同节奏）。
 */

import { backendErrorText } from "@/lib/errorI18n";
import { message } from "@/lib/toast";
import { useBotStore } from "@/stores/feature/botStore";
import type { ScheduledTask } from "@/types";
import { Button, Empty, List, Popconfirm, Switch, Tag, Typography } from "antd";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BotRoutineFormModal } from "./BotRoutineFormModal";

const { Text } = Typography;

const AUTO_REFRESH_INTERVAL_MS = 15_000;

export interface BotRoutinesPanelProps {
  botId: string;
}

export function BotRoutinesPanel({ botId }: BotRoutinesPanelProps) {
  const { t, i18n } = useTranslation();
  const listBotRoutines = useBotStore((s) => s.listBotRoutines);
  const pauseBotRoutine = useBotStore((s) => s.pauseBotRoutine);
  const resumeBotRoutine = useBotStore((s) => s.resumeBotRoutine);
  const deleteBotRoutine = useBotStore((s) => s.deleteBotRoutine);

  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<ScheduledTask | null>(null);
  const [busy, setBusy] = useState<Record<string, boolean>>({});

  const load = useCallback(async () => {
    try {
      const rows = await listBotRoutines(botId);
      setTasks(Array.isArray(rows) ? rows : []);
      setFailed(false);
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  }, [botId, listBotRoutines]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setFailed(false);
    setTasks([]);
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

  const toggleStatus = useCallback(
    async (task: ScheduledTask, active: boolean) => {
      setBusy((prev) => ({ ...prev, [task.id]: true }));
      try {
        await (active ? resumeBotRoutine(task.id) : pauseBotRoutine(task.id));
        await load();
      } catch (e) {
        message.error(backendErrorText(e));
      } finally {
        setBusy((prev) => ({ ...prev, [task.id]: false }));
      }
    },
    [load, pauseBotRoutine, resumeBotRoutine],
  );

  const confirmDelete = useCallback(
    async (task: ScheduledTask) => {
      try {
        await deleteBotRoutine(task.id);
        await load();
      } catch (e) {
        message.error(backendErrorText(e));
      }
    },
    [deleteBotRoutine, load],
  );

  const openCreate = useCallback(() => {
    setEditing(null);
    setFormOpen(true);
  }, []);

  const openEdit = useCallback((task: ScheduledTask) => {
    setEditing(task);
    setFormOpen(true);
  }, []);

  const locale = i18n.language;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }} data-testid="bot-routines-panel">
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
        <Text type="secondary" style={{ fontSize: 12 }} ellipsis>
          {t("bots.routineEmptyDescription")}
        </Text>
        <Button
          size="small"
          type="primary"
          icon={<Plus size={14} />}
          onClick={openCreate}
          data-testid="bot-routine-add"
        >
          {t("bots.routineAdd")}
        </Button>
      </div>

      {loading
        ? <Text type="secondary">{t("common.loading")}</Text>
        : failed
        ? <Text type="danger">{t("bots.routineSaveFailed")}</Text>
        : tasks.length === 0
        ? (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                <Text type="secondary">{t("bots.routineEmpty")}</Text>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t("bots.routineEmptyDescription")}
                </Text>
              </div>
            }
          />
        )
        : (
          <List
            size="small"
            dataSource={[...tasks].sort((a, b) => a.name.localeCompare(b.name, locale))}
            renderItem={(task) => (
              <List.Item
                style={{ padding: "6px 4px", borderRadius: 6 }}
                data-testid="bot-routine-item"
                actions={[
                  <Switch
                    key="switch"
                    size="small"
                    checked={task.status === "active"}
                    loading={busy[task.id] === true}
                    onChange={(checked) => void toggleStatus(task, checked)}
                    aria-label={task.status === "active" ? t("bots.routinePause") : t("bots.routineResume")}
                  />,
                  <Button
                    key="edit"
                    type="text"
                    size="small"
                    icon={<Pencil size={14} />}
                    aria-label={t("common.edit")}
                    onClick={() => openEdit(task)}
                  />,
                  <Popconfirm
                    key="delete"
                    title={t("bots.routineDeleteConfirm")}
                    okText={t("bots.routineDelete")}
                    cancelText={t("common.cancel")}
                    okButtonProps={{ danger: true }}
                    onConfirm={() => void confirmDelete(task)}
                  >
                    <Button
                      type="text"
                      size="small"
                      danger
                      icon={<Trash2 size={14} />}
                      aria-label={t("bots.routineDelete")}
                    />
                  </Popconfirm>,
                ]}
              >
                <div style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
                  <Text ellipsis strong>
                    {task.name}
                    {task.status === "paused" && (
                      <Tag style={{ marginInlineStart: 8 }}>{t("bots.routinePause")}</Tag>
                    )}
                  </Text>
                  <Text type="secondary" style={{ fontSize: 12 }} ellipsis>
                    {task.cronExpression ?? "-"}
                    {" · "}
                    {task.timezone ? task.timezone : t("bots.routineTimezoneLocal")}
                  </Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {t("bots.routineNextRun")}
                    {": "}
                    {task.nextRunAt ? new Date(task.nextRunAt).toLocaleString(locale) : "-"}
                    {" · "}
                    {t("bots.routineLastRun")}
                    {": "}
                    {task.lastRunAt ? new Date(task.lastRunAt).toLocaleString(locale) : "-"}
                  </Text>
                </div>
              </List.Item>
            )}
          />
        )}

      <BotRoutineFormModal
        open={formOpen}
        botId={botId}
        editing={editing}
        onClose={() => setFormOpen(false)}
        onSaved={() => void load()}
      />
    </div>
  );
}
