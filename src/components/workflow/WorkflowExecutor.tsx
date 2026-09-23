// SPDX-License-Identifier: AGPL-3.0-only
// Phase 4: WorkflowExecutor — 工作流执行面板（动态 UI 实时构建）

import type { JsonSchemaProperty, Variable, WorkflowTemplateResponse } from "@/components/workflow/types";
import { WorkflowLogPanel } from "@/components/workflow/WorkflowLogPanel";
import { showBackendError } from "@/lib/errorI18n";
import { invoke, logIpcError } from "@/lib/invoke";
import { isSecretOf } from "@/lib/workflowVariables";
import type { LlmDiagnoseV2 } from "@/stores/feature/workflowEditorStore";
import { useWorkflowStore, WORKFLOW_EXEC_CANCELLED } from "@/stores/feature/workflowStore";
import type { WorkflowDefinition, WorkflowExecution } from "@/types";
import {
  App,
  Button,
  Col,
  Descriptions,
  Divider,
  Empty,
  Form,
  Input,
  InputNumber,
  Modal,
  Row,
  Select,
  Space,
  Switch,
  Tag,
  Typography,
} from "antd";
import { Play, RotateCcw, Square, Wand2 } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DiagnosticFix } from "./types/workflow.types";

const { Text } = Typography;

interface WorkflowExecutorProps {
  workflow: WorkflowTemplateResponse;
  open: boolean;
  onClose: () => void;
}

const statusColor: Record<string, string> = {
  waiting: "default",
  pending: "default",
  ready: "default",
  running: "processing",
  in_progress: "processing",
  success: "success",
  completed: "success",
  failed: "error",
  error: "error",
  timeout: "error",
  skipped: "default",
  cancelled: "warning",
  partially_completed: "warning",
  paused: "warning",
};

/** AI 自愈修复应用到模板快照的结果 */
interface SelfHealApplyResult {
  template: WorkflowTemplateResponse;
  applied: string[];
  unsupported: string[];
}

/**
 * 把 V2 协议诊断修复应用到模板数据层（纯函数，返回新对象）。
 *
 * 支持可在数据层直接落地的 fix：
 * - set_node_field / set_timeout / enable_retry / delete_node / delete_edge / update_variable
 * 需编辑器或版本上下文的 fix（remove_debater_step / edit_asset_file / rollback_to_version）
 * 归入 `unsupported`，由 UI 提示用户在编辑器中处理，避免在数据层盲目改动结构。
 */
function applySelfHealFixesToTemplate(
  template: WorkflowTemplateResponse,
  fixes: DiagnosticFix[],
): SelfHealApplyResult {
  const nodes = template.nodes.map((n) => ({ ...n }));
  const edges = template.edges.map((e) => ({ ...e }));
  const variables = (template.variables ?? []).map((v) => ({ ...v }));
  const applied: string[] = [];
  const unsupported: string[] = [];

  for (const fix of fixes) {
    switch (fix.actionType) {
      case "set_node_field": {
        const node = nodes.find((n) => n.id === fix.nodeId);
        if (!node) {
          unsupported.push(fix.actionType);
          break;
        }
        // SAFE: 模板节点 config 统一为 Record<string, unknown>
        const cfg = (node as unknown as { config?: Record<string, unknown> }).config ?? {};
        (node as unknown as { config: Record<string, unknown> }).config = { ...cfg, [fix.field]: fix.value };
        applied.push(fix.actionType);
        break;
      }
      case "set_timeout": {
        const node = nodes.find((n) => n.id === fix.nodeId);
        if (!node) {
          unsupported.push(fix.actionType);
          break;
        }
        (node as unknown as { timeout?: number }).timeout = fix.timeoutMs;
        applied.push(fix.actionType);
        break;
      }
      case "enable_retry": {
        const node = nodes.find((n) => n.id === fix.nodeId);
        if (!node) {
          unsupported.push(fix.actionType);
          break;
        }
        (node as unknown as { retry?: unknown }).retry = {
          enabled: true,
          maxRetries: fix.maxRetries,
          backoffType: "Exponential",
          baseDelayMs: 1000,
          maxDelayMs: 60000,
        };
        applied.push(fix.actionType);
        break;
      }
      case "delete_node": {
        const idx = nodes.findIndex((n) => n.id === fix.nodeId);
        if (idx === -1) {
          unsupported.push(fix.actionType);
          break;
        }
        nodes.splice(idx, 1);
        // 级联删除关联边，保持图结构一致
        for (let i = edges.length - 1; i >= 0; i--) {
          if (edges[i].source === fix.nodeId || edges[i].target === fix.nodeId) {
            edges.splice(i, 1);
          }
        }
        applied.push(fix.actionType);
        break;
      }
      case "delete_edge": {
        const idx = edges.findIndex((e) => e.id === fix.edgeId);
        if (idx === -1) {
          unsupported.push(fix.actionType);
          break;
        }
        edges.splice(idx, 1);
        applied.push(fix.actionType);
        break;
      }
      case "update_variable": {
        const v = variables.find((x) => x.name === fix.name);
        if (v) {
          v.value = fix.value;
        } else {
          variables.push({ name: fix.name, varType: "string", value: fix.value, isSecret: false });
        }
        applied.push(fix.actionType);
        break;
      }
      default:
        unsupported.push(fix.actionType);
        break;
    }
  }
  return { template: { ...template, nodes, edges, variables }, applied, unsupported };
}

