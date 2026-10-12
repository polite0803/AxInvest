import { Tabs } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentProfileList } from "./AgentProfileList";
import { DataVendorsTab } from "./DataVendorsTab";
import { ExpertPromptList } from "./ExpertPromptList";
import { RolePromptList } from "./RolePromptList";
import { StockAnalysisConfigPanel } from "./StockAnalysisConfigPanel";

/**
 * 设置弹窗保留的 5 个纯分析配置 tab（FR-5.1）。
 * schedule / recommendation 两个定时业务 tab 已随系统级任务中心（/tasks）
 * 收口；旧调用方仍可能传入这两个 defaultTab，统一回退 experts。
 */
const VALID_SETTING_TABS = ["experts", "roles", "profiles", "data", "params"] as const;

function resolveInitialTab(defaultTab: string | undefined): string {
  if (defaultTab && (VALID_SETTING_TABS as readonly string[]).includes(defaultTab)) {
    return defaultTab;
  }
  return "experts";
}

export function StockAnalysisSettings({ defaultTab }: { defaultTab?: string } = {}) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState(() => resolveInitialTab(defaultTab));

  return (
    <div className="p-6 pb-12">
      <Tabs
        size="small"
        activeKey={activeTab}
        onChange={setActiveTab}
        items={[
          {
            key: "experts",
            label: t("stockAnalysis.settings.tab.experts"),
            children: <ExpertPromptList />,
          },
          {
            key: "roles",
            label: t("stockAnalysis.settings.tab.roles"),
            children: <RolePromptList />,
          },
          {
            key: "profiles",
            label: t("stockAnalysis.settings.tab.profiles"),
            children: <AgentProfileList />,
          },
          {
            key: "data",
            label: t("stockAnalysis.settings.tab.data"),
            children: <DataVendorsTab />,
          },
          {
            key: "params",
            label: t("stockAnalysis.settings.tab.params"),
            children: <StockAnalysisConfigPanel />,
          },
        ]}
      />
    </div>
  );
}
