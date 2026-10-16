import { JobCard } from "@/components/tasks/JobCard";
import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { horizonLabelKey } from "@/lib/stock-analysis-utils";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { TimeTravelBatchAnalysis, TimeTravelBatchState, TimeTravelPeriod } from "@/types";
import {
  Alert,
  Button,
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
import { BatchStatusTag } from "./BatchStatusTag";
import { useBatchAnalysisColumns, useBatchHorizonColumns } from "./batchTables";

const { Text } = Typography;

const ALL_PERIODS: TimeTravelPeriod[] = ["ultra_short", "short", "mid", "long"];
const QUICK_AGO = [5, 20, 60, 90];
const POLL_INTERVAL_MS = 2000;
const TERMINAL: ReadonlyArray<TimeTravelBatchState["status"]> = [
  "completed",
  "partial",
  "failed",
  "interrupted",
];

type DateMode = "ago" | "fixed";

/**
 * 时间旅行批次复盘卡片：选定历史交易日 → as-of 封闭世界一轮统一回放
 * 「智能荐股 + 趋势智选」两条平级选股链 → 两链候选合并去重后逐只 replay 分析
 * → 退出 guard 后按 2/5/28/90 交易日四档反思（未到期留 pending 由 18:00 cron 补做）。
 *
 * 挂载位置（同一组件，两处渲染，无第二套实现）：
 *   1. 投资中心标题栏「批次复盘」按钮打开的右侧 Drawer —— 主入口，live/as-of
 *      两种世界观下都可发起，与模式切换同属投资中心顶层模式层（业务 tab 不托管）；
 *   2. 时间旅行回放工作台（/replay-workbench）历史验证区，与 ReplaySweep 并列。
 * 批次复盘是历史验证逻辑，不属于选股页的当日生产。卡片自带历史交易日选择
 * （N 交易日前 / 指定日期），后端自开 as_of guard，与全局锚点正交。
 */
export function TimeTravelBatchCard() {
  const { t } = useTranslation();

  // 工作台选了锚点日时默认 fixed 模式带入该日；未选锚点（asOfDate=null）时
  // 默认「N 个交易日前」模式。仅初始化读一次（getState，不订阅）：
  // 用户在卡片内改日期后不应被外部 store 变化覆盖。
  const initialAnchor = useTimeAnchorStore.getState().asOfDate;
  const [dateMode, setDateMode] = useState<DateMode>(initialAnchor ? "fixed" : "ago");
  const [tradingDaysAgo, setTradingDaysAgo] = useState<number>(90);
  const [fixedDate, setFixedDate] = useState<Dayjs | null>(
    initialAnchor ? dayjs(initialAnchor) : null,
  );
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

  const analysisColumns = useBatchAnalysisColumns();

  return (
    <JobCard
      testId="ttb-card"
      icon={<FlaskConical size={16} />}
      title={t("replayWorkbench.batch.title")}
      description={t("replayWorkbench.batch.description")}
    >
      <Space orientation="vertical" size={12} style={{ width: "100%" }}>
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
              label: t(horizonLabelKey(p) ?? p),
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
    </JobCard>
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
  const horizonColumns = useBatchHorizonColumns();

  return (
    <Space orientation="vertical" size={12} style={{ width: "100%" }} data-testid="ttb-progress">
      <Space wrap>
        <BatchStatusTag status={batch.status} />
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
        <Tag color="geekblue">{t("replayWorkbench.batch.recoCount", { n: batch.recoPicks ?? 0 })}</Tag>
        <Tag color="purple">{t("replayWorkbench.batch.serenityCount", { n: batch.serenityPicks ?? 0 })}</Tag>
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
          columns={horizonColumns}
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
