// SPDX-License-Identifier: AGPL-3.0-only

/**
 * dev mock 的 DynamicUI 归属闸门（缺陷批 #186）。
 *
 * 判据来源是后端锚点 `src-tauri/src/commands/dynamic_ui.rs`（提交 `43803e5ac`）：
 * - `TRUSTED_SCHEMA_ORIGINS` = `builtin` / `user` / `ai`，白名单取反 ⇒ fail-closed；
 * - `load_schema_for_command` = 「不存在 ⇒ `DYNAMIC_UI_SCHEMA_NOT_FOUND`」+「非可信 origin ⇒ 拒」，
 *   update / delete / form_data / pin / 回滚 全部先过它，再单独挡 `is_builtin`；
 * - `create_dynamic_ui_schema` **硬写** `origin = "user"`、`owner_id = ""`，
 *   请求体的 `origin` / `ownerId` 被静默丢弃。
 *
 * 本文件锁的是「mock 不比真机宽松」这条不变量 —— 宽松的后果不是测试红，而是
 * `npm run dev` 下能改内置、能删插件拥有的 Schema，这类缺陷在开发期完全看不见。
 *
 * ⚠ 参数形状说明：`handleCommand` 会把入参**递归** snake_case（见其 `convertToSnakeCase`），
 * 故这里只用无大小写歧义的键（`id` / `origin` / `ownerId`→`owner_id` 由 mock 侧读取，
 * 本批后 mock 已不再读这两个入参）。`schemaJson` 落库受该转换影响是**既存缺陷**，与判据无关，
 * 本文件刻意不断言它。
 */

import { beforeEach, describe, expect, it } from "vitest";

import { TRUSTED_SCHEMA_ORIGINS } from "@/types";
import type { DynamicUISchemaRecord } from "@/types";
import { handleCommand } from "../browserMock";

const SCHEMAS_KEY = "axagent.mock.dynamicUI.schemas";
const VERSIONS_KEY = "axagent.mock.dynamicUI.versions";

function makeRecord(patch: Partial<DynamicUISchemaRecord> = {}): DynamicUISchemaRecord {
  return {
    id: "s-1",
    title: "测试页面",
    description: "",
    schemaJson: "{}",
    category: "custom",
    tags: [],
    version: "1.0.0",
    isBuiltin: false,
    origin: "user",
    ownerId: "",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...patch,
  };
}

/** 直接写 localStorage 造行：create 通道已无法造出插件 / 内置来源的行（这正是要锁的行为）。 */
function seed(...records: DynamicUISchemaRecord[]): void {
  localStorage.setItem(SCHEMAS_KEY, JSON.stringify(records));
}

/**
 * 造一行「来源在词表之外」的记录（空串 / 拼错 / 大小写变体）。
 * 走 `unknown` 断言是刻意的：TS 类型不许这些值，但真机 localStorage / DB 里就是可能有，
 * fail-closed 判据必须拦住它们而不是假设它们不存在。
 */
function seedWithOrigin(origin: string): void {
  seed(makeRecord({ origin: origin as unknown as DynamicUISchemaRecord["origin"] }));
}

function stored(): DynamicUISchemaRecord[] {
  return JSON.parse(localStorage.getItem(SCHEMAS_KEY) ?? "[]") as DynamicUISchemaRecord[];
}

async function rejectMessage(fn: () => Promise<unknown>): Promise<string> {
  const err = await fn().then(
    () => null,
    (e: unknown) => e as Error,
  );
  expect(err, "该调用必须被拒绝").toBeInstanceOf(Error);
  return (err as Error).message;
}

beforeEach(() => {
  localStorage.clear();
});

describe("可信来源词表", () => {
  it("与后端 dynamic_ui.rs 的 TRUSTED_SCHEMA_ORIGINS 同集合（三条，不含 plugin）", () => {
    expect([...TRUSTED_SCHEMA_ORIGINS].sort()).toEqual(["ai", "builtin", "user"]);
    expect(TRUSTED_SCHEMA_ORIGINS.has("plugin")).toBe(false);
    expect(TRUSTED_SCHEMA_ORIGINS.has("")).toBe(false);
  });
});

describe("create：origin 是宿主写的信任锚，入参一律不采信", () => {
  it('传 origin="ai" 仍落库 user，ownerId 恒空串', async () => {
    const created = await handleCommand<DynamicUISchemaRecord>("create_dynamic_ui_schema", {
      req: {
        title: "AI 生成页",
        description: "",
        category: "custom",
        tags: [],
        origin: "ai",
      },
    });

    expect(created.origin).toBe("user");
    expect(created.ownerId).toBe("");
    expect(created.isBuiltin).toBe(false);
    expect(stored()[0]?.origin).toBe("user");
  });

  it('传 origin="plugin" + ownerId 也造不出插件行（后端同形：请求体那两个字段被丢弃）', async () => {
    const created = await handleCommand<DynamicUISchemaRecord>("create_dynamic_ui_schema", {
      req: {
        title: "伪装插件页",
        description: "",
        category: "custom",
        tags: [],
        origin: "plugin",
        ownerId: "vendor.plugin",
      },
    });

    expect(created.origin).toBe("user");
    expect(created.ownerId).toBe("");
    expect(stored()[0]?.ownerId).toBe("");
  });
});

