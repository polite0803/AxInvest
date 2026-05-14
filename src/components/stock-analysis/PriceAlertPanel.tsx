import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import { BellOutlined, DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { Button, Card, Form, Input, message, Select, Table, Tag } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface PriceAlert {
  id: string;
  stockCode: string;
  stockName: string;
  condition: string;
  targetPrice: number;
  isTriggered: boolean;
  triggeredAt: number | null;
  createdAt: number;
}

export function PriceAlertPanel() {
  const { t } = useTranslation();
  const [alerts, setAlerts] = useState<PriceAlert[]>([]);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();

  const loadAlerts = async () => {
    try {
      const list = await invoke<PriceAlert[]>("list_price_alerts");
      setAlerts(list);
    } catch {
      // 后端未运行或无数据时静默
    }
  };

  useEffect(() => {
    loadAlerts();
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    (async () => {
      try {
        unlisten = await listen<PriceAlert>("price-alert-triggered", () => {
          message.info(t("stockAnalysis.alert.triggered"));
          loadAlerts();
        });
      } catch {
        // 事件监听可选，静默处理
      }
    })();
    return () => {
      if (unlisten) { unlisten(); }
    };
  }, [t]);

  const addAlert = async (values: { stockCode: string; stockName: string; condition: string; targetPrice: number }) => {
    try {
      await invoke("create_price_alert", values);
      form.resetFields();
      setAdding(false);
      await loadAlerts();
    } catch {
      // 静默处理
    }
  };

  const removeAlert = async (id: string) => {
    try {
      await invoke("delete_price_alert", { id });
      await loadAlerts();
    } catch {
      // 静默处理
    }
  };

  const columns = [
    {
      title: t("stockAnalysis.alert.code"),
      dataIndex: "stockCode",
      width: 80,
      render: (v: string) => <Tag>{v}</Tag>,
    },
    {
      title: t("stockAnalysis.alert.name"),
      dataIndex: "stockName",
      width: 80,
    },
    {
      title: t("stockAnalysis.alert.condition"),
      dataIndex: "condition",
      width: 60,
      render: (v: string) => (v === "above" ? ">" : "<"),
    },
    {
      title: t("stockAnalysis.alert.price"),
      dataIndex: "targetPrice",
      width: 70,
      align: "right" as const,
    },
    {
      title: "",
      key: "action",
      width: 40,
      render: (_: unknown, record: PriceAlert) => (
        <Button
          size="small"
          type="text"
          danger
          icon={<DeleteOutlined />}
          onClick={() => removeAlert(record.id)}
        />
      ),
    },
  ];

  return (
    <Card
      size="small"
      title={
        <span>
          <BellOutlined /> {t("stockAnalysis.alert.title")}
        </span>
      }
      extra={<Button size="small" icon={<PlusOutlined />} onClick={() => setAdding(!adding)} />}
    >
      {adding && (
        <Form form={form} layout="inline" size="small" onFinish={addAlert} className="mb-2 flex flex-col gap-1">
          <Form.Item name="stockCode" rules={[{ required: true }]}>
            <Input placeholder={t("stockAnalysis.alert.code")} style={{ width: 80 }} />
          </Form.Item>
          <Form.Item name="stockName" rules={[{ required: true }]}>
            <Input placeholder={t("stockAnalysis.alert.name")} style={{ width: 80 }} />
          </Form.Item>
          <Form.Item name="condition" rules={[{ required: true }]} initialValue="above">
            <Select style={{ width: 70 }}>
              <Select.Option value="above">{t("stockAnalysis.alert.above")}</Select.Option>
              <Select.Option value="below">{t("stockAnalysis.alert.below")}</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="targetPrice" rules={[{ required: true }]}>
            <Input type="number" placeholder={t("stockAnalysis.alert.price")} style={{ width: 80 }} />
          </Form.Item>
          <Form.Item>
            <Button type="primary" htmlType="submit" size="small">{t("common.confirm")}</Button>
          </Form.Item>
        </Form>
      )}
      <Table
        dataSource={alerts}
        columns={columns}
        rowKey="id"
        size="small"
        pagination={false}
        locale={{ emptyText: t("stockAnalysis.alert.empty") }}
      />
    </Card>
  );
}
