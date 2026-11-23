// SPDX-License-Identifier: AGPL-3.0-only

// Bot Store —— Bot 归属层的列表 / CRUD / SOUL / 记忆 / 线程读取
//
// 命令契约冻结在 `docs/bot-integration/IPC.md`（9 个命令、11 个错误码）：
// 参数一律 camelCase（Tauri v2 默认 `rename_all = "camelCase"`），
// 错误经 `backendErrorText` 翻译为 `error.<CODE>` 文案后落到 `error`。
//
// 分层：本文件属 feature 层。`components/bot/**` 可直读本 store；
// `components/chat/**` 禁止引用（跨模块走 `pages/**` 的 props 注入）。

import { backendErrorText } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type {
  Bot,
  BotAssignment,
  BotDepartment,
  BotMemory,
  BotSoul,
  BotWorkflow,
  BotWorkflowRun,
  Conversation,
  CreateBotAssignmentInput,
  CreateBotInput,
  CreateBotWorkflowInput,
  CreateBotWorkflowScheduleInput,
  DeleteBotResult,
  RunBotWorkflowInput,
  SaveBotDepartmentInput,
  ScheduledTask,
  UpdateBotInput,
  UpdateBotWorkflowInput,
} from "@/types";
import { create } from "zustand";

export interface BotStoreState {
  /** 侧栏 / 列表数据源（后端已按 sortOrder 排序） */
  bots: Bot[];
  loading: boolean;
  /** 创建 / 更新的在途标记 */
  saving: boolean;
  /** 删除的在途标记 */
  deleting: boolean;
  /** 最近一次失败的本地化文案（null = 无错误） */
  error: string | null;
  /** 当前选中的 Bot（详情面板与新建线程的归属来源） */
  activeBotId: string | null;

  // P3：编排工作流（workflow slice，扁平挂在 botStore 上，不另起 store）
  workflows: BotWorkflow[];
  workflowRuns: BotWorkflowRun[];
  workflowLoading: boolean;
  workflowSaving: boolean;

  loadBots: (includeDisabled?: boolean) => Promise<Bot[]>;
  setActiveBot: (botId: string | null) => void;
  /** 单个 Bot 的最新快照（编辑前刷新，避免列表快照过期） */
  getBot: (botId: string) => Promise<Bot>;
  createBot: (input: CreateBotInput) => Promise<Bot>;
  updateBot: (botId: string, input: UpdateBotInput) => Promise<Bot>;
  deleteBot: (botId: string) => Promise<DeleteBotResult>;
  /** 只读：SOUL 正文 + hash + 漂移标记 */
  readSoul: (botId: string) => Promise<BotSoul>;
  /** 写 DB + 原子重写镜像；成功后同步列表里的 soul / hash / drift */
  writeSoul: (botId: string, content: string) => Promise<BotSoul>;
  /** 只读：MEMORY.md 索引 + 预算 + 主题清单 */
  readMemory: (botId: string) => Promise<BotMemory>;
  /** 只读：该 Bot 的线程列表（直接返回既有 Conversation DTO） */
  listThreads: (botId: string, includeArchived?: boolean) => Promise<Conversation[]>;
  /** P2-a：该 Bot 的例程（IPC-P2 §2.2：含 paused/disabled 全量，不设分页） */
  listBotRoutines: (botId: string) => Promise<ScheduledTask[]>;
  createBotRoutine: (input: BotRoutineInput) => Promise<ScheduledTask>;
  /**
   * P2-a：编辑例程。P1/IPC-P2 §2.5 的 update 语义是**完整 DTO 回传**（以原任务为底覆盖
   * 名称/prompt/cron），timezone 走独立顶层参数：undefined=不改，null=清除（回落宿主 Local）。
   */
  updateBotRoutine: (taskId: string, updatedTask: ScheduledTask, timezone?: string | null) => Promise<void>;
  deleteBotRoutine: (taskId: string) => Promise<void>;
  pauseBotRoutine: (taskId: string) => Promise<void>;
  resumeBotRoutine: (taskId: string) => Promise<void>;

