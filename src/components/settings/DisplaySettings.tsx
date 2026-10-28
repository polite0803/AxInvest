// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { formatThemeName, SHIKI_DARK_THEMES, SHIKI_LIGHT_THEMES } from "@/constants/codeThemes";
import { invoke, isTauri, logIpcError } from "@/lib/invoke";
import { useSettingsStore } from "@/stores";
import { PRESET_PREVIEW, type ThemePreset } from "@/theme/shadcnTheme";
import { ColorPicker, Divider, Segmented, Slider } from "antd";
import { Monitor, Moon, Sun } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "./SettingsGroup";
import { SettingsSelect } from "./SettingsSelect";

export function DisplaySettings() {
  const { t } = useTranslation();

  const THEME_PRESETS: {
    key: ThemePreset;
    label: string;
  }[] = [
    // 2026 设计改版新增预设
    { key: "deep-dusk", label: t("settings.themePresetDeepDusk") },
    { key: "light-dawn", label: t("settings.themePresetLightDawn") },
    { key: "oceanic-dark", label: t("settings.themePresetOceanicDark") },
    { key: "forest-dark", label: t("settings.themePresetForestDark") },
    // 21th 设计系统 — 新野兽派风格
    { key: "21th-dark", label: t("settings.themePreset21thDark") },
    { key: "21th-light", label: t("settings.themePreset21thLight") },
    // 经典预设
    { key: "cyberpunk-dark", label: "Cyberpunk Dark" },
    { key: "cyberpunk-light", label: "Cyberpunk Light" },
    { key: "paperclip-dark", label: "Paperclip Dark" },
    { key: "paperclip-light", label: "Paperclip Light" },
    { key: "dark-elegance", label: "Dark Elegance" },
    { key: "dark-neon", label: "Dark Neon" },
    { key: "light-professional", label: "Light Pro" },
    { key: "light-minimal", label: "Light Minimal" },
  ];

  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.saveSettings);
  const [systemFonts, setSystemFonts] = useState<string[]>([]);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }
    invoke<string[]>("list_system_fonts")
      .then(setSystemFonts)
      .catch(logIpcError("list_system_fonts"));
  }, []);

  const rowStyle = { padding: "4px 0" };

  const lightThemeOptions = useMemo(
    () =>
      SHIKI_LIGHT_THEMES.map((id) => ({
        label: formatThemeName(id),
        value: id,
      })),
    [],
  );
  const darkThemeOptions = useMemo(
    () =>
      SHIKI_DARK_THEMES.map((id) => ({
        label: formatThemeName(id),
        value: id,
      })),
    [],
  );

  return (
    <div className="p-6 pb-12">
      <SettingsGroup title={t("settings.groupTheme")}>
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="display:themeMode">
          <span>{t("settings.theme.label")}</span>
          <Segmented
            data-testid="dark-mode-toggle"
            value={settings.themeMode}
            onChange={(val) => saveSettings({ themeMode: val as string })}
            options={[
              {
                label: t("settings.themeSystem"),
                value: "system",
                icon: <Monitor size={14} />,
              },
              {
                label: t("settings.themeLight"),
                value: "light",
                icon: <Sun size={14} />,
              },
              {
                label: t("settings.themeDark"),
                value: "dark",
                icon: <Moon size={14} />,
              },
            ]}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div
          style={{ ...rowStyle, display: "flex", flexWrap: "wrap", alignItems: "flex-start", gap: 8 }}
          data-search-key="display:themePreset"
        >
          <span style={{ paddingTop: 4 }}>{t("settings.themePreset")}</span>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap", justifyContent: "flex-end" }}>
            {THEME_PRESETS.map((preset) => {
              const isActive = (settings.themePreset || "deep-dusk") === preset.key;
              const preview = PRESET_PREVIEW[preset.key];
              return (
                <Tooltip key={preset.key} title={preset.label}>
                  <div
                    role="button"
                    tabIndex={0}
                    onClick={() => saveSettings({ themePreset: preset.key })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        saveSettings({ themePreset: preset.key });
                      }
                    }}
                    style={{
                      width: 48,
                      height: 48,
                      borderRadius: 8,
                      backgroundColor: preview.bg,
                      cursor: "pointer",
                      border: isActive
                        ? `2px solid ${preview.accent}`
                        : "2px solid transparent",
                      display: "flex",
                      flexDirection: "column",
                      alignItems: "center",
                      justifyContent: "center",
                      gap: 4,
                      transition: "border-color 0.2s",
                      boxShadow: isActive
                        ? `0 0 0 1px ${preview.accent}`
                        : "none",
                    }}
                  >
                    <div
                      style={{
                        width: 20,
                        height: 4,
                        borderRadius: 2,
                        backgroundColor: preview.accent,
                      }}
                    />
                    <div style={{ display: "flex", gap: 2 }}>
                      <div
                        style={{
                          width: 8,
                          height: 8,
                          borderRadius: 2,
                          backgroundColor: preview.accent,
                          opacity: 0.6,
                        }}
                      />
                      <div
                        style={{
                          width: 8,
                          height: 8,
                          borderRadius: 2,
                          backgroundColor: preview.accent,
                          opacity: 0.3,
                        }}
                      />
                    </div>
                  </div>
                </Tooltip>
              );
            })}
          </div>
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="display:primaryColor">
          <span>{t("settings.primaryColor")}</span>
          <div
            style={{
              display: "flex",
              gap: 8,
              alignItems: "center",
              flexWrap: "wrap",
            }}
          >
            {[
              "#17A93D",
              "#1677ff",
              "#1890ff",
              "#13c2c2",
              "#2f54eb",
              "#722ed1",
              "#eb2f96",
              "#fa541c",
              "#faad14",
              "#fadb14",
              "#a0d911",
              "#000000",
            ].map((color) => (
              <div
                key={color}
                role="button"
                tabIndex={0}
                onClick={() => saveSettings({ primaryColor: color })}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    saveSettings({ primaryColor: color });
                  }
                }}
                style={{
                  width: 24,
                  height: 24,
                  borderRadius: "50%",
                  backgroundColor: color,
                  cursor: "pointer",
                  border: settings.primaryColor === color
                    ? "2px solid currentColor"
                    : "2px solid transparent",
                  boxShadow: settings.primaryColor === color
                    ? `0 0 0 1px ${color}`
                    : "none",
                  transition: "box-shadow 0.2s, transform 0.2s",
                }}
              />
            ))}
            <ColorPicker
              value={settings.primaryColor}
              onChangeComplete={(color) => saveSettings({ primaryColor: color.toHexString() })}
              size="small"
            />
          </div>
        </div>
      </SettingsGroup>
      <SettingsGroup title={t("settings.groupFontRadius")}>
        <div style={{ padding: "4px 0" }} data-search-key="display:fontSize">
          <span>{t("settings.fontSize")}</span>
          <Slider
            min={12}
            max={20}
            value={settings.fontSize}
            onChange={(val) => saveSettings({ fontSize: val })}
            marks={{ 12: "12", 14: "14", 16: "16", 18: "18", 20: "20" }}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={{ padding: "4px 0" }} data-search-key="display:fontWeight">
          <span>{t("settings.fontWeight")}</span>
          <Slider
            min={100}
            max={900}
            step={100}
            value={settings.fontWeight}
            onChange={(val) => saveSettings({ fontWeight: val })}
            marks={{
              100: "100",
              300: "300",
              400: "400",
              500: "500",
              700: "700",
              900: "900",
            }}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="display:fontFamily">
          <span>{t("settings.fontFamily")}</span>
          <SettingsSelect
            searchable
            value={settings.fontFamily || ""}
            onChange={(val) => saveSettings({ fontFamily: val })}
            options={[
              { label: t("settings.fontDefault"), value: "" },
              ...systemFonts.map((f) => ({ label: f, value: f })),
            ]}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="display:codeFontFamily">
          <span>{t("settings.codeFontFamily")}</span>
          <SettingsSelect
            searchable
            value={settings.codeFontFamily || ""}
            onChange={(val) => saveSettings({ codeFontFamily: val })}
            options={[
              { label: t("settings.fontDefault"), value: "" },
              ...systemFonts.map((f) => ({ label: f, value: f })),
            ]}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="display:codeThemeLight">
          <span>{t("settings.codeThemeLight")}</span>
          <SettingsSelect
            searchable
            value={settings.codeThemeLight || "github-light"}
            onChange={(val) => saveSettings({ codeThemeLight: val })}
            options={lightThemeOptions}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={rowStyle} className="flex items-center justify-between" data-search-key="display:codeThemeDark">
          <span>{t("settings.codeThemeDark")}</span>
          <SettingsSelect
            searchable
            value={settings.codeTheme || "poimandres"}
            onChange={(val) => saveSettings({ codeTheme: val })}
            options={darkThemeOptions}
          />
        </div>
        <Divider style={{ margin: "4px 0" }} />
        <div style={{ padding: "4px 0" }} data-search-key="display:borderRadius">
          <span>{t("settings.borderRadius")}</span>
          <Slider
            min={0}
            max={20}
            value={settings.borderRadius}
            onChange={(val) => saveSettings({ borderRadius: val })}
            marks={{ 0: "0", 4: "4", 8: "8", 12: "12", 16: "16", 20: "20" }}
          />
        </div>
      </SettingsGroup>
    </div>
  );
}
