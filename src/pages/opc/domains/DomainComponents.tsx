// i18n-exempt: 业务逻辑/API 描述/日志字符串，非 UI 展示文本
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 行业 UI 组件 — 可复用的行业展示组件
 */

import { translateBackendError } from "@/lib/errorI18n";
import {
  ApiOutlined,
  BarChartOutlined,
  BulbOutlined,
  DashboardOutlined,
  FundProjectionScreenOutlined,
  LineChartOutlined,
  PlayCircleOutlined,
  RocketOutlined,
  SyncOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import {
  Alert,
  App,
  Badge,
  Button,
  Card,
  Col,
  Divider,
  Empty,
  Progress,
  Row,
  Segmented,
  Space,
  Spin,
  Statistic,
  Steps,
  Tag,
  Timeline,
  Typography,
} from "antd";
import { useTranslation } from "react-i18next";
import type { KpiValue, RiskLevel } from "./types";
import { useDomainData } from "./useDomainData";

const { Text } = Typography;

/**
 * 风险等级 → 展示样式。
 *
 * 用 `Record<RiskLevel, …>` 而不是嵌套三元：后者一旦漏了新等级会**静默落进最后
 * 那个 `else` 分支**（历史上 `RiskLevel` 序列化成 PascalCase 时，风险提示就永远
 * 落在绿色 success 分支）。写成 Record 后，`RiskLevel` 新增取值会在**编译期**
 * 报缺键，而不是运行时悄悄降级。
 *
 * `critical` 与 `high` 同级（AntD `Alert` 最重只有 `error`），但数值色取更深的红
 * (`#a8071a` vs `#cf1322`) 以示区分。
 */
const RISK_ALERT_TYPE: Record<RiskLevel, "error" | "warning" | "success"> = {
  critical: "error",
  high: "error",
  medium: "warning",
  low: "success",
};

const RISK_COLOR: Record<RiskLevel, string> = {
  critical: "#a8071a",
  high: "#cf1322",
  medium: "#d48806",
  low: "#3f8600",
};

/**
 * 分区取数失败提示。
 *
 * 失败与「确实没有」在 UI 上必须分开：`Empty` 表达的是「后端答了，说没有」，
 * 取数失败时若也落进 `Empty`，就是把「拿不到」伪装成一条正常结论。
 * 有该提示在场时，各组件一律不再渲染 `Empty`。
 */
function SectionError({ error }: { error?: string | null }) {
  if (!error) {
    return null;
  }
  return <Alert type="error" showIcon title={error} style={{ marginBottom: 16 }} />;
}

/**
 * 单个 KPI 卡片 —— **仪表盘面板与决策面板共用同一个实现**。
 *
 * 三态只在这一处映射（两处各写一份必然会漂移，而漂移的后果是伪造数据）：
 * - `available` ⇒ 渲染真实数值 + 单位。**含「统计结果恰为 0」**（真实观测，该显示 0）；
 * - `empty` / `no_data_source` ⇒ **绝不渲染 `value`**。后端在这两态下把 `value` 填成
 *   占位 `0.0`（`domain_pack_kpi_service::resolve_kpi_value`），把它显示成 `0` 就是把
 *   「没采集」伪装成「观测值为 0」。这里固定渲染 `—` + 明确状态文案，文案复用仪表盘
 *   既有的两条 i18n key（11 个语言包已全覆盖，无需新增 key）。
 *
 * ⚠️ 不要用 `value === 0` 反推状态：真实观测恰好为 0 与「无数据」是两件事。
 *
 * 单位一律走 `suffix`：各行业 `config/opc/domain_packs/<domain_pack_id>/runtime.yaml` 的
 * `unit` 全量取值域是
 * `篇 / 次 / % / 字 / 轮 / 份 / 分 / 个 / 张 / CNY / 天 / 人 / 单 / 级 / 次每月 / 行`，
 * **没有前缀型单位**。曾按「货币前缀、其余后缀」分派，但本仓 unit 里连 `¥` 都不存在
 * （货币写成 `CNY`），而把 `%` 放进 `prefix` 会渲染成 `%0.00`（读作「百分之零」错位）。
 *
 * 小数位按值自适应：整数不带小数点（`25 篇` 而不是 `25.00 篇`），非整数保留 2 位
 * （比率类）。这与「三态」无关，不影响真实值的呈现。
 *
 * 本卡片只渲染后端 `KpiValue` 真实下发的字段。曾写过一个 `kpi.trend` 分支，但后端
 * 结构体（`opc::analytics::KpiValue`）根本没有该字段、全仓也无任何赋值点，属幽灵字段，
 * 已删除（连同 `types.ts` 的声明一并清理）。
 */
function KpiStatCard({ kpi }: { kpi: KpiValue }) {
  const { t } = useTranslation();
  const unavailable = kpi.availability !== "available";

  return (
    <Card size="small" className="h-full">
      <Statistic
        title={kpi.name || kpi.key}
        value={unavailable ? "—" : kpi.value}
        precision={unavailable ? undefined : Number.isInteger(kpi.value) ? 0 : 2}
        suffix={kpi.unit ?? ""}
      />
      {unavailable && (
        <Text type="secondary" style={{ fontSize: 12 }}>
          {kpi.availability === "no_data_source"
            ? t("opc.domain.dashboard.noDataSource")
            : t("opc.domain.dashboard.noDataYet")}
        </Text>
      )}
    </Card>
  );
}

export function DomainDashboard({
  dashboard,
  loading,
  error,
  kpiTimeRange,
  onTimeRangeChange,
  onRefresh,
}: {
  dashboard: ReturnType<typeof useDomainData>["dashboard"];
  loading: boolean;
  error?: string | null;
  kpiTimeRange: "7" | "30" | "90";
  onTimeRangeChange: (range: "7" | "30" | "90") => void;
  onRefresh: () => void;
}) {
  const { t } = useTranslation();

  return (
    <Card
      style={{ marginBottom: 24 }}
      title={
        <span>
          <DashboardOutlined style={{ marginRight: 8 }} />
          {t("opc.domain.dashboard.title")}
        </span>
      }
      extra={
        <Space>
          <Segmented
            value={kpiTimeRange}
            onChange={(v) => onTimeRangeChange(v as "7" | "30" | "90")}
            options={[
              { label: t("opc.domain.dashboard.7days"), value: "7" },
              { label: t("opc.domain.dashboard.30days"), value: "30" },
              { label: t("opc.domain.dashboard.90days"), value: "90" },
            ]}
          />
          <Button icon={<SyncOutlined spin={loading} />} onClick={onRefresh}>
            {t("opc.domain.refresh")}
          </Button>
        </Space>
      }
    >
      {loading
        ? (
          <div style={{ textAlign: "center", padding: 40 }}>
            <Spin />
          </div>
        )
        : dashboard && dashboard.kpis.length > 0
        ? (
          <>
            <SectionError error={error} />
            {dashboard.risk_level && dashboard.violations.length > 0 && (
              <Alert
                type={RISK_ALERT_TYPE[dashboard.risk_level]}
                showIcon
                style={{ marginBottom: 16 }}
                title={t("opc.domain.analysis.riskLevel") + ": " + dashboard.risk_level}
                description={
                  <ul style={{ margin: 0, paddingLeft: 20 }}>
                    {dashboard.violations.map((v) => <li key={v.rule}>{v.message}</li>)}
                  </ul>
                }
              />
            )}
            <Row gutter={[16, 16]} style={{ marginBottom: 16 }}>
              {dashboard.kpis.map((kpi) => (
                <Col xs={12} sm={8} md={6} key={kpi.id || kpi.key}>
                  <KpiStatCard kpi={kpi} />
                </Col>
              ))}
            </Row>
          </>
        )
        : (
          <>
            <SectionError error={error} />
            {!error && (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={t("opc.domain.dashboard.noData")}
              />
            )}
          </>
        )}
    </Card>
  );
}

/**
 * 行业工作流步骤组件
 */
export function DomainWorkflowSteps({
  steps,
  loading,
  error,
}: {
  steps: ReturnType<typeof useDomainData>["workflowSteps"];
  loading: boolean;
  error?: string | null;
}) {
  const { t } = useTranslation();

  return (
    <Card
      style={{ marginBottom: 24 }}
      title={
        <span>
          <LineChartOutlined style={{ marginRight: 8 }} />
          {t("opc.domain.workflowSteps.title")}
        </span>
      }
    >
      <SectionError error={error} />
      {loading
        ? (
          <div style={{ textAlign: "center", padding: 40 }}>
            <Spin />
          </div>
        )
        : steps.length > 0
        ? (
          <Steps
            orientation="vertical"
            current={-1}
            items={steps.map((step) => ({
              title: (
                <Space>
                  <Text strong>{step.name}</Text>
                  <Tag color="blue">
                    {t("opc.domain.workflowSteps.step")} {step.stepOrder}
                  </Tag>
                </Space>
              ),
              content: step.description,
            }))}
          />
        )
        : error
        ? null
        : (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={t("opc.domain.workflowSteps.noData")}
          />
        )}
    </Card>
  );
}

/**
 * 行业自动化规则组件
 */
export function DomainAutomationRules({
  rules,
  loading,
  error,
  running,
  onRunAll,
}: {
  rules: ReturnType<typeof useDomainData>["automationRules"];
  loading: boolean;
  error?: string | null;
  running: boolean;
  onRunAll: () => Promise<string[]>;
}) {
  const { t } = useTranslation();
  const { message } = App.useApp();

  const handleRunAll = async () => {
    try {
      const triggered = await onRunAll();
      if (triggered.length > 0) {
        message.success(t("opc.domain.rules.triggered", { count: triggered.length }));
      } else {
        message.info(t("opc.domain.rules.nothingTriggered"));
      }
    } catch (e) {
      // 「跑没跑成」与「跑完没命中」必须分开报 —— 前者用 `runFailed`，后者才是 nothingTriggered
      message.error(t("opc.domain.rules.runFailed", { error: translateBackendError(e) }));
    }
  };

  return (
    <Card
      style={{ marginBottom: 24 }}
      title={
        <span>
          <ThunderboltOutlined style={{ marginRight: 8 }} />
          {t("opc.domain.rules.title")}
        </span>
      }
      extra={
        <Button
          type="primary"
          size="small"
          icon={<PlayCircleOutlined />}
          loading={running}
          onClick={handleRunAll}
          disabled={rules.filter((r) => r.is_enabled).length === 0}
        >
          {t("opc.domain.rules.runAll")}
        </Button>
      }
    >
      <SectionError error={error} />
      {loading
        ? (
          <div style={{ textAlign: "center", padding: 40 }}>
            <Spin />
          </div>
        )
        : rules.length > 0
        ? (
          <Row gutter={[16, 16]}>
            {rules.map((rule) => (
              <Col xs={24} sm={12} md={8} key={rule.id}>
                <Card
                  size="small"
                  title={
                    <Space>
                      <Text strong>{rule.name}</Text>
                      <Badge
                        status={rule.is_enabled ? "success" : "default"}
                        text={rule.is_enabled
                          ? t("opc.domain.rules.enabled")
                          : t("opc.domain.rules.disabled")}
                      />
                    </Space>
                  }
                >
                  <div style={{ marginBottom: 8 }}>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t("opc.domain.rules.conditions")}:
                    </Text>
                    <div style={{ marginTop: 4 }}>
                      <Tag color="blue">{rule.trigger}</Tag>
                      {rule.conditions.map((c, i) => <Tag key={`c-${c.field}-${i}`}>{c.field} {c.operator}</Tag>)}
                      {rule.conditions.length === 0 && (
                        <Text type="secondary" style={{ fontSize: 12 }}>{t("opc.domain.rules.noConditions")}</Text>
                      )}
                    </div>
                  </div>
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {t("opc.domain.rules.actions")}:
                    </Text>
                    <div style={{ marginTop: 4 }}>
                      {rule.actions.map((a, i) => (
                        <Tag color="green" key={`a-${a.action_type}-${i}`}>{a.action_type}</Tag>
                      ))}
                      {rule.actions.length === 0 && (
                        <Text type="secondary" style={{ fontSize: 12 }}>{t("opc.domain.rules.noActions")}</Text>
                      )}
                    </div>
                  </div>
                </Card>
              </Col>
            ))}
          </Row>
        )
        : error
        ? null
        : (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={t("opc.domain.rules.noData")}
          />
        )}
    </Card>
  );
}

