// SPDX-License-Identifier: AGPL-3.0-only
/**
 * P3 e2e 的 IPC 替身：只在测试页注入，不改 src/ 业务实现。
 * browserMockBots.ts 只实现 P1/P2 命令，P3 的 8 条编排命令在浏览器模式没有数据面。
 * 这里按 IPC-P3.md §2/§3 的冻结 DTO 与错误码做最小状态机，经 Tauri IPC 入口驱动真实页面。
 * 边界：这是 UI ↔ IPC 契约层 e2e，不替代 Rust 运行时状态机的证据。
 */
export interface P3ShimBot { id: string; name: string; soul?: string; approvalMode?: string; }
export interface P3ShimSeed { bots: P3ShimBot[]; }

export function installP3Shim(seed: P3ShimSeed): void {
  const w = window as any;
  const now = () => Date.now();
  const uid = (prefix: string) => prefix + "-" + Math.random().toString(36).slice(2, 10);
  const state = {
    workflows: [] as any[], runs: [] as any[], assignments: [] as any[],
    schedules: [] as any[], receipts: [] as any[],
    log: [] as Array<{ cmd: string; args: any; at: number }>,
  };
  const listeners = new Map<string, Set<number>>();
  const callbacks = new Map<number, (event: any) => void>();
  const handlerIds = new Map<string, number[]>();
  let nextCallbackId = 10000;
  let nextEventId = 1;

  const botName = (id: string) => seed.bots.find((b) => b.id === id)?.name ?? id;
  const emit = (event: string, payload: any) => {
    for (const id of listeners.get(event) ?? []) {
      callbacks.get(id)?.({ event, id, payload });
    }
  };
  const log = (cmd: string, args: any) => { state.log.push({ cmd, args: args ?? null, at: now() }); };
  const fail = (code: string): never => { throw new Error(code); };
  const terminals = new Set(["completed", "failed", "cancelled"]);
  const active = (a: any) => !terminals.has(a.status);

  const makeAssignment = (input: any): any => {
    const parent = input.parentAssignmentId
      ? state.assignments.find((a: any) => a.id === input.parentAssignmentId)
      : null;
    const depth = parent ? parent.depth + 1 : 1;
    if (depth > 3) fail("BOT_DELEGATE_MAX_DEPTH");
    const rootId = parent ? parent.rootId : input.rootId ?? input.rootAssignmentId ?? uid("root");
    let cursor = parent;
    while (cursor) {
      if (cursor.assigneeBotId === input.assigneeBotId) fail("BOT_DELEGATE_CYCLE");
      cursor = cursor.parentId
        ? state.assignments.find((a: any) => a.id === cursor.parentId)
        : null;
    }
    if (parent) {
      const siblings = state.assignments.filter((a: any) => a.parentId === parent.id && active(a));
      if (siblings.length >= 2) fail("BOT_DELEGATE_BUSY");
    }
    const a = {
      id: input.id ?? uid("asmt"), rootId,
      parentId: parent ? parent.id : null, depth,
      origin: input.origin ?? "person",
      requesterBotId: input.requesterBotId ?? null,
      requesterConversationId: input.requesterConversationId ?? null,
      assigneeBotId: input.assigneeBotId,
      assigneeName: botName(input.assigneeBotId),
      mode: input.mode ?? "dispatch",
      taskText: input.taskText ?? input.question ?? "",
      status: input.status ?? "completed",
      resultText: input.resultText ?? (depth > 1
        ? "[L" + depth + " · " + (input.stepKey ?? "step") + " · " + botName(input.assigneeBotId) + "] 子链回执"
        : "[步骤 " + (input.stepKey ?? "step") + " · " + botName(input.assigneeBotId) + "] 步骤回执"),
      errorCode: null, errorDetail: null,
      workflowRunId: input.workflowRunId ?? null, stepKey: input.stepKey ?? null,
      createdAt: now(), updatedAt: now(),
    };
    state.assignments.push(a);
    state.receipts.push({ id: uid("msg"), text: a.resultText, rootId, depth, stepKey: a.stepKey });
    emit("bot-assignment-updated", a);
    return a;
  };

  const wfDto = (raw: any) => ({
    id: raw.id ?? uid("wf"), name: raw.name, description: raw.description ?? "",
    ownerScope: raw.ownerScope ?? "instance", departmentId: raw.departmentId ?? null,
    steps: raw.steps ?? [], maxConcurrency: raw.maxConcurrency ?? 2,
    totalTimeoutSecs: raw.totalTimeoutSecs ?? 900, createdAt: now(), updatedAt: now(),
  });

  const runDto = (workflow: any, args: any) => {
    const steps = workflow.steps as any[];
    const outputs: Record<string, any> = {};
    for (const step of steps) {
      outputs[step.stepKey] = {
        assignmentId: uid("asmt"),
        resultText: "[步骤 " + step.stepKey + " · " + botName(step.botId) + "] 完成；提示词=" + step.promptTemplate,
      };
    }
    return {
      id: uid("run"), workflowId: workflow.id, workflowName: workflow.name,
      status: "running", origin: "person", requesterBotId: null,
      requesterConversationId: args.requesterConversationId ?? "", anchorMessageId: uid("msg"),
      context: { inputs: args.inputs ?? {}, outputs, skipped: ["skipped-step"] },
      completedSteps: steps.length, totalSteps: steps.length + 1,
      errorCode: null, errorDetail: null, startedAt: now(), finishedAt: null,
      createdAt: now(), updatedAt: now(),
    };
  };

  const invoke = async (cmd: string, args: any): Promise<any> => {
    log(cmd, args);
    switch (cmd) {
      case "list_bots": return seed.bots.map((b) => ({
        id: b.id, name: b.name, soul: b.soul ?? "", title: null, description: null,
        soulHash: "00000000", soulDrift: false, icon: "🤖", color: null, section: "default",
        modelId: null, providerId: null, effort: null, workdir: null, agentProfileId: null,
        approvalMode: b.approvalMode ?? "full", isEnabled: true, sortOrder: 0,
        createdAt: 1700000000000, updatedAt: 1700000000000,
      }));
      case "get_bot": return seed.bots.find((b) => b.id === args?.botId) ?? null;
      case "list_bot_departments": return [];
      case "list_bot_assignments": {
        const f = args ?? {};
        return state.assignments.filter((a: any) => {
          if (f.rootId && a.rootId !== f.rootId) return false;
          if (f.botId && a.assigneeBotId !== f.botId && a.requesterBotId !== f.botId) return false;
          if (f.status && a.status !== f.status) return false;
          return true;
        });
      }
      case "create_bot_assignment": return makeAssignment(args ?? {});
      case "cancel_bot_assignment": {
        const a = state.assignments.find((x: any) => x.id === args?.assignmentId);
        if (!a || !active(a)) fail("BOT_ASSIGNMENT_NOT_FOUND");
        const ids = new Set<string>([a.id]);
        let grew = true;
        while (grew) {
          grew = false;
          for (const child of state.assignments) {
            if (child.parentId && ids.has(child.parentId) && !ids.has(child.id)) {
              ids.add(child.id); grew = true;
            }
          }
        }
        for (const row of state.assignments) {
          if (ids.has(row.id) && active(row)) {
            row.status = "cancelled"; row.updatedAt = now();
            emit("bot-assignment-updated", row);
          }
        }
        return undefined;
      }
      case "list_bot_workflows": return state.workflows;
      case "create_bot_workflow": {
        const wf = wfDto(args?.input ?? {});
        state.workflows.push(wf);
        return wf;
      }
      case "update_bot_workflow": {
        const wf = state.workflows.find((x: any) => x.id === args?.id);
        if (!wf) fail("BOT_WORKFLOW_NOT_FOUND");
        if (state.runs.some((r: any) => r.workflowId === wf.id && !terminals.has(r.status))) {
          fail("BOT_WORKFLOW_RUN_BUSY");
        }
        const patch = { ...args };
        delete patch.id;
        Object.assign(wf, patch, { updatedAt: now() });
        return wf;
      }
      case "delete_bot_workflow": {
        const wf = state.workflows.find((x: any) => x.id === args?.id);
        if (!wf) fail("BOT_WORKFLOW_NOT_FOUND");
        if (state.runs.some((r: any) => r.workflowId === wf.id && !terminals.has(r.status))) {
          fail("BOT_WORKFLOW_RUN_BUSY");
        }
        state.workflows = state.workflows.filter((x: any) => x.id !== wf.id);
        return undefined;
      }
      case "list_bot_workflow_runs": {
        const f = args ?? {};
        return state.runs.filter((r: any) =>
          (!f.workflowId || r.workflowId === f.workflowId) && (!f.status || r.status === f.status));
      }
      case "run_bot_workflow": {
        if (!args || !("workflowId" in args) || !("requesterConversationId" in args)) fail("ERR_VALIDATION");
        const wf = state.workflows.find((x: any) => x.id === args.workflowId);
        if (!wf) fail("BOT_WORKFLOW_NOT_FOUND");
        const run = runDto(wf, args);
        state.runs.push(run);
        for (const [index, step] of wf.steps.entries()) {
          const rootAssignment = makeAssignment({
            id: run.context.outputs[step.stepKey].assignmentId, rootId: run.id,
            workflowRunId: run.id, stepKey: step.stepKey, assigneeBotId: step.botId,
            taskText: step.promptTemplate, origin: "person",
            requesterConversationId: run.requesterConversationId,
          });
          if (index === 0) {
            makeAssignment({
              parentAssignmentId: rootAssignment.id, rootId: run.id, workflowRunId: run.id,
              stepKey: step.stepKey + "-L2",
              assigneeBotId: seed.bots[(index + 1) % seed.bots.length]?.id ?? step.botId,
              taskText: "L2 子委派：" + step.stepKey, origin: "bot", requesterBotId: step.botId,
            });
          }
        }
        emit("bot-workflow-run-updated", run);
        return run;
      }
      case "cancel_bot_workflow_run": {
        const run = state.runs.find((r: any) => r.id === args?.runId);
        if (!run) fail("BOT_WORKFLOW_RUN_NOT_FOUND");
        if (terminals.has(run.status)) fail("BOT_WORKFLOW_CANCEL_INVALID");
        run.status = "cancelled"; run.updatedAt = now();
        for (const a of state.assignments) {
          if (a.workflowRunId === run.id && active(a)) a.status = "cancelled";
        }
        emit("bot-workflow-run-updated", run);
        return undefined;
      }
      case "create_bot_workflow_schedule": {
        const input = args?.input ?? {};
        const task = {
          id: uid("task"), name: "Bot 编排: " + input.workflowId, taskType: "bot_workflow",
          cronExpression: input.cronExpression, timezone: input.timezone ?? null,
          status: input.enabled === false ? "paused" : "active", botId: null,
          params: { workflowId: input.workflowId }, createdAt: new Date().toISOString(),
        };
        state.schedules.push(task);
        return task;
      }
      case "plugin:event|listen": {
        const id = nextEventId++;
        const set = listeners.get(args.event) ?? new Set<number>();
        set.add(args.handler);
        listeners.set(args.event, set);
        handlerIds.set(args.event, [...(handlerIds.get(args.event) ?? []), args.handler]);
        return id;
      }
      case "plugin:event|unlisten": {
        for (const id of handlerIds.get(args.event) ?? []) callbacks.delete(id);
        listeners.delete(args.event);
        handlerIds.delete(args.event);
        return undefined;
      }
      default:
        return cmd.startsWith("list_") ? [] : undefined;
    }
  };

  w.isTauri = true;
  w.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback: (cb: (event: any) => void) => {
      const id = nextCallbackId++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback: (id: number) => callbacks.delete(id),
    convertFileSrc: (p: string) => p,
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  w.__P3_TEST__ = {
    state, emit, invoke,
    listeners: () => Object.fromEntries([...listeners].map(([k, v]) => [k, [...v]])),
  };
}
