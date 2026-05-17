import { invoke } from "@/lib/invoke";
import { Button, InputNumber, message, Select, Slider, Switch, Tag } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "./SettingsGroup";

interface StockAnalysisConfig {
  dataSources: {
    tencent: boolean;
    eastmoney: boolean;
    sina: boolean;
  };
  analysis: {
    maxDebateRounds: number;
    klinePeriod: string;
    klineLimit: number;
    newsLimit: number;
  };
  trading: {
    enabled: boolean;
    maxSinglePositionPct: number;
    maxTotalPositionPct: number;
    maxPositions: number;
  };
  model: {
    temperature: number;
    maxTokens: number;
  };
}

const DEFAULTS: StockAnalysisConfig = {
  dataSources: { tencent: true, eastmoney: true, sina: true },
  analysis: { maxDebateRounds: 3, klinePeriod: "daily", klineLimit: 120, newsLimit: 30 },
  trading: { enabled: false, maxSinglePositionPct: 30, maxTotalPositionPct: 80, maxPositions: 10 },
  model: { temperature: 0.3, maxTokens: 4096 },
};

const SETTINGS_KEY = "stock_analysis_config";

function useStockAnalysisConfig() {
  const [config, setConfig] = useState<StockAnalysisConfig>(DEFAULTS);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<string | null>("get_setting", { key: SETTINGS_KEY })
      .then((v) => {
        if (v) {
          try { setConfig({ ...DEFAULTS, ...JSON.parse(v) }); } catch { /* keep defaults */ }
        }
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const save = async (partial: Partial<StockAnalysisConfig>) => {
    const merged = { ...config, ...partial };
    setConfig(merged);
    try {
      await invoke("set_setting", { key: SETTINGS_KEY, value: JSON.stringify(merged) });
    } catch {
      message.error("设置保存失败");
    }
  };

  return { config, save, loading };
}

export function StockAnalysisSettings() {
  const { t } = useTranslation();
  const { config, save, loading } = useStockAnalysisConfig();
  const rowStyle = { padding: "4px 0" };

  if (loading) return <div className="p-6">{t("common.loading")}</div>;

  return (
    <div className="p-6 pb-12">
      {/* 数据源 */}
      <SettingsGroup title={t("stockAnalysis.settings.dataSources")}>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>腾讯财经 <Tag color="blue">行情</Tag></span>
          <Switch checked={config.dataSources.tencent}
            onChange={(v) => save({ dataSources: { ...config.dataSources, tencent: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>东方财富 <Tag color="green">财务/K线</Tag></span>
          <Switch checked={config.dataSources.eastmoney}
            onChange={(v) => save({ dataSources: { ...config.dataSources, eastmoney: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>新浪财经 <Tag color="orange">新闻</Tag></span>
          <Switch checked={config.dataSources.sina}
            onChange={(v) => save({ dataSources: { ...config.dataSources, sina: v } })} />
        </div>
      </SettingsGroup>

      {/* 分析参数 */}
      <SettingsGroup title={t("stockAnalysis.settings.analysis")}>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.debateRounds")}</span>
          <InputNumber min={1} max={5} size="small" style={{ width: 80 }}
            value={config.analysis.maxDebateRounds}
            onChange={(v) => v && save({ analysis: { ...config.analysis, maxDebateRounds: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.klinePeriod")}</span>
          <Select size="small" style={{ width: 120 }}
            value={config.analysis.klinePeriod}
            onChange={(v) => save({ analysis: { ...config.analysis, klinePeriod: v } })}
            options={[
              { value: "daily", label: t("stockAnalysis.settings.periodDaily") },
              { value: "weekly", label: t("stockAnalysis.settings.periodWeekly") },
              { value: "monthly", label: t("stockAnalysis.settings.periodMonthly") },
            ]} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.klineLimit")}</span>
          <InputNumber min={60} max={500} step={30} size="small" style={{ width: 80 }}
            value={config.analysis.klineLimit}
            onChange={(v) => v && save({ analysis: { ...config.analysis, klineLimit: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.newsLimit")}</span>
          <InputNumber min={10} max={100} size="small" style={{ width: 80 }}
            value={config.analysis.newsLimit}
            onChange={(v) => v && save({ analysis: { ...config.analysis, newsLimit: v } })} />
        </div>
      </SettingsGroup>

      {/* 交易设置 */}
      <SettingsGroup title={t("stockAnalysis.settings.trading")}>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.tradingEnabled")}</span>
          <Switch checked={config.trading.enabled}
            onChange={async (v) => {
              await invoke("toggle_trading_enabled", { enabled: v });
              save({ trading: { ...config.trading, enabled: v } });
            }} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.maxSinglePosition")}</span>
          <InputNumber min={5} max={50} size="small" style={{ width: 80 }} suffix="%"
            value={config.trading.maxSinglePositionPct}
            onChange={(v) => v != null && save({ trading: { ...config.trading, maxSinglePositionPct: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.maxTotalPosition")}</span>
          <InputNumber min={10} max={100} size="small" style={{ width: 80 }} suffix="%"
            value={config.trading.maxTotalPositionPct}
            onChange={(v) => v != null && save({ trading: { ...config.trading, maxTotalPositionPct: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.maxPositions")}</span>
          <InputNumber min={1} max={20} size="small" style={{ width: 80 }}
            value={config.trading.maxPositions}
            onChange={(v) => v != null && save({ trading: { ...config.trading, maxPositions: v } })} />
        </div>
      </SettingsGroup>

      {/* 模型参数 */}
      <SettingsGroup title={t("stockAnalysis.settings.model")}>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.temperature")} ({config.model.temperature})</span>
          <Slider min={0} max={1} step={0.1} style={{ width: 200 }}
            value={config.model.temperature}
            onChange={(v) => save({ model: { ...config.model, temperature: v } })} />
        </div>
        <div style={rowStyle} className="flex items-center justify-between">
          <span>{t("stockAnalysis.settings.maxTokens")}</span>
          <Select size="small" style={{ width: 120 }}
            value={config.model.maxTokens}
            onChange={(v) => save({ model: { ...config.model, maxTokens: v } })}
            options={[
              { value: 1024, label: "1024" },
              { value: 2048, label: "2048" },
              { value: 4096, label: "4096" },
              { value: 8192, label: "8192" },
            ]} />
        </div>
        <div style={rowStyle} className="flex justify-end pt-2">
          <Button size="small" onClick={() => save(DEFAULTS)}>
            {t("stockAnalysis.settings.resetDefaults")}
          </Button>
        </div>
      </SettingsGroup>
    </div>
  );
}