  // ── P2-b：部门 ──
  listBotDepartments: () => Promise<BotDepartment[]>;
  createBotDepartment: (input: SaveBotDepartmentInput) => Promise<BotDepartment>;
  updateBotDepartment: (departmentId: string, input: SaveBotDepartmentInput) => Promise<BotDepartment>;
  deleteBotDepartment: (departmentId: string) => Promise<void>;
  addBotToDepartment: (departmentId: string, botId: string) => Promise<void>;
  removeBotFromDepartment: (departmentId: string, botId: string) => Promise<void>;
  // ── P2-b：委派 ──
  createBotAssignment: (input: CreateBotAssignmentInput) => Promise<BotAssignment>;
  listBotAssignments: (filter?: BotAssignmentFilter) => Promise<BotAssignment[]>;
  cancelBotAssignment: (assignmentId: string) => Promise<void>;

  // P3：编排定义 / 运行 / 定时（IPC-P3 §2）
  loadWorkflows: (departmentId?: string | null) => Promise<BotWorkflow[]>;
  createWorkflow: (input: CreateBotWorkflowInput) => Promise<BotWorkflow>;
  updateWorkflow: (workflowId: string, input: UpdateBotWorkflowInput) => Promise<BotWorkflow>;
  deleteWorkflow: (workflowId: string) => Promise<void>;
  runWorkflow: (input: RunBotWorkflowInput) => Promise<BotWorkflowRun>;
  loadWorkflowRuns: (filter?: BotWorkflowRunFilter) => Promise<BotWorkflowRun[]>;
  cancelWorkflowRun: (runId: string) => Promise<void>;
  createWorkflowSchedule: (input: CreateBotWorkflowScheduleInput) => Promise<ScheduledTask>;
}

/** IPC-P2B §2 `list_bot_assignments` 过滤参数（皆可选）。 */
export interface BotAssignmentFilter {
  requesterBotId?: string | null;
  assigneeBotId?: string | null;
  botId?: string | null;
  status?: BotAssignment["status"] | null;
  /** IPC-P3 §1：按委派树根汇总（root_id = 自身 id 的链根）。 */
  rootId?: string | null;
}

/** IPC-P3 §2.4 list_bot_workflow_runs 过滤参数。 */
export interface BotWorkflowRunFilter {
  workflowId?: string | null;
  status?: BotWorkflowRun["status"] | null;
}

/** IPC-P2 §2.1 create_bot_scheduled_task 入参 */
export interface BotRoutineInput {
  botId: string;
  name: string;
  prompt: string;
  cronExpression: string;
  timezone?: string | null;
}

/** 防止并发 loadBots 的旧响应覆盖新响应（切换 includeDisabled 时的高发点）。 */
let loadSeq = 0;

