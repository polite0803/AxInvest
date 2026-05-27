import { invoke } from "@/lib/invoke";
import { Tabs } from "antd";
import { useCallback, useState } from "react";
import { ExpertPromptList } from "./ExpertPromptList";
import { RolePromptList } from "./RolePromptList";
import { StockAnalysisConfigPanel } from "./StockAnalysisConfigPanel";

export function StockAnalysisSettings() {
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
            label: "专家",
            children: <ExpertPromptList />,
          },
          {
            key: "roles",
            label: "角色",
            children: <RolePromptList />,
          },
          {
            key: "params",
            label: "参数",
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
