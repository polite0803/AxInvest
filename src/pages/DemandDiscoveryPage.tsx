// SPDX-License-Identifier: AGPL-3.0-only

import { invoke, listen } from "@/lib/invoke";
import { openExternal } from "@/lib/openExternal";
import type {
  CapabilityMatchItem,
  DeliveryInvoice,
  DeliverySummary,
  DemandLead,
  DemandPlatform,
  DemandSubscription,
  DiscoverLeadsSummary,
  LeadCapabilityMatch,
  SaveDemandLeadInput,
  SaveDemandPlatformInput,
  SaveDemandSubscriptionInput,
  ScanPolicy,
  SubscriptionScanSummary,
} from "@/types";
import { DownOutlined } from "@ant-design/icons";
import {
  Alert,
  App,
  Button,
  Card,
  Col,
  Descriptions,
  Dropdown,
  Empty,
  Flex,
  Form,
  Input,
  InputNumber,
  List,
  Modal,
  Progress,
  Row,
  Segmented,
  Select,
  Space,
  Statistic,
  Switch,
  Table,
  Tag,
  theme,
  Timeline,
  Tooltip,
  Typography,
} from "antd";
import type { MenuProps } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type CapabilityEntry,
  type CapabilityGap,
  type CapabilityInventory,
  type Delivery,
  DELIVERY_STATUS_COLOR_MAP,
} from "./opc/utils/constants";

/** 连接器类型（与后端 add_platform 的 platform_type 一致） */
const PLATFORM_TYPES = ["scanner", "api", "mock", "manual"] as const;

/** 连接器状态 → Tag 颜色 */
const STATUS_COLOR: Record<string, string> = {
  ok: "green",
  error: "red",
  idle: "default",
  skipped: "orange",
};

/** 线索生命周期 → Tag 颜色 */
const LEAD_STATUS_COLOR: Record<string, string> = {
  new: "blue",
  evaluated: "geekblue",
  contacted: "cyan",
  won: "green",
  lost: "red",
};

/** 线索生命周期合法下一步动作（与后端 is_legal_status_transition 对齐） */
const NEXT_STATUS_ACTIONS: Record<string, Array<{ status: string; i18nKey: string }>> = {
  new: [
    { status: "contacted", i18nKey: "opc.demand.markContacted" },
    { status: "lost", i18nKey: "opc.demand.markLost" },
  ],
  evaluated: [
    { status: "contacted", i18nKey: "opc.demand.markContacted" },
    { status: "lost", i18nKey: "opc.demand.markLost" },
  ],
  contacted: [
    { status: "won", i18nKey: "opc.demand.markWon" },
    { status: "lost", i18nKey: "opc.demand.markLost" },
  ],
};

/** 生命周期状态全集（与后端枚举一致） */
const LEAD_STATUSES = ["new", "evaluated", "contacted", "won", "lost"] as const;

/** 能力匹配结论 → Tag 颜色 */
const VERDICT_COLOR: Record<string, string> = {
  ready: "green",
  partial: "gold",
  missing: "red",
};

/** 订阅扫描默认 cron（与后端 DEFAULT_SCAN_CRON 一致：每 6 小时） */
const DEFAULT_SCAN_CRON = "0 */6 * * *";

/** 商业价值分 → 文字颜色（≥80 极高 / ≥60 高 / ≥40 中 / 其余低） */
function scoreColor(
  score: number,
  token: { colorError: string; colorWarning: string; colorSuccess: string; colorTextTertiary: string },
): string {
  if (score >= 80) {
    return token.colorError;
  }
  if (score >= 60) {
    return token.colorWarning;
  }
  if (score >= 40) {
    return token.colorSuccess;
  }
  return token.colorTextTertiary;
}

function formatBudget(lead: DemandLead): string {
  const { budgetMin, budgetMax, budgetCurrency } = lead;
  if (budgetMin === null && budgetMax === null) {
    return "—";
  }
  const symbol = budgetCurrency === "CNY" ? "¥" : budgetCurrency === "USD" ? "$" : `${budgetCurrency} `;
  if (budgetMin !== null && budgetMax !== null) {
    return `${symbol}${budgetMin} – ${symbol}${budgetMax}`;
  }
  const only = budgetMin ?? budgetMax;
  return `${symbol}${only}`;
}

function formatTs(ts: number | null): string {
  if (ts === null || ts <= 0) {
    return "—";
  }
  return new Date(ts * 1000).toLocaleString();
}

interface PlatformFormValues {
  id?: string;
  name: string;
  platformType: string;
  baseUrl?: string;
  /** API Token（写入 config_json.api_token，扫描器凭证三层断链修复） */
  apiToken?: string;
  enabled: boolean;
}

/** 订阅表单值（平台多选为空数组 = 跟随全局启用的平台） */
interface SubscriptionFormValues {
  keyword: string;
  intervalHours: number;
  minScore: number;
  platforms: string[];
}

/** 手动补录表单值（P1-4；预算与联系方式均可选） */
interface LeadFormValues {
  title: string;
  description: string;
  budgetMin?: number;
  budgetMax?: number;
  budgetCurrency?: string;
  contactName?: string;
  contactEmail?: string;
  contactPhone?: string;
  sourceUrl?: string;
}

/** 扫描策略单字段：label + 数值输入（增强项 9 的 UI 原子） */
function PolicyField(props: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number | null) => void;
}) {
  const { label, value, min, max, step, onChange } = props;
  return (
    <Flex vertical gap={4}>
      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
        {label}
      </Typography.Text>
      <InputNumber
        size="small"
        style={{ width: 150 }}
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={onChange}
      />
    </Flex>
  );
}

