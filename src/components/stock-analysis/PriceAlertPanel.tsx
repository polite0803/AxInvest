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
      if (Array.isArray(list)) { setAlerts(list); }
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
      width: 60,
      render: (v: string) => <Tag style={{ fontSize: 10, padding: "0 4px" }}>{v}</Tag>,
    },
    {
      title: t("stockAnalysis.alert.name"),
      dataIndex: "stockName",
      ellipsis: true as const,
    },
    {
      title: t("stockAnalysis.alert.condition"),
      dataIndex: "condition",
      width: 44,
      render: (v: string) => (v === "above" ? "≥" : "≤"),
    },
    {
      title: t("stockAnalysis.alert.price"),
      dataIndex: "targetPrice",
      width: 58,
      align: "right" as const,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: "",
      key: "action",
      width: 30,
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
      bodyStyle={{ padding: "8px 8px" }}
      title={
        <span>
          <BellOutlined /> {t("stockAnalysis.alert.title")}
        </span>
      }
      extra={
        <div className="flex gap-1 items-center">
          <Tag color={alerts.length > 0 ? "green" : "default"} style={{ fontSize: 10, lineHeight: "18px" }}>
            {alerts.length > 0 ? t("stockAnalysis.monitoringActive") : t("stockAnalysis.monitoringPaused")}
          </Tag>
          <Button size="small" icon={<PlusOutlined />} onClick={() => setAdding(!adding)} />
        </div>
      }
    >
      {adding && (
        <Form form={form} size="small" onFinish={addAlert} className="mb-2">
          <div className="flex gap-1 mb-1">
            <Form.Item name="stockCode" rules={[{ required: true }]} noStyle>
              <Input placeholder={t("stockAnalysis.alert.code")} style={{ width: 72 }} size="small" />
            </Form.Item>
            <Form.Item name="stockName" rules={[{ required: true }]} noStyle>
              <Input placeholder={t("stockAnalysis.alert.name")} style={{ width: 72 }} size="small" />
            </Form.Item>
            <Form.Item name="condition" rules={[{ required: true }]} initialValue="above" noStyle>
              <Select style={{ width: 60 }} size="small">
                <Select.Option value="above">≥</Select.Option>
                <Select.Option value="below">≤</Select.Option>
              </Select>
            </Form.Item>
          </div>
          <div className="flex gap-1">
            <Form.Item name="targetPrice" rules={[{ required: true }]} noStyle>
              <Input type="number" placeholder={t("stockAnalysis.alert.price")} style={{ width: 100 }} size="small" />
            </Form.Item>
            <Form.Item noStyle>
              <Button type="primary" htmlType="submit" size="small">{t("common.confirm")}</Button>
            </Form.Item>
          </div>
        </Form>
      )}
      <Table
        dataSource={alerts}
        columns={columns}
        rowKey="id"
        size="small"
        pagination={false}
        className="sa-alert-table"
        locale={{ emptyText: t("stockAnalysis.alert.empty") }}
      />
    </Card>
  );
}
