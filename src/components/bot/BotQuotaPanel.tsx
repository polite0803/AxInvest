import { backendErrorText } from "@/lib/errorI18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { BotDepartmentQuota } from "@/types";
import { App, Button, Input, Popconfirm, Skeleton, Space, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

const LIMITS = {
  maxConcurrency: { min: 1, max: 4, defaultValue: 2 },
  totalTimeoutSecs: { min: 60, max: 3600, defaultValue: 900 },
  maxSteps: { min: 1, max: 12, defaultValue: 12 },
  maxInFlightPerDepth: { min: 1, max: 4, defaultValue: 2 },
  stepTimeoutSecs: { min: 30, max: 900, defaultValue: 300 },
  maxConcurrentRuns: { min: 1, max: 8, defaultValue: 2 },
} as const;

type FieldKey = keyof typeof LIMITS;
type QuotaForm = Record<FieldKey, number>;

const FIELDS: FieldKey[] = [
  "maxConcurrency",
  "totalTimeoutSecs",
  "maxSteps",
  "maxInFlightPerDepth",
  "stepTimeoutSecs",
  "maxConcurrentRuns",
];

function toForm(quota: BotDepartmentQuota | null): QuotaForm {
  return {
    maxConcurrency: quota?.maxConcurrency ?? LIMITS.maxConcurrency.defaultValue,
    totalTimeoutSecs: quota?.totalTimeoutSecs ?? LIMITS.totalTimeoutSecs.defaultValue,
    maxSteps: quota?.maxSteps ?? LIMITS.maxSteps.defaultValue,
    maxInFlightPerDepth: quota?.maxInFlightPerDepth ?? LIMITS.maxInFlightPerDepth.defaultValue,
    stepTimeoutSecs: quota?.stepTimeoutSecs ?? LIMITS.stepTimeoutSecs.defaultValue,
    maxConcurrentRuns: quota?.maxConcurrentRuns ?? LIMITS.maxConcurrentRuns.defaultValue,
  };
}

export function BotQuotaPanel({ departmentId }: { departmentId: string }) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const quota = useBotStore((s) => s.departmentQuota);
  const quotaLoading = useBotStore((s) => s.quotaLoading);
  const quotaSaving = useBotStore((s) => s.quotaSaving);
  const getDepartmentQuota = useBotStore((s) => s.getDepartmentQuota);
  const setDepartmentQuota = useBotStore((s) => s.setDepartmentQuota);
  const resetDepartmentQuota = useBotStore((s) => s.resetDepartmentQuota);

  const [form, setForm] = useState<QuotaForm>(() => toForm(null));
  const [errorText, setErrorText] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setErrorText(null);
    void getDepartmentQuota(departmentId)
      .then((q) => {
        if (alive) { setForm(toForm(q)); }
      })
      .catch((e) => {
        if (alive) { setErrorText(backendErrorText(e)); }
      });
    return () => {
      alive = false;
    };
  }, [departmentId, getDepartmentQuota]);

  useEffect(() => {
    if (quota?.departmentId === departmentId) {
      setForm(toForm(quota));
    }
  }, [quota, departmentId]);

  const labels: Record<FieldKey, { label: string; hint: string; testId: string }> = useMemo(
    () => ({
      maxConcurrency: {
        label: t("bot.quota.maxConcurrency"),
        hint: t("bot.quota.maxConcurrencyHint"),
        testId: "bot-quota-max-concurrency",
      },
      totalTimeoutSecs: {
        label: t("bot.quota.totalTimeout"),
        hint: t("bot.quota.totalTimeoutHint"),
        testId: "bot-quota-total-timeout",
      },
      maxSteps: {
        label: t("bot.quota.maxSteps"),
        hint: t("bot.quota.maxStepsHint"),
        testId: "bot-quota-max-steps",
      },
      maxInFlightPerDepth: {
        label: t("bot.quota.perDepth"),
        hint: t("bot.quota.perDepthHint"),
        testId: "bot-quota-max-in-flight",
      },
      stepTimeoutSecs: {
        label: t("bot.quota.stepTimeout"),
        hint: t("bot.quota.stepTimeoutHint"),
        testId: "bot-quota-step-timeout",
      },
      maxConcurrentRuns: {
        label: t("bot.quota.maxRuns"),
        hint: t("bot.quota.maxRunsHint"),
        testId: "bot-quota-max-runs",
      },
    }),
    [t],
  );

  const validate = useCallback((): FieldKey | null => {
    for (const field of FIELDS) {
      const value = form[field];
      const { min, max } = LIMITS[field];
      if (!Number.isInteger(value) || value < min || value > max) { return field; }
    }
    return null;
  }, [form]);

  const onSave = useCallback(async () => {
    const invalid = validate();
    if (invalid) {
      setErrorText(`${t("bot.quota.invalidRange")}: ${labels[invalid].label}`);
      return;
    }
    setErrorText(null);
    try {
      await setDepartmentQuota({ departmentId, ...form });
      message.success(t("bot.quota.saved"));
    } catch (e) {
      const raw = backendErrorText(e);
      setErrorText(raw);
    }
  }, [departmentId, form, labels, message, setDepartmentQuota, t, validate]);

  const onReset = useCallback(async () => {
    setErrorText(null);
    try {
      await resetDepartmentQuota(departmentId);
      const next = await getDepartmentQuota(departmentId);
      setForm(toForm(next));
      message.success(t("bot.quota.saved"));
    } catch (e) {
      setErrorText(backendErrorText(e));
    }
  }, [departmentId, getDepartmentQuota, message, resetDepartmentQuota, t]);

  if (quotaLoading && quota?.departmentId !== departmentId) {
    return <Skeleton active paragraph={{ rows: 6 }} />;
  }

  return (
    <div data-testid="bot-quota-panel" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <div>
        <Text strong>{t("bot.quota.title")}</Text>
        <div>
          <Text type="secondary" style={{ fontSize: 12 }}>{t("bot.quota.subtitle")}</Text>
        </div>
      </div>
      {quota?.isDefault
        ? (
          <Text type="secondary" data-testid="bot-quota-default-hint" style={{ fontSize: 12 }}>
            {t("bot.quota.inheritHint")}
          </Text>
        )
        : null}
      {FIELDS.map((field) => (
        <div key={field} style={{ display: "grid", gridTemplateColumns: "1fr 160px", gap: 8, alignItems: "center" }}>
          <div>
            <div>{labels[field].label}</div>
            <Text type="secondary" style={{ fontSize: 12 }}>{labels[field].hint}</Text>
          </div>
          <Input
            data-testid={labels[field].testId}
            type="number"
            min={LIMITS[field].min}
            max={LIMITS[field].max}
            value={String(form[field])}
            onChange={(e) => {
              const value = Number.parseInt(e.target.value, 10);
              setForm((prev) => ({ ...prev, [field]: Number.isNaN(value) ? 0 : value }));
            }}
          />
        </div>
      ))}
      {errorText ? <Text type="danger" data-testid="bot-quota-error">{errorText}</Text> : null}
      <Space>
        <Button
          type="primary"
          onClick={() => {
            void onSave();
          }}
          loading={quotaSaving}
          data-testid="bot-quota-save"
        >
          {t("bot.quota.save")}
        </Button>
        <Popconfirm
          title={t("bot.quota.resetConfirm")}
          onConfirm={() => {
            void onReset();
          }}
          okText={t("common.confirm")}
        >
          <Button disabled={quotaSaving} data-testid="bot-quota-reset">{t("bot.quota.reset")}</Button>
        </Popconfirm>
      </Space>
    </div>
  );
}
