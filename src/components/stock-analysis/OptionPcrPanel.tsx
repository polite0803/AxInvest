import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface OptionPCR {
  stockCode: string;
  date: string;
  callVolume: number;
  putVolume: number;
  callOi: number;
  putOi: number;
  volumePcr: number;
  oiPcr: number;
}

export function OptionPcrPanel({ stockCode }: { stockCode: string }) {
  const { t } = useTranslation();
  const [pcr, setPcr] = useState<OptionPCR | null>(null);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    if (!stockCode) { return; }
    setLoading(true);
    setFetchError(false);
    try {
      const result = await invoke<OptionPCR | null>("get_stock_option_pcr", { stockCode });
      setPcr(result);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, [stockCode]);

  useEffect(() => {
    load();
  }, [load]);

  const pcrColor = (v: number) => v > 1 ? "var(--sa-green)" : "var(--sa-red)";

  return (
    <Card
      size="small"
      title={`📊 ${t("stockAnalysis.optionPcr")}`}
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {!stockCode
        ? <Empty description={t("stockAnalysis.searchPlaceholder")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : !pcr
        ? <Empty description={t("stockAnalysis.noRecords")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <div className="space-y-2">
            <div className="text-xs text-gray-500">{pcr.date}</div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <span className="text-xs">{t("stockAnalysis.optionPcrVolume")}</span>
                <div className="text-lg font-bold" style={{ color: pcrColor(pcr.volumePcr) }}>
                  {pcr.volumePcr.toFixed(2)}
                </div>
              </div>
              <div>
                <span className="text-xs">{t("stockAnalysis.optionPcrOi")}</span>
                <div className="text-lg font-bold" style={{ color: pcrColor(pcr.oiPcr) }}>
                  {pcr.oiPcr.toFixed(2)}
                </div>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-1 text-xs text-gray-500">
              <span>Call Vol: {pcr.callVolume.toLocaleString()}</span>
              <span>Put Vol: {pcr.putVolume.toLocaleString()}</span>
              <span>Call OI: {pcr.callOi.toLocaleString()}</span>
              <span>Put OI: {pcr.putOi.toLocaleString()}</span>
            </div>
          </div>
        )}
    </Card>
  );
}
