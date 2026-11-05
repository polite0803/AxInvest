// SPDX-License-Identifier: AGPL-3.0-only

import type { ReactNode } from "react";
import type { RunLogLine, RunQuery, RunStep, SourceId, UnifiedRun } from "../model";

/**
 * 详情形态（点一行弹出的抽屉）的入参。
 *
 * 中心只给「这一行 + 关闭动作」两样，**不给源特定 props** —— 各源详情组件要什么
 * （整条 run 的 raw / 只要 nativeId / 只要 batchId）由该源自己在 `renderDetail` 里安排。
 */
export interface RunDetailContext {
  readonly run: UnifiedRun;
  readonly close: () => void;
}

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
  /**
   * 纯文本源名 —— **插件贡献的源专用**（任务 #148）。
   *
   * 与 `PluginPageDto.title` 同口径：文案归插件产物自己负责，不占宿主 11 语言 key。
   * 渲染侧唯一判据是 `adapter.name ?? t(adapter.nameKey)`（宿主源自带 `nameKey`，
   * 插件源自带 `name`），不许出现「两个都有时看谁」的分支。
   */
  readonly name?: string;
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
  /**
   * 点一行弹出的**详情形态**，由该源自己声明（任务 #151 / PLAN §100）。
   *
   * 为什么放适配器而不是留在中心：这五个源的详情组件 props 本来就各不相同
   * （pipeline 要整条 run 的 `raw`、批次只要 `nativeId`、cron 只要 `taskId + title`），
   * 让中心替它们做适配就等于「一个中性分发器知道每个源怎么渲染」—— 于是每加一个源
   * 都要回来改这个文件，而它正是股票耦合的落脚处。
   *
   * ⚠ 这是**宿主内**的渲染位。§93 已裁第三方 React 不得进主 origin ⇒ 插件贡献的源
   * 今天不声明它；未声明一律走中心的空抽屉兜底，与改造前的表现逐字一致。
   */
  renderDetail?(ctx: RunDetailContext): ReactNode;
}
