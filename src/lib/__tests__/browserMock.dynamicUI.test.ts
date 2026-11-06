// SPDX-License-Identifier: AGPL-3.0-only

/**
 * dev mock 的 DynamicUI 归属闸门（缺陷批 #186 / #188）。
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
 * 那两个入参 mock 已不再读）。
 * **#188 第 2 件改了这条前提**：嵌套 `req` 的 camelCase 字段（`schemaJson` / `formDataJson` /
 * `instanceKey` / `changeLog` / `schemaId`）现在由 `readMockReqField` 两种键形都读，
 * ⇒ 底部新增的 `describe("req 嵌套入参…")` 就是钉这条的，改前那批用例刻意不断言的部分已由它接管。
 */

import { beforeEach, describe, expect, it } from "vitest";

import { TRUSTED_SCHEMA_ORIGINS } from "@/types";
import type {
  DynamicUIFormDataRecord,
  DynamicUIPinRecord,
  DynamicUISchemaRecord,
  DynamicUISchemaVersion,
} from "@/types";
import { handleCommand } from "../browserMock";

const SCHEMAS_KEY = "axagent.mock.dynamicUI.schemas";
const VERSIONS_KEY = "axagent.mock.dynamicUI.versions";
const FORM_KEY = "axagent.mock.dynamicUI.formData";
const PINS_KEY = "axagent.mock.dynamicUI.pins";

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

/** 表单数据行夹具：mock 侧按 `(schemaId, instanceKey)` 复合匹配，两条键都必须可断言。 */
function makeFormRecord(patch: Partial<DynamicUIFormDataRecord> = {}): DynamicUIFormDataRecord {
  return {
    id: "f-1",
    schemaId: "s-1",
    formDataJson: '{"secret":true}',
    instanceKey: "__default__",
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...patch,
  };
}

function seedForm(...records: DynamicUIFormDataRecord[]): void {
  localStorage.setItem(FORM_KEY, JSON.stringify(records));
}

function storedForm(): DynamicUIFormDataRecord[] {
  return JSON.parse(localStorage.getItem(FORM_KEY) ?? "[]") as DynamicUIFormDataRecord[];
}

function makePin(patch: Partial<DynamicUIPinRecord> = {}): DynamicUIPinRecord {
  return {
    schemaId: "s-1",
    title: "钉入的页面",
    groupName: "g",
    position: 0,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...patch,
  };
}

function seedPins(...records: DynamicUIPinRecord[]): void {
  localStorage.setItem(PINS_KEY, JSON.stringify(records));
}

function storedPins(): DynamicUIPinRecord[] {
  return JSON.parse(localStorage.getItem(PINS_KEY) ?? "[]") as DynamicUIPinRecord[];
}

/** 版本快照夹具：`id` 是数字主键，`schemaId` 用来造「跨 Schema 的版本」这一被后端拒绝的情形。 */
function makeVersion(patch: Partial<DynamicUISchemaVersion> = {}): DynamicUISchemaVersion {
  return {
    id: 11,
    schemaId: "s-1",
    version: "1.0.0",
    title: "历史标题",
    description: "历史说明",
    schemaJson: '{"history":true}',
    category: "custom",
    tags: [],
    changeLog: "初始版本",
    createdAt: 1,
    ...patch,
  };
}

function seedVersions(...records: DynamicUISchemaVersion[]): void {
  localStorage.setItem(VERSIONS_KEY, JSON.stringify(records));
}

function storedVersions(): DynamicUISchemaVersion[] {
  return JSON.parse(localStorage.getItem(VERSIONS_KEY) ?? "[]") as DynamicUISchemaVersion[];
}

