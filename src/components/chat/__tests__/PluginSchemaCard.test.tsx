// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 聊天消息侧 schema 卡片插槽（B8-1 实现半边 + 任务 #216 前置 ① 的键迁移）验收用例。
 *
 * 四条都打在**用户能看到的东西**上，不回头调用判据函数自证：
 * 1. 给注册表键 `card` + `dataContext` ⇒ 画出卡片标题、schema 标题与表格值（数据来自 dataContext，
 *    卡体来自库）；`schemaId` 由宿主按 `plugin.{pluginId}.{key}` 拼，不由消息给；
 * 2. schema 取不到 ⇒ 明确的错误态（既有 11 语言译文），不崩、不白屏；
 * 3. 载荷不合契约 / **归属不明**（未在清单里声明的卡、裸 schemaId）⇒ 静默不渲染（fail-closed）；
 * 4. 宿主卡片文件不 import 任何股票 / 插件产物（§93 安全边界的静态钉子）。
 *
 * 固定装置刻意取**非股票语义**（发行说明卡）：验收点正是「宿主不 import 任何股票文件也能画出卡片」。
 */

import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import path from "node:path";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// t 返回 key 本身：断言命中的是哪个既有 key，而不是某条译文（译文由 11 语言文件各自负责）
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: vi.fn() };
});

import { CardContainer } from "@/components/dynamicUI/containers/CardContainer";
import { DataTable } from "@/components/dynamicUI/data/DataTable";
import { componentRegistry } from "@/lib/dynamicUI/ComponentRegistry";
import { __resetCardRegistryForTests, registerCardContributions } from "@/lib/dynamicUI/pluginCardRegistry";
import { invoke } from "@/lib/invoke";
import type { PluginTaskCardsInfoDto, UISchema } from "@/types";
import { PluginSchemaCardSlot } from "../PluginSchemaCardSlot";

const invokeMock = vi.mocked(invoke);

/** 非股票语义的固定装置：一张发行说明卡（标题 + 两列明细）。 */
const RELEASE_NOTE_SCHEMA: UISchema = {
  version: "1.0.0",
  id: "release-note-card",
  type: "Card",
  props: { title: "发行说明" },
  children: [
    {
      version: "1.0.0",
      id: "rows",
      type: "Table",
      props: {
        columns: [
          { title: "条目", dataIndex: "name" },
          { title: "数量", dataIndex: "qty" },
        ],
      },
    },
  ],
};

const RELEASE_NOTE_DATA = {
  rows: [
    { id: "r1", name: "新增离线模式", qty: 3 },
    { id: "r2", name: "修复启动崩溃", qty: 1 },
  ],
};

/** 清单声明（后端 `plugin_list_task_cards` 的投影形状，任务 #216 前置 ②）。 */
const DECLARED_CARDS: PluginTaskCardsInfoDto[] = [{
  id: "demo",
  name: "Demo",
  version: "1.0.0",
  description: "demo plugin",
  author: null,
  enabled: true,
  taskCards: [{
    id: "release-note",
    title: "发行说明",
    surface: "chat",
    summaryCommand: "rows",
    detailSchema: "release-note-card",
    tone: null,
    order: 0,
    actions: [],
  }],
}];

/** 宿主拼出来的卡体行 id：`plugin.{pluginId}.{ui.json key}`。 */
const RELEASE_NOTE_ROW_ID = "plugin.demo.release-note-card";

function renderSlot(meta: Record<string, unknown> | null | undefined) {
  return render(
    <MemoryRouter>
      <PluginSchemaCardSlot meta={meta} />
    </MemoryRouter>,
  );
}

beforeAll(() => {
  // 生产里由 main.tsx 调 registerAllBuiltins() 注册全部内置组件；这里只注册固定装置用到的两种，
  // 断言的仍是同一条渲染链（componentRegistry → DynamicUIRenderer → 容器组件）。
  componentRegistry.register({
    type: "Card",
    component: CardContainer,
    category: "container",
    label: "card",
  });
  componentRegistry.register({
    type: "Table",
    component: DataTable,
    category: "data-display",
    label: "table",
  });
});

beforeEach(() => {
  __resetCardRegistryForTests();
  registerCardContributions(DECLARED_CARDS);
  invokeMock.mockReset();
  // 投影与取 schema 走同一条 invoke：按命令名分流，避免两条命令互相喂错返回体。
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "plugin_list_task_cards") {
      return DECLARED_CARDS;
    }
    if (command === "get_dynamic_ui_schema") {
      return { id: RELEASE_NOTE_ROW_ID, schemaJson: JSON.stringify(RELEASE_NOTE_SCHEMA) };
    }
    throw new Error(`unexpected command: ${command}`);
  });
});

