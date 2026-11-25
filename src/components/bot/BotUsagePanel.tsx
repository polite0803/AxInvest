// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot / 部门「用量与费用」页签（P5-b B4）。
 *
 * 数据源：`get_bot_usage_summary` / `list_bot_usage` / `get_bot_cost_quota` 三条只读命令，
 * 配额增删改走 `set_bot_cost_quota` / `reset_bot_cost_quota`。用量是低频数据，
 * 打开页签时主动拉取（IPC-P5 §3 不新增事件）。费用一律标注「估算」（ADR-P5-4）。
 */

import { backendErrorText } from "@/lib/errorI18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { BotCostQuota, BotCostQuotaInput, BotCostQuotaScope, BotUsageEntry, BotUsageSummary } from "@/types";
import { App, Button, Input, List, Popconfirm, Space, Table, Tag, Tooltip, Typography } from "antd";
import type { TableProps } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

/** 与后端 `bot_cost_quota.rs::validate_quota_input` 逐项对齐。 */
const WINDOW_MIN = 1;
const WINDOW_MAX = 24 * 365;

interface CostQuotaForm {
  maxTotalTokens: number;
  maxCostUsd: number;
  windowHours: number;
}

type CostField = keyof CostQuotaForm;

function toForm(quota: BotCostQuota | null): CostQuotaForm {
  return {
    maxTotalTokens: quota?.maxTotalTokens ?? 0,
    maxCostUsd: quota?.maxCostUsd ?? 0,
    windowHours: quota?.windowHours ?? 168,
  };
}

function formatTokens(value: number): string {
  return value.toLocaleString();
}

function formatUsd(value: number): string {
  return `$${value.toFixed(4)}`;
}

export interface BotUsagePanelProps {
  scope: BotCostQuotaScope;
  scopeId: string;
}

