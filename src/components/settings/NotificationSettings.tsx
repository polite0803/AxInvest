// SPDX-License-Identifier: AGPL-3.0-only

import { useSettingsStore } from "@/stores";
import { Divider, Switch, Typography } from "antd";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "./SettingsGroup";

const { Text } = Typography;

/**
 * 通知偏好设置 — 四个已定义但此前从未接 UI 的开关：notificationsEnabled（总闸，覆盖
 * lib/notification.ts 的通知中心写入 + desktop.rs 的桌面通知发送）、notifyErrors
 * （错误类分闸，另覆盖 invoke.ts logIpcError 的错误 toast）、notifyImport（导入类分闸，
 * 覆盖 knowledgeStore / wikiStore / DirectoryImportWizard 三条发射点）、notifyBackup
 * （后端 backup.rs / webdav.rs / init/services.rs 三条定时环的是否广播事件，前端 App.tsx 接成通知）。
 */
export function NotificationSettings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);

  const notificationsEnabled = settings.notificationsEnabled ?? true;
  const notifyErrors = settings.notifyErrors ?? true;
  const notifyImport = settings.notifyImport ?? true;
  const notifyBackup = settings.notifyBackup ?? true;

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
    </div>
  );
}
