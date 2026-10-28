// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { FEATURE_FLAGS } from "@/constants/featureFlags";
import { NAV_GROUPS } from "@/lib/navGroups";
import { builtinNavItems } from "@/lib/navRegistry";
import { useAgentPanelStore, useOnboardingStore } from "@/stores";
import { theme } from "antd";
import { ArrowLeft, Bot, HelpCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate } from "react-router-dom";

/**
 * 页面标题：按导航项最长前缀匹配（`navRegistry`），回落到导航组（`navGroups`）。
 *
 * 2026-10-28：删除原 20+ 项 `PAGE_LABELS` 手写映射 —— 它与侧栏 `navRegistry`
 * 是同一批路径的两个副本，新增页面必漏一处。现在单一真相源是 navRegistry / navGroups。
 */
function resolvePageLabel(pathname: string): string | null {
  // 股票工作台有自己的 sa-header，不再叠加 AppHeader。
  if (pathname === "/invest" || pathname.startsWith("/invest/")) {
    return null;
  }
  const item = builtinNavItems
    .filter((entry) => pathname === entry.path || pathname.startsWith(`${entry.path}/`))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (item) {
    return item.labelKey;
  }
  const group = NAV_GROUPS.find(
    (entry) => entry.path && (pathname === entry.path || pathname.startsWith(`${entry.path}/`)),
  );
  return group?.labelKey ?? null;
}

/** 非对话页面的上下文摘要 — 后续通过 store 动态注入实际数值 */
function getPageContext(pathname: string, t: (k: string) => string): string | null {
  if (pathname.startsWith("/knowledge")) { return t("appHeader.knowledgeContext"); }
  if (pathname.startsWith("/gateway")) { return t("appHeader.gatewayContext"); }
  if (pathname.startsWith("/chat/files")) { return t("appHeader.filesContext"); }
  if (pathname.startsWith("/chat/terminal")) { return t("appHeader.terminalContext"); }
  return null;
}

export function AppHeader() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const location = useLocation();
  const navigate = useNavigate();
  const toggleHelp = useOnboardingStore((s) => s.toggle);
  const toggleAgentPanel = useAgentPanelStore((s) => s.toggle);
  const isAgentPanelOpen = useAgentPanelStore((s) => s.isOpen);
  const agentInTheLoopEnabled = FEATURE_FLAGS.AGENT_IN_THE_LOOP;

  const isChatPage = location.pathname === "/chat" || location.pathname === "/" || location.pathname === "";
  const labelKey = resolvePageLabel(location.pathname);
  const contextSummary = getPageContext(location.pathname, t);

  // 对话页不显示 AppHeader（对话页有自己的 TabBar）
  if (isChatPage) { return null; }
  // 股票业务页面不显示 AppHeader（股票页面有自己的 sa-header）
  if (labelKey === null) { return null; }

  return (
    <div
      className="ax-cyber-border"
      style={{
        height: 40,
        minHeight: 40,
        minWidth: 0,
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "0 12px",
        borderBottom: `1px solid ${token.colorBorderSecondary}`,
        backgroundColor: "transparent",
        position: "relative",
        gap: 8,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, flex: 1, minWidth: 0 }}>
        {/* 返回对话 */}
        <Tooltip title={t("common.back")}>
          <button
            type="button"
            onClick={() => navigate("/")}
            className="titlebar-btn"
            aria-label={t("common.back")}
            style={{ color: token.colorTextQuaternary, flexShrink: 0 }}
          >
            <ArrowLeft size={15} />
          </button>
        </Tooltip>

        {/* 页面名 */}
        <span
          style={{
            fontSize: 13,
            fontWeight: 600,
            color: token.colorText,
            letterSpacing: "0.03em",
            whiteSpace: "nowrap",
          }}
        >
          {labelKey ? t(labelKey) : t("nav.app")}
        </span>

        {/* 上下文摘要 */}
        {contextSummary && (
          <span
            style={{
              fontSize: 11,
              color: token.colorTextQuaternary,
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {contextSummary}
          </span>
        )}
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
        {agentInTheLoopEnabled && (
          <Tooltip title={isAgentPanelOpen ? t("appHeader.closeAgentPanel") : t("appHeader.openAgentPanel")}>
            <button
              type="button"
              onClick={toggleAgentPanel}
              className={`titlebar-btn${isAgentPanelOpen ? " text-[var(--color-primary)]" : ""}`}
              aria-label={t("appHeader.agentPanel")}
              style={{ color: isAgentPanelOpen ? undefined : token.colorTextQuaternary, flexShrink: 0 }}
            >
              <Bot size={16} />
            </button>
          </Tooltip>
        )}
        <Tooltip title={t("help.title")}>
          <button
            type="button"
            onClick={toggleHelp}
            className="titlebar-btn"
            aria-label={t("help.title")}
            style={{ color: token.colorTextQuaternary, flexShrink: 0 }}
          >
            <HelpCircle size={16} />
          </button>
        </Tooltip>
      </div>
    </div>
  );
}
