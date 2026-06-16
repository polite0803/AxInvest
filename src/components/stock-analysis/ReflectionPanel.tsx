import { invoke } from "@/lib/invoke";
import { Button, Card, Checkbox, Empty, Input, message, Select, Space, Switch, Table, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

interface ReflectionRow {
  id: string;
  stockCode: string;
  stockName: string;
  asOfDate: string;
  hindsightDate: string;
  actualOutcome: string;
  whatWentWrong: string | null;
  missedSignals: string | null;
  fixForFuture: string | null;
  reflectionDepth: string;
  minConfidenceThreshold: number;
  status: string;
  createdAt: number;
  decisionJson?: string;
}

interface CronJobResponse {
  id: string;
  name: string;
  schedule: string;
  status: string;
}

export function ReflectionPanel() {
  const { t } = useTranslation();

  const CRON_PRESETS = [
    { label: t("stockAnalysis.reflection.daily0600"), value: "0 6 * * *" },
    { label: t("stockAnalysis.reflection.daily1200"), value: "0 12 * * *" },
    { label: t("stockAnalysis.reflection.daily1800"), value: "0 18 * * *" },
    { label: t("stockAnalysis.reflection.weekday0600"), value: "0 6 * * 1-5" },
  ];

  const [reflections, setReflections] = useState<ReflectionRow[]>([]);
  const [cronJobs, setCronJobs] = useState<CronJobResponse[]>([]);
  const [cronExpr, setCronExpr] = useState("0 6 * * *");
  const [threshold, setThreshold] = useState(0);
  const [depth, setDepth] = useState("light");
  const [expandedId, setExpandedId] = useState<string | null>(null);

  // 手动触发参数
  const [manualCode, setManualCode] = useState("");
  const [manualAsOf, setManualAsOf] = useState("");
  const [manualOutcome, setManualOutcome] = useState("");
  const [manualDepth, setManualDepth] = useState("light");
  const [running, setRunning] = useState(false);

  const load = async () => {
    try {
      const [r, c] = await Promise.all([
        invoke<ReflectionRow[]>("list_reflections", {}),
        invoke<CronJobResponse[]>("list_validate_decisions_crons", {}),
      ]);
      if (Array.isArray(r)) { setReflections(r); }
      if (Array.isArray(c)) { setCronJobs(c); }
    } catch { /* ignore */ }
  };

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      invoke<ReflectionRow[]>("list_reflections", {}),
      invoke<CronJobResponse[]>("list_validate_decisions_crons", {}),
    ])
      .then(([r, c]) => {
        if (cancelled) { return; }
        if (Array.isArray(r)) { setReflections(r); }
        if (Array.isArray(c)) { setCronJobs(c); }
      })
      .catch(() => {/* ignore */});
    return () => {
      cancelled = true;
    };
  }, []);

  // 后端 CronJobStatus 用 #[serde(rename_all = "snake_case")] 序列化,
  // 实际 status 字符串是 "active" / "paused" / "disabled"（小写）
  // 这里必须用小写比较,否则 isEnabled/activeCron 永远错误。
  const activeCron = cronJobs.find((j) => j.status === "active") ?? cronJobs[0];
  const isEnabled = cronJobs.some((j) => j.status === "active");

  const toggleCron = async (enable: boolean) => {
    try {
      if (enable && !activeCron) {
        await invoke("create_validate_decisions_cron", {
          cronExpression: cronExpr,
          minConfidenceThreshold: threshold,
          reflectionDepth: depth,
          enabled: true,
        });
      } else if (activeCron) {
        await invoke("toggle_validate_decisions_cron", { id: activeCron.id, enabled: enable });
      }
      await load();
    } catch { /* ignore */ }
  };

  const deleteCron = async () => {
    if (!activeCron) { return; }
    try {
      await invoke("delete_validate_decisions_cron", { id: activeCron.id });
      await load();
    } catch { /* ignore */ }
  };

  const handleManualReflection = async () => {
    if (!manualCode.trim() || !manualAsOf.trim() || !manualOutcome.trim()) {
      message.warning(t("stockAnalysis.reflection.fillRequired"));
      return;
    }
    setRunning(true);
    try {
      await invoke("run_reflection_now", {
        stockCode: manualCode.trim(),
        stockName: "",
        asOfDate: manualAsOf.trim(),
        actualOutcome: manualOutcome.trim(),
        reflectionDepth: manualDepth,
      });
      message.success(t("stockAnalysis.reflection.triggerSuccess"));
      await load();
    } catch (e) {
      message.error(t("stockAnalysis.reflection.triggerFailed", { error: String(e) }));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div style={{ padding: 16 }}>
      {/* 手动触发 */}
      <Card
        title={t("stockAnalysis.reflection.manualTitle")}
        size="small"
        style={{ marginBottom: 16 }}
      >
        <Space direction="vertical" style={{ width: "100%" }}>
          <Space wrap>
            <Input
              placeholder={t("stockAnalysis.reflection.placeholderCode")}
              value={manualCode}
              onChange={(e) => setManualCode(e.target.value)}
              style={{ width: 120 }}
            />
            <Input
              placeholder={t("stockAnalysis.reflection.placeholderDate")}
              value={manualAsOf}
              onChange={(e) => setManualAsOf(e.target.value)}
              style={{ width: 140 }}
            />
            <Input
              placeholder={t("stockAnalysis.reflection.placeholderOutcome")}
              value={manualOutcome}
              onChange={(e) => setManualOutcome(e.target.value)}
              style={{ width: 200 }}
            />
            <Select
              value={manualDepth}
              onChange={setManualDepth}
              options={[
                { label: t("stockAnalysis.reflection.depthLight"), value: "light" },
                { label: t("stockAnalysis.reflection.depthDeep"), value: "deep" },
              ]}
              style={{ width: 100 }}
            />
            <Button
              type="primary"
              onClick={handleManualReflection}
              loading={running}
              disabled={running}
            >
              {t("stockAnalysis.reflection.startReflection")}
            </Button>
          </Space>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t("stockAnalysis.reflection.manualHint")}
          </Text>
        </Space>
      </Card>

      {/* 定时校验配置 */}
      <Card title={t("stockAnalysis.reflection.scheduleTitle")} size="small" style={{ marginBottom: 16 }}>
        <Space direction="vertical" style={{ width: "100%" }}>
          <Space>
            <Switch checked={isEnabled} onChange={toggleCron} />
            <Text>{t("stockAnalysis.reflection.scheduleDesc")}</Text>
          </Space>
          <Space wrap>
            <Select
              value={cronExpr}
              onChange={setCronExpr}
              options={CRON_PRESETS}
              style={{ width: 150 }}
              disabled={isEnabled}
            />
            <Select
              value={depth}
              onChange={setDepth}
              options={[
                { label: t("stockAnalysis.reflection.depthLight"), value: "light" },
                { label: t("stockAnalysis.reflection.depthDeep"), value: "deep" },
              ]}
              style={{ width: 110 }}
              disabled={isEnabled}
            />
            <Input
              type="number"
              min={0}
              max={100}
              value={threshold}
              onChange={(e) => setThreshold(Number(e.target.value))}
              style={{ width: 100 }}
              disabled={isEnabled}
              addonAfter={t("stockAnalysis.reflection.confidence")}
            />
            {activeCron && (
              <Button danger size="small" onClick={deleteCron}>
                {t("stockAnalysis.reflection.deleteBtn")}
              </Button>
            )}
          </Space>
          {activeCron && (
            <Text type="secondary">
              {t("stockAnalysis.reflection.scheduleDescFull", {
                schedule: activeCron.schedule,
                status: isEnabled
                  ? t("stockAnalysis.reflection.running")
                  : t("stockAnalysis.reflection.paused"),
              })}
            </Text>
          )}
        </Space>
      </Card>

      {/* 反思历史 */}
      <Card
        title={t("stockAnalysis.reflection.historyTitle")}
        size="small"
        extra={<Button size="small" onClick={load}>{t("stockAnalysis.reflection.refreshBtn")}</Button>}
      >
        {reflections.length === 0
          ? <Empty description={t("stockAnalysis.reflection.empty")} />
          : (
            <Table
              dataSource={reflections}
              rowKey="id"
              pagination={{ pageSize: 10, size: "small" }}
              size="small"
              scroll={{ x: 800 }}
              expandable={{
                expandedRowKeys: expandedId ? [expandedId] : [],
                onExpandedRowsChange: (keys: readonly React.Key[]) => setExpandedId(keys[0] as string || null),
                expandedRowRender: (r: ReflectionRow) => (
                  <ExpandedReflectionRow row={r} t={t as (key: string, opts?: object) => string} />
                ),
              }}
            >
              <Table.Column title={t("stockAnalysis.reflection.colCode")} dataIndex="stockCode" width={90} />
              <Table.Column title={t("stockAnalysis.reflection.colName")} dataIndex="stockName" width={100} />
              <Table.Column title={t("stockAnalysis.reflection.colAsOf")} dataIndex="asOfDate" width={100} />
              <Table.Column
                title={t("stockAnalysis.reflection.colResult")}
                dataIndex="actualOutcome"
                width={160}
                render={(v: string) => <Text type="danger">{v}</Text>}
              />
              <Table.Column
                title={t("stockAnalysis.reflection.colCause")}
                dataIndex="whatWentWrong"
                ellipsis
                render={(v: string | null) => v || "-"}
              />
              <Table.Column
                title={t("stockAnalysis.reflection.colTime")}
                dataIndex="createdAt"
                width={150}
                render={(v: number) => new Date(v).toLocaleDateString("zh-CN")}
              />
            </Table>
          )}
      </Card>
    </div>
  );
}

