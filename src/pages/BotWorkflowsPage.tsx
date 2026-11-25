// SPDX-License-Identifier: AGPL-3.0-only

/**
 * `/bots/workflows` —— P3 编排页（T6'''）。
 *
 * 组合层职责：从 botStore 读 workflow slice 与 P2-b 部门/委派数据，注入三个中立组件；
 * 监听后端 `bot-workflow-run-updated` / `bot-assignment-updated` 事件刷新运行与链视图。
 * 不新增通知组件，也不改动 locale（i18n 键由门禁线统一落）。
 */

import { BotDelegationChainPanel } from "@/components/bot/BotDelegationChainPanel";
import { BotWorkflowEditor } from "@/components/bot/BotWorkflowEditor";
import { BotWorkflowRunsPanel } from "@/components/bot/BotWorkflowRunsPanel";
import { filterWorkflowsByDepartment } from "@/lib/botWorkflowValidation";
import { backendErrorText } from "@/lib/errorI18n";
import { listen } from "@/lib/invoke";
import { useBotStore, useConversationStore } from "@/stores";
import type {
  BotAssignment,
  BotDepartment,
  BotWorkflow,
  CreateBotWorkflowInput,
  IpcEventName,
  ScheduledTask,
} from "@/types";
import { App, Button, Card, Empty, Form, Input, Select, Space, Spin, Tabs, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text, Title } = Typography;

