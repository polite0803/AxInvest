import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { Button, Card, Empty, Form, Input, message, Popconfirm, Select, Spin, Switch, Table, Tag } from "antd";
import { useEffect, useState } from "react";

interface CronJobRow {
  id: string;
  name: string;
  description: string;
  schedule: string;
  status: string;
  runCount: number;
  lastRunAt: number | null;
}

const CRON_PRESETS = [
  { label: "每日 09:00", value: "0 9 * * *" },
  { label: "每日 15:30 (收盘)", value: "30 15 * * *" },
  { label: "每周一 09:00", value: "0 9 * * 1" },
  { label: "每小时", value: "0 * * * *" },
];

export function ScheduledAnalysisTab() {
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const [jobs, setJobs] = useState<CronJobRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();

  // 自选股自动扫描
  const [wlScanJobs, setWlScanJobs] = useState<CronJobRow[]>([]);
  const [wlCron, setWlCron] = useState("0 21 * * 1-5");

  const loadWlScan = async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_watchlist_scan_crons");
      if (Array.isArray(list)) { setWlScanJobs(list); }
    } catch { /* 后端未运行 */ }
  };

  const toggleWlScan = async (job: CronJobRow | null, enable: boolean) => {
    try {
      if (enable && !job) {
        await invoke("create_watchlist_scan_cron", {
          cronExpression: wlCron,
          enabled: true,
        });
        message.success("自选股自动扫描已开启");
      } else if (!enable && job) {
        await invoke("toggle_watchlist_scan_cron", { id: job.id, enabled: false });
        message.success("自选股自动扫描已暂停");
      }
      loadWlScan();
    } catch {
      message.error("操作失败");
    }
  };

  const deleteWlScan = async (id: string) => {
    try {
      await invoke("delete_watchlist_scan_cron", { id });
      loadWlScan();
    } catch { /* 静默 */ }
  };

  const load = async () => {
    setLoading(true);
    try {
      const list = await invoke<CronJobRow[]>("list_stock_crons");
      if (Array.isArray(list)) { setJobs(list); }
    } catch { /* 后端未运行 */ }
    setLoading(false);
  };

  useEffect(() => {
    load();
    loadWlScan();
  }, []);

  const create = async (values: any) => {
    try {
      await invoke("create_stock_cron", {
        stockCode: values.stockCode,
        stockName: values.stockName,
        cronExpression: values.cronExpression,
      });
      form.resetFields();
      setAdding(false);
      load();
      message.success("定时任务已创建");
    } catch {
      message.error("创建失败");
    }
  };

  const toggle = async (id: string, active: boolean) => {
    try {
      await invoke("toggle_stock_cron", { id, enabled: active });
      load();
    } catch { /* 静默 */ }
  };

  const remove = async (id: string) => {
    try {
      await invoke("delete_stock_cron", { id });
      load();
    } catch { /* 静默 */ }
  };

  const columns = [
    {
      title: "ID",
      dataIndex: "name",
      ellipsis: true,
      width: 100,
      render: (v: string) => <span className="text-xs font-mono">{v}</span>,
    },
    { title: "描述", dataIndex: "description", ellipsis: true },
    {
      title: "Cron",
      dataIndex: "schedule",
      width: 120,
      render: (v: string) => <Tag className="text-xs m-0 font-mono">{v}</Tag>,
    },
    {
      title: "已执行",
      dataIndex: "runCount",
      width: 50,
      align: "center" as const,
      render: (v: number) => <span className="text-xs">{v}</span>,
    },
    {
      title: "状态",
      dataIndex: "status",
      width: 60,
      render: (v: string, record: CronJobRow) => (
        <Switch
          size="small"
          checked={v === "active"}
          onChange={(checked) => toggle(record.id, checked)}
        />
      ),
    },
    {
      title: "",
      key: "action",
      width: 30,
      render: (_: unknown, record: CronJobRow) => (
        <Popconfirm title="确认删除?" onConfirm={() => remove(record.id)}>
          <Button size="small" type="text" danger icon={<DeleteOutlined />} />
        </Popconfirm>
      ),
    },
  ];

  if (loading) { return <Spin size="small" />; }

  return (
    <div className="flex flex-col gap-3">
      <div className="text-sm text-gray-500">
        基于上游 CronJobStore 持久化 — 按 Cron 表达式周期性对指定股票执行完整分析流程
      </div>

      {/* 自选股自动扫描 */}
      <Card size="small" title="自选股自动扫描" styles={{ body: { padding: "8px 12px" } }}>
        <div className="flex items-center gap-3">
          <Switch
            checked={wlScanJobs.some((j) => j.status === "active")}
            onChange={(checked) => toggleWlScan(wlScanJobs.length > 0 ? wlScanJobs[0] : null, checked)}
          />
          <span className="text-xs text-gray-400">
            {wlScanJobs.some((j) => j.status === "active") ? "已开启" : "已关闭"}
          </span>
          <Select
            size="small"
            style={{ width: 180 }}
            value={wlCron}
            onChange={setWlCron}
            options={[
              { label: "交易日 21:00 (每日收盘)", value: "0 21 * * 1-5" },
              { label: "每日 09:00 (开盘前)", value: "0 9 * * *" },
              { label: "每日 15:30 (收盘)", value: "30 15 * * *" },
              { label: "每周一 09:00", value: "0 9 * * 1" },
            ]}
          />
          {wlScanJobs.length > 0 && (
            <Popconfirm
              title="删除此定时任务?"
              onConfirm={() => deleteWlScan(wlScanJobs[0].id)}
            >
              <Button size="small" type="text" danger icon={<DeleteOutlined />} />
            </Popconfirm>
          )}
        </div>
      </Card>

      <Card
        size="small"
        title="新建任务"
        styles={{ body: { padding: "8px 12px" } }}
        extra={
          <Button size="small" icon={<PlusOutlined />} onClick={() => setAdding(!adding)}>
            {adding ? "取消" : "新建"}
          </Button>
        }
      >
        {adding && (
          <Form form={form} size="small" layout="inline" onFinish={create}>
            <Form.Item name="stockCode" rules={[{ required: true }]} initialValue={stockCode}>
              <Input placeholder="股票代码" style={{ width: 100 }} />
            </Form.Item>
            <Form.Item name="stockName" rules={[{ required: true }]} initialValue={stockName}>
              <Input placeholder="股票名称" style={{ width: 100 }} />
            </Form.Item>
            <Form.Item name="cronExpression" rules={[{ required: true }]} initialValue="0 9 * * *">
              <Select style={{ width: 160 }} options={CRON_PRESETS} placeholder="Cron 表达式" />
            </Form.Item>
            <Form.Item>
              <Button type="primary" htmlType="submit">创建</Button>
            </Form.Item>
          </Form>
        )}
      </Card>

      {jobs.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无定时任务" />
        : <Table size="small" dataSource={jobs} columns={columns} rowKey="id" pagination={false} />}
    </div>
  );
}
