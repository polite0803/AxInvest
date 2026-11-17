// SPDX-License-Identifier: AGPL-3.0-only

import { SettingsGroup } from "@/components/common/SettingsGroup";
import { invoke } from "@/lib/invoke";
import { useSettingsStore } from "@/stores";
import type { NotificationChannelConfig } from "@/types";
import { Divider, Input, Switch, Tag, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

/** 后端 `notification_list_channels` 的返回项（三态投影）。 */
interface ChannelStatus {
  id: string;
  enabled: boolean;
  configured: boolean;
  active: boolean;
}

/** 各渠道需要的字段（决定面板上显示哪些输入框）。 */
const CHANNEL_FIELDS: Record<string, readonly ("token" | "apiUrl" | "topic")[]> = {
  pushplus: ["token", "apiUrl", "topic"],
  serverchan: ["token", "apiUrl"],
  ntfy: ["apiUrl", "token", "topic"],
  gotify: ["apiUrl", "token"],
};

const CHANNEL_IDS = ["pushplus", "serverchan", "ntfy", "gotify"] as const;

function emptyChannel(id: string): NotificationChannelConfig {
  return { id, enabled: false, token: "", apiUrl: null, topic: null };
}

/**
 * 通知偏好设置 + 出站推送渠道（判据③）。
 *
 * 前四个开关是通知中心总闸/分闸；下面的渠道面板让用户填凭据 + 开关，
 * 配置随 `AppSettings.notificationChannels` 落库，保存后调用
 * `notification_apply_channel_config` 让共享 dispatcher 即时重装 ——
 * **关一个渠道只改配置，主壳代码路径不动**。
 */
export function NotificationSettings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);

  const notificationsEnabled = settings.notificationsEnabled ?? true;
  const notifyErrors = settings.notifyErrors ?? true;
  const notifyImport = settings.notifyImport ?? true;
  const notifyBackup = settings.notifyBackup ?? true;

  const [statuses, setStatuses] = useState<Record<string, ChannelStatus>>({});
  // 文本输入用本地草稿（逐键只改草稿），失焦/开关时一次性落库并重装。
  const [drafts, setDrafts] = useState<Record<string, NotificationChannelConfig>>({});

  const refreshStatuses = useCallback(async () => {
    try {
      const list = await invoke<ChannelStatus[]>("notification_list_channels");
      setStatuses(Object.fromEntries(list.map((s) => [s.id, s])));
    } catch {
      // 浏览器模式无后端：保持空状态（面板仍可编辑配置）
      setStatuses({});
    }
  }, []);

  useEffect(() => {
    void refreshStatuses();
  }, [refreshStatuses]);

  const channels = useMemo<NotificationChannelConfig[]>(
    () => settings.notificationChannels ?? [],
    [settings.notificationChannels],
  );

  const valueOf = useCallback(
    (id: string): NotificationChannelConfig => drafts[id] ?? channels.find((c) => c.id === id) ?? emptyChannel(id),
    [drafts, channels],
  );

  const stage = useCallback((id: string, patch: Partial<NotificationChannelConfig>) => {
    setDrafts((prev) => ({ ...prev, [id]: { ...(prev[id] ?? emptyChannel(id)), ...patch } }));
  }, []);

  const commit = useCallback(
    async (id: string, patch?: Partial<NotificationChannelConfig>) => {
      const current = valueOf(id);
      const merged = patch ? { ...current, ...patch } : current;
      const nextChannels = CHANNEL_IDS.map((cid) =>
        cid === id ? merged : channels.find((c) => c.id === cid) ?? emptyChannel(cid)
      );
      setDrafts((prev) => {
        const copy = { ...prev };
        delete copy[id];
        return copy;
      });
      await saveSettings({ notificationChannels: nextChannels });
      try {
        await invoke("notification_apply_channel_config");
        await refreshStatuses();
      } catch {
        // 浏览器模式无后端：仅本地保存
      }
    },
    [valueOf, channels, saveSettings, refreshStatuses],
  );

  const rowStyle = { padding: "4px 0" };

  return (
    <div className="p-6 pb-12">
      <SettingsGroup title={t("settings.notificationCenter.title")}>
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="notification:enabled"
        >
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span>{t("settings.notificationsEnabled")}</span>
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t("settings.notificationsEnabledDesc")}
            </Text>
          </div>
          <Switch
            checked={notificationsEnabled}
            onChange={(next) => saveSettings({ notificationsEnabled: next })}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="notification:errors"
        >
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span>{t("settings.notifyErrors")}</span>
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t("settings.notifyErrorsDesc")}
            </Text>
          </div>
          <Switch checked={notifyErrors} onChange={(next) => saveSettings({ notifyErrors: next })} />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="notification:import"
        >
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span>{t("settings.notifyImport")}</span>
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t("settings.notifyImportDesc")}
            </Text>
          </div>
          <Switch checked={notifyImport} onChange={(next) => saveSettings({ notifyImport: next })} />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div
          style={rowStyle}
          className="flex items-center justify-between"
          data-search-key="notification:backup"
        >
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span>{t("settings.notifyBackup")}</span>
            <Text type="secondary" style={{ fontSize: 12 }}>
              {t("settings.notifyBackupDesc")}
            </Text>
          </div>
          <Switch checked={notifyBackup} onChange={(next) => saveSettings({ notifyBackup: next })} />
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("settings.notificationChannels.title")}>
        <Text type="secondary" style={{ fontSize: 12, display: "block", marginBottom: 8 }}>
          {t("settings.notificationChannelsDesc")}
        </Text>
        {CHANNEL_IDS.map((id, idx) => {
          const cfg = valueOf(id);
          const status = statuses[id];
          const fields = CHANNEL_FIELDS[id];
          return (
            <div key={id} data-search-key={`notification:channel:${id}`}>
              {idx > 0 && <Divider style={{ margin: "8px 0" }} />}
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <span>{t(`settings.notificationChannels.${id}`)}</span>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  {status && (
                    <Tag color={status.active ? "green" : "default"}>
                      {status.active
                        ? t("settings.notificationChannels.active")
                        : status.enabled && !status.configured
                        ? t("settings.notificationChannels.needsCredentials")
                        : t("settings.notificationChannels.inactive")}
                    </Tag>
                  )}
                  <Switch checked={cfg.enabled} onChange={(next) => void commit(id, { enabled: next })} />
                </div>
              </div>
              <div style={{ display: "grid", gap: 6, marginTop: 6 }}>
                {fields.includes("token") && (
                  <Input.Password
                    placeholder={t("settings.notificationChannels.tokenPlaceholder")}
                    value={cfg.token}
                    onChange={(e) => stage(id, { token: e.target.value })}
                    onBlur={() => void commit(id)}
                  />
                )}
                {fields.includes("apiUrl") && (
                  <Input
                    placeholder={t("settings.notificationChannels.apiUrlPlaceholder")}
                    value={cfg.apiUrl ?? ""}
                    onChange={(e) => stage(id, { apiUrl: e.target.value || null })}
                    onBlur={() => void commit(id)}
                  />
                )}
                {fields.includes("topic") && (
                  <Input
                    placeholder={t("settings.notificationChannels.topicPlaceholder")}
                    value={cfg.topic ?? ""}
                    onChange={(e) => stage(id, { topic: e.target.value || null })}
                    onBlur={() => void commit(id)}
                  />
                )}
              </div>
            </div>
          );
        })}
      </SettingsGroup>
    </div>
  );
}
