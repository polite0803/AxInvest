// SPDX-License-Identifier: AGPL-3.0-only

import { showBackendError, translateBackendError } from "@/lib/errorI18n";
import { invoke, isTauri, logIpcError } from "@/lib/invoke";
import { useBackupStore, useSchedulerStore, useSettingsStore } from "@/stores";
import type { TaskReport } from "@/types";
import {
  App,
  Button,
  Divider,
  Form,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Select,
  Switch,
  Tag,
  theme,
  Typography,
} from "antd";
import { Calendar, Clock, Edit2, History, Pause, Play, Plus, RefreshCw, Rocket, Trash2, Zap } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CronScheduleBuilder, parseCronFields, validateCronSyntax } from "./CronScheduleBuilder";
import type { FieldState } from "./CronScheduleBuilder";
import { SettingsGroup } from "./SettingsGroup";

const { Text } = Typography;

const rowStyle: React.CSSProperties = { padding: "4px 0" };

/**
 * 系统托管任务类型 —— 由专用命令/其他功能模块创建和管理，不属于用户自建任务，
 * 不应出现在「自定义任务」列表中（备份/清理在页面开关区管理，知识库抓取在知识库管理）。
 * 其余类型（custom / general / 旧版计划任务页写入的自由 platform 值等）均视为用户任务。
 */
const SYSTEM_TASK_TYPES = new Set([
  "backup",
  "cleanup",
  "daily_summary",
  "knowledge_source_fetch_all",
]);

type Weekday =
  | "monday"
  | "tuesday"
  | "wednesday"
  | "thursday"
  | "friday"
  | "saturday"
  | "sunday";

/** cron 星期数字（0=周日）→ i18n 键后缀 */
const CRON_DOW_KEYS: Weekday[] = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
];

/**
 * 后端 ScheduledTaskDto.config（TaskConfigDto，rename_all = camelCase）。
 * 字段拼写必须保持 camelCase，否则更新任务时 serde 反序列化直接失败。
 */
interface TaskConfig {
  timeoutSeconds: number;
  retryOnFailure: boolean;
  maxRetries: number;
  retryDelaySeconds: number;
  notificationEnabled: boolean;
  runOnStartup: boolean;
}

/**
 * 后端 ScheduledTaskDto（#[serde(rename_all = "camelCase")]）的前端镜像。
 * 后端只持久化 cronExpression，不存在 schedule_config（可视化配置仅在表单态存在）。
 */
interface ScheduledTask {
  id: string;
  name: string;
  description: string;
  taskType: string;
  cronExpression: string | null;
  intervalSeconds: number | null;
  nextRunAt: string | null;
  lastRunAt: string | null;
  lastResult: {
    success: boolean;
    output: string | null;
    error: string | null;
    durationMs: number;
    /** 被前置闸门跳过（非交易日等），未真正执行业务（旧记录缺省为 false） */
    skipped?: boolean;
  } | null;
  status: "active" | "paused" | "disabled";
  config: TaskConfig;
  createdAt: string;
  updatedAt: string;
  workflowId?: string | null;
}

/** 任务表单：调度配置统一由 CronScheduleBuilder 产出 cron 字符串（与后端持久化口径一致） */
interface TaskFormData {
  name: string;
  description: string;
  template_type?: string;
  workflow_id?: string | null;
  cron_expression: string;
}

interface TaskTemplate {
  templateType?: string;
  template_type: string;
  name: string;
  description: string;
  workflow_id: string | null;
  default_schedule?: string;
}

