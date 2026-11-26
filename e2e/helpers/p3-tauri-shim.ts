// SPDX-License-Identifier: AGPL-3.0-only
/**
 * P3/P4 e2e 的 IPC 替身：只在测试页注入，不改 src/ 业务实现。
 * browserMockBots.ts 只实现 P1/P2 命令，P3 的 8 条编排命令在浏览器模式没有数据面。
 * 这里按 IPC-P3.md §2/§3 的冻结 DTO 与错误码做最小状态机，经 Tauri IPC 入口驱动真实页面。
 * 边界：这是 UI ↔ IPC 契约层 e2e，不替代 Rust 运行时状态机的证据。
 *
 * P4 扩展（docs/bot-integration/P4/IPC-P4.md §1/§3 冻结契约、ADR-P4 D2/D3）：
 *   · 6 条新命令：get/set/reset_bot_department_quota、list/answer/cancel_bot_input_request；
 *     以及 agent_respond_ask 的 P4 分支（命中 askId 时转 answered，否则保持旧行为）；
 *   · 事件 bot-input-request-updated（pending/answered/expired/cancelled 各发一次）；
 *   · 部门配额只收紧不放松：ownerScope='department' 的定义超配额 → BOT_QUOTA_EXCEEDED:字段；
 *     部门在跑编排数达 maxConcurrentRuns → 触发被拒（ADR-P4 D3「拒绝不裁剪」）；
 *   · 未落库部门返回 MODEL-P4 §2.1 默认值且 isDefault=true（与后端 get 语义对齐）；
 *   · 测试钩子：__P3_TEST__.seedInputRequest（模拟 B2 落 pending 行 + 事件）、
 *     __P3_TEST__.expireInputRequests（模拟 B2 启动清扫/超时转 expired）。
 */
export interface P3ShimBot {
  id: string;
  name: string;
  soul?: string;
  approvalMode?: string;
}
export interface P3ShimDepartment {
  id: string;
  name: string;
  chiefBotId?: string | null;
}
export interface P3ShimSeed {
  bots: P3ShimBot[];
  departments?: P3ShimDepartment[];
}

