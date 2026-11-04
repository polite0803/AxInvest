// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import type { PluginTaskRunRowDto, PluginTaskSourceDto } from "@/types";
import { mapRunTrigger, runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * 插件任务源的**泛型适配器**工厂（任务 #148，PLAN §97.4）。
 *
 * 为什么是工厂而不是让插件交一个 adapter 文件：§93 已裁「插件不进主 origin」——
 * 宿主只吃两类值（schema 卡片 / iframe 富卡片），代码形态的适配器属于第三方 React
 * 注入那一条被否决的路。这里改为**声明式**：插件给 `(id, title, command)`，
 * 宿主用同一份工厂把它接成 `TaskSourceAdapter`，取数走 T2 命名命令、
 * 行形状是 `PluginTaskRunRow`（camelCase，契约见 `axagent-plugin-proto`）。
 *
 * 因此新增一个插件任务源 = 改插件清单 + 实现一条命令，宿主前端零改动。
 */

/**
 * `RunState` 的七态（与 plugin-proto 的 `PLUGIN_TASK_RUN_STATES` 逐字同源）。
 *
 * 用 `Record<RunState, RunState>` 而不是裸字符串数组：`model.ts` 若加第八态，
 * 这张表少一行就**编译不过**，而不是「新状态静默落 waiting」。跨语言镜像拦不住
 * （Rust 侧的表要等 B8-3 生成 DTO 才能同源），至少要锁住前端这一侧的漂移。
 */
const RUN_STATE_VALUES: Record<RunState, RunState> = {
  queued: "queued",
  running: "running",
  waiting: "waiting",
  paused: "paused",
  succeeded: "succeeded",
  failed: "failed",
  canceled: "canceled",
};
const RUN_STATES: readonly string[] = Object.values(RUN_STATE_VALUES);

/**
 * 表外状态的落点。
 *
 * 选 `waiting` 而不是 `running` / `failed`：`waiting` 在中心的语义是「还没轮到」，
 * 既不会把一条状态未知的执行标成正在跑（骗过「进行中」统计），也不会标成失败
 * （骗过健康度）。原始状态仍完整留在 `raw` 里，不丢信息。
 */
const UNKNOWN_STATE_FALLBACK: RunState = "waiting";

export function createPluginTaskSourceAdapter(
  pluginId: string,
  source: PluginTaskSourceDto,
): TaskSourceAdapter {
  return {
    id: source.id,
    // 插件源没有宿主 i18n key（文案归产物自己负责）⇒ nameKey 留空，渲染侧读 `name`。
    // 留空串而不是删字段：既有 4 个消费点都按 `name ?? t(nameKey)` 取值，字段缺了会 TypeError。
    nameKey: "",
    name: source.title,
    color: source.color ?? "default",

    async list(): Promise<UnifiedRun[]> {
      // 与 `stock-analysis-utils.computeEvidenceWeights` 同一条通道：宿主按 (pluginId, name)
      // 发命令帧，返回体是 JSON 字符串。命令失败 ⇒ 抛错，由 `aggregateRuns` 收成
      // 单源 SourceFetchError（一个插件坏了不拖垮整页），这里不做静默空列表。
      const raw = await invoke<string>("plugin_execute_command", {
        pluginId,
        name: source.command,
        input: {},
      });
      const parsed: unknown = JSON.parse(raw);
      if (!Array.isArray(parsed)) {
        throw new Error(
          `插件任务源「${source.id}」的取数命令「${source.command}」应返回行数组，实际 ${typeof parsed}`,
        );
      }
      return parsed.map((row) => toUnifiedRun(source.id, row as PluginTaskRunRowDto));
    },
  };
}

/** 一行插件记录 → `UnifiedRun`。`id` / `source` 由宿主拼，插件不得自带前缀。 */
function toUnifiedRun(sourceId: string, row: PluginTaskRunRowDto): UnifiedRun {
  const state = RUN_STATES.includes(row.state) ? (row.state as RunState) : UNKNOWN_STATE_FALLBACK;
  const deepLink = row.deepLink ?? undefined;
  return {
    id: runIdOf(sourceId, row.nativeId),
    source: sourceId,
    nativeId: row.nativeId,
    name: row.name,
    trigger: mapRunTrigger(row.trigger ?? undefined),
    state,
    attempts: 0,
    startedAt: row.startedAt ?? undefined,
    finishedAt: row.finishedAt ?? undefined,
    // 插件没给耗时时用两端点算，算不出（缺端点 / 倒挂）就不显示，不编一个数。
    durationMs: resolveDurationMs(row),
    deepLink,
    capabilities: {
      // 只有真给了站内路由才算「可打开」：指向不存在的路由会让中心交出 404。
      openSource: Boolean(deepLink),
    },
    raw: row.raw ?? row,
  };
}

/** `durationMs` 优先取插件给的；缺省时由两端点相减，倒挂（结束早于开始）视为未知。 */
function resolveDurationMs(row: PluginTaskRunRowDto): number | undefined {
  if (typeof row.durationMs === "number" && row.durationMs >= 0) {
    return row.durationMs;
  }
  const { startedAt, finishedAt } = row;
  if (typeof startedAt === "number" && typeof finishedAt === "number" && finishedAt >= startedAt) {
    return finishedAt - startedAt;
  }
  return undefined;
}
