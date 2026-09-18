// SPDX-License-Identifier: AGPL-3.0-only

import { useWorkspaceTabNavigator } from "@/hooks/useWorkspaceTabNavigator";
import { executeShortcutAction } from "@/lib/shortcutActions";
import { getShortcutBinding, matchesShortcutEvent, SHORTCUT_ACTIONS } from "@/lib/shortcuts";
import { isWorkspaceTabGated, matchWorkspaceTabShortcut } from "@/lib/workspaceShortcuts";
import { useConversationStore, useSettingsStore, useTabStore, useUIStore } from "@/stores";
import { useCallback, useEffect } from "react";
import { useNavigate } from "react-router-dom";

export function useKeyboardShortcuts(): void {
  const navigate = useNavigate();
  const settings = useSettingsStore((s) => s.settings);
  const switchWorkspaceTab = useWorkspaceTabNavigator();

  const handleKeyDown = useCallback(
    async (e: KeyboardEvent) => {
      // 在输入框或工作流画布中不触发全局快捷键，避免与 useWorkflowShortcuts 冲突
      const isInputField = e.target instanceof HTMLInputElement
        || e.target instanceof HTMLTextAreaElement
        || e.target instanceof HTMLSelectElement;
      const isWorkflowCanvas = e.target instanceof HTMLElement && e.target.closest(".react-flow") != null;
      if (isInputField || isWorkflowCanvas) {
        return;
      }

      // ── Tab navigation shortcuts (Ctrl+Tab / Ctrl+Shift+Tab) ──
      const isMod = e.metaKey || e.ctrlKey;
      if (isMod && e.key === "Tab") {
        e.preventDefault();
        const { tabs, activeTabId, setActiveTab } = useTabStore.getState();
        if (tabs.length <= 1) {
          return;
        }
        const currentIdx = tabs.findIndex((t) => t.id === activeTabId);
        if (currentIdx === -1) {
          return;
        }
        const direction = e.shiftKey ? -1 : 1;
        const nextIdx = (currentIdx + direction + tabs.length) % tabs.length;
        setActiveTab(tabs[nextIdx].id);
        return;
      }

      // ── 工作台功能 Tab 快捷键（桌面 Ctrl/Cmd+1..8；浏览器 Ctrl+Alt+1..8）──
      // 组合键判定与界面提示共用 @/lib/workspaceShortcuts，避免「提示的是 Ctrl+1、按下无效」。
      const workspaceTab = matchWorkspaceTabShortcut(e);
      if (workspaceTab) {
        // 门控 Tab（开发工具）被设置关闭时，快捷键一并失效 —— 与切换栏的可见性判断保持一致
        if (isWorkspaceTabGated(workspaceTab) && settings.showDeveloperTools === false) {
          return;
        }
        e.preventDefault();
        switchWorkspaceTab(workspaceTab);
        return;
      }

      const matchedAction = SHORTCUT_ACTIONS.find((action) => {
        const binding = getShortcutBinding(settings, action);
        return binding && matchesShortcutEvent(e, binding);
      });
      if (matchedAction) {
        console.info("[shortcut-local-hit]", {
          action: matchedAction,
          binding: getShortcutBinding(settings, matchedAction),
          key: e.key,
          metaKey: e.metaKey,
          ctrlKey: e.ctrlKey,
          shiftKey: e.shiftKey,
          altKey: e.altKey,
        });
        e.preventDefault();
        await executeShortcutAction(matchedAction);
        return;
      }

      if (!isMod) {
        return;
      }

      switch (e.key.toLowerCase()) {
        case "f":
          e.preventDefault();
          // ① 先切回对话 Tab —— 必须走唯一入口（同时写 store + URL）。
          //    原来是 navigate("/")：URL 不带 `?ws=chat`，WorkspaceHub 会按「无显式诉求」
          //    保持上次 Tab，于是停在终端/工作流时按 Ctrl+F 画面根本不动。
          switchWorkspaceTab("chat");
          // ② 再登记「聚焦会话搜索」意图。此刻 ChatSidebar 大概率尚未挂载
          //    （ChatPage 是切 Tab 后才渲染的），所以用 store 状态而非 DOM 查询 + setTimeout：
          //    旧的 `document.querySelector(".chat-sidebar-search input")` 在两种情况下
          //    必然返回 null —— 搜索框默认 searchVisible=false 根本不渲染；且非对话 Tab 时
          //    ChatSidebar 未挂载。`?.focus()` 把失败静默吞掉，表现为「快捷键无反应」。
          useUIStore.getState().requestChatSearchFocus();
          return;
        case "w":
          e.preventDefault();
          // Close the active tab instead of just clearing the conversation
          {
            const { activeTabId, closeTab } = useTabStore.getState();
            if (activeTabId) {
              closeTab(activeTabId);
            } else {
              useConversationStore.getState().setActiveConversation(null);
            }
          }
          return;
        default:
          return;
      }
    },
    [navigate, settings, switchWorkspaceTab],
  );

  const handleKeyDownEsc = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (
          window.location.pathname === "/settings"
          || window.location.pathname.startsWith("/settings/")
        ) {
          navigate("/");
          return;
        }
        window.dispatchEvent(new CustomEvent("axagent:escape"));
      }
    },
    [navigate],
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keydown", handleKeyDownEsc);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keydown", handleKeyDownEsc);
    };
  }, [handleKeyDown, handleKeyDownEsc]);
}