export function BotWorkflowsPage() {
  const { t } = useTranslation();
  const { message: antdMessage } = App.useApp();
  const [scheduleForm] = Form.useForm<{ workflowId: string; cronExpression: string; timezone?: string }>();

  const bots = useBotStore((s) => s.bots);
  const loading = useBotStore((s) => s.loading);
  const workflowLoading = useBotStore((s) => s.workflowLoading);
  const workflowSaving = useBotStore((s) => s.workflowSaving);
  const workflows = useBotStore((s) => s.workflows);
  const workflowRuns = useBotStore((s) => s.workflowRuns);
  const loadBots = useBotStore((s) => s.loadBots);
  const loadWorkflows = useBotStore((s) => s.loadWorkflows);
  const createWorkflow = useBotStore((s) => s.createWorkflow);
  const updateWorkflow = useBotStore((s) => s.updateWorkflow);
  const runWorkflow = useBotStore((s) => s.runWorkflow);
  const loadWorkflowRuns = useBotStore((s) => s.loadWorkflowRuns);
  const cancelWorkflowRun = useBotStore((s) => s.cancelWorkflowRun);
  const createWorkflowSchedule = useBotStore((s) => s.createWorkflowSchedule);
  const listBotDepartments = useBotStore((s) => s.listBotDepartments);
  const listBotAssignments = useBotStore((s) => s.listBotAssignments);
  const cancelBotAssignment = useBotStore((s) => s.cancelBotAssignment);
  const activeConversationId = useConversationStore((s) => s.activeConversationId);

  const [departments, setDepartments] = useState<BotDepartment[]>([]);
  const [assignments, setAssignments] = useState<BotAssignment[]>([]);
  const [departmentFilter, setDepartmentFilter] = useState<string>("");
  const [editing, setEditing] = useState<BotWorkflow | null>(null);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string | null>(null);
  const [busyRuns, setBusyRuns] = useState<string[]>([]);
  const [busyAssignments, setBusyAssignments] = useState<Record<string, boolean>>({});
  const [chainErrorCode, setChainErrorCode] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState("definitions");

  const reloadDepartments = useCallback(() => {
    void listBotDepartments()
      .then((rows) => setDepartments(Array.isArray(rows) ? rows : []))
      .catch(() => setDepartments([]));
  }, [listBotDepartments]);

  const reloadRuns = useCallback(() => {
    void loadWorkflowRuns({})
      .catch((e) => antdMessage.error(backendErrorText(e)));
  }, [loadWorkflowRuns, antdMessage]);

  const reloadAssignments = useCallback(() => {
    void listBotAssignments({})
      .then((rows) => setAssignments(Array.isArray(rows) ? rows : []))
      .catch(() => setAssignments([]));
  }, [listBotAssignments]);

  useEffect(() => {
    void loadBots(true).catch(() => {});
    reloadDepartments();
    reloadRuns();
    reloadAssignments();
  }, [loadBots, reloadDepartments, reloadRuns, reloadAssignments]);

  useEffect(() => {
    void loadWorkflows(departmentFilter || undefined)
      .catch((e) => antdMessage.error(backendErrorText(e)));
  }, [loadWorkflows, departmentFilter, antdMessage]);

  useEffect(() => {
    let alive = true;
    const unsubscribes: Array<() => void> = [];
    // P3 事件名已在 IPC-P3 §3 冻结，后端 schema-gen 尚未重生成；此处按冻结契约收窄类型。
    const subscribe = (event: string, reload: () => void) => {
      void listen(event as IpcEventName, () => {
        if (alive) {
          reload();
        }
      })
        .then((unlisten) => {
          if (alive) {
            unsubscribes.push(unlisten);
          } else {
            unlisten();
          }
        })
        .catch(() => {});
    };
    subscribe("bot-workflow-run-updated", reloadRuns);
    subscribe("bot-assignment-updated", reloadAssignments);
    return () => {
      alive = false;
      for (const unlisten of unsubscribes) {
        unlisten();
      }
    };
  }, [reloadRuns, reloadAssignments]);

  const visibleWorkflows = useMemo(
    () => filterWorkflowsByDepartment(workflows, departmentFilter || null),
    [workflows, departmentFilter],
  );
  const selectedWorkflow = useMemo(
    () => visibleWorkflows.find((workflow) => workflow.id === selectedWorkflowId) ?? visibleWorkflows[0] ?? null,
    [visibleWorkflows, selectedWorkflowId],
  );
  const selectedRuns = useMemo(
    () => (selectedWorkflow ? workflowRuns.filter((run) => run.workflowId === selectedWorkflow.id) : workflowRuns),
    [workflowRuns, selectedWorkflow],
  );

  const handleSubmitWorkflow = async (input: CreateBotWorkflowInput, id: string | null) => {
    const saved = id ? await updateWorkflow(id, input) : await createWorkflow(input);
    setEditing(null);
    setSelectedWorkflowId(saved.id);
    setActiveTab("definitions");
    void loadWorkflows(departmentFilter || undefined);
  };

  const handleRun = async () => {
    if (!selectedWorkflow) {
      return;
    }
    try {
      const run = await runWorkflow({
        workflowId: selectedWorkflow.id,
        requesterConversationId: activeConversationId,
      });
      setActiveTab("runs");
      setChainErrorCode(run.errorCode);
      reloadRuns();
      reloadAssignments();
    } catch (e) {
      antdMessage.error(backendErrorText(e));
    }
  };

  const handleCancelRun = async (runId: string) => {
    setBusyRuns((prev) => [...prev, runId]);
    try {
      await cancelWorkflowRun(runId);
      reloadRuns();
      reloadAssignments();
    } finally {
      setBusyRuns((prev) => prev.filter((id) => id !== runId));
    }
  };

  const handleCancelAssignment = (assignment: BotAssignment) => {
    setBusyAssignments((prev) => ({ ...prev, [assignment.id]: true }));
    void cancelBotAssignment(assignment.id)
      .then(() => {
        reloadAssignments();
        reloadRuns();
      })
      .catch((e) => antdMessage.error(backendErrorText(e)))
      .finally(() => {
        setBusyAssignments((prev) => {
          const next = { ...prev };
          delete next[assignment.id];
          return next;
        });
      });
  };

  const handleCreateSchedule = async (values: { workflowId: string; cronExpression: string; timezone?: string }) => {
    try {
      const task: ScheduledTask = await createWorkflowSchedule({
        workflowId: values.workflowId,
        cronExpression: values.cronExpression,
        timezone: values.timezone ?? null,
      });
      if (task) {
        scheduleForm.resetFields();
        antdMessage.success(t("bot.workflowSchedule.title"));
      }
    } catch (e) {
      antdMessage.error(backendErrorText(e));
    }
  };

  const workflowOptions = visibleWorkflows.map((workflow) => ({ value: workflow.id, label: workflow.name }));
  const filterOptions = [
    { value: "", label: t("bot.workflow.department") },
    { value: "instance", label: t("bot.workflow.ownerInstance") },
    ...departments.map((department) => ({ value: department.id, label: department.name })),
  ];

  const definitionsTab = (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <Card size="small" data-testid="bot-workflow-list">
        <Space wrap style={{ marginBottom: 8 }}>
          <Select
            value={departmentFilter}
            options={filterOptions}
            data-testid="bot-workflow-department-filter"
            onChange={(value) => setDepartmentFilter(value ?? "")}
          />
          <Button
            type="primary"
            data-testid="bot-workflow-new"
            onClick={() => {
              setEditing(null);
              setActiveTab("editor");
            }}
          >
            {t("bot.workflow.new")}
          </Button>
        </Space>
        {visibleWorkflows.length === 0
          ? (
            /* 空态不许丢（PLAN-P3 §6 DoD）：无编排 / 无 Bot 都停在这里，而不是空白页。 */
            <Empty
              data-testid="bot-workflow-empty"
              description={
                <Space direction="vertical" size={2}>
                  <Text>{t("bot.workflow.empty")}</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {t("bot.workflow.emptyDescription")}
                  </Text>
                </Space>
              }
            >
              <Button
                type="primary"
                data-testid="bot-workflow-create-first"
                disabled={bots.length === 0}
                onClick={() => {
                  setEditing(null);
                  setActiveTab("editor");
                }}
              >
                {t("bot.workflow.createFirst")}
              </Button>
            </Empty>
          )
          : (
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {visibleWorkflows.map((workflow) => (
                <Card
                  size="small"
                  key={workflow.id}
                  data-testid="bot-workflow-item"
                  data-workflow-id={workflow.id}
                  title={workflow.name}
                  extra={
                    <Space size={4}>
                      <Button
                        size="small"
                        type="link"
                        data-testid="bot-workflow-edit"
                        onClick={() => {
                          setEditing(workflow);
                          setActiveTab("editor");
                        }}
                      >
                        {t("bot.workflow.edit")}
                      </Button>
                    </Space>
                  }
                >
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {workflow.description}
                  </Text>
                </Card>
              ))}
            </div>
          )}
      </Card>
    </div>
  );

  const editorTab = (
    <BotWorkflowEditor
      bots={bots}
      departments={departments}
      workflows={visibleWorkflows}
      editing={editing}
      saving={workflowSaving}
      onSubmit={(input, id) => {
        return handleSubmitWorkflow(input, id);
      }}
      onCancelEdit={() => {
        setEditing(null);
        setActiveTab("definitions");
      }}
    />
  );

  const runsTab = (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <Card size="small">
        <Space wrap>
          <Select
            value={selectedWorkflow?.id}
            options={workflowOptions}
            style={{ minWidth: 220 }}
            data-testid="bot-workflow-run-select"
            onChange={(value) => setSelectedWorkflowId(value)}
          />
          <Button type="primary" data-testid="bot-workflow-run" onClick={() => void handleRun()}>
            {t("bot.workflowRun.run")}
          </Button>
        </Space>
      </Card>
      <BotWorkflowRunsPanel
        runs={selectedRuns}
        assignments={assignments}
        busyRunIds={busyRuns}
        onCancel={handleCancelRun}
      />
    </div>
  );

  const chainTab = (
    <BotDelegationChainPanel
      assignments={assignments}
      errorCode={chainErrorCode}
      busy={busyAssignments}
      onCancel={handleCancelAssignment}
    />
  );

  const scheduleCard = (
    <Card size="small" title={t("bot.workflowSchedule.title")} data-testid="bot-workflow-schedule">
      <Form<{ workflowId: string; cronExpression: string; timezone?: string }>
        form={scheduleForm}
        layout="vertical"
        onFinish={(values) => {
          void handleCreateSchedule(values);
        }}
      >
        <Form.Item name="workflowId" label={t("bot.workflow.title")} rules={[{ required: true }]}>
          <Select options={workflowOptions} data-testid="bot-workflow-schedule-workflow" />
        </Form.Item>
        <Form.Item name="cronExpression" label={t("bot.workflowSchedule.cron")} rules={[{ required: true }]}>
          <Input data-testid="bot-workflow-schedule-cron" placeholder="0 9 * * *" />
        </Form.Item>
        <Form.Item name="timezone" label={t("bot.workflowSchedule.timezone")}>
          <Input data-testid="bot-workflow-schedule-timezone" />
        </Form.Item>
        <Text type="secondary" style={{ fontSize: 12 }}>
          {t("bot.workflowSchedule.hint")}
        </Text>
        <div style={{ marginTop: 8 }}>
          <Button type="primary" htmlType="submit" data-testid="bot-workflow-schedule-create">
            {t("bot.workflowSchedule.create")}
          </Button>
        </div>
      </Form>
    </Card>
  );

  return (
    <div style={{ padding: 16 }} data-testid="bot-workflows-page">
      <Title level={4}>{t("bot.workflow.title")}</Title>
      {(loading || workflowLoading) && workflows.length === 0
        ? <Spin data-testid="bot-workflow-loading" />
        : (
          <Tabs
            activeKey={activeTab}
            onChange={setActiveTab}
            data-testid="bot-workflow-tabs"
            items={[
              { key: "definitions", label: t("bot.workflow.title"), children: definitionsTab },
              { key: "editor", label: t("bot.workflow.new"), children: editorTab },
              { key: "runs", label: t("bot.workflowRun.title"), children: runsTab },
              { key: "chain", label: t("bot.assignment.chain"), children: chainTab },
            ]}
          />
        )}
      {scheduleCard}
    </div>
  );
}