/**
 * 行业分析决策组件
 */
export function DomainAnalysisDecision({
  decision,
  loading,
  error,
  decisionDays,
  onDaysChange,
  onExecute,
}: {
  decision: ReturnType<typeof useDomainData>["decision"];
  loading: boolean;
  error?: string | null;
  decisionDays: number;
  onDaysChange: (days: number) => void;
  onExecute: () => Promise<void>;
}) {
  const { t } = useTranslation();

  return (
    <Card
      style={{ marginBottom: 24 }}
      title={
        <span>
          <BarChartOutlined style={{ marginRight: 8 }} />
          {t("opc.domain.analysis.title")}
        </span>
      }
      extra={
        <Space>
          <Segmented
            value={String(decisionDays)}
            onChange={(v) => onDaysChange(Number(v))}
            options={[
              { label: t("opc.domain.analysis.timeRange7d"), value: "7" },
              { label: t("opc.domain.analysis.timeRange30d"), value: "30" },
              { label: t("opc.domain.analysis.timeRange90d"), value: "90" },
            ]}
          />
          <Button
            type="primary"
            icon={<PlayCircleOutlined />}
            loading={loading}
            onClick={onExecute}
          >
            {t("opc.domain.analysis.execute")}
          </Button>
        </Space>
      }
    >
      {loading
        ? (
          <div style={{ textAlign: "center", padding: 40 }}>
            <Spin />
          </div>
        )
        : decision
        ? (
          <>
            <SectionError error={error} />
            <Alert
              type={RISK_ALERT_TYPE[decision.risk_level]}
              showIcon
              title={decision.summary}
              description={t("opc.domain.analysis.riskLevel") + ": " + decision.risk_level}
              style={{ marginBottom: 16 }}
            />
            {decision.kpis.length > 0 && (
              <Row gutter={[16, 16]} style={{ marginBottom: 16 }}>
                {decision.kpis.map((kpi) => (
                  <Col xs={12} sm={8} md={6} key={kpi.id || kpi.key}>
                    <KpiStatCard kpi={kpi} />
                  </Col>
                ))}
              </Row>
            )}
            <Row gutter={[16, 16]} style={{ marginBottom: 16 }}>
              <Col xs={12} sm={8}>
                <Card size="small">
                  <Progress
                    type="dashboard"
                    percent={Math.round(decision.confidence * 100)}
                    format={(p) => `${p}%`}
                  />
                  <div style={{ textAlign: "center", marginTop: 8 }}>
                    <Text type="secondary">{t("opc.domain.analysis.confidence")}</Text>
                  </div>
                </Card>
              </Col>
              <Col xs={12} sm={8}>
                <Card size="small">
                  <Statistic
                    title={t("opc.domain.analysis.decisionType")}
                    value={decision.decision_type}
                  />
                </Card>
              </Col>
              <Col xs={12} sm={8}>
                <Card size="small">
                  <Statistic
                    title={t("opc.domain.analysis.riskLevelTitle")}
                    value={decision.risk_level}
                    valueStyle={{ color: RISK_COLOR[decision.risk_level] }}
                  />
                </Card>
              </Col>
            </Row>
            {decision.recommendations.length > 0 && <Divider>{t("opc.domain.analysis.recommendations")}</Divider>}
            <Timeline
              items={decision.recommendations.map((rec) => ({
                children: <Text>{rec}</Text>,
              }))}
            />
          </>
        )
        : (
          <>
            <SectionError error={error} />
            {!error && (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={t("opc.domain.analysis.noData")}
              />
            )}
          </>
        )}
    </Card>
  );
}