export function SchedulerSettings() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();
  const inTauri = isTauri();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);
  const backupSettings = useBackupStore((s) => s.backupSettings);
  const updateBackupSettings = useBackupStore((s) => s.updateBackupSettings);
  // 夜间长时任务：预算 / 报告 / 恢复
  const budget = useSchedulerStore((s) => s.budget);
  const budgetLoading = useSchedulerStore((s) => s.loading);
  const fetchBudget = useSchedulerStore((s) => s.fetchBudget);
  const setBudgetCmd = useSchedulerStore((s) => s.setBudget);
  const getTaskReport = useSchedulerStore((s) => s.getTaskReport);
  const restorePendingTasks = useSchedulerStore((s) => s.restorePendingTasks);

  const [budgetInput, setBudgetInput] = useState<number | null>(null);
  const [reportTaskId, setReportTaskId] = useState<string | undefined>(undefined);
  const [report, setReport] = useState<TaskReport | null>(null);
  const [reportLoading, setReportLoading] = useState(false);
  const [restoreLoading, setRestoreLoading] = useState(false);

  interface ExecutionRecord {
    id: string;
    task_id: string;
    started_at: number;
    completed_at: number | null;
    success: boolean;
    output: string | null;
    error: string | null;
    duration_ms: number;
  }

  const [customTasks, setCustomTasks] = useState<ScheduledTask[]>([]);
  const [taskTemplates, setTaskTemplates] = useState<TaskTemplate[]>([]);
  const [taskModalOpen, setTaskModalOpen] = useState(false);
  const [selectedTemplate, setSelectedTemplate] = useState<string | null>(null);
  const [editingTask, setEditingTask] = useState<ScheduledTask | null>(null);
  const [form] = Form.useForm<TaskFormData>();
  const [loading, setLoading] = useState(false);
  const [executing, setExecuting] = useState<Record<string, boolean>>({});
  const [historyMap, setHistoryMap] = useState<
    Record<string, ExecutionRecord[]>
  >({});
  const [expandedHistory, setExpandedHistory] = useState<
    Record<string, boolean>
  >({});

  const loadCustomTasks = useCallback(async () => {
    try {
      const tasks = await invoke<ScheduledTask[]>("list_scheduled_tasks");
      // 排除系统托管任务；旧版「计划任务」页创建的任务 taskType 为 general 或自由 platform 值，
      // 同样是用户自建任务，必须一并展示（历史上此处白名单 === "custom" 导致这些任务不可见）
      setCustomTasks(
        Array.isArray(tasks) ? tasks.filter((t) => !SYSTEM_TASK_TYPES.has(t.taskType)) : [],
      );
    } catch (e) {
      logIpcError("Failed to load scheduled tasks")(e);
    }
  }, []);

  const loadTemplates = useCallback(async () => {
    try {
      const templates = await invoke<TaskTemplate[]>(
        "get_scheduled_task_templates",
      );
      setTaskTemplates(templates);
    } catch (e) {
      logIpcError("Failed to load task templates")(e);
    }
  }, []);

  useEffect(() => {
    if (inTauri) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      loadCustomTasks();
      loadTemplates();
      // eslint-disable-next-line react-hooks/set-state-in-effect
      fetchBudget();
    }
  }, [inTauri, loadCustomTasks, loadTemplates, fetchBudget]);

  // 预算加载完成后把当前上限填入输入框（仅首次）
  useEffect(() => {
    if (budget?.maxBudget != null && budgetInput == null) {
      setBudgetInput(budget.maxBudget);
    }
  }, [budget, budgetInput]);

  const handleAutoBackupChange = async (enabled: boolean) => {
    if (!backupSettings) {
      return;
    }
    const newSettings = { ...backupSettings, enabled };
    await updateBackupSettings(newSettings);
    message.success(t("settings.scheduler.saved"));
  };

  const handleBackupIntervalChange = async (intervalHours: number | null) => {
    if (!backupSettings || !intervalHours) {
      return;
    }
    const newSettings = { ...backupSettings, intervalHours };
    await updateBackupSettings(newSettings);
    message.success(t("settings.scheduler.saved"));
  };

  const handleWebdavSyncChange = async (syncEnabled: boolean) => {
    saveSettings({ webdavSyncEnabled: syncEnabled });
    if (inTauri) {
      try {
        await invoke("restart_webdav_sync");
      } catch (e) {
        logIpcError("Failed to restart WebDAV sync")(e);
      }
    }
    message.success(t("settings.scheduler.saved"));
  };

  const handleWebdavIntervalChange = async (syncIntervalMinutes: number) => {
    saveSettings({ webdavSyncIntervalMinutes: syncIntervalMinutes });
    if (inTauri) {
      try {
        await invoke("restart_webdav_sync");
      } catch (e) {
        logIpcError("Failed to restart WebDAV sync")(e);
      }
    }
    message.success(t("settings.scheduler.saved"));
  };

  const handleClosedLoopChange = (closedLoopEnabled: boolean) => {
    saveSettings({ closedLoopEnabled: closedLoopEnabled });
    message.success(t("settings.scheduler.saved"));
  };

  const handleClosedLoopIntervalChange = (
    closedLoopIntervalMinutes: number,
  ) => {
    saveSettings({ closedLoopIntervalMinutes: closedLoopIntervalMinutes });
    message.success(t("settings.scheduler.saved"));
  };

  const webdavIntervalOptions = [
    { value: 15, label: t("settings.scheduler.minutes", { count: 15 }) },
    { value: 30, label: t("settings.scheduler.minutes", { count: 30 }) },
    { value: 60, label: t("settings.scheduler.hour") },
    { value: 120, label: t("settings.scheduler.hours", { count: 2 }) },
    { value: 360, label: t("settings.scheduler.hours", { count: 6 }) },
    { value: 720, label: t("settings.scheduler.hours", { count: 12 }) },
    { value: 1440, label: t("settings.scheduler.hours", { count: 24 }) },
  ];

  const closedLoopIntervalOptions = [
    { value: 1, label: t("settings.scheduler.minutes", { count: 1 }) },
    { value: 5, label: t("settings.scheduler.minutes", { count: 5 }) },
    { value: 10, label: t("settings.scheduler.minutes", { count: 10 }) },
    { value: 15, label: t("settings.scheduler.minutes", { count: 15 }) },
    { value: 30, label: t("settings.scheduler.minutes", { count: 30 }) },
    { value: 60, label: t("settings.scheduler.hour") },
  ];

  // antd Form 校验器：cron 表达式必须通过与后端一致的语法/频率校验
  const cronValidator = useCallback(
    (_rule: unknown, value: string) => {
      const errorKey = validateCronSyntax(value ?? "");
      return errorKey ? Promise.reject(new Error(t(errorKey))) : Promise.resolve();
    },
    [t],
  );

  const getStatusColor = (status: string) => {
    switch (status) {
      case "active":
        return "green";
      case "paused":
        return "orange";
      case "disabled":
        return "red";
      default:
        return "default";
    }
  };

  const getStatusText = (status: string) => {
    switch (status) {
      case "active":
        return t("settings.scheduler.taskActive");
      case "paused":
        return t("settings.scheduler.taskPaused");
      case "disabled":
        return t("settings.scheduler.taskDisabled");
      default:
        return status;
    }
  };

  const formatNextRun = (dateStr: string | null | undefined) => {
    if (!dateStr) {
      return "-";
    }
    try {
      const date = new Date(dateStr);
      if (Number.isNaN(date.getTime())) {
        return "-";
      }
      // 启动后下次时刻尚未重算时，后端把 next_run_at 置为 epoch 0（原显示 1970/1/1）
      if (date.getTime() <= 0) {
        return t("settings.scheduler.nextRunPending");
      }
      return date.toLocaleString();
    } catch {
      return dateStr;
    }
  };

  /**
   * 列表「调度」列：可识别的可视化形态给本地化描述，其余原样展示 cron。
   */
  const formatScheduleDescription = (task: ScheduledTask): string => {
    const cron = task.cronExpression;
    if (!cron) {
      return "-";
    }
    const f = parseCronFields(cron);
    if (!f) {
      return cron;
    }
    const isAny = (fld: FieldState) => fld.mode === "any";
    /** list 模式且只选了一个值时取出该值（如每天 09:00 的分/时字段） */
    const single = (fld: FieldState): number | null =>
      fld.mode === "list" && fld.selected.length === 1 ? fld.selected[0] : null;
    const mm = single(f.minute);
    const hh = single(f.hour);
    const hhmm = mm !== null && hh !== null
      ? `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}`
      : null;

    // 每 N 分钟：*/N * * * *（起始必须为 0，覆盖全天）
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
    // 每 N 小时：m */N * * *
    if (
      f.hour.mode === "step"
      && mm !== null
      && isAny(f.dom)
      && isAny(f.month)
      && isAny(f.dow)
    ) {
      return t("settings.scheduler.intervalDesc", { hours: f.hour.interval });
    }
    if (hhmm && isAny(f.dom) && isAny(f.month) && isAny(f.dow)) {
      return `${t("settings.scheduler.dailyDesc")} ${hhmm}`;
    }
    if (hhmm && f.dow.mode === "list" && isAny(f.dom) && isAny(f.month)) {
      // 周网格按周一开头、周日收尾排列（0=周日排到最后）
      const dayNames = f.dow.selected
        .slice()
        .sort((a, b) => (a === 0 ? 7 : a) - (b === 0 ? 7 : b))
        .map((d) => t(`settings.scheduler.${CRON_DOW_KEYS[d]}`))
        .join(", ");
      return `${t("settings.scheduler.weeklyDesc", { days: dayNames || "-" })} ${hhmm}`;
    }
    if (hhmm && f.dom.mode === "list" && isAny(f.month) && isAny(f.dow)) {
      const days = f.dom.selected
        .slice()
        .sort((a, b) => a - b)
        .join(",");
      return `${t("settings.scheduler.monthlyDesc", { day: days })} ${hhmm}`;
    }
    return cron;
  };

  const openCreateModal = () => {
    setEditingTask(null);
    setSelectedTemplate(null);
    form.resetFields();
    form.setFieldsValue({ cron_expression: "0 9 * * *" });
    setTaskModalOpen(true);
  };

  const openEditModal = (task: ScheduledTask) => {
    setEditingTask(task);
    setSelectedTemplate(null);
    form.setFieldsValue({
      name: task.name,
      description: task.description,
      workflow_id: task.workflowId,
      cron_expression: task.cronExpression ?? "0 9 * * *",
    });
    setTaskModalOpen(true);
  };

  const handleTemplateSelect = (templateType: string) => {
    const template = taskTemplates.find(
      (t) => t.template_type === templateType,
    );
    if (template) {
      setSelectedTemplate(templateType);
      form.setFieldsValue({
        name: template.name,
        description: template.description,
        template_type: template.template_type,
        workflow_id: template.workflow_id,
        cron_expression: template.default_schedule || "0 9 * * *",
      });
    }
  };

  const handleCreateOrUpdateTask = async () => {
    try {
      const values = await form.validateFields();
      setLoading(true);

      // 调度配置的唯一真源是 builder 产出的 cron 字符串，与后端持久化口径一致
      const cronExpression = values.cron_expression.trim();

      if (editingTask) {
        // 后端 update_scheduled_task 的 task 参数按完整 ScheduledTaskDto（camelCase）
        // 反序列化：taskType/status/config/createdAt/updatedAt 均为必填，缺字段会直接
        // 400。因此以原任务为底，只覆盖名称/描述/cron，其余字段原样回传。
        const updatedTask: ScheduledTask = {
          ...editingTask,
          name: values.name,
          description: values.description,
          cronExpression: cronExpression,
        };
        await invoke("update_scheduled_task", {
          taskId: editingTask.id,
          task: updatedTask,
        });
        message.success(t("settings.scheduler.updateTask") + " - OK");
      } else {
        await invoke("create_scheduled_task", {
          name: values.name,
          description: values.description,
          cronExpression: cronExpression,
          taskType: values.workflow_id ? "workflow" : "custom",
          workflowId: values.workflow_id || null,
        });
        message.success(t("settings.scheduler.createTask") + " - OK");
      }

      setTaskModalOpen(false);
      await loadCustomTasks();
    } catch (e) {
      logIpcError("Failed to save task")(e);
      showBackendError(message, e);
    } finally {
      setLoading(false);
    }
  };

  const handlePauseTask = async (taskId: string) => {
    try {
      await invoke("pause_scheduled_task", { taskId });
      await loadCustomTasks();
      message.success(t("settings.scheduler.pauseTask") + " - OK");
    } catch (e) {
      logIpcError("Failed to pause task")(e);
      showBackendError(message, e);
    }
  };

  const handleResumeTask = async (taskId: string) => {
    try {
      await invoke("resume_scheduled_task", { taskId });
      await loadCustomTasks();
      message.success(t("settings.scheduler.resumeTask") + " - OK");
    } catch (e) {
      logIpcError("Failed to resume task")(e);
      showBackendError(message, e);
    }
  };

  const handleDeleteTask = async (taskId: string) => {
    try {
      await invoke("delete_scheduled_task", { taskId });
      await loadCustomTasks();
      message.success(t("settings.scheduler.deleteTask") + " - OK");
    } catch (e) {
      logIpcError("Failed to delete task")(e);
      showBackendError(message, e);
    }
  };

  // 手动执行任务
  const handleExecuteNow = async (taskId: string) => {
    try {
      setExecuting((prev) => ({ ...prev, [taskId]: true }));
      await invoke("execute_scheduled_task", { taskId });
      message.success(t("settings.scheduler.executed") + " - OK");
      // 内置任务（荐股/扫描/反思等）的结果由调度 handler 异步 spawn 落库，
      // 立即刷新可能读到旧态；延迟一小段再拉列表
      await new Promise((resolve) => setTimeout(resolve, 800));
      await loadCustomTasks();
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setExecuting((prev) => ({ ...prev, [taskId]: false }));
    }
  };

  // 加载执行历史
  const handleLoadHistory = async (taskId: string) => {
    const isExpanded = expandedHistory[taskId];
    if (isExpanded) {
      setExpandedHistory((prev) => ({ ...prev, [taskId]: false }));
      return;
    }
    try {
      const records = await invoke<ExecutionRecord[]>(
        "get_task_execution_history",
        { taskId },
      );
      setHistoryMap((prev) => ({ ...prev, [taskId]: records }));
      setExpandedHistory((prev) => ({ ...prev, [taskId]: true }));
    } catch (e) {
      logIpcError("Failed to load history")(e);
    }
  };

  // 快速创建模板任务
  const handleQuickCreate = async (templateType: string) => {
    try {
      setLoading(true);
      const cmdMap: Record<string, string> = {
        daily_summary: "create_daily_summary_task",
        backup: "create_backup_task",
        cleanup: "create_cleanup_task",
      };
      const cmd = cmdMap[templateType];
      if (!cmd) {
        message.error(t("settings.scheduler.unknownTemplateType"));
        return;
      }
      await invoke(cmd);
      message.success(t("settings.scheduler.quickCreated") + " - OK");
      await loadCustomTasks();
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setLoading(false);
    }
  };

  // 刷新所有任务
  const handleRefreshAll = async () => {
    try {
      await invoke("load_scheduled_tasks_from_db");
      await loadCustomTasks();
      message.success(t("settings.scheduler.refreshed"));
    } catch (e) {
      showBackendError(message, e);
    }
  };

  const handleSaveBudget = async () => {
    try {
      const saved = await setBudgetCmd(budgetInput);
      setBudgetInput(saved.maxBudget);
      message.success(t("settings.scheduler.budgetSaved"));
    } catch (e) {
      showBackendError(message, e);
    }
  };

  const handleLoadReport = async () => {
    if (!reportTaskId) {
      return;
    }
    setReportLoading(true);
    try {
      const r = await getTaskReport(reportTaskId);
      setReport(r);
      message.success(t("settings.scheduler.reportLoaded"));
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setReportLoading(false);
    }
  };

  const handleRestoreTasks = async () => {
    setRestoreLoading(true);
    try {
      const ids = await restorePendingTasks();
      if (ids.length > 0) {
        message.success(t("settings.scheduler.restored", { count: ids.length }));
      } else {
        message.info(t("settings.scheduler.noRestorable"));
      }
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setRestoreLoading(false);
    }
  };

  return (
    <div>
      <SettingsGroup title={t("settings.scheduler.autoBackup")}>
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="scheduler:autoBackupEnabled"
        >
          <span>{t("settings.scheduler.enabled")}</span>
          <Switch
            id="scheduler-settings-switch-150"
            checked={backupSettings?.enabled ?? false}
            onChange={handleAutoBackupChange}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="scheduler:backupInterval">
          <span>{t("settings.scheduler.backupInterval")}</span>
          <div className="flex items-center gap-2">
            <InputNumber
              id="scheduler-settings-inputnumber-151"
              min={1}
              max={720}
              value={backupSettings?.intervalHours ?? 24}
              onChange={handleBackupIntervalChange}
              style={{ width: 80 }}
            />
            <span
              style={{ fontSize: 12, color: "var(--color-text-secondary)" }}
            >
              {t("settings.scheduler.hoursUnit")}
            </span>
          </div>
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="scheduler:maxBackupCount">
          <span>{t("settings.scheduler.maxCount")}</span>
          <InputNumber
            id="scheduler-settings-inputnumber-152"
            min={1}
            max={100}
            value={backupSettings?.maxCount ?? 10}
            onChange={async (v) => {
              if (!backupSettings || !v) {
                return;
              }
              await updateBackupSettings({ ...backupSettings, maxCount: v });
            }}
            style={{ width: 80 }}
          />
        </div>
        {backupSettings?.enabled && (
          <>
            <Divider style={{ margin: "4px 0" }} />
            <div style={rowStyle} className="flex items-center justify-between">
              <span
                style={{ fontSize: 12, color: "var(--color-text-secondary)" }}
              >
                {t("settings.scheduler.status")}
              </span>
              <Tag color="green">{t("settings.scheduler.running")}</Tag>
            </div>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.scheduler.webdavSync")}>
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="scheduler:webdavSyncEnabled"
        >
          <span>{t("settings.scheduler.enabled")}</span>
          <Switch
            id="scheduler-settings-switch-153"
            checked={settings.webdavSyncEnabled ?? false}
            onChange={handleWebdavSyncChange}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="scheduler:syncInterval">
          <span>{t("settings.scheduler.syncInterval")}</span>
          <Select
            id="scheduler-settings-select-154"
            value={settings.webdavSyncIntervalMinutes ?? 60}
            options={webdavIntervalOptions}
            onChange={handleWebdavIntervalChange}
            style={{ width: 120 }}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="scheduler:maxRemoteBackups"
        >
          <span>{t("settings.scheduler.maxRemoteBackups")}</span>
          <InputNumber
            id="scheduler-settings-inputnumber-155"
            min={1}
            max={100}
            value={settings.webdavMaxRemoteBackups ?? 10}
            onChange={(v) => v && saveSettings({ webdavMaxRemoteBackups: v })}
            style={{ width: 80 }}
          />
        </div>
        {settings.webdavSyncEnabled && (
          <>
            <Divider style={{ margin: "4px 0" }} />
            <div style={rowStyle} className="flex items-center justify-between">
              <span
                style={{ fontSize: 12, color: "var(--color-text-secondary)" }}
              >
                {t("settings.scheduler.status")}
              </span>
              <Tag color="green">{t("settings.scheduler.running")}</Tag>
            </div>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.scheduler.closedLoop")}>
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="scheduler:closedLoopEnabled"
        >
          <span>{t("settings.scheduler.enabled")}</span>
          <Switch
            id="scheduler-settings-switch-156"
            checked={settings.closedLoopEnabled ?? true}
            onChange={handleClosedLoopChange}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="scheduler:nudgeInterval">
          <span>{t("settings.scheduler.nudgeInterval")}</span>
          <Select
            id="scheduler-settings-select-157"
            value={settings.closedLoopIntervalMinutes ?? 5}
            options={closedLoopIntervalOptions}
            onChange={handleClosedLoopIntervalChange}
            style={{ width: 120 }}
          />
        </div>
        {(settings.closedLoopEnabled ?? true) && (
          <>
            <Divider style={{ margin: "4px 0" }} />
            <div style={rowStyle} className="flex items-center justify-between">
              <span
                style={{ fontSize: 12, color: "var(--color-text-secondary)" }}
              >
                {t("settings.scheduler.status")}
              </span>
              <Tag color="blue">{t("settings.scheduler.running")}</Tag>
            </div>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup
        title={t("settings.scheduler.customTasks")}
        extra={
          <div className="flex items-center gap-2">
            <Button
              size="small"
              icon={<RefreshCw size={14} />}
              onClick={handleRefreshAll}
            >
              {t("scheduler.refresh")}
            </Button>
            <Button
              size="small"
              icon={<Zap size={14} />}
              onClick={() => handleQuickCreate("daily_summary")}
            >
              {t("scheduler.dailyReport")}
            </Button>
            <Button
              size="small"
              icon={<Calendar size={14} />}
              onClick={() => handleQuickCreate("backup")}
            >
              {t("scheduler.backup")}
            </Button>
            <Button
              size="small"
              icon={<Rocket size={14} />}
              onClick={() => handleQuickCreate("cleanup")}
            >
              {t("scheduler.cleanup")}
            </Button>
            <Button
              type="primary"
              size="small"
              icon={<Plus size={14} />}
              onClick={openCreateModal}
            >
              {t("settings.scheduler.addTask")}
            </Button>
          </div>
        }
      >
        {customTasks.length === 0
          ? (
            <div
              style={{
                textAlign: "center",
                padding: "20px 0",
                color: "var(--color-text-secondary)",
              }}
            >
              {t("settings.scheduler.noTasks")}
            </div>
          )
          : (
            customTasks.map((task) => (
              <div key={task.id}>
                <div
                  style={rowStyle}
                  className="flex items-center justify-between"
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      style={{
                        fontWeight: 500,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {task.name}
                    </span>
                    <Tag color={getStatusColor(task.status)}>
                      {getStatusText(task.status)}
                    </Tag>
                    {task.lastResult
                      && (task.lastResult.skipped
                        ? (
                          <Tag color="default" style={{ fontSize: 10 }}>
                            {t("settings.scheduler.skipped")}
                          </Tag>
                        )
                        : (
                          <Tag
                            color={task.lastResult.success ? "green" : "red"}
                            style={{ fontSize: 10 }}
                          >
                            {task.lastResult.success ? "✓" : "✗"} {task.lastResult.durationMs}ms
                          </Tag>
                        ))}
                  </div>
                  <div className="flex items-center gap-1 flex-shrink-0">
                    <Button
                      type="text"
                      size="small"
                      icon={executing[task.id] ? <RefreshCw size={14} className="animate-spin" /> : <Play size={14} />}
                      onClick={() => handleExecuteNow(task.id)}
                      loading={executing[task.id]}
                      title={t("scheduler.executeNow")}
                    />
                    {task.status === "active"
                      ? (
                        <Button
                          type="text"
                          size="small"
                          icon={<Pause size={14} />}
                          onClick={() => handlePauseTask(task.id)}
                          title={t("settings.scheduler.pauseTask")}
                        />
                      )
                      : (
                        <Button
                          type="text"
                          size="small"
                          icon={<Play size={14} />}
                          onClick={() => handleResumeTask(task.id)}
                          title={t("settings.scheduler.resumeTask")}
                        />
                      )}
                    <Button
                      type="text"
                      size="small"
                      icon={<History size={14} />}
                      onClick={() => handleLoadHistory(task.id)}
                      title={t("scheduler.executionHistory")}
                    />
                    <Button
                      type="text"
                      size="small"
                      icon={<Edit2 size={14} />}
                      onClick={() => openEditModal(task)}
                      title={t("settings.scheduler.editTask")}
                    />
                    <Popconfirm
                      title={t("settings.scheduler.deleteTaskConfirm")}
                      onConfirm={() => handleDeleteTask(task.id)}
                      okText={t("common.yes")}
                      cancelText={t("common.no")}
                    >
                      <Button
                        type="text"
                        size="small"
                        danger
                        icon={<Trash2 size={14} />}
                        title={t("settings.scheduler.deleteTask")}
                      />
                    </Popconfirm>
                  </div>
                </div>
                {task.description && (
                  <div
                    style={{
                      fontSize: 12,
                      color: "var(--color-text-secondary)",
                      marginBottom: 4,
                    }}
                  >
                    {task.description}
                  </div>
                )}
                <div
                  style={{
                    fontSize: 12,
                    color: "var(--color-text-secondary)",
                    marginBottom: 4,
                  }}
                >
                  <Clock
                    size={12}
                    style={{ display: "inline", marginRight: 4 }}
                  />
                  <span>
                    {t("settings.scheduler.schedule")}: {formatScheduleDescription(task)}
                  </span>
                  <span style={{ marginLeft: 16 }}>
                    {t("settings.scheduler.nextRunAt")}: {formatNextRun(task.nextRunAt)}
                  </span>
                </div>

                {/* 执行历史 */}
                {expandedHistory[task.id] && (
                  <div style={{ marginTop: 8, marginBottom: 8 }}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t("scheduler.historyTitle")}
                    </Text>
                    {historyMap[task.id]?.length === 0
                      ? (
                        <div
                          style={{ fontSize: 12, color: token.colorTextTertiary, padding: "4px 0" }}
                        >
                          {t("scheduler.noRecords")}
                        </div>
                      )
                      : (
                        <div
                          style={{
                            maxHeight: 200,
                            overflowY: "auto",
                            marginTop: 4,
                          }}
                        >
                          {(historyMap[task.id] || []).slice(0, 20).map((rec) => {
                            // 历史表无 skipped 结构列：闸门跳过记录 success=true 且
                            // output 为「…本轮跳过」，按文案识别（与 services.rs 闸门文案对应）。
                            // 匹配词用码点写（跳过 = U+8DF3 U+8FC7），避免 CJK 字面量
                            const isSkipped = rec.success && !!rec.output?.includes("\u8df3\u8fc7");
                            return (
                              <div
                                key={rec.id}
                                style={{
                                  display: "flex",
                                  alignItems: "center",
                                  gap: 8,
                                  padding: "3px 6px",
                                  fontSize: 12,
                                  borderBottom: "1px solid var(--color-border)",
                                  backgroundColor: rec.success
                                    ? undefined
                                    : token.colorErrorBg,
                                }}
                              >
                                <Tag
                                  color={isSkipped
                                    ? "default"
                                    : rec.success
                                    ? "green"
                                    : "red"}
                                  style={{ fontSize: 10, margin: 0 }}
                                >
                                  {isSkipped
                                    ? t("settings.scheduler.skipped")
                                    : rec.success
                                    ? t("scheduler.success")
                                    : t("scheduler.failed")}
                                </Tag>
                                <span
                                  style={{
                                    flex: 1,
                                    overflow: "hidden",
                                    textOverflow: "ellipsis",
                                    whiteSpace: "nowrap",
                                  }}
                                >
                                  {rec.output
                                    ? translateBackendError(rec.output)
                                    : rec.error
                                    ? translateBackendError(rec.error)
                                    : "-"}
                                </span>
                                {!isSkipped && (
                                  <span style={{ color: token.colorTextTertiary, whiteSpace: "nowrap" }}>
                                    {rec.duration_ms}ms
                                  </span>
                                )}
                                <span
                                  style={{
                                    color: token.colorTextQuaternary,
                                    whiteSpace: "nowrap",
                                    fontSize: 10,
                                  }}
                                >
                                  {rec.started_at
                                    ? new Date(rec.started_at).toLocaleString()
                                    : "-"}
                                </span>
                              </div>
                            );
                          })}
                        </div>
                      )}
                  </div>
                )}
                <Divider style={{ margin: "4px 0" }} />
              </div>
            ))
          )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.scheduler.nightTask")}>
        {/* 成本预算门控 */}
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="scheduler:maxBudget"
        >
          <div className="flex items-center gap-2">
            <span>{t("settings.scheduler.maxBudget")}</span>
            {budget?.tripped && <Tag color="red">{t("settings.scheduler.budgetTripped")}</Tag>}
          </div>
          <div className="flex items-center gap-2">
            <InputNumber
              id="scheduler-settings-inputnumber-budget"
              min={0}
              step={0.1}
              precision={2}
              value={budgetInput ?? null}
              onChange={setBudgetInput}
              placeholder={t("settings.scheduler.budgetPlaceholder")}
              style={{ width: 120 }}
            />
            <Button
              size="small"
              loading={budgetLoading}
              onClick={handleSaveBudget}
            >
              {t("common.save")}
            </Button>
          </div>
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>
            {t("settings.scheduler.spent")}
          </span>
          <span>${(budget?.spent ?? 0).toFixed(4)}</span>
        </div>
        <Divider style={{ margin: "4px 0" }} />

        {/* 长时任务报告 */}
        <div style={rowStyle}>
          <div className="flex items-center gap-2">
            <Select
              style={{ flex: 1 }}
              placeholder={t("settings.scheduler.selectReportTask")}
              value={reportTaskId}
              onChange={setReportTaskId}
              options={customTasks.map((task) => ({
                value: task.id,
                label: task.name,
              }))}
              allowClear
            />
            <Button
              size="small"
              loading={reportLoading}
              onClick={handleLoadReport}
            >
              {t("settings.scheduler.viewReport")}
            </Button>
          </div>
          {report && (
            <div
              style={{
                marginTop: 8,
                fontSize: 12,
                color: "var(--color-text-secondary)",
              }}
            >
              <div>{t("settings.scheduler.reportStatus")}: {report.status}</div>
              <div>
                {t("settings.scheduler.reportDuration")}: {(report.durationMs / 1000).toFixed(1)}s
              </div>
              <div>
                {t("settings.scheduler.reportCost")}: ${report.costUsd.toFixed(4)}
              </div>
              <div>
                {t("settings.scheduler.reportApprovals")}: {report.approvals.length}
              </div>
              <div>
                {t("settings.scheduler.reportGeneratedAt")}: {new Date(report.generatedAt).toLocaleString()}
              </div>
            </div>
          )}
        </div>
        <Divider style={{ margin: "4px 0" }} />

        {/* 未完成任务恢复 */}
        <div style={rowStyle}>
          <Button
            size="small"
            icon={<RefreshCw size={14} />}
            loading={restoreLoading}
            onClick={handleRestoreTasks}
          >
            {t("settings.scheduler.restoreTasks")}
          </Button>
        </div>
      </SettingsGroup>

      <Modal
        title={editingTask
          ? t("settings.scheduler.editTask")
          : t("settings.scheduler.addTask")}
        open={taskModalOpen}
        onOk={handleCreateOrUpdateTask}
        onCancel={() => setTaskModalOpen(false)}
        confirmLoading={loading}
        okText={editingTask
          ? t("settings.scheduler.updateTask")
          : t("settings.scheduler.createTask")}
        cancelText={t("common.cancel")}
        width={700}
      >
        {!editingTask && (
          <>
            <div style={{ marginBottom: 16 }}>
              <label
                style={{ fontWeight: 500, display: "block", marginBottom: 8 }}
              >
                {t("settings.scheduler.selectTemplate") || "Select Template"}
              </label>

              <div style={{ marginBottom: 12 }}>
                <div style={{ fontSize: 12, color: token.colorTextTertiary, marginBottom: 6 }}>
                  {t("settings.scheduler.reportTemplates")}
                </div>
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "repeat(2, 1fr)",
                    gap: 8,
                  }}
                >
                  {taskTemplates.flatMap((template) =>
                    !template.workflow_id
                      ? [
                        <div
                          key={template.template_type}
                          role="button"
                          tabIndex={0}
                          onClick={() => handleTemplateSelect(template.template_type)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              handleTemplateSelect(template.template_type);
                            }
                          }}
                          style={{
                            padding: "10px",
                            border: selectedTemplate === template.template_type
                              ? "2px solid var(--color-primary)"
                              : "1px solid var(--color-border)",
                            borderRadius: 6,
                            cursor: "pointer",
                            backgroundColor: selectedTemplate === template.template_type
                              ? "var(--color-bg-tertiary)"
                              : "var(--color-bg-secondary)",
                            transition: "box-shadow 0.2s, transform 0.2s",
                          }}
                        >
                          <div
                            style={{
                              fontWeight: 500,
                              marginBottom: 2,
                              fontSize: 13,
                              color: "var(--color-text)",
                            }}
                          >
                            {t(`settings.scheduler.${template.template_type}`, {
                              defaultValue: template.name,
                            })}
                          </div>
                          <div
                            style={{
                              fontSize: 12,
                              color: "var(--color-text-secondary)",
                            }}
                          >
                            {t(`settings.scheduler.${template.template_type}Desc`, {
                              defaultValue: template.description,
                            })}
                          </div>
                        </div>,
                      ]
                      : []
                  )}
                </div>
              </div>

              <div>
                <div
                  style={{
                    fontSize: 12,
                    color: "var(--color-text-secondary)",
                    marginBottom: 6,
                  }}
                >
                  {t("settings.scheduler.workflowTemplates")}
                </div>
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "repeat(2, 1fr)",
                    gap: 8,
                  }}
                >
                  {taskTemplates.flatMap((template) =>
                    template.workflow_id
                      ? [
                        <div
                          key={template.template_type}
                          role="button"
                          tabIndex={0}
                          onClick={() => handleTemplateSelect(template.template_type)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              handleTemplateSelect(template.template_type);
                            }
                          }}
                          style={{
                            padding: "10px",
                            border: selectedTemplate === template.template_type
                              ? "2px solid var(--color-success)"
                              : "1px solid var(--color-border)",
                            borderRadius: 6,
                            cursor: "pointer",
                            backgroundColor: selectedTemplate === template.template_type
                              ? "var(--color-bg-tertiary)"
                              : "var(--color-bg-secondary)",
                            transition: "box-shadow 0.2s, transform 0.2s",
                          }}
                        >
                          <div
                            style={{
                              fontWeight: 500,
                              marginBottom: 2,
                              fontSize: 13,
                              color: "var(--color-text)",
                            }}
                          >
                            {t(`settings.scheduler.${template.template_type}`, {
                              defaultValue: template.name,
                            })}
                          </div>
                          <div
                            style={{
                              fontSize: 12,
                              color: "var(--color-text-secondary)",
                            }}
                          >
                            {t(`settings.scheduler.${template.template_type}Desc`, {
                              defaultValue: template.description,
                            })}
                          </div>
                        </div>,
                      ]
                      : []
                  )}
                </div>
              </div>
            </div>
            <Divider />
          </>
        )}
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          <Form.Item
            name="name"
            label={t("settings.scheduler.taskName")}
            rules={[{ required: true, message: "Please input task name" }]}
          >
            <Input name="name" placeholder={t("settings.scheduler.taskName")} />
          </Form.Item>
          <Form.Item
            name="description"
            label={t("settings.scheduler.taskDescription")}
          >
            <Input.TextArea
              name="description"
              placeholder={t("settings.scheduler.taskDescription")}
              rows={2}
            />
          </Form.Item>
          <Divider>{t("settings.scheduler.scheduleSettings")}</Divider>
          <Form.Item
            name="cron_expression"
            label={t("settings.scheduler.scheduleType")}
            rules={[{ validator: cronValidator }]}
          >
            <CronScheduleBuilder />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
