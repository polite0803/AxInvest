// SPDX-License-Identifier: AGPL-3.0-only

import { showBackendError } from "@/lib/errorI18n";
import { invoke, logIpcError } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import type { ScheduledTask, TaskConfig } from "@/types";
import {
  Alert,
  App,
  Button,
  Collapse,
  Divider,
  Form,
  Input,
  InputNumber,
  Modal,
  Radio,
  Select,
  Switch,
  theme,
} from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { CronScheduleBuilder, validateCronSyntax } from "../CronScheduleBuilder";
import { getInvestKindDef, INVEST_KIND_DEFS, type InvestTaskKind } from "./investTaskDefs";
import { InvestTaskFields } from "./InvestTaskFields";

/** 后端 get_scheduled_task_templates 返回结构（snake_case 原样：后端未做 rename） */
interface TaskTemplate {
  templateType?: string;
  template_type: string;
  name: string;
  description: string;
  workflow_id: string | null;
  default_schedule?: string;
}

type CreateCategory = "invest" | "custom" | "system";

interface SystemQuickDef {
  type: "backup" | "cleanup" | "daily_summary";
  command: string;
  /** 后端各命令内置默认时点（backup 02:00 / cleanup 周日 03:00 / summary 09:00） */
  defaultCron: string;
}

/** 系统托管任务：一键创建（后端命令要求必填 name，默认时点见各 defaultCron） */
const SYSTEM_QUICK_DEFS: SystemQuickDef[] = [
  { type: "backup", command: "create_backup_task", defaultCron: "0 2 * * *" },
  { type: "cleanup", command: "create_cleanup_task", defaultCron: "0 3 * * 0" },
  { type: "daily_summary", command: "create_daily_summary_task", defaultCron: "0 9 * * *" },
];

export interface TaskCreateModalProps {
  open: boolean;
  /** 非空 = 编辑态（仅名称/描述/cron；内置投资任务无更新命令，不允许编辑） */
  editing: ScheduledTask | null;
  onClose: () => void;
  /** 创建/更新/快速创建成功后的回调（通常用于刷新列表） */
  onSaved: () => void;
}

/**
 * 外层门控：首次 open 前不挂载实现体。实现体内部 Form.useForm 与 <Form> 同生命周期挂载，
 * 规避 antd「Instance created by useForm is not connected to any Form element」dev 警告；
 * 首次打开后常驻，以保留 Modal 关闭动画与 destroyOnHidden 的重挂载时序。
 */
export function TaskCreateModal(props: TaskCreateModalProps) {
  const [everOpened, setEverOpened] = useState(props.open);
  useEffect(() => {
    if (props.open) {
      setEverOpened(true);
    }
  }, [props.open]);
  if (!everOpened) {
    return null;
  }
  return <TaskCreateModalImpl {...props} />;
}

