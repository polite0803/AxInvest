import { expect, test } from "@playwright/test";

/**
 * P1「Bot 归属层」端到端验收（浏览器模式，browserMock）。
 *
 * ## 这些用例为什么不会被 CI 跳过
 *
 * CI 现在用显式 heavy tag 过滤排除重用例集；本文件**不带任何 heavy tag**，
 * 所以标题里即使出现 `Agent` / `Conversation` / `Gateway`，也照样执行。
 * 第 1 条用例的标题刻意保留 `Conversation` 一词，作为「旧标题正则会误跳过」的活证据。
 *
 * ## 数据从哪来
 *
 * 全部走 browserMock 的 localStorage（键前缀 `axagent_`）：
 *   · `axagent_bots`                 —— Bot DTO（camelCase，与 `src/types/bot.ts` 一致）
 *   · `axagent_conversations`        —— 会话 DTO，归属键 `botId`（mock 同时兼容 `bot_id`）
 *   · `axagent_bot_memory_<botId>`   —— MEMORY.md 索引与主题清单
 * 种入用 `page.addInitScript`：只写一次（`__p1_bot_seeded` 标记），刷新后保留，
 * 才能验证「刷新/重启后线程仍归属该 Bot」。
 *
 * ## 选择器契约
 *
 * 已落地的 UI 提供了下列稳定钩子，本文件只用这些：
 *   · `[data-testid="bot-list-item"]` / `bot-create-btn` / `bot-edit-btn` / `bot-delete-btn`
 *   · `[data-testid="bot-soul-panel"]` / `bot-memory-panel` / `bot-threads-panel`
 *   · `#bot-editor-name`（BotEditor 输入框 id）
 *   · `.conv-item`（会话条目，带 `data-conv-id`）与 `[data-testid="new-conversation-btn"]`
 *   · Ant Design 的 `.ant-modal-footer .ant-btn-primary` / `.ant-popconfirm-buttons .ant-btn-primary`
 * 少任何一个，本文件会以「元素不存在」明确失败，而不是静默跳过。
 */

interface SeedBot {
  id: string;
  name: string;
  soul: string;
}

interface SeedConversation {
  id: string;
  title: string;
  botId: string | null;
}

const BASE_BOT = {
  title: null,
  description: null,
  soulHash: "00000000",
  soulDrift: false,
  icon: "🤖",
  color: null,
  section: "default",
  modelId: null,
  providerId: null,
  effort: null,
  workdir: null,
  approvalMode: "full",
  agentProfileId: null,
  isEnabled: true,
  sortOrder: 0,
  createdAt: 1_700_000_000_000,
  updatedAt: 1_700_000_000_000,
};

function seedScript(bots: SeedBot[], conversations: SeedConversation[], memory = "") {
  return `(() => {
    if (localStorage.getItem("__p1_bot_seeded")) { return; }
    const bots = ${
    JSON.stringify(
      bots.map((b, i) => ({ ...BASE_BOT, ...b, sortOrder: i })),
    )
  };
    localStorage.setItem("axagent_bots", JSON.stringify(bots));
    const conversations = ${
    JSON.stringify(
      conversations.map((c, i) => ({
        id: c.id,
        title: c.title,
        botId: c.botId,
        modelId: "mock-model",
        providerId: "mock-provider",
        systemPrompt: null,
        temperature: null,
        maxTokens: null,
        topP: null,
        searchEnabled: false,
        searchProviderId: null,
        thinkingBudget: null,
        enabledMcpServerIds: [],
        enabledKnowledgeBaseIds: [],
        enabledMemoryNamespaceIds: [],
        enabledWikiIds: [],
        isPinned: false,
        isArchived: false,
        contextCompression: false,
        categoryId: null,
        parentConversationId: null,
        mode: "chat",
        messageCount: 1,
        createdAt: 1_700_000_000_000 + i,
        updatedAt: 1_700_000_000_000 + i,
        enabledSkillIds: [],
        sessionType: "conversation",
      })),
    )
  };
    localStorage.setItem("axagent_conversations", JSON.stringify(conversations));
    const memory = ${JSON.stringify(memory)};
    if (memory) {
      for (const b of bots) {
        localStorage.setItem(
          "axagent_bot_memory_" + b.id,
          JSON.stringify({
            indexContent: memory,
            topics: [
              {
                relativePath: "memory/invest.md",
                category: "invest",
                title: "投研",
                updatedAt: 1700000000000,
              },
            ],
          }),
        );
      }
    }
    localStorage.setItem("__p1_bot_seeded", "1");
  })();`;
}

const botItems = (page: import("@playwright/test").Page) => page.locator('[data-testid="bot-list-item"]');

