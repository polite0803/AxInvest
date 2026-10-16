// SPDX-License-Identifier: AGPL-3.0-only

/** 预算用量状态（对应后端 `scheduler::gate::BudgetState`，scheduler_budget 内存态）。 */
export interface BudgetState {
  /** 成本上限（USD）。null = 不限。 */
  maxBudget: number | null;
  /** 已累计成本（USD）。 */
  spent: number;
  /** 是否已熔断（累计成本超限）。 */
  tripped: boolean;
}

/** 审批汇总条目（对后端 `report::ApprovalSummary`）。 */
export interface ApprovalSummary {
  approver: string | null;
  status: string | null;
  decision: string | null;
}

/** 长时任务报告（对后端 `report::TaskReport`）。 */
export interface TaskReport {
  taskId: string;
  title: string;
  status: string;
  durationMs: number;
  /** 成本折算（USD） */
  costUsd: number;
  /** 审批记录汇总 */
  approvals: ApprovalSummary[];
  /** 报告生成时间（epoch ms） */
  generatedAt: number;
}

// ── 定时任务（任务中心「定时任务」tab，对齐后端 commands/scheduled_task.rs）─────

/**
 * 后端 TaskConfigDto（rename_all=camelCase）。
 * 字段拼写必须保持 camelCase：更新任务时后端 serde 反序列化直接依赖。
 */
export interface TaskConfig {
  timeoutSeconds: number;
  retryOnFailure: boolean;
  maxRetries: number;
  retryDelaySeconds: number;
  notificationEnabled: boolean;
  runOnStartup: boolean;
}

/** 后端 TaskRunResultDto（rename_all=camelCase）。 */
export interface TaskRunResult {
  success: boolean;
  output: string | null;
  error: string | null;
  durationMs: number;
  /** 被前置闸门跳过（非交易日等），未真正执行业务（旧记录缺省为 false） */
  skipped?: boolean;
  /**
   * P1-10：批量任务「部分完成」——达到单轮墙钟预算提前收尾 / 队列被单轮上限
   * 截断（属部分完成而非失败，剩余标的下轮续跑；旧记录缺省为 false）。
   */
  partial?: boolean;
}

/**
 * 后端 ScheduledTaskDto（rename_all=camelCase）的前端镜像。
 * 后端只持久化 cronExpression，不存在 schedule_config（可视化配置仅在表单态存在）。
 */
export interface ScheduledTask {
  id: string;
  name: string;
  description: string;
  taskType: string;
  cronExpression: string | null;
  intervalSeconds: number | null;
  nextRunAt: string | null;
  lastRunAt: string | null;
  lastResult: TaskRunResult | null;
  status: "active" | "paused" | "disabled";
  config: TaskConfig;
  createdAt: string;
  updatedAt: string;
  workflowId?: string | null;
  /** 结构化业务参数（参数化任务使用，如时间旅行批次的 asOf/档位勾选） */
  params?: unknown;
}

/**
 * 任务执行历史记录（get_task_execution_history 返回）。
 *
 * ⚠ 字段为 **snake_case**：后端权威类型是 runtime-core
 * `cron_job::ExecutionRecord`，未加 `#[serde(rename_all = "camelCase")]`，
 * 实际 IPC 线序即 snake_case。这是全站 camelCase 铁律（AGENTS 禁区 13）
 * 的已知历史例外——该 DTO 还被持久化 JSON 消费，加 rename 会破坏旧数据，
 * 故前端镜像逐字保持 snake_case，禁止顺手改成 camelCase。
 */
export interface TaskExecutionRecord {
  id: string;
  task_id: string;
  started_at: number;
  completed_at: number | null;
  success: boolean;
  output: string | null;
  error: string | null;
  duration_ms: number;
}