export function installP3Shim(seed: P3ShimSeed): void {
  const w = window as any;
  // 常量必须定义在函数体内：Playwright 的 addInitScript 只序列化函数源码，
  // 模块级标识符在页面上下文里不存在（首轮实测 'P4_QUOTA_DEFAULTS is not defined'）。
  /** MODEL-P4 §2.1 bot_department_quotas 默认值与 CHECK 范围（单一口径来源）。 */
  const P4_QUOTA_DEFAULTS = {
    maxConcurrency: 2,
    totalTimeoutSecs: 900,
    maxSteps: 12,
    maxInFlightPerDepth: 2,
    stepTimeoutSecs: 300,
    maxConcurrentRuns: 2,
  };
  const P4_QUOTA_RANGES: Record<string, [number, number]> = {
    maxConcurrency: [1, 4],
    totalTimeoutSecs: [60, 3600],
    maxSteps: [1, 12],
    maxInFlightPerDepth: [1, 4],
    stepTimeoutSecs: [30, 900],
    maxConcurrentRuns: [1, 8],
  };
  /** ADR-P4 D5：pending 请求 300s 过期。 */
  const P4_INPUT_EXPIRY_MS = 300_000;
  const P4_INPUT_STATUSES = ["pending", "answered", "expired", "cancelled"];

  const now = () => Date.now();
  const uid = (prefix: string) => prefix + "-" + Math.random().toString(36).slice(2, 10);
  // P4 刷新恢复用例需要状态跨 reload 存活：sessionStorage 仅在同标签页内共享，
  // 每个 Playwright test 独立 context，不污染其它用例。
  let persisted: { quotas?: any[]; inputRequests?: any[] } = {};
  try {
    persisted = JSON.parse(sessionStorage.getItem("__P3_SHIM_STATE__") ?? "{}");
  } catch {
    persisted = {};
  }
  const state = {
    workflows: [] as any[],
    runs: [] as any[],
    assignments: [] as any[],
    schedules: [] as any[],
    receipts: [] as any[],
    quotas: persisted.quotas ?? [],
    inputRequests: persisted.inputRequests ?? [],
    log: [] as Array<{ cmd: string; args: any; at: number }>,
  };
  const persist = () => {
    try {
      sessionStorage.setItem(
        "__P3_SHIM_STATE__",
        JSON.stringify({
          quotas: state.quotas,
          inputRequests: state.inputRequests,
        }),
      );
    } catch {
      // 无 sessionStorage 时退回纯内存状态
    }
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
  const log = (cmd: string, args: any) => {
    state.log.push({ cmd, args: args ?? null, at: now() });
  };
  const fail = (code: string): never => {
    throw new Error(code);
  };
  const terminals = new Set(["completed", "failed", "cancelled"]);
  const active = (a: any) => !terminals.has(a.status);

  // ---------- P4-a 部门配额 ----------
  const departmentExists = (id: string) => (seed.departments ?? []).some((d) => d.id === id);

  const quotaFor = (departmentId: string): any => {
    const row = state.quotas.find((q: any) => q.departmentId === departmentId);
    return row ?? {
      departmentId,
      ...P4_QUOTA_DEFAULTS,
      isDefault: true,
      updatedAt: null,
    };
  };

  /** ADR-P4 D3：超限直接拒绝，错误码带字段名（不裁剪为上限）。 */
  const quotaViolation = (field: string, defined: number, ceiling: number): never => {
    const err = new Error("BOT_QUOTA_EXCEEDED:" + field) as Error & {
      detail?: string;
      param?: string;
      code?: string;
    };
    err.code = "BOT_QUOTA_EXCEEDED";
    err.param = field;
    err.detail = field + ": 定义值 " + defined + " 超过部门配额上限 " + ceiling;
    throw err;
  };

  /** 定义层校验：仅 ownerScope='department' 受配额约束（ADR-P4 D2）。 */
  const checkDefinition = (wf: any): void => {
    if (wf.ownerScope !== "department" || !wf.departmentId) { return; }
    const quota = quotaFor(wf.departmentId);
    const steps = (wf.steps ?? []) as any[];
    if (steps.length > quota.maxSteps) { quotaViolation("maxSteps", steps.length, quota.maxSteps); }
    if (wf.maxConcurrency > quota.maxConcurrency) {
      quotaViolation("maxConcurrency", wf.maxConcurrency, quota.maxConcurrency);
    }
    if (wf.totalTimeoutSecs > quota.totalTimeoutSecs) {
      quotaViolation("totalTimeoutSecs", wf.totalTimeoutSecs, quota.totalTimeoutSecs);
    }
    for (const step of steps) {
      // MODEL-P4 §3.1：步骤级超时缺省继承工作流总预算，因此仅在显式给出时比较。
      if (
        typeof step?.stepTimeoutSecs === "number"
        && step.stepTimeoutSecs > quota.stepTimeoutSecs
      ) {
        quotaViolation("stepTimeoutSecs", step.stepTimeoutSecs, quota.stepTimeoutSecs);
      }
    }
  };

  /** 运行层校验：部门在跑编排数达上限即拒绝。 */
  const checkRunCapacity = (wf: any, excludeRunId?: string | null): void => {
    if (wf.ownerScope !== "department" || !wf.departmentId) { return; }
    const quota = quotaFor(wf.departmentId);
    const deptIds = new Set(
      state.workflows
        .filter((x: any) => x.ownerScope === "department" && x.departmentId === wf.departmentId)
        .map((x: any) => x.id),
    );
    const running = state.runs.filter(
      (r: any) => deptIds.has(r.workflowId) && !terminals.has(r.status) && r.id !== excludeRunId,
    ).length;
    if (running >= quota.maxConcurrentRuns) {
      quotaViolation("maxConcurrentRuns", running + 1, quota.maxConcurrentRuns);
    }
  };

  // ---------- P4-b 输入请求 ----------
  const inputDto = (raw: any): any => {
    const createdAt = raw.createdAt ?? now();
    return {
      id: raw.id ?? uid("input"),
      askId: raw.askId ?? uid("ask"),
      conversationId: raw.conversationId ?? "",
      botId: raw.botId ?? null,
      workflowRunId: raw.workflowRunId ?? null,
      assignmentId: raw.assignmentId ?? null,
      origin: raw.origin ?? "person",
      question: raw.question ?? "",
      options: raw.options ?? [],
      status: raw.status ?? "pending",
      answer: raw.answer ?? null,
      answeredAt: raw.answeredAt ?? null,
      expiresAt: raw.expiresAt ?? createdAt + P4_INPUT_EXPIRY_MS,
      createdAt,
      updatedAt: raw.updatedAt ?? createdAt,
    };
  };

  const findInput = (requestId: string) => state.inputRequests.find((x: any) => x.id === requestId);

  // 事件 payload 必须是**快照**：真 Tauri 每次事件都会序列化，消费方拿到的是独立对象；
  // 若直接 emit 可变行的引用，测试末尾读事件数组会看到所有事件都是最后一态（实测踩过）。
  const inputSnapshot = (row: any) => ({ ...row, options: [...(row.options ?? [])] });

  const seedInputRequest = (input: any): any => {
    const row = inputDto(input);
    state.inputRequests.push(row);
    persist();
    emit("bot-input-request-updated", inputSnapshot(row));
    return row;
  };

  const settleInput = (row: any, status: string, patch: any = {}): any => {
    row.status = status;
    Object.assign(row, patch, { updatedAt: now() });
    persist();
    emit("bot-input-request-updated", inputSnapshot(row));
    return { ...row };
  };

  const expireInputRequests = (at: number = now()): any[] => {
    const hit = state.inputRequests.filter(
      (x: any) => x.status === "pending" && x.expiresAt <= at,
    );
    for (const row of hit) { settleInput(row, "expired"); }
    return hit;
  };

  const makeAssignment = (input: any): any => {
    const parent = input.parentAssignmentId
      ? state.assignments.find((a: any) => a.id === input.parentAssignmentId)
      : null;
    const depth = parent ? parent.depth + 1 : 1;
    if (depth > 3) { fail("BOT_DELEGATE_MAX_DEPTH"); }
    const rootId = parent ? parent.rootId : input.rootId ?? input.rootAssignmentId ?? uid("root");
    let cursor = parent;
    while (cursor) {
      if (cursor.assigneeBotId === input.assigneeBotId) { fail("BOT_DELEGATE_CYCLE"); }
      cursor = cursor.parentId
        ? state.assignments.find((a: any) => a.id === cursor.parentId)
        : null;
    }
    if (parent) {
      const siblings = state.assignments.filter((a: any) => a.parentId === parent.id && active(a));
      if (siblings.length >= 2) { fail("BOT_DELEGATE_BUSY"); }
    }
    const a = {
      id: input.id ?? uid("asmt"),
      rootId,
      parentId: parent ? parent.id : null,
      depth,
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
      errorCode: null,
      errorDetail: null,
      workflowRunId: input.workflowRunId ?? null,
      stepKey: input.stepKey ?? null,
      createdAt: now(),
      updatedAt: now(),
    };
    state.assignments.push(a);
    state.receipts.push({ id: uid("msg"), text: a.resultText, rootId, depth, stepKey: a.stepKey });
    emit("bot-assignment-updated", a);
    return a;
  };

  const wfDto = (raw: any) => ({
    id: raw.id ?? uid("wf"),
    name: raw.name,
    description: raw.description ?? "",
    ownerScope: raw.ownerScope ?? "instance",
    departmentId: raw.departmentId ?? null,
    steps: raw.steps ?? [],
    maxConcurrency: raw.maxConcurrency ?? 2,
    totalTimeoutSecs: raw.totalTimeoutSecs ?? 900,
    createdAt: now(),
    updatedAt: now(),
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
      id: uid("run"),
      workflowId: workflow.id,
      workflowName: workflow.name,
      status: "running",
      origin: "person",
      requesterBotId: null,
      requesterConversationId: args.requesterConversationId ?? "",
      anchorMessageId: uid("msg"),
      context: { inputs: args.inputs ?? {}, outputs, skipped: ["skipped-step"] },
      completedSteps: steps.length,
      totalSteps: steps.length + 1,
      errorCode: null,
      errorDetail: null,
      startedAt: now(),
      finishedAt: null,
      createdAt: now(),
      updatedAt: now(),
    };
  };

  const invoke = async (cmd: string, args: any): Promise<any> => {
    log(cmd, args);
    switch (cmd) {
      case "list_bots":
        return seed.bots.map((b) => ({
          id: b.id,
          name: b.name,
          soul: b.soul ?? "",
          title: null,
          description: null,
          soulHash: "00000000",
          soulDrift: false,
          icon: "🤖",
          color: null,
          section: "default",
          modelId: null,
          providerId: null,
          effort: null,
          workdir: null,
          agentProfileId: null,
          approvalMode: b.approvalMode ?? "full",
          isEnabled: true,
          sortOrder: 0,
          createdAt: 1700000000000,
          updatedAt: 1700000000000,
        }));
      case "get_bot":
        return seed.bots.find((b) => b.id === args?.botId) ?? null;
      case "list_bot_departments":
        return (seed.departments ?? []).map((d) => ({
          id: d.id,
          name: d.name,
          chiefBotId: d.chiefBotId ?? null,
        }));

      // ---------- P4-a：部门配额 3 命令（IPC-P4 §1） ----------
      case "get_bot_department_quota": {
        const departmentId = args?.departmentId;
        if (!departmentId || !departmentExists(departmentId)) { fail("BOT_DEPARTMENT_NOT_FOUND"); }
        return quotaFor(departmentId);
      }
      case "set_bot_department_quota": {
        const input = args?.input ?? {};
        const departmentId = input.departmentId;
        if (!departmentId || !departmentExists(departmentId)) { fail("BOT_DEPARTMENT_NOT_FOUND"); }
        const next: any = { departmentId, isDefault: false, updatedAt: now() };
        for (const field of Object.keys(P4_QUOTA_RANGES)) {
          const value = input[field];
          if (value === undefined) { continue; }
          const [min, max] = P4_QUOTA_RANGES[field];
          if (typeof value !== "number" || !Number.isInteger(value) || value < min || value > max) {
            const err = new Error("BOT_QUOTA_INVALID:" + field) as Error & {
              code?: string;
              param?: string;
            };
            err.code = "BOT_QUOTA_INVALID";
            err.param = field;
            throw err;
          }
          next[field] = value;
        }
        const index = state.quotas.findIndex((q: any) => q.departmentId === departmentId);
        const merged = {
          ...P4_QUOTA_DEFAULTS,
          ...(index >= 0 ? state.quotas[index] : {}),
          ...next,
        };
        if (index >= 0) { state.quotas[index] = merged; }
        else { state.quotas.push(merged); }
        persist();
        return merged;
      }
      case "reset_bot_department_quota": {
        const departmentId = args?.departmentId;
        if (!departmentId || !departmentExists(departmentId)) { fail("BOT_DEPARTMENT_NOT_FOUND"); }
        state.quotas = state.quotas.filter((q: any) => q.departmentId !== departmentId);
        persist();
        // IPC-P4 §1：reset 签名为 Result<(), ErrorResponse>，幂等（无行也 Ok），前端保存后自行 get。
        return undefined;
      }

      // ---------- P4-b：输入请求 3 命令（IPC-P4 §1） ----------
      case "list_bot_input_requests": {
        const f = args?.filter ?? args ?? {};
        if (f.status != null && !P4_INPUT_STATUSES.includes(f.status)) { fail("ERR_VALIDATION"); }
        return state.inputRequests
          .filter((x: any) => {
            if (f.conversationId && x.conversationId !== f.conversationId) { return false; }
            if (f.botId && x.botId !== f.botId) { return false; }
            if (f.status && x.status !== f.status) { return false; }
            if (f.workflowRunId && x.workflowRunId !== f.workflowRunId) { return false; }
            return true;
          })
          .slice()
          .sort((a: any, b: any) => b.createdAt - a.createdAt);
      }
      case "answer_bot_input_request": {
        const row = findInput(args?.requestId);
        if (!row) { fail("BOT_INPUT_REQUEST_NOT_FOUND"); }
        if (row.status !== "pending") { fail("BOT_INPUT_REQUEST_NOT_PENDING"); }
        const answer = typeof args?.answer === "string" ? args.answer : "";
        if (answer.trim() === "") { fail("BOT_INPUT_ANSWER_EMPTY"); }
        return settleInput(row, "answered", { answer, answeredAt: now() });
      }
      case "cancel_bot_input_request": {
        const row = findInput(args?.requestId);
        if (!row) { fail("BOT_INPUT_REQUEST_NOT_FOUND"); }
        if (row.status !== "pending") { fail("BOT_INPUT_REQUEST_NOT_PENDING"); }
        return settleInput(row, "cancelled");
      }
      // IPC-P4 §5：签名不变（askId/answer）；命中 P4 行时转 answered，否则保持 P2 行为（undefined）。
      case "agent_respond_ask": {
        const row = state.inputRequests.find((x: any) => x.askId === args?.askId);
        if (!row) { return undefined; }
        if (row.status !== "pending") { fail("BOT_INPUT_REQUEST_NOT_PENDING"); }
        const answer = typeof args?.answer === "string" ? args.answer : "";
        if (answer.trim() === "") { fail("BOT_INPUT_ANSWER_EMPTY"); }
        return settleInput(row, "answered", { answer, answeredAt: now() });
      }

      case "list_bot_assignments": {
        const f = args ?? {};
        return state.assignments.filter((a: any) => {
          if (f.rootId && a.rootId !== f.rootId) { return false; }
          if (f.botId && a.assigneeBotId !== f.botId && a.requesterBotId !== f.botId) { return false; }
          if (f.status && a.status !== f.status) { return false; }
          return true;
        });
      }
      case "create_bot_assignment":
        return makeAssignment(args ?? {});
      case "cancel_bot_assignment": {
        const a = state.assignments.find((x: any) => x.id === args?.assignmentId);
        if (!a || !active(a)) { fail("BOT_ASSIGNMENT_NOT_FOUND"); }
        const ids = new Set<string>([a.id]);
        let grew = true;
        while (grew) {
          grew = false;
          for (const child of state.assignments) {
            if (child.parentId && ids.has(child.parentId) && !ids.has(child.id)) {
              ids.add(child.id);
              grew = true;
            }
          }
        }
        for (const row of state.assignments) {
          if (ids.has(row.id) && active(row)) {
            row.status = "cancelled";
            row.updatedAt = now();
            emit("bot-assignment-updated", row);
          }
        }
        return undefined;
      }
      case "list_bot_workflows":
        return state.workflows;
      case "create_bot_workflow": {
        const wf = wfDto(args?.input ?? {});
        checkDefinition(wf); // ADR-P4 D2/D3：定义超配额即拒绝，不写库
        state.workflows.push(wf);
        return wf;
      }
      case "update_bot_workflow": {
        const wf = state.workflows.find((x: any) => x.id === args?.id);
        if (!wf) { fail("BOT_WORKFLOW_NOT_FOUND"); }
        if (state.runs.some((r: any) => r.workflowId === wf.id && !terminals.has(r.status))) {
          fail("BOT_WORKFLOW_RUN_BUSY");
        }
        const patch = { ...args };
        delete patch.id;
        checkDefinition({ ...wf, ...patch });
        Object.assign(wf, patch, { updatedAt: now() });
        return wf;
      }
      case "delete_bot_workflow": {
        const wf = state.workflows.find((x: any) => x.id === args?.id);
        if (!wf) { fail("BOT_WORKFLOW_NOT_FOUND"); }
        if (state.runs.some((r: any) => r.workflowId === wf.id && !terminals.has(r.status))) {
          fail("BOT_WORKFLOW_RUN_BUSY");
        }
        state.workflows = state.workflows.filter((x: any) => x.id !== wf.id);
        return undefined;
      }
      case "list_bot_workflow_runs": {
        const f = args ?? {};
        return state.runs.filter((r: any) =>
          (!f.workflowId || r.workflowId === f.workflowId) && (!f.status || r.status === f.status)
        );
      }
      case "run_bot_workflow": {
        if (!args || !("workflowId" in args) || !("requesterConversationId" in args)) { fail("ERR_VALIDATION"); }
        const wf = state.workflows.find((x: any) => x.id === args.workflowId);
        if (!wf) { fail("BOT_WORKFLOW_NOT_FOUND"); }
        checkDefinition(wf);
        checkRunCapacity(wf); // ADR-P4 D3：部门在跑编排数达上限即拒绝
        const run = runDto(wf, args);
        state.runs.push(run);
        for (const [index, step] of wf.steps.entries()) {
          const rootAssignment = makeAssignment({
            id: run.context.outputs[step.stepKey].assignmentId,
            rootId: run.id,
            workflowRunId: run.id,
            stepKey: step.stepKey,
            assigneeBotId: step.botId,
            taskText: step.promptTemplate,
            origin: "person",
            requesterConversationId: run.requesterConversationId,
          });
          if (index === 0) {
            makeAssignment({
              parentAssignmentId: rootAssignment.id,
              rootId: run.id,
              workflowRunId: run.id,
              stepKey: step.stepKey + "-L2",
              assigneeBotId: seed.bots[(index + 1) % seed.bots.length]?.id ?? step.botId,
              taskText: "L2 子委派：" + step.stepKey,
              origin: "bot",
              requesterBotId: step.botId,
            });
          }
        }
        emit("bot-workflow-run-updated", run);
        return run;
      }
      case "cancel_bot_workflow_run": {
        const run = state.runs.find((r: any) => r.id === args?.runId);
        if (!run) { fail("BOT_WORKFLOW_RUN_NOT_FOUND"); }
        if (terminals.has(run.status)) { fail("BOT_WORKFLOW_CANCEL_INVALID"); }
        run.status = "cancelled";
        run.updatedAt = now();
        for (const a of state.assignments) {
          if (a.workflowRunId === run.id && active(a)) { a.status = "cancelled"; }
        }
        emit("bot-workflow-run-updated", run);
        return undefined;
      }
      case "create_bot_workflow_schedule": {
        const input = args?.input ?? {};
        const task = {
          id: uid("task"),
          name: "Bot 编排: " + input.workflowId,
          taskType: "bot_workflow",
          cronExpression: input.cronExpression,
          timezone: input.timezone ?? null,
          status: input.enabled === false ? "paused" : "active",
          botId: null,
          params: { workflowId: input.workflowId },
          createdAt: new Date().toISOString(),
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
        for (const id of handlerIds.get(args.event) ?? []) { callbacks.delete(id); }
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
    state,
    emit,
    invoke,
    listeners: () => Object.fromEntries([...listeners].map(([k, v]) => [k, [...v]])),
    // P4 钩子：模拟 B2 落库 / B2 超时清扫，让 UI 在没有 Rust 运行时的浏览器模式下也能被点亮。
    seedInputRequest,
    expireInputRequests,
    quotaFor,
  };
}
