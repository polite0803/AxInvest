import { ReloadOutlined } from "@ant-design/icons";
import { Alert, App, Button, Card, Empty, Space, Spin, Table, Tag, Tooltip } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { invoke } from "@/lib/invoke";
import { horizonSuffix } from "@/lib/stock-analysis-utils";
import type { RecoIcStats, RecoLoopCell, RecoLoopView } from "@/types";

/**
 * 荐股反思闭环面板（2026-10-20）。
 *
 * 荐股链的反思形态是**数值回写逐格权重**，不产出叙事 —— 之前唯一的可见位置是
 * 「回测 → 策略」矩阵底部一行闸状态，智能荐股页完全看不到，也没有手动重算入口。
 * 本面板把 `reco_ic_stats.loop` 的逐格结果和 `recalc_reco_loop_weights` 接到这里：
 * - 状态列区分「已校准 / 负 IC 降权 / 样本不足 / IC 测不出」，未校准不得看起来像校准过；
 * - 样本数列让「n=3 也算了一格」与「n=40 只是 IC 不可测」可区分；
 * - `lastRecalcAt` 是**最近计算时刻**（后端心跳），不是最近写库时刻。
 */
export function RecoLoopPanel({ asOfDate }: { asOfDate?: string | null }) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [loop, setLoop] = useState<RecoLoopView | null>(null);
  const [loading, setLoading] = useState(true);
  const [recalculating, setRecalculating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const stats = await invoke<RecoIcStats>("reco_ic_stats", {
        generation: asOfDate ?? null,
      });
      setLoop(stats.loop ?? null);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setLoop(null);
    } finally {
      setLoading(false);
    }
  }, [asOfDate]);

  useEffect(() => {
    void load();
  }, [load]);

  const recalc = useCallback(async () => {
    setRecalculating(true);
    try {
      const result = await invoke<{ written: number }>("recalc_reco_loop_weights", {
        asOfDate: asOfDate ?? null,
      });
      await load();
      message.success(t("stockAnalysis.recoLoop.recalcSuccess", { written: result.written }));
    } catch (e) {
      message.error(
        t("stockAnalysis.recoLoop.recalcFailed", {
          error: e instanceof Error ? e.message : String(e),
        }),
      );
    } finally {
      setRecalculating(false);
    }
  }, [asOfDate, load, message, t]);

  const activeCells = useMemo(
    () => (loop?.cells ?? []).filter((c) => c.status !== "not_in_matrix"),
    [loop],
  );

  const summary = useMemo(() => {
    const calibrated = activeCells.filter((c) =>
      c.status === "ic_non_negative" || c.status === "demoted_negative_ic"
    ).length;
    const demoted = activeCells.filter((c) => c.status === "demoted_negative_ic").length;
    return { calibrated, demoted, uncalibrated: activeCells.length - calibrated };
  }, [activeCells]);

  const statusTag = (cell: RecoLoopCell) => {
    switch (cell.status) {
      case "demoted_negative_ic":
        return (
          <Tooltip
            title={t("stockAnalysis.recoLoop.demotedTip", {
              ic: cell.rankIc != null ? cell.rankIc.toFixed(3) : "—",
            })}
          >
            <Tag color="volcano" data-testid="reco-loop-status-demoted">
              {t("stockAnalysis.recoLoop.statusDemoted")}
            </Tag>
          </Tooltip>
        );
      case "ic_non_negative":
        return (
          <Tag color="green" data-testid="reco-loop-status-calibrated">
            {t("stockAnalysis.recoLoop.statusCalibrated")}
          </Tag>
        );
      case "insufficient_samples":
        return (
          <Tag data-testid="reco-loop-status-insufficient">
            {t("stockAnalysis.recoLoop.statusInsufficient")}
          </Tag>
        );
      case "ic_unmeasurable":
        return (
          <Tooltip title={t("stockAnalysis.recoLoop.icUnmeasurableTip")}>
            <Tag color="gold" data-testid="reco-loop-status-ic">
              {t("stockAnalysis.recoLoop.statusIcUnmeasurable")}
            </Tag>
          </Tooltip>
        );
      default:
        return <Tag>{cell.status}</Tag>;
    }
  };

  const columns: ColumnsType<RecoLoopCell> = [
    {
      title: t("stockAnalysis.backtest.colStyle"),
      dataIndex: "style",
      key: "style",
      width: 92,
      render: (style: string) =>
        t(`stockAnalysis.recommendation.style${style.charAt(0).toUpperCase() + style.slice(1)}`),
    },
    {
      title: t("stockAnalysis.recoLoop.colPeriod"),
      dataIndex: "period",
      key: "period",
      width: 78,
      render: (period: string) => {
        const suffix = horizonSuffix(period);
        return suffix ? t(`stockAnalysis.timeHorizon${suffix}`) : period;
      },
    },
    {
      title: t("stockAnalysis.recoLoop.colStatus"),
      key: "status",
      width: 132,
      render: (_, cell) => statusTag(cell),
    },
    {
      title: t("stockAnalysis.recoLoop.colSamples"),
      dataIndex: "samples",
      key: "samples",
      width: 64,
      align: "right",
      render: (n: number) => <span data-testid="reco-loop-samples">{n}</span>,
    },
    {
      title: t("stockAnalysis.backtest.colWinRate"),
      dataIndex: "winRate",
      key: "winRate",
      width: 70,
      align: "right",
      render: (v: number | null) => (v == null ? "—" : `${(v * 100).toFixed(0)}%`),
    },
    {
      title: t("stockAnalysis.recoLoop.colRankIc"),
      dataIndex: "rankIc",
      key: "rankIc",
      width: 78,
      align: "right",
      render: (v: number | null) => (v == null ? "—" : v.toFixed(3)),
    },
    {
      title: t("stockAnalysis.recoLoop.colWeight"),
      key: "weight",
      width: 106,
      align: "right",
      render: (_, cell) => (
        <span>
          {cell.oldWeight.toFixed(2)}
          {" → "}
          <strong>{cell.newWeight.toFixed(2)}</strong>
        </span>
      ),
    },
    {
      title: t("stockAnalysis.recoLoop.colOuter"),
      key: "outer",
      width: 132,
      render: (_, cell) => {
        const parts: string[] = [];
        if (cell.moverPenalty != null) {
          parts.push(t("stockAnalysis.recoLoop.moverPenalty", { p: cell.moverPenalty.toFixed(2) }));
        }
        if (cell.dqiPenalty != null) {
          parts.push(t("stockAnalysis.recoLoop.dqiPenalty", { p: cell.dqiPenalty.toFixed(2) }));
        }
        return parts.length === 0
          ? <span style={{ color: "var(--muted)" }}>—</span>
          : <span style={{ fontSize: 11 }}>{parts.join(" · ")}</span>;
      },
    },
  ];

  const gateText = loop
    ? t(
      loop.gate === "on"
        ? "stockAnalysis.backtest.loopGateOn"
        : loop.gate === "shadow"
        ? "stockAnalysis.backtest.loopGateShadow"
        : "stockAnalysis.backtest.loopGateOff",
      {
        time: loop.lastRecalcAt > 0
          ? new Date(loop.lastRecalcAt).toLocaleString()
          : t("stockAnalysis.backtest.loopNeverRecalc"),
      },
    )
    : "";

  return (
    <Card
      size="small"
      title={t("stockAnalysis.recoLoop.title")}
      data-testid="reco-loop-panel"
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Space size={6}>
          {loop && (
            <Tag color={loop.gate === "on" ? "green" : loop.gate === "shadow" ? "gold" : "default"}>
              {loop.gate}
            </Tag>
          )}
          <Button
            size="small"
            icon={<ReloadOutlined />}
            loading={recalculating}
            disabled={!!asOfDate}
            title={asOfDate ? t("stockAnalysis.recoLoop.recalcReplayDisabled") : undefined}
            onClick={() => void recalc()}
            data-testid="reco-loop-recalc"
          >
            {t("stockAnalysis.recoLoop.recalcNow")}
          </Button>
        </Space>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : error
        ? (
          <Alert
            type="error"
            showIcon
            className="text-xs!"
            title={t("stockAnalysis.recoLoop.loadFailed", { error })}
          />
        )
        : !loop
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.recoLoop.empty")} />
        : (
          <>
            <div style={{ fontSize: 11, color: "var(--muted)", marginBottom: 6 }}>
              {gateText}
              <span style={{ marginLeft: 8 }}>
                {t("stockAnalysis.recoLoop.summary", summary)}
              </span>
            </div>
            <Alert
              type="info"
              showIcon
              className="text-xs! mb-2!"
              data-testid="reco-loop-scope-hint"
              title={<span className="text-xs">{t("stockAnalysis.recoLoop.scopeHint")}</span>}
            />
            {asOfDate && (
              <Alert
                type="warning"
                showIcon
                className="text-xs! mb-2!"
                data-testid="reco-loop-replay-note"
                title={<span className="text-xs">{t("stockAnalysis.recoLoop.replayScopeNote")}</span>}
              />
            )}
            {activeCells.length === 0
              ? (
                <Empty
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                  description={t("stockAnalysis.recoLoop.empty")}
                />
              )
              : (
                <Table
                  size="small"
                  rowKey={(r) => `${r.style}_${r.period}`}
                  columns={columns}
                  dataSource={activeCells}
                  pagination={false}
                  scroll={{ y: 240 }}
                />
              )}
          </>
        )}
    </Card>
  );
}
