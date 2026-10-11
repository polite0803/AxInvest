import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { TimeTravelBatchAnalysis, TimeTravelBatchState, TimeTravelPeriod } from "@/types";
import {
  Alert,
  Button,
  Card,
  Checkbox,
  Collapse,
  DatePicker,
  InputNumber,
  Progress,
  Segmented,
  Space,
  Switch,
  Table,
  Tag,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import dayjs, { type Dayjs } from "dayjs";
import { FlaskConical, Play } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

const ALL_PERIODS: TimeTravelPeriod[] = ["ultra_short", "short", "mid", "long"];
const QUICK_AGO = [5, 20, 60, 90];
const POLL_INTERVAL_MS = 2000;
const TERMINAL: ReadonlyArray<TimeTravelBatchState["status"]> = ["completed", "partial", "failed"];

type DateMode = "ago" | "fixed";

/**
 * 时间旅行批次复盘卡片：选定历史交易日 → as-of 封闭世界内存荐股 + 逐只 replay 分析
 * → 退出 guard 后按 2/5/28/90 交易日四档反思（未到期留 pending 由 18:00 cron 补做）。
 */
export function TimeTravelBatchCard() {
  const { t } = useTranslation();

  const [dateMode, setDateMode] = useState<DateMode>("ago");
  const [tradingDaysAgo, setTradingDaysAgo] = useState<number>(90);
  const [fixedDate, setFixedDate] = useState<Dayjs | null>(null);
  const [periods, setPeriods] = useState<TimeTravelPeriod[]>(ALL_PERIODS);
  const [minConfidence, setMinConfidence] = useState<number>(50);
  const [topN, setTopN] = useState<number>(20);
  const [reflect, setReflect] = useState<boolean>(true);

  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [batch, setBatch] = useState<TimeTravelBatchState | null>(null);
  const pollTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    return () => {
      if (pollTimer.current) {
        clearInterval(pollTimer.current);
      }
    };
  }, []);

  const stopPolling = () => {
    if (pollTimer.current) {
      clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  };

  const startPolling = (batchId: string) => {
    stopPolling();
    const tick = async () => {
      try {
        const next = await invoke<TimeTravelBatchState | null>("get_time_travel_batch", {
          batchId,
        });
        if (next) {
          setBatch(next);
          if (TERMINAL.includes(next.status)) {
            stopPolling();
          }
        }
      } catch (e) {
        // 轮询失败只留痕，不打断批次（后台仍在跑）
        console.warn("[time-travel-batch] poll failed", e);
      }
    };
    void tick();
    pollTimer.current = setInterval(() => void tick(), POLL_INTERVAL_MS);
  };

  const handleStart = async () => {
    setStartError(null);
    if (periods.length === 0) {
      setStartError(t("replayWorkbench.batch.needPeriod"));
      return;
    }
    if (dateMode === "ago" && (!tradingDaysAgo || tradingDaysAgo <= 0)) {
      setStartError(t("replayWorkbench.batch.needAgo"));
      return;
    }
    if (dateMode === "fixed" && !fixedDate) {
      setStartError(t("replayWorkbench.batch.needDate"));
      return;
    }

    const payload = {
      date: dateMode === "fixed" ? fixedDate?.format("YYYY-MM-DD") ?? null : null,
      tradingDaysAgo: dateMode === "ago" ? tradingDaysAgo : null,
      periods,
      minConfidence,
      topN,
      runReco: true,
      runSerenity: false,
      reflect,
    };

    setStarting(true);
    try {
      const batchId = await invoke<string>("run_time_travel_batch", { request: payload });
      setBatch(null);
      startPolling(batchId);
    } catch (e) {
      setStartError(translateBackendError(e));
    } finally {
      setStarting(false);
    }
  };

  const running = batch !== null && !TERMINAL.includes(batch.status);
  const finishedCount = batch ? batch.ok + batch.fail : 0;
  const percent = batch && batch.total > 0 ? Math.round((finishedCount / batch.total) * 100) : 0;

  const analysisColumns: ColumnsType<TimeTravelBatchAnalysis> = [
    {
      title: t("replayWorkbench.batch.col.stock"),
      dataIndex: "stockCode",
      key: "stock",
      render: (_, r) => (
        <span>
          <Text strong>{r.stockCode}</Text> <Text type="secondary">{r.stockName}</Text>
        </span>
      ),
    },
    {
      title: t("replayWorkbench.batch.col.period"),
      dataIndex: "period",
      key: "period",
      width: 96,
      render: (p: string) => <Tag>{t(`replayWorkbench.batch.period.${p}`, { defaultValue: p })}</Tag>,
    },
    {
      title: t("replayWorkbench.batch.col.confidence"),
      dataIndex: "confidence",
      key: "confidence",
      width: 90,
    },
    {
      title: t("replayWorkbench.batch.col.status"),
      key: "status",
      width: 110,
      render: (_, r) =>
        r.analysisId
          ? <Tag color="success">{t("replayWorkbench.batch.analysisOk")}</Tag>
          : <Tag color="error">{t("replayWorkbench.batch.analysisFail")}</Tag>,
    },
    {
      title: t("replayWorkbench.batch.col.detail"),
      key: "detail",
      ellipsis: true,
      render: (_, r) =>
        r.error
          ? <Text type="danger">{r.error}</Text>
          : <Text type="secondary">{r.analysisId?.slice(0, 8)}</Text>,
    },
  ];

  return (
    <Card
      size="small"
      style={{ marginBottom: 16 }}
      title={
        <span>
          <span style={{ marginRight: 8 }}>3️⃣</span>
          {t("replayWorkbench.batch.title")}
        </span>
      }
      extra={<FlaskConical size={14} />}
    >
      <Space direction="vertical" size={12} style={{ width: "100%" }}>
        <Text type="secondary">{t("replayWorkbench.batch.description")}</Text>

        <Space wrap size="middle">
          <Segmented
            value={dateMode}
            onChange={(v) => setDateMode(v as DateMode)}
            options={[
              { label: t("replayWorkbench.batch.modeAgo"), value: "ago" },
              { label: t("replayWorkbench.batch.modeFixed"), value: "fixed" },
            ]}
          />
          {dateMode === "ago"
            ? (
              <Space>
                <InputNumber
                  min={1}
                  max={500}
                  value={tradingDaysAgo}
                  onChange={(v) => setTradingDaysAgo(Number(v) || 0)}
                  addonAfter={t("replayWorkbench.batch.tradingDays")}
                  data-testid="ttb-days-ago"
                />
                {QUICK_AGO.map((n) => (
                  <Button
                    key={n}
                    size="small"
                    type={tradingDaysAgo === n ? "primary" : "default"}
                    onClick={() => setTradingDaysAgo(n)}
                  >
                    {n}
                  </Button>
                ))}
              </Space>
            )
            : (
              <DatePicker
                value={fixedDate}
                onChange={(d) => setFixedDate(d)}
                maxDate={dayjs()}
                allowClear={false}
                data-testid="ttb-fixed-date"
              />
            )}
        </Space>

        <Space wrap>
          <span>{t("replayWorkbench.batch.periods")}:</span>
          <Checkbox.Group
            value={periods}
            onChange={(v) => setPeriods(v as TimeTravelPeriod[])}
            options={ALL_PERIODS.map((p) => ({
              label: t(`replayWorkbench.batch.period.${p}`),
              value: p,
            }))}
          />
        </Space>

        <Collapse
          ghost
          items={[
            {
              key: "advanced",
              label: t("replayWorkbench.batch.advanced"),
              children: (
                <Space wrap size="large">
                  <span>
                    {t("replayWorkbench.batch.minConfidence")}:{" "}
                    <InputNumber
                      min={0}
                      max={100}
                      value={minConfidence}
                      onChange={(v) => setMinConfidence(Number(v) || 0)}
                    />
                  </span>
                  <span>
                    {t("replayWorkbench.batch.topN")}:{" "}
                    <InputNumber
                      min={0}
                      max={200}
                      value={topN}
                      onChange={(v) => setTopN(Number(v) || 0)}
                    />
                  </span>
                  <span>
                    {t("replayWorkbench.batch.reflectNow")}: <Switch checked={reflect} onChange={setReflect} />
                  </span>
                </Space>
              ),
            },
          ]}
        />

        {startError && (
          <Alert
            type="error"
            showIcon
            message={startError}
            data-testid="ttb-start-error"
          />
        )}

        <div>
          <Button
            type="primary"
            icon={<Play size={12} />}
            loading={starting || running}
            onClick={handleStart}
            data-testid="ttb-start-btn"
          >
            {running
              ? t("replayWorkbench.batch.running")
              : t("replayWorkbench.batch.start")}
          </Button>
        </div>

        {batch && <BatchProgress batch={batch} percent={percent} analysisColumns={analysisColumns} />}
      </Space>
    </Card>
  );
}

