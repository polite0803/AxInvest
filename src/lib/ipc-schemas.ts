// SPDX-License-Identifier: AGPL-3.0-only

/**
 * IPC 返回值的**运行时**契约（zod）。
 *
 * ## 为什么需要
 *
 * `invoke<T>(cmd, args)` 的泛型 `T` 只是**编译期断言** —— 它管得住「我怎么用它」，
 * 管不住「后端实际返回了什么」。于是当后端 DTO 漂移（改字段名 / 换嵌套 / 出错时返回
 * `undefined`）时，前端拿到的是一个静默的 `undefined`：`const list = await invoke<Foo[]>()`
 * 里 `list.map` 才在很远的地方炸，或者根本不炸，只表现为「这块面板一直空」。
 *
 * 典型实证（2026-09-19）：`get_stock_analysis` 在 `src/lib/browserMock.ts` 中**没有**
 * 显式 mock 分支，因而落到 default 的 `get_` 前缀兜底返回 `{}` —— 而 `{}` 不含 `id`，
 * 与本文的 `z.object({ id: z.string() })` 不符 ⇒ 该校验会抛 `IpcSchemaError`。
 *
 * ⚠ 对照（这条对照本身就是一次教训）：`list_stock_analyses` **同样**没有显式 mock 分支，
 * 但 default 的 `list_` 前缀规则返回 `[]`，**恰好**与 `z.array(...)` 相符，故不受影响。
 * 两个命令「都没分支」却「一个违约一个不违约」—— 说明兜底行为必须**逐命令核对
 * `browserMock.ts` 的 default 前缀规则**，不能按印象推断。本文首版注释曾把两者一并
 * 写成「落在兜底分支返回 `undefined`」，与被测代码事实不符（已订正）。
 *
 * ## 设计原则（避免把门禁做成噪声）
 *
 * 1. **只锁「形状 + 关键字段」**，不锁全部字段 —— 契约层的职责是抓住「返回类型整体不对」，
 *    而不是复刻 DTO。字段名一律从 `@/types` 的类型派生（`Pick<…>`），
 *    保证与编译期类型**同源**，不手抄。
 * 2. **只覆盖已登记的命令**。未登记的命令不校验（渐进扩面，避免一次性全量校验
 *    把历史遗留的 DTO 漂移全炸出来）。
 * 3. **失败即抛错**（`IpcSchemaError`），不走 `console.warn` —— 软判据等于没判据
 *    （项目既有判据：从不 fail 的门禁 ≈ 没装门禁）。错误消息里必须点名命令、
 *    路径、期望与实际，并提示「若处于浏览器模式，可能是该命令缺 mock 分支」。
 *
 * ## 如何扩面
 *
 * 在 `RESULT_SCHEMAS` 里加一条：命令名 → schema。字段名用 `Pick<YourType, "a" | "b">`
 * 从 `@/types` 取，**不要**手写字符串数组以外的副本。加完跑 `npm run typecheck`：
 * 若某字段在类型里不存在，`z.ZodType<Pick<…>>` 会让 tsc 直接报错。
 */

import type { AnalysisSummary } from "@/types";
import { z } from "zod";

/** 校验失败时抛出。消息面向排障：点名命令、路径、期望与实际。 */
export class IpcSchemaError extends Error {
  constructor(
    public readonly cmdName: string,
    public readonly path: string,
    public readonly expected: string,
    public readonly actual: string,
  ) {
    super(
      `IPC 契约不符：命令 "${cmdName}" 的返回值在 ${path} 处期望 ${expected}，实际 ${actual}。`
        + `\n  可能原因：① 后端 DTO 漂移（字段改名 / 类型变化）；`
        + `② 浏览器 mock 模式缺该命令的显式 mock 分支（default 兜底按命令前缀给出 [] 或 {}，未必合契约）；`
        + `③ 命令本身报错但被上游吞掉。`
        + "\n  排查：用 scripts/check-contracts.mjs，或直接看该命令的后端实现。",
    );
    this.name = "IpcSchemaError";
  }
}

/** 列表项契约：只锁「能标识一行」的最小字段集。 */
type AnalysisListItemKeys = Pick<AnalysisSummary, "id" | "stockCode" | "status">;

const analysisListItemSchema: z.ZodType<AnalysisListItemKeys> = z.object({
  id: z.string(),
  stockCode: z.string(),
  status: z.string(),
});

/** 命令名 → 返回值契约。**渐进扩面**：只登记已确证形状的命令。 */
const RESULT_SCHEMAS: Record<string, z.ZodType> = {
  // 分析历史列表：调用点 `stockAnalysisStore.ts:1085` 按数组消费
  list_stock_analyses: z.array(analysisListItemSchema),
  // 单条分析详情：只锁主键 —— 该命令的返回体较大且多处调用点的类型声明不一致
  // （`HistoricalAnalysisPanel` 只取 blackboardSnapshot，`WhatIfBacktest` 当 AnalysisRecord 用），
  // 先锁住「是对象且有 id」，避免用一份声明去否定另一处。
  get_stock_analysis: z.object({ id: z.string() }),
};

/** 该命令是否已登记契约。 */
export function hasIpcSchema(cmd: string): boolean {
  return Object.hasOwn(RESULT_SCHEMAS, cmd);
}

/**
 * 校验返回值。已登记的命令不符合契约时**抛 `IpcSchemaError`**；
 * 未登记的命令直接放行（返回 `undefined` 表示「无需处理」）。
 */
export function validateIpcResult(cmd: string, data: unknown): void {
  const schema = RESULT_SCHEMAS[cmd];
  if (!schema) { return; }

  const parsed = schema.safeParse(data);
  if (parsed.success) { return; }

  const first = parsed.error.issues[0];
  const path = first?.path?.length ? first.path.join(".") : "(根)";
  throw new IpcSchemaError(
    cmd,
    path,
    first?.message ?? "符合契约",
    data === undefined ? "undefined" : data === null ? "null" : typeof data,
  );
}
