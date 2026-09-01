// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import type { DemandLead, DemandPlatform, DiscoverLeadsSummary, SaveDemandPlatformInput } from "@/types";
import {
  App,
  Button,
  Card,
  Col,
  Empty,
  Flex,
  Form,
  Input,
  InputNumber,
  Modal,
  Row,
  Segmented,
  Select,
  Space,
  Statistic,
  Switch,
  Table,
  Tag,
  theme,
  Typography,
} from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

/** 连接器类型（与后端 add_platform 的 platform_type 一致） */
const PLATFORM_TYPES = ["scanner", "api", "mock", "manual"] as const;

/** 机会等级 → Tag 颜色：热度越高越暖 */
const LEVEL_COLOR: Record<string, string> = {
  very_high: "red",
  high: "volcano",
  medium: "gold",
  low: "default",
};

/** 连接器状态 → Tag 颜色 */
const STATUS_COLOR: Record<string, string> = {
  ok: "green",
  error: "red",
  idle: "default",
};

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
  enabled: boolean;
}

export function DemandDiscoveryPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message } = App.useApp();

  const [tab, setTab] = useState<"leads" | "platforms">("leads");
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);

  const [query, setQuery] = useState("");
  const [minScore, setMinScore] = useState<number>(0);
  const [leads, setLeads] = useState<DemandLead[]>([]);
  const [platforms, setPlatforms] = useState<DemandPlatform[]>([]);
  const [summary, setSummary] = useState<DiscoverLeadsSummary | null>(null);

  const [editing, setEditing] = useState<DemandPlatform | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [form] = Form.useForm<PlatformFormValues>();

  const loadLeads = useCallback(async () => {
    setLoading(true);
    try {
      const rows = await invoke<DemandLead[]>("opc_list_leads", {
        limit: 200,
        minScore: minScore > 0 ? minScore : null,
      });
      setLeads(rows ?? []);
    } catch (e) {
      message.error(t("opc.demand.loadFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [minScore, message, t]);

  const loadPlatforms = useCallback(async () => {
    try {
      const rows = await invoke<DemandPlatform[]>("opc_list_platforms");
      setPlatforms(rows ?? []);
    } catch (e) {
      message.error(t("opc.demand.loadFailed", { error: String(e) }));
    }
  }, [message, t]);

  useEffect(() => {
    void loadLeads();
    void loadPlatforms();
  }, [loadLeads, loadPlatforms]);

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

  const openCreate = useCallback(() => {
    setEditing(null);
    form.setFieldsValue({
      id: undefined,
      name: "",
      platformType: "scanner",
      baseUrl: undefined,
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
  }, [form, loadPlatforms, message, t]);

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

  const leadColumns = useMemo(
    () => [
      {
        title: t("opc.demand.colTitle"),
        dataIndex: "title",
        key: "title",
        width: 320,
        render: (title: string, row: DemandLead) => (
          <Flex vertical gap={2}>
            <Typography.Text strong ellipsis={{ tooltip: title }}>
              {title}
            </Typography.Text>
            {row.description
              ? (
                <Typography.Text type="secondary" ellipsis={{ tooltip: row.description }} style={{ fontSize: 12 }}>
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
        title: t("opc.demand.colOpportunityLevel"),
        dataIndex: "opportunityLevel",
        key: "opportunityLevel",
        width: 110,
        render: (level: string) => (
          <Tag color={LEVEL_COLOR[level] ?? "default"}>
            {t(`opc.demand.opportunityLevel.${level}`, { defaultValue: level })}
          </Tag>
        ),
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
              <Typography.Link href={url} target="_blank" rel="noreferrer">
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
    ],
    [t, token],
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
        width: 100,
        render: (status: string) => (
          <Tag color={STATUS_COLOR[status] ?? "default"}>
            {t(`opc.demand.platformStatus.${status}`, { defaultValue: status })}
          </Tag>
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
            onChange={(value) => setTab(value as "leads" | "platforms")}
            options={[
              { label: t("opc.demand.leads"), value: "leads" },
              { label: t("opc.demand.platforms"), value: "platforms" },
            ]}
          />
        </Flex>

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
          </Flex>
        </Card>

        {summary
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

        <Card size="small">
          {tab === "leads"
            ? (
              <Table<DemandLead>
                rowKey="id"
                size="small"
                loading={loading}
                columns={leadColumns}
                dataSource={leads}
                scroll={{ x: 1200 }}
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
                  scroll={{ x: 1040 }}
                  pagination={false}
                />
              </>
            )}
        </Card>
      </Flex>

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
          <Form.Item name="enabled" label={t("opc.demand.formEnabled")} valuePropName="checked">
            <Switch />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
