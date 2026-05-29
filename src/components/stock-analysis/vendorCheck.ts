import { invoke } from "@/lib/invoke";
import { message } from "antd";
import i18next from "i18next";

const PANEL_VENDORS: Record<string, string[]> = {
  limitup: ["ths", "baidu_stock", "iwencai"],
  dragontiger: ["eastmoney", "baidu_stock"],
  sectors: ["ths", "baidu_stock"],
  north: ["ths", "baidu_stock"],
  screener: ["eastmoney", "tencent", "ths", "baidu_stock", "iwencai", "akshare"],
  events: ["cninfo", "eastmoney", "baidu_stock"],
};

export type VendorCheckResult =
  | { status: "ok" }
  | { status: "disabled"; panelName: string }
  | { status: "backend_offline" };

export async function checkVendorEnabled(panelKey: string): Promise<VendorCheckResult> {
  const names = PANEL_VENDORS[panelKey];
  if (!names) { return { status: "ok" }; }
  try {
    const tmpl: any = await invoke("get_workflow_template", { id: "stock-analysis" });
    const vars: { name: string; value: any }[] = tmpl?.variables ?? [];
    const enabledSet = new Set<string>();
    for (const v of vars) {
      if (v.name.startsWith("vendor_") && v.value) {
        enabledSet.add(v.name.replace("vendor_", ""));
      }
    }
    if (!names.some((n) => enabledSet.has(n))) {
      message.warning(i18next.t("stockAnalysis.settings.vendor.disabled", { names: names.join(" / ") }));
      return { status: "disabled", panelName: panelKey };
    }
  } catch {
    message.error(i18next.t("stockAnalysis.settings.vendor.backendOffline"));
    return { status: "backend_offline" };
  }
  return { status: "ok" };
}
