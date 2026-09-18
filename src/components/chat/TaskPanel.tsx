// SPDX-License-Identifier: AGPL-3.0-only

import { invoke, listen, logIpcError } from "@/lib/invoke";
import { message } from "@/lib/toast";
import { Badge, Button, Empty, Popconfirm, Spin, Tag, Typography } from "antd";
import { AlertCircle, Circle, LoaderCircle, StopCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text, Paragraph } = Typography;

/**
 * 后端 `BackgroundTaskInfo` 的前端投影。
 *
 * 字段名必须是 camelCase：后端结构体带 `#[serde(rename_all = "camelCase")]`
 * （AGENTS.md 禁区 13），IPC 层的键名就是 `taskType` / `exitCode` 这一套。
 * 此前这里写的是 snake_case（`task_type` / `exit_code`），而 `@/lib/invoke`
 * 不做任何键名转换 —— 于是这两个字段在前端**恒为 undefined**：
 * 类型标签渲染成空 Tag、退出码永不显示，而类型检查与单测全部通过。
 */
interface BackgroundTask {
  id: string;
  title: string;
  description: string;
  taskType: string;
  command?: string;
  prompt?: string;
  status: string;
  output: string;
  exitCode?: number;
  conversationId?: string;
  idempotencyKey?: string;
  attempt: number;
  resumeFrom?: string;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number;
}

/**
 * 一条状态迁移记录（`task_events` 的前端投影）。
 *
 * 时间线是 `task_events` 的**读路径**：只写不读的审计表等于死表 ——
 * 表建了、数据在写，但没有任何一处能把它显示出来。
 */
interface TaskEvent {
  id: string;
  taskId: string;
  source: string;
  fromStatus?: string | null;
  toStatus: string;
  actor: string;
  reason?: string | null;
  payload?: string | null;
  createdAt: number;
}

/** `task_events.created_at` 与 `background_tasks.*` 一样是**毫秒**时间戳。 */
function formatEventTime(ms: number): string {
  return new Date(ms).toLocaleString();
}

