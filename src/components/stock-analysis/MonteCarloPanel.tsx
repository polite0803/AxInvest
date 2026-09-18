import { invoke } from "@/lib/invoke";
import type { McRunRequest, RobustnessResult } from "@/types/market-sim";
import {
  Button,
  Card,
  Checkbox,
  Col,
  Descriptions,
  Divider,
  Input,
  InputNumber,
  Row,
  Spin,
  Statistic,
  Table,
  Tag,
} from "antd";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

interface MonteCarloPanelProps {
  /** 当前分析标的代码 —— 作为面板默认值，用户仍可覆盖 */
  stockCode?: string;
  /** 当前价（元）—— 面板内部换算为「分」提交后端 */
  referencePriceYuan?: number | null;
}

interface ScenarioConfig {
  key: string;
  label: string;
  enabled: boolean;
  paths: number;
}

const FALLBACK_STOCK_CODE = "000001";
const FALLBACK_REF_PRICE_FEN = 1000;

/**
 * 默认场景配置。
 *
 * 压力场景（闪崩 / 高波动）默认**开启** —— 这个面板的意义就是回答
 * 「最坏会怎样」，只跑 normal/bull/bear 等于不测压力。
 */
const DEFAULT_SCENARIOS: ScenarioConfig[] = [
  { key: "normal", label: "stockAnalysis.monteCarlo.normal", enabled: true, paths: 20 },
  { key: "bull", label: "stockAnalysis.monteCarlo.bull", enabled: true, paths: 20 },
  { key: "bear", label: "stockAnalysis.monteCarlo.bear", enabled: true, paths: 20 },
  { key: "flash_crash", label: "stockAnalysis.monteCarlo.flashCrash", enabled: true, paths: 15 },
  { key: "high_vol", label: "stockAnalysis.monteCarlo.highVol", enabled: true, paths: 15 },
];

/** 场景预设：key → 每场景路径数（0 表示不启用） */
const PRESETS: Record<string, Record<string, number>> = {
  quick: { normal: 10, bull: 10, bear: 10, flash_crash: 0, high_vol: 0 },
  stress: { normal: 10, bull: 0, bear: 10, flash_crash: 30, high_vol: 30 },
  deep: { normal: 40, bull: 40, bear: 40, flash_crash: 40, high_vol: 40 },
};

/**
 * MonteCarloPanel — 多场景鲁棒性（压力）测试面板。
 *
 * 作为股票分析页「模拟仿真」标签的子面板，标的默认取自当前分析上下文。
 *
 * ⚠️ 读结果前必读：
 * - `survivalRate` 是**上涨场景占比**（终价高于参考价的场景数 / 有效场景数），
 *   由勾选了哪些场景决定，**不代表个股质地**；
 * - `consistencyScore` 为 `null` 表示**不可判定**（各场景涨跌幅均值趋零，
 *   变异系数无定义），这是最分歧的情形，不是最一致。
 */
