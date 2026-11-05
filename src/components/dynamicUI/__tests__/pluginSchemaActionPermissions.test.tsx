// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件来源 Schema 的动作权限面（fail-closed）—— 正负控。
 *
 * 缺陷形态（2026-11-05 实测）：`SchemaValidator` 只校验 `dataSource.config.endpoint`，
 * 而 `actions[].type === "invoke"` 走 `EventHandlerEngine.executeActions`，构造 `ActionContext`
 * 时不传 `permissions` ⇒ `ActionRouter` 直接 `invoke()`。插件 `schema.json`（`origin = "plugin"`、
 * `ownerId = pluginId`）的 onMount 因此可调任意已注册命令。
 *
 * 本文件从**真实渲染链**起步（DynamicUIStandalone / DynamicUIRenderer + 真 actionRouter +
 * 真 isWildcardMatch），只把 IPC 换成替身，断言打在「IPC 有没有发生」上：
 * - 插件来源：非 `dynamic_ui_*` 命令被拒且**一次 IPC 都不发**；
 * - 插件来源：`dynamic_ui_*` 放行，且参数原样送达；
 * - 用户来源：行为与改动前一致（照发）。
 */

import { act, render, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DynamicUIRenderer } from "@/components/dynamicUI/DynamicUIRenderer";
import { DynamicUIStandalone } from "@/components/dynamicUI/DynamicUIStandalone";
import { SchemaIdContext } from "@/components/dynamicUI/SchemaIdContext";
import type { DynamicAction, DynamicUISchemaRecord, UISchema } from "@/types";

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  getSchema: vi.fn(),
  loadFormData: vi.fn(),
  saveFormData: vi.fn(),
  /** 兜底解析（resolveSchemaActionPermissions）看到的 store 快照 */
  storeState: { currentSchema: null as unknown, schemas: [] as unknown[] },
}));

vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: (...args: unknown[]) => h.invoke(...args) };
});

// `@/stores` barrel：仅 DynamicUIStandalone 用（整店解构）
vi.mock("@/stores", () => ({
  useDynamicUIStore: () => ({
    getSchema: h.getSchema,
    loadFormData: h.loadFormData,
    saveFormData: h.saveFormData,
  }),
}));

// actionRouter 的兜底解析按具体文件导入 ⇒ 单独 mock，只提供 getState 快照
vi.mock("@/stores/feature/dynamicUIStore", () => ({
  useDynamicUIStore: { getState: () => h.storeState },
}));

// actionRouter 静态导入的四个 store：按真实导入路径 mock，避免拉入重型依赖树
vi.mock("@/stores/domain/conversationStore", () => ({
  useConversationStore: { getState: () => ({ createConversation: vi.fn(), sendMessage: vi.fn() }) },
}));
vi.mock("@/stores/feature/providerStore", () => ({
  useProviderStore: { getState: () => ({ providers: [] }) },
}));
vi.mock("@/stores/feature/settingsStore", () => ({
  useSettingsStore: { getState: () => ({ settings: {} }) },
}));
vi.mock("@/stores/feature/skillExtensionStore", () => ({
  useSkillExtensionStore: { getState: () => ({ getHandler: vi.fn(), skills: [] }) },
}));
vi.mock("@/stores/feature/securityStore", () => ({
  useSecurityStore: (selector: (s: { checkInput: () => { isClean: boolean } }) => unknown) =>
    selector({ checkInput: () => ({ isClean: true }) }),
}));

const SCHEMA_ID = "schema-42";

const invokeAction = (command: string, args?: Record<string, unknown>): DynamicAction => ({
  type: "invoke",
  config: { command, ...(args ? { args } : {}) },
});

const makeSchema = (action: DynamicAction): UISchema =>
  ({
    id: "root",
    version: "1.0",
    type: "Container",
    props: {},
    events: [{ trigger: "onMount", actions: [action] }],
  }) as unknown as UISchema;

function makeRecord(origin: DynamicUISchemaRecord["origin"], action: DynamicAction): DynamicUISchemaRecord {
  return {
    id: SCHEMA_ID,
    title: "plugin page",
    description: "",
    schemaJson: JSON.stringify(makeSchema(action)),
    category: "",
    tags: [],
    version: "1.0",
    isBuiltin: false,
    origin,
    ownerId: origin === "plugin" ? "own-pack" : "",
    createdAt: "",
    updatedAt: "",
  };
}

