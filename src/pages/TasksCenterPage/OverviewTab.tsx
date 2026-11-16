// SPDX-License-Identifier: AGPL-3.0-only

import {
  ensureCardContributionsLoaded,
  getContributions,
  type PluginCardContribution,
  useCardRegistryVersion,
} from "@/lib/dynamicUI/pluginCardRegistry";
import { showBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { invokePluginCommand } from "@/lib/pluginCommand";
import { resolvePluginForCommand } from "@/lib/pluginCommandResolve";
import { useTaskCenterStore } from "@/stores";
import { Button, Card, Empty, Input, List, message, Modal, Progress, Space, Statistic, Tag, Typography } from "antd";
import { AlertTriangle, CheckCircle2, Clock4, ListChecks, Loader2, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { getAdapter } from "./adapters/registry";
import { formatDuration } from "./formatUtils";
import type { RunState, UnifiedRun } from "./model";
import { activeRuns, attentionRuns, computeHealth, summarizeWindow } from "./model";
import { RunStatusTag } from "./RunStatusTag";
import { SourceChip } from "./SourceChip";

/**
 * 概览（Temporal Web / Windmill UI 范式）：
 * 健康统计卡 + 需关注 + Live（正在运行）+ 近 24h 汇总。
 */

/** 卡片行集的最小形状：行级动作只需要 `id` 与展示字段，其余字段不解释。 */
interface CardRow {
  id: string;
  improvementSummary?: string;
}

function isCardRow(row: unknown): row is CardRow {
  return typeof row === "object" && row !== null && typeof (row as { id?: unknown }).id === "string";
}

/** tone（antd 色板名）→ 健康卡数字色；未登记的 tone 退回概览卡原紫色。 */
const CARD_TONE_HEX: Record<string, string> = { gold: "#faad14" };

export function OverviewTab({
  onViewRuns,
}: {
  /** 跳到全部运行 Tab（状态筛选已由健康卡写入 store） */
  onViewRuns: () => void;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const runs = useTaskCenterStore((s) => s.runs);
  const loading = useTaskCenterStore((s) => s.loading);
  const setFilters = useTaskCenterStore((s) => s.setFilters);
  const selectRun = useTaskCenterStore((s) => s.selectRun);
  const loadRuns = useTaskCenterStore((s) => s.loadRuns);

  // P4：任务卡投影（#216 ③-B）。待审批进化卡由宿主投影声明（`host:evolution-approval`），
  // 不再直接 import 股票域 store / 抽屉（#178④）。计数与行集同源（{count} 契约 =
  // summaryCommand 行数，后端不出第二个 count 读数）；行级审批按钮由宿主渲染
  // （§130.2：schema args 无行级插值，绑不了行 id）。
  const cardVersion = useCardRegistryVersion();
  const taskCards = useMemo(() => getContributions("task_card"), [cardVersion]);
  const [cardRows, setCardRows] = useState<Record<string, CardRow[]>>({});
  const [activeCardKey, setActiveCardKey] = useState<string | null>(null);
  // 拒绝走受控弹层：需要输入原因，okButtonProps 要随输入联动 —— 静态 Modal.confirm 的
  // keep-open 得靠 onOk 返回 rejected promise，会漏 unhandled rejection（antd 内部派生链）。
  const [rejectRowId, setRejectRowId] = useState<string | null>(null);
  const [rejectReason, setRejectReason] = useState("");

  // #341：`summaryCommand` 可能是宿主命令或插件命名命令（如宿主审批卡指向
  // `stock_list_pending_evolution_plans`）—— 按插件注册表解析路由，解析不到才走宿主 invoke。
  const runSummaryCommand = useCallback(async (name: string): Promise<unknown[]> => {
    const pluginId = await resolvePluginForCommand(name);
    if (pluginId) {
      return invokePluginCommand<unknown[]>(name);
    }
    return invoke<unknown[]>(name);
  }, []);

  const fetchCardRows = useCallback((cards: readonly PluginCardContribution[]) => {
    for (const card of cards) {
      void runSummaryCommand(card.summaryCommand)
        .then((rows) => {
          setCardRows((prev) => ({ ...prev, [card.key]: rows.filter(isCardRow) }));
        })
        .catch((error: unknown) => {
          // 读不到行集按空处理（同注册表口径：少卡片，不让页面报错）。
          console.warn("[OverviewTab] 卡片取数失败，按空行集处理", card.summaryCommand, error);
          setCardRows((prev) => ({ ...prev, [card.key]: [] }));
        });
    }
  }, [runSummaryCommand]);

  useEffect(() => {
    void ensureCardContributionsLoaded();
  }, []);

  useEffect(() => {
    fetchCardRows(taskCards);
  }, [taskCards, fetchCardRows]);

  const activeCard = taskCards.find((c) => c.key === activeCardKey) ?? null;
  const activeRows = activeCard ? cardRows[activeCard.key] ?? [] : [];

  // 宿主卡的 title / actions[].title 是宿主 i18n key（消费方 t() 渲染）；插件卡是自带纯文本。
  const cardTitle = (card: PluginCardContribution) => (card.pluginId === "host" ? t(card.title) : card.title);

  const runCardAction = (card: PluginCardContribution, actionId: string, rowId: string) => {
    const action = card.actions.find((a) => a.id === actionId);
    if (!action || rowId === "") {
      return;
    }
    if (action.id === "reject") {
      // 拒绝必填原因（后端同样拒空）⇒ 受控弹层收集输入，确定按钮随非空联动。
      setRejectRowId(rowId);
      setRejectReason("");
      return;
    }
    const run = (extra: Record<string, unknown>) => {
      // #343：动作命令可能是宿主或插件命名命令 —— 与 summaryCommand 同一路由。
      void (async () => {
        const pluginId = await resolvePluginForCommand(action.command);
        const invoker = pluginId ? invokePluginCommand : invoke;
        invoker(action.command, { historyId: rowId, reviewer: null, ...extra })
          .then(() => {
            message.success(t(`stockAnalysis.evolutionApproval.${action.id}Success`));
            fetchCardRows([card]);
          })
          .catch((e) => showBackendError(message, e));
      })();
    };
    if (!action.mutating) {
      void run({});
      return;
    }
    Modal.confirm({
      title: t(action.title),
      onOk: () => run({}),
    });
  };

  const submitReject = () => {
    const card = activeCard;
    if (!card || rejectRowId === null) {
      return;
    }
    void invokePluginCommand("stock_reject_evolution_plan", {
      historyId: rejectRowId,
      reviewer: null,
      reason: rejectReason.trim(),
    })
      .then(() => {
        message.success(t("stockAnalysis.evolutionApproval.rejectSuccess"));
        fetchCardRows([card]);
        setRejectRowId(null);
      })
      // 失败不关弹层：错误提示后可原地重试
      .catch((e) => showBackendError(message, e));
  };

  const health = useMemo(() => computeHealth(runs), [runs]);
  const attention = useMemo(() => attentionRuns(runs).slice(0, 8), [runs]);
  const live = useMemo(() => activeRuns(runs), [runs]);
  const summary = useMemo(() => {
    const counts = summarizeWindow(runs, 24 * 60 * 60 * 1000);
    return {
      ...counts,
      total: counts.succeeded + counts.failed + counts.canceled,
    };
  }, [runs]);

  const viewByState = (states: RunState[]) => {
    setFilters({ states });
    onViewRuns();
  };

  const handleRetry = async (run: UnifiedRun) => {
    const adapter = getAdapter(run.source);
    if (!adapter?.retry) {
      return;
    }
    try {
      await adapter.retry(run.nativeId);
      message.success(t("tasksCenter.action.retryStarted"));
      await loadRuns(true);
    } catch (e) {
      showBackendError(message, e);
    }
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      {/* 健康统计卡 */}
      <Space wrap size="middle">
        <HealthCard
          color="#1677ff"
          icon={<Loader2 size={18} />}
          label={t("tasksCenter.health.running")}
          value={health.running}
          onClick={() => viewByState(["running"])}
        />
        <HealthCard
          color="#8c8c8c"
          icon={<Clock4 size={18} />}
          label={t("tasksCenter.health.queued")}
          value={health.queued}
          onClick={() => viewByState(["queued"])}
        />
        <HealthCard
          color="#ff4d4f"
          icon={<AlertTriangle size={18} />}
          label={t("tasksCenter.health.failed")}
          value={health.failed}
          onClick={() => viewByState(["failed", "waiting"])}
        />
        <HealthCard
          color="#faad14"
          icon={<Clock4 size={18} />}
          label={t("tasksCenter.health.waiting")}
          value={health.waiting}
          onClick={() => viewByState(["waiting", "paused"])}
        />
        {taskCards.map((card) => (
          <HealthCard
            key={card.key}
            color={CARD_TONE_HEX[card.tone ?? ""] ?? "#722ed1"}
            icon={<ShieldCheck size={18} />}
            label={cardTitle(card)}
            value={cardRows[card.key]?.length ?? 0}
            onClick={card.actions.length > 0 ? () => setActiveCardKey(card.key) : undefined}
          />
        ))}
      </Space>

      {/* 需关注 */}
      <Card
        size="small"
        title={
          <Space>
            <AlertTriangle size={15} />
            {t("tasksCenter.attention.title")}
          </Space>
        }
        extra={
          <Button type="link" size="small" onClick={() => viewByState(["failed", "waiting"])}>
            {t("tasksCenter.attention.viewAll")}
          </Button>
        }
      >
        {attention.length === 0
          ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("tasksCenter.attention.empty")}
            />
          )
          : (
            <List
              size="small"
              dataSource={attention}
              renderItem={(run) => (
                <List.Item
                  actions={run.capabilities.retry
                    ? [
                      <Button
                        key="retry"
                        size="small"
                        type="link"
                        onClick={() => void handleRetry(run)}
                      >
                        {t("tasksCenter.action.retry")}
                      </Button>,
                    ]
                    : undefined}
                >
                  <Space size={8}>
                    <RunStatusTag state={run.state} badges={run.badges} />
                    <SourceChip source={run.source} />
                    <Typography.Text>{run.name}</Typography.Text>
                    {run.error?.message && (
                      <Typography.Text type="secondary" style={{ fontSize: 12 }} ellipsis>
                        {run.error.message}
                      </Typography.Text>
                    )}
                  </Space>
                </List.Item>
              )}
            />
          )}
      </Card>

      {/* Live 正在运行 */}
      <Card
        size="small"
        title={
          <Space>
            <Loader2 size={15} className="animate-spin" />
            {t("tasksCenter.live.title")}
          </Space>
        }
      >
        {live.length === 0
          ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("tasksCenter.live.empty")}
            />
          )
          : (
            <List
              size="small"
              dataSource={live}
              renderItem={(run: UnifiedRun) => (
                <List.Item
                  onClick={() => selectRun(run)}
                  style={{ cursor: "pointer", width: "100%" }}
                >
                  <Space
                    direction="vertical"
                    size={2}
                    style={{ width: "100%" }}
                  >
                    <Space
                      style={{ justifyContent: "space-between", width: "100%" }}
                    >
                      <Space size={8}>
                        <Tag color="processing">
                          {t(`tasksCenter.state.${run.state}`)}
                        </Tag>
                        <SourceChip source={run.source} />
                        <Typography.Text strong>{run.name}</Typography.Text>
                        {run.stage && (
                          <Typography.Text type="secondary">
                            {run.stage}
                          </Typography.Text>
                        )}
                      </Space>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {run.startedAt
                          ? formatDuration(Date.now() - run.startedAt, locale)
                          : "-"}
                      </Typography.Text>
                    </Space>
                    {run.progress !== undefined && <Progress percent={run.progress} size="small" />}
                  </Space>
                </List.Item>
              )}
            />
          )}
      </Card>

      {/* 近 24h 汇总 */}
      <Card size="small" title={t("tasksCenter.summary.title")}>
        <Space wrap size="large">
          <Statistic
            title={t("tasksCenter.summary.succeeded")}
            value={summary.succeeded}
            prefix={<CheckCircle2 size={16} color="#52c41a" />}
          />
          <Statistic
            title={t("tasksCenter.summary.failed")}
            value={summary.failed}
            prefix={<AlertTriangle size={16} color="#ff4d4f" />}
          />
          <Statistic
            title={t("tasksCenter.summary.canceled")}
            value={summary.canceled}
          />
          <Statistic title={t("tasksCenter.summary.total")} value={summary.total} />
        </Space>
        {loading && (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            <ListChecks size={12} /> {t("tasksCenter.runs.refresh")}
          </Typography.Text>
        )}
      </Card>

      {
        /* P4：卡片行级审批弹层（任务中心第二入口）。行级动作由宿主渲染（§130.2）：
          行集来自卡片 summaryCommand（与计数同源），动作按钮来自投影 actions[]。 */
      }
      <Modal
        open={activeCard !== null}
        title={activeCard ? cardTitle(activeCard) : null}
        footer={null}
        width={720}
        onCancel={() => setActiveCardKey(null)}
      >
        {activeRows.length === 0
          ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("stockAnalysis.evolutionApproval.emptyPending")}
            />
          )
          : (
            <List
              size="small"
              dataSource={activeRows}
              rowKey={(row) => row.id}
              renderItem={(row) => (
                <List.Item
                  actions={activeCard!.actions.map((action) => (
                    <Button
                      key={action.id}
                      size="small"
                      type="link"
                      onClick={() => runCardAction(activeCard!, action.id, row.id)}
                    >
                      {activeCard!.pluginId === "host" ? t(action.title) : action.title}
                    </Button>
                  ))}
                >
                  <Typography.Text ellipsis>{row.improvementSummary || row.id}</Typography.Text>
                </List.Item>
              )}
            />
          )}
      </Modal>

      {/* P4：拒绝原因弹层（受控）：原因非空才放行；失败留在原地可重试 */}
      <Modal
        open={rejectRowId !== null}
        title={t("stockAnalysis.evolutionApproval.reject")}
        okButtonProps={{ disabled: rejectReason.trim() === "" }}
        onOk={submitReject}
        onCancel={() => setRejectRowId(null)}
      >
        <Input.TextArea
          value={rejectReason}
          onChange={(e) => setRejectReason(e.target.value)}
          placeholder={t("stockAnalysis.evolutionApproval.rejectReasonPlaceholder")}
        />
      </Modal>
    </Space>
  );
}

function HealthCard({
  color,
  icon,
  label,
  value,
  onClick,
}: {
  color: string;
  icon: React.ReactNode;
  label: string;
  value: number;
  onClick?: () => void;
}) {
  return (
    <Card
      size="small"
      hoverable={onClick !== undefined}
      onClick={onClick}
      styles={{ body: { padding: 16, minWidth: 130 } }}
    >
      <Statistic
        title={
          <span style={{ color }}>
            {icon} {label}
          </span>
        }
        value={value}
        valueStyle={{ color }}
      />
    </Card>
  );
}
