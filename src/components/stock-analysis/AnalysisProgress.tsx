import { useProviderStore, useStockAnalysisStore } from "@/stores";
import { Button, Steps, Tag } from "antd";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import dayjs from "dayjs";

const STAGES = [
  "stage.dataLoading",
  "stage.analysis",
  "stage.debate",
  "stage.risk",
  "stage.decision",
];

const TOTAL_ANALYSTS = 7;

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
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const defaultProviderId = useProviderStore((s) => s.providers.find((p) => p.enabled)?.id ?? "");

  if (status === "idle") return null;

  const currentStep = status === "completed" ? 4 : currentStage;

  const subProgress = useMemo(() => {
    const analystCount = Object.keys(analystReports).filter((k) =>
      k !== "investment-plan" && k !== "bull-researcher" && k !== "bear-researcher",
    ).length;
    switch (currentStage) {
      case 1: return analystCount > 0 ? `${analystCount}/${TOTAL_ANALYSTS} 分析师` : null;
      case 2: return debateRounds.length > 0 ? `${debateRounds.length}/3 轮辩论` : null;
      case 3: return Object.keys(riskAssessments).length > 0
        ? `${Object.keys(riskAssessments).length} 项评估` : null;
      default: return null;
    }
  }, [currentStage, analystReports, debateRounds, riskAssessments]);

  return (
    <div>
      {error && (
        <div className="mb-2 p-2 rounded flex justify-between items-center" style={{ color: "#ff4d4f", background: "#fff2f0" }}>
          <span>{error}</span>
          {stockCode && status === "error" && (
            <Button size="small" danger onClick={() => startAnalysis(stockCode, dayjs().format("YYYY-MM-DD"), defaultProviderId)}>
              {t("stockAnalysis.retry")}
            </Button>
          )}
        </div>
      )}
      {llmStatus === "placeholder" && (
        <Tag color="orange" style={{ marginBottom: 8 }}>
          ⚠️ 离线模式 (LLM Driver 未连接，占位数据)
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
              {i === currentStep && subProgress && (
                <Tag style={{ marginLeft: 6, fontSize: 10 }}>{subProgress}</Tag>
              )}
            </span>
          ),
        }))}
      />
    </div>
  );
}