/**
 * 行业学习指标组件
 */
export function DomainLearningMetrics({
  metrics,
  loading,
  error,
  onRefresh,
}: {
  metrics: ReturnType<typeof useDomainData>["learningMetrics"];
  loading: boolean;
  error?: string | null;
  onRefresh: () => Promise<void>;
}) {
  const { t } = useTranslation();

  return (
    <Card
      style={{ marginBottom: 24 }}
      title={
        <span>
          <FundProjectionScreenOutlined style={{ marginRight: 8 }} />
          {t("opc.domain.metrics.title")}
        </span>
      }
      extra={
        <Button icon={<SyncOutlined spin={loading} />} loading={loading} onClick={onRefresh}>
          {t("opc.domain.metrics.refresh")}
        </Button>
      }
    >
      <SectionError error={error} />
      {loading
        ? (
          <div style={{ textAlign: "center", padding: 40 }}>
            <Spin />
          </div>
        )
        : metrics
        ? (
          <Row gutter={[16, 16]}>
            <Col xs={12} sm={6}>
              <Card size="small">
                <Statistic
                  title={t("opc.domain.metrics.totalSamples")}
                  value={metrics.total_samples}
                  prefix={<BulbOutlined />}
                />
              </Card>
            </Col>
            <Col xs={12} sm={6}>
              <Card size="small" title={t("opc.domain.metrics.decisionAccuracy")}>
                <Progress
                  type="circle"
                  percent={Math.round(metrics.decision_accuracy * 100)}
                />
              </Card>
            </Col>
            <Col xs={12} sm={6}>
              <Card size="small" title={t("opc.domain.metrics.riskAccuracy")}>
                <Progress
                  type="circle"
                  percent={Math.round(metrics.risk_prediction_accuracy * 100)}
                />
              </Card>
            </Col>
            <Col xs={12} sm={6}>
              <Card size="small">
                <Statistic
                  title={t("opc.domain.metrics.avgFeedback")}
                  value={metrics.avg_feedback_score}
                  precision={2}
                  prefix={<BulbOutlined />}
                />
                <Tag
                  color={metrics.improvement_trend === "improving"
                    ? "green"
                    : metrics.improvement_trend === "stable"
                    ? "blue"
                    : metrics.improvement_trend === "declining"
                    ? "red"
                    : "default"}
                  style={{ marginTop: 8 }}
                >
                  {t(`opc.domain.metrics.trend_${metrics.improvement_trend}`)}
                </Tag>
              </Card>
            </Col>
          </Row>
        )
        : (
          <>
            {!error && (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={t("opc.domain.metrics.noData")}
              />
            )}
          </>
        )}
    </Card>
  );
}

