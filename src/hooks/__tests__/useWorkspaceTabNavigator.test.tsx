// SPDX-License-Identifier: AGPL-3.0-only

import { useWorkspaceTabNavigator } from "@/hooks/useWorkspaceTabNavigator";
import { useWorkspaceTabStore } from "@/stores";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { beforeEach, describe, expect, it } from "vitest";

/** 探针：暴露导航后的真实地址，断言必须打在 URL 上（而非只断言 store） */
function NavProbe() {
  const switchTab = useWorkspaceTabNavigator();
  const location = useLocation();
  return (
    <div>
      <button type="button" onClick={() => switchTab("terminal")}>
        go-terminal
      </button>
      <span data-testid="addr">{`${location.pathname}${location.search}`}</span>
    </div>
  );
}

function renderAt(entry: string) {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <NavProbe />
    </MemoryRouter>,
  );
}

function addr(): string {
  return screen.getByTestId("addr").textContent ?? "";
}

describe("useWorkspaceTabNavigator — store 与 URL 双写契约", () => {
  beforeEach(() => {
    useWorkspaceTabStore.setState({ activeTab: "chat" });
  });

  it("切换后 store 与 URL 同时更新（只写一处会让 Hub 读侧把 Tab 拉回去）", () => {
    renderAt("/chat");
    expect(addr()).toBe("/chat");

    fireEvent.click(screen.getByText("go-terminal"));

    expect(useWorkspaceTabStore.getState().activeTab).toBe("terminal");
    const params = new URLSearchParams(addr().split("?")[1] ?? "");
    expect(params.get("ws")).toBe("terminal");
  });

  it("切 Tab 时清掉 chat 专属参数（否则 WorkspaceHub 的 chat 参数分支会强制切回对话）", () => {
    renderAt("/chat?conversationId=abc&prompt=hi");

    fireEvent.click(screen.getByText("go-terminal"));

    const params = new URLSearchParams(addr().split("?")[1] ?? "");
    expect(params.get("ws")).toBe("terminal");
    expect(params.has("conversationId")).toBe(false);
    expect(params.has("prompt")).toBe(false);
  });

  it("保留与 chat 无关的查询参数（如业务页带过来的 stockCode）", () => {
    renderAt("/chat?stockCode=600519");

    fireEvent.click(screen.getByText("go-terminal"));

    const params = new URLSearchParams(addr().split("?")[1] ?? "");
    expect(params.get("stockCode")).toBe("600519");
    expect(params.get("ws")).toBe("terminal");
  });

  it("从业务页切换：地址落到 /chat（业务页查询参数不带入工作台）", () => {
    renderAt("/invest?tab=workspace&view=trade");

    fireEvent.click(screen.getByText("go-terminal"));

    expect(addr().startsWith("/chat?")).toBe(true);
    expect(addr()).not.toContain("view=trade");
  });
});
