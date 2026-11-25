// SPDX-License-Identifier: AGPL-3.0-only

/**
 * BotList：纯展示组件（数据与回调全走 props）。
 * 断言点放在「交互契约」上：选中 / 新建回调带正确的 id，选中态可被读屏识别。
 */

import type { Bot } from "@/types";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BotList } from "../BotList";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({ t: (key: string) => key }),
}));

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

describe("BotList", () => {
  it("渲染标题、新建按钮与每个 Bot 的名称/角色", () => {
    render(
      <BotList
        bots={[makeBot({ id: "bot-1", name: "投研", title: "A 股" }), makeBot({ id: "bot-2", name: "复盘" })]}
        activeBotId="bot-1"
        onSelect={vi.fn()}
        onCreate={vi.fn()}
      />,
    );

    expect(screen.getByText("bots.title")).toBeInTheDocument();
    expect(screen.getByTestId("bot-create-btn")).toHaveTextContent("bots.newBot");
    expect(screen.getAllByTestId("bot-list-item")).toHaveLength(2);
    expect(screen.getByText("投研")).toBeInTheDocument();
    expect(screen.getByText("A 股")).toBeInTheDocument();
    expect(screen.getByText("复盘")).toBeInTheDocument();
  });

  it("点击条目把 botId 交给 onSelect；当前项 aria-pressed=true", async () => {
    const onSelect = vi.fn();
    render(
      <BotList
        bots={[makeBot({ id: "bot-1" }), makeBot({ id: "bot-2", name: "另一个" })]}
        activeBotId="bot-2"
        onSelect={onSelect}
        onCreate={vi.fn()}
      />,
    );

    const items = screen.getAllByTestId("bot-list-item");
    expect(items[0]).toHaveAttribute("aria-pressed", "false");
    expect(items[1]).toHaveAttribute("aria-pressed", "true");

    await userEvent.click(items[0]);
    expect(onSelect).toHaveBeenCalledWith("bot-1");
  });

  it("键盘 Enter / 空格触发选中（可访问性）", () => {
    const onSelect = vi.fn();
    render(<BotList bots={[makeBot({ id: "bot-9" })]} activeBotId={null} onSelect={onSelect} onCreate={vi.fn()} />);

    const item = screen.getByTestId("bot-list-item");
    fireEvent.keyDown(item, { key: "Enter" });
    fireEvent.keyDown(item, { key: " " });

    expect(onSelect).toHaveBeenCalledTimes(2);
    expect(onSelect).toHaveBeenNthCalledWith(1, "bot-9");
  });

  it("新建按钮触发 onCreate", async () => {
    const onCreate = vi.fn();
    render(<BotList bots={[]} activeBotId={null} onSelect={vi.fn()} onCreate={onCreate} />);

    await userEvent.click(screen.getByTestId("bot-create-btn"));
    expect(onCreate).toHaveBeenCalledTimes(1);
  });

  it("空列表只渲染标题与新建入口", () => {
    render(<BotList bots={[]} activeBotId={null} onSelect={vi.fn()} onCreate={vi.fn()} />);

    expect(screen.queryAllByTestId("bot-list-item")).toHaveLength(0);
    expect(screen.getByTestId("bot-create-btn")).toBeInTheDocument();
  });

  it("B4-R：待答徽章按 botId 逐行计数（双 Bot 各显示自己的数，无 pending 的行不显示）", () => {
    render(
      <BotList
        bots={[
          makeBot({ id: "bot-a", name: "A" }),
          makeBot({ id: "bot-b", name: "B" }),
          makeBot({ id: "bot-c", name: "C" }),
        ]}
        activeBotId={null}
        pendingByBot={{ "bot-a": 1, "bot-b": 3 }}
        onSelect={vi.fn()}
        onCreate={vi.fn()}
      />,
    );

    const badges = screen.getAllByTestId("bot-input-badge-row");
    // 回归锁：全局计数形态会让三行各渲染同一个数（3 个徽章），这里必须只有两个。
    expect(badges).toHaveLength(2);
    expect(badges[0]).toHaveAttribute("data-bot-id", "bot-a");
    expect(badges[0]).toHaveTextContent("1");
    expect(badges[1]).toHaveAttribute("data-bot-id", "bot-b");
    expect(badges[1]).toHaveTextContent("3");
    // 无 pending 的 bot-c 行内不得出现徽章
    expect(badges.some((b) => b.getAttribute("data-bot-id") === "bot-c")).toBe(false);
  });
});