describe("update：归属闸门 + 内置守卫", () => {
  it("插件拥有的行拒绝改写，文案带出实际 origin，且行未被修改", async () => {
    seed(makeRecord({ origin: "plugin", ownerId: "vendor.plugin" }));

    const msg = await rejectMessage(() =>
      handleCommand("update_dynamic_ui_schema", { id: "s-1", req: { title: "恶意标题" } })
    );

    expect(msg).toContain("拒绝更新");
    expect(msg).toContain('origin = "plugin"');
    expect(stored()[0]?.title).toBe("测试页面");
  });

  it("未知 / 空 origin 一律拒（fail-closed，与后端 plugin_owned_denied 同判据）", async () => {
    for (const origin of ["", "bogus", "Plugin", "PLUGIN", " user"]) {
      seedWithOrigin(origin);
      const msg = await rejectMessage(() =>
        handleCommand("update_dynamic_ui_schema", { id: "s-1", req: { title: "越权" } })
      );
      expect(msg, `origin = ${JSON.stringify(origin)} 必须拒绝`).toContain("拒绝更新");
      expect(stored()[0]?.title).toBe("测试页面");
    }
  });

  it("内置行过了归属闸门仍被 is_builtin 挡：结构化错误码与后端同码", async () => {
    seed(makeRecord({ isBuiltin: true }));

    const msg = await rejectMessage(() =>
      handleCommand("update_dynamic_ui_schema", { id: "s-1", req: { title: "改内置" } })
    );

    expect(JSON.parse(msg)).toMatchObject({ code: "DYNAMIC_UI_BUILTIN_NOT_MODIFIABLE" });
    expect(stored()[0]?.title).toBe("测试页面");
  });

  it("不存在的 id 报 DYNAMIC_UI_SCHEMA_NOT_FOUND（后端不再放行任意 id）", async () => {
    seed(makeRecord({ id: "other" }));

    const msg = await rejectMessage(() =>
      handleCommand("update_dynamic_ui_schema", { id: "missing", req: { title: "x" } })
    );

    expect(JSON.parse(msg)).toMatchObject({ code: "DYNAMIC_UI_SCHEMA_NOT_FOUND" });
  });

  it("正控：可信来源（user / ai / builtin 非内置位）正常改写并自增版本", async () => {
    for (const origin of ["user", "ai"] as const) {
      seed(makeRecord({ origin }));
      const updated = await handleCommand<DynamicUISchemaRecord>("update_dynamic_ui_schema", {
        id: "s-1",
        req: { title: `改过-${origin}` },
      });
      expect(updated.title).toBe(`改过-${origin}`);
      expect(updated.version).toBe("1.0.1");
      expect(stored()[0]?.version).toBe("1.0.1");
    }
  });
});

describe("delete：归属闸门 + 内置守卫 + 级联清理", () => {
  it("插件拥有的行拒删，行仍在", async () => {
    seed(makeRecord({ origin: "plugin", ownerId: "vendor.plugin" }));

    const msg = await rejectMessage(() => handleCommand("delete_dynamic_ui_schema", { id: "s-1" }));

    expect(msg).toContain("拒绝删除");
    expect(msg).toContain('origin = "plugin"');
    expect(stored()).toHaveLength(1);
  });

  it("未知 origin 的行同样拒删（白名单取反，不给未知值放行）", async () => {
    seedWithOrigin("");

    await expect(handleCommand("delete_dynamic_ui_schema", { id: "s-1" })).rejects.toThrow("拒绝删除");
    expect(stored()).toHaveLength(1);
  });

  it("内置行拒删：DYNAMIC_UI_BUILTIN_NOT_DELETABLE", async () => {
    seed(makeRecord({ isBuiltin: true }));

    const msg = await rejectMessage(() => handleCommand("delete_dynamic_ui_schema", { id: "s-1" }));

    expect(JSON.parse(msg)).toMatchObject({ code: "DYNAMIC_UI_BUILTIN_NOT_DELETABLE" });
    expect(stored()).toHaveLength(1);
  });

  it("不存在的 id 报错而非静默成功（改前 mock 是静默 filter，真机拒）", async () => {
    seed(makeRecord({ id: "other" }));

    const msg = await rejectMessage(() => handleCommand("delete_dynamic_ui_schema", { id: "missing" }));

    expect(JSON.parse(msg)).toMatchObject({ code: "DYNAMIC_UI_SCHEMA_NOT_FOUND" });
  });

  it("正控：可信行删得掉，版本历史同步清理", async () => {
    seed(makeRecord());
    localStorage.setItem(
      VERSIONS_KEY,
      JSON.stringify([{ id: 1, schemaId: "s-1", version: "1.0.0", changeLog: "初始版本" }]),
    );

    await expect(handleCommand("delete_dynamic_ui_schema", { id: "s-1" })).resolves.toBeUndefined();

    expect(stored()).toHaveLength(0);
    expect(JSON.parse(localStorage.getItem(VERSIONS_KEY) ?? "[]")).toHaveLength(0);
  });
});
