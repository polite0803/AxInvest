// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 浏览器模式（`npm run dev`，无 Tauri 后端）下 `Settings → 数据库` 卡片的 mock 契约。
 *
 * ## 这个用例钉的是哪条崩溃链
 *
 * `get_schema_status` / `repair_schema` 原本都**没有桩**，于是落到 `browserMock.ts`
 * default 分支（`get_*` → `{}`，其余 → `undefined`），出口再经 `convertToCamelCase`。
 * 组件侧 `DatabaseSettings.tsx` 因此：
 *
 * 1. `schemaStatus ? (…)` —— `{}` 是 truthy ⇒ 进入「成功」分支；
 * 2. `schemaStatus.pending_apply > 0` 是 `undefined > 0` = false ⇒ Alert 判成 success；
 * 3. 渲染 `schemaStatusUpToDate{actual: undefined, expected: undefined}` ⇒ 显示
 *    「库中 undefined 张表」；
 * 4. `schemaStatus.notes.map(...)` —— `notes` 是 `undefined` ⇒ **抛 TypeError**；
 * 5. 异常被 `ContentArea.tsx` 的 `PageErrorBoundary` 兜住 ⇒ **整个设置页**被错误兜底
 *    替换（不止掉一张卡）。
 *
 * 所以本文件断言的是「mock 必须给出**形态完整**的响应」，而不是「不要崩」——
 * 前者是后者的充分条件，且不依赖组件加兜底（组件刻意不加 `?.` / `?? []`，
 * 见 team-lead 的判据：加兜底只会让卡片静默渲染成 undefined，又一次说谎）。
 *
 * ⚠ 本文件的断言在**修前必然失败**（`{}` 的键集不对、`notes` 不是数组；
 * `repair_schema` 是 resolve 而非 reject），是有意为之的回归防线。
 */

import { describe, expect, it } from "vitest";

import { handleCommand } from "../browserMock";

/** 与 `dao::migrations::SchemaStatus` / `DatabaseSettings.tsx` 的 interface 同集合 */
const SCHEMA_STATUS_KEYS = [
  "dialect",
  "tables_expected",
  "tables_actual",
  "pending_apply",
  "pending_unsupported",
  "pending_manual",
  "advisories",
  "notes",
  "applied_version",
  "latest_version",
  "probe_error",
].sort();

describe("browserMock: get_schema_status", () => {
  it("返回 11 个字段齐全的 SchemaStatus，且 notes 是数组", async () => {
    const status = await handleCommand<Record<string, unknown>>("get_schema_status");

    // ① 11 个字段一个都不能少。
    //    修前：default 分支给 `{}` ⇒ 本条当场失败（`[] !== 11 个键`）。
    expect(Object.keys(status).sort()).toEqual(SCHEMA_STATUS_KEYS);

    // ② `notes` 必须是数组：组件对它直接 `.map`，`undefined` 会抛 TypeError
    //    并顺着 PageErrorBoundary 把整页带走。
    //    修前：`status.notes` 是 `undefined` ⇒ 本条当场失败。
    expect(Array.isArray(status.notes)).toBe(true);
  });

  it("出口不做 camel 转换（否则字段全读成 undefined，显示静默错值）", async () => {
    const status = await handleCommand<Record<string, unknown>>("get_schema_status");

    // 两个命令若漏登记 `SNAKE_CASE_RESPONSE_COMMANDS`，出口会把
    // `tables_expected` 转成 `tablesExpected`、`probe_error` 转成 `probeError`，
    // 组件全程按 snake_case 读 ⇒ 键都在、值全是 `undefined`。
    expect(status).not.toHaveProperty("tablesExpected");
    expect(status).not.toHaveProperty("probeError");
    expect(status).toHaveProperty("tables_expected");
    expect(status).toHaveProperty("probe_error");
  });

  it("probe_error 非 null：浏览器模式不得渲染成「结构已收敛」", async () => {
    const status = await handleCommand<{ probe_error: unknown }>("get_schema_status");

    // 没有真实数据库就没有结构可探测。若这里给 null（并让
    // tables_actual === tables_expected），卡片会显示「结构已收敛」——
    // 又是一张说谎的卡片，正是本轮要根除的东西。
    expect(typeof status.probe_error).toBe("string");
    expect((status.probe_error as string).length).toBeGreaterThan(0);
  });
});

describe("browserMock: repair_schema", () => {
  it("reject —— 「根本没跑」只能用 Err 表达，不能返回空报告", async () => {
    // 修前：`repair_schema` 落到 default 分支返回 `undefined`（resolve）
    // ⇒ 本条当场失败。
    await expect(handleCommand("repair_schema")).rejects.toThrow();
  });

  it("抛的是给人看的原因，不是 JSON 包错误码", async () => {
    // `DatabaseSettings` 直接把 `e.message` 塞进已翻译的
    // `schemaRepairFailed{error}` 句子，**没有**走 `parseBackendError`；
    // 用 `JSON.stringify({code,…})` 会让界面显示一整串 JSON。
    const err = await handleCommand("repair_schema").then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(Error);
    const msg = (err as Error).message;
    expect(msg.length).toBeGreaterThan(0);
    expect(msg.startsWith("{")).toBe(false);
  });
});
