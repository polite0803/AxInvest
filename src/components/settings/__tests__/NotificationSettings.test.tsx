// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 通知偏好面板的呈现与接线约束
 *
 * 钉死三条：
 * 1. **本节不得渲染成空白** —— 该 section 原先挂的是 Popover 浮层组件（trigger 被换成空
 *    `<span/>`），整节 DOM 只剩一个空元素；这里断言四条开关真实落在页面正文里。
 * 2. **开关读的是 settings 里的真实字段** —— 四个字段各映射一条，读错字段会被 checked 断言抓住。
 * 3. **拨动会写回对应字段** —— saveSettings 收到的是本行的字段名，不是别的。
 */

import { act, fireEvent, render, screen } from "@testing-library/react";
import { App } from "antd";
import { describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "@/stores/feature/settingsStore";
import { NotificationSettings } from "../NotificationSettings";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

interface Prefs {
  notificationsEnabled?: boolean;
  notifyErrors?: boolean;
  notifyImport?: boolean;
  notifyBackup?: boolean;
}

const ROW_KEYS = ["notification:enabled", "notification:errors", "notification:import", "notification:backup"];

function setPrefs(prefs: Prefs) {
  useSettingsStore.setState((s) => ({ settings: { ...s.settings, ...prefs } }));
}

function renderPanel() {
  return render(
    <App>
      <NotificationSettings />
    </App>,
  );
}

describe("NotificationSettings", () => {
  it("四条开关渲染在页面正文里（不是空壳）", () => {
    setPrefs({ notificationsEnabled: true, notifyErrors: true, notifyImport: true, notifyBackup: true });
    const { container } = renderPanel();

    expect(screen.getAllByRole("switch")).toHaveLength(4);
    for (const key of ROW_KEYS) {
      expect(container.querySelector(`[data-search-key="${key}"]`)).not.toBeNull();
    }
  });

  it("开关状态跟随偏好：总闸关、错误开、导入关、备份开", () => {
    setPrefs({ notificationsEnabled: false, notifyErrors: true, notifyImport: false, notifyBackup: true });
    renderPanel();

    expect(screen.getAllByRole("switch").map((s) => s.getAttribute("aria-checked"))).toEqual([
      "false",
      "true",
      "false",
      "true",
    ]);
  });

  it("逐行拨动写回各自字段：enabled / errors / import / backup", async () => {
    setPrefs({ notificationsEnabled: true, notifyErrors: true, notifyImport: true, notifyBackup: true });
    const originalSave = useSettingsStore.getState().saveSettings;
    const saveSettings = vi.fn();
    useSettingsStore.setState({ saveSettings });

    try {
      renderPanel();

      await act(async () => {
        for (const s of screen.getAllByRole("switch")) {
          fireEvent.click(s);
        }
      });

      expect(saveSettings).toHaveBeenNthCalledWith(1, { notificationsEnabled: false });
      expect(saveSettings).toHaveBeenNthCalledWith(2, { notifyErrors: false });
      expect(saveSettings).toHaveBeenNthCalledWith(3, { notifyImport: false });
      expect(saveSettings).toHaveBeenNthCalledWith(4, { notifyBackup: false });
    } finally {
      // 还原 action 会让仍挂载的组件重渲染（saveSettings 是组件订阅的字段），须在 act 内
      await act(async () => {
        useSettingsStore.setState({ saveSettings: originalSave });
      });
    }
  });
});