export function MonteCarloPanel({ stockCode: stockCodeProp, referencePriceYuan }: MonteCarloPanelProps = {}) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<RobustnessResult | null>(null);
  const [stockCode, setStockCode] = useState(() => stockCodeProp?.trim() || FALLBACK_STOCK_CODE);
  const [refPrice, setRefPrice] = useState(() =>
    referencePriceYuan != null && referencePriceYuan > 0
      ? Math.round(referencePriceYuan * 100)
      : FALLBACK_REF_PRICE_FEN
  );
  const [simMs, setSimMs] = useState(50);
  const [scenarios, setScenarios] = useState<ScenarioConfig[]>(DEFAULT_SCENARIOS);
  const tokenRef = useRef(0);
  const mountedRef = useRef(false);

  // 上下文（当前分析标的 / 现价）可能在挂载后才到（行情异步加载）⇒ 变化时同步。
  // 首次渲染已由 useState 初始化函数处理，故跳过。
  useEffect(() => {
    if (!mountedRef.current) {
      mountedRef.current = true;
      return;
    }
    if (stockCodeProp?.trim()) {
      setStockCode(stockCodeProp.trim());
    }
  }, [stockCodeProp]);

  useEffect(() => {
    if (referencePriceYuan != null && referencePriceYuan > 0) {
      setRefPrice(Math.round(referencePriceYuan * 100));
    }
  }, [referencePriceYuan]);

  const toggleScenario = (key: string) => {
    setScenarios((prev) => prev.map((s) => (s.key === key ? { ...s, enabled: !s.enabled } : s)));
  };

  const setPaths = (key: string, paths: number) => {
    setScenarios((prev) => prev.map((s) => (s.key === key ? { ...s, paths } : s)));
  };

  const applyPreset = (presetKey: string) => {
    const preset = PRESETS[presetKey];
    if (!preset) {
      return;
    }
    setScenarios((prev) =>
      prev.map((s) => {
        const paths = preset[s.key];
        if (paths == null) {
          return s;
        }
        return { ...s, enabled: paths > 0, paths: paths > 0 ? paths : s.paths };
      })
    );
  };

  const handleRun = async () => {
    const activeScenarios = scenarios.filter((s) => s.enabled);
    if (activeScenarios.length === 0) {
      setError(t("stockAnalysis.monte-carlo-panel.select-at-least-one-scenario"));
      return;
    }

    const myToken = ++tokenRef.current;
    setLoading(true);
    setError(null);
    setReport(null);

    try {
      const request: McRunRequest = {
        stockCode,
        referencePrice: refPrice,
        maxSimTimeNs: simMs * 1_000_000,
        scenarios: activeScenarios.map((s) => ({
          scenario: s.key,
          paths: s.paths,
        })),
      };

      const result = await invoke<RobustnessResult>("market_sim_run_mc", { request });
      if (myToken !== tokenRef.current) {
        return;
      }
      setReport(result);
    } catch (e: unknown) {
      if (myToken !== tokenRef.current) {
        return;
      }
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
    } finally {
      if (myToken === tokenRef.current) {
        setLoading(false);
      }
    }
  };

  const totalPaths = scenarios.filter((s) => s.enabled).reduce((sum, s) => sum + s.paths, 0);
  const refPriceYuanText = refPrice > 0 ? (refPrice / 100).toFixed(2) : null;
  const consistency = report?.consistencyScore ?? null;

  return (
    <div className="space-y-4">
      {/* 配置区 */}
      <Card size="small" title={t("stockAnalysis.monte-carlo-panel.robustness-test-config")}>
        <div className="mb-3 flex flex-wrap items-center gap-4">
          <label className="text-sm font-medium">
            {t("stockAnalysis.monte-carlo-panel.stock-code")}
            {/* 股票代码是标识符：用文本输入，避免 InputNumber 把 000001 读成 1 */}
            <Input
              className="ml-2"
              style={{ width: 110 }}
              maxLength={6}
              value={stockCode}
              onChange={(e) => setStockCode(e.target.value)}
            />
          </label>
          <label className="text-sm font-medium">
            {t("stockAnalysis.monte-carlo-panel.reference-price")}
            <InputNumber
              className="ml-2"
              style={{ width: 120 }}
              min={1}
              value={refPrice}
              onChange={(v) => setRefPrice(v ?? FALLBACK_REF_PRICE_FEN)}
            />
            {refPriceYuanText && (
              <span className="ml-2 text-xs text-secondary">
                {t("stockAnalysis.simulation.yuanEquivalent", { yuan: refPriceYuanText })}
              </span>
            )}
          </label>
          <label className="text-sm font-medium">
            {t("stockAnalysis.monte-carlo-panel.duration-ms")}
            <InputNumber
              className="ml-2"
              style={{ width: 100 }}
              min={1}
              max={1000}
              value={simMs}
              onChange={(v) => setSimMs(v ?? 50)}
            />
          </label>
        </div>

        <Divider style={{ margin: "8px 0" }} />

        {/* 场景预设：一键切换测试侧重，省去逐个勾选 */}
        <div className="mb-3 flex flex-wrap items-center gap-2">
          <span className="text-xs text-secondary">{t("stockAnalysis.monte-carlo-panel.preset-label")}</span>
          <Button size="small" onClick={() => applyPreset("quick")}>
            {t("stockAnalysis.monte-carlo-panel.preset-quick")}
          </Button>
          <Button size="small" onClick={() => applyPreset("stress")}>
            {t("stockAnalysis.monte-carlo-panel.preset-stress")}
          </Button>
          <Button size="small" onClick={() => applyPreset("deep")}>
            {t("stockAnalysis.monte-carlo-panel.preset-deep")}
          </Button>
        </div>

        <div className="mb-3 flex flex-wrap gap-4">
          {scenarios.map((sc) => (
            <div key={sc.key} className="flex items-center gap-2 rounded-lg border px-3 py-1.5">
              <Checkbox checked={sc.enabled} onChange={() => toggleScenario(sc.key)} />
              <span className="text-sm">{t(sc.label)}</span>
              <InputNumber
                size="small"
                style={{ width: 65 }}
                min={1}
                max={100}
                value={sc.paths}
                disabled={!sc.enabled}
                onChange={(v) => setPaths(sc.key, v ?? 10)}
              />
            </div>
          ))}
        </div>

        <div className="flex items-center justify-between">
          <span className="text-sm text-secondary">
            {t("stockAnalysis.monte-carlo-panel.total-summary", {
              totalPaths,
              simTime: ((totalPaths * simMs) / 1000).toFixed(1),
            })}
          </span>
          <Button type="primary" onClick={handleRun} loading={loading}>
            {loading
              ? t("stockAnalysis.monte-carlo-panel.running")
              : t("stockAnalysis.monte-carlo-panel.run-robustness-test")}
          </Button>
        </div>
      </Card>

      {/* 加载态 */}
      {loading && (
        <Card size="small">
          <div className="flex items-center justify-center py-8">
            <Spin
              size="large"
              description={t("stockAnalysis.monte-carlo-panel.running-simulation-tip", { totalPaths })}
            />
          </div>
        </Card>
      )}

      {/* 错误态 */}
      {error && (
        <Card size="small">
          <div className="py-4 text-center text-red">{error}</div>
        </Card>
      )}

      {/* 结果区 */}
      {report && !loading && (
        <>
          {/* 核心指标 */}
          <Row gutter={[12, 12]}>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.monte-carlo-panel.total-paths-stat")}
                  value={report.totalPaths}
                  suffix={t("stockAnalysis.monte-carlo-panel.paths-unit")}
                  styles={{ content: { fontSize: 22 } }}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.monte-carlo-panel.cross-scenario-survival-rate")}
                  value={report.survivalRate}
                  suffix="%"
                  precision={1}
                  styles={{ content: { fontSize: 22, color: report.survivalRate >= 50 ? "#52c41a" : "#f5222d" } }}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.monte-carlo-panel.consistency-score")}
                  value={consistency ?? "—"}
                  precision={consistency == null ? undefined : 2}
                  suffix={consistency == null
                    ? t("stockAnalysis.monte-carlo-panel.consistency-undetermined")
                    : consistency < 1.0
                    ? t("stockAnalysis.monte-carlo-panel.consistency-stable-suffix")
                    : t("stockAnalysis.monte-carlo-panel.consistency-volatile-suffix")}
                  styles={{ content: { fontSize: 22 } }}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card size="small" hoverable>
                <div className="text-sm text-secondary">{t("stockAnalysis.monte-carlo-panel.best-worst-scenario")}</div>
                <div className="mt-1">
                  <Tag color="green">{report.bestScenario}</Tag>
                  <Tag color="red">{report.worstScenario}</Tag>
                </div>
              </Card>
            </Col>
          </Row>

          {/* 场景详情表格 */}
          <Card
            size="small"
            title={
              <span>
                📊 {t("stockAnalysis.monte-carlo-panel.scenario-detail")} · <Tag color="blue">{report.stockCode}</Tag>
                {" "}
                {t("stockAnalysis.monte-carlo-panel.reference-price-label", { price: report.referencePrice })}
              </span>
            }
          >
            <Table
              dataSource={report.scenarioResults}
              rowKey="scenario"
              size="small"
              pagination={false}
              columns={[
                {
                  title: t("stockAnalysis.monte-carlo-panel.column-scenario"),
                  dataIndex: "label",
                  key: "label",
                  render: (label: string, record: McScenarioResult) => (
                    <span>
                      {label}
                      <Tag className="ml-2" color="default">{record.scenario}</Tag>
                    </span>
                  ),
                },
                {
                  title: t("stockAnalysis.monte-carlo-panel.column-paths"),
                  dataIndex: "paths",
                  key: "paths",
                  width: 80,
                },
                {
                  title: t("stockAnalysis.monte-carlo-panel.column-avg-trades"),
                  dataIndex: "avgTotalTrades",
                  key: "avgTotalTrades",
                  width: 100,
                  render: (v: number) => v.toFixed(1),
                },
                {
                  title: t("stockAnalysis.monte-carlo-panel.column-final-price"),
                  dataIndex: "avgFinalMidPrice",
                  key: "avgFinalMidPrice",
                  width: 120,
                  render: (v: number | null) => (v ?? "—"),
                },
                {
                  title: t("stockAnalysis.monte-carlo-panel.column-price-change"),
                  dataIndex: "priceChangePct",
                  key: "priceChangePct",
                  width: 100,
                  render: (v: number | null) => {
                    if (v == null) {
                      return "—";
                    }
                    // A 股配色约定：涨红跌绿
                    const color = v >= 0 ? "#f5222d" : "#52c41a";
                    return <span style={{ color }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
                  },
                },
              ]}
            />
          </Card>

          {/* 解读 */}
          <Card size="small" title={t("stockAnalysis.monte-carlo-panel.interpretation")}>
            <Descriptions column={1} size="small">
              <Descriptions.Item label={t("stockAnalysis.monte-carlo-panel.survival-rate-analysis")}>
                {report.survivalRate >= 70
                  ? t("stockAnalysis.monte-carlo-panel.survival-rate-high")
                  : report.survivalRate >= 40
                  ? t("stockAnalysis.monte-carlo-panel.survival-rate-medium")
                  : t("stockAnalysis.monte-carlo-panel.survival-rate-low")}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.monte-carlo-panel.consistency")}>
                {consistency == null
                  ? t("stockAnalysis.monte-carlo-panel.consistency-undetermined-desc")
                  : consistency < 0.5
                  ? t("stockAnalysis.monte-carlo-panel.consistency-high")
                  : consistency < 1.0
                  ? t("stockAnalysis.monte-carlo-panel.consistency-acceptable")
                  : t("stockAnalysis.monte-carlo-panel.consistency-environment-dependent")}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.monte-carlo-panel.advice")}>
                {report.bestScenario === report.worstScenario
                  ? t("stockAnalysis.monte-carlo-panel.advice-consistent")
                  : t("stockAnalysis.monte-carlo-panel.advice-different", {
                    best: report.bestScenario,
                    worst: report.worstScenario,
                  })}
              </Descriptions.Item>
            </Descriptions>
          </Card>
        </>
      )}

      {/* 初始提示 */}
      {!report && !loading && !error && (
        <Card size="small">
          <div className="py-8 text-center text-secondary">
            <p className="mb-2 text-base">{t("stockAnalysis.monte-carlo-panel.empty-state-title")}</p>
            <p className="text-sm">
              {t("stockAnalysis.monte-carlo-panel.empty-state-desc")}
            </p>
          </div>
        </Card>
      )}
    </div>
  );
}

// 辅助接口（Table 用）
interface McScenarioResult {
  scenario: string;
  label: string;
  paths: number;
  avgTotalTrades: number;
  avgFinalMidPrice: number | null;
  priceChangePct: number | null;
}
