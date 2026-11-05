// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 聊天消息侧 schema 卡片插槽（B8-1 实现半边）的验收用例。
 *
 * 三条都打在**用户能看到的东西**上，不回头调用判据函数自证：
 * 1. 给 `(schemaId, dataContext)` ⇒ 画出标题与表格值（数据来自 `dataContext`，schema 来自库）；
 * 2. schema 取不到 ⇒ 明确的错误态（既有 11 语言译文），不崩、不白屏；
 * 3. 载荷不合契约 ⇒ 静默不渲染（一条坏消息不该炸掉整条渲染树），且宿主卡片文件里
 *    **没有任何股票 / 插件产物 import**（§93 安全边界的静态钉子）。
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
import { invoke } from "@/lib/invoke";
import type { UISchema } from "@/types";
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
  invokeMock.mockReset();
});

describe("PluginSchemaCardSlot —— 消息侧 schema 卡片", () => {
  it("给 schemaId + 数据 ⇒ 画出卡片标题、schema 标题与表格值", async () => {
    invokeMock.mockResolvedValue({
      id: "release-note-card",
      schemaJson: JSON.stringify(RELEASE_NOTE_SCHEMA),
    });

    renderSlot({
      pluginCards: [{
        schemaId: "release-note-card",
        title: "本期变更",
        dataContext: RELEASE_NOTE_DATA,
      }],
    });

    // 宿主 chrome（插件给的标题）与 schema 自己的 Card 标题
    expect(screen.getByText("本期变更")).toBeInTheDocument();
    expect(await screen.findByText("发行说明")).toBeInTheDocument();
    // dataContext.rows 经 Table 节点（id = rows）绑进表格
    expect(await screen.findByText("新增离线模式")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("修复启动崩溃")).toBeInTheDocument();

    // 取 schema 走的是既有命令，且只认 schemaId
    expect(invokeMock).toHaveBeenCalledWith("get_dynamic_ui_schema", { id: "release-note-card" });
  });

  it("schema 取不到 ⇒ 明确错误态（既有译文 + 原因），不崩、不白屏", async () => {
    invokeMock.mockRejectedValue(new Error("DYNAMIC_UI_SCHEMA_NOT_FOUND: release-note-card"));

    renderSlot({ pluginCards: [{ schemaId: "release-note-card" }] });

    // 错误态落到既有 key（11 语言已有译文），原因原样带出便于定位
    expect(await screen.findByText("dynamicUI.schemaNotFound")).toBeInTheDocument();
    expect(screen.getByText("DYNAMIC_UI_SCHEMA_NOT_FOUND: release-note-card")).toBeInTheDocument();
    // 不白屏：卡片容器仍在（有可定位的宿主节点）
    expect(document.querySelector('[data-plugin-schema-card="release-note-card"]')).not.toBeNull();
  });

  it("schemaJson 坏掉同样落错误态，而不是抛到消息渲染树", async () => {
    invokeMock.mockResolvedValue({ id: "release-note-card", schemaJson: "{not json" });

    renderSlot({ pluginCards: [{ schemaId: "release-note-card" }] });

    expect(await screen.findByText("dynamicUI.schemaNotFound")).toBeInTheDocument();
  });

  it("载荷不合契约 ⇒ 静默不渲染（不占位、不报错）", () => {
    const { container } = renderSlot({
      pluginCards: [null, { schemaId: "" }, { schemaId: 42 }, "not-a-card"],
    });
    expect(container).toBeEmptyDOMElement();
    expect(invokeMock).not.toHaveBeenCalled();

    // meta 本身缺失 / 不是对象 / 键不是数组，三种退化都不渲染
    expect(renderSlot(undefined).container).toBeEmptyDOMElement();
    expect(renderSlot({ pluginCards: "nope" }).container).toBeEmptyDOMElement();
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
