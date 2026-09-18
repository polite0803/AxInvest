// SPDX-License-Identifier: AGPL-3.0-only

import { DEFAULT_WORKSPACE_TAB, isWorkspaceTab, type WorkspaceTab } from "@/lib/workspaceTabs";
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

export type { WorkspaceTab };

interface WorkspaceState {
  /** 当前激活的功能 Tab */
  activeTab: WorkspaceTab;
  /** 切换功能 Tab（仅改状态；URL 同步由调用方负责，见 WorkspaceSwitcher / useKeyboardShortcuts） */
  setActiveTab: (tab: WorkspaceTab) => void;
}

/**
 * localStorage key。刻意不用 "axagent-" 前缀：本 Tab 状态属 AxInvest 业务层，
 * 与 "axagent-tab-storage"（会话 Tab，上游基座能力）区分开。
 */
export const WORKSPACE_TAB_STORAGE_KEY = "axinvest-workspace-tab";

/**
 * 工作台 Tab 状态。
 * 管理 /chat 路由下功能 Tab 的切换（对话/仪表盘/工作流/终端/文件/知识源/多智能体/开发工具）。
 *
 * 持久化理由：工作台 Tab 是「用户的工作位置」，刷新/重开应回到原位（对齐 IDE 习惯）。
 * 它同时是「点侧栏工作台入口不回弹到对话」的修法基础 —— 只有存在持久化值，
 * 「回工作台（读上次位置）」与「我要对话（显式 ?ws=chat）」才能被区分开。
 */
export const useWorkspaceStore = create<WorkspaceState>()(
  persist(
    (set) => ({
      activeTab: DEFAULT_WORKSPACE_TAB,
      setActiveTab: (tab) => set({ activeTab: tab }),
    }),
    {
      name: WORKSPACE_TAB_STORAGE_KEY,
      storage: createJSONStorage(() => localStorage),
      // 仅持久化数据字段，不持久化 actions
      partialize: (state) => ({ activeTab: state.activeTab }),
      // 脏值防线：旧版本持久化的 Tab 可能已被删除/改名。无校验时应用会停在一个
      // WorkspaceHub 渲染不出的值上，静默落到 default 分支＝显示对话页。
      merge: (persisted, current) => {
        const saved = (persisted ?? {}) as Partial<WorkspaceState>;
        return {
          ...current,
          activeTab: isWorkspaceTab(saved.activeTab) ? saved.activeTab : current.activeTab,
        };
      },
    },
  ),
);
