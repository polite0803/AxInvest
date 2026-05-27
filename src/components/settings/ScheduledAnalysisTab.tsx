import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { Button, Card, Empty, Form, Input, message, Popconfirm, Select, Spin, Switch, Table, Tag } from "antd";
import { useEffect, useState } from "react";

interface Schedule {
  id: string;
  stockCode: string;
  stockName: string;
  cronExpression: string;
  enabled: boolean;
  createdAt: number;
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
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();

  const load = async () => {
    setLoading(true);
    try {
      const list = await invoke<Schedule[]>("list_analysis_schedules");
      if (Array.isArray(list)) { setSchedules(list); }
    } catch { /* 后端未运行 */ }
    setLoading(false);
  };

  useEffect(() => {
    load();
  }, []);

  const create = async (values: any) => {
    try {
      await invoke("create_analysis_schedule", values);
      form.resetFields();
      setAdding(false);
      load();
      message.success("定时任务已创建");
    } catch {
      message.error("创建失败");
    }
  };

  const toggle = async (id: string, enabled: boolean) => {
    try {
      await invoke("toggle_analysis_schedule", { id, enabled });
      load();
    } catch { /* 静默 */ }
  };

  const remove = async (id: string) => {
    try {
      await invoke("delete_analysis_schedule", { id });
      load();
    } catch { /* 静默 */ }
  };

  const columns = [
    { title: "代码", dataIndex: "stockCode", width: 70, render: (v: string) => <Tag className="text-xs m-0">{v}</Tag> },
    { title: "名称", dataIndex: "stockName", ellipsis: true },
    {
      title: "Cron",
      dataIndex: "cronExpression",
      width: 120,
      render: (v: string) => <span className="text-xs font-mono">{v}</span>,
    },
    {
      title: "状态",
      dataIndex: "enabled",
      width: 60,
      render: (v: boolean, record: Schedule) => (
        <Switch
          size="small"
          checked={v}
          onChange={(checked) => toggle(record.id, checked)}
        />
      ),
    },
    {
      title: "",
      key: "action",
      width: 30,
      render: (_: unknown, record: Schedule) => (
        <Popconfirm title="确认删除?" onConfirm={() => remove(record.id)}>
          <Button size="small" type="text" danger icon={<DeleteOutlined />} />
        </Popconfirm>
      ),
    },
  ];

  if (loading) { return <Spin size="small" />; }

  return (
    <div className="flex flex-col gap-3">
      <div className="text-sm text-gray-500">定时自动分析 — 按 Cron 表达式周期性对指定股票执行完整分析流程</div>

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

      {schedules.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无定时任务" />
        : <Table size="small" dataSource={schedules} columns={columns} rowKey="id" pagination={false} />}
    </div>
  );
}