export function DemandDiscoveryPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();

  const [tab, setTab] = useState<
    "leads" | "platforms" | "subscriptions" | "delivery" | "capabilities" | "deliveries"
  >("leads");
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [proactiveScanning, setProactiveScanning] = useState(false);

  const [query, setQuery] = useState("");
  const [minScore, setMinScore] = useState<number>(0);
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [leads, setLeads] = useState<DemandLead[]>([]);
  const [platforms, setPlatforms] = useState<DemandPlatform[]>([]);
  const [summary, setSummary] = useState<DiscoverLeadsSummary | null>(null);

  const [editing, setEditing] = useState<DemandPlatform | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [form] = Form.useForm<PlatformFormValues>();

  // ── 订阅（v133，定时扫描）────────────────────────────────────────────
  const [subscriptions, setSubscriptions] = useState<DemandSubscription[]>([]);
  const [invoices, setInvoices] = useState<DeliveryInvoice[]>([]);
  const [deliverySummary, setDeliverySummary] = useState<DeliverySummary | null>(null);
  const [subEditing, setSubEditing] = useState<DemandSubscription | null>(null);
  const [subModalOpen, setSubModalOpen] = useState(false);
  const [subSaving, setSubSaving] = useState(false);
  const [subScanning, setSubScanning] = useState(false);
  const [subSummary, setSubSummary] = useState<SubscriptionScanSummary | null>(null);
  const [scanCron, setScanCron] = useState(DEFAULT_SCAN_CRON);
  const [subForm] = Form.useForm<SubscriptionFormValues>();

  // ── 能力匹配（P3，给「响应」环节做判断依据）─────────────────────────
  const [capMatch, setCapMatch] = useState<LeadCapabilityMatch | null>(null);
  const [capModalOpen, setCapModalOpen] = useState(false);

  // ── 手动补录（P1-4）─────────────────────────────────────────────────
  const [leadModalOpen, setLeadModalOpen] = useState(false);
  const [leadSaving, setLeadSaving] = useState(false);
  const [leadForm] = Form.useForm<LeadFormValues>();

  // ── 扫描策略（增强项 9：并发/限流/重试/去重窗口 UI 可配置）───────────
  const [scanPolicy, setScanPolicy] = useState<ScanPolicy | null>(null);
  const [policySaving, setPolicySaving] = useState(false);
  const [capMatching, setCapMatching] = useState(false);

  // ── 能力驱动扫描（原方案 workflow 形态：能力集扫描 → Agent 检索词 → Loop 逐词扫描）──
  const [capScanOpen, setCapScanOpen] = useState(false);
  const [capFocus, setCapFocus] = useState("");
  const [capMaxKeywords, setCapMaxKeywords] = useState<number>(5);
  const [capScanning, setCapScanning] = useState(false);
  /** 完成事件监听的反注册（页面卸载 / 重复触发时清理） */
  const capUnlistenRef = useRef<(() => void) | null>(null);
  useEffect(
    () => () => {
      capUnlistenRef.current?.();
      capUnlistenRef.current = null;
    },
    [],
  );

  const loadLeads = useCallback(async () => {
    setLoading(true);
    try {
      const rows = await invoke<DemandLead[]>("opc_list_leads", {
        limit: 200,
        minScore: minScore > 0 ? minScore : null,
        status: statusFilter ?? null,
      });
      setLeads(rows ?? []);
    } catch (e) {
      message.error(t("opc.demand.loadFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [minScore, statusFilter, message, t]);

  const loadPlatforms = useCallback(async () => {
    try {
      const rows = await invoke<DemandPlatform[]>("opc_list_platforms");
      setPlatforms(rows ?? []);
    } catch (e) {
      message.error(t("opc.demand.loadFailed", { error: String(e) }));
    }
  }, [message, t]);

  const loadSubscriptions = useCallback(async () => {
    try {
      const rows = await invoke<DemandSubscription[]>("opc_list_subscriptions");
      setSubscriptions(rows ?? []);
    } catch (e) {
      message.error(t("opc.demand.loadFailed", { error: String(e) }));
    }
  }, [message, t]);

  /** 加载扫描策略（增强项 9：并发/限流/重试/去重窗口） */
  const loadScanPolicy = useCallback(async () => {
    try {
      setScanPolicy(await invoke<ScanPolicy>("opc_get_scan_policy"));
    } catch (e) {
      message.error(t("opc.demand.policyLoadFailed", { error: String(e) }));
    }
  }, [message, t]);

  const saveScanPolicy = useCallback(async () => {
    if (!scanPolicy) { return; }
    setPolicySaving(true);
    try {
      const saved = await invoke<ScanPolicy>("opc_save_scan_policy", { policy: scanPolicy });
      setScanPolicy(saved);
      message.success(t("opc.demand.policySaved"));
    } catch (e) {
      message.error(t("opc.demand.policySaveFailed", { error: String(e) }));
    } finally {
      setPolicySaving(false);
    }
  }, [scanPolicy, message, t]);

  /** 加载发票账本与交付汇总（P4 交付页数据源） */
  const loadInvoices = useCallback(async () => {
    try {
      const [rows, summary] = await Promise.all([
        invoke<DeliveryInvoice[]>("opc_list_invoices"),
        invoke<DeliverySummary>("opc_get_delivery_summary"),
      ]);
      setInvoices(rows ?? []);
      setDeliverySummary(summary);
    } catch (e) {
      message.error(t("opc.demand.loadFailed", { error: String(e) }));
    }
  }, [message, t]);

  /** won 线索开票（后端幂等：已有发票直接返回） */
  const createInvoice = useCallback(
    async (row: DemandLead) => {
      try {
        await invoke<DeliveryInvoice>("opc_create_invoice_from_lead", { leadId: row.id });
        message.success(t("opc.demand.invoiceCreated"));
        void loadInvoices();
      } catch (e) {
        message.error(t("opc.demand.invoiceFailed", { error: String(e) }));
      }
    },
    [loadInvoices, message, t],
  );

  /** 推进发票状态机（draft → sent → paid，后端校验单向迁移） */
  const advanceInvoice = useCallback(
    async (inv: DeliveryInvoice, status: "draft" | "sent" | "paid") => {
      try {
        await invoke<DeliveryInvoice>("opc_update_invoice_status", {
          invoiceId: inv.id,
          status,
        });
        void loadInvoices();
      } catch (e) {
        message.error(t("opc.demand.invoiceFailed", { error: String(e) }));
      }
    },
    [loadInvoices, message, t],
  );

  /** 删除发票（作废用删除代替） */
  const removeInvoice = useCallback(
    async (inv: DeliveryInvoice) => {
      try {
        await invoke("opc_delete_invoice", { invoiceId: inv.id });
        void loadInvoices();
      } catch (e) {
        message.error(t("opc.demand.invoiceFailed", { error: String(e) }));
      }
    },
    [loadInvoices, message, t],
  );

  useEffect(() => {
    void loadLeads();
    void loadPlatforms();
    void loadSubscriptions();
    void loadInvoices();
    void loadScanPolicy();
  }, [loadLeads, loadPlatforms, loadSubscriptions, loadInvoices, loadScanPolicy]);

  const runScan = useCallback(async () => {
    const kw = query.trim();
    if (!kw) {
      message.warning(t("opc.demand.enterSearchQuery"));
      return;
    }
    setScanning(true);
    try {
      const result = await invoke<DiscoverLeadsSummary>("opc_discover_and_evaluate_leads", {
        query: kw,
      });
      setSummary(result);
      await loadLeads();
      await loadPlatforms();
      message.success(
        t("opc.demand.proactiveScanComplete", {
          saved: result?.totalSaved ?? 0,
          highValue: result?.highValueCount ?? 0,
        }),
      );
    } catch (e) {
      message.error(t("opc.demand.scanFailed", { error: String(e) }));
    } finally {
      setScanning(false);
    }
  }, [loadLeads, loadPlatforms, message, query, t]);

  /**
   * 能力驱动扫描：执行 opc-demand-discovery 工作流（v5）。
   * 引擎在 tokio::spawn 中异步执行，完成以 workflow:execution-completed 事件为准；
   * 扫描产物（线索）落库后由 loadLeads / loadPlatforms 刷新。
   */
  const runCapabilityScan = useCallback(async () => {
    if (capScanning) {
      return;
    }
    // 防重复触发：先清掉上一次的监听
    capUnlistenRef.current?.();
    capUnlistenRef.current = null;
    setCapScanning(true);
    setCapScanOpen(false);
    try {
      const executionId = await invoke<string>("workflow_execute", {
        workflowId: "opc-demand-discovery",
        // Variable 字段为后端 snake_case serde（无 rename_all），与 workflowStore 先例一致
        variables: [
          { name: "focus", var_type: "string", value: capFocus.trim(), is_secret: false },
          { name: "max_keywords", var_type: "number", value: capMaxKeywords, is_secret: false },
        ],
      });
      const unlisten = await listen<{
        workflow_id: string;
        execution_id: string | null;
        status: string;
        total_time_ms: number;
        error?: string;
      }>("workflow:execution-completed", (event) => {
        const payload = event.payload;
        if (!payload || payload.workflow_id !== "opc-demand-discovery") {
          return;
        }
        // panic 兜底路径 execution_id 为 null，此时按 workflow_id 匹配即可
        if (payload.execution_id !== null && payload.execution_id !== executionId) {
          return;
        }
        capUnlistenRef.current?.();
        capUnlistenRef.current = null;
        setCapScanning(false);
        void loadLeads();
        void loadPlatforms();
        if (payload.status === "failed") {
          message.error(
            t("opc.demand.capabilityScanFailed", { error: payload.error ?? "" }),
          );
        } else {
          message.success(t("opc.demand.capabilityScanComplete"));
        }
      });
      capUnlistenRef.current = unlisten;
    } catch (e) {
      setCapScanning(false);
      message.error(t("opc.demand.capabilityScanFailed", { error: String(e) }));
    }
  }, [capScanning, capFocus, capMaxKeywords, loadLeads, loadPlatforms, message, t]);

  /** 主动评估：对现有线索批量重跑规则打分并落库（原 OPC 面板入口，合并至此） */
  const runProactiveScan = useCallback(async () => {
    setProactiveScanning(true);
    try {
      const result = await invoke<{
        total_queries: number;
        total_scanned: number;
        total_saved: number;
        high_value_count: number;
      }>("opc_proactive_evaluate_and_save_leads", { min_score: 0.0 });
      message.success(
        t("opc.demand.proactiveScanComplete", {
          saved: result.total_saved,
          highValue: result.high_value_count,
        }),
      );
      void loadLeads();
    } catch (e) {
      message.error(t("opc.demand.scanFailed", { error: String(e) }));
    } finally {
      setProactiveScanning(false);
    }
  }, [loadLeads, message, t]);

  /** 执行需求交付：为线索创建交付记录并触发交付工作流（产出 Delivery，交付记录页查看） */
  const executeDelivery = useCallback(
    async (row: DemandLead) => {
      try {
        await invoke<Delivery>("opc_execute_demand_workflow", { lead_id: row.id });
        message.success(t("opc.demand.deliveryStarted"));
      } catch (e) {
        message.error(String(e));
      }
    },
    [message, t],
  );

  const openCreate = useCallback(() => {
    setEditing(null);
    form.setFieldsValue({
      id: undefined,
      name: "",
      platformType: "scanner",
      baseUrl: undefined,
      apiToken: undefined,
      enabled: true,
    });
    setModalOpen(true);
  }, [form]);

  const openEdit = useCallback(
    (row: DemandPlatform) => {
      setEditing(row);
      form.setFieldsValue({
        id: row.id,
        name: row.name,
        platformType: row.platformType,
        baseUrl: row.baseUrl ?? undefined,
        apiToken: typeof row.config?.api_token === "string" && row.config.api_token
          ? row.config.api_token
          : undefined,
        enabled: row.enabled,
      });
      setModalOpen(true);
    },
    [form],
  );

  const submitPlatform = useCallback(async () => {
    const values = await form.validateFields();
    const input: SaveDemandPlatformInput = {
      ...(values.id ? { id: values.id } : {}),
      name: values.name,
      platformType: values.platformType,
      // 后端用空串表达「清空 base_url」，null/undefined 表示「不改」
      baseUrl: values.baseUrl ?? "",
      enabled: values.enabled,
      // 凭证合并进 config_json（保留既有扩展字段）；空串 = 清除已存 token
      config: {
        ...editing?.config,
        api_token: values.apiToken ?? "",
      },
    };
    setSaving(true);
    try {
      await invoke<DemandPlatform>("opc_save_platform", { input });
      message.success(t("opc.demand.platformSaved"));
      setModalOpen(false);
      await loadPlatforms();
    } catch (e) {
      message.error(t("opc.demand.saveFailed", { error: String(e) }));
    } finally {
      setSaving(false);
    }
  }, [editing, form, loadPlatforms, message, t]);

  const removePlatform = useCallback(
    (row: DemandPlatform) => {
      Modal.confirm({
        title: t("opc.demand.confirmDelete"),
        content: row.name,
        okText: t("common.delete"),
        cancelText: t("common.cancel"),
        okButtonProps: { danger: true },
        onOk: async () => {
          try {
            await invoke<void>("opc_delete_platform", { id: row.id });
            message.success(t("opc.demand.platformDeleted"));
            await loadPlatforms();
          } catch (e) {
            message.error(t("opc.demand.deleteFailed", { error: String(e) }));
          }
        },
      });
    },
    [loadPlatforms, message, t],
  );

  const togglePlatform = useCallback(
    async (row: DemandPlatform, enabled: boolean) => {
      try {
        await invoke<DemandPlatform>("opc_save_platform", {
          input: { id: row.id, enabled },
        });
        await loadPlatforms();
      } catch (e) {
        message.error(t("opc.demand.saveFailed", { error: String(e) }));
      }
    },
    [loadPlatforms, message, t],
  );

  // ── 手动补录（P1-4）─────────────────────────────────────────────────
  const openCreateLead = useCallback(() => {
    leadForm.resetFields();
    leadForm.setFieldsValue({ budgetCurrency: "CNY" });
    setLeadModalOpen(true);
  }, [leadForm]);

  const submitLead = useCallback(async () => {
    const values = await leadForm.validateFields();
    const input: SaveDemandLeadInput = {
      title: values.title,
      description: values.description,
      budgetMin: values.budgetMin ?? null,
      budgetMax: values.budgetMax ?? null,
      budgetCurrency: values.budgetCurrency ?? null,
      contactName: values.contactName ?? null,
      contactEmail: values.contactEmail ?? null,
      contactPhone: values.contactPhone ?? null,
      sourceUrl: values.sourceUrl ?? null,
    };
    setLeadSaving(true);
    try {
      await invoke<DemandLead>("opc_create_lead", { input });
      message.success(t("opc.demand.leadCreated"));
      setLeadModalOpen(false);
      await loadLeads();
    } catch (e) {
      message.error(t("opc.demand.leadCreateFailed", { error: String(e) }));
    } finally {
      setLeadSaving(false);
    }
  }, [leadForm, loadLeads, message, t]);

  // ── 订阅操作 ────────────────────────────────────────────────────────
  const openCreateSubscription = useCallback(() => {
    setSubEditing(null);
    subForm.setFieldsValue({
      keyword: "",
      intervalHours: 6,
      minScore: 60,
      platforms: [],
    });
    setSubModalOpen(true);
  }, [subForm]);

  const openEditSubscription = useCallback(
    (row: DemandSubscription) => {
      setSubEditing(row);
      subForm.setFieldsValue({
        keyword: row.keyword,
        intervalHours: row.intervalHours,
        minScore: row.minScore,
        platforms: row.platforms,
      });
      setSubModalOpen(true);
    },
    [subForm],
  );

  const submitSubscription = useCallback(async () => {
    const values = await subForm.validateFields();
    const input: SaveDemandSubscriptionInput = {
      ...(subEditing ? { id: subEditing.id } : {}),
      keyword: values.keyword,
      intervalHours: values.intervalHours,
      minScore: values.minScore,
      platforms: values.platforms ?? [],
    };
    setSubSaving(true);
    try {
      await invoke<DemandSubscription>("opc_save_subscription", { input });
      message.success(t("opc.demand.subscriptionSaved"));
      setSubModalOpen(false);
      await loadSubscriptions();
    } catch (e) {
      message.error(t("opc.demand.saveFailed", { error: String(e) }));
    } finally {
      setSubSaving(false);
    }
  }, [loadSubscriptions, message, subEditing, subForm, t]);

  const removeSubscription = useCallback(
    (row: DemandSubscription) => {
      Modal.confirm({
        title: t("opc.demand.confirmDeleteSubscription"),
        okText: t("common.delete"),
        cancelText: t("common.cancel"),
        okButtonProps: { danger: true },
        onOk: async () => {
          try {
            await invoke<void>("opc_delete_subscription", { id: row.id });
            message.success(t("opc.demand.subscriptionDeleted"));
            await loadSubscriptions();
          } catch (e) {
            message.error(t("opc.demand.deleteFailed", { error: String(e) }));
          }
        },
      });
    },
    [loadSubscriptions, message, t],
  );

  const toggleSubscription = useCallback(
    async (row: DemandSubscription, enabled: boolean) => {
      try {
        await invoke<DemandSubscription>("opc_save_subscription", {
          input: { id: row.id, enabled },
        });
        await loadSubscriptions();
      } catch (e) {
        message.error(t("opc.demand.saveFailed", { error: String(e) }));
      }
    },
    [loadSubscriptions, message, t],
  );

  /** 立即扫一遍全部启用的订阅（忽略间隔） */
  const runSubscriptionScan = useCallback(async () => {
    setSubScanning(true);
    try {
      const result = await invoke<SubscriptionScanSummary>("opc_run_subscription_scan", {
        onlyDue: false,
      });
      setSubSummary(result);
      await loadSubscriptions();
      await loadLeads();
      message.success(
        t("opc.demand.subscriptionScanComplete", {
          saved: result?.totalSaved ?? 0,
          hits: result?.highValueHits ?? 0,
        }),
      );
    } catch (e) {
      message.error(t("opc.demand.scanFailed", { error: String(e) }));
    } finally {
      setSubScanning(false);
    }
  }, [loadLeads, loadSubscriptions, message, t]);

  /** 装配/更新定时扫描任务（幂等） */
  const ensureScanJob = useCallback(async () => {
    try {
      await invoke("opc_ensure_demand_scan_job", { cronExpression: scanCron });
      message.success(t("opc.demand.scanJobReady"));
    } catch (e) {
      message.error(t("opc.demand.saveFailed", { error: String(e) }));
    }
  }, [message, scanCron, t]);

  /** 更新线索生命周期状态（P0 状态机） */
  const changeLeadStatus = useCallback(
    async (row: DemandLead, status: string) => {
      try {
        await invoke<DemandLead>("opc_update_lead_status", { leadId: row.id, status });
        message.success(t("opc.demand.statusUpdated"));
        await loadLeads();
      } catch (e) {
        message.error(t("opc.demand.statusChangeFailed", { error: String(e) }));
      }
    },
    [loadLeads, message, t],
  );

  /** 线索一键转实现工作流（P2 转化链） */
  const convertLead = useCallback(
    async (row: DemandLead) => {
      try {
        await invoke<unknown>("opc_convert_lead_to_workflow", { leadId: row.id });
        message.success(t("opc.demand.convertSuccess"));
        await loadLeads();
      } catch (e) {
        message.error(t("opc.demand.convertFailed", { error: String(e) }));
      }
    },
    [loadLeads, message, t],
  );

  /** 启动实现工作流执行 */
  const runLeadWorkflow = useCallback(
    async (row: DemandLead) => {
      try {
        await invoke<string>("opc_run_lead_workflow", { leadId: row.id });
        message.success(t("opc.demand.runSuccess"));
        await loadLeads();
      } catch (e) {
        message.error(t("opc.demand.runFailed", { error: String(e) }));
      }
    },
    [loadLeads, message, t],
  );

  /** 能力匹配：这条需求「能不能接、缺什么」（P3） */
  const matchCapabilities = useCallback(
    async (row: DemandLead) => {
      setCapMatching(true);
      // 先清空上一次结论并立即开弹窗，避免点击后无反馈或看到过期数据
      setCapMatch(null);
      setCapModalOpen(true);
      try {
        const result = await invoke<LeadCapabilityMatch>("opc_match_lead_capabilities", {
          leadId: row.id,
        });
        setCapMatch(result ?? null);
      } catch (e) {
        setCapMatch(null);
        message.error(t("opc.demand.capMatchFailed", { error: String(e) }));
      } finally {
        setCapMatching(false);
      }
    },
    [message, t],
  );

  /** 状态标记下拉菜单（按当前状态给出合法迁移项） */
  const statusMenuItems = useCallback(
    (row: DemandLead): MenuProps => ({
      items: (NEXT_STATUS_ACTIONS[row.status] ?? []).map(({ status, i18nKey }) => ({
        key: status,
        label: t(i18nKey),
      })),
      onClick: ({ key }) => void changeLeadStatus(row, key),
    }),
    [changeLeadStatus, t],
  );

  const leadColumns = useMemo(
    () => [
      {
        title: t("opc.demand.colTitle"),
        dataIndex: "title",
        key: "title",
        width: "45%",
        render: (title: string, row: DemandLead) => (
          <Flex vertical gap={2} style={{ minWidth: 0, width: "100%" }}>
            <Typography.Text
              strong
              ellipsis={{ tooltip: title }}
              style={{ display: "block", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
            >
              {title}
            </Typography.Text>
            {row.description
              ? (
                <Typography.Text
                  type="secondary"
                  ellipsis={{ tooltip: row.description }}
                  style={{
                    fontSize: 12,
                    display: "block",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {row.description}
                </Typography.Text>
              )
              : null}
          </Flex>
        ),
      },
      {
        title: t("opc.demand.colPlatform"),
        dataIndex: "platform",
        key: "platform",
        width: 110,
        render: (platform: string) => <Tag>{platform}</Tag>,
      },
      {
        title: t("opc.demand.colCommercialValue"),
        dataIndex: "commercialValueScore",
        key: "commercialValueScore",
        width: 100,
        sorter: (a: DemandLead, b: DemandLead) => a.commercialValueScore - b.commercialValueScore,
        defaultSortOrder: "descend" as const,
        render: (score: number) => (
          <Typography.Text strong style={{ color: scoreColor(score, token) }}>
            {score.toFixed(1)}
          </Typography.Text>
        ),
      },
      {
        title: t("opc.demand.colPainScore"),
        dataIndex: "painScore",
        key: "painScore",
        width: 100,
        responsive: ["xl" as const],
        render: (score: number) => score.toFixed(0),
      },
      {
        title: t("opc.demand.colMarketGap"),
        dataIndex: "marketGapScore",
        key: "marketGapScore",
        width: 100,
        responsive: ["xl" as const],
        render: (score: number) => score.toFixed(0),
      },
      {
        title: t("opc.demand.colDemandType"),
        dataIndex: "demandType",
        key: "demandType",
        width: 110,
        responsive: ["lg" as const],
        render: (demandType: string) => (
          <Tag>{t(`opc.demand.demandType.${demandType}`, { defaultValue: demandType })}</Tag>
        ),
      },
      {
        title: t("opc.demand.colStatus"),
        dataIndex: "status",
        key: "status",
        width: 100,
        render: (status: string) => (
          <Tag color={LEAD_STATUS_COLOR[status] ?? "default"}>
            {t(`opc.demand.leadStatus.${status}`, { defaultValue: status })}
          </Tag>
        ),
      },
      {
        title: t("opc.demand.colBudget"),
        key: "budget",
        width: 160,
        responsive: ["lg" as const],
        render: (_: unknown, row: DemandLead) => formatBudget(row),
      },
      {
        title: t("opc.demand.colConfidence"),
        dataIndex: "confidence",
        key: "confidence",
        width: 90,
        responsive: ["xl" as const],
        render: (confidence: number) => `${(confidence * 100).toFixed(0)}%`,
      },
      {
        title: t("opc.demand.colSource"),
        dataIndex: "sourceUrl",
        key: "sourceUrl",
        width: 90,
        render: (url: string | null) =>
          url
            ? (
              <Typography.Link
                href={url}
                target="_blank"
                rel="noreferrer"
                onClick={(e) => {
                  // Tauri WebView 会吞掉 target="_blank" 原生跳转，接管为 opener
                  e.preventDefault();
                  void openExternal(url);
                }}
              >
                {t("opc.demand.openSource")}
              </Typography.Link>
            )
            : "—",
      },
      {
        title: t("opc.demand.colEvaluatedAt"),
        dataIndex: "createdAt",
        key: "createdAt",
        width: 170,
        responsive: ["xl" as const],
        render: (ts: number) => formatTs(ts),
      },
      {
        title: t("opc.demand.colActions"),
        key: "actions",
        width: 340,
        render: (_: unknown, row: DemandLead) => (
          <Space size={4}>
            {/* 能力匹配永远可用：先看能不能接，再决定要不要转化 */}
            <Button type="link" size="small" onClick={() => void matchCapabilities(row)}>
              {t("opc.demand.capMatch")}
            </Button>
            {/* 交付线：创建交付记录并触发交付工作流（与转化实现线互补） */}
            <Button type="link" size="small" onClick={() => void executeDelivery(row)}>
              {t("opc.demand.executeDelivery")}
            </Button>
            {row.linkedWorkflowId
              ? (
                <Button type="link" size="small" onClick={() => void runLeadWorkflow(row)}>
                  {t("opc.demand.runWorkflow")}
                </Button>
              )
              : (
                <Button type="link" size="small" onClick={() => void convertLead(row)}>
                  {t("opc.demand.convertToWorkflow")}
                </Button>
              )}
            {/* won = 交付完成的入场券，开票即进入账本（P4 交付闭环入口） */}
            {row.status === "won"
              ? (
                <Button type="link" size="small" onClick={() => void createInvoice(row)}>
                  {t("opc.demand.createInvoice")}
                </Button>
              )
              : null}
            {row.linkedWorkflowId
              ? <Tag color="success">{t("opc.demand.convertedTag")}</Tag>
              : (NEXT_STATUS_ACTIONS[row.status] ?? []).length > 0
              ? (
                <Dropdown menu={statusMenuItems(row)}>
                  <Button size="small">
                    {t("opc.demand.changeStatus")}
                    <DownOutlined style={{ fontSize: 10 }} />
                  </Button>
                </Dropdown>
              )
              : null}
          </Space>
        ),
      },
    ],
    [t, token, statusMenuItems, convertLead, runLeadWorkflow, matchCapabilities, createInvoice, executeDelivery],
  );

  const platformColumns = useMemo(
    () => [
      {
        title: t("opc.demand.colName"),
        dataIndex: "name",
        key: "name",
        width: 180,
        render: (name: string, row: DemandPlatform) => (
          <Flex vertical gap={2}>
            <Typography.Text strong>{name}</Typography.Text>
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              {row.id}
            </Typography.Text>
          </Flex>
        ),
      },
      {
        title: t("opc.demand.colPlatformType"),
        dataIndex: "platformType",
        key: "platformType",
        width: 120,
        render: (platformType: string) => (
          <Tag>{t(`opc.demand.platformType.${platformType}`, { defaultValue: platformType })}</Tag>
        ),
      },
      {
        title: t("opc.demand.colBaseUrl"),
        dataIndex: "baseUrl",
        key: "baseUrl",
        width: 240,
        render: (baseUrl: string | null) =>
          baseUrl ? <Typography.Text ellipsis={{ tooltip: baseUrl }}>{baseUrl}</Typography.Text> : "—",
      },
      {
        title: t("opc.demand.colEnabled"),
        dataIndex: "enabled",
        key: "enabled",
        width: 90,
        render: (enabled: boolean, row: DemandPlatform) => (
          <Switch
            size="small"
            checked={enabled}
            onChange={(checked) => void togglePlatform(row, checked)}
          />
        ),
      },
      {
        title: t("opc.demand.colStatus"),
        dataIndex: "status",
        key: "status",
        width: 170,
        render: (status: string, row: DemandPlatform) => (
          <Flex vertical gap={2}>
            <Tooltip
              title={status === "error" && row.lastError
                ? row.lastError
                : status === "skipped"
                ? t("opc.demand.platformSkippedHint")
                : undefined}
            >
              <Tag color={STATUS_COLOR[status] ?? "default"}>
                {t(`opc.demand.platformStatus.${status}`, { defaultValue: status })}
              </Tag>
            </Tooltip>
            {status === "error" && row.lastError
              ? (
                <Typography.Text
                  type="danger"
                  style={{ fontSize: 12 }}
                  ellipsis={{ tooltip: row.lastError }}
                >
                  {row.lastError}
                </Typography.Text>
              )
              : null}
          </Flex>
        ),
      },
      {
        title: t("opc.demand.colLastSync"),
        dataIndex: "lastSyncAt",
        key: "lastSyncAt",
        width: 170,
        render: (ts: number | null) =>
          ts === null
            ? <Typography.Text type="secondary">{t("opc.demand.neverSynced")}</Typography.Text>
            : formatTs(ts),
      },
      {
        title: t("opc.demand.colActions"),
        key: "actions",
        width: 140,
        render: (_: unknown, row: DemandPlatform) => (
          <Space size={4}>
            <Button type="link" size="small" onClick={() => openEdit(row)}>
              {t("opc.demand.actionEdit")}
            </Button>
            <Button type="link" size="small" danger onClick={() => removePlatform(row)}>
              {t("common.delete")}
            </Button>
          </Space>
        ),
      },
    ],
    [openEdit, removePlatform, t, togglePlatform],
  );

  /** 订阅表格列 */
  const subscriptionColumns = useMemo(
    () => [
      {
        title: t("opc.demand.subColKeyword"),
        dataIndex: "keyword",
        key: "keyword",
        width: 220,
        render: (keyword: string) => <Typography.Text strong>{keyword}</Typography.Text>,
      },
      {
        title: t("opc.demand.subColPlatforms"),
        dataIndex: "platforms",
        key: "platforms",
        width: 200,
        render: (platforms: string[]) =>
          platforms.length === 0
            ? <Typography.Text type="secondary">{t("opc.demand.subAllPlatforms")}</Typography.Text>
            : (
              <Space size={4} wrap>
                {platforms.map((p) => <Tag key={p}>{p}</Tag>)}
              </Space>
            ),
      },
      {
        title: t("opc.demand.subColInterval"),
        dataIndex: "intervalHours",
        key: "intervalHours",
        width: 110,
        render: (hours: number) => t("opc.demand.subIntervalValue", { hours }),
      },
      {
        title: t("opc.demand.subColMinScore"),
        dataIndex: "minScore",
        key: "minScore",
        width: 110,
        render: (score: number) => (
          <Typography.Text style={{ color: scoreColor(score, token) }}>{Math.round(score)}</Typography.Text>
        ),
      },
      {
        title: t("opc.demand.subColLastScan"),
        dataIndex: "lastScannedAt",
        key: "lastScannedAt",
        width: 170,
        render: (ts: number | null) =>
          ts === null
            ? <Typography.Text type="secondary">{t("opc.demand.neverSynced")}</Typography.Text>
            : formatTs(ts),
      },
      {
        title: t("opc.demand.subColLastHits"),
        dataIndex: "lastHitCount",
        key: "lastHitCount",
        width: 100,
        render: (count: number) =>
          count > 0
            ? <Tag color="volcano">{count}</Tag>
            : <Typography.Text type="secondary">0</Typography.Text>,
      },
      {
        title: t("opc.demand.formEnabled"),
        dataIndex: "enabled",
        key: "enabled",
        width: 90,
        render: (enabled: boolean, row: DemandSubscription) => (
          <Switch
            size="small"
            checked={enabled}
            onChange={(checked) => void toggleSubscription(row, checked)}
          />
        ),
      },
      {
        title: t("opc.demand.colActions"),
        key: "actions",
        width: 140,
        render: (_: unknown, row: DemandSubscription) => (
          <Space size={4}>
            <Button type="link" size="small" onClick={() => openEditSubscription(row)}>
              {t("opc.demand.actionEdit")}
            </Button>
            <Button type="link" size="small" danger onClick={() => removeSubscription(row)}>
              {t("common.delete")}
            </Button>
          </Space>
        ),
      },
    ],
    [openEditSubscription, removeSubscription, t, token, toggleSubscription],
  );

  const INVOICE_STATUS_COLOR: Record<string, string> = {
    draft: "default",
    sent: "processing",
    paid: "success",
  };

  /** 发票账本列（P4 交付页） */
  const invoiceColumns = useMemo(
    () => [
      {
        title: t("opc.demand.delivColTitle"),
        dataIndex: "title",
        key: "title",
        width: 220,
        ellipsis: true,
      },
      {
        title: t("opc.demand.delivColLead"),
        dataIndex: "leadId",
        key: "leadId",
        width: 150,
        ellipsis: true,
        render: (id: string) => <Typography.Text copyable={{ text: id }}>{id}</Typography.Text>,
      },
      {
        title: t("opc.demand.delivColAmount"),
        key: "amount",
        width: 130,
        render: (_: unknown, row: DeliveryInvoice) => `${row.currency} ${row.amount.toLocaleString()}`,
      },
      {
        title: t("opc.demand.delivColStatus"),
        dataIndex: "status",
        key: "status",
        width: 100,
        render: (s: DeliveryInvoice["status"]) => (
          <Tag color={INVOICE_STATUS_COLOR[s] ?? "default"}>
            {t(`opc.demand.invoiceStatus.${s}`, { defaultValue: s })}
          </Tag>
        ),
      },
      {
        title: t("opc.demand.delivColIssuedAt"),
        dataIndex: "issuedAt",
        key: "issuedAt",
        width: 160,
        render: (ts: number | null) => (ts ? formatTs(ts) : "—"),
      },
      {
        title: t("opc.demand.delivColPaidAt"),
        dataIndex: "paidAt",
        key: "paidAt",
        width: 160,
        render: (ts: number | null) => (ts ? formatTs(ts) : "—"),
      },
      {
        title: t("opc.demand.delivColActions"),
        key: "actions",
        width: 220,
        render: (_: unknown, row: DeliveryInvoice) => (
          <Space size={4}>
            {row.status === "draft"
              ? (
                <Button
                  type="link"
                  size="small"
                  onClick={() => void advanceInvoice(row, "sent")}
                >
                  {t("opc.demand.delivMarkSent")}
                </Button>
              )
              : null}
            {row.status === "sent"
              ? (
                <Button
                  type="link"
                  size="small"
                  onClick={() => void advanceInvoice(row, "paid")}
                >
                  {t("opc.demand.delivMarkPaid")}
                </Button>
              )
              : null}
            {row.status !== "paid"
              ? (
                <Button type="link" size="small" danger onClick={() => void removeInvoice(row)}>
                  {t("opc.demand.delivDelete")}
                </Button>
              )
              : null}
          </Space>
        ),
      },
    ],
    [advanceInvoice, removeInvoice, t, token],
  );

  /** 能力匹配命中的列 */
  const capMatchColumns = useMemo(
    () => [
      {
        title: t("opc.demand.capColName"),
        dataIndex: "name",
        key: "name",
        render: (name: string, row: CapabilityMatchItem) => (
          <Flex vertical gap={2}>
            <Typography.Text strong>{name}</Typography.Text>
            {row.summary
              ? (
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {row.summary}
                </Typography.Text>
              )
              : null}
          </Flex>
        ),
      },
      {
        title: t("opc.demand.capColKind"),
        dataIndex: "kind",
        key: "kind",
        width: 110,
        render: (kind: string) => <Tag>{kind}</Tag>,
      },
      {
        title: t("opc.demand.capColDomain"),
        dataIndex: "domain",
        key: "domain",
        width: 110,
        render: (domain: string) => <Tag>{t(`capabilityDomain.${domain}`, { defaultValue: domain })}</Tag>,
      },
      {
        title: t("opc.demand.capColScore"),
        dataIndex: "retrievalScore",
        key: "retrievalScore",
        width: 100,
        render: (score: number) => (
          <Typography.Text style={{ color: scoreColor(score * 100, token) }}>
            {score.toFixed(3)}
          </Typography.Text>
        ),
      },
    ],
    [t, token],
  );

  return (
    <div
      className="h-full"
      style={{ overflow: "auto", backgroundColor: token.colorBgLayout, padding: 16 }}
    >
      <Flex vertical gap={12}>
        <Flex align="center" justify="space-between" gap={12} wrap>
          <Typography.Title level={4} style={{ margin: 0 }}>
            {t("opc.demand.pageTitle")}
          </Typography.Title>
          <Segmented
            value={tab}
            onChange={(value) => setTab(value as "leads" | "platforms" | "subscriptions" | "delivery")}
            options={[
              { label: t("opc.demand.leads"), value: "leads" },
              { label: t("opc.demand.platforms"), value: "platforms" },
              { label: t("opc.demand.subscriptions"), value: "subscriptions" },
              { label: t("opc.demand.delivery"), value: "delivery" },
              { label: t("opc.demand.capabilities"), value: "capabilities" },
              { label: t("opc.demand.deliveries"), value: "deliveries" },
            ]}
          />
        </Flex>

        {/* 关键词扫描工具栏只服务于「线索 / 平台」两页；订阅/交付页有自己的内容区 */}
        {tab === "subscriptions" || tab === "delivery" || tab === "capabilities" || tab === "deliveries"
          ? null
          : (
            <Card size="small">
              <Flex gap={8} wrap align="center">
                <Input
                  allowClear
                  style={{ flex: 1, minWidth: 240 }}
                  value={query}
                  placeholder={t("opc.demand.searchQueryPlaceholder")}
                  onChange={(e) => setQuery(e.target.value)}
                  onPressEnter={() => void runScan()}
                />
                <Space size={4}>
                  <Typography.Text type="secondary">{t("opc.demand.minScore")}</Typography.Text>
                  <InputNumber
                    min={0}
                    max={100}
                    step={5}
                    style={{ width: 96 }}
                    value={minScore}
                    onChange={(value) => setMinScore(value ?? 0)}
                  />
                  <Select
                    allowClear
                    style={{ width: 110 }}
                    placeholder={t("opc.demand.statusFilter")}
                    value={statusFilter}
                    onChange={(value) => setStatusFilter(value ?? null)}
                    options={LEAD_STATUSES.map((s) => ({
                      value: s,
                      label: t(`opc.demand.leadStatus.${s}`, { defaultValue: s }),
                    }))}
                  />
                </Space>
                <Button type="primary" loading={scanning} onClick={() => void runScan()}>
                  {scanning ? t("opc.demand.scanning") : t("opc.demand.btnDiscover")}
                </Button>
                <Button
                  onClick={() => {
                    void loadLeads();
                    void loadPlatforms();
                  }}
                >
                  {t("opc.demand.btnRefresh")}
                </Button>
                <Button onClick={openCreateLead}>{t("opc.demand.manualEntry")}</Button>
                <Button loading={capScanning} onClick={() => setCapScanOpen(true)}>
                  {t("opc.demand.capabilityScan")}
                </Button>
                <Button loading={proactiveScanning} onClick={() => void runProactiveScan()}>
                  {t("opc.demand.btnProactiveScan")}
                </Button>
              </Flex>
            </Card>
          )}

        {summary && tab !== "subscriptions" && tab !== "delivery" && tab !== "capabilities" && tab !== "deliveries"
          ? (
            <Card size="small">
              <Row gutter={16}>
                <Col span={6}>
                  <Statistic title={t("opc.demand.statScanned")} value={summary.totalScanned} />
                </Col>
                <Col span={6}>
                  <Statistic title={t("opc.demand.statEvaluated")} value={summary.totalEvaluated} />
                </Col>
                <Col span={6}>
                  <Statistic
                    title={t("opc.demand.statSaved")}
                    value={summary.totalSaved}
                    styles={{ content: { color: token.colorSuccess } }}
                  />
                </Col>
                <Col span={6}>
                  <Statistic
                    title={t("opc.demand.statHighValue")}
                    value={summary.highValueCount}
                    styles={{ content: { color: token.colorWarning } }}
                  />
                </Col>
              </Row>
            </Card>
          )
          : null}

        {tab === "subscriptions"
          ? (
            <Card size="small">
              <Flex vertical gap={8}>
                <Flex gap={8} wrap align="center" justify="space-between">
                  <Space size={8} wrap>
                    <Typography.Text type="secondary">
                      {t("opc.demand.subCronLabel")}
                    </Typography.Text>
                    <Input
                      style={{ width: 160 }}
                      value={scanCron}
                      onChange={(e) => setScanCron(e.target.value)}
                      placeholder={DEFAULT_SCAN_CRON}
                    />
                    <Button onClick={() => void ensureScanJob()}>
                      {t("opc.demand.enableScheduledScan")}
                    </Button>
                  </Space>
                  <Space size={8}>
                    <Button onClick={() => void loadSubscriptions()}>
                      {t("opc.demand.btnRefresh")}
                    </Button>
                    <Button onClick={openCreateSubscription} type="default">
                      {t("opc.demand.addSubscription")}
                    </Button>
                    <Button
                      type="primary"
                      loading={subScanning}
                      onClick={() => void runSubscriptionScan()}
                    >
                      {subScanning
                        ? t("opc.demand.scanning")
                        : t("opc.demand.scanNow")}
                    </Button>
                  </Space>
                </Flex>

                {subSummary
                  ? (
                    <Row gutter={16}>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.subStatScanned")}
                          value={subSummary.scannedSubscriptions}
                        />
                      </Col>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.statSaved")}
                          value={subSummary.totalSaved}
                          styles={{ content: { color: token.colorSuccess } }}
                        />
                      </Col>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.subStatRefreshed")}
                          value={subSummary.totalRefreshed}
                        />
                      </Col>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.subStatHits")}
                          value={subSummary.highValueHits}
                          styles={{ content: { color: token.colorWarning } }}
                        />
                      </Col>
                    </Row>
                  )
                  : null}

                <Table<DemandSubscription>
                  rowKey="id"
                  size="small"
                  loading={loading}
                  columns={subscriptionColumns}
                  dataSource={subscriptions}
                  tableLayout="fixed"
                  locale={{
                    emptyText: <Empty description={t("opc.demand.noSubscriptions")} />,
                  }}
                  pagination={false}
                />
              </Flex>
            </Card>
          )
          : tab === "delivery"
          ? (
            <Card size="small">
              <Flex vertical gap={12}>
                <Flex justify="flex-end">
                  <Button onClick={() => void loadInvoices()}>
                    {t("opc.demand.btnRefresh")}
                  </Button>
                </Flex>

                {deliverySummary
                  ? (
                    <Row gutter={16}>
                      <Col span={6}>
                        <Statistic title={t("opc.demand.delivStatWon")} value={deliverySummary.wonLeads} />
                      </Col>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.delivStatInvoices")}
                          value={deliverySummary.invoiceCount}
                        />
                      </Col>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.delivStatPaid")}
                          value={deliverySummary.paidCount}
                          styles={{ content: { color: token.colorSuccess } }}
                        />
                      </Col>
                      <Col span={6}>
                        <Statistic
                          title={t("opc.demand.delivStatConversion")}
                          value={(deliverySummary.conversionRate * 100).toFixed(1)}
                          suffix="%"
                          styles={{ content: { color: token.colorWarning } }}
                        />
                      </Col>
                    </Row>
                  )
                  : null}

                {deliverySummary && deliverySummary.revenues.length > 0
                  ? (
                    <Space size={16} wrap>
                      {deliverySummary.revenues.map((r) => (
                        <Statistic
                          key={r.currency}
                          title={t("opc.demand.delivRevenue", { currency: r.currency })}
                          value={r.paidTotal.toLocaleString()}
                          styles={{ content: { color: token.colorSuccess } }}
                        />
                      ))}
                    </Space>
                  )
                  : null}

                <Table<DeliveryInvoice>
                  rowKey="id"
                  size="small"
                  loading={loading}
                  columns={invoiceColumns}
                  dataSource={invoices}
                  tableLayout="fixed"
                  locale={{
                    emptyText: <Empty description={t("opc.demand.delivNoInvoices")} />,
                  }}
                  pagination={false}
                />
              </Flex>
            </Card>
          )
          : tab === "capabilities"
          ? <CapabilitiesPanel />
          : tab === "deliveries"
          ? <DeliveriesPanel />
          : (
            <Card size="small">
              {tab === "leads"
                ? (
                  <Table<DemandLead>
                    rowKey="id"
                    size="small"
                    loading={loading}
                    columns={leadColumns}
                    dataSource={leads}
                    tableLayout="fixed"
                    locale={{ emptyText: <Empty description={t("opc.demand.noLeadsFound")} /> }}
                    pagination={{ pageSize: 20, showSizeChanger: true }}
                  />
                )
                : (
                  <>
                    <Flex justify="flex-end" style={{ marginBottom: 8 }}>
                      <Button type="primary" size="small" onClick={openCreate}>
                        {t("opc.demand.addPlatform")}
                      </Button>
                    </Flex>
                    <Table<DemandPlatform>
                      rowKey="id"
                      size="small"
                      loading={loading}
                      columns={platformColumns}
                      dataSource={platforms}
                      tableLayout="fixed"
                      pagination={false}
                    />
                    {scanPolicy && (
                      <Card
                        size="small"
                        title={t("opc.demand.scanPolicyTitle")}
                        style={{ marginTop: 12 }}
                        extra={
                          <Button
                            type="primary"
                            size="small"
                            loading={policySaving}
                            onClick={() => void saveScanPolicy()}
                          >
                            {t("opc.demand.savePolicy")}
                          </Button>
                        }
                      >
                        <Flex gap={16} wrap>
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanConcurrency")}
                            value={scanPolicy.concurrency}
                            min={1}
                            max={32}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, concurrency: v ?? 1 })}
                          />
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanRateLimit")}
                            value={scanPolicy.rateLimitPerMin}
                            min={0}
                            max={600}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, rateLimitPerMin: v ?? 0 })}
                          />
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanRetryMax")}
                            value={scanPolicy.retryMax}
                            min={0}
                            max={5}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, retryMax: v ?? 0 })}
                          />
                          <PolicyField
                            label={t("opc.demand.scanRetryBackoffMs")}
                            value={scanPolicy.retryBackoffMs}
                            min={0}
                            max={10_000}
                            step={100}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, retryBackoffMs: v ?? 0 })}
                          />
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanTimeoutSecs")}
                            value={scanPolicy.timeoutSecs}
                            min={1}
                            max={120}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, timeoutSecs: v ?? 15 })}
                          />
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanDeduplicateWindowHours")}
                            value={scanPolicy.dedupWindowHours}
                            min={0}
                            max={8760}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, dedupWindowHours: v ?? 0 })}
                          />
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanMaxLeadsPerSync")}
                            value={scanPolicy.maxLeadsPerScan}
                            min={1}
                            max={5000}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, maxLeadsPerScan: v ?? 200 })}
                          />
                          <Flex vertical gap={4}>
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {t("opc.demand.configDescriptions.scanLlmEvalEnabled")}
                            </Typography.Text>
                            <Switch
                              size="small"
                              checked={scanPolicy.llmEvalEnabled}
                              onChange={(v) => setScanPolicy({ ...scanPolicy, llmEvalEnabled: v })}
                            />
                          </Flex>
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanLlmEvalMaxLeads")}
                            value={scanPolicy.llmEvalMaxLeads}
                            min={1}
                            max={100}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, llmEvalMaxLeads: v ?? 20 })}
                          />
                          <PolicyField
                            label={t("opc.demand.configDescriptions.scanLlmEvalMinRuleScore")}
                            value={scanPolicy.llmEvalMinRuleScore}
                            min={0}
                            max={100}
                            onChange={(v) => setScanPolicy({ ...scanPolicy, llmEvalMinRuleScore: v ?? 30 })}
                          />
                        </Flex>
                      </Card>
                    )}
                  </>
                )}
            </Card>
          )}
      </Flex>

      <Modal
        open={capModalOpen}
        title={t("opc.demand.capMatchTitle")}
        width={760}
        footer={null}
        onCancel={() => setCapModalOpen(false)}
        destroyOnHidden
      >
        {capMatch === null
          ? (
            <Empty
              description={capMatching ? t("opc.demand.capMatching") : t("opc.demand.capNoResult")}
            />
          )
          : (
            <Flex vertical gap={12}>
              <Flex align="center" gap={12} wrap>
                <Tag color={VERDICT_COLOR[capMatch.verdict] ?? "default"}>
                  {t(`opc.demand.capVerdict.${capMatch.verdict}`, {
                    defaultValue: capMatch.verdict,
                  })}
                </Tag>
                <Typography.Text type="secondary">
                  {t("opc.demand.capBestScore", { score: capMatch.bestScore.toFixed(3) })}
                </Typography.Text>
              </Flex>

              <Flex vertical gap={4}>
                <Typography.Text type="secondary">
                  {t("opc.demand.capRequiredDomains")}
                </Typography.Text>
                {capMatch.requiredDomains.length === 0
                  ? (
                    <Typography.Text type="secondary">
                      {t("opc.demand.capNoRequiredDomains")}
                    </Typography.Text>
                  )
                  : (
                    <Space size={4} wrap>
                      {capMatch.requiredDomains.map((d) => (
                        <Tag key={d}>{t(`capabilityDomain.${d}`, { defaultValue: d })}</Tag>
                      ))}
                    </Space>
                  )}
              </Flex>

              <Flex vertical gap={4}>
                <Typography.Text type="secondary">
                  {t("opc.demand.capMissingDomains")}
                </Typography.Text>
                {capMatch.missingDomains.length === 0
                  ? <Typography.Text type="secondary">{t("opc.demand.capNoMissing")}</Typography.Text>
                  : (
                    <Space size={4} wrap>
                      {capMatch.missingDomains.map((d) => (
                        <Tag key={d} color="red">
                          {t(`capabilityDomain.${d}`, { defaultValue: d })}
                        </Tag>
                      ))}
                    </Space>
                  )}
              </Flex>

              {capMatch.gapHint
                ? <Alert type="warning" showIcon message={capMatch.gapHint} />
                : null}

              <Table<CapabilityMatchItem>
                rowKey="capabilityId"
                size="small"
                columns={capMatchColumns}
                dataSource={capMatch.matches}
                pagination={false}
                locale={{ emptyText: <Empty description={t("opc.demand.capNoMatches")} /> }}
              />
            </Flex>
          )}
      </Modal>

      <Modal
        open={modalOpen}
        title={editing ? t("opc.demand.editPlatform") : t("opc.demand.addPlatform")}
        okText={t("common.save")}
        cancelText={t("common.cancel")}
        confirmLoading={saving}
        onOk={() => void submitPlatform()}
        onCancel={() => setModalOpen(false)}
        destroyOnHidden
      >
        <Form<PlatformFormValues> form={form} layout="vertical" preserve={false}>
          <Form.Item
            name="id"
            label={t("opc.demand.formId")}
            extra={editing ? undefined : t("opc.demand.formIdTip")}
          >
            <Input disabled={editing !== null} placeholder={t("opc.demand.formIdPlaceholder")} />
          </Form.Item>
          <Form.Item
            name="name"
            label={t("opc.demand.formName")}
            rules={[{ required: true, message: t("opc.demand.formNameRequired") }]}
          >
            <Input placeholder={t("opc.demand.formNamePlaceholder")} />
          </Form.Item>
          <Form.Item name="platformType" label={t("opc.demand.formPlatformType")} initialValue="scanner">
            <Select
              options={PLATFORM_TYPES.map((value) => ({
                value,
                label: t(`opc.demand.platformType.${value}`, { defaultValue: value }),
              }))}
            />
          </Form.Item>
          <Form.Item name="baseUrl" label={t("opc.demand.formBaseUrl")}>
            <Input placeholder={t("opc.demand.formBaseUrlPlaceholder")} />
          </Form.Item>
          <Form.Item
            name="apiToken"
            label={t("opc.demand.formApiToken")}
            extra={t("opc.demand.formApiTokenTip")}
          >
            <Input.Password
              autoComplete="new-password"
              placeholder={t("opc.demand.formApiTokenPlaceholder")}
            />
          </Form.Item>
          <Form.Item name="enabled" label={t("opc.demand.formEnabled")} valuePropName="checked">
            <Switch />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        open={subModalOpen}
        title={subEditing
          ? t("opc.demand.editSubscription")
          : t("opc.demand.addSubscription")}
        okText={t("common.save")}
        cancelText={t("common.cancel")}
        confirmLoading={subSaving}
        onOk={() => void submitSubscription()}
        onCancel={() => setSubModalOpen(false)}
        destroyOnHidden
      >
        <Form<SubscriptionFormValues> form={subForm} layout="vertical" preserve={false}>
          <Form.Item
            name="keyword"
            label={t("opc.demand.subFormKeyword")}
            rules={[{ required: true, message: t("opc.demand.subFormKeywordRequired") }]}
          >
            <Input placeholder={t("opc.demand.subFormKeywordPlaceholder")} />
          </Form.Item>
          <Form.Item
            name="intervalHours"
            label={t("opc.demand.subFormInterval")}
            extra={t("opc.demand.subFormIntervalTip")}
          >
            <InputNumber min={1} max={720} step={1} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item
            name="minScore"
            label={t("opc.demand.subFormMinScore")}
            extra={t("opc.demand.subFormMinScoreTip")}
          >
            <InputNumber min={0} max={100} step={5} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item
            name="platforms"
            label={t("opc.demand.subFormPlatforms")}
            extra={t("opc.demand.subFormPlatformsTip")}
          >
            <Select
              mode="multiple"
              allowClear
              options={platforms.map((p) => ({ value: p.id, label: p.name }))}
            />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        open={leadModalOpen}
        title={t("opc.demand.manualEntry")}
        okText={t("common.save")}
        cancelText={t("common.cancel")}
        confirmLoading={leadSaving}
        onOk={() => void submitLead()}
        onCancel={() => setLeadModalOpen(false)}
        destroyOnHidden
        width={640}
      >
        <Form<LeadFormValues> form={leadForm} layout="vertical" preserve={false}>
          <Form.Item
            name="title"
            label={t("opc.demand.leadFormTitle")}
            rules={[{ required: true, message: t("opc.demand.leadFormTitleRequired") }]}
          >
            <Input placeholder={t("opc.demand.leadFormTitlePlaceholder")} />
          </Form.Item>
          <Form.Item
            name="description"
            label={t("opc.demand.leadFormDescription")}
            rules={[{ required: true, message: t("opc.demand.leadFormDescriptionRequired") }]}
          >
            <Input.TextArea
              rows={4}
              placeholder={t("opc.demand.leadFormDescriptionPlaceholder")}
            />
          </Form.Item>
          <Row gutter={12}>
            <Col span={8}>
              <Form.Item name="budgetMin" label={t("opc.demand.leadFormBudgetMin")}>
                <InputNumber min={0} style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="budgetMax" label={t("opc.demand.leadFormBudgetMax")}>
                <InputNumber min={0} style={{ width: "100%" }} />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item
                name="budgetCurrency"
                label={t("opc.demand.leadFormBudgetCurrency")}
                initialValue="CNY"
              >
                <Select
                  options={[
                    { value: "CNY", label: "CNY (¥)" },
                    { value: "USD", label: "USD ($)" },
                  ]}
                />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={12}>
            <Col span={8}>
              <Form.Item name="contactName" label={t("opc.demand.leadFormContactName")}>
                <Input />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="contactEmail" label={t("opc.demand.leadFormContactEmail")}>
                <Input />
              </Form.Item>
            </Col>
            <Col span={8}>
              <Form.Item name="contactPhone" label={t("opc.demand.leadFormContactPhone")}>
                <Input />
              </Form.Item>
            </Col>
          </Row>
          <Form.Item name="sourceUrl" label={t("opc.demand.leadFormSourceUrl")}>
            <Input placeholder={t("opc.demand.leadFormSourceUrlPlaceholder")} />
          </Form.Item>
        </Form>
      </Modal>

      {/* 能力驱动扫描配置（原方案 workflow 形态入口） */}
      <Modal
        open={capScanOpen}
        title={t("opc.demand.capabilityScan")}
        okText={t("opc.demand.capabilityScanStart")}
        cancelText={t("common.cancel")}
        confirmLoading={capScanning}
        onOk={() => void runCapabilityScan()}
        onCancel={() => setCapScanOpen(false)}
        destroyOnHidden
      >
        <Flex vertical gap={12}>
          <Typography.Text type="secondary">
            {t("opc.demand.capabilityScanDesc")}
          </Typography.Text>
          <Flex vertical gap={4}>
            <Typography.Text type="secondary">
              {t("opc.demand.capabilityFocus")}
            </Typography.Text>
            <Input
              allowClear
              value={capFocus}
              placeholder={t("opc.demand.capabilityFocusPlaceholder")}
              onChange={(e) => setCapFocus(e.target.value)}
              onPressEnter={() => void runCapabilityScan()}
            />
          </Flex>
          <Flex vertical gap={4}>
            <Typography.Text type="secondary">
              {t("opc.demand.capabilityMaxKeywords")}
            </Typography.Text>
            <InputNumber
              min={1}
              max={10}
              step={1}
              style={{ width: 120 }}
              value={capMaxKeywords}
              onChange={(value) => setCapMaxKeywords(value ?? 5)}
            />
          </Flex>
        </Flex>
      </Modal>
    </div>
  );
}

