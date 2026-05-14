import { useStockAnalysisStore } from "@/stores";
import { AnalystReportCard } from "./AnalystReportCard";

export function AnalystReportGrid() {
  const analystReports = useStockAnalysisStore((s) => s.analystReports);

  if (Object.keys(analystReports).length === 0) { return null; }

  return (
    <div
      className="grid gap-2"
      style={{ gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))" }}
    >
      {Object.entries(analystReports).map(([expertId, report]) => (
        <AnalystReportCard key={expertId} expertId={expertId} report={report} />
      ))}
    </div>
  );
}
