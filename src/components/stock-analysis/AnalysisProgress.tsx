import { useStockAnalysisStore } from "@/stores";
import { Steps } from "antd";
import { useTranslation } from "react-i18next";

const STAGES = [
  "stage.dataLoading",
  "stage.analysis",
  "stage.debate",
  "stage.risk",
  "stage.decision",
];

export function AnalysisProgress() {
  const { t } = useTranslation();
  const status = useStockAnalysisStore((s) => s.status);
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const error = useStockAnalysisStore((s) => s.error);

  if (status === "idle") { return null; }

  let currentStep = 0;
  if (status === "running" || status === "completed") {
    const reportCount = Object.keys(analystReports).length;
    if (reportCount >= 1) { currentStep = 1; }
    if (debateRounds.length > 0) { currentStep = 2; }
    if (Object.keys(riskAssessments).length >= 1) { currentStep = 3; }
  }
  if (status === "completed") { currentStep = 4; }

  return (
    <div>
      {error && (
        <div className="mb-2 p-2 rounded" style={{ color: "#ff4d4f", background: "#fff2f0" }}>
          {error}
        </div>
      )}
      <Steps
        size="small"
        current={currentStep}
        status={status === "error" ? "error" : "process"}
        items={STAGES.map((s) => ({ title: t(`stockAnalysis.${s}`) }))}
      />
    </div>
  );
}
