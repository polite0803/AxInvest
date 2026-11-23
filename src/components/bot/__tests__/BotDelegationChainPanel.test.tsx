// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 多层委派链视图锁（T6'''）：rootId 过滤、depth 1/2/3 徽章与顺序、P2-b 列表项复用。
 */

import { render, screen } from "@testing-library/react";
import { App } from "antd";
import type { ComponentProps } from "react";
import { describe, expect, it } from "vitest";

import type { BotAssignment } from "@/types";
import { BotDelegationChainPanel } from "../BotDelegationChainPanel";

function node(id: string, depth: number, createdAt: number, rootId = "root"): BotAssignment {
  return {
    id,
    rootId,
    parentId: depth === 1 ? null : `parent-${depth - 1}`,
    depth,
    origin: "bot" as BotAssignment["origin"],
    workflowRunId: null,
    stepKey: null,
    requesterBotId: null,
    requesterConversationId: null,
    assigneeBotId: `bot-${id}`,
    assigneeName: id,
    mode: "dispatch",
    taskText: `任务-${id}`,
    status: "running",
    resultText: null,
    errorCode: null,
    errorDetail: null,
    startedAt: null,
    finishedAt: null,
    createdAt,
    updatedAt: createdAt,
  };
}

const ALL = [
  node("l1", 1, 10),
  node("l3-early", 3, 30),
  node("other-root", 1, 5, "other"),
  node("l2", 2, 20),
  node("l3-late", 3, 40),
];

function renderPanel(props: Partial<ComponentProps<typeof BotDelegationChainPanel>> = {}) {
  return render(
    <App>
      <BotDelegationChainPanel assignments={ALL} rootId="root" {...props} />
    </App>,
  );
}

describe("BotDelegationChainPanel", () => {
  it("按 rootId 过滤并按 depth 再 createdAt 排序", () => {
    renderPanel();
    expect(screen.getByTestId("bot-delegation-chain-order")).toHaveTextContent(
      "l1 > l2 > l3-early > l3-late",
    );
    expect(screen.queryByText("other-root")).toBeNull();
    expect(screen.getAllByTestId("bot-delegation-depth-group")).toHaveLength(3);
  });

  it("depth 1/2/3 徽章只渲染 bot.assignment.depth 文案键", () => {
    renderPanel();
    const badges = screen.getAllByTestId("bot-delegation-depth-badge");
    expect(badges.map((badge) => badge.getAttribute("data-depth"))).toEqual(["1", "2", "3"]);
    expect(badges[0].textContent).toContain("1");
    expect(badges[1].textContent).toContain("2");
    expect(badges[2].textContent).toContain("3");
  });

  it("复用 P2-b Assignment 列表项（保留 bot-assignment-item testid）", () => {
    renderPanel();
    expect(screen.getAllByTestId("bot-assignment-item")).toHaveLength(4);
  });

  it("cycle / maxDepth 错误码映射到冻结的 error 键", () => {
    const { rerender } = renderPanel({ errorCode: "BOT_DELEGATE_CYCLE" });
    expect(screen.getByTestId("bot-delegation-chain-error").textContent).toBeTruthy();
    rerender(
      <App>
        <BotDelegationChainPanel assignments={ALL} rootId="root" errorCode="BOT_DELEGATE_MAX_DEPTH" />
      </App>,
    );
    expect(screen.getByTestId("bot-delegation-chain-error").textContent).toBeTruthy();
  });
});
