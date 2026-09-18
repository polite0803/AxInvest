import { invoke } from "@/lib/invoke";
import type { SimRunRequest, SimRunResult } from "@/types/market-sim";
import {
  Button,
  Card,
  Col,
  Descriptions,
  Divider,
  Form,
  Input,
  InputNumber,
  Row,
  Space,
  Spin,
  Statistic,
  Tag,
} from "antd";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

interface MarketSimPanelProps {
  /** 当前分析标的代码 —— 作为面板默认值，用户仍可覆盖 */
  stockCode?: string;
  /** 当前价（元）—— 面板内部换算为「分」提交后端 */
  referencePriceYuan?: number | null;
}

/** 无上下文时的兜底标的（用户未选股票时） */
const FALLBACK_STOCK_CODE = "000001";
/** 无上下文时的兜底参考价（1000 分 = 10.00 元） */
const FALLBACK_REF_PRICE_FEN = 1000;

/**
 * MarketSimPanel — ABIDES-inspired 多 Agent 市场模拟面板。
 *
 * 用户可配置模拟参数，运行多 Agent DES 仿真并查看**市场级**统计结果。
 * 作为股票分析页「模拟仿真」标签的子面板，标的默认取自当前分析上下文。
 *
 * ⚠️ 当前仿真市场中**不含「你自己的订单」**（`build_default_agents` 只有
 * 做市商/动量/价值/噪声），因此结果描述的是市场微观结构，不是本笔交易的
 * 冲击成本 —— 后者需要给内核加 order 注入入口（见前置 B）。
 */
