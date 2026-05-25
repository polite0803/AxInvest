import { StockAnalysisConfigPanel } from "./StockAnalysisConfigPanel";

export function StockAnalysisSettings() {
  return (
    <div className="p-6 pb-12">
      <StockAnalysisConfigPanel showVendorHealth />
    </div>
  );
}
