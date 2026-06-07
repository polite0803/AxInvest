import { SettingOutlined } from "@ant-design/icons";
import { Button, Empty, Space, Typography } from "antd";
import { useTranslation } from "react-i18next";

export type PanelEmptyKind =
  | "noData"
  | "vendorDisabled"
  | "noStock"
  | "backendOffline"
  | "connectionFailed";

export interface PanelEmptyProps {
  kind: PanelEmptyKind;
  /** vendor names to show when kind === "vendorDisabled" */
  vendorNames?: string[];
  /** description override (defaults to i18n key) */
  description?: string;
  /** "open settings" button onClick handler (page-level) */
  onOpenSettings?: () => void;
  /** hide the "open settings" CTA */
  hideCta?: boolean;
  /** image style override */
  image?: React.ReactNode;
}

/**
 * 统一面板空状态：用于 stock-analysis 右侧栏各面板在"无数据/数据源未启用/未选股"等场景下。
 * 不再以"网络/API 不可用"搪塞，而是给出明确原因 + 可执行的恢复动作。
 */
export function PanelEmpty(props: PanelEmptyProps) {
  const { t } = useTranslation();
  const { kind, vendorNames, description, onOpenSettings, hideCta, image } = props;

  const descText = (() => {
    if (description) { return description; }
    switch (kind) {
      case "noData":
        return t("stockAnalysis.settings.panels.noData");
      case "noStock":
        return t("stockAnalysis.selectStockFirst");
      case "backendOffline":
        return t("stockAnalysis.settings.vendor.backendOffline");
      case "connectionFailed":
        return t("stockAnalysis.settings.vendor.connectionFailed");
      case "vendorDisabled":
        return vendorNames && vendorNames.length > 0
          ? t("stockAnalysis.settings.vendor.disabled", { names: vendorNames.join(" / ") })
          : t("stockAnalysis.settings.vendor.disabledGeneric");
      default:
        return t("stockAnalysis.settings.panels.noData");
    }
  })();

  const showCta = !hideCta && onOpenSettings && (kind === "vendorDisabled" || kind === "backendOffline");

  return (
    <Empty
      image={image ?? Empty.PRESENTED_IMAGE_SIMPLE}
      description={
        <Space direction="vertical" size={4} className="text-center">
          <Typography.Text type="secondary" className="text-xs">
            {descText}
          </Typography.Text>
          {showCta && (
            <Button
              size="small"
              type="link"
              icon={<SettingOutlined />}
              onClick={onOpenSettings}
              className="px-0"
            >
              {t("stockAnalysis.settings.openDataSource")}
            </Button>
          )}
        </Space>
      }
    />
  );
}
