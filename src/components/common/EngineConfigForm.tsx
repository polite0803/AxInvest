// SPDX-License-Identifier: AGPL-3.0-only

/**
 * EngineConfigForm — 字段声明驱动的通用配置表单（纯展示，无 store 依赖）。
 *
 * 2026-11-07（任务 #193，组件中立性 B 类）由 `src/components/settings/EngineConfigForm.tsx`
 * 下沉至中性目录 `common/`：调用方跨两个 feature（`settings/EngineDetailPanel` 与
 * `devtools/RLTrainingConfig`），而组件本身只吃 `config` + `fields` + `onSave`，
 * 不读任何 feature store ⇒ 属跨 feature 共用基元。
 */

import { Button, Form, Input, InputNumber, Select, Switch } from "antd";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

export interface ConfigField {
  key: string;
  label: string;
  type: "text" | "number" | "select" | "switch" | "slider";
  options?: { label: string; value: string }[];
  min?: number;
  max?: number;
  step?: number;
}

interface EngineConfigFormProps {
  config: Record<string, unknown>;
  fields: ConfigField[];
  onSave: (config: Record<string, unknown>) => void;
}

export function EngineConfigForm({ config, fields, onSave }: EngineConfigFormProps) {
  const { t } = useTranslation();
  const [form] = Form.useForm();

  useEffect(() => {
    form.setFieldsValue(config);
  }, [config, form]);

  const handleFinish = (values: Record<string, unknown>) => {
    onSave(values);
  };

  return (
    <Form
      form={form}
      layout="vertical"
      size="small"
      onFinish={handleFinish}
      initialValues={config}
    >
      {fields.map((field) => (
        <Form.Item
          key={field.key}
          name={field.key}
          label={field.label}
          valuePropName={field.type === "switch" ? "checked" : "value"}
        >
          {field.type === "switch" ? <Switch /> : field.type === "number"
            ? (
              <InputNumber
                style={{ width: "100%" }}
                min={field.min}
                max={field.max}
                step={field.step}
              />
            )
            : field.type === "select"
            ? (
              <Select
                options={field.options?.map((o) => ({ label: o.label, value: o.value }))}
              />
            )
            : field.type === "slider"
            ? (
              <InputNumber
                style={{ width: "100%" }}
                min={field.min ?? 0}
                max={field.max ?? 100}
                step={field.step ?? 1}
              />
            )
            : <Input />}
        </Form.Item>
      ))}

      <Form.Item>
        <Button type="primary" htmlType="submit">
          {t("settings.evolution.saveConfig")}
        </Button>
      </Form.Item>
    </Form>
  );
}