function formatJson(val: string | null): string {
  if (!val) { return "-"; }
  try {
    const parsed = JSON.parse(val);
    return Array.isArray(parsed) ? parsed.join("; ") : val;
  } catch {
    return val;
  }
}

interface ParamSuggestion {
  param: string;
  current_value: number;
  suggested_value: number;
  reason: string;
}

function parseParamsSuggestion(row: ReflectionRow): ParamSuggestion[] {
  if (!row.decisionJson) { return []; }
  try {
    const parsed = JSON.parse(row.decisionJson);
    const raw = parsed?.params_suggestion ?? parsed?.reflection?.params_suggestion;
    if (Array.isArray(raw)) {
      return raw as ParamSuggestion[];
    }
    return [];
  } catch {
    return [];
  }
}

function ExpandedReflectionRow({ row, t }: { row: ReflectionRow; t: (key: string, opts?: object) => string }) {
  const suggestions = parseParamsSuggestion(row);
  const [checkedParams, setCheckedParams] = useState<string[]>([]);
  const [applying, setApplying] = useState(false);

  const handleApply = async () => {
    const selected = suggestions.filter((s) => checkedParams.includes(s.param));
    if (selected.length === 0) { return; }
    setApplying(true);
    try {
      await invoke("apply_param_suggestions", {
        updates: selected.map((s) => ({ param: s.param, value: s.suggested_value })),
      });
      message.success(t("stockAnalysis.reflection.applySuccess"));
      setCheckedParams([]);
    } catch (e) {
      message.error(t("stockAnalysis.reflection.applyFailed", { error: String(e) }));
    } finally {
      setApplying(false);
    }
  };

  return (
    <div style={{ padding: "8px 0" }}>
      <Space direction="vertical" style={{ width: "100%" }}>
        <Space>
          <Tag color="red">{row.actualOutcome}</Tag>
          <Tag>
            {row.reflectionDepth === "deep"
              ? t("stockAnalysis.reflection.depthDeepLabel")
              : t("stockAnalysis.reflection.depthLightLabel")}
          </Tag>
          <Text type="secondary">
            {t("stockAnalysis.reflection.asOfLabel", {
              asOf: row.asOfDate,
              hindsight: row.hindsightDate,
            })}
          </Text>
        </Space>
        <div>
          <Text strong>{t("stockAnalysis.reflection.causeLabel")}</Text>
          <Text>{row.whatWentWrong || "-"}</Text>
        </div>
        <div>
          <Text strong>{t("stockAnalysis.reflection.signalsLabel")}</Text>
          <Text>{formatJson(row.missedSignals)}</Text>
        </div>
        <div>
          <Text strong>{t("stockAnalysis.reflection.improveLabel")}</Text>
          <Text>{row.fixForFuture || "-"}</Text>
        </div>
        {suggestions.length > 0 && (
          <div>
            <Text strong style={{ fontSize: 13 }}>
              ⚙ {t("stockAnalysis.reflection.paramSuggestTitle")}
            </Text>
            <div style={{ marginTop: 8 }}>
              <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
                <thead>
                  <tr style={{ borderBottom: "1px solid #d9d9d9" }}>
                    <th style={{ padding: "4px 8px", textAlign: "left", width: 32 }} />
                    <th style={{ padding: "4px 8px", textAlign: "left" }}>
                      {t("stockAnalysis.reflection.paramCol")}
                    </th>
                    <th style={{ padding: "4px 8px", textAlign: "right", width: 80 }}>
                      {t("stockAnalysis.reflection.paramCurrent")}
                    </th>
                    <th style={{ padding: "4px 8px", textAlign: "right", width: 80 }}>
                      {t("stockAnalysis.reflection.paramSuggested")}
                    </th>
                    <th style={{ padding: "4px 8px", textAlign: "left" }}>
                      {t("stockAnalysis.reflection.paramReason")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {suggestions.map((s) => (
                    <tr key={s.param} style={{ borderBottom: "1px solid #f0f0f0" }}>
                      <td style={{ padding: "4px 8px" }}>
                        <Checkbox
                          checked={checkedParams.includes(s.param)}
                          onChange={(e) => {
                            setCheckedParams(
                              e.target.checked
                                ? [...checkedParams, s.param]
                                : checkedParams.filter((p) => p !== s.param),
                            );
                          }}
                        />
                      </td>
                      <td style={{ padding: "4px 8px", fontFamily: "monospace" }}>
                        {s.param}
                      </td>
                      <td style={{ padding: "4px 8px", textAlign: "right" }}>
                        {s.current_value}
                      </td>
                      <td style={{ padding: "4px 8px", textAlign: "right" }}>
                        <Text strong style={{ color: "#1890ff" }}>{s.suggested_value}</Text>
                      </td>
                      <td style={{ padding: "4px 8px" }}>
                        <Text type="secondary">{s.reason}</Text>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <Button
                type="primary"
                size="small"
                style={{ marginTop: 8 }}
                loading={applying}
                disabled={checkedParams.length === 0}
                onClick={handleApply}
              >
                {t("stockAnalysis.reflection.applyParamsBtn", { count: checkedParams.length })}
              </Button>
            </div>
          </div>
        )}
      </Space>
    </div>
  );
}
