// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App as AntdApp } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SourceManager, type SourceManagerProps } from "../SourceManager";

// 任务 #219：SourceManager 改为页级注入后，本测试钉住三件事：
// ① memory / wiki 切片确实来自 props（而非组件内部直连 store）；
// ② 记忆图经 renderMemoryGraph 注入渲染，onClose 能收起；
// ③ SourceCard 删除后的跨域刷新走注入的 loadNamespaces。

const h = vi.hoisted(() => {
  const fetchSources = vi.fn(async () => {});
  const fetchProviders = vi.fn(async () => {});
  const loadBases = vi.fn(async () => {});
  const deleteSource = vi.fn(async () => {});
  return {
    fetchSources,
    fetchProviders,
    loadBases,
    deleteSource,
    sourceState: {
      sources: [
        { id: "kb-1", name: "KB-One", containerType: "knowledge", sortOrder: 0, enabled: true },
        { id: "ns-1", name: "Mem-One", containerType: "memory", sortOrder: 1, enabled: true },
        { id: "w-1", name: "Wiki-Src", containerType: "wiki", sortOrder: 2, enabled: true },
      ],
      loading: false,
      error: null,
      fetchSources,
      deleteSource,
      searchAllSources: vi.fn(async () => ({ context: "", totalResults: 0, sources: [] })),
      getSourceConfig: vi.fn(async () => ({})),
      updateSourceEmbedding: vi.fn(async () => ({ embeddingChanged: false })),
      rebuildSourceIndex: vi.fn(async () => {}),
    },
    knowledgeState: { bases: [], loadBases, loading: false },
    knowledgeSourceState: {
      loadSources: vi.fn(async () => {}),
      fetchAll: vi.fn(async () => []),
      fetchUrlToWiki: vi.fn(async () => null),
      githubRepoImport: vi.fn(async () => null),
      sitemapCrawl: vi.fn(async () => null),
    },
  };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("react-router-dom", () => ({
  useNavigate: () => vi.fn(),
}));

vi.mock("@/stores", () => {
  const useSourceStore = (selector?: (s: typeof h.sourceState) => unknown) =>
    selector ? selector(h.sourceState) : h.sourceState;
  useSourceStore.getState = () => h.sourceState;
  const providerState = { providers: [] as unknown[], fetchProviders: h.fetchProviders };
  const useProviderStore = (selector?: (s: typeof providerState) => unknown) =>
    selector ? selector(providerState) : providerState;
  const useKnowledgeStore = (selector?: (s: typeof h.knowledgeState) => unknown) =>
    selector ? selector(h.knowledgeState) : h.knowledgeState;
  useKnowledgeStore.getState = () => h.knowledgeState;
  const useKnowledgeSourceStore = () => h.knowledgeSourceState;
  return { useSourceStore, useProviderStore, useKnowledgeStore, useKnowledgeSourceStore };
});

vi.mock("@/components/shared/EmbeddingModelSelect", () => ({
  EmbeddingModelSelect: () => null,
}));

vi.mock("@/components/shared/ModelSelect", () => ({
  useEmbeddingProviderLabel: () => (id: string) => id,
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(async () => ({})),
}));

vi.mock("@/lib/errorI18n", () => ({
  showBackendError: vi.fn(),
  backendErrorText: (e: unknown) => String(e),
}));

vi.mock("../DirectoryImportWizard", () => ({
  DirectoryImportWizard: () => null,
}));

vi.mock("../KnowledgeBaseDocuments", () => ({
  KnowledgeBaseDocuments: () => null,
}));

function makeProps(): SourceManagerProps {
  return {
    memory: {
      namespaces: [{ id: "ns-1", name: "NS-One", scope: "global", sortOrder: 0 }],
      loading: false,
      loadNamespaces: vi.fn(async () => {}),
    },
    wiki: {
      wikis: [{
        id: "w-1",
        name: "Wiki-One",
        rootPath: "/tmp/wiki",
        schemaVersion: "1",
        noteCount: 2,
        sourceCount: 1,
        createdAt: 0,
        updatedAt: 0,
      }],
      loadWikis: vi.fn(async () => {}),
      deleteWiki: vi.fn(async () => {}),
    },
    renderMemoryGraph: (onClose) => (
      <div data-testid="injected-memory-graph">
        <button type="button" onClick={onClose}>close-graph</button>
      </div>
    ),
  };
}

function renderManager(props: SourceManagerProps) {
  return render(
    <AntdApp>
      <SourceManager {...props} />
    </AntdApp>,
  );
}

/** 按文本定位 antd Tabs 页签并点击（先例：BacktestPage.test.tsx） */
function clickTab(container: HTMLElement, label: string) {
  const tab = [...container.querySelectorAll(".ant-tabs-tab")].find((el) => el.textContent?.includes(label));
  if (!tab) {
    throw new Error(`未找到页签：${label}`);
  }
  fireEvent.click(tab);
}

describe("SourceManager 页级注入消费（任务 #219）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("memory 页签消费注入切片：加载命名空间、渲染注入的记忆图且 onClose 可收起", async () => {
    const props = makeProps();
    const { container } = renderManager(props);
    clickTab(container, "sourceManager.tab.memory");

    await waitFor(() => {
      expect(props.memory.loadNamespaces).toHaveBeenCalledTimes(1);
    });
    // 命名空间列表来自注入切片
    expect(screen.getByText("NS-One")).toBeInTheDocument();
    // 记忆源卡片来自 sourceStore（barrel 引用，不在本任务键面）
    expect(screen.getByText("Mem-One")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "memory.graph.show" }));
    expect(screen.getByTestId("injected-memory-graph")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "close-graph" }));
    expect(screen.queryByTestId("injected-memory-graph")).not.toBeInTheDocument();
  });

  it("wiki 页签消费注入切片：加载 wiki 列表并渲染 WikiCard", async () => {
    const props = makeProps();
    const { container } = renderManager(props);
    clickTab(container, "sourceManager.tab.wiki");

    await waitFor(() => {
      expect(props.wiki.loadWikis).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByText("Wiki-One")).toBeInTheDocument();
  });

  it("SourceCard 删除后的跨域刷新走注入的 loadNamespaces", async () => {
    const props = makeProps();
    const { container } = renderManager(props);
    clickTab(container, "sourceManager.tab.memory");
    await waitFor(() => {
      expect(props.memory.loadNamespaces).toHaveBeenCalledTimes(1);
    });

    // Mem-One 卡片内唯一的无名按钮 = 图标删除键
    const card = screen.getByText("Mem-One").closest(".ant-card");
    if (!card) {
      throw new Error("未找到 Mem-One 源卡片");
    }
    fireEvent.click(within(card as HTMLElement).getByRole("button", { name: "" }));

    // modal.confirm 的确认键触发 onOk → deleteSource + 三路刷新
    const okButton = await screen.findByRole("button", { name: "sourceManager.confirmDeleteOk" });
    fireEvent.click(okButton);

    await waitFor(() => {
      expect(h.deleteSource).toHaveBeenCalledTimes(1);
      // mount 时 1 次 + 删除后刷新 1 次
      expect(props.memory.loadNamespaces).toHaveBeenCalledTimes(2);
    });
    expect(h.fetchSources).toHaveBeenCalled();
    expect(h.loadBases).toHaveBeenCalled();
  });
});