function BatchProgress({
  batch,
  percent,
  analysisColumns,
}: {
  batch: TimeTravelBatchState;
  percent: number;
  analysisColumns: ColumnsType<TimeTravelBatchAnalysis>;
}) {
  const { t } = useTranslation();

  const statusTag = () => {
    switch (batch.status) {
      case "completed":
        return <Tag color="success">{t("replayWorkbench.batch.status.completed")}</Tag>;
      case "partial":
        return <Tag color="warning">{t("replayWorkbench.batch.status.partial")}</Tag>;
      case "failed":
        return <Tag color="error">{t("replayWorkbench.batch.status.failed")}</Tag>;
      default:
        return <Tag color="processing">{t("replayWorkbench.batch.status.running")}</Tag>;
    }
  };

  return (
    <Space direction="vertical" size={12} style={{ width: "100%" }} data-testid="ttb-progress">
      <Space wrap>
        {statusTag()}
        <Tag>
          {batch.phase === "reflection"
            ? t("replayWorkbench.batch.phaseReflection")
            : t("replayWorkbench.batch.phaseAnalysis")}
        </Tag>
        <Text type="secondary">
          {t("replayWorkbench.batch.asOfLabel")}: {batch.asOf}
        </Text>
      </Space>

      {batch.phase === "analysis" && (
        <Progress percent={percent} size="small" status={batch.status === "failed" ? "exception" : "active"} />
      )}

      <Space wrap>
        <Tag color="success">{t("replayWorkbench.batch.okCount", { n: batch.ok })}</Tag>
        <Tag color="error">{t("replayWorkbench.batch.failCount", { n: batch.fail })}</Tag>
        <Tag>{t("replayWorkbench.batch.totalCount", { n: batch.total })}</Tag>
      </Space>

      {batch.message && <Alert type="info" showIcon message={batch.message} />}
      {batch.errorDetail && <Alert type="warning" showIcon message={batch.errorDetail} />}

      {batch.horizons.length > 0 && (
        <Table
          size="small"
          pagination={false}
          rowKey="period"
          dataSource={batch.horizons}
          columns={[
            {
              title: t("replayWorkbench.batch.col.period"),
              dataIndex: "period",
              key: "period",
              render: (p: string) => <Tag>{t(`replayWorkbench.batch.period.${p}`, { defaultValue: p })}</Tag>,
            },
            {
              title: t("replayWorkbench.batch.col.holdingDays"),
              dataIndex: "holdingDays",
              key: "holdingDays",
              width: 80,
            },
            {
              title: t("replayWorkbench.batch.col.hindsight"),
              dataIndex: "hindsightDate",
              key: "hindsightDate",
              width: 120,
            },
            {
              title: t("replayWorkbench.batch.col.due"),
              dataIndex: "due",
              key: "due",
              width: 90,
              render: (due: boolean) =>
                due
                  ? <Tag color="blue">{t("replayWorkbench.batch.dueYes")}</Tag>
                  : <Tag>{t("replayWorkbench.batch.dueNo")}</Tag>,
            },
            {
              title: t("replayWorkbench.batch.col.reflection"),
              key: "reflection",
              render: (_, h) => (
                <Space size={4}>
                  <Tag color="success">{h.completed}</Tag>
                  <Tag>{h.pending}</Tag>
                  {h.failed > 0 && <Tag color="error">{h.failed}</Tag>}
                </Space>
              ),
            },
          ]}
        />
      )}

      {batch.analyses.length > 0 && (
        <Collapse
          ghost
          items={[
            {
              key: "analyses",
              label: t("replayWorkbench.batch.analysesDetail", { n: batch.analyses.length }),
              children: (
                <Table<TimeTravelBatchAnalysis>
                  size="small"
                  pagination={false}
                  rowKey={(r) => `${r.stockCode}-${r.period}`}
                  columns={analysisColumns}
                  dataSource={batch.analyses}
                  scroll={{ y: 320 }}
                />
              ),
            },
          ]}
        />
      )}
    </Space>
  );
}
