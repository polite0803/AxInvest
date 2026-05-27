import { Tabs } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentProfileList } from "./AgentProfileList";
import { DataVendorsTab } from "./DataVendorsTab";
import { ExpertPromptList } from "./ExpertPromptList";
import { RolePromptList } from "./RolePromptList";
import { StockAnalysisConfigPanel } from "./StockAnalysisConfigPanel";

export function StockAnalysisSettings() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState("experts");

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
