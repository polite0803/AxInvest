// QuantLab — 量化实验台主入口
// 4 个 tab：回测运行 / 策略管理 / Rhai 编辑器 / 指标对比

import { Card, Tabs, Typography } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { BacktestTab } from "@/components/quant/tabs/BacktestTab";
import { CompareTab } from "@/components/quant/tabs/CompareTab";
import { RhaiEditorTab } from "@/components/quant/tabs/RhaiEditorTab";
import { StrategyListTab } from "@/components/quant/tabs/StrategyListTab";

const { Title, Paragraph } = Typography;

type TabKey = "backtest" | "strategies" | "rhai" | "compare";

export function QuantLab() {
  const { t } = useTranslation();
  const [active, setActive] = useState<TabKey>("backtest");

  return (
    <div style={{ padding: "16px 24px", maxWidth: 1400, margin: "0 auto" }}>
      <Card
        size="small"
        style={{ marginBottom: 16 }}
        styles={{ body: { padding: "12px 20px" } }}
      >
        <Title level={4} style={{ margin: 0 }}>
          {t("quant.title")}
        </Title>
        <Paragraph type="secondary" style={{ margin: "4px 0 0" }}>
          {t("quant.subtitle")}
        </Paragraph>
      </Card>

      <Tabs
        activeKey={active}
        onChange={(k) => setActive(k as TabKey)}
        items={[
          {
            key: "backtest",
            label: t("quant.tabs.backtest"),
            children: <BacktestTab />,
          },
          {
            key: "strategies",
            label: t("quant.tabs.strategies"),
            children: <StrategyListTab />,
          },
          {
            key: "rhai",
            label: t("quant.tabs.rhai"),
            children: <RhaiEditorTab />,
          },
          {
            key: "compare",
            label: t("quant.tabs.compare"),
            children: <CompareTab />,
          },
        ]}
      />
    </div>
  );
}
