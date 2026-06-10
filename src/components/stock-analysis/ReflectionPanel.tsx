import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Input, Select, Space, Switch, Table, Tag, Typography } from "antd";
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
    load();
  }, []);

  const activeCron = cronJobs.find((j) => j.status === "Active") || cronJobs[0];
  const isEnabled = cronJobs.some((j) => j.status === "Active");

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

  return (
    <div style={{ padding: 16 }}>
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
              options={[{ label: t("stockAnalysis.reflection.depthLight"), value: "light" }, {
                label: t("stockAnalysis.reflection.depthDeep"),
                value: "deep",
              }]}
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
              <Button danger size="small" onClick={deleteCron}>{t("stockAnalysis.reflection.deleteBtn")}</Button>
            )}
          </Space>
          {activeCron && (
            <Text type="secondary">
              {t("stockAnalysis.reflection.scheduleDescFull", {
                schedule: activeCron.schedule,
                status: isEnabled ? t("stockAnalysis.reflection.running") : t("stockAnalysis.reflection.paused"),
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
        {reflections.length === 0 ? <Empty description={t("stockAnalysis.reflection.empty")} /> : (
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
                <div style={{ padding: "8px 0" }}>
                  <Space direction="vertical" style={{ width: "100%" }}>
                    <Space>
                      <Tag color="red">{r.actualOutcome}</Tag>
                      <Tag>
                        {r.reflectionDepth === "deep"
                          ? t("stockAnalysis.reflection.depthDeepLabel")
                          : t("stockAnalysis.reflection.depthLightLabel")}
                      </Tag>
                      <Text type="secondary">
                        {t("stockAnalysis.reflection.asOfLabel", { asOf: r.asOfDate, hindsight: r.hindsightDate })}
                      </Text>
                    </Space>
                    <div>
                      <Text strong>{t("stockAnalysis.reflection.causeLabel")}</Text>
                      <Text>{r.whatWentWrong || "-"}</Text>
                    </div>
                    <div>
                      <Text strong>{t("stockAnalysis.reflection.signalsLabel")}</Text>
                      <Text>{formatJson(r.missedSignals)}</Text>
                    </div>
                    <div>
                      <Text strong>{t("stockAnalysis.reflection.improveLabel")}</Text>
                      <Text>{r.fixForFuture || "-"}</Text>
                    </div>
                  </Space>
                </div>
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
