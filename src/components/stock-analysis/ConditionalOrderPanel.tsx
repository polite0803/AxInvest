import { invoke } from "@/lib/invoke";
import type { ConditionalOrderItem, ConditionalOrderListView } from "@/types";
import { DeleteOutlined, PlusOutlined, SwitcherOutlined } from "@ant-design/icons";
import { Alert, App, Badge, Button, Card, Form, Input, InputNumber, Select, Switch, Table, Tag } from "antd";
import type { TFunction } from "i18next";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/** 条件类别 → 既有的告警类型键（六类语义与价格预警一一对应，复用而不另造一套译名）。 */
const CONDITION_TYPE_KEYS: Record<string, string> = {
  stop_loss: "stockAnalysis.alert.typeStopLoss",
  take_profit: "stockAnalysis.alert.typeTakeProfit",
  resistance_break: "stockAnalysis.alert.typeResistance",
  support_break: "stockAnalysis.alert.typeSupport",
  change_pct: "stockAnalysis.alert.typeChange",
  turnover_rate: "stockAnalysis.alert.typeVolume",
};

/** 类别串的取值域以后端 `ConditionType::as_str()` 为权威；未认识的类别落「未知」而不是硬套六类之一。 */
export function conditionTypeLabel(t: TFunction, conditionType: string): string {
  const key = CONDITION_TYPE_KEYS[conditionType];
  return key ? t(key) : t("stockAnalysis.alert.typeUnknown");
}

/** 价格类四类用「元」，比率类两类用「%」——与后端 `condition_value` 的双口径注释同一份判定。 */
function isPriceKind(conditionType: string): boolean {
  return (
    conditionType === "stop_loss"
    || conditionType === "take_profit"
    || conditionType === "resistance_break"
    || conditionType === "support_break"
  );
}

/** 阈值输入框的提示语按类别换单位 —— 一格同时装价格与百分比正是 H-2 那个缺陷的形态。 */
export function thresholdPlaceholderKey(conditionType: string): string {
  return isPriceKind(conditionType) ? "stockAnalysis.alert.price" : "stockAnalysis.conditional.thresholdPct";
}

function formatClock(millis: number | null): string {
  if (!millis) { return "—"; }
  return new Date(millis).toLocaleString();
}

