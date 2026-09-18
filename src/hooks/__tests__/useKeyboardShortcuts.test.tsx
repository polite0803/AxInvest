// SPDX-License-Identifier: AGPL-3.0-only

import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { useUIStore, useWorkspaceTabStore } from "@/stores";
import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it } from "vitest";

function wrapper({ children }: { children: ReactNode }) {
  return <MemoryRouter initialEntries={["/chat?ws=terminal"]}>{children}</MemoryRouter>;
}

function pressCtrlF(target: EventTarget = window): void {
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true }),
    );
  });
}

describe("useKeyboardShortcuts — Ctrl+F 会话搜索", () => {
  beforeEach(() => {
    useWorkspaceTabStore.setState({ activeTab: "terminal" });
    useUIStore.getState().consumeChatSearchFocus();
  });

  it("停在非对话 Tab 时按 Ctrl+F：切回对话 Tab **且**登记聚焦请求", () => {
    renderHook(() => useKeyboardShortcuts(), { wrapper });

    pressCtrlF();

    // 只切 Tab 不登记请求 ⇒ 搜索框不会展开、也不会聚焦；
    // 只登记请求不切 Tab ⇒ 停在终端时 ChatSidebar 未挂载，请求无人消费。
    expect(useWorkspaceTabStore.getState().activeTab).toBe("chat");
    expect(useUIStore.getState().chatSearchFocusRequest).toBe(1);
  });

  it("每次 Ctrl+F 都是新请求（计数累加），消费后归零", () => {
    renderHook(() => useKeyboardShortcuts(), { wrapper });

    pressCtrlF();
    pressCtrlF();

    expect(useUIStore.getState().chatSearchFocusRequest).toBe(2);
    useUIStore.getState().consumeChatSearchFocus();
    expect(useUIStore.getState().chatSearchFocusRequest).toBe(0);
  });

  it("在输入框内按 Ctrl+F 不触发（不抢编辑器原生查找）", () => {
    renderHook(() => useKeyboardShortcuts(), { wrapper });

    const input = document.createElement("input");
    document.body.appendChild(input);
    pressCtrlF(input);
    input.remove();

    expect(useUIStore.getState().chatSearchFocusRequest).toBe(0);
    expect(useWorkspaceTabStore.getState().activeTab).toBe("terminal");
  });
});
