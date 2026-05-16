import { useStockAnalysisStore } from "@/stores";
import { Card, Tag } from "antd";
import { useTranslation } from "react-i18next";

/** 风险类型 → 标签名映射（匹配后端 AnalysisEvent::RiskAssessment 的 risk_type 字段） */
const RISK_INFO: Record<string, { label: string; color: string }> = {
  "aggressive-debator": { label: "风险激进", color: "#f85149" },
  "conservative-debator": { label: "风险保守", color: "#3fb950" },
  "neutral-debator": { label: "风险中性", color: "#58a6ff" },
  "research-manager": { label: "研究经理", color: "#d29922" },
  "comprehensive": { label: "综合评估", color: "#a371f7" },
};

function riskDisplay(type: string) {
  const info = RISK_INFO[type];
  if (info) { return info; }
  // 回退：截取短名用于颜色 hash
  const hash = type.split("").reduce((a, c) => a + c.charCodeAt(0), 0);
  const hue = hash % 360;
  return { label: type, color: `hsl(${hue}, 50%, 45%)` };
}

export function RiskMatrix() {
  const { t } = useTranslation();
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);

  if (Object.keys(riskAssessments).length === 0) { return null; }

  return (
    <Card size="small" title={t("stockAnalysis.riskAssessment")}>
      <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))" }}>
        {Object.entries(riskAssessments).map(([type, report]) => {
          const { label, color } = riskDisplay(type);
          return (
            <div key={type} className="p-2 rounded" style={{ background: "var(--color-bg-elevated)" }}>
              <Tag color={color}>{label}</Tag>
              <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>{report}</p>
            </div>
          );
        })}
      </div>
    </Card>
  );
}