/** 结构化错误码断言用：拒绝串不是 JSON（插件归属那条是裸中文串）时返回 `null`。 */
function errorCodeOf(message: string): string | null {
  try {
    return (JSON.parse(message) as { code?: string }).code ?? null;
  } catch {
    return null;
  }
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

// ══════════════════════════════════════════════════════════════════════
// 以下四组是 #188 的「闸门未对齐」那一半：后端已过闸而 mock 未过闸的命令。
// 判据锚点逐条写在用例名里（行号取自 `grep -n` 的改前/改后读数）。
// ══════════════════════════════════════════════════════════════════════

describe("表单数据 save / get / delete：补归属闸门（后端 :689 / :739 / :759）", () => {
  it("save：宿主属插件 ⇒ 拒且零落库（负控；后端 dynamic_ui.rs:689）", async () => {
    seed(makeRecord({ origin: "plugin", ownerId: "vendor.plugin" }));
    seedForm();

    const msg = await rejectMessage(() =>
      handleCommand("save_dynamic_ui_form_data", {
        req: { schemaId: "s-1", formDataJson: '{"x":1}' },
      })
    );

    expect(msg).toContain("拒绝保存表单数据");
    expect(msg).toContain('origin = "plugin"');
    expect(storedForm()).toEqual([]);
  });

  it("save：宿主不存在 ⇒ DYNAMIC_UI_SCHEMA_NOT_FOUND，且不写入（后端不再放行任意 schemaId）", async () => {
    seed(makeRecord({ id: "other" }));
    seedForm();

    const msg = await rejectMessage(() =>
      handleCommand("save_dynamic_ui_form_data", {
        req: { schemaId: "missing", formDataJson: '{"x":1}' },
      })
    );

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_SCHEMA_NOT_FOUND");
    expect(storedForm()).toEqual([]);
  });

  it("save：宿主 origin 未知（空串）⇒ 拒（fail-closed，与 plugin_owned_denied 同判据）", async () => {
    // 走 `seedWithOrigin`（内部按 unknown 断言）：TS 类型不许空串，但真机 localStorage 里就是可能有
    seedWithOrigin("");

    const msg = await rejectMessage(() =>
      handleCommand("save_dynamic_ui_form_data", {
        req: { schemaId: "s-1", formDataJson: "{}" },
      })
    );

    expect(msg).toContain("拒绝保存表单数据");
    expect(storedForm()).toEqual([]);
  });

  it("save：内置宿主**放行** —— 后端 form_data 只过归属闸门、不挡 is_builtin，mock 不许更严", async () => {
    seed(makeRecord({ origin: "builtin", isBuiltin: true }));

    const record = await handleCommand<DynamicUIFormDataRecord>("save_dynamic_ui_form_data", {
      req: { schemaId: "s-1", formDataJson: '{"ok":1}' },
    });

    expect(record.schemaId).toBe("s-1");
    expect(storedForm()).toHaveLength(1);
  });

  it("get：宿主属插件 ⇒ 拒，且拒绝串里不回显已存表单值（后端 :739，闸门在查询之前）", async () => {
    seed(makeRecord({ origin: "plugin" }));
    seedForm(makeFormRecord({ formDataJson: '{"leaked":"secret"}' }));

    const msg = await rejectMessage(() => handleCommand("get_dynamic_ui_form_data", { schemaId: "s-1" }));

    expect(msg).toContain("拒绝读取表单数据");
    expect(msg).not.toContain("leaked");
  });

  it("get：宿主不存在 ⇒ 结构化 NOT_FOUND（改前 mock 静默返回 null，真机拒）", async () => {
    seed(makeRecord({ id: "other" }));

    const msg = await rejectMessage(() => handleCommand("get_dynamic_ui_form_data", { schemaId: "missing" }));

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_SCHEMA_NOT_FOUND");
  });

  it("get：正控 —— 可信宿主读回自身表单值", async () => {
    seed(makeRecord({ origin: "ai" }));
    seedForm(makeFormRecord({ formDataJson: '{"v":7}' }));

    const record = await handleCommand<DynamicUIFormDataRecord>("get_dynamic_ui_form_data", {
      schemaId: "s-1",
    });

    expect(record?.formDataJson).toBe('{"v":7}');
  });

  it("delete：宿主属插件 ⇒ 拒且表单行仍在（后端 :759）", async () => {
    seed(makeRecord({ origin: "plugin" }));
    seedForm(makeFormRecord());

    const msg = await rejectMessage(() => handleCommand("delete_dynamic_ui_form_data", { schemaId: "s-1" }));

    expect(msg).toContain("拒绝删除表单数据");
    expect(storedForm()).toHaveLength(1);
  });

  it("delete：宿主不存在 ⇒ 结构化 NOT_FOUND（改前 mock 无条件 filter，真机拒）", async () => {
    seed(makeRecord({ id: "other" }));
    seedForm(makeFormRecord({ schemaId: "missing" }));

    const msg = await rejectMessage(() => handleCommand("delete_dynamic_ui_form_data", { schemaId: "missing" }));

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_SCHEMA_NOT_FOUND");
    expect(storedForm()).toHaveLength(1);
  });

  it("delete：正控 —— 可信宿主的表单行删得掉", async () => {
    seed(makeRecord());
    seedForm(makeFormRecord());

    await expect(handleCommand("delete_dynamic_ui_form_data", { schemaId: "s-1" })).resolves.toBeUndefined();

    expect(storedForm()).toHaveLength(0);
  });
});

describe("钉入 pin / unpin：补归属闸门（后端 :1070，unpin 例外见 :1130）", () => {
  it("pin：宿主属插件 ⇒ 拒且不产生钉入（负控）", async () => {
    seed(makeRecord({ origin: "plugin", ownerId: "vendor.plugin" }));
    seedPins();

    const msg = await rejectMessage(() =>
      handleCommand("pin_dynamic_ui_schema", { schemaId: "s-1", title: "t", groupName: "g" })
    );

    expect(msg).toContain("拒绝钉入");
    expect(msg).toContain('origin = "plugin"');
    expect(storedPins()).toEqual([]);
  });

  it("pin：宿主不存在 ⇒ 结构化 NOT_FOUND（后端同判据：挡悬空导航项）", async () => {
    seed(makeRecord({ id: "other" }));
    seedPins();

    const msg = await rejectMessage(() =>
      handleCommand("pin_dynamic_ui_schema", { schemaId: "missing", title: "t", groupName: "g" })
    );

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_SCHEMA_NOT_FOUND");
    expect(storedPins()).toEqual([]);
  });

  it("pin：正控 —— 可信宿主钉得进，position 采纳入参", async () => {
    seed(makeRecord({ origin: "user" }));

    const pin = await handleCommand<DynamicUIPinRecord>("pin_dynamic_ui_schema", {
      schemaId: "s-1",
      title: "t",
      groupName: "g",
      position: 3,
    });

    expect(pin.schemaId).toBe("s-1");
    expect(storedPins()).toHaveLength(1);
    expect(storedPins()[0]?.position).toBe(3);
  });

  it("unpin：宿主属插件 ⇒ 拒且钉入行仍在（负控）", async () => {
    seed(makeRecord({ origin: "plugin" }));
    seedPins(makePin());

    const msg = await rejectMessage(() => handleCommand("unpin_dynamic_ui_schema", { schemaId: "s-1" }));

    expect(msg).toContain("拒绝取消钉入");
    expect(storedPins()).toHaveLength(1);
  });

  it("unpin：宿主**不存在**时放行 —— 这是后端 :1130 明写的唯一例外，不许顺手收紧", async () => {
    // 后端理由（原文注释）：delete 不级联 pins 表，若这里也要求宿主存在，
    // 被删 schema 遗留的悬空钉入就永远清不掉。
    seed(makeRecord({ id: "other" }));
    seedPins(makePin({ schemaId: "dangling" }));

    await expect(handleCommand("unpin_dynamic_ui_schema", { schemaId: "dangling" })).resolves.toBeUndefined();

    expect(storedPins()).toHaveLength(0);
  });

  it("unpin：宿主存在且 origin 未知（大小写变体不在白名单 ⇒ 不可信）⇒ 拒", async () => {
    seedWithOrigin("Plugin");
    seedPins(makePin());

    const msg = await rejectMessage(() => handleCommand("unpin_dynamic_ui_schema", { schemaId: "s-1" }));

    expect(msg).toContain("拒绝取消钉入");
    expect(storedPins()).toHaveLength(1);
  });

  it("unpin：正控 —— 可信宿主的钉入取消得掉", async () => {
    seed(makeRecord({ origin: "ai" }));
    seedPins(makePin());

    await expect(handleCommand("unpin_dynamic_ui_schema", { schemaId: "s-1" })).resolves.toBeUndefined();

    expect(storedPins()).toHaveLength(0);
  });
});

describe("回滚 restore：闸门顺序与错误文案对齐后端（:624 → :626 → :630 → :637）", () => {
  it("插件宿主 ⇒ 拒且 schema 未被改写（负控；改前 mock 直接回滚成功）", async () => {
    seed(makeRecord({ origin: "plugin", schemaJson: '{"live":1}' }));
    seedVersions(makeVersion());

    const msg = await rejectMessage(() =>
      handleCommand("restore_dynamic_ui_schema_version", { schemaId: "s-1", versionId: 11 })
    );

    expect(msg).toContain("拒绝回滚");
    expect(stored()[0]?.schemaJson).toBe('{"live":1}');
  });

  it("宿主不存在 ⇒ 结构化 NOT_FOUND（先过闸门，再看版本表）", async () => {
    seed(makeRecord({ id: "other" }));
    seedVersions(makeVersion({ schemaId: "other" }));

    const msg = await rejectMessage(() =>
      handleCommand("restore_dynamic_ui_schema_version", { schemaId: "missing", versionId: 11 })
    );

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_SCHEMA_NOT_FOUND");
  });

  it("内置宿主 ⇒ DYNAMIC_UI_BUILTIN_NOT_ROLLBACK（后端 :626，改前 mock 无此守卫）", async () => {
    seed(makeRecord({ isBuiltin: true }));
    seedVersions(makeVersion());

    const msg = await rejectMessage(() =>
      handleCommand("restore_dynamic_ui_schema_version", { schemaId: "s-1", versionId: 11 })
    );

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_BUILTIN_NOT_ROLLBACK");
  });

  it("版本不存在 ⇒ 与后端逐字同文案「版本 {id} 不存在」（闸门先于版本查找）", async () => {
    seed(makeRecord());
    seedVersions(makeVersion());

    const msg = await rejectMessage(() =>
      handleCommand("restore_dynamic_ui_schema_version", { schemaId: "s-1", versionId: 999 })
    );

    expect(msg).toBe("版本 999 不存在");
  });

  it("跨 Schema 的版本 ⇒ 后端 :637 同文案，且不落库（改前把两种情形混成 Version not found）", async () => {
    seed(makeRecord({ schemaJson: '{"live":1}' }));
    seedVersions(makeVersion({ schemaId: "someone-else" }));

    const msg = await rejectMessage(() =>
      handleCommand("restore_dynamic_ui_schema_version", { schemaId: "s-1", versionId: 11 })
    );

    expect(msg).toBe("版本 11 不属于 Schema s-1，拒绝回滚");
    expect(stored()[0]?.schemaJson).toBe('{"live":1}');
  });

  it("正控：可信非内置宿主回滚生效，live 版本沿用目标版本号（不 bump）", async () => {
    seed(makeRecord({ schemaJson: '{"live":1}', version: "1.2.0" }));
    seedVersions(makeVersion());

    const restored = await handleCommand<DynamicUISchemaRecord>("restore_dynamic_ui_schema_version", {
      schemaId: "s-1",
      versionId: 11,
    });

    expect(restored.schemaJson).toBe('{"history":true}');
    expect(restored.title).toBe("历史标题");
    expect(stored()[0]?.version).toBe("1.0.0");
  });

  it("顺序自证：内置 + 版本不存在 ⇒ 拿到内置码（证明 is_builtin 在版本查找之前判）", async () => {
    seed(makeRecord({ isBuiltin: true }));
    seedVersions();

    const msg = await rejectMessage(() =>
      handleCommand("restore_dynamic_ui_schema_version", { schemaId: "s-1", versionId: 999 })
    );

    expect(errorCodeOf(msg)).toBe("DYNAMIC_UI_BUILTIN_NOT_ROLLBACK");
  });
});

describe("req 嵌套入参的键形兼容：schemaJson / formDataJson 真正落库（#188 第 2 件）", () => {
  it("create：req.schemaJson 经递归 snake 化后仍写入记录，而不是静默丢键", async () => {
    const payload = '{"version":"1.0","id":"root","type":"Text"}';

    const created = await handleCommand<DynamicUISchemaRecord>("create_dynamic_ui_schema", {
      req: {
        title: "带内容的页面",
        description: "",
        category: "custom",
        tags: [],
        schemaJson: payload,
      },
    });

    // 返回体与落库行**都**必须是入参原文（改前恒 `undefined` ⇒ JSON.stringify 直接丢键）
    expect(created.schemaJson).toBe(payload);
    expect(stored()[0]?.schemaJson).toBe(payload);
  });

  it("create：初始版本快照同样带上 schemaJson（快照取自落库行）", async () => {
    const payload = '{"version":"1.0","id":"root","type":"Column"}';

    const created = await handleCommand<DynamicUISchemaRecord>("create_dynamic_ui_schema", {
      req: { title: "t", description: "", category: "custom", tags: [], schemaJson: payload },
    });

    const versions = storedVersions();
    expect(versions).toHaveLength(1);
    expect(versions[0]?.schemaJson).toBe(payload);
    expect(versions[0]?.schemaId).toBe(created.id);
  });

  it("update：新 schemaJson 覆盖生效（改前恒落回旧值 ⇒ dev 改内容永不生效）", async () => {
    seed(makeRecord({ schemaJson: '{"old":1}' }));

    const updated = await handleCommand<DynamicUISchemaRecord>("update_dynamic_ui_schema", {
      id: "s-1",
      req: { schemaJson: '{"new":2}' },
    });

    expect(updated.schemaJson).toBe('{"new":2}');
    expect(stored()[0]?.schemaJson).toBe('{"new":2}');
  });

  it("update：不传 schemaJson ⇒ 保留旧值（与后端 Option<String> 语义一致，不许写空串）", async () => {
    seed(makeRecord({ schemaJson: '{"old":1}' }));

    const updated = await handleCommand<DynamicUISchemaRecord>("update_dynamic_ui_schema", {
      id: "s-1",
      req: { title: "只改标题" },
    });

    expect(updated.schemaJson).toBe('{"old":1}');
    expect(stored()[0]?.schemaJson).toBe('{"old":1}');
  });

  it("update：req.changeLog 同样两种键形都读，版本记录带上说明", async () => {
    seed(makeRecord());

    await handleCommand("update_dynamic_ui_schema", {
      id: "s-1",
      req: { schemaJson: '{"x":1}', changeLog: "这次改了内容" },
    });

    const versions = storedVersions();
    expect(versions).toHaveLength(1);
    expect(versions[0]?.changeLog).toBe("这次改了内容");
  });

  it("save：req.schemaId / formDataJson / instanceKey 三键全部按宿主落库并可读回", async () => {
    seed(makeRecord());

    await handleCommand("save_dynamic_ui_form_data", {
      req: { schemaId: "s-1", formDataJson: '{"a":1}', instanceKey: "k1" },
    });

    const rows = storedForm();
    expect(rows).toHaveLength(1);
    expect(rows[0]?.schemaId).toBe("s-1");
    expect(rows[0]?.formDataJson).toBe('{"a":1}');
    expect(rows[0]?.instanceKey).toBe("k1");

    const back = await handleCommand<DynamicUIFormDataRecord>("get_dynamic_ui_form_data", {
      schemaId: "s-1",
      instanceKey: "k1",
    });
    expect(back?.formDataJson).toBe('{"a":1}');
  });

  it("save：同一 (宿主, instanceKey) 重复保存是覆盖，不同 instanceKey 并存", async () => {
    seed(makeRecord());

    await handleCommand("save_dynamic_ui_form_data", {
      req: { schemaId: "s-1", formDataJson: '{"v":1}', instanceKey: "k1" },
    });
    await handleCommand("save_dynamic_ui_form_data", {
      req: { schemaId: "s-1", formDataJson: '{"v":2}', instanceKey: "k1" },
    });
    await handleCommand("save_dynamic_ui_form_data", {
      req: { schemaId: "s-1", formDataJson: '{"v":1}', instanceKey: "k2" },
    });

    const rows = storedForm();
    expect(rows).toHaveLength(2);
    expect(rows.find((r) => r.instanceKey === "k1")?.formDataJson).toBe('{"v":2}');
  });
});
