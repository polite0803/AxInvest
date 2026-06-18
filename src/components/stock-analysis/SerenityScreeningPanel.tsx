import { invoke } from "@/lib/invoke";
import {
  type SerenityCandidate,
  type StepStage,
  type TrendInfo,
  useSerenityStore,
} from "@/stores/feature/serenityStore";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { PlayCircleOutlined, ReloadOutlined, StockOutlined } from "@ant-design/icons";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Alert, Button, Card, Empty, Space, Tag, Typography } from "antd";
import { useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";

const { Text, Title } = Typography;

/** 根据节点 id 推测当前阶段 */
function inferStage(nodeId?: string): StepStage {
  if (!nodeId) { return "loading"; }
  if (nodeId.startsWith("t-") || nodeId === "a-trend-scanner") { return "scanning"; }
  if (nodeId.startsWith("a-chain-")) { return "decomposing"; }
  if (nodeId.startsWith("a-chokepoint-")) { return "identifying"; }
  if (nodeId.startsWith("a-candidate") || nodeId.startsWith("t-candidates")) { return "mapping"; }
  if (nodeId.startsWith("s-")) { return "saving"; }
  return "loading";
}

export function SerenityScreeningPanel() {
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const {
    running,
    setRunning,
    stage,
    setStage,
    candidates,
    setCandidates,
    trends,
    setTrends,
    error,
    setError,
    completedNodes,
    totalNodes,
    setCompletedNodes,
    setTotalNodes,
  } = useSerenityStore();
  const { t } = useTranslation();
  const getStageLabel = (s: StepStage) => t(`serenityPanel.stage_${s}` as const);

  // 监听工作流事件
  useEffect(() => {
    let unlistenStep: UnlistenFn | undefined;
    let unlistenDone: UnlistenFn | undefined;

    (async () => {
      unlistenStep = await listen<{
        nodeId?: string;
        status?: string;
        completedNodes?: number;
        totalNodes?: number;
        output?: unknown;
      }>("serenity-screening-step", (event) => {
        const st = inferStage(event.payload.nodeId);
        setStage(st);
        if (typeof event.payload.completedNodes === "number") {
          setCompletedNodes(event.payload.completedNodes);
        }
        if (typeof event.payload.totalNodes === "number") {
          setTotalNodes(event.payload.totalNodes);
        }

        // 从 a-trend-scanner 输出中提取趋势信息
        if (event.payload.nodeId === "a-trend-scanner" && event.payload.output) {
          try {
            const out: Record<string, unknown> = typeof event.payload.output === "string"
              ? JSON.parse(event.payload.output)
              : (event.payload.output as Record<string, unknown>);
            if (out?.trends) {
              setTrends(out.trends as TrendInfo[]);
            }
          } catch {
            // ignore parse errors in progress events
          }
        }
      });

      unlistenDone = await listen<{
        status?: string;
        result?: { candidates?: SerenityCandidate[] };
        candidates?: SerenityCandidate[];
        error?: string;
      }>("serenity-screening-completed", (event) => {
        setRunning(false);
        if (event.payload.status === "failed") {
          setStage("error");
          setError(event.payload.error ?? t("serenityPanel.errorUnknown"));
          return;
        }
        setStage("done");

        // 兼容两种格式：result.candidates 或直接 candidates
        // result = {candidates: [...]} 或裸数组
        const raw = event.payload.result?.candidates ?? event.payload.candidates ?? event.payload.result ?? [];
        const rawArr = Array.isArray(raw) ? (raw as Array<Record<string, unknown>>) : [];
        const normalized: SerenityCandidate[] = rawArr.map((c) => ({
          stockCode: (c.stockCode ?? c.stock_code ?? c.stockName ?? "") as string,
          stockName: (c.stockName ?? c.stock_name ?? "") as string,
          stock_code: (c.stockCode ?? c.stock_code ?? "") as string,
          relevance: (c.relevance ?? "") as string,
          serenityScore: (c.serenityScore ?? c.serenity_score ?? 0) as number,
          confidence: (c.confidence ?? 0) as number,
          bottleneckProduct: (c.bottleneckProduct ?? c.bottleneck_product ?? "") as string,
          primaryRisk: (c.primaryRisk ?? c.primary_risk ?? "") as string,
        }));
        setCandidates(normalized);
      });
    })();

    return () => {
      unlistenStep?.();
      unlistenDone?.();
    };
    // 稳定引用：运行时调用的 setXxx 不会变化
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleRun = useCallback(async () => {
    setRunning(true);
    setStage("loading");
    setError(null);

    try {
      const r = await invoke<any>("run_serenity_screening");
      // r = {status: "completed", result: {candidates: [...]}}
      const arr = r?.result?.candidates ?? r?.candidates?.candidates ?? r?.candidates;
      if (Array.isArray(arr) && arr.length > 0) {
        setCandidates(arr as SerenityCandidate[]);
      }
      setStage("done");
    } catch (err) {
      setStage("error");
      setError(typeof err === "string" ? err : t("serenityPanel.callFailed"));
    } finally {
      setRunning(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [t]);

  const handleAnalyze = useCallback(
    (code: string) => {
      if (code) { startAnalysis(code); }
    },
    [startAnalysis],
  );

  // 候选标签颜色
  const relevanceColor = (rel: string) => {
    if (rel === "direct") { return "green"; }
    if (rel === "indirect") { return "blue"; }
    return "default";
  };
  const relevanceLabel = (rel: string) => {
    if (rel === "direct") { return t("serenityPanel.directBenefit"); }
    if (rel === "indirect") { return t("serenityPanel.indirectBenefit"); }
    return t("serenityPanel.themeRelated");
  };

  return (
    <div className="flex flex-col gap-3">
      {/* 操作栏 */}
      <div className="flex items-center justify-between">
        <Text type="secondary" className="text-xs">
          {t("serenityPanel.desc")}
        </Text>
        <Button
          type="primary"
          icon={running ? <ReloadOutlined spin /> : <PlayCircleOutlined />}
          loading={running}
          onClick={handleRun}
        >
          {running ? t("serenityPanel.running") : t("serenityPanel.run")}
        </Button>
      </div>

      {/* 进度状态 */}
      {running && (
        <Alert
          type="info"
          showIcon
          message={<>
            {getStageLabel(stage)}
            {totalNodes > 0 && <span className="ml-1 text-xs opacity-70">({completedNodes}/{totalNodes})</span>}
          </>}
          description={stage !== "loading"
            ? t("serenityPanel.analyzing")
            : undefined}
        />
      )}

      {/* 错误 */}
      {error && (
        <Alert
          type="error"
          showIcon
          message={t("serenityPanel.stage_error")}
          description={error}
          closable
          onClose={() => setError(null)}
        />
      )}

      {/* 趋势摘要 */}
      {trends.length > 0 && !running && (
        <Card size="small" title={t("serenityPanel.trendTitle")} className="w-full">
          <Space direction="vertical" className="w-full">
            {trends.map((tr, i) => (
              <div key={i} className="flex items-center gap-2 text-sm">
                <Tag color="purple">{tr.confidence ?? "?"}%</Tag>
                <Text strong>{tr.trend_name ?? tr.trendName}</Text>
                {tr.bottleneck_candidate && (
                  <Text type="secondary" className="text-xs">
                    {t("serenityPanel.bottleneckLink")}
                    {tr.bottleneck_candidate}
                  </Text>
                )}
              </div>
            ))}
          </Space>
        </Card>
      )}

      {/* 候选股列表 */}
      {candidates.length > 0 && (
        <div className="flex flex-col gap-2">
          <Title level={5} className="m-0">
            {t("serenityPanel.candidateTitle")} ({candidates.length})
          </Title>
          {candidates.map((c, i) => {
            const code = c.stock_code ?? c.stockCode ?? "";
            const name = c.stockName ?? c.stock_name ?? "";
            return (
              <Card
                key={`${code}-${i}`}
                size="small"
                hoverable
                className="w-full"
                onClick={() => handleAnalyze(code)}
              >
                <div className="flex items-start justify-between">
                  <div className="flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <Text strong className="text-sm">
                        {name}
                      </Text>
                      <Text type="secondary" className="text-xs font-mono">
                        {code}
                      </Text>
                      {c.relevance && (
                        <Tag color={relevanceColor(c.relevance)} className="text-xs">
                          {relevanceLabel(c.relevance)}
                        </Tag>
                      )}
                    </div>
                    {c.bottleneckProduct ?? c.bottleneck_product
                      ? (
                        <Text type="secondary" className="text-xs">
                          {t("serenityPanel.bottleneckProduct")}
                          {c.bottleneckProduct ?? c.bottleneck_product}
                        </Text>
                      )
                      : null}
                    {c.primaryRisk ?? c.primary_risk
                      ? (
                        <Text type="danger" className="text-xs">
                          {t("serenityPanel.riskPrefix")}
                          {c.primaryRisk ?? c.primary_risk}
                        </Text>
                      )
                      : null}
                  </div>
                  <div className="flex flex-col items-end gap-1">
                    <Tag color="purple" className="text-xs font-bold">
                      {c.serenityScore ?? c.serenity_score ?? 0}
                      {t("serenityPanel.scoreSuffix")}
                    </Tag>
                    {c.confidence
                      ? (
                        <Text type="secondary" className="text-xs">
                          {t("serenityPanel.confidencePrefix")}
                          {c.confidence}%
                        </Text>
                      )
                      : null}
                  </div>
                </div>
              </Card>
            );
          })}
        </div>
      )}

      {/* 空状态 */}
      {!running && !error && candidates.length === 0 && trends.length === 0 && (
        <Empty
          image={<StockOutlined style={{ fontSize: 48, opacity: 0.3 }} />}
          description={t("serenityPanel.emptyHint")}
        />
      )}
    </div>
  );
}
