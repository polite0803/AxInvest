// SPDX-License-Identifier: AGPL-3.0-only

// 统一通知工具 — 通知中心数据的读写入口，写入受通知偏好闸门控制

// 只 import 具体模块、不走 `@/stores` barrel：barrel 会拉起 agentStore，
// 而 agentStore 反向 import 本文件（pushNotification），构成循环依赖。
import { useSettingsStore } from "@/stores/feature/settingsStore";

const STORAGE_KEY = "axagent-notifications";
const MAX_NOTIFICATIONS = 50;

export interface Notification {
  id: string;
  type: "info" | "success" | "warning" | "error";
  title: string;
  message?: string;
  timestamp: number;
  read: boolean;
  persistent?: boolean;
  /** 按业务类别分流到对应的偏好开关；命中类别的开关优先于 type 判据 */
  category?: "import";
  action?: {
    label: string;
    onClick: () => void;
  };
}

/** 从 localStorage 读取所有通知 */
export function getNotifications(): Notification[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored ? JSON.parse(stored) : [];
  } catch {
    return [];
  }
}

/** 写入通知到 localStorage */
function saveNotifications(list: Notification[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(list));
  } catch (e) {
    console.warn("Failed to persist notifications", e);
  }
}

/** 通知偏好闸门：总闸关闭、或该条命中的分类开关关闭时整条写入路径静默。字段默认 true，故仅显式 false 才拦。 */
function isSuppressedByPreference(input: Pick<Notification, "type" | "category">): boolean {
  const settings = useSettingsStore.getState().settings;
  if (settings.notificationsEnabled === false) {
    return true;
  }
  // 分类开关优先：导入类只看 notifyImport，不叠加 notifyErrors 判据
  if (input.category === "import") {
    return settings.notifyImport === false;
  }
  return input.type === "error" && settings.notifyErrors === false;
}

/** 添加一条新通知，同时写入 localStorage 并派发 CustomEvent；被偏好闸门拦下时返回 null */
export function addNotification(
  notification: Omit<Notification, "id" | "timestamp" | "read">,
): Notification | null {
  if (isSuppressedByPreference(notification)) {
    return null;
  }

  const newNotification: Notification = {
    ...notification,
    id: `notif-${Date.now()}-${Math.random().toString(36).slice(2, 11)}`,
    timestamp: Date.now(),
    read: false,
  };

  const existing = getNotifications();
  existing.unshift(newNotification);
  if (existing.length > MAX_NOTIFICATIONS) {
    existing.splice(MAX_NOTIFICATIONS);
  }

  saveNotifications(existing);

  window.dispatchEvent(
    new CustomEvent("axagent:notification", { detail: newNotification }),
  );

  return newNotification;
}

/** 标记通知为已读 */
export function markAsRead(id: string): Notification[] {
  const list = getNotifications().map((n) => n.id === id ? { ...n, read: true } : n);
  saveNotifications(list);
  return list;
}

/** 标记全部通知为已读 */
export function markAllAsRead(): Notification[] {
  const list = getNotifications().map((n) => ({ ...n, read: true }));
  saveNotifications(list);
  return list;
}

/** 删除单条通知 */
export function dismissNotification(id: string): Notification[] {
  const list = getNotifications().filter((n) => n.id !== id);
  saveNotifications(list);
  return list;
}

/** 清除所有通知（保留 persistent） */
export function clearAllNotifications(): Notification[] {
  const list = getNotifications().filter((n) => n.persistent);
  saveNotifications(list);
  return list;
}