/** 从 JsonSchemaProperty + Variable 推导动态表单字段 */
interface DynamicField {
  name: string;
  label: string;
  description?: string;
  type: "string" | "number" | "integer" | "boolean" | "enum" | "object" | "array";
  format?: string;
  required: boolean;
  default?: unknown;
  enumValues: unknown[];
  isSecret: boolean;
}

/** 解析 variables 的原始值类型（后端可能用字符串表达类型） */
function inferVarType(value: unknown): DynamicField["type"] {
  if (typeof value === "number") {
    return Number.isInteger(value) ? "integer" : "number";
  }
  if (typeof value === "boolean") { return "boolean"; }
  if (Array.isArray(value)) { return "array"; }
  if (value !== null && typeof value === "object") { return "object"; }
  return "string";
}

/**
 * 构建动态表单字段：
 * 优先用 inputSchema.properties（带完整类型/枚举/必填信息），
 * 再叠加 variables（提供默认值与秘密标记），schema 缺失时直接用 variables 推导。
 */
function buildDynamicFields(workflow: WorkflowTemplateResponse): DynamicField[] {
  const schemaProps = workflow.inputSchema?.properties ?? {};
  const required = new Set(workflow.inputSchema?.required ?? []);
  const varMap = new Map<string, Variable>();
  for (const v of workflow.variables ?? []) {
    varMap.set(v.name, v);
  }

  const fields: DynamicField[] = [];

  // 1) schema 定义的字段
  for (const [name, prop] of Object.entries(schemaProps as Record<string, JsonSchemaProperty>)) {
    const v = varMap.get(name);
    let type = prop.type as DynamicField["type"];
    if (prop.enumValues && prop.enumValues.length > 0) {
      type = "enum";
    }
    fields.push({
      name,
      label: name,
      description: prop.description,
      type,
      format: prop.format,
      required: required.has(name),
      default: v?.value ?? prop.default,
      enumValues: prop.enumValues ?? [],
      // 必须走 isSecretOf：工作流变量来自 DB 模板，键是 snake_case 的 is_secret，
      // 直接读 v.isSecret 恒 false → 密钥类变量不再显示「密钥」标记、输入框不退化为密码框。
      isSecret: v ? isSecretOf(v) : false,
    });
  }

  // 2) 未被 schema 覆盖的 variables
  for (const v of workflow.variables ?? []) {
    if (schemaProps[v.name]) { continue; }
    fields.push({
      name: v.name,
      label: v.name,
      description: v.description,
      type: inferVarType(v.value),
      required: false,
      default: v.value,
      enumValues: [],
      isSecret: isSecretOf(v),
    });
  }

  return fields;
}

