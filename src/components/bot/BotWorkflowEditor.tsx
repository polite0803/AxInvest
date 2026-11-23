// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 编排定义编辑器（T6'''）。步骤卡片 / 依赖勾选 / Bot 下拉 / 失败策略 / 预算。
 *
 * 组件中立性：Bot 与部门数据由页面注入；store 只由页面读写。
 * 提交前跑 validateWorkflowSteps 纯函数，错误码与后端 IPC-P3 §4 一致，
 * 后端仍是最终裁决者；前端校验只减少无效 IPC。
 */

import { validateWorkflowSteps, WORKFLOW_MAX_CONCURRENCY, workflowErrorI18nKey } from "@/lib/botWorkflowValidation";
import { backendErrorText } from "@/lib/errorI18n";
import type {
  Bot,
  BotDepartment,
  BotWorkflow,
  BotWorkflowStep,
  BotWorkflowStepOnFailure,
  BotWorkflowValidationIssue,
  CreateBotWorkflowInput,
} from "@/types";
import { Alert, App, Button, Card, Checkbox, Form, Input, InputNumber, Select, Space, Tag, Typography } from "antd";
import { Plus, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { TextArea } = Input;
const { Text } = Typography;

interface EditorFormValues {
  name: string;
  description?: string;
  ownerScope: "instance" | "department";
  departmentId?: string | null;
  maxConcurrency: number;
  totalTimeoutSecs: number;
}

export interface BotWorkflowEditorProps {
  bots: Bot[];
  departments: BotDepartment[];
  workflows: BotWorkflow[];
  editing: BotWorkflow | null;
  saving?: boolean;
  /** id 为 null 表示新建，否则更新。 */
  onSubmit: (input: CreateBotWorkflowInput, id: string | null) => Promise<void>;
  onCancelEdit: () => void;
}

function makeStep(usedKeys: string[]): BotWorkflowStep {
  let index = usedKeys.length + 1;
  while (usedKeys.includes(`step${index}`)) {
    index += 1;
  }
  return {
    stepKey: `step${index}`,
    botId: "",
    promptTemplate: "",
    dependsOn: [],
    timeoutSecs: 300,
    onFailure: "abort",
  };
}

export function BotWorkflowEditor({
  bots,
  departments,
  workflows,
  editing,
  saving = false,
  onSubmit,
  onCancelEdit,
}: BotWorkflowEditorProps) {
  const { t } = useTranslation();
  const { message: antdMessage } = App.useApp();
  const [form] = Form.useForm<EditorFormValues>();
  const [steps, setSteps] = useState<BotWorkflowStep[]>([makeStep([])]);
  const [issues, setIssues] = useState<BotWorkflowValidationIssue[]>([]);
  const [submitting, setSubmitting] = useState(false);

  const botOptions = useMemo(
    () => bots.filter((bot) => bot.isEnabled).map((bot) => ({ value: bot.id, label: bot.name })),
    [bots],
  );
  const departmentOptions = useMemo(
    () => departments.map((department) => ({ value: department.id, label: department.name })),
    [departments],
  );

  useEffect(() => {
    if (editing) {
      form.setFieldsValue({
        name: editing.name,
        description: editing.description,
        ownerScope: editing.ownerScope,
        departmentId: editing.departmentId,
        maxConcurrency: editing.maxConcurrency,
        totalTimeoutSecs: editing.totalTimeoutSecs,
      });
      setSteps(
        editing.steps.length > 0
          ? editing.steps.map((step) => ({ ...step, dependsOn: [...step.dependsOn] }))
          : [makeStep([])],
      );
    } else {
      form.resetFields();
      form.setFieldsValue({ ownerScope: "instance", maxConcurrency: 2, totalTimeoutSecs: 900 });
      setSteps([makeStep([])]);
    }
    setIssues([]);
  }, [editing, form]);

  const patchStep = (index: number, patch: Partial<BotWorkflowStep>) => {
    setSteps((prev) => prev.map((step, i) => (i === index ? { ...step, ...patch } : step)));
    setIssues([]);
  };

  const handleSubmit = async (values: EditorFormValues) => {
    const result = validateWorkflowSteps(steps);
    if (!result.ok) {
      setIssues(result.issues);
      return;
    }
    if (values.ownerScope === "department" && !values.departmentId) {
      setIssues([{ code: "BOT_WORKFLOW_DEPARTMENT_REQUIRED" }]);
      return;
    }
    const input: CreateBotWorkflowInput = {
      name: values.name,
      description: values.description ?? "",
      ownerScope: values.ownerScope,
      departmentId: values.ownerScope === "department" ? values.departmentId : null,
      steps,
      maxConcurrency: values.maxConcurrency,
      totalTimeoutSecs: values.totalTimeoutSecs,
    };
    setSubmitting(true);
    try {
      await onSubmit(input, editing?.id ?? null);
      setIssues([]);
      if (!editing) {
        form.resetFields();
        form.setFieldsValue({ ownerScope: "instance", maxConcurrency: 2, totalTimeoutSecs: 900 });
        setSteps([makeStep([])]);
      }
    } catch (e) {
      antdMessage.error(backendErrorText(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Card size="small" title={t(editing ? "bot.workflow.edit" : "bot.workflow.new")} data-testid="bot-workflow-editor">
      <Form<EditorFormValues>
        form={form}
        layout="vertical"
        initialValues={{ ownerScope: "instance", maxConcurrency: 2, totalTimeoutSecs: 900 }}
        onFinish={(values) => {
          void handleSubmit(values);
        }}
      >
        <Form.Item
          name="name"
          label={t("bot.workflow.name")}
          rules={[{ required: true, whitespace: true, message: t("bot.workflow.nameRequired") }]}
        >
          <Input maxLength={100} data-testid="bot-workflow-name" />
        </Form.Item>
        <Form.Item name="description" label={t("bot.workflow.description")}>
          <Input.TextArea rows={2} maxLength={2000} data-testid="bot-workflow-description" />
        </Form.Item>
        <Form.Item name="ownerScope" label={t("bot.workflow.ownerScope")}>
          <Select
            data-testid="bot-workflow-owner-scope"
            options={[
              { value: "instance", label: t("bot.workflow.ownerInstance") },
              { value: "department", label: t("bot.workflow.ownerDepartment") },
            ]}
          />
        </Form.Item>
        <Form.Item name="departmentId" label={t("bot.workflow.department")}>
          <Select
            allowClear
            placeholder={t("bot.workflow.departmentPlaceholder")}
            options={departmentOptions}
            data-testid="bot-workflow-department"
          />
        </Form.Item>
        <Form.Item name="maxConcurrency" label={t("bot.workflow.maxConcurrency")}>
          <InputNumber min={1} max={WORKFLOW_MAX_CONCURRENCY} data-testid="bot-workflow-concurrency" />
        </Form.Item>
        <Form.Item name="totalTimeoutSecs" label={t("bot.workflow.totalTimeoutSecs")}>
          <InputNumber min={60} max={3600} step={60} data-testid="bot-workflow-total-timeout" />
        </Form.Item>

        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <Text strong>{t("bot.workflow.steps")}</Text>
          <Button
            size="small"
            icon={<Plus size={14} />}
            data-testid="bot-workflow-add-step"
            onClick={() =>
              setSteps((prev) => [
                ...prev,
                makeStep(prev.map((s) =>
                  s.stepKey
                )),
              ])}
          >
            {t("bot.workflow.addStep")}
          </Button>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 8 }}>
          {steps.map((step, index) => (
            <Card
              size="small"
              key={`${index}-${step.stepKey}`}
              title={
                <Space size={4}>
                  <Tag data-testid="bot-workflow-step-index">{index + 1}</Tag>
                  <Text>{step.stepKey || t("bot.workflow.stepKey")}</Text>
                </Space>
              }
              extra={
                <Button
                  type="text"
                  size="small"
                  danger
                  icon={<Trash2 size={14} />}
                  disabled={steps.length <= 1}
                  data-testid="bot-workflow-remove-step"
                  onClick={() =>
                    setSteps((prev) => prev.filter((_, i) => i !== index))}
                >
                  {t("bot.workflow.removeStep")}
                </Button>
              }
              data-testid="bot-workflow-step-card"
            >
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                <Form.Item label={t("bot.workflow.stepKey")}>
                  <Input
                    value={step.stepKey}
                    maxLength={32}
                    data-testid="bot-workflow-step-key"
                    onChange={(e) =>
                      patchStep(index, { stepKey: e.target.value })}
                  />
                </Form.Item>
                <Form.Item label={t("bot.workflow.assignee")}>
                  <Select
                    value={step.botId || undefined}
                    options={botOptions}
                    data-testid="bot-workflow-step-bot"
                    onChange={(value) => patchStep(index, { botId: value })}
                  />
                </Form.Item>
              </div>
              <Form.Item label={t("bot.workflow.prompt")}>
                <TextArea
                  rows={3}
                  maxLength={20000}
                  value={step.promptTemplate}
                  data-testid="bot-workflow-step-prompt"
                  onChange={(e) => patchStep(index, { promptTemplate: e.target.value })}
                />
              </Form.Item>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 8 }}>
                <Form.Item label={t("bot.workflow.dependsOn")}>
                  <Checkbox.Group
                    value={step.dependsOn}
                    options={steps
                      .filter((candidate) => candidate.stepKey && candidate.stepKey !== step.stepKey)
                      .map((candidate) => ({ label: candidate.stepKey, value: candidate.stepKey }))}
                    data-testid="bot-workflow-step-depends"
                    onChange={(value) => patchStep(index, { dependsOn: value.map(String) })}
                  />
                </Form.Item>
                <Form.Item label={t("bot.workflow.timeoutSecs")}>
                  <InputNumber
                    min={1}
                    max={300}
                    value={step.timeoutSecs}
                    data-testid="bot-workflow-step-timeout"
                    onChange={(value) => patchStep(index, { timeoutSecs: value ?? 300 })}
                  />
                </Form.Item>
                <Form.Item label={t("bot.workflow.onFailure")}>
                  <Select<BotWorkflowStepOnFailure>
                    value={step.onFailure}
                    data-testid="bot-workflow-step-on-failure"
                    options={[
                      { value: "abort", label: t("bot.workflow.onFailureAbort") },
                      { value: "continue", label: t("bot.workflow.onFailureContinue") },
                    ]}
                    onChange={(value) => patchStep(index, { onFailure: value })}
                  />
                </Form.Item>
              </div>
            </Card>
          ))}
        </div>

        {issues.length > 0
          ? (
            <Alert
              type="error"
              showIcon
              style={{ marginTop: 8 }}
              data-testid="bot-workflow-validation-errors"
              message={issues
                .map((item) => {
                  const key = workflowErrorI18nKey(item.code);
                  return key ? t(key) : item.code;
                })
                .join("；")}
            />
          )
          : null}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 12 }}>
          {editing
            ? (
              <Button onClick={onCancelEdit} data-testid="bot-workflow-cancel-edit">
                {t("common.cancel")}
              </Button>
            )
            : null}
          <Button type="primary" htmlType="submit" loading={submitting || saving} data-testid="bot-workflow-save">
            {t("bot.workflow.save")}
          </Button>
        </div>
      </Form>
      <Text type="secondary" style={{ fontSize: 12 }}>
        {`${workflows.length} ${t("bot.workflow.title")}`}
      </Text>
    </Card>
  );
}
