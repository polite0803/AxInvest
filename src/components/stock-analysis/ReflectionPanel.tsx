import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Input, Select, Space, Switch, Table, Tag, Typography } from "antd";
import { useEffect, useState } from "react";

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

const CRON_PRESETS = [
  { label: "每天 6:00", value: "0 6 * * *" },
  { label: "每天 12:00", value: "0 12 * * *" },
  { label: "每天 18:00", value: "0 18 * * *" },
  { label: "每工作日 6:00", value: "0 6 * * 1-5" },
];

export function ReflectionPanel() {
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
      <Card title="定时校验配置" size="small" style={{ marginBottom: 16 }}>
        <Space direction="vertical" style={{ width: "100%" }}>
          <Space>
            <Switch checked={isEnabled} onChange={toggleCron} />
            <Text>每天自动校验30天前的分析结果，loss触发反思</Text>
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
              options={[{ label: "简要反思", value: "light" }, { label: "深度反思", value: "deep" }]}
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
              addonAfter="最低置信度"
            />
            {activeCron && <Button danger size="small" onClick={deleteCron}>删除任务</Button>}
          </Space>
          {activeCron && (
            <Text type="secondary">当前调度: {activeCron.schedule} | 状态: {isEnabled ? "运行中" : "已暂停"}</Text>
          )}
        </Space>
      </Card>

      {/* 反思历史 */}
      <Card title="反思历史" size="small" extra={<Button size="small" onClick={load}>刷新</Button>}>
        {reflections.length === 0 ? <Empty description="暂无反思记录" /> : (
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
                      <Tag>{r.reflectionDepth === "deep" ? "深度反思" : "简要反思"}</Tag>
                      <Text type="secondary">as-of: {r.asOfDate} → 后见: {r.hindsightDate}</Text>
                    </Space>
                    <div>
                      <Text strong>错因分析：</Text>
                      <Text>{r.whatWentWrong || "-"}</Text>
                    </div>
                    <div>
                      <Text strong>被忽视的信号：</Text>
                      <Text>{formatJson(r.missedSignals)}</Text>
                    </div>
                    <div>
                      <Text strong>改进建议：</Text>
                      <Text>{r.fixForFuture || "-"}</Text>
                    </div>
                  </Space>
                </div>
              ),
            }}
          >
            <Table.Column title="代码" dataIndex="stockCode" width={90} />
            <Table.Column title="名称" dataIndex="stockName" width={100} />
            <Table.Column title="分析日" dataIndex="asOfDate" width={100} />
            <Table.Column
              title="结果"
              dataIndex="actualOutcome"
              width={160}
              render={(v: string) => <Text type="danger">{v}</Text>}
            />
            <Table.Column title="错因" dataIndex="whatWentWrong" ellipsis render={(v: string | null) => v || "-"} />
            <Table.Column
              title="时间"
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
