// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 归属层类型 —— 与 `docs/bot-integration/IPC.md` §3 逐字对齐（**冻结契约**）。
 *
 * 命名约定：后端 Rust 结构体 snake_case + `#[serde(rename_all = "camelCase")]`，
 * TS 侧消费 camelCase（AGENTS.md 禁区 13）。要改字段先改契约文档，不要在 TS 侧单边改名。
 *
 * `UpdateBotInput` 的可空字段语义：`null` = 显式置空，`undefined` / 缺键 = 不改。
 * 前端组装请求时必须**省略**未修改的键，不能传 `undefined` 占位。
 */

/** Bot 审批级别。P1 仅保存配置（字段可编辑），执行语义在 P2 生效。 */
export type ApprovalMode = "ask" | "edits" | "auto" | "full" | "custom";

/** 审批级别枚举值（渲染顺序即声明顺序），与后端校验集合一致。 */
export const APPROVAL_MODES: readonly ApprovalMode[] = ["ask", "edits", "auto", "full", "custom"];

export interface Bot {
  id: string;
  name: string;
  title: string | null;
  description: string | null;
  soul: string;
  soulHash: string;
  soulDrift: boolean;
  icon: string;
  color: string | null;
  section: string;
  modelId: string | null;
  providerId: string | null;
  effort: string | null;
  workdir: string | null;
  approvalMode: ApprovalMode;
  agentProfileId: string | null;
  isEnabled: boolean;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface CreateBotInput {
  name: string;
  title?: string | null;
  description?: string | null;
  soul?: string;
  icon?: string;
  color?: string | null;
  section?: string;
  modelId?: string | null;
  providerId?: string | null;
  effort?: string | null;
  workdir?: string | null;
  approvalMode?: ApprovalMode;
  agentProfileId?: string | null;
}

export interface UpdateBotInput extends Partial<CreateBotInput> {
  isEnabled?: boolean;
  sortOrder?: number;
}

export interface BotSoul {
  content: string;
  hash: string;
  drift: boolean;
}

export interface MemoryBudget {
  maxLines: number;
  maxBytes: number;
}

export interface MemoryTopic {
  relativePath: string;
  category: string;
  title: string;
  updatedAt: number;
}

export interface BotMemory {
  indexContent: string;
  lineCount: number;
  byteCount: number;
  budget: MemoryBudget;
  topics: MemoryTopic[];
}

export interface DeleteBotResult {
  detachedThreads: number;
  memoryPath: string;
}