// ── 能力与缺口面板（自 OPC 面板 DemandDiscoveryTab 合并，能力库 + 缺口一页两区） ──

function CapabilitiesPanel() {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [inventory, setInventory] = useState<CapabilityInventory | null>(null);
  const [loading, setLoading] = useState(false);
  const [gaps, setGaps] = useState<CapabilityGap[]>([]);
  const [gapsLoading, setGapsLoading] = useState(false);
  const [analyzingGaps, setAnalyzingGaps] = useState(false);

  const loadInventory = useCallback(async () => {
    setLoading(true);
    try {
      setInventory(await invoke<CapabilityInventory>("opc_scan_capabilities"));
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setInventory(null);
    } finally {
      setLoading(false);
    }
  }, [t]);

  const loadGaps = useCallback(async () => {
    setGapsLoading(true);
    try {
      setGaps(await invoke<CapabilityGap[]>("opc_list_capability_gaps", {}));
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setGaps([]);
    } finally {
      setGapsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadInventory();
    void loadGaps();
  }, [loadInventory, loadGaps]);

  const analyzeGaps = useCallback(async () => {
    setAnalyzingGaps(true);
    try {
      const result = await invoke<{
        auto_created_gaps: string[];
        missing_keywords_count: number;
      }>("opc_analyze_capability_gaps", {});
      if (result.auto_created_gaps.length > 0) {
        message.success(
          t("opc.demand.gapsAnalyzedCreated", { count: result.auto_created_gaps.length }),
        );
      } else {
        message.info(t("opc.demand.gapsAnalyzedNoNew"));
      }
      void loadGaps();
    } catch (e) {
      message.error(String(e));
    } finally {
      setAnalyzingGaps(false);
    }
  }, [loadGaps, message, t]);

  const closeGap = useCallback(
    async (id: string) => {
      try {
        await invoke("opc_close_capability_gap", { id });
        message.success(t("opc.demand.gapClosed"));
        void loadGaps();
      } catch (e) {
        message.error(String(e));
      }
    },
    [loadGaps, message, t],
  );

  const renderTable = (title: string, data: CapabilityEntry[]) => {
    const cols: ColumnsType<CapabilityEntry> = [
      { title: t("opc.demand.colName"), dataIndex: "name", key: "name", width: 150 },
      { title: t("opc.demand.colDescription"), dataIndex: "description", key: "description", ellipsis: true },
      {
        title: t("opc.demand.colSource"),
        dataIndex: "source",
        key: "source",
        width: 100,
        render: (v: string) => <Tag>{v}</Tag>,
      },
      { title: t("opc.demand.colType"), dataIndex: "capability_type", key: "type", width: 100 },
    ];
    return (
      <Card title={`${title} (${data.length})`} size="small" style={{ marginBottom: 16 }}>
        <Table
          rowKey="id"
          size="small"
          dataSource={data}
          columns={cols}
          pagination={{ pageSize: 5 }}
          tableLayout="fixed"
        />
      </Card>
    );
  };

  const gapColumns: ColumnsType<CapabilityGap> = [
    { title: t("opc.demand.colTitle"), dataIndex: "title", key: "title", width: 200, ellipsis: true },
    { title: t("opc.demand.colDescription"), dataIndex: "description", key: "description", ellipsis: true },
    {
      title: t("opc.demand.colGapType"),
      dataIndex: "gap_type",
      key: "gap_type",
      width: 110,
      render: (v: string) => <Tag color="orange">{v}</Tag>,
    },
    {
      title: t("opc.demand.colSuggestedAction"),
      dataIndex: "suggested_action",
      key: "suggested_action",
      ellipsis: true,
    },
    {
      title: t("opc.demand.colPriority"),
      dataIndex: "priority",
      key: "priority",
      width: 90,
      render: (v: number) => <Tag color={v <= 2 ? "red" : v === 3 ? "orange" : "default"}>{v}</Tag>,
    },
    {
      title: t("opc.demand.colStatus"),
      dataIndex: "status",
      key: "status",
      width: 100,
      render: (v: string) => <Tag color={v === "open" ? "orange" : "green"}>{t(`opc.demand.gapStatus.${v}`)}</Tag>,
    },
    {
      title: t("opc.demand.colActions"),
      key: "actions",
      width: 120,
      render: (_: unknown, r: CapabilityGap) =>
        r.status === "open"
          ? (
            <Button size="small" onClick={() => void closeGap(r.id)}>
              {t("opc.demand.actionCloseGap")}
            </Button>
          )
          : null,
    },
  ];

  return (
    <Flex vertical gap={12}>
      <Card size="small">
        <Space wrap>
          <Button loading={loading} onClick={() => void loadInventory()}>
            {t("opc.demand.btnScanCapabilities")}
          </Button>
          <Button danger loading={analyzingGaps} onClick={() => void analyzeGaps()}>
            {t("opc.demand.btnAnalyzeGaps")}
          </Button>
          <Button onClick={() => void loadGaps()}>{t("opc.demand.btnRefresh")}</Button>
          {inventory && (
            <Tag color="blue">
              {t("opc.demand.totalCapabilities", { count: inventory.total_count })}
            </Tag>
          )}
        </Space>
      </Card>

      {inventory
        ? (
          <>
            {renderTable(t("opc.demand.categoryTools"), inventory.tools)}
            {renderTable(t("opc.demand.categorySkills"), inventory.skills)}
            {renderTable(t("opc.demand.categoryMcpTools"), inventory.mcp_tools)}
            {renderTable(t("opc.demand.categoryWorkflows"), inventory.workflows)}
            {renderTable(t("opc.demand.categoryAgents"), inventory.agents)}
          </>
        )
        : <Empty description={t("opc.demand.capNoResult")} />}

      <Card title={t("opc.demand.gaps")} size="small">
        <Table<CapabilityGap>
          rowKey="id"
          size="small"
          loading={gapsLoading}
          dataSource={gaps}
          columns={gapColumns}
          pagination={{ pageSize: 10 }}
          tableLayout="fixed"
          locale={{ emptyText: <Empty description={t("opc.demand.gapsEmpty")} /> }}
        />
      </Card>
    </Flex>
  );
}

// ── 交付记录面板（自 OPC 面板 DemandDiscoveryTab 合并；记录由线索「执行交付」产生） ──

function DeliveriesPanel() {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [deliveries, setDeliveries] = useState<Delivery[]>([]);
  const [loading, setLoading] = useState(false);
  const [detailOpen, setDetailOpen] = useState(false);
  const [currentDelivery, setCurrentDelivery] = useState<Delivery | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setDeliveries(await invoke<Delivery[]>("opc_list_deliveries", {}));
    } catch (e) {
      message.error(t("opc.common.loadFailed", { error: String(e) }));
      setDeliveries([]);
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleRetry = useCallback(
    async (id: string) => {
      try {
        await invoke("opc_retry_delivery", { id });
        message.success(t("opc.demand.deliveryRetried"));
        void load();
      } catch (e) {
        message.error(String(e));
      }
    },
    [load, message, t],
  );

  const handleCancel = useCallback(
    async (id: string) => {
      try {
        await invoke("opc_cancel_delivery", { id });
        message.success(t("opc.demand.deliveryCancelled"));
        void load();
      } catch (e) {
        message.error(String(e));
      }
    },
    [load, message, t],
  );

  const columns: ColumnsType<Delivery> = [
    { title: t("opc.demand.colTitle"), dataIndex: "title", key: "title", width: 200, ellipsis: true },
    {
      title: t("opc.demand.colStatus"),
      dataIndex: "status",
      key: "status",
      width: 120,
      render: (v: string) => (
        <Tag color={DELIVERY_STATUS_COLOR_MAP[v] || "default"}>
          {t(`opc.delivery.status.${v}`)}
        </Tag>
      ),
    },
    {
      title: t("opc.demand.colProgress"),
      dataIndex: "progress",
      key: "progress",
      width: 150,
      render: (v: number) => <Progress percent={Math.round(v * 100)} size="small" />,
    },
    {
      title: t("opc.demand.colTemplate"),
      dataIndex: "workflow_template_id",
      key: "template",
      width: 180,
      ellipsis: true,
    },
    {
      title: t("opc.demand.colStartedAt"),
      dataIndex: "started_at",
      key: "started_at",
      width: 160,
      render: (v: number | null) => (v ? new Date(v * 1000).toLocaleString() : "-"),
    },
    {
      title: t("opc.demand.colResult"),
      dataIndex: "result_summary",
      key: "result",
      ellipsis: true,
    },
    {
      title: t("opc.demand.colActions"),
      key: "actions",
      width: 200,
      render: (_: unknown, r: Delivery) => (
        <Space size="small">
          <Button
            size="small"
            onClick={() => {
              setCurrentDelivery(r);
              setDetailOpen(true);
            }}
          >
            {t("opc.demand.actionViewDetail")}
          </Button>
          {r.status === "failed" && (
            <Button size="small" type="primary" onClick={() => void handleRetry(r.id)}>
              {t("opc.demand.actionRetry")}
            </Button>
          )}
          {["pending", "running"].includes(r.status) && (
            <Button
              size="small"
              danger
              onClick={() => void handleCancel(r.id)}
            >
              {t("opc.demand.actionCancel")}
            </Button>
          )}
        </Space>
      ),
    },
  ];

  const delivery = currentDelivery;
  const timelineSteps = delivery
    ? [
      {
        title: t("opc.demand.deliverySteps.initiated"),
        time: delivery.started_at ? new Date(delivery.started_at * 1000).toLocaleString() : "-",
        color: "green",
      },
      {
        title: t("opc.demand.deliverySteps.processing"),
        time: delivery.progress > 0 ? t("opc.demand.deliverySteps.inProgress") : "-",
        color: delivery.progress > 0 ? "blue" : "gray",
      },
      {
        title: t("opc.demand.deliverySteps.completed"),
        time: delivery.completed_at
          ? new Date(delivery.completed_at * 1000).toLocaleString()
          : "-",
        color: delivery.status === "completed" || delivery.status === "delivered"
          ? "green"
          : delivery.status === "failed"
          ? "red"
          : "gray",
      },
    ]
    : [];

  return (
    <Flex vertical gap={12}>
      <Card size="small">
        <Button onClick={() => void load()} loading={loading}>
          {t("opc.demand.btnRefresh")}
        </Button>
      </Card>

      <Card size="small">
        <Table<Delivery>
          rowKey="id"
          loading={loading}
          dataSource={deliveries}
          columns={columns}
          pagination={{ pageSize: 10 }}
          tableLayout="fixed"
          locale={{ emptyText: <Empty description={t("opc.demand.deliveriesEmpty")} /> }}
        />
      </Card>

      <Modal
        title={delivery ? `${t("opc.demand.deliveryDetail")}: ${delivery.title}` : null}
        open={detailOpen}
        onCancel={() => setDetailOpen(false)}
        footer={delivery && (
          <Space>
            {(delivery.status === "failed" || delivery.status === "cancelled") && (
              <Button
                type="primary"
                onClick={() => void handleRetry(delivery.id)}
              >
                {t("opc.demand.actionRetry")}
              </Button>
            )}
            {["pending", "running"].includes(delivery.status) && (
              <Button
                danger
                onClick={() => void handleCancel(delivery.id)}
              >
                {t("opc.demand.actionCancel")}
              </Button>
            )}
            <Button onClick={() => setDetailOpen(false)}>{t("opc.demand.actionClose")}</Button>
          </Space>
        )}
        width={720}
      >
        {delivery && (
          <Flex vertical gap={12}>
            <Card size="small" title={t("opc.demand.deliveryInfo")}>
              <Descriptions column={2} size="small">
                <Descriptions.Item label={t("opc.demand.colStatus")}>
                  <Tag color={DELIVERY_STATUS_COLOR_MAP[delivery.status] || "default"}>
                    {t(`opc.delivery.status.${delivery.status}`)}
                  </Tag>
                </Descriptions.Item>
                <Descriptions.Item label={t("opc.demand.colProgress")}>
                  <Progress percent={Math.round(delivery.progress * 100)} />
                </Descriptions.Item>
                <Descriptions.Item label={t("opc.demand.colTemplate")}>
                  {delivery.workflow_template_id || "-"}
                </Descriptions.Item>
                <Descriptions.Item label={t("opc.demand.leadId")}>
                  {delivery.lead_id}
                </Descriptions.Item>
                <Descriptions.Item label={t("opc.demand.colStartedAt")}>
                  {delivery.started_at
                    ? new Date(delivery.started_at * 1000).toLocaleString()
                    : "-"}
                </Descriptions.Item>
                <Descriptions.Item label={t("opc.demand.colCompletedAt")}>
                  {delivery.completed_at
                    ? new Date(delivery.completed_at * 1000).toLocaleString()
                    : "-"}
                </Descriptions.Item>
              </Descriptions>
            </Card>

            <Card size="small" title={t("opc.demand.deliveryTimeline")}>
              <Timeline
                items={timelineSteps.map((s) => ({
                  color: s.color,
                  children: (
                    <div>
                      <div className="font-medium">{s.title}</div>
                      <div className="text-xs text-gray-500">{s.time}</div>
                    </div>
                  ),
                }))}
              />
            </Card>

            {delivery.result_summary && (
              <Card size="small" title={t("opc.demand.deliveryResult")}>
                <Typography.Paragraph
                  ellipsis={{ rows: 3, expandable: true, symbol: t("opc.demand.expand") }}
                >
                  {delivery.result_summary}
                </Typography.Paragraph>
              </Card>
            )}

            {delivery.deliverables && delivery.deliverables.length > 0 && (
              <Card size="small" title={t("opc.demand.deliverables")}>
                <List
                  size="small"
                  dataSource={delivery.deliverables}
                  renderItem={(d, idx) => (
                    <List.Item key={idx}>
                      <Space>
                        <Tag color="blue">
                          {(d.name as string) || t("opc.demand.deliverable")}
                        </Tag>
                        <span>{(d.type as string) || ""}</span>
                      </Space>
                    </List.Item>
                  )}
                />
              </Card>
            )}

            {delivery.errors && delivery.errors.length > 0 && (
              <Card size="small" title={t("opc.demand.errors")}>
                <Alert
                  type="error"
                  showIcon
                  message={t("opc.demand.deliveryError")}
                  description={
                    <ul>
                      {delivery.errors.map((e, idx) => <li key={idx}>{(e.message as string) || JSON.stringify(e)}</li>)}
                    </ul>
                  }
                />
              </Card>
            )}
          </Flex>
        )}
      </Modal>
    </Flex>
  );
}