test.describe("Bot 归属层（P1）", () => {
  test("Bot 列表可见：标题含 Conversation 的用例在显式 tag 过滤下仍会执行", async ({ page }) => {
    await page.addInitScript(
      seedScript([{ id: "bot-a", name: "投研助手", soul: "# 只回答 A 股问题" }], []),
    );
    await page.goto("/bots");
    await expect(botItems(page).filter({ hasText: "投研助手" })).toHaveCount(1, {
      timeout: 30_000,
    });
  });

  test("创建 Bot 后列表即时出现（不刷新）", async ({ page }) => {
    await page.addInitScript(seedScript([], []));
    await page.goto("/bots");
    await page.locator('[data-testid="bot-create-btn"]').click();
    await page.locator("#bot-editor-name").fill("新建机器人");
    await page.locator(".ant-modal-footer .ant-btn-primary").click();
    await expect(botItems(page).filter({ hasText: "新建机器人" })).toHaveCount(1, {
      timeout: 15_000,
    });
  });

  test("在 Bot 下新建线程，刷新后仍归属该 Bot（持久化）", async ({ page }) => {
    await page.addInitScript(
      seedScript([{ id: "bot-a", name: "投研助手", soul: "# SOUL" }], []),
    );
    await page.goto("/chat?bot=bot-a");
    await page.locator('[data-testid="new-conversation-btn"]').click();
    // 标题由 i18n 决定（「新对话」/「New chat」…），这里只断言「出现了一条线程」。
    await expect(page.locator(".conv-item")).toHaveCount(1, { timeout: 15_000 });

    // 等落库完成再刷新：只等 UI 计数会在高并发（fullyParallel）下与 IPC 写入竞态，
    // 出现「刷新后线程还在内存里、localStorage 尚未写入」的偶发红。
    await page.waitForFunction(() => {
      const raw = localStorage.getItem("axagent_conversations");
      return JSON.parse(raw ?? "[]").length > 0;
    });

    await page.reload();
    await expect(page.locator(".conv-item").first()).toBeVisible({ timeout: 15_000 });

    const stored = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("axagent_conversations") ?? "[]") as {
        botId?: string | null;
        bot_id?: string | null;
      }[]
    );
    expect(stored.length).toBeGreaterThan(0);
    expect(stored.every((c) => (c.botId ?? c.bot_id) === "bot-a")).toBe(true);
  });

  test("聊天侧栏按 Bot 过滤：只显示该 Bot 的线程（Web 模式不恒空）", async ({ page }) => {
    await page.addInitScript(
      seedScript(
        [
          { id: "bot-a", name: "投研助手", soul: "# A" },
          { id: "bot-b", name: "写作助手", soul: "# B" },
        ],
        [
          { id: "conv-a", title: "A 的线程", botId: "bot-a" },
          { id: "conv-b", title: "B 的线程", botId: "bot-b" },
          { id: "conv-orphan", title: "未归属线程", botId: null },
        ],
      ),
    );
    await page.goto("/chat?bot=bot-a");
    await expect(page.locator(".conv-item").filter({ hasText: "A 的线程" })).toHaveCount(1, {
      timeout: 30_000,
    });
    await expect(page.locator(".conv-item").filter({ hasText: "B 的线程" })).toHaveCount(0);
    await expect(page.locator(".conv-item").filter({ hasText: "未归属线程" })).toHaveCount(0);
  });

  test("Bot 详情展示 SOUL 与记忆预算（只读）", async ({ page }) => {
    await page.addInitScript(
      seedScript(
        [{ id: "bot-a", name: "投研助手", soul: "# 只回答 A 股问题" }],
        [],
        "# MEMORY\n- 偏好：先看财报",
      ),
    );
    await page.goto("/bots");
    await botItems(page).filter({ hasText: "投研助手" }).click();
    await expect(page.locator('[data-testid="bot-soul-panel"]')).toContainText("只回答 A 股问题", {
      timeout: 15_000,
    });
    const memory = page.locator('[data-testid="bot-memory-panel"]');
    await expect(memory).toContainText("偏好：先看财报");
    await expect(memory).toContainText("2/200");
  });

  test("删除 Bot 后线程保留为未归属（ADR-009）", async ({ page }) => {
    await page.addInitScript(
      seedScript(
        [{ id: "bot-a", name: "投研助手", soul: "# A" }],
        [{ id: "conv-a", title: "要保留的线程", botId: "bot-a" }],
      ),
    );
    await page.goto("/bots");
    await botItems(page).filter({ hasText: "投研助手" }).click();
    await page.locator('[data-testid="bot-delete-btn"]').click();
    await expect(page.locator(".ant-popconfirm")).toContainText("线程与记忆会保留");
    await page.locator(".ant-popconfirm-buttons .ant-btn-primary").click();

    await expect(botItems(page)).toHaveCount(0, { timeout: 15_000 });
    await page.goto("/chat");
    await expect(page.locator(".conv-item").filter({ hasText: "要保留的线程" })).toHaveCount(1, {
      timeout: 15_000,
    });
  });
});
