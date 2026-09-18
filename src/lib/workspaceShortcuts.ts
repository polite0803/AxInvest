// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 工作台 Tab 快捷键的**唯一真相源**：判定（match）与提示（label）必须共用同一份逻辑，
 * 否则会出现「界面提示 Ctrl+1、实际按下无效」这类显示与行为不一致的静默缺陷。
 *
 * 组合键分环境（这不是美化，是必须）：
 *   - 桌面端（Tauri）：Cmd/Ctrl + 1..8
 *   - 浏览器端：Ctrl+1..9 **被浏览器本身占用**（切浏览器标签），必须退到 Ctrl+Alt + 1..8
 */

import { isTauri } from "@/lib/invoke";
import { GATED_WORKSPACE_TABS, WORKSPACE_TABS, type WorkspaceTab } from "@/lib/workspaceTabs";

/** 当前环境的工作台快捷键修饰键显示名（桌面 "Ctrl" / 浏览器 "Ctrl+Alt"） */
function modifierLabel(): string {
  return isTauri() ? "Ctrl" : "Ctrl+Alt";
}

/**
 * 判定按键事件是否命中工作台 Tab 序号（1-based 下标 → 见 WORKSPACE_TABS 顺序）。
 * 返回 Tab key；未命中返回 null。**不**做门控判断（devtools 是否可见由调用方决定）。
 */
export function matchWorkspaceTabShortcut(e: KeyboardEvent): WorkspaceTab | null {
  const hit = /^([1-9])$/.exec(e.key);
  if (!hit) {
    return null;
  }
  const ordinal = Number(hit[1]);

  if (isTauri()) {
    // 桌面端：Cmd/Ctrl + N，且不带 Alt（避免与系统 Alt 组合冲突）
    if (!(e.metaKey || e.ctrlKey) || e.altKey) {
      return null;
    }
  } else {
    // 浏览器端：必须带 Alt，否则 Ctrl+N 被浏览器抢走
    if (!e.ctrlKey || !e.altKey) {
      return null;
    }
  }

  const meta = WORKSPACE_TABS[ordinal - 1];
  return meta ? meta.key : null;
}

/** 该 Tab 是否受设置门控（门控关闭时快捷键需一并失效，保持与切换栏一致） */
export function isWorkspaceTabGated(tab: WorkspaceTab): boolean {
  return GATED_WORKSPACE_TABS.includes(tab);
}

/** 用于 UI 提示的快捷键文案，如 "Ctrl+3"。序号取 WORKSPACE_TABS 下标 + 1。 */
export function workspaceTabShortcutLabel(tab: WorkspaceTab): string {
  const ordinal = WORKSPACE_TABS.findIndex((meta) => meta.key === tab) + 1;
  return ordinal > 0 ? `${modifierLabel()}+${ordinal}` : "";
}
