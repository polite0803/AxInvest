import { StockAnalysisConfigPanel } from "@/components/settings/StockAnalysisConfigPanel";
import { invoke } from "@/lib/invoke";
import { Drawer } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

export function StockAnalysisSettingsModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const [health, setHealth] = useState<Record<string, "ok" | "fail" | "pending">>({});
  const [checking, setChecking] = useState(false);

  const checkVendor = async (name: string) => {
    const key = name.replace(/^vendor_/, "");
    const mapped: Record<string, string> = { baidu_stock: "baiduStock" };
    const vendor = mapped[key] ?? key;
    try {
      await invoke("check_vendor_health", { vendor });
      setHealth((prev) => ({ ...prev, [name]: "ok" }));
    } catch {
      setHealth((prev) => ({ ...prev, [name]: "fail" }));
    }
  };

  const checkAll = async () => {
    setChecking(true);
    const keys = [
      "vendor_tencent",
      "vendor_eastmoney",
      "vendor_sina",
      "vendor_ths",
      "vendor_cninfo",
      "vendor_baidu_stock",
      "vendor_iwencai",
      "vendor_akshare",
      "vendor_mootdx",
    ];
    for (const k of keys) { await checkVendor(k); }
    setChecking(false);
  };

  return (
    <Drawer
      title={t("stockAnalysis.settings.title")}
      placement="right"
      rootClassName="sacp-drawer"
      open={open}
      onClose={onClose}
    >
      <StockAnalysisConfigPanel
        showVendorHealth
        vendorHealth={health}
        checkingVendors={checking}
        onCheckVendor={checkVendor}
        onCheckAllVendors={checkAll}
      />
    </Drawer>
  );
}