export function MarketSimPanel({ stockCode, referencePriceYuan }: MarketSimPanelProps = {}) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SimRunResult | null>(null);
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const tokenRef = useRef(0);

  const defaultStockCode = stockCode?.trim() || FALLBACK_STOCK_CODE;
  const defaultRefPrice = referencePriceYuan != null && referencePriceYuan > 0
    ? Math.round(referencePriceYuan * 100)
    : FALLBACK_REF_PRICE_FEN;

  // 上下文（当前分析标的 / 现价）可能在挂载后才到（行情异步加载），
  // 且 antd `initialValues` 不会随 props 更新 ⇒ 必须显式同步。
  useEffect(() => {
    form.setFieldsValue({ stockCode: defaultStockCode, referencePrice: defaultRefPrice });
  }, [defaultStockCode, defaultRefPrice, form]);

  // 参考价以「分」为提交单位，用户看到的是分，容易误读 ⇒ 同时回显「元」
  const refPriceFen = Form.useWatch("referencePrice", form);
  const refPriceYuanText = typeof refPriceFen === "number" && refPriceFen > 0
    ? (refPriceFen / 100).toFixed(2)
    : null;

  const handleRun = async () => {
    const values = await form.validateFields();
    const myToken = ++tokenRef.current;
    setLoading(true);
    setError(null);
    setResult(null);

    try {
      const request: SimRunRequest = {
        stockCode: String(values.stockCode ?? FALLBACK_STOCK_CODE).trim(),
        referencePrice: values.referencePrice ?? FALLBACK_REF_PRICE_FEN,
        maxSimTimeNs: (values.maxSimTimeMs ?? 50) * 1_000_000,
        agentConfig: {
          marketMakers: values.marketMakers ?? 1,
          momentumAgents: values.momentumAgents ?? 1,
          valueAgents: values.valueAgents ?? 1,
          noiseAgents: values.noiseAgents ?? 2,
        },
      };

      const res = await invoke<SimRunResult>("market_sim_run", { request });
      if (myToken !== tokenRef.current) {
        return;
      }
      setResult(res);
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

  return (
    <div className="space-y-4">
      {/* 配置区 */}
      <Card
        size="small"
        title={t("stockAnalysis.marketSimPanel.simConfig")}
        className="[&_.ant-card-head-title]:flex [&_.ant-card-head-title]:items-center"
      >
        <Form
          form={form}
          layout="inline"
          initialValues={{
            stockCode: defaultStockCode,
            referencePrice: defaultRefPrice,
            maxSimTimeMs: 50,
            marketMakers: 1,
            momentumAgents: 1,
            valueAgents: 1,
            noiseAgents: 2,
          }}
          style={{ flexWrap: "wrap", gap: 12 }}
        >
          <Form.Item label={t("stockAnalysis.marketSimPanel.stockCode")} name="stockCode" rules={[{ required: true }]}>
            {/* 股票代码是标识符而非数量：用文本输入，避免 antd InputNumber 把 000001 读成 1 */}
            <Input style={{ width: 110 }} maxLength={6} placeholder="600519" />
          </Form.Item>
          <Form.Item
            label={t("stockAnalysis.marketSimPanel.referencePrice")}
            name="referencePrice"
            rules={[{ required: true }]}
            extra={refPriceYuanText
              ? t("stockAnalysis.simulation.yuanEquivalent", { yuan: refPriceYuanText })
              : undefined}
          >
            <InputNumber style={{ width: 120 }} min={1} />
          </Form.Item>
          <Form.Item
            label={t("stockAnalysis.marketSimPanel.simDuration")}
            name="maxSimTimeMs"
            rules={[{ required: true }]}
          >
            <InputNumber style={{ width: 120 }} min={1} max={1000} />
          </Form.Item>
          <Divider style={{ margin: "8px 0" }} />
          <Form.Item label={t("stockAnalysis.marketSimPanel.marketMaker")} name="marketMakers">
            <InputNumber style={{ width: 80 }} min={0} max={5} />
          </Form.Item>
          <Form.Item label={t("stockAnalysis.marketSimPanel.momentum")} name="momentumAgents">
            <InputNumber style={{ width: 80 }} min={0} max={5} />
          </Form.Item>
          <Form.Item label={t("stockAnalysis.marketSimPanel.value")} name="valueAgents">
            <InputNumber style={{ width: 80 }} min={0} max={5} />
          </Form.Item>
          <Form.Item label={t("stockAnalysis.marketSimPanel.noise")} name="noiseAgents">
            <InputNumber style={{ width: 80 }} min={0} max={10} />
          </Form.Item>
          <Form.Item>
            <Button type="primary" onClick={handleRun} loading={loading}>
              {loading ? t("stockAnalysis.marketSimPanel.simulating") : t("stockAnalysis.marketSimPanel.runSimulation")}
            </Button>
          </Form.Item>
        </Form>
      </Card>

      {/* 结果区 */}
      {loading && (
        <Card size="small">
          <div className="flex items-center justify-center py-8">
            <Space orientation="vertical" align="center">
              <Spin size="large" />
              <span className="text-secondary text-sm">{t("stockAnalysis.marketSimPanel.desRunning")}</span>
            </Space>
          </div>
        </Card>
      )}

      {error && (
        <Card size="small">
          <div className="py-4 text-center">
            <span className="text-red">{error}</span>
          </div>
        </Card>
      )}

      {result && !loading && (
        <>
          {/* 核心指标 */}
          <Row gutter={[12, 12]}>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.marketSimPanel.totalEvents")}
                  value={result.totalEvents}
                  suffix={t("stockAnalysis.marketSimPanel.eventsSuffix")}
                  styles={{ content: { fontSize: 22 } }}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.marketSimPanel.totalTrades")}
                  value={result.stats.totalTrades}
                  suffix={t("stockAnalysis.marketSimPanel.tradesSuffix")}
                  styles={{ content: { fontSize: 22 } }}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.marketSimPanel.wallClock")}
                  value={result.wallClockMs}
                  suffix="ms"
                  styles={{ content: { fontSize: 22 } }}
                />
              </Card>
            </Col>
            <Col span={6}>
              <Card size="small" hoverable>
                <Statistic
                  title={t("stockAnalysis.marketSimPanel.finalMidPrice")}
                  value={result.finalMidPrice ?? "—"}
                  suffix={result.finalMidPrice ? t("stockAnalysis.marketSimPanel.fenSuffix") : ""}
                  styles={{ content: { fontSize: 22 } }}
                />
              </Card>
            </Col>
          </Row>

          {/* 详细统计 */}
          <Card
            size="small"
            title={
              <span>
                {t("stockAnalysis.marketSimPanel.simDetails")}{" "}
                <Tag color="blue" style={{ marginRight: 0 }}>
                  {result.stockCode}
                </Tag>
              </span>
            }
          >
            <Descriptions column={3} size="small" bordered>
              <Descriptions.Item label={t("stockAnalysis.marketSimPanel.simTimeVirtual")}>
                {(result.simTimeNs / 1_000_000).toFixed(2)} ms
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.marketSimPanel.agentCount")}>
                {result.agentCount}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.marketSimPanel.refPrice")}>
                {result.referencePrice} {t("stockAnalysis.marketSimPanel.fenUnit")}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.marketSimPanel.maxQueueDepth")}>
                {result.stats.maxQueueDepth}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.marketSimPanel.totalOrders")}>
                {result.stats.totalOrders}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.marketSimPanel.totalTradesLabel")}>
                {result.stats.totalTrades > 0
                  ? `${result.stats.totalTrades} ${t("stockAnalysis.marketSimPanel.tradesUnit")}`
                  : "0"}
              </Descriptions.Item>
            </Descriptions>
          </Card>
        </>
      )}

      {/* 首次进入提示 */}
      {!result && !loading && !error && (
        <Card size="small">
          <div className="py-8 text-center text-secondary">
            <p className="mb-2 text-base">{t("stockAnalysis.marketSimPanel.emptyHint")}</p>
            <p className="text-sm">
              {t("stockAnalysis.marketSimPanel.emptyDesc")}
            </p>
          </div>
        </Card>
      )}
    </div>
  );
}
