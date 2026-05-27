import type { Variable, WorkflowTemplateInput, WorkflowTemplateResponse } from "@/components/workflow/types";
import { invoke } from "@/lib/invoke";
import { Button, Input, InputNumber, message, Select, Slider, Space, Switch, Tag, theme } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "./SettingsGroup";

const TEMPLATE_ID = "stock-analysis";

/** 生成默认参数变量列表（首次初始化） */
function getDefaultVariables(): Variable[] {
  const vars: Variable[] = [];
  const b = (name: string, val: unknown, desc: string, type: string) =>
    vars.push({ name, var_type: type, value: val, description: desc, is_secret: false });
  // 分析
  b("analysis_maxDebateRounds", 3, "辩论轮数", "number");
  b("analysis_klinePeriod", "daily", "K线周期: daily/weekly/monthly", "string");
  b("analysis_klineLimit", 120, "K线数量 (60-500)", "number");
  b("analysis_newsLimit", 30, "新闻数量 (10-100)", "number");
  b("analysis_maxConcurrent", 9, "并行分析数 (1-20)", "number");
  b("analysis_temperature", 0.3, "LLM 温度 (0-2)", "number");
  b("analysis_maxTokens", 4096, "LLM Max Tokens", "number");
  b("analysis_timeoutSecs", 300, "LLM 超时 (秒)", "number");
  // 评分权重
  b("scoring_trend", 30, "趋势评分权重 (0-100)", "number");
  b("scoring_deviation", 20, "乖离率评分权重 (0-100)", "number");
  b("scoring_macd", 15, "MACD 评分权重 (0-100)", "number");
  b("scoring_volume", 15, "量能评分权重 (0-100)", "number");
  b("scoring_rsi", 10, "RSI 评分权重 (0-100)", "number");
  b("scoring_support", 10, "支撑评分权重 (0-100)", "number");
  // 规则
  b("rule_rsiOverbought", 80, "RSI 超买阈值 (50-100)", "number");
  b("rule_biasLimit", 5, "乖离率追高阈值 (%)", "number");
  b("rule_volumeSignalBlock", true, "放量下跌时禁止买入", "boolean");
  b("rule_bearLowScore", 30, "空头+低分禁买阈值", "number");
  b("rule_rsiOversold", 20, "RSI 超卖提醒阈值", "number");
  b("rule_autoStopLossPct", 5, "自动止损百分比 (%)", "number");
  // 仓位
  b("pos_maxSingleStockPct", 20, "单股最大仓位 (%)", "number");
  b("pos_maxTotalPositions", 10, "最大持仓数量", "number");
  b("pos_maxSectorExposurePct", 40, "单一行业最大暴露 (%)", "number");
  // 估值
  b("value_dcfGrowthRate", 8, "DCF 增长率 (%)", "number");
  b("value_dcfPerpetualRate", 3, "DCF 永续增长率 (%)", "number");
  b("value_dcfDiscountRate", 10, "DCF 折现率 (%)", "number");
  b("value_moatThreshold", 60, "宽护城河阈值 (30-90)", "number");
  b("value_fScoreBuyThreshold", 7, "F-Score 买入阈值 (3-9)", "number");
  b("value_safetyMarginMin", 20, "最低安全边际 (%)", "number");
  // 监控
  b("monitor_pollIntervalSecs", 30, "监控轮询间隔 (秒)", "number");
  b("monitor_changePctThreshold", 5, "涨跌幅异常阈值 (%)", "number");
  b("monitor_turnoverThreshold", 10, "换手率异常阈值 (%)", "number");
  b("var_confidence", 95, "VaR 置信度 (%)", "number");
  b("kelly_fraction", 0.25, "凯利公式下注比例", "number");
  b("risk_free_rate", 2.0, "无风险利率 (%)", "number");
  b("outlier_method", "zscore", "异常值处理方法: zscore/iqr", "string");
  b("outlier_threshold", 3.0, "异常值阈值 (Z-Score)", "number");
  b("min_confidence", 0.6, "最低置信度", "number");
  return vars;
}

function parseEnumOptions(desc?: string): string[] {
  if (!desc) { return []; }
  const match = desc.match(/: (.+)/);
  if (match) { return match[1].split(/\s*\/\s*/).map((s) => s.trim()); }
  return [];
}

function inferStep(v: Variable): number {
  if (v.description?.includes("温度")) { return 0.1; }
  return 1;
}

// eslint-disable-next-line @typescript-eslint/no-empty-interface
interface Props {}

