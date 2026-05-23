import { useStockAnalysisStore } from "@/stores";
import { Button, Progress, Steps, Tag } from "antd";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

const STAGES = [
  "stage.dataLoading",
  "stage.analysis",
  "stage.debate",
  "stage.risk",
  "stage.decision",
];

const TOTAL_ANALYSTS = 9; // 技术/情绪/消息/基本面/政策/资金/解禁/研报/行业
const TOTAL_DEBATE_ROUNDS = 6; // bull-r1..3 + bear-r1..3

export function AnalysisProgress() {
  const { t } = useTranslation();
  const status = useStockAnalysisStore((s) => s.status);
  const currentStage = useStockAnalysisStore((s) => s.currentStage);
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const error = useStockAnalysisStore((s) => s.error);
  const llmStatus = useStockAnalysisStore((s) => s.llmStatus);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const progressMessage = useStockAnalysisStore((s) => s.progressMessage);
  const progressPct = useStockAnalysisStore((s) => s.progressPct);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);

  if (status === "idle") { return null; }

  const currentStep = status === "completed" ? 4 : currentStage;

  const subProgress = useMemo(() => {
    const analystCount = Object.keys(analystReports).filter((k) =>
      k !== "investment-plan" && k !== "bull-researcher" && k !== "bear-researcher"
    ).length;
    switch (currentStage) {
      case 1:
        return analystCount > 0
          ? t("stockAnalysis.analystCount", { current: analystCount, total: TOTAL_ANALYSTS })
          : null;
      case 2:
        return debateRounds.length > 0
          ? t("stockAnalysis.debateRounds", { current: debateRounds.length, total: TOTAL_DEBATE_ROUNDS })
          : null;
      case 3:
        return Object.keys(riskAssessments).length > 0
          ? t("stockAnalysis.riskAssessCount", { count: Object.keys(riskAssessments).length })
          : null;
      default:
        return null;
    }
  }, [currentStage, analystReports, debateRounds, riskAssessments]);

  return (
    <div>
      {error && (
        <div
          className="mb-1 p-1.5 rounded flex justify-between items-center text-xs"
          style={{ color: "#ff4d4f", background: "#fff2f0" }}
        >
          <span>{error}</span>
          {stockCode && status === "error" && (
            <Button
              size="small"
              danger
              onClick={() => startAnalysis(stockCode)}
            >
              {t("stockAnalysis.retry")}
            </Button>
          )}
        </div>
      )}
      {llmStatus === "placeholder" && (
        <Tag color="orange" style={{ marginBottom: 8 }}>
          ⚠️ {t("stockAnalysis.offlineMode")}
        </Tag>
      )}
      <Steps
        size="small"
        current={currentStep}
        status={status === "error" ? "error" : "process"}
        items={STAGES.map((s, i) => ({
          title: (
            <span>
              {t(`stockAnalysis.${s}`)}
              {i === currentStep && subProgress && <Tag style={{ marginLeft: 6, fontSize: 10 }}>{subProgress}</Tag>}
            </span>
          ),
        }))}
      />

      {/* 实时进度提示 */}
      {(status === "running" || status === "loading") && progressMessage && (
        <div className="mt-1 flex items-center gap-1.5 text-xs" style={{ color: "var(--color-text-secondary)" }}>
          <span className="inline-block w-2 h-2 rounded-full bg-blue-500 animate-pulse" />
          <span>{progressMessage}</span>
        </div>
      )}
      {status === "completed" && progressMessage && (
        <div className="mt-1 text-xs" style={{ color: "var(--color-success-text, #52c41a)" }}>
          {progressMessage}
        </div>
      )}
      {status === "error" && progressMessage && (
        <div className="mt-1 text-xs" style={{ color: "#ff4d4f" }}>
          {progressMessage}
        </div>
      )}

      {/* 进度条 */}
      {(status === "running" || status === "loading") && progressPct > 0 && (
        <Progress percent={progressPct} size="small" showInfo={false} className="mt-1" strokeColor="#1677ff" />
      )}
    </div>
  );
}
