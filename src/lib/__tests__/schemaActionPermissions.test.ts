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

  it("插件来源给受限面：只开 dynamic_ui_*，其余维度显式为空", () => {
    const perms = permissionsForSchemaOrigin("plugin");
    expect(perms).toEqual({
      commands: [...PLUGIN_SCHEMA_ALLOWED_COMMANDS],
      events: [],
      storeRead: [],
      storeWrite: [],
      navigate: [],
    });
    expect(perms!.commands).toEqual(["dynamic_ui_*"]);
  });

  it("判不出来的来源一律按不可信（fail-closed）", () => {
    for (const origin of [undefined, null, "", "PLUGIN", "unknown-origin", 42]) {
      expect(isTrustedSchemaOrigin(origin)).toBe(false);
      expect(permissionsForSchemaOrigin(origin)).toEqual(createPluginSchemaPermissions());
    }
  });

  it("权限面与 isWildcardMatch 的组合语义：放行 dynamic_ui_*，拒绝其余命令", () => {
    const commands = permissionsForSchemaOrigin("plugin")!.commands!;
    expect(isWildcardMatch("dynamic_ui_list_pins", commands)).toBe(true);
    expect(isWildcardMatch("dynamic_ui_get_dynamic_ui_form_data", commands)).toBe(true);
    expect(isWildcardMatch("delete_scheduled_task", commands)).toBe(false);
    // 前缀必须带尾下划线：只共享 "dynamic_ui" 字面不能算命中
    expect(isWildcardMatch("dynamic_uix", commands)).toBe(false);
  });

  it("非持久化 Schema（无记录）维持现状", () => {
    expect(permissionsForSchemaRecord(null)).toBeUndefined();
    expect(permissionsForSchemaRecord(undefined)).toBeUndefined();
  });
});

describe("按 schemaId 兜底解析", () => {
  it("currentSchema 命中且为插件来源 ⇒ 受限", () => {
    h.state.currentSchema = record("s-1", "plugin");
    expect(resolveSchemaActionPermissions("s-1")?.commands).toEqual(["dynamic_ui_*"]);
  });

  it("currentSchema 是别的 id ⇒ 不误判（继续找 schemas 列表）", () => {
    h.state.currentSchema = record("other", "plugin");
    expect(resolveSchemaActionPermissions("s-1")).toBeUndefined();
  });

  it("schemas 列表命中插件来源 ⇒ 受限", () => {
    h.state.schemas = [record("s-1", "plugin")];
    expect(resolveSchemaActionPermissions("s-1")?.commands).toEqual(["dynamic_ui_*"]);
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
