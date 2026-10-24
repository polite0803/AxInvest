/**
 * R1 复盘→进化：EvolutionDriftPanel
 *
 * 三段 UI：
 * 1. 权重表（每个 (strategy, period) 的现权重 / 旧权重 / 变化 / 胜率 / 样本 / 解释）
 * 2. 权重时间线 sparkline（点击某行后展开）
 * 3. 调整原因 Top 5
 *
 * 数据来源：stockAnalysisStore.evolutionDashboard
 * 重算按钮：调用 stockAnalysisStore.recalcEvolutionNow
 */

import {
  ArrowDownOutlined,
  ArrowUpOutlined,
  AuditOutlined,
  MinusOutlined,
  ReloadOutlined,
  SettingOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { App, Button, Empty, Modal, Space, Spin, Table, Tag, Tooltip } from "antd";
import type { ColumnsType } from "antd/es/table";
import type { ReactNode } from "react";
import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { invoke } from "@/lib/invoke";
import { agreementColor } from "@/lib/stock-analysis-utils";
import {
  type EvolutionHistoryEntry,
  type EvolutionRecentChangeRow,
  type EvolutionStrategyStatRow,
  type EvolutionStrategySummaryRow,
  useStockAnalysisStore,
} from "@/stores/feature/stockAnalysisStore";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { evolutionStatusMeta, formatEvolutionTime, parseTrigger } from "./evolution-ui-utils";
import { EvolutionApprovalDrawer } from "./EvolutionApprovalDrawer";

const trendIcon: Record<string, ReactNode> = {
  up: <ArrowUpOutlined style={{ color: "#cf1322" }} />,
  down: <ArrowDownOutlined style={{ color: "#389e0d" }} />,
  stable: <MinusOutlined style={{ color: "#8c8c8c" }} />,
};

export function EvolutionDriftPanel() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const dashboard = useStockAnalysisStore((s) => s.evolutionDashboard);
  const recalculating = useStockAnalysisStore((s) => s.evolutionRecalculating);
  const lastError = useStockAnalysisStore((s) => s.evolutionLastError);
  const fetchDashboard = useStockAnalysisStore((s) => s.fetchEvolutionDashboard);
  const recalc = useStockAnalysisStore((s) => s.recalcEvolutionNow);
  const fetchAgreementHistory = useStockAnalysisStore((s) => s.fetchAgreementScoreHistory);
  const agreementHistory = useStockAnalysisStore((s) => s.agreementScoreHistory);
  const agreementLoading = useStockAnalysisStore((s) => s.agreementScoreHistoryLoading);
  const evolutionHistory = useStockAnalysisStore((s) => s.evolutionHistory);
  const evolutionHistoryLoading = useStockAnalysisStore((s) => s.evolutionHistoryLoading);
  const evolutionTriggering = useStockAnalysisStore((s) => s.evolutionTriggering);
  const fetchEvolutionHistory = useStockAnalysisStore((s) => s.fetchEvolutionHistory);
  const triggerStockEvolution = useStockAnalysisStore((s) => s.triggerStockEvolution);
  // P4：待审批进化计划
  const pendingEvolutionPlans = useStockAnalysisStore((s) => s.pendingEvolutionPlans);
  const fetchPendingEvolutionPlans = useStockAnalysisStore((s) => s.fetchPendingEvolutionPlans);
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [timelineData, setTimelineData] = useState<
    Array<{ appliedAt: number; newWeight: number; deltaPct: number; trigger: string }>
  >(
    [],
  );
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [calibrating, setCalibrating] = useState(false);
  const [calibrateResult, setCalibrateResult] = useState<
    {
      bestParams: Record<string, number> | null;
      grid: Array<Record<string, number>>;
      totalReflections: number;
      parsedSuggestions: number;
      // 阶段 3 / 决-3：结构层（L2/L3）被挡在参数网格外的条数与「未声明层级」条数。
      // 两者都不是 0 时必须在弹窗里出声，否则「本轮没调出参数」与「本轮根因本来就不在
      // 参数层」在用户眼里是同一件事。
      structuralExcluded?: number;
      tierUnlabelled?: number;
      message?: string;
    } | null
  >(null);
  const [calibrateModalOpen, setCalibrateModalOpen] = useState(false);
  // P4：进化审批抽屉
  const [approvalOpen, setApprovalOpen] = useState(false);
  const [approvalSelected, setApprovalSelected] = useState<EvolutionHistoryEntry | null>(null);

  useEffect(() => {
    fetchDashboard(asOfDate);
    fetchAgreementHistory(50);
    fetchEvolutionHistory(50);
    fetchPendingEvolutionPlans();
  }, [
    asOfDate,
    fetchDashboard,
    fetchAgreementHistory,
    fetchEvolutionHistory,
    fetchPendingEvolutionPlans,
  ]);

  const openApproval = useCallback((row: EvolutionHistoryEntry) => {
    setApprovalSelected(row);
    setApprovalOpen(true);
  }, []);

  // 审批/拒绝/回滚后：历史与 dashboard 都可能变化（accepted 后生效配置切换）
  const handleApprovalChanged = useCallback(async () => {
    await fetchEvolutionHistory(50);
    await fetchDashboard(asOfDate);
  }, [fetchEvolutionHistory, fetchDashboard, asOfDate]);

  const handleRecalc = async () => {
    await recalc(asOfDate);
    if (!lastError) {
      message.success(t("stockAnalysis.evolutionDrift.recalcSuccess"));
    }
  };

  // 批3-C：手动触发一次进化（触发原因/改动/回测对比透传展示在下方历史区）
  const handleTriggerEvolution = async (reason?: string) => {
    await triggerStockEvolution(reason);
    await fetchEvolutionHistory(50);
    message.success(t("stockAnalysis.evolutionDrift.triggerSuccess"));
  };

  const handleSelect = async (record: EvolutionStrategyStatRow) => {
    const key = `${record.strategyId}_${record.period}`;
    setSelectedKey(key);
    setTimelineLoading(true);
    try {
      const points = await (await import("@/lib/invoke")).invoke<
        Array<{
          appliedAt: number;
          newWeight: number;
          oldWeight: number;
          deltaPct: number;
          trigger: string;
        }>
      >("get_evolution_drift_timeline", {
        strategyId: record.strategyId,
        period: record.period,
        limit: 30,
      });
      setTimelineData(
        points.map((p) => ({
          appliedAt: p.appliedAt,
          newWeight: p.newWeight,
          deltaPct: p.deltaPct,
          trigger: p.trigger,
        })),
      );
    } catch (e) {
      console.error("[EvolutionDriftPanel] timeline fetch failed:", e);
      setTimelineData([]);
    } finally {
      setTimelineLoading(false);
    }
  };

  const handleCalibrate = useCallback(async () => {
    setCalibrating(true);
    setCalibrateResult(null);
    try {
      const result = await invoke<{
        bestParams: Record<string, number> | null;
        grid: Array<Record<string, number>>;
        totalReflections: number;
        parsedSuggestions: number;
        structuralExcluded?: number;
        tierUnlabelled?: number;
        message?: string;
      }>("calibrate_portfolio_mgr_params");
      setCalibrateResult(result);
      setCalibrateModalOpen(true);
      if (result.bestParams) {
        message.success(t("stockAnalysis.evolutionDrift.calibrateSuccess"));
      } else {
        message.info(result.message || t("stockAnalysis.evolutionDrift.calibrateNoData"));
      }
    } catch (e) {
      console.error("[EvolutionDriftPanel] calibrate failed:", e);
      message.error(t("stockAnalysis.evolutionDrift.calibrateFailed", { msg: String(e) }));
    } finally {
      setCalibrating(false);
    }
  }, [message, t]);

  const handleApplyBest = useCallback(async () => {
    if (!calibrateResult?.bestParams) { return; }
    const bp = calibrateResult.bestParams;
    const updates = Object.entries(bp)
      .filter(([key]) => key !== "score")
      .map(([key, value]) => {
        const paramName = key.replace(/([A-Z])/g, "_$1").toLowerCase();
        return { param: paramName, value };
      });
    try {
      await invoke("apply_param_suggestions", { updates });
      message.success(t("stockAnalysis.evolutionDrift.calibrateApplyDone"));
      setCalibrateModalOpen(false);
    } catch (e) {
      console.error("[EvolutionDriftPanel] apply best params failed:", e);
      message.error(t("stockAnalysis.evolutionDrift.calibrateFailed", { msg: String(e) }));
    }
  }, [calibrateResult, message, t]);

  if (!dashboard) {
    return (
      <div style={{ padding: 16, textAlign: "center" }}>
        <Spin description={t("stockAnalysis.evolutionDrift.loading")} />
        {lastError && (
          <div style={{ marginTop: 12, color: "#cf1322" }}>
            {t("stockAnalysis.evolutionDrift.error", { msg: lastError })}
          </div>
        )}
      </div>
    );
  }

  const columns: ColumnsType<EvolutionStrategyStatRow> = [
    {
      title: t("stockAnalysis.evolutionDrift.table.strategy"),
      dataIndex: "strategyId",
      key: "strategyId",
      width: 100,
      render: (v: string) => <Tag color="blue">{v}</Tag>,
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.period"),
      dataIndex: "period",
      key: "period",
      width: 80,
      render: (v: string) => <Tag>{v}</Tag>,
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.oldWeight"),
      dataIndex: "oldWeight",
      key: "oldWeight",
      width: 90,
      align: "right",
      render: (v: number) => v.toFixed(2),
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.newWeight"),
      dataIndex: "newWeight",
      key: "newWeight",
      width: 90,
      align: "right",
      render: (v: number) => (
        <strong style={{ color: v > 1.0 ? "#cf1322" : v < 1.0 ? "#389e0d" : undefined }}>
          {v.toFixed(2)}
        </strong>
      ),
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.delta"),
      dataIndex: "deltaPct",
      key: "deltaPct",
      width: 90,
      align: "right",
      render: (v: number) => {
        if (Math.abs(v) < 0.5) { return <span style={{ color: "#8c8c8c" }}>—</span>; }
        const color = v > 0 ? "#cf1322" : "#389e0d";
        return <span style={{ color, fontWeight: 500 }}>{(v > 0 ? "+" : "") + v.toFixed(1) + "%"}</span>;
      },
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.winRate"),
      dataIndex: "winRate",
      key: "winRate",
      width: 80,
      align: "right",
      render: (v: number) => `${(v * 100).toFixed(0)}%`,
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.sampleSize"),
      dataIndex: "sampleSize",
      key: "sampleSize",
      width: 80,
      align: "right",
      render: (v: number) => (
        <Tooltip title={t("stockAnalysis.evolutionDrift.table.confidenceLabel", { c: 0 })}>
          <Tag color={v >= 20 ? "green" : v >= 5 ? "orange" : "default"}>{v}</Tag>
        </Tooltip>
      ),
    },
    {
      title: t("stockAnalysis.evolutionDrift.table.rationale"),
      dataIndex: "rationale",
      key: "rationale",
      ellipsis: true,
    },
  ];

  return (
    <React.Fragment>
      <div style={{ padding: 16 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
          <h3 style={{ margin: 0 }}>
            <ThunderboltOutlined /> {t("stockAnalysis.evolutionDrift.title")}
          </h3>
          <Button
            type="primary"
            icon={<ReloadOutlined />}
            loading={recalculating}
            onClick={handleRecalc}
          >
            {t("stockAnalysis.evolutionDrift.recalcNow")}
          </Button>
          <Button
            icon={<SettingOutlined />}
            loading={calibrating}
            onClick={handleCalibrate}
          >
            {t("stockAnalysis.evolutionDrift.calibrateParams")}
          </Button>
          <Button
            type="dashed"
            icon={<ThunderboltOutlined />}
            loading={evolutionTriggering}
            onClick={() => handleTriggerEvolution(undefined)}
          >
            {t("stockAnalysis.evolutionDrift.triggerEvolution")}
          </Button>
        </div>

        <div style={{ marginBottom: 12, fontSize: 12, color: "#8c8c8c" }}>
          {t("stockAnalysis.evolutionDrift.lastRecalcAt", {
            time: formatEvolutionTime(dashboard.lastRecalcAt),
          })}
          {asOfDate && (
            <Tag color="purple" style={{ marginLeft: 8 }}>
              {t("stockAnalysis.evolutionDrift.replayMode", { date: asOfDate })}
            </Tag>
          )}
          {
            /* #31：权重分母按**起算代际**筛样 ⇒ 必须说清「为什么样本变少」——
              「早于起算代」与「代际未知」的处置不同（前者等新样本，后者去补盖章）。 */
          }
          {dashboard != null && (
            <div style={{ marginTop: 4 }}>
              {t("stockAnalysis.evolutionDrift.generationScope", {
                floor: dashboard.generationFloor ?? 0,
                preFloor: dashboard.excludedPreFloorGeneration ?? 0,
                unknown: dashboard.excludedUnknownGeneration ?? 0,
              })}
            </div>
          )}
        </div>

        {/* P4：待人工审批的进化计划（计划-审批两段式：聚合落 pending_approval 后在此处理） */}
        {(pendingEvolutionPlans?.length ?? 0) > 0 && (
          <div
            style={{
              marginBottom: 16,
              padding: "10px 12px",
              border: "1px solid #ffd591",
              borderRadius: 6,
              background: "#fff7e6",
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <Space>
                <AuditOutlined style={{ color: "#fa8c16" }} />
                <strong>
                  {t("stockAnalysis.evolutionApproval.panelPendingTitle", {
                    count: pendingEvolutionPlans!.length,
                  })}
                </strong>
              </Space>
              <Button
                size="small"
                type="primary"
                ghost
                onClick={() => openApproval(pendingEvolutionPlans![0])}
              >
                {t("stockAnalysis.evolutionApproval.reviewAction")}
              </Button>
            </div>
            <ul style={{ margin: "6px 0 0", paddingLeft: 18 }}>
              {pendingEvolutionPlans!.slice(0, 5).map((p) => (
                <li
                  key={p.id}
                  style={{ cursor: "pointer", fontSize: 12, marginBottom: 2 }}
                  onClick={() => openApproval(p)}
                >
                  <Tag color="orange">{p.evolutionType}</Tag>
                  <span style={{ color: "#8c8c8c" }}>{formatEvolutionTime(p.createdAt)}</span>
                  {" · "}
                  {parseTrigger(p.trigger, t)}
                </li>
              ))}
            </ul>
          </div>
        )}

        <div style={{ marginBottom: 24 }}>
          <h4 style={{ marginTop: 0 }}>
            {t("stockAnalysis.evolutionDrift.summaryTitle")}
          </h4>
          <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
            {dashboard.strategySummary.map((s: EvolutionStrategySummaryRow) => (
              <div
                key={s.strategyId}
                style={{
                  border: "1px solid #d9d9d9",
                  borderRadius: 4,
                  padding: "8px 12px",
                  minWidth: 140,
                }}
              >
                <div style={{ fontSize: 12, color: "#8c8c8c" }}>{s.strategyId}</div>
                <div style={{ display: "flex", alignItems: "center", gap: 4, marginTop: 4 }}>
                  {trendIcon[s.trend] ?? trendIcon.stable}
                  <span style={{ fontSize: 18, fontWeight: 600 }}>{s.avgWeight.toFixed(2)}</span>
                </div>
                <div style={{ fontSize: 11, color: "#8c8c8c", marginTop: 2 }}>
                  {t("stockAnalysis.evolutionDrift.summarySamples", { n: s.totalSamples })} ·{" "}
                  {t("stockAnalysis.evolutionDrift.summaryWinRate", { p: (s.avgWinRate * 100).toFixed(0) })}
                </div>
              </div>
            ))}
          </div>
        </div>

        <h4>{t("stockAnalysis.evolutionDrift.tableTitle")}</h4>
        <Table
          size="small"
          rowKey={(r) => `${r.strategyId}_${r.period}`}
          columns={columns}
          dataSource={dashboard.stats}
          pagination={false}
          rowClassName={(r) => `${r.strategyId}_${r.period}` === selectedKey ? "ant-table-row-selected" : ""}
          onRow={(r) => ({
            onClick: () => handleSelect(r),
            style: { cursor: "pointer" },
          })}
          locale={{
            emptyText: <Empty description={t("stockAnalysis.evolutionDrift.noData")} />,
          }}
        />

        {selectedKey && (
          <div style={{ marginTop: 16, padding: 12, background: "#fafafa", borderRadius: 4 }}>
            <h4 style={{ marginTop: 0 }}>
              {t("stockAnalysis.evolutionDrift.timelineTitle", { key: selectedKey })}
            </h4>
            {timelineLoading
              ? <Spin size="small" />
              : timelineData.length === 0
              ? <Empty description={t("stockAnalysis.evolutionDrift.noTimeline")} />
              : <TimelineSparkline points={timelineData} />}
          </div>
        )}

        <div style={{ marginTop: 24 }}>
          <h4>{t("stockAnalysis.evolutionDrift.reasonsTitle")}</h4>
          {dashboard.recentChanges.length === 0
            ? <Empty description={t("stockAnalysis.evolutionDrift.noReasons")} />
            : (
              <ul style={{ paddingLeft: 16, margin: 0 }}>
                {dashboard.recentChanges.map((r: EvolutionRecentChangeRow) => (
                  <li key={r.id} style={{ marginBottom: 8, fontSize: 13 }}>
                    <Tag color="blue">{r.strategyId}</Tag>
                    <Tag>{r.period}</Tag>
                    {
                      /* 分析链 trigger（cron/manual/rule）：与荐股闭环的 reco-loop 分键空间，
                        dashboard 已按 trigger 过滤；标签保留是为了审计时能直接读出来源。 */
                    }
                    <Tag color="purple" data-testid="evolution-change-trigger">{r.trigger}</Tag>
                    <Tag color={r.deltaPct > 0 ? "red" : r.deltaPct < 0 ? "green" : "default"}>
                      {(r.deltaPct > 0 ? "+" : "") + r.deltaPct.toFixed(1) + "%"}
                    </Tag>
                    <span style={{ color: "#8c8c8c", marginLeft: 8 }}>
                      {formatEvolutionTime(r.appliedAt)}
                    </span>
                    {r.rationale && (
                      <div style={{ color: "#595959", marginTop: 2, marginLeft: 8 }}>
                        {r.rationale}
                      </div>
                    )}
                  </li>
                ))}
              </ul>
            )}
        </div>

        {/* 双视角一致性趋势 — 结束处插入校准 Modal */}
      </div>{" "}
      {/* 关闭主 content div */}

      <Modal
        title={
          <>
            <SettingOutlined /> {t("stockAnalysis.evolutionDrift.calibrateParams")}
          </>
        }
        open={calibrateModalOpen}
        onCancel={() => setCalibrateModalOpen(false)}
        width={720}
        footer={calibrateResult?.bestParams
          ? [
            <Button key="close" onClick={() => setCalibrateModalOpen(false)}>{t("common.close")}</Button>,
            <Button key="apply" type="primary" onClick={handleApplyBest}>
              {t("stockAnalysis.evolutionDrift.calibrateApplyBest")}
            </Button>,
          ]
          : [<Button key="close" onClick={() => setCalibrateModalOpen(false)}>{t("common.close")}</Button>]}
      >
        {calibrateResult?.bestParams && (
          <div>
            <h4>{t("stockAnalysis.evolutionDrift.calibrateBestTitle")}</h4>
            <div style={{ background: "var(--bg-secondary, #f5f5f5)", padding: 12, borderRadius: 6, marginBottom: 16 }}>
              <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
                {[
                  { key: "buyThreshold", label: t("stockAnalysis.evolutionDrift.calibrateBuyThreshold") },
                  { key: "increaseThreshold", label: t("stockAnalysis.evolutionDrift.calibrateIncreaseThreshold") },
                  { key: "holdThreshold", label: t("stockAnalysis.evolutionDrift.calibrateHoldThreshold") },
                  { key: "watchThreshold", label: t("stockAnalysis.evolutionDrift.calibrateWatchThreshold") },
                  { key: "reduceThreshold", label: t("stockAnalysis.evolutionDrift.calibrateReduceThreshold") },
                  { key: "capExtreme", label: t("stockAnalysis.evolutionDrift.calibrateCapExtreme") },
                  { key: "capHigh", label: t("stockAnalysis.evolutionDrift.calibrateCapHigh") },
                  { key: "capMid", label: t("stockAnalysis.evolutionDrift.calibrateCapMid") },
                  { key: "priorBull", label: t("stockAnalysis.evolutionDrift.calibratePriorBull") },
                  {
                    key: "priorSideways",
                    label: t("stockAnalysis.evolutionDrift.calibratePriorSideways"),
                  },
                  { key: "priorBear", label: t("stockAnalysis.evolutionDrift.calibratePriorBear") },
                ].map(({ key, label }) => (
                  <div key={key} style={{ minWidth: 100 }}>
                    <div style={{ fontSize: 11, color: "var(--muted, #8c8c8c)" }}>{label}</div>
                    <div style={{ fontSize: 16, fontWeight: 600 }}>
                      {calibrateResult.bestParams![key]?.toFixed(2) ?? "—"}
                    </div>
                  </div>
                ))}
              </div>
              <div style={{ marginTop: 8, fontSize: 12, color: "var(--muted, #8c8c8c)" }}>
                {t("stockAnalysis.evolutionDrift.calibrateRefreshCount", { total: calibrateResult.totalReflections })}
                ·{" "}
                {t("stockAnalysis.evolutionDrift.calibrateParsedCount", { parsed: calibrateResult.parsedSuggestions })}
              </div>
            </div>
            <h4>{t("stockAnalysis.evolutionDrift.calibrateGridTitle")}</h4>
            <Table
              size="small"
              rowKey={(_, i) => String(i)}
              pagination={false}
              dataSource={calibrateResult.grid}
              columns={[
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateScore"),
                  dataIndex: "score",
                  key: "score",
                  width: 60,
                  align: "right",
                  render: (v: number) => (
                    <Tag color={v >= 50 ? "green" : v >= 0 ? "orange" : "red"}>{v.toFixed(1)}</Tag>
                  ),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateBuyThreshold"),
                  dataIndex: "buyThreshold",
                  width: 70,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateIncreaseThreshold"),
                  dataIndex: "increaseThreshold",
                  width: 70,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateHoldThreshold"),
                  dataIndex: "holdThreshold",
                  width: 70,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateWatchThreshold"),
                  dataIndex: "watchThreshold",
                  width: 70,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateReduceThreshold"),
                  dataIndex: "reduceThreshold",
                  width: 70,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateCapHigh"),
                  dataIndex: "capHigh",
                  width: 60,
                  align: "right",
                  render: (v: number) => `${v.toFixed(0)}%`,
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibrateCapMid"),
                  dataIndex: "capMid",
                  width: 60,
                  align: "right",
                  render: (v: number) => `${v.toFixed(0)}%`,
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibratePriorBull"),
                  dataIndex: "priorBull",
                  width: 60,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibratePriorSideways"),
                  dataIndex: "priorSideways",
                  width: 60,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
                {
                  title: t("stockAnalysis.evolutionDrift.calibratePriorBear"),
                  dataIndex: "priorBear",
                  width: 60,
                  align: "right",
                  render: (v: number) => v.toFixed(2),
                },
              ]}
            />
          </div>
        )}
        {calibrateResult?.message && !calibrateResult.bestParams && <Empty description={calibrateResult.message} />}
        {(calibrateResult?.structuralExcluded ?? 0) > 0 && (
          <div style={{ marginTop: 12, fontSize: 12, color: "#d46b08" }}>
            {t("stockAnalysis.evolutionDrift.calibrateStructuralCount", {
              count: calibrateResult!.structuralExcluded,
            })}
          </div>
        )}
        {(calibrateResult?.tierUnlabelled ?? 0) > 0 && (
          <div style={{ marginTop: 4, fontSize: 12, color: "var(--muted, #8c8c8c)" }}>
            {t("stockAnalysis.evolutionDrift.calibrateTierUnlabelledCount", {
              count: calibrateResult!.tierUnlabelled,
            })}
          </div>
        )}
      </Modal>

      <div style={{ marginTop: 24 }}>
        <h4>{t("stockAnalysis.evolutionDrift.evolutionHistoryTitle")}</h4>
        {evolutionHistoryLoading
          ? <Spin size="small" />
          : !evolutionHistory || evolutionHistory.length === 0
          ? <Empty description={t("stockAnalysis.evolutionDrift.noEvolutionData")} />
          : (
            <ul style={{ paddingLeft: 16, margin: 0 }}>
              {evolutionHistory.map((h) => {
                const meta = evolutionStatusMeta(h.status);
                const pending = h.status === "pending_approval";
                return (
                  <li key={h.id} style={{ marginBottom: 10, fontSize: 13 }}>
                    <Tag color={meta.color}>{t(meta.textKey)}</Tag>
                    <Tag>{h.evolutionType}</Tag>
                    <Tag>{formatEvolutionTime(h.createdAt)}</Tag>
                    <span style={{ color: "#595959", marginLeft: 4 }}>
                      {parseTrigger(h.trigger, t)}
                    </span>
                    <Button
                      size="small"
                      type={pending ? "primary" : "link"}
                      style={{ marginLeft: 8, padding: pending ? undefined : 0 }}
                      onClick={() => openApproval(h)}
                    >
                      {pending
                        ? t("stockAnalysis.evolutionApproval.reviewAction")
                        : t("stockAnalysis.evolutionApproval.viewAction")}
                    </Button>
                    {h.rejectReason && (
                      <div style={{ color: "#cf1322", marginTop: 2, marginLeft: 4, fontSize: 12 }}>
                        {h.rejectReason}
                      </div>
                    )}
                    {h.improvementSummary && (
                      <div style={{ color: "#8c8c8c", marginTop: 2, marginLeft: 4 }}>
                        {h.improvementSummary}
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
      </div>

      <div style={{ marginTop: 24 }}>
        <h4>{t("stockAnalysis.evolutionDrift.agreementTrendTitle")}</h4>
        {agreementLoading
          ? <Spin size="small" />
          : !agreementHistory || agreementHistory.length === 0
          ? <Empty description={t("stockAnalysis.evolutionDrift.noAgreementData")} />
          : (
            <div>
              <div className="flex items-center gap-2 mb-2">
                <span className="text-xs" style={{ color: "var(--muted)" }}>
                  {t("stockAnalysis.evolutionDrift.agreementRecentAvg")}: {Math.round(
                    agreementHistory.slice(0, 10).reduce((s, r) => s + r.agreementScore, 0)
                      / Math.min(10, agreementHistory.length),
                  )}/100
                </span>
              </div>
              <AgreementSparkline points={agreementHistory} />
            </div>
          )}
      </div>

      {/* 参数变更台账：每次应用换代 + 留痕 + 到期两代命中率评估 */}
      <ParamChangeLedger />

      {/* P4：进化计划审批抽屉（双入口之一：本面板待办卡 + 历史查看） */}
      <EvolutionApprovalDrawer
        open={approvalOpen}
        selected={approvalSelected}
        onSelect={setApprovalSelected}
        onClose={() => setApprovalOpen(false)}
        onChanged={() => void handleApprovalChanged()}
      />
    </React.Fragment>
  );
}

/** 极简 sparkline：避免引入 recharts/chart 包 */
function TimelineSparkline({
  points,
}: {
  points: Array<{ appliedAt: number; newWeight: number; deltaPct: number; trigger: string }>;
}) {
  if (points.length === 0) { return null; }
  const minW = Math.min(...points.map((p) => p.newWeight));
  const maxW = Math.max(...points.map((p) => p.newWeight));
  const range = maxW - minW || 0.01;
  const width = 600;
  const height = 60;
  const xStep = points.length > 1 ? width / (points.length - 1) : width;
  const path = points
    .map((p, i) => {
      const x = i * xStep;
      const y = height - ((p.newWeight - minW) / range) * (height - 10) - 5;
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg width={width} height={height} style={{ display: "block" }}>
      <line x1={0} y1={height - 5} x2={width} y2={height - 5} stroke="#d9d9d9" strokeWidth={1} />
      <path d={path} fill="none" stroke="#1677ff" strokeWidth={2} />
      {points.map((p, i) => {
        const x = i * xStep;
        const y = height - ((p.newWeight - minW) / range) * (height - 10) - 5;
        return <circle key={i} cx={x} cy={y} r={3} fill="#1677ff" />;
      })}
    </svg>
  );
}

/** Phase 3: 一致性分数趋势 sparkline（紫色调） */
function AgreementSparkline({
  points,
}: {
  points: Array<
    {
      exitAt: number;
      agreementScore: number;
      stockCode: string;
      stockName: string;
      returnPct: number;
      wasCorrect: number;
    }
  >;
}) {
  const { t } = useTranslation();
  if (points.length === 0) { return null; }
  const minS = Math.min(...points.map((p) => p.agreementScore));
  const maxS = Math.max(...points.map((p) => p.agreementScore));
  const range = maxS - minS || 0.01;
  const width = 600;
  const height = 60;
  const xStep = points.length > 1 ? width / (points.length - 1) : width;
  const path = points
    .map((p, i) => {
      const x = i * xStep;
      const y = height - ((p.agreementScore - minS) / range) * (height - 10) - 5;
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <div>
      <svg width={width} height={height} style={{ display: "block" }}>
        <line x1={0} y1={height - 5} x2={width} y2={height - 5} stroke="#d9d9d9" strokeWidth={1} />
        <path d={path} fill="none" stroke="#7c3aed" strokeWidth={2} />
        {points.map((p, i) => {
          const x = i * xStep;
          const y = height - ((p.agreementScore - minS) / range) * (height - 10) - 5;
          // 2026-09-21: 走单一真相源（原内联 60/40 档位表）
          const color = agreementColor(p.agreementScore);
          return <circle key={i} cx={x} cy={y} r={3} fill={color} />;
        })}
      </svg>
      <div className="flex items-center gap-2 mt-1">
        <span className="text-[10px]" style={{ color: "#7c3aed" }}>
          {t("stockAnalysis.evolutionDrift.agreementTrendLabel")}
        </span>
        <span className="text-[10px]" style={{ color: "var(--muted)" }}>
          {t("stockAnalysis.evolutionDrift.agreementRange")}: {minS}-{maxS}
        </span>
      </div>
    </div>
  );
}

/** 一条参数变更台账（后端 `workflow_param_changes` 的 camelCase 投影）。 */
type ParamChangeRow = {
  id: string;
  param: string;
  fromVersion: number;
  toVersion: number;
  oldValueJson: string;
  newValueJson: string;
  reflectionId: string | null;
  reason: string | null;
  status: string;
  reviewAfterMs: number;
  baselineRate: number | null;
  baselineSamples: number | null;
  afterRate: number | null;
  afterSamples: number | null;
  verdictNote: string | null;
  rollbackVersion: number | null;
  appliedAtMs: number;
};

/**
 * 参数变更台账：应用即换代（旧版本先快照），到期后只比较**变更前/后两代**样本。
 * 「样本不足」在这里只表示评估窗口内样本不够（两侧各需 10 条），不是对反思本身的判定。
 */
function ParamChangeLedger() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const [rows, setRows] = useState<ParamChangeRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [evaluating, setEvaluating] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await invoke<ParamChangeRow[]>("list_param_changes", { limit: 30 });
      setRows(Array.isArray(r) ? r : []);
    } catch {
      setRows([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const evaluate = useCallback(async () => {
    setEvaluating(true);
    try {
      const r = await invoke<{ evaluated: number }>("evaluate_param_changes");
      message.success(
        t("stockAnalysis.evolutionDrift.paramChangesEvaluated", { count: r.evaluated }),
      );
      await load();
    } catch (e) {
      message.error(
        t("stockAnalysis.evolutionDrift.paramChangesFailed", { msg: String(e) }),
      );
    } finally {
      setEvaluating(false);
    }
  }, [load, message, t]);

  const rollback = useCallback(
    async (id: string) => {
      try {
        await invoke("rollback_param_change", { changeId: id });
        message.success(t("stockAnalysis.evolutionDrift.paramChangesRolledBack"));
        await load();
      } catch (e) {
        message.error(
          t("stockAnalysis.evolutionDrift.paramChangesFailed", { msg: String(e) }),
        );
      }
    },
    [load, message, t],
  );

  const statusMeta = (status: string): { color: string; label: string } => {
    switch (status) {
      case "pending":
        return { color: "default", label: t("stockAnalysis.evolutionDrift.paramChangesStatus.pending") };
      case "insufficient":
        return {
          color: "gold",
          label: t("stockAnalysis.evolutionDrift.paramChangesStatus.insufficient"),
        };
      case "kept":
        return { color: "green", label: t("stockAnalysis.evolutionDrift.paramChangesStatus.kept") };
      case "rollback_suggested":
        return {
          color: "orange",
          label: t("stockAnalysis.evolutionDrift.paramChangesStatus.rollbackSuggested"),
        };
      case "rolled_back":
        return {
          color: "red",
          label: t("stockAnalysis.evolutionDrift.paramChangesStatus.rolledBack"),
        };
      default:
        return { color: "default", label: status };
    }
  };

  const rateText = (correct: number | null, samples: number | null): string =>
    correct == null || samples == null || samples === 0
      ? "—"
      : `${(correct * 100).toFixed(1)}% (n=${samples})`;

  return (
    <div style={{ marginTop: 24 }} data-testid="param-change-ledger">
      <Space style={{ marginBottom: 4 }}>
        <h4 style={{ margin: 0 }}>{t("stockAnalysis.evolutionDrift.paramChangesTitle")}</h4>
        <Button size="small" icon={<ReloadOutlined />} loading={evaluating} onClick={() => void evaluate()}>
          {t("stockAnalysis.evolutionDrift.paramChangesEvaluate")}
        </Button>
      </Space>
      <div style={{ fontSize: 12, color: "var(--muted, #8c8c8c)", margin: "4px 0 8px" }}>
        {t("stockAnalysis.evolutionDrift.paramChangesHint")}
      </div>
      {loading
        ? <Spin size="small" />
        : rows.length === 0
        ? <Empty description={t("stockAnalysis.evolutionDrift.paramChangesEmpty")} />
        : (
          <ul style={{ paddingLeft: 16, margin: 0 }}>
            {rows.map((r) => {
              const meta = statusMeta(r.status);
              return (
                <li key={r.id} style={{ marginBottom: 10, fontSize: 12 }}>
                  <Tag color={meta.color}>{meta.label}</Tag>
                  <strong>{r.param}</strong>
                  <span style={{ marginLeft: 6, color: "var(--muted, #8c8c8c)" }}>
                    v{r.fromVersion}→v{r.toVersion}
                  </span>
                  <span style={{ marginLeft: 8 }}>
                    {r.oldValueJson} → {r.newValueJson}
                  </span>
                  <div style={{ color: "var(--muted, #8c8c8c)", marginTop: 2 }}>
                    {t("stockAnalysis.evolutionDrift.paramChangesRates", {
                      before: rateText(r.baselineRate, r.baselineSamples),
                      after: rateText(r.afterRate, r.afterSamples),
                    })}
                    {r.verdictNote ? ` · ${r.verdictNote}` : ""}
                  </div>
                  {r.status !== "rolled_back" && (
                    <Button size="small" type="link" onClick={() => void rollback(r.id)}>
                      {t("stockAnalysis.evolutionDrift.paramChangesRollback")}
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
    </div>
  );
}