export function ConditionalOrderPanel() {
  const { message, modal } = App.useApp();
  const { t } = useTranslation();
  const [view, setView] = useState<ConditionalOrderListView>({ items: [], unreadable: [], evaluating: 0 });
  const [loadError, setLoadError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();
  const conditionType = Form.useWatch("conditionType", form) as string | undefined;
  const actionType = Form.useWatch("actionType", form) as string | undefined;

  const load = useCallback(async () => {
    try {
      const next = await invoke<ConditionalOrderListView>("list_conditional_orders");
      setView(next);
      setLoadError(null);
    } catch (e) {
      // 读失败与「表里确实没有」是两种状态：清空表格再显示「暂无」就是把故障伪装成空集
      setView({ items: [], unreadable: [], evaluating: 0 });
      setLoadError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const createOrder = async (values: Record<string, unknown>) => {
    try {
      await invoke("create_conditional_order", {
        req: {
          stockCode: values.stockCode,
          stockName: values.stockName,
          conditionType: values.conditionType,
          conditionValue: values.conditionValue,
          actionType: values.actionType,
          actionValue: values.actionValue ?? null,
          coolDownMinutes: values.coolDownMinutes ?? null,
          maxTriggersPerDay: values.maxTriggersPerDay ?? null,
          activeStart: values.activeStart || null,
          activeEnd: values.activeEnd || null,
        },
      });
      form.resetFields();
      setAdding(false);
      await load();
    } catch (e) {
      message.error(t("stockAnalysis.conditional.createFailed", { reason: String(e) }));
    }
  };

  const toggle = async (item: ConditionalOrderItem, enabled: boolean) => {
    try {
      await invoke("set_conditional_order_enabled", { orderId: item.orderId, enabled });
      await load();
    } catch (e) {
      message.error(t("stockAnalysis.conditional.toggleFailed", { reason: String(e) }));
    }
  };

  const remove = (item: ConditionalOrderItem) => {
    modal.confirm({
      title: t("stockAnalysis.conditional.confirmDelete"),
      content: `${item.stockCode} · ${item.conditionText}`,
      onOk: async () => {
        try {
          await invoke("delete_conditional_order", { orderId: item.orderId });
          await load();
        } catch (e) {
          message.error(t("stockAnalysis.conditional.deleteFailed", { reason: String(e) }));
        }
      },
    });
  };

  const columns = [
    {
      title: t("stockAnalysis.alert.code"),
      dataIndex: "stockCode",
      width: 76,
      render: (v: string) => <Tag className="text-xs m-0">{v}</Tag>,
    },
    { title: t("stockAnalysis.alert.name"), dataIndex: "stockName", ellipsis: true as const, width: 90 },
    {
      title: t("stockAnalysis.alert.condition"),
      dataIndex: "conditionText",
      // 类别名与阈值都由后端 describe() 给一句人话，前端不再自己拼数与单位
      render: (v: string, record: ConditionalOrderItem) => (
        <span title={conditionTypeLabel(t, record.conditionType)}>{v}</span>
      ),
    },
    {
      title: t("stockAnalysis.conditional.action"),
      dataIndex: "actionText",
      width: 110,
    },
    {
      title: t("stockAnalysis.conditional.triggeredToday"),
      dataIndex: "triggersToday",
      width: 60,
      align: "right" as const,
      render: (v: number) => v || "—",
    },
    {
      title: t("stockAnalysis.conditional.lastTriggered"),
      dataIndex: "lastTriggeredAt",
      width: 130,
      render: (v: number | null) => formatClock(v),
    },
    {
      title: t("stockAnalysis.conditional.enabled"),
      dataIndex: "enabled",
      width: 60,
      render: (v: boolean, record: ConditionalOrderItem) => (
        <Switch
          size="small"
          checked={v}
          onChange={(next) => void toggle(record, next)}
        />
      ),
    },
    {
      title: "",
      key: "delete",
      width: 34,
      render: (_: unknown, record: ConditionalOrderItem) => (
        <Button
          size="small"
          type="text"
          danger
          icon={<DeleteOutlined />}
          onClick={() => remove(record)}
        />
      ),
    },
  ];

  return (
    <Card
      size="small"
      styles={{ body: { padding: "8px 8px" } }}
      title={
        <span>
          <SwitcherOutlined /> {t("stockAnalysis.conditional.title")}
        </span>
      }
      extra={
        <div className="flex gap-2 items-center">
          {
            /* 两个计数各有自己的分母与文案，不并排成可比的两个数：
              `evaluating` 是「正在被行情评估的规则数」（启用且能读回），
              `items.length` 是「表里的行数」（含已停用）。 */
          }
          <Badge
            status={view.evaluating > 0 ? "processing" : "default"}
            text={t("stockAnalysis.conditional.evaluating", { count: view.evaluating })}
          />
          <Button size="small" icon={<PlusOutlined />} onClick={() => setAdding(!adding)} />
        </div>
      }
    >
      {adding && (
        <Form form={form} size="small" onFinish={createOrder} className="mb-2">
          <div className="flex gap-1 mb-1 flex-wrap">
            <Form.Item name="stockCode" rules={[{ required: true }]} noStyle>
              <Input placeholder={t("stockAnalysis.alert.code")} style={{ width: 72 }} size="small" />
            </Form.Item>
            <Form.Item name="stockName" rules={[{ required: true }]} noStyle>
              <Input placeholder={t("stockAnalysis.alert.name")} style={{ width: 72 }} size="small" />
            </Form.Item>
            <Form.Item name="conditionType" initialValue="stop_loss" noStyle>
              <Select
                style={{ width: 110 }}
                size="small"
                options={Object.keys(CONDITION_TYPE_KEYS).map((kind) => ({
                  value: kind,
                  label: conditionTypeLabel(t, kind),
                }))}
              />
            </Form.Item>
            <Form.Item name="conditionValue" rules={[{ required: true }]} noStyle>
              <InputNumber
                size="small"
                style={{ width: 96 }}
                min={0.01}
                step={isPriceKind(conditionType ?? "stop_loss") ? 0.01 : 0.1}
                placeholder={t(thresholdPlaceholderKey(conditionType ?? "stop_loss"))}
              />
            </Form.Item>
          </div>
          <div className="flex gap-1 flex-wrap">
            <Form.Item name="actionType" initialValue="notify" noStyle>
              <Select
                style={{ width: 96 }}
                size="small"
                options={[
                  { value: "notify", label: t("stockAnalysis.conditional.actionNotify") },
                  // 档位标签只有顶层一套（`stockAnalysis.action*`）：条件单此前自带
                  // actionSell/actionBuy/actionReduce 三个副本，同一动作在一屏里出现「减仓 / 减持」两种说法
                  { value: "sell", label: t("stockAnalysis.actionSell") },
                  { value: "buy", label: t("stockAnalysis.actionBuy") },
                  { value: "reduce", label: t("stockAnalysis.actionReduce") },
                ]}
              />
            </Form.Item>
            {/* 只有 buy / reduce 需要参数；sell 的空值是「卖出全部持仓」这个确定语义，不是缺值 */}
            {(actionType === "buy" || actionType === "reduce") && (
              <Form.Item name="actionValue" rules={[{ required: true }]} noStyle>
                <InputNumber
                  size="small"
                  style={{ width: 96 }}
                  min={actionType === "buy" ? 1 : 0.01}
                  max={actionType === "reduce" ? 1 : undefined}
                  step={actionType === "buy" ? 100 : 0.05}
                  placeholder={t(
                    actionType === "buy"
                      ? "stockAnalysis.conditional.actionValueQty"
                      : "stockAnalysis.conditional.actionValueRatio",
                  )}
                />
              </Form.Item>
            )}
            <Form.Item name="coolDownMinutes" noStyle>
              <InputNumber
                size="small"
                style={{ width: 84 }}
                min={0}
                placeholder={t("stockAnalysis.conditional.coolDown")}
              />
            </Form.Item>
            <Form.Item name="maxTriggersPerDay" noStyle>
              <InputNumber
                size="small"
                style={{ width: 84 }}
                min={0}
                placeholder={t("stockAnalysis.conditional.maxPerDay")}
              />
            </Form.Item>
            <Form.Item name="activeStart" noStyle>
              <Input size="small" style={{ width: 88 }} placeholder={t("stockAnalysis.conditional.windowStart")} />
            </Form.Item>
            <Form.Item name="activeEnd" noStyle>
              <Input size="small" style={{ width: 88 }} placeholder={t("stockAnalysis.conditional.windowEnd")} />
            </Form.Item>
            <Form.Item noStyle>
              <Button type="primary" htmlType="submit" size="small">{t("stockAnalysis.conditional.create")}</Button>
            </Form.Item>
          </div>
        </Form>
      )}

      {loadError
        ? (
          <Alert
            type="error"
            showIcon
            title={t("stockAnalysis.conditional.loadFailed", { reason: loadError })}
          />
        )
        : (
          <>
            {view.unreadable.length > 0 && (
              <Alert
                type="warning"
                showIcon
                className="mb-1"
                title={t("stockAnalysis.conditional.unreadable", { count: view.unreadable.length })}
                description={view.unreadable.map((u) => `${u.orderId}: ${u.reason}`).join(" / ")}
              />
            )}
            <Table
              dataSource={view.items}
              columns={columns}
              rowKey="orderId"
              size="small"
              pagination={false}
              locale={{ emptyText: t("stockAnalysis.conditional.empty") }}
            />
            {view.items.length > 0 && (
              <div className="text-xs opacity-60 mt-1">
                {t("stockAnalysis.conditional.totalRows", { count: view.items.length })}
              </div>
            )}
          </>
        )}
    </Card>
  );
}
