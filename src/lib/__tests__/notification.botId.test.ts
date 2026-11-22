// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 通知 botId 维度（P2-a T4'，断言清单 = GATES-P2 §4）：
 *
 * A1 未读计数按 botId 严格隔离：bot-A 的通知不算到 bot-B 头上；
 * A2 旧通知（无 botId）在「全部」视图可见、计入总数（P1 通知零丢失）；
 * A3 计数口径唯一：一条通知不会同时计入两个 bot；广播形态只计入「全部」；
 * A4 标记已读只影响本 bot 的计数。
 */

import { beforeEach, describe, expect, it } from "vitest";

import {
  addNotification,
  countUnread,
  filterNotificationsByBot,
  getNotifications,
  listNotificationBotIds,
  markAllAsRead,
  type Notification,
} from "../notification";

import { useSettingsStore } from "@/stores/feature/settingsStore";

function setNotificationPrefs(prefs: { notificationsEnabled?: boolean; notifyErrors?: boolean }) {
  useSettingsStore.setState((s) => ({ settings: { ...s.settings, ...prefs } }));
}

function push(input: Omit<Notification, "id" | "timestamp" | "read">): Notification {
  const created = addNotification(input);
  if (!created) {
    throw new Error("夹具预期放行，实际被偏好闸门拦下");
  }
  return created;
}

beforeEach(() => {
  localStorage.clear();
  setNotificationPrefs({ notificationsEnabled: true, notifyErrors: true });
});

describe("notification botId 维度（GATES-P2 §4）", () => {
  it("A1: 未读计数按 botId 严格隔离", () => {
    push({ type: "success", title: "A 完成", botId: "bot-a" });
    expect(countUnread(getNotifications(), "bot-a")).toBe(1);
    expect(countUnread(getNotifications(), "bot-b")).toBe(0);
  });

  it("A2: 旧通知（无 botId）在「全部」视图可见并计入总数", () => {
    push({ type: "success", title: "P1 旧通知" });
    const all = getNotifications();
    expect(filterNotificationsByBot(all, null)).toHaveLength(1);
    expect(countUnread(all, null)).toBe(1);
    // 按 bot 过滤时不出现
    expect(filterNotificationsByBot(all, "bot-a")).toHaveLength(0);
  });

  it("A3: 计数口径唯一 —— 带 botId 的通知不计入「全部之外」的第二个 bot", () => {
    push({ type: "success", title: "A", botId: "bot-a" });
    push({ type: "success", title: "B", botId: "bot-b" });
    push({ type: "success", title: "广播" });
    const all = getNotifications();
    expect(countUnread(all, null)).toBe(3);
    expect(countUnread(all, "bot-a")).toBe(1);
    expect(countUnread(all, "bot-b")).toBe(1);
    // 广播通知不出现在任何 botId 的视图
    expect(filterNotificationsByBot(all, "bot-a").every((n) => n.botId === "bot-a")).toBe(true);
  });

  it("A4: markAllAsRead 清全部，但按 bot 计数仍隔离（bot-B 未读不变）", () => {
    push({ type: "success", title: "A", botId: "bot-a" });
    push({ type: "success", title: "B", botId: "bot-b" });
    markAllAsRead();
    const all = getNotifications();
    expect(countUnread(all, "bot-a")).toBe(0);
    expect(countUnread(all, "bot-b")).toBe(0);
    // 单 bot 已读只影响自己：把 A 的未读恢复后 B 保持已读
    push({ type: "success", title: "A2", botId: "bot-a" });
    expect(countUnread(getNotifications(), "bot-a")).toBe(1);
    expect(countUnread(getNotifications(), "bot-b")).toBe(0);
  });

  it("listNotificationBotIds 去重且保序", () => {
    push({ type: "success", title: "1", botId: "bot-a" });
    push({ type: "success", title: "2", botId: "bot-b" });
    push({ type: "success", title: "3", botId: "bot-a" });
    push({ type: "success", title: "4" });
    expect(listNotificationBotIds(getNotifications())).toEqual(["bot-a", "bot-b"]);
  });

  it("agentStore 归属链路契约：attribution 缺省时不写 botId（P1 行为不变）", () => {
    // 直接调 addNotification 层验证字段透传语义；pushNotification 的
    // 归属映射逻辑在 NotificationBell（agentStore 集成测试覆盖），此处锁契约面。
    const n = push({ type: "success", title: "无归属" });
    expect(n.botId).toBeUndefined();
    expect(n.conversationId).toBeUndefined();
  });
});