/** 排空「promise 链 + React 状态提交」：executeActions → router.execute →（被拒 | invoke）全在微任务里。 */
async function settle(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

beforeEach(() => {
  h.invoke.mockReset().mockResolvedValue("ipc-ok");
  h.getSchema.mockReset();
  h.loadFormData.mockReset().mockResolvedValue(null);
  h.saveFormData.mockReset().mockResolvedValue({});
  h.storeState.currentSchema = null;
  h.storeState.schemas = [];
});

describe("插件页面容器那条链（DynamicUIStandalone）", () => {
  // `DynamicUIRenderer` 的节点渲染器调 `useNavigate()` ⇒ 必须在 Router 上下文里渲染。
  // 缺了它整棵树会在渲染期抛错：**「被拒」的用例会假绿**（树根本没起来，自然没有 IPC），
  // 而「放行」的用例拿不到调用 —— 首轮实测 4 失败 3 通过，就是这条。
  const renderStandalone = () =>
    render(
      <MemoryRouter>
        <SchemaIdContext.Provider value={{ schemaId: SCHEMA_ID }}>
          <DynamicUIStandalone schemaId={SCHEMA_ID} autosave={false} />
        </SchemaIdContext.Provider>
      </MemoryRouter>,
    );

  it("插件来源 schema 的非 dynamic_ui_* invoke：被拒且不发 IPC", async () => {
    h.getSchema.mockResolvedValue(makeRecord("plugin", invokeAction("delete_scheduled_task", { taskId: "t-1" })));
    renderStandalone();
    await waitFor(() => expect(h.getSchema).toHaveBeenCalledWith(SCHEMA_ID));
    await settle();

    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("插件来源 schema 的 dynamic_ui_* invoke：放行，命令与参数原样送达", async () => {
    const args = { schemaId: SCHEMA_ID, instanceKey: "main" };
    h.getSchema.mockResolvedValue(
      makeRecord("plugin", invokeAction("dynamic_ui_get_dynamic_ui_form_data", args)),
    );
    renderStandalone();

    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(1));
    expect(h.invoke).toHaveBeenCalledWith("dynamic_ui_get_dynamic_ui_form_data", args);
  });

  it("用户来源 schema：维持现状，照常发 IPC（不收紧）", async () => {
    const args = { taskId: "t-1" };
    h.getSchema.mockResolvedValue(makeRecord("user", invokeAction("delete_scheduled_task", args)));
    renderStandalone();

    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(1));
    expect(h.invoke).toHaveBeenCalledWith("delete_scheduled_task", args);
  });

  it("判不出来源（记录缺 origin）按不可信处理：被拒", async () => {
    const record = makeRecord("user", invokeAction("delete_scheduled_task"));
    delete (record as unknown as Record<string, unknown>).origin;
    h.getSchema.mockResolvedValue(record);
    renderStandalone();
    await waitFor(() => expect(h.getSchema).toHaveBeenCalled());
    await settle();

    expect(h.invoke).not.toHaveBeenCalled();
  });
});

describe("受限来源的 update-schema 只能改自己的树", () => {
  const restricted = { commands: ["dynamic_ui_*"], events: [], storeRead: [], storeWrite: [], navigate: [] };

  /** 收集广播事件（渲染树不依赖具体 React 结构，直接钉派发出来的 detail）。 */
  function collectUpdates() {
    const events: CustomEvent[] = [];
    const handler = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("dynamic-ui:schema-update", handler);
    return {
      events,
      stop: () => window.removeEventListener("dynamic-ui:schema-update", handler),
    };
  }

  const updateAction = {
    type: "update-schema",
    schemaId: "root",
    operation: "replace",
    path: "/children/0",
  } as unknown as DynamicAction;

  it("受限来源：派发的更新钉在当前 scope，且标记受限", async () => {
    const { events, stop } = collectUpdates();
    const { executeActions } = await import("@/lib/dynamicUI/EventHandlerEngine");
    await executeActions([updateAction], { permissions: restricted, scope: "dui-self" });
    stop();

    expect(events).toHaveLength(1);
    const detail = events[0].detail as { scope?: string; sourceRestricted?: boolean };
    expect(detail.scope).toBe("dui-self");
    expect(detail.sourceRestricted).toBe(true);
  });

  it("受限来源但没有 scope ⇒ 不派发（缺省 scope 等于广播进可信树）", async () => {
    const { events, stop } = collectUpdates();
    const { executeActions } = await import("@/lib/dynamicUI/EventHandlerEngine");
    await executeActions([updateAction], { permissions: restricted });
    stop();

    expect(events).toHaveLength(0);
  });

  it("可信来源维持原行为：不带 scope（广播）", async () => {
    const { events, stop } = collectUpdates();
    const { executeActions } = await import("@/lib/dynamicUI/EventHandlerEngine");
    await executeActions([updateAction], {});
    stop();

    expect(events).toHaveLength(1);
    expect((events[0].detail as { scope?: string }).scope).toBeUndefined();
  });
});

describe("按 schemaId 兜底（DynamicUIRenderer 直渲染，覆盖路由入口）", () => {
  const renderWithPluginRecord = (action: DynamicAction) => {
    h.storeState.currentSchema = makeRecord("plugin", action);
    render(
      <MemoryRouter>
        <SchemaIdContext.Provider value={{ schemaId: SCHEMA_ID }}>
          <DynamicUIRenderer schema={makeSchema(action)} />
        </SchemaIdContext.Provider>
      </MemoryRouter>,
    );
  };

  it("store 里命中插件来源记录：非 dynamic_ui_* 被拒且不发 IPC", async () => {
    renderWithPluginRecord(invokeAction("delete_scheduled_task"));
    await settle();

    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("store 里命中插件来源记录：dynamic_ui_* 放行", async () => {
    renderWithPluginRecord(invokeAction("dynamic_ui_list_pins"));

    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(1));
    expect(h.invoke).toHaveBeenCalledWith("dynamic_ui_list_pins", {});
  });

  it("store 里没有对应记录（内存态 Schema）：维持现状", async () => {
    render(
      <MemoryRouter>
        <SchemaIdContext.Provider value={{ schemaId: "in-memory" }}>
          <DynamicUIRenderer schema={makeSchema(invokeAction("delete_scheduled_task"))} />
        </SchemaIdContext.Provider>
      </MemoryRouter>,
    );

    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(1));
  });
});
