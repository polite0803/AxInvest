import { invoke } from "@/lib/invoke";
import { message } from "antd";

const PANEL_VENDORS: Record<string, string[]> = {
  limitup: ["ths", "baidu_stock", "iwencai"],
  dragontiger: ["eastmoney", "baidu_stock"],
  sectors: ["ths", "baidu_stock"],
  north: ["ths", "baidu_stock"],
  screener: ["eastmoney", "tencent", "ths", "baidu_stock", "iwencai", "akshare"],
  events: ["cninfo", "eastmoney", "baidu_stock"],
};

/** 刷新前检查 vendor 是否开启，未开启则弹提示并返回 false */
export async function checkVendorEnabled(panelKey: string): Promise<boolean> {
  const names = PANEL_VENDORS[panelKey];
  if (!names) { return true; }
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
      message.warning(`数据源未开启：${names.join(" / ")} 均未启用`);
      return false;
    }
  } catch { /* 后端未运行不阻塞 */ }
  return true;
}
