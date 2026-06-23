// SPDX-License-Identifier: AGPL-3.0-only

/* eslint-disable react-refresh/only-export-components */
import { Tooltip } from "@/components/layout/Tooltip";
import { invoke } from "@/lib/invoke";
import { type ExecutionPhase, TERMINAL_PHASES, useExecutionStore } from "@/stores/feature/executionStore";
import { listen } from "@tauri-apps/api/event";
 
import {
  Alert,
  Badge,
  Button,
  Card,
  Divider,
  InputNumber,
  message,
  Modal,
  Progress,
  Slider,
  Space,
  Switch,
  Tag,
  theme,
  Typography,
} from "antd";
import {
  AlertTriangle,
  Brain,
  CheckCircle,
  Clock,
  Lightbulb,
  RefreshCw,
  Settings,
  Sparkles,
  TrendingUp,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

interface QualityMetricsData {
  task_success_score: number;
  tool_efficiency_score: number;
  iteration_efficiency_score: number;
  time_efficiency_score: number;
  error_recovery_score: number;
  goal_completion_score: number;
  overall_weighted_score: number;
}

interface ReflectionData {
  task_id: string;
  timestamp: string;
  quality_score: number;
  quality_analysis: string;
  efficiency_analysis: string;
  error_patterns: string[];
  reusable_patterns: string[];
  knowledge_suggestions: string[];
  improvement_suggestions: string[];
  overall_summary: string;
  quality_metrics: QualityMetricsData | null;
}

interface Insight {
  id: string;
  category: string;
  title: string;
  content: string;
  source_task_id: string;
  confidence: number;
  tags: string[];
  created_at: string;
  usage_count: number;
  last_used: string | null;
}

interface ReflectionPanelProps {
  /** 当前会话 ID；传此 prop 时，组件会从 executionStore 派生 executionRecord 并自动触发 */
  conversationId?: string;
  /** 旧版 API：外部显式传入的 taskId（向后兼容） */
  taskId?: string;
  /** 旧版 API：外部显式传入的 taskDescription */
  taskDescription?: string;
  /** 旧版 API：外部显式传入的执行记录 */
  executionRecord?: {
    success: boolean;
    error?: string;
    toolsUsed: string[];
    iterations: number;
    durationMs: number;
    startedAtMs?: number;
  };
  /** 反思完成回调 */
  onReflectionComplete?: (reflection: ReflectionData) => void;
  /** 是否自动在 agent 结束时触发（默认 true，需要 conversationId） */
  autoTrigger?: boolean;
}

const categoryIcons: Record<string, React.ReactNode> = {
  error_pattern: <AlertTriangle size={14} className="text-red-500" />,
  success_pattern: <CheckCircle size={14} className="text-green-500" />,
  optimization: <TrendingUp size={14} className="text-blue-500" />,
  knowledge: <Lightbulb size={14} className="text-yellow-500" />,
  workflow: <RefreshCw size={14} className="text-purple-500" />,
  tool_usage: <Sparkles size={14} className="text-orange-500" />,
};

const categoryColors: Record<string, string> = {
  error_pattern: "red",
  success_pattern: "green",
  optimization: "blue",
  knowledge: "gold",
  workflow: "purple",
  tool_usage: "orange",
};

/** 从 executionStore 派生执行记录的 hook */
function useDerivedExecutionRecord(conversationId?: string) {
  const phase: ExecutionPhase | undefined = useExecutionStore((s) => s.phases[conversationId ?? ""]) as
    | ExecutionPhase
    | undefined;
  const pool = useExecutionStore((s) => conversationId ? s.agentPool[conversationId] ?? [] : []);
  const agentStatus = useExecutionStore((s) => conversationId ? s.agentStatus[conversationId] : undefined);

  // L1: 捕获挂载时的 now，避免 render 中调用 Date.now()（react-hooks/purity）
  const [now] = useState(() => Date.now());

  return useMemo(() => {
    if (!conversationId) {
      return null;
    }
    if (!pool || pool.length === 0) {
      return null;
    }
    // M3: 仅在终态才派生（completed/failed/cancelled），使用强类型 Set
    if (!phase || !TERMINAL_PHASES.has(phase)) {
      return null;
    }
    // 工具去重（按 name，保留顺序）
    const seen = new Set<string>();
    const toolsUsed: string[] = [];
    let earliest = Number.POSITIVE_INFINITY;
    let latestEnd = 0;
    let lastError: string | undefined;
    for (const item of pool) {
      if (item.name && !seen.has(item.name)) {
        seen.add(item.name);
        toolsUsed.push(item.name);
      }
      if (typeof item.startedAt === "number") {
        earliest = Math.min(earliest, item.startedAt);
      }
      const end = typeof item.duration === "number" && typeof item.startedAt === "number"
        ? item.startedAt + item.duration
        : (typeof item.startedAt === "number" ? item.startedAt : 0);
      latestEnd = Math.max(latestEnd, end);
      if (item.error) {
        lastError = item.error;
      }
    }
    if (!Number.isFinite(earliest)) {
      earliest = now;
    }
    if (latestEnd === 0) {
      latestEnd = now;
    }
    const durationMs = Math.max(0, latestEnd - earliest);
    // M2: success 仅当 phase=completed 且**没有非空错误**。空字符串 agentStatus 视为 OK。
    const hasRealError = !!lastError || (!!agentStatus && agentStatus.trim().length > 0);
    const success = phase === "completed" && !hasRealError;
    const description = pool[pool.length - 1]?.taskDescription
      ?? pool[pool.length - 1]?.currentTask
      ?? pool[pool.length - 1]?.name
      ?? `Conversation ${conversationId}`;
    return {
      taskId: `${conversationId}-${earliest}`,
      taskDescription: description,
      success,
      error: lastError ?? (hasRealError ? agentStatus : undefined),
      toolsUsed,
      iterations: pool.length,
      durationMs,
      startedAtMs: earliest,
    };
  }, [conversationId, phase, pool, agentStatus, now]);
}

function QualityScore({
  score,
  t,
}: {
  score: number;
  t: (key: string) => string;
}) {
  const { token } = theme.useToken();
  const color = score >= 7 ? token.colorSuccess : score >= 4 ? token.colorWarning : token.colorError;
  const label = score >= 7
    ? t("reflection.excellent")
    : score >= 4
    ? t("reflection.good")
    : t("reflection.needsImprovement");

  return (
    <div className="flex items-center gap-3">
      <Progress
        type="circle"
        percent={score * 10}
        size={50}
        strokeColor={color}
        format={() => score}
      />
      <div>
        <Text strong style={{ fontSize: 16 }}>
          {t("reflection.qualityScore")}
        </Text>
        <div>
          <Tag color={score >= 7 ? "green" : score >= 4 ? "gold" : "red"}>
            {label}
          </Tag>
        </div>
      </div>
    </div>
  );
}

function AnalysisSection({
  title,
  icon,
  content,
  type,
}: {
  title: string;
  icon: React.ReactNode;
  content: string;
  type: "success" | "warning" | "info" | "error";
}) {
  return (
    <Alert
      type={type}
      message={
        <div className="flex items-center gap-2">
          {icon}
          <Text strong>{title}</Text>
        </div>
      }
      description={<Text className="text-sm whitespace-pre-line">{content}</Text>}
      className="mb-3"
    />
  );
}

function PatternList({
  patterns,
  type,
  t,
}: {
  patterns: string[];
  type: "error" | "success";
  t: (key: string) => string;
}) {
  if (patterns.length === 0) {
    return null;
  }

  return (
    <div className="mb-3">
      <Text strong className="mb-2 block">
        {type === "error"
          ? t("reflection.errorPatterns")
          : t("reflection.reusablePatterns")}
      </Text>
      <div className="flex flex-wrap gap-2">
        {patterns.map((pattern) => (
          <Tooltip
            key={pattern}
            title={pattern.length > 50 ? pattern : undefined}
          >
            <Tag
              color={type === "error" ? "red" : "green"}
              icon={type === "error" ? <AlertTriangle size={12} /> : <CheckCircle size={12} />}
            >
              {pattern.length > 50 ? pattern.substring(0, 50) + "..." : pattern}
            </Tag>
          </Tooltip>
        ))}
      </div>
    </div>
  );
}

function InsightCard({
  insight,
  onFeedback,
  onDelete,
}: {
  insight: Insight;
  onFeedback?: (id: string, useful: boolean) => void;
  onDelete?: (id: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Card size="small" className="insight-card">
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-2">
          {categoryIcons[insight.category] || <Lightbulb size={14} />}
          <Text strong className="text-sm">
            {insight.title}
          </Text>
        </div>
        <Tag color={categoryColors[insight.category] || "default"}>
          {(insight.confidence * 100).toFixed(0)}%
        </Tag>
      </div>
      <Text type="secondary" className="text-xs block mt-1">
        {insight.content.length > 100
          ? insight.content.substring(0, 100) + "..."
          : insight.content}
      </Text>
      <div className="flex items-center gap-2 mt-2 flex-wrap">
        {insight.tags.slice(0, 3).map((tag) => (
          <Tag key={tag} className="text-xs">
            {tag}
          </Tag>
        ))}
        {insight.usage_count > 0 && (
          <Badge
            count={insight.usage_count}
            size="small"
            title={t("reflection.usageCount")}
          />
        )}
        <div className="ml-auto flex items-center gap-1">
          {onFeedback && (
            <>
              <Button
                size="small"
                type="text"
                icon={<CheckCircle size={12} className="text-green-500" />}
                onClick={() => onFeedback(insight.id, true)}
                title={t("reflection.useful")}
              />
              <Button
                size="small"
                type="text"
                icon={<XCircle size={12} className="text-red-500" />}
                onClick={() => onFeedback(insight.id, false)}
                title={t("reflection.notUseful")}
              />
            </>
          )}
          {onDelete && (
            <Button
              size="small"
              type="text"
              danger
              icon={<AlertTriangle size={12} />}
              onClick={() => onDelete(insight.id)}
              title={t("reflection.deleteInsight")}
            />
          )}
        </div>
      </div>
    </Card>
  );
}

interface ReflectionConfig {
  enabled: boolean;
  min_quality_threshold: number;
  store_insights: boolean;
  max_history: number;
  insight_decay_days: number;
  max_insights: number;
  use_error_classifier: boolean;
}

const REFLECTION_CONFIG_DEFAULT: ReflectionConfig = {
  enabled: true,
  min_quality_threshold: 5,
  store_insights: true,
  max_history: 200,
  insight_decay_days: 30,
  max_insights: 500,
  use_error_classifier: true,
};

// L7: 跨挂载缓存（模块级）— 避免每次打开设置弹窗都向后端请求一次
let __reflectionConfigCache: { value: ReflectionConfig; at: number } | null = null;
const CONFIG_CACHE_TTL_MS = 30_000;

function SectionHeader({
  icon,
  title,
}: {
  icon: React.ReactNode;
  title: string;
}) {
  const { token } = theme.useToken();
  return (
    <div
      className="flex items-center gap-2 mb-3 mt-1"
      style={{ color: token.colorPrimary }}
    >
      <span
        className="inline-flex items-center justify-center rounded-md"
        style={{
          width: 24,
          height: 24,
          background: `${token.colorPrimary}15`,
        }}
      >
        {icon}
      </span>
      <Text strong style={{ fontSize: 13, letterSpacing: 0.4 }}>
        {title}
      </Text>
      <div
        className="flex-1 ml-2"
        style={{
          height: 1,
          background: `linear-gradient(to right, ${token.colorPrimary}40, transparent)`,
        }}
      />
    </div>
  );
}

function FieldRow({
  label,
  hint,
  control,
}: {
  label: string;
  hint?: string;
  control: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4 mb-3">
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium">{label}</div>
        {hint && (
          <div className="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">
            {hint}
          </div>
        )}
      </div>
      <div className="flex-shrink-0">{control}</div>
    </div>
  );
}

function ReflectionSettingsModal({
  open,
  onClose,
  onSaved,
}: {
  open: boolean;
  onClose: () => void;
  onSaved: (cfg: ReflectionConfig) => void;
}) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [cfg, setCfg] = useState<ReflectionConfig | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    // L7: 先用缓存填充，再后台刷新
    Promise.resolve().then(() => {
      if (cancelled) { return; }
      if (__reflectionConfigCache && Date.now() - __reflectionConfigCache.at < CONFIG_CACHE_TTL_MS) {
        setCfg(__reflectionConfigCache.value);
        setDirty(false);
      }
      return invoke<ReflectionConfig>("get_reflection_config");
    })
      .then((remote) => {
        if (cancelled || !remote) { return; }
        setCfg(remote);
        setDirty(false);
        __reflectionConfigCache = { value: remote, at: Date.now() };
      })
      .catch(() => {
        if (!cancelled && !__reflectionConfigCache) {
          setCfg(REFLECTION_CONFIG_DEFAULT);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const update = <K extends keyof ReflectionConfig>(
    key: K,
    value: ReflectionConfig[K],
  ) => {
    setCfg((prev) => (prev ? { ...prev, [key]: value } : prev));
    setDirty(true);
  };

  const handleSave = async () => {
    if (!cfg) {
      return;
    }
    setSaving(true);
    try {
      await invoke("update_reflection_config", { config: cfg });
      // L7: 写后立即刷新缓存
      __reflectionConfigCache = { value: cfg, at: Date.now() };
      message.success(t("reflection.settings.saved"));
      onSaved(cfg);
      onClose();
    } catch (e) {
      message.error(`${t("reflection.settings.saveFailed")}: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const handleReset = () => {
    setCfg(REFLECTION_CONFIG_DEFAULT);
    setDirty(true);
  };

  return (
    <Modal
      open={open}
      onCancel={onClose}
      title={
        <div className="flex items-center gap-2">
          <Settings size={18} style={{ color: token.colorPrimary }} />
          <span>{t("reflection.settings.title")}</span>
        </div>
      }
      width={560}
      destroyOnClose
      footer={
        <div className="flex items-center justify-between">
          <Button
            type="text"
            icon={<RefreshCw size={14} />}
            onClick={handleReset}
            disabled={!cfg}
          >
            {t("reflection.settings.reset")}
          </Button>
          <Space>
            <Button onClick={onClose}>{t("reflection.settings.cancel")}</Button>
            <Button
              type="primary"
              loading={saving}
              disabled={!cfg || !dirty}
              onClick={handleSave}
            >
              {t("reflection.settings.save")}
            </Button>
          </Space>
        </div>
      }
    >
      {cfg
        ? (
          <div>
            <Text type="secondary" className="text-xs block mb-4">
              {t("reflection.settings.subtitle")}
            </Text>

            <SectionHeader
              icon={<Brain size={14} />}
              title={t("reflection.settings.core")}
            />
            <FieldRow
              label={t("reflection.settings.enabled")}
              hint={t("reflection.settings.enabledHint")}
              control={
                <Switch
                  checked={cfg.enabled}
                  onChange={(v) => update("enabled", v)}
                />
              }
            />

            <Divider style={{ margin: "12px 0" }} />

            <SectionHeader
              icon={<CheckCircle size={14} />}
              title={t("reflection.settings.qualitySection")}
            />
            <FieldRow
              label={t("reflection.settings.qualityThreshold")}
              hint={`${t("reflection.settings.qualityThresholdHint")} · ${cfg.min_quality_threshold}/10`}
              control={
                <div style={{ width: 200 }}>
                  <Slider
                    min={1}
                    max={10}
                    value={cfg.min_quality_threshold}
                    onChange={(v) => update("min_quality_threshold", v as number)}
                    marks={{ 1: "1", 5: "5", 10: "10" }}
                  />
                </div>
              }
            />

            <Divider style={{ margin: "12px 0" }} />

            <SectionHeader
              icon={<Clock size={14} />}
              title={t("reflection.settings.storageSection")}
            />
            <FieldRow
              label={t("reflection.settings.maxHistory")}
              hint={t("reflection.settings.maxHistoryHint")}
              control={
                <InputNumber
                  min={10}
                  max={2000}
                  step={10}
                  value={cfg.max_history}
                  onChange={(v) => update("max_history", (v as number | null) ?? 200)}
                />
              }
            />
            <FieldRow
              label={t("reflection.settings.maxInsights")}
              hint={t("reflection.settings.maxInsightsHint")}
              control={
                <InputNumber
                  min={10}
                  max={2000}
                  step={10}
                  value={cfg.max_insights}
                  onChange={(v) => update("max_insights", (v as number | null) ?? 500)}
                />
              }
            />

            <Divider style={{ margin: "12px 0" }} />

            <SectionHeader
              icon={<Lightbulb size={14} />}
              title={t("reflection.settings.insightSection")}
            />
            <FieldRow
              label={t("reflection.settings.storeInsights")}
              hint={t("reflection.settings.storeInsightsHint")}
              control={
                <Switch
                  checked={cfg.store_insights}
                  onChange={(v) => update("store_insights", v)}
                />
              }
            />
            <FieldRow
              label={t("reflection.settings.decayDays")}
              hint={t("reflection.settings.decayDaysHint")}
              control={
                <InputNumber
                  min={0}
                  max={365}
                  step={1}
                  value={cfg.insight_decay_days}
                  onChange={(v) => update("insight_decay_days", (v as number | null) ?? 30)}
                  addonAfter="d"
                />
              }
            />

            <Divider style={{ margin: "12px 0" }} />

            <SectionHeader
              icon={<AlertTriangle size={14} />}
              title={t("reflection.settings.errorSection")}
            />
            <FieldRow
              label={t("reflection.settings.useErrorClassifier")}
              hint={t("reflection.settings.useErrorClassifierHint")}
              control={
                <Switch
                  checked={cfg.use_error_classifier}
                  onChange={(v) => update("use_error_classifier", v)}
                />
              }
            />
          </div>
        )
        : (
          <div className="flex items-center justify-center h-40">
            <Text type="secondary">...</Text>
          </div>
        )}
    </Modal>
  );
}

function QualityMetricsBreakdown({
  metrics,
  t,
}: {
  metrics: QualityMetricsData;
  t: (key: string) => string;
}) {
  const { token } = theme.useToken();
  const dimensions = [
    {
      key: "taskSuccessScore",
      value: metrics.task_success_score,
      color: token.colorPrimary,
    },
    {
      key: "toolEfficiencyScore",
      value: metrics.tool_efficiency_score,
      color: token.colorSuccess,
    },
    {
      key: "iterationEfficiencyScore",
      value: metrics.iteration_efficiency_score,
      color: "#722ed1",
    },
    {
      key: "timeEfficiencyScore",
      value: metrics.time_efficiency_score,
      color: token.colorWarning,
    },
    {
      key: "errorRecoveryScore",
      value: metrics.error_recovery_score,
      color: "#eb2f96",
    },
    {
      key: "goalCompletionScore",
      value: metrics.goal_completion_score,
      color: "#13c2c2",
    },
  ];

  return (
    <div className="mb-4">
      <Text strong className="mb-2 block">
        {t("reflection.qualityMetrics")}
      </Text>
      <div className="grid grid-cols-1 gap-2">
        {dimensions.map((dim) => (
          <div key={dim.key} className="flex items-center gap-2">
            <Text className="text-xs w-24 flex-shrink-0">
              {t(`reflection.${dim.key}`)}
            </Text>
            <Progress
              percent={(dim.value / 10) * 100}
              size="small"
              strokeColor={dim.color}
              format={() => dim.value.toFixed(1)}
              className="flex-1"
            />
          </div>
        ))}
      </div>
    </div>
  );
}

export function ReflectionPanel({
  conversationId,
  taskId: externalTaskId,
  taskDescription: externalTaskDescription,
  executionRecord: externalExecutionRecord,
  onReflectionComplete,
  autoTrigger = true,
}: ReflectionPanelProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [isRefecting, setIsRefecting] = useState(false);
  const [reflection, setReflection] = useState<ReflectionData | null>(null);
  const [insights, setInsights] = useState<Insight[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [autoEnabled, setAutoEnabled] = useState(autoTrigger);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [runVersion, setRunVersion] = useState(0);
  const reflectedRunRef = useRef<string | null>(null);

  // 监听后端自动触发的 reflection-updated 事件，使面板刷新
  useEffect(() => {
    if (!conversationId) {
      return;
    }
    let unlisten: (() => void) | undefined;
    // L4: debounce 多个事件落入同一窗口
    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    const DEBOUNCE_MS = 500;
    void (async () => {
      try {
        unlisten = await listen<{
          taskId: string;
          conversationId: string;
          qualityScore?: number;
          bridgedCount?: number;
          newInsightsCount?: number;
          reflection?: ReflectionData;
        }>(
          "reflection-updated",
          (event) => {
            if (event.payload?.conversationId !== conversationId) {
              return;
            }
            // M1: 优先使用后端 emit 出来的完整 reflection，避免前端再调一次 reflect_on_task
            if (event.payload.reflection) {
              setReflection(event.payload.reflection);
              onReflectionComplete?.(event.payload.reflection);
            }
            // 始终触发一次增量刷新（拉取最新 insights）
            if (debounceTimer) {
              clearTimeout(debounceTimer);
            }
            debounceTimer = setTimeout(() => {
              setRunVersion((v) => v + 1);
              debounceTimer = null;
            }, DEBOUNCE_MS);
          },
        );
      } catch {
        // listen 仅在 Tauri 环境下可用，非 Tauri 时静默忽略
      }
    })();
    return () => {
      unlisten?.();
      if (debounceTimer) {
        clearTimeout(debounceTimer);
      }
    };
  }, [conversationId, onReflectionComplete]);

  // 派生执行记录
  const derived = useDerivedExecutionRecord(conversationId);
  const record = externalExecutionRecord ?? derived ?? null;
  const taskId = externalTaskId ?? derived?.taskId ?? conversationId ?? "";
  const taskDescription = externalTaskDescription
    ?? derived?.taskDescription
    ?? "";

  const performReflection = useCallback(async () => {
    if (!record || !taskId) {
      return;
    }
    const runKey = `${taskId}#${record.startedAtMs ?? record.durationMs}#${runVersion}`;
    if (reflectedRunRef.current === runKey) {
      return;
    }
    reflectedRunRef.current = runKey;

    setIsRefecting(true);
    setReflection(null);
    setError(null);

    try {
      const result = await invoke<ReflectionData>("reflect_on_task", {
        // 同时传 camelCase 和 snake_case，兼容 tauri v1/v2 命令参数解析
        taskId,
        task_id: taskId,
        taskDescription,
        task_description: taskDescription,
        success: record.success,
        error: record.error ?? null,
        toolsUsed: record.toolsUsed,
        tools_used: record.toolsUsed,
        iterations: record.iterations,
        durationMs: record.durationMs,
        duration_ms: record.durationMs,
        startedAtMs: record.startedAtMs ?? null,
        started_at_ms: record.startedAtMs ?? null,
      });

      setReflection(result);
      onReflectionComplete?.(result);

      try {
        const fetchedInsights = await invoke<Insight[]>(
          "get_reflection_insights",
          { category: null },
        );
        setInsights(fetchedInsights.slice(-10));
      } catch {
        // insights fetch is non-critical
      }
    } catch (e) {
      setError(String(e));
      // 失败时允许重试
      reflectedRunRef.current = null;
    } finally {
      setIsRefecting(false);
    }
  }, [record, taskId, taskDescription, onReflectionComplete, runVersion]);

  // 自动触发：当派生记录或外部记录变化时
  useEffect(() => {
    if (!autoEnabled) {
      return;
    }
    if (!record || !taskId) {
      return;
    }
    const runKey = `${taskId}#${record.startedAtMs ?? record.durationMs}#${runVersion}`;
    if (reflectedRunRef.current === runKey) {
      return;
    }
    reflectedRunRef.current = runKey;

    let cancelled = false;
    setIsRefecting(true);
    setReflection(null);
    setError(null);

    invoke<ReflectionData>("reflect_on_task", {
      // 同时传 camelCase 和 snake_case，兼容 tauri v1/v2 命令参数解析
      taskId,
      task_id: taskId,
      taskDescription,
      task_description: taskDescription,
      success: record.success,
      error: record.error ?? null,
      toolsUsed: record.toolsUsed,
      tools_used: record.toolsUsed,
      iterations: record.iterations,
      durationMs: record.durationMs,
      duration_ms: record.durationMs,
      startedAtMs: record.startedAtMs ?? null,
      started_at_ms: record.startedAtMs ?? null,
    })
      .then((result) => {
        if (cancelled) { return; }
        setReflection(result);
        onReflectionComplete?.(result);

        return invoke<Insight[]>("get_reflection_insights", { category: null }).then((fetchedInsights) => {
          if (!cancelled) {
            setInsights(fetchedInsights.slice(-10));
          }
        });
      })
      .catch((e) => {
        if (cancelled) { return; }
        setError(String(e));
        // 失败时允许重试
        reflectedRunRef.current = null;
      })
      .finally(() => {
        if (!cancelled) { setIsRefecting(false); }
      });
    return () => {
      cancelled = true;
    };
  }, [autoEnabled, record, taskId, taskDescription, onReflectionComplete, runVersion]);

  const handleStartReflection = () => {
    reflectedRunRef.current = null;
    void performReflection();
  };

  const handleReset = () => {
    setIsRefecting(false);
    setReflection(null);
    setError(null);
    setInsights([]);
    reflectedRunRef.current = null;
  };

  const handleInsightFeedback = useCallback(async (id: string, useful: boolean) => {
    // M7: 反馈即时反馈给用户
    try {
      const updated = await invoke<Insight | null>("record_insight_feedback", { id, useful });
      if (updated) {
        message.success(
          useful
            ? t("reflection.feedback.thanksUseful")
            : t("reflection.feedback.thanksNotUseful"),
        );
        // 同步本地 cache 的 confidence
        setInsights((prev) =>
          prev.map((it) => (it.id === id
            ? { ...it, confidence: updated.confidence }
            : it)
          )
        );
      } else {
        message.warning(t("reflection.feedback.notFound"));
      }
    } catch (e) {
      message.error(`${t("reflection.feedback.failed")}: ${e}`);
    }
  }, [t]);

  const handleInsightDelete = useCallback(async (id: string) => {
    // M7: 删前轻量确认
    const confirmed = typeof window !== "undefined" && typeof window.confirm === "function"
      ? window.confirm(t("reflection.delete.confirm"))
      : true;
    if (!confirmed) {
      return;
    }
    try {
      const ok = await invoke<boolean>("delete_insight", { id });
      if (ok) {
        setInsights((prev) => prev.filter((i) => i.id !== id));
        message.success(t("reflection.delete.success"));
      } else {
        message.warning(t("reflection.feedback.notFound"));
      }
    } catch (e) {
      message.error(`${t("reflection.delete.failed")}: ${e}`);
    }
  }, [t]);

  if (error) {
    return (
      <Card size="small" className="reflection-panel">
        <Alert
          type="error"
          message={t("reflection.reflectError")}
          description={error}
        />
        <Button
          type="link"
          icon={<RefreshCw size={14} />}
          onClick={handleReset}
          className="mt-2"
        >
          {t("reflection.retry")}
        </Button>
      </Card>
    );
  }

  if (!record) {
    return (
      <Card size="small" className="reflection-panel">
        <div className="flex items-center justify-center h-32 text-zinc-400">
          <Brain size={24} className="mr-2" />
          <Text type="secondary">{t("reflection.noReflection")}</Text>
        </div>
        <div className="mt-2 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Switch
              size="small"
              checked={autoEnabled}
              onChange={setAutoEnabled}
            />
            <Text type="secondary" className="text-xs">
              {t("reflection.autoTrigger")}
            </Text>
          </div>
          <Button
            type="primary"
            size="small"
            icon={<Brain size={12} />}
            onClick={handleStartReflection}
            disabled={!taskId}
          >
            {t("reflection.startReflection")}
          </Button>
        </div>
      </Card>
    );
  }

  if (isRefecting && !reflection) {
    return (
      <Card
        size="small"
        className="reflection-panel"
        title={
          <div className="flex items-center gap-2">
            <Brain size={16} className="text-blue-500 animate-pulse" />
            <span>{t("reflection.reflecting")}</span>
          </div>
        }
      >
        <div className="flex items-center justify-center h-40 flex-col gap-4">
          <Brain size={48} className="text-blue-400 animate-pulse" />
          <Text type="secondary">{t("reflection.analyzing")}</Text>
        </div>
      </Card>
    );
  }

  return (
    <Card
      size="small"
      className="reflection-panel"
      title={
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Brain size={16} className="text-purple-500" />
            <span>{t("reflection.title")}</span>
            <Tag color="purple">
              {reflection?.task_id || taskId || "unknown"}
            </Tag>
          </div>
          <div className="flex items-center gap-1">
            <Tooltip title={t("reflection.settings.title")}>
              <Button
                type="text"
                size="small"
                icon={
                  <Settings
                    size={14}
                    style={{ color: token.colorPrimary }}
                  />
                }
                onClick={() => setSettingsOpen(true)}
                aria-label={t("reflection.settings.title")}
              />
            </Tooltip>
            <Tooltip title={t("reflection.retry")}>
              <Button
                type="text"
                size="small"
                icon={<RefreshCw size={14} />}
                onClick={handleReset}
                aria-label={t("reflection.retry")}
              />
            </Tooltip>
          </div>
        </div>
      }
    >
      {reflection && (
        <>
          <div className="mb-4">
            <QualityScore score={reflection.quality_score} t={t} />
          </div>

          {reflection.quality_metrics && (
            <QualityMetricsBreakdown
              metrics={reflection.quality_metrics}
              t={t}
            />
          )}

          <div className="grid grid-cols-2 gap-4 mb-4">
            <div>
              <Text type="secondary" className="text-xs">
                {t("reflection.errorPatterns")}
              </Text>
              <div className="text-lg font-medium text-red-500">
                {reflection.error_patterns.length}
              </div>
            </div>
            <div>
              <Text type="secondary" className="text-xs">
                {t("reflection.reusablePatterns")}
              </Text>
              <div className="text-lg font-medium text-green-500">
                {reflection.reusable_patterns.length}
              </div>
            </div>
          </div>

          <AnalysisSection
            title={t("reflection.qualityAnalysis")}
            icon={<CheckCircle size={14} className="text-green-500" />}
            content={reflection.quality_analysis}
            type="success"
          />

          <AnalysisSection
            title={t("reflection.efficiencyAnalysis")}
            icon={<Clock size={14} className="text-blue-500" />}
            content={reflection.efficiency_analysis}
            type="info"
          />

          <PatternList
            patterns={reflection.error_patterns}
            type="error"
            t={t}
          />
          <PatternList
            patterns={reflection.reusable_patterns}
            type="success"
            t={t}
          />

          {reflection.knowledge_suggestions.length > 0 && (
            <div className="mb-3">
              <Text strong className="mb-2 block">
                {t("reflection.knowledgeSuggestions")}
              </Text>
              {reflection.knowledge_suggestions.map((suggestion) => (
                <Alert
                  key={suggestion}
                  type="info"
                  message={suggestion}
                  className="mb-2"
                  icon={<Lightbulb size={14} />}
                />
              ))}
            </div>
          )}

          {reflection.improvement_suggestions.length > 0 && (
            <div className="mb-3">
              <Text strong className="mb-2 block">
                {t("reflection.improvementSuggestions")}
              </Text>
              {reflection.improvement_suggestions.map((suggestion) => (
                <Alert
                  key={suggestion}
                  type="warning"
                  message={suggestion}
                  className="mb-2"
                />
              ))}
            </div>
          )}

          {insights.length > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between mb-2">
                <Text strong>{t("reflection.generatedInsights")}</Text>
                <Badge count={insights.length} />
              </div>
              <div className="space-y-2">
                {insights.map((insight) => (
                  <InsightCard
                    key={insight.id}
                    insight={insight}
                    onFeedback={handleInsightFeedback}
                    onDelete={handleInsightDelete}
                  />
                ))}
              </div>
            </div>
          )}

          <Alert
            type="info"
            message={t("reflection.summary")}
            description={reflection.overall_summary}
            className="mt-4"
          />
        </>
      )}
      <ReflectionSettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onSaved={() => {
          // 配置变更后无需立即重算（已生效于下次 reflect 调用）
        }}
      />
    </Card>
  );
}

export function useReflection() {
  const [reflection, setReflection] = useState<ReflectionData | null>(null);
  const [isRefecting, setIsRefecting] = useState(false);
  const [insights, setInsights] = useState<Insight[]>([]);
  const [error, setError] = useState<string | null>(null);

  const startReflection = useCallback(
    async (params: {
      taskId: string;
      taskDescription: string;
      success: boolean;
      error?: string;
      toolsUsed: string[];
      iterations: number;
      durationMs: number;
      startedAtMs?: number;
    }) => {
      setIsRefecting(true);
      setReflection(null);
      setError(null);

      try {
        const result = await invoke<ReflectionData>("reflect_on_task", {
          task_id: params.taskId,
          task_description: params.taskDescription,
          success: params.success,
          error: params.error || null,
          tools_used: params.toolsUsed,
          iterations: params.iterations,
          duration_ms: params.durationMs,
          started_at_ms: params.startedAtMs ?? null,
        });

        setReflection(result);

        try {
          const fetchedInsights = await invoke<Insight[]>(
            "get_reflection_insights",
            { category: null },
          );
          setInsights(fetchedInsights.slice(-10));
        } catch {
          // non-critical
        }

        return result;
      } catch (e) {
        setError(String(e));
        throw e;
      } finally {
        setIsRefecting(false);
      }
    },
    [],
  );

  const reset = useCallback(() => {
    setReflection(null);
    setIsRefecting(false);
    setInsights([]);
    setError(null);
  }, []);

  return {
    reflection,
    isRefecting,
    insights,
    error,
    startReflection,
    reset,
  };
}