export function TaskPanel() {
  const { t } = useTranslation();
  const [tasks, setTasks] = useState<BackgroundTask[]>([]);
  const [loading, setLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [events, setEvents] = useState<TaskEvent[]>([]);
  const [eventsLoading, setEventsLoading] = useState(false);
  const [eventsError, setEventsError] = useState<string | null>(null);

  const STATUS_CONFIG: Record<
    string,
    { color: string; icon: React.ReactNode; label: string }
  > = {
    pending: {
      color: "default",
      icon: <Circle size={10} />,
      label: t("task.statusPending"),
    },
    running: {
      color: "processing",
      icon: (
        <LoaderCircle
          size={10}
          style={{ animation: "spin 1s linear infinite" }}
        />
      ),
      label: t("task.statusRunning"),
    },
    completed: {
      color: "success",
      icon: <Circle size={10} fill="currentColor" />,
      label: t("task.statusCompleted"),
    },
    failed: {
      color: "error",
      icon: <Circle size={10} fill="currentColor" />,
      label: t("task.statusFailed"),
    },
    stopped: {
      color: "warning",
      icon: <StopCircle size={10} />,
      label: t("task.statusStopped"),
    },
  };

  /**
   * 状态字符串 → 展示配置。
   *
   * 不认识的字符串**不映射到 pending**。原实现是
   * `STATUS_CONFIG[task.status] || STATUS_CONFIG.pending`，而 `pending` 的标签是
   * 「等待中」—— 一个状态字段漂移的任务（后端 `TaskStatus::Unknown` 会把无法识别的
   * 库值归零为 `""`）会被显示成「等待中」，而它可能正在跑。这是**归因字段说谎**：
   * 界面主动给出了一个它并不知道的结论。改为显式展示原始值与「未知状态」。
   */
  const statusConfig = (status: string): { color: string; icon: React.ReactNode; label: string } => {
    const known = STATUS_CONFIG[status];
    if (known) { return known; }
    return {
      color: "warning",
      icon: <AlertCircle size={10} />,
      label: status
        ? `${t("task.statusUnknown")} (${status})`
        : t("task.statusUnknown"),
    };
  };

  /** 任务类型标签。`task.bash` / `task.agent` 两个 key 此前已存在但无人消费。 */
  const taskTypeLabel = (taskType: string): string => {
    const map: Record<string, string> = {
      bash: t("task.bash"),
      agent: t("task.agent"),
    };
    return map[taskType] ?? taskType;
  };

  /** 事件来源标签（`command` | `tool` | `restore` | `other`）。 */
  const sourceLabel = (source: string): string => {
    const map: Record<string, string> = {
      command: t("task.sourceCommand"),
      tool: t("task.sourceTool"),
      restore: t("task.sourceRestore"),
      other: t("task.sourceOther"),
    };
    return map[source] ?? source;
  };

  const fetchTasks = useCallback(async () => {
    try {
      const result = await invoke<BackgroundTask[]>("list_background_tasks");
      setTasks(result);
    } catch (e) {
      logIpcError("Failed to fetch tasks")(e);
    }
  }, []);

  const loadEvents = useCallback(
    async (taskId: string) => {
      setEventsLoading(true);
      setEventsError(null);
      try {
        const rows = await invoke<TaskEvent[]>("list_task_events", {
          taskId,
          limit: 100,
        });
        setEvents(rows);
      } catch (e) {
        logIpcError("Failed to load task events")(e);
        // 读失败必须显式告知：把错误渲染成空时间线，等于告诉用户
        // 「这个任务没有任何状态迁移」，那是伪造结论。
        setEventsError(t("task.timelineLoadFail"));
        setEvents([]);
      } finally {
        setEventsLoading(false);
      }
    },
    [t],
  );

  const toggleExpand = useCallback(
    (taskId: string) => {
      const next = expandedId === taskId ? null : taskId;
      setExpandedId(next);
      if (next) {
        loadEvents(next);
      } else {
        setEvents([]);
        setEventsError(null);
      }
    },
    [expandedId, loadEvents],
  );

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoading(true);
    fetchTasks().finally(() => setLoading(false));

    let unlistenCreated: (() => void) | undefined;
    let unlistenUpdated: (() => void) | undefined;

    // 后端 `app.emit("background-task:created", &id)` 派发的是**裸字符串 id**，
    // 不是对象；此前的泛型 `{ task_id: string }` 与实际载荷不符。
    listen<string>("background-task:created", () => {
      fetchTasks();
    }).then((fn) => {
      unlistenCreated = fn;
    });

    listen<string>("background-task:updated", () => {
      fetchTasks();
    }).then((fn) => {
      unlistenUpdated = fn;
    });

    const interval = setInterval(fetchTasks, 5000);

    return () => {
      clearInterval(interval);
      unlistenCreated?.();
      unlistenUpdated?.();
    };
  }, [fetchTasks]);

  const handleStop = useCallback(
    async (taskId: string) => {
      try {
        await invoke("stop_background_task", { taskId });
        message.success(t("task.stopped"));
        fetchTasks();
      } catch (e) {
        message.error(`${t("task.stopFail")}: ${String(e)}`);
      }
    },
    [fetchTasks, t],
  );

  const runningCount = tasks.filter((task) => task.status === "running").length;
  const pendingCount = tasks.filter((task) => task.status === "pending").length;

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <div
        style={{
          padding: "12px 16px",
          borderBottom: "1px solid var(--color-border)",
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <span style={{ fontWeight: 600, fontSize: 14 }}>
          {t("task.title")}
          {(runningCount > 0 || pendingCount > 0) && (
            <Badge
              count={runningCount + pendingCount}
              size="small"
              style={{ marginLeft: 8 }}
            />
          )}
        </span>
        <Button size="small" onClick={fetchTasks} loading={loading}>
          {t("task.refresh")}
        </Button>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "8px 12px" }}>
        {loading && tasks.length === 0 && <Spin style={{ display: "block", margin: "24px auto" }} />}

        {!loading && tasks.length === 0 && (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={t("task.empty")}
            style={{ marginTop: 32 }}
          />
        )}

        {tasks.map((task) => {
          const cfg = statusConfig(task.status);
          const isExpanded = expandedId === task.id;

          return (
            <div
              key={task.id}
              style={{
                marginBottom: 8,
                border: "1px solid var(--color-border)",
                borderRadius: 8,
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  padding: "8px 12px",
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  cursor: "pointer",
                  backgroundColor: isExpanded
                    ? "var(--color-fill-alter)"
                    : "transparent",
                }}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    toggleExpand(task.id);
                  }
                }}
                onClick={() => toggleExpand(task.id)}
              >
                <span
                  style={{
                    color: `var(--color-${cfg.color})`,
                    display: "flex",
                  }}
                >
                  {cfg.icon}
                </span>
                <Text strong style={{ flex: 1, fontSize: 13 }}>
                  {task.title}
                </Text>
                <Tag color={cfg.color} style={{ margin: 0 }}>
                  {cfg.label}
                </Tag>
                <Tag style={{ margin: 0, fontSize: 12 }}>
                  {taskTypeLabel(task.taskType)}
                </Tag>
                {task.status === "running" && (
                  <Popconfirm
                    title={t("task.stopConfirm")}
                    onConfirm={(e) => {
                      e?.stopPropagation();
                      handleStop(task.id);
                    }}
                    onCancel={(e) => e?.stopPropagation()}
                    okText={t("task.stop")}
                    cancelText={t("common.cancel")}
                  >
                    <Button
                      type="text"
                      size="small"
                      danger
                      icon={<StopCircle size={14} />}
                      onClick={(e) => e.stopPropagation()}
                    />
                  </Popconfirm>
                )}
              </div>

              {isExpanded && (
                <div
                  style={{
                    padding: "8px 12px 12px",
                    borderTop: "1px solid var(--color-border-secondary)",
                  }}
                >
                  {task.description && (
                    <Paragraph
                      type="secondary"
                      style={{ fontSize: 12, marginBottom: 8 }}
                    >
                      {task.description}
                    </Paragraph>
                  )}
                  {task.command && (
                    <Text
                      code
                      style={{
                        fontSize: 12,
                        display: "block",
                        marginBottom: 8,
                      }}
                    >
                      $ {task.command}
                    </Text>
                  )}
                  {task.exitCode != null && (
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t("task.exitCode")}: {task.exitCode}
                    </Text>
                  )}
                  <div
                    style={{
                      marginTop: 8,
                      background: "var(--color-fill-secondary)",
                      borderRadius: 6,
                      padding: "8px 12px",
                      maxHeight: 300,
                      overflow: "auto",
                      fontFamily: "var(--font-mono, 'JetBrains Mono', ui-monospace, monospace)",
                      fontSize: 12,
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-all",
                    }}
                  >
                    {task.output || t("task.noOutput")}
                  </div>

                  {/* ── 状态时间线（task_events 读路径）────────────────── */}
                  <div style={{ marginTop: 12 }}>
                    <Text strong style={{ fontSize: 12, display: "block", marginBottom: 6 }}>
                      {t("task.timeline")}
                    </Text>
                    {eventsLoading && <Spin size="small" />}
                    {!eventsLoading && eventsError && (
                      <Text type="danger" style={{ fontSize: 12 }}>
                        {eventsError}
                      </Text>
                    )}
                    {!eventsLoading && !eventsError && events.length === 0 && (
                      <Text type="secondary" style={{ fontSize: 12 }}>
                        {t("task.timelineEmpty")}
                      </Text>
                    )}
                    {!eventsLoading
                      && !eventsError
                      && events.map((ev) => {
                        const toCfg = statusConfig(ev.toStatus);
                        return (
                          <div
                            key={ev.id}
                            style={{
                              display: "flex",
                              alignItems: "baseline",
                              flexWrap: "wrap",
                              gap: 6,
                              padding: "3px 0",
                              borderTop: "1px dashed var(--color-border-secondary)",
                            }}
                          >
                            <Text
                              type="secondary"
                              style={{
                                fontSize: 11,
                                flexShrink: 0,
                                fontVariantNumeric: "tabular-nums",
                              }}
                            >
                              {formatEventTime(ev.createdAt)}
                            </Text>
                            <Text style={{ fontSize: 12 }}>
                              {ev.fromStatus
                                ? `${statusConfig(ev.fromStatus).label} → `
                                : ""}
                              {toCfg.label}
                            </Text>
                            <Tag style={{ margin: 0, fontSize: 11 }}>
                              {sourceLabel(ev.source)}
                            </Tag>
                            <Text type="secondary" style={{ fontSize: 11 }}>
                              {ev.actor}
                            </Text>
                            {ev.reason && (
                              <Text type="secondary" style={{ fontSize: 11 }}>
                                {ev.reason}
                              </Text>
                            )}
                          </div>
                        );
                      })}
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
