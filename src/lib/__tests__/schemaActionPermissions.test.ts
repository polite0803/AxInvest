// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 来源 → 动作权限面的判据单测（纯函数 + 按 id 兜底解析）。
 * 渲染链上的正负控见 `src/components/dynamicUI/__tests__/pluginSchemaActionPermissions.test.tsx`。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createPluginSchemaPermissions,
  isTrustedSchemaOrigin,
  permissionsForSchemaOrigin,
  permissionsForSchemaRecord,
  PLUGIN_SCHEMA_ALLOWED_COMMANDS,
  resolveSchemaActionPermissions,
} from "@/lib/dynamicUI/schemaActionPermissions";
import { isWildcardMatch } from "@/lib/skillPermissions";
import type { DynamicUISchemaRecord } from "@/types";

const h = vi.hoisted(() => ({
  state: { currentSchema: null as unknown, schemas: [] as unknown[] },
}));

vi.mock("@/stores/feature/dynamicUIStore", () => ({
  useDynamicUIStore: { getState: () => h.state },
}));

const record = (
  id: string,
  origin: string,
): DynamicUISchemaRecord => ({ id, origin } as unknown as DynamicUISchemaRecord);

beforeEach(() => {
  h.state.currentSchema = null;
  h.state.schemas = [];
});

describe("来源判定", () => {
  it("可信来源（builtin / user / ai）不注入权限面", () => {
    for (const origin of ["builtin", "user", "ai"]) {
      expect(isTrustedSchemaOrigin(origin)).toBe(true);
      expect(permissionsForSchemaOrigin(origin)).toBeUndefined();
    }
  });

  it("插件来源给受限面：只开四条命令，其余维度显式为空", () => {
    const perms = permissionsForSchemaOrigin("plugin");
    expect(perms).toEqual({
      commands: [...PLUGIN_SCHEMA_ALLOWED_COMMANDS],
      events: [],
      storeRead: [],
      storeWrite: [],
      navigate: [],
    });
    expect(perms!.commands).toEqual([
      "get_dynamic_ui_schema",
      "get_dynamic_ui_form_data",
      "save_dynamic_ui_form_data",
      "delete_dynamic_ui_form_data",
    ]);
  });

  it("判不出来的来源一律按不可信（fail-closed）", () => {
    for (const origin of [undefined, null, "", "PLUGIN", "unknown-origin", 42]) {
      expect(isTrustedSchemaOrigin(origin)).toBe(false);
      expect(permissionsForSchemaOrigin(origin)).toEqual(createPluginSchemaPermissions());
    }
  });

  it("权限面与 isWildcardMatch 的组合语义：放行四条自身命令，拒绝 Schema CRUD", () => {
    const commands = permissionsForSchemaOrigin("plugin")!.commands!;
    // 正控：四条放行的命令逐条命中（漏一条即本页无法持久化自身状态）
    for (const cmd of PLUGIN_SCHEMA_ALLOWED_COMMANDS) {
      expect(isWildcardMatch(cmd, commands)).toBe(true);
    }
    // 负控：提权链上的三条写命令 + 其余 dynamic_ui_* 一律拒绝。
    // create 把 origin 硬写成 "user"（可信）⇒ 放行它等于让插件自造一个不受限的 Schema；
    // update / delete 不按 owner_id 限定目标 ⇒ 放行它等于能改写/删除他人 Schema。
    for (
      const cmd of [
        "create_dynamic_ui_schema",
        "update_dynamic_ui_schema",
        "delete_dynamic_ui_schema",
        "restore_dynamic_ui_schema_version",
        "edit_dynamic_ui_schema_nl",
        "generate_dynamic_ui_schema_nl",
        "pin_dynamic_ui_schema",
        "unpin_dynamic_ui_schema",
        "list_dynamic_ui_schemas",
      ]
    ) {
      expect(isWildcardMatch(cmd, commands)).toBe(false);
    }
    // 非 dynamic_ui 族本来就不在面上
    expect(isWildcardMatch("delete_scheduled_task", commands)).toBe(false);
    // 无通配 ⇒ 前缀相似不再算命中（旧的 "dynamic_ui_*" 形态会把上面全放行）
    expect(isWildcardMatch("dynamic_ui_anything_else", commands)).toBe(false);
  });

  it("非持久化 Schema（无记录）维持现状", () => {
    expect(permissionsForSchemaRecord(null)).toBeUndefined();
    expect(permissionsForSchemaRecord(undefined)).toBeUndefined();
  });
});

describe("按 schemaId 兜底解析", () => {
  it("currentSchema 命中且为插件来源 ⇒ 受限", () => {
    h.state.currentSchema = record("s-1", "plugin");
    expect(resolveSchemaActionPermissions("s-1")?.commands).toEqual([...PLUGIN_SCHEMA_ALLOWED_COMMANDS]);
  });

  it("currentSchema 是别的 id ⇒ 不误判（继续找 schemas 列表）", () => {
    h.state.currentSchema = record("other", "plugin");
    expect(resolveSchemaActionPermissions("s-1")).toBeUndefined();
  });

  it("schemas 列表命中插件来源 ⇒ 受限", () => {
    h.state.schemas = [record("s-1", "plugin")];
    expect(resolveSchemaActionPermissions("s-1")?.commands).toEqual([...PLUGIN_SCHEMA_ALLOWED_COMMANDS]);
  });

  it("列表命中用户来源 ⇒ 不收紧", () => {
    h.state.schemas = [record("s-1", "user")];
    expect(resolveSchemaActionPermissions("s-1")).toBeUndefined();
  });

  it("无 schemaId / 未命中任何记录 ⇒ undefined", () => {
    expect(resolveSchemaActionPermissions(null)).toBeUndefined();
    expect(resolveSchemaActionPermissions("missing")).toBeUndefined();
  });
});
