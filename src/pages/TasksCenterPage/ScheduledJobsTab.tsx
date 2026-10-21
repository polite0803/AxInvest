// SPDX-License-Identifier: AGPL-3.0-only

import { showBackendError, translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { useTaskCenterStore } from "@/stores";
import type { ScheduledTask, TaskExecutionRecord } from "@/types";
import {
  App,
  Button,
  Drawer,
  Empty,
  Popconfirm,
  Select,
  Space,
  Switch,
  Table,
  type TableColumnsType,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import { History, Pencil, Plus, RefreshCw, Rocket, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { type FieldState, parseCronFields } from "./CronScheduleBuilder";
import { formatDateTime, formatDuration, formatRelativeOrAbsolute } from "./formatUtils";
import { TaskCreateModal } from "./forms/TaskCreateModal";
import { isNextRunStale, mapRunTrigger } from "./model";
import { canExecuteNow, classifyScheduledTask, isBuiltinTask, type ScheduledTaskGroup } from "./taskGrouping";

const AUTO_REFRESH_INTERVAL_MS = 15_000;

/** 妖股漏检核查的调度输出末尾带 `run_id=<mrr-…>`；据此把「这一次执行」接回面板轮次（D-5）。 */
const MOVER_RUN_ID_PATTERN = /run_id=([0-9a-zA-Z-]+)/;

/** 分组筛选值 */
type GroupFilter = "all" | ScheduledTaskGroup;

const GROUP_OPTIONS: ReadonlyArray<{ value: GroupFilter; labelKey: string }> = [
  { value: "all", labelKey: "tasksCenter.group.all" },
  { value: "invest", labelKey: "tasksCenter.group.invest" },
  { value: "opc", labelKey: "tasksCenter.group.opc" },
  { value: "system", labelKey: "tasksCenter.group.system" },
  { value: "custom", labelKey: "tasksCenter.group.custom" },
];

const GROUP_TAG_COLOR: Record<ScheduledTaskGroup, string> = {
  invest: "blue",
  opc: "purple",
  system: "default",
  custom: "gold",
};

/** cron 星期数字（0=周日）→ i18n 键后缀（与设置页旧定时任务面板同口径） */
const CRON_DOW_KEYS = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
] as const;

/** cron 表达式 → 人话描述；无法识别的形态原样返回 cron。复用 settings.scheduler.* 文案。 */
function describeCron(cron: string, t: (k: string, opts?: Record<string, unknown>) => string): string {
  const f = parseCronFields(cron);
  if (!f) {
    return cron;
  }
  const isAny = (fld: FieldState) => fld.mode === "any";
  const single = (fld: FieldState): number | null =>
    fld.mode === "list" && fld.selected.length === 1 ? fld.selected[0] : null;
  const mm = single(f.minute);
  const hh = single(f.hour);
  const hhmm = mm !== null && hh !== null
    ? `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}`
    : null;

  if (
    f.minute.mode === "step"
    && f.minute.start === 0
    && isAny(f.hour)
    && isAny(f.dom)
    && isAny(f.month)
    && isAny(f.dow)
  ) {
    return t("settings.scheduler.minutes", { count: f.minute.interval });
  }
  if (f.hour.mode === "step" && mm !== null && isAny(f.dom) && isAny(f.month) && isAny(f.dow)) {
    return t("settings.scheduler.intervalDesc", { hours: f.hour.interval });
  }
  if (hhmm && isAny(f.dom) && isAny(f.month) && isAny(f.dow)) {
    return `${t("settings.scheduler.dailyDesc")} ${hhmm}`;
  }
  if (hhmm && f.dow.mode === "list" && isAny(f.dom) && isAny(f.month)) {
    const dayNames = f.dow.selected
      .slice()
      .sort((a, b) => (a === 0 ? 7 : a) - (b === 0 ? 7 : b))
      .map((d) => t(`settings.scheduler.${CRON_DOW_KEYS[d]}`))
      .join(", ");
    return `${t("settings.scheduler.weeklyDesc", { days: dayNames || "-" })} ${hhmm}`;
  }
  if (hhmm && f.dom.mode === "list" && isAny(f.month) && isAny(f.dow)) {
    const days = f.dom.selected.slice().sort((a, b) => a - b).join(",");
    return `${t("settings.scheduler.monthlyDesc", { day: days })} ${hhmm}`;
  }
  return cron;
}

export function ScheduledJobsTab() {
  const { t, i18n } = useTranslation();
  const { message, modal } = App.useApp();
  // 立即执行后的延迟刷新：必须可取消 —— 否则卸载后 800ms 的回调仍会发 IPC，
  // 在测试里会串到下一条用例（实测把「15s 自动刷新」那条的调用计数从 2 顶成 3）。
  const refreshTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (refreshTimer.current !== null) {
        window.clearTimeout(refreshTimer.current);
        refreshTimer.current = null;
      }
    },
    [],
  );
  const navigate = useNavigate();
  const locale = i18n.language;

  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [groupFilter, setGroupFilter] = useState<GroupFilter>("all");
  const [executing, setExecuting] = useState<Record<string, boolean>>({});
  const [pausing, setPausing] = useState<Record<string, boolean>>({});

  // 执行历史 Drawer
  const [historyTask, setHistoryTask] = useState<ScheduledTask | null>(null);
  const [records, setRecords] = useState<TaskExecutionRecord[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);

  // 新建 / 编辑任务 Modal（editingTask 非空 = 编辑态）
  const [createOpen, setCreateOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<ScheduledTask | null>(null);

  const openCreate = useCallback(() => {
    setEditingTask(null);
    setCreateOpen(true);
  }, []);

  const openEdit = useCallback((task: ScheduledTask) => {
    setEditingTask(task);
    setCreateOpen(true);
  }, []);

  const loadTasks = useCallback(async (silent: boolean) => {
    if (silent) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }
    try {
      const rows = await invoke<ScheduledTask[]>("list_scheduled_tasks");
      setTasks(Array.isArray(rows) ? rows : []);
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [message]);

  // 首次加载
  useEffect(() => {
    void loadTasks(false);
  }, [loadTasks]);

  // 15s 自动刷新（卸载必清理）
  useEffect(() => {
    if (!autoRefresh) {
      return;
    }
    const timer = window.setInterval(() => {
      void loadTasks(true);
    }, AUTO_REFRESH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [autoRefresh, loadTasks]);

  const toggleStatus = useCallback(
    async (task: ScheduledTask, active: boolean) => {
      setPausing((prev) => ({ ...prev, [task.id]: true }));
      try {
        await invoke(active ? "resume_scheduled_task" : "pause_scheduled_task", {
          taskId: task.id,
        });
        await loadTasks(true);
      } catch (e) {
        showBackendError(message, e);
      } finally {
        setPausing((prev) => ({ ...prev, [task.id]: false }));
      }
    },
    [loadTasks, message],
  );

  const executeNow = useCallback(
    async (task: ScheduledTask) => {
      if (!canExecuteNow(task)) {
        return;
      }
      setExecuting((prev) => ({ ...prev, [task.id]: true }));
      try {
        const res = await invoke<{
          success?: boolean;
          output?: string | null;
          error?: string | null;
          awaited?: boolean;
        }>("execute_scheduled_task", { taskId: task.id });
        // 内置长任务在后台跑：立刻刷新「全部运行」，让后端在跑登记变成一条 running 行；
        // 之后的 2 秒轮询会把它推进到完成态。短任务此时可能已经结束，下面延迟重拉一次。
        void useTaskCenterStore.getState().loadRuns(true);
        // 等到了终态（如漏检核查：一次库内扫描）⇒ 直接把这一轮的结论摊开给用户；
        // 长任务只回「已触发」，不能把 ack 当结果（2026-10-21 实机反馈）。
        if (res?.awaited && (res.output ?? "").trim() !== "") {
          modal.info({
            title: `${t("tasksCenter.scheduled.resultTitle")} · ${task.name}`,
            content: (
              <pre style={{ whiteSpace: "pre-wrap", fontSize: 12, margin: 0 }}>
                {res.output}
              </pre>
            ),
            width: 640,
          });
        } else if (res && res.success === false && res.error) {
          showBackendError(message, res.error);
        } else {
          message.success(t("tasksCenter.scheduled.executedOk", { name: task.name }));
        }
        // 内置任务结果异步落库，延迟再拉列表（同一时刻只保留一个，且卸载时清理）
        if (refreshTimer.current !== null) { window.clearTimeout(refreshTimer.current); }
        refreshTimer.current = window.setTimeout(() => {
          refreshTimer.current = null;
          void loadTasks(true);
          void useTaskCenterStore.getState().loadRuns(true);
        }, 800);
      } catch (e) {
        showBackendError(message, e);
      } finally {
        setExecuting((prev) => ({ ...prev, [task.id]: false }));
      }
    },
    [loadTasks, message, t],
  );

  const confirmDelete = useCallback(
    async (task: ScheduledTask) => {
      try {
        await invoke("delete_scheduled_task", { taskId: task.id });
        message.success(t("tasksCenter.scheduled.deletedOk", { name: task.name }));
        await loadTasks(true);
      } catch (e) {
        showBackendError(message, e);
      }
    },
    [loadTasks, message, t],
  );

  const openHistory = useCallback(
    async (task: ScheduledTask) => {
      setHistoryTask(task);
      setRecords([]);
      setHistoryLoading(true);
      try {
        const rows = await invoke<TaskExecutionRecord[]>("get_task_execution_history", {
          taskId: task.id,
        });
        setRecords(Array.isArray(rows) ? rows : []);
      } catch (e) {
        showBackendError(message, e);
      } finally {
        setHistoryLoading(false);
      }
    },
    [message],
  );

  const filteredTasks = useMemo(() => {
    const enriched = tasks.map((task) => ({ task, group: classifyScheduledTask(task) }));
    return (
      groupFilter === "all"
        ? enriched
        : enriched.filter(({ group }) => group === groupFilter)
    ).sort((a, b) => a.task.name.localeCompare(b.task.name, locale));
  }, [tasks, groupFilter, locale]);

  const columns: TableColumnsType<ScheduledTask> = useMemo(() => {
    const groupOf = new Map(tasks.map((task) => [task.id, classifyScheduledTask(task)]));

    return [
      {
        title: t("tasksCenter.column.name"),
        dataIndex: "name",
        key: "name",
        width: 220,
        render: (_: unknown, task: ScheduledTask) => (
          <Space orientation="vertical" size={0}>
            <Typography.Text strong>{task.name}</Typography.Text>
            {task.description && (
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {task.description}
              </Typography.Text>
            )}
          </Space>
        ),
      },
      {
        title: t("tasksCenter.column.group"),
        key: "group",
        width: 110,
        render: (_: unknown, task: ScheduledTask) => {
          const group = groupOf.get(task.id) ?? classifyScheduledTask(task);
          return <Tag color={GROUP_TAG_COLOR[group]}>{t(`tasksCenter.group.${group}`)}</Tag>;
        },
      },
      {
        title: t("tasksCenter.column.schedule"),
        key: "schedule",
        width: 200,
        render: (_: unknown, task: ScheduledTask) =>
          task.cronExpression
            ? (
              <Space orientation="vertical" size={0}>
                <Typography.Text>{describeCron(task.cronExpression, t)}</Typography.Text>
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {task.cronExpression}
                </Typography.Text>
              </Space>
            )
            : <Typography.Text type="secondary">-</Typography.Text>,
      },
      {
        title: t("tasksCenter.column.status"),
        key: "status",
        width: 90,
        render: (_: unknown, task: ScheduledTask) => (
          <Switch
            size="small"
            checked={task.status === "active"}
            loading={pausing[task.id] === true}
            onChange={(checked) => void toggleStatus(task, checked)}
          />
        ),
      },
      {
        title: t("tasksCenter.column.nextRun"),
        key: "nextRunAt",
        width: 170,
        render: (_: unknown, task: ScheduledTask) => {
          const stale = isNextRunStale(task);
          return (
            <Space size={4}>
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {task.nextRunAt ? formatRelativeOrAbsolute(task.nextRunAt, locale) : "-"}
              </Typography.Text>
              {stale && (
                <Tooltip title={t("tasksCenter.nextRun.staleHint")}>
                  <Tag color="warning" style={{ marginInlineEnd: 0 }}>
                    {t("tasksCenter.nextRun.stale")}
                  </Tag>
                </Tooltip>
              )}
            </Space>
          );
        },
      },
      {
        title: t("tasksCenter.column.lastRun"),
        key: "lastRun",
        width: 200,
        render: (_: unknown, task: ScheduledTask) => {
          if (!task.lastRunAt || !task.lastResult) {
            return <Typography.Text type="secondary">{t("tasksCenter.lastRun.none")}</Typography.Text>;
          }
          const r = task.lastResult;
          const tag = r.skipped
            ? <Tag color="default">{t("tasksCenter.lastRun.skipped")}</Tag>
            : r.partial
            ? <Tag color="warning">{t("tasksCenter.lastRun.partial")}</Tag>
            : r.success
            ? <Tag color="success">{t("tasksCenter.lastRun.success")}</Tag>
            : <Tag color="error">{t("tasksCenter.lastRun.failed")}</Tag>;
          return (
            <Space orientation="vertical" size={0}>
              <Space size={4}>
                {tag}
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {formatRelativeOrAbsolute(task.lastRunAt, locale)}
                </Typography.Text>
              </Space>
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {formatDuration(r.durationMs, locale)}
              </Typography.Text>
            </Space>
          );
        },
      },
      {
        title: t("tasksCenter.column.actions"),
        key: "actions",
        width: 200,
        render: (_: unknown, task: ScheduledTask) => {
          const executable = canExecuteNow(task);
          const builtin = isBuiltinTask(task);
          const group = groupOf.get(task.id) ?? classifyScheduledTask(task);
          return (
            <Space size={4}>
              <Tooltip
                title={executable ? t("tasksCenter.action.execute") : t("tasksCenter.execute.unsupported")}
              >
                <Button
                  type="text"
                  size="small"
                  aria-label={t("tasksCenter.action.execute")}
                  icon={<Rocket size={14} />}
                  loading={executing[task.id] === true}
                  disabled={!executable}
                  onClick={() => void executeNow(task)}
                />
              </Tooltip>
              <Tooltip title={t("tasksCenter.action.history")}>
                <Button
                  type="text"
                  size="small"
                  aria-label={t("tasksCenter.action.history")}
                  icon={<History size={14} />}
                  onClick={() => void openHistory(task)}
                />
              </Tooltip>
              {/* 内置投资/系统任务无通用更新命令（参数封装各异），仅自定义/工作流任务可编辑 */}
              {!builtin && (
                <Tooltip title={t("tasksCenter.action.edit")}>
                  <Button
                    type="text"
                    size="small"
                    aria-label={t("tasksCenter.action.edit")}
                    icon={<Pencil size={14} />}
                    onClick={() => openEdit(task)}
                  />
                </Tooltip>
              )}
              {group === "opc" && (
                <Tooltip title={t("tasksCenter.action.editInOpc")}>
                  <Button
                    type="link"
                    size="small"
                    onClick={() => navigate("/demand-discovery")}
                  >
                    {t("tasksCenter.action.opcDelivery")}
                  </Button>
                </Tooltip>
              )}
              <Popconfirm
                title={t("tasksCenter.delete.confirmTitle", { name: task.name })}
                description={builtin ? t("tasksCenter.delete.builtinWarning") : undefined}
                okText={t("tasksCenter.delete.ok")}
                cancelText={t("tasksCenter.delete.cancel")}
                okButtonProps={{ danger: true }}
                onConfirm={() => void confirmDelete(task)}
              >
                <Tooltip title={t("tasksCenter.action.delete")}>
                  <Button
                    type="text"
                    size="small"
                    danger
                    aria-label={t("tasksCenter.action.delete")}
                    icon={<Trash2 size={14} />}
                  />
                </Tooltip>
              </Popconfirm>
            </Space>
          );
        },
      },
    ];
  }, [t, locale, pausing, executing, toggleStatus, executeNow, openHistory, openEdit, confirmDelete, navigate, tasks]);

  return (
    <Space orientation="vertical" size={12} style={{ width: "100%" }}>
      <Space wrap style={{ justifyContent: "space-between", width: "100%" }}>
        <Space wrap>
          <Select<GroupFilter>
            value={groupFilter}
            onChange={setGroupFilter}
            style={{ width: 160 }}
            options={GROUP_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) }))}
          />
          <Button type="primary" size="small" icon={<Plus size={14} />} onClick={openCreate}>
            {t("tasksCenter.create.button")}
          </Button>
        </Space>
        <Space wrap>
          <Space size={6}>
            <Typography.Text type="secondary">{t("tasksCenter.scheduled.autoRefresh")}</Typography.Text>
            <Switch size="small" checked={autoRefresh} onChange={setAutoRefresh} />
          </Space>
          <Button
            size="small"
            icon={<RefreshCw size={14} className={refreshing ? "animate-spin" : undefined} />}
            onClick={() => void loadTasks(false)}
          >
            {t("tasksCenter.scheduled.refresh")}
          </Button>
        </Space>
      </Space>

      <Table<ScheduledTask>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={filteredTasks.map(({ task }) => task)}
        pagination={{ pageSize: 20, size: "small", showSizeChanger: false }}
        locale={{
          emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("tasksCenter.scheduled.empty")} />,
        }}
      />

      <Drawer
        title={historyTask ? t("tasksCenter.history.title", { name: historyTask.name }) : ""}
        open={historyTask !== null}
        onClose={() => setHistoryTask(null)}
        styles={{ wrapper: { width: 560 } }}
      >
        <Table<TaskExecutionRecord>
          rowKey="id"
          size="small"
          loading={historyLoading}
          dataSource={[...records].sort((a, b) => b.started_at - a.started_at)}
          pagination={false}
          columns={[
            {
              title: t("tasksCenter.history.started"),
              dataIndex: "started_at",
              key: "started_at",
              render: (v: number) => (
                <Typography.Text style={{ fontSize: 12 }}>{formatDateTime(v, locale)}</Typography.Text>
              ),
            },
            {
              title: t("tasksCenter.history.duration"),
              dataIndex: "duration_ms",
              key: "duration_ms",
              width: 90,
              render: (v: number) => (
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {formatDuration(v, locale)}
                </Typography.Text>
              ),
            },
            {
              title: t("tasksCenter.column.trigger"),
              key: "trigger",
              width: 80,
              render: (_: unknown, r: TaskExecutionRecord) => (
                <Tag>{t(`tasksCenter.trigger.${mapRunTrigger(r.trigger)}`)}</Tag>
              ),
            },
            {
              title: t("tasksCenter.history.result"),
              key: "result",
              width: 80,
              // 与「上一次运行」那一列同判据（:343）：success=false 有两种意思——
              // 真故障与「部分完成」（超墙钟收尾 / 单轮上限截断）。历史行的 partial
              // 来自 cron_job_history.partial，缺省 false 时退化成「失败」（旧行即如此）。
              render: (_: unknown, r: TaskExecutionRecord) =>
                r.success
                  ? <Tag color="success">{t("tasksCenter.lastRun.success")}</Tag>
                  : r.partial
                  ? <Tag color="warning">{t("tasksCenter.lastRun.partial")}</Tag>
                  : <Tag color="error">{t("tasksCenter.lastRun.failed")}</Tag>,
            },
            {
              title: t("tasksCenter.history.detail"),
              key: "detail",
              // 运行记录这两个字段可能带的是后端 ErrorResponse 的 **JSON 串**（命令层按
              // 错误码规范返回），不译会让界面出现 `{"code":"...","detail":"..."}` 字面量；
              // 非 JSON 的自由文本原样透传（translateBackendError 的零回归路径）。
              render: (_: unknown, r: TaskExecutionRecord) => {
                const moverRun = MOVER_RUN_ID_PATTERN.exec(r.output ?? "")?.[1] ?? null;
                return (
                  <>
                    {r.error
                      ? (
                        <Typography.Text type="danger" style={{ fontSize: 12 }}>
                          {translateBackendError(r.error)}
                        </Typography.Text>
                      )
                      : r.output
                      ? (
                        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                          {translateBackendError(r.output)}
                        </Typography.Text>
                      )
                      : "-"}
                    {moverRun
                      ? (
                        <Typography.Link
                          style={{ fontSize: 12, marginLeft: 8 }}
                          data-testid="open-mover-run"
                          onClick={() => navigate(`/invest?tab=workspace&view=backtest&moverRun=${moverRun}`)}
                        >
                          {t("tasksCenter.history.openMoverRun")}
                        </Typography.Link>
                      )
                      : null}
                  </>
                );
              },
            },
          ]}
          locale={{
            emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("tasksCenter.history.empty")} />,
          }}
        />
      </Drawer>

      <TaskCreateModal
        open={createOpen}
        editing={editingTask}
        onClose={() => setCreateOpen(false)}
        onSaved={() => void loadTasks(true)}
      />
    </Space>
  );
}