function TaskCreateModalImpl({ open, editing, onClose, onSaved }: TaskCreateModalProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const { token } = theme.useToken();
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);

  const [category, setCategory] = useState<CreateCategory>("invest");
  const [investKind, setInvestKind] = useState<InvestTaskKind>("stock-analysis");
  const [templates, setTemplates] = useState<TaskTemplate[]>([]);
  const [selectedTemplate, setSelectedTemplate] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const [investForm] = Form.useForm();
  const [customForm] = Form.useForm();

  const isEdit = editing !== null;

  const cronValidator = useCallback(
    (_rule: unknown, value: string) => {
      // 校验行为与设置页旧定时任务面板完全一致：纯逻辑复用 validateCronSyntax
      const errorKey = validateCronSyntax(value ?? "");
      return errorKey ? Promise.reject(new Error(t(errorKey))) : Promise.resolve();
    },
    [t],
  );

  // 打开时初始化：编辑态回填；新建态拉模板 + 重置投资表单
  useEffect(() => {
    if (!open) {
      return;
    }
    setSaving(false);
    setSelectedTemplate(null);
    if (editing) {
      setCategory("custom");
      customForm.setFieldsValue({
        name: editing.name,
        description: editing.description,
        cronExpression: editing.cronExpression,
        workflow_id: editing.workflowId ?? null,
      });
      return;
    }
    setCategory("invest");
    setInvestKind("stock-analysis");
    investForm.resetFields();
    investForm.setFieldsValue({
      ...getInvestKindDef("stock-analysis").initialValues,
      stockCode,
      stockName,
    });
    customForm.resetFields();
    customForm.setFieldsValue({
      cronExpression: "0 9 * * *",
      timeoutSeconds: 300,
      retryOnFailure: true,
      maxRetries: 3,
      retryDelaySeconds: 60,
      notificationEnabled: false,
      runOnStartup: false,
    });
    invoke<TaskTemplate[]>("get_scheduled_task_templates")
      .then((rows) => setTemplates(Array.isArray(rows) ? rows : []))
      .catch(logIpcError("Failed to load task templates"));
    // 仅在 open 跳变时初始化；stockCode/stockName 有意只取打开瞬间的值
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, editing]);

  const handleKindChange = (kind: InvestTaskKind) => {
    setInvestKind(kind);
    investForm.resetFields();
    const init = { ...getInvestKindDef(kind).initialValues };
    if (kind === "stock-analysis") {
      init.stockCode = stockCode;
      init.stockName = stockName;
    }
    if (kind === "stock-recommendation" && !init.name) {
      init.name = t("settings.scheduled-recommendation.default-name");
    }
    investForm.setFieldsValue(init);
  };

  const handleSelectTemplate = (template: TaskTemplate) => {
    setSelectedTemplate(template.template_type);
    customForm.setFieldsValue({
      name: template.name,
      description: template.description,
      workflow_id: template.workflow_id,
      cronExpression: template.default_schedule || "0 9 * * *",
    });
  };

  const submitInvest = async () => {
    const values = await investForm.validateFields();
    const def = getInvestKindDef(investKind);
    setSaving(true);
    try {
      await invoke(def.command, def.buildArgs(values));
      message.success(t("tasksCenter.create.createdOk"));
      onSaved();
      onClose();
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setSaving(false);
    }
  };

  const submitCustomCreate = async () => {
    const values = await customForm.validateFields();
    const config: TaskConfig = {
      timeoutSeconds: values.timeoutSeconds ?? 300,
      retryOnFailure: Boolean(values.retryOnFailure),
      maxRetries: values.maxRetries ?? 3,
      retryDelaySeconds: values.retryDelaySeconds ?? 60,
      notificationEnabled: Boolean(values.notificationEnabled),
      runOnStartup: Boolean(values.runOnStartup),
    };
    setSaving(true);
    try {
      await invoke("create_scheduled_task", {
        name: values.name,
        description: values.description,
        cronExpression: values.cronExpression?.trim(),
        taskType: values.workflow_id ? "workflow" : "custom",
        workflowId: values.workflow_id ?? null,
        config,
      });
      message.success(t("tasksCenter.create.createdOk"));
      onSaved();
      onClose();
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setSaving(false);
    }
  };

  const submitEdit = async () => {
    if (!editing) {
      return;
    }
    const values = await customForm.validateFields();
    // 后端按完整 ScheduledTaskDto 反序列化（config/status/createdAt/updatedAt 均必填），
    // 以原任务为底只覆盖名称/描述/cron，其余原样回传
    const updatedTask: ScheduledTask = {
      ...editing,
      name: values.name,
      description: values.description,
      cronExpression: values.cronExpression?.trim(),
    };
    setSaving(true);
    try {
      await invoke("update_scheduled_task", { taskId: editing.id, task: updatedTask });
      message.success(t("tasksCenter.create.updatedOk"));
      onSaved();
      onClose();
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setSaving(false);
    }
  };

  const handleSystemQuickCreate = async (def: SystemQuickDef) => {
    setSaving(true);
    try {
      // 后端三命令均要求必填 name；描述/时点走默认（默认值与后端内置值对齐）
      await invoke(def.command, {
        name: t(`settings.scheduler.${def.type}`),
        description: t(`settings.scheduler.${def.type}Desc`),
        cronExpression: def.defaultCron,
      });
      message.success(t("tasksCenter.create.createdOk"));
      onSaved();
      onClose();
    } catch (e) {
      showBackendError(message, e);
    } finally {
      setSaving(false);
    }
  };

  const handleOk = () => {
    if (isEdit) {
      void submitEdit();
    } else if (category === "invest") {
      void submitInvest();
    } else if (category === "custom") {
      void submitCustomCreate();
    }
    // system 类别无 footer，不会走到这里
  };

  const reportTemplates = useMemo(() => templates.filter((x) => !x.workflow_id), [templates]);
  const workflowTemplates = useMemo(() => templates.filter((x) => x.workflow_id), [templates]);

  const renderTemplateCards = (rows: TaskTemplate[], accent: string) => (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
      {rows.map((template) => {
        const active = selectedTemplate === template.template_type;
        return (
          <div
            key={template.template_type}
            role="button"
            tabIndex={0}
            onClick={() => handleSelectTemplate(template)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                handleSelectTemplate(template);
              }
            }}
            style={{
              padding: 10,
              border: active ? `2px solid ${accent}` : `1px solid ${token.colorBorder}`,
              borderRadius: token.borderRadius,
              cursor: "pointer",
              backgroundColor: active ? token.colorFillTertiary : token.colorFillQuaternary,
            }}
          >
            <div style={{ fontWeight: 500, marginBottom: 2, fontSize: 13, color: token.colorText }}>
              {t(`settings.scheduler.${template.template_type}`, { defaultValue: template.name })}
            </div>
            <div style={{ fontSize: 12, color: token.colorTextSecondary }}>
              {t(`settings.scheduler.${template.template_type}Desc`, {
                defaultValue: template.description,
              })}
            </div>
          </div>
        );
      })}
    </div>
  );

  return (
    <Modal
      title={isEdit ? t("tasksCenter.create.editTitle") : t("tasksCenter.create.title")}
      open={open}
      onOk={handleOk}
      onCancel={onClose}
      confirmLoading={saving}
      okText={isEdit ? t("settings.scheduler.updateTask") : t("settings.scheduler.createTask")}
      cancelText={t("common.cancel")}
      width={720}
      footer={!isEdit && category === "system" ? null : undefined}
      destroyOnHidden
    >
      {!isEdit && (
        <Radio.Group
          key="category-switch"
          value={category}
          onChange={(e) => setCategory(e.target.value as CreateCategory)}
          optionType="button"
          buttonStyle="solid"
          style={{ marginBottom: 16 }}
          options={[
            { value: "invest", label: t("tasksCenter.create.category.invest") },
            { value: "custom", label: t("tasksCenter.create.category.custom") },
            { value: "system", label: t("tasksCenter.create.category.system") },
          ]}
        />
      )}

      {/* ── 投资任务（容器常驻：两个 Form 首帧即挂载，消除 useForm 未连接警告） ── */}
      <div key="invest-form-wrap" style={{ display: !isEdit && category === "invest" ? undefined : "none" }}>
        <Form form={investForm} layout="vertical">
          <Form.Item label={t("tasksCenter.create.selectKind")}>
            <Select
              value={investKind}
              onChange={handleKindChange}
              options={INVEST_KIND_DEFS.map((def) => ({
                value: def.kind,
                label: t(def.nameKey),
              }))}
              optionRender={(option) => {
                const def = INVEST_KIND_DEFS.find((d) => d.kind === option.value);
                return (
                  <div>
                    <div>{t(def?.nameKey ?? "")}</div>
                    <div style={{ fontSize: 12, color: token.colorTextTertiary }}>
                      {t(def?.descKey ?? "")}
                    </div>
                  </div>
                );
              }}
            />
          </Form.Item>

          <InvestTaskFields kind={investKind} />

          <Form.Item
            name="cronExpression"
            label={t("settings.scheduler.scheduleType")}
            rules={[{ validator: cronValidator }]}
          >
            <CronScheduleBuilder />
          </Form.Item>
        </Form>
      </div>

      {/* ── 工作流与自定义（容器常驻，仅 display 切换；隐藏表单不会被 handleOk 提交） ── */}
      <div key="custom-form-wrap" style={{ display: isEdit || category === "custom" ? undefined : "none" }}>
        <Form form={customForm} layout="vertical">
          {!isEdit && (
            <>
              <div style={{ fontSize: 12, color: token.colorTextTertiary, marginBottom: 6 }}>
                {t("settings.scheduler.reportTemplates")}
              </div>
              {renderTemplateCards(reportTemplates, token.colorPrimary)}
              <Divider style={{ margin: "12px 0" }} />
              <div style={{ fontSize: 12, color: token.colorTextSecondary, marginBottom: 6 }}>
                {t("settings.scheduler.workflowTemplates")}
              </div>
              {renderTemplateCards(workflowTemplates, token.colorSuccess)}
              <Divider />
            </>
          )}

          {/* workflow_id 为隐藏字段：由模板卡片写入，决定 taskType=workflow */}
          <Form.Item name="workflow_id" hidden>
            <Input />
          </Form.Item>
          <Form.Item
            name="name"
            label={t("settings.scheduler.taskName")}
            rules={[{ required: true, message: t("common.required") }]}
          >
            <Input placeholder={t("settings.scheduler.taskName")} />
          </Form.Item>
          <Form.Item name="description" label={t("settings.scheduler.taskDescription")}>
            <Input.TextArea rows={2} placeholder={t("settings.scheduler.taskDescription")} />
          </Form.Item>
          <Form.Item
            name="cronExpression"
            label={t("settings.scheduler.scheduleType")}
            rules={[{ validator: cronValidator }]}
          >
            <CronScheduleBuilder />
          </Form.Item>

          {!isEdit && (
            <Collapse
              ghost
              // forceRender：折叠态也要让内部 Form.Item 注册，否则用户不展开面板时
              // validateFields 取不到这些字段，TaskConfig 默认值（尤其 retryOnFailure=true）会丢
              items={[
                {
                  key: "advanced",
                  forceRender: true,
                  label: t("tasksCenter.create.advanced"),
                  children: (
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-x-4">
                      <Form.Item name="timeoutSeconds" label={t("tasksCenter.config.timeoutSeconds")}>
                        <InputNumber min={10} max={86_400} className="!w-full" />
                      </Form.Item>
                      <Form.Item name="retryDelaySeconds" label={t("tasksCenter.config.retryDelaySeconds")}>
                        <InputNumber min={0} max={3600} className="!w-full" />
                      </Form.Item>
                      <Form.Item name="maxRetries" label={t("tasksCenter.config.maxRetries")}>
                        <InputNumber min={0} max={10} className="!w-full" />
                      </Form.Item>
                      <Form.Item name="retryOnFailure" label={t("tasksCenter.config.retryOnFailure")}>
                        <Switch />
                      </Form.Item>
                      <Form.Item name="notificationEnabled" label={t("tasksCenter.config.notificationEnabled")}>
                        <Switch />
                      </Form.Item>
                      <Form.Item name="runOnStartup" label={t("tasksCenter.config.runOnStartup")}>
                        <Switch />
                      </Form.Item>
                    </div>
                  ),
                },
              ]}
            />
          )}
        </Form>
      </div>

      {/* ── 系统任务（一键创建） ── */}
      {!isEdit && category === "system" && (
        <div key="system-quick">
          <Alert
            type="info"
            showIcon
            style={{ marginBottom: 12 }}
            title={t("tasksCenter.create.systemHint")}
          />
          <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
            {SYSTEM_QUICK_DEFS.map((def) => (
              <Button
                key={def.type}
                block
                loading={saving}
                onClick={() => void handleSystemQuickCreate(def)}
                style={{ height: "auto", padding: "12px 8px", whiteSpace: "normal" }}
              >
                <div style={{ fontWeight: 500 }}>{t(`settings.scheduler.${def.type}`)}</div>
                <div style={{ fontSize: 12, color: token.colorTextSecondary, marginTop: 4 }}>
                  {t(`settings.scheduler.${def.type}Desc`)}
                </div>
              </Button>
            ))}
          </div>
        </div>
      )}
    </Modal>
  );
}
