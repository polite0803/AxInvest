import { useTrackedTask } from "@/hooks/useTrackedTask";
import { invoke } from "@/lib/invoke";
import type { TaskAccepted } from "@/types";
import type {
  CloseSweepReport,
  HistoryBackfillReport,
  MarketCoverage,
  UniverseScanStats,
} from "@/types/stock-analysis";
import { Button, Card, message, Space, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 阶段 0 的三个采集动作（全量段扫 → 补最新收盘 → 补窗口历史）+ 四个计数域。
 *
 * 从「妖股漏检核查」面板里抽出来单独成一卡，挂在回测验证 → 策略页签顶部：
 * 原先它埋在两层面板之下（矩阵卡尾部 → 核查卡中部），实测用户找不到。
 *
 * 状态归本组件独有、全站只挂一份。不要在原面板里再渲染一份副本 —— 两个实例
 * 会各自起两条 `useTrackedTask` 轨，互相覆盖同一份终态报告。
 */
export function UniverseSamplePanel() {
  const { t } = useTranslation();
  /** 覆盖读数：`null` = 取不到（不是 0）。 */
  const [coverage, setCoverage] = useState<MarketCoverage | null>(null);
  const [sampleAction, setSampleAction] = useState<"scan" | "sweep" | "history" | null>(null);

  // 慢命令（收盘采集 / 历史回补）已改为异步任务：受理即返回，进度看任务中心，
  // 报告在终态后从 result_json 取回。两个动作各一条任务轨，互不覆盖。
  const {
    start: startSweep,
    cancel: cancelSweep,
    running: sweepRunning,
    result: sweepResult,
    error: sweepError,
    cancelled: sweepCancelled,
  } = useTrackedTask<CloseSweepReport>();
  const {
    start: startHistory,
    cancel: cancelHistory,
    running: historyRunning,
    result: historyResult,
    error: historyError,
    cancelled: historyCancelled,
  } = useTrackedTask<HistoryBackfillReport>();

  const loadCoverage = useCallback(async () => {
    try {
      const c = await invoke<unknown>("get_market_coverage", {});
      // 形状守卫：mock 兜底/契约漂移时不能让「拿不到」被读成 0 只，也不能让缺 closeDates 崩渲染
      setCoverage(isCoverage(c) ? c : null);
    } catch {
      setCoverage(null);
    }
  }, []);

  useEffect(() => {
    void loadCoverage();
  }, [loadCoverage]);

  const runSampleAction = async (
    kind: "scan" | "sweep" | "history",
    run: () => Promise<string>,
  ) => {
    setSampleAction(kind);
    try {
      message.success(await run());
      void loadCoverage();
    } catch (e) {
      message.error(detailOf(e));
    } finally {
      setSampleAction(null);
    }
  };

  const fullMarketScan = () =>
    runSampleAction("scan", async () => {
      const r = await invoke<UniverseScanStats>("run_universe_code_scan", { shards: 1 });
      assertReport(r, "run_universe_code_scan", "registered");
      return t("stockAnalysis.moverRecall.fullScanDone", {
        registered: r?.registered ?? 0,
        codes: r?.codesTried ?? 0,
      });
    });

  // 受理即返回：运行态由「任务运行中」标签 + 取消按钮表达（不额外弹 toast，
  // 否则快速完成时「已提交」会排在终态提示之后，反而误导）
  const sweepLatestClose = async () => {
    setSampleAction("sweep");
    try {
      await startSweep(() => invoke<TaskAccepted>("sweep_market_close", {}));
    } finally {
      setSampleAction(null);
    }
  };

  const backfillHistory = async () => {
    setSampleAction("history");
    try {
      await startHistory(() => invoke<TaskAccepted>("backfill_market_close_history", { limit: 500 }));
    } finally {
      setSampleAction(null);
    }
  };

  // 任务终态：断言与提示时点后移到「报告取回」处（判据不减，原先在 await 之后做）
  useEffect(() => {
    if (!sweepResult) {
      return;
    }
    try {
      assertReport(sweepResult, "sweep_market_close", "written");
    } catch (e) {
      message.error(detailOf(e));
      return;
    }
    if (sweepCancelled) {
      message.warning(t("stockAnalysis.moverRecall.taskCancelled"));
    } else {
      message.success(t("stockAnalysis.moverRecall.sweepDone", {
        written: sweepResult.written ?? 0,
        total: sweepResult.universeSize ?? 0,
        missing: sweepResult.missingQuotes ?? 0,
      }));
    }
    void loadCoverage();
  }, [sweepResult, sweepCancelled, loadCoverage, t]);

  useEffect(() => {
    if (sweepError) {
      message.error(sweepError);
    }
  }, [sweepError]);

  useEffect(() => {
    if (!historyResult) {
      return;
    }
    try {
      assertReport(historyResult, "backfill_market_close_history", "requested");
    } catch (e) {
      message.error(detailOf(e));
      return;
    }
    // 已写入的行无论取消与否都有效 ⇒ 覆盖读数一律重取
    void loadCoverage();
    if (historyCancelled) {
      message.warning(t("stockAnalysis.moverRecall.taskCancelled"));
      return;
    }
    // 全失败不能伪装成「补齐」：把首条原因带回给用户（500/0/500 那次就是被这里吞掉的）
    if ((historyResult.failed ?? 0) > 0) {
      message.error(historyResult.error ?? `backfill failed: ${historyResult.failed ?? 0}`);
      return;
    }
    message.success(
      (historyResult.requested ?? 0) === 0
        ? t("stockAnalysis.moverRecall.backfillNone")
        : t("stockAnalysis.moverRecall.backfillDone", {
          codes: historyResult.codes ?? 0,
          rows: historyResult.rows ?? 0,
          noNew: historyResult.noNew ?? 0,
          noData: historyResult.noData ?? 0,
          failed: historyResult.failed ?? 0,
        }),
    );
  }, [historyResult, historyCancelled, loadCoverage, t]);

  useEffect(() => {
    if (historyError) {
      message.error(historyError);
    }
  }, [historyError]);

  return (
    <Card
      size="small"
      title={t("stockAnalysis.moverRecall.sampleTitle")}
      data-testid="mover-sample"
      styles={{ body: { padding: "10px 12px" } }}
    >
      <div style={{ color: "var(--color-text-secondary)", marginBottom: 8 }}>
        {coverage
          ? t("stockAnalysis.moverRecall.sampleCounts", {
            total: coverage.reportedTotal > 0 ? coverage.reportedTotal : "-",
            enumerated: coverage.universeSize,
            confirmed: coverage.confirmedByClist,
            close: coverage.latestCloseCodes,
            date: coverage.closeDates.length > 0
              ? coverage.closeDates[coverage.closeDates.length - 1]
              : "-",
          })
          : t("stockAnalysis.moverRecall.sampleCoverageUnknown")}
      </div>
      <Space size={8} wrap>
        <Button
          type="primary"
          loading={sampleAction === "scan"}
          disabled={sweepRunning || historyRunning
            || (sampleAction !== null && sampleAction !== "scan")}
          onClick={() => void fullMarketScan()}
          title={t("stockAnalysis.moverRecall.fullScanHint")}
        >
          {t("stockAnalysis.moverRecall.fullScan")}
        </Button>
        <Button
          type="primary"
          loading={sampleAction === "sweep"}
          disabled={sweepRunning || historyRunning
            || (sampleAction !== null && sampleAction !== "sweep")}
          onClick={() => void sweepLatestClose()}
          title={t("stockAnalysis.moverRecall.sweepCloseHint")}
        >
          {t("stockAnalysis.moverRecall.sweepClose")}
        </Button>
        <Button
          type="primary"
          loading={sampleAction === "history"}
          disabled={sweepRunning || historyRunning
            || (sampleAction !== null && sampleAction !== "history")}
          onClick={() => void backfillHistory()}
          title={t("stockAnalysis.moverRecall.backfillHistoryHint")}
        >
          {t("stockAnalysis.moverRecall.backfillHistory")}
        </Button>
        {/* 受理后按钮立即解冻（不再 await 到执行结束）；运行中给显式状态与取消入口 */}
        {(sweepRunning || historyRunning) && (
          <>
            <Tag color="processing">{t("stockAnalysis.moverRecall.taskRunning")}</Tag>
            <Button onClick={() => void (sweepRunning ? cancelSweep() : cancelHistory())}>
              {t("stockAnalysis.moverRecall.taskCancel")}
            </Button>
          </>
        )}
      </Space>
      <div
        style={{ color: "var(--color-text-tertiary)", fontSize: 11, marginTop: 4 }}
        data-testid="mover-sample-hints"
      >
        {t("stockAnalysis.moverRecall.fullScanHint")}
        {" · "}
        {t("stockAnalysis.moverRecall.sweepCloseHint")}
        {" · "}
        {t("stockAnalysis.moverRecall.backfillHistoryHint")}
      </div>
    </Card>
  );
}

/**
 * 采集命令必须返回一份带计数的报告。`undefined`／缺字段（浏览器模式的 mock 对这些
 * 行情命令只回 `undefined`）不得读成「0 条 ⇒ 都做完了」——那是把「命令不存在」演成成功。
 */
function assertReport(r: unknown, cmd: string, field: string): void {
  const o = r as Record<string, unknown> | null;
  if (o === null || typeof o !== "object" || typeof o[field] !== "number") {
    throw new Error(`${cmd} returned no numeric "${field}" (market-data commands need the desktop app)`);
  }
}

/** `get_market_coverage` 的形状守卫：缺计数/closeDates 即视为「取不到」，不渲染成 0。 */
function isCoverage(c: unknown): c is MarketCoverage {
  if (c == null || typeof c !== "object") {
    return false;
  }
  const o = c as Record<string, unknown>;
  return typeof o.universeSize === "number"
    && typeof o.confirmedByClist === "number"
    && typeof o.reportedTotal === "number"
    && typeof o.latestCloseCodes === "number"
    && Array.isArray(o.closeDates);
}

/** 后端错误按 `detail` 成句呈现（已是完整说明），非 JSON 时退回消息原文。 */
function detailOf(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);
  try {
    const parsed = JSON.parse(msg) as { code?: unknown; detail?: unknown };
    if (typeof parsed.detail === "string" && parsed.detail.length > 0) {
      return parsed.detail;
    }
    if (typeof parsed.code === "string") {
      return parsed.code;
    }
  } catch { /* 非 JSON ⇒ 原文 */ }
  return msg;
}
