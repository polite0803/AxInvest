// SPDX-License-Identifier: AGPL-3.0-only

/**
 * BotSoulPanel：查看 / 漂移提示 / 编辑保存（PLAN T4「SOUL 面板」）。
 * store 替身化：只验证面板对 `read_bot_soul` / `write_bot_soul` 的编排。
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { BotSoulPanel } from "../BotSoulPanel";

const h = vi.hoisted(() => ({ readSoul: vi.fn(), writeSoul: vi.fn() }));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/stores/feature/botStore", () => ({
  useBotStore: (selector: (state: typeof h) => unknown) => selector(h),
}));

describe("BotSoulPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    h.readSoul.mockResolvedValue({ content: "# 你是投研助手", hash: "abcd1234", drift: false });
    h.writeSoul.mockResolvedValue({ content: "# 新", hash: "eeff0011", drift: false });
  });

  it("渲染 SOUL 正文，未漂移时不显示提示", async () => {
    render(<BotSoulPanel botId="bot-1" />);

    expect(await screen.findByText("# 你是投研助手")).toBeInTheDocument();
    expect(h.readSoul).toHaveBeenCalledWith("bot-1");
    expect(screen.queryByTestId("bot-soul-drift")).not.toBeInTheDocument();
  });

  it("漂移时显示提示（P1 只提示，不自动合并）", async () => {
    h.readSoul.mockResolvedValue({ content: "# 库内", hash: "abcd1234", drift: true });
    render(<BotSoulPanel botId="bot-1" />);

    expect(await screen.findByTestId("bot-soul-drift")).toHaveTextContent("bots.soulDrift");
  });

  it("空 SOUL 显示空态", async () => {
    h.readSoul.mockResolvedValue({ content: "   ", hash: "", drift: false });
    render(<BotSoulPanel botId="bot-1" />);

    expect(await screen.findByText("bots.soulEmpty")).toBeInTheDocument();
  });

  it("读取失败显示错误态", async () => {
    h.readSoul.mockRejectedValue(new Error("BOT_NOT_FOUND"));
    render(<BotSoulPanel botId="bot-1" />);

    expect(await screen.findByText("bots.soulLoadFailed")).toBeInTheDocument();
  });

  it("编辑后保存调用 write_bot_soul，并回到查看态", async () => {
    render(<BotSoulPanel botId="bot-1" />);
    await screen.findByText("# 你是投研助手");

    await userEvent.click(screen.getByTestId("bot-soul-edit"));
    const editor = screen.getByTestId("bot-soul-editor");
    expect(editor).toHaveValue("# 你是投研助手");

    await userEvent.clear(editor);
    await userEvent.type(editor, "# 新");
    await userEvent.click(screen.getByTestId("bot-soul-save"));

    await waitFor(() => {
      expect(h.writeSoul).toHaveBeenCalledWith("bot-1", "# 新");
    });
    await waitFor(() => {
      expect(screen.queryByTestId("bot-soul-editor")).not.toBeInTheDocument();
    });
    expect(screen.getByText("# 新")).toBeInTheDocument();
  });

  it("保存失败留在编辑态并提示", async () => {
    h.writeSoul.mockRejectedValueOnce(new Error("BOT_SOUL_WRITE_FAILED"));
    render(<BotSoulPanel botId="bot-1" />);
    await screen.findByText("# 你是投研助手");

    await userEvent.click(screen.getByTestId("bot-soul-edit"));
    await userEvent.click(screen.getByTestId("bot-soul-save"));

    await waitFor(() => {
      expect(screen.getByText("bots.saveFailed")).toBeInTheDocument();
    });
    expect(screen.getByTestId("bot-soul-editor")).toBeInTheDocument();
  });
});
