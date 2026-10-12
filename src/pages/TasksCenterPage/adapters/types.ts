// SPDX-License-Identifier: AGPL-3.0-only

import type { RunLogLine, RunQuery, RunStep, SourceId, UnifiedRun } from "../model";

/**
 * 任务源适配器（Task Source Adapter）。
 *
 * 每个任务源一个文件；中心只通过注册表拿适配器，不写任何源特定逻辑。
 * 新增任务源 = 新增适配器 + registry 注册一行。
 *
 * 能力约定：
 * - list 必选；get / cancel / retry / pause / resume 按需实现，
 *   适配器同时要在映射出的 UnifiedRun.capabilities 中如实声明；
 * - getSteps/getLogs/getIO/getHistory 仅在对应能力为 true 时会被调用；
 * - eventTopics 声明该源推送的 Tauri 事件名，由中心统一订阅（P2-2 起）。
 */
export interface TaskSourceAdapter {
  readonly id: SourceId;
  /** i18n 键（tasksCenter.source.<id>） */
  readonly nameKey: string;
  /** antd Tag 色板 */
  readonly color: string;
  list(query?: RunQuery): Promise<UnifiedRun[]>;
  get?(nativeId: string): Promise<UnifiedRun | null>;
  cancel?(nativeId: string): Promise<void>;
  retry?(nativeId: string): Promise<void>;
  pause?(nativeId: string): Promise<void>;
  resume?(nativeId: string): Promise<void>;
  getSteps?(nativeId: string): Promise<RunStep[]>;
  getLogs?(nativeId: string): Promise<RunLogLine[]>;
  getIO?(nativeId: string): Promise<{ input?: unknown; output?: unknown }>;
  getHistory?(nativeId: string): Promise<UnifiedRun[]>;
  readonly eventTopics?: readonly string[];
}
