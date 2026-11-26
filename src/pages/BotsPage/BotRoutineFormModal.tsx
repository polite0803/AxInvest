// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 例程创建/编辑表单（P2-a T4'）。
 *
 * - cron 校验复用任务中心的 `validateCronSyntax`（纯逻辑，5 段 + 拒每分钟），
 *   报错文案走 `bots.routineCronInvalid`（与 §2.1 后端错误码同源语义）。
 * - 时区：下拉预设宿主 Local（= 后端 null 语义）+ 常见 IANA 清单 + 自由输入校验
 *   （`Intl.supportedValuesOf("timeZone")`，与 GATES-P2 §2 A3 的 JS 侧解析口径一致）。
 * - 创建走 `create_bot_scheduled_task`；编辑走 `update_scheduled_task`
 *   完整 DTO 回传 + 顶层 timezone 参数（undefined=不改 / null=清除）。
 */

import { backendErrorText } from "@/lib/errorI18n";
import { validateCronSyntax } from "@/pages/TasksCenterPage/CronScheduleBuilder";
import { useBotStore } from "@/stores/feature/botStore";
import type { ScheduledTask } from "@/types";
import { App, Form, Input, Modal, Select } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

/** 时区下拉的预设清单（常用 IANA 名；「跟随系统」= null 另行表达） */
const COMMON_TIMEZONES = [
  "Asia/Shanghai",
  "Asia/Hong_Kong",
  "Asia/Taipei",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Asia/Seoul",
  "Asia/Kolkata",
  "Asia/Dubai",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Paris",
  "Europe/Moscow",
  "America/New_York",
  "America/Chicago",
  "America/Los_Angeles",
  "UTC",
] as const;

/** IANA 名是否可解析（与 GATES-P2 §2 A3 同口径：拒 UTC+8 这类偏移串）。导出供单测直断。 */
export function isValidTimeZone(tz: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: tz }).format(new Date());
    return true;
  } catch {
    return false;
  }
}

export interface BotRoutineFormModalProps {
  open: boolean;
  botId: string;
  /** 非空 = 编辑态 */
  editing: ScheduledTask | null;
  onClose: () => void;
  onSaved: () => void;
}

interface RoutineFormValues {
  name: string;
  prompt: string;
  cronExpression: string;
  /** undefined/空 = 跟随系统（null）；否则 IANA 名 */
  timezone?: string;
}

export function BotRoutineFormModal({ open, botId, editing, onClose, onSaved }: BotRoutineFormModalProps) {
  const { t } = useTranslation();
  const { message: antdMessage } = App.useApp();
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const createBotRoutine = useBotStore((s) => s.createBotRoutine);
  const updateBotRoutine = useBotStore((s) => s.updateBotRoutine);

  const isEdit = editing !== null;

  useEffect(() => {
    if (!open) {
      return;
    }
    setSaving(false);
    if (editing) {
      form.setFieldsValue({
        name: editing.name,
        prompt: editing.description,
        cronExpression: editing.cronExpression ?? "",
        timezone: editing.timezone ?? undefined,
      });
    } else {
      form.resetFields();
      form.setFieldsValue({ cronExpression: "0 9 * * *" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, editing]);

  const cronValidator = useCallback(
    (_rule: unknown, value: string) => {
      // 空值即无效（必填语义由 required 规则先挡；这里兜「纯空格」）
      if (!(value ?? "").trim()) {
        return Promise.reject(new Error(t("bots.routineCronInvalid")));
      }
      const errorKey = validateCronSyntax(value ?? "");
      // 任务中心返回 settings.* 文案键；Bot 例程按权威清单用 bots.routineCronInvalid
      return errorKey ? Promise.reject(new Error(t("bots.routineCronInvalid"))) : Promise.resolve();
    },
    [t],
  );

  const timezoneValidator = useCallback(
    (_rule: unknown, value: string | undefined) => {
      if (!value) {
        return Promise.resolve(); // 空 = 跟随系统（null）
      }
      return isValidTimeZone(value)
        ? Promise.resolve()
        : Promise.reject(new Error(t("bots.routineTimezoneInvalid")));
    },
    [t],
  );

  // Select 也允许自定义输入（showSearch + 非预设值）：optionFilterPrim 兜搜索
  const timezoneOptions = useMemo(
    () => [
      { value: "__local__", label: t("bots.routineTimezoneLocal") },
      ...COMMON_TIMEZONES.map((tz) => ({ value: tz, label: tz })),
    ],
    [t],
  );

  const handleOk = useCallback(async () => {
    // validateFields 校验失败会 reject（antd 约定）；吞掉这个 rejection，
    // 错误展示由 Form.Item 自身的 rules 完成 —— 不让未处理 rejection 冒到全局。
    let values: RoutineFormValues;
    try {
      values = (await form.validateFields()) as RoutineFormValues;
    } catch {
      return;
    }
    const timezone = !values.timezone || values.timezone === "__local__" ? null : values.timezone;
    setSaving(true);
    try {
      if (editing) {
        // P1 语义：完整 DTO 回传（config/status/createdAt 等以原任务为底原样回传）
        const updatedTask: ScheduledTask = {
          ...editing,
          name: values.name,
          description: values.prompt,
          cronExpression: values.cronExpression.trim(),
        };
        await updateBotRoutine(editing.id, updatedTask, timezone);
      } else {
        await createBotRoutine({
          botId,
          name: values.name,
          prompt: values.prompt,
          cronExpression: values.cronExpression.trim(),
          timezone,
        });
      }
      setSaving(false);
      onSaved();
      onClose();
    } catch (e) {
      setSaving(false);
      antdMessage.error(backendErrorText(e));
    }
  }, [botId, createBotRoutine, editing, form, onClose, onSaved, t, updateBotRoutine, antdMessage]);

  return (
    <Modal
      title={isEdit ? t("bots.routineName") : t("bots.routineAdd")}
      open={open}
      onOk={() => void handleOk()}
      onCancel={onClose}
      confirmLoading={saving}
      okText={isEdit ? t("common.edit") : t("bots.routineAdd")}
      cancelText={t("common.cancel")}
      destroyOnHidden
      data-testid="bot-routine-modal"
    >
      <Form form={form} layout="vertical">
        <Form.Item
          name="name"
          label={t("bots.routineName")}
          rules={[{ required: true, message: t("bots.routineNameRequired") }]}
        >
          <Input placeholder={t("bots.routineName")} />
        </Form.Item>
        <Form.Item
          name="prompt"
          label={t("bots.routinePrompt")}
          rules={[{ required: true, message: t("bots.routinePromptRequired") }]}
        >
          <Input.TextArea rows={3} placeholder={t("bots.routinePrompt")} />
        </Form.Item>
        <Form.Item
          name="cronExpression"
          label={t("bots.routineCron")}
          extra={t("bots.routineCronHint")}
          rules={[{ required: true }, { validator: cronValidator }]}
        >
          <Input placeholder="0 9 * * *" />
        </Form.Item>
        <Form.Item name="timezone" label={t("bots.routineTimezone")} rules={[{ validator: timezoneValidator }]}>
          <Select
            showSearch
            allowClear
            placeholder={t("bots.routineTimezoneLocal")}
            options={timezoneOptions}
            // 自由输入非预设时区（例如用户机器上的冷门 IANA 名）
            filterOption={(input, option) => (option?.label ?? "").toLowerCase().includes(input.toLowerCase())}
          />
        </Form.Item>
      </Form>
    </Modal>
  );
}
