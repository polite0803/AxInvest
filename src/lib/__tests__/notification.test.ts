// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "@/stores/feature/settingsStore";
import {
  addNotification,
  clearAllNotifications,
  dismissNotification,
  getNotifications,
  markAllAsRead,
  markAsRead,
  type Notification,
} from "../notification";

/** 夹具显式声明通知偏好前提，不依赖 DEFAULT_SETTINGS 的隐式默认值 */
function setNotificationPrefs(prefs: {
  notificationsEnabled?: boolean;
  notifyErrors?: boolean;
  notifyImport?: boolean;
}) {
  useSettingsStore.setState((s) => ({ settings: { ...s.settings, ...prefs } }));
}

/** addNotification 现在可能返回 null（被偏好闸门拦下），落库类断言统一走这个非空包装 */
function push(input: Omit<Notification, "id" | "timestamp" | "read">): Notification {
  const created = addNotification(input);
  if (!created) {
    throw new Error("夹具预期放行，实际被偏好闸门拦下");
  }
  return created;
}

function makeNotif(overrides: Partial<Notification> = {}): Notification {
  return {
    id: "notif-1",
    type: "info",
    title: "Test",
    timestamp: Date.now(),
    read: false,
    ...overrides,
  };
}

describe("notification", () => {
  beforeEach(() => {
    localStorage.clear();
    setNotificationPrefs({ notificationsEnabled: true, notifyErrors: true, notifyImport: true });
  });

  describe("getNotifications", () => {
    it("无存储数据时应返回空数组", () => {
      expect(getNotifications()).toEqual([]);
    });

    it("应返回存储的通知列表", () => {
      const notifs: Notification[] = [makeNotif()];
      localStorage.setItem("axagent-notifications", JSON.stringify(notifs));
      expect(getNotifications()).toEqual(notifs);
    });

    it("解析失败时应返回空数组", () => {
      localStorage.setItem("axagent-notifications", "invalid{{");
      expect(getNotifications()).toEqual([]);
    });
  });

  describe("addNotification", () => {
    it("应创建通知并生成 id、timestamp", () => {
      const notif = push({ type: "success", title: "完成" });

      expect(notif.id).toMatch(/^notif-\d+-[a-z0-9]+$/);
      expect(notif.timestamp).toBeGreaterThan(0);
      expect(notif.read).toBe(false);
      expect(notif.type).toBe("success");
      expect(notif.title).toBe("完成");
    });

    it("应持久化到 localStorage", () => {
      push({ type: "info", title: "Test" });
      const stored = getNotifications();
      expect(stored).toHaveLength(1);
      expect(stored[0].title).toBe("Test");
    });

    it("应派发 CustomEvent", () => {
      const handler = vi.fn();
      window.addEventListener("axagent:notification", handler);

      const notif = push({ type: "info", title: "Test" });

      expect(handler).toHaveBeenCalledTimes(1);
      expect(handler.mock.calls[0][0].detail).toEqual(notif);

      window.removeEventListener("axagent:notification", handler);
    });

    it("应限制最大通知数为 50", () => {
      for (let i = 0; i < 55; i++) {
        push({ type: "info", title: `Notif ${i}` });
      }
      const stored = getNotifications();
      expect(stored).toHaveLength(50);
    });

    it("新通知应排在前面", () => {
      push({ type: "info", title: "First" });
      push({ type: "info", title: "Second" });
      const stored = getNotifications();
      expect(stored[0].title).toBe("Second");
      expect(stored[1].title).toBe("First");
    });
  });

  describe("markAsRead", () => {
    it("应标记指定通知为已读", () => {
      push({ type: "info", title: "A" });
      const notif = push({ type: "info", title: "B" });

      const updated = markAsRead(notif.id);
      const target = updated.find((n) => n.id === notif.id);
      expect(target?.read).toBe(true);
    });

    it("不应影响其他通知", () => {
      const a = push({ type: "info", title: "A" });
      push({ type: "info", title: "B" });

      const updated = markAsRead(a.id);
      const other = updated.find((n) => n.title === "B");
      expect(other?.read).toBe(false);
    });
  });

  describe("markAllAsRead", () => {
    it("应标记所有通知为已读", () => {
      push({ type: "info", title: "A" });
      push({ type: "info", title: "B" });

      const updated = markAllAsRead();
      expect(updated.every((n) => n.read)).toBe(true);
    });
  });

  describe("dismissNotification", () => {
    it("应删除指定通知", () => {
      const notif = push({ type: "info", title: "A" });
      push({ type: "info", title: "B" });

      const updated = dismissNotification(notif.id);
      expect(updated).toHaveLength(1);
      expect(updated[0].title).toBe("B");
    });
  });

  describe("clearAllNotifications", () => {
    it("应保留 persistent 通知", () => {
      push({ type: "info", title: "Normal" });
      push({ type: "info", title: "Persistent", persistent: true });

      const updated = clearAllNotifications();
      expect(updated).toHaveLength(1);
      expect(updated[0].title).toBe("Persistent");
    });
  });

  describe("通知偏好闸门", () => {
    it("总闸关闭时返回 null、不落库、不派发事件", () => {
      setNotificationPrefs({ notificationsEnabled: false });
      const handler = vi.fn();
      window.addEventListener("axagent:notification", handler);

      expect(addNotification({ type: "info", title: "被拦" })).toBeNull();

      expect(getNotifications()).toHaveLength(0);
      expect(handler).not.toHaveBeenCalled();
      window.removeEventListener("axagent:notification", handler);
    });

    it("notifyErrors 关闭时只拦 error 类，其他类型照常落库", () => {
      setNotificationPrefs({ notifyErrors: false });

      expect(addNotification({ type: "error", title: "失败" })).toBeNull();
      expect(addNotification({ type: "success", title: "完成" })).not.toBeNull();

      const stored = getNotifications();
      expect(stored).toHaveLength(1);
      expect(stored[0].title).toBe("完成");
    });

    it("偏好为 undefined 时按默认放行", () => {
      useSettingsStore.setState((s) => {
        const settings = { ...s.settings };
        delete settings.notificationsEnabled;
        delete settings.notifyErrors;
        delete settings.notifyImport;
        return { settings };
      });

      expect(addNotification({ type: "error", title: "默认放行" })).not.toBeNull();
    });

    it("notifyImport 关闭时导入类全拦（success / error 都不落库）", () => {
      setNotificationPrefs({ notifyImport: false });

      expect(addNotification({ type: "success", category: "import", title: "导入完成" })).toBeNull();
      expect(addNotification({ type: "error", category: "import", title: "导入失败" })).toBeNull();
      expect(getNotifications()).toHaveLength(0);
    });

    it("分类开关优先：notifyErrors 关不误伤导入类，非导入 error 仍被拦", () => {
      setNotificationPrefs({ notifyErrors: false });

      expect(addNotification({ type: "error", category: "import", title: "导入失败" })).not.toBeNull();
      expect(addNotification({ type: "error", title: "执行失败" })).toBeNull();
      expect(getNotifications()).toHaveLength(1);
    });

    it("总闸优先于分类：总闸关时导入类即使 notifyImport 开也拦", () => {
      setNotificationPrefs({ notificationsEnabled: false, notifyImport: true });

      expect(addNotification({ type: "success", category: "import", title: "导入完成" })).toBeNull();
    });
  });
});