/** number 控件 — 窄屏竖排，宽屏横排 */
function NumberControl({ v, value, onChange }: {
  v: Variable;
  value: unknown;
  onChange: (name: string, val: unknown) => void;
}) {
  const hasPct = v.description?.includes("%") ?? false;
  const val = Number(value ?? 0);
  return (
    <span className="sacp-number">
      <Slider
        min={0}
        max={v.description?.includes("温度") ? 2 : 100}
        step={inferStep(v)}
        className="sacp-number-slider"
        value={val}
        onChange={(v2) => onChange(v.name, v2)}
      />
      <InputNumber
        size="small"
        className="sacp-number-input"
        value={val}
        suffix={hasPct ? "%" : undefined}
        onChange={(v2) => v2 != null && onChange(v.name, v2)}
      />
    </span>
  );
}

function VariableControl({ v, value, onChange }: {
  v: Variable;
  value: unknown;
  onChange: (name: string, val: unknown) => void;
}) {
  switch (v.var_type) {
    case "boolean":
      return <Switch checked={!!value} onChange={(c) => onChange(v.name, c)} />;
    case "enum": {
      const options = parseEnumOptions(v.description);
      return (
        <Select
          size="small"
          style={{ width: 140 }}
          value={String(value ?? "")}
          onChange={(val) => onChange(v.name, val)}
          options={options.map((o) => ({ value: o, label: o }))}
        />
      );
    }
    case "number":
      return <NumberControl v={v} value={value} onChange={onChange} />;
    default:
      return (
        <Input
          size="small"
          style={{ maxWidth: 180 }}
          value={String(value ?? "")}
          onChange={(e) => onChange(v.name, e.target.value)}
        />
      );
  }
}