export function BotUsagePanel({ scope, scopeId }: BotUsagePanelProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const getBotCostQuota = useBotStore((s) => s.getBotCostQuota);
  const setBotCostQuota = useBotStore((s) => s.setBotCostQuota);
  const resetBotCostQuota = useBotStore((s) => s.resetBotCostQuota);
  const getBotUsageSummary = useBotStore((s) => s.getBotUsageSummary);
  const listBotUsage = useBotStore((s) => s.listBotUsage);
  const saving = useBotStore((s) => s.costQuotaSaving);

  const [quota, setQuota] = useState<BotCostQuota | null>(null);
  const [summary, setSummary] = useState<BotUsageSummary | null>(null);
  const [entries, setEntries] = useState<BotUsageEntry[]>([]);
  const [form, setForm] = useState<CostQuotaForm>(() => toForm(null));
  const [loading, setLoading] = useState(true);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [refreshSeq, setRefreshSeq] = useState(0);

  const reload = useCallback(async () => {
    const currentQuota = await getBotCostQuota(scope, scopeId);
    const [nextSummary, nextEntries] = await Promise.all([
      getBotUsageSummary(scope, scopeId, currentQuota.isDefault ? null : currentQuota.windowHours),
      listBotUsage(scope === "bot" ? { botId: scopeId } : { departmentId: scopeId }),
    ]);
    setQuota(currentQuota);
    setForm(toForm(currentQuota));
    setSummary(nextSummary);
    setEntries(nextEntries);
  }, [getBotCostQuota, getBotUsageSummary, listBotUsage, scope, scopeId]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setErrorText(null);
    void reload()
      .catch((e) => {
        if (alive) {
          setErrorText(backendErrorText(e));
        }
      })
      .finally(() => {
        if (alive) {
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [reload, refreshSeq]);

  const labels: Record<CostField, string> = {
    maxTotalTokens: t("bot.usage.totalTokens"),
    maxCostUsd: `${t("bot.usage.totalCost")} (${t("bot.usage.estimated")})`,
    windowHours: t("bot.usage.window"),
  };

  const validate = useCallback((): CostField | null => {
    if (!Number.isInteger(form.maxTotalTokens) || form.maxTotalTokens < 0) {
      return "maxTotalTokens";
    }
    if (!Number.isFinite(form.maxCostUsd) || form.maxCostUsd < 0) {
      return "maxCostUsd";
    }
    if (
      !Number.isInteger(form.windowHours)
      || form.windowHours < WINDOW_MIN
      || form.windowHours > WINDOW_MAX
    ) {
      return "windowHours";
    }
    return null;
  }, [form]);

  const onSave = useCallback(async () => {
    const invalid = validate();
    if (invalid) {
      setErrorText(`${t("bot.quota.invalidRange")}: ${labels[invalid]}`);
      return;
    }
    setErrorText(null);
    const input: BotCostQuotaInput = { scope, scopeId, ...form };
    try {
      const saved = await setBotCostQuota(input);
      setQuota(saved);
      setForm(toForm(saved));
      message.success(t("bots.saved"));
      setRefreshSeq((n) => n + 1);
    } catch (e) {
      setErrorText(backendErrorText(e));
    }
  }, [form, labels, message, scope, scopeId, setBotCostQuota, t, validate]);

  const onReset = useCallback(async () => {
    setErrorText(null);
    try {
      await resetBotCostQuota(scope, scopeId);
      setQuota(null);
      setForm(toForm(null));
      message.success(t("bots.saved"));
      setRefreshSeq((n) => n + 1);
    } catch (e) {
      setErrorText(backendErrorText(e));
    }
  }, [message, resetBotCostQuota, scope, scopeId, t]);

  const columns: TableProps<BotUsageEntry>["columns"] = [
    {
      title: t("bot.usage.window"),
      key: "createdAt",
      render: (_, row) => new Date(row.createdAt).toLocaleString(),
    },
    {
      title: t("bot.usage.pricingSource"),
      dataIndex: "pricingSource",
      key: "pricingSource",
    },
    {
      title: t("bot.usage.totalTokens"),
      dataIndex: "totalTokens",
      key: "totalTokens",
      render: (value: number) => formatTokens(value),
    },
    {
      // 估算标识随列标题出现（ADR-P5-4：费用列不得裸奔）。
      title: `${t("bot.usage.totalCost")} (${t("bot.usage.estimated")})`,
      dataIndex: "estimatedCostUsd",
      key: "estimatedCostUsd",
      render: (value: number) => formatUsd(value),
    },
  ];

  const tokenRows: Array<{ key: string; label: string; value: number }> = [
    { key: "inputTokens", label: t("bot.usage.inputTokens"), value: summary?.inputTokens ?? 0 },
    { key: "outputTokens", label: t("bot.usage.outputTokens"), value: summary?.outputTokens ?? 0 },
    {
      key: "cacheCreationTokens",
      label: t("bot.usage.cacheCreationTokens"),
      value: summary?.cacheCreationTokens ?? 0,
    },
    { key: "cacheReadTokens", label: t("bot.usage.cacheReadTokens"), value: summary?.cacheReadTokens ?? 0 },
  ];

  return (
    <div data-testid="bot-usage-panel" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
        <div>
          <Text strong>{t("bot.usage.title")}</Text>
          <div>
            <Text type="secondary" style={{ fontSize: 12 }}>{t("bot.usage.subtitle")}</Text>
          </div>
        </div>
        <Space>
          {summary?.breached
            ? <Tag color="red" data-testid="bot-usage-breached">{t("bot.usage.breached")}</Tag>
            : null}
          <Button
            size="small"
            onClick={() => setRefreshSeq((n) => n + 1)}
            loading={loading}
            data-testid="bot-usage-refresh"
          >
            {t("bot.usage.refresh")}
          </Button>
        </Space>
      </div>

      {loading && !summary
        ? <List size="small" loading />
        : (
          <>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "6px 16px" }}>
              {tokenRows.map((row) => (
                <div key={row.key} style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                  <Text type="secondary">{row.label}</Text>
                  <Text data-testid={`bot-usage-${row.key}`}>{formatTokens(row.value)}</Text>
                </div>
              ))}
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                <Text>{t("bot.usage.totalTokens")}</Text>
                <Text strong data-testid="bot-usage-total-tokens">
                  {formatTokens(summary?.totalTokens ?? 0)}
                </Text>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                <Text>{t("bot.usage.totalCost")}</Text>
                <Space size={4}>
                  <Text strong data-testid="bot-usage-total-cost">
                    {formatUsd(summary?.estimatedCostUsd ?? 0)}
                  </Text>
                  <Tooltip title={t("bot.usage.estimatedHint")}>
                    <Tag data-testid="bot-usage-estimated">{t("bot.usage.estimated")}</Tag>
                  </Tooltip>
                </Space>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                <Text type="secondary">{t("bot.usage.window")}</Text>
                <Text type="secondary" data-testid="bot-usage-window">
                  {summary?.windowHours ?? 168}h
                </Text>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                <Text type="secondary">{t("bot.usage.pricingSource")}</Text>
                <Text type="secondary" data-testid="bot-usage-pricing-source">
                  {Array.from(new Set(entries.map((entry) => entry.pricingSource))).join(", ") || "—"}
                </Text>
              </div>
            </div>
            {summary?.pricingNote
              ? <Text type="secondary" style={{ fontSize: 12 }}>{summary.pricingNote}</Text>
              : null}
            {entries.length === 0
              ? <Text type="secondary" data-testid="bot-usage-empty">{t("bot.usage.empty")}</Text>
              : (
                <Table<BotUsageEntry>
                  size="small"
                  rowKey="id"
                  columns={columns}
                  dataSource={entries}
                  pagination={{ pageSize: 5, hideOnSinglePage: true }}
                />
              )}
          </>
        )}

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div>
          <Text strong>{t("bot.usage.quotaTitle")}</Text>
          <div>
            <Text type="secondary" style={{ fontSize: 12 }}>{t("bot.usage.quotaHint")}</Text>
          </div>
        </div>
        {quota?.isDefault
          ? (
            <Text type="secondary" data-testid="bot-usage-quota-default-hint" style={{ fontSize: 12 }}>
              {t("bot.quota.inheritHint")}
            </Text>
          )
          : null}
        {(Object.keys(form) as CostField[]).map((field) => (
          <div
            key={field}
            style={{ display: "grid", gridTemplateColumns: "1fr 160px", gap: 8, alignItems: "center" }}
          >
            <div>{labels[field]}</div>
            <Input
              data-testid={`bot-usage-quota-${field}`}
              type="number"
              min={field === "windowHours" ? WINDOW_MIN : 0}
              max={field === "windowHours" ? WINDOW_MAX : undefined}
              value={String(form[field])}
              onChange={(e) => {
                const value = Number(e.target.value);
                setForm((prev) => ({ ...prev, [field]: Number.isNaN(value) ? 0 : value }));
              }}
            />
          </div>
        ))}
        {errorText
          ? <Text type="danger" data-testid="bot-usage-quota-error">{errorText}</Text>
          : null}
        <Space>
          <Button
            type="primary"
            onClick={() => {
              void onSave();
            }}
            loading={saving}
            data-testid="bot-usage-quota-save"
          >
            {t("bot.quota.save")}
          </Button>
          <Popconfirm
            title={t("bot.quota.reset")}
            onConfirm={() => {
              void onReset();
            }}
            okText={t("common.confirm")}
            cancelText={t("common.cancel")}
          >
            <Button disabled={saving} data-testid="bot-usage-quota-reset">{t("bot.quota.reset")}</Button>
          </Popconfirm>
        </Space>
      </div>
    </div>
  );
}
