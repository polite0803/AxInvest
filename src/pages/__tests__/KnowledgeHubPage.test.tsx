// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { KnowledgeHubPage } from "../KnowledgeHubPage";

// 任务 #219：SourceManager 的跨域能力（memory / llmWiki / 记忆图）必须由页面层注入。
// 本测试钉住注入接线：页面用选择器只取所需切片、动作引用原样透传、渲染函数可回调 onClose。

// vi.hoisted 先于 import 执行，mock 工厂与断言共享同一份状态（避免工厂 TDZ）
const h = vi.hoisted(() => {
  const loadNamespaces = vi.fn(async () => {});
  const loadWikis = vi.fn(async () => {});
  const deleteWiki = vi.fn(async () => {});
  return {
    loadNamespaces,
    loadWikis,
    deleteWiki,
    memoryState: {
      namespaces: [{ id: "ns-1", name: "ns-one" }],
      loading: true,
      loadNamespaces,
      // 干扰字段：页面只应注入切片，不得把整个 store 透传下去
      items: [],
    },
    wikiState: {
      wikis: [{ id: "w-1", name: "wiki-one" }],
      loadWikis,
      deleteWiki,
      // 干扰字段
      pages: [],
    },
    captured: {
      props: null as null | {
        memory: { namespaces: unknown[]; loading: boolean; loadNamespaces: () => Promise<void> };
        wiki: { wikis: unknown[]; loadWikis: () => Promise<void>; deleteWiki: (id: string) => Promise<void> };
        renderMemoryGraph: (onClose: () => void) => ReactNode;
      },
    },
  };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("react-router-dom", () => ({
  useNavigate: () => vi.fn(),
}));

vi.mock("@/stores/feature/memoryStore", () => ({
  useMemoryStore: (selector: (s: typeof h.memoryState) => unknown) => selector(h.memoryState),
}));

vi.mock("@/stores/feature/llmWikiStore", () => ({
  useLlmWikiStore: (selector: (s: typeof h.wikiState) => unknown) => selector(h.wikiState),
}));

vi.mock("@/components/settings/SourceManager", () => ({
  SourceManager: (props: NonNullable<typeof h.captured.props>) => {
    h.captured.props = props;
    return <div data-testid="source-manager-stub" />;
  },
}));

vi.mock("@/components/memory/MemoryGraphView", () => ({
  MemoryGraphView: ({ onClose }: { onClose: () => void }) => (
    <button type="button" data-testid="memory-graph-stub" onClick={onClose}>
      memory-graph
    </button>
  ),
}));

function injectedProps() {
  const props = h.captured.props;
  if (!props) {
    throw new Error("SourceManager 未收到页级注入 props");
  }
  return props;
}

describe("KnowledgeHubPage 页级注入（任务 #219）", () => {
  it("memoryStore 只注入所需切片，动作引用原样透传", () => {
    render(<KnowledgeHubPage />);
    const props = injectedProps();
    expect(props.memory.namespaces).toBe(h.memoryState.namespaces);
    expect(props.memory.loading).toBe(true);
    expect(props.memory.loadNamespaces).toBe(h.loadNamespaces);
    // 不得整 store 透传（干扰字段 items 不许出现在切片里）
    expect(Object.keys(props.memory).sort()).toEqual(["loadNamespaces", "loading", "namespaces"]);
  });

  it("llmWikiStore 只注入所需切片，动作引用原样透传", () => {
    render(<KnowledgeHubPage />);
    const props = injectedProps();
    expect(props.wiki.wikis).toBe(h.wikiState.wikis);
    expect(props.wiki.loadWikis).toBe(h.loadWikis);
    expect(props.wiki.deleteWiki).toBe(h.deleteWiki);
    expect(Object.keys(props.wiki).sort()).toEqual(["deleteWiki", "loadWikis", "wikis"]);
  });

  it("renderMemoryGraph 渲染 memory 域视图并透传 onClose", async () => {
    render(<KnowledgeHubPage />);
    const onClose = vi.fn();
    const node = injectedProps().renderMemoryGraph(onClose);
    render(<>{node}</>);
    await userEvent.click(screen.getByTestId("memory-graph-stub"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
