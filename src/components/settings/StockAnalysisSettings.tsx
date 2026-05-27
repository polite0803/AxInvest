import { invoke } from "@/lib/invoke";
import { Tabs } from "antd";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { ExpertPromptList } from "./ExpertPromptList";
import { RolePromptList } from "./RolePromptList";
import { StockAnalysisConfigPanel } from "./StockAnalysisConfigPanel";

export function StockAnalysisSettings() {
  const { t } = useTranslation();
  const [vendorHealth, setVendorHealth] = useState<Record<string, "ok" | "fail" | "pending">>({});
  const [checkingVendors, setCheckingVendors] = useState(false);

  const checkVendor = useCallback(async (name: string) => {
    try {
      await invoke("check_vendor_health", { vendor: name });
      setVendorHealth((prev) => ({ ...prev, [name]: "ok" }));
    } catch {
      setVendorHealth((prev) => ({ ...prev, [name]: "fail" }));
    }
  }, []);

  const checkAllVendors = useCallback(async () => {
    setCheckingVendors(true);
    const vendors = ["tencent", "eastmoney", "sina", "ths", "cninfo", "baiduStock", "iwencai", "akshare", "mootdx"];
    for (const v of vendors) { setVendorHealth((prev) => ({ ...prev, [v]: "pending" })); }
    for (const v of vendors) { await checkVendor(v); }
    setCheckingVendors(false);
  }, [checkVendor]);

  return (
    <div className="p-6 pb-12">
      <Tabs
        size="small"
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
            key: "params",
            label: t("stockAnalysis.settings.tab.params"),
            children: (
              <StockAnalysisConfigPanel
                showVendorHealth
                vendorHealth={vendorHealth}
                checkingVendors={checkingVendors}
                onCheckVendor={checkVendor}
                onCheckAllVendors={checkAllVendors}
              />
            ),
          },
        ]}
      />
    </div>
  );
}
