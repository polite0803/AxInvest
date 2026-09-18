// SPDX-License-Identifier: AGPL-3.0-only

import { useWorkspaceTabActivity } from "@/hooks/useWorkspaceTabActivity";
import { useWorkspaceTabNavigator } from "@/hooks/useWorkspaceTabNavigator";
import { workspaceTabShortcutLabel } from "@/lib/workspaceShortcuts";
import { GATED_WORKSPACE_TABS, WORKSPACE_TAB_ICONS, WORKSPACE_TABS } from "@/lib/workspaceTabs";
import { useSettingsStore, useUIStore, useWorkspaceTabStore } from "@/stores";
import { theme } from "antd";
import { useTranslation } from "react-i18next";

/**
 * 工作台功能切换栏。
 * 紧凑的水平按钮组，位于内容区顶部，在 /chat 路由下显示。
 * 对话页作为核心枢纽，其他功能（仪表盘/工作流/终端/文件/知识源/多智能体/开发工具）以 Tab 形式切换。
 * 开发工具 Tab 由设置 show_developer_tools 门控（默认开启）。
 *
 * Tab 定义来自 @/lib/workspaceTabs（唯一真相源），与快捷键/命令面板共用，
 * 避免「切换栏有 8 个但快捷键只认 6 个」这类漂移。
 * 点击一律走 useWorkspaceTabNavigator —— 它同时写 store 与 `?ws=`，保证 Tab 可深链、可刷新恢复。
 *
 * 活动点：非当前 Tab 上有工作进行中时，右上角显示小圆点（信号见 useWorkspaceTabActivity）。
 * 只做「进行中」一色，**不做**「出错」色 —— 错误需要「已读」语义（用户看过就该熄灭），
 * 那要额外维护一份已读状态，否则会退化成常亮噪声。
 */
export function WorkspaceSwitcher() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const activeTab = useWorkspaceTabStore((s) => s.activeTab);
  const switchTab = useWorkspaceTabNavigator();
  const showDevTools = useSettingsStore((s) => s.settings.showDeveloperTools !== false);
  const deviceLayout = useUIStore((s) => s.deviceLayout);
  const activity = useWorkspaceTabActivity();
  // 移动端窄屏：只留图标，避免 8 个文字标签把切换栏压成横向滚动条
  const iconOnly = deviceLayout === "mobile";
  const visibleTabs = WORKSPACE_TABS.filter(
    (tab) => showDevTools || !GATED_WORKSPACE_TABS.includes(tab.key),
  );

  return (
    <div
      className="ax-workspace-switcher"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 2,
        padding: "4px 12px",
        backgroundColor: token.colorBgContainer,
        borderBottom: `1px solid ${token.colorBorderSecondary}`,
        flexShrink: 0,
        overflowX: "auto",
        whiteSpace: "nowrap",
      }}
    >
      {visibleTabs.map(({ key, labelKey }) => {
        const Icon = WORKSPACE_TAB_ICONS[key];
        const isActive = activeTab === key;
        const label = t(labelKey);
        const shortcut = workspaceTabShortcutLabel(key);
        // 活动点只在**非当前** Tab 上出现：当前 Tab 用户正看着，提示是多余的
        const running = !isActive && activity[key] === true;
        const activityText = t("nav.tabActivity");
        return (
          <button
            key={key}
            type="button"
            title={[
              shortcut ? `${label} (${shortcut})` : label,
              running ? activityText : null,
            ].filter(Boolean).join(" · ")}
            aria-label={running ? `${label} · ${activityText}` : label}
            aria-current={isActive ? "page" : undefined}
            onClick={() => switchTab(key)}
            style={{
              position: "relative",
              display: "inline-flex",
              alignItems: "center",
              gap: 6,
              padding: "4px 10px",
              border: "none",
              borderRadius: 6,
              cursor: "pointer",
              fontSize: 13,
              flexShrink: 0,
              fontWeight: isActive ? 500 : 400,
              color: isActive ? token.colorPrimary : token.colorTextSecondary,
              backgroundColor: isActive
                ? token.colorPrimaryBg
                : "transparent",
              transition: "all 0.15s",
            }}
            onMouseEnter={(e) => {
              if (!isActive) {
                (e.currentTarget as HTMLElement).style.backgroundColor = token.colorFillQuaternary;
              }
            }}
            onMouseLeave={(e) => {
              if (!isActive) {
                (e.currentTarget as HTMLElement).style.backgroundColor = "transparent";
              }
            }}
          >
            <Icon size={14} />
            {!iconOnly && <span className="ws-label">{label}</span>}
            {running && (
              <span
                className="ws-activity-dot"
                data-testid={`ws-activity-${key}`}
                // 纯装饰：活动信息已并入按钮的 aria-label / title
                aria-hidden="true"
                style={{
                  position: "absolute",
                  top: 3,
                  right: 3,
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  backgroundColor: token.colorPrimary,
                  pointerEvents: "none",
                }}
              />
            )}
          </button>
        );
      })}
    </div>
  );
}