export function StockAnalysisConfigPanel(_props: Props) {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [template, setTemplate] = useState<WorkflowTemplateResponse | null>(null);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    invoke<WorkflowTemplateResponse | null>("get_workflow_template", { id: TEMPLATE_ID })
      .then(async (rsp) => {
        if (rsp && (!rsp.variables || rsp.variables.length === 0)) {
          // 首次加载时，若模板无 variables，用默认值初始化并保存回模板
          const defaults = getDefaultVariables();
          const input: WorkflowTemplateInput = {
            name: rsp.name,
            description: rsp.description,
            icon: rsp.icon,
            tags: rsp.tags,
            trigger_config: rsp.trigger_config,
            nodes: rsp.nodes,
            edges: rsp.edges,
            input_schema: rsp.input_schema,
            output_schema: rsp.output_schema,
            variables: defaults,
            error_config: rsp.error_config,
          };
          invoke<boolean>("update_workflow_template", { id: TEMPLATE_ID, input }).catch(() => {});
          rsp.variables = defaults;
        }
        if (rsp) {
          setTemplate(rsp);
          const map: Record<string, unknown> = {};
          for (const v of rsp.variables) { map[v.name] = v.value; }
          setValues(map);
        } else {
          // 模板不存在时（浏览器模式），直接用默认变量渲染
          const defaults = getDefaultVariables();
          const map: Record<string, unknown> = {};
          for (const v of defaults) { map[v.name] = v.value; }
          setValues(map);
        }
      })
      .catch(() => message.error(t("stockAnalysis.settings.loadFailed")))
      .finally(() => setLoading(false));
  }, [t]);

  // 工具 → 参数配对
  const toolGroups = useMemo(() => {
    const allVars = template?.variables ?? getDefaultVariables();
    const varMap: Record<string, Variable> = {};
    for (const v of allVars) { varMap[v.name] = v; }

    const resolve = (names: string[]) => names.map((n) => varMap[n]).filter(Boolean);

    return [
      {
        tool: "compute_scoring",
        label: t("stockAnalysis.settings.group.scoring"),
        vars: resolve([
          "scoring_trend",
          "scoring_deviation",
          "scoring_macd",
          "scoring_volume",
          "scoring_rsi",
          "scoring_support",
        ]),
      },
      {
        tool: "compute_valuation",
        label: t("stockAnalysis.settings.group.value"),
        vars: resolve([
          "value_dcfGrowthRate",
          "value_dcfPerpetualRate",
          "value_dcfDiscountRate",
          "value_moatThreshold",
          "value_fScoreBuyThreshold",
          "value_safetyMarginMin",
        ]),
      },
      {
        tool: "compute_portfolio_risk",
        label: t("stockAnalysis.settings.group.pos"),
        vars: resolve(["pos_maxSingleStockPct", "pos_maxTotalPositions", "pos_maxSectorExposurePct"]),
      },
      {
        tool: "calcs",
        label: t("stockAnalysis.settings.group.riskModel"),
        vars: resolve([
          "var_confidence",
          "kelly_fraction",
          "risk_free_rate",
          "outlier_method",
          "outlier_threshold",
          "min_confidence",
        ]),
      },
      {
        tool: "rules",
        label: t("stockAnalysis.settings.group.rule"),
        vars: resolve([
          "rule_rsiOverbought",
          "rule_rsiOversold",
          "rule_biasLimit",
          "rule_volumeSignalBlock",
          "rule_bearLowScore",
          "rule_autoStopLossPct",
        ]),
      },
      {
        tool: "agent_executor",
        label: t("stockAnalysis.settings.group.agentRuntime"),
        vars: resolve([
          "analysis_temperature",
          "analysis_maxTokens",
          "analysis_timeoutSecs",
          "analysis_maxDebateRounds",
          "analysis_maxConcurrent",
        ]),
      },
      {
        tool: "tool_executor",
        label: t("stockAnalysis.settings.group.toolRuntime"),
        vars: resolve([
          "tool_timeoutSecs",
          "tool_retryMax",
          "analysis_klinePeriod",
          "analysis_klineLimit",
          "analysis_newsLimit",
        ]),
      },
      {
        tool: "monitor",
        label: t("stockAnalysis.settings.group.monitor"),
        vars: resolve(["monitor_pollIntervalSecs", "monitor_changePctThreshold", "monitor_turnoverThreshold"]),
      },
    ].filter((g) => g.vars.length > 0);
  }, [template]);

  const handleChange = (name: string, val: unknown) => {
    setValues((prev) => ({ ...prev, [name]: val }));
  };

  const handleSave = async () => {
    if (!template) { return; }
    setSaving(true);
    const updatedVars = template.variables.map((v) => ({ ...v, value: values[v.name] ?? v.value }));
    const input: WorkflowTemplateInput = {
      name: template.name,
      description: template.description,
      icon: template.icon,
      tags: template.tags,
      trigger_config: template.trigger_config,
      nodes: template.nodes,
      edges: template.edges,
      input_schema: template.input_schema,
      output_schema: template.output_schema,
      variables: updatedVars,
      error_config: template.error_config,
    };
    try {
      await invoke<boolean>("update_workflow_template", { id: TEMPLATE_ID, input });
      message.success(t("stockAnalysis.settings.saveSuccess"));
    } catch {
      message.error(t("stockAnalysis.settings.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div style={{ textAlign: "center", padding: 24, color: token.colorTextQuaternary }}>{t("common.loading")}</div>
    );
  }

  const rowStyle = { padding: "4px 0" };

  const handleOptimize = async () => {
    setSaving(true);
    try {
      const weights = await invoke<any>("optimize_scoring_weights");
      if (weights) {
        const map: Record<string, number> = {
          scoring_trend: weights.trendWeight,
          scoring_deviation: weights.deviationWeight,
          scoring_macd: weights.macdWeight,
          scoring_volume: weights.volumeWeight,
          scoring_rsi: weights.rsiWeight,
          scoring_support: weights.supportWeight,
        };
        setValues((prev) => ({ ...prev, ...map }));
        message.success(t("stockAnalysis.settings.optimize.success"));
      }
    } catch {
      message.error(t("stockAnalysis.settings.optimize.failed"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex justify-end">
        <Button size="small" loading={saving} onClick={handleOptimize}>
          {t("stockAnalysis.settings.optimize.btn")}
        </Button>
      </div>
      {toolGroups.map((g) => (
        <SettingsGroup
          key={g.tool}
          title={
            <Space size={4}>
              <span>{g.label}</span>
              <Tag className="text-xs m-0" color="default">⚙️ {g.tool}</Tag>
            </Space>
          }
        >
          <div className="sacp-vars">
            {g.vars.map((v) => (
              <div key={v.name} style={rowStyle} className="flex items-center justify-between sacp-row">
                <span className="sacp-var-label" style={{ fontSize: 13, color: token.colorText }}>
                  {v.description ?? v.name}
                </span>
                <span style={{ display: "inline-flex", alignItems: "center", gap: 8, flexShrink: 0, marginLeft: 16 }}>
                  <VariableControl v={v} value={values[v.name]} onChange={handleChange} />
                </span>
              </div>
            ))}
          </div>
        </SettingsGroup>
      ))}
      <div style={{ display: "flex", justifyContent: "flex-end", paddingTop: 8 }}>
        <Button type="primary" loading={saving} onClick={handleSave}>
          {t("stockAnalysis.settings.saveConfig")}
        </Button>
      </div>
    </div>
  );
}