/** 按字段类型渲染动态控件 */
function renderFieldControl(field: DynamicField) {
  const { type, enumValues, isSecret, description } = field;

  if (type === "boolean") {
    return <Switch aria-label={field.name} />;
  }

  if (type === "number" || type === "integer") {
    return (
      <InputNumber
        style={{ width: "100%" }}
        precision={type === "integer" ? 0 : undefined}
        aria-label={field.name}
      />
    );
  }

  if (type === "enum" && enumValues.length > 0) {
    return (
      <Select
        aria-label={field.name}
        options={enumValues.map((v) => ({ value: v, label: String(v) }))}
      />
    );
  }

  if (type === "array") {
    return (
      <Select
        mode="tags"
        aria-label={field.name}
        placeholder={description}
        open={false}
        suffixIcon={null}
      />
    );
  }

  if (type === "object") {
    return (
      <Input.TextArea
        rows={4}
        aria-label={field.name}
        placeholder={description}
      />
    );
  }

  // string
  if (isSecret) {
    return <Input.Password aria-label={field.name} />;
  }
  if (field.format === "textarea" || (description?.length ?? 0) > 60) {
    return <Input.TextArea rows={3} aria-label={field.name} />;
  }
  return <Input aria-label={field.name} />;
}

export function WorkflowExecutor({ workflow, open, onClose }: WorkflowExecutorProps) {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [form] = Form.useForm();
  const [execution, setExecution] = useState<WorkflowExecution | null>(null);
  // AI 自愈：执行失败后回喂 LLM 诊断，产出修复动作应用到模板
  const [selfHealLoading, setSelfHealLoading] = useState(false);
  const [selfHealReport, setSelfHealReport] = useState<LlmDiagnoseV2 | null>(null);
  const [selfHealApplying, setSelfHealApplying] = useState(false);
  const isExecuting = useWorkflowStore((s) => s.isExecuting);
  const executeWorkflow = useWorkflowStore((s) => s.executeWorkflow);
  const cancelExecution = useWorkflowStore((s) => s.cancelExecution);

  const statusLabel: Record<string, string> = useMemo(() => ({
    waiting: t("rl.status.idle"),
    pending: t("rl.status.idle"),
    ready: t("rl.status.idle"),
    running: t("rl.status.running"),
    in_progress: t("rl.status.running"),
    success: t("rl.status.completed"),
    completed: t("rl.status.completed"),
    failed: t("rl.status.failed"),
    error: t("rl.status.failed"),
    timeout: t("rl.status.failed"),
    skipped: t("workflow.executor.skipped"),
    cancelled: t("workflow.executor.cancelled"),
    partially_completed: t("workflow.executor.partiallyCompleted"),
    paused: t("workflow.executor.paused"),
  }), [t]);

  /** 工作流简化定义（用于节点展示） */
  const workflowDefinition: WorkflowDefinition = useMemo(() => {
    const nodes = (workflow.nodes ?? []).map((n) => ({
      id: n.id,
      type:
        ("type" in n && typeof n.type === "string" ? n.type : "action") as WorkflowDefinition["nodes"][number]["type"],
      label: n.title ?? n.id,
      config: ("config" in n ? n.config : {}) as Record<string, unknown>,
      position: n.position ?? { x: 0, y: 0 },
    }));
    const edges = (workflow.edges ?? []).map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      label: e.label,
    }));
    const variables: Record<string, unknown> = {};
    for (const v of workflow.variables ?? []) {
      variables[v.name] = v.value;
    }
    return {
      id: workflow.id,
      name: workflow.name,
      description: workflow.description ?? "",
      version: workflow.version,
      nodes,
      edges,
      variables,
      createdAt: workflow.createdAt,
      updatedAt: workflow.updatedAt,
      status: workflow.isPreset ? ("active" as const) : ("draft" as const),
    };
  }, [workflow]);

  const fields = useMemo(() => buildDynamicFields(workflow), [workflow]);

  const handleExecute = useCallback(async () => {
    try {
      const raw = form.getFieldsValue();
      // 类型归一化：number/integer → number；boolean → boolean；object/array 尝试 JSON.parse
      const inputs: Record<string, unknown> = {};
      for (const f of fields) {
        const val = raw[f.name];
        if (val === undefined || val === null || val === "") {
          continue;
        }
        // 用户配置类型错误时显式报错并中止执行，而非静默传脏数据到下游
        if ((f.type === "number" || f.type === "integer") && typeof val === "string") {
          const num = Number(val);
          if (Number.isNaN(num)) {
            message.error(t("workflow.executor.invalidField", { name: f.label }));
            return;
          }
          inputs[f.name] = num;
          continue;
        }
        if (f.type === "number" || f.type === "integer") {
          inputs[f.name] = typeof val === "number" ? val : Number(val);
        } else if (f.type === "boolean") {
          inputs[f.name] = Boolean(val);
        } else if (f.type === "object" && typeof val === "string") {
          try {
            inputs[f.name] = JSON.parse(val);
          } catch {
            message.error(t("workflow.executor.invalidField", { name: f.label }));
            return;
          }
        } else if (f.type === "array" && Array.isArray(val)) {
          inputs[f.name] = val.map((x) => {
            if (typeof x !== "string") {
              return x;
            }
            try {
              return JSON.parse(x);
            } catch {
              return x;
            }
          });
        } else {
          inputs[f.name] = val;
        }
      }
      const exec = await executeWorkflow(workflow.id, inputs);
      setExecution(exec);
    } catch (e) {
      // 用户主动取消：走提示而非红字错误
      if (e instanceof Error && e.message === WORKFLOW_EXEC_CANCELLED) {
        message.info(t("workflow.executor.cancelled"));
        return;
      }
      showBackendError(message, e);
    }
  }, [form, workflow.id, executeWorkflow, fields, message, t]);

  const handleCancel = useCallback(() => {
    void cancelExecution();
  }, [cancelExecution]);

  const handleClose = useCallback(() => {
    setExecution(null);
    form.resetFields();
    onClose();
  }, [form, onClose]);

  const handleReExecute = useCallback(() => {
    setExecution(null);
    setSelfHealReport(null);
  }, []);

  /** 构造执行失败上下文（失败节点错误 + error 级日志），注入诊断请求供 LLM 感知 */
  const buildFailureContext = useCallback((exec: WorkflowExecution): string => {
    const parts: string[] = [];
    for (const ns of exec.nodeStates) {
      if (ns.status === "failed" && ns.error) {
        parts.push(`- 节点 ${ns.nodeId} 执行失败: ${ns.error}`);
      }
    }
    for (const log of exec.logs) {
      if (log.level === "error") {
        parts.push(`- [${log.nodeName || log.nodeId}] ${log.message}`);
      }
    }
    return parts.length > 0 ? `\n\n【执行失败上下文】\n${parts.join("\n")}` : "";
  }, []);

  /** AI 自愈：执行失败后调 LLM 诊断（带失败上下文），返回诊断报告 */
  const handleSelfHeal = useCallback(async () => {
    if (!execution) { return; }
    setSelfHealLoading(true);
    setSelfHealReport(null);
    try {
      const failureCtx = buildFailureContext(execution);
      const report = await invoke<LlmDiagnoseV2>("llm_diagnose_workflow", {
        request: {
          nodes: workflow.nodes,
          workflow_name: workflow.name,
          workflow_description: `${workflow.description ?? ""}${failureCtx}`,
        },
      });
      setSelfHealReport(report);
      if (!report.issues || report.issues.length === 0) {
        message.info(t("workflow.executor.selfHealNoIssues"));
      }
    } catch (error) {
      logIpcError("AI 自愈诊断")(error);
      showBackendError(message, error);
    } finally {
      setSelfHealLoading(false);
    }
  }, [execution, workflow, buildFailureContext, message, t]);

  /** 应用自愈修复：节点级 fix 落到模板数据层并保存（复用 update_workflow_template） */
  const handleApplySelfHealFixes = useCallback(async () => {
    const fixes = selfHealReport?.fixes;
    if (!fixes || fixes.length === 0) { return; }
    setSelfHealApplying(true);
    try {
      const { template, applied, unsupported } = applySelfHealFixesToTemplate(workflow, fixes);
      await invoke<boolean>("update_workflow_template", {
        id: template.id,
        input: {
          name: template.name,
          description: template.description ?? "",
          icon: template.icon,
          tags: template.tags,
          nodes: template.nodes,
          edges: template.edges,
          variables: (template.variables ?? []).map((v) => ({
            name: v.name,
            var_type: v.varType,
            value: v.value,
            description: v.description,
            is_secret: v.isSecret,
          })),
          trigger_config: template.triggerConfig,
        },
      });
      setSelfHealReport(null);
      setExecution(null);
      message.success(t("workflow.executor.selfHealApplied", { count: applied.length }));
      if (unsupported.length > 0) {
        message.warning(t("workflow.executor.selfHealUnsupported", { count: unsupported.length }));
      }
    } catch (error) {
      logIpcError("AI 自愈应用修复")(error);
      showBackendError(message, error);
    } finally {
      setSelfHealApplying(false);
    }
  }, [selfHealReport, workflow, message, t]);

  return (
    <Modal
      title={`${t("workflow.executor.execute")}: ${workflow.name}`}
      open={open}
      onCancel={handleClose}
      width={720}
      footer={null}
      destroyOnHidden
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {/* 动态输入表单 */}
        {!execution && !isExecuting && (
          <>
            {fields.length > 0
              ? (
                <>
                  <Text strong style={{ display: "block" }}>
                    {t("workflow.executor.inputVariables")}
                  </Text>
                  <Form form={form} layout="vertical" size="small">
                    <Row gutter={16}>
                      {fields.map((f) => (
                        <Col span={f.type === "object" || f.type === "array" ? 24 : 12} key={f.name}>
                          <Form.Item
                            name={f.name}
                            label={
                              <Space size={4}>
                                <span>{f.label}</span>
                                {f.isSecret && (
                                  <Tag color="red" style={{ fontSize: 11, marginInlineEnd: 0 }}>
                                    {t("workflow.executor.secret")}
                                  </Tag>
                                )}
                                {f.required && (
                                  <Tag color="blue" style={{ fontSize: 11, marginInlineEnd: 0 }}>
                                    {t("workflow.executor.required")}
                                  </Tag>
                                )}
                              </Space>
                            }
                            tooltip={f.description}
                            initialValue={f.default}
                            valuePropName={f.type === "boolean" ? "checked" : "value"}
                            rules={f.required
                              ? [{ required: true, message: t("workflow.executor.requiredField", { name: f.label }) }]
                              : []}
                          >
                            {renderFieldControl(f)}
                          </Form.Item>
                        </Col>
                      ))}
                    </Row>
                  </Form>
                  <Button
                    type="primary"
                    icon={<Play size={14} />}
                    onClick={handleExecute}
                    loading={isExecuting}
                    block
                  >
                    {isExecuting ? t("workflow.executor.executing") : t("workflow.executor.execute")}
                  </Button>
                </>
              )
              : (
                <>
                  <Empty description={t("workflow.executor.noInputVariables")} />
                  <Button
                    type="primary"
                    icon={<Play size={14} />}
                    onClick={handleExecute}
                    loading={isExecuting}
                    block
                  >
                    {isExecuting ? t("workflow.executor.executing") : t("workflow.executor.execute")}
                  </Button>
                </>
              )}
          </>
        )}

        {/* 执行中状态 */}
        {isExecuting && (
          <div style={{ textAlign: "center", padding: 16 }}>
            <Text type="secondary">{t("workflow.executor.executing")}</Text>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8, marginTop: 12, justifyContent: "center" }}>
              {workflowDefinition.nodes.map((node) => (
                <Tag key={node.id} color="processing">
                  {node.label}
                </Tag>
              ))}
            </div>
            <Button
              danger
              icon={<Square size={14} />}
              onClick={handleCancel}
              style={{ marginTop: 16 }}
            >
              {t("workflow.executor.cancelExecution")}
            </Button>
          </div>
        )}

        {/* 执行结果 */}
        {execution && !isExecuting && (
          <>
            <Descriptions size="small" column={2} bordered>
              <Descriptions.Item label={t("workflow.executor.status")}>
                <Tag color={execution.status === "completed" ? "success" : "error"}>
                  {execution.status === "completed"
                    ? t("workflow.executor.executionSuccess")
                    : t("workflow.executor.executionFailed")}
                </Tag>
              </Descriptions.Item>
              <Descriptions.Item label={t("workflow.executor.duration")}>
                {execution.finishedAt && execution.startedAt
                  ? `${((execution.finishedAt - execution.startedAt) / 1000).toFixed(1)}s`
                  : "-"}
              </Descriptions.Item>
            </Descriptions>

            {/* 节点状态 */}
            <div>
              <Text strong style={{ display: "block", marginBottom: 8 }}>
                {t("workflow.executor.nodeExecutionStatus")}
              </Text>
              <Space wrap>
                {execution.nodeStates.map((ns) => (
                  <Tag key={ns.nodeId} color={statusColor[ns.status]}>
                    {workflowDefinition.nodes.find((n) => n.id === ns.nodeId)?.label ?? ns.nodeId}:{" "}
                    {statusLabel[ns.status]}
                  </Tag>
                ))}
              </Space>
            </div>

            {/* 输出变量 */}
            {execution.outputs && Object.keys(execution.outputs).length > 0 && (
              <div>
                <Text strong style={{ display: "block", marginBottom: 8 }}>{t("workflow.executor.outputResult")}</Text>
                <pre
                  style={{
                    backgroundColor: "var(--color-fill-tertiary)",
                    padding: 8,
                    borderRadius: 4,
                    fontSize: 12,
                    maxHeight: 120,
                    overflow: "auto",
                  }}
                >
                  {JSON.stringify(execution.outputs, null, 2)}
                </pre>
              </div>
            )}

            {/* 日志 */}
            {execution.logs.length > 0 && (
              <div>
                <Text strong style={{ display: "block", marginBottom: 8 }}>{t("workflow.executor.executionLog")}</Text>
                <WorkflowLogPanel
                  logs={execution.logs}
                  maxHeight={200}
                />
              </div>
            )}

            {/* AI 自愈：执行失败后回喂 LLM 诊断并支持一键应用修复到模板 */}
            {execution.status !== "completed" && (
              <div>
                <Divider style={{ margin: "4px 0" }} />
                <Space wrap>
                  <Button
                    icon={<Wand2 size={14} />}
                    onClick={handleSelfHeal}
                    loading={selfHealLoading}
                    disabled={!!selfHealReport}
                  >
                    {t("workflow.executor.selfHeal")}
                  </Button>
                </Space>
                {selfHealReport && (
                  <div
                    style={{
                      marginTop: 12,
                      padding: 12,
                      background: "var(--color-fill-tertiary)",
                      borderRadius: 6,
                      display: "flex",
                      flexDirection: "column",
                      gap: 8,
                    }}
                  >
                    <Text strong>{t("workflow.executor.selfHealTitle")}</Text>
                    {selfHealReport.summary && <Text style={{ fontSize: 12 }}>{selfHealReport.summary}</Text>}
                    {(selfHealReport.issues ?? []).length > 0 && (
                      <ul style={{ margin: 0, paddingLeft: 18, fontSize: 12 }}>
                        {(selfHealReport.issues ?? []).slice(0, 8).map((iss, idx) => (
                          <li key={idx} style={{ marginBottom: 4 }}>
                            <Space size={6} align="start">
                              <Tag
                                color={iss.severity === "critical" || iss.severity === "high"
                                    || iss.severity === "error"
                                  ? "red"
                                  : iss.severity === "medium" || iss.severity === "warning"
                                  ? "orange"
                                  : "default"}
                                style={{ fontSize: 10, marginInlineEnd: 0 }}
                              >
                                {iss.severity}
                              </Tag>
                              <span>
                                {iss.title}
                                {iss.suggestion && (
                                  <Text type="secondary" style={{ display: "block", fontSize: 11 }}>
                                    {iss.suggestion}
                                  </Text>
                                )}
                              </span>
                            </Space>
                          </li>
                        ))}
                      </ul>
                    )}
                    {selfHealReport.fixes && selfHealReport.fixes.length > 0 && (
                      <Space wrap>
                        <Button
                          type="primary"
                          size="small"
                          onClick={handleApplySelfHealFixes}
                          loading={selfHealApplying}
                        >
                          {t("workflow.executor.selfHealApply", { count: selfHealReport.fixes.length })}
                        </Button>
                        <Text type="secondary" style={{ fontSize: 12 }}>
                          {t("workflow.executor.selfHealApplyHint")}
                        </Text>
                      </Space>
                    )}
                  </div>
                )}
              </div>
            )}

            <Divider style={{ margin: "4px 0" }} />

            <Space>
              <Button icon={<RotateCcw size={14} />} onClick={handleReExecute}>
                {t("workflow.executor.reExecute")}
              </Button>
              <Button type="primary" onClick={handleClose}>
                {t("workflow.executor.close")}
              </Button>
            </Space>
          </>
        )}
      </div>
    </Modal>
  );
}
