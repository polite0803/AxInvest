import { invoke } from "@/lib/invoke";
import { Button, Drawer, InputNumber, message, Select, Slider, Switch, Tabs, Tag } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface DataSourcesConfig {
  tencent: boolean;
  eastmoney: boolean;
  sina: boolean;
  akshare: boolean;
  baiduStock: boolean;
  cninfo: boolean;
  iwencai: boolean;
  mootdx: boolean;
  ths: boolean;
}

interface AnalysisParams {
  maxDebateRounds: number;
  klinePeriod: string;
  klineLimit: number;
  newsLimit: number;
  maxConcurrent: number;
}

interface ModelParams {
  temperature: number;
  maxTokens: number;
}

interface ScoringWeights {
  trend: number;
  deviation: number;
  macd: number;
  volume: number;
  rsi: number;
  support: number;
}

interface RuleParams {
  rsiOverbought: number;
  biasLimit: number;
  volumeSignalBlock: boolean;
  bearLowScore: number;
  rsiOversold: number;
  autoStopLossPct: number;
}

interface PositionParams {
  maxSingleStockPct: number;
  maxTotalPositions: number;
  maxSectorExposurePct: number;
}

interface ValueParams {
  dcfGrowthRate: number;
  dcfPerpetualRate: number;
  dcfDiscountRate: number;
  moatThreshold: number;
  fScoreBuyThreshold: number;
  safetyMarginMin: number;
}

interface MonitorParams {
  pollIntervalSecs: number;
  changePctThreshold: number;
  turnoverThreshold: number;
}

interface FullConfig {
  dataSources: DataSourcesConfig;
  analysis: AnalysisParams;
  model: ModelParams;
  scoring: ScoringWeights;
  rules: RuleParams;
  position: PositionParams;
  value: ValueParams;
  monitor: MonitorParams;
}

const DEFAULTS: FullConfig = {
  dataSources: {
    tencent: true,
    eastmoney: true,
    sina: true,
    akshare: false,
    baiduStock: false,
    cninfo: false,
    iwencai: false,
    mootdx: false,
    ths: false,
  },
  analysis: { maxDebateRounds: 3, klinePeriod: "daily", klineLimit: 120, newsLimit: 30, maxConcurrent: 9 },
  model: { temperature: 0.3, maxTokens: 4096 },
  scoring: { trend: 30, deviation: 20, macd: 15, volume: 15, rsi: 10, support: 10 },
  rules: {
    rsiOverbought: 80,
    biasLimit: 5,
    volumeSignalBlock: true,
    bearLowScore: 30,
    rsiOversold: 20,
    autoStopLossPct: 5,
  },
  position: { maxSingleStockPct: 20, maxTotalPositions: 10, maxSectorExposurePct: 40 },
  value: {
    dcfGrowthRate: 8,
    dcfPerpetualRate: 3,
    dcfDiscountRate: 10,
    moatThreshold: 60,
    fScoreBuyThreshold: 7,
    safetyMarginMin: 20,
  },
  monitor: { pollIntervalSecs: 30, changePctThreshold: 5, turnoverThreshold: 10 },
};

const SETTINGS_KEY = "stock_analysis_config";