export const useBotStore = create<BotStoreState>((set) => ({
  bots: [],
  loading: false,
  saving: false,
  deleting: false,
  error: null,
  activeBotId: null,
  workflows: [],
  workflowRuns: [],
  workflowLoading: false,
  workflowSaving: false,

  loadBots: async (includeDisabled = false) => {
    const seq = ++loadSeq;
    set({ loading: true, error: null });
    try {
      const bots = await invoke<Bot[]>("list_bots", { includeDisabled });
      if (seq === loadSeq) {
        set({ bots, loading: false });
      }
      return bots;
    } catch (e) {
      if (seq === loadSeq) {
        set({ loading: false, error: backendErrorText(e) });
      }
      throw e;
    }
  },

  setActiveBot: (botId) => {
    set({ activeBotId: botId });
  },

  getBot: async (botId) => {
    const bot = await invoke<Bot>("get_bot", { botId });
    set((s) => ({
      bots: s.bots.some((b) => b.id === bot.id) ? s.bots.map((b) => (b.id === bot.id ? bot : b)) : [bot, ...s.bots],
    }));
    return bot;
  },

  createBot: async (input) => {
    set({ saving: true, error: null });
    try {
      const bot = await invoke<Bot>("create_bot", { input });
      set((s) => ({
        bots: [bot, ...s.bots.filter((b) => b.id !== bot.id)],
        activeBotId: bot.id,
        saving: false,
      }));
      return bot;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  updateBot: async (botId, input) => {
    set({ saving: true, error: null });
    try {
      const bot = await invoke<Bot>("update_bot", { botId, input });
      set((s) => ({
        bots: s.bots.map((b) => (b.id === bot.id ? bot : b)),
        saving: false,
      }));
      return bot;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  deleteBot: async (botId) => {
    set({ deleting: true, error: null });
    try {
      const result = await invoke<DeleteBotResult>("delete_bot", { botId });
      set((s) => ({
        bots: s.bots.filter((b) => b.id !== botId),
        activeBotId: s.activeBotId === botId ? null : s.activeBotId,
        deleting: false,
      }));
      return result;
    } catch (e) {
      set({ deleting: false, error: backendErrorText(e) });
      throw e;
    }
  },

  readSoul: (botId) => invoke<BotSoul>("read_bot_soul", { botId }),

  writeSoul: async (botId, content) => {
    set({ saving: true, error: null });
    try {
      const soul = await invoke<BotSoul>("write_bot_soul", { botId, content });
      set((s) => ({
        saving: false,
        bots: s.bots.map((b) =>
          b.id === botId ? { ...b, soul: soul.content, soulHash: soul.hash, soulDrift: soul.drift } : b
        ),
      }));
      return soul;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  readMemory: (botId) => invoke<BotMemory>("read_bot_memory", { botId }),

  listThreads: (botId, includeArchived = false) =>
    invoke<Conversation[]>("list_bot_threads", { botId, includeArchived }),

  listBotRoutines: (botId) => invoke<ScheduledTask[]>("list_bot_scheduled_tasks", { botId }),

  createBotRoutine: async (input) => {
    set({ saving: true, error: null });
    try {
      const task = await invoke<ScheduledTask>("create_bot_scheduled_task", {
        botId: input.botId,
        name: input.name,
        prompt: input.prompt,
        cronExpression: input.cronExpression,
        timezone: input.timezone ?? null,
      });
      set({ saving: false });
      return task;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  updateBotRoutine: async (taskId, updatedTask, timezone) => {
    set({ saving: true, error: null });
    try {
      await invoke("update_scheduled_task", {
        taskId,
        task: updatedTask,
        // T5' 冻结契约 §2.5：外层 undefined=不改；null=清除回落 Local
        ...(timezone === undefined ? {} : { timezone }),
      });
      set({ saving: false });
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  deleteBotRoutine: async (taskId) => {
    try {
      await invoke("delete_scheduled_task", { taskId });
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  pauseBotRoutine: async (taskId) => {
    await invoke("pause_scheduled_task", { taskId });
  },

  resumeBotRoutine: async (taskId) => {
    await invoke("resume_scheduled_task", { taskId });
  },

  // ── P2-b：部门（IPC-P2B §1 五命令）──

  listBotDepartments: () => invoke<BotDepartment[]>("list_bot_departments"),

  createBotDepartment: async (input) => {
    set({ saving: true, error: null });
    try {
      const department = await invoke<BotDepartment>("create_bot_department", { input });
      set({ saving: false });
      return department;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  updateBotDepartment: async (departmentId, input) => {
    set({ saving: true, error: null });
    try {
      const department = await invoke<BotDepartment>("update_bot_department", { departmentId, input });
      set({ saving: false });
      return department;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  deleteBotDepartment: async (departmentId) => {
    try {
      await invoke("delete_bot_department", { departmentId });
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  addBotToDepartment: async (departmentId, botId) => {
    try {
      await invoke("add_bot_to_department", { departmentId, botId });
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  removeBotFromDepartment: async (departmentId, botId) => {
    try {
      await invoke("remove_bot_from_department", { departmentId, botId });
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  // ── P2-b：委派（IPC-P2B §1 三命令；mode 前端固定 dispatch，IPC-P2B §2.3）──

  createBotAssignment: async (input) => {
    try {
      return await invoke<BotAssignment>("create_bot_assignment", {
        assigneeBotId: input.assigneeBotId,
        taskText: input.taskText,
        mode: input.mode ?? "dispatch",
        requesterConversationId: input.requesterConversationId ?? null,
        requesterBotId: input.requesterBotId ?? null,
      });
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  listBotAssignments: (filter) =>
    invoke<BotAssignment[]>("list_bot_assignments", {
      requesterBotId: filter?.requesterBotId ?? null,
      assigneeBotId: filter?.assigneeBotId ?? null,
      botId: filter?.botId ?? null,
      status: filter?.status ?? null,
      // IPC-P3 §1：新增可选 rootId 过滤，旧调用方传 null 行为不变。
      rootId: filter?.rootId ?? null,
    }),

  cancelBotAssignment: async (assignmentId) => {
    try {
      await invoke("cancel_bot_assignment", { assignmentId });
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  // ── P3：编排（IPC-P3 §2 命令名与参数逐字对齐）──

  loadWorkflows: async (departmentId) => {
    set({ workflowLoading: true, error: null });
    try {
      // undefined = 全部可见；显式 null = 仅 instance 归属。
      const params = departmentId === undefined ? {} : { departmentId };
      const workflows = await invoke<BotWorkflow[]>("list_bot_workflows", params);
      set({ workflows: Array.isArray(workflows) ? workflows : [], workflowLoading: false });
      return workflows;
    } catch (e) {
      set({ workflowLoading: false, error: backendErrorText(e) });
      throw e;
    }
  },

  createWorkflow: async (input) => {
    set({ workflowSaving: true, error: null });
    try {
      const workflow = await invoke<BotWorkflow>("create_bot_workflow", { input });
      set((s) => ({
        workflows: [workflow, ...s.workflows.filter((w) => w.id !== workflow.id)],
        workflowSaving: false,
      }));
      return workflow;
    } catch (e) {
      set({ workflowSaving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  updateWorkflow: async (workflowId, input) => {
    set({ workflowSaving: true, error: null });
    try {
      const workflow = await invoke<BotWorkflow>("update_bot_workflow", { workflowId, input });
      set((s) => ({
        workflows: s.workflows.map((w) => (w.id === workflow.id ? workflow : w)),
        workflowSaving: false,
      }));
      return workflow;
    } catch (e) {
      set({ workflowSaving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  deleteWorkflow: async (workflowId) => {
    try {
      await invoke("delete_bot_workflow", { workflowId });
      set((s) => ({
        workflows: s.workflows.filter((w) => w.id !== workflowId),
        workflowRuns: s.workflowRuns.filter((r) => r.workflowId !== workflowId),
      }));
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  runWorkflow: async (input) => {
    set({ error: null });
    try {
      const run = await invoke<BotWorkflowRun>("run_bot_workflow", { input });
      set((s) => ({ workflowRuns: [run, ...s.workflowRuns.filter((r) => r.id !== run.id)] }));
      return run;
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  loadWorkflowRuns: async (filter) => {
    set({ workflowLoading: true, error: null });
    try {
      const runs = await invoke<BotWorkflowRun[]>("list_bot_workflow_runs", {
        workflowId: filter?.workflowId ?? null,
        status: filter?.status ?? null,
      });
      set({ workflowRuns: Array.isArray(runs) ? runs : [], workflowLoading: false });
      return runs;
    } catch (e) {
      set({ workflowLoading: false, error: backendErrorText(e) });
      throw e;
    }
  },

  cancelWorkflowRun: async (runId) => {
    try {
      await invoke("cancel_bot_workflow_run", { runId });
      set((s) => ({
        workflowRuns: s.workflowRuns.map((r) =>
          r.id === runId ? { ...r, status: "cancelled" as const, updatedAt: Date.now() } : r
        ),
      }));
    } catch (e) {
      set({ error: backendErrorText(e) });
      throw e;
    }
  },

  createWorkflowSchedule: async (input) => {
    set({ workflowSaving: true, error: null });
    try {
      const task = await invoke<ScheduledTask>("create_bot_workflow_schedule", { input });
      set({ workflowSaving: false });
      return task;
    } catch (e) {
      set({ workflowSaving: false, error: backendErrorText(e) });
      throw e;
    }
  },
}));