describe("PluginSchemaCardSlot —— 消息侧 schema 卡片", () => {
  it("给注册表键 + 数据 ⇒ 画出卡片标题、schema 标题与表格值", async () => {
    renderSlot({
      pluginCards: [{
        card: "demo:release-note",
        title: "本期变更",
        dataContext: RELEASE_NOTE_DATA,
      }],
    });

    // 宿主 chrome（消息给的标题）与 schema 自己的 Card 标题
    expect(screen.getByText("本期变更")).toBeInTheDocument();
    expect(await screen.findByText("发行说明")).toBeInTheDocument();
    // dataContext.rows 经 Table 节点（id = rows）绑进表格
    expect(await screen.findByText("新增离线模式")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("修复启动崩溃")).toBeInTheDocument();

    // 取 schema 用的是**宿主拼的行 id**（消息里没有写 id 的机会）
    expect(invokeMock).toHaveBeenCalledWith("get_dynamic_ui_schema", { id: RELEASE_NOTE_ROW_ID });
  });

  it("旧键 schemaId 仍受理，但只在它能被注册表反查到归属时", async () => {
    renderSlot({ pluginCards: [{ schemaId: RELEASE_NOTE_ROW_ID }] });

    expect(await screen.findByText("发行说明")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("get_dynamic_ui_schema", { id: RELEASE_NOTE_ROW_ID });
  });

  it("schema 取不到 ⇒ 明确错误态（既有译文 + 原因），不崩、不白屏", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "plugin_list_task_cards") {
        return DECLARED_CARDS;
      }
      throw new Error(`DYNAMIC_UI_SCHEMA_NOT_FOUND: ${RELEASE_NOTE_ROW_ID}`);
    });

    renderSlot({ pluginCards: [{ card: "demo:release-note" }] });

    // 错误态落到既有 key（11 语言已有译文），原因原样带出便于定位
    expect(await screen.findByText("dynamicUI.schemaNotFound")).toBeInTheDocument();
    expect(screen.getByText(`DYNAMIC_UI_SCHEMA_NOT_FOUND: ${RELEASE_NOTE_ROW_ID}`))
      .toBeInTheDocument();
    // 不白屏：卡片容器仍在（有可定位的宿主节点）
    expect(
      document.querySelector(`[data-plugin-schema-card="${RELEASE_NOTE_ROW_ID}"]`),
    ).not.toBeNull();
  });

  it("schemaJson 坏掉同样落错误态，而不是抛到消息渲染树", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "plugin_list_task_cards") {
        return DECLARED_CARDS;
      }
      // 注册表幂等加载还会投宿主投影；fallback 只服务 get_dynamic_ui_schema
      if (command === "task_card_host_cards") {
        return [];
      }
      return { id: RELEASE_NOTE_ROW_ID, schemaJson: "{not json" };
    });

    renderSlot({ pluginCards: [{ card: "demo:release-note" }] });

    expect(await screen.findByText("dynamicUI.schemaNotFound")).toBeInTheDocument();
  });

  it("归属不明 / 载荷不合契约 ⇒ 静默不渲染（fail-closed）", () => {
    // 未声明的卡键、裸 schemaId、空 / 非字符串、非对象条目：一条都不该画。
    // （注册表投影由挂载幂等拉取，与消息内容无关；这里钉的是渲染面与 schema 取数都 fail-closed。）
    const schemaCalls = () => invokeMock.mock.calls.filter(([cmd]) => cmd === "get_dynamic_ui_schema");
    const { container } = renderSlot({
      pluginCards: [
        null,
        { schemaId: "" },
        { schemaId: 42 },
        { schemaId: "release-note-card" },
        { card: "ghost:never-declared" },
        "not-a-card",
      ],
    });
    expect(container).toBeEmptyDOMElement();
    expect(schemaCalls()).toEqual([]);

    // meta 本身缺失 / 不是对象 / 键不是数组，三种退化都不渲染，也不发生 schema 取数
    expect(renderSlot(undefined).container).toBeEmptyDOMElement();
    expect(renderSlot({ pluginCards: "nope" }).container).toBeEmptyDOMElement();
    expect(schemaCalls()).toEqual([]);
  });

  it("宿主卡片文件不 import 任何股票 / 插件产物（§93：第三方代码不进主 origin）", () => {
    const files = ["../PluginSchemaCard.tsx", "../PluginSchemaCardSlot.tsx"];
    for (const rel of files) {
      const source = readFileSync(path.resolve(import.meta.dirname, rel), "utf8");
      const importLines = source.match(/^import[^;]+;/gm) ?? [];
      expect(importLines.filter((line) => /stock|components\/plugin/i.test(line))).toEqual([]);
    }
  });
});
