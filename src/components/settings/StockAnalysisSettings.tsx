import { invoke } from "@/lib/invoke";
import { useCallback, useState } from "react";
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
    for (const v of vendors) {
      setVendorHealth((prev) => ({ ...prev, [v]: "pending" }));
    }
    for (const v of vendors) {
      await checkVendor(v);
    }
    setCheckingVendors(false);
  }, [checkVendor]);

  return (
    <div className="p-6 pb-12">
      <StockAnalysisConfigPanel
        showVendorHealth
        vendorHealth={vendorHealth}
        checkingVendors={checkingVendors}
        onCheckVendor={checkVendor}
        onCheckAllVendors={checkAllVendors}
      />
    </div>
  );
}