/**
 * 学习与进化配置面板
 */
export function DomainLearningPanel({
  learningConfig,
  loading,
  error,
  onReflect,
  onEvolve,
  onSelfImprove,
}: {
  learningConfig: NonNullable<ReturnType<typeof useDomainData>["learningConfig"]> | null;
  loading: boolean;
  error?: string | null;
  onReflect: () => Promise<void>;
  onEvolve: () => Promise<void>;
  onSelfImprove: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const { message } = App.useApp();

  if (!learningConfig) {
    return (
      <Card
        title={
          <span>
            <ApiOutlined style={{ marginRight: 8 }} />
            {t("opc.domain.learning.title")}
          </span>
        }
      >
        <SectionError error={error} />
        {loading
          ? (
            <div style={{ textAlign: "center", padding: 40 }}>
              <Spin />
            </div>
          )
          : !error && (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("opc.domain.learning.actions.configNotFound")}
            />
          )}
      </Card>
    );
  }

  return (
    <Card
      title={
        <span>
          <ApiOutlined style={{ marginRight: 8 }} />
          {t("opc.domain.learning.title")}
        </span>
      }
    >
      <Row gutter={[16, 16]}>
        {/* 反思 */}
        <Col xs={24} sm={12} md={6}>
          <Card size="small" style={{ height: "100%" }}>
            <Space orientation="vertical" size={8} style={{ width: "100%" }}>
              <Space>
                <BulbOutlined />
                <strong>{t("opc.domain.learning.reflection.label")}</strong>
                <Tag color={learningConfig.reflectionEnabled ? "green" : "default"}>
                  {learningConfig.reflectionEnabled
                    ? t("opc.domain.learning.reflection.enabled")
                    : t("opc.domain.learning.reflection.disabled")}
                </Tag>
              </Space>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("opc.domain.learning.reflection.description")}
              </Text>
              <Button
                size="small"
                icon={<BulbOutlined />}
                onClick={async () => {
                  if (!learningConfig.reflectionEnabled) {
                    message.warning(t("opc.domain.learning.reflection.notEnabled"));
                    return;
                  }
                  await onReflect();
                }}
                disabled={!learningConfig.reflectionEnabled}
                block
              >
                {t("opc.domain.learning.reflection.trigger")}
              </Button>
            </Space>
          </Card>
        </Col>

        {/* 进化 */}
        <Col xs={24} sm={12} md={6}>
          <Card size="small" style={{ height: "100%" }}>
            <Space orientation="vertical" size={8} style={{ width: "100%" }}>
              <Space>
                <ThunderboltOutlined />
                <strong>{t("opc.domain.learning.evolution.label")}</strong>
                <Tag color={learningConfig.evolutionEnabled ? "green" : "default"}>
                  {learningConfig.evolutionEnabled
                    ? t("opc.domain.learning.evolution.enabled")
                    : t("opc.domain.learning.evolution.disabled")}
                </Tag>
              </Space>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("opc.domain.learning.evolution.description")}
              </Text>
              <Button
                size="small"
                icon={<RocketOutlined />}
                onClick={async () => {
                  if (!learningConfig.evolutionEnabled) {
                    message.warning(t("opc.domain.learning.evolution.notEnabled"));
                    return;
                  }
                  await onEvolve();
                }}
                disabled={!learningConfig.evolutionEnabled}
                block
              >
                {t("opc.domain.learning.evolution.trigger")}
              </Button>
            </Space>
          </Card>
        </Col>

        {/* 自我改进 */}
        <Col xs={24} sm={12} md={6}>
          <Card size="small" style={{ height: "100%" }}>
            <Space orientation="vertical" size={8} style={{ width: "100%" }}>
              <Space>
                <PlayCircleOutlined />
                <strong>{t("opc.domain.learning.selfImprovement.label")}</strong>
                <Tag color={learningConfig.selfImprovementEnabled ? "green" : "default"}>
                  {learningConfig.selfImprovementEnabled
                    ? t("opc.domain.learning.selfImprovement.enabled")
                    : t("opc.domain.learning.selfImprovement.disabled")}
                </Tag>
              </Space>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("opc.domain.learning.selfImprovement.description")}
              </Text>
              <Button
                size="small"
                icon={<PlayCircleOutlined />}
                onClick={async () => {
                  if (!learningConfig.selfImprovementEnabled) {
                    message.warning(t("opc.domain.learning.selfImprovement.notEnabled"));
                    return;
                  }
                  await onSelfImprove();
                }}
                disabled={!learningConfig.selfImprovementEnabled}
                block
              >
                {t("opc.domain.learning.selfImprovement.trigger")}
              </Button>
            </Space>
          </Card>
        </Col>

        {/* 强化学习 */}
        <Col xs={24} sm={12} md={6}>
          <Card size="small" style={{ height: "100%" }}>
            <Space orientation="vertical" size={8} style={{ width: "100%" }}>
              <Space>
                <FundProjectionScreenOutlined />
                <strong>{t("opc.domain.learning.reinforcementLearning.label")}</strong>
                <Tag color={learningConfig.reinforcementLearningEnabled ? "green" : "default"}>
                  {learningConfig.reinforcementLearningEnabled
                    ? t("opc.domain.learning.reinforcementLearning.enabled")
                    : t("opc.domain.learning.reinforcementLearning.disabled")}
                </Tag>
              </Space>
              <Text type="secondary" style={{ fontSize: 12 }}>
                {t("opc.domain.learning.reinforcementLearning.description")}
              </Text>
            </Space>
          </Card>
        </Col>
      </Row>
    </Card>
  );
}
