// SPDX-License-Identifier: AGPL-3.0-only

/**
 * BotsPage：组合层契约。
 *
 * 只钉「跨模块接线」——Bot → 会话的归属链路（T4/T5 的验收点）：
 *   - 新建线程必须带 `botId`，随后跳到 `/chat?bot=<id>`；
 *   - 打开线程先 `setActiveConversation(id)` 再跳 `/chat?bot=<id>`
 *     （ChatPage 的 `?conversationId=` 只在挂载时消费一次，已经挂载的情况下靠这条路径）；
 *   - 没有选中 Bot 时自动选中第一个。
 *
 * 子组件与 store 全部替身化：本文件测的是本页的编排，不是 antd 或子组件的行为。
 */

import type { Bot } from "@/types";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { BotsPage } from "../BotsPage";

// vi.mock 工厂会被提升到文件顶部，工厂里引用的变量必须来自 vi.hoisted
const h = vi.hoisted(() => {
  const fn = () => vi.fn();
  const botState = {
    bots: [] as unknown[],
    loading: false,
    saving: false,
    deleting: false,
    error: null as string | null,
    activeBotId: null as string | null,
    loadBots: fn(),
    setActiveBot: fn(),
    createBot: fn(),
    updateBot: fn(),
    deleteBot: fn(),
  };
  const providerState = {
    providers: [] as Array<{ id: string; name: string; enabled: boolean; models: unknown[] }>,
    fetchProviders: fn(),
  };
  const conversationStore = {
    createConversation: fn(),
    setActiveConversation: fn(),
  };
  return {
    botState,
    providerState,
    conversationStore,
    toastSuccess: fn(),
    toastError: fn(),
  };
});

const { botState, providerState, conversationStore, toastError } = h;
const { loadBots, setActiveBot } = botState;
const { fetchProviders } = providerState;
const { createConversation, setActiveConversation } = conversationStore;

function makeBot(overrides: Partial<Bot> = {}): Bot {
  return {
    id: "bot-1",
    name: "投研助手",
    title: null,
    description: null,
    soul: "",
    soulHash: "",
    soulDrift: false,
    icon: "🤖",
    color: null,
    section: "research",
    modelId: null,
    providerId: null,
    effort: null,
    workdir: null,
    approvalMode: "full",
    agentProfileId: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    ...overrides,
  };
}

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/stores", () => ({
  useBotStore: (selector: (state: typeof h.botState) => unknown) => selector(h.botState),
  useProviderStore: (selector: (state: typeof h.providerState) => unknown) => selector(h.providerState),
  useConversationStore: Object.assign(() => undefined, { getState: () => h.conversationStore }),
}));

vi.mock("@/lib/toast", () => ({
  message: { success: h.toastSuccess, error: h.toastError },
}));

vi.mock("@/lib/errorI18n", () => ({
  backendErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

vi.mock("@/components/bot/BotList", () => ({
  BotList: ({
    bots,
    onSelect,
    onCreate,
  }: {
    bots: Bot[];
    onSelect: (id: string) => void;
    onCreate: () => void;
  }) => (
    <div data-testid="bot-list">
      <button type="button" data-testid="create-bot" onClick={onCreate}>
        create
      </button>
      {bots.map((bot) => (
        <button
          key={bot.id}
          type="button"
          data-testid={`select-${bot.id}`}
          onClick={() => onSelect(bot.id)}
        >
          {bot.name}
        </button>
      ))}
    </div>
  ),
}));

vi.mock("@/components/bot/BotDetail", () => ({
  BotDetail: ({
    bot,
    onCreateThread,
    onOpenThread,
  }: {
    bot: Bot;
    onCreateThread: () => void;
    onOpenThread: (id: string) => void;
  }) => (
    <div data-testid="bot-detail" data-bot-id={bot.id}>
      <button type="button" data-testid="create-thread" onClick={onCreateThread}>
        new-thread
      </button>
      <button type="button" data-testid="open-thread" onClick={() => onOpenThread("conv-7")}>
        open-thread
      </button>
    </div>
  ),
}));

vi.mock("@/components/bot/BotEmptyState", () => ({
  BotEmptyState: ({ onCreate }: { onCreate: () => void }) => (
    <button type="button" data-testid="bot-empty" onClick={onCreate}>
      empty
    </button>
  ),
}));

vi.mock("@/components/bot/BotEditor", () => ({
  BotEditor: () => <div data-testid="bot-editor" />,
}));

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{`${location.pathname}${location.search}`}</div>;
}

function renderPage() {
  return render(
    <MemoryRouter initialEntries={["/bots"]}>
      <BotsPage />
      <LocationProbe />
    </MemoryRouter>,
  );
}

describe("BotsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    botState.bots = [makeBot({ id: "bot-1" })];
    botState.activeBotId = "bot-1";
    botState.loading = false;
    botState.error = null;
    providerState.providers = [];
    createConversation.mockResolvedValue(undefined);
    setActiveConversation.mockResolvedValue(undefined);
  });

  it("挂载时加载 Bot 列表与模型提供方", () => {
    renderPage();

    expect(loadBots).toHaveBeenCalledTimes(1);
    expect(fetchProviders).toHaveBeenCalledTimes(1);
  });

  it("没有选中 Bot 时自动选中第一个", () => {
    botState.activeBotId = null;
    renderPage();

    expect(setActiveBot).toHaveBeenCalledWith("bot-1");
  });

  it("新建线程带 botId，并跳到 /chat?bot=<id>", async () => {
    renderPage();

    await userEvent.click(screen.getByTestId("create-thread"));

    await waitFor(() => {
      expect(createConversation).toHaveBeenCalledWith("chat.newConversation", undefined, undefined, {
        botId: "bot-1",
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("location")).toHaveTextContent("/chat?bot=bot-1");
    });
  });

  it("打开线程先设活动会话，再带 Bot 上下文进入对话页", async () => {
    renderPage();

    await userEvent.click(screen.getByTestId("open-thread"));

    expect(setActiveConversation).toHaveBeenCalledWith("conv-7");
    await waitFor(() => {
      expect(screen.getByTestId("location")).toHaveTextContent("/chat?bot=bot-1");
    });
  });

  it("切换到另一个 Bot 时列表选中项与详情一起切换（props 注入）", async () => {
    botState.bots = [makeBot({ id: "bot-1" }), makeBot({ id: "bot-2", name: "复盘助手" })];
    renderPage();

    expect(screen.getByTestId("bot-detail")).toHaveAttribute("data-bot-id", "bot-1");

    await userEvent.click(screen.getByTestId("select-bot-2"));
    expect(setActiveBot).toHaveBeenCalledWith("bot-2");
  });

  it("没有 Bot 时展示空状态，且不渲染详情", () => {
    botState.bots = [];
    botState.activeBotId = null;
    renderPage();

    expect(screen.getByTestId("bot-empty")).toBeInTheDocument();
    expect(screen.queryByTestId("bot-detail")).not.toBeInTheDocument();
  });

  it("新建线程失败时提示错误且不跳转", async () => {
    createConversation.mockRejectedValueOnce(new Error("CONVERSATION_CREATE_FAILED"));
    renderPage();

    await userEvent.click(screen.getByTestId("create-thread"));

    await waitFor(() => {
      expect(toastError).toHaveBeenCalledWith("CONVERSATION_CREATE_FAILED");
    });
    expect(screen.getByTestId("location")).toHaveTextContent("/bots");
  });
});