function Row({ label, desc, children }: { label: string; desc?: string; children: React.ReactNode }) {
  return (
    <div style={{ padding: "6px 0", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <span style={{ fontSize: 13 }}>{label}</span>
        {desc && <div style={{ fontSize: 11, color: "var(--muted)", marginTop: 1 }}>{desc}</div>}
      </div>
      <div style={{ flexShrink: 0, marginLeft: 16 }}>{children}</div>
    </div>
  );
}

export function StockAnalysisSettingsModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<FullConfig>(DEFAULTS);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!open) { return; }
    setLoading(true);
    invoke<string | null>("get_setting", { key: SETTINGS_KEY })
      .then((v) => {
        if (v) {
          try {
            setConfig({ ...DEFAULTS, ...JSON.parse(v) });
          } catch { /* keep defaults */ }
        }
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [open]);

  const save = async (patch: Partial<FullConfig>) => {
    const merged = { ...config, ...patch };
    setConfig(merged);
    try {
      await invoke("set_setting", { key: SETTINGS_KEY, value: JSON.stringify(merged) });
    } catch {
      message.error(t("common.saveFailed"));
    }
  };

  const resetDefaults = () => {
    setConfig(DEFAULTS);
    invoke("set_setting", { key: SETTINGS_KEY, value: JSON.stringify(DEFAULTS) }).catch(() => {});
  };

  const sc = config.scoring;
  const totalWeight = sc.trend + sc.deviation + sc.macd + sc.volume + sc.rsi + sc.support;

  // ── Data Sources Tab ──
  const VENDORS: [keyof DataSourcesConfig, string, string, string][] = [
    ["tencent", "腾讯财经", "报价", "blue"],
    ["eastmoney", "东方财富", "财务+K线", "green"],
    ["sina", "新浪财经", "新闻", "orange"],
    ["ths", "同花顺", "数据", "purple"],
    ["cninfo", "巨潮资讯", "披露", "geekblue"],
    ["baiduStock", "百度股票", "数据", "cyan"],
    ["iwencai", "问财", "选股", "magenta"],
    ["akshare", "AKShare", "数据", "gold"],
    ["mootdx", "Mootdx", "本地", "lime"],
  ];

  const tabItems = [
    {
      key: "datasource",
      label: "数据源",
      children: (
        <div>
          {VENDORS.map(([key, name, tag, color]) => (
            <Row key={key} label={name}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <Tag color={color}>{tag}</Tag>
                <Switch
                  checked={config.dataSources[key]}
                  onChange={(v) => save({ dataSources: { ...config.dataSources, [key]: v } })}
                />
              </div>
            </Row>
          ))}
        </div>
      ),
    },
    {
      key: "analysis",
      label: "分析",
      children: (
        <div>
          <Row label="辩论轮数" desc="多空辩论的回合数（1-10）">
            <InputNumber
              min={1}
              max={10}
              size="small"
              style={{ width: 72 }}
              value={config.analysis.maxDebateRounds}
              onChange={(v) => v !== null && save({ analysis: { ...config.analysis, maxDebateRounds: v } })}
            />
          </Row>
          <Row label="K线周期">
            <Select
              size="small"
              style={{ width: 100 }}
              value={config.analysis.klinePeriod}
              onChange={(v) => save({ analysis: { ...config.analysis, klinePeriod: v } })}
              options={[
                { value: "daily", label: "日线" },
                { value: "weekly", label: "周线" },
                { value: "monthly", label: "月线" },
              ]}
            />
          </Row>
          <Row label="K线数量" desc="60-500">
            <InputNumber
              min={60}
              max={500}
              size="small"
              style={{ width: 72 }}
              value={config.analysis.klineLimit}
              onChange={(v) => v !== null && save({ analysis: { ...config.analysis, klineLimit: v } })}
            />
          </Row>
          <Row label="新闻数量" desc="10-100">
            <InputNumber
              min={10}
              max={100}
              size="small"
              style={{ width: 72 }}
              value={config.analysis.newsLimit}
              onChange={(v) => v !== null && save({ analysis: { ...config.analysis, newsLimit: v } })}
            />
          </Row>
          <Row label="并行分析数" desc="1-20">
            <InputNumber
              min={1}
              max={20}
              size="small"
              style={{ width: 72 }}
              value={config.analysis.maxConcurrent}
              onChange={(v) => v !== null && save({ analysis: { ...config.analysis, maxConcurrent: v } })}
            />
          </Row>
        </div>
      ),
    },
    {
      key: "scoring",
      label: "算法",
      children: (
        <div>
          <div style={{ fontSize: 11, color: "var(--muted)", marginBottom: 8 }}>
            评分权重（总计需=100，当前={totalWeight}）
          </div>
          {(["trend", "deviation", "macd", "volume", "rsi", "support"] as const).map((key) => {
            const labels: Record<string, string> = {
              trend: "趋势",
              deviation: "乖离率",
              macd: "MACD",
              volume: "量能",
              rsi: "RSI",
              support: "支撑",
            };
            return (
              <Row key={key} label={`${labels[key]} (${config.scoring[key]})`}>
                <Slider
                  style={{ width: 120, margin: 0 }}
                  min={0}
                  max={50}
                  step={1}
                  value={config.scoring[key]}
                  onChange={(v) => save({ scoring: { ...config.scoring, [key]: v } })}
                />
              </Row>
            );
          })}
          <Button size="small" type="link" onClick={() => save({ scoring: { ...DEFAULTS.scoring } })}>
            恢复默认权重
          </Button>
        </div>
      ),
    },
    {
      key: "rules",
      label: "规则",
      children: (
        <div>
          <Row label="RSI 超买阈值" desc="高于此值禁止买入">
            <InputNumber
              min={50}
              max={100}
              size="small"
              style={{ width: 72 }}
              value={config.rules.rsiOverbought}
              onChange={(v) => v !== null && save({ rules: { ...config.rules, rsiOverbought: v } })}
            />
          </Row>
          <Row label="乖离率追高阈值 (%)" desc="高于此值禁止追高">
            <InputNumber
              min={1}
              max={20}
              size="small"
              style={{ width: 72 }}
              value={config.rules.biasLimit}
              onChange={(v) => v !== null && save({ rules: { ...config.rules, biasLimit: v } })}
            />
          </Row>
          <Row label="放量下跌禁买">
            <Switch
              checked={config.rules.volumeSignalBlock}
              onChange={(v) => save({ rules: { ...config.rules, volumeSignalBlock: v } })}
            />
          </Row>
          <Row label="空头+低分阈值" desc="空头排列且评分低于此值则禁买">
            <InputNumber
              min={0}
              max={100}
              size="small"
              style={{ width: 72 }}
              value={config.rules.bearLowScore}
              onChange={(v) => v !== null && save({ rules: { ...config.rules, bearLowScore: v } })}
            />
          </Row>
          <Row label="RSI 超卖提醒" desc="低于此值时提示超跌反弹">
            <InputNumber
              min={5}
              max={40}
              size="small"
              style={{ width: 72 }}
              value={config.rules.rsiOversold}
              onChange={(v) => v !== null && save({ rules: { ...config.rules, rsiOversold: v } })}
            />
          </Row>
          <Row label="自动止损 (%)" desc="未设止损时自动计算入场价-此百分比">
            <InputNumber
              min={1}
              max={20}
              size="small"
              style={{ width: 72 }}
              value={config.rules.autoStopLossPct}
              onChange={(v) => v !== null && save({ rules: { ...config.rules, autoStopLossPct: v } })}
            />
          </Row>
        </div>
      ),
    },
    {
      key: "position",
      label: "仓位",
      children: (
        <div>
          <Row label="单股最大仓位 (%)" desc="单只股票占比上限">
            <InputNumber
              min={5}
              max={100}
              size="small"
              style={{ width: 72 }}
              value={config.position.maxSingleStockPct}
              onChange={(v) => v !== null && save({ position: { ...config.position, maxSingleStockPct: v } })}
            />
          </Row>
          <Row label="最大持仓数" desc="同时持有的股票数量上限">
            <InputNumber
              min={1}
              max={50}
              size="small"
              style={{ width: 72 }}
              value={config.position.maxTotalPositions}
              onChange={(v) => v !== null && save({ position: { ...config.position, maxTotalPositions: v } })}
            />
          </Row>
          <Row label="单一行业最大暴露 (%)" desc="同一行业占比上限">
            <InputNumber
              min={10}
              max={100}
              size="small"
              style={{ width: 72 }}
              value={config.position.maxSectorExposurePct}
              onChange={(v) => v !== null && save({ position: { ...config.position, maxSectorExposurePct: v } })}
            />
          </Row>
        </div>
      ),
    },
    {
      key: "value",
      label: "估值",
      children: (
        <div>
          <Row label="DCF 阶段1 增长率 (%)" desc="前5年高增长期">
            <InputNumber
              min={1}
              max={30}
              size="small"
              style={{ width: 72 }}
              value={config.value.dcfGrowthRate}
              onChange={(v) => v !== null && save({ value: { ...config.value, dcfGrowthRate: v } })}
            />
          </Row>
          <Row label="DCF 永续增长率 (%)" desc="阶段2 永续增长">
            <InputNumber
              min={0}
              max={10}
              size="small"
              style={{ width: 72 }}
              value={config.value.dcfPerpetualRate}
              onChange={(v) => v !== null && save({ value: { ...config.value, dcfPerpetualRate: v } })}
            />
          </Row>
          <Row label="DCF 折现率 (%)">
            <InputNumber
              min={5}
              max={20}
              size="small"
              style={{ width: 72 }}
              value={config.value.dcfDiscountRate}
              onChange={(v) => v !== null && save({ value: { ...config.value, dcfDiscountRate: v } })}
            />
          </Row>
          <Row label="护城河阈值" desc=">=此值判定为宽护城河">
            <InputNumber
              min={30}
              max={90}
              size="small"
              style={{ width: 72 }}
              value={config.value.moatThreshold}
              onChange={(v) => v !== null && save({ value: { ...config.value, moatThreshold: v } })}
            />
          </Row>
          <Row label="F-Score 买入阈值" desc=">=此值认为财务健康">
            <InputNumber
              min={3}
              max={9}
              size="small"
              style={{ width: 72 }}
              value={config.value.fScoreBuyThreshold}
              onChange={(v) => v !== null && save({ value: { ...config.value, fScoreBuyThreshold: v } })}
            />
          </Row>
          <Row label="最低安全边际 (%)">
            <InputNumber
              min={5}
              max={60}
              size="small"
              style={{ width: 72 }}
              value={config.value.safetyMarginMin}
              onChange={(v) => v !== null && save({ value: { ...config.value, safetyMarginMin: v } })}
            />
          </Row>
        </div>
      ),
    },
    {
      key: "monitor",
      label: "监控",
      children: (
        <div>
          <Row label="轮询间隔 (秒)" desc="30-600">
            <InputNumber
              min={10}
              max={600}
              size="small"
              style={{ width: 72 }}
              value={config.monitor.pollIntervalSecs}
              onChange={(v) => v !== null && save({ monitor: { ...config.monitor, pollIntervalSecs: v } })}
            />
          </Row>
          <Row label="涨跌幅异常 (%)" desc="变动超过此值触发告警">
            <InputNumber
              min={1}
              max={20}
              size="small"
              style={{ width: 72 }}
              value={config.monitor.changePctThreshold}
              onChange={(v) => v !== null && save({ monitor: { ...config.monitor, changePctThreshold: v } })}
            />
          </Row>
          <Row label="换手率异常 (%)" desc="换手率超过此值触发告警">
            <InputNumber
              min={1}
              max={50}
              size="small"
              style={{ width: 72 }}
              value={config.monitor.turnoverThreshold}
              onChange={(v) => v !== null && save({ monitor: { ...config.monitor, turnoverThreshold: v } })}
            />
          </Row>
        </div>
      ),
    },
    {
      key: "llm",
      label: "LLM",
      children: (
        <div>
          <Row label="Temperature" desc="0-1，越高越随机">
            <Slider
              style={{ width: 120, margin: 0 }}
              min={0}
              max={1}
              step={0.05}
              value={config.model.temperature}
              onChange={(v) => save({ model: { ...config.model, temperature: v } })}
            />
          </Row>
          <Row label="Max Tokens" desc="256-16384">
            <InputNumber
              min={256}
              max={16384}
              step={256}
              size="small"
              style={{ width: 80 }}
              value={config.model.maxTokens}
              onChange={(v) => v !== null && save({ model: { ...config.model, maxTokens: v } })}
            />
          </Row>
        </div>
      ),
    },
  ];

  return (
    <Drawer
      title="股票分析设置"
      placement="right"
      width={420}
      open={open}
      onClose={onClose}
      footer={
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <Button onClick={resetDefaults}>恢复默认</Button>
          <Button type="primary" onClick={onClose}>关闭</Button>
        </div>
      }
    >
      {loading
        ? <div style={{ textAlign: "center", padding: 24 }}>加载中...</div>
        : <Tabs tabPosition="top" size="small" items={tabItems} />}
    </Drawer>
  );
}
