import { translateBackendError } from "@/lib/errorI18n";
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import type { ConditionTriggerPayload, MonitorAlertPayload, MonitorStatus } from "@/types";
import { BellOutlined, DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { Alert, App, Badge, Button, Card, Form, Input, InputNumber, Select, Switch, Table, Tabs, Tag } from "antd";
import type { TFunction } from "i18next";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface PriceAlert {
  id: string;
  stockCode: string;
  stockName: string;
  /** 兼容列（6→5 有损，**不得**作为展示依据）：只在 `alertType` 为空的老行上回落 */
  condition: string;
  /** 老字段：仅 price 类告警有效；新代码读 `threshold` */
  targetPrice: number;
  /** 权威类型（v203）：stop_loss / take_profit / resistance / support / change / volume */
  alertType: string | null;
  /** 阈值语义（v203）：price / change_pct / turnover_rate */
  conditionType: string | null;
  /** 语义阈值（v203）：价格类=绝对价，另两类=百分数 */
  threshold: number | null;
  /** 后端是 `i32`（0/1），**不是 boolean** —— 声明成 boolean 会让真值判断与类型检查一起失真 */
  isTriggered: 0 | 1;
  triggeredAt: number | null;
  createdAt: number;
}

/** 兼容列 → 权威类型的回落映射（与后端 `legacy_condition_to_alert_type` 同序）。 */
const LEGACY_CONDITION_TO_TYPE: Record<string, string> = {
  above: "take_profit",
  below: "stop_loss",
  change_up: "change",
  change_down: "change",
  volume_spike: "volume",
};

/** 权威类型：优先 `alertType`；v203 之前的老行按兼容列回落。 */
export function canonicalAlertTypeOf(
  alert: Pick<PriceAlert, "alertType" | "condition">,
): string | undefined {
  if (alert.alertType) { return alert.alertType; }
  return LEGACY_CONDITION_TO_TYPE[alert.condition];
}

/** 阈值语义：老行（`conditionType` 为空）只有价格类阈值。 */
export function conditionTypeOf(alert: Pick<PriceAlert, "conditionType">): string {
  return alert.conditionType ?? "price";
}

/** 语义阈值：优先新列，老行回落 `targetPrice`。 */
export function thresholdOf(alert: Pick<PriceAlert, "threshold" | "targetPrice">): number {
  return alert.threshold ?? alert.targetPrice;
}

/**
 * 后端 `MonitorAlert.alert_type` 的六类 → i18n 键。
 * 未知值一律落 `typeUnknown`，不把「没见过的类型」当成六类中的某一类。
 */
const ALERT_TYPE_KEYS: Record<string, string> = {
  stop_loss: "stockAnalysis.alert.typeStopLoss",
  take_profit: "stockAnalysis.alert.typeTakeProfit",
  resistance: "stockAnalysis.alert.typeResistance",
  support: "stockAnalysis.alert.typeSupport",
  change: "stockAnalysis.alert.typeChange",
  volume: "stockAnalysis.alert.typeVolume",
};

export function alertTypeLabel(t: TFunction, alertType: string | undefined): string {
  const key = alertType ? ALERT_TYPE_KEYS[alertType] : undefined;
  return key ? t(key) : t("stockAnalysis.alert.typeUnknown");
}

/** 时段档位码 → i18n 键。未知码落 `sessionUnknown`，不把没见过的档位塞进已知那一档。 */
const SESSION_KEYS: Record<string, string> = {
  non_trading_day: "stockAnalysis.monitor.sessionNonTradingDay",
  pre_open: "stockAnalysis.monitor.sessionPreOpen",
  morning: "stockAnalysis.monitor.sessionMorning",
  lunch_break: "stockAnalysis.monitor.sessionLunchBreak",
  afternoon: "stockAnalysis.monitor.sessionAfternoon",
  post_close: "stockAnalysis.monitor.sessionPostClose",
};

export function monitorSessionKey(code: string): string {
  return SESSION_KEYS[code] ?? "stockAnalysis.monitor.sessionUnknown";
}

/**
 * 读数面板的三态。`readFailed` 与「未装配」是两件事：命令本身报错时不能报成
 * 「监控引擎未装配」，否则一次 IPC 故障会被读成一条结构性缺失。
 */
export type MonitorStatusState =
  | { kind: "loading" }
  | { kind: "readFailed"; reason: string }
  | { kind: "ok"; value: MonitorStatus };

export interface MonitorSentence {
  key: string;
  params: Record<string, unknown>;
  level: "error" | "warning" | "info";
}

/**
 * 缺席句子清单 —— 一条一句，互不合并、不相加成一个数（PLAN §9 H-4）。
 *
 * 顺序固定：读失败 → 未装配 →（时段停摆 / 本轮取数失败 / 连续全失败 / 首轮未完成）。
 * 前两条各自独占：未装配时计数全是 0，再说一句「本轮 0/0 失败」就是把「没人在取数」
 * 演成「没有失败」。
 */
export function monitorAbsenceSentences(state: MonitorStatusState): MonitorSentence[] {
  if (state.kind === "loading") { return []; }
  if (state.kind === "readFailed") {
    return [{ key: "stockAnalysis.monitor.readFailed", level: "error", params: { reason: state.reason } }];
  }
  const s = state.value;
  if (!s.assembled) {
    return [{ key: "stockAnalysis.monitor.notAssembled", level: "error", params: {} }];
  }
  const out: MonitorSentence[] = [];
  if (!s.tradingNow) {
    out.push({
      key: "stockAnalysis.monitor.pausedBySession",
      level: "warning",
      params: { sessionKey: monitorSessionKey(s.session) },
    });
  }
  if (s.roundCodesFailed > 0) {
    out.push({
      key: "stockAnalysis.monitor.fetchFailed",
      level: "warning",
      params: { failed: s.roundCodesFailed, total: s.roundCodesTotal },
    });
  }
  // 连击要两轮以上才单独立一句：一轮的失败上一句已经说过，凑成两句会把同一件事数成两个问题
  if (s.consecutiveAllFailedRounds >= 2) {
    out.push({
      key: "stockAnalysis.monitor.allFailedRounds",
      level: "error",
      params: { rounds: s.consecutiveAllFailedRounds },
    });
  }
  if (s.tradingNow && s.configsActive > 0 && s.roundFinishedAtMs === 0) {
    out.push({
      key: "stockAnalysis.monitor.firstRoundPending",
      level: "info",
      params: { interval: s.pollIntervalSecs },
    });
  }
  return out;
}

export function PriceAlertPanel() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const [alerts, setAlerts] = useState<PriceAlert[]>([]);
  const [history, setHistory] = useState<PriceAlert[]>([]);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();
  const [activeTab, setActiveTab] = useState("active");
  const [monitorState, setMonitorState] = useState<MonitorStatusState>({ kind: "loading" });
  // H-2：建单表单里那一格同时装价格与百分比 ⇒ 单位随所选条件变（换手率/涨跌幅按 % 读）
  const watchedCondition = Form.useWatch("condition", form) as string | undefined;
  const formIsPercent = watchedCondition === "change_up" || watchedCondition === "change_down"
    || watchedCondition === "volume_spike";
  // 裁定 4 = ①：T+0 自动重跑面板开关。链路此前**已全通**（引擎 maybe_trigger_t0 读配置、
  // services.rs 注入了后端重跑 callback、前端 listener 在 store 里），只差这一个入口 ——
  // store 的两个方法此前零组件调用者，等于「用户看不见的功能」。
  const t0 = useStockAnalysisStore((s) => s.t0Config);
  const t0Loading = useStockAnalysisStore((s) => s.t0Loading);
  const setT0Config = useStockAnalysisStore((s) => s.setT0Config);
  const fetchT0Config = useStockAnalysisStore((s) => s.fetchT0Config);

  /** 保存失败必须说出来：静默吞掉会让面板显示「已开启」而后端没收到（store 已回滚乐观态）。 */
  const applyT0 = async (patch: Parameters<typeof setT0Config>[0]) => {
    try {
      await setT0Config(patch);
    } catch (e) {
      message.error(
        t("stockAnalysis.alert.t0.saveFailed", {
          reason: e instanceof Error ? e.message : String(e),
        }),
      );
    }
  };

  /** 取数读数单独一条路：命令报错是「读不到」，不等于「引擎未装配」。 */
  const loadMonitorStatus = async () => {
    try {
      const s = await invoke<MonitorStatus>("monitor_status");
      if (s && typeof s.assembled === "boolean") {
        setMonitorState({ kind: "ok", value: s });
      } else {
        setMonitorState({ kind: "readFailed", reason: t("stockAnalysis.monitor.emptyResponse") });
      }
    } catch (e) {
      setMonitorState({ kind: "readFailed", reason: e instanceof Error ? e.message : String(e) });
    }
  };

  useEffect(() => {
    void loadMonitorStatus();
    void fetchT0Config();
    // 只在挂载时取一次；建单/删单/告警触发后经 loadAlerts 一并刷新
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const loadAlerts = async () => {
    try {
      const list = await invoke<PriceAlert[]>("list_price_alerts");
      if (Array.isArray(list)) {
        setAlerts(list.filter((a) => !a.isTriggered));
        setHistory(list.filter((a) => a.isTriggered));
      }
    } catch (e) {
      // H-1：读不到列表要说出来（此前静默吞成空列表 ⇒ 「没有告警」与「读失败」同屏）
      message.error(translateBackendError(e));
    }
    // 建单/删单都会改「本轮应取数几只」，读数跟着一起刷新，否则面板会继续报上一轮的数
    void loadMonitorStatus();
  };

  useEffect(() => {
    // 挂载时走**同一个** loadAlerts（此前这里另抄了一遍「取列表 + 按 isTriggered 分流」：
    // 两处逻辑各改一处就分叉，且上面的 H-3 判据只会打在其中一个上）。
    void loadAlerts();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen<MonitorAlertPayload>("price-alert-triggered", (event) => {
      if (cancelled) { return; }
      const alert = event.payload;
      message.info(
        t("stockAnalysis.alert.triggeredDetail", {
          name: alert.stockName || alert.stockCode,
          type: alertTypeLabel(t, alert.alertType),
          price: alert.currentPrice.toFixed(2),
        }),
      );
      loadAlerts();
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {/* 可选 */});
    return () => {
      if (unlisten) { unlisten(); }
      cancelled = true;
    };
  }, [t, message]);

  // 条件单触发（`conditional_orders` 那条链）与价格告警是两个实体：它不写 price_alerts，
  // 所以这里不 reload 告警列表，只把「哪条规则、什么条件、要做什么」说出来。
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen<ConditionTriggerPayload>("conditional-order-triggered", (event) => {
      if (cancelled) { return; }
      const trigger = event.payload;
      message.info(
        t("stockAnalysis.conditional.triggeredDetail", {
          name: trigger.stockName || trigger.stockCode,
          condition: trigger.condition,
          action: trigger.action,
        }),
      );
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {/* 可选 */});
    return () => {
      if (unlisten) { unlisten(); }
      cancelled = true;
    };
  }, [t, message]);

  const addAlert = async (values: Record<string, unknown>) => {
    try {
      await invoke("create_price_alert", {
        stockCode: values.stockCode,
        stockName: values.stockName,
        condition: values.condition,
        targetPrice: values.targetPrice,
      });
      form.resetFields();
      setAdding(false);
      loadAlerts();
    } catch (e) {
      // H-1：建单失败此前完全静默（用户以为建上了）
      message.error(translateBackendError(e));
    }
  };

  const removeAlert = async (id: string) => {
    try {
      await invoke("delete_price_alert", { id });
      loadAlerts();
    } catch (e) {
      // H-1：删单失败此前完全静默（行还在，用户以为删了）
      message.error(translateBackendError(e));
    }
  };

  const columns = [
    {
      title: t("stockAnalysis.alert.code"),
      dataIndex: "stockCode",
      width: 60,
      render: (v: string) => <Tag className="text-xs m-0">{v}</Tag>,
    },
    { title: t("stockAnalysis.alert.name"), dataIndex: "stockName", ellipsis: true as const },
    {
      // H-2：类型列读**权威 `alertType`**（老行按兼容列回落），不再直接显示兼容列的英文 token；
      // 方向也由类型判 —— 兼容列 6→5 有损（resistance 与 take_profit 同写 above）
      title: t("stockAnalysis.alert.condition"),
      key: "alertType",
      width: 86,
      render: (_: unknown, record: PriceAlert) => {
        const canonical = canonicalAlertTypeOf(record);
        const below = canonical === "stop_loss" || canonical === "support";
        const isPrice = conditionTypeOf(record) === "price";
        return (
          <span className="text-xs">
            {alertTypeLabel(t, canonical)}
            <Tag color={below ? "green" : "red"} className="text-xs m-0 ml-1">
              {isPrice ? (below ? "≤" : "≥") : (below ? "<" : ">")}
            </Tag>
          </span>
        );
      },
    },
    {
      // H-2：阈值列按 `conditionType` 分列单位 —— 一格同时装价格与百分比时，
      // 涨幅告警的 5 会被渲染成「5.00」当成价格读
      title: t("stockAnalysis.alert.price"),
      key: "threshold",
      width: 76,
      align: "right" as const,
      render: (_: unknown, record: PriceAlert) => {
        const v = thresholdOf(record);
        return conditionTypeOf(record) === "price" ? v.toFixed(2) : `${v.toFixed(2)}%`;
      },
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

  const historyColumns = [
    ...columns.slice(0, 4),
    {
      title: t("stockAnalysis.alert.triggerTime"),
      dataIndex: "triggeredAt",
      width: 80,
      render: (v: number | null) => v ? new Date(v).toLocaleDateString() : "—",
    },
    columns[4],
  ];

  return (
    <Card
      size="small"
      styles={{ body: { padding: "8px 8px" } }}
      title={
        <span>
          <BellOutlined /> {t("stockAnalysis.alert.title")}
        </span>
      }
      extra={
        <div className="flex gap-1 items-center">
          <Badge
            status={alerts.length > 0 ? "processing" : "default"}
            text={alerts.length > 0
              ? t("stockAnalysis.alert.activeWithCount", { count: alerts.length })
              : t("stockAnalysis.alert.none")}
          />
          <Button size="small" icon={<PlusOutlined />} onClick={() => setAdding(!adding)} />
        </div>
      }
    >
      {monitorAbsenceSentences(monitorState).map((sent) => (
        <Alert
          data-testid="monitor-absence"
          key={sent.key}
          type={sent.level}
          showIcon
          className="mb-1"
          title={t(sent.key, {
            ...sent.params,
            ...(typeof sent.params.sessionKey === "string"
              ? { session: t(sent.params.sessionKey) }
              : {}),
          })}
        />
      ))}
      {
        /* T+0 自动重跑：默认关（引擎侧 `TZeroConfig::default().enabled = false`，面板不替它开）。
          成本句**上屏常显**而不是塞进 tooltip —— 「开启要花多少」属于开关之前就该知道的信息。 */
      }
      <div className="flex gap-2 items-center flex-wrap text-xs" data-testid="t0-settings">
        <Switch
          size="small"
          checked={t0.enabled}
          loading={t0Loading}
          data-testid="t0-switch"
          onChange={(v) => void applyT0({ enabled: v })}
        />
        <span>{t("stockAnalysis.alert.t0.title")}</span>
        {t0.enabled && (
          <>
            <InputNumber
              size="small"
              style={{ width: 96 }}
              min={0.1}
              step={0.5}
              value={t0.changePctThreshold ?? undefined}
              addonBefore={t("stockAnalysis.alert.t0.changePct")}
              data-testid="t0-change-pct"
              onChange={(v) => void applyT0({ changePctThreshold: typeof v === "number" ? v : null })}
            />
            <InputNumber
              size="small"
              style={{ width: 96 }}
              min={0.1}
              step={0.5}
              value={t0.turnoverRateThreshold ?? undefined}
              addonBefore={t("stockAnalysis.alert.t0.turnover")}
              data-testid="t0-turnover"
              onChange={(v) => void applyT0({ turnoverRateThreshold: typeof v === "number" ? v : null })}
            />
            <InputNumber
              size="small"
              style={{ width: 108 }}
              min={1}
              step={5}
              value={t0.minIntervalMinutes}
              addonBefore={t("stockAnalysis.alert.t0.intervalMin")}
              data-testid="t0-interval"
              onChange={(v) => {
                if (typeof v === "number") { void applyT0({ minIntervalMinutes: v }); }
              }}
            />
          </>
        )}
      </div>
      <div className="text-xs opacity-60 mb-1" data-testid="t0-cost">
        {t("stockAnalysis.alert.t0.cost")}
      </div>
      {adding && (
        <Form form={form} size="small" onFinish={addAlert} className="mb-2">
          <div className="flex gap-1 mb-1 flex-wrap">
            <Form.Item name="stockCode" rules={[{ required: true }]} noStyle>
              <Input placeholder={t("stockAnalysis.alert.code")} style={{ width: 72 }} size="small" />
            </Form.Item>
            <Form.Item name="stockName" rules={[{ required: true }]} noStyle>
              <Input placeholder={t("stockAnalysis.alert.name")} style={{ width: 72 }} size="small" />
            </Form.Item>
            <Form.Item name="condition" initialValue="above" noStyle>
              <Select
                style={{ width: 80 }}
                size="small"
                options={[
                  { value: "above", label: t("stockAnalysis.alert.conditionAbove") },
                  { value: "below", label: t("stockAnalysis.alert.conditionBelow") },
                  { value: "change_up", label: t("stockAnalysis.alert.conditionUp") },
                  { value: "change_down", label: t("stockAnalysis.alert.conditionDown") },
                  { value: "volume_spike", label: t("stockAnalysis.alert.conditionVolume") },
                ]}
              />
            </Form.Item>
          </div>
          <div className="flex gap-1">
            <Form.Item name="targetPrice" rules={[{ required: true }]} noStyle>
              <InputNumber
                size="small"
                placeholder={t("stockAnalysis.alert.price")}
                style={{ width: 100 }}
                min={0}
                step={0.01}
                // H-2：同一格装价格与百分比 ⇒ 按所选条件给单位后缀（符号语言无关，不进 i18n）
                addonAfter={formIsPercent ? "%" : undefined}
              />
            </Form.Item>
            <Form.Item noStyle>
              <Button type="primary" htmlType="submit" size="small">{t("common.confirm")}</Button>
            </Form.Item>
          </div>
        </Form>
      )}
      <Tabs
        size="small"
        activeKey={activeTab}
        onChange={setActiveTab}
        items={[
          {
            key: "active",
            label: t("stockAnalysis.alert.activeTab", { count: alerts.length }),
            children: (
              <Table
                dataSource={alerts}
                columns={columns}
                rowKey="id"
                size="small"
                pagination={false}
                locale={{ emptyText: t("stockAnalysis.alert.empty") }}
              />
            ),
          },
          {
            key: "history",
            label: t("stockAnalysis.alert.triggeredTab", { count: history.length }),
            children: (
              <Table
                dataSource={history.slice(0, 20)}
                columns={historyColumns}
                rowKey="id"
                size="small"
                pagination={false}
                locale={{ emptyText: t("stockAnalysis.alert.noHistory") }}
              />
            ),
          },
        ]}
      />
    </Card>
  );
}
